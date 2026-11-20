//! v2.0, main database schema 11 (the database clean-up): the upgrade from every historic schema, orphan rules, AUTOINCREMENT
//! marks, duplicate uids, a failed rebuild, enforced foreign keys, tolerance of an older binary's re-created `stats`, and the
//! shape of a fresh file. Self-made data only.
#[path = "common/mod.rs"]
mod dbc;

use dbc::{db_bytes, historic, historic_prerelease_v4, open_fixed, tmp};
use rusqlite::types::Value as Sql;
use rusqlite::{params, Connection};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use xexam_backend::auth::Role;
use xexam_backend::database::schema::V11_REBUILDS;
use xexam_backend::database::{NewSession, Store, UserChange};
use xexam_backend::error::DbError;
use xexam_backend::models::{Attempt, AttemptResult};

// ---------------------------------------------------------------- helpers

/// A raw connection with foreign keys OFF (rusqlite's bundled SQLite switches them on by default), to plant orphans.
fn raw_off(p: &Path) -> Connection {
    let c = Connection::open(p).expect("raw open");
    c.execute_batch("PRAGMA foreign_keys=OFF").expect("fk off");
    c
}

/// A raw connection with foreign keys ON, like the store's own.
fn raw_on(p: &Path) -> Connection {
    let c = Connection::open(p).expect("raw open");
    c.execute_batch("PRAGMA foreign_keys=ON").expect("fk on");
    c
}

fn rows(c: &Connection, sql: &str) -> Vec<Vec<Sql>> {
    let mut st = c.prepare(sql).expect("prepare");
    let n = st.column_count();
    st.query_map([], |r| (0..n).map(|i| r.get::<_, Sql>(i)).collect())
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows")
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).expect("count")
}

fn text(c: &Connection, sql: &str) -> String {
    c.query_row(sql, [], |r| r.get(0)).expect("text")
}

fn exec(c: &Connection, sql: &str) {
    c.execute_batch(sql)
        .unwrap_or_else(|e| panic!("{e}: {sql}"));
}

fn has_table(c: &Connection, t: &str) -> bool {
    count(
        c,
        &format!("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='{t}'"),
    ) > 0
}

fn backups(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = match std::fs::read_dir(dir.join("backups")) {
        Ok(rd) => rd.map(|e| e.expect("entry").path()).collect(),
        Err(_) => Vec::new(),
    };
    v.sort();
    v
}

fn master(p: &Path) -> Vec<(String, String, String, Option<String>)> {
    let c = Connection::open(p).expect("open");
    let mut st = c
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY rowid")
        .expect("prepare");
    let rows = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .expect("query");
    rows.map(|r| r.expect("row")).collect()
}

fn fresh_master() -> Vec<(String, String, String, Option<String>)> {
    let dir = tmp();
    let p = dir.path().join("fresh.db");
    drop(open_fixed(&p));
    master(&p)
}

/// The nine rebuilt tables with their primary-key ordering.
const NINE: [(&str, &str); 9] = [
    ("banks", "id"),
    ("questions", "id"),
    ("imports", "id"),
    ("attempts", "id"),
    ("attempt_items", "attempt_id, seq"),
    ("sessions", "token_hash"),
    ("user_stats", "user_id, question_id"),
    ("user_settings", "user_id, key"),
    ("user_banks", "user_id, bank_id"),
];

/// Columns an upgrade step changes on purpose (attach to the legacy bank, bank snapshot, claim by the first administrator,
/// owner of a bank, start time of a session): left out when kept rows are compared.
const MUTATED: [(&str, &str); 6] = [
    ("questions", "bank_id"),
    ("imports", "bank_id"),
    ("attempts", "banks"),
    ("attempts", "user_id"),
    ("banks", "owner_id"),
    ("sessions", "started_at"),
];

/// (columns, rows) of the stable columns of every rebuilt table the file has.
type Dump = BTreeMap<String, (Vec<String>, Vec<Vec<Sql>>)>;

fn stable_dump(c: &Connection) -> Dump {
    let mut out = Dump::new();
    for (t, order) in NINE {
        if !has_table(c, t) {
            continue;
        }
        let cols: Vec<String> = c
            .prepare("SELECT name FROM pragma_table_info(?1)")
            .expect("prepare")
            .query_map(params![t], |r| r.get::<_, String>(0))
            .expect("query")
            .map(|r| r.expect("row"))
            .filter(|n| !MUTATED.contains(&(t, n.as_str())))
            .collect();
        let sql = format!("SELECT {} FROM {t} ORDER BY {order}", cols.join(", "));
        out.insert(t.to_string(), (cols, rows(c, &sql)));
    }
    out
}

fn counts(c: &Connection, tables: &[&str]) -> BTreeMap<String, i64> {
    tables
        .iter()
        .filter(|t| has_table(c, t))
        .map(|t| {
            (
                (*t).to_string(),
                count(c, &format!("SELECT COUNT(*) FROM {t}")),
            )
        })
        .collect()
}

fn sequences(c: &Connection) -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    for r in rows(c, "SELECT name, seq FROM sqlite_sequence") {
        if let (Sql::Text(n), Sql::Integer(s)) = (&r[0], &r[1]) {
            out.insert(n.clone(), *s);
        }
    }
    out
}

const TABLES_FOR_COUNTS: [&str; 12] = [
    "banks",
    "questions",
    "imports",
    "attempts",
    "attempt_items",
    "sessions",
    "user_stats",
    "user_settings",
    "user_banks",
    "users",
    "stats",
    "settings",
];

/// What the planted orphans must do (empty for a file without any).
#[derive(Default)]
struct Planted {
    /// Rows that must disappear, per table
    removed: BTreeMap<&'static str, i64>,
    /// The report entries, per relation
    orphans: BTreeMap<&'static str, i64>,
    /// Planted rows that are kept (detached / re-owned): a `WHERE` clause that leaves them out, per table
    kept: BTreeMap<&'static str, &'static str>,
}

impl Planted {
    fn removed_total(&self) -> i64 {
        self.orphans
            .iter()
            .filter(|(k, _)| {
                !matches!(
                    **k,
                    "banks.owner_id" | "questions.bank_id" | "imports.bank_id"
                )
            })
            .map(|(_, n)| n)
            .sum()
    }
}

const TS: &str = "2026-01-02 10:00";
const Q: &str = r#"{"kind": "single", "stem": "Made up?"}"#;

