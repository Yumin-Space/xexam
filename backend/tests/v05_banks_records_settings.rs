//! v0.5: schema v2 (migration, backup, fix-up), question banks, exam records with per-question detail, settings —
//! store-level tests plus API tests through `build_router` with the real engine (real parser + real database).
#[path = "api_common/mod.rs"]
mod apic;
#[path = "common/mod.rs"]
mod dbc;

use apic::{build, get, post_json, post_raw, send, FakeEngine, Resp, TestApp};
use axum::body::Body;
use axum::http::StatusCode;
use dbc::{
    baseline, hotspot, multi, open_fixed, seed_questions, sheet, single, tmp, ADMIN, FIXED_NOW,
};
use rusqlite::types::Value as Sql;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use xexam_backend::api::AppState;
use xexam_backend::database::{scoped_id, FnClock, Store, RECORD_RETENTION};
use xexam_backend::error::DbError;
use xexam_backend::models::{
    Attempt, AttemptResult, DeletedBank, ImportTarget, ParsedSheet, Question,
};
use xexam_backend::services::RealEngine;

const MSG_BANK_NOT_FOUND: &str = "这个题库不存在（可能已被删除），请刷新页面后再试。";
const MSG_RECORD_NOT_FOUND: &str = "这条考试记录不存在（可能已被删除），请刷新页面后再试。";
const MSG_BAD_TARGET: &str = "上传目标不对，请刷新页面后再试。";
const MSG_BAD_REQUEST: &str = "请求的格式不对。";
const MSG_BAD_SETTINGS: &str = "设置的值不对。";
const GONE: &str = "这次上传的预览已经过期或已被处理，请重新上传。";

// ---------------------------------------------------------------- helpers

fn raw(p: &Path) -> Connection {
    Connection::open(p).expect("raw open")
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

fn keys(v: &Value) -> Vec<String> {
    v.as_object().expect("object").keys().cloned().collect()
}

/// Four questions on two sheets, with parser ids `<prefix>1` … `<prefix>4`.
fn workbook(prefix: &str) -> (Vec<ParsedSheet>, Vec<Question>) {
    (
        vec![sheet("S1", 3), sheet("S2", 1)],
        vec![
            single("S1", &format!("{prefix}1"), "一つ目", 1),
            single("S1", &format!("{prefix}2"), "二つ目", 2),
            multi("S1", &format!("{prefix}3"), "三つ目", 3),
            hotspot("S2", &format!("{prefix}4"), "四つ目", 1),
        ],
    )
}

fn attempt(results: &[(&str, bool)]) -> Attempt {
    Attempt {
        results: results
            .iter()
            .map(|(id, correct)| AttemptResult {
                id: (*id).to_string(),
                correct: *correct,
            })
            .collect(),
        sheets: vec!["S1".to_string()],
        mode: "all".to_string(),
        seconds: 42,
        kind: Default::default(),
    }
}

/// Ids of a bank's questions in `seq` order.
fn bank_ids(db: &Path, bank: i64) -> Vec<String> {
    let c = raw(db);
    let mut st = c
        .prepare("SELECT id FROM questions WHERE bank_id=?1 ORDER BY seq")
        .expect("prepare");
    st.query_map([bank], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("ids")
}

fn active_in(db: &Path, bank: i64) -> i64 {
    raw(db)
        .query_row(
            "SELECT COUNT(*) FROM questions WHERE bank_id=?1 AND active=1",
            [bank],
            |r| r.get(0),
        )
        .expect("count")
}

/// A clock that returns a new minute on every call ("2026-05-01 10:00", "2026-05-01 10:01", …).
fn ticking() -> Arc<FnClock<impl Fn() -> String + Send + Sync + 'static>> {
    let n = Arc::new(AtomicUsize::new(0));
    Arc::new(FnClock(move || {
        let i = n.fetch_add(1, Ordering::SeqCst);
        format!("2026-05-01 {:02}:{:02}", 10 + i / 60, i % 60)
    }))
}

fn backups(dir: &Path) -> Vec<PathBuf> {
    match std::fs::read_dir(dir.join("backups")) {
        Ok(rd) => rd.map(|e| e.expect("entry").path()).collect(),
        Err(_) => Vec::new(),
    }
}

// ---------------------------------------------------------------- schema v2: fresh, migration, backup, fix-up

#[test]
fn fresh_database_is_schema_2_without_a_legacy_bank() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    let s = open_fixed(&db);
    assert!(s.migration_backup().is_none());
    assert_eq!(s.banks().expect("banks"), json!({"banks": []}));
    assert_eq!(s.bank().expect("bank")["questions"], json!([]));
    drop(s);
    let c = raw(&db);
    // intended difference (v0.6, v0.6.5, v0.8): a new database goes straight to the current schema (tests/v06_users_and_sign_in.rs,
    // tests/v065_bank_access_and_practice.rs, tests/v08_ai_link.rs)
    assert_eq!(text(&c, "SELECT value FROM meta WHERE key='schema'"), "6");
    assert_eq!(count(&c, "SELECT COUNT(*) FROM banks"), 0);
    for t in ["banks", "attempt_items", "settings"] {
        assert_eq!(
            count(
                &c,
                &format!("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='{t}'")
            ),
            1,
            "{t}"
        );
    }
    assert!(
        backups(dir.path()).is_empty(),
        "an empty database is not backed up"
    );
    // reopening an empty v2 database writes nothing
    let before = dbc::db_bytes(&db);
    drop(open_fixed(&db));
    assert_eq!(dbc::db_bytes(&db), before);
}

const Q_COLS: &str =
    "SELECT id, sheet, row_no, seq, data, active, added_at, updated_at FROM questions ORDER BY id";
const ATTEMPT_COLS: &str =
    "SELECT id, at, sheets, mode, total, correct, percent, seconds FROM attempts ORDER BY id";
const IMPORT_COLS: &str = "SELECT id, at, filename, added, updated, removed, restored, skipped, total FROM imports ORDER BY id";

