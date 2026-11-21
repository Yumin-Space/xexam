//! v2.0 AI database clean-up (schema 10; since v2.1 the file is schema 11): the retired `secrets` table and the retired `settings` / `limits` / `chats`
//! columns are dropped by every path; nothing else changes. Fresh and upgraded files are the same, a failed step changes
//! nothing, the old key bytes leave the live files. All content is made up.
mod common;

use common::*;
use rusqlite::Connection;
use serde_json::json;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use xexam_ai_service::config::JournalMode;
use xexam_ai_service::limits::DEFS;
use xexam_ai_service::store::{schema, tables, write_schema9_fixture, Store, DB_FILE};

/// Made-up ciphertext bytes of the fixture key; they must not survive in the live files.
const MARKER: &[u8] = b"MARKER-CIPHERTEXT-0123456789-abcdef-XYZ";

fn open(dir: &Path) -> Store {
    Store::open(dir, JournalMode::Delete).expect("open store")
}

fn raw(path: &Path) -> Connection {
    Connection::open(path).expect("open file")
}

fn schema_of(c: &Connection) -> String {
    c.query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
        r.get(0)
    })
    .expect("schema")
}

fn columns(c: &Connection, table: &str) -> Vec<String> {
    let mut st = c
        .prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
        .expect("prepare");
    let rows = st
        .query_map([table], |r| r.get::<_, String>(0))
        .expect("query");
    rows.collect::<Result<_, _>>().expect("rows")
}

fn table_names(c: &Connection) -> Vec<String> {
    let mut st = c
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .expect("prepare");
    let rows = st.query_map([], |r| r.get::<_, String>(0)).expect("query");
    rows.collect::<Result<_, _>>().expect("rows")
}

type MasterRow = (String, String, String, Option<String>);

fn master(c: &Connection) -> Vec<MasterRow> {
    let mut st = c
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name")
        .expect("prepare");
    let rows = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .expect("query");
    rows.collect::<Result<_, _>>().expect("rows")
}

fn backups(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir.join("backups"))
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    out.sort();
    out
}

fn fresh_master() -> Vec<MasterRow> {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    master(&raw(&dir.path().join(DB_FILE)))
}

fn count(c: &Connection, table: &str) -> i64 {
    c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .expect("count")
}

fn execute_all(c: &Connection, steps: &[&str]) {
    for s in steps {
        c.execute_batch(s).expect("step");
    }
}

fn add_cols(c: &Connection, cols: &[(&str, &str, &str)]) {
    for (table, _, def) in cols {
        c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {def};"))
            .expect("column");
    }
}

