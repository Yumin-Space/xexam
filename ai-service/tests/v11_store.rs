//! v1.1 storage: AI database schema 5 (fresh and upgraded files, the CHECKs, the keys, the plans of the new queries).
use rusqlite::Connection;
use std::path::Path;
use xexam_ai_service::config::JournalMode;
use xexam_ai_service::store::{schema, Store, DB_FILE};

fn master(path: &Path) -> Vec<(String, String, Option<String>)> {
    let c = Connection::open(path).expect("open");
    let mut st = c
        .prepare("SELECT type, name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name")
        .expect("prepare");
    let rows = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query");
    rows.collect::<Result<_, _>>().expect("rows")
}

fn open(dir: &Path) -> Store {
    Store::open(dir, JournalMode::Delete).expect("open store")
}

/// A schema-4 file: the tables of version 1 plus the columns of 2, 3 and 4, a user and a task row.
fn write_v4(dir: &Path) {
    let c = Connection::open(dir.join(DB_FILE)).expect("fixture");
    c.execute_batch(schema::SCHEMA).expect("v1");
    for (table, _, def) in schema::V2_COLUMNS
        .iter()
        .chain(schema::V3_COLUMNS.iter())
        .chain(schema::V4_COLUMNS.iter())
    {
        c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {def};"))
            .expect("column");
    }
    c.execute_batch(schema::V3_INDEXES).expect("indexes");
    c.execute("INSERT INTO settings (id, updated_at) VALUES (1, 1)", [])
        .expect("settings");
    c.execute("INSERT INTO limits (id, updated_at) VALUES (1, 1)", [])
        .expect("limits");
    c.execute(
        "INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at) VALUES (7, 'x', 'kept', 1, 1)",
        [],
    )
    .expect("user");
    c.execute("INSERT INTO meta (key, value) VALUES ('schema', '4')", [])
        .expect("meta");
}

