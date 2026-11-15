//! v1.3 storage: AI database schema 7 (upgrade from schema 6, chat tree, connections / pool / slots, shared study answers,
//! job items, connection statistics, limits, retention). Made-up sample data only.
use rusqlite::Connection;
use std::path::Path;
use xexam_ai_service::config::JournalMode;
use xexam_ai_service::identity::{Identity, Role};
use xexam_ai_service::limits::{Limits, DEFS};
use xexam_ai_service::retention;
use xexam_ai_service::secret::{Sealed, SecretName};
use xexam_ai_service::store::conn::{self, ConnChange, NewConn, NewConnSecret, NewPool};
use xexam_ai_service::store::data::{self, StudyInsert};
use xexam_ai_service::store::items;
use xexam_ai_service::store::shared::{self, NewShared, SharedKey};
use xexam_ai_service::store::stats::{self, GenCallRow, UsageRow};
use xexam_ai_service::store::tables;
use xexam_ai_service::store::{schema, Store, DB_FILE};

const NOW: i64 = 1_800_000_000;
const HASH: &str = "0123456789abcdef0123456789abcdef";

fn open(dir: &Path) -> Store {
    Store::open(dir, JournalMode::Delete).expect("open store")
}

fn who(id: i64) -> Identity {
    Identity {
        id,
        stamp: format!("{:032x}", 0x7000 + id),
        username: format!("user{id}"),
        role: Role::User,
        ai_allowed: true,
        sid: "0".repeat(64),
    }
}

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

/// Names of the tables, indexes and their columns, without the DDL text (an `ALTER TABLE ADD COLUMN` rewrites that text).
fn shape(path: &Path) -> Vec<String> {
    let c = Connection::open(path).expect("open");
    let mut out = Vec::new();
    for (ty, name, _) in master(path) {
        if ty == "table" {
            let mut st = c
                .prepare("SELECT name, type, \"notnull\", dflt_value FROM pragma_table_info(?1) ORDER BY name")
                .expect("pragma");
            let cols: Vec<String> = st
                .query_map([&name], |r| {
                    Ok(format!(
                        "{}:{}:{}:{:?}",
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, Option<String>>(3)?
                    ))
                })
                .expect("cols")
                .collect::<Result<_, _>>()
                .expect("cols");
            out.push(format!("table {name} {}", cols.join(",")));
        } else {
            out.push(format!("{ty} {name}"));
        }
    }
    out
}

/// A schema-6 file as v1.2 wrote it: the tables of version 1 plus the columns of 2 .. 6 and the late column, no schema-7
/// piece. `custom` = the administrator changed three limits; AI is on with one connection and one saved key.
fn write_v6(dir: &Path, custom: bool) {
    let c = Connection::open(dir.join(DB_FILE)).expect("fixture");
    c.execute_batch(schema::SCHEMA).expect("v1");
    for (table, _, def) in schema::V2_COLUMNS
        .iter()
        .chain(schema::V3_COLUMNS.iter())
        .chain(schema::V4_COLUMNS.iter())
        .chain(schema::V5_COLUMNS.iter())
        .chain(schema::V6_COLUMNS.iter())
    {
        c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {def};"))
            .expect("column");
    }
    c.execute_batch(schema::V3_INDEXES).expect("indexes");
    c.execute_batch(schema::V5_TABLES).expect("tables");
    c.execute_batch("ALTER TABLE gen_calls ADD COLUMN tokens_cache_write INTEGER;")
        .expect("late");
    c.execute_batch(
        "INSERT INTO meta (key, value) VALUES ('schema', '6'), ('unlimited_default', '1'), ('import_max_50', '1');
         INSERT INTO settings (id, provider, base_url, format, gateway_auth, model, manual_model, usage_option,
                               max_tokens, enabled, verified_at, verified_fp, updated_at, updated_by)
           VALUES (1, 'gateway', 'https://gw.example.com/v1/chat', 'openai', 'bearer', 'demo-chat', 0, 1, 4096, 1,
                   500, 'fp-demo', 400, 'root');
         INSERT INTO secrets (name, key_id, nonce, ciphertext, hint, rev, updated_at)
           VALUES ('api_key', 'k1', zeroblob(24), x'0102030405', '...demo', 3, 300);
         INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at) VALUES (7, 'x', 'kept', 1, 1);
         INSERT INTO chats (id, user_id, title, created_at, updated_at, kind, ref) VALUES
           (1, 7, 'plain chat', 10, 20, 'chat', NULL),
           (2, 7, 'practice', 11, 21, 'practice', 'abc'),
           (3, 7, 'empty', 12, 22, 'chat', NULL);
         INSERT INTO messages (id, chat_id, role, content, created_at) VALUES
           (1, 1, 'user', 'q1', 10), (2, 1, 'assistant', 'a1', 11), (3, 1, 'user', 'q2', 12), (4, 1, 'assistant', 'a2', 13);
         INSERT INTO messages (id, chat_id, role, content, created_at, kind, question_id, source) VALUES
           (5, 2, 'user', 'hint?', 14, 'hint', 'qa', 'practice'), (6, 2, 'assistant', 'hint!', 15, 'hint', 'qa', 'practice'),
           (7, 2, 'user', 'why?', 16, 'explain', 'qa', 'practice'), (8, 2, 'assistant', 'because', 17, 'explain', 'qa', 'practice');
         INSERT INTO usage (user_id, at, day, kind, provider, model, latency_ms, status)
           VALUES (7, 100, '2026-11-01', 'chat', 'gateway', 'demo-chat', 5, 'ok');
         INSERT INTO limits (id, updated_at) VALUES (1, 1);",
    )
    .expect("rows");
    if custom {
        c.execute_batch(
            "UPDATE limits SET import_max_questions = 30, memory_tokens = 777, memories_per_user = 12 WHERE id = 1",
        )
        .expect("custom limits");
    } else {
        // the v1.2 defaults the upgrade moves
        c.execute_batch(
            "UPDATE limits SET import_max_questions = 50, memory_tokens = 1000, memories_per_user = 50 WHERE id = 1",
        )
        .expect("default limits");
    }
}

