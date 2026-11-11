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