/// A coherent, self-made data set for a schema-`n` file (`pre`: the pre-release v4 shape).
fn seed_coherent(c: &Connection, n: u32, pre: bool) {
    exec(
        c,
        &format!(
            "INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at) VALUES \
               ('q1', 'Cooking', 1, 0, '{Q}', 1, '{TS}', '{TS}'), ('q2', 'Cooking', 2, 1, '{Q}', 1, '{TS}', '{TS}'), \
               ('q3', 'Cooking', 3, 2, '{Q}', 0, '{TS}', '{TS}'); \
             INSERT INTO stats(question_id, attempts, correct, wrong, streak, last, at) VALUES \
               ('q1', 3, 1, 2, 0, 0, '{TS}'), ('q2', 1, 1, 0, 1, 1, '{TS}'); \
             INSERT INTO attempts(id, at, sheets, mode, total, correct, percent, seconds) VALUES \
               (7, '{TS}', '[\"Cooking\"]', 'all', 2, 1, 50, 30), (9, '{TS}', '[\"Cooking\"]', 'all', 1, 1, 100, 12); \
             INSERT INTO imports(id, at, filename, added, updated, removed, restored, skipped, total) \
               VALUES (3, '{TS}', 'kitchen.xlsx', 3, 0, 0, 0, 0, 3); \
             INSERT INTO attempts(id, at, sheets, mode, total, correct, percent, seconds) VALUES (12, 'x', '[]', 'all', 0, 0, 0, 0); \
             DELETE FROM attempts WHERE id = 12; \
             INSERT INTO imports(id, at, filename, added, updated, removed, restored, skipped, total) VALUES (8, 'x', 'x', 0, 0, 0, 0, 0, 0); \
             DELETE FROM imports WHERE id = 8;"
        ),
    );
    if n >= 2 {
        exec(
            c,
            &format!(
                "INSERT INTO banks(id, name, filename, legacy_ids, created_at, updated_at) \
                   VALUES (5, 'Kitchen', 'kitchen.xlsx', 1, '{TS}', '{TS}'); \
                 INSERT INTO banks(id, name, filename, legacy_ids, created_at, updated_at) VALUES (9, 'x', 'x', 0, 'x', 'x'); \
                 DELETE FROM banks WHERE id = 9; \
                 UPDATE questions SET bank_id = 5 WHERE id IN ('q1', 'q2'); \
                 UPDATE imports SET bank_id = 5 WHERE id = 3; \
                 INSERT INTO attempt_items(attempt_id, seq, question_id, answer, correct) VALUES \
                   (7, 0, 'q1', '[\"A\"]', 0), (7, 1, 'q2', NULL, 1), (9, 0, 'q1', '[\"B\"]', 1); \
                 INSERT INTO settings(key, value) VALUES ('theme', '\"dark\"'), ('uiLang', '\"ja\"');"
            ),
        );
    }
    if n >= 3 {
        let (col, val) = if pre {
            (", all_banks", ", 1")
        } else if n >= 4 {
            (", new_banks", ", 1")
        } else {
            ("", "")
        };
        let user = |id: i64, name: &str, role: &str| {
            format!(
                "INSERT INTO users(id, username, password_hash, role, created_at, password_changed_at{col}) \
                 VALUES ({id}, '{name}', 'placeholder-hash', '{role}', '{TS}', '{TS}'{val});"
            )
        };
        exec(
            c,
            &format!(
                "{} {} {} \
                 UPDATE attempts SET user_id = 1 WHERE id = 7; UPDATE attempts SET user_id = 2 WHERE id = 9; \
                 INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at) \
                   VALUES ('tok-a', 2, '{TS}', 1700000000, 1800000000); \
                 INSERT INTO user_stats(user_id, question_id, attempts, correct, wrong, streak, last, at) VALUES \
                   (1, 'q1', 3, 1, 2, 0, 0, '{TS}'), (2, 'q1', 1, 1, 0, 1, 1, '{TS}'); \
                 INSERT INTO user_settings(user_id, key, value) VALUES (1, 'theme', '\"dark\"'), (2, 'uiLang', '\"ja\"');",
                user(1, "admin", "admin"),
                user(2, "ann", "user"),
                user(3, "bob", "user")
            ),
        );
    }
    if n >= 4 {
        exec(
            c,
            "UPDATE banks SET owner_id = 1 WHERE id = 5; \
             INSERT INTO user_banks(user_id, bank_id) VALUES (2, 5), (3, 5);",
        );
    }
    if n >= 8 {
        exec(
            c,
            "INSERT INTO activity_log(ts, last_ts, actor, action, result) VALUES (1700000000, 1700000000, 'system', 'test.row', 'ok')",
        );
    }
}

/// One orphan of every kind the file's shape allows.
fn plant_orphans(c: &Connection, n: u32) -> Planted {
    let mut p = Planted::default();
    if n >= 3 {
        exec(
            c,
            &format!(
                "INSERT INTO attempts(id, at, sheets, mode, total, correct, percent, seconds, user_id) \
                   VALUES (50, '{TS}', '[]', 'all', 1, 0, 0, 5, 99); \
                 INSERT INTO attempt_items(attempt_id, seq, question_id, answer, correct) VALUES (50, 0, 'q1', NULL, 0), (60, 0, 'q1', NULL, 0); \
                 INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at) \
                   VALUES ('tok-o', 98, '{TS}', 1700000000, 1800000000); \
                 INSERT INTO user_stats(user_id, question_id) VALUES (97, 'q1'); \
                 INSERT INTO user_settings(user_id, key, value) VALUES (96, 'k', '\"v\"');"
            ),
        );
        for (t, k) in [
            ("attempts", 1),
            ("attempt_items", 2),
            ("sessions", 1),
            ("user_stats", 1),
            ("user_settings", 1),
        ] {
            p.removed.insert(t, k);
        }
        p.orphans.insert("attempts.user_id", 1);
        p.orphans.insert("attempt_items.attempt_id", 2);
        p.orphans.insert("sessions.user_id", 1);
        p.orphans.insert("user_stats.user_id", 1);
        p.orphans.insert("user_settings.user_id", 1);
    }
    if n >= 4 {
        exec(
            c,
            &format!(
                "INSERT INTO user_banks(user_id, bank_id) VALUES (95, 5), (2, 77); \
                 INSERT INTO banks(id, name, filename, legacy_ids, created_at, updated_at, owner_id) \
                   VALUES (6, 'Orphan owner', 'o.xlsx', 0, '{TS}', '{TS}', 94);"
            ),
        );
        p.removed.insert("user_banks", 2);
        p.orphans.insert("user_banks", 2);
        p.orphans.insert("banks.owner_id", 1);
        p.kept.insert("banks", "id <> 6");
    }
    if n >= 2 {
        exec(
            c,
            &format!(
                "INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id) \
                   VALUES ('q9', 'Cooking', 9, 9, '{Q}', 0, '{TS}', '{TS}', 66); \
                 INSERT INTO imports(id, at, filename, added, updated, removed, restored, skipped, total, bank_id) \
                   VALUES (4, '{TS}', 'gone.xlsx', 1, 0, 0, 0, 0, 1, 66);"
            ),
        );
        p.orphans.insert("questions.bank_id", 1);
        p.orphans.insert("imports.bank_id", 1);
        p.kept.insert("questions", "id <> 'q9'");
        p.kept.insert("imports", "id <> 4");
    }
    p
}

