//! v0.7: SQLite in WAL mode (with the `XEXAM_SQLITE_JOURNAL=delete` opt-out, the checkpoint at shutdown, migrations and their
//! backups in WAL mode, the reset-password command next to a running server) and the system log (`GET /api/logs`, the
//! in-memory buffer fed by the tracing layer, no secrets in the log). Made-up data only.
#[path = "api_common/mod.rs"]
mod apic;
#[path = "common/mod.rs"]
mod dbc;

use apic::{build, send, Resp};
use axum::body::Body;
use axum::Router;
use rusqlite::Connection;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicI64;
use std::sync::{Arc, OnceLock};
use xexam_backend::api::{build_router, AppState, Inner};
use xexam_backend::auth::{self, LoginThrottle, Role};
use xexam_backend::database::{JournalMode, Store, SystemClock};
use xexam_backend::logs::{BufferLayer, LogBuffer};
use xexam_backend::models::BankAccess;
use xexam_backend::services::{PendingStore, RealEngine};

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";
const MSG_ADMIN_ONLY: &str = "只有管理员可以进行这个操作。";
const MSG_BAD_REQUEST: &str = "请求的格式不对。";

fn raw(p: &Path) -> Connection {
    Connection::open(p).expect("raw open")
}

fn journal_of(p: &Path) -> String {
    raw(p)
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .expect("journal_mode")
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).expect("count")
}

fn wal_of(db: &Path) -> PathBuf {
    let mut p = db.as_os_str().to_os_string();
    p.push("-wal");
    PathBuf::from(p)
}

fn wal_len(db: &Path) -> Option<u64> {
    std::fs::metadata(wal_of(db)).ok().map(|m| m.len())
}

/// Two made-up questions of the six-column layout (`lang` "en"), so the command-line program's start (which retires the
/// questions of the old layouts, v0.7) leaves them alone.
fn import_two(s: &Store) {
    let qs: Vec<_> = [
        dbc::single("Pantry", "w1", "Is basil a herb?", 2),
        dbc::single("Pantry", "w2", "Is salt a spice?", 3),
    ]
    .into_iter()
    .map(|mut q| {
        q.lang = "en".to_string();
        q
    })
    .collect();
    s.apply_import("pantry.xlsx", &[dbc::sheet("Pantry", 2)], &qs)
        .expect("import");
}

// ---------------------------------------------------------------- WAL

#[test]
fn a_new_database_is_in_wal_mode() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = Store::open(&db).expect("open");
    assert_eq!(s.journal_mode(), "wal");
    // persistent: any other connection sees it too
    assert_eq!(journal_of(&db), "wal");
    import_two(&s);
    assert!(
        wal_len(&db).unwrap_or(0) > 0,
        "the commit went into the -wal file"
    );
    assert!(!dir.path().join("xexam.db-journal").exists());
    drop(s);
    // the last connection to close copies everything into xexam.db and removes the -wal file
    assert_eq!(wal_len(&db), None);
    assert_eq!(count(&raw(&db), "SELECT COUNT(*) FROM questions"), 2);
}

#[test]
fn the_opt_out_keeps_the_delete_journal() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = Store::open_with(&db, Arc::new(SystemClock), JournalMode::Delete).expect("open");
    assert_eq!(s.journal_mode(), "delete");
    import_two(&s);
    assert_eq!(wal_len(&db), None, "no -wal file in DELETE mode");
    assert_eq!(s.checkpoint().expect("checkpoint"), None, "nothing to do");
    drop(s);
    assert_eq!(journal_of(&db), "delete");
    // a WAL database opened with the opt-out (or by an older version, which always asks for DELETE) goes back to DELETE
    drop(Store::open(&db).expect("wal"));
    assert_eq!(journal_of(&db), "wal");
    let s = Store::open_with(&db, Arc::new(SystemClock), JournalMode::Delete).expect("back");
    assert_eq!(s.journal_mode(), "delete");
    assert_eq!(count(&raw(&db), "SELECT COUNT(*) FROM questions"), 2);
    drop(s);
    assert_eq!(journal_of(&db), "delete");
    assert_eq!(wal_len(&db), None);
}