fn limits_of(dir: &Path) -> (i64, i64, i64) {
    let c = Connection::open(dir.join(DB_FILE)).expect("open");
    c.query_row(
        "SELECT import_file_questions, memory_tokens, memories_per_user FROM limits WHERE id = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .expect("limits")
}

#[test]
fn a_schema_6_file_upgrades_to_7_keeping_every_row() {
    let dir = tempfile::tempdir().expect("dir");
    write_v6(dir.path(), false);
    let store = open(dir.path());
    // one backup of the schema-6 file, taken before the change; it still opens as schema 6
    let copies: Vec<_> = std::fs::read_dir(dir.path().join("backups"))
        .expect("backups")
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(copies.len(), 1);
    let name = copies[0].file_name().to_string_lossy().to_string();
    assert!(name.starts_with("xexam-ai-schema6-"), "{name}");
    let b = Connection::open(copies[0].path()).expect("backup");
    let v: String = b
        .query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
            r.get(0)
        })
        .expect("schema");
    assert_eq!(v, "6");
    let has_conn: bool = b
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'connections')",
            [],
            |r| r.get(0),
        )
        .expect("master");
    assert!(!has_conn);
    drop(b);
    // schema 7; each chat is one line: the path equals the old ORDER BY id list
    store
        .read(|c| {
            let v: String =
                c.query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
                    r.get(0)
                })?;
            assert_eq!(v, "7");
            for (chat, want) in [(1, vec![1, 2, 3, 4]), (2, vec![5, 6, 7, 8]), (3, vec![])] {
                let all: Vec<i64> = data::messages(c, chat)?.iter().map(|m| m.id).collect();
                let line: Vec<i64> = data::path(c, chat, None)?.iter().map(|m| m.id).collect();
                assert_eq!(line, want, "chat {chat}");
                assert_eq!(all, want, "chat {chat}");
            }
            let roots: i64 = c.query_row(
                "SELECT COUNT(*) FROM messages WHERE parent_id IS NULL",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(roots, 2, "one root per non-empty chat");
            // v1.3 reasoning columns: every old message reads as "none"
            let kept: i64 = c.query_row(
                "SELECT COUNT(*) FROM messages WHERE reasoning IS NULL AND reasoning_ms IS NULL AND reasoning_truncated = 0 AND content <> ''",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(kept, 8);
            assert!(data::messages(c, 1)?.iter().all(|m| m.reasoning.is_none()
                && m.reasoning_ms.is_none()
                && !m.reasoning_truncated));
            assert_eq!(data::chat(c, 7, 1)?.expect("chat").leaf_id, Some(4));
            assert_eq!(data::chat(c, 7, 2)?.expect("chat").leaf_id, Some(8));
            assert_eq!(data::chat(c, 7, 3)?.expect("chat").leaf_id, None);
            assert_eq!(data::chat(c, 7, 1)?.expect("chat").message_count, 4);
            // the usage row stays without a connection
            let (cid, cname): (Option<i64>, Option<String>) =
                c.query_row("SELECT connection_id, connection FROM usage", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
            assert_eq!((cid, cname), (None, None));
            // connection 1 from the old settings row, keys copied as they are (aad_ver 1)
            let conns = conn::connections(c)?;
            assert_eq!(conns.len(), 1);
            let k = &conns[0];
            assert_eq!(
                (
                    k.id,
                    k.name.as_str(),
                    k.provider.as_str(),
                    k.base_url.as_str()
                ),
                (
                    1,
                    "gw.example.com",
                    "gateway",
                    "https://gw.example.com/v1/chat"
                )
            );
            assert_eq!(
                (k.status.as_str(), k.rev, k.tested_at),
                ("ok", 1, Some(500))
            );
            assert_eq!(k.updated_by, "root");
            let secrets = conn::conn_secrets(c, 1)?;
            assert_eq!(secrets.len(), 1);
            assert_eq!(
                (
                    secrets[0].name,
                    secrets[0].rev,
                    secrets[0].aad_ver,
                    secrets[0].hint.as_str()
                ),
                (SecretName::ApiKey, 3, 1, "...demo")
            );
            assert_eq!(secrets[0].ciphertext, vec![1, 2, 3, 4, 5]);
            // the model and the default slot; AI stays on
            let pool = conn::pool(c)?;
            assert_eq!(pool.len(), 1);
            assert_eq!(
                (
                    pool[0].model.as_str(),
                    pool[0].status.as_str(),
                    pool[0].max_tokens
                ),
                ("demo-chat", "ok", 4096)
            );
            let slots = conn::slots(c)?;
            assert_eq!(slots, [Some(pool[0].id), None, None, None, None, None]);
            let enabled: i64 = c.query_row("SELECT enabled FROM settings", [], |r| r.get(0))?;
            assert_eq!(enabled, 1);
            Ok(())
        })
        .expect("read");
    // limits that were never changed moved to the v1.3 defaults
    drop(store);
    assert_eq!(limits_of(dir.path()), (100, 200, 30));
    // a second start is a no-op: no new backup, same rows, no second connection
    let before = master(&dir.path().join(DB_FILE));
    let store = open(dir.path());
    assert_eq!(
        std::fs::read_dir(dir.path().join("backups"))
            .expect("backups")
            .count(),
        1
    );
    assert_eq!(store.read(conn::connections).expect("c").len(), 1);
    drop(store);
    assert_eq!(master(&dir.path().join(DB_FILE)), before);
    assert_eq!(limits_of(dir.path()), (100, 200, 30));
}

#[test]
fn custom_limits_stay_and_an_unconfigured_file_gets_no_connection() {
    let dir = tempfile::tempdir().expect("dir");
    write_v6(dir.path(), true);
    drop(open(dir.path()));
    assert_eq!(limits_of(dir.path()), (30, 777, 12));
    // an installation whose AI was never configured: no connection, no model
    let dir = tempfile::tempdir().expect("dir");
    write_v6(dir.path(), false);
    {
        let c = Connection::open(dir.path().join(DB_FILE)).expect("open");
        c.execute_batch(
            "UPDATE settings SET updated_by = '', model = '', enabled = 0, verified_fp = NULL, verified_at = NULL;
             DELETE FROM secrets;",
        )
        .expect("clear");
    }
    let store = open(dir.path());
    store
        .read(|c| {
            assert!(conn::connections(c)?.is_empty());
            assert!(conn::pool(c)?.is_empty());
            assert_eq!(conn::slots(c)?, [None; 6]);
            Ok(())
        })
        .expect("read");
}

#[test]
fn a_model_that_was_never_verified_is_migrated_as_untested() {
    let dir = tempfile::tempdir().expect("dir");
    write_v6(dir.path(), false);
    {
        let c = Connection::open(dir.path().join(DB_FILE)).expect("open");
        c.execute_batch(
            "UPDATE settings SET verified_fp = NULL, verified_at = NULL, manual_model = 1, base_url = 'http://[::1]:8080/x',
               updated_by = 'root'",
        )
        .expect("update");
    }
    let store = open(dir.path());
    store
        .read(|c| {
            let pool = conn::pool(c)?;
            assert_eq!(
                (pool[0].status.as_str(), pool[0].manual),
                ("untested", true)
            );
            assert_eq!(conn::connections(c)?[0].name, "::1");
            Ok(())
        })
        .expect("read");
}

#[test]
fn an_upgraded_file_has_the_shape_of_a_fresh_one() {
    let fresh = tempfile::tempdir().expect("dir");
    drop(open(fresh.path()));
    let old = tempfile::tempdir().expect("dir");
    write_v6(old.path(), false);
    drop(open(old.path()));
    assert_eq!(
        shape(&fresh.path().join(DB_FILE)),
        shape(&old.path().join(DB_FILE))
    );
    // a fresh file: the six slots exist, the limits row carries the v1.3 defaults, no backup
    let c = Connection::open(fresh.path().join(DB_FILE)).expect("open");
    let slots: i64 = c
        .query_row("SELECT COUNT(*) FROM model_slots", [], |r| r.get(0))
        .expect("slots");
    assert_eq!(slots, 6);
    assert!(!fresh.path().join("backups").exists());
    drop(c);
    assert_eq!(limits_of(fresh.path()), (100, 200, 30));
}

#[test]
fn every_column_and_table_of_schema_7_exists_in_a_fresh_file() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    let c = Connection::open(dir.path().join(DB_FILE)).expect("open");
    for (table, column, _) in schema::V7_COLUMNS.iter().chain(schema::LATE_COLUMNS.iter()) {
        let present: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
                [table, column],
                |r| r.get(0),
            )
            .expect("pragma");
        assert!(present, "{table}.{column}");
    }
    // the v1.3 reasoning of an answer: nullable text / integer, and a NOT NULL flag defaulting to 0
    for (column, ty, notnull, default) in [
        ("reasoning", "TEXT", 0, None),
        ("reasoning_ms", "INTEGER", 0, None),
        ("reasoning_truncated", "INTEGER", 1, Some("0")),
    ] {
        let got: (String, i64, Option<String>) = c
            .query_row(
                "SELECT type, \"notnull\", dflt_value FROM pragma_table_info('messages') WHERE name = ?1",
                [column],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("column");
        assert_eq!(
            got,
            (ty.to_string(), notnull, default.map(str::to_string)),
            "{column}"
        );
    }
    for t in [
        "connections",
        "connection_secrets",
        "models",
        "model_slots",
        "study_shared",
        "gen_items",
    ] {
        let present: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [t],
                |r| r.get(0),
            )
            .expect("master");
        assert!(present, "{t}");
    }
    for i in [
        "idx_messages_parent",
        "idx_connections_name",
        "idx_study_shared_key",
        "idx_study_shared_used",
    ] {
        let present: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
                [i],
                |r| r.get(0),
            )
            .expect("master");
        assert!(present, "{i}");
    }
    // the CHECKs of the new columns
    c.pragma_update(None, "foreign_keys", "ON").expect("fk");
    assert!(c
        .execute("UPDATE settings SET features_off = -1", [])
        .is_err());
    assert!(c
        .execute("UPDATE limits SET gen_max_chunks = 51", [])
        .is_err());
    assert!(c
        .execute("UPDATE limits SET summary_max_items = 9", [])
        .is_err());
    assert!(c
        .execute("UPDATE limits SET import_file_questions = 301", [])
        .is_err());
    assert!(c
        .execute("UPDATE limits SET daily_token_budget = -1", [])
        .is_err());
    assert!(c
        .execute("UPDATE limits SET study_share_days = 6", [])
        .is_err());
    assert!(c
        .execute(
            "INSERT INTO connections (name, provider, base_url, created_at, updated_at) VALUES ('', 'openai', 'u', 1, 1)",
            []
        )
        .is_err());
    assert!(c
        .execute(
            "UPDATE model_slots SET slot = 'bogus' WHERE slot = 'chat'",
            []
        )
        .is_err());
}