/// The expected `(table, from, parent, to)` of every foreign key of schema 11.
const FKS: [(&str, &str, &str, &str); 10] = [
    ("attempt_items", "attempt_id", "attempts", "id"),
    ("attempts", "user_id", "users", "id"),
    ("banks", "owner_id", "users", "id"),
    ("imports", "bank_id", "banks", "id"),
    ("questions", "bank_id", "banks", "id"),
    ("sessions", "user_id", "users", "id"),
    ("user_banks", "bank_id", "banks", "id"),
    ("user_banks", "user_id", "users", "id"),
    ("user_settings", "user_id", "users", "id"),
    ("user_stats", "user_id", "users", "id"),
];

fn assert_foreign_keys(c: &Connection) {
    let mut got: Vec<(String, String, String, String)> = Vec::new();
    for t in [
        "meta",
        "users",
        "activity_log",
        "system_settings",
        "banks",
        "questions",
        "imports",
        "attempts",
        "attempt_items",
        "sessions",
        "user_stats",
        "user_settings",
        "user_banks",
    ] {
        for r in rows(
            c,
            &format!(
                "SELECT \"from\", \"table\", \"to\", on_delete FROM pragma_foreign_key_list('{t}')"
            ),
        ) {
            let s = |v: &Sql| match v {
                Sql::Text(s) => s.clone(),
                other => panic!("{other:?}"),
            };
            assert_eq!(s(&r[3]), "NO ACTION", "{t}");
            got.push((t.to_string(), s(&r[0]), s(&r[1]), s(&r[2])));
        }
    }
    got.sort();
    let mut want: Vec<_> = FKS
        .iter()
        .map(|(a, b, p, d)| {
            (
                (*a).to_string(),
                (*b).to_string(),
                (*p).to_string(),
                (*d).to_string(),
            )
        })
        .collect();
    want.sort();
    assert_eq!(got, want);
}

fn assert_clean(c: &Connection) {
    assert!(
        rows(c, "PRAGMA foreign_key_check").is_empty(),
        "foreign_key_check must be empty"
    );
    assert_eq!(text(c, "PRAGMA integrity_check"), "ok");
}

enum Case {
    Hist(u32),
    Pre4,
    Python,
}