#[test]
fn migrating_the_python_sample_keeps_every_row_and_writes_a_backup() {
    let dir = tmp();
    let db = dir.path().join("sample.db");
    std::fs::copy(baseline("fixtures/sample.db"), &db).expect("copy the fixture");
    let (q0, s0, a0, i0, all0) = {
        let c = raw(&db);
        (
            rows(&c, Q_COLS),
            rows(&c, "SELECT * FROM stats ORDER BY question_id"),
            rows(&c, ATTEMPT_COLS),
            rows(&c, IMPORT_COLS),
            [
                rows(&c, "SELECT * FROM questions ORDER BY id"),
                rows(&c, "SELECT * FROM stats ORDER BY question_id"),
                rows(&c, "SELECT * FROM attempts ORDER BY id"),
                rows(&c, "SELECT * FROM imports ORDER BY id"),
                rows(&c, "SELECT * FROM meta ORDER BY key"),
            ],
        )
    };
    let (first_added, last_import_at, last_file): (String, String, String) = {
        let c = raw(&db);
        (
            text(&c, "SELECT MIN(added_at) FROM questions"),
            text(&c, "SELECT at FROM imports ORDER BY id DESC LIMIT 1"),
            text(&c, "SELECT filename FROM imports ORDER BY id DESC LIMIT 1"),
        )
    };
    assert_eq!(last_file, "demo_v2.xlsx");

    let s = open_fixed(&db);
    // the backup: under <dir of the database>/backups, opens, still schema 1, same rows as the original
    let found = backups(dir.path());
    assert_eq!(found.len(), 1, "{found:?}");
    let backup = s
        .migration_backup()
        .expect("a backup was written")
        .to_path_buf();
    assert_eq!(
        std::fs::canonicalize(&backup).expect("canon"),
        std::fs::canonicalize(&found[0]).expect("canon")
    );
    let name = backup.file_name().and_then(|n| n.to_str()).expect("name");
    assert!(
        name.starts_with("xexam-schema1-")
            && name.ends_with(".db")
            && name.len() == "xexam-schema1-20260101-000000.db".len(),
        "{name}"
    );
    {
        let b = raw(&backup);
        assert_eq!(text(&b, "SELECT value FROM meta WHERE key='schema'"), "1");
        assert_eq!(
            count(&b, "SELECT COUNT(*) FROM sqlite_master WHERE name='banks'"),
            0
        );
        let ok: String = b
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .expect("check");
        assert_eq!(ok, "ok");
        assert_eq!(
            [
                rows(&b, "SELECT * FROM questions ORDER BY id"),
                rows(&b, "SELECT * FROM stats ORDER BY question_id"),
                rows(&b, "SELECT * FROM attempts ORDER BY id"),
                rows(&b, "SELECT * FROM imports ORDER BY id"),
                rows(&b, "SELECT * FROM meta ORDER BY key"),
            ],
            all0
        );
    }
    // the migrated database: every question id / stat / attempt / import unchanged, all attached to one legacy bank
    let c = raw(&db);
    assert_eq!(rows(&c, Q_COLS), q0);
    assert_eq!(rows(&c, "SELECT * FROM stats ORDER BY question_id"), s0);
    assert_eq!(rows(&c, ATTEMPT_COLS), a0);
    assert_eq!(rows(&c, IMPORT_COLS), i0);
    assert_eq!(
        rows(
            &c,
            "SELECT id, name, filename, legacy_ids, created_at, updated_at FROM banks"
        ),
        vec![vec![
            Sql::Integer(1),
            Sql::Text("demo_v2".into()),
            Sql::Text("demo_v2.xlsx".into()),
            Sql::Integer(1),
            Sql::Text(first_added),
            Sql::Text(last_import_at.clone()),
        ]]
    );
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM questions WHERE bank_id IS NOT 1"),
        0
    );
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM imports WHERE bank_id IS NOT 1"),
        0
    );
    assert_eq!(
        count(
            &c,
            r#"SELECT COUNT(*) FROM attempts WHERE banks <> '[{"id": 1, "name": "demo_v2"}]'"#
        ),
        0
    );
    // intended difference (v0.6, v0.6.5, v0.8): v1 → v2 → v3 → v4 → v5 in one go (one schema-1 backup); v3 / v4 / v5 are
    // covered in tests/v06_users_and_sign_in.rs, tests/v065_bank_access_and_practice.rs and tests/v08_ai_link.rs
    assert_eq!(text(&c, "SELECT value FROM meta WHERE key='schema'"), "6");
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM attempt_items"),
        0,
        "old records have no detail"
    );
    // what the API shows
    let banks = s.banks().expect("banks");
    let b = &banks["banks"][0];
    assert_eq!(banks["banks"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        keys(b),
        [
            "id",
            "name",
            "filename",
            "legacy",
            "createdAt",
            "updatedAt",
            "total",
            "inactive",
            "sheets",
            "studied",
            "records",
            // v0.6.5: the owner (the default administrator created by the migration) and whether the viewer may manage it
            "ownerId",
            "owner",
            "manageable"
        ]
    );
    assert_eq!(
        (
            b["ownerId"].clone(),
            b["owner"].clone(),
            b["manageable"].clone()
        ),
        (json!(1), json!("admin"), json!(true))
    );
    assert_eq!(
        (
            b["legacy"].clone(),
            b["total"].clone(),
            b["inactive"].clone()
        ),
        (json!(true), json!(6), json!(1))
    );
    assert_eq!(b["updatedAt"], last_import_at);
    assert_eq!(
        b["sheets"],
        json!([{"name": "demo", "usable": 6, "inactive": 1}])
    );
    assert_eq!(
        b["studied"],
        count(
            &c,
            "SELECT COUNT(*) FROM user_stats s JOIN questions q ON q.id = s.question_id"
        )
    );
    assert_eq!(b["records"], count(&c, "SELECT COUNT(*) FROM attempts"));
    let recs = s.records(ADMIN).expect("records");
    assert_eq!(
        recs["records"][0]["banks"],
        json!([{"id": 1, "name": "demo_v2", "deleted": false}])
    );
    assert_eq!(recs["records"][0]["answered"], Value::Null);
    assert_eq!(recs["records"][0]["detail"], false);
    drop(c);
    drop(s);
    // re-opening the migrated database: not a single byte changes, no second backup
    let before = dbc::db_bytes(&db);
    let s = open_fixed(&db);
    assert!(s.migration_backup().is_none());
    s.bank().expect("bank");
    s.banks().expect("banks");
    drop(s);
    assert_eq!(dbc::db_bytes(&db), before);
    assert_eq!(backups(dir.path()).len(), 1);
}

#[test]
fn a_failed_backup_refuses_to_migrate() {
    let dir = tmp();
    let db = dir.path().join("sample.db");
    std::fs::copy(baseline("fixtures/sample.db"), &db).expect("copy");
    // `backups` exists as a file, so the backup directory cannot be created
    std::fs::write(dir.path().join("backups"), b"not a directory").expect("write");
    let before = dbc::db_bytes(&db);
    match Store::open(&db) {
        Err(DbError::Backup(_)) => {}
        Err(other) => panic!("expected a backup error, got {other}"),
        Ok(_) => panic!("must not open (and migrate) without a backup"),
    }
    assert_eq!(dbc::db_bytes(&db), before, "nothing migrated");
    assert_eq!(
        text(&raw(&db), "SELECT value FROM meta WHERE key='schema'"),
        "1"
    );
}

#[test]
fn reopening_a_v2_database_with_data_never_writes() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    let s = open_fixed(&db);
    let (sh, qs) = workbook("a");
    s.apply_import("legacy.xlsx", &sh, &qs).expect("legacy");
    s.apply_import_for(ImportTarget::New, "other.xlsx", &sh, &qs)
        .expect("new");
    s.record_attempt_with_answers(ADMIN, &attempt(&[("a1", true)]), &[Some(json!(["A"]))])
        .expect("attempt");
    s.update_settings(ADMIN, &[("theme".into(), json!("dark"))])
        .expect("settings");
    drop(s);
    let before = dbc::db_bytes(&db);
    let s = open_fixed(&db);
    s.bank().expect("bank");
    s.banks().expect("banks");
    s.records(ADMIN).expect("records");
    s.record(ADMIN, 1).expect("record");
    s.settings(ADMIN).expect("settings");
    s.progress(ADMIN).expect("progress");
    s.plan_import_for(ImportTarget::Bank(2), &sh, &qs)
        .expect("plan");
    drop(s);
    assert_eq!(dbc::db_bytes(&db), before);
    assert!(!dir.path().join("x.db-journal").exists());
    assert!(backups(dir.path()).is_empty());
}

#[test]
fn rows_written_by_an_older_binary_are_attached_to_the_legacy_bank() {
    // (a) a legacy bank exists: rows without a bank join it
    let dir = tmp();
    let db = dir.path().join("x.db");
    let s = open_fixed(&db);
    let (sh, qs) = workbook("a");
    s.apply_import("legacy.xlsx", &sh, &qs).expect("legacy");
    s.apply_import_for(ImportTarget::New, "other.xlsx", &sh, &qs)
        .expect("new");
    drop(s);
    {
        let c = raw(&db);
        c.execute_batch(
            "INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at) \
             VALUES('old1', 'S9', 1, 9, '{}', 1, '2026-06-01 00:00', '2026-06-01 00:00'); \
             INSERT INTO imports(at, filename, added, updated, removed, restored, skipped, total) \
             VALUES('2026-06-01 00:00', 'old.xlsx', 1, 0, 0, 0, 0, 1);",
        )
        .expect("simulate v0.4 writes");
    }
    drop(open_fixed(&db));
    let c = raw(&db);
    assert_eq!(
        text(
            &c,
            "SELECT CAST(bank_id AS TEXT) FROM questions WHERE id='old1'"
        ),
        "1"
    );
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM imports WHERE bank_id IS NULL"),
        0
    );
    assert_eq!(count(&c, "SELECT COUNT(*) FROM banks"), 2, "no new bank");
    // an older binary re-importing an unchanged workbook writes only an import row: it joins the legacy bank too
    c.execute(
        "INSERT INTO imports(at, filename, added, updated, removed, restored, skipped, total)          VALUES('2026-06-02 00:00', 'same.xlsx', 0, 0, 0, 0, 0, 5)",
        [],
    )
    .expect("simulate a v0.4 import row");
    drop(c);
    drop(open_fixed(&db));
    let c = raw(&db);
    assert_eq!(
        rows(&c, "SELECT bank_id FROM imports ORDER BY id"),
        vec![
            vec![Sql::Integer(1)],
            vec![Sql::Integer(2)],
            vec![Sql::Integer(1)],
            vec![Sql::Integer(1)]
        ]
    );
    drop(c);

    // (b) no legacy bank: one is created for them, named after the unattached import
    let dir = tmp();
    let db = dir.path().join("y.db");
    let s = open_fixed(&db);
    s.apply_import_for(ImportTarget::New, "other.xlsx", &sh, &qs)
        .expect("new");
    drop(s);
    {
        let c = raw(&db);
        c.execute_batch(
            "INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at) \
             VALUES('old1', 'S9', 1, 0, '{}', 1, '2026-06-01 00:00', '2026-06-02 00:00'); \
             INSERT INTO imports(at, filename, added, updated, removed, restored, skipped, total) \
             VALUES('2026-06-03 00:00', 'Old Bank.XLSX', 1, 0, 0, 0, 0, 1);",
        )
        .expect("simulate v0.4 writes");
    }
    drop(open_fixed(&db));
    let c = raw(&db);
    assert_eq!(
        rows(
            &c,
            "SELECT id, name, filename, legacy_ids, created_at, updated_at FROM banks ORDER BY id"
        ),
        vec![
            vec![
                Sql::Integer(1),
                Sql::Text("other".into()),
                Sql::Text("other.xlsx".into()),
                Sql::Integer(0),
                Sql::Text(FIXED_NOW.into()),
                Sql::Text(FIXED_NOW.into()),
            ],
            vec![
                Sql::Integer(2),
                Sql::Text("Old Bank".into()),
                Sql::Text("Old Bank.XLSX".into()),
                Sql::Integer(1),
                Sql::Text("2026-06-01 00:00".into()),
                Sql::Text("2026-06-03 00:00".into()),
            ],
        ]
    );
    assert_eq!(
        text(
            &c,
            "SELECT CAST(bank_id AS TEXT) FROM questions WHERE id='old1'"
        ),
        "2"
    );
    assert_eq!(active_in(&db, 1), 4, "the other bank is untouched");
    drop(c);
    // and that is a one-time write: the next open changes nothing
    let before = dbc::db_bytes(&db);
    drop(open_fixed(&db));
    assert_eq!(dbc::db_bytes(&db), before);
}

