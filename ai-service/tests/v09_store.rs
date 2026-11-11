//! v0.9 storage (contracts 4 and 5): AI database schema 3 (fresh and upgraded files, backups, plans), the study chats
//! and their ask transaction, saved lookups, usage by feature and the batched retention sweep. Store level only; all
//! content is made up.
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use xexam_ai_service::config::JournalMode;
use xexam_ai_service::identity::{Identity, Role};
use xexam_ai_service::store::data::{self, StudyError, StudyInsert, StudySample};
use xexam_ai_service::store::stats::{self, UsageRow};
use xexam_ai_service::store::{schema, Store, DB_FILE};

const NOW: i64 = 1_800_000_000;
const KEY32: &str = "0123456789abcdef0123456789abcdef";

fn who(id: i64) -> Identity {
    Identity {
        id,
        stamp: format!("{:032x}", 0x5000 + id),
        username: format!("user{id}"),
        role: Role::User,
        ai_allowed: true,
        sid: "0".repeat(64),
    }
}

fn open(dir: &Path) -> Store {
    Store::open(dir, JournalMode::Delete).expect("open store")
}

fn schema_of(c: &Connection) -> String {
    c.query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
        r.get(0)
    })
    .expect("schema")
}

fn has_column(c: &Connection, table: &str, column: &str) -> bool {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
        [table, column],
        |r| r.get(0),
    )
    .expect("table info")
}

fn index_exists(c: &Connection, name: &str) -> bool {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
        [name],
        |r| r.get(0),
    )
    .expect("index")
}

