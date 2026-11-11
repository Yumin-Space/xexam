//! AI database schema 2 (round-2 contracts 3.1): the upgrade of a schema-1 database (backup copy first, rows and keys
//! kept, new columns with their defaults), reopening writes nothing, new and upgraded databases have the same
//! `sqlite_master`, a newer schema and a failed backup are refused. All content is made up.
mod common;

use common::*;
use rusqlite::Connection;
use serde_json::json;
use std::path::{Path, PathBuf};
use xexam_ai_service::secret::{self, MasterKey, SecretName};
use xexam_ai_service::store::{schema, tables, DB_FILE};

/// The encryption key of the fixture (made up).
const MASTER: [u8; 32] = [7; 32];

fn master_hex() -> String {
    xexam_ai_service::util::hex(&MASTER)
}

/// Writes a schema-1 AI database like a pre-release v0.8 instance left it: the v1 tables, the settings / limits rows,
/// `meta.schema = '1'`, a sealed provider key, a user with a chat (two messages) and a memory.
fn write_v1_fixture(data_dir: &Path) -> PathBuf {
    std::fs::create_dir_all(data_dir).expect("data dir");
    let path = data_dir.join(DB_FILE);
    let c = Connection::open(&path).expect("open fixture");
    c.pragma_update(None, "journal_mode", "wal").expect("wal");
    c.execute_batch(schema::SCHEMA).expect("v1 schema");
    let now = xexam_ai_service::util::unix_now();
    c.execute(
        "INSERT INTO settings (id, updated_at, model) VALUES (1, ?1, 'demo-chat-small')",
        [now],
    )
    .expect("settings");
    c.execute("INSERT INTO limits (id, updated_at) VALUES (1, ?1)", [now])
        .expect("limits");
    c.execute("INSERT INTO meta (key, value) VALUES ('schema', '1')", [])
        .expect("meta");
    let m = MasterKey::from_master(&MASTER);
    let sealed = m.seal(SecretName::ApiKey, KEY.as_bytes()).expect("seal");
    tables::put_secret(
        &c,
        SecretName::ApiKey,
        m.key_id(),
        &sealed,
        &secret::hint(KEY),
        now,
    )
    .expect("secret");
    c.execute(
        "INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at) VALUES (5, ?1, 'sam', ?2, ?2)",
        // the stamp the test users get (`TestApp::user_with_id`)
        rusqlite::params![format!("{:032x}", 0xabc0_0000_u64 + 5 * 7919), now],
    )
    .expect("user");
    c.execute(
        "INSERT INTO chats (user_id, title, created_at, updated_at) VALUES (5, 'An old chat', ?1, ?1)",
        [now],
    )
    .expect("chat");
    let chat = c.last_insert_rowid();
    for (role, text) in [("user", "an old question"), ("assistant", "an old answer")] {
        c.execute(
            "INSERT INTO messages (chat_id, role, content, created_at, finished_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            rusqlite::params![chat, role, text, now],
        )
        .expect("message");
    }
    c.execute(
        "INSERT INTO memories (user_id, content, created_at, updated_at) VALUES (5, 'likes short answers', ?1, ?1)",
        [now],
    )
    .expect("memory");
    c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .expect("checkpoint");
    drop(c);
    path
}

fn master_rows(path: &Path) -> Vec<(String, String, String, Option<String>)> {
    let c = Connection::open(path).expect("open");
    let mut st = c
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name")
        .expect("prepare");
    let rows = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .expect("query");
    rows.collect::<Result<_, _>>().expect("rows")
}

fn backups(data_dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(data_dir.join("backups"))
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    out.sort();
    out
}

fn has_column(c: &Connection, table: &str, column: &str) -> bool {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
        [table, column],
        |r| r.get(0),
    )
    .expect("table info")
}

fn opts_with_key() -> Opts {
    Opts {
        secret_key: Some(master_hex()),
        ..Opts::default()
    }
}

