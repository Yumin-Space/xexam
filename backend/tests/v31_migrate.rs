//! v3.1, main database schema 13: the additive upgrade from schema 12 (categories, devices, push tables, dropped index), its
//! backup and activity row, idempotence, tolerance of an older binary's re-created index, and the foreign-key graph of the new
//! tables. Self-made data only.
#[path = "common/mod.rs"]
mod dbc;

use dbc::{db_bytes, fixed, historic, open_fixed, tmp, FIXED_NOW, SCHEMA_12_OBJECTS_V30};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::Path;
use xexam_backend::database::timefmt::Clock;

const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const SKEY: &str = "0123456789abcdef0123456789abcdef";
const TS: &str = "2026-01-02 10:00";

fn raw_on(p: &Path) -> Connection {
    let c = Connection::open(p).expect("raw open");
    c.execute_batch("PRAGMA foreign_keys=ON").expect("fk on");
    c
}

fn text(c: &Connection, sql: &str) -> String {
    c.query_row(sql, [], |r| r.get(0)).expect("text")
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).expect("count")
}

fn sqlite_master(path: &Path) -> Vec<Value> {
    let c = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open");
    let mut st = c
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY rowid")
        .expect("prepare");
    st.query_map([], |r| {
        Ok(json!([
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?
        ]))
    })
    .expect("query")
    .map(|r| r.expect("row"))
    .collect()
}

fn names(path: &Path) -> Vec<(String, String)> {
    sqlite_master(path)
        .iter()
        .map(|r| {
            (
                r[0].as_str().expect("type").to_string(),
                r[1].as_str().expect("name").to_string(),
            )
        })
        .collect()
}

/// A schema-12 file with a made-up administrator, bank and question (so the upgrade has data to protect).
fn v12_with_data(db: &Path) {
    historic(db, 12);
    let c = Connection::open(db).expect("open");
    c.execute_batch("PRAGMA foreign_keys=OFF").expect("fk off");
    c.execute_batch(&format!(
        "INSERT INTO users(id, username, password_hash, role, created_at, password_changed_at) \
           VALUES (1, 'admin', 'x', 'admin', '{TS}', '{TS}'); \
         INSERT INTO banks(id, name, filename, legacy_ids, created_at, updated_at, owner_id) \
           VALUES (5, 'General knowledge', 'general.xlsx', 0, '{TS}', '{TS}', 1); \
         INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id) \
           VALUES ('q1', 'Quiz', 1, 0, '{{\"kind\": \"single\", \"stem\": \"Made up?\"}}', 1, '{TS}', '{TS}', 5); \
         INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at) \
           VALUES ('{HASH}', 1, '{TS}', 1700000000, 1700000100);"
    ))
    .expect("seed");
}

fn backups(dir: &Path) -> Vec<String> {
    match std::fs::read_dir(dir.join("backups")) {
        Ok(rd) => rd
            .map(|e| e.expect("entry").file_name().to_string_lossy().to_string())
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[test]
fn historic_12_matches_the_v30_object_list() {
    let dir = tmp();
    let db = dir.path().join("h12.db");
    historic(&db, 12);
    let got = names(&db);
    let want: Vec<(String, String)> = SCHEMA_12_OBJECTS_V30
        .iter()
        .map(|(t, n)| ((*t).to_string(), (*n).to_string()))
        .collect();
    assert_eq!(got, want);
}

#[test]
fn fresh_v13_equals_upgraded_v12() {
    let dir = tmp();
    let fresh = dir.path().join("fresh.db");
    drop(open_fixed(&fresh));
    let up = dir.path().join("up.db");
    historic(&up, 12);
    drop(open_fixed(&up));
    assert_eq!(
        sqlite_master(&fresh),
        sqlite_master(&up),
        "a fresh file and an upgraded v12 file have the same schema, SQL text included"
    );
    let c = Connection::open(&fresh).expect("open");
    assert_eq!(text(&c, "SELECT value FROM meta WHERE key='schema'"), "13");
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM sqlite_master WHERE name='idx_questions_sheet'"
        ),
        0
    );
}

#[test]
fn a_v12_file_with_data_gets_one_backup_and_one_migrate_row() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    v12_with_data(&db);
    let s = open_fixed(&db);
    drop(s);
    let b = backups(dir.path());
    assert_eq!(b.len(), 1, "{b:?}");
    assert!(
        b[0].starts_with("xexam-schema12-") && b[0].ends_with(".db"),
        "{b:?}"
    );
    let c = Connection::open(&db).expect("open");
    assert_eq!(text(&c, "SELECT value FROM meta WHERE key='schema'"), "13");
    let rows: Vec<(i64, String)> = c
        .prepare("SELECT ts, detail FROM activity_log WHERE action='system.migrate'")
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].1, r#"{"from":12,"to":13}"#);
    assert_eq!(rows[0].0, fixed(FIXED_NOW).unix(), "the injected clock");
    // The data survived, and the session has no device yet
    assert_eq!(count(&c, "SELECT COUNT(*) FROM questions"), 1);
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM sessions WHERE device_id IS NULL"),
        1
    );
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM banks WHERE category_id IS NULL AND sort = 0"
        ),
        1
    );
    // The backup is a schema-12 file
    let bc = Connection::open(dir.path().join("backups").join(&b[0])).expect("backup");
    assert_eq!(text(&bc, "SELECT value FROM meta WHERE key='schema'"), "12");
}

#[test]
fn an_empty_v12_file_gets_no_backup() {
    let dir = tmp();
    let db = dir.path().join("e.db");
    historic(&db, 12);
    drop(open_fixed(&db));
    assert!(backups(dir.path()).is_empty());
}