fn master_rows(path: &Path) -> Vec<(String, String, Option<String>)> {
    let c = Connection::open(path).expect("open");
    let mut st = c
        .prepare("SELECT type, name, sql FROM sqlite_master ORDER BY type, name")
        .expect("prepare");
    let rows = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
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

/// A schema-1 (`version` "1") or schema-2 file with a user, a chat with two messages and a usage row.
fn write_old_fixture(dir: &Path, version: &str, with_rows: bool) {
    let c = Connection::open(dir.join(DB_FILE)).expect("fixture");
    c.execute_batch(schema::SCHEMA).expect("v1 schema");
    if version == "2" {
        for (table, _, def) in schema::V2_COLUMNS {
            c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {def};"))
                .expect("v2 column");
        }
    }
    c.execute(
        "INSERT INTO settings (id, updated_at) VALUES (1, ?1)",
        [NOW],
    )
    .expect("settings");
    c.execute("INSERT INTO limits (id, updated_at) VALUES (1, ?1)", [NOW])
        .expect("limits");
    c.execute(
        "INSERT INTO meta (key, value) VALUES ('schema', ?1)",
        [version],
    )
    .expect("meta");
    if with_rows {
        c.execute(
            "INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at) VALUES (5, 'aa', 'sam', ?1, ?1)",
            [NOW],
        )
        .expect("user");
        c.execute(
            "INSERT INTO chats (user_id, title, created_at, updated_at) VALUES (5, 'An old chat', ?1, ?1)",
            [NOW],
        )
        .expect("chat");
        let chat = c.last_insert_rowid();
        for (role, text) in [("user", "an old question"), ("assistant", "an old answer")] {
            c.execute(
                "INSERT INTO messages (chat_id, role, content, created_at, finished_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                params![chat, role, text, NOW],
            )
            .expect("message");
        }
        c.execute(
            "INSERT INTO usage (user_id, at, day, kind, provider, model, latency_ms, status)
             VALUES (5, ?1, '2026-11-12', 'chat', 'openai', 'demo-model', 12, 'ok')",
            [NOW],
        )
        .expect("usage");
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Schema 3

#[test]
fn a_fresh_database_is_schema_3_with_every_column_and_index() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    store
        .read(|c| {
            assert_eq!(schema_of(c), "3");
            for (table, column, _) in schema::V2_COLUMNS.iter().chain(schema::V3_COLUMNS.iter()) {
                assert!(has_column(c, table, column), "{table}.{column}");
            }
            for name in [
                "idx_chats_ref",
                "idx_messages_study",
                "idx_messages_streaming",
            ] {
                assert!(index_exists(c, name), "{name}");
            }
            // the columns the review dropped
            assert!(!has_column(c, "messages", "record_id"));
            assert!(!has_column(c, "chats", "free"));
            assert!(!index_exists(c, "idx_usage_purpose"));
            Ok(())
        })
        .expect("read");
    assert!(backups(dir.path()).is_empty());
}

fn upgraded_from(version: &str, with_rows: bool) {
    let dir = tempfile::tempdir().expect("dir");
    write_old_fixture(dir.path(), version, with_rows);
    let store = open(dir.path());
    let copies = backups(dir.path());
    assert_eq!(copies.len(), 1, "{copies:?}");
    let name = copies[0]
        .file_name()
        .and_then(|n| n.to_str())
        .expect("name");
    assert!(
        name.starts_with(&format!("xexam-ai-schema{version}-")) && name.ends_with(".db"),
        "{name}"
    );
    // the copy is the old database as it was
    let copy = Connection::open(&copies[0]).expect("copy");
    assert_eq!(schema_of(&copy), version);
    assert!(!has_column(&copy, "chats", "kind"));
    drop(copy);
    store
        .read(|c| {
            assert_eq!(schema_of(c), "3");
            for (table, column, _) in schema::V2_COLUMNS.iter().chain(schema::V3_COLUMNS.iter()) {
                assert!(has_column(c, table, column), "{table}.{column}");
            }
            for name in ["idx_chats_ref", "idx_messages_study", "idx_messages_streaming"] {
                assert!(index_exists(c, name), "{name}");
            }
            let ok: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
            assert_eq!(ok, "ok");
            if with_rows {
                // old rows keep their content and get the defaults
                let kinds: Vec<(String, Option<String>, Option<i64>)> = {
                    let mut st = c.prepare("SELECT kind, ref, record_id FROM chats")?;
                    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                    rows.collect::<Result<_, _>>()?
                };
                assert_eq!(kinds, vec![("chat".to_string(), None, None)]);
                let n: i64 = c.query_row(
                    "SELECT COUNT(*) FROM messages WHERE kind = 'chat' AND question_id IS NULL AND source IS NULL
                     AND lang IS NULL AND sample_used IS NULL",
                    [],
                    |r| r.get(0),
                )?;
                assert_eq!(n, 2);
                let purposes: i64 =
                    c.query_row("SELECT COUNT(*) FROM usage WHERE purpose IS NULL", [], |r| {
                        r.get(0)
                    })?;
                assert_eq!(purposes, 1);
                let titles = data::list_chats(c, 5)?;
                assert_eq!(titles[0].title, "An old chat");
                assert_eq!(titles[0].kind, "chat");
            }
            Ok(())
        })
        .expect("read");
    // reopening makes no second copy
    drop(store);
    let _ = open(dir.path());
    assert_eq!(backups(dir.path()).len(), 1);
}

#[test]
fn a_schema_1_file_is_backed_up_and_upgraded() {
    upgraded_from("1", true);
}

#[test]
fn a_schema_2_file_is_backed_up_and_upgraded() {
    upgraded_from("2", true);
}

#[test]
fn an_empty_schema_2_file_is_backed_up_too() {
    upgraded_from("2", false);
}

#[test]
fn a_failed_backup_refuses_the_upgrade_and_changes_nothing() {
    let dir = tempfile::tempdir().expect("dir");
    write_old_fixture(dir.path(), "2", true);
    // `backups` is a file: the copy cannot be written
    std::fs::write(dir.path().join("backups"), b"not a directory").expect("file");
    let path = dir.path().join(DB_FILE);
    let before = std::fs::read(&path).expect("read");
    let err = match Store::open(dir.path(), JournalMode::Delete) {
        Ok(_) => panic!("the upgrade must not run without a copy"),
        Err(e) => e,
    };
    assert!(
        err.starts_with("cannot back up the AI database before the upgrade (")
            && err.ends_with("); nothing was changed"),
        "{err}"
    );
    assert!(before == std::fs::read(&path).expect("read"));
    let c = Connection::open(&path).expect("open");
    assert_eq!(schema_of(&c), "2");
}

#[test]
fn fresh_and_upgraded_databases_have_the_same_master_table() {
    let fresh = tempfile::tempdir().expect("dir");
    drop(open(fresh.path()));
    let fresh_rows = master_rows(&fresh.path().join(DB_FILE));
    for version in ["1", "2"] {
        let dir = tempfile::tempdir().expect("dir");
        write_old_fixture(dir.path(), version, true);
        drop(open(dir.path()));
        assert_eq!(
            fresh_rows,
            master_rows(&dir.path().join(DB_FILE)),
            "from schema {version}"
        );
    }
}

#[test]
fn reopening_a_current_database_writes_nothing() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    let path = dir.path().join(DB_FILE);
    let bytes = std::fs::read(&path).expect("read");
    let mtime = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .expect("mtime");
    drop(open(dir.path()));
    assert!(bytes == std::fs::read(&path).expect("read"));
    assert_eq!(
        mtime,
        std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .expect("mtime")
    );
    assert!(backups(dir.path()).is_empty());
}

#[test]
fn a_newer_schema_is_refused_with_the_schema_3_line() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    let path = dir.path().join(DB_FILE);
    {
        let c = Connection::open(&path).expect("open");
        c.execute("UPDATE meta SET value = '4' WHERE key = 'schema'", [])
            .expect("set");
    }
    let err = match Store::open(dir.path(), JournalMode::Delete) {
        Ok(_) => panic!("schema 4 must be refused"),
        Err(e) => e,
    };
    assert_eq!(
        err,
        format!(
            "The AI database {} has schema 4, written by a newer ai-service; this version (schema 3) cannot open it. Run the newer version, or restore a backup.",
            path.display()
        )
    );
}