#[tokio::test]
async fn a_schema_1_database_is_backed_up_then_upgraded() {
    let mut t = start_with(opts_with_key()).await;
    let data_dir = t.dir.path().join("ai");
    // Replace the fresh database of the start with the schema-1 fixture, then start again
    t.app.0.begin_shutdown();
    t.app.0.store.release_file();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(data_dir.join(format!("{DB_FILE}{suffix}")));
    }
    let path = write_v1_fixture(&data_dir);
    t.restart(opts_with_key()).await;

    // the copy was made first, and it is the schema-1 database as it was
    let copies = backups(&data_dir);
    assert_eq!(copies.len(), 1, "{copies:?}");
    let name = copies[0]
        .file_name()
        .and_then(|n| n.to_str())
        .expect("name")
        .to_string();
    assert!(
        name.starts_with("xexam-ai-schema1-") && name.ends_with(".db"),
        "{name}"
    );
    let stamp = name
        .trim_start_matches("xexam-ai-schema1-")
        .trim_end_matches(".db");
    assert!(
        stamp.len() == 15
            && stamp.as_bytes()[8] == b'-'
            && stamp
                .bytes()
                .enumerate()
                .all(|(i, b)| i == 8 || b.is_ascii_digit()),
        "YYYYMMDD-HHMMSS: {stamp}"
    );
    let copy = Connection::open(&copies[0]).expect("copy");
    let v: String = copy
        .query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
            r.get(0)
        })
        .expect("schema");
    assert_eq!(v, "1");
    assert!(!has_column(&copy, "users", "function_enabled"));
    assert!(!has_column(&copy, "users", "free_mode"));
    let n: i64 = copy
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .expect("count");
    assert_eq!(n, 2);
    drop(copy);
    assert!(captured_contains(&format!(
        "Upgraded the AI database to schema 2. A copy of the previous version was saved first: {}.",
        copies[0].display()
    )));

    // the upgraded database: schema 2, the rows kept, the new columns with their defaults
    let c = t.db();
    let v: String = c
        .query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
            r.get(0)
        })
        .expect("schema");
    assert_eq!(v, "2");
    let (fe, free): (i64, i64) = c
        .query_row(
            "SELECT (SELECT function_enabled FROM users WHERE id = 5), (SELECT free_mode FROM users WHERE id = 5)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("defaults");
    assert_eq!((fe, free), (1, 0));
    let (chats, messages, memories): (i64, i64, i64) = c
        .query_row(
            "SELECT (SELECT COUNT(*) FROM chats), (SELECT COUNT(*) FROM messages), (SELECT COUNT(*) FROM memories)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("counts");
    assert_eq!((chats, messages, memories), (1, 2, 1));
    drop(c);

    // the sealed key still opens with the same encryption key; the old user is not in free mode
    let admin = t.user("admin", true, false);
    let (s, v) = t.get("/api/ai/admin/settings", &admin).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["keyProblem"], json!(null));
    assert_eq!(
        v["keys"]["apiKey"],
        json!({"set": true, "hint": secret::hint(KEY), "unreadable": false})
    );
    let sam = t.user_with_id(5, "sam", false, true);
    let (s, v) = t.get("/api/ai/chats", &sam).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["chats"][0]["title"], json!("An old chat"));
    assert_eq!(v["chats"][0]["messageCount"], json!(2));

    // reopening an upgraded database writes nothing and makes no second copy
    t.app.0.begin_shutdown();
    t.app.0.store.release_file();
    let before = std::fs::read(&path).expect("read");
    t.restart(opts_with_key()).await;
    t.app.0.store.release_file();
    let after = std::fs::read(&path).expect("read");
    assert!(before == after, "reopening changed the database file");
    assert_eq!(backups(&data_dir).len(), 1);
}

#[tokio::test]
async fn new_and_upgraded_databases_have_the_same_tables() {
    let fresh = start().await;
    let fresh_rows = master_rows(&fresh.db_path());

    let mut t = start_with(opts_with_key()).await;
    let data_dir = t.dir.path().join("ai");
    t.app.0.begin_shutdown();
    t.app.0.store.release_file();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(data_dir.join(format!("{DB_FILE}{suffix}")));
    }
    let path = write_v1_fixture(&data_dir);
    t.restart(opts_with_key()).await;
    let upgraded_rows = master_rows(&path);
    assert_eq!(fresh_rows, upgraded_rows);
    // both carry the two columns of schema 2
    let users_sql = fresh_rows
        .iter()
        .find(|r| r.0 == "table" && r.1 == "users")
        .and_then(|r| r.3.clone())
        .expect("users");
    assert!(
        users_sql.contains("function_enabled INTEGER NOT NULL DEFAULT 1"),
        "{users_sql}"
    );
    assert!(
        users_sql.contains("free_mode INTEGER NOT NULL DEFAULT 0"),
        "{users_sql}"
    );
    // a new database needs no copy
    assert!(backups(&fresh.dir.path().join("ai")).is_empty());
}