#[test]
fn an_upgraded_schema_4_file_equals_a_fresh_schema_5_file() {
    let fresh = tempfile::tempdir().expect("dir");
    drop(open(fresh.path()));
    let old = tempfile::tempdir().expect("dir");
    write_v4(old.path());
    let store = open(old.path());
    // the backup of the old file was taken first
    let copies: Vec<_> = std::fs::read_dir(old.path().join("backups"))
        .expect("backups")
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(copies.len(), 1);
    let name = copies[0].file_name().to_string_lossy().into_owned();
    assert!(
        name.starts_with("xexam-ai-schema4-") && name.ends_with(".db"),
        "{name}"
    );
    let copy = Connection::open(copies[0].path()).expect("copy");
    let v: String = copy
        .query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
            r.get(0)
        })
        .expect("v");
    assert_eq!(v, "4");
    drop(copy);
    // the same tables, columns and indexes either way
    drop(store);
    assert_eq!(
        master(&fresh.path().join(DB_FILE)),
        master(&old.path().join(DB_FILE))
    );
    // the user row of the old file survived with the default features
    let c = Connection::open(old.path().join(DB_FILE)).expect("open");
    let (name, off): (String, i64) = c
        .query_row(
            "SELECT username, features_off FROM users WHERE id = 7",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("user");
    assert_eq!((name.as_str(), off), ("kept", 0));
    let limits: (i64, i64, i64, i64) = c
        .query_row(
            "SELECT gen_per_day, gen_running, import_max_mb, import_max_questions FROM limits WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("limits");
    assert_eq!(limits, (3, 2, 20, 50));
}

#[test]
fn the_checks_and_keys_of_the_new_columns() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    let c = Connection::open(dir.path().join(DB_FILE)).expect("open");
    c.pragma_update(None, "foreign_keys", "ON").expect("fk");
    // limits ranges equal limits::DEFS
    for d in xexam_ai_service::limits::DEFS
        .iter()
        .filter(|d| d.column.starts_with("gen_") || d.column.starts_with("import_"))
    {
        let below = d.min - 1;
        let above = d.max + 1;
        for bad in [below, above] {
            let r = c.execute(
                &format!("UPDATE limits SET {} = ?1 WHERE id = 1", d.column),
                [bad],
            );
            assert!(r.is_err(), "{} = {bad} must be refused", d.column);
        }
        c.execute(
            &format!("UPDATE limits SET {} = ?1 WHERE id = 1", d.column),
            [d.default],
        )
        .expect("default");
    }
    // features_off: any bits are accepted (checked in code), a negative number is not
    c.execute("INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at, features_off) VALUES (1, 'x', 'a', 1, 1, 63)", [])
        .expect("63");
    c.execute("UPDATE users SET features_off = 64 WHERE id = 1", [])
        .expect("64 is a bit the code checks, not the table");
    assert!(c
        .execute("UPDATE users SET features_off = -1 WHERE id = 1", [])
        .is_err());
    // one active job per user is a database rule, finished ones do not count
    let ins = |key: &str| {
        c.execute(
            "INSERT INTO tasks (user_id, kind, state, payload, attempts, run_after, created_at, updated_at, job_key)
             VALUES (1, 'bank_gen', 'queued', '{}', 0, 1, 1, 1, ?1)",
            [key],
        )
    };
    ins("k1").expect("first");
    assert!(ins("k2").is_err(), "a second active job of the same user");
    c.execute("UPDATE tasks SET state = 'done'", [])
        .expect("done");
    ins("k3").expect("after the first ended");
    assert!(ins("k3").is_err(), "a job key is unique");
    // gen_calls survive the user and the task (set null), and the purpose is free text checked in code
    c.execute(
        "INSERT INTO gen_calls (task_id, user_id, purpose, at, day, provider, latency_ms, status) VALUES (1, 1, 'recognise', 5, '2026-11-12', 'openai', 9, 'ok')",
        [],
    )
    .expect("call");
    c.execute("DELETE FROM tasks WHERE id = 1", [])
        .expect("task");
    c.execute("DELETE FROM users WHERE id = 1", [])
        .expect("user");
    let (task, user): (Option<i64>, Option<i64>) = c
        .query_row("SELECT task_id, user_id FROM gen_calls", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("row");
    assert_eq!((task, user), (None, None));
    assert!(c
        .execute(
            "INSERT INTO gen_calls (at, day, provider, latency_ms, status) VALUES (5, 'd', 'p', 9, 'weird')",
            [],
        )
        .is_err());
}

#[test]
fn the_plans_of_the_new_queries_use_their_indexes() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let plan = |sql: &str| -> String {
        store
            .read(|c| {
                let mut st = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
                let rows = st.query_map([], |r| r.get::<_, String>(3))?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?.join(" | "))
            })
            .expect("plan")
    };
    // the call caps' union: the recognising rows of one user
    let p = plan("SELECT at - latency_ms / 1000 FROM gen_calls WHERE user_id = 1 AND purpose = 'recognise' AND day >= '2026-11-11' AND at - latency_ms / 1000 >= 5");
    assert!(p.contains("idx_gen_calls_user"), "{p}");
    // the quota
    let p = plan("SELECT COUNT(*) FROM tasks WHERE user_id = 1 AND kind = 'bank_gen' AND charged = 1 AND charged_at >= 5");
    assert!(p.contains("idx_tasks_charged"), "{p}");
    // the busy rule and the job by key
    let p = plan("SELECT id FROM tasks WHERE user_id = 1 AND kind = 'bank_gen' AND state IN ('queued', 'running')");
    assert!(
        p.contains("idx_tasks_active") || p.contains("idx_tasks_user"),
        "{p}"
    );
    let p = plan("SELECT id FROM tasks WHERE job_key = 'abc'");
    assert!(p.contains("idx_tasks_job"), "{p}");
    // the statistics of a period and a job's calls
    let p = plan("SELECT COUNT(*) FROM gen_calls WHERE day BETWEEN '2026-11-01' AND '2026-11-30'");
    assert!(p.contains("idx_gen_calls_day"), "{p}");
    let p = plan("SELECT SUM(tokens_in) FROM gen_calls WHERE task_id = 3");
    assert!(p.contains("idx_gen_calls_task"), "{p}");
}