fn run_case(case: Case, with_orphans: bool) {
    let (label, n, pre) = match case {
        Case::Hist(n) => (format!("schema {n}"), n, false),
        Case::Pre4 => ("pre-release 4".to_string(), 4, true),
        Case::Python => ("python sample".to_string(), 1, false),
    };
    let dir = tmp();
    let db = dir.path().join("xexam.db");
    let python = matches!(case, Case::Python);
    let (dump, planted) = if python {
        std::fs::copy(dbc::baseline("fixtures/sample.db"), &db).expect("copy the sample");
        (
            stable_dump(&Connection::open(&db).expect("open")),
            Planted::default(),
        )
    } else {
        if pre {
            historic_prerelease_v4(&db);
        } else {
            historic(&db, n);
        }
        let c = raw_off(&db);
        seed_coherent(&c, n, pre);
        let dump = stable_dump(&c);
        let planted = if with_orphans {
            plant_orphans(&c, n)
        } else {
            Planted::default()
        };
        (dump, planted)
    };
    // The state the backup must hold
    let (before_counts, before_seq, old_stats, old_settings) = {
        let c = Connection::open(&db).expect("open");
        (
            counts(&c, &TABLES_FOR_COUNTS),
            sequences(&c),
            rows(
                &c,
                "SELECT question_id, attempts, correct, wrong, streak, last, at FROM stats ORDER BY question_id",
            ),
            if has_table(&c, "settings") {
                rows(&c, "SELECT key, value FROM settings ORDER BY key")
            } else {
                Vec::new()
            },
        )
    };
    let had_activity = {
        let c = Connection::open(&db).expect("open");
        if has_table(&c, "activity_log") {
            count(&c, "SELECT COUNT(*) FROM activity_log")
        } else {
            0
        }
    };

    let s = open_fixed(&db);
    // ---- the backup: one file named after the old schema, holding the old state
    let found = backups(dir.path());
    assert_eq!(found.len(), 1, "{label}: {found:?}");
    let name = found[0].file_name().and_then(|f| f.to_str()).expect("name");
    assert!(
        name.starts_with(&format!("xexam-schema{n}-")),
        "{label}: {name}"
    );
    assert_eq!(
        s.migration_backup()
            .map(|p| std::fs::canonicalize(p).expect("canon")),
        Some(std::fs::canonicalize(&found[0]).expect("canon"))
    );
    {
        let b = Connection::open(&found[0]).expect("open the backup");
        assert_eq!(
            text(&b, "SELECT value FROM meta WHERE key='schema'"),
            n.to_string()
        );
        assert_eq!(text(&b, "PRAGMA integrity_check"), "ok");
        assert_eq!(
            counts(&b, &TABLES_FOR_COUNTS),
            before_counts,
            "{label}: backup counts"
        );
        assert!(
            has_table(&b, "stats"),
            "{label}: the backup still has stats"
        );
    }
    // ---- the report
    let report = s.migration_report();
    let got: BTreeMap<&str, i64> = report.orphans.iter().map(|(k, v)| (*k, *v)).collect();
    assert_eq!(got, planted.orphans, "{label}: orphans in the report");
    assert_eq!(report.uid_duplicates, 0);
    assert!(report.kept_legacy.is_empty());
    let legacy: Vec<&str> = report.legacy_rows.iter().map(|(t, _)| *t).collect();
    assert!(legacy.contains(&"stats"), "{label}: {legacy:?}");
    drop(s);

    let c = Connection::open(&db).expect("open");
    c.execute_batch("PRAGMA foreign_keys=ON").expect("fk on");
    assert_eq!(text(&c, "SELECT value FROM meta WHERE key='schema'"), "11");
    assert_clean(&c);
    assert_foreign_keys(&c);
    // ---- counts: seeded minus planted orphans
    let after = counts(&c, &TABLES_FOR_COUNTS);
    for (t, was) in &before_counts {
        if matches!(t.as_str(), "stats" | "settings") {
            assert!(!after.contains_key(t), "{label}: {t} must be gone");
            continue;
        }
        if !dump.contains_key(t) && t != "users" {
            // a table the upgrade created or filled itself (user_stats of a v1 / v2 file, ...)
            continue;
        }
        if (t == "banks" && n == 1) || (pre && t == "user_banks") {
            continue;
        }
        let want = was - planted.removed.get(t.as_str()).copied().unwrap_or(0);
        assert_eq!(after.get(t), Some(&want), "{label}: rows of {t}");
    }
    // ---- kept rows identical
    for (t, (cols, kept)) in &dump {
        let order = NINE
            .iter()
            .find(|(n, _)| n == t)
            .map(|(_, o)| *o)
            .expect("order");
        let filter = planted.kept.get(t.as_str()).copied().unwrap_or("1");
        let sql = format!(
            "SELECT {} FROM {t} WHERE {filter} ORDER BY {order}",
            cols.join(", ")
        );
        if pre && t == "user_banks" {
            // the pre-release conversion adds the rows of the users that had "every bank"
            continue;
        }
        assert_eq!(&rows(&c, &sql), kept, "{label}: kept rows of {t}");
    }
    // ---- legacy tables gone, their rows in the administrator's rows
    for t in ["stats", "settings"] {
        assert!(!has_table(&c, t), "{label}: {t}");
    }
    if n <= 2 {
        assert_eq!(
            rows(
                &c,
                "SELECT question_id, attempts, correct, wrong, streak, last, at FROM user_stats WHERE user_id = 1 ORDER BY question_id"
            ),
            old_stats,
            "{label}: stats copied to the first administrator"
        );
        assert_eq!(
            rows(
                &c,
                "SELECT key, value FROM user_settings WHERE user_id = 1 ORDER BY key"
            ),
            old_settings,
            "{label}: settings copied"
        );
    }
    // ---- the schema equals a fresh file's
    drop(c);
    if pre {
        // The pre-release conversion adds `new_banks` after the other columns and drops `all_banks` (so the stored `users` text
        // differs in column order; `users` is not rebuilt): compare everything else
        let strip = |m: Vec<(String, String, String, Option<String>)>| -> Vec<_> {
            m.into_iter()
                .filter(|r| !(r.0 == "table" && r.1 == "users"))
                .collect()
        };
        assert_eq!(
            strip(master(&db)),
            strip(fresh_master()),
            "{label}: sqlite_master"
        );
    } else {
        assert_eq!(master(&db), fresh_master(), "{label}: sqlite_master");
    }
    let c = Connection::open(&db).expect("open");
    // ---- the one migrate row
    let details = rows(
        &c,
        "SELECT detail FROM activity_log WHERE action = 'system.migrate'",
    );
    assert_eq!(details.len(), 1, "{label}");
    let Sql::Text(detail) = &details[0][0] else {
        panic!("detail")
    };
    let detail: Value = serde_json::from_str(detail).expect("json");
    assert_eq!(detail["from"], n);
    assert_eq!(detail["to"], 11);
    match planted.removed_total() {
        0 => assert!(detail.get("orphansRemoved").is_none(), "{label}: {detail}"),
        k => assert_eq!(detail["orphansRemoved"], k, "{label}"),
    }
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM activity_log"),
        had_activity + 1,
        "{label}: the old log rows stay"
    );
    // ---- AUTOINCREMENT high-water marks
    let seq = sequences(&c);
    for (name, was) in &before_seq {
        if matches!(name.as_str(), "attempts" | "imports" | "banks") {
            assert_eq!(seq.get(name), Some(was), "{label}: sqlite_sequence {name}");
        }
    }
    drop(c);
    // ---- a re-open writes nothing and makes no second backup
    let bytes = db_bytes(&db);
    let s = open_fixed(&db);
    assert!(s.migration_backup().is_none());
    assert!(s.migration_report().orphans.is_empty());
    assert!(s.migration_report().legacy_rows.is_empty());
    drop(s);
    assert_eq!(db_bytes(&db), bytes, "{label}: re-open changed the file");
    assert_eq!(backups(dir.path()).len(), 1, "{label}");
}

// ---------------------------------------------------------------- 1. every historic schema

#[test]
fn every_historic_schema_upgrades_to_11() {
    for n in 1..=10 {
        run_case(Case::Hist(n), true);
    }
}

#[test]
fn every_historic_schema_upgrades_to_11_without_orphans() {
    for n in 1..=10 {
        run_case(Case::Hist(n), false);
    }
}

#[test]
fn the_pre_release_v4_file_upgrades_to_11() {
    run_case(Case::Pre4, true);
}

#[test]
fn the_python_sample_upgrades_to_11() {
    run_case(Case::Python, false);
}

// ---------------------------------------------------------------- 2. orphan rules