#[test]
fn the_start_up_repair_marks_streaming_answers_as_interrupted() {
    let dir = tempfile::tempdir().expect("dir");
    {
        let store = open(dir.path());
        store
            .write(|tx| {
                data::ensure_user(tx, &who(1), NOW)?;
                let chat = data::create_chat(tx, 1, "t", NOW)?;
                data::insert_message(
                    tx,
                    &data::NewMessage {
                        chat,
                        role: "assistant",
                        content: "",
                        status: "streaming",
                        model: Some("m"),
                        free: false,
                        now: NOW,
                    },
                )?;
                Ok::<(), rusqlite::Error>(())
            })
            .expect("write");
    }
    let store = open(dir.path());
    let (status, code): (String, String) = store
        .read(|c| {
            c.query_row("SELECT status, error_code FROM messages", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
        })
        .expect("row");
    assert_eq!(
        (status.as_str(), code.as_str()),
        ("error", "ai.interrupted")
    );
}

// ---------------------------------------------------------------------------------------------------------------------
// Query plans

fn plan(c: &Connection, sql: &str, p: &[&dyn rusqlite::ToSql]) -> String {
    let mut st = c
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("explain");
    let rows = st
        .query_map(p, |r| r.get::<_, String>(3))
        .expect("plan rows");
    rows.collect::<Result<Vec<_>, _>>()
        .expect("rows")
        .join(" | ")
}

fn seed_generated(c: &Connection) -> i64 {
    // two users, a study chat and other chats, with a few hundred messages
    c.execute_batch(
        "INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at) VALUES (1, 'a', 'u1', 1, 1);",
    )
    .expect("user");
    c.execute(
        "INSERT INTO chats (user_id, title, created_at, updated_at, kind, ref) VALUES (1, 't', 1, 1, 'practice', ?1)",
        [KEY32],
    )
    .expect("chat");
    let chat = c.last_insert_rowid();
    for q in 0..20 {
        for (i, kind) in ["hint", "explain"].iter().enumerate() {
            for (source, status) in [
                ("practice", "done"),
                ("result", "done"),
                ("practice", "error"),
            ] {
                for role in ["user", "assistant"] {
                    c.execute(
                        "INSERT INTO messages (chat_id, role, content, status, created_at, kind, question_id, question_no, source, lang)
                         VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?8, 'en')",
                        params![
                            chat,
                            role,
                            if status == "error" { "" } else { "text" },
                            if role == "user" { "done" } else { status },
                            kind,
                            format!("q{q}"),
                            q as i64 + i as i64,
                            source
                        ],
                    )
                    .expect("message");
                }
            }
        }
    }
    chat
}

#[test]
fn the_study_lookups_use_the_covering_indexes_without_sorting() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    store
        .read(|c| {
            let chat = seed_generated(c);
            for sql in [data::SQL_STUDY_LATEST, data::SQL_STUDY_LAST_GOOD] {
                let p = plan(
                    c,
                    sql,
                    &[&chat as &dyn rusqlite::ToSql, &"q3", &"hint", &"practice"],
                );
                assert!(p.contains("idx_messages_study"), "{p}");
                assert!(!p.contains("TEMP B-TREE"), "{p}");
            }
            let p = plan(
                c,
                data::SQL_CHAT_BY_REF,
                &[&1_i64 as &dyn rusqlite::ToSql, &"practice", &KEY32],
            );
            assert!(p.contains("idx_chats_ref"), "{p}");
            for sql in [data::SQL_SUMMARY_LATEST, data::SQL_SUMMARY_LAST_GOOD] {
                let p = plan(c, sql, &[&chat as &dyn rusqlite::ToSql]);
                assert!(p.contains("idx_messages_chat"), "{p}");
                assert!(!p.contains("TEMP B-TREE"), "{p}");
            }
            let p = plan(
                c,
                "SELECT COUNT(*) FROM messages WHERE status = 'streaming'",
                &[],
            );
            assert!(p.contains("idx_messages_streaming"), "{p}");
            Ok(())
        })
        .expect("read");
}

#[test]
fn the_indexed_lookup_equals_a_not_indexed_scan_on_generated_rows() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    store
        .read(|c| {
            let chat = seed_generated(c);
            for q in ["q0", "q7", "q19", "q99"] {
                for kind in ["hint", "explain"] {
                    for source in ["practice", "result"] {
                        let indexed: Option<i64> = c
                            .query_row(
                                data::SQL_STUDY_LATEST,
                                params![chat, q, kind, source],
                                |r| r.get(0),
                            )
                            .ok();
                        let scan: Option<i64> = c
                            .query_row(
                                "SELECT id FROM messages NOT INDEXED WHERE chat_id = ?1 AND question_id = ?2 AND kind = ?3
                                 AND source = ?4 AND role = 'assistant' ORDER BY id DESC LIMIT 1",
                                params![chat, q, kind, source],
                                |r| r.get(0),
                            )
                            .ok();
                        assert_eq!(indexed, scan, "{q} {kind} {source}");
                        let indexed: Option<i64> = c
                            .query_row(
                                data::SQL_STUDY_LAST_GOOD,
                                params![chat, q, kind, source],
                                |r| r.get(0),
                            )
                            .ok();
                        let scan: Option<i64> = c
                            .query_row(
                                "SELECT id FROM messages NOT INDEXED WHERE chat_id = ?1 AND question_id = ?2 AND kind = ?3
                                 AND source = ?4 AND role = 'assistant' AND status = 'done' AND content <> ''
                                 ORDER BY id DESC LIMIT 1",
                                params![chat, q, kind, source],
                                |r| r.get(0),
                            )
                            .ok();
                        assert_eq!(indexed, scan, "good {q} {kind} {source}");
                    }
                }
            }
            Ok(())
        })
        .expect("read");
}

// ---------------------------------------------------------------------------------------------------------------------
// prepare_study

fn insert<'a>(kind: &'a str, chat_ref: &'a str) -> StudyInsert<'a> {
    StudyInsert {
        chat_kind: "practice",
        chat_ref,
        title: "2026-11-11 21:58",
        record_id: None,
        kind,
        question_id: Some("q1"),
        question_no: Some(3),
        source: Some("practice"),
        lang: "en",
        sample: None,
        user_content: "Stem?\n\nA. one\nB. two",
        model: "demo-model",
        now: NOW,
        chats_per_user: 1000,
        messages_per_chat: 400,
    }
}

fn run(store: &Store, user: i64, s: &StudyInsert<'_>) -> Result<data::StudyIds, StudyError> {
    store.write(|tx| data::prepare_study(tx, &who(user), s, &mut |_| true))
}