#[test]
fn an_upgrade_from_schema_1_reaches_7_with_the_chat_tree() {
    let dir = tempfile::tempdir().expect("dir");
    {
        let c = Connection::open(dir.path().join(DB_FILE)).expect("fixture");
        c.execute_batch(schema::SCHEMA).expect("v1");
        c.execute_batch(
            "INSERT INTO meta (key, value) VALUES ('schema', '1');
             INSERT INTO settings (id, updated_at) VALUES (1, 1);
             INSERT INTO limits (id, updated_at) VALUES (1, 1);
             INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at) VALUES (7, 'x', 'kept', 1, 1);
             INSERT INTO chats (id, user_id, title, created_at, updated_at) VALUES (1, 7, 't', 1, 1);
             INSERT INTO messages (chat_id, role, content, created_at) VALUES (1, 'user', 'q', 1), (1, 'assistant', 'a', 2);",
        )
        .expect("rows");
    }
    let store = open(dir.path());
    store
        .read(|c| {
            let ids: Vec<i64> = data::path(c, 1, None)?.iter().map(|m| m.id).collect();
            assert_eq!(ids, vec![1, 2]);
            // never configured: no connection; the old defaults moved once (schema 5 -> 6 -> 7)
            assert!(conn::connections(c)?.is_empty());
            Ok(())
        })
        .expect("read");
    drop(store);
    // 1000 chars-default memory (200 old default) -> 1000 tokens -> 200 (untouched), import 50 -> 100
    assert_eq!(limits_of(dir.path()), (100, 200, 30));
}

// ---------------------------------------------------------------------------------------------------------------------
// The chat tree

fn new_chat(store: &Store) -> i64 {
    store
        .write(|tx| {
            data::ensure_user(tx, &who(1), NOW)?;
            data::create_chat(tx, 1, "tree", NOW)
        })
        .expect("chat")
}

fn msg(c: &Connection, chat: i64, role: &str, text: &str, parent: Option<i64>) -> i64 {
    data::insert_message(
        c,
        &data::NewMessage {
            chat,
            role,
            content: text,
            status: "done",
            model: None,
            free: false,
            now: NOW,
            parent,
            connection: if role == "assistant" {
                Some("main")
            } else {
                None
            },
        },
    )
    .expect("insert")
}

#[test]
fn inserting_follows_the_leaf_and_moves_it() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let chat = new_chat(&store);
    let (q1, a1, q2, a2) = store
        .write(|tx| {
            let q1 = msg(tx, chat, "user", "q1", None);
            let a1 = msg(tx, chat, "assistant", "a1", None);
            let q2 = msg(tx, chat, "user", "q2", None);
            let a2 = msg(tx, chat, "assistant", "a2", None);
            Ok::<_, rusqlite::Error>((q1, a1, q2, a2))
        })
        .expect("write");
    store
        .read(|c| {
            let path = data::path(c, chat, None)?;
            assert_eq!(
                path.iter().map(|m| m.id).collect::<Vec<_>>(),
                vec![q1, a1, q2, a2]
            );
            assert_eq!(
                path.iter().map(|m| m.parent_id).collect::<Vec<_>>(),
                vec![None, Some(q1), Some(a1), Some(q2)]
            );
            assert_eq!(path[1].connection.as_deref(), Some("main"));
            assert_eq!(path[0].connection, None);
            assert_eq!(data::chat(c, 1, chat)?.expect("chat").leaf_id, Some(a2));
            // ALL rows are counted
            assert_eq!(data::chat(c, 1, chat)?.expect("chat").message_count, 4);
            assert!(data::versions(c, chat, &path)?.is_empty());
            Ok(())
        })
        .expect("read");
}

#[test]
fn regenerate_and_edit_make_versions_and_the_path_follows_the_leaf() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let chat = new_chat(&store);
    let (q1, a1, q2, a2, a2b, q2b, a2c) = store
        .write(|tx| {
            let q1 = msg(tx, chat, "user", "q1", None);
            let a1 = msg(tx, chat, "assistant", "a1", None);
            let q2 = msg(tx, chat, "user", "q2", None);
            let a2 = msg(tx, chat, "assistant", "a2", None);
            // regenerate: a second answer of q2
            let a2b = msg(tx, chat, "assistant", "a2 again", Some(q2));
            // edit: a second version of q2 (same parent a1), with its own answer
            let q2b = msg(tx, chat, "user", "q2 edited", Some(a1));
            let a2c = msg(tx, chat, "assistant", "a2 edited", None);
            Ok::<_, rusqlite::Error>((q1, a1, q2, a2, a2b, q2b, a2c))
        })
        .expect("write");
    store
        .read(|c| {
            // the viewed line is the edited one
            let path = data::path(c, chat, None)?;
            assert_eq!(
                path.iter().map(|m| m.id).collect::<Vec<_>>(),
                vec![q1, a1, q2b, a2c]
            );
            let v = data::versions(c, chat, &path)?;
            assert_eq!(v.get(&q2b), Some(&vec![q2, q2b]));
            assert_eq!(v.get(&a2c), None, "one answer under the edited question");
            assert!(!v.contains_key(&q1) && !v.contains_key(&a1));
            // the older line, read from its leaf
            let old = data::path(c, chat, Some(a2))?;
            assert_eq!(
                old.iter().map(|m| m.id).collect::<Vec<_>>(),
                vec![q1, a1, q2, a2]
            );
            let v = data::versions(c, chat, &old)?;
            assert_eq!(v.get(&a2), Some(&vec![a2, a2b]));
            assert_eq!(v.get(&q2), Some(&vec![q2, q2b]));
            assert_eq!(data::answer_versions(c, chat, q2)?, 2);
            assert_eq!(data::answer_versions(c, chat, q2b)?, 1);
            // the end of the line that runs through a version: the newest child at each step
            assert_eq!(data::line_end(c, chat, q2)?, a2b);
            assert_eq!(
                data::line_end(c, chat, a1)?,
                a2c,
                "newest child of a1 is the edited question"
            );
            assert_eq!(data::line_end(c, chat, a2c)?, a2c);
            // on_path follows the leaf
            assert!(data::on_path(c, chat, q2b)?);
            assert!(!data::on_path(c, chat, q2)?);
            assert!(data::on_path(c, chat, q1)?);
            assert_eq!(data::messages(c, chat)?.len(), 7);
            Ok(())
        })
        .expect("read");
}

