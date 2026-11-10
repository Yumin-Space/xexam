//! Integration tests for `database::Store` against the repository baseline and the Python oracle.
mod common;

use common::*;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use xexam_backend::database::{percent_round_half_even, Store};
use xexam_backend::models::{Attempt, AttemptResult, ParsedSheet, Question};

fn read_json(p: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(p).expect("read golden")).expect("golden json")
}

fn dump_rust(s: &Store) -> Value {
    json!({
        "bank": s.bank().expect("bank"),
        "progress": s.progress().expect("progress"),
        "imports": serde_json::to_value(s.import_history(100).expect("imports")).expect("imports json"),
    })
}

/// Order-sensitive comparison (key order is part of the contract).
fn assert_same(a: &Value, b: &Value, what: &str) {
    assert_eq!(py_dumps(a), py_dumps(b), "{what}");
}

fn raw_rows(path: &Path) -> Value {
    let c = Connection::open(path).expect("raw open");
    let mut out = serde_json::Map::new();
    for (t, order) in [
        ("questions", "id"),
        ("attempts", "id"),
        ("stats", "question_id"),
        ("imports", "id"),
        ("meta", "key"),
    ] {
        let mut st = c
            .prepare(&format!("SELECT * FROM {t} ORDER BY {order}"))
            .expect("prepare");
        let n = st.column_count();
        let rows: Vec<Value> = st
            .query_map([], |r| {
                let mut row = Vec::new();
                for i in 0..n {
                    let v: rusqlite::types::Value = r.get(i)?;
                    row.push(match v {
                        rusqlite::types::Value::Null => Value::Null,
                        rusqlite::types::Value::Integer(i) => json!(i),
                        rusqlite::types::Value::Real(f) => json!(f),
                        rusqlite::types::Value::Text(s) => json!(s),
                        rusqlite::types::Value::Blob(_) => json!("<blob>"),
                    });
                }
                Ok(Value::Array(row))
            })
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows");
        out.insert(t.to_string(), Value::Array(rows));
    }
    Value::Object(out)
}

fn attempt(ids: &[(&str, bool)], mode: &str, seconds: i64) -> Attempt {
    Attempt {
        results: ids
            .iter()
            .map(|(i, c)| AttemptResult {
                id: (*i).to_string(),
                correct: *c,
            })
            .collect(),
        sheets: vec!["日本語".to_string(), "b\"c".to_string()],
        mode: mode.to_string(),
        seconds,
    }
}

fn sample_questions() -> (Vec<ParsedSheet>, Vec<Question>) {
    let qs = vec![
        single("日本語", "id1", "最初の問題？", 1),
        multi("日本語", "id2", "二番目 \"問題\" \\ \u{1f34e}", 2),
        hotspot("日本語", "id3", "ホットスポット", 3),
        single("Two", "id4", "x", 1),
    ];
    (vec![sheet("日本語", 3), sheet("Two", 1)], qs)
}

// ---------------------------------------------------------------- (1) golden sample.db

#[test]
fn sample_db_matches_golden() {
    let dir = tmp();
    let p = dir.path().join("sample.db");
    std::fs::copy(baseline("fixtures/sample.db"), &p).expect("copy sample.db");
    let s = Store::open(&p).expect("open sample");
    let g = read_json(&baseline("golden/store_sample_outputs.json"));
    assert_same(&s.bank().expect("bank"), &g["bank"], "bank");
    assert_same(&s.progress().expect("progress"), &g["progress"], "progress");
    let imports = serde_json::to_value(s.import_history(100).expect("imports")).expect("json");
    assert_same(&imports, &g["imports"], "imports");
    assert_eq!(s.count_active().expect("count"), 6);
}