#[tokio::test]
async fn a_taken_backup_name_gets_a_number() {
    let mut t = start_with(opts_with_key()).await;
    let data_dir = t.dir.path().join("ai");
    t.app.0.begin_shutdown();
    t.app.0.store.release_file();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(data_dir.join(format!("{DB_FILE}{suffix}")));
    }
    write_v1_fixture(&data_dir);
    // Every name of the next few seconds is taken already
    std::fs::create_dir_all(data_dir.join("backups")).expect("dir");
    let now = jiff::Zoned::now();
    let mut taken = Vec::new();
    for s in 0..4 {
        let at = now
            .checked_add(jiff::SignedDuration::from_secs(s))
            .expect("time");
        let p = data_dir.join("backups").join(format!(
            "xexam-ai-schema1-{}.db",
            at.strftime("%Y%m%d-%H%M%S")
        ));
        std::fs::write(&p, b"taken").expect("write");
        taken.push(p);
    }
    t.restart(opts_with_key()).await;
    let copies = backups(&data_dir);
    let made: Vec<&PathBuf> = copies.iter().filter(|p| !taken.contains(p)).collect();
    assert_eq!(made.len(), 1, "{copies:?}");
    let name = made[0].file_name().and_then(|n| n.to_str()).expect("name");
    assert!(name.ends_with("-2.db"), "{name}");
    for p in &taken {
        assert_eq!(std::fs::read(p).expect("read"), b"taken");
    }
}

#[tokio::test]
async fn a_newer_schema_is_refused() {
    let t = start().await;
    let path = t.db_path();
    t.app.0.begin_shutdown();
    t.app.0.store.release_file();
    {
        let c = Connection::open(&path).expect("open");
        c.execute("UPDATE meta SET value = '3' WHERE key = 'schema'", [])
            .expect("set");
    }
    let cfg = test_config(t.dir.path(), &t.backend.url, &Opts::default());
    let err = match xexam_ai_service::app::App::open(cfg) {
        Ok(_) => panic!("a schema-3 database must be refused"),
        Err(e) => e,
    };
    assert_eq!(
        err,
        format!(
            "The AI database {} has schema 3, written by a newer ai-service; this version (schema 2) cannot open it. Run the newer version, or restore a backup.",
            path.display()
        )
    );
}

#[tokio::test]
async fn a_failed_backup_refuses_to_start_and_changes_nothing() {
    let t = start().await;
    let data_dir = t.dir.path().join("ai");
    t.app.0.begin_shutdown();
    t.app.0.store.release_file();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(data_dir.join(format!("{DB_FILE}{suffix}")));
    }
    let path = write_v1_fixture(&data_dir);
    // `backups` is a file: the copy cannot be written
    std::fs::write(data_dir.join("backups"), b"not a directory").expect("write");
    let before = std::fs::read(&path).expect("read");
    let cfg = test_config(t.dir.path(), &t.backend.url, &Opts::default());
    let err = match xexam_ai_service::app::App::open(cfg) {
        Ok(_) => panic!("the upgrade must not run without a copy"),
        Err(e) => e,
    };
    assert!(
        err.starts_with("cannot back up the AI database before the upgrade (")
            && err.ends_with("); nothing was changed"),
        "{err}"
    );
    let c = Connection::open(&path).expect("open");
    let v: String = c
        .query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
            r.get(0)
        })
        .expect("schema");
    assert_eq!(v, "1");
    assert!(!has_column(&c, "users", "function_enabled"));
    drop(c);
    assert!(before == std::fs::read(&path).expect("read"));
}
