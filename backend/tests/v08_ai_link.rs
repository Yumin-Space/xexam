//! v0.8: the main service's side of the AI link — schema v5 (`users.ai_enabled`, `users.uid`: the migration from v4, fresh
//! databases, the stamp fix-up after a rollback), the AI+ switch in the users API and in the signed-in user, the auth step
//! `GET /internal/ai/auth` with its signed identity statement, the token routes
//! `POST /internal/ai/{introspect,users,log,heartbeat}`, what ai-service's last heartbeat changes (`"aiAvailable"` /
//! `"aiService"` in `GET /api/auth/state`, `"aiAvailable"` / `"aiSuspended"` in `/api/users*`, the 409 refusal of a user's
//! AI+ service switch while AI+ is not available), and where `serve` finds or creates the internal token. Made-up users and
//! an obviously fake token only.
#[path = "api_common/mod.rs"]
mod apic;
#[path = "common/mod.rs"]
mod dbc;

use apic::{build, send, Resp};
use axum::body::Body;
use axum::Router;
use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use xexam_backend::api::guard;
use xexam_backend::api::internal::InternalToken;
use xexam_backend::api::{build_router, AppState};
use xexam_backend::auth::{self, Role};
use xexam_backend::config::Config;
use xexam_backend::database::{Store, UserChange};
use xexam_backend::logs::{BufferLayer, BurstReport, LogBuffer};
use xexam_backend::models::BankAccess;
use xexam_backend::services::RealEngine;

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";
/// The request's own Host when Nginx calls the backend (`proxy_pass http://backend:8765/…`): never on the allowlist, and
/// never looked at by the auth step
const NGINX_HOST: &str = "backend:8765";
const TOKEN: &str = "fake-internal-token-for-tests-0123456789";

const MSG_SIGN_IN_REQUIRED: &str = "请先登录。";
const MSG_PASSWORD_CHANGE_REQUIRED: &str = "请先设置新密码。";
const MSG_BAD_REQUEST: &str = "请求的格式不对。";
const MSG_AI_SERVICE_OFF: &str = "请先在 AI 设置里开启 AI+ 功能。";

// ---------------------------------------------------------------- helpers

struct App {
    router: Router,
    state: AppState,
    store: Arc<Store>,
    logs: Arc<LogBuffer>,
    db: PathBuf,
    _dir: tempfile::TempDir,
}

fn app_cfg(cfg: Config, link: bool) -> App {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let store = Arc::new(Store::open(&db).expect("store"));
    let engine = Arc::new(RealEngine::new(store.clone()));
    let logs = Arc::new(LogBuffer::default());
    let state = AppState::new_with_logs(cfg, store.clone(), engine, None, logs.clone());
    if link {
        assert!(state
            .0
            .ai_link
            .set_token(InternalToken::new(TOKEN).expect("token")));
    }
    App {
        router: build_router(state.clone()),
        state,
        store,
        logs,
        db,
        _dir: dir,
    }
}

fn app() -> App {
    app_cfg(apic::default_config(), true)
}

fn raw(p: &Path) -> Connection {
    Connection::open(p).expect("raw open")
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).expect("count")
}

fn text(c: &Connection, sql: &str) -> String {
    c.query_row(sql, [], |r| r.get(0)).expect("text")
}

fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Creates a user with the store, as `POST /api/users/create` does (the password hash is a placeholder: these users never
/// sign in through the API; their sessions are started with the store).
fn add_user(a: &App, name: &str, role: Role, ai: bool) -> i64 {
    match a
        .store
        .create_user_opts(
            name,
            "placeholder-hash",
            role,
            &BankAccess::Only(Vec::new()),
            false,
            false,
            ai,
        )
        .expect("create")
    {
        UserChange::Done(id) => id,
        other => panic!("{other:?}"),
    }
}

/// Clears the "must change the password" flag of user `id` (keeping the password; ends its sessions).
fn clear_flag(a: &App, id: i64) {
    let u = a.store.user_by_id(id).expect("lookup").expect("user");
    a.store
        .change_own_password(
            id,
            &u.password_hash,
            &u.password_hash,
            &auth::token_hash(&auth::new_session_token()),
            auth::unix_now(),
        )
        .expect("clear")
        .expect("user");
}

/// Starts a session for user `id` with the store (as the sign-in route does). Returns the `Cookie` value and the token
/// hash (the `sid` of the identity statement).
fn session(a: &App, id: i64) -> (String, String) {
    let u = a.store.user_by_id(id).expect("lookup").expect("user");
    let token = auth::new_session_token();
    let hash = auth::token_hash(&token);
    a.store
        .start_session(id, &hash, &u.password_hash, auth::unix_now())
        .expect("start")
        .expect("enabled");
    (format!("{}={token}", auth::SESSION_COOKIE), hash)
}

/// The default administrator (id 1) without the pending password change, with a session.
fn admin(a: &App) -> (String, String) {
    clear_flag(a, 1);
    session(a, 1)
}

fn uid_of(a: &App, id: i64) -> String {
    a.store.user_by_id(id).expect("lookup").expect("user").uid
}

/// `GET /internal/ai/auth` as Nginx sends it: the browser's cookie and the `X-Original-*` headers.
async fn auth_step(
    a: &App,
    cookie: Option<&str>,
    method: Option<&str>,
    host: Option<&str>,
    origin: Option<&str>,
) -> Resp {
    let mut hs: Vec<(&str, &[u8])> = Vec::new();
    if let Some(c) = cookie {
        hs.push(("cookie", c.as_bytes()));
    }
    if let Some(m) = method {
        hs.push(("x-original-method", m.as_bytes()));
    }
    if let Some(h) = host {
        hs.push(("x-original-host", h.as_bytes()));
    }
    if let Some(o) = origin {
        hs.push(("x-original-origin", o.as_bytes()));
    }
    send(
        &a.router,
        build(
            "GET",
            "/internal/ai/auth",
            Some(NGINX_HOST),
            &hs,
            Body::empty(),
        ),
    )
    .await
}

/// The auth step of a browser GET from the allowed host.
async fn auth_get(a: &App, cookie: &str) -> Resp {
    auth_step(a, Some(cookie), Some("GET"), Some(HOST), None).await
}

/// A token route as the AI service calls it (`token`: the `X-Xexam-Internal-Token` header).
async fn internal(a: &App, path: &str, token: Option<&str>, body: &str) -> Resp {
    let len = body.len().to_string();
    let mut hs: Vec<(&str, &[u8])> = vec![
        ("content-type", b"application/json"),
        ("content-length", len.as_bytes()),
    ];
    if let Some(t) = token {
        hs.push(("x-xexam-internal-token", t.as_bytes()));
    }
    send(
        &a.router,
        build(
            "POST",
            path,
            Some(NGINX_HOST),
            &hs,
            Body::from(body.to_string()),
        ),
    )
    .await
}

/// A heartbeat body as ai-service sends it (compact JSON, keys in the contract's order).
fn beat(enabled: bool, suspended: &[i64], stopping: bool) -> String {
    json!({"version": "0.8", "enabled": enabled, "suspended": suspended, "stopping": stopping})
        .to_string()
}

/// `POST /internal/ai/heartbeat` with the test token.
async fn heartbeat(a: &App, body: &str) -> Resp {
    internal(a, "/internal/ai/heartbeat", Some(TOKEN), body).await
}

/// An accepted heartbeat: ai-service reports `enabled`, the `suspended` users and whether it is stopping.
async fn report(a: &App, enabled: bool, suspended: &[i64], stopping: bool) {
    let r = heartbeat(a, &beat(enabled, suspended, stopping)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json(), json!({"ok": true}));
}

async fn api(a: &App, method: &str, path: &str, cookie: &str, body: Option<&str>) -> Resp {
    let mut hs: Vec<(&str, Vec<u8>)> = vec![("cookie", cookie.as_bytes().to_vec())];
    let body = match body {
        Some(b) => {
            hs.push(("origin", ORIGIN.as_bytes().to_vec()));
            hs.push(("content-type", b"application/json".to_vec()));
            hs.push(("content-length", b.len().to_string().into_bytes()));
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let hrefs: Vec<(&str, &[u8])> = hs.iter().map(|(k, v)| (*k, v.as_slice())).collect();
    send(&a.router, build(method, path, Some(HOST), &hrefs, body)).await
}

fn b64url_decode(s: &str) -> Vec<u8> {
    let val = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'-' => 62,
            b'_' => 63,
            _ => panic!("not base64url: {c}"),
        }
    };
    let mut out = Vec::new();
    for chunk in s.as_bytes().chunks(4) {
        assert!(chunk.len() >= 2, "no padding, no stray character");
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= val(c) << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    out
}

/// HMAC-SHA-256, written here again (independent of the backend's own) to check the statement as the AI service will.
fn hmac(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|b| b ^ 0x36));
    inner.update(msg);
    let mut outer = Sha256::new();
    outer.update(block.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().to_vec()
}

/// Checks the identity statement with `key` the way the AI service does and returns its payload.
fn open_statement(statement: &str, key: &str) -> Option<Value> {
    assert!(statement.len() <= 2048, "{}", statement.len());
    let (p64, s64) = statement.split_once('.')?;
    assert!(!s64.contains('.'), "exactly one dot");
    if b64url_decode(s64)
        != hmac(
            key.as_bytes(),
            format!("xexam-identity-v1.{p64}").as_bytes(),
        )
    {
        return None;
    }
    let payload = String::from_utf8(b64url_decode(p64)).expect("utf-8");
    // compact JSON (usernames never contain spaces)
    assert!(
        !payload.contains(": ") && !payload.contains(", "),
        "{payload}"
    );
    Some(serde_json::from_str(&payload).expect("json"))
}

/// The statement of an allowed auth step (204, no body, no cookie), checked with the test token.
fn statement_of(r: &Resp) -> Value {
    assert_eq!(r.status, 204, "{}", r.text());
    assert!(r.body.is_empty());
    assert_eq!(r.header("cache-control").as_deref(), Some("no-store"));
    assert!(r.header("set-cookie").is_none());
    assert!(r.header("x-xexam-deny").is_none());
    let s = r.header("x-xexam-identity").expect("identity");
    open_statement(&s, TOKEN).expect("the signature verifies with the internal token")
}

fn keys(v: &Value) -> Vec<String> {
    v.as_object()
        .expect("object")
        .keys()
        .cloned()
        .collect::<Vec<_>>()
}

/// A refused auth step: `status`, `X-Xexam-Deny: deny`, `{"error": <error>, "code": <code>}`.
fn assert_denied(r: &Resp, status: u16, deny: &str, code: &str, error: &str) {
    assert_eq!(r.status, status, "{}", r.text());
    assert_eq!(r.header("x-xexam-deny").as_deref(), Some(deny));
    assert!(r.header("x-xexam-identity").is_none());
    assert_eq!(r.json(), json!({"error": error, "code": code}));
    assert_eq!(
        r.header("content-type").as_deref(),
        Some("application/json; charset=utf-8")
    );
}

fn assert_not_found(r: &Resp) {
    assert_eq!(r.status, 404, "{}", r.text());
    assert_eq!(r.text(), "not found");
    assert_eq!(
        r.header("content-type").as_deref(),
        Some("text/plain; charset=utf-8")
    );
}

/// The `sqlite_master` rows of a database, in order.
fn sqlite_master(path: &Path) -> Vec<Value> {
    let c = raw(path);
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

fn backups(dir: &Path) -> Vec<String> {
    match std::fs::read_dir(dir.join("backups")) {
        Ok(rd) => {
            let mut v: Vec<String> = rd
                .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        }
        Err(_) => Vec::new(),
    }
}

// ---------------------------------------------------------------- schema v5

#[test]
fn a_fresh_database_is_schema_5_and_every_user_gets_a_stamp() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = Store::open(&db).expect("open");
    assert!(s.created_default_admin());
    assert!(s.migration_backup().is_none());
    let c = raw(&db);
    assert_eq!(text(&c, "SELECT value FROM meta WHERE key='schema'"), "5");
    // the v5 columns come after the v4 ones
    let columns: Vec<String> = c
        .prepare("SELECT name FROM pragma_table_info('users') ORDER BY cid")
        .expect("prepare")
        .query_map([], |r| r.get(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(
        columns[columns.len() - 4..],
        ["new_banks", "can_manage_banks", "ai_enabled", "uid"]
    );
    // the default administrator (created by the v3 step, before the column existed) has a stamp and AI+ off
    let admin = s.user_by_id(1).expect("lookup").expect("admin");
    assert!(is_lower_hex(&admin.uid, 32), "{}", admin.uid);
    assert!(!admin.ai_enabled);
    // users created later get their own stamp, set once
    let ids: Vec<i64> = ["ann", "ben", "cat"]
        .iter()
        .map(|n| {
            match s
                .create_user(n, "placeholder-hash", Role::User)
                .expect("create")
            {
                UserChange::Done(id) => id,
                other => panic!("{other:?}"),
            }
        })
        .collect();
    let stamps: Vec<String> = ids
        .iter()
        .map(|id| s.user_by_id(*id).expect("lookup").expect("user").uid)
        .collect();
    for stamp in &stamps {
        assert!(is_lower_hex(stamp, 32), "{stamp}");
    }
    assert_eq!(
        count(&c, "SELECT COUNT(DISTINCT uid) FROM users"),
        4,
        "unique"
    );
    drop(c);
    drop(s);
    // reopening changes nothing, the stamps included
    let bytes = dbc::db_bytes(&db);
    let s = Store::open(&db).expect("reopen");
    assert_eq!(
        s.user_by_id(ids[0]).expect("lookup").expect("user").uid,
        stamps[0]
    );
    drop(s);
    assert_eq!(dbc::db_bytes(&db), bytes);
}

/// A schema-4 database (v0.7.5) with data, made from a schema-5 one the way `tests/integration/migrate_check.py` does:
/// the two v5 columns dropped and the schema set back to 4. Users: admin (1), ann (2), ben (3, AI+ was on, dropped).
fn write_v4_database(db: &Path) {
    let s = Store::open(db).expect("open");
    for (name, ai) in [("ann", false), ("ben", true)] {
        s.create_user_opts(
            name,
            "placeholder-hash",
            Role::User,
            &BankAccess::Only(Vec::new()),
            true,
            false,
            ai,
        )
        .expect("create");
    }
    drop(s);
    raw(db)
        .execute_batch(
            "ALTER TABLE users DROP COLUMN ai_enabled; ALTER TABLE users DROP COLUMN uid; \
             UPDATE meta SET value = '4' WHERE key = 'schema';",
        )
        .expect("back to schema 4");
}

#[test]
fn a_v4_database_is_upgraded_to_schema_5_after_one_backup() {
    let dir = dbc::tmp();
    let fresh = dir.path().join("fresh").join("xexam.db");
    drop(Store::open(&fresh).expect("fresh"));
    let db = dir.path().join("xexam.db");
    write_v4_database(&db);
    let before: Vec<(i64, String, i64)> = raw(&db)
        .prepare("SELECT id, username, new_banks FROM users ORDER BY id")
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(before.len(), 3);
    let s = Store::open(&db).expect("open");
    assert!(!s.converted_prerelease());
    assert!(!s.created_default_admin());
    let backup = s.migration_backup().expect("backup").to_path_buf();
    let name = backup.file_name().and_then(|n| n.to_str()).expect("name");
    assert!(name.starts_with("xexam-schema4-"), "{name}");
    assert_eq!(backups(dir.path()).len(), 1);
    // the backup is the v4 database, untouched
    {
        let b = raw(&backup);
        assert_eq!(text(&b, "SELECT value FROM meta WHERE key='schema'"), "4");
        assert_eq!(
            count(
                &b,
                "SELECT COUNT(*) FROM pragma_table_info('users') WHERE name IN ('ai_enabled', 'uid')"
            ),
            0
        );
    }
    let c = raw(&db);
    assert_eq!(text(&c, "SELECT value FROM meta WHERE key='schema'"), "5");
    // every user: AI+ off (also ben, whose earlier switch went with the dropped column), a unique 32-hex stamp
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM users WHERE ai_enabled = 0"),
        3
    );
    let stamps: Vec<String> = c
        .prepare("SELECT uid FROM users ORDER BY id")
        .expect("prepare")
        .query_map([], |r| r.get(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    for stamp in &stamps {
        assert!(is_lower_hex(stamp, 32), "{stamp}");
    }
    assert_eq!(count(&c, "SELECT COUNT(DISTINCT uid) FROM users"), 3);
    // nothing else changed
    let after: Vec<(i64, String, i64)> = c
        .prepare("SELECT id, username, new_banks FROM users ORDER BY id")
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(after, before);
    drop(c);
    drop(s);
    // a fresh database and an upgraded one have the same schema
    assert_eq!(sqlite_master(&db), sqlite_master(&fresh));
    // the next open has nothing to do
    let bytes = dbc::db_bytes(&db);
    let s = Store::open(&db).expect("reopen");
    assert!(s.migration_backup().is_none());
    drop(s);
    assert_eq!(dbc::db_bytes(&db), bytes, "byte-identical");
    assert_eq!(backups(dir.path()).len(), 1);
}

#[test]
fn users_an_older_binary_created_after_a_rollback_get_a_stamp_on_the_next_open() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = Store::open(&db).expect("open");
    let admin_uid = s.user_by_id(1).expect("lookup").expect("admin").uid;
    drop(s);
    // v0.7.5 after a rollback reads "5" as "nothing to migrate" and creates users without the v5 columns
    raw(&db)
        .execute_batch(
            "INSERT INTO users(username, password_hash, role, disabled, created_at, password_changed_at, \
             must_change_password, new_banks, can_manage_banks) \
             VALUES ('dana', 'x', 'user', 0, '2026-02-01 10:00', '2026-02-01 10:00', 1, 0, 0);",
        )
        .expect("older binary user");
    assert_eq!(
        count(&raw(&db), "SELECT COUNT(*) FROM users WHERE uid = ''"),
        1
    );
    let s = Store::open(&db).expect("open");
    assert!(s.migration_backup().is_none(), "not a migration");
    let dana = s.user_by_name("dana").expect("lookup").expect("dana");
    assert!(is_lower_hex(&dana.uid, 32), "{}", dana.uid);
    assert!(!dana.ai_enabled);
    assert_eq!(
        s.user_by_id(1).expect("lookup").expect("admin").uid,
        admin_uid,
        "a stamp never changes"
    );
    drop(s);
    let bytes = dbc::db_bytes(&db);
    drop(Store::open(&db).expect("reopen"));
    assert_eq!(dbc::db_bytes(&db), bytes, "check first, then write");
}

// ---------------------------------------------------------------- the AI+ switch in the API

#[tokio::test]
async fn ai_in_the_users_api_and_ai_allowed_in_the_signed_in_user() {
    use tracing_subscriber::layer::SubscriberExt;
    let lines = Arc::new(LogBuffer::new(1000));
    let _guard = tracing::subscriber::set_default(
        tracing_subscriber::registry().with(BufferLayer::new(lines.clone())),
    );
    let logged = |from: u64| -> Vec<String> {
        lines
            .since(Some(from))
            .lines
            .into_iter()
            .map(|l| l.message)
            .collect()
    };
    let a = app();
    let (c, _) = admin(&a);
    // AI+ is available (ai-service reported the AI+ service on): users' AI+ service may be switched
    report(&a, true, &[], false).await;
    // the administrator may always use AI (whatever is stored)
    let me = api(&a, "GET", "/api/auth/state", &c, None).await.json();
    assert_eq!(
        keys(&me["user"]),
        [
            "id",
            "username",
            "role",
            "mustChangePassword",
            "canManageBanks",
            "aiAllowed"
        ]
    );
    assert_eq!(me["user"]["aiAllowed"], true);
    // create with and without AI+
    let mark = lines.since(None).last;
    let r = api(
        &a,
        "POST",
        "/api/users/create",
        &c,
        Some(r#"{"username": "sam", "password": "initial-pass", "role": "user", "ai": true}"#),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(
        logged(mark),
        [
            r#"user "admin" created user "sam""#,
            r#"user "admin" turned AI+ on for user "sam""#
        ]
    );
    let mark = lines.since(None).last;
    let r = api(
        &a,
        "POST",
        "/api/users/create",
        &c,
        Some(r#"{"username": "kim", "password": "initial-pass", "role": "user"}"#),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(logged(mark), [r#"user "admin" created user "kim""#]);
    let users = r.json()["users"].clone();
    let row = |name: &str| -> Value {
        users
            .as_array()
            .expect("users")
            .iter()
            .find(|u| u["username"] == name)
            .cloned()
            .expect("row")
    };
    assert_eq!(
        keys(&row("sam")),
        [
            "id",
            "username",
            "role",
            "disabled",
            "createdAt",
            "lastLoginAt",
            "mustChangePassword",
            "records",
            "banks",
            "newBanks",
            "canManageBanks",
            "ai",
            "ownedBanks",
            "lock",
            "aiSuspended"
        ]
    );
    assert_eq!(row("sam")["ai"], true);
    assert_eq!(row("kim")["ai"], false);
    assert_eq!(row("admin")["ai"], false, "the stored flag, not aiAllowed");
    // the stamp is never sent to the browser
    let sam = row("sam")["id"].as_i64().expect("id");
    let kim = row("kim")["id"].as_i64().expect("id");
    let sam_uid = uid_of(&a, sam);
    assert!(is_lower_hex(&sam_uid, 32));
    assert!(!r.text().contains(&sam_uid));
    // anything but a bool is refused
    for bad in [r#""yes""#, "1", "null"] {
        let r = api(
            &a,
            "POST",
            "/api/users/create",
            &c,
            Some(&format!(
                r#"{{"username": "x1", "password": "initial-pass", "role": "user", "ai": {bad}}}"#
            )),
        )
        .await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (400, MSG_BAD_REQUEST.into())
        );
        let r = api(
            &a,
            "POST",
            "/api/users/update",
            &c,
            Some(&format!(r#"{{"id": {sam}, "ai": {bad}}}"#)),
        )
        .await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (400, MSG_BAD_REQUEST.into())
        );
    }
    assert!(a.store.user_by_name("x1").expect("lookup").is_none());
    // what the signed-in user sees, read on every request
    clear_flag(&a, sam);
    clear_flag(&a, kim);
    let (sc, _) = session(&a, sam);
    let (kc, _) = session(&a, kim);
    let state = api(&a, "GET", "/api/auth/state", &sc, None).await;
    assert_eq!(state.json()["user"]["aiAllowed"], true);
    assert!(!state.text().contains(&sam_uid));
    assert_eq!(
        api(&a, "GET", "/api/auth/state", &kc, None).await.json()["user"]["aiAllowed"],
        false
    );
    // switching AI+ off: one line (no "updated" line), effective at once, no new sign-in needed
    let mark = lines.since(None).last;
    let r = api(
        &a,
        "POST",
        "/api/users/update",
        &c,
        Some(&format!(r#"{{"id": {sam}, "ai": false}}"#)),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(
        logged(mark),
        [r#"user "admin" turned AI+ off for user "sam""#]
    );
    assert_eq!(
        api(&a, "GET", "/api/auth/state", &sc, None).await.json()["user"]["aiAllowed"],
        false
    );
    // no change: the usual line only
    let mark = lines.since(None).last;
    api(
        &a,
        "POST",
        "/api/users/update",
        &c,
        Some(&format!(r#"{{"id": {sam}, "ai": false}}"#)),
    )
    .await;
    assert_eq!(logged(mark), [r#"user "admin" updated user "sam""#]);
    // together with another change: both lines
    let mark = lines.since(None).last;
    api(
        &a,
        "POST",
        "/api/users/update",
        &c,
        Some(&format!(
            r#"{{"id": {kim}, "username": "kimi", "ai": true}}"#
        )),
    )
    .await;
    assert_eq!(
        logged(mark),
        [
            r#"user "admin" updated user "kimi" (renamed from "kim")"#,
            r#"user "admin" turned AI+ on for user "kimi""#
        ]
    );
    assert_eq!(
        api(&a, "GET", "/api/auth/state", &kc, None).await.json()["user"]["aiAllowed"],
        true,
        "a rename keeps the session"
    );
    // a role change keeps the stored switch; an administrator always may
    api(
        &a,
        "POST",
        "/api/users/update",
        &c,
        Some(&format!(r#"{{"id": {sam}, "role": "admin"}}"#)),
    )
    .await;
    let u = a.store.user_by_id(sam).expect("lookup").expect("sam");
    assert!(!u.ai_enabled);
    assert_eq!(u.uid, sam_uid, "the stamp never changes");
    let (sc, _) = session(&a, sam);
    assert_eq!(
        api(&a, "GET", "/api/auth/state", &sc, None).await.json()["user"]["aiAllowed"],
        true
    );
}

// ---------------------------------------------------------------- GET /internal/ai/auth

#[tokio::test]
async fn the_auth_step_signs_who_is_asking() {
    let a = app();
    let (ac, asid) = admin(&a);
    let sam = add_user(&a, "sam", Role::User, true);
    let kim = add_user(&a, "kim", Role::User, false);
    clear_flag(&a, sam);
    clear_flag(&a, kim);
    let (sc, ssid) = session(&a, sam);
    let (kc, _) = session(&a, kim);

    let now = auth::unix_now();
    let st = statement_of(&auth_get(&a, &ac).await);
    assert_eq!(
        keys(&st),
        [
            "v",
            "id",
            "stamp",
            "username",
            "role",
            "aiAllowed",
            "sid",
            "exp"
        ]
    );
    assert_eq!(st["v"], 1);
    assert_eq!(st["id"], 1);
    assert_eq!(st["stamp"], uid_of(&a, 1));
    assert_eq!(st["username"], "admin");
    assert_eq!(st["role"], "admin");
    assert_eq!(st["aiAllowed"], true);
    assert_eq!(st["sid"], asid.as_str());
    let exp = st["exp"].as_i64().expect("exp");
    assert!(
        (now + 59..=auth::unix_now() + 61).contains(&exp),
        "{exp} vs {now}"
    );
    // a plain user with and without AI+
    let st = statement_of(&auth_get(&a, &sc).await);
    assert_eq!(
        (
            st["id"].clone(),
            st["username"].clone(),
            st["role"].clone(),
            st["aiAllowed"].clone(),
            st["sid"].clone(),
            st["stamp"].clone()
        ),
        (
            json!(sam),
            json!("sam"),
            json!("user"),
            json!(true),
            json!(ssid),
            json!(uid_of(&a, sam))
        )
    );
    let st = statement_of(&auth_get(&a, &kc).await);
    assert_eq!(
        (st["username"].clone(), st["aiAllowed"].clone()),
        (json!("kim"), json!(false))
    );
    // the signature is the internal token's: another key does not verify it
    let r = auth_get(&a, &kc).await;
    let s = r.header("x-xexam-identity").expect("identity");
    assert!(open_statement(&s, "another-fake-token-0000000000000000").is_none());
    // a POST from the same origin, and one without any Origin (not a browser), are fine too
    statement_of(&auth_step(&a, Some(&sc), Some("POST"), Some(HOST), Some(ORIGIN)).await);
    statement_of(&auth_step(&a, Some(&sc), Some("POST"), Some(HOST), None).await);
    // an empty Origin counts as none
    statement_of(&auth_step(&a, Some(&sc), Some("POST"), Some(HOST), Some("")).await);
}

#[tokio::test]
async fn the_auth_step_refuses_like_the_api() {
    let a = app();
    let (ac, _) = admin(&a);
    // no cookie, a cookie that is no session, garbage
    for cookie in [
        None,
        Some(format!(
            "{}={}",
            auth::SESSION_COOKIE,
            auth::new_session_token()
        )),
        Some("xexam_session=short".to_string()),
    ] {
        let r = auth_step(&a, cookie.as_deref(), Some("GET"), Some(HOST), None).await;
        assert_denied(&r, 401, "signin", "auth.signin", MSG_SIGN_IN_REQUIRED);
        assert_eq!(r.header("cache-control").as_deref(), Some("no-store"));
    }
    // a pending password change
    let pat = add_user(&a, "pat", Role::User, true);
    let (pc, _) = session(&a, pat);
    let r = auth_get(&a, &pc).await;
    assert_denied(
        &r,
        403,
        "password",
        "auth.password",
        MSG_PASSWORD_CHANGE_REQUIRED,
    );
    // a disabled user (the session row is still there): no valid session
    let dan = add_user(&a, "dan", Role::User, true);
    clear_flag(&a, dan);
    let (dc, dsid) = session(&a, dan);
    statement_of(&auth_get(&a, &dc).await);
    raw(&a.db)
        .execute_batch(&format!("UPDATE users SET disabled = 1 WHERE id = {dan}"))
        .expect("disable");
    assert_denied(
        &auth_get(&a, &dc).await,
        401,
        "signin",
        "auth.signin",
        MSG_SIGN_IN_REQUIRED,
    );
    // an expired session
    raw(&a.db)
        .execute_batch(&format!(
            "UPDATE users SET disabled = 0 WHERE id = {dan}; \
             UPDATE sessions SET expires_at = {} WHERE token_hash = '{dsid}'",
            auth::unix_now() - 1
        ))
        .expect("expire");
    assert_denied(
        &auth_get(&a, &dc).await,
        401,
        "signin",
        "auth.signin",
        MSG_SIGN_IN_REQUIRED,
    );
    // a deleted user
    let r = api(
        &a,
        "POST",
        "/api/users/delete",
        &ac,
        Some(&format!(r#"{{"id": {pat}}}"#)),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_denied(
        &auth_get(&a, &pc).await,
        401,
        "signin",
        "auth.signin",
        MSG_SIGN_IN_REQUIRED,
    );
}

/// The auth step of a browser POST with these `Cookie` headers (as Nginx passes them on, one per header line).
async fn auth_cookies(a: &App, cookies: &[&str]) -> Resp {
    let mut hs: Vec<(&str, &[u8])> = cookies.iter().map(|c| ("cookie", c.as_bytes())).collect();
    hs.push(("x-original-method", b"POST"));
    hs.push(("x-original-host", HOST.as_bytes()));
    hs.push(("x-original-origin", ORIGIN.as_bytes()));
    send(
        &a.router,
        build(
            "GET",
            "/internal/ai/auth",
            Some(NGINX_HOST),
            &hs,
            Body::empty(),
        ),
    )
    .await
}

/// Nginx keys the per-session limit on starting AI answers by `$cookie_xexam_session`: the first cookie of that name in
/// any letter case, its value as written, an empty value not counted. The auth step signs a request in only when that
/// key is the session's own token, so a second cookie, an empty one in front, another letter case or a padded value can
/// never move a signed-in user out of their bucket.
#[tokio::test]
async fn the_auth_step_takes_exactly_one_plain_session_cookie() {
    let a = app();
    let sam = add_user(&a, "sam", Role::User, true);
    clear_flag(&a, sam);
    let (sc, ssid) = session(&a, sam);
    let (kc, _) = session(&a, sam);
    let t = sc.split_once('=').expect("pair").1.to_string();
    let other = kc.split_once('=').expect("pair").1.to_string();
    // one plain pair, also among other cookies
    for ok in [
        sc.clone(),
        format!("theme=dark; {sc}; CF_Authorization=a.b.c"),
        format!("theme=dark;  {sc}"),
    ] {
        let st = statement_of(&auth_cookies(&a, &[&ok]).await);
        assert_eq!(st["sid"], ssid.as_str(), "{ok}");
    }
    statement_of(&auth_cookies(&a, &["theme=dark", &sc]).await);
    // everything Nginx would key differently (or not at all): no session
    for bad in [
        format!("xexam_session=; {sc}"),
        format!("{sc}; xexam_session="),
        format!("XEXAM_SESSION=junk; {sc}"),
        format!("{sc}; Xexam_Session={other}"),
        format!("{sc}; {sc}"),
        format!("{sc}; {kc}"),
        format!("{sc},xexam_session=x"),
        format!("xexam_session= {t}"),
        format!("xexam_session ={t}"),
        format!("xexam_session={t} ; theme=dark"),
        format!("xexam_session=\"{t}\""),
        format!("\t{sc}"),
        format!("XEXAM_SESSION={t}"),
    ] {
        let r = auth_cookies(&a, &[&bad]).await;
        assert_denied(&r, 401, "signin", "auth.signin", MSG_SIGN_IN_REQUIRED);
    }
    // the same across two Cookie header lines (HTTP/1.1 may send several)
    for pair in [
        ["xexam_session=", sc.as_str()],
        [sc.as_str(), kc.as_str()],
        [sc.as_str(), "XEXAM_SESSION=junk"],
    ] {
        let r = auth_cookies(&a, &pair).await;
        assert_denied(&r, 401, "signin", "auth.signin", MSG_SIGN_IN_REQUIRED);
    }
    // the rest of the API reads the cookie as before (no limit is keyed on it there)
    let r = api(
        &a,
        "GET",
        "/api/auth/state",
        &format!("xexam_session=; {sc}"),
        None,
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["user"]["username"], "sam", "{}", r.text());
}

#[tokio::test]
async fn the_auth_step_runs_the_host_and_origin_guard_on_the_original_request_only() {
    let a = app_cfg(
        apic::config_with(&[("XEXAM_ALLOWED_HOSTS", "app.example.com")]),
        true,
    );
    let (ac, _) = admin(&a);
    let cfg = &a.state.0.cfg;
    // a Host that is not allowed: 403 guard with the guard's own message
    let r = auth_step(&a, Some(&ac), Some("GET"), Some("evil.example.com"), None).await;
    let message = guard::deny(cfg, Some("evil.example.com"), None)
        .expect("denied")
        .message;
    assert_denied(&r, 403, "guard", "auth.guard", &message);
    // the request's own Host is never read: without X-Original-Host the guard refuses, even though Host is allowed
    let r = send(
        &a.router,
        build(
            "GET",
            "/internal/ai/auth",
            Some(HOST),
            &[("cookie", ac.as_bytes()), ("x-original-method", b"GET")],
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(r.status, 403, "{}", r.text());
    assert_eq!(r.header("x-xexam-deny").as_deref(), Some("guard"));
    // and an allowed X-Original-Host passes whatever the request's own Host is (Nginx sends `backend:8765`)
    statement_of(&auth_step(&a, Some(&ac), Some("GET"), Some("app.example.com"), None).await);
    statement_of(
        &auth_step(
            &a,
            Some(&ac),
            Some("POST"),
            Some("app.example.com"),
            Some("https://app.example.com"),
        )
        .await,
    );
    // a foreign Origin: refused for a POST, not for a GET (like /api/*)
    let evil = "https://evil.example.com";
    let r = auth_step(&a, Some(&ac), Some("POST"), Some(HOST), Some(evil)).await;
    let message = guard::deny(cfg, Some(HOST), Some(evil))
        .expect("denied")
        .message;
    assert_denied(&r, 403, "guard", "auth.guard", &message);
    statement_of(&auth_step(&a, Some(&ac), Some("GET"), Some(HOST), Some(evil)).await);
    // a missing method is checked like a POST
    let r = auth_step(&a, Some(&ac), None, Some(HOST), Some(evil)).await;
    assert_eq!(r.status, 403);
    // the guard runs before the session: no cookie at all still gets the guard's answer
    let r = auth_step(&a, None, Some("GET"), Some("evil.example.com"), None).await;
    assert_eq!(r.header("x-xexam-deny").as_deref(), Some("guard"));
    // rejections go through the same rate-limited log as the guard of /api/*: the first one was logged at once, the
    // later ones are held back and summed up
    match a
        .state
        .0
        .rejections
        .flush(Instant::now() + Duration::from_secs(61))
    {
        Some(BurstReport::Many { count, last }) => {
            assert!(count >= 4, "{count}");
            assert!(last.contains("AI request"), "{last}");
            assert!(last.contains("evil.example.com"), "{last}");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn the_auth_step_never_refreshes_a_session() {
    let a = app();
    let (ac, asid) = admin(&a);
    let old = auth::unix_now() - 2 * 3600;
    let stamps = |a: &App| -> (i64, i64) {
        raw(&a.db)
            .query_row(
                "SELECT last_seen_at, expires_at FROM sessions WHERE token_hash = ?1",
                [&asid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("session")
    };
    raw(&a.db)
        .execute(
            "UPDATE sessions SET last_seen_at = ?1, expires_at = ?2 WHERE token_hash = ?3",
            rusqlite::params![old, old + 10 * 24 * 3600, asid],
        )
        .expect("age the session");
    let before = stamps(&a);
    // past the refresh window, with an Origin (which lets /api/* refresh the cookie): nothing is written
    statement_of(&auth_step(&a, Some(&ac), Some("POST"), Some(HOST), Some(ORIGIN)).await);
    statement_of(&auth_get(&a, &ac).await);
    assert_eq!(stamps(&a), before);
    // the token route that re-checks a session does not refresh it either
    let r = internal(
        &a,
        "/internal/ai/introspect",
        Some(TOKEN),
        &json!({"sid": asid, "id": 1}).to_string(),
    )
    .await;
    assert_eq!(r.json()["ok"], true, "{}", r.text());
    assert_eq!(stamps(&a), before);
    // while an ordinary API request with an Origin does (the window really was open)
    let r = api(&a, "POST", "/api/settings", &ac, Some("{}")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(r.header("set-cookie").is_some());
    assert_ne!(stamps(&a), before);
}

#[tokio::test]
async fn without_a_token_the_auth_step_answers_503_and_the_token_routes_404() {
    let a = app_cfg(apic::default_config(), false);
    let (ac, asid) = admin(&a);
    let r = auth_get(&a, &ac).await;
    assert_eq!(r.status, 503);
    assert!(r.body.is_empty());
    assert!(r.header("x-xexam-identity").is_none());
    for (path, body) in [
        ("/internal/ai/introspect", json!({"sid": asid, "id": 1})),
        ("/internal/ai/users", json!({"ids": [1]})),
        (
            "/internal/ai/heartbeat",
            json!({"version": "0.8", "enabled": true, "suspended": [], "stopping": false}),
        ),
        (
            "/internal/ai/log",
            json!({"lines": [{"level": "INFO", "message": "hello"}]}),
        ),
    ] {
        assert_not_found(&internal(&a, path, Some(TOKEN), &body.to_string()).await);
    }
    assert!(a.logs.since(None).lines.is_empty());
    // the heartbeat was not taken: AI+ is not available, and the administrator sees that the link is off
    assert_eq!(
        a.state.0.ai_link.state(),
        xexam_backend::api::internal::ServiceState::default()
    );
    let state = api(&a, "GET", "/api/auth/state", &ac, None).await.json();
    assert_eq!(state["aiAvailable"], false);
    assert_eq!(
        state["aiService"],
        json!({"connected": false, "stopped": false, "lastSeen": null, "version": null, "enabled": false, "link": false})
    );
    match a
        .state
        .0
        .ai_link
        .flush_refusals(Instant::now() + Duration::from_secs(61))
    {
        Some(BurstReport::Many { count, last }) => {
            assert_eq!(count, 3, "the first was logged at once");
            assert_eq!(last, "POST /internal/ai/log (the AI link is off)");
        }
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------- the token routes

#[tokio::test]
async fn internal_paths_are_hidden_without_the_token() {
    let a = app();
    let (ac, asid) = admin(&a);
    let body = json!({"sid": asid, "id": 1}).to_string();
    for path in [
        "/internal/ai/introspect",
        "/internal/ai/users",
        "/internal/ai/heartbeat",
        "/internal/ai/log",
    ] {
        // no token, a wrong one, a prefix of the right one, the right one with a space
        for token in [
            None,
            Some("wrong-fake-token-0000000000000000000000"),
            Some(&TOKEN[..TOKEN.len() - 1]),
            Some(&*format!("{TOKEN} ")),
        ] {
            assert_not_found(&internal(&a, path, token, &body).await);
        }
        // the right token with another method
        for method in ["GET", "HEAD", "PUT", "DELETE"] {
            let r = send(
                &a.router,
                build(
                    method,
                    path,
                    Some(NGINX_HOST),
                    &[("x-xexam-internal-token", TOKEN.as_bytes())],
                    Body::empty(),
                ),
            )
            .await;
            assert_eq!(r.status, 404, "{method} {path}");
        }
    }
    // unknown internal paths, with the token
    for path in [
        "/internal/",
        "/internal/ai",
        "/internal/ai/",
        "/internal/ai/nope",
        "/internal/ai/log/",
        "/internal/ai/auth/x",
        "/internal/ai/heartbeat/",
        "/internal/ai/Heartbeat",
    ] {
        assert_not_found(&internal(&a, path, Some(TOKEN), "{}").await);
    }
    assert!(
        a.state.0.ai_link.state().last_seen.is_none(),
        "no heartbeat was taken"
    );
    // the auth step is GET only
    for method in ["POST", "HEAD", "PUT"] {
        let r = send(
            &a.router,
            build(
                method,
                "/internal/ai/auth",
                Some(NGINX_HOST),
                &[("cookie", ac.as_bytes())],
                Body::empty(),
            ),
        )
        .await;
        assert_eq!(r.status, 404, "{method}");
        assert!(r.header("x-xexam-identity").is_none());
    }
    // not reachable under /api/: an ordinary unknown API path there (sign-in first, then 404)
    for path in [
        "/api/internal/ai/log",
        "/api/internal/ai/users",
        "/api/internal/ai/heartbeat",
    ] {
        let r = send(
            &a.router,
            build(
                "POST",
                path,
                Some(HOST),
                &[
                    ("x-xexam-internal-token", TOKEN.as_bytes()),
                    ("content-type", b"application/json"),
                ],
                Body::from("{}"),
            ),
        )
        .await;
        assert_eq!(r.status, 401, "{}", r.text());
        let r = api(&a, "POST", path, &ac, Some("{}")).await;
        assert_eq!(r.status, 404, "{}", r.text());
    }
    let r = api(&a, "GET", "/api/internal/ai/auth", &ac, None).await;
    assert_eq!(r.status, 404);
    assert!(r.header("x-xexam-identity").is_none());
    assert!(
        a.logs.since(None).lines.is_empty(),
        "nothing was logged into the System log"
    );
    // the refusals are counted in the AI link's own rate-limited log
    match a
        .state
        .0
        .ai_link
        .flush_refusals(Instant::now() + Duration::from_secs(61))
    {
        Some(BurstReport::Many { count, last }) => {
            assert_eq!(count, 15, "16 token refusals, the first logged at once");
            assert_eq!(last, "POST /internal/ai/log");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn introspect_rechecks_a_session_without_refreshing_it() {
    let a = app();
    let (_, asid) = admin(&a);
    let sam = add_user(&a, "sam", Role::User, true);
    clear_flag(&a, sam);
    let (_, ssid) = session(&a, sam);
    let call = |sid: &str, id: i64| json!({"sid": sid, "id": id}).to_string();
    let r = internal(
        &a,
        "/internal/ai/introspect",
        Some(TOKEN),
        &call(&ssid, sam),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(
        r.header("content-type").as_deref(),
        Some("application/json; charset=utf-8")
    );
    let v = r.json();
    assert_eq!(keys(&v), ["ok", "user"]);
    assert_eq!(
        keys(&v["user"]),
        ["id", "stamp", "username", "role", "aiAllowed"]
    );
    assert_eq!(
        v,
        json!({"ok": true, "user": {"id": sam, "stamp": uid_of(&a, sam), "username": "sam", "role": "user", "aiAllowed": true}})
    );
    let r = internal(&a, "/internal/ai/introspect", Some(TOKEN), &call(&asid, 1)).await;
    assert_eq!(r.json()["user"]["role"], "admin");
    assert_eq!(r.json()["user"]["aiAllowed"], true);
    let not_ok = |r: &Resp| {
        assert_eq!(r.status, 200, "{}", r.text());
        assert_eq!(r.json(), json!({"ok": false}));
    };
    // another user's id, an unknown session
    not_ok(&internal(&a, "/internal/ai/introspect", Some(TOKEN), &call(&ssid, 1)).await);
    not_ok(
        &internal(
            &a,
            "/internal/ai/introspect",
            Some(TOKEN),
            &call(&"0".repeat(64), sam),
        )
        .await,
    );
    // AI+ switched off: still a session, aiAllowed false at once
    raw(&a.db)
        .execute_batch(&format!("UPDATE users SET ai_enabled = 0 WHERE id = {sam}"))
        .expect("off");
    let r = internal(
        &a,
        "/internal/ai/introspect",
        Some(TOKEN),
        &call(&ssid, sam),
    )
    .await;
    assert_eq!(r.json()["user"]["aiAllowed"], false);
    // a pending password change, a disabled user, an expired session
    raw(&a.db)
        .execute_batch(&format!(
            "UPDATE users SET must_change_password = 1 WHERE id = {sam}"
        ))
        .expect("pending");
    not_ok(
        &internal(
            &a,
            "/internal/ai/introspect",
            Some(TOKEN),
            &call(&ssid, sam),
        )
        .await,
    );
    raw(&a.db)
        .execute_batch(&format!(
            "UPDATE users SET must_change_password = 0, disabled = 1 WHERE id = {sam}"
        ))
        .expect("disabled");
    not_ok(
        &internal(
            &a,
            "/internal/ai/introspect",
            Some(TOKEN),
            &call(&ssid, sam),
        )
        .await,
    );
    raw(&a.db)
        .execute_batch(&format!(
            "UPDATE users SET disabled = 0 WHERE id = {sam}; UPDATE sessions SET expires_at = {} WHERE token_hash = '{ssid}'",
            auth::unix_now() - 1
        ))
        .expect("expired");
    not_ok(
        &internal(
            &a,
            "/internal/ai/introspect",
            Some(TOKEN),
            &call(&ssid, sam),
        )
        .await,
    );
    // malformed requests
    for body in [
        json!({"sid": ssid.to_uppercase(), "id": sam}),
        json!({"sid": &ssid[1..], "id": sam}),
        json!({"sid": 5, "id": sam}),
        json!({"id": sam}),
        json!({"sid": ssid, "id": "2"}),
        json!({"sid": ssid, "id": 0}),
        json!({"sid": ssid, "id": -1}),
        json!({"sid": ssid, "id": 1.5}),
        json!({"sid": ssid}),
        json!([1]),
    ] {
        let r = internal(
            &a,
            "/internal/ai/introspect",
            Some(TOKEN),
            &body.to_string(),
        )
        .await;
        assert_eq!(r.status, 400, "{body}");
    }
    let r = internal(&a, "/internal/ai/introspect", Some(TOKEN), "not json").await;
    assert_eq!(r.status, 400);
    // the content type and the size
    let r = send(
        &a.router,
        build(
            "POST",
            "/internal/ai/introspect",
            Some(NGINX_HOST),
            &[
                ("x-xexam-internal-token", TOKEN.as_bytes()),
                ("content-type", b"text/plain"),
            ],
            Body::from(call(&ssid, sam)),
        ),
    )
    .await;
    assert_eq!(r.status, 415);
    let big = json!({"sid": ssid, "id": sam, "pad": "x".repeat(70 * 1024)}).to_string();
    let r = internal(&a, "/internal/ai/introspect", Some(TOKEN), &big).await;
    assert_eq!(r.status, 413);
}

#[tokio::test]
async fn users_reports_the_requested_users_and_the_holders_of_ai_plus() {
    let a = app();
    let sam = add_user(&a, "sam", Role::User, true);
    let kim = add_user(&a, "kim", Role::User, false);
    let boss = add_user(&a, "boss", Role::Admin, false);
    let old = add_user(&a, "old", Role::User, true);
    raw(&a.db)
        .execute_batch(&format!("UPDATE users SET disabled = 1 WHERE id = {old}"))
        .expect("disable");
    let row = |id: i64, name: &str, role: &str, disabled: bool, ai: bool, allowed: bool| json!({"id": id, "stamp": uid_of(&a, id), "username": name, "role": role, "disabled": disabled, "ai": ai, "aiAllowed": allowed});
    let ask = |body: Value| {
        let a = &a;
        async move {
            let r = internal(a, "/internal/ai/users", Some(TOKEN), &body.to_string()).await;
            assert_eq!(r.status, 200, "{}", r.text());
            r.json()
        }
    };
    // only the requested users that exist (999: deleted), each once, by id
    let v = ask(json!({"ids": [999, kim, sam, kim], "holders": false})).await;
    assert_eq!(
        v,
        json!({"users": [row(sam, "sam", "user", false, true, true), row(kim, "kim", "user", false, false, false)]})
    );
    assert_eq!(
        keys(&v["users"][0]),
        [
            "id",
            "stamp",
            "username",
            "role",
            "disabled",
            "ai",
            "aiAllowed"
        ]
    );
    // the holders: every administrator and every user with AI+ (disabled ones too), plus the requested ones
    let v = ask(json!({"ids": [kim], "holders": true})).await;
    assert_eq!(
        v,
        json!({"users": [
            row(1, "admin", "admin", false, false, true),
            row(sam, "sam", "user", false, true, true),
            row(kim, "kim", "user", false, false, false),
            row(boss, "boss", "admin", false, false, true),
            row(old, "old", "user", true, true, true),
        ]})
    );
    assert_eq!(ask(json!({})).await, json!({"users": []}));
    assert_eq!(
        ask(json!({"holders": true})).await["users"]
            .as_array()
            .map(Vec::len),
        Some(4)
    );
    // malformed
    for body in [
        json!({"ids": [1, "2"]}),
        json!({"ids": [0]}),
        json!({"ids": [-3]}),
        json!({"ids": [1.0]}),
        json!({"ids": 1}),
        json!({"ids": [1], "holders": "yes"}),
        json!({"ids": (1..=10_001).collect::<Vec<i64>>()}),
    ] {
        let r = internal(&a, "/internal/ai/users", Some(TOKEN), &body.to_string()).await;
        assert_eq!(r.status, 400, "{}", r.text());
    }
    let r = internal(
        &a,
        "/internal/ai/users",
        Some(TOKEN),
        &json!({"ids": (1..=10_000).collect::<Vec<i64>>()}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["users"].as_array().map(Vec::len), Some(5));
}

#[tokio::test]
async fn log_lines_of_the_ai_service_go_into_the_system_log() {
    let a = app();
    let send_lines = |lines: Value| {
        let a = &a;
        async move {
            internal(
                a,
                "/internal/ai/log",
                Some(TOKEN),
                &json!({ "lines": lines }).to_string(),
            )
            .await
        }
    };
    let r = send_lines(json!([
        {"level": "INFO", "message": "AI link: connected to the main service"},
        {"level": "WARN", "message": "two\nlines\twith \u{1b}[31mcontrols\u{7f}\u{85}"},
        {"level": "ERROR", "message": "x".repeat(2500)},
        {"level": "INFO", "message": "y".repeat(2000)},
    ]))
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json(), json!({"ok": true}));
    let got: Vec<(String, String, String)> = a
        .logs
        .since(None)
        .lines
        .into_iter()
        .map(|l| (l.level.to_string(), l.target, l.message))
        .collect();
    assert_eq!(
        got,
        [
            (
                "info".to_string(),
                "xexam_ai_service".to_string(),
                "AI: AI link: connected to the main service".to_string()
            ),
            (
                "warn".to_string(),
                "xexam_ai_service".to_string(),
                "AI: twolineswith [31mcontrols".to_string()
            ),
            (
                "error".to_string(),
                "xexam_ai_service".to_string(),
                format!("AI: {}…", "x".repeat(2000))
            ),
            (
                "info".to_string(),
                "xexam_ai_service".to_string(),
                format!("AI: {}", "y".repeat(2000))
            ),
        ]
    );
    // 50 lines at most; one bad line refuses the whole batch (nothing is logged)
    let last = a.logs.since(None).last;
    let fifty: Vec<Value> = (0..50)
        .map(|i| json!({"level": "INFO", "message": format!("line {i}")}))
        .collect();
    assert_eq!(send_lines(json!(fifty)).await.status, 200);
    assert_eq!(a.logs.since(Some(last)).lines.len(), 50);
    let last = a.logs.since(None).last;
    let mut too_many = fifty.clone();
    too_many.push(json!({"level": "INFO", "message": "one more"}));
    for bad in [
        json!(too_many),
        json!([{"level": "INFO", "message": "fine"}, {"level": "DEBUG", "message": "no"}]),
        json!([{"level": "info", "message": "lower case"}]),
        json!([{"level": "INFO"}]),
        json!([{"level": "INFO", "message": 5}]),
        json!([{"message": "no level"}]),
        json!("not a list"),
        json!(null),
    ] {
        let r = send_lines(bad.clone()).await;
        assert_eq!(r.status, 400, "{bad}");
    }
    let r = internal(&a, "/internal/ai/log", Some(TOKEN), "{}").await;
    assert_eq!(r.status, 400);
    assert_eq!(a.logs.since(Some(last)).lines.len(), 0);
    assert_eq!(send_lines(json!([])).await.status, 200);
}

/// A full batch the contract allows (50 lines of 2,000 characters) is never refused for its size, however its characters
/// are written: plain ASCII, characters outside the BMP (4 bytes each) or `\u0001` escapes (6 bytes each, stripped
/// before logging). The other token routes keep their 64 KiB limit.
#[tokio::test]
async fn a_full_log_batch_fits_the_body_limit() {
    use xexam_backend::api::internal::{MAX_BODY, MAX_LOG_BODY, MAX_LOG_CHARS, MAX_LOG_LINES};
    let a = app();
    let batch = |message: &str, level: &str| {
        let line = json!({"level": level, "message": message});
        json!({ "lines": vec![line; MAX_LOG_LINES] }).to_string()
    };
    for (unit, logged) in [("a", "a"), ("\u{1f600}", "\u{1f600}"), ("\u{1}", "")] {
        let body = batch(&unit.repeat(MAX_LOG_CHARS), "ERROR");
        assert!(body.len() > MAX_BODY, "{}", body.len());
        assert!(body.len() <= MAX_LOG_BODY, "{}", body.len());
        let last = a.logs.since(None).last;
        let r = internal(&a, "/internal/ai/log", Some(TOKEN), &body).await;
        assert_eq!(r.status, 200, "{unit:?}: {}", r.text());
        let lines = a.logs.since(Some(last)).lines;
        assert_eq!(lines.len(), MAX_LOG_LINES);
        assert!(lines
            .iter()
            .all(|l| l.message == format!("AI: {}", logged.repeat(MAX_LOG_CHARS))));
    }
    // more than that: 413, nothing logged
    let last = a.logs.since(None).last;
    let body = format!(
        r#"{{"lines": [{{"level": "INFO", "message": "{}"}}]}}"#,
        "a".repeat(MAX_LOG_BODY)
    );
    let r = internal(&a, "/internal/ai/log", Some(TOKEN), &body).await;
    assert_eq!(r.status, 413, "{}", r.text());
    assert_eq!(a.logs.since(Some(last)).lines.len(), 0);
    // the other token routes: 64 KiB
    let body = format!(r#"{{"ids": [], "pad": "{}"}}"#, "a".repeat(MAX_BODY));
    let r = internal(&a, "/internal/ai/users", Some(TOKEN), &body).await;
    assert_eq!(r.status, 413, "{}", r.text());
    let r = internal(&a, "/internal/ai/introspect", Some(TOKEN), &body).await;
    assert_eq!(r.status, 413, "{}", r.text());
}

// ---------------------------------------------------------------- the heartbeat: is AI+ available?

/// Captures the log lines written on this thread while the guard lives (what a handler logs).
fn capture() -> (Arc<LogBuffer>, tracing::subscriber::DefaultGuard) {
    use tracing_subscriber::layer::SubscriberExt;
    let lines = Arc::new(LogBuffer::new(1000));
    let guard = tracing::subscriber::set_default(
        tracing_subscriber::registry().with(BufferLayer::new(lines.clone())),
    );
    (lines, guard)
}

/// The messages logged after the line `mark` (`LogBuffer::since(None).last` taken before).
fn messages_after(lines: &LogBuffer, mark: u64) -> Vec<String> {
    lines
        .since(Some(mark))
        .lines
        .into_iter()
        .map(|l| l.message)
        .collect()
}

#[tokio::test]
async fn the_heartbeat_keeps_the_last_report_of_ai_service() {
    use xexam_backend::api::internal::{ServiceState, MAX_SUSPENDED_IDS};
    let (lines, _guard) = capture();
    let a = app();
    let link = &a.state.0.ai_link;
    assert_eq!(link.state(), ServiceState::default(), "none at the start");
    assert!(!link.ai_available());
    // an accepted report: 200 {"ok": true} (JSON, the usual headers)
    let before = auth::unix_now();
    let r = heartbeat(
        &a,
        r#"{"version":"0.8","enabled":true,"suspended":[7,3,7],"stopping":false}"#,
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json(), json!({"ok": true}));
    assert_eq!(
        r.header("content-type").as_deref(),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(r.header("cache-control").as_deref(), Some("no-store"));
    let s = link.state();
    assert!(s.connected && s.enabled && !s.stopped && s.ai_available());
    assert_eq!(s.version.as_deref(), Some("0.8"));
    let seen = s.last_seen.expect("last seen");
    assert!((before..=auth::unix_now()).contains(&seen), "{seen}");
    assert_eq!(
        s.suspended.iter().copied().collect::<Vec<_>>(),
        [3, 7],
        "duplicates ignored"
    );
    assert!(link.suspended(3) && link.suspended(7) && !link.suspended(5));
    // each accepted report replaces the one before; the version is kept trimmed
    report(&a, false, &[], false).await;
    let s = link.state();
    assert!(s.connected && !s.enabled && !s.ai_available());
    assert!(s.suspended.is_empty());
    let r = heartbeat(
        &a,
        &json!({"version": " 0.8.1+build-2 ", "enabled": true, "suspended": [i64::MAX], "stopping": false})
            .to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(link.state().version.as_deref(), Some("0.8.1+build-2"));
    assert!(link.suspended(i64::MAX));
    // the most suspended users one report may carry
    let most: Vec<i64> = (1..=i64::try_from(MAX_SUSPENDED_IDS).expect("n")).collect();
    let r = heartbeat(
        &a,
        &json!({"version": "0.8", "enabled": true, "suspended": most, "stopping": false})
            .to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(link.state().suspended.len(), MAX_SUSPENDED_IDS);
    report(&a, true, &[12, 40], false).await;
    let kept = link.state();
    // anything else is 400 and the last report stays as it was
    let base = json!({"version": "0.8", "enabled": true, "suspended": [], "stopping": false});
    let with = |key: &str, value: Value| {
        let mut v = base.clone();
        v[key] = value;
        v.to_string()
    };
    let without = |key: &str| {
        let mut v = base.clone();
        v.as_object_mut().expect("object").remove(key);
        v.to_string()
    };
    let too_many: Vec<i64> = (1..=i64::try_from(MAX_SUSPENDED_IDS + 1).expect("n")).collect();
    let bad: Vec<String> = vec![
        without("version"),
        without("enabled"),
        without("suspended"),
        without("stopping"),
        with("extra", json!(1)),
        with("version", json!(8)),
        with("version", json!(null)),
        with("version", json!("")),
        with("version", json!("   ")),
        with("version", json!("0.8 beta")),
        with("version", json!("v0.8/x")),
        with("version", json!("1".repeat(21))),
        with("enabled", json!("true")),
        with("enabled", json!(1)),
        with("enabled", json!(null)),
        with("stopping", json!("no")),
        with("stopping", json!(0)),
        with("suspended", json!("12")),
        with("suspended", json!(null)),
        with("suspended", json!({"12": true})),
        with("suspended", json!([0])),
        with("suspended", json!([-1])),
        with("suspended", json!([1.5])),
        with("suspended", json!([1.0])),
        with("suspended", json!(["2"])),
        with("suspended", json!([true])),
        with("suspended", json!([null])),
        with("suspended", json!(too_many)),
        "{}".to_string(),
        "[1]".to_string(),
        "\"ok\"".to_string(),
        "not json".to_string(),
    ];
    for body in &bad {
        let r = heartbeat(&a, body).await;
        assert_eq!(r.status, 400, "{body}: {}", r.text());
        assert_eq!(link.state(), kept, "{body}");
    }
    // the content type and the size limit of the token routes
    let r = send(
        &a.router,
        build(
            "POST",
            "/internal/ai/heartbeat",
            Some(NGINX_HOST),
            &[
                ("x-xexam-internal-token", TOKEN.as_bytes()),
                ("content-type", b"text/plain"),
            ],
            Body::from(beat(false, &[], false)),
        ),
    )
    .await;
    assert_eq!(r.status, 415);
    let big = with("pad", json!("x".repeat(70 * 1024)));
    assert_eq!(heartbeat(&a, &big).await.status, 413);
    assert_eq!(link.state(), kept);
    // without the token (none, a wrong one): 404 like an unknown path, counted with the other refused token requests
    for token in [
        None,
        Some("wrong-fake-token-0000000000000000000000"),
        Some(&TOKEN[1..]),
    ] {
        let r = internal(&a, "/internal/ai/heartbeat", token, &beat(false, &[], true)).await;
        assert_not_found(&r);
    }
    assert_eq!(link.state(), kept);
    match link.flush_refusals(Instant::now() + Duration::from_secs(61)) {
        Some(BurstReport::Many { count, last }) => {
            assert_eq!(count, 2, "three refusals, the first logged at once");
            assert_eq!(last, "POST /internal/ai/heartbeat");
        }
        other => panic!("{other:?}"),
    }
    // heartbeats are never logged, neither into the System log nor by the handler: the only line is the first refusal
    assert!(a.logs.since(None).lines.is_empty());
    assert_eq!(
        messages_after(&lines, 0),
        ["refused an internal request without a valid token: POST /internal/ai/heartbeat"]
    );
}

#[tokio::test]
async fn the_state_route_says_whether_ai_plus_is_available() {
    let a = app();
    let (ac, _) = admin(&a);
    let sam = add_user(&a, "sam", Role::User, true);
    let kim = add_user(&a, "kim", Role::User, false);
    let boss = add_user(&a, "boss", Role::Admin, false);
    for id in [sam, kim, boss] {
        clear_flag(&a, id);
    }
    let (sc, _) = session(&a, sam);
    let (kc, _) = session(&a, kim);
    let (bc, _) = session(&a, boss);
    let state = |cookie: String| {
        let a = &a;
        async move {
            let r = api(a, "GET", "/api/auth/state", &cookie, None).await;
            assert_eq!(r.status, 200, "{}", r.text());
            r.json()
        }
    };
    // what every signed-in user and every administrator get, with the key orders
    let check = |label: &'static str, available: bool, service: Value| {
        let (ac, sc, kc, bc) = (ac.clone(), sc.clone(), kc.clone(), bc.clone());
        let state = &state;
        async move {
            for c in [sc, kc] {
                let v = state(c).await;
                assert_eq!(keys(&v), ["user", "library", "aiAvailable"], "{label}");
                assert_eq!(v["aiAvailable"], available, "{label}");
            }
            for c in [ac, bc] {
                let v = state(c).await;
                assert_eq!(
                    keys(&v),
                    ["user", "library", "aiAvailable", "aiService"],
                    "{label}"
                );
                assert_eq!(v["aiAvailable"], available, "{label}");
                assert_eq!(
                    keys(&v["aiService"]),
                    [
                        "connected",
                        "stopped",
                        "lastSeen",
                        "version",
                        "enabled",
                        "link"
                    ],
                    "{label}"
                );
                assert_eq!(v["aiService"], service, "{label}");
            }
        }
    };
    // no report since the main service started
    check(
        "no report",
        false,
        json!({"connected": false, "stopped": false, "lastSeen": null, "version": null, "enabled": false, "link": true}),
    )
    .await;
    // a fresh report, the AI+ service on
    report(&a, true, &[], false).await;
    let seen = a.state.0.ai_link.state().last_seen.expect("seen");
    check(
        "fresh, on",
        true,
        json!({"connected": true, "stopped": false, "lastSeen": seen, "version": "0.8", "enabled": true, "link": true}),
    )
    .await;
    // 59 s old: still connected
    a.state
        .0
        .ai_link
        .age_report_for_tests(Duration::from_secs(59));
    check(
        "59 s",
        true,
        json!({"connected": true, "stopped": false, "lastSeen": seen - 59, "version": "0.8", "enabled": true, "link": true}),
    )
    .await;
    // 61 s old: not connected any more (the version and the time of the last report stay known)
    a.state
        .0
        .ai_link
        .age_report_for_tests(Duration::from_secs(2));
    check(
        "61 s",
        false,
        json!({"connected": false, "stopped": false, "lastSeen": seen - 61, "version": "0.8", "enabled": false, "link": true}),
    )
    .await;
    // a fresh report, the AI+ service off
    report(&a, false, &[], false).await;
    let seen = a.state.0.ai_link.state().last_seen.expect("seen");
    check(
        "fresh, off",
        false,
        json!({"connected": true, "stopped": false, "lastSeen": seen, "version": "0.8", "enabled": false, "link": true}),
    )
    .await;
    // the last report while ai-service stopped
    report(&a, true, &[sam], true).await;
    let seen = a.state.0.ai_link.state().last_seen.expect("seen");
    check(
        "stopping",
        false,
        json!({"connected": false, "stopped": true, "lastSeen": seen, "version": "0.8", "enabled": false, "link": true}),
    )
    .await;
    // started again
    report(&a, true, &[], false).await;
    let seen = a.state.0.ai_link.state().last_seen.expect("seen");
    check(
        "started again",
        true,
        json!({"connected": true, "stopped": false, "lastSeen": seen, "version": "0.8", "enabled": true, "link": true}),
    )
    .await;
    // signed out: unchanged
    let r = send(
        &a.router,
        build("GET", "/api/auth/state", Some(HOST), &[], Body::empty()),
    )
    .await;
    assert_eq!(r.json(), json!({"user": null, "defaultAdmin": null}));
    // a demoted administrator no longer gets the details
    let r = api(
        &a,
        "POST",
        "/api/users/update",
        &ac,
        Some(&format!(r#"{{"id": {boss}, "role": "user"}}"#)),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let (bc, _) = session(&a, boss);
    let v = state(bc).await;
    assert_eq!(keys(&v), ["user", "library", "aiAvailable"]);
}

#[tokio::test]
async fn every_users_list_says_whether_ai_plus_is_available_and_who_is_suspended() {
    let a = app();
    let (ac, _) = admin(&a);
    let sam = add_user(&a, "sam", Role::User, true);
    let kim = add_user(&a, "kim", Role::User, false);
    let boss = add_user(&a, "boss", Role::Admin, true);
    let list = || {
        let (a, ac) = (&a, ac.clone());
        async move {
            let r = api(a, "GET", "/api/users", &ac, None).await;
            assert_eq!(r.status, 200, "{}", r.text());
            r.json()
        }
    };
    let suspended_of = |v: &Value| -> Vec<(String, bool)> {
        v["users"]
            .as_array()
            .expect("users")
            .iter()
            .map(|u| {
                (
                    u["username"].as_str().expect("name").to_string(),
                    u["aiSuspended"].as_bool().expect("aiSuspended"),
                )
            })
            .collect()
    };
    let none = vec![
        ("admin".to_string(), false),
        ("sam".to_string(), false),
        ("kim".to_string(), false),
        ("boss".to_string(), false),
    ];
    // no report: not available, nobody suspended
    let v = list().await;
    assert_eq!(keys(&v), ["users", "aiAvailable"]);
    assert_eq!(v["aiAvailable"], false);
    assert_eq!(suspended_of(&v), none);
    for u in v["users"].as_array().expect("users") {
        assert_eq!(keys(u).last().map(String::as_str), Some("aiSuspended"));
        assert_eq!(keys(u)[keys(u).len() - 2], "lock");
    }
    // sam suspended (an administrator named in a report is never shown as suspended; 999 does not exist)
    report(&a, true, &[sam, 1, boss, 999], false).await;
    let v = list().await;
    assert_eq!(v["aiAvailable"], true);
    let mut with_sam = none.clone();
    with_sam[1].1 = true;
    assert_eq!(suspended_of(&v), with_sam);
    // kept while the report is stale, but AI+ is not available then
    a.state
        .0
        .ai_link
        .age_report_for_tests(Duration::from_secs(61));
    let v = list().await;
    assert_eq!(v["aiAvailable"], false);
    assert_eq!(suspended_of(&v), with_sam);
    // the AI+ service off: not available, the suspension still shown
    report(&a, false, &[sam], false).await;
    let v = list().await;
    assert_eq!(v["aiAvailable"], false);
    assert_eq!(suspended_of(&v), with_sam);
    // every answer that returns the list carries both (here: AI+ available, kim suspended)
    report(&a, true, &[kim], false).await;
    let mut with_kim = none.clone();
    with_kim[2].1 = true;
    for (path, body) in [
        ("/api/users/unlock", format!(r#"{{"id": {kim}}}"#)),
        (
            "/api/users/update",
            format!(r#"{{"id": {kim}, "username": "kim"}}"#),
        ),
        (
            "/api/users/password",
            format!(r#"{{"id": {kim}, "password": "another-pass"}}"#),
        ),
    ] {
        let r = api(&a, "POST", path, &ac, Some(&body)).await;
        assert_eq!(r.status, 200, "{path}: {}", r.text());
        let v = r.json();
        assert_eq!(keys(&v), ["users", "aiAvailable"], "{path}");
        assert_eq!(v["aiAvailable"], true, "{path}");
        assert_eq!(suspended_of(&v), with_kim, "{path}");
    }
    let r = api(
        &a,
        "POST",
        "/api/users/create",
        &ac,
        Some(r#"{"username": "lee", "password": "initial-pass", "role": "user"}"#),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(keys(&r.json()), ["users", "aiAvailable"]);
    let lee = r.json()["users"][4]["id"].as_i64().expect("lee");
    let r = api(
        &a,
        "POST",
        "/api/users/delete",
        &ac,
        Some(&format!(r#"{{"id": {lee}}}"#)),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(keys(&r.json()), ["users", "aiAvailable"]);
    assert_eq!(suspended_of(&r.json()), with_kim);
    // the login and password-change answers are unchanged
    let r = api(
        &a,
        "POST",
        "/api/auth/password",
        &ac,
        Some(r#"{"current": "admin", "password": "a-new-admin-pass"}"#),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(keys(&r.json()), ["user"]);
}

#[tokio::test]
async fn switching_a_users_ai_plus_service_is_refused_while_ai_plus_is_not_available() {
    let (lines, _guard) = capture();
    let a = app();
    let (ac, _) = admin(&a);
    let sam = add_user(&a, "sam", Role::User, true);
    let kim = add_user(&a, "kim", Role::User, false);
    let ai_of = |id: i64| {
        a.store
            .user_by_id(id)
            .expect("lookup")
            .expect("user")
            .ai_enabled
    };
    let call = |path: &'static str, body: String| {
        let (a, ac) = (&a, ac.clone());
        async move { api(a, "POST", path, &ac, Some(&body)).await }
    };
    let refused = |r: &Resp, what: &str| {
        assert_eq!(r.status, 409, "{what}: {}", r.text());
        assert_eq!(
            r.json(),
            json!({"error": MSG_AI_SERVICE_OFF, "code": "ai.serviceOff"}),
            "{what}"
        );
        assert_eq!(keys(&r.json()), ["error", "code"]);
        assert_eq!(
            r.header("content-type").as_deref(),
            Some("application/json; charset=utf-8")
        );
        assert_eq!(r.header("cache-control").as_deref(), Some("no-store"));
    };
    let user_count = || count(&raw(&a.db), "SELECT COUNT(*) FROM users");
    let attempts = |label: &'static str| {
        let call = &call;
        async move {
            for (what, path, body) in [
                ("on", "/api/users/update", format!(r#"{{"id": {kim}, "ai": true}}"#)),
                ("off", "/api/users/update", format!(r#"{{"id": {sam}, "ai": false}}"#)),
                (
                    "the stored value",
                    "/api/users/update",
                    format!(r#"{{"id": {sam}, "ai": true}}"#),
                ),
                (
                    "with other fields",
                    "/api/users/update",
                    format!(
                        r#"{{"id": {kim}, "username": "kimi", "disabled": true, "canManageBanks": true, "ai": false}}"#
                    ),
                ),
                (
                    "create with AI+",
                    "/api/users/create",
                    r#"{"username": "newbie", "password": "initial-pass", "role": "user", "ai": true}"#
                        .to_string(),
                ),
            ] {
                let r = call(path, body).await;
                refused(&r, &format!("{label}: {what}"));
            }
        }
    };
    let unchanged = |label: &str| {
        assert!(ai_of(sam), "{label}");
        assert!(!ai_of(kim), "{label}");
        let k = a.store.user_by_id(kim).expect("lookup").expect("kim");
        assert_eq!(
            (k.username.as_str(), k.disabled, k.can_manage_banks),
            ("kim", false, false),
            "{label}"
        );
        assert!(
            a.store.user_by_name("newbie").expect("lookup").is_none(),
            "{label}"
        );
        assert_eq!(user_count(), 3, "{label}");
    };
    // no report since the main service started
    let mark = lines.since(None).last;
    attempts("no report").await;
    unchanged("no report");
    assert_eq!(
        messages_after(&lines, mark),
        Vec::<String>::new(),
        "no log line"
    );
    // the body is still checked first: a malformed request stays 400
    for (path, body, error) in [
        (
            "/api/users/update",
            format!(r#"{{"id": {kim}, "ai": "yes"}}"#),
            MSG_BAD_REQUEST,
        ),
        (
            "/api/users/create",
            r#"{"username": "newbie", "password": "initial-pass", "role": "user", "ai": 1}"#
                .to_string(),
            MSG_BAD_REQUEST,
        ),
        (
            "/api/users/create",
            r#"{"username": "newbie", "password": "short", "role": "user", "ai": true}"#
                .to_string(),
            "密码需要 8–128 个字符。",
        ),
        (
            "/api/users/create",
            r#"{"username": "new bie", "password": "initial-pass", "role": "user", "ai": true}"#
                .to_string(),
            "用户名不对：1–40 个字符，只能用字母、数字和 . _ - @。",
        ),
    ] {
        let r = call(path, body.clone()).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (400, error.to_string()),
            "{body}"
        );
    }
    unchanged("malformed");
    // everything without "ai" (or with "ai": false on create) works as before
    let r = call(
        "/api/users/create",
        r#"{"username": "lee", "password": "initial-pass", "role": "user", "ai": false}"#
            .to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let lee = a.store.user_by_name("lee").expect("lookup").expect("lee");
    assert!(!lee.ai_enabled);
    let r = call(
        "/api/users/update",
        format!(r#"{{"id": {}, "username": "leo"}}"#, lee.id),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = call("/api/users/delete", format!(r#"{{"id": {}}}"#, lee.id)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    unchanged("other changes");
    // a fresh report with the AI+ service off, a stale one, a stopping one: still refused
    report(&a, false, &[], false).await;
    attempts("off").await;
    report(&a, true, &[], false).await;
    a.state
        .0
        .ai_link
        .age_report_for_tests(Duration::from_secs(61));
    attempts("stale").await;
    report(&a, true, &[], true).await;
    attempts("stopping").await;
    unchanged("after the refusals");
    // available: stored and logged as before
    report(&a, true, &[], false).await;
    let mark = lines.since(None).last;
    let r = call(
        "/api/users/update",
        format!(r#"{{"id": {kim}, "ai": true}}"#),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["aiAvailable"], true);
    assert!(ai_of(kim));
    let r = call(
        "/api/users/update",
        format!(r#"{{"id": {sam}, "ai": false}}"#),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(!ai_of(sam));
    let r = call(
        "/api/users/create",
        r#"{"username": "newbie", "password": "initial-pass", "role": "user", "ai": true}"#
            .to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(
        a.store
            .user_by_name("newbie")
            .expect("lookup")
            .expect("newbie")
            .ai_enabled
    );
    assert_eq!(
        messages_after(&lines, mark),
        [
            r#"user "admin" turned AI+ on for user "kim""#,
            r#"user "admin" turned AI+ off for user "sam""#,
            r#"user "admin" created user "newbie""#,
            r#"user "admin" turned AI+ on for user "newbie""#,
        ]
    );
    // without the AI link (no internal token) AI+ is never available
    let b = app_cfg(apic::default_config(), false);
    let (bc, _) = admin(&b);
    let r = api(
        &b,
        "POST",
        "/api/users/create",
        &bc,
        Some(r#"{"username": "sam", "password": "initial-pass", "role": "user", "ai": true}"#),
    )
    .await;
    refused(&r, "no link");
    assert!(b.store.user_by_name("sam").expect("lookup").is_none());
}

// ---------------------------------------------------------------- serve and the token file

/// A port nobody listens on right now.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

/// Whether `GET /api/info` answers 200 on `port` (the answer's HTTP version follows the request's).
fn answers(port: u16) -> bool {
    use std::io::{Read, Write};
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    if s.write_all(b"GET /api/info HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut buf = [0u8; 64];
    let n = s.read(&mut buf).unwrap_or(0);
    let head = String::from_utf8_lossy(&buf[..n]).into_owned();
    head.starts_with("HTTP/1.") && head.get(8..13) == Some(" 200 ")
}

/// A running server, stopped when dropped (also when an assertion fails).
struct Running(std::process::Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Runs `xexam-backend serve --data <data>` with `env` (the AI link variables are removed first) until it answers, stops
/// it and returns what it wrote to stderr.
fn serve_once(data: &Path, env: &[(&str, &str)]) -> String {
    use std::io::Read;
    let port = free_port();
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_xexam-backend"));
    cmd.args(["serve", "--port", &port.to_string(), "--data"])
        .arg(data)
        .env_remove("XEXAM_INTERNAL_TOKEN")
        .env_remove("XEXAM_LINK_DIR")
        .env_remove("XEXAM_DATA")
        .env_remove("XEXAM_LOG")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut server = Running(cmd.spawn().expect("start the server"));
    let start = Instant::now();
    while !answers(port) {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "the server did not start"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let stderr = server.0.stderr.take();
    drop(server);
    let mut err = String::new();
    stderr
        .expect("stderr")
        .read_to_string(&mut err)
        .expect("read stderr");
    err
}

#[test]
fn serve_finds_or_creates_the_token_and_the_subcommands_never_do() {
    let dir = dbc::tmp();
    // no variable: <data dir>/link/internal.token, created once and read at the next start
    let data = dir.path().join("data-a");
    let err = serve_once(&data, &[]);
    let file = data.join("link").join("internal.token");
    let token = std::fs::read_to_string(&file).expect("token file");
    assert_eq!(token.len(), 43);
    assert!(err.contains("AI link: "), "{err}");
    assert!(err.contains("internal.token"), "{err}");
    assert!(!err.contains(&token), "the token is never logged");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    serve_once(&data, &[]);
    assert_eq!(std::fs::read_to_string(&file).expect("token file"), token);
    // XEXAM_LINK_DIR moves it
    let data = dir.path().join("data-b");
    let link = dir.path().join("shared-link");
    serve_once(&data, &[("XEXAM_LINK_DIR", link.to_str().expect("utf-8"))]);
    assert!(link.join("internal.token").is_file());
    assert!(!data.join("link").exists());
    // XEXAM_INTERNAL_TOKEN: no file at all
    let data = dir.path().join("data-c");
    let err = serve_once(&data, &[("XEXAM_INTERNAL_TOKEN", TOKEN)]);
    assert!(err.contains("AI link: using XEXAM_INTERNAL_TOKEN"), "{err}");
    assert!(!err.contains(TOKEN), "the token is never logged");
    assert!(!data.join("link").exists());
    // too short: the link is off (no file either)
    let data = dir.path().join("data-d");
    let err = serve_once(&data, &[("XEXAM_INTERNAL_TOKEN", "too-short")]);
    assert!(
        err.contains("XEXAM_INTERNAL_TOKEN is too short (at least 32 characters); AI link off"),
        "{err}"
    );
    assert!(!data.join("link").exists());
    // whitespace around the value, or around the file's content, is ignored (as the AI service does)
    let data = dir.path().join("data-f");
    let padded = format!("  {TOKEN} \n");
    let err = serve_once(&data, &[("XEXAM_INTERNAL_TOKEN", &padded)]);
    assert!(err.contains("AI link: using XEXAM_INTERNAL_TOKEN"), "{err}");
    assert!(!err.contains("AI link off"), "{err}");
    let data = dir.path().join("data-g");
    let file = data.join("link").join("internal.token");
    std::fs::create_dir_all(data.join("link")).expect("mkdir");
    std::fs::write(&file, format!(" {TOKEN}\r\n\n")).expect("write");
    let err = serve_once(&data, &[]);
    assert!(err.contains("AI link: "), "{err}");
    assert!(!err.contains("AI link off"), "{err}");
    assert_eq!(
        std::fs::read_to_string(&file).expect("read"),
        format!(" {TOKEN}\r\n\n"),
        "a hand-written file is left as it is"
    );
    // a subcommand that opens the database does not create the link directory
    let data = dir.path().join("data-e");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xexam-backend"))
        .args(["migrate-progress", "--data"])
        .arg(&data)
        .env_remove("XEXAM_INTERNAL_TOKEN")
        .env_remove("XEXAM_LINK_DIR")
        .output()
        .expect("run");
    assert!(out.status.success(), "{out:?}");
    assert!(data.join("xexam.db").is_file());
    assert!(!data.join("link").exists());
}