/// A schema-`n` file (1 ..= 9) made from the public history texts in the order of the upgrade chain, with a user, a chat
/// with two messages, a memory, old connection settings, a made-up key row and non-default limits.
fn history_file(path: &Path, n: u32) {
    if n == 9 {
        write_schema9_fixture(path).expect("v9 fixture");
    } else {
        let c = raw(path);
        c.execute_batch(schema::SCHEMA).expect("v1");
        if n >= 2 {
            add_cols(&c, &schema::V2_COLUMNS);
        }
        if n >= 3 {
            add_cols(&c, &schema::V3_COLUMNS);
            execute_all(&c, &[schema::V3_INDEXES]);
        }
        if n >= 4 {
            add_cols(&c, &schema::V4_COLUMNS);
        }
        if n >= 5 {
            add_cols(&c, &schema::V5_COLUMNS);
            execute_all(&c, &[schema::V5_TABLES]);
        }
        if n >= 6 {
            add_cols(&c, &schema::V6_COLUMNS);
        }
        if n >= 7 {
            add_cols(&c, &schema::V7_COLUMNS);
            add_cols(&c, &schema::LATE_COLUMNS);
            execute_all(&c, &[schema::V7_TABLES]);
            add_cols(&c, &schema::V16_COLUMNS);
            execute_all(&c, &[schema::V7_INDEXES]);
            c.execute(schema::V7_SEED, [10]).expect("seed");
        } else if n >= 5 {
            c.execute_batch("ALTER TABLE gen_calls ADD COLUMN tokens_cache_write INTEGER;")
                .expect("late");
        }
        if n >= 8 {
            execute_all(&c, &[schema::V8_TABLES]);
        }
        c.execute("INSERT INTO settings (id, updated_at) VALUES (1, 5)", [])
            .expect("settings");
        c.execute("INSERT INTO limits (id, updated_at) VALUES (1, 5)", [])
            .expect("limits");
        c.execute(
            "INSERT INTO meta (key, value) VALUES ('schema', ?1)",
            [n.to_string()],
        )
        .expect("meta");
        if n >= 6 {
            c.execute_batch(
                "INSERT INTO meta (key, value) VALUES ('unlimited_default', '1'), ('import_max_50', '1');",
            )
            .expect("flags");
        }
    }
    let c = raw(path);
    // the rows every schema-n file of the test carries
    c.execute_batch(
        "INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at) VALUES (5, 'aa', 'sam', 1, 1);
         INSERT INTO chats (id, user_id, title, created_at, updated_at) VALUES (1, 5, 'An old chat', 10, 10);
         INSERT INTO messages (chat_id, role, content, created_at, finished_at) VALUES
           (1, 'user', 'an old question', 10, 10), (1, 'assistant', 'an old answer', 11, 11);
         INSERT INTO memories (user_id, content, created_at, updated_at) VALUES (5, 'likes short answers', 10, 10);
         INSERT INTO usage (user_id, at, day, kind, provider, model, latency_ms, status)
           VALUES (5, 100, '2026-11-01', 'chat', 'gateway', 'demo-chat', 5, 'ok');
         UPDATE settings SET provider = 'gateway', base_url = 'https://gw.example.com/v1/chat', format = 'openai',
                gateway_auth = 'bearer', model = 'demo-chat', manual_model = 0, usage_option = 1, max_tokens = 4096,
                enabled = 1, verified_at = 500, verified_fp = 'fp-demo', retention_days = 45, updated_at = 400,
                updated_by = 'root';
         UPDATE limits SET per_user_running = 2, global_running = 9, chats_per_user = 777, messages_per_chat = 123,
                memories_per_user = 12, memory_chars = 222, message_chars = 4321, updated_by = 'root';",
    )
    .expect("rows");
    c.execute(
        "INSERT INTO secrets (name, key_id, nonce, ciphertext, hint, rev, updated_at)
         VALUES ('api_key', 'k1', zeroblob(24), ?1, '...demo', 3, 300)",
        [MARKER],
    )
    .expect("secret");
    if n >= 6 {
        c.execute(
            "INSERT OR IGNORE INTO meta (key, value) VALUES ('secret_rev', '3')",
            [],
        )
        .expect("rev");
    }
    if n >= 7 {
        c.execute_batch("UPDATE settings SET features_off = 9 WHERE id = 1")
            .expect("features");
    }
}

#[test]
fn a_fresh_database_is_schema_11_without_the_retired_items() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    let c = raw(&dir.path().join(DB_FILE));
    assert_eq!(schema::SCHEMA_VERSION, "11");
    assert_eq!(schema_of(&c), "11");
    assert!(!table_names(&c).contains(&"secrets".to_string()));
    assert_eq!(
        columns(&c, "settings"),
        [
            "id",
            "enabled",
            "retention_days",
            "updated_at",
            "updated_by",
            "features_off"
        ]
    );
    let want: BTreeSet<String> = DEFS
        .iter()
        .map(|d| d.column.to_string())
        .chain(["id", "updated_at", "updated_by"].map(String::from))
        .collect();
    let got: BTreeSet<String> = columns(&c, "limits").into_iter().collect();
    assert_eq!(got, want);
    assert!(!columns(&c, "chats").contains(&"free".to_string()));
    assert!(backups(dir.path()).is_empty());
    let fk = c
        .prepare("PRAGMA foreign_key_check")
        .expect("fk")
        .query_map([], |_| Ok(()))
        .expect("fk rows")
        .count();
    assert_eq!(fk, 0);
}