fn counts(store: &Store) -> (i64, i64) {
    store
        .read(|c| {
            c.query_row(
                "SELECT (SELECT COUNT(*) FROM chats), (SELECT COUNT(*) FROM messages)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .expect("counts")
}

#[test]
fn a_new_study_chat_gets_the_pair_with_every_column() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let mut s = insert("explain", KEY32);
    s.source = Some("result");
    s.record_id = Some(42);
    s.lang = "zh-CN";
    s.chat_kind = "exam";
    let ids = run(&store, 7, &s).expect("ask");
    assert!(ids.new_chat);
    assert_eq!(ids.title, "2026-11-11 21:58");
    assert_eq!(ids.record_id, Some(42));
    assert_ne!(ids.user_msg, ids.answer);
    store
        .read(|c| {
            let chat = data::chat(c, 7, ids.chat)?.expect("chat");
            assert_eq!((chat.kind.as_str(), chat.record_id), ("exam", Some(42)));
            assert_eq!(chat.message_count, 2);
            let chat_ref: String =
                c.query_row("SELECT ref FROM chats WHERE id = ?1", [ids.chat], |r| r.get(0))?;
            assert_eq!(chat_ref, KEY32);
            let m = data::messages(c, ids.chat)?;
            assert_eq!(m.len(), 2);
            let (u, a) = (&m[0], &m[1]);
            assert_eq!((u.id, a.id), (ids.user_msg, ids.answer));
            assert_eq!((u.role.as_str(), u.status.as_str()), ("user", "done"));
            assert_eq!((a.role.as_str(), a.status.as_str()), ("assistant", "streaming"));
            assert_eq!(u.content, "Stem?\n\nA. one\nB. two");
            assert_eq!(u.model, None);
            assert_eq!(a.model.as_deref(), Some("demo-model"));
            for x in [u, a] {
                assert_eq!(x.kind, "explain");
                assert_eq!(x.question_id.as_deref(), Some("q1"));
                assert_eq!(x.question_no, Some(3));
                assert_eq!(x.source.as_deref(), Some("result"));
                assert_eq!(x.lang.as_deref(), Some("zh-CN"));
                assert_eq!(x.record_id, Some(42));
                assert_eq!(x.sample, None);
                assert!(!x.free);
            }
            let finished: (Option<i64>, Option<i64>) = c.query_row(
                "SELECT (SELECT finished_at FROM messages WHERE id = ?1), (SELECT finished_at FROM messages WHERE id = ?2)",
                [ids.user_msg, ids.answer],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            assert_eq!(finished, (Some(NOW), None));
            Ok(())
        })
        .expect("read");
}

#[test]
fn the_same_ref_finds_the_chat_and_another_kind_makes_another() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let a = run(&store, 1, &insert("hint", KEY32)).expect("first");
    let mut s2 = insert("explain", KEY32);
    s2.title = "ignored: the chat exists";
    s2.now = NOW + 5;
    let b = run(&store, 1, &s2).expect("second");
    assert!(a.new_chat && !b.new_chat);
    assert_eq!(a.chat, b.chat);
    assert_eq!(b.title, "2026-11-11 21:58");
    // the same ref under another chat kind (an exam) is another chat
    let mut s3 = insert("explain", KEY32);
    s3.chat_kind = "exam";
    s3.source = Some("result");
    let c = run(&store, 1, &s3).expect("third");
    assert!(c.new_chat);
    assert_ne!(c.chat, a.chat);
    // the same ref of another user is another chat
    let d = run(&store, 2, &insert("hint", KEY32)).expect("other user");
    assert!(d.new_chat);
    assert_ne!(d.chat, a.chat);
    assert_eq!(counts(&store), (3, 8));
    store
        .read(|c| {
            assert_eq!(data::chat_by_ref(c, 1, "practice", KEY32)?, Some(a.chat));
            assert_eq!(
                data::chat_by_ref(c, 1, "exam", KEY32)?,
                Some(c_chat(c, 1, "exam"))
            );
            assert_eq!(data::chat_by_ref(c, 3, "practice", KEY32)?, None);
            assert_eq!(
                data::chat_by_ref(c, 1, "practice", "ffffffffffffffffffffffffffffffff")?,
                None
            );
            Ok(())
        })
        .expect("read");
}

fn c_chat(c: &Connection, user: i64, kind: &str) -> i64 {
    c.query_row(
        "SELECT id FROM chats WHERE user_id = ?1 AND kind = ?2",
        params![user, kind],
        |r| r.get(0),
    )
    .expect("chat")
}

#[test]
fn the_chat_limit_applies_only_to_a_new_chat() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let mut s = insert("hint", KEY32);
    s.chats_per_user = 1;
    run(&store, 1, &s).expect("first chat");
    // the existing chat is still reachable at the limit
    run(&store, 1, &s).expect("same chat");
    // a second chat is refused
    let mut other = insert("hint", "ffffffffffffffffffffffffffffffff");
    other.chats_per_user = 1;
    match run(&store, 1, &other) {
        Err(StudyError::ChatLimit(1)) => {}
        r => panic!("{r:?}"),
    }
    assert_eq!(counts(&store), (1, 4));
}

#[test]
fn the_message_limit_counts_the_pair() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let mut s = insert("hint", KEY32);
    s.messages_per_chat = 4;
    run(&store, 1, &s).expect("pair 1");
    run(&store, 1, &s).expect("pair 2");
    match run(&store, 1, &s) {
        Err(StudyError::MessageLimit(4)) => {}
        r => panic!("{r:?}"),
    }
    assert_eq!(counts(&store), (1, 4));
}