// ---------------------------------------------------------------- banks and import targets

#[test]
fn the_same_workbook_in_two_banks_has_distinct_ids_and_no_cross_retire() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    let s = open_fixed(&db);
    let (sh, qs) = workbook("p");
    let plan = s
        .plan_import_for(ImportTarget::New, &sh, &qs)
        .expect("plan")
        .expect("new is always possible");
    assert_eq!(
        (plan["added"].clone(), plan["usable"].clone()),
        (json!(4), json!(4))
    );
    let (r1, b1) = s
        .apply_import_for(ImportTarget::New, "same.xlsx", &sh, &qs)
        .expect("one")
        .expect("created");
    assert_eq!(r1, plan, "the write reports what the preview showed");
    let (r2, b2) = s
        .apply_import_for(ImportTarget::New, "same.xlsx", &sh, &qs)
        .expect("two")
        .expect("created");
    assert_eq!(
        r2["added"], 4,
        "a new bank never sees another bank's questions"
    );
    assert_eq!((b1, b2), (1, 2));
    let ids1 = bank_ids(&db, b1);
    let ids2 = bank_ids(&db, b2);
    let want1: Vec<String> = qs.iter().map(|q| scoped_id(b1, &q.id)).collect();
    let want2: Vec<String> = qs.iter().map(|q| scoped_id(b2, &q.id)).collect();
    assert_eq!(ids1, want1);
    assert_eq!(ids2, want2);
    assert!(ids1.iter().all(|i| !ids2.contains(i)));
    assert!(
        ids1.iter().all(|i| qs.iter().all(|q| &q.id != i)),
        "scoped ids differ from parser ids"
    );
    // names: same file → "(2)", "(3)"
    s.apply_import_for(ImportTarget::New, "SAME.xlsx", &sh, &qs)
        .expect("three");
    s.apply_import_for(ImportTarget::New, "same.xlsx", &sh, &qs)
        .expect("four");
    let names: Vec<Value> = s.banks().expect("banks")["banks"]
        .as_array()
        .expect("list")
        .iter()
        .map(|b| b["name"].clone())
        .collect();
    assert_eq!(
        names,
        [
            json!("same"),
            json!("same (2)"),
            json!("SAME"),
            json!("same (3)")
        ]
    );
    // re-import a subset into bank 1: only bank 1 retires anything
    let (r, _) = s
        .apply_import_for(ImportTarget::Bank(b1), "same.xlsx", &sh, &qs[..2])
        .expect("update")
        .expect("exists");
    assert_eq!(
        (r["removed"].clone(), r["unchanged"].clone()),
        (json!(2), json!(2))
    );
    assert_eq!(active_in(&db, b1), 2);
    assert_eq!(active_in(&db, b2), 4);
    // /api/bank lists every bank's questions, grouped by bank, each with its bank id at the end
    let bank = s.bank().expect("bank");
    let qbanks: Vec<i64> = bank["questions"]
        .as_array()
        .expect("questions")
        .iter()
        .map(|q| q["bank"].as_i64().expect("bank id"))
        .collect();
    assert_eq!(qbanks, [1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4]);
    assert_eq!(
        keys(&bank["questions"][0]).last().map(String::as_str),
        Some("bank")
    );
}

#[test]
fn updating_a_bank_retires_only_within_it_and_keeps_its_name() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    let s = Store::open_with_clock(&db, ticking()).expect("open");
    let (sh, qs) = workbook("q");
    s.apply_import("legacy.xlsx", &sh, &qs).expect("legacy");
    let (_, b) = s
        .apply_import_for(ImportTarget::New, "Mine.xlsx", &sh, &qs)
        .expect("new")
        .expect("created");
    let before = s.banks().expect("banks")["banks"][1].clone();
    assert_eq!(before["name"], "Mine");
    // the bank's questions get a new stem for #1, #3 and #4 disappear
    let mut v2 = qs[..2].to_vec();
    v2[0].stem = "一つ目（改）".into();
    let plan = s
        .plan_import_for(ImportTarget::Bank(b), &sh, &v2)
        .expect("plan")
        .expect("exists");
    assert_eq!(
        (
            plan["added"].clone(),
            plan["updated"].clone(),
            plan["unchanged"].clone(),
            plan["removed"].clone()
        ),
        (json!(0), json!(1), json!(1), json!(2))
    );
    assert_eq!(plan["removed_samples"], json!(["三つ目", "四つ目"]));
    let (result, id) = s
        .apply_import_for(ImportTarget::Bank(b), "Mine v2.xlsx", &sh, &v2)
        .expect("apply")
        .expect("exists");
    assert_eq!((result, id), (plan, b));
    assert_eq!(active_in(&db, 1), 4, "the legacy bank is untouched");
    assert_eq!(active_in(&db, b), 2);
    let after = s.banks().expect("banks")["banks"][1].clone();
    assert_eq!(after["name"], "Mine", "updating never renames");
    assert_eq!(after["filename"], "Mine v2.xlsx");
    assert_ne!(after["updatedAt"], before["updatedAt"]);
    assert_eq!(after["createdAt"], before["createdAt"]);
    assert_eq!(
        (after["total"].clone(), after["inactive"].clone()),
        (json!(2), json!(2))
    );
    assert_eq!(
        after["sheets"],
        json!([{"name": "S1", "usable": 2, "inactive": 1}, {"name": "S2", "usable": 0, "inactive": 1}]),
        "a sheet with only retired questions is still listed"
    );
    let import_banks = rows(&raw(&db), "SELECT bank_id FROM imports ORDER BY id");
    assert_eq!(
        import_banks,
        vec![
            vec![Sql::Integer(1)],
            vec![Sql::Integer(b)],
            vec![Sql::Integer(b)]
        ]
    );
    // an unknown bank: no plan, nothing written
    let snapshot = dbc::db_bytes(&db);
    assert_eq!(
        s.plan_import_for(ImportTarget::Bank(99), &sh, &qs)
            .expect("plan"),
        None
    );
    assert_eq!(
        s.apply_import_for(ImportTarget::Bank(99), "x.xlsx", &sh, &qs)
            .expect("apply"),
        None
    );
    assert_eq!(dbc::db_bytes(&db), snapshot);
}

#[test]
fn the_legacy_flow_without_a_target_is_unchanged() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    let s = open_fixed(&db);
    // another bank first: the legacy flow must neither see nor retire it
    let (sh, qs) = workbook("z");
    s.apply_import_for(ImportTarget::New, "other.xlsx", &sh, &qs)
        .expect("new");
    let (sh, qs) = workbook("a");
    let plan = s.plan_import(&sh, &qs).expect("plan");
    assert_eq!(plan["added"], 4);
    let r = s.apply_import("first.xlsx", &sh, &qs).expect("first");
    assert_eq!(r, plan);
    let legacy = s.banks().expect("banks")["banks"][1].clone();
    assert_eq!(
        (legacy["name"].clone(), legacy["legacy"].clone()),
        (json!("first"), json!(true))
    );
    assert_eq!(
        bank_ids(&db, 2),
        ["a1", "a2", "a3", "a4"],
        "parser ids, unscoped"
    );
    // the v0.4 update semantics, within the legacy bank
    let mut v2 = vec![
        qs[0].clone(),
        qs[1].clone(),
        single("S1", "a5", "五つ目", 5),
    ];
    v2[1].stem = "変更".into();
    let r = s
        .apply_import("second.xlsx", &[sheet("S1", 3)], &v2)
        .expect("second");
    assert_eq!(
        (
            r["added"].clone(),
            r["updated"].clone(),
            r["unchanged"].clone(),
            r["removed"].clone()
        ),
        (json!(1), json!(1), json!(1), json!(2))
    );
    let r = s.apply_import("third.xlsx", &sh, &qs).expect("third");
    assert_eq!(r["restored"], 2);
    let banks = s.banks().expect("banks");
    assert_eq!(
        banks["banks"].as_array().map(Vec::len),
        Some(2),
        "still one legacy bank"
    );
    assert_eq!(banks["banks"][1]["filename"], "third.xlsx");
    assert_eq!(banks["banks"][1]["name"], "first");
    assert_eq!(active_in(&db, 1), 4, "the other bank is untouched");
    let bank = s.bank().expect("bank");
    assert_eq!(bank["file"], "third.xlsx");
    assert_eq!(s.import_history(10).expect("imports").len(), 4);
}