/// Opens an upgrade from schema `n` (already written) and checks everything the contract promises.
fn check_upgrade(dir: &Path, n: u32, fresh: &[MasterRow]) {
    let before = dir.join("before.db");
    // an independent copy of the file as it was, to compare row counts and values (not the backup the code makes)
    std::fs::copy(dir.join(DB_FILE), &before).expect("copy");
    let store = open(dir);
    drop(store);
    let live = raw(&dir.join(DB_FILE));
    let old = raw(&before);

    // exactly one backup, named after the old schema, still holding the retired items
    let copies = backups(dir);
    assert_eq!(copies.len(), 1, "n = {n}: {copies:?}");
    let name = copies[0]
        .file_name()
        .and_then(|x| x.to_str())
        .expect("name");
    assert!(
        name.starts_with(&format!("xexam-ai-schema{n}-")) && name.ends_with(".db"),
        "{name}"
    );
    let copy = raw(&copies[0]);
    assert_eq!(schema_of(&copy), n.to_string());
    assert_eq!(count(&copy, "secrets"), 1);
    assert!(columns(&copy, "settings").contains(&"verified_fp".to_string()));
    assert!(columns(&copy, "limits").contains(&"message_chars".to_string()));
    drop(copy);

    // the live file: schema 10, the same objects as a new file, nothing retired left
    assert_eq!(schema_of(&live), "11");
    assert_eq!(
        master(&live),
        fresh,
        "n = {n}: sqlite_master differs from a fresh file"
    );
    assert!(!table_names(&live).contains(&"secrets".to_string()));
    for (table, column) in schema::RETIRED_COLUMNS {
        assert!(
            !columns(&live, table).contains(&column.to_string()),
            "{table}.{column}"
        );
    }
    let ok: String = live
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .expect("integrity");
    assert_eq!(ok, "ok");
    let fk: i64 = live
        .prepare("PRAGMA foreign_key_check")
        .expect("fk")
        .query_map([], |_| Ok(()))
        .expect("fk rows")
        .count() as i64;
    assert_eq!(fk, 0);

    // every table that existed before keeps its rows (model_slots is rebuilt for schemas 7 / 8, meta holds the stamp)
    for t in table_names(&old) {
        if matches!(
            t.as_str(),
            "secrets" | "model_slots" | "meta" | "sqlite_sequence"
        ) {
            continue;
        }
        assert_eq!(count(&live, &t), count(&old, &t), "n = {n}: rows of {t}");
    }

    // settings: switch, retention and author kept; the global feature switches kept
    let s: (i64, i64, i64, String) = live
        .query_row(
            "SELECT enabled, retention_days, updated_at, updated_by FROM settings WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("settings");
    assert_eq!(s, (1, 45, 400, "root".to_string()), "n = {n}");
    let off: i64 = live
        .query_row("SELECT features_off FROM settings WHERE id = 1", [], |r| {
            r.get(0)
        })
        .expect("features_off");
    assert_eq!(off, if n >= 7 { 9 } else { 0 }, "n = {n}");

    // limits: every value still there keeps its value (the old schemas move the memory ones by their documented rules)
    let live_limits = tables::load_limits(&live).expect("limits");
    let old_cols: BTreeSet<String> = columns(&old, "limits").into_iter().collect();
    for d in DEFS.iter().filter(|d| old_cols.contains(d.column)) {
        if n < 7 && matches!(d.column, "memories_per_user" | "memory_tokens") {
            continue;
        }
        let was: i64 = old
            .query_row(
                &format!("SELECT {} FROM limits WHERE id = 1", d.column),
                [],
                |r| r.get(0),
            )
            .expect("old value");
        // the 10 -> 11 step moves the old pace defaults of a schema-9 file (15 / 250000 -> 13 / 230000), others stay
        let want = match (d.column, was) {
            ("pace_rpm", 15) => 13,
            ("pace_tpm", 250_000) => 230_000,
            (_, v) => v,
        };
        assert_eq!(live_limits.get(d.key), want, "n = {n}: {}", d.key);
    }
    let by: String = live
        .query_row("SELECT updated_by FROM limits WHERE id = 1", [], |r| {
            r.get(0)
        })
        .expect("by");
    assert_eq!(by, "root");

    // the chat survived in full
    let msgs: i64 = live
        .query_row("SELECT COUNT(*) FROM messages WHERE chat_id = 1", [], |r| {
            r.get(0)
        })
        .expect("messages");
    assert_eq!(msgs, 2);

    // a second open writes nothing and makes no second copy
    let bytes = std::fs::read(dir.join(DB_FILE)).expect("read");
    drop(live);
    drop(old);
    drop(open(dir));
    assert!(
        bytes == std::fs::read(dir.join(DB_FILE)).expect("read"),
        "n = {n}"
    );
    assert_eq!(backups(dir).len(), 1, "n = {n}");
}

#[test]
fn every_schema_1_to_9_upgrades_to_10() {
    let fresh = fresh_master();
    for n in 1..=9u32 {
        let dir = tempfile::tempdir().expect("dir");
        history_file(&dir.path().join(DB_FILE), n);
        check_upgrade(dir.path(), n, &fresh);
    }
}

#[test]
fn the_pre_release_schema_2_file_with_chats_free_upgrades_too() {
    let fresh = fresh_master();
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join(DB_FILE);
    {
        let c = raw(&path);
        c.execute_batch(schema::SCHEMA).expect("v1");
        // the earlier build: users.function_enabled and a per-chat chats.free, but no users.free_mode
        c.execute_batch(
            "ALTER TABLE users ADD COLUMN function_enabled INTEGER NOT NULL DEFAULT 1;
             ALTER TABLE chats ADD COLUMN free INTEGER NOT NULL DEFAULT 0;
             INSERT INTO settings (id, updated_at) VALUES (1, 5);
             INSERT INTO limits (id, updated_at) VALUES (1, 5);
             INSERT INTO meta (key, value) VALUES ('schema', '2');",
        )
        .expect("fixture");
    }
    drop(open(dir.path()));
    let c = raw(&path);
    assert_eq!(schema_of(&c), "11");
    assert!(!columns(&c, "chats").contains(&"free".to_string()));
    assert_eq!(master(&c), fresh);
    assert_eq!(backups(dir.path()).len(), 1);
}

#[test]
fn old_connection_settings_and_key_move_to_connection_1_before_they_are_dropped() {
    for n in [1u32, 6] {
        let dir = tempfile::tempdir().expect("dir");
        history_file(&dir.path().join(DB_FILE), n);
        drop(open(dir.path()));
        let c = raw(&dir.path().join(DB_FILE));
        let (name, provider, url): (String, String, String) = c
            .query_row(
                "SELECT name, provider, base_url FROM connections WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("connection 1");
        assert_eq!(
            (name.as_str(), provider.as_str(), url.as_str()),
            (
                "gw.example.com",
                "gateway",
                "https://gw.example.com/v1/chat"
            ),
            "n = {n}"
        );
        let (aad, rev, cipher): (i64, i64, Vec<u8>) = c
            .query_row(
                "SELECT aad_ver, rev, ciphertext FROM connection_secrets WHERE connection_id = 1 AND name = 'api_key'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("key");
        assert_eq!((aad, rev, cipher.as_slice()), (1, 3, MARKER), "n = {n}");
        let model: String = c
            .query_row(
                "SELECT m.model FROM models m JOIN model_slots s ON s.model_id = m.id WHERE s.slot = 'default'",
                [],
                |r| r.get(0),
            )
            .expect("default model");
        assert_eq!(model, "demo-chat");
        assert!(!table_names(&c).contains(&"secrets".to_string()));
    }
}

#[test]
fn reopening_a_fresh_database_writes_nothing() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    let a = std::fs::read(dir.path().join(DB_FILE)).expect("read");
    drop(open(dir.path()));
    assert!(a == std::fs::read(dir.path().join(DB_FILE)).expect("read"));
    assert!(backups(dir.path()).is_empty());
}

#[test]
fn the_repair_re_adds_a_missing_live_column_and_never_a_retired_one() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    let path = dir.path().join(DB_FILE);
    raw(&path)
        .execute_batch("ALTER TABLE limits DROP COLUMN recognize_max_calls;")
        .expect("drop");
    drop(open(dir.path()));
    let c = raw(&path);
    assert!(columns(&c, "limits").contains(&"recognize_max_calls".to_string()));
    for (table, column) in schema::RETIRED_COLUMNS {
        assert!(
            !columns(&c, table).contains(&column.to_string()),
            "{table}.{column}"
        );
    }
    assert_eq!(schema_of(&c), "11");
    assert!(backups(dir.path()).is_empty(), "the repair makes no copy");
    drop(c);
    let bytes = std::fs::read(&path).expect("read");
    drop(open(dir.path()));
    assert!(
        bytes == std::fs::read(&path).expect("read"),
        "the next open writes nothing"
    );
}

#[test]
fn a_failed_step_changes_nothing_and_the_next_start_upgrades() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join(DB_FILE);
    write_schema9_fixture(&path).expect("fixture");
    {
        let c = raw(&path);
        c.execute(
            "INSERT INTO secrets (name, key_id, nonce, ciphertext, hint, rev, updated_at)
             VALUES ('api_key', 'k1', zeroblob(24), ?1, '...demo', 3, 300)",
            [MARKER],
        )
        .expect("secret");
        // a trigger that names a retired column makes its DROP COLUMN fail
        c.execute_batch("CREATE TRIGGER t AFTER UPDATE ON settings BEGIN SELECT NEW.model; END;")
            .expect("trigger");
    }
    let before_master = master(&raw(&path));
    let err = match Store::open(dir.path(), JournalMode::Delete) {
        Ok(_) => panic!("the upgrade must fail"),
        Err(e) => e,
    };
    assert!(err.starts_with("cannot upgrade the AI database"), "{err}");
    let c = raw(&path);
    assert_eq!(schema_of(&c), "9");
    assert_eq!(master(&c), before_master);
    assert_eq!(count(&c, "secrets"), 1);
    assert!(columns(&c, "settings").contains(&"model".to_string()));
    drop(c);
    assert_eq!(backups(dir.path()).len(), 1, "the copy was made first");
    raw(&path)
        .execute_batch("DROP TRIGGER t;")
        .expect("drop trigger");
    drop(open(dir.path()));
    let c = raw(&path);
    assert_eq!(schema_of(&c), "11");
    assert!(!table_names(&c).contains(&"secrets".to_string()));
}