#[test]
fn the_environment_variable_chooses_the_mode() {
    assert_eq!(apic::default_config().sqlite_journal, JournalMode::Wal);
    for (value, want) in [
        ("delete", JournalMode::Delete),
        ("DELETE", JournalMode::Delete),
        (" delete ", JournalMode::Delete),
        ("wal", JournalMode::Wal),
        ("", JournalMode::Wal),
        ("truncate", JournalMode::Wal),
        ("off", JournalMode::Wal),
    ] {
        assert_eq!(
            apic::config_with(&[("XEXAM_SQLITE_JOURNAL", value)]).sqlite_journal,
            want,
            "{value:?}"
        );
    }
}

#[test]
fn the_checkpoint_at_shutdown_leaves_no_wal_content() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = Store::open(&db).expect("open");
    import_two(&s);
    let before = wal_len(&db).expect("a -wal file");
    assert!(before > 0);
    let c = s.checkpoint().expect("checkpoint").expect("WAL mode");
    assert!(!c.busy, "{c:?}");
    // after a complete TRUNCATE checkpoint SQLite reports the (now empty) log: 0 frames
    assert_eq!((c.log_frames, c.checkpointed), (0, 0));
    assert_eq!(wal_len(&db), Some(0), "TRUNCATE empties the -wal file");
    // xexam.db alone now holds everything: a copy of just that file (what a backup at rest takes) has the questions
    let copy = dir.path().join("copy.db");
    std::fs::copy(&db, &copy).expect("copy");
    assert_eq!(count(&raw(&copy), "SELECT COUNT(*) FROM questions"), 2);
    // the store keeps working after the checkpoint
    s.update_settings(dbc::ADMIN, &[("theme".into(), json!("dark"))])
        .expect("settings");
    assert_eq!(s.settings(dbc::ADMIN).expect("settings")["theme"], "dark");
}

#[test]
fn a_checkpoint_while_another_connection_reads_reports_busy() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = Store::open(&db).expect("open");
    import_two(&s);
    let reader = raw(&db);
    reader.execute_batch("BEGIN").expect("begin");
    assert_eq!(count(&reader, "SELECT COUNT(*) FROM questions"), 2);
    let c = s.checkpoint().expect("checkpoint").expect("WAL mode");
    assert!(c.busy, "a reader holds the -wal file: {c:?}");
    reader.execute_batch("COMMIT").expect("commit");
    let c = s.checkpoint().expect("checkpoint").expect("WAL mode");
    assert!(!c.busy);
    assert_eq!(wal_len(&db), Some(0));
}

