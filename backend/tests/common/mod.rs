//! Shared helpers for the database integration tests.
#![allow(dead_code)]
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use xexam_backend::database::{FixedClock, Store};
use xexam_backend::models::{Answer, Item, Kind, Opt, ParsedSheet, Question, Zh};

/// Repository root = the parent of backend/ (no machine-specific path is hardcoded).
pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}
pub const FIXED_NOW: &str = "2026-03-04 05:06";
/// Id of the default administrator `admin`: the first user of every new or migrated database, who owns the data of earlier
/// versions (v0.6). Store calls on study records, statistics and settings take a user id.
pub const ADMIN: i64 = 1;

pub fn baseline(rel: &str) -> PathBuf {
    repo().join("tests/baseline").join(rel)
}

pub fn fixed(now: &str) -> Arc<FixedClock> {
    Arc::new(FixedClock(now.to_string()))
}

pub fn open_fixed(path: &Path) -> Store {
    Store::open_with_clock(path, fixed(FIXED_NOW)).expect("open store")
}

/// The bytes of the database at `path` as they are on disk: the database file followed by its `-wal` file, if any (v0.7).
///
/// In WAL mode a commit lands in `<db>-wal` first and reaches the database file only at a checkpoint (or when the last
/// connection closes), so "nothing was written" means that neither file changed. The `-shm` file is left out: it is an index
/// that readers update too.
pub fn db_bytes(path: &Path) -> Vec<u8> {
    let mut bytes = std::fs::read(path).expect("read the database file");
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    if let Ok(more) = std::fs::read(PathBuf::from(wal)) {
        bytes.extend(more);
    }
    bytes
}

pub fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

/// Adds made-up questions with these ids (sheet "Cooking", no bank) straight into the database at `db`. Since v0.6.5 an exam
/// counts only questions that exist, so tests that record exams with made-up ids create them first. They are stored as
/// retired (`active = 0`, like questions left out of a re-import): they still count in exams, but no bank lists them and
/// `count_active` does not count them, so the rest of a test sees the database as before.
#[allow(dead_code)]
pub fn seed_questions(db: &Path, ids: &[&str]) {
    let c = rusqlite::Connection::open(db).expect("open for seeding");
    for (i, id) in ids.iter().enumerate() {
        c.execute(
            "INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at)              VALUES(?1, 'Cooking', ?2, ?2, '{\"kind\": \"single\", \"stem\": \"Made up?\"}', 0, '2026-01-01 10:00', '2026-01-01 10:00')",
            rusqlite::params![id, i64::try_from(i).unwrap_or(0) + 1],
        )
        .expect("seed question");
    }
}

/// Build a parsed single-choice question by hand.
pub fn single(sheet: &str, id: &str, stem: &str, row: i64) -> Question {
    Question {
        kind: Kind::Single,
        tag: String::new(),
        stem: stem.to_string(),
        options: vec![
            Opt {
                key: "A".into(),
                text: "はい".into(),
            },
            Opt {
                key: "B".into(),
                text: "いいえ".into(),
            },
        ],
        items: vec![],
        answer: Answer::Letters(vec!["B".into()]),
        translation: "中文翻译\nA. 是\nB. 否".into(),
        english: String::new(),
        en: None,
        lang: String::new(),
        ja: None,
        zh: Some(Zh {
            stem: "中文题干".into(),
            options: Some(vec!["是".into(), "否".into()]),
            items: None,
            choices: None,
        }),
        explanation: "解析 \"quoted\" \\ back".into(),
        mnemonic: "口诀".into(),
        note: String::new(),
        explain: None,
        id: id.to_string(),
        sheet: sheet.to_string(),
        row,
    }
}

pub fn multi(sheet: &str, id: &str, stem: &str, row: i64) -> Question {
    let mut q = single(sheet, id, stem, row);
    q.kind = Kind::Multi;
    q.answer = Answer::Letters(vec!["A".into(), "B".into()]);
    q
}