#[test]
fn a_refused_bind_writes_nothing_in_either_branch() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    // a new chat
    let s = insert("hint", KEY32);
    let r = store.write(|tx| data::prepare_study(tx, &who(1), &s, &mut |_| false));
    assert!(matches!(r, Err(StudyError::Busy)), "{r:?}");
    assert_eq!(counts(&store), (0, 0));
    // an existing chat
    run(&store, 1, &s).expect("pair");
    let mut seen = Vec::new();
    let r = store.write(|tx| {
        data::prepare_study(tx, &who(1), &s, &mut |chat| {
            seen.push(chat);
            false
        })
    });
    assert!(matches!(r, Err(StudyError::Busy)), "{r:?}");
    assert_eq!(seen.len(), 1);
    assert_eq!(counts(&store), (1, 2));
}

#[test]
fn the_record_id_is_set_once_and_never_overwritten() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let first = run(&store, 1, &insert("hint", KEY32)).expect("no record yet");
    assert_eq!(first.record_id, None);
    let mut s = insert("explain", KEY32);
    s.source = Some("result");
    s.record_id = Some(10);
    assert_eq!(run(&store, 1, &s).expect("with record").record_id, Some(10));
    s.record_id = Some(99);
    assert_eq!(
        run(&store, 1, &s).expect("other record").record_id,
        Some(10)
    );
}

#[test]
fn a_summary_chat_stores_the_sample_and_the_record() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let s = StudyInsert {
        chat_kind: "summary",
        chat_ref: "r42@2026-11-11 21:58",
        title: "2026-11-11 21:58",
        record_id: Some(42),
        kind: "summary",
        question_id: None,
        question_no: None,
        source: None,
        lang: "ja",
        sample: Some(StudySample {
            used: 70,
            usable: 80,
            wrong: 90,
        }),
        user_content: "[AI summary] 2026-11-11 21:58 practice 70/80/90",
        model: "demo-model",
        now: NOW,
        chats_per_user: 1000,
        messages_per_chat: 400,
    };
    let ids = run(&store, 1, &s).expect("summary");
    store
        .read(|c| {
            let m = data::messages(c, ids.chat)?;
            for x in &m {
                assert_eq!(x.kind, "summary");
                assert_eq!(x.record_id, Some(42));
                assert_eq!(
                    x.sample,
                    Some(StudySample {
                        used: 70,
                        usable: 80,
                        wrong: 90
                    })
                );
                assert_eq!(
                    x.to_json()["sample"],
                    json!({"used": 70, "usable": 80, "wrong": 90})
                );
            }
            Ok(())
        })
        .expect("read");
}

// ---------------------------------------------------------------------------------------------------------------------
// Saved lookups, running info, JSON

fn finish(store: &Store, msg: i64, content: &str, status: &str, code: Option<&str>) {
    store
        .write(|tx| {
            data::finish_answer(tx, msg, content, status, code, false, NOW + 10)?;
            Ok::<(), rusqlite::Error>(())
        })
        .expect("finish");
}

fn ask(
    store: &Store,
    user: i64,
    kind: &str,
    q: &str,
    source: &str,
    chat_kind: &str,
    now: i64,
) -> data::StudyIds {
    let mut s = insert(kind, KEY32);
    s.chat_kind = chat_kind;
    s.question_id = Some(q);
    s.source = Some(source);
    if source == "result" {
        s.record_id = Some(5);
    }
    s.now = now;
    run(store, user, &s).expect("ask")
}

#[test]
fn saved_returns_the_newest_answer_and_the_last_good_only_when_needed() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    // nothing yet
    let none = store
        .read(|c| data::study_saved(c, 1, "practice", KEY32, "hint", "q1", "practice"))
        .expect("none");
    assert_eq!(none.chat_id, None);
    assert!(none.latest.is_none() && none.last_good.is_none());
    assert_eq!(
        none.to_json(false),
        json!({"chatId": null, "latest": null, "lastGood": null})
    );

    let first = ask(&store, 1, "hint", "q1", "practice", "practice", NOW);
    // the chat exists, the answer is still streaming: latest only
    let s = store
        .read(|c| data::study_saved(c, 1, "practice", KEY32, "hint", "q1", "practice"))
        .expect("streaming");
    assert_eq!(s.chat_id, Some(first.chat));
    assert_eq!(
        s.latest.as_ref().map(|a| a.status.as_str()),
        Some("streaming")
    );
    assert!(s.last_good.is_none());

    finish(&store, first.answer, "first hint", "done", None);
    let s = store
        .read(|c| data::study_saved(c, 1, "practice", KEY32, "hint", "q1", "practice"))
        .expect("done");
    assert_eq!(
        s.latest.as_ref().map(|a| a.content.as_str()),
        Some("first hint")
    );
    assert!(s.last_good.is_none(), "latest is done: no second copy");

    // a regenerate that fails: latest is the error, lastGood the first hint
    let second = ask(&store, 1, "hint", "q1", "practice", "practice", NOW + 20);
    finish(&store, second.answer, "", "error", Some("ai.up.timeout"));
    let s = store
        .read(|c| data::study_saved(c, 1, "practice", KEY32, "hint", "q1", "practice"))
        .expect("failed");
    let latest = s.latest.clone().expect("latest");
    assert_eq!(
        (latest.status.as_str(), latest.error_code.as_deref()),
        ("error", Some("ai.up.timeout"))
    );
    assert_eq!(latest.message_id, second.answer);
    assert_eq!(
        s.last_good.as_ref().map(|a| a.content.as_str()),
        Some("first hint")
    );
    let j = s.to_json(false);
    assert_eq!(j["latest"]["error"], json!("ai.up.timeout"));
    assert_eq!(
        j["lastGood"]
            .as_object()
            .map(|o| o.keys().cloned().collect::<Vec<_>>()),
        Some(
            [
                "messageId",
                "kind",
                "questionId",
                "questionNo",
                "source",
                "status",
                "error",
                "truncated",
                "content",
                "lang",
                "model",
                "createdAt",
                "finishedAt"
            ]
            .iter()
            .map(|s| s.to_string())
            .collect()
        )
    );

    // a stopped regenerate with text is not "good"; the newest done answer with content wins
    let third = ask(&store, 1, "hint", "q1", "practice", "practice", NOW + 30);
    finish(&store, third.answer, "partial", "stopped", None);
    let s = store
        .read(|c| data::study_saved(c, 1, "practice", KEY32, "hint", "q1", "practice"))
        .expect("stopped");
    assert_eq!(
        s.latest.as_ref().map(|a| a.status.as_str()),
        Some("stopped")
    );
    assert_eq!(
        s.last_good.as_ref().map(|a| a.content.as_str()),
        Some("first hint")
    );

    // a new good answer replaces both
    let fourth = ask(&store, 1, "hint", "q1", "practice", "practice", NOW + 40);
    finish(&store, fourth.answer, "second hint", "done", None);
    let s = store
        .read(|c| data::study_saved(c, 1, "practice", KEY32, "hint", "q1", "practice"))
        .expect("good");
    assert_eq!(
        s.latest.as_ref().map(|a| a.content.as_str()),
        Some("second hint")
    );
    assert!(s.last_good.is_none());
}