#[test]
fn a_migration_in_wal_mode_backs_up_what_is_still_in_the_wal_file() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    // A schema-1 database (written by the old Python version), switched to WAL, with a row that exists only in the -wal file:
    // the writer keeps its connection open and never checkpoints
    std::fs::copy(dbc::baseline("fixtures/sample.db"), &db).expect("copy");
    let writer = raw(&db);
    let mode: String = writer
        .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
        .expect("wal");
    assert_eq!(mode, "wal");
    writer
        .execute_batch(
            "PRAGMA wal_autocheckpoint=0; \
             INSERT INTO stats(question_id, attempts, correct, wrong, streak, last, at) \
             VALUES('walonly0001', 4, 3, 1, 2, 1, '2026-01-02 03:04');",
        )
        .expect("insert");
    assert!(wal_len(&db).unwrap_or(0) > 0);
    let stats = count(&writer, "SELECT COUNT(*) FROM stats");

    let s = Store::open(&db).expect("migrate");
    assert_eq!(s.journal_mode(), "wal");
    let backup = s.migration_backup().expect("a backup").to_path_buf();
    // the migrated database claimed the row for the default administrator
    assert_eq!(
        count(
            &raw(&db),
            "SELECT COUNT(*) FROM user_stats WHERE user_id=1 AND question_id='walonly0001'"
        ),
        1
    );
    drop(s);
    drop(writer);
    // the backup (VACUUM INTO) is one self-contained file: copied alone into another directory it still has the row and
    // the old schema
    let elsewhere = dbc::tmp();
    let alone = elsewhere.path().join("backup.db");
    std::fs::copy(&backup, &alone).expect("copy the backup");
    let b = raw(&alone);
    assert_eq!(count(&b, "SELECT COUNT(*) FROM stats"), stats);
    assert_eq!(
        count(
            &b,
            "SELECT COUNT(*) FROM stats WHERE question_id='walonly0001'"
        ),
        1
    );
    let schema: Option<String> = b
        .query_row("SELECT value FROM meta WHERE key='schema'", [], |r| {
            r.get(0)
        })
        .ok();
    assert!(
        matches!(schema.as_deref(), None | Some("1")),
        "the backup is the schema-1 database: {schema:?}"
    );
    assert_eq!(
        count(
            &b,
            "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('users', 'banks')"
        ),
        0
    );
}

#[test]
fn reopening_an_up_to_date_wal_database_writes_nothing() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    {
        let s = Store::open(&db).expect("open");
        import_two(&s);
    }
    let before = dbc::db_bytes(&db);
    let s = Store::open(&db).expect("reopen");
    s.bank().expect("bank");
    s.progress(dbc::ADMIN).expect("progress");
    // nothing reached the -wal file while the store is open either
    assert_eq!(dbc::db_bytes(&db), before);
    drop(s);
    assert_eq!(dbc::db_bytes(&db), before);
}

fn cli(args: &[&str]) -> std::process::Output {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_xexam-backend"));
    for (k, _) in std::env::vars() {
        if k.starts_with("XEXAM_") {
            cmd.env_remove(k);
        }
    }
    cmd.args(args).output().expect("run xexam-backend")
}