#[test]
fn a_schema_9_file_without_import_max_50_upgrades() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join(DB_FILE);
    write_schema9_fixture(&path).expect("fixture");
    raw(&path)
        .execute("DELETE FROM meta WHERE key = 'import_max_50'", [])
        .expect("delete");
    drop(open(dir.path()));
    let c = raw(&path);
    assert_eq!(schema_of(&c), "11");
    let flag: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM meta WHERE key = 'import_max_50'",
            [],
            |r| r.get(0),
        )
        .expect("flag");
    assert_eq!(flag, 0, "the retired flag is not written any more");
    assert_eq!(backups(dir.path()).len(), 1);
}

#[test]
fn a_schema_9_file_without_the_unlimited_flag_takes_the_one_time_path() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join(DB_FILE);
    write_schema9_fixture(&path).expect("fixture");
    raw(&path)
        .execute_batch(
            "DELETE FROM meta WHERE key = 'unlimited_default';
             UPDATE limits SET gen_unlimited = 0, import_unlimited = 0;",
        )
        .expect("old state");
    drop(open(dir.path()));
    let c = raw(&path);
    let (g, i): (i64, i64) = c
        .query_row(
            "SELECT gen_unlimited, import_unlimited FROM limits WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("limits");
    assert_eq!((g, i), (1, 1));
    assert_eq!(schema_of(&c), "11");
    // the upgrade copy and the one of the one-time step
    assert_eq!(backups(dir.path()).len(), 2);
}