#[test]
fn switching_the_leaf_touches_no_timestamp_and_stays_inside_the_chat() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let chat = new_chat(&store);
    let other = new_chat(&store);
    let (a1, a1b) = store
        .write(|tx| {
            let q = msg(tx, chat, "user", "q", None);
            let a1 = msg(tx, chat, "assistant", "a1", None);
            let a1b = msg(tx, chat, "assistant", "a1b", Some(q));
            Ok::<_, rusqlite::Error>((a1, a1b))
        })
        .expect("write");
    let foreign = store
        .write(|tx| Ok::<_, rusqlite::Error>(msg(tx, other, "user", "elsewhere", None)))
        .expect("write");
    let stamp = |c: &Connection| -> i64 {
        c.query_row("SELECT updated_at FROM chats WHERE id = ?1", [chat], |r| {
            r.get(0)
        })
        .expect("updated_at")
    };
    let before = store.read(|c| Ok(stamp(c))).expect("stamp");
    store
        .write(|tx| {
            data::touch_chat(tx, chat, NOW + 50)?;
            data::set_leaf(tx, chat, a1)?;
            Ok::<_, rusqlite::Error>(())
        })
        .expect("leaf");
    store
        .read(|c| {
            assert_eq!(
                stamp(c),
                NOW + 50,
                "set_leaf leaves updated_at as touch_chat set it"
            );
            assert_ne!(before, stamp(c));
            assert_eq!(data::chat(c, 1, chat)?.expect("chat").leaf_id, Some(a1));
            Ok(())
        })
        .expect("read");
    // a message of another chat is refused, the leaf stays
    let err = store.write(|tx| data::set_leaf(tx, chat, foreign));
    assert!(err.is_err());
    // a parent of another chat is refused too
    let err = store.write(|tx| {
        data::insert_message(
            tx,
            &data::NewMessage {
                chat,
                role: "user",
                content: "x",
                status: "done",
                model: None,
                free: false,
                now: NOW,
                parent: Some(foreign),
                connection: None,
            },
        )
    });
    assert!(err.is_err());
    store
        .read(|c| {
            assert_eq!(data::chat(c, 1, chat)?.expect("chat").leaf_id, Some(a1));
            // a path never crosses into another chat
            assert!(data::path(c, chat, Some(foreign))?.is_empty());
            assert_eq!(data::path(c, chat, Some(a1b))?.len(), 2);
            Ok(())
        })
        .expect("read");
}

#[test]
fn the_admin_pick_of_a_chat_is_stored_and_cleared() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let chat = new_chat(&store);
    let k = store
        .write(|tx| conn::insert_connection(tx, &new_conn("Main", NOW)))
        .expect("connection");
    store
        .write(|tx| data::set_chat_pick(tx, chat, Some((k, "vendor:model-x"))))
        .expect("pick");
    assert_eq!(
        store
            .read(|c| data::chat(c, 1, chat))
            .expect("chat")
            .expect("row")
            .pick,
        Some((k, "vendor:model-x".to_string()))
    );
    store
        .write(|tx| data::set_chat_pick(tx, chat, None))
        .expect("clear");
    assert_eq!(
        store
            .read(|c| data::chat(c, 1, chat))
            .expect("chat")
            .expect("row")
            .pick,
        None
    );
}

fn raw_picks(dir: &Path) -> Vec<(i64, Option<i64>, Option<String>)> {
    let c = Connection::open(dir.join(DB_FILE)).expect("db");
    let mut st = c
        .prepare("SELECT id, pick_conn, pick_model FROM chats ORDER BY id")
        .expect("sql");
    let rows = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("rows");
    rows.collect::<rusqlite::Result<Vec<_>>>().expect("picks")
}

#[test]
fn a_removed_connection_takes_the_chat_picks_of_it_and_a_stale_pick_reads_as_none() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let (one, two, three) = (new_chat(&store), new_chat(&store), new_chat(&store));
    let (a, b) = store
        .write(|tx| {
            let a = conn::insert_connection(tx, &new_conn("Main", NOW))?;
            let b = conn::insert_connection(tx, &new_conn("Backup", NOW))?;
            data::set_chat_pick(tx, one, Some((a, "demo-small")))?;
            data::set_chat_pick(tx, two, Some((b, "demo-large")))?;
            // a pick of a connection that is not there (a file of an earlier v1.3 build)
            data::set_chat_pick(tx, three, Some((99, "demo-gone")))?;
            Ok::<_, rusqlite::Error>((a, b))
        })
        .expect("picks");
    let pick = |chat: i64| {
        store
            .read(|c| data::chat(c, 1, chat))
            .expect("chat")
            .expect("row")
            .pick
    };
    assert_eq!(pick(three), None, "a stale pick is not read");
    let listed = store.read(|c| data::list_chats(c, 1)).expect("list");
    assert_eq!(listed.iter().filter(|r| r.pick.is_some()).count(), 2);
    store
        .write(|tx| conn::delete_connection(tx, b))
        .expect("delete");
    assert_eq!(pick(one), Some((a, "demo-small".to_string())));
    assert_eq!(pick(two), None);
    assert_eq!(
        raw_picks(dir.path()),
        vec![
            (one, Some(a), Some("demo-small".to_string())),
            (two, None, None),
            (three, Some(99), Some("demo-gone".to_string())),
        ]
    );
}

#[test]
fn a_schema_7_file_with_stale_chat_picks_is_cleaned_at_the_next_start() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let (one, two) = (new_chat(&store), new_chat(&store));
    let a = store
        .write(|tx| {
            let a = conn::insert_connection(tx, &new_conn("Main", NOW))?;
            data::set_chat_pick(tx, one, Some((a, "demo-small")))?;
            data::set_chat_pick(tx, two, Some((99, "demo-gone")))?;
            Ok::<_, rusqlite::Error>(a)
        })
        .expect("picks");
    drop(store);
    let want = vec![
        (one, Some(a), Some("demo-small".to_string())),
        (two, None, None),
    ];
    drop(open(dir.path()));
    assert_eq!(raw_picks(dir.path()), want);
    // again: nothing changes
    drop(open(dir.path()));
    assert_eq!(raw_picks(dir.path()), want);
}

#[test]
fn study_asks_append_at_the_viewed_end_and_move_the_leaf() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let ask = |q: &str, now: i64| {
        let s = StudyInsert {
            chat_kind: "practice",
            chat_ref: HASH,
            title: "2026-11-11 21:58",
            record_id: None,
            kind: "hint",
            question_id: Some(q),
            question_no: Some(1),
            source: Some("practice"),
            lang: "en",
            sample: None,
            user_content: "Stem?",
            model: "demo-model",
            now,
            chats_per_user: 1000,
            messages_per_chat: 400,
        };
        store
            .write(|tx| data::prepare_study(tx, &who(1), &s, &mut |_| true))
            .expect("ask")
    };
    let a = ask("q1", NOW);
    let b = ask("q2", NOW + 1);
    assert_eq!(a.chat, b.chat);
    store
        .read(|c| {
            let path = data::path(c, a.chat, None)?;
            assert_eq!(
                path.iter().map(|m| m.id).collect::<Vec<_>>(),
                vec![a.user_msg, a.answer, b.user_msg, b.answer]
            );
            assert_eq!(path[0].parent_id, None);
            assert_eq!(path[2].parent_id, Some(a.answer));
            assert_eq!(
                data::chat(c, 1, a.chat)?.expect("chat").leaf_id,
                Some(b.answer)
            );
            // the lookups ignore the branch: switch the view to the first pair, q2 is still found
            data::set_leaf(c, a.chat, a.answer)?;
            let saved = data::study_saved(c, 1, "practice", HASH, "hint", "q2", "practice")?;
            assert_eq!(saved.latest.map(|x| x.message_id), Some(b.answer));
            Ok(())
        })
        .expect("read");
}

// ---------------------------------------------------------------------------------------------------------------------
// Connections, pool, slots

fn new_conn<'a>(name: &'a str, now: i64) -> NewConn<'a> {
    NewConn {
        name,
        provider: "openai",
        base_url: "https://api.example.com/v1",
        format: "openai",
        gateway_auth: "bearer",
        lists_models: true,
        by: "root",
        now,
    }
}

fn pool_entry(connection_id: i64, model: &str) -> NewPool<'_> {
    NewPool {
        connection_id,
        model,
        manual: false,
        status: "ok",
        error_code: None,
        usage_option: true,
        max_tokens: 8192,
        context: Some(128_000),
        latency_ms: Some(120),
        tested_at: Some(NOW),
    }
}