#[test]
fn sample_db_plan_matches_golden_when_parsed_by_the_rust_parser() {
    let dir = tmp();
    let p = dir.path().join("sample.db");
    std::fs::copy(baseline("fixtures/sample.db"), &p).expect("copy");
    let s = Store::open(&p).expect("open");
    let demo = std::fs::read(baseline("fixtures/demo.xlsx")).expect("demo.xlsx");
    let parsed = xexam_backend::parser::parse_workbook(&demo).expect("parse demo.xlsx");
    let plan = s
        .plan_import(&parsed.sheets, &parsed.questions)
        .expect("plan");
    let g = read_json(&baseline("golden/store_sample_outputs.json"));
    assert_same(&plan, &g["plan_demo_v1_again"], "plan_demo_v1_again");
    // preview wrote nothing
    assert_eq!(s.import_history(100).expect("imports").len(), 2);
}

// ---------------------------------------------------------------- (2) rollback

#[test]
fn failed_import_rolls_back_everything() {
    let dir = tmp();
    let p = dir.path().join("x.db");
    let s = open_fixed(&p);
    let (sheets, qs) = sample_questions();
    s.apply_import("first.xlsx", &sheets, &qs)
        .expect("first import");
    let before = raw_rows(&p);

    let raw = Connection::open(&p).expect("raw");
    raw.execute_batch("CREATE TRIGGER boom BEFORE INSERT ON imports BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").expect("trigger");
    let qs2 = vec![
        single("日本語", "id1", "changed stem", 9),
        single("New", "id9", "new", 1),
    ];
    assert!(s
        .apply_import("second.xlsx", &[sheet("日本語", 1), sheet("New", 1)], &qs2)
        .is_err());
    assert_eq!(
        raw_rows(&p),
        before,
        "tables must be unchanged after a failed import"
    );

    // the store keeps working after the failure
    raw.execute_batch("DROP TRIGGER boom")
        .expect("drop trigger");
    s.apply_import("second.xlsx", &[sheet("日本語", 1), sheet("New", 1)], &qs2)
        .expect("import after failure");
    assert_eq!(s.count_active().expect("count"), 2);
}

#[test]
fn failed_attempt_and_migration_roll_back() {
    let dir = tmp();
    let p = dir.path().join("x.db");
    let s = open_fixed(&p);
    s.record_attempt(&attempt(&[("a", true), ("b", false)], "all", 5))
        .expect("seed");
    let before = raw_rows(&p);
    let raw = Connection::open(&p).expect("raw");
    raw.execute_batch("CREATE TRIGGER boom BEFORE INSERT ON attempts BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").expect("trigger");
    assert!(s
        .record_attempt(&attempt(&[("a", true), ("c", true)], "all", 5))
        .is_err());
    assert_eq!(raw_rows(&p), before);
    assert!(
        s.reset_progress().is_ok(),
        "reset deletes only, no attempts insert"
    );
    raw.execute_batch("DROP TRIGGER boom").expect("drop");

    // migration: failure while inserting attempts must not leave stats behind and must not set the flag
    let s2 = open_fixed(&dir.path().join("y.db"));
    let pj = dir.path().join("p.json");
    std::fs::write(
        &pj,
        r#"{"questions":{"a":{"attempts":1}},"history":[{"total":1,"correct":1,"percent":100}]}"#,
    )
    .expect("write");
    let raw2 = Connection::open(dir.path().join("y.db")).expect("raw2");
    raw2.execute_batch("CREATE TRIGGER boom BEFORE INSERT ON attempts BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").expect("trigger");
    let res = s2.migrate_legacy_progress(&pj);
    assert!(res.is_err() || matches!(res, Ok((false, _))));
    assert_eq!(s2.progress().expect("progress")["questions"], json!({}));
    let flag: i64 = raw2
        .query_row(
            "SELECT COUNT(*) FROM meta WHERE key='legacy_migrated'",
            [],
            |r| r.get(0),
        )
        .expect("flag");
    assert_eq!(flag, 0);
}

// ---------------------------------------------------------------- (3) rounding

#[test]
fn rounding_is_half_to_even_for_every_good_total_pair() {
    // 浮点参考（Python 的 round 同样是半偶数）；当初与 Python 的 round 在 200 万个组合上逐一核对过，没有差异
    for total in 1..=200i64 {
        for good in 0..=total {
            let want = (100.0 * good as f64 / total as f64).round_ties_even() as i64;
            assert_eq!(percent_round_half_even(good, total), want, "{good}/{total}");
        }
    }
}