#[test]
fn saved_keeps_kinds_questions_and_places_apart() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let h = ask(&store, 1, "hint", "q1", "practice", "practice", NOW);
    finish(&store, h.answer, "hint text", "done", None);
    let e = ask(&store, 1, "explain", "q1", "practice", "practice", NOW + 1);
    finish(&store, e.answer, "practice explanation", "done", None);
    let r = ask(&store, 1, "explain", "q1", "result", "practice", NOW + 2);
    finish(&store, r.answer, "result explanation", "done", None);
    let o = ask(&store, 1, "explain", "q2", "practice", "practice", NOW + 3);
    finish(&store, o.answer, "other question", "done", None);
    let get = |kind: &str, q: &str, source: &str| {
        store
            .read(|c| data::study_saved(c, 1, "practice", KEY32, kind, q, source))
            .expect("saved")
            .latest
            .map(|a| a.content)
    };
    assert_eq!(get("hint", "q1", "practice").as_deref(), Some("hint text"));
    assert_eq!(
        get("explain", "q1", "practice").as_deref(),
        Some("practice explanation")
    );
    assert_eq!(
        get("explain", "q1", "result").as_deref(),
        Some("result explanation")
    );
    assert_eq!(
        get("explain", "q2", "practice").as_deref(),
        Some("other question")
    );
    assert_eq!(get("hint", "q1", "result"), None);
    assert_eq!(get("hint", "q2", "practice"), None);
    // the result answer carries the chat's record
    let rec = store
        .read(|c| data::study_saved(c, 1, "practice", KEY32, "explain", "q1", "result"))
        .expect("saved")
        .latest
        .expect("latest");
    assert_eq!(rec.record_id, Some(5));
    // another user's ref is empty
    let other = store
        .read(|c| data::study_saved(c, 2, "practice", KEY32, "hint", "q1", "practice"))
        .expect("other");
    assert_eq!(other.chat_id, None);
    assert!(other.latest.is_none());
    // another chat kind with the same ref is empty
    let exam = store
        .read(|c| data::study_saved(c, 1, "exam", KEY32, "hint", "q1", "practice"))
        .expect("exam");
    assert_eq!(exam.chat_id, None);
}

#[test]
fn summary_saved_finds_the_newest_summary_of_its_ref() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let chat_ref = "r42@2026-11-11 21:58";
    let make = |now: i64| {
        let s = StudyInsert {
            chat_kind: "summary",
            chat_ref,
            title: "2026-11-11 21:58",
            record_id: Some(42),
            kind: "summary",
            question_id: None,
            question_no: None,
            source: None,
            lang: "en",
            sample: Some(StudySample {
                used: 3,
                usable: 3,
                wrong: 4,
            }),
            user_content: "[AI summary] x",
            model: "demo-model",
            now,
            chats_per_user: 1000,
            messages_per_chat: 400,
        };
        run(&store, 1, &s).expect("summary")
    };
    let a = make(NOW);
    finish(&store, a.answer, "first summary", "done", None);
    let b = make(NOW + 10);
    finish(&store, b.answer, "", "error", Some("ai.up.timeout"));
    let s = store
        .read(|c| data::summary_saved(c, 1, chat_ref))
        .expect("saved");
    assert_eq!(s.chat_id, Some(a.chat));
    assert_eq!(s.latest.as_ref().map(|x| x.message_id), Some(b.answer));
    assert_eq!(
        s.last_good.as_ref().map(|x| x.content.as_str()),
        Some("first summary")
    );
    let j = s.to_json(true);
    assert_eq!(j["lastGood"]["recordId"], json!(42));
    assert_eq!(
        j["lastGood"]["sample"],
        json!({"used": 3, "usable": 3, "wrong": 4})
    );
    assert_eq!(
        j["latest"]
            .as_object()
            .map(|o| o.keys().cloned().collect::<Vec<_>>()),
        Some(
            [
                "messageId",
                "recordId",
                "status",
                "error",
                "truncated",
                "content",
                "lang",
                "model",
                "createdAt",
                "finishedAt",
                "sample"
            ]
            .iter()
            .map(|s| s.to_string())
            .collect()
        )
    );
    // a record id reused with another time never reaches this chat; another user sees nothing
    assert_eq!(
        store
            .read(|c| data::summary_saved(c, 1, "r42@2026-11-12 08:00"))
            .expect("other at")
            .chat_id,
        None
    );
    assert_eq!(
        store
            .read(|c| data::summary_saved(c, 2, chat_ref))
            .expect("other user")
            .chat_id,
        None
    );
}