pub fn hotspot(sheet: &str, id: &str, stem: &str, row: i64) -> Question {
    let mut q = single(sheet, id, stem, row);
    q.kind = Kind::Hotspot;
    q.tag = "ホットスポット".into();
    q.options = vec![];
    q.items = vec![Item {
        text: "太陽は東から昇る。".into(),
        choices: vec!["はい".into(), "いいえ".into()],
    }];
    q.answer = Answer::Indexes(vec![0]);
    q.zh = Some(Zh {
        stem: "对每句话选择".into(),
        options: None,
        items: Some(vec!["太阳从东边升起。".into()]),
        choices: None,
    });
    q
}

pub fn sheet(name: &str, usable: i64) -> ParsedSheet {
    ParsedSheet {
        name: name.to_string(),
        total: usable,
        usable,
        skipped: vec![],
        warnings: vec![],
    }
}

/// The same question as the Python-side dict (what `parse_workbook` would produce).
pub fn q_json(q: &Question) -> Value {
    serde_json::to_value(q).expect("question json")
}

pub fn sheets_json(s: &[ParsedSheet]) -> Value {
    serde_json::to_value(s).expect("sheets json")
}

pub fn py_dumps(v: &Value) -> String {
    xexam_backend::database::pyjson::to_string(v).expect("dumps")
}

// ---------------------------------------------------------------- historic schemas (v2.0)

/// Adds each `(table, column, definition)` of a history list that the table does not have yet.
fn add_columns(c: &rusqlite::Connection, list: &[(&str, &str, &str)], skip: &[&str]) {
    for (table, column, definition) in list {
        if skip.contains(column) {
            continue;
        }
        c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {definition};"))
            .expect("add column");
    }
}

fn build_historic(path: &Path, n: u32, prerelease_v4: bool) {
    use xexam_backend::database::schema as sc;
    assert!((1..=12).contains(&n), "schema {n}");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("create dir");
    }
    let c = rusqlite::Connection::open(path).expect("open historic");
    // (foreign keys are not part of any historic schema)
    c.execute_batch(sc::SCHEMA).expect("v1");
    if n >= 2 {
        c.execute_batch(sc::MIGRATE_V2_TABLES).expect("v2 tables");
        add_columns(&c, &sc::V2_COLUMNS, &[]);
        c.execute_batch(sc::MIGRATE_V2_INDEXES).expect("v2 indexes");
    }
    if n >= 3 {
        c.execute_batch(sc::MIGRATE_V3_TABLES).expect("v3 tables");
        add_columns(&c, &sc::V3_COLUMNS, &[]);
        c.execute_batch(sc::MIGRATE_V3_INDEXES).expect("v3 indexes");
    }
    if n >= 4 {
        c.execute_batch(sc::MIGRATE_V4_TABLES).expect("v4 tables");
        // The pre-release v4 stored `users.all_banks` instead of `users.new_banks`
        let skip: &[&str] = if prerelease_v4 { &["new_banks"] } else { &[] };
        add_columns(&c, &sc::V4_COLUMNS, skip);
        if prerelease_v4 {
            c.execute_batch("ALTER TABLE users ADD COLUMN all_banks INTEGER NOT NULL DEFAULT 0;")
                .expect("all_banks");
        }
        c.execute_batch(sc::MIGRATE_V4_INDEXES).expect("v4 indexes");
    }
    if n >= 5 {
        add_columns(&c, &sc::V5_COLUMNS, &[]);
    }
    if n >= 6 {
        add_columns(&c, &sc::V6_COLUMNS, &[]);
    }
    if n >= 7 {
        add_columns(&c, &sc::V7_COLUMNS, &[]);
        c.execute_batch(sc::MIGRATE_V7_INDEXES).expect("v7 indexes");
    }
    if n >= 8 {
        add_columns(&c, &sc::V8_COLUMNS, &[]);
        c.execute_batch(sc::MIGRATE_V8_TABLES).expect("v8 tables");
        c.execute_batch(sc::MIGRATE_V8_INDEXES).expect("v8 indexes");
    }
    if n >= 9 {
        add_columns(&c, &sc::V9_COLUMNS, &[]);
    }
    if n >= 10 {
        add_columns(&c, &sc::V10_COLUMNS, &[]);
    }
    if n >= 11 {
        // The v11 step on an empty file, from the same constants and in the same order as `migrate_v11` (foreign keys off, as
        // the upgrade runs it): legacy tables dropped, the uid index, then the nine rebuilds with their indexes after the rename
        c.execute_batch("PRAGMA foreign_keys=OFF").expect("fk off");
        c.execute_batch("DROP TABLE IF EXISTS stats; DROP TABLE IF EXISTS settings;")
            .expect("drop legacy tables");
        c.execute_batch(sc::V11_USERS_UID_INDEX).expect("uid index");
        for r in &sc::V11_REBUILDS {
            c.execute_batch(r.create).expect("rebuild create");
            c.execute_batch(&format!(
                "INSERT INTO new_{t}({cols}) SELECT {cols} FROM {t}; DROP TABLE {t}; ALTER TABLE new_{t} RENAME TO {t};",
                t = r.table,
                cols = r.columns
            ))
            .expect("rebuild copy");
            if !r.indexes.is_empty() {
                c.execute_batch(r.indexes).expect("rebuild indexes");
            }
        }
    }
    if n >= 12 {
        add_columns(&c, &sc::V12_COLUMNS, &[]);
        c.execute_batch(sc::V12_DISPLAY_NAME_INDEX)
            .expect("v12 index");
    }
    // schema 1 files may have no stamp at all; a stamp "1" is read the same way
    c.execute(
        "INSERT INTO meta(key, value) VALUES('schema', ?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![n.to_string()],
    )
    .expect("stamp");
}