#[test]
fn a_newer_schema_is_refused_and_the_message_names_schema_11() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    let path = dir.path().join(DB_FILE);
    raw(&path)
        .execute("UPDATE meta SET value = '12' WHERE key = 'schema'", [])
        .expect("set");
    let err = match Store::open(dir.path(), JournalMode::Delete) {
        Ok(_) => panic!("schema 12 must be refused"),
        Err(e) => e,
    };
    assert!(err.contains("has schema 12"), "{err}");
    assert!(err.contains("(schema 11)"), "{err}");
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn the_old_key_bytes_leave_the_live_files_in_wal_mode() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join(DB_FILE);
    write_schema9_fixture(&path).expect("fixture");
    {
        let c = raw(&path);
        c.execute(
            "INSERT INTO secrets (name, key_id, nonce, ciphertext, hint, rev, updated_at)
             VALUES ('api_key', 'k1', zeroblob(24), ?1, '...demo', 3, 300)",
            [MARKER],
        )
        .expect("secret");
        // switch the fixture to WAL like a running service (the mode is stored in the file)
        c.query_row("PRAGMA journal_mode = wal", [], |_| Ok(()))
            .expect("wal");
        c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .expect("checkpoint");
    }
    assert!(
        contains(&std::fs::read(&path).expect("read"), MARKER),
        "fixture holds the marker"
    );
    let store = Store::open(dir.path(), JournalMode::Wal).expect("upgrade");
    assert_eq!(store.journal_mode(), "wal");
    // the live files, while the store is still open
    let wal = dir.path().join(format!("{DB_FILE}-wal"));
    let main = std::fs::read(&path).expect("main");
    let log = std::fs::read(&wal).unwrap_or_default();
    assert!(!contains(&main, MARKER), "marker left in the main file");
    assert!(!contains(&log, MARKER), "marker left in the -wal file");
    drop(store);
    // the copy keeps it (documented)
    let copies = backups(dir.path());
    assert_eq!(copies.len(), 1);
    assert!(contains(&std::fs::read(&copies[0]).expect("copy"), MARKER));
}