/// Legacy bank L (ids l1..l4), new bank N (scoped ids of n1..n4); exams: only L, only N, mixed; with answers.
fn two_banks_with_records(db: &Path) -> (Store, Vec<String>, Vec<String>) {
    let s = open_fixed(db);
    let (sh, qs) = workbook("l");
    s.apply_import("legacy.xlsx", &sh, &qs).expect("legacy");
    let (sh, qs) = workbook("n");
    s.apply_import_for(ImportTarget::New, "new.xlsx", &sh, &qs)
        .expect("new");
    let l = bank_ids(db, 1);
    let n = bank_ids(db, 2);
    let res = |ids: &[&String]| -> Attempt {
        attempt(&ids.iter().map(|i| (i.as_str(), false)).collect::<Vec<_>>())
    };
    let ans = |k: usize| vec![Some(json!(["A"])); k];
    s.record_attempt_with_answers(ADMIN, &res(&[&l[0], &l[1]]), &ans(2))
        .expect("only L");
    s.record_attempt_with_answers(ADMIN, &res(&[&n[0], &n[1], &n[2]]), &ans(3))
        .expect("only N");
    s.record_attempt_with_answers(ADMIN, &res(&[&l[2], &n[3]]), &ans(2))
        .expect("mixed");
    (s, l, n)
}

#[test]
fn deleting_a_bank_without_records_keeps_the_study_data() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    let (s, l, n) = two_banks_with_records(&db);
    assert_eq!(
        s.delete_bank(2, false).expect("delete"),
        Some(DeletedBank {
            id: 2,
            legacy: false,
            name: "new".to_string(),
        })
    );
    let c = raw(&db);
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM questions WHERE bank_id=2"),
        0
    );
    assert_eq!(count(&c, "SELECT COUNT(*) FROM questions"), 4);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM imports"), 1);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM banks"), 1);
    // (v0.6: statistics are per user, in user_stats)
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM user_stats"),
        7,
        "stats stay"
    );
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempts"), 3);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempt_items"), 7);
    let recs = s.records(ADMIN).expect("records");
    // newest first: mixed, only N, only L
    assert_eq!(
        recs["records"][0]["banks"],
        json!([{"id": 1, "name": "legacy", "deleted": false}, {"id": 2, "name": "new", "deleted": true}])
    );
    assert_eq!(
        recs["records"][1]["banks"],
        json!([{"id": 2, "name": "new", "deleted": true}])
    );
    let detail = s.record(ADMIN, 2).expect("record").expect("exists");
    assert!(detail["items"]
        .as_array()
        .expect("items")
        .iter()
        .all(|i| i["question"].is_null()));
    assert_eq!(detail["items"][0]["questionId"], n[0].as_str());
    let progress = s.progress(ADMIN).expect("progress");
    assert_eq!(progress["questions"][&n[0]]["attempts"], 1);
    assert_eq!(progress["questions"][&l[0]]["attempts"], 1);
    assert_eq!(s.delete_bank(2, false).expect("again"), None);
}

#[test]
fn deleting_a_bank_with_records_removes_its_study_data() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    let (s, l, n) = two_banks_with_records(&db);
    assert_eq!(
        s.delete_bank(2, true).expect("delete"),
        Some(DeletedBank {
            id: 2,
            legacy: false,
            name: "new".to_string(),
        })
    );
    let c = raw(&db);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM questions"), 4);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM imports"), 1);
    assert_eq!(
        rows(
            &c,
            "SELECT question_id FROM user_stats ORDER BY question_id"
        ),
        vec![
            vec![Sql::Text(l[0].clone())],
            vec![Sql::Text(l[1].clone())],
            vec![Sql::Text(l[2].clone())]
        ],
        "only the legacy bank's stats are left"
    );
    // exam 2 (only N) is gone with its detail; exam 3 (mixed) keeps only its L item; exam 1 untouched
    assert_eq!(
        rows(&c, "SELECT id FROM attempts ORDER BY id"),
        vec![vec![Sql::Integer(1)], vec![Sql::Integer(3)]]
    );
    assert_eq!(
        rows(
            &c,
            "SELECT attempt_id, seq, question_id FROM attempt_items ORDER BY attempt_id, seq"
        ),
        vec![
            vec![Sql::Integer(1), Sql::Integer(0), Sql::Text(l[0].clone())],
            vec![Sql::Integer(1), Sql::Integer(1), Sql::Text(l[1].clone())],
            vec![Sql::Integer(3), Sql::Integer(0), Sql::Text(l[2].clone())],
        ]
    );
    assert!(n
        .iter()
        .all(|i| s.progress(ADMIN).expect("p")["questions"].get(i).is_none()));
    // deleting the legacy bank reports it as such
    assert_eq!(
        s.delete_bank(1, true).expect("delete"),
        Some(DeletedBank {
            id: 1,
            legacy: true,
            name: "legacy".to_string(),
        })
    );
    // exam 1 named only the legacy bank: gone; exam 3 named the legacy bank and bank 2, which no longer exists: gone too,
    // instead of staying behind as an empty record
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempts"), 0);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempt_items"), 0);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM user_stats"), 0);
    // a later upload without target creates a fresh legacy bank
    let (sh, qs) = workbook("l");
    s.apply_import("again.xlsx", &sh, &qs).expect("again");
    assert_eq!(
        rows(&c, "SELECT id, legacy_ids FROM banks"),
        vec![vec![Sql::Integer(3), Sql::Integer(1)]]
    );
}

// ---------------------------------------------------------------- records

#[test]
fn records_list_detail_and_deletes() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    let s = open_fixed(&db);
    let (sh, qs) = workbook("a");
    s.apply_import("bank.xlsx", &sh, &qs).expect("import");
    // a record from before v0.5 (no detail), claimed by the administrator (v0.6)
    raw(&db)
        .execute(
            "INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, user_id) VALUES('2026-01-01 00:00', '[\"S1\"]', 'weak', 2, 1, 50, 9, 1)",
            [],
        )
        .expect("old record");
    // "ghost" never existed: since v0.6.5 it is left out of the record and named in "notCounted"
    let p = s
        .record_attempt_with_answers(
            ADMIN,
            &attempt(&[("a1", true), ("a3", false), ("a4", false), ("ghost", true)]),
            &[
                Some(json!(["B"])),
                Some(json!(["A", "C"])),
                Some(json!([0, null])),
                None,
            ],
        )
        .expect("attempt");
    assert_eq!(p["notCounted"], json!(["ghost"]));
    assert!(
        p["questions"].get("ghost").is_none(),
        "no statistics for it"
    );
    let recs = s.records(ADMIN).expect("records");
    assert_eq!(keys(&recs), ["records", "retention"]);
    assert_eq!(recs["retention"], 500);
    let r = &recs["records"][0];
    assert_eq!(
        keys(r),
        [
            "id", "at", "banks", "sheets", "mode", "total", "correct", "percent", "seconds",
            "answered", "detail", "kind"
        ]
    );
    assert_eq!(
        r,
        &json!({"id": 2, "at": FIXED_NOW, "banks": [{"id": 1, "name": "bank", "deleted": false}], "sheets": ["S1"],
                "mode": "all", "total": 3, "correct": 1, "percent": 33, "seconds": 42, "answered": 3, "detail": true,
                "kind": "exam"})
    );
    assert_eq!(
        recs["records"][1],
        json!({"id": 1, "at": "2026-01-01 00:00", "banks": [], "sheets": ["S1"], "mode": "weak", "total": 2,
               "correct": 1, "percent": 50, "seconds": 9, "answered": null, "detail": false, "kind": "exam"})
    );
    // retire a3 (it still shows, with active false)
    s.apply_import(
        "bank.xlsx",
        &sh,
        &[qs[0].clone(), qs[1].clone(), qs[3].clone()],
    )
    .expect("retire a3");
    let d = s.record(ADMIN, 2).expect("record").expect("exists");
    assert_eq!(keys(&d), ["record", "items"]);
    // v0.9: the detail of one record also names its session (null for a record saved without one)
    let mut want = recs["records"][0].clone();
    want["sessionKey"] = json!(null);
    want["startedAt"] = json!(null);
    assert_eq!(d["record"], want);
    let items = d["items"].as_array().expect("items");
    assert_eq!(items.len(), 3);
    assert_eq!(
        keys(&items[0]),
        ["seq", "questionId", "answer", "correct", "question"]
    );
    assert_eq!(
        (
            items[0]["seq"].clone(),
            items[0]["questionId"].clone(),
            items[0]["answer"].clone(),
            items[0]["correct"].clone()
        ),
        (json!(0), json!("a1"), json!(["B"]), json!(true))
    );
    assert_eq!(items[1]["answer"], json!(["A", "C"]));
    assert_eq!(items[2]["answer"], json!([0, null]));
    let qk = keys(&items[0]["question"]);
    assert_eq!(qk[qk.len() - 5..], ["id", "sheet", "row", "bank", "active"]);
    assert_eq!(items[0]["question"]["active"], true);
    assert_eq!(items[0]["question"]["stem"], "一つ目");
    assert_eq!(
        items[1]["question"]["active"], false,
        "retired questions are shown"
    );
    assert_eq!(items[1]["question"]["bank"], 1);
    let old = s.record(ADMIN, 1).expect("record").expect("exists");
    assert_eq!(old["items"], json!([]));
    assert_eq!(s.record(ADMIN, 99).expect("record"), None);

    // delete one: record and detail go, stats stay
    let stats_before = s.progress(ADMIN).expect("p")["questions"].clone();
    assert!(s.delete_record(ADMIN, 2).expect("delete"));
    assert!(!s.delete_record(ADMIN, 2).expect("again"));
    assert_eq!(count(&raw(&db), "SELECT COUNT(*) FROM attempt_items"), 0);
    assert_eq!(s.progress(ADMIN).expect("p")["questions"], stats_before);
    assert_eq!(
        s.records(ADMIN).expect("r")["records"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    // delete all, keeping the stats; then with the stats
    s.record_attempt(ADMIN, &attempt(&[("a1", true)]))
        .expect("attempt");
    s.delete_all_records(ADMIN, false).expect("all");
    assert_eq!(s.records(ADMIN).expect("r")["records"], json!([]));
    assert_eq!(count(&raw(&db), "SELECT COUNT(*) FROM attempt_items"), 0);
    assert_ne!(s.progress(ADMIN).expect("p")["questions"], json!({}));
    s.delete_all_records(ADMIN, true).expect("all");
    assert_eq!(s.progress(ADMIN).expect("p")["questions"], json!({}));
    // /api/reset also clears the detail
    s.record_attempt(ADMIN, &attempt(&[("a1", true)]))
        .expect("attempt");
    s.reset_progress(ADMIN).expect("reset");
    assert_eq!(count(&raw(&db), "SELECT COUNT(*) FROM attempt_items"), 0);
}

#[test]
fn retention_prunes_records_together_with_their_detail() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    let s = open_fixed(&db);
    seed_questions(&db, &["q"]);
    {
        // RECORD_RETENTION exams of the administrator with two detail rows each, written directly (one transaction instead of 500)
        let mut c = raw(&db);
        let tx = c.transaction().expect("tx");
        for i in 1..=RECORD_RETENTION {
            tx.execute(
                "INSERT INTO attempts(id, at, sheets, mode, total, correct, percent, seconds, user_id) VALUES(?1, 't', '[]', 'all', 2, 1, 50, 1, 1)",
                [i],
            )
            .expect("attempt");
            for seq in 0..2 {
                tx.execute(
                    "INSERT INTO attempt_items(attempt_id, seq, question_id, answer, correct) VALUES(?1, ?2, 'q', NULL, 0)",
                    [i, seq],
                )
                .expect("item");
            }
        }
        tx.commit().expect("commit");
    }
    let p = s
        .record_attempt_with_answers(ADMIN, &attempt(&[("q", true)]), &[Some(json!(["A"]))])
        .expect("one more");
    assert_eq!(
        p["history"].as_array().map(Vec::len),
        Some(60),
        "/api/progress still shows 60"
    );
    let c = raw(&db);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempts"), RECORD_RETENTION);
    assert_eq!(
        count(&c, "SELECT MIN(id) FROM attempts"),
        2,
        "the oldest exam was pruned"
    );
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM attempt_items WHERE attempt_id=1"),
        0,
        "with its detail"
    );
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM attempt_items"),
        (RECORD_RETENTION - 1) * 2 + 1
    );
    assert_eq!(
        s.records(ADMIN).expect("records")["records"]
            .as_array()
            .map(Vec::len),
        Some(RECORD_RETENTION as usize)
    );
}