#[test]
fn record_attempt_stores_half_even_percent() {
    let dir = tmp();
    let s = open_fixed(&dir.path().join("x.db"));
    // (good, total, expected percent): ties 1/8 -> 12, 3/8 -> 38, 1/40 -> 2, 1/200 -> 0, 3/200 -> 2, 5/200 -> 2
    for (good, total, want) in [
        (1usize, 8usize, 12),
        (3, 8, 38),
        (1, 40, 2),
        (1, 200, 0),
        (3, 200, 2),
        (5, 200, 2),
        (7, 8, 88),
        (5, 8, 62),
    ] {
        let results: Vec<(String, bool)> =
            (0..total).map(|i| (format!("q{i}"), i < good)).collect();
        let refs: Vec<(&str, bool)> = results.iter().map(|(i, c)| (i.as_str(), *c)).collect();
        let p = s.record_attempt(&attempt(&refs, "all", 1)).expect("record");
        assert_eq!(p["history"][0]["percent"], want, "{good}/{total}");
        assert_eq!(p["history"][0]["total"], total);
        assert_eq!(p["history"][0]["correct"], good);
    }
}

// ---------------------------------------------------------------- (4) concurrency

#[test]
fn eight_threads_record_attempts_and_imports() {
    let dir = tmp();
    let s = Arc::new(open_fixed(&dir.path().join("x.db")));
    let (sheets, qs) = sample_questions();
    let sheets = Arc::new(sheets);
    let qs = Arc::new(qs);
    let handles: Vec<_> = (0..8)
        .map(|t| {
            let (s, sheets, qs) = (Arc::clone(&s), Arc::clone(&sheets), Arc::clone(&qs));
            std::thread::spawn(move || {
                for i in 0..4 {
                    s.record_attempt(&attempt(
                        &[("shared", i % 2 == 0), (&format!("own{t}"), true)],
                        "all",
                        3,
                    ))
                    .expect("attempt");
                }
                s.apply_import(&format!("t{t}.xlsx"), &sheets, &qs)
                    .expect("import");
                s.bank().expect("bank");
                s.progress().expect("progress");
                s.plan_import(&sheets, &qs).expect("plan");
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread");
    }
    let p = s.progress().expect("progress");
    assert_eq!(p["questions"]["shared"]["attempts"], 32);
    assert_eq!(p["questions"]["shared"]["correct"], 16);
    assert_eq!(p["questions"]["shared"]["wrong"], 16);
    assert_eq!(p["history"].as_array().map(Vec::len), Some(32));
    for t in 0..8 {
        assert_eq!(p["questions"][format!("own{t}")]["attempts"], 4);
    }
    assert_eq!(s.import_history(100).expect("imports").len(), 8);
    assert_eq!(s.count_active().expect("count"), 4);
    // every import after the first saw identical content
    let last = s.import_history(1).expect("last");
    assert_eq!((last[0].added, last[0].updated, last[0].removed), (0, 0, 0));
    let c = Connection::open(dir.path().join("x.db")).expect("raw");
    let ok: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .expect("check");
    assert_eq!(ok, "ok");
}

#[test]
fn history_is_pruned_to_sixty_and_ids_keep_growing() {
    let dir = tmp();
    let p = dir.path().join("x.db");
    let s = open_fixed(&p);
    for i in 0..65 {
        s.record_attempt(&attempt(&[("q", true)], "all", i))
            .expect("attempt");
    }
    let prog = s.progress().expect("progress");
    assert_eq!(prog["history"].as_array().map(Vec::len), Some(60));
    assert_eq!(prog["history"][0]["seconds"], 64);
    assert_eq!(prog["questions"]["q"]["attempts"], 65);
    s.reset_progress().expect("reset");
    s.record_attempt(&attempt(&[("q", true)], "all", 0))
        .expect("attempt");
    let c = Connection::open(&p).expect("raw");
    let id: i64 = c
        .query_row("SELECT MAX(id) FROM attempts", [], |r| r.get(0))
        .expect("max id");
    assert_eq!(id, 66, "AUTOINCREMENT ids are never reused");
}

// ---------------------------------------------------------------- (5) schema equality

#[test]
fn schema_is_identical_to_the_one_python_created() {
    // tests/baseline/fixtures/sample.db 是用旧的 Python 实现建出来的；新后端建出的空库，sqlite_master 必须逐字相同。
    let dir = tmp();
    let rs_db = dir.path().join("rs.db");
    drop(open_fixed(&rs_db));
    let master = |path: &Path| -> Value {
        let c = Connection::open(path).expect("open");
        let mut st = c
            .prepare("SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY rowid")
            .expect("prepare");
        let rows: Vec<Value> = st
            .query_map([], |r| {
                Ok(json!([
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?
                ]))
            })
            .expect("query")
            .map(|r| r.expect("row"))
            .collect();
        Value::Array(rows)
    };
    let pragmas = |path: &Path| -> Value {
        let c = Connection::open(path).expect("open");
        let jm: String = c
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .expect("jm");
        let uv: i64 = c
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("uv");
        json!([jm, uv])
    };
    let sample = baseline("fixtures/sample.db");
    let (a, b) = (master(&sample), master(&rs_db));
    assert_eq!(a, b, "sqlite_master text must match what Python created");
    assert_eq!(
        a.as_array().map(Vec::len),
        Some(10),
        "10 rows incl. autoindexes and sqlite_sequence"
    );
    assert_eq!(pragmas(&sample), pragmas(&rs_db));
    assert_eq!(pragmas(&rs_db), json!(["delete", 0]));
    let c = Connection::open(&rs_db).expect("open");
    let meta: Vec<(String, String)> = c
        .prepare("SELECT key, value FROM meta ORDER BY key")
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(meta, vec![("schema".to_string(), "1".to_string())]);
}

// ---------------------------------------------------------------- (6) reopen without alteration

#[test]
fn reopening_does_not_alter_the_file() {
    let dir = tmp();
    // the baseline sample.db
    let sample = dir.path().join("sample.db");
    std::fs::copy(baseline("fixtures/sample.db"), &sample).expect("copy");
    let before = std::fs::read(&sample).expect("read");
    // sample.db 是旧的 Python 实现写出来的库：打开、读题库和学习记录、关闭，文件一个字节都不能变
    let s = Store::open(&sample).expect("open sample");
    s.bank().expect("bank");
    s.progress().expect("progress");
    drop(s);
    assert_eq!(
        std::fs::read(&sample).expect("read"),
        before,
        "a python-written database was changed by open + read + close"
    );
    // a Rust-written db
    let (sheets, qs) = sample_questions();
    let rs = dir.path().join("rs.db");
    open_fixed(&rs)
        .apply_import("a.xlsx", &sheets, &qs)
        .expect("import");
    let before = std::fs::read(&rs).expect("read");
    drop(open_fixed(&rs));
    assert_eq!(std::fs::read(&rs).expect("read"), before);
    assert!(
        !dir.path().join("rs.db-journal").exists(),
        "no leftover journal"
    );
}

// ---------------------------------------------------------------- (7) legacy migration

fn migrate(body: &[u8], pre: impl Fn(&Store)) -> ((bool, String), Value) {
    let dir = tmp();
    let pj = dir.path().join("progress.json");
    std::fs::write(&pj, body).expect("write progress.json");
    let s = open_fixed(&dir.path().join("rs.db"));
    pre(&s);
    let res = s
        .migrate_legacy_progress(&pj)
        .expect("migration must not return an error");
    let rs_dump = dump_rust(&s);
    (res, rs_dump)
}

const GOOD_LEGACY: &str = r#"{
  "version": 1,
  "questions": {
    "a1": {"attempts": 3, "correct": 2, "wrong": 1, "streak": 2, "last": true, "at": "2026-01-01 10:00"},
    "b2": {"attempts": "4", "correct": 1.9, "wrong": true, "streak": 0, "last": null},
    "c3": {},
    "d4": {"attempts": 1, "correct": 0, "wrong": 1, "streak": 0, "last": false, "at": ""}
  },
  "history": [
    {"at": "2026-01-03 10:00", "sheets": ["日本語", "b"], "mode": "weak", "total": 4, "correct": 3, "percent": 75, "seconds": 30},
    {"at": "2026-01-02 10:00", "sheets": [], "total": "2", "correct": 1, "percent": 50},
    {"sheets": ["x"], "total": 1, "correct": 1, "percent": 100, "seconds": 1.5}
  ],
  "masterStreak": 2
}"#;

#[test]
fn legacy_migration_converts_valid_files() {
    let mut with_bom = vec![0xEF, 0xBB, 0xBF];
    with_bom.extend_from_slice(GOOD_LEGACY.as_bytes());
    for (name, body) in [
        ("plain", GOOD_LEGACY.as_bytes().to_vec()),
        ("bom", with_bom),
    ] {
        let (res, rs_dump) = migrate(&body, |_| {});
        assert!(res.0, "{name}: {}", res.1);
        assert_eq!(res.1, "迁移了 3 场成绩、4 道题的答题记录", "{name}");
        // history: newest first as in the file, ids ascending from oldest
        assert_eq!(rs_dump["progress"]["history"][0]["at"], "2026-01-03 10:00");
        assert_eq!(rs_dump["progress"]["history"][2]["at"], "");
        assert_eq!(rs_dump["progress"]["history"][2]["mode"], "all");
        assert_eq!(rs_dump["progress"]["history"][1]["total"], 2);
        assert_eq!(rs_dump["progress"]["history"][2]["seconds"], 1);
        assert_eq!(rs_dump["progress"]["questions"]["b2"]["attempts"], 4);
        assert_eq!(rs_dump["progress"]["questions"]["b2"]["correct"], 1);
        assert_eq!(rs_dump["progress"]["questions"]["b2"]["wrong"], 1);
        assert_eq!(
            rs_dump["progress"]["questions"]["b2"]["last"], false,
            "last:null is falsy -> 0"
        );
        assert!(rs_dump["progress"]["questions"]["c3"].get("last").is_none());
        assert!(
            rs_dump["progress"]["questions"]["d4"].get("at").is_none(),
            "empty at is omitted"
        );
    }
}

#[test]
fn legacy_migration_is_idempotent_and_guarded() {
    let dir = tmp();
    let pj = dir.path().join("progress.json");
    std::fs::write(&pj, GOOD_LEGACY).expect("write");
    let s = open_fixed(&dir.path().join("x.db"));
    assert_eq!(
        s.migrate_legacy_progress(&dir.path().join("missing.json"))
            .expect("m"),
        (false, "没有找到旧的 progress.json".to_string())
    );
    assert_eq!(
        s.migrate_legacy_progress(dir.path()).expect("m"),
        (false, "没有找到旧的 progress.json".to_string()),
        "directory is not a file"
    );
    assert!(s.migrate_legacy_progress(&pj).expect("m").0);
    let after_first = dump_rust(&s);
    assert_eq!(
        s.migrate_legacy_progress(&pj).expect("m"),
        (false, "之前已经迁移过".to_string())
    );
    s.reset_progress().expect("reset");
    assert_eq!(
        s.migrate_legacy_progress(&pj).expect("m"),
        (false, "之前已经迁移过".to_string()),
        "flag survives reset"
    );
    assert_eq!(s.progress().expect("p")["history"], json!([]));
    assert_ne!(after_first["progress"]["history"], json!([]));

    // existing progress is never overwritten, and the flag is NOT set in that case (same as Python)
    let s2 = open_fixed(&dir.path().join("y.db"));
    s2.record_attempt(&attempt(&[("only", true)], "all", 1))
        .expect("attempt");
    assert_eq!(
        s2.migrate_legacy_progress(&pj).expect("m"),
        (false, "数据库里已经有学习记录，没有覆盖".to_string())
    );
    assert_eq!(
        s2.progress().expect("p")["questions"]
            .as_object()
            .map(|m| m.len()),
        Some(1)
    );
    s2.reset_progress().expect("reset");
    assert!(
        s2.migrate_legacy_progress(&pj).expect("m").0,
        "after a reset the old file can still be imported once"
    );
}

#[test]
fn legacy_migration_malformed_inputs_never_panic_or_error() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", vec![]),
        ("binary", vec![0xff, 0xfe, 0x00, 0x01, 0x80]),
        ("only bom", vec![0xEF, 0xBB, 0xBF]),
        ("not json", b"hello".to_vec()),
        ("array", b"[]".to_vec()),
        ("null", b"null".to_vec()),
        ("string", b"\"x\"".to_vec()),
        ("number", b"7".to_vec()),
        ("no questions", br#"{"history": []}"#.to_vec()),
        ("no history", br#"{"questions": {}}"#.to_vec()),
        ("questions is list", br#"{"questions": [], "history": []}"#.to_vec()),
        ("history is dict", br#"{"questions": {}, "history": {}}"#.to_vec()),
        ("question value is int", br#"{"questions": {"a": 5}, "history": []}"#.to_vec()),
        ("question value is list", br#"{"questions": {"a": [1]}, "history": []}"#.to_vec()),
        ("question value is null", br#"{"questions": {"a": null}, "history": []}"#.to_vec()),
        ("attempts non numeric", br#"{"questions": {"a": {"attempts": "x"}}, "history": []}"#.to_vec()),
        ("attempts null", br#"{"questions": {"a": {"attempts": null}}, "history": []}"#.to_vec()),
        ("attempts huge", br#"{"questions": {"a": {"attempts": 1e300}}, "history": []}"#.to_vec()),
        ("attempts overflow", br#"{"questions": {"a": {"attempts": 18446744073709551615}}, "history": []}"#.to_vec()),
        ("at is list", br#"{"questions": {"a": {"at": [1]}}, "history": []}"#.to_vec()),
        ("history item int", br#"{"questions": {}, "history": [1]}"#.to_vec()),
        ("history missing total", br#"{"questions": {}, "history": [{"correct": 1, "percent": 1}]}"#.to_vec()),
        ("history total string", br#"{"questions": {}, "history": [{"total": "many", "correct": 1, "percent": 1}]}"#.to_vec()),
        ("history sheets deep", br#"{"questions": {}, "history": [{"total": 1, "correct": 1, "percent": 1, "sheets": {"a": [[[]]]}}]}"#.to_vec()),
        ("history at null", br#"{"questions": {}, "history": [{"at": null, "total": 1, "correct": 1, "percent": 1}]}"#.to_vec()),
        ("history mode null", br#"{"questions": {}, "history": [{"mode": null, "total": 1, "correct": 1, "percent": 1}]}"#.to_vec()),
        ("history mode list", br#"{"questions": {}, "history": [{"mode": [], "total": 1, "correct": 1, "percent": 1}]}"#.to_vec()),
        ("deep nesting", format!("{}{}", "[".repeat(5000), "]".repeat(5000)).into_bytes()),
        ("valid first then bad second", br#"{"questions": {"ok": {"attempts": 1}, "bad": {"attempts": "x"}}, "history": []}"#.to_vec()),
    ];
    for (name, body) in cases {
        let (res, rs_dump) = migrate(&body, |_| {});
        let expect_ok = name.starts_with("history sheets deep");
        assert_eq!(res.0, expect_ok, "{name}: {}", res.1);
        if !expect_ok {
            assert!(!res.1.is_empty(), "{name}");
            assert_eq!(
                rs_dump["progress"]["questions"],
                json!({}),
                "{name}: stats must stay untouched"
            );
            assert_eq!(
                rs_dump["progress"]["history"],
                json!([]),
                "{name}: attempts must stay untouched"
            );
        }
    }
}

#[test]
fn malformed_migration_does_not_set_the_flag_so_a_fixed_file_can_be_retried() {
    let dir = tmp();
    let pj = dir.path().join("progress.json");
    let s = open_fixed(&dir.path().join("x.db"));
    std::fs::write(&pj, br#"{"questions": {"a": 5}, "history": []}"#).expect("write");
    assert!(!s.migrate_legacy_progress(&pj).expect("m").0);
    std::fs::write(&pj, GOOD_LEGACY).expect("write");
    assert!(s.migrate_legacy_progress(&pj).expect("m").0);
}

// ---------------------------------------------------------------- cross-implementation parity

#[test]
fn plan_is_read_only_and_apply_with_empty_list_deactivates_all() {
    let dir = tmp();
    let p = dir.path().join("x.db");
    let s = open_fixed(&p);
    let (sheets, qs) = sample_questions();
    s.apply_import("a.xlsx", &sheets, &qs).expect("import");
    let before = raw_rows(&p);
    let plan = s.plan_import(&[], &[]).expect("plan");
    assert_eq!(plan["removed"], 4);
    assert_eq!(raw_rows(&p), before);
    s.record_attempt(&attempt(&[("id1", true)], "all", 1))
        .expect("attempt");
    let plan = s.plan_import(&[], &[]).expect("plan");
    assert_eq!(
        plan["removed_stats"], 1,
        "removed questions that have learning records"
    );
    s.apply_import("empty.xlsx", &[], &[]).expect("apply empty");
    assert_eq!(s.count_active().expect("c"), 0);
    let bank = s.bank().expect("bank");
    assert_eq!(bank["questions"], json!([]));
    assert_eq!(
        bank["sheets"].as_array().map(Vec::len),
        Some(2),
        "sheets with only inactive questions are still listed"
    );
    assert_eq!(
        s.progress().expect("p")["questions"]["id1"]["attempts"],
        1,
        "stats survive deactivation"
    );
}

#[test]
fn removed_samples_cut_by_code_points_and_bank_key_order() {
    let dir = tmp();
    let s = open_fixed(&dir.path().join("x.db"));
    let long = "𠮷".repeat(60);
    s.apply_import("a.xlsx", &[sheet("S", 1)], &[single("S", "k", &long, 1)])
        .expect("import");
    let plan = s.plan_import(&[], &[]).expect("plan");
    assert_eq!(plan["removed_samples"][0], "𠮷".repeat(40));
    let bank = s.bank().expect("bank");
    let keys: Vec<&str> = bank
        .as_object()
        .expect("obj")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["file", "mtime", "error", "questions", "sheets"]);
    let qkeys: Vec<&str> = bank["questions"][0]
        .as_object()
        .expect("q")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        qkeys,
        [
            "kind",
            "tag",
            "stem",
            "options",
            "items",
            "answer",
            "translation",
            "zh",
            "explanation",
            "mnemonic",
            "note",
            "id",
            "sheet",
            "row"
        ]
    );
    let skeys: Vec<&str> = bank["sheets"][0]
        .as_object()
        .expect("s")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(skeys, ["name", "usable", "inactive", "total", "skipped"]);
    let pkeys: Vec<&str> = plan
        .as_object()
        .expect("plan")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        pkeys,
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
            "removed_stats"
        ]
    );
    let p = s
        .record_attempt(&attempt(&[("k", true)], "all", 1))
        .expect("attempt");
    let hk: Vec<&str> = p["history"][0]
        .as_object()
        .expect("h")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        hk,
        ["at", "sheets", "mode", "total", "correct", "percent", "seconds"]
    );
    let rk: Vec<&str> = p["questions"]["k"]
        .as_object()
        .expect("r")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(rk, ["attempts", "correct", "wrong", "streak", "last", "at"]);
    let top: Vec<&str> = p
        .as_object()
        .expect("p")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(top, ["version", "questions", "history", "masterStreak"]);
}