#[test]
fn running_info_has_the_shape_and_the_record_rule() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let p = ask(&store, 1, "hint", "q1", "practice", "practice", NOW);
    let r = ask(&store, 1, "explain", "q1", "result", "practice", NOW + 1);
    let info = store
        .read(|c| data::running_info(c, 1, p.chat, p.answer))
        .expect("info")
        .expect("some");
    assert_eq!(
        info,
        json!({"chatId": p.chat, "messageId": p.answer, "chatKind": "practice", "kind": "hint",
               "questionId": "q1", "questionNo": 3, "source": "practice", "recordId": null})
    );
    let keys: Vec<String> = info.as_object().expect("obj").keys().cloned().collect();
    assert_eq!(
        keys,
        [
            "chatId",
            "messageId",
            "chatKind",
            "kind",
            "questionId",
            "questionNo",
            "source",
            "recordId"
        ]
    );
    let info = store
        .read(|c| data::running_info(c, 1, r.chat, r.answer))
        .expect("info")
        .expect("some");
    assert_eq!(info["recordId"], json!(5));
    // another user, wrong chat, missing message
    assert!(store
        .read(|c| data::running_info(c, 2, p.chat, p.answer))
        .expect("q")
        .is_none());
    assert!(store
        .read(|c| data::running_info(c, 1, p.chat + 99, p.answer))
        .expect("q")
        .is_none());
    assert!(store
        .read(|c| data::running_info(c, 1, p.chat, 9999))
        .expect("q")
        .is_none());
}

#[test]
fn chat_and_message_json_carry_the_v09_keys() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let a = ask(&store, 1, "explain", "q1", "result", "exam", NOW);
    store
        .read(|c| {
            let chats = data::list_chats(c, 1)?;
            let j = chats[0].to_json(true);
            let keys: Vec<&String> = j.as_object().expect("obj").keys().collect();
            assert_eq!(
                keys,
                [
                    "id",
                    "title",
                    "createdAt",
                    "updatedAt",
                    "messageCount",
                    "running",
                    "kind",
                    "recordId"
                ]
            );
            assert_eq!(
                (j["kind"].clone(), j["recordId"].clone()),
                (json!("exam"), json!(5))
            );
            let m = data::messages(c, a.chat)?;
            let j = m[1].to_json();
            let keys: Vec<&String> = j.as_object().expect("obj").keys().collect();
            assert_eq!(
                keys,
                [
                    "id",
                    "role",
                    "content",
                    "status",
                    "error",
                    "truncated",
                    "model",
                    "createdAt",
                    "free",
                    "kind",
                    "questionId",
                    "questionNo",
                    "source",
                    "lang",
                    "recordId",
                    "sample"
                ]
            );
            // an AI Chat message has nulls
            data::ensure_user(c, &who(1), NOW)?;
            Ok(())
        })
        .expect("read");
    // a plain chat message
    let plain = store
        .write(|tx| {
            let chat = data::create_chat(tx, 1, "plain", NOW)?;
            let msg = data::insert_message(
                tx,
                &data::NewMessage {
                    chat,
                    role: "user",
                    content: "hi",
                    status: "done",
                    model: None,
                    free: false,
                    now: NOW,
                },
            )?;
            Ok::<(i64, i64), rusqlite::Error>((chat, msg))
        })
        .expect("plain");
    store
        .read(|c| {
            let m = data::messages(c, plain.0)?;
            let j = m[0].to_json();
            assert_eq!(j["kind"], json!("chat"));
            for k in [
                "questionId",
                "questionNo",
                "source",
                "lang",
                "recordId",
                "sample",
            ] {
                assert_eq!(j[k], Value::Null, "{k}");
            }
            // memory suggest may only use AI Chat messages
            assert!(data::user_message(c, 1, plain.1)?.is_some());
            assert!(data::user_message(c, 1, a.user_msg)?.is_none());
            Ok(())
        })
        .expect("read");
}

// ---------------------------------------------------------------------------------------------------------------------
// Usage and retention

fn usage(user: i64, status: &'static str, tin: i64, tout: i64) -> UsageRow {
    UsageRow {
        user_id: Some(user),
        at: NOW,
        kind: "chat",
        provider: "openai",
        model: "demo-model".into(),
        tokens_in: Some(tin),
        tokens_out: Some(tout),
        tokens_cached: None,
        latency_ms: 10,
        first_byte_ms: Some(5),
        status,
        error_code: None,
    }
}