#[test]
fn connections_are_created_ordered_saved_with_a_revision_and_deleted() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let (a, b) = store
        .write(|tx| {
            let a = conn::insert_connection(tx, &new_conn("Main", NOW))?;
            let b = conn::insert_connection(tx, &new_conn("Backup", NOW))?;
            Ok::<_, rusqlite::Error>((a, b))
        })
        .expect("create");
    store
        .read(|c| {
            let rows = conn::connections(c)?;
            assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![a, b]);
            assert_eq!(
                (rows[0].rev, rows[0].status.as_str(), rows[0].sort),
                (1, "ok", 0)
            );
            assert_eq!(rows[1].sort, 1);
            // names are unique, case-insensitively
            assert!(conn::name_taken(c, "main", None)?);
            assert!(!conn::name_taken(c, "main", Some(a))?);
            assert!(!conn::name_taken(c, "other", None)?);
            Ok(())
        })
        .expect("read");
    assert!(store
        .write(|tx| conn::insert_connection(tx, &new_conn("MAIN", NOW)))
        .is_err());
    let change = |name: &'static str| ConnChange {
        name,
        provider: "gateway",
        base_url: "https://gw.example.com/v1",
        format: "anthropic",
        gateway_auth: "x-api-key",
        lists_models: false,
        by: "admin2",
        now: NOW + 5,
    };
    // a save with the right revision goes through and bumps it; the old revision then conflicts
    assert!(store
        .write(|tx| conn::update_connection(tx, a, 1, &change("Main 2")))
        .expect("save"));
    assert!(!store
        .write(|tx| conn::update_connection(tx, a, 1, &change("Main 3")))
        .expect("stale"));
    let row = store
        .read(|c| conn::connection(c, a))
        .expect("get")
        .expect("row");
    assert_eq!(
        (
            row.rev,
            row.name.as_str(),
            row.provider.as_str(),
            row.format.as_str(),
            row.lists_models
        ),
        (2, "Main 2", "gateway", "anthropic", false)
    );
    assert_eq!(
        (row.updated_by.as_str(), row.updated_at),
        ("admin2", NOW + 5)
    );
    // order and status
    store
        .write(|tx| {
            conn::order_connections(tx, &[b, a])?;
            conn::set_connection_status(tx, a, "broken", Some("ai.up.auth"), Some(NOW + 9))?;
            Ok::<_, rusqlite::Error>(())
        })
        .expect("order");
    store
        .read(|c| {
            let rows = conn::connections(c)?;
            assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![b, a]);
            let a_row = rows.iter().find(|r| r.id == a).expect("a");
            assert_eq!(
                (
                    a_row.status.as_str(),
                    a_row.error_code.as_deref(),
                    a_row.tested_at
                ),
                ("broken", Some("ai.up.auth"), Some(NOW + 9))
            );
            Ok(())
        })
        .expect("read");
    // set_connection_status keeps tested_at when None
    store
        .write(|tx| conn::set_connection_status(tx, a, "ok", None, None))
        .expect("ok");
    assert_eq!(
        store
            .read(|c| conn::connection(c, a))
            .expect("get")
            .expect("row")
            .tested_at,
        Some(NOW + 9)
    );
}

#[test]
fn keys_are_sealed_rows_with_a_never_reused_revision() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let id = store
        .write(|tx| conn::insert_connection(tx, &new_conn("Main", NOW)))
        .expect("create");
    let sealed = Sealed {
        nonce: [7u8; 24],
        ciphertext: vec![9, 8, 7],
    };
    let put = |name: SecretName, aad: i64| {
        store
            .write(|tx| {
                conn::put_conn_secret(
                    tx,
                    id,
                    &NewConnSecret {
                        name,
                        key_id: "k2",
                        sealed: &sealed,
                        hint: "...abcd",
                        aad_ver: aad,
                        now: NOW,
                    },
                )
            })
            .expect("put")
    };
    let r1 = put(SecretName::ApiKey, 2);
    let r2 = put(SecretName::GatewayKey, 2);
    let r3 = put(SecretName::ApiKey, 2);
    assert!(r1 < r2 && r2 < r3);
    let rows = store.read(|c| conn::conn_secrets(c, id)).expect("secrets");
    assert_eq!(rows.len(), 2);
    let api = rows
        .iter()
        .find(|r| r.name == SecretName::ApiKey)
        .expect("api");
    assert_eq!((api.rev, api.aad_ver, api.key_id.as_str()), (r3, 2, "k2"));
    assert_eq!(api.nonce, vec![7u8; 24]);
    assert!(store
        .write(|tx| conn::delete_conn_secret(tx, id, SecretName::GatewayKey))
        .expect("delete"));
    assert_eq!(
        store.read(|c| conn::conn_secrets(c, id)).expect("s").len(),
        1
    );
    // deleting the connection removes its keys
    store
        .write(|tx| conn::delete_connection(tx, id))
        .expect("delete connection");
    let left: i64 = store
        .read(|c| c.query_row("SELECT COUNT(*) FROM connection_secrets", [], |r| r.get(0)))
        .expect("count");
    assert_eq!(left, 0);
}

#[test]
fn the_pool_slots_and_the_cascade_of_a_removed_connection() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let (a, b) = store
        .write(|tx| {
            let a = conn::insert_connection(tx, &new_conn("Main", NOW))?;
            let b = conn::insert_connection(tx, &new_conn("Backup", NOW))?;
            Ok::<_, rusqlite::Error>((a, b))
        })
        .expect("create");
    let (m1, m2, m3) = store
        .write(|tx| {
            let m1 = conn::upsert_pool(tx, &pool_entry(a, "demo-small"))?;
            let m2 = conn::upsert_pool(tx, &pool_entry(a, "demo-large"))?;
            let m3 = conn::upsert_pool(tx, &pool_entry(b, "demo-small"))?;
            Ok::<_, rusqlite::Error>((m1, m2, m3))
        })
        .expect("pool");
    assert!(m1 != m2 && m2 != m3);
    // the same (connection, model) again refreshes the entry and keeps the id
    let again = store
        .write(|tx| {
            let mut p = pool_entry(a, "demo-small");
            p.status = "failed";
            p.error_code = Some("ai.up.timeout");
            p.manual = true;
            conn::upsert_pool(tx, &p)
        })
        .expect("again");
    assert_eq!(again, m1);
    store
        .read(|c| {
            let pool = conn::pool(c)?;
            assert_eq!(pool.len(), 3);
            assert_eq!(
                pool.iter().map(|p| p.id).collect::<Vec<_>>(),
                vec![m1, m2, m3]
            );
            let p1 = &pool[0];
            assert_eq!(
                (
                    p1.status.as_str(),
                    p1.error_code.as_deref(),
                    p1.manual,
                    p1.context
                ),
                ("failed", Some("ai.up.timeout"), true, Some(128_000))
            );
            Ok(())
        })
        .expect("read");
    store
        .write(|tx| conn::set_pool_status(tx, m1, "ok", None, Some(50), Some(NOW + 1)))
        .expect("status");
    let p1 = store
        .read(|c| conn::pool_row(c, m1))
        .expect("row")
        .expect("p1");
    assert_eq!(
        (p1.status.as_str(), p1.latency_ms, p1.tested_at),
        ("ok", Some(50), Some(NOW + 1))
    );
    // slots
    store
        .write(|tx| {
            conn::set_slot(tx, "default", Some(m1), "root")?;
            conn::set_slot(tx, "generate", Some(m3), "root")?;
            conn::set_slot(tx, "chat", Some(m2), "root")?;
            Ok::<_, rusqlite::Error>(())
        })
        .expect("slots");
    assert_eq!(
        store.read(conn::slots).expect("slots"),
        [Some(m1), Some(m2), None, None, Some(m3), None]
    );
    assert!(store
        .write(|tx| conn::set_slot(tx, "bogus", Some(m1), "root"))
        .is_err());
    // removing a pool model empties the slots that used it
    store.write(|tx| conn::remove_pool(tx, m2)).expect("remove");
    assert_eq!(
        store.read(conn::slots).expect("slots"),
        [Some(m1), None, None, None, Some(m3), None]
    );
    // removing the connection takes its models; the slot that pointed at one becomes empty
    store
        .write(|tx| conn::delete_connection(tx, b))
        .expect("delete");
    assert_eq!(
        store.read(conn::slots).expect("slots"),
        [Some(m1), None, None, None, None, None]
    );
    assert_eq!(store.read(conn::pool).expect("pool").len(), 1);
    // usage rows keep the id of a removed connection (no foreign key)
    store
        .write(|tx| {
            stats::insert_usage(
                tx,
                &UsageRow {
                    connection_id: Some(b),
                    connection: Some("Backup".to_string()),
                    ..usage_row(None, NOW)
                },
            )
        })
        .expect("usage");
}