#[test]
fn enabling_leaves_the_author_alone_and_retention_sets_it() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let before = store.read(tables::load_settings).expect("settings");
    assert!(!before.enabled);
    assert!(store.write(|tx| tables::set_enabled(tx, true)).expect("on"));
    assert!(!store
        .write(|tx| tables::set_enabled(tx, true))
        .expect("again"));
    let s = store.read(tables::load_settings).expect("settings");
    assert!(s.enabled);
    assert_eq!(
        (s.updated_at, s.updated_by.as_str()),
        (before.updated_at, "")
    );
    assert!(store
        .write(|tx| tables::set_retention(tx, 30, "root", 1_700_000_000))
        .expect("retention"));
    assert!(!store
        .write(|tx| tables::set_retention(tx, 30, "other", 1_700_000_001))
        .expect("same"));
    let s = store.read(tables::load_settings).expect("settings");
    assert_eq!(
        (
            s.retention_days,
            s.updated_at,
            s.updated_by.as_str(),
            s.enabled
        ),
        (30, 1_700_000_000, "root", true)
    );
    assert!(store
        .write(|tx| tables::set_enabled(tx, false))
        .expect("off"));
    let s = store.read(tables::load_settings).expect("settings");
    assert_eq!(
        (s.updated_at, s.updated_by.as_str()),
        (1_700_000_000, "root")
    );
}

#[tokio::test]
async fn the_retention_answer_is_unchanged() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let (s, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"retentionDays": 30}),
        )
        .await;
    assert_eq!((s, v["retentionDays"].clone()), (200, json!(30)), "{v}");
    assert_eq!(v["updatedBy"], json!("admin"));
    let c = t.db();
    let by: String = c
        .query_row("SELECT updated_by FROM settings WHERE id = 1", [], |r| {
            r.get(0)
        })
        .expect("by");
    assert_eq!(by, "admin");
    // the same value again changes nothing
    let (s, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"retentionDays": 30}),
        )
        .await;
    assert_eq!((s, v["retentionDays"].clone()), (200, json!(30)), "{v}");
}