#[test]
fn orphans_follow_the_rules() {
    let dir = tmp();
    let db = dir.path().join("xexam.db");
    historic(&db, 10);
    {
        let c = raw_off(&db);
        seed_coherent(&c, 10, false);
        // an unclaimed record stays NULL in the step (an allowed NULL) and is claimed by the open
        exec(
            &c,
            &format!(
                "INSERT INTO attempts(id, at, sheets, mode, total, correct, percent, seconds) \
                   VALUES (20, '{TS}', '[]', 'all', 1, 1, 100, 5);"
            ),
        );
        plant_orphans(&c, 10);
    }
    let before_q: i64 = count(
        &Connection::open(&db).expect("open"),
        "SELECT COUNT(*) FROM questions",
    );
    let s = open_fixed(&db);
    let r = s.migration_report().clone();
    drop(s);
    let get = |k: &str| r.orphans.iter().find(|(rel, _)| *rel == k).map(|(_, n)| *n);
    assert_eq!(get("attempts.user_id"), Some(1));
    assert_eq!(get("attempt_items.attempt_id"), Some(2));
    assert_eq!(get("sessions.user_id"), Some(1));
    assert_eq!(get("user_stats.user_id"), Some(1));
    assert_eq!(get("user_settings.user_id"), Some(1));
    assert_eq!(get("user_banks"), Some(2));
    assert_eq!(get("banks.owner_id"), Some(1));
    assert_eq!(get("questions.bank_id"), Some(1));
    assert_eq!(get("imports.bank_id"), Some(1));
    let c = raw_on(&db);
    assert_clean(&c);
    // deleted
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempts WHERE id = 50"), 0);
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM attempt_items WHERE attempt_id IN (50, 60)"
        ),
        0
    );
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM sessions WHERE token_hash = 'tok-o'"
        ),
        0
    );
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM user_stats WHERE user_id = 97"),
        0
    );
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM user_settings WHERE user_id = 96"),
        0
    );
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM user_banks WHERE user_id = 95 OR bank_id = 77"
        ),
        0
    );
    // kept: the question and the import are attached to the legacy bank, the bank is owned by the first administrator
    assert_eq!(count(&c, "SELECT COUNT(*) FROM questions"), before_q);
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM questions q JOIN banks b ON b.id = q.bank_id WHERE q.id = 'q9' AND b.legacy_ids = 1"
        ),
        1
    );
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM imports i JOIN banks b ON b.id = i.bank_id WHERE i.id = 4 AND b.legacy_ids = 1"
        ),
        1
    );
    assert_eq!(count(&c, "SELECT owner_id FROM banks WHERE id = 6"), 1);
    assert_eq!(count(&c, "SELECT user_id FROM attempts WHERE id = 20"), 1);
    // the activity row names the removed rows (1 + 2 + 1 + 1 + 1 + 2)
    let detail = text(
        &c,
        "SELECT detail FROM activity_log WHERE action = 'system.migrate'",
    );
    let v: Value = serde_json::from_str(&detail).expect("json");
    assert_eq!(v["orphansRemoved"], 8);
    assert_eq!((v["from"].clone(), v["to"].clone()), (10.into(), 11.into()));
    // the backup holds every removed row
    let b = Connection::open(&backups(dir.path())[0]).expect("backup");
    assert_eq!(count(&b, "SELECT COUNT(*) FROM attempts WHERE id = 50"), 1);
    assert_eq!(
        count(
            &b,
            "SELECT COUNT(*) FROM attempt_items WHERE attempt_id IN (50, 60)"
        ),
        2
    );
    assert_eq!(
        count(
            &b,
            "SELECT COUNT(*) FROM sessions WHERE token_hash = 'tok-o'"
        ),
        1
    );
    assert_eq!(
        count(
            &b,
            "SELECT COUNT(*) FROM user_banks WHERE user_id = 95 OR bank_id = 77"
        ),
        2
    );
    assert_eq!(
        count(
            &b,
            "SELECT COUNT(*) FROM banks WHERE id = 6 AND owner_id = 94"
        ),
        1
    );
}

#[test]
fn a_bank_of_a_missing_owner_stays_ownerless_without_an_administrator() {
    // No administrator at all: the step clears the owner, the every-open owner fix-up then creates the default one and
    // hands the bank over
    let dir = tmp();
    let db = dir.path().join("xexam.db");
    historic(&db, 10);
    {
        let c = raw_off(&db);
        exec(
            &c,
            &format!(
                "INSERT INTO banks(id, name, filename, legacy_ids, created_at, updated_at, owner_id) \
                   VALUES (1, 'Lonely', 'l.xlsx', 1, '{TS}', '{TS}', 42);"
            ),
        );
    }
    let s = open_fixed(&db);
    assert!(s.created_default_admin());
    drop(s);
    let c = raw_on(&db);
    assert_clean(&c);
    assert_eq!(count(&c, "SELECT owner_id FROM banks WHERE id = 1"), 1);
}

// ---------------------------------------------------------------- 3. AUTOINCREMENT marks

#[test]
fn autoincrement_marks_survive() {
    let dir = tmp();
    let db = dir.path().join("xexam.db");
    historic(&db, 10);
    {
        let c = raw_off(&db);
        exec(
            &c,
            &format!(
                "INSERT INTO users(id, username, password_hash, role, created_at, password_changed_at, new_banks) \
                   VALUES (1, 'admin', 'h', 'admin', '{TS}', '{TS}', 1); \
                 INSERT INTO banks(id, name, filename, legacy_ids, created_at, updated_at, owner_id) VALUES \
                   (1, 'A', 'a.xlsx', 1, '{TS}', '{TS}', 1), (2, 'B', 'b.xlsx', 0, '{TS}', '{TS}', 1), (3, 'C', 'c.xlsx', 0, '{TS}', '{TS}', 1); \
                 INSERT INTO attempts(id, at, sheets, mode, total, correct, percent, seconds, user_id) VALUES \
                   (1, '{TS}', '[]', 'all', 1, 1, 100, 1, 1), (2, '{TS}', '[]', 'all', 1, 1, 100, 1, 1), (5, '{TS}', '[]', 'all', 1, 1, 100, 1, 1); \
                 INSERT INTO imports(id, at, filename, added, updated, removed, restored, skipped, total, bank_id) VALUES \
                   (1, '{TS}', 'a.xlsx', 1, 0, 0, 0, 0, 1, 1), (4, '{TS}', 'c.xlsx', 1, 0, 0, 0, 0, 1, 3); \
                 INSERT INTO meta(key, value) VALUES ('banks_known', '3'), ('users_known', '1'); \
                 DELETE FROM attempts WHERE id = 5; DELETE FROM imports WHERE id = 4; DELETE FROM banks WHERE id = 3;"
            ),
        );
        let seq = sequences(&c);
        assert_eq!(seq.get("banks"), Some(&3));
        assert_eq!(seq.get("attempts"), Some(&5));
        assert_eq!(seq.get("imports"), Some(&4));
    }
    drop(open_fixed(&db));
    let c = raw_on(&db);
    let seq = sequences(&c);
    assert_eq!(seq.get("banks"), Some(&3));
    assert_eq!(seq.get("attempts"), Some(&5));
    assert_eq!(seq.get("imports"), Some(&4));
    // the next ids are the old high-water mark + 1, never a reused one
    exec(
        &c,
        &format!(
            "INSERT INTO banks(name, filename, legacy_ids, created_at, updated_at, owner_id) VALUES ('D', 'd.xlsx', 0, '{TS}', '{TS}', 1); \
             INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, user_id) VALUES ('{TS}', '[]', 'all', 1, 1, 100, 1, 1); \
             INSERT INTO imports(at, filename, added, updated, removed, restored, skipped, total, bank_id) VALUES ('{TS}', 'd.xlsx', 1, 0, 0, 0, 0, 1, 1);"
        ),
    );
    assert_eq!(count(&c, "SELECT MAX(id) FROM banks"), 4);
    assert_eq!(count(&c, "SELECT MAX(id) FROM attempts"), 6);
    assert_eq!(count(&c, "SELECT MAX(id) FROM imports"), 5);
}