#[test]
fn the_global_feature_switches_are_stored_in_the_settings_row() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    assert_eq!(store.read(conn::global_features_off).expect("read"), 0);
    store
        .write(|tx| conn::set_features_off(tx, 0b101))
        .expect("set");
    assert_eq!(store.read(conn::global_features_off).expect("read"), 5);
    store
        .write(|tx| conn::set_features_off(tx, 1000))
        .expect("clamped");
    assert_eq!(store.read(conn::global_features_off).expect("read"), 63);
}

// ---------------------------------------------------------------------------------------------------------------------
// Shared study answers

fn key<'a>(q: &'a str, kind: &'a str, lang: &'a str) -> SharedKey<'a> {
    SharedKey {
        question_id: q,
        content_hash: HASH,
        kind,
        lang,
    }
}

fn answer(text: &str, now: i64) -> NewShared<'_> {
    NewShared {
        content: text,
        model: "demo-chat",
        connection: "Main",
        tokens_in: Some(100),
        tokens_out: Some(50),
        now,
    }
}

#[test]
fn shared_answers_first_write_wins_replace_counts_generations_and_hits_are_counted() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let k = key("q1", "hint", "en");
    assert!(store
        .read(|c| shared::shared_get(c, &k))
        .expect("get")
        .is_none());
    assert!(store
        .write(|tx| shared::shared_put_first(tx, &k, &answer("first", NOW)))
        .expect("put"));
    // a racing second writer loses and changes nothing
    assert!(!store
        .write(|tx| shared::shared_put_first(tx, &k, &answer("second", NOW + 1)))
        .expect("put 2"));
    let row = store
        .read(|c| shared::shared_get(c, &k))
        .expect("get")
        .expect("row");
    assert_eq!(
        (
            row.content.as_str(),
            row.gen,
            row.hits,
            row.model.as_str(),
            row.connection.as_str()
        ),
        ("first", 1, 0, "demo-chat", "Main")
    );
    assert_eq!(row.last_hit_at, None);
    // other kinds, languages and question ids are other rows
    for other in [
        key("q1", "explain", "en"),
        key("q1", "hint", "zh"),
        key("q2", "hint", "en"),
    ] {
        assert!(store
            .read(|c| shared::shared_get(c, &other))
            .expect("get")
            .is_none());
    }
    store
        .write(|tx| shared::shared_hit(tx, row.id, NOW + 10))
        .expect("hit");
    store
        .write(|tx| shared::shared_hit(tx, row.id, NOW + 20))
        .expect("hit");
    store
        .write(|tx| shared::shared_replace(tx, &k, &answer("regenerated", NOW + 30)))
        .expect("replace");
    let row = store
        .read(|c| shared::shared_get(c, &k))
        .expect("get")
        .expect("row");
    assert_eq!(
        (
            row.content.as_str(),
            row.gen,
            row.hits,
            row.updated_at,
            row.created_at,
            row.last_hit_at
        ),
        ("regenerated", 2, 2, NOW + 30, NOW, Some(NOW + 20))
    );
    // replace of a missing key stores it as the first one
    let k2 = key("q9", "explain", "ja");
    store
        .write(|tx| shared::shared_replace(tx, &k2, &answer("new", NOW + 40)))
        .expect("replace new");
    assert_eq!(
        store
            .read(|c| shared::shared_get(c, &k2))
            .expect("get")
            .expect("row")
            .gen,
        1
    );
    // rows, hits, rows hit at or after the day start
    assert_eq!(
        store
            .read(|c| shared::shared_stats(c, NOW + 15))
            .expect("stats"),
        (2, 2, 1)
    );
    assert_eq!(
        store
            .read(|c| shared::shared_stats(c, NOW + 100))
            .expect("stats"),
        (2, 2, 0)
    );
    assert_eq!(
        store.write(|tx| shared::shared_clear(tx)).expect("clear"),
        2
    );
    assert_eq!(
        store.read(|c| shared::shared_stats(c, 0)).expect("stats"),
        (0, 0, 0)
    );
}

#[test]
fn shared_pruning_by_age_then_by_cap_goes_by_last_use() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    store
        .write(|tx| {
            for i in 0..6 {
                let q = format!("q{i}");
                shared::shared_put_first(tx, &key(&q, "hint", "en"), &answer("x", 1000 + i))?;
            }
            // q0 was hit recently: its last use is newer than its creation
            let id = shared::shared_get(tx, &key("q0", "hint", "en"))?
                .expect("q0")
                .id;
            shared::shared_hit(tx, id, 5000)?;
            Ok::<_, rusqlite::Error>(())
        })
        .expect("fill");
    // older than 1003 by last use: q1 and q2 go (q0 was used at 5000)
    assert_eq!(
        store
            .write(|tx| shared::shared_prune(tx, 1003, 100))
            .expect("age"),
        2
    );
    // cap 2 of the 4 left: the two least recently used (q3, q4) go; q5 and the hit q0 stay
    assert_eq!(
        store
            .write(|tx| shared::shared_prune(tx, 0, 2))
            .expect("cap"),
        2
    );
    store
        .read(|c| {
            let mut left = Vec::new();
            for i in 0..6 {
                let q = format!("q{i}");
                if shared::shared_get(c, &key(&q, "hint", "en"))?.is_some() {
                    left.push(i);
                }
            }
            assert_eq!(left, vec![0, 5]);
            Ok(())
        })
        .expect("read");
    // a cap of 0 empties the table
    assert_eq!(
        store
            .write(|tx| shared::shared_prune(tx, 0, 0))
            .expect("zero"),
        2
    );
}

#[test]
fn the_v13_retention_pass_prunes_by_the_days_setting_and_sweeps_orphan_items() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let now = 1_800_000_000;
    store
        .write(|tx| {
            // studyShareDays is 180 by default: one row 200 days old, one 10 days old
            shared::shared_put_first(
                tx,
                &key("old", "hint", "en"),
                &answer("x", now - 200 * 86_400),
            )?;
            shared::shared_put_first(
                tx,
                &key("new", "hint", "en"),
                &answer("y", now - 10 * 86_400),
            )?;
            data::ensure_user(tx, &who(1), now)?;
            Ok::<_, rusqlite::Error>(())
        })
        .expect("fill");
    // two jobs: one running, one finished
    let (running, done) = store
        .write(|tx| {
            tx.execute(
                "INSERT INTO tasks (user_id, kind, state, run_after, created_at, updated_at)
                 VALUES (1, 'bank_gen', 'running', 1, 1, 1)",
                [],
            )?;
            let running = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO tasks (user_id, kind, state, run_after, created_at, updated_at)
                 VALUES (1, 'bank_gen', 'done', 1, 1, 1)",
                [],
            )?;
            let done = tx.last_insert_rowid();
            items::items_add(tx, running, 1, "{}", now)?;
            items::items_add(tx, done, 1, "{}", now)?;
            Ok::<_, rusqlite::Error>((running, done))
        })
        .expect("jobs");
    let (rows, it) = retention::sweep_v13(&store, now).expect("sweep");
    assert_eq!((rows, it), (1, 1));
    store
        .read(|c| {
            assert!(shared::shared_get(c, &key("old", "hint", "en"))?.is_none());
            assert!(shared::shared_get(c, &key("new", "hint", "en"))?.is_some());
            assert_eq!(items::items_after(c, running, 0, 10)?.len(), 1);
            assert!(items::items_after(c, done, 0, 10)?.is_empty());
            Ok(())
        })
        .expect("read");
    // a shorter keep period from the limits row is honoured
    store
        .write(|tx| {
            let l = Limits {
                study_share_days: 7,
                ..tables::load_limits(tx)?
            };
            tables::save_limits(tx, &l, "root", now)
        })
        .expect("limits");
    assert_eq!(retention::sweep_v13(&store, now).expect("sweep"), (1, 0));
}