#[test]
fn reopening_a_v13_file_is_byte_identical() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    v12_with_data(&db);
    drop(open_fixed(&db));
    let before = db_bytes(&db);
    drop(open_fixed(&db));
    assert_eq!(db_bytes(&db), before);
}

#[test]
fn a_newer_schema_opens_unchanged() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    drop(open_fixed(&db));
    Connection::open(&db)
        .expect("open")
        .execute("UPDATE meta SET value='14' WHERE key='schema'", [])
        .expect("stamp");
    let before = db_bytes(&db);
    drop(open_fixed(&db));
    assert_eq!(db_bytes(&db), before);
    let c = Connection::open(&db).expect("open");
    assert_eq!(text(&c, "SELECT value FROM meta WHERE key='schema'"), "14");
}

#[test]
fn an_index_an_older_binary_re_created_is_dropped_again() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    drop(open_fixed(&db));
    Connection::open(&db)
        .expect("open")
        .execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_questions_sheet ON questions(sheet, active);",
        )
        .expect("re-create");
    drop(open_fixed(&db));
    let c = Connection::open(&db).expect("open");
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM sqlite_master WHERE name='idx_questions_sheet'"
        ),
        0
    );
}

#[test]
fn the_sheet_filter_uses_the_layout_index() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    drop(open_fixed(&db));
    let c = Connection::open(&db).expect("open");
    let plan: Vec<String> = c
        .prepare("EXPLAIN QUERY PLAN SELECT id FROM questions WHERE bank_id=?1 AND sheet=?2")
        .expect("prepare")
        .query_map(rusqlite::params![1, "Quiz"], |r| r.get::<_, String>(3))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert!(
        plan.iter().any(|d| d.contains("idx_questions_layout")),
        "{plan:?}"
    );
}

#[test]
fn the_new_tables_delete_child_first() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    drop(open_fixed(&db));
    let c = raw_on(&db);
    let p256dh = "A".repeat(87);
    let auth = "B".repeat(22);
    c.execute_batch(&format!(
        "INSERT INTO users(username, password_hash, role, created_at, password_changed_at) \
           VALUES ('learner', 'x', 'user', '{TS}', '{TS}'); \
         INSERT INTO bank_categories(id, name, sort, created_at, updated_at) VALUES (1, 'Science', 1, 1700000000, 1700000000); \
         INSERT INTO banks(id, name, filename, legacy_ids, created_at, updated_at, owner_id, category_id, sort) \
           VALUES (7, 'General knowledge', 'general.xlsx', 0, '{TS}', '{TS}', 1, 1, 1); \
         INSERT INTO devices(id, user_id, cookie_hash, created_at, active_at) \
           VALUES (1, 2, '{HASH}', 1700000000, 1700000000); \
         INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at, device_id) \
           VALUES ('{HASH}', 2, '{TS}', 1700000000, 1700000100, 1); \
         INSERT INTO push_subscriptions(id, device_id, endpoint, p256dh, auth, key_id, created_at, confirmed_at) \
           VALUES (1, 1, 'https://push.example.com/abc', '{p256dh}', '{auth}', '0123456789abcdef', 1700000000, 1700000000); \
         INSERT INTO saved_sessions(user_id, session_key, device_id, kind, answered, total, started_at, saved_at) \
           VALUES (2, '{SKEY}', 1, 'practice', 3, 10, 1700000000, 1700000000); \
         INSERT INTO push_outbox(subscription_id, kind, topic, payload, created_at, expires_at, next_at) \
           VALUES (1, 'test', 'test', '{{\"v\":1}}', 1700000000, 1700003600, 1700000000);"
    ))
    .expect("seed");
    let fails = |sql: &str| {
        let e = c.execute_batch(sql).expect_err(sql);
        assert!(e.to_string().contains("FOREIGN KEY"), "{sql}: {e}");
    };
    // Parents first fail
    fails("DELETE FROM bank_categories WHERE id=1");
    fails("DELETE FROM devices WHERE id=1");
    fails("DELETE FROM push_subscriptions WHERE id=1");
    fails("DELETE FROM users WHERE id=2");
    // Child first works
    c.execute_batch(
        "DELETE FROM push_outbox WHERE subscription_id=1; \
         DELETE FROM push_subscriptions WHERE device_id=1; \
         DELETE FROM saved_sessions WHERE user_id=2; \
         DELETE FROM sessions WHERE device_id=1; \
         DELETE FROM devices WHERE user_id=2; \
         UPDATE banks SET category_id=NULL, sort=1 WHERE category_id=1; \
         DELETE FROM bank_categories WHERE id=1; \
         DELETE FROM users WHERE id=2;",
    )
    .expect("child first");
    assert_eq!(
        c.prepare("PRAGMA foreign_key_check")
            .expect("prepare")
            .query_map([], |r| r.get::<_, String>(0))
            .expect("query")
            .count(),
        0
    );
}

#[test]
fn the_new_checks_reject_bad_values() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    drop(open_fixed(&db));
    let c = raw_on(&db);
    assert!(c
        .execute(
            "INSERT INTO bank_categories(name, created_at, updated_at) VALUES ('', 1, 1)",
            []
        )
        .is_err());
    c.execute(
        "INSERT INTO bank_categories(name, created_at, updated_at) VALUES ('Science', 1, 1)",
        [],
    )
    .expect("ok");
    assert!(
        c.execute(
            "INSERT INTO bank_categories(name, created_at, updated_at) VALUES ('science', 1, 1)",
            []
        )
        .is_err(),
        "names are unique case-insensitively"
    );
}