#[test]
fn the_mark_of_an_empty_autoincrement_table_survives_too() {
    // The table is empty but its mark is not: nothing to copy, the mark must still be restored
    let dir = tmp();
    let db = dir.path().join("xexam.db");
    historic(&db, 10);
    {
        let c = raw_off(&db);
        exec(
            &c,
            &format!(
                "INSERT INTO users(id, username, password_hash, role, created_at, password_changed_at, new_banks) \
                   VALUES (1, 'admin', 'h', 'admin', '{TS}', '{TS}', 1); \
                 INSERT INTO attempts(id, at, sheets, mode, total, correct, percent, seconds, user_id) \
                   VALUES (30, '{TS}', '[]', 'all', 1, 1, 100, 1, 1); \
                 DELETE FROM attempts WHERE id = 30;"
            ),
        );
    }
    drop(open_fixed(&db));
    let c = raw_on(&db);
    assert_eq!(sequences(&c).get("attempts"), Some(&30));
}

// ---------------------------------------------------------------- 4. duplicate uids

#[test]
fn duplicate_uids_skip_the_unique_index() {
    let dir = tmp();
    let db = dir.path().join("xexam.db");
    historic(&db, 10);
    let same = "0123456789abcdef0123456789abcdef";
    {
        let c = raw_off(&db);
        exec(
            &c,
            &format!(
                "INSERT INTO users(id, username, password_hash, role, created_at, password_changed_at, new_banks, uid) VALUES \
                   (1, 'admin', 'h', 'admin', '{TS}', '{TS}', 1, '{same}'), \
                   (2, 'ann', 'h', 'user', '{TS}', '{TS}', 1, '{same}'), \
                   (3, 'bob', 'h', 'user', '{TS}', '{TS}', 1, 'fedcba9876543210fedcba9876543210');"
            ),
        );
    }
    let s = open_fixed(&db);
    assert_eq!(
        s.migration_report().uid_duplicates,
        1,
        "one value is shared"
    );
    drop(s);
    let c = raw_on(&db);
    assert_eq!(text(&c, "SELECT value FROM meta WHERE key='schema'"), "11");
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'idx_users_uid'"
        ),
        0
    );
    // no uid was changed
    assert_eq!(
        count(
            &c,
            &format!("SELECT COUNT(*) FROM users WHERE uid = '{same}'")
        ),
        2
    );
    // every other index and table is there
    let fresh = fresh_master();
    let mut want: Vec<_> = fresh
        .into_iter()
        .filter(|r| r.1 != "idx_users_uid")
        .collect();
    let mut got = master(&db);
    want.sort();
    got.sort();
    assert_eq!(got, want);
    drop(c);
    // while duplicates remain every open says so and writes nothing
    let bytes = db_bytes(&db);
    let s = open_fixed(&db);
    assert_eq!(s.migration_report().uid_duplicates, 1);
    drop(s);
    assert_eq!(db_bytes(&db), bytes);
    // fix by hand: the next open creates the index, the open after writes nothing
    exec(
        &raw_on(&db),
        "UPDATE users SET uid = '11111111111111111111111111111111' WHERE id = 2",
    );
    let s = open_fixed(&db);
    assert_eq!(s.migration_report().uid_duplicates, 0);
    drop(s);
    let c = raw_on(&db);
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'idx_users_uid'"
        ),
        1
    );
    drop(c);
    let bytes = db_bytes(&db);
    drop(open_fixed(&db));
    assert_eq!(db_bytes(&db), bytes);
    // the late index sits at the end of `sqlite_master` instead of after `system_settings`: same objects, same text
    let (mut got, mut want) = (master(&db), fresh_master());
    got.sort();
    want.sort();
    assert_eq!(got, want);
}

// ---------------------------------------------------------------- 5. a failed rebuild changes nothing

fn expect_migration_error(db: &Path) -> DbError {
    match Store::open_with_clock(db, dbc::fixed(dbc::FIXED_NOW)) {
        Err(e) => e,
        Ok(_) => panic!("the upgrade must stop"),
    }
}

/// A schema-10 file with a little data, one made-up administrator and an attempt.
fn small_v10(db: &Path) {
    historic(db, 10);
    let c = raw_off(db);
    exec(
        &c,
        &format!(
            "INSERT INTO users(id, username, password_hash, role, created_at, password_changed_at, new_banks) \
               VALUES (1, 'admin', 'h', 'admin', '{TS}', '{TS}', 1); \
             INSERT INTO banks(id, name, filename, legacy_ids, created_at, updated_at, owner_id) \
               VALUES (1, 'A', 'a.xlsx', 1, '{TS}', '{TS}', 1); \
             INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id) \
               VALUES ('q1', 'Cooking', 1, 0, '{Q}', 1, '{TS}', '{TS}', 1); \
             INSERT INTO meta(key, value) VALUES ('banks_known', '1'), ('users_known', '1');"
        ),
    );
}