// ---------------------------------------------------------------------------------------------------------------------
// Job items

#[test]
fn job_items_keep_their_numbers_and_are_read_in_arrival_order() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let task = store
        .write(|tx| {
            data::ensure_user(tx, &who(1), NOW)?;
            tx.execute(
                "INSERT INTO tasks (user_id, kind, state, run_after, created_at, updated_at)
                 VALUES (1, 'bank_gen', 'running', 1, 1, 1)",
                [],
            )?;
            Ok::<_, rusqlite::Error>(tx.last_insert_rowid())
        })
        .expect("task");
    let nos: Vec<i64> = store
        .write(|tx| {
            let mut v = Vec::new();
            for slot in [3, 1, 2] {
                v.push(items::items_add(
                    tx,
                    task,
                    slot,
                    &format!("{{\"slot\":{slot}}}"),
                    NOW,
                )?);
            }
            Ok::<_, rusqlite::Error>(v)
        })
        .expect("add");
    assert_eq!(nos, vec![1, 2, 3]);
    store
        .write(|tx| items::items_remove(tx, task, 2))
        .expect("remove");
    // a number is not reused while a later one exists
    assert_eq!(
        store
            .write(|tx| items::items_add(tx, task, 9, "{}", NOW))
            .expect("add"),
        4
    );
    let rows = store
        .read(|c| items::items_after(c, task, 1, 10))
        .expect("after");
    assert_eq!(
        rows.iter().map(|r| (r.no, r.slot)).collect::<Vec<_>>(),
        vec![(3, 2), (4, 9)]
    );
    assert_eq!(rows[0].data, "{\"slot\":2}");
    let limited = store
        .read(|c| items::items_after(c, task, 0, 2))
        .expect("limited");
    assert_eq!(limited.iter().map(|r| r.no).collect::<Vec<_>>(), vec![1, 3]);
    // removing the newest item never frees its number
    store
        .write(|tx| items::items_remove(tx, task, 4))
        .expect("remove newest");
    assert_eq!(
        store
            .write(|tx| items::items_add(tx, task, 5, "{}", NOW))
            .expect("add"),
        5
    );
    let tail = store
        .read(|c| items::items_after(c, task, 3, 10))
        .expect("tail");
    assert_eq!(tail.iter().map(|r| r.no).collect::<Vec<_>>(), vec![5]);
    store
        .write(|tx| items::items_clear(tx, task))
        .expect("clear");
    assert!(store
        .read(|c| items::items_after(c, task, 0, 10))
        .expect("none")
        .is_empty());
    // the items go with their task (cascade)
    store
        .write(|tx| items::items_add(tx, task, 1, "{}", NOW))
        .expect("add");
    store
        .write(|tx| {
            tx.execute("DELETE FROM tasks WHERE id = ?1", [task])
                .map(|_| ())
        })
        .expect("delete task");
    let left: i64 = store
        .read(|c| c.query_row("SELECT COUNT(*) FROM gen_items", [], |r| r.get(0)))
        .expect("count");
    assert_eq!(left, 0);
}

// ---------------------------------------------------------------------------------------------------------------------
// Statistics

fn usage_row(user: Option<i64>, at: i64) -> UsageRow {
    UsageRow {
        user_id: user,
        at,
        kind: "chat",
        provider: "openai",
        model: "demo-chat".to_string(),
        tokens_in: Some(100),
        tokens_out: Some(40),
        tokens_cached: None,
        tokens_cache_write: None,
        latency_ms: 10,
        first_byte_ms: None,
        status: "ok",
        error_code: None,
        connection_id: None,
        connection: None,
    }
}

fn gen_call(at: i64, cid: Option<i64>, name: Option<&str>) -> GenCallRow {
    GenCallRow {
        task_id: 0,
        user_id: 0,
        purpose: "generate",
        at,
        provider: "openai",
        model: "demo-large".to_string(),
        tokens_in: Some(1000),
        tokens_out: Some(500),
        tokens_cached: None,
        tokens_cache_write: None,
        latency_ms: 20,
        first_byte_ms: None,
        status: "ok",
        error_code: None,
        asked: 0,
        accepted: 0,
        truncated: false,
        connection_id: cid,
        connection: name.map(str::to_string),
    }
}

#[test]
fn statistics_split_by_connection_and_the_day_total_counts_both_tables() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    store
        .write(|tx| {
            // two connections and an old row without one
            for (cid, name, n) in [
                (Some(1), Some("Main"), 2),
                (Some(2), Some("Backup"), 1),
                (None, None, 1),
            ] {
                for _ in 0..n {
                    stats::insert_usage(
                        tx,
                        &UsageRow {
                            connection_id: cid,
                            connection: name.map(str::to_string),
                            ..usage_row(None, NOW)
                        },
                    )?;
                }
            }
            // a connection test is the "other" row of its connection
            stats::insert_usage(
                tx,
                &UsageRow {
                    kind: "verify",
                    connection_id: Some(1),
                    connection: Some("Main".to_string()),
                    ..usage_row(None, NOW)
                },
            )?;
            // a generation call on connection 1 (no user: a NULL user id is allowed)
            stats::insert_gen_call(tx, &gen_call(NOW, Some(1), Some("Main")))?;
            Ok::<_, rusqlite::Error>(())
        })
        .expect("fill");
    let day = xexam_ai_service::util::local_date(NOW);
    store
        .read(|c| {
            let by_conn = stats::by_connection(c, &day, &day)?;
            let view: Vec<(Option<i64>, i64, i64)> = by_conn
                .iter()
                .map(|r| (r.connection_id, r.sums.calls, r.sums.tokens_in))
                .collect();
            // Main: 2 chat + 1 verify + 1 generation; Backup: 1; the row without a connection comes last
            assert_eq!(
                view,
                vec![(Some(1), 4, 1000 + 300), (Some(2), 1, 100), (None, 1, 100)]
            );
            let total = stats::totals(c, &day, &day)?;
            assert_eq!(total.calls, 6);
            assert_eq!(
                by_conn.iter().map(|r| r.sums.calls).sum::<i64>(),
                total.calls
            );
            let by_model = stats::by_model_connection(c, &day, &day)?;
            assert_eq!(by_model.iter().map(|r| r.sums.calls).sum::<i64>(), 6);
            let main: Vec<(&str, i64)> = by_model
                .iter()
                .filter(|r| r.connection_id == Some(1))
                .map(|r| (r.model.as_str(), r.sums.calls))
                .collect();
            assert!(main.contains(&("demo-chat", 2)));
            assert!(main.contains(&("demo-large", 1)));
            assert!(
                main.contains(&("", 1)),
                "the connection test is the other row"
            );
            // the old by_model still adds up too
            assert_eq!(
                stats::by_model(c, &day, &day)?
                    .iter()
                    .map(|r| r.1.calls)
                    .sum::<i64>(),
                6
            );
            // tokens of the day: input + output, usage and gen_calls
            assert_eq!(stats::tokens_today(c, &day)?, 5 * 140 + 1500);
            assert_eq!(stats::tokens_today(c, "1999-01-01")?, 0);
            Ok(())
        })
        .expect("read");
}

#[test]
fn tokens_today_ignores_missing_counts() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    store
        .write(|tx| {
            stats::insert_usage(
                tx,
                &UsageRow {
                    tokens_in: None,
                    tokens_out: Some(7),
                    ..usage_row(None, NOW)
                },
            )
        })
        .expect("usage");
    assert_eq!(
        store
            .read(|c| stats::tokens_today(c, &xexam_ai_service::util::local_date(NOW)))
            .expect("tokens"),
        7
    );
}