#[test]
fn attempt_snapshot_names_the_banks_in_order_of_first_appearance() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    let s = open_fixed(&db);
    let (sh, qs) = workbook("a");
    s.apply_import("Legacy.xlsx", &sh, &qs).expect("legacy");
    s.apply_import_for(ImportTarget::New, "Second.xlsx", &sh, &qs)
        .expect("new");
    let n = bank_ids(&db, 2);
    s.record_attempt(
        ADMIN,
        &attempt(&[
            (&n[0], true),
            ("unknown", true),
            ("a1", false),
            (&n[1], true),
        ]),
    )
    .expect("attempt");
    assert_eq!(
        text(&raw(&db), "SELECT banks FROM attempts"),
        r#"[{"id": 2, "name": "Second"}, {"id": 1, "name": "Legacy"}]"#
    );
    // an exam of questions that do not exist (any more) is not saved at all (v0.6.5)
    assert!(matches!(
        s.record_attempt(ADMIN, &attempt(&[("unknown", true)])),
        Err(DbError::QuestionsGone)
    ));
    assert_eq!(count(&raw(&db), "SELECT COUNT(*) FROM attempts"), 1);
    let banks = s.banks().expect("banks");
    assert_eq!(
        (
            banks["banks"][0]["records"].clone(),
            banks["banks"][1]["records"].clone()
        ),
        (json!(1), json!(1))
    );
    assert_eq!(
        (
            banks["banks"][0]["studied"].clone(),
            banks["banks"][1]["studied"].clone()
        ),
        (json!(1), json!(2))
    );
}

// ---------------------------------------------------------------- settings

#[test]
fn settings_defaults_update_invalid_and_reset() {
    let dir = tmp();
    let db = dir.path().join("x.db");
    let s = open_fixed(&db);
    let defaults =
        json!({"theme": "system", "uiLang": "en", "autoAdvance": false, "questionLang": "ja"});
    let got = s.settings(ADMIN).expect("settings");
    assert_eq!(got, defaults);
    assert_eq!(
        keys(&got),
        ["theme", "uiLang", "autoAdvance", "questionLang"]
    );
    let got = s
        .update_settings(
            ADMIN,
            &[
                ("uiLang".into(), json!("ja")),
                ("theme".into(), json!("dark")),
                ("autoAdvance".into(), json!(true)),
                ("questionLang".into(), json!("zh")),
            ],
        )
        .expect("update");
    assert_eq!(
        keys(&got),
        ["theme", "uiLang", "autoAdvance", "questionLang"]
    );
    assert_eq!(
        got,
        json!({"theme": "dark", "uiLang": "ja", "autoAdvance": true, "questionLang": "zh"})
    );
    // one invalid pair: nothing written (v0.6: settings are per user, in user_settings)
    let before = rows(&raw(&db), "SELECT * FROM user_settings ORDER BY key");
    assert!(s
        .update_settings(
            ADMIN,
            &[
                ("theme".into(), json!("light")),
                ("uiLang".into(), json!("fr"))
            ]
        )
        .is_err());
    assert!(s
        .update_settings(ADMIN, &[("other".into(), json!("x"))])
        .is_err());
    assert_eq!(
        rows(&raw(&db), "SELECT * FROM user_settings ORDER BY key"),
        before
    );
    // invalid stored values are ignored
    raw(&db)
        .execute_batch("UPDATE user_settings SET value='\"purple\"' WHERE key='theme'; UPDATE user_settings SET value='not json' WHERE key='uiLang';")
        .expect("corrupt");
    assert_eq!(s.settings(ADMIN).expect("s")["theme"], "system");
    assert_eq!(s.settings(ADMIN).expect("s")["uiLang"], "en");
    // survives a restart
    drop(s);
    let s = open_fixed(&db);
    assert_eq!(s.settings(ADMIN).expect("s")["autoAdvance"], true);
    // reset keeps questionLang
    assert_eq!(
        s.reset_settings(ADMIN).expect("reset"),
        json!({"theme": "system", "uiLang": "en", "autoAdvance": false, "questionLang": "zh"})
    );
    assert_eq!(
        rows(&raw(&db), "SELECT key, value FROM user_settings"),
        vec![vec![
            Sql::Text("questionLang".into()),
            Sql::Text("\"zh\"".into())
        ]]
    );
}

// ---------------------------------------------------------------- API (real engine)

struct RealApp {
    app: TestApp,
    store: Arc<Store>,
    db: PathBuf,
    _dir: Option<tempfile::TempDir>,
}

fn real_app_at(db: &Path) -> RealApp {
    let store = Arc::new(Store::open_with_clock(db, dbc::fixed(FIXED_NOW)).expect("store"));
    let engine = Arc::new(RealEngine::new(store.clone()));
    let mut state = AppState::new(apic::default_config(), store.clone(), engine, None);
    // TEST-ONLY: these tests import the committed 3-column demo fixture through the API (production code always refuses
    // the old layouts; tests/v07_bank_access_and_layouts.rs checks that)
    Arc::get_mut(&mut state.0)
        .expect("a new state is not shared yet")
        .old_layouts = xexam_backend::services::OldLayouts::AllowForTests;
    RealApp {
        app: TestApp {
            router: apic::signed_in_router(&state),
            state,
            engine: FakeEngine::new(),
            shutdown_rx: None,
            tmp: None,
        },
        store,
        db: db.to_path_buf(),
        _dir: None,
    }
}

fn real_app() -> RealApp {
    let dir = tmp();
    let mut a = real_app_at(&dir.path().join("xexam.db"));
    a._dir = Some(dir);
    a
}

fn demo() -> Vec<u8> {
    std::fs::read(baseline("fixtures/demo.xlsx")).expect("demo.xlsx")
}

async fn upload_to(a: &RealApp, target: Option<&[u8]>, name: &str) -> Resp {
    let mut hs: Vec<(&str, &[u8])> = vec![
        ("content-type", b"application/octet-stream"),
        ("x-filename", name.as_bytes()),
    ];
    if let Some(t) = target {
        hs.push(("x-bank-target", t));
    }
    post_raw(&a.app, "/api/upload", demo(), &hs).await
}