#[test]
fn a_failed_rebuild_changes_nothing() {
    // (a) a row that violates a CHECK of the new table
    let dir = tmp();
    let db = dir.path().join("xexam.db");
    small_v10(&db);
    exec(
        &raw_off(&db),
        &format!(
            "PRAGMA ignore_check_constraints=ON; \
             INSERT INTO attempts(id, at, sheets, mode, total, correct, percent, seconds, user_id, kind) \
               VALUES (1, '{TS}', '[]', 'all', 1, 1, 100, 1, 1, 'bogus');"
        ),
    );
    let before = db_bytes(&db);
    let e = expect_migration_error(&db);
    assert!(matches!(e, DbError::Migration(_)), "{e}");
    assert!(e.to_string().contains("attempts"), "{e}");
    assert_eq!(db_bytes(&db), before, "the file is byte-identical");
    assert_eq!(
        text(&raw_on(&db), "SELECT value FROM meta WHERE key='schema'"),
        "10"
    );
    assert_eq!(backups(dir.path()).len(), 1, "the backup stays");
    // the foreign-key switch is not left off: a new connection of the same process is unaffected, and the store comes up once
    // the bad row is gone; the retry writes a second backup
    exec(&raw_off(&db), "DELETE FROM attempts WHERE kind = 'bogus'");
    let s = open_fixed(&db);
    drop(s);
    let c = raw_on(&db);
    assert_eq!(text(&c, "SELECT value FROM meta WHERE key='schema'"), "11");
    assert_clean(&c);
    assert_eq!(backups(dir.path()).len(), 2);

    // (b) an unexpected extra column is never dropped silently
    let dir = tmp();
    let db = dir.path().join("xexam.db");
    small_v10(&db);
    exec(&raw_off(&db), "ALTER TABLE questions ADD COLUMN extra TEXT");
    let before = db_bytes(&db);
    let e = expect_migration_error(&db);
    assert!(
        matches!(e, DbError::Migration(ref m) if m == "unexpected column questions.extra"),
        "{e}"
    );
    assert_eq!(db_bytes(&db), before);
    assert_eq!(
        text(&raw_on(&db), "SELECT value FROM meta WHERE key='schema'"),
        "10"
    );
    exec(&raw_off(&db), "ALTER TABLE questions DROP COLUMN extra");
    drop(open_fixed(&db));
    assert_eq!(
        text(&raw_on(&db), "SELECT value FROM meta WHERE key='schema'"),
        "11"
    );

    // (c) a trigger on a rebuilt table would be dropped with the old table: stop instead
    let dir = tmp();
    let db = dir.path().join("xexam.db");
    small_v10(&db);
    exec(
        &raw_off(&db),
        "CREATE TRIGGER mine AFTER INSERT ON questions BEGIN SELECT 1; END",
    );
    let before = db_bytes(&db);
    let e = expect_migration_error(&db);
    assert!(
        matches!(e, DbError::Migration(ref m) if m.contains("trigger")),
        "{e}"
    );
    assert_eq!(db_bytes(&db), before);
}

// ---------------------------------------------------------------- 6. foreign keys are enforced

fn attempt(ids: &[&str]) -> Attempt {
    Attempt {
        results: ids
            .iter()
            .map(|i| AttemptResult {
                id: (*i).to_string(),
                correct: true,
            })
            .collect(),
        sheets: vec!["Cooking".to_string()],
        mode: "all".to_string(),
        seconds: 5,
        kind: Default::default(),
        limit: None,
        end: None,
    }
}

fn user_id(s: &Store, name: &str, role: Role) -> i64 {
    match s
        .create_user(name, "placeholder-hash", role)
        .expect("create")
    {
        UserChange::Done(id) => id,
        other => panic!("{other:?}"),
    }
}

fn session(s: &Store, user: i64, token: &str) {
    s.create_session(&NewSession {
        token_hash: token,
        user_id: user,
        now: 1_700_000_000,
        expires_at: 1_700_000_100,
        remember: true,
        ip: None,
        ua: None,
    })
    .expect("session");
}

#[test]
fn foreign_keys_are_enforced() {
    let dir = tmp();
    let db = dir.path().join("xexam.db");
    let s = open_fixed(&db);
    let qs = vec![
        dbc::single("Cooking", "id1", "One?", 1),
        dbc::single("Cooking", "id2", "Two?", 2),
    ];
    s.apply_import("a.xlsx", &[dbc::sheet("Cooking", 2)], &qs)
        .expect("import");
    let ann = user_id(&s, "ann", Role::User);
    session(&s, ann, "tok-ann");
    let rec = s
        .record_attempt(ann, &attempt(&["id1", "id2"]))
        .expect("record");
    assert!(rec.is_object() || rec.is_array() || rec.is_null() || rec.is_number());

    // a connection with foreign keys on cannot break the links
    let c = raw_on(&db);
    let err = |sql: &str| {
        c.execute_batch(sql)
            .expect_err(sql)
            .to_string()
            .to_lowercase()
    };
    assert!(err(&format!("DELETE FROM users WHERE id = {ann}")).contains("foreign key"));
    assert!(err("DELETE FROM banks WHERE id = 1").contains("foreign key"));
    assert!(err(
        "INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at) VALUES ('x', 999, 't', 1, 2)"
    )
    .contains("foreign key"));
    assert!(err("INSERT INTO attempt_items(attempt_id, seq, question_id, correct) VALUES (999, 0, 'id1', 1)")
        .contains("foreign key"));
    assert!(
        err("INSERT INTO user_banks(user_id, bank_id) VALUES (1, 999)").contains("foreign key")
    );
    // but a record may name a question that is gone (no foreign key by design)
    exec(&c, "DELETE FROM questions WHERE id = 'id2'");
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM attempt_items WHERE question_id = 'id2'"
        ),
        1
    );
    drop(c);

    // the store's own paths still work: session purge, record delete, prune, delete all records
    assert_eq!(s.purge_expired_sessions(1_800_000_000).expect("purge"), 1);
    session(&s, ann, "tok-ann2");
    let rec_id = count(&raw_on(&db), "SELECT MAX(id) FROM attempts");
    assert!(s.delete_record(ann, rec_id).expect("delete record"));
    // prune: more than RECORD_RETENTION records, then one more through the store
    {
        let c = raw_on(&db);
        let tx = c.unchecked_transaction().expect("tx");
        for i in 0..505 {
            tx.execute(
                "INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, user_id) VALUES (?1, '[]', 'all', 1, 1, 100, 1, ?2)",
                params![format!("2026-05-01 10:{:02}", i % 60), ann],
            )
            .expect("bulk attempt");
        }
        tx.execute(
            "INSERT INTO attempt_items(attempt_id, seq, question_id, correct) SELECT id, 0, 'id1', 1 FROM attempts WHERE user_id = ?1",
            params![ann],
        )
        .expect("items");
        tx.commit().expect("commit");
    }
    s.record_attempt(ann, &attempt(&["id1"]))
        .expect("record + prune");
    let c = raw_on(&db);
    assert_eq!(
        count(
            &c,
            &format!("SELECT COUNT(*) FROM attempts WHERE user_id = {ann}")
        ),
        xexam_backend::database::RECORD_RETENTION
    );
    assert_clean(&c);
    drop(c);
    s.delete_all_records(ann, true).expect("delete all records");

    // an administrator who changed a system setting can be deleted (no link on `updated_by`)
    let boss = user_id(&s, "boss", Role::Admin);
    s.set_system_setting("test.switch", "1", Some(boss))
        .expect("setting");
    session(&s, boss, "tok-boss");
    match s.delete_user(1, boss).expect("delete the admin") {
        UserChange::Done(name) => assert_eq!(name, "boss"),
        other => panic!("{other:?}"),
    }
    // a user with sessions, records and a bank assignment
    match s.delete_user(1, ann).expect("delete ann") {
        UserChange::Done(name) => assert_eq!(name, "ann"),
        other => panic!("{other:?}"),
    }
    // the bank: with and without records
    assert!(s.delete_bank(1, false).expect("delete bank").is_some());
    s.apply_import("b.xlsx", &[dbc::sheet("Cooking", 2)], &qs)
        .expect("import again");
    assert!(s
        .delete_bank(2, true)
        .expect("delete bank with records")
        .is_some());
    drop(s);
    let c = raw_on(&db);
    assert_clean(&c);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM banks"), 0);
}