#[test]
fn the_retention_sweep_removes_old_finished_tasks_only() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    store
        .write(|tx| {
            tx.execute("INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at) VALUES (1, 'x', 'a', 1, 1)", [])?;
            for (i, (state, at)) in [("done", 100), ("failed", 100), ("done", 9_000_000), ("queued", 100)].iter().enumerate() {
                tx.execute(
                    "INSERT INTO tasks (user_id, kind, state, payload, attempts, run_after, created_at, updated_at, job_key)
                     VALUES (1, 'bank_gen', ?1, '{}', 0, 1, 1, ?2, ?3)",
                    rusqlite::params![state, at, format!("k{i}")],
                )?;
            }
            Ok::<_, rusqlite::Error>(())
        })
        .expect("rows");
    // a queued task would trip the unique index with another queued one, so the fixture has exactly one
    let n = store
        .write(|tx| xexam_ai_service::store::stats::sweep_tasks(tx, 1_000_000))
        .expect("sweep");
    assert_eq!(n, 2);
    let left: i64 = store
        .read(|c| c.query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0)))
        .expect("count");
    assert_eq!(left, 2);
}

#[test]
fn the_unlimited_switches_default_on_and_an_older_schema_6_file_is_moved_once_after_a_backup() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    let read = |p: &std::path::Path| -> (i64, i64) {
        Connection::open(p.join(DB_FILE))
            .expect("open")
            .query_row(
                "SELECT gen_unlimited, import_unlimited FROM limits WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("limits")
    };
    // a fresh file: on, and no backup was needed
    assert_eq!(read(dir.path()), (1, 1));
    assert!(!dir.path().join("backups").exists());
    // a schema-6 file from before the new default (switches stored 0, no marker)
    let c = Connection::open(dir.path().join(DB_FILE)).expect("open");
    c.execute(
        "UPDATE limits SET gen_unlimited = 0, import_unlimited = 0",
        [],
    )
    .expect("old values");
    c.execute("DELETE FROM meta WHERE key = 'unlimited_default'", [])
        .expect("marker");
    drop(c);
    drop(open(dir.path()));
    assert_eq!(read(dir.path()), (1, 1));
    let copies = std::fs::read_dir(dir.path().join("backups"))
        .expect("backups")
        .count();
    assert_eq!(copies, 1, "the backup of the old file");
    // an administrator's later choice stays across restarts
    let c = Connection::open(dir.path().join(DB_FILE)).expect("open");
    c.execute("UPDATE limits SET gen_unlimited = 0", [])
        .expect("admin choice");
    drop(c);
    drop(open(dir.path()));
    assert_eq!(read(dir.path()).0, 0);
}

#[test]
fn a_stored_import_limit_above_fifty_is_lowered_once_after_a_backup() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    let read = |p: &std::path::Path| -> i64 {
        Connection::open(p.join(DB_FILE))
            .expect("open")
            .query_row(
                "SELECT import_max_questions FROM limits WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .expect("limit")
    };
    // a fresh file: the default 50 and no backup
    assert_eq!(read(dir.path()), 50);
    assert!(!dir.path().join("backups").exists());
    // a file written while the ceiling was 100 (the old CHECK allowed it; no marker)
    let c = Connection::open(dir.path().join(DB_FILE)).expect("open");
    c.execute_batch(
        "PRAGMA ignore_check_constraints = ON; UPDATE limits SET import_max_questions = 100;",
    )
    .expect("old value");
    c.execute("DELETE FROM meta WHERE key = 'import_max_50'", [])
        .expect("marker");
    drop(c);
    drop(open(dir.path()));
    assert_eq!(read(dir.path()), 50);
    let copies = std::fs::read_dir(dir.path().join("backups"))
        .expect("backups")
        .count();
    assert_eq!(copies, 1, "the backup of the old file");
    // a value an administrator set later (below the ceiling) stays across restarts
    let c = Connection::open(dir.path().join(DB_FILE)).expect("open");
    c.execute("UPDATE limits SET import_max_questions = 30", [])
        .expect("admin choice");
    drop(c);
    drop(open(dir.path()));
    assert_eq!(read(dir.path()), 30);
}