#[test]
fn reset_password_works_while_the_server_has_the_database_open() {
    let dir = dbc::tmp();
    let data = dir.path().to_str().expect("utf-8 path").to_string();
    let db = dir.path().join("xexam.db");
    // the "server": a store that stays open, with uncheckpointed writes in the -wal file and a signed-in session
    let s = Store::open(&db).expect("open");
    import_two(&s);
    let _cookie = apic::test_session(&s, "admin");
    assert!(count(&raw(&db), "SELECT COUNT(*) FROM sessions") > 0);
    assert!(wal_len(&db).unwrap_or(0) > 0);
    let out = cli(&["reset-password", "admin", "--data", &data]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let password = stdout
        .lines()
        .find_map(|l| l.strip_prefix("New password for 'admin': "))
        .expect("the new password is printed")
        .trim()
        .to_string();
    // the running store sees the command's commit at once
    let u = s.user_by_name("admin").expect("lookup").expect("admin");
    assert!(auth::verify_password(&password, &u.password_hash));
    assert!(u.must_change_password);
    assert_eq!(count(&raw(&db), "SELECT COUNT(*) FROM sessions"), 0);
    assert_eq!(s.count_active().expect("count"), 2, "nothing else changed");
    // the server's file stays in WAL mode
    assert_eq!(journal_of(&db), "wal");
    drop(s);
    assert_eq!(wal_len(&db), None);
}

// ---------------------------------------------------------------- GET /api/logs

struct App {
    router: Router,
    store: Arc<Store>,
    logs: Arc<LogBuffer>,
    _dir: tempfile::TempDir,
}

fn app() -> App {
    let dir = dbc::tmp();
    let store = Arc::new(Store::open(&dir.path().join("xexam.db")).expect("store"));
    let logs = Arc::new(LogBuffer::new(5));
    let state = AppState(Arc::new(Inner {
        cfg: apic::default_config(),
        store: store.clone(),
        pending: PendingStore::new(),
        parse_gate: Arc::new(tokio::sync::Semaphore::new(2)),
        shutdown: None,
        engine: Arc::new(RealEngine::new(store.clone())),
        hash_gate: Arc::new(tokio::sync::Semaphore::new(auth::HASH_CONCURRENCY)),
        throttle: LoginThrottle::default(),
        last_session_purge: AtomicI64::new(0),
        default_admin_hashes: Default::default(),
        logs: logs.clone(),
        rejections: Default::default(),
        failed_sign_ins: Default::default(),
        name_locks: Default::default(),
        old_layouts: Default::default(),
    }));
    App {
        router: build_router(state),
        store,
        logs,
        _dir: dir,
    }
}

async fn get(a: &App, path: &str, cookie: Option<&str>) -> Resp {
    let hs: Vec<(&str, &[u8])> = cookie
        .map(|c| vec![("cookie", c.as_bytes())])
        .unwrap_or_default();
    send(
        &a.router,
        build("GET", path, Some(HOST), &hs, Body::empty()),
    )
    .await
}

fn add_user(a: &App, name: &str, role: Role, can_manage_banks: bool) -> String {
    let hash = a
        .store
        .user_by_name("admin")
        .expect("lookup")
        .expect("admin")
        .password_hash;
    a.store
        .create_user_with(
            name,
            &hash,
            role,
            &BankAccess::Only(Vec::new()),
            false,
            can_manage_banks,
        )
        .expect("create");
    apic::test_session(&a.store, name)
}

fn line(a: &App, level: tracing::Level, message: &str) -> u64 {
    a.logs.push_at(
        "2026-01-02 03:04:05".to_string(),
        level,
        "xexam_backend::api",
        message.to_string(),
    )
}

#[tokio::test]
async fn an_administrator_reads_the_log_after_a_sequence_number() {
    let a = app();
    let c = apic::test_session(&a.store, "admin");
    let r = get(&a, "/api/logs", Some(&c)).await;
    assert_eq!(r.status, 200);
    let boot = r.json()["boot"].as_str().expect("boot").to_string();
    assert_eq!(boot, a.logs.boot());
    assert_eq!(
        r.json(),
        json!({"lines": [], "last": 0, "dropped": false, "boot": boot})
    );
    assert_eq!(
        r.header("content-type").as_deref(),
        Some("application/json; charset=utf-8")
    );

    line(&a, tracing::Level::INFO, "user \"admin\" signed in");
    line(
        &a,
        tracing::Level::WARN,
        "rejected GET /api/bank: Host \"evil.example\" is not in the allowed list",
    );
    line(
        &a,
        tracing::Level::ERROR,
        "database error: sqlite: disk I/O error",
    );
    let r = get(&a, "/api/logs", Some(&c)).await;
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(
        v.as_object().expect("object").keys().collect::<Vec<_>>(),
        ["lines", "last", "dropped", "boot"]
    );
    assert_eq!(
        v["lines"][0],
        json!({
            "seq": 1,
            "time": "2026-01-02 03:04:05",
            "level": "info",
            "target": "xexam_backend::api",
            "message": "user \"admin\" signed in"
        })
    );
    assert_eq!(v["lines"][2]["level"], "error");
    assert_eq!(
        (v["last"].clone(), v["dropped"].clone()),
        (json!(3), json!(false))
    );

    // polling: only the newer lines
    let r = get(&a, "/api/logs?after=2", Some(&c)).await;
    let v = r.json();
    assert_eq!(v["lines"].as_array().map(Vec::len), Some(1));
    assert_eq!(v["lines"][0]["seq"], 3);
    assert_eq!(v["last"], 3);
    let v = get(&a, "/api/logs?after=3", Some(&c)).await.json();
    assert_eq!(
        v,
        json!({"lines": [], "last": 3, "dropped": false, "boot": boot})
    );

    // the buffer of this app keeps 5 lines: a client that last saw line 2 missed lines 3 to 4
    for i in 4..=9 {
        line(&a, tracing::Level::INFO, &format!("line {i}"));
    }
    let v = get(&a, "/api/logs?after=2", Some(&c)).await.json();
    let seqs: Vec<i64> = v["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .map(|l| l["seq"].as_i64().expect("seq"))
        .collect();
    assert_eq!(seqs, [5, 6, 7, 8, 9]);
    assert_eq!(
        (v["last"].clone(), v["dropped"].clone()),
        (json!(9), json!(true))
    );
    let v = get(&a, "/api/logs?after=4", Some(&c)).await.json();
    assert_eq!(v["dropped"], false);
    // a number from before a restart of the backend (larger than any line now): every kept line
    let v = get(&a, "/api/logs?after=9999", Some(&c)).await.json();
    assert_eq!(v["lines"].as_array().map(Vec::len), Some(5));
    // a client that last polled another process (its boot id differs): every kept line, even when its number is not larger
    let v = get(&a, "/api/logs?after=8&boot=0123456789ab", Some(&c))
        .await
        .json();
    assert_eq!(v["lines"].as_array().map(Vec::len), Some(5));
    assert_eq!(v["boot"], boot.as_str());
    // the same boot id: only the newer lines
    let v = get(&a, &format!("/api/logs?after=8&boot={boot}"), Some(&c))
        .await
        .json();
    assert_eq!(v["lines"].as_array().map(Vec::len), Some(1));
    // HEAD answers like GET without a body
    let r = send(
        &a.router,
        build(
            "HEAD",
            "/api/logs",
            Some(HOST),
            &[("cookie", c.as_bytes())],
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(r.status, 200);
    assert!(r.body.is_empty());
}

#[tokio::test]
async fn a_malformed_after_is_a_bad_request() {
    let a = app();
    let c = apic::test_session(&a.store, "admin");
    for q in [
        "after=",
        "after=-1",
        "after=abc",
        "after=1.5",
        "after=%2B1",
        "after=1&after=2",
        "after=99999999999999999999",
        "boot=",
        "boot=a%20b",
        "boot=a&boot=b",
        "after=1&boot=%C3%A9",
    ] {
        let r = get(&a, &format!("/api/logs?{q}"), Some(&c)).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (400, MSG_BAD_REQUEST.to_string()),
            "{q}"
        );
    }
    // other parameters are ignored; percent-encoded digits are digits
    for q in [
        "",
        "x=1",
        "after=0",
        "after=%31",
        "after=0&x=y",
        "after=0&boot=Ab_-09",
    ] {
        let r = get(&a, &format!("/api/logs?{q}"), Some(&c)).await;
        assert_eq!(r.status, 200, "{q}");
    }
}

#[tokio::test]
async fn only_administrators_read_the_log() {
    let a = app();
    line(&a, tracing::Level::INFO, "user \"admin\" signed in");
    let plain = add_user(&a, "cook", Role::User, false);
    let manager = add_user(&a, "baker", Role::User, true);
    for c in [&plain, &manager] {
        for path in ["/api/logs", "/api/logs?after=0"] {
            let r = get(&a, path, Some(c)).await;
            assert_eq!(
                (r.status.as_u16(), r.error()),
                (403, MSG_ADMIN_ONLY.to_string()),
                "{path}"
            );
        }
    }
    let r = get(&a, "/api/logs", None).await;
    assert_eq!(r.status, 401);
    // a second administrator may
    let boss = add_user(&a, "boss", Role::Admin, false);
    assert_eq!(get(&a, "/api/logs", Some(&boss)).await.status, 200);
    // the Host guard still comes first
    let r = send(
        &a.router,
        build(
            "GET",
            "/api/logs",
            Some("evil.example"),
            &[("cookie", boss.as_bytes())],
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(r.status, 403);
    // POST is not a route
    let r = send(
        &a.router,
        build(
            "POST",
            "/api/logs",
            Some(HOST),
            &[
                ("cookie", boss.as_bytes()),
                ("origin", ORIGIN.as_bytes()),
                ("content-type", b"application/json"),
                ("content-length", b"2"),
            ],
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(r.status, 404);
}

#[tokio::test]
async fn a_flood_of_requests_rejected_by_the_host_check_is_summed_up_in_the_log() {
    let buf = captured();
    let a = app();
    let host = "flood-test.example";
    for i in 0..3000 {
        let method = if i % 2 == 0 { "GET" } else { "POST" };
        let r = send(
            &a.router,
            build(method, "/api/info", Some(host), &[], Body::empty()),
        )
        .await;
        assert_eq!(r.status, 403);
    }
    let lines: Vec<String> = buf
        .since(None)
        .lines
        .into_iter()
        .filter(|l| l.message.contains(host))
        .map(|l| l.message)
        .collect();
    // the first one is logged at once; the rest wait for the once-a-minute summary (not due within this test)
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("rejected GET /api/info: Host"),
        "{}",
        lines[0]
    );
}

// ---------------------------------------------------------------- nothing secret in the log

/// The buffer every log event of this test binary goes to (installed once as the global subscriber, without a filter: every
/// level, from every thread, including the blocking tasks).
fn captured() -> Arc<LogBuffer> {
    static BUF: OnceLock<Arc<LogBuffer>> = OnceLock::new();
    BUF.get_or_init(|| {
        use tracing_subscriber::layer::SubscriberExt;
        let buf = Arc::new(LogBuffer::new(100_000));
        let subscriber = tracing_subscriber::registry().with(BufferLayer::new(buf.clone()));
        tracing::subscriber::set_global_default(subscriber).expect("the only global subscriber");
        buf
    })
    .clone()
}

async fn post(a: &App, path: &str, cookie: Option<&str>, body: &str) -> Resp {
    let len = body.len().to_string();
    let mut hs: Vec<(&str, &[u8])> = vec![
        ("origin", ORIGIN.as_bytes()),
        ("content-type", b"application/json"),
        ("content-length", len.as_bytes()),
    ];
    if let Some(c) = cookie {
        hs.push(("cookie", c.as_bytes()));
    }
    send(
        &a.router,
        build("POST", path, Some(HOST), &hs, Body::from(body.to_string())),
    )
    .await
}

fn session_cookie(r: &Resp) -> String {
    r.headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("xexam_session="))
        .and_then(|v| v.split(';').next())
        .expect("session cookie")
        .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_log_never_holds_passwords_tokens_cookies_or_question_text() {
    let buf = captured();
    let a = app();
    let stem = "Which made-up herb grows on the moon (stem 7d1c)?";
    a.store
        .apply_import(
            "moon.xlsx",
            &[dbc::sheet("Moon", 1)],
            &[dbc::single("Moon", "m1", stem, 2)],
        )
        .expect("import");
    let secrets = [
        "first-admin-pass-81",
        "wrong-pass-guess-27",
        "cook-initial-pass-33",
        "cook-reset-pass-45",
    ];
    // the default administrator: forced change, then sign out and back in
    let r = post(
        &a,
        "/api/auth/login",
        None,
        r#"{"username": "admin", "password": "admin"}"#,
    )
    .await;
    assert_eq!(r.status, 200);
    let c = session_cookie(&r);
    let r = post(
        &a,
        "/api/auth/password",
        Some(&c),
        &json!({"password": secrets[0]}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200);
    let c = session_cookie(&r);
    // failed sign-ins until the lock, for an account and for an unknown name
    for name in ["admin", "nobody-here"] {
        for _ in 0..7 {
            post(
                &a,
                "/api/auth/login",
                None,
                &json!({"username": name, "password": secrets[1]}).to_string(),
            )
            .await;
        }
    }
    // users: create, reset the password, sign in as them
    let r = post(
        &a,
        "/api/users/create",
        Some(&c),
        &json!({"username": "cook", "password": secrets[2], "role": "user", "banks": "all"})
            .to_string(),
    )
    .await;
    assert_eq!(r.status, 200);
    let id = r.json()["users"]
        .as_array()
        .expect("users")
        .iter()
        .find(|u| u["username"] == "cook")
        .and_then(|u| u["id"].as_i64())
        .expect("cook");
    let r = post(
        &a,
        "/api/users/password",
        Some(&c),
        &json!({"id": id, "password": secrets[3]}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200);
    let r = post(
        &a,
        "/api/auth/login",
        None,
        &json!({"username": "cook", "password": secrets[3]}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200);
    let uc = session_cookie(&r);
    // requests with a bad or foreign cookie, a foreign Host and Origin, an admin route as a plain user, an exam
    let bad = format!("xexam_session={}", auth::new_session_token());
    get(&a, "/api/bank", Some(&bad)).await;
    get(&a, "/api/logs", Some(&uc)).await;
    send(
        &a.router,
        build(
            "GET",
            "/api/bank",
            Some("evil.example"),
            &[("cookie", c.as_bytes())],
            Body::empty(),
        ),
    )
    .await;
    let r = send(
        &a.router,
        build(
            "POST",
            "/api/attempt",
            Some(HOST),
            &[
                ("cookie", c.as_bytes()),
                ("origin", b"https://evil.example"),
                ("content-type", b"application/json"),
            ],
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(r.status, 403);
    let r = post(
        &a,
        "/api/attempt",
        Some(&c),
        r#"{"results": [{"id": "m1", "correct": false, "answer": ["A"]}], "sheets": ["Moon"], "mode": "all", "seconds": 3}"#,
    )
    .await;
    assert_eq!(r.status, 200);
    post(&a, "/api/auth/logout", Some(&uc), "{}").await;

    let text: String = buf
        .since(None)
        .lines
        .iter()
        .map(|l| format!("{} {} {}\n", l.level, l.target, l.message))
        .collect();
    assert!(text.contains("signed in"), "the flow was logged:\n{text}");
    assert!(text.contains("locked for"), "the lock was logged:\n{text}");
    // v0.7: users are named by their username, never by id; a typed name that matches no account is never written
    for want in [
        "user \"admin\" changed their password",
        "failed sign-in of user \"admin\"",
        // (the failed sign-ins of the unknown name come within the same minute: the burst summary holds them back)
        "sign-in of a username without an account from other browsers locked for",
        "sign-in of user \"admin\" from other browsers locked for",
        "user \"admin\" created user \"cook\"",
        "user \"admin\" reset the password of user \"cook\"",
        "user \"cook\" signed in",
    ] {
        assert!(text.contains(want), "{want:?} is missing:\n{text}");
    }
    for unwanted in ["user 1 ", "user 2 ", "of user 1", "of user 2"] {
        assert!(
            !text.contains(unwanted),
            "{unwanted:?} names a user by id:\n{text}"
        );
    }
    let mut forbidden: Vec<String> = secrets.iter().map(|s| s.to_string()).collect();
    for cookie in [&c, &uc, &bad] {
        let value = cookie.split_once('=').map(|(_, v)| v).expect("value");
        forbidden.push(value.to_string());
        forbidden.push(auth::token_hash(value));
    }
    forbidden.push(stem.to_string());
    forbidden.push("nobody-here".to_string());
    forbidden.push("stem 7d1c".to_string());
    forbidden.push("中文题干".to_string());
    for s in &forbidden {
        assert!(
            !text.contains(s.as_str()),
            "the log contains {s:?}:\n{text}"
        );
    }
    // no password hash either
    assert!(!text.contains("$argon2"), "{text}");
}