async fn confirm(a: &RealApp, preview: &Resp) -> Resp {
    let tok = preview.json()["token"].as_str().expect("token").to_string();
    post_json(
        &a.app,
        "/api/import/confirm",
        &format!(r#"{{"token": "{tok}"}}"#),
    )
    .await
}

#[tokio::test]
async fn api_uploads_with_and_without_a_bank_target() {
    let a = real_app();
    let r = get(&a.app, "/api/banks").await;
    assert_eq!(
        (r.status, r.text().as_str()),
        (StatusCode::OK, r#"{"banks": []}"#)
    );

    // no header: v0.4 behaviour, the confirm response has no bankId
    let p = upload_to(&a, None, "demo.xlsx").await;
    assert_eq!(p.status, 200, "{}", p.text());
    let usable = p.json()["usable"].as_i64().expect("usable");
    assert!(usable > 0);
    let c = confirm(&a, &p).await;
    assert_eq!(c.status, 200, "{}", c.text());
    assert_eq!(keys(&c.json()), ["result", "bank"]);
    let banks = get(&a.app, "/api/banks").await.json();
    assert_eq!(banks["banks"][0]["legacy"], true);
    assert_eq!(banks["banks"][0]["name"], "demo");

    // "new" (trimmed, any case): a second bank; the preview counts every question as added
    for (i, target) in [&b"new"[..], b" NEW ", b"New"].iter().enumerate() {
        let p = upload_to(&a, Some(target), "demo.xlsx").await;
        assert_eq!(p.status, 200, "{}", p.text());
        let pj = p.json();
        assert_eq!(
            (
                pj["added"].as_i64(),
                pj["removed"].as_i64(),
                pj["unchanged"].as_i64()
            ),
            (Some(usable), Some(0), Some(0))
        );
        assert_eq!(
            keys(&pj),
            [
                "sheets",
                "added",
                "updated",
                "unchanged",
                "restored",
                "removed",
                "skipped",
                "usable",
                "removed_samples",
                "removed_stats",
                "filename",
                "token"
            ],
            "the preview shape is unchanged"
        );
        let c = confirm(&a, &p).await;
        assert_eq!(keys(&c.json()), ["result", "bank", "bankId"]);
        assert_eq!(c.json()["bankId"], i as i64 + 2);
        assert!(c.text().ends_with(&format!(r#", "bankId": {}}}"#, i + 2)));
    }
    let banks = get(&a.app, "/api/banks").await.json();
    let names: Vec<&str> = banks["banks"]
        .as_array()
        .expect("list")
        .iter()
        .map(|b| b["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, ["demo", "demo (2)", "demo (3)", "demo (4)"]);

    // an id: update that bank; the preview is computed against it only
    let p = upload_to(&a, Some(b"2"), "demo again.xlsx").await;
    assert_eq!(p.status, 200, "{}", p.text());
    assert_eq!(
        (p.json()["added"].as_i64(), p.json()["unchanged"].as_i64()),
        (Some(0), Some(usable))
    );
    let c = confirm(&a, &p).await;
    assert_eq!(c.json()["bankId"], 2);
    let banks = get(&a.app, "/api/banks").await.json();
    assert_eq!(banks["banks"][1]["filename"], "demo again.xlsx");
    assert_eq!(banks["banks"][1]["name"], "demo (2)");

    // /api/bank: every question ends with its bank id
    let bank = get(&a.app, "/api/bank").await.json();
    let qs = bank["questions"].as_array().expect("questions");
    assert_eq!(qs.len() as i64, usable * 4);
    assert!(qs
        .iter()
        .all(|q| keys(q).last().map(String::as_str) == Some("bank")));
    assert_eq!(qs[0]["bank"], 1);
    assert_eq!(qs[qs.len() - 1]["bank"], 4);

    // unknown id → 404, garbage → 400; neither touches the staged upload
    let staged = upload_to(&a, Some(b"3"), "staged.xlsx").await;
    assert_eq!(staged.status, 200);
    for target in [&b"999"[..], b"0", b" 5 ", b"000000000000000099"] {
        let r = upload_to(&a, Some(target), "x.xlsx").await;
        assert_eq!(
            (r.status, r.error().as_str()),
            (StatusCode::NOT_FOUND, MSG_BANK_NOT_FOUND),
            "{:?}",
            String::from_utf8_lossy(target)
        );
    }
    for target in [
        &b"abc"[..],
        b"",
        b"   ",
        b"-1",
        b"+1",
        b"1.5",
        b"0x1",
        b"1e3",
        b"1234567890123456789",
        b"new2",
        b"new bank",
        b"2 3",
        b"\xe6\x96\xb0",
        b"legacy",
    ] {
        let r = upload_to(&a, Some(target), "x.xlsx").await;
        assert_eq!(
            (r.status, r.error().as_str()),
            (StatusCode::BAD_REQUEST, MSG_BAD_TARGET),
            "{:?}",
            String::from_utf8_lossy(target)
        );
    }
    let c = confirm(&a, &staged).await;
    assert_eq!(
        (c.status, c.json()["bankId"].clone()),
        (StatusCode::OK, json!(3))
    );
}

#[tokio::test]
async fn a_deleted_target_bank_is_404_at_confirm_and_deleting_drops_its_upload() {
    let a = real_app();
    for _ in 0..2 {
        let p = upload_to(&a, Some(b"new"), "demo.xlsx").await;
        assert_eq!(confirm(&a, &p).await.status, 200);
    }
    // deleted behind the API's back (e.g. another process): confirm writes nothing and answers 404, the token is used up
    let p = upload_to(&a, Some(b"1"), "demo.xlsx").await;
    assert_eq!(p.status, 200);
    assert!(a.store.delete_bank(1, false).expect("delete").is_some());
    let before = dbc::db_bytes(&a.db);
    let c = confirm(&a, &p).await;
    assert_eq!(
        (c.status, c.error().as_str()),
        (StatusCode::NOT_FOUND, MSG_BANK_NOT_FOUND)
    );
    assert_eq!(dbc::db_bytes(&a.db), before, "nothing written");
    let again = confirm(&a, &p).await;
    assert_eq!(
        (again.status, again.error().as_str()),
        (StatusCode::CONFLICT, GONE)
    );

    // deleted through the API: the staged upload for that bank is dropped
    let p = upload_to(&a, Some(b"2"), "demo.xlsx").await;
    let d = post_json(&a.app, "/api/banks/delete", r#"{"id": 2}"#).await;
    assert_eq!(d.status, 200, "{}", d.text());
    assert_eq!(confirm(&a, &p).await.status, 409);
    // … but an upload for another target stays
    let keep = upload_to(&a, Some(b"new"), "demo.xlsx").await;
    let other = upload_to(&a, None, "legacy.xlsx").await;
    assert_eq!(
        confirm(&a, &other).await.status,
        200,
        "creates the legacy bank (3)"
    );
    let keep2 = upload_to(&a, Some(b"new"), "demo.xlsx").await;
    assert_eq!(
        confirm(&a, &keep).await.status,
        409,
        "replaced by later uploads (single slot)"
    );
    let d = post_json(&a.app, "/api/banks/delete", r#"{"id": 3}"#).await;
    assert_eq!(d.status, 200);
    assert_eq!(
        confirm(&a, &keep2).await.status,
        200,
        "a new-bank upload is never dropped"
    );
    // a headerless upload targets the legacy bank: deleting that bank drops it
    let p = upload_to(&a, None, "demo.xlsx").await;
    assert_eq!(
        confirm(&a, &p).await.status,
        200,
        "creates a new legacy bank"
    );
    let p = upload_to(&a, None, "demo.xlsx").await;
    let legacy_id = get(&a.app, "/api/banks").await.json()["banks"]
        .as_array()
        .expect("list")
        .iter()
        .find(|b| b["legacy"] == true)
        .map(|b| b["id"].clone())
        .expect("legacy bank");
    let d = post_json(
        &a.app,
        "/api/banks/delete",
        &format!(r#"{{"id": {legacy_id}}}"#),
    )
    .await;
    assert_eq!(d.status, 200);
    assert_eq!(confirm(&a, &p).await.status, 409);
}

#[tokio::test]
async fn api_banks_delete_validates_and_answers_banks_bank_progress() {
    let a = real_app();
    let p = upload_to(&a, None, "demo.xlsx").await;
    assert_eq!(confirm(&a, &p).await.status, 200);
    let p = upload_to(&a, Some(b"new"), "second.xlsx").await;
    assert_eq!(confirm(&a, &p).await.status, 200);
    let qid = get(&a.app, "/api/bank").await.json()["questions"]
        .as_array()
        .expect("questions")
        .iter()
        .find(|q| q["bank"] == 2)
        .map(|q| q["id"].as_str().expect("id").to_string())
        .expect("a question of bank 2");
    let r = post_json(
        &a.app,
        "/api/attempt",
        &format!(r#"{{"results": [{{"id": "{qid}", "correct": false, "answer": ["A"]}}]}}"#),
    )
    .await;
    assert_eq!(r.status, 200);
    let before = dbc::db_bytes(&a.db);
    for body in [
        "{}",
        r#"{"id": "2"}"#,
        r#"{"id": true}"#,
        r#"{"id": 2.0}"#,
        r#"{"id": null}"#,
        r#"{"id": [2]}"#,
        r#"{"id": 2, "withRecords": "yes"}"#,
        r#"{"id": 2, "withRecords": 1}"#,
        r#"{"id": 2, "withRecords": null}"#,
    ] {
        let r = post_json(&a.app, "/api/banks/delete", body).await;
        assert_eq!(
            (r.status, r.error().as_str()),
            (StatusCode::BAD_REQUEST, MSG_BAD_REQUEST),
            "{body}"
        );
    }
    for body in [
        r#"{"id": 99}"#,
        r#"{"id": 0}"#,
        r#"{"id": -2}"#,
        r#"{"id": 99999999999999999999999}"#,
    ] {
        let r = post_json(&a.app, "/api/banks/delete", body).await;
        assert_eq!(
            (r.status, r.error().as_str()),
            (StatusCode::NOT_FOUND, MSG_BANK_NOT_FOUND),
            "{body}"
        );
    }
    assert_eq!(dbc::db_bytes(&a.db), before, "nothing written");
    let r = post_json(
        &a.app,
        "/api/banks/delete",
        r#"{"id": 2, "withRecords": true, "x": 1}"#,
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let j = r.json();
    assert_eq!(keys(&j), ["banks", "bank", "progress"]);
    assert_eq!(j["banks"].as_array().map(Vec::len), Some(1));
    assert!(j["bank"]["questions"]
        .as_array()
        .expect("questions")
        .iter()
        .all(|q| q["bank"] == 1));
    assert_eq!(j["progress"], get(&a.app, "/api/progress").await.json());
    assert_eq!(
        j["progress"]["questions"],
        json!({}),
        "withRecords removed the stats"
    );
    assert_eq!(
        j["progress"]["history"],
        json!([]),
        "and the exam that had only this bank"
    );
    let r = post_json(
        &a.app,
        "/api/banks/delete",
        r#"{"id": 1, "withRecords": false}"#,
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["banks"], json!([]));
}

#[tokio::test]
async fn api_records_attempt_answers_and_query_parsing() {
    let a = real_app();
    let p = upload_to(&a, None, "demo.xlsx").await;
    assert_eq!(confirm(&a, &p).await.status, 200);
    let bank = get(&a.app, "/api/bank").await.json();
    let ids: Vec<String> = bank["questions"]
        .as_array()
        .expect("questions")
        .iter()
        .map(|q| q["id"].as_str().expect("id").to_string())
        .collect();
    let body = format!(
        r#"{{"results": [
            {{"id": "{0}", "correct": true, "answer": ["B"]}},
            {{"id": "{1}", "correct": false, "answer": ["A", 7, null]}},
            {{"id": "{2}", "correct": false, "answer": "A"}},
            {{"id": "{3}", "correct": true, "answer": ["toolongvalue"]}},
            {{"id": "{4}", "correct": true}}
        ], "sheets": ["demo"], "mode": "weak", "seconds": 61}}"#,
        ids[0], ids[1], ids[2], ids[3], ids[4]
    );
    let r = post_json(&a.app, "/api/attempt", &body).await;
    assert_eq!(r.status, 200);
    assert_eq!(
        keys(&r.json()),
        // v0.9: the new record's id comes last
        [
            "version",
            "questions",
            "history",
            "masterStreak",
            "recordId"
        ],
        "response unchanged"
    );

    let r = get(&a.app, "/api/records").await;
    assert_eq!(r.status, 200);
    let j = r.json();
    assert_eq!(keys(&j), ["records", "retention"]);
    assert_eq!(j["retention"], 500);
    assert_eq!(
        j["records"][0],
        json!({"id": 1, "at": FIXED_NOW, "banks": [{"id": 1, "name": "demo", "deleted": false}], "sheets": ["demo"],
               "mode": "weak", "total": 5, "correct": 3, "percent": 60, "seconds": 61, "answered": 2, "detail": true,
               "kind": "exam"})
    );
    let r = get(&a.app, "/api/record?id=1").await;
    assert_eq!(r.status, 200, "{}", r.text());
    let d = r.json();
    assert_eq!(keys(&d), ["record", "items"]);
    // v0.9: the detail also names the session the record was saved from (null here: the request sent none)
    let mut want = j["records"][0].clone();
    want["sessionKey"] = json!(null);
    want["startedAt"] = json!(null);
    assert_eq!(d["record"], want);
    let answers: Vec<Value> = d["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["answer"].clone())
        .collect();
    assert_eq!(
        answers,
        [
            json!(["B"]),
            json!(["A", 7, null]),
            Value::Null,
            Value::Null,
            Value::Null
        ]
    );
    let mut q0 = bank["questions"][0].clone();
    q0.as_object_mut()
        .expect("q")
        .insert("active".into(), json!(true));
    assert_eq!(
        d["items"][0]["question"], q0,
        "the /api/bank object plus a trailing active"
    );
    // HEAD mirrors GET
    let h = send(
        &a.app.router,
        build(
            "HEAD",
            "/api/record?id=1",
            Some("localhost"),
            &[],
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(h.status, 200);
    assert!(h.body.is_empty());
    assert_eq!(h.header("content-length"), r.header("content-length"));

    // query parsing
    for q in [
        "/api/record",
        "/api/record?",
        "/api/record?id",
        "/api/record?id=",
        "/api/record?id=abc",
        "/api/record?id=-1",
        "/api/record?id=+1",
        "/api/record?id=1.0",
        "/api/record?id=%201",
        "/api/record?id=1&id=1",
        "/api/record?id=1234567890123456789",
        "/api/record?ID=1",
        "/api/record?x=1",
    ] {
        let r = get(&a.app, q).await;
        assert_eq!(
            (r.status, r.error().as_str()),
            (StatusCode::BAD_REQUEST, MSG_BAD_REQUEST),
            "{q}"
        );
    }
    for q in [
        "/api/record?id=%31",
        "/api/record?x=2&id=1",
        "/api/record?%69d=01",
    ] {
        let r = get(&a.app, q).await;
        assert_eq!(r.status, 200, "{q}");
        assert_eq!(r.json()["record"]["id"], 1, "{q}");
    }
    for q in [
        "/api/record?id=0",
        "/api/record?id=2",
        "/api/record?id=999999999999999999",
    ] {
        let r = get(&a.app, q).await;
        assert_eq!(
            (r.status, r.error().as_str()),
            (StatusCode::NOT_FOUND, MSG_RECORD_NOT_FOUND),
            "{q}"
        );
    }

    // POST /api/records/delete
    let r = post_json(
        &a.app,
        "/api/attempt",
        &format!(
            r#"{{"results": [{{"id": "{}", "correct": true}}]}}"#,
            ids[0]
        ),
    )
    .await;
    assert_eq!(r.status, 200);
    let before = dbc::db_bytes(&a.db);
    for body in [
        "{}",
        r#"{"id": "1"}"#,
        r#"{"id": true}"#,
        r#"{"id": 1.5}"#,
        r#"{"id": null}"#,
        r#"{"all": false}"#,
        r#"{"all": 1}"#,
        r#"{"all": "true"}"#,
        r#"{"all": true, "withStats": 1}"#,
        r#"{"all": true, "withStats": null}"#,
        r#"{"id": 1, "all": true}"#,
        r#"{"id": 1, "withStats": true}"#,
        r#"{"withStats": true}"#,
    ] {
        let r = post_json(&a.app, "/api/records/delete", body).await;
        assert_eq!(
            (r.status, r.error().as_str()),
            (StatusCode::BAD_REQUEST, MSG_BAD_REQUEST),
            "{body}"
        );
    }
    for body in [
        r#"{"id": 99}"#,
        r#"{"id": -1}"#,
        r#"{"id": 99999999999999999999999}"#,
    ] {
        let r = post_json(&a.app, "/api/records/delete", body).await;
        assert_eq!(
            (r.status, r.error().as_str()),
            (StatusCode::NOT_FOUND, MSG_RECORD_NOT_FOUND),
            "{body}"
        );
    }
    assert_eq!(dbc::db_bytes(&a.db), before, "nothing written");
    let stats = get(&a.app, "/api/progress").await.json()["questions"].clone();
    let r = post_json(&a.app, "/api/records/delete", r#"{"id": 1}"#).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let j = r.json();
    assert_eq!(keys(&j), ["records", "retention", "progress"]);
    assert_eq!(j["records"].as_array().map(Vec::len), Some(1));
    assert_eq!(j["records"][0]["id"], 2);
    assert_eq!(j["retention"], 500);
    assert_eq!(j["progress"]["questions"], stats, "stats unchanged");
    assert_eq!(get(&a.app, "/api/record?id=1").await.status, 404);
    let r = post_json(&a.app, "/api/records/delete", r#"{"all": true}"#).await;
    assert_eq!(r.json()["records"], json!([]));
    assert_eq!(
        r.json()["progress"]["questions"],
        stats,
        "withStats defaults to false"
    );
    let r = post_json(
        &a.app,
        "/api/records/delete",
        r#"{"all": true, "withStats": true}"#,
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["progress"]["questions"], json!({}));
}

#[tokio::test]
async fn api_settings_roundtrip_validation_reset_and_restart() {
    let dir = tmp();
    let db = dir.path().join("xexam.db");
    let a = real_app_at(&db);
    let defaults =
        r#"{"theme": "system", "uiLang": "en", "autoAdvance": false, "questionLang": "ja"}"#;
    let r = get(&a.app, "/api/settings").await;
    assert_eq!((r.status, r.text().as_str()), (StatusCode::OK, defaults));
    let r = post_json(&a.app, "/api/settings", r#"{"theme": "dark"}"#).await;
    assert_eq!(
        r.text(),
        r#"{"theme": "dark", "uiLang": "en", "autoAdvance": false, "questionLang": "ja"}"#
    );
    let r = post_json(
        &a.app,
        "/api/settings",
        r#"{"uiLang": "zh", "autoAdvance": true, "questionLang": "en", "unknown": [1], "Theme": 5}"#,
    )
    .await;
    assert_eq!(
        r.text(),
        r#"{"theme": "dark", "uiLang": "zh", "autoAdvance": true, "questionLang": "en"}"#
    );
    let r = post_json(&a.app, "/api/settings", "{}").await;
    assert_eq!(r.status, 200, "an empty update just answers the settings");
    let before = dbc::db_bytes(&db);
    for body in [
        r#"{"theme": "blue"}"#,
        r#"{"theme": "Dark"}"#,
        r#"{"theme": null}"#,
        r#"{"uiLang": "fr"}"#,
        r#"{"uiLang": ["en"]}"#,
        r#"{"autoAdvance": "true"}"#,
        r#"{"autoAdvance": 1}"#,
        r#"{"questionLang": 1}"#,
        r#"{"theme": "light", "uiLang": "xx"}"#,
    ] {
        let r = post_json(&a.app, "/api/settings", body).await;
        assert_eq!(
            (r.status, r.error().as_str()),
            (StatusCode::BAD_REQUEST, MSG_BAD_SETTINGS),
            "{body}"
        );
    }
    assert_eq!(dbc::db_bytes(&db), before, "nothing written");
    // restart: the settings live in the database
    drop(a);
    let a = real_app_at(&db);
    assert_eq!(
        get(&a.app, "/api/settings").await.text(),
        r#"{"theme": "dark", "uiLang": "zh", "autoAdvance": true, "questionLang": "en"}"#
    );
    let r = post_raw(
        &a.app,
        "/api/settings/reset",
        vec![],
        &[("content-type", b"application/json")],
    )
    .await;
    assert_eq!(
        (r.status, r.text().as_str()),
        (
            StatusCode::OK,
            r#"{"theme": "system", "uiLang": "en", "autoAdvance": false, "questionLang": "en"}"#
        )
    );
    let r = post_json(&a.app, "/api/settings/reset", "{}").await;
    assert_eq!(r.json()["questionLang"], "en");
    let r = post_json(&a.app, "/api/settings/reset", "[]").await;
    assert_eq!(
        (r.status, r.error().as_str()),
        (StatusCode::BAD_REQUEST, "bad json")
    );
}

const NEW_POSTS: [&str; 4] = [
    "/api/banks/delete",
    "/api/records/delete",
    "/api/settings",
    "/api/settings/reset",
];
const NEW_GETS: [&str; 4] = [
    "/api/banks",
    "/api/records",
    "/api/record?id=1",
    "/api/settings",
];

#[tokio::test]
async fn new_routes_go_through_the_same_guards() {
    let a = real_app();
    let p = upload_to(&a, None, "demo.xlsx").await;
    assert_eq!(confirm(&a, &p).await.status, 200);
    let r = post_json(&a.app, "/api/settings", r#"{"theme": "light"}"#).await;
    assert_eq!(r.status, 200);
    let before = dbc::db_bytes(&a.db);
    let bodies = [
        r#"{"id": 1, "withRecords": true}"#,
        r#"{"all": true, "withStats": true}"#,
        r#"{"theme": "dark"}"#,
        "{}",
    ];
    const JSON_CT: (&str, &[u8]) = ("content-type", b"application/json");
    const TEXT_CT: (&str, &[u8]) = ("content-type", b"text/plain");
    const EVIL_ORIGIN: (&str, &[u8]) = ("origin", b"https://evil.example.com");
    for (path, body) in NEW_POSTS.iter().zip(bodies) {
        let post =
            |host: &'static str, hs: &'static [(&'static str, &'static [u8])], body: Vec<u8>| {
                let router = a.app.router.clone();
                async move {
                    send(
                        &router,
                        build("POST", path, Some(host), hs, Body::from(body)),
                    )
                    .await
                }
            };
        // Host
        let r = post("evil.example.com", &[JSON_CT], body.as_bytes().to_vec()).await;
        assert_eq!(r.status, 403, "{path}");
        assert!(
            r.error().contains("这个请求的 Host 是「evil.example.com」"),
            "{path}: {}",
            r.error()
        );
        // Origin (before Content-Type)
        let r = post(
            "localhost",
            &[TEXT_CT, EVIL_ORIGIN],
            body.as_bytes().to_vec(),
        )
        .await;
        assert_eq!(r.status, 403, "{path}");
        assert!(
            r.error().contains("这个请求来自「evil.example.com」"),
            "{path}: {}",
            r.error()
        );
        // Content-Type
        let r = post("localhost", &[TEXT_CT], body.as_bytes().to_vec()).await;
        assert_eq!(
            (r.status.as_u16(), r.error().as_str()),
            (415, "bad content type"),
            "{path}"
        );
        // size
        let big = format!(r#"{{"pad": "{}"}}"#, "a".repeat(1_000_000));
        let r = post("localhost", &[JSON_CT], big.into_bytes()).await;
        assert_eq!(
            (r.status.as_u16(), r.error().as_str()),
            (413, "too large"),
            "{path}"
        );
        // JSON
        for bad in [&b"{not json"[..], b"[1]", b"null", b"\"x\""] {
            let r = post("localhost", &[JSON_CT], bad.to_vec()).await;
            assert_eq!(
                (r.status.as_u16(), r.error().as_str()),
                (400, "bad json"),
                "{path}"
            );
        }
        // other methods
        let r = send(
            &a.app.router,
            build("DELETE", path, Some("localhost"), &[], Body::empty()),
        )
        .await;
        assert_eq!(r.status, 405, "{path}");
        // GET on a POST-only path
        if !NEW_GETS.contains(path) {
            let r = get(&a.app, path).await;
            assert_eq!(
                (r.status, r.text().as_str()),
                (StatusCode::NOT_FOUND, "not found"),
                "{path}"
            );
        }
    }
    assert_eq!(
        dbc::db_bytes(&a.db),
        before,
        "no rejected request wrote anything"
    );
    for path in NEW_GETS {
        let r = send(
            &a.app.router,
            build("GET", path, Some("evil.example.com"), &[], Body::empty()),
        )
        .await;
        assert_eq!(r.status, 403, "{path}");
        assert!(
            r.text().starts_with("Forbidden host: evil.example.com"),
            "{path}"
        );
        let r = send(
            &a.app.router,
            build("HEAD", path, Some("evil.example.com"), &[], Body::empty()),
        )
        .await;
        assert_eq!(r.status, 403, "{path}");
        assert!(r.body.is_empty());
        let r = get(&a.app, path).await;
        assert!(r.status == 200 || r.status == 404, "{path}: {}", r.status);
        assert_eq!(
            r.header("content-type").as_deref(),
            Some("application/json; charset=utf-8")
        );
        assert_eq!(r.header("cache-control").as_deref(), Some("no-store"));
        // POSTing to a GET-only path is still 404
        let r = post_json(&a.app, path.split('?').next().unwrap_or(path), "{}").await;
        if !NEW_POSTS.contains(&path) {
            assert_eq!(r.status, 404, "{path}");
        }
    }
    // X-Bank-Target is only read after the guards: a bad target behind an evil Host is still a 403
    let r = send(
        &a.app.router,
        build(
            "POST",
            "/api/upload",
            Some("evil.example.com"),
            &[
                ("content-type", b"application/octet-stream"),
                ("x-bank-target", b"garbage"),
            ],
            Body::from(demo()),
        ),
    )
    .await;
    assert_eq!(r.status, 403);
    let r = post_raw(
        &a.app,
        "/api/upload",
        demo(),
        &[
            ("content-type", b"text/plain"),
            ("x-bank-target", b"garbage"),
        ],
    )
    .await;
    assert_eq!(r.status, 415);
}

#[tokio::test]
async fn engines_without_banks_answer_500_on_the_new_routes() {
    // A v0.4-style engine (default trait methods) must fail closed, not pretend
    let a = apic::app();
    for path in NEW_GETS {
        let r = get(&a, path).await;
        assert_eq!(r.status, 500, "{path}");
        assert_eq!(r.error(), "服务器内部出错，请稍后再试。");
    }
    let r = post_json(&a, "/api/settings", r#"{"theme": "dark"}"#).await;
    assert_eq!(r.status, 500);
    let r = post_json(&a, "/api/banks/delete", r#"{"id": 1}"#).await;
    assert_eq!(r.status, 500);
    let r = apic::upload(&a, vec![1; 10], Some("x.xlsx")).await;
    assert_eq!(
        r.status, 400,
        "the legacy upload path still works with such an engine"
    );
}