#[test]
fn usage_by_purpose_has_four_rows_in_a_fixed_order() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let day = xexam_ai_service::util::local_date(NOW);
    let (from, to) = (day.to_string(), day.to_string());
    // nothing: four zero rows
    let rows = store
        .read(|c| stats::by_purpose(c, &from, &to))
        .expect("empty");
    assert_eq!(
        rows.iter().map(|r| r.0).collect::<Vec<_>>(),
        ["chat", "hint", "explain", "summary"]
    );
    assert!(rows.iter().all(|r| r.1.calls == 0));
    store
        .write(|tx| {
            data::ensure_user(tx, &who(1), NOW)?;
            stats::insert_usage(tx, &usage(1, "ok", 10, 20))?;
            stats::insert_usage_purpose(tx, &usage(1, "ok", 1, 2), None)?;
            stats::insert_usage_purpose(tx, &usage(1, "ok", 100, 200), Some("hint"))?;
            stats::insert_usage_purpose(tx, &usage(1, "error", 0, 0), Some("hint"))?;
            stats::insert_usage_purpose(tx, &usage(1, "ok", 5, 6), Some("summary"))?;
            // a non-chat call never counts
            let mut v = usage(1, "ok", 999, 999);
            v.kind = "verify";
            stats::insert_usage(tx, &v)?;
            Ok::<(), rusqlite::Error>(())
        })
        .expect("usage");
    let rows = store
        .read(|c| stats::by_purpose(c, &from, &to))
        .expect("rows");
    let get = |p: &str| {
        rows.iter()
            .find(|r| r.0 == p)
            .map(|r| r.1.clone())
            .expect("row")
    };
    let chat = get("chat");
    assert_eq!(
        (chat.calls, chat.tokens_in, chat.tokens_out, chat.errors),
        (2, 11, 22, 0)
    );
    let hint = get("hint");
    assert_eq!(
        (hint.calls, hint.tokens_in, hint.tokens_out, hint.errors),
        (2, 100, 200, 1)
    );
    assert_eq!(get("explain").calls, 0);
    assert_eq!(get("summary").calls, 1);
    assert_eq!(
        rows.iter().map(|r| r.0).collect::<Vec<_>>(),
        ["chat", "hint", "explain", "summary"]
    );
    // the CHECK refuses an unknown purpose
    let bad =
        store.write(|tx| stats::insert_usage_purpose(tx, &usage(1, "ok", 1, 1), Some("chatty")));
    assert!(bad.is_err());
}

#[test]
fn the_retention_sweep_works_in_batches_and_keeps_running_chats() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    store
        .write(|tx| {
            data::ensure_user(tx, &who(1), NOW)?;
            for i in 0..250 {
                let chat = data::create_chat(tx, 1, "old", 1000 + i)?;
                data::insert_message(
                    tx,
                    &data::NewMessage {
                        chat,
                        role: "user",
                        content: "x",
                        status: "done",
                        model: None,
                        free: false,
                        now: 1000 + i,
                    },
                )?;
            }
            // a recent chat
            data::create_chat(tx, 1, "recent", NOW)?;
            Ok::<(), rusqlite::Error>(())
        })
        .expect("seed");
    let running = [1_i64, 2];
    let mut batches = Vec::new();
    loop {
        let n = store
            .write(|tx| stats::sweep_chats(tx, NOW - 100, &running, 100))
            .expect("sweep");
        batches.push(n);
        if n < 100 {
            break;
        }
    }
    // 248 old chats are deletable: 100 + 100 + 48
    assert_eq!(batches, [100, 100, 48]);
    let left: Vec<i64> = store
        .read(|c| {
            let mut st = c.prepare("SELECT id FROM chats ORDER BY id")?;
            let rows = st.query_map([], |r| r.get(0))?;
            rows.collect()
        })
        .expect("left");
    assert_eq!(left, [1, 2, 251]);
    // the messages of deleted chats went with them
    let messages: i64 = store
        .read(|c| c.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0)))
        .expect("count");
    assert_eq!(messages, 2);
    // the oldest go first
    let mut s2 = 0;
    store
        .write(|tx| {
            data::create_chat(tx, 1, "o1", 10)?;
            data::create_chat(tx, 1, "o2", 20)?;
            s2 = stats::sweep_chats(tx, NOW - 100, &running, 1)?;
            Ok::<(), rusqlite::Error>(())
        })
        .expect("one");
    assert_eq!(s2, 1);
    let titles: Vec<String> = store
        .read(|c| {
            let mut st = c.prepare("SELECT title FROM chats WHERE title IN ('o1', 'o2')")?;
            let rows = st.query_map([], |r| r.get(0))?;
            rows.collect()
        })
        .expect("titles");
    assert_eq!(titles, ["o2"]);
}

#[test]
fn a_burst_sweep_lets_other_writers_in_between_batches() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};
    use xexam_ai_service::retention::{sweep_batches, SWEEP_PAUSE};
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    store
        .write(|tx| {
            data::ensure_user(tx, &who(1), NOW)?;
            for i in 0..250 {
                data::create_chat(tx, 1, "old", 1000 + i)?;
            }
            Ok::<(), rusqlite::Error>(())
        })
        .expect("seed");
    let done = AtomicBool::new(false);
    let (deleted, got, longest) = std::thread::scope(|sc| {
        let other = sc.spawn(|| {
            let mut got = 0usize;
            let mut longest = Duration::ZERO;
            while !done.load(Ordering::SeqCst) {
                let t = Instant::now();
                store
                    .write(|tx| tx.execute("UPDATE chats SET title = title WHERE id = 0", []))
                    .expect("other write");
                longest = longest.max(t.elapsed());
                got += 1;
                std::thread::sleep(Duration::from_millis(10));
            }
            (got, longest)
        });
        let n = sweep_batches(&store, NOW - 100, &[], SWEEP_PAUSE);
        done.store(true, Ordering::SeqCst);
        let (got, longest) = other.join().expect("join");
        (n, got, longest)
    });
    assert_eq!(deleted, 250);
    // 13 batches with 12 pauses of 20 ms: the other writer gets in again and again, and never waits for the sweep
    assert!(got >= 6, "the other writer got the lock only {got} times");
    assert!(
        longest < Duration::from_millis(250),
        "longest wait {longest:?}"
    );
}