// ---------------------------------------------------------------- 7. an older binary's re-created `stats`

#[test]
fn an_older_binary_rollback_is_tolerated() {
    let dir = tmp();
    let db = dir.path().join("xexam.db");
    drop(open_fixed(&db));
    let fresh = master(&db);
    // v1.9 runs the v1 text on every open: an empty `stats` appears
    exec(&raw_on(&db), xexam_backend::database::schema::SCHEMA);
    assert!(has_table(&raw_on(&db), "stats"));
    let s = open_fixed(&db);
    assert!(s.migration_report().kept_legacy.is_empty());
    assert!(s.migration_backup().is_none());
    drop(s);
    assert!(
        !has_table(&raw_on(&db), "stats"),
        "the empty table is dropped"
    );
    assert_eq!(master(&db), fresh, "the result equals a fresh file");
    // and the open after writes nothing
    let bytes = db_bytes(&db);
    drop(open_fixed(&db));
    assert_eq!(db_bytes(&db), bytes);

    // with one row in it the table is kept and reported
    exec(&raw_on(&db), xexam_backend::database::schema::SCHEMA);
    exec(
        &raw_on(&db),
        "INSERT INTO stats(question_id, attempts, correct, wrong, streak, last, at) VALUES ('q1', 1, 1, 0, 1, 1, 't')",
    );
    let s = open_fixed(&db);
    assert_eq!(s.migration_report().kept_legacy, vec![("stats", 1)]);
    drop(s);
    assert_eq!(
        count(&raw_on(&db), "SELECT COUNT(*) FROM stats"),
        1,
        "the row is still there"
    );
    // once the row is gone the table goes with the next open
    exec(&raw_on(&db), "DELETE FROM stats");
    drop(open_fixed(&db));
    assert!(!has_table(&raw_on(&db), "stats"));
    assert_eq!(master(&db), fresh);
    // Documented, not a code test: a v1.9 binary reads "11" through `Some(_) => 10` (anything newer than 9 is "nothing to
    // migrate"), so it opens a schema-11 file as it is; its delete paths are child-first and satisfy the foreign keys.
}

// ---------------------------------------------------------------- 8. a fresh file

#[test]
fn fresh_is_schema_11() {
    let dir = tmp();
    let db = dir.path().join("xexam.db");
    let s = open_fixed(&db);
    assert!(s.migration_backup().is_none());
    assert!(s.migration_report().orphans.is_empty());
    drop(s);
    assert!(backups(dir.path()).is_empty(), "no backup for a new file");
    let c = raw_on(&db);
    assert_eq!(text(&c, "SELECT value FROM meta WHERE key='schema'"), "11");
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM activity_log WHERE action = 'system.migrate'"
        ),
        0
    );
    assert_foreign_keys(&c);
    assert_clean(&c);
    for t in ["stats", "settings"] {
        assert!(!has_table(&c, t), "{t}");
    }
    for idx in [
        "idx_users_uid",
        "idx_user_stats_question",
        "idx_imports_bank",
    ] {
        assert_eq!(
            count(
                &c,
                &format!("SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='{idx}'")
            ),
            1,
            "{idx}"
        );
    }
    // the rebuilt tables are stored from the constants
    for r in &V11_REBUILDS {
        let sql = text(
            &c,
            &format!(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='{}'",
                r.table
            ),
        );
        assert!(
            sql.starts_with(&format!("CREATE TABLE \"{}\"", r.table)),
            "{sql}"
        );
    }
    // the book-keeping DELETE by question uses the new index
    let plan: Vec<String> = c
        .prepare("EXPLAIN QUERY PLAN DELETE FROM user_stats WHERE question_id IN ('a', 'b')")
        .expect("prepare")
        .query_map([], |r| r.get::<_, String>(3))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert!(
        plan.iter().any(|d| d.contains("idx_user_stats_question")),
        "{plan:?}"
    );
    let plan: Vec<String> = c
        .prepare("EXPLAIN QUERY PLAN DELETE FROM imports WHERE bank_id = 1")
        .expect("prepare")
        .query_map([], |r| r.get::<_, String>(3))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert!(
        plan.iter().any(|d| d.contains("idx_imports_bank")),
        "{plan:?}"
    );
    // meta keys of a fresh file
    let meta: Vec<(String, String)> = c
        .prepare("SELECT key, value FROM meta ORDER BY key")
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(
        meta,
        vec![
            ("banks_known".to_string(), "0".to_string()),
            ("schema".to_string(), "11".to_string()),
            ("users_known".to_string(), "1".to_string())
        ]
    );
}