// ---------------------------------------------------------------------------------------------------------------------
// Limits

#[test]
fn all_33_limits_round_trip_through_their_own_columns() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let mut m = serde_json::Map::new();
    for (i, d) in DEFS.iter().enumerate() {
        // a distinct in-range value per limit; a mix-up of columns changes at least one
        let v = d.min + (i64::try_from(i).expect("i") * 7 + 1).min(d.max - d.min);
        m.insert(d.key.to_string(), serde_json::Value::from(v));
    }
    let l = Limits::default().apply_update(&m).expect("in range");
    store
        .write(|tx| tables::save_limits(tx, &l, "root", NOW))
        .expect("save");
    let back = store.read(tables::load_limits).expect("load");
    assert_eq!(back, l);
    for d in &DEFS {
        assert_eq!(back.get(d.key), l.get(d.key), "{}", d.key);
    }
    assert_eq!(DEFS.len(), 33);
}

#[test]
fn a_fresh_file_reads_the_default_limits() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    assert_eq!(
        store.read(tables::load_limits).expect("load"),
        Limits::default()
    );
}

// —— hint + analysis: one switch ————————————————————————————————————————————————————————————————————————————————————

fn feature_bits(dir: &Path) -> Vec<(i64, i64)> {
    let c = Connection::open(dir.join(DB_FILE)).expect("db");
    let mut st = c
        .prepare("SELECT id, features_off FROM users WHERE id >= 100 ORDER BY id")
        .expect("prepare");
    let rows = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("rows");
    rows
}

fn add_feature_users(dir: &Path, bits: &[i64]) {
    let c = Connection::open(dir.join(DB_FILE)).expect("db");
    for (i, b) in bits.iter().enumerate() {
        c.execute(
            "INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at, features_off)
             VALUES (?1, ?2, ?3, 1, 1, ?4)",
            rusqlite::params![100 + i as i64, format!("stamp-{i}"), format!("demo-{i}"), b],
        )
        .expect("user");
    }
}

#[test]
fn the_upgrade_to_7_pairs_hint_and_explain_and_nobody_loses_access() {
    let dir = tempfile::tempdir().expect("dir");
    write_v6(dir.path(), false);
    // hint only off, explain only off, both off, hint + summary off, nothing off, explain + generate off, all off
    add_feature_users(dir.path(), &[2, 4, 6, 2 | 8, 0, 4 | 16, 63]);
    drop(open(dir.path()));
    assert_eq!(
        feature_bits(dir.path()),
        vec![
            (100, 0),
            (101, 0),
            (102, 6),
            (103, 8),
            (104, 0),
            (105, 16),
            (106, 63)
        ]
    );
}

#[test]
fn a_schema_7_file_with_split_hint_and_explain_bits_is_paired_at_the_next_start() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    add_feature_users(dir.path(), &[2, 6, 4 | 1]);
    Connection::open(dir.path().join(DB_FILE))
        .expect("db")
        .execute("UPDATE settings SET features_off = 4 | 32 WHERE id = 1", [])
        .expect("global");
    let store = open(dir.path());
    assert_eq!(feature_bits(dir.path()), vec![(100, 0), (101, 6), (102, 1)]);
    assert_eq!(store.read(conn::global_features_off).expect("read"), 32);
}

// —— v1.3 reasoning columns ——————————————————————————————————————————————————————————————————————————————————————————

/// (reasoning, reasoning_ms, reasoning_truncated) of a message, read straight from the file.
fn raw_reasoning(c: &Connection, id: i64) -> (Option<String>, Option<i64>, i64) {
    c.query_row(
        "SELECT reasoning, reasoning_ms, reasoning_truncated FROM messages WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .expect("reasoning")
}

#[test]
fn a_schema_7_file_without_the_reasoning_columns_gets_them_on_open() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let chat = new_chat(&store);
    let (q, a) = store
        .write(|tx| {
            let q = msg(tx, chat, "user", "question", None);
            let a = msg(tx, chat, "assistant", "answer", Some(q));
            Ok::<_, rusqlite::Error>((q, a))
        })
        .expect("rows");
    drop(store);
    // a schema-7 file of an earlier v1.3 build: the three columns are not there yet
    let c = Connection::open(dir.path().join(DB_FILE)).expect("open");
    for col in ["reasoning", "reasoning_ms", "reasoning_truncated"] {
        c.execute_batch(&format!("ALTER TABLE messages DROP COLUMN {col}"))
            .expect("drop");
    }
    drop(c);
    drop(open(dir.path()));
    let c = Connection::open(dir.path().join(DB_FILE)).expect("open");
    for col in ["reasoning", "reasoning_ms", "reasoning_truncated"] {
        let present: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name = ?1)",
                [col],
                |r| r.get(0),
            )
            .expect("pragma");
        assert!(present, "{col}");
    }
    // the rows are intact and read as "no reasoning"
    let content: String = c
        .query_row("SELECT content FROM messages WHERE id = ?1", [a], |r| {
            r.get(0)
        })
        .expect("content");
    assert_eq!(content, "answer");
    assert_eq!(raw_reasoning(&c, a), (None, None, 0));
    assert_eq!(raw_reasoning(&c, q), (None, None, 0));
    // no backup, no version change
    assert!(!dir.path().join("backups").exists());
    let v: String = c
        .query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
            r.get(0)
        })
        .expect("schema");
    assert_eq!(v, "7");
}

#[test]
fn set_reasoning_trims_caps_and_clears() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let chat = new_chat(&store);
    let a = store
        .write(|tx| {
            let q = msg(tx, chat, "user", "question", None);
            Ok::<_, rusqlite::Error>(msg(tx, chat, "assistant", "answer", Some(q)))
        })
        .expect("rows");
    let read = |store: &Store| {
        store
            .read(|c| Ok::<_, rusqlite::Error>(raw_reasoning(c, a)))
            .expect("read")
    };
    // trimmed on both ends, the truncation flag is passed on
    store
        .write(|tx| data::set_reasoning(tx, a, "  \n think it over \t\n", Some(1200), true))
        .expect("set");
    assert_eq!(
        read(&store),
        (Some("think it over".to_string()), Some(1200), 1)
    );
    // whitespace only: all three columns are cleared
    store
        .write(|tx| data::set_reasoning(tx, a, " \n\t ", Some(5), true))
        .expect("set");
    assert_eq!(read(&store), (None, None, 0));
    // 40 000 bytes of multibyte text: the head up to 32 KiB, cut on a char boundary, flagged without the caller's flag
    let long = "é思".repeat(8_000);
    assert!(long.len() > 39_000);
    store
        .write(|tx| data::set_reasoning(tx, a, &long, Some(7), false))
        .expect("set");
    let (text, ms, cut) = read(&store);
    let text = text.expect("kept");
    assert!(text.len() <= data::MAX_REASONING_BYTES, "{}", text.len());
    assert!(text.len() > data::MAX_REASONING_BYTES - 4);
    assert!(long.starts_with(&text), "the head is kept");
    assert_eq!((ms, cut), (Some(7), 1));
    // flush_answer and finish_answer leave the reasoning alone
    store
        .write(|tx| {
            data::flush_answer(tx, a, "partial")?;
            data::finish_answer(tx, a, "final", "done", None, false, NOW)
        })
        .expect("flush and finish");
    let (text2, ms2, cut2) = read(&store);
    assert_eq!(
        (text2.as_deref(), ms2, cut2),
        (Some(text.as_str()), Some(7), 1)
    );
    // reset_answer (the in-place retry) clears all three
    store
        .write(|tx| data::reset_answer(tx, a, "demo-chat", false))
        .expect("reset");
    assert_eq!(read(&store), (None, None, 0));
    // cap_head itself
    assert_eq!(data::cap_head("abc", 3), ("abc", false));
    assert_eq!(data::cap_head("abcd", 3), ("abc", true));
    assert_eq!(data::cap_head("é思", 2), ("é", true));
    assert_eq!(data::cap_head("é思", 1), ("", true));
}