/// A genuine, empty schema-`n` file (1..=12) built from the public schema constants in chain order, with `meta.schema = n`.
/// No default administrator and no `meta` marks: tests add self-made rows with raw rusqlite (the bundled SQLite has foreign keys ON on a raw
/// connection: switch them off first to plant orphans).
pub fn historic(path: &Path, n: u32) {
    build_historic(path, n, false);
}

/// The pre-release variant of schema 4 (v0.7 builds before its release): `users.all_banks` instead of `users.new_banks`.
pub fn historic_prerelease_v4(path: &Path) {
    build_historic(path, 4, true);
}

/// The type and name of every object a schema-12 database (v3.0) has, in `sqlite_master` rowid order.
pub const SCHEMA_12_OBJECTS_V30: [(&str, &str); 42] = [
    ("table", "meta"),
    ("index", "sqlite_autoindex_meta_1"),
    ("table", "sqlite_sequence"),
    ("table", "users"),
    ("index", "sqlite_autoindex_users_1"),
    ("table", "activity_log"),
    ("index", "idx_activity_ts"),
    ("index", "idx_activity_user"),
    ("index", "idx_activity_action"),
    ("index", "idx_activity_prune"),
    ("index", "idx_activity_src"),
    ("table", "system_settings"),
    ("index", "sqlite_autoindex_system_settings_1"),
    ("index", "idx_users_uid"),
    ("table", "banks"),
    ("index", "idx_banks_owner"),
    ("index", "idx_banks_ai_job"),
    ("table", "questions"),
    ("index", "sqlite_autoindex_questions_1"),
    ("index", "idx_questions_sheet"),
    ("index", "idx_questions_bank"),
    ("index", "idx_questions_layout"),
    ("table", "imports"),
    ("index", "idx_imports_bank"),
    ("table", "attempts"),
    ("index", "idx_attempts_user"),
    ("index", "idx_attempts_session"),
    ("table", "attempt_items"),
    ("index", "sqlite_autoindex_attempt_items_1"),
    ("index", "idx_attempt_items_question"),
    ("table", "sessions"),
    ("index", "sqlite_autoindex_sessions_1"),
    ("index", "idx_sessions_user"),
    ("table", "user_stats"),
    ("index", "sqlite_autoindex_user_stats_1"),
    ("index", "idx_user_stats_question"),
    ("table", "user_settings"),
    ("index", "sqlite_autoindex_user_settings_1"),
    ("table", "user_banks"),
    ("index", "sqlite_autoindex_user_banks_1"),
    ("index", "idx_user_banks_bank"),
    ("index", "idx_users_display_name"),
];
