//! v0.6: users and sign-in — schema v3 (migration from v2 and v1, fresh databases, fix-ups), sign-in, the login throttle, the
//! session cookie, the access rules of every route, the forced password change, the users API with its safety rules, per-user
//! study data, sessions (expiry, refresh, purge, logout) and the `reset-password` command.
//!
//! These tests use the real router with the real engine and database and sign in through the API (no helper cookie), so they
//! test exactly what a browser goes through. Only made-up data is used.
#[path = "api_common/mod.rs"]
mod apic;
#[path = "common/mod.rs"]
mod dbc;

use apic::{build, send, Resp};
use axum::body::Body;
use axum::Router;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicI64;
use std::sync::Arc;
use std::time::Duration;
use xexam_backend::api::auth::{Access, ROUTES};
use xexam_backend::api::{build_router, AppState, Inner};
use xexam_backend::auth::{self, LoginThrottle, ThrottleLimits};
use xexam_backend::config::Config;
use xexam_backend::database::schema::{MIGRATE_V2_INDEXES, MIGRATE_V2_TABLES, SCHEMA, V2_COLUMNS};
use xexam_backend::database::{Store, UserChange, UserUpdate};
use xexam_backend::error::DbError;
use xexam_backend::models::{Attempt, AttemptResult, ImportTarget};
use xexam_backend::services::{Engine, PendingStore, RealEngine};

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";

const MSG_SIGN_IN_REQUIRED: &str = "请先登录。";
const MSG_ADMIN_ONLY: &str = "只有管理员可以进行这个操作。";
const MSG_BAD_LOGIN: &str = "用户名或密码不对。";
const MSG_TOO_MANY_LOGINS: &str = "登录失败次数太多，请稍后再试。";
const MSG_PASSWORD_CHANGE_REQUIRED: &str = "请先设置新密码。";
const MSG_SAME_PASSWORD: &str = "新密码不能和原来的一样。";
const MSG_BAD_USERNAME: &str = "用户名不对：1–40 个字符，只能用字母、数字和 . _ - @。";
const MSG_BAD_PASSWORD: &str = "密码需要 8–128 个字符。";
const MSG_USERNAME_TAKEN: &str = "这个用户名已经有人用了。";
const MSG_USER_NOT_FOUND: &str = "这个用户不存在（可能已被删除），请刷新页面后再试。";
const MSG_NOT_YOURSELF: &str = "不能对自己的账户做这个操作。";
const MSG_WRONG_PASSWORD: &str = "当前密码不对。";
const MSG_BAD_REQUEST: &str = "请求的格式不对。";
const MSG_RECORD_NOT_FOUND: &str = "这条考试记录不存在（可能已被删除），请刷新页面后再试。";

const ADMIN_PW: &str = "kitchen-admin-1";

// ---------------------------------------------------------------- helpers

struct App {
    router: Router,
    store: Arc<Store>,
    state: AppState,
    db: PathBuf,
    _dir: tempfile::TempDir,
}

fn app_with(cfg: Config, limits: ThrottleLimits) -> App {
    app_with_engine(cfg, limits, |store| Arc::new(RealEngine::new(store)))
}

/// Like [`app_with`], with the engine made by `make` from the real store.
fn app_with_engine(
    cfg: Config,
    limits: ThrottleLimits,
    make: impl FnOnce(Arc<Store>) -> Arc<dyn Engine>,
) -> App {
    app_with_throttle(cfg, LoginThrottle::new(limits), make)
}

/// Like [`app_with_engine`], with a ready-made throttle (the lock tests give it a clock they can move forward).
fn app_with_throttle(
    cfg: Config,
    throttle: LoginThrottle,
    make: impl FnOnce(Arc<Store>) -> Arc<dyn Engine>,
) -> App {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let store = Arc::new(Store::open(&db).expect("store"));
    let engine = make(store.clone());
    let state = AppState(Arc::new(Inner {
        cfg,
        store: store.clone(),
        pending: PendingStore::new(),
        parse_gate: Arc::new(tokio::sync::Semaphore::new(2)),
        shutdown: None,
        engine,
        hash_gate: Arc::new(tokio::sync::Semaphore::new(auth::HASH_CONCURRENCY)),
        throttle,
        last_session_purge: AtomicI64::new(0),
        default_admin_hashes: Default::default(),
        logs: Default::default(),
        rejections: Default::default(),
        failed_sign_ins: Default::default(),
        name_locks: Default::default(),
        old_layouts: Default::default(),
    }));
    App {
        router: build_router(state.clone()),
        store,
        state,
        db,
        _dir: dir,
    }
}

fn app() -> App {
    app_with(apic::default_config(), ThrottleLimits::default())
}

fn raw(p: &Path) -> Connection {
    Connection::open(p).expect("raw open")
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).expect("count")
}

/// One request. `cookie`: the `Cookie` header; `origin`: the `Origin` header; `body`: a JSON body (POST).
async fn call(
    a: &App,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    origin: Option<&str>,
    body: Option<&str>,
) -> Resp {
    let mut hs: Vec<(&str, Vec<u8>)> = Vec::new();
    if let Some(c) = cookie {
        hs.push(("cookie", c.as_bytes().to_vec()));
    }
    if let Some(o) = origin {
        hs.push(("origin", o.as_bytes().to_vec()));
    }
    let body = match body {
        Some(b) => {
            hs.push(("content-type", b"application/json".to_vec()));
            hs.push(("content-length", b.len().to_string().into_bytes()));
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let hrefs: Vec<(&str, &[u8])> = hs.iter().map(|(k, v)| (*k, v.as_slice())).collect();
    send(&a.router, build(method, path, Some(HOST), &hrefs, body)).await
}

async fn get(a: &App, path: &str, cookie: Option<&str>) -> Resp {
    call(a, "GET", path, cookie, None, None).await
}

async fn post(a: &App, path: &str, cookie: Option<&str>, body: &str) -> Resp {
    call(a, "POST", path, cookie, Some(ORIGIN), Some(body)).await
}

/// `xexam_session=<token>` from a sign-in response.
fn cookie_of(r: &Resp) -> String {
    let sc = r.header("set-cookie").expect("set-cookie");
    sc.split(';').next().expect("pair").to_string()
}

async fn login(a: &App, name: &str, password: &str) -> Resp {
    post(
        a,
        "/api/auth/login",
        None,
        &json!({"username": name, "password": password}).to_string(),
    )
    .await
}

/// Signs in as the default administrator and replaces the default password. Returns the cookie (of the session that the
/// password change started).
async fn admin(a: &App) -> String {
    let r = login(a, "admin", "admin").await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["user"]["mustChangePassword"], true);
    let c = cookie_of(&r);
    let r = post(
        a,
        "/api/auth/password",
        Some(&c),
        &json!({ "password": ADMIN_PW }).to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    // The new password comes with a new session (the old cookie is no longer valid)
    cookie_of(&r)
}

/// Creates `name` (role `role`) as the administrator, signs in as them and sets their own password. Returns (id, cookie).
/// Questions made up with `dbc::seed_questions`, in a made-up bank of their own (v0.7: a plain user sees only the banks they
/// have a row for, so questions without a bank are left to administrators). Call it before the users are created: their
/// `"banks": "all"` then includes the bank.
fn seed_banked(db: &Path, ids: &[&str]) {
    dbc::seed_questions(db, ids);
    raw(db)
        .execute_batch(
            "INSERT INTO banks(name, filename, legacy_ids, created_at, updated_at, owner_id) \
             VALUES ('Made-up bank', 'made-up.xlsx', 0, '2026-01-01 10:00', '2026-01-01 10:00', 1); \
             UPDATE questions SET bank_id = last_insert_rowid() WHERE bank_id IS NULL;",
        )
        .expect("bank of the seeded questions");
}

/// The user sees every question bank (`"banks": "all"`): in v0.6 every user saw every bank, which is what these tests are
/// about; v0.6.5's per-user banks (a new user sees none by default) are tested in tests/v065_bank_access_and_practice.rs.
async fn user(a: &App, admin_cookie: &str, name: &str, role: &str) -> (i64, String) {
    let r = post(
        a,
        "/api/users/create",
        Some(admin_cookie),
        &json!({"username": name, "password": "initial-pass", "role": role, "banks": "all"})
            .to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let id = r.json()["users"]
        .as_array()
        .expect("users")
        .iter()
        .find(|u| u["username"] == name)
        .and_then(|u| u["id"].as_i64())
        .expect("id");
    let r = login(a, name, "initial-pass").await;
    assert_eq!(r.status, 200, "{}", r.text());
    let c = cookie_of(&r);
    let r = post(
        a,
        "/api/auth/password",
        Some(&c),
        &json!({ "password": format!("{name}-own-pass") }).to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    (id, cookie_of(&r))
}

fn attempt(ids: &[(&str, bool)]) -> Attempt {
    Attempt {
        results: ids
            .iter()
            .map(|(id, correct)| AttemptResult {
                id: (*id).to_string(),
                correct: *correct,
            })
            .collect(),
        sheets: vec!["Cooking".to_string()],
        mode: "all".to_string(),
        seconds: 12,
        kind: Default::default(),
    }
}

fn attempt_body(ids: &[(&str, bool)]) -> String {
    attempt_body_with_sheets(ids, &["Cooking"])
}

fn attempt_body_with_sheets(ids: &[(&str, bool)], sheets: &[&str]) -> String {
    let results: Vec<Value> = ids
        .iter()
        .map(|(id, c)| json!({"id": id, "correct": c, "answer": ["A"]}))
        .collect();
    json!({"results": results, "sheets": sheets, "mode": "all", "seconds": 12}).to_string()
}

fn backups(dir: &Path) -> Vec<PathBuf> {
    match std::fs::read_dir(dir.join("backups")) {
        Ok(rd) => rd.map(|e| e.expect("entry").path()).collect(),
        Err(_) => Vec::new(),
    }
}

/// A schema-2 database (as v0.5 wrote it) with made-up data: one bank, two questions, stats, one exam with detail, settings.
fn write_v2_database(db: &Path) {
    let c = raw(db);
    c.execute_batch(SCHEMA).expect("v1");
    c.execute_batch(MIGRATE_V2_TABLES).expect("v2 tables");
    for (table, _, definition) in V2_COLUMNS {
        c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {definition};"))
            .expect("v2 column");
    }
    c.execute_batch(MIGRATE_V2_INDEXES).expect("v2 index");
    c.execute_batch(
        r#"
        INSERT INTO banks(id, name, filename, legacy_ids, created_at, updated_at)
          VALUES(1, 'Kitchen basics', 'Kitchen basics.xlsx', 1, '2026-01-01 10:00', '2026-01-01 10:00');
        INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id) VALUES
          ('q1', 'Cooking', 1, 0, '{"kind": "single", "stem": "Water boils at?"}', 1, '2026-01-01 10:00', '2026-01-01 10:00', 1),
          ('q2', 'Cooking', 2, 1, '{"kind": "single", "stem": "Salt or sugar?"}', 1, '2026-01-01 10:00', '2026-01-01 10:00', 1);
        INSERT INTO stats(question_id, attempts, correct, wrong, streak, last, at) VALUES
          ('q2', 1, 1, 0, 1, 1, '2026-01-02 10:00'),
          ('q1', 3, 1, 2, 0, 0, '2026-01-02 10:00');
        INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, banks)
          VALUES('2026-01-02 10:00', '["Cooking"]', 'all', 2, 1, 50, 30, '[{"id": 1, "name": "Kitchen basics"}]');
        INSERT INTO attempt_items(attempt_id, seq, question_id, answer, correct) VALUES(1, 0, 'q1', '["A"]', 0), (1, 1, 'q2', NULL, 1);
        INSERT INTO settings(key, value) VALUES('theme', '"dark"'), ('uiLang', '"ja"');
        INSERT INTO meta(key, value) VALUES('schema', '2');
        "#,
    )
    .expect("v2 data");
}

fn rows(c: &Connection, sql: &str) -> Vec<Vec<rusqlite::types::Value>> {
    let mut st = c.prepare(sql).expect("prepare");
    let n = st.column_count();
    st.query_map([], |r| (0..n).map(|i| r.get(i)).collect())
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows")
}

// ---------------------------------------------------------------- schema v3

#[test]
fn v2_database_migrates_to_v3_with_a_backup_an_admin_and_the_data_claimed() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    write_v2_database(&db);
    let before = {
        let c = raw(&db);
        [
            rows(&c, "SELECT * FROM stats ORDER BY question_id"),
            rows(&c, "SELECT * FROM settings ORDER BY key"),
            rows(&c, "SELECT * FROM attempts ORDER BY id"),
            rows(&c, "SELECT * FROM attempt_items ORDER BY attempt_id, seq"),
        ]
    };
    let s = Store::open(&db).expect("open");
    assert!(s.created_default_admin());
    // the backup: xexam-schema2-<stamp>.db, still schema 2, same rows, no user tables
    let backup = s.migration_backup().expect("backup").to_path_buf();
    let name = backup.file_name().and_then(|n| n.to_str()).expect("name");
    assert!(
        name.starts_with("xexam-schema2-")
            && name.len() == "xexam-schema2-20260101-000000.db".len(),
        "{name}"
    );
    assert_eq!(backups(dir.path()).len(), 1);
    {
        let b = raw(&backup);
        assert_eq!(
            b.query_row("SELECT value FROM meta WHERE key='schema'", [], |r| r
                .get::<_, String>(0))
                .expect("schema"),
            "2"
        );
        assert_eq!(
            count(&b, "SELECT COUNT(*) FROM sqlite_master WHERE name='users'"),
            0
        );
        assert_eq!(
            [
                rows(&b, "SELECT * FROM stats ORDER BY question_id"),
                rows(&b, "SELECT * FROM settings ORDER BY key"),
                rows(&b, "SELECT * FROM attempts ORDER BY id"),
                rows(&b, "SELECT * FROM attempt_items ORDER BY attempt_id, seq"),
            ],
            before
        );
    }
    // the default administrator, who must change the password
    let u = s.user_by_name("ADMIN").expect("lookup").expect("admin");
    assert_eq!(
        (
            u.id,
            u.username.as_str(),
            u.role,
            u.disabled,
            u.must_change_password
        ),
        (1, "admin", auth::Role::Admin, false, true)
    );
    assert!(u.password_hash.starts_with("$argon2id$"));
    assert!(auth::verify_password("admin", &u.password_hash));
    // the claim: attempts, a copy of stats and settings (the old tables stay)
    let c = raw(&db);
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM attempts WHERE user_id IS NOT 1"),
        0
    );
    assert_eq!(
        rows(
            &c,
            "SELECT question_id, attempts, correct, wrong, streak, last, at FROM user_stats WHERE user_id=1 ORDER BY question_id"
        ),
        before[0]
    );
    assert_eq!(
        rows(
            &c,
            "SELECT key, value FROM user_settings WHERE user_id=1 ORDER BY key"
        ),
        before[1]
    );
    assert_eq!(
        rows(&c, "SELECT * FROM stats ORDER BY question_id"),
        before[0]
    );
    assert_eq!(
        c.query_row("SELECT value FROM meta WHERE key='schema'", [], |r| r
            .get::<_, String>(0))
            .expect("schema"),
        // v0.6.5: v2 → v3 → v4 in one go (v4: tests/v065_bank_access_and_practice.rs)
        "4"
    );
    // what the API shows for the administrator: the old data, in the old order
    let p = s.progress(1).expect("progress");
    assert_eq!(
        p["questions"]
            .as_object()
            .expect("q")
            .keys()
            .collect::<Vec<_>>(),
        ["q2", "q1"],
        "statistics keep their order"
    );
    assert_eq!(p["questions"]["q1"]["wrong"], 2);
    assert_eq!(p["history"].as_array().map(Vec::len), Some(1));
    assert_eq!(s.settings(1).expect("settings")["theme"], "dark");
    assert_eq!(s.records(1).expect("records")["records"][0]["answered"], 1);
    assert_eq!(s.progress(2).expect("p")["history"], json!([]));
    drop(c);
    drop(s);
    // reopening: nothing written, no second admin or backup
    let bytes = dbc::db_bytes(&db);
    let s = Store::open(&db).expect("reopen");
    assert!(!s.created_default_admin());
    assert!(s.migration_backup().is_none());
    s.progress(1).expect("progress");
    drop(s);
    assert_eq!(dbc::db_bytes(&db), bytes);
    assert_eq!(backups(dir.path()).len(), 1);
}

#[test]
fn v1_database_goes_to_v3_in_one_step_with_one_schema1_backup() {
    let dir = dbc::tmp();
    let db = dir.path().join("sample.db");
    std::fs::copy(dbc::baseline("fixtures/sample.db"), &db).expect("copy the fixture");
    let (stats, attempts) = {
        let c = raw(&db);
        (
            count(&c, "SELECT COUNT(*) FROM stats"),
            count(&c, "SELECT COUNT(*) FROM attempts"),
        )
    };
    assert!(stats > 0 && attempts > 0);
    let s = Store::open(&db).expect("open");
    assert!(s.created_default_admin());
    let found = backups(dir.path());
    assert_eq!(found.len(), 1);
    let name = found[0].file_name().and_then(|n| n.to_str()).expect("name");
    assert!(name.starts_with("xexam-schema1-"), "{name}");
    let c = raw(&db);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM users"), 1);
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM user_stats WHERE user_id=1"),
        stats
    );
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM attempts WHERE user_id=1"),
        attempts
    );
    assert_eq!(count(&c, "SELECT COUNT(*) FROM banks"), 1);
    assert_eq!(
        s.progress(1).expect("p")["history"]
            .as_array()
            .map(Vec::len),
        Some(usize::try_from(attempts).expect("n"))
    );
}

#[test]
fn a_fresh_database_starts_at_v3_with_the_default_admin_and_no_backup() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = Store::open(&db).expect("open");
    assert!(s.created_default_admin());
    assert!(s.migration_backup().is_none());
    assert!(backups(dir.path()).is_empty());
    let u = s.user_by_name("admin").expect("lookup").expect("admin");
    assert!(u.must_change_password);
    drop(s);
    let bytes = dbc::db_bytes(&db);
    let s = Store::open(&db).expect("reopen");
    assert!(!s.created_default_admin());
    drop(s);
    assert_eq!(dbc::db_bytes(&db), bytes, "reopen writes nothing");
}

#[test]
fn reopening_without_users_or_with_unowned_records_fixes_them() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    drop(Store::open(&db).expect("open"));
    // as if every user was removed by hand and an older binary then saved an exam (no user_id)
    raw(&db)
        .execute_batch(
            "DELETE FROM users; \
             INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds) VALUES('t', '[]', 'all', 1, 1, 100, 1);",
        )
        .expect("edit");
    let s = Store::open(&db).expect("reopen");
    assert!(s.created_default_admin());
    let id = s.user_by_name("admin").expect("lookup").expect("admin").id;
    assert_eq!(
        count(
            &raw(&db),
            &format!("SELECT COUNT(*) FROM attempts WHERE user_id={id}")
        ),
        1
    );
}

// ---------------------------------------------------------------- sign-in

#[tokio::test]
async fn login_ok_wrong_password_unknown_and_disabled_users() {
    let a = app();
    // the default administrator signs in and must change the password
    let r = login(&a, " Admin ", "admin").await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(
        r.json(),
        json!({"user": {"id": 1, "username": "admin", "role": "admin", "mustChangePassword": true, "canManageBanks": true}})
    );
    assert!(a
        .store
        .user_by_id(1)
        .expect("u")
        .expect("admin")
        .last_login_at
        .is_some());
    let first = cookie_of(&r);
    // every sign-in gets a new token, even when the old cookie is sent along
    let r2 = call(
        &a,
        "POST",
        "/api/auth/login",
        Some(&first),
        Some(ORIGIN),
        Some(r#"{"username": "admin", "password": "admin"}"#),
    )
    .await;
    assert_eq!(r2.status, 200);
    assert_ne!(cookie_of(&r2), first);
    let c = admin(&a).await;
    let (id, _) = user(&a, &c, "cook", "user").await;
    // wrong password, unknown user, malformed name, disabled user: the same 401, no cookie
    for (name, pw) in [
        ("admin", "admin"),
        ("admin", "wrong-password"),
        ("nobody", "whatever-123"),
        ("not a name!", "whatever-123"),
    ] {
        let r = login(&a, name, pw).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (401, MSG_BAD_LOGIN.to_string()),
            "{name}"
        );
        assert!(r.header("set-cookie").is_none());
    }
    let r = post(
        &a,
        "/api/users/update",
        Some(&c),
        &json!({"id": id, "disabled": true}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = login(&a, "cook", "cook-own-pass").await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (401, MSG_BAD_LOGIN.to_string())
    );
    // malformed bodies
    for body in [
        r#"{}"#,
        r#"{"username": "admin"}"#,
        r#"{"username": 1, "password": "x"}"#,
    ] {
        let r = post(&a, "/api/auth/login", None, body).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (400, MSG_BAD_REQUEST.to_string()),
            "{body}"
        );
    }
    // the state route
    let r = get(&a, "/api/auth/state", None).await;
    assert_eq!(r.json(), json!({"user": null, "defaultAdmin": null}));
    let r = get(&a, "/api/auth/state", Some(&c)).await;
    assert_eq!(r.json()["user"]["mustChangePassword"], false);
}

#[tokio::test]
async fn throttle_answers_429_and_recovers() {
    let mut limits = ThrottleLimits::default();
    limits.locks[0] = Duration::from_millis(400);
    let a = app_with(apic::default_config(), limits);
    let c = admin(&a).await;
    drop(c);
    for _ in 0..5 {
        assert_eq!(login(&a, "admin", "not-the-password").await.status, 401);
    }
    // locked: even the right password gets 429, and so does any spelling of the name
    for name in ["admin", "ADMIN"] {
        let r = login(&a, name, ADMIN_PW).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (429, MSG_TOO_MANY_LOGINS.to_string())
        );
    }
    // another name is not affected
    assert_eq!(login(&a, "somebody", "not-the-password").await.status, 401);
    tokio::time::sleep(Duration::from_millis(450)).await;
    let r = login(&a, "admin", ADMIN_PW).await;
    assert_eq!(r.status, 200, "{}", r.text());
    // a success clears the name's failures
    assert_eq!(login(&a, "admin", "not-the-password").await.status, 401);
    assert_eq!(login(&a, "admin", ADMIN_PW).await.status, 200);
}

#[tokio::test]
async fn cookie_attributes_over_http_and_https() {
    let want = |secure: bool| {
        move |sc: &str| {
            let parts: Vec<&str> = sc.split("; ").collect();
            assert!(
                parts[0].starts_with("xexam_session=") && parts[0].len() == 14 + 43,
                "{sc}"
            );
            assert_eq!(
                &parts[1..],
                if secure {
                    &[
                        "HttpOnly",
                        "SameSite=Strict",
                        "Path=/",
                        "Max-Age=2592000",
                        "Secure",
                    ][..]
                } else {
                    &["HttpOnly", "SameSite=Strict", "Path=/", "Max-Age=2592000"][..]
                },
                "{sc}"
            );
        }
    };
    let a = app();
    let body = r#"{"username": "admin", "password": "admin"}"#;
    for (origin, secure) in [
        (None, false),
        (Some("http://127.0.0.1:8765"), false),
        (Some("http://192.168.1.20:8765"), false),
        (Some("https://127.0.0.1"), true),
    ] {
        let mut cfg = apic::default_config();
        cfg.allowed_origins.insert("192.168.1.20".into());
        let a2 = if origin == Some("http://192.168.1.20:8765") {
            app_with(cfg, ThrottleLimits::default())
        } else {
            app()
        };
        let r = call(&a2, "POST", "/api/auth/login", None, origin, Some(body)).await;
        assert_eq!(r.status, 200, "{origin:?}: {}", r.text());
        want(secure)(&r.header("set-cookie").expect("cookie"));
    }
    // XEXAM_COOKIE_SECURE=1 forces Secure
    let forced = app_with(
        apic::config_with(&[("XEXAM_COOKIE_SECURE", "1")]),
        ThrottleLimits::default(),
    );
    let r = call(
        &forced,
        "POST",
        "/api/auth/login",
        None,
        Some(ORIGIN),
        Some(body),
    )
    .await;
    want(true)(&r.header("set-cookie").expect("cookie"));
    // logout: an expired cookie
    let r = login(&a, "admin", "admin").await;
    let c = cookie_of(&r);
    let r = post(&a, "/api/auth/logout", Some(&c), "{}").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json(), json!({"ok": true}));
    assert_eq!(
        r.header("set-cookie").as_deref(),
        Some("xexam_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0")
    );
}

// ---------------------------------------------------------------- access rules

#[tokio::test]
async fn every_non_public_route_needs_a_session() {
    let a = app();
    let junk = format!("xexam_session={}", auth::new_session_token());
    let mut checked = 0;
    for r in ROUTES {
        for cookie in [None, Some(junk.as_str()), Some("xexam_session=short")] {
            let resp = if r.method == "GET" {
                for method in ["GET", "HEAD"] {
                    let resp = call(&a, method, r.path, cookie, None, None).await;
                    if r.access == Access::Public {
                        assert_ne!(resp.status, 401, "{method} {}", r.path);
                    } else {
                        assert_eq!(resp.status, 401, "{method} {}", r.path);
                    }
                }
                call(&a, "GET", r.path, cookie, None, None).await
            } else {
                // the session is checked before the Content-Type and the body: a wrong type still gets 401
                let mut hs: Vec<(&str, &[u8])> = vec![
                    ("origin", ORIGIN.as_bytes()),
                    ("content-type", b"text/plain"),
                ];
                if let Some(c) = cookie {
                    hs.push(("cookie", c.as_bytes()));
                }
                send(
                    &a.router,
                    build("POST", r.path, Some(HOST), &hs, Body::from("x")),
                )
                .await
            };
            if r.access == Access::Public {
                assert_ne!(resp.status, 401, "{} {}", r.method, r.path);
            } else {
                assert_eq!(
                    (resp.status.as_u16(), resp.error()),
                    (401, MSG_SIGN_IN_REQUIRED.to_string()),
                    "{} {}",
                    r.method,
                    r.path
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, ROUTES.len() * 3);
    assert_eq!(
        ROUTES
            .iter()
            .filter(|r| r.access == Access::Public)
            .map(|r| (r.method, r.path))
            .collect::<Vec<_>>(),
        [
            ("GET", "/api/info"),
            ("GET", "/api/auth/state"),
            ("POST", "/api/auth/login")
        ]
    );
    // unknown paths need a session too (then they are 404 as before)
    for (method, path) in [
        ("GET", "/api/nothing"),
        ("POST", "/api/nothing"),
        ("GET", "/index.html"),
    ] {
        let r = call(
            &a,
            method,
            path,
            None,
            Some(ORIGIN),
            (method == "POST").then_some("{}"),
        )
        .await;
        assert_eq!(r.status, 401, "{method} {path}");
    }
    let c = admin(&a).await;
    assert_eq!(get(&a, "/api/nothing", Some(&c)).await.status, 404);
    assert_eq!(post(&a, "/api/nothing", Some(&c), "{}").await.status, 404);
    // the Host and Origin guards still come first
    let r = send(
        &a.router,
        build("GET", "/api/bank", Some("evil.example"), &[], Body::empty()),
    )
    .await;
    assert_eq!(r.status, 403);
    let r = call(
        &a,
        "POST",
        "/api/attempt",
        None,
        Some("https://evil.example"),
        Some("{}"),
    )
    .await;
    assert_eq!(r.status, 403);
    assert!(r.error().contains("evil.example"));
    // /api/info is public and unchanged
    let r = get(&a, "/api/info", None).await;
    assert_eq!(r.status, 200);
    assert_eq!(
        r.json().as_object().expect("o").keys().collect::<Vec<_>>(),
        ["version", "arch", "canShutdown", "bankCount"]
    );
}

#[tokio::test]
async fn a_plain_user_gets_403_on_every_admin_route() {
    let a = app();
    seed_banked(&a.db, &["q1", "q2"]);
    let c = admin(&a).await;
    let (_, uc) = user(&a, &c, "cook", "user").await;
    let admin_routes: Vec<_> = ROUTES
        .iter()
        .filter(|r| r.access == Access::Admin)
        .collect();
    // v0.6.5: the five bank management routes moved to `Access::BankManager` (403 `MSG_NO_BANK_PERMISSION` for this user,
    // see tests/v065_bank_access_and_practice.rs); v0.7 adds `GET /api/logs`
    assert_eq!(admin_routes.len(), 8);
    for r in admin_routes {
        let resp = if r.method == "GET" {
            get(&a, r.path, Some(&uc)).await
        } else {
            let mut hs: Vec<(&str, &[u8])> = vec![
                ("origin", ORIGIN.as_bytes()),
                ("content-type", b"application/octet-stream"),
                ("cookie", uc.as_bytes()),
            ];
            hs.push(("content-length", b"2"));
            send(
                &a.router,
                build("POST", r.path, Some(HOST), &hs, Body::from("{}")),
            )
            .await
        };
        assert_eq!(
            (resp.status.as_u16(), resp.error()),
            (403, MSG_ADMIN_ONLY.to_string()),
            "{} {}",
            r.method,
            r.path
        );
    }
    // the shared routes work for them
    for path in [
        "/api/bank",
        "/api/banks",
        "/api/progress",
        "/api/records",
        "/api/settings",
    ] {
        assert_eq!(get(&a, path, Some(&uc)).await.status, 200, "{path}");
    }
    assert_eq!(
        post(
            &a,
            "/api/attempt",
            Some(&uc),
            &attempt_body(&[("q1", true)])
        )
        .await
        .status,
        200
    );
}

#[tokio::test]
async fn forced_password_change() {
    let a = app();
    let c = admin(&a).await;
    let r = post(
        &a,
        "/api/users/create",
        Some(&c),
        r#"{"username": "baker", "password": "initial-pass", "role": "user"}"#,
    )
    .await;
    assert_eq!(r.status, 200);
    // two browsers
    let b1 = cookie_of(&login(&a, "baker", "initial-pass").await);
    let b2 = cookie_of(&login(&a, "baker", "initial-pass").await);
    // only state, password, logout and info are reachable
    for r in ROUTES
        .iter()
        .filter(|r| r.access != Access::Public && !r.during_password_change)
    {
        let resp = if r.method == "GET" {
            get(&a, r.path, Some(&b1)).await
        } else {
            post(&a, r.path, Some(&b1), "{}").await
        };
        assert_eq!(
            (resp.status.as_u16(), resp.error()),
            (403, MSG_PASSWORD_CHANGE_REQUIRED.to_string()),
            "{} {}",
            r.method,
            r.path
        );
    }
    assert_eq!(get(&a, "/api/info", Some(&b1)).await.status, 200);
    assert_eq!(
        get(&a, "/api/auth/state", Some(&b1)).await.json()["user"]["mustChangePassword"],
        true
    );
    // same password, too short
    let r = post(
        &a,
        "/api/auth/password",
        Some(&b1),
        r#"{"password": "initial-pass"}"#,
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (400, MSG_SAME_PASSWORD.to_string())
    );
    let r = post(
        &a,
        "/api/auth/password",
        Some(&b1),
        r#"{"password": "short"}"#,
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (400, MSG_BAD_PASSWORD.to_string())
    );
    // a new one: the flag is cleared, the other browser is signed out
    let r = post(
        &a,
        "/api/auth/password",
        Some(&b1),
        r#"{"password": "baker-new-pass"}"#,
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["user"]["mustChangePassword"], false);
    // the session is rotated: this browser gets a new cookie, its old one ends like every other session of the user
    let old_b1 = b1;
    let b1 = cookie_of(&r);
    assert_ne!(b1, old_b1);
    assert_eq!(get(&a, "/api/bank", Some(&b1)).await.status, 200);
    assert_eq!(get(&a, "/api/bank", Some(&old_b1)).await.status, 401);
    assert_eq!(get(&a, "/api/bank", Some(&b2)).await.status, 401);
    assert_eq!(login(&a, "baker", "initial-pass").await.status, 401);
    // in normal mode the current password is required and checked
    let r = post(
        &a,
        "/api/auth/password",
        Some(&b1),
        r#"{"password": "baker-third-pass"}"#,
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (400, MSG_BAD_REQUEST.to_string())
    );
    let r = post(
        &a,
        "/api/auth/password",
        Some(&b1),
        r#"{"current": "wrong-one!", "password": "baker-third-pass"}"#,
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (403, MSG_WRONG_PASSWORD.to_string())
    );
    let r = post(
        &a,
        "/api/auth/password",
        Some(&b1),
        r#"{"current": "baker-new-pass", "password": "baker-third-pass"}"#,
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(login(&a, "baker", "baker-third-pass").await.status, 200);
}

// ---------------------------------------------------------------- users API

#[tokio::test]
async fn users_crud_and_validation() {
    let a = app();
    seed_banked(&a.db, &["q1"]);
    let c = admin(&a).await;
    let r = get(&a, "/api/users", Some(&c)).await;
    let list = r.json();
    assert_eq!(list["users"].as_array().map(Vec::len), Some(1));
    let u = &list["users"][0];
    assert_eq!(
        u.as_object().expect("o").keys().collect::<Vec<_>>(),
        [
            "id",
            "username",
            "role",
            "disabled",
            "createdAt",
            "lastLoginAt",
            "mustChangePassword",
            "records",
            // v0.6.5 (tests/v065_bank_access_and_practice.rs); "newBanks" v0.7
            "banks",
            "newBanks",
            "canManageBanks",
            "ownedBanks",
            "lock"
        ]
    );
    assert_eq!(u["lock"], Value::Null);
    assert_eq!(
        (
            u["id"].clone(),
            u["username"].clone(),
            u["role"].clone(),
            u["disabled"].clone(),
            u["mustChangePassword"].clone(),
            u["records"].clone()
        ),
        (
            json!(1),
            json!("admin"),
            json!("admin"),
            json!(false),
            json!(false),
            json!(0)
        )
    );
    assert!(u["lastLoginAt"].is_string());
    // create: validation
    for (body, status, msg) in [
        (
            json!({"username": "", "password": "initial-pass", "role": "user"}),
            400,
            MSG_BAD_USERNAME,
        ),
        (
            json!({"username": "two words", "password": "initial-pass", "role": "user"}),
            400,
            MSG_BAD_USERNAME,
        ),
        (
            json!({"username": "a".repeat(41), "password": "initial-pass", "role": "user"}),
            400,
            MSG_BAD_USERNAME,
        ),
        (
            json!({"username": 7, "password": "initial-pass", "role": "user"}),
            400,
            MSG_BAD_USERNAME,
        ),
        (
            json!({"password": "initial-pass", "role": "user"}),
            400,
            MSG_BAD_USERNAME,
        ),
        (
            json!({"username": "cook", "password": "1234567", "role": "user"}),
            400,
            MSG_BAD_PASSWORD,
        ),
        (
            json!({"username": "cook", "password": "x".repeat(129), "role": "user"}),
            400,
            MSG_BAD_PASSWORD,
        ),
        (
            json!({"username": "cook", "role": "user"}),
            400,
            MSG_BAD_REQUEST,
        ),
        (
            json!({"username": "cook", "password": "initial-pass", "role": "owner"}),
            400,
            MSG_BAD_REQUEST,
        ),
        (
            json!({"username": "cook", "password": "initial-pass"}),
            400,
            MSG_BAD_REQUEST,
        ),
        (
            json!({"username": "ADMIN", "password": "initial-pass", "role": "user"}),
            409,
            MSG_USERNAME_TAKEN,
        ),
    ] {
        let r = post(&a, "/api/users/create", Some(&c), &body.to_string()).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (status, msg.to_string()),
            "{body}"
        );
    }
    let r = post(
        &a,
        "/api/users/create",
        Some(&c),
        // v0.6.5: "banks": "all" keeps the v0.6 situation this test is about (every user sees every bank)
        r#"{"username": "  cook.1@home ", "password": "initial-pass", "role": "user", "banks": "all"}"#,
    )
    .await;
    assert_eq!(r.status, 200);
    let users = r.json()["users"].clone();
    assert_eq!(users[1]["username"], "cook.1@home", "trimmed");
    assert_eq!(users[1]["mustChangePassword"], true);
    assert_eq!(users[1]["lastLoginAt"], Value::Null);
    let id = users[1]["id"].as_i64().expect("id");
    // update: rename (case-insensitively unique), role, disabled; validation
    for (body, status, msg) in [
        (
            json!({"id": id, "username": "Admin"}),
            409,
            MSG_USERNAME_TAKEN,
        ),
        (
            json!({"id": id, "username": "bad name"}),
            400,
            MSG_BAD_USERNAME,
        ),
        (json!({"id": id, "role": "boss"}), 400, MSG_BAD_REQUEST),
        (json!({"id": id, "disabled": "yes"}), 400, MSG_BAD_REQUEST),
        (json!({"id": "2"}), 400, MSG_BAD_REQUEST),
        (json!({"id": true}), 400, MSG_BAD_REQUEST),
        (json!({}), 400, MSG_BAD_REQUEST),
        (json!({"id": 999, "role": "user"}), 404, MSG_USER_NOT_FOUND),
    ] {
        let r = post(&a, "/api/users/update", Some(&c), &body.to_string()).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (status, msg.to_string()),
            "{body}"
        );
    }
    let uc = cookie_of(&login(&a, "cook.1@home", "initial-pass").await);
    // renaming keeps the session; a new role signs them out
    let r = post(
        &a,
        "/api/users/update",
        Some(&c),
        &json!({"id": id, "username": "Cook"}).to_string(),
    )
    .await;
    assert_eq!(r.json()["users"][1]["username"], "Cook");
    assert_eq!(
        get(&a, "/api/auth/state", Some(&uc)).await.json()["user"]["username"],
        "Cook"
    );
    let r = post(
        &a,
        "/api/users/update",
        Some(&c),
        &json!({"id": id, "role": "admin"}).to_string(),
    )
    .await;
    assert_eq!(r.json()["users"][1]["role"], "admin");
    assert_eq!(
        get(&a, "/api/auth/state", Some(&uc)).await.json()["user"],
        Value::Null
    );
    // disabling signs them out too
    let uc = cookie_of(&login(&a, "cook", "initial-pass").await);
    let r = post(
        &a,
        "/api/users/update",
        Some(&c),
        &json!({"id": id, "disabled": true}).to_string(),
    )
    .await;
    assert_eq!(r.json()["users"][1]["disabled"], true);
    assert_eq!(
        get(&a, "/api/auth/state", Some(&uc)).await.json()["user"],
        Value::Null
    );
    let r = post(
        &a,
        "/api/users/update",
        Some(&c),
        &json!({"id": id, "disabled": false, "role": "user"}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200);
    // reset password: new initial password, must change, sessions gone
    let uc = cookie_of(&login(&a, "cook", "initial-pass").await);
    let r = post(
        &a,
        "/api/users/password",
        Some(&c),
        &json!({"id": id, "password": "short"}).to_string(),
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (400, MSG_BAD_PASSWORD.to_string())
    );
    let r = post(
        &a,
        "/api/users/password",
        Some(&c),
        &json!({"id": 999, "password": "another-pass"}).to_string(),
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (404, MSG_USER_NOT_FOUND.to_string())
    );
    let r = post(
        &a,
        "/api/users/password",
        Some(&c),
        &json!({"id": id, "password": "another-pass"}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["users"][1]["mustChangePassword"], true);
    assert_eq!(
        get(&a, "/api/auth/state", Some(&uc)).await.json()["user"],
        Value::Null
    );
    assert_eq!(login(&a, "cook", "initial-pass").await.status, 401);
    let uc = cookie_of(&login(&a, "cook", "another-pass").await);
    // the new password comes with a new session cookie
    let uc = cookie_of(
        &post(
            &a,
            "/api/auth/password",
            Some(&uc),
            r#"{"password": "cook-own-pass"}"#,
        )
        .await,
    );
    assert_eq!(
        post(
            &a,
            "/api/attempt",
            Some(&uc),
            &attempt_body(&[("q1", true)])
        )
        .await
        .status,
        200
    );
    assert_eq!(
        get(&a, "/api/users", Some(&c)).await.json()["users"][1]["records"],
        1
    );
    // delete: everything of theirs goes
    let r = post(
        &a,
        "/api/users/delete",
        Some(&c),
        &json!({"id": 999}).to_string(),
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (404, MSG_USER_NOT_FOUND.to_string())
    );
    post(&a, "/api/settings", Some(&uc), r#"{"theme": "dark"}"#).await;
    let r = post(
        &a,
        "/api/users/delete",
        Some(&c),
        &json!({"id": id}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["users"].as_array().map(Vec::len), Some(1));
    let db = raw(&a.db);
    for table in ["sessions", "attempts", "user_stats", "user_settings"] {
        assert_eq!(
            count(
                &db,
                &format!("SELECT COUNT(*) FROM {table} WHERE user_id={id}")
            ),
            0,
            "{table}"
        );
    }
    assert_eq!(count(&db, "SELECT COUNT(*) FROM attempt_items"), 0);
    assert_eq!(get(&a, "/api/bank", Some(&uc)).await.status, 401);
}

#[tokio::test]
async fn nobody_can_lock_themselves_out_and_the_last_admin_stays() {
    let a = app();
    let c = admin(&a).await;
    for (path, body) in [
        ("/api/users/update", json!({"id": 1, "disabled": true})),
        ("/api/users/update", json!({"id": 1, "role": "user"})),
        ("/api/users/delete", json!({"id": 1})),
        (
            "/api/users/password",
            json!({"id": 1, "password": "brand-new-pass"}),
        ),
    ] {
        let r = post(&a, path, Some(&c), &body.to_string()).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (409, MSG_NOT_YOURSELF.to_string()),
            "{path} {body}"
        );
    }
    // renaming yourself is fine, and so is "changing" to what you already are
    let r = post(
        &a,
        "/api/users/update",
        Some(&c),
        r#"{"id": 1, "username": "chef", "role": "admin", "disabled": false}"#,
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    // a second administrator can be disabled, demoted and deleted while the first stays
    let (b, bc) = user(&a, &c, "sous", "admin").await;
    assert_eq!(get(&a, "/api/users", Some(&bc)).await.status, 200);
    for body in [
        json!({"id": b, "disabled": true}),
        json!({"id": b, "disabled": false}),
        json!({"id": b, "role": "user"}),
    ] {
        assert_eq!(
            post(&a, "/api/users/update", Some(&c), &body.to_string())
                .await
                .status,
            200
        );
    }
    assert_eq!(
        post(
            &a,
            "/api/users/delete",
            Some(&c),
            &json!({"id": b}).to_string()
        )
        .await
        .status,
        200
    );
    // the last enabled administrator, as seen by the store (through the API the actor is always that administrator, so the
    // rule above answers first; the store check also covers concurrent requests)
    let s = &a.store;
    for change in [
        UserUpdate {
            disabled: Some(true),
            ..UserUpdate::default()
        },
        UserUpdate {
            role: Some(auth::Role::User),
            ..UserUpdate::default()
        },
    ] {
        assert_eq!(
            s.update_user(999, 1, &change).expect("update"),
            UserChange::LastAdmin
        );
    }
    assert_eq!(
        s.delete_user(999, 1).expect("delete"),
        UserChange::LastAdmin
    );
    assert_eq!(
        s.update_user(
            999,
            1,
            &UserUpdate {
                username: Some("chef2".into()),
                ..UserUpdate::default()
            }
        )
        .expect("rename"),
        UserChange::Done(xexam_backend::database::UpdatedUser {
            username: "chef2".into(),
            renamed_from: Some("chef".into()),
        })
    );
    // with a disabled second admin the first is still the last enabled one
    let id = match s
        .create_user(
            "spare",
            &auth::hash_password("spare-pass-1").expect("h"),
            auth::Role::Admin,
        )
        .expect("c")
    {
        UserChange::Done(id) => id,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        s.update_user(
            1,
            id,
            &UserUpdate {
                disabled: Some(true),
                ..UserUpdate::default()
            }
        )
        .expect("u"),
        UserChange::Done(xexam_backend::database::UpdatedUser {
            username: "spare".into(),
            renamed_from: None,
        })
    );
    assert_eq!(
        s.delete_user(999, 1).expect("delete"),
        UserChange::LastAdmin
    );
    assert_eq!(
        s.delete_user(1, id).expect("delete"),
        UserChange::Done("spare".to_string()),
        "a disabled admin is not the last one"
    );
    let r = post(
        &a,
        "/api/users/update",
        Some(&c),
        r#"{"id": 1, "disabled": true}"#,
    )
    .await;
    assert_eq!(r.error(), MSG_NOT_YOURSELF);
}

// ---------------------------------------------------------------- per-user data

#[tokio::test]
async fn study_data_is_per_user() {
    let a = app();
    seed_banked(&a.db, &["q1", "q2", "q3"]);
    let c = admin(&a).await;
    let (uid, uc) = user(&a, &c, "cook", "user").await;
    // each submits an exam
    let r = post(
        &a,
        "/api/attempt",
        Some(&c),
        &attempt_body(&[("q1", false), ("q2", true)]),
    )
    .await;
    assert_eq!(r.status, 200);
    let r = post(
        &a,
        "/api/attempt",
        Some(&uc),
        &attempt_body(&[("q3", false)]),
    )
    .await;
    assert_eq!(r.status, 200);
    let p_admin = get(&a, "/api/progress", Some(&c)).await.json();
    let p_user = get(&a, "/api/progress", Some(&uc)).await.json();
    assert_eq!(
        p_admin["questions"]
            .as_object()
            .expect("q")
            .keys()
            .collect::<Vec<_>>(),
        ["q1", "q2"]
    );
    assert_eq!(
        p_user["questions"]
            .as_object()
            .expect("q")
            .keys()
            .collect::<Vec<_>>(),
        ["q3"]
    );
    let ra = get(&a, "/api/records", Some(&c)).await.json();
    let ru = get(&a, "/api/records", Some(&uc)).await.json();
    assert_eq!(ra["records"].as_array().map(Vec::len), Some(1));
    assert_eq!(ru["records"].as_array().map(Vec::len), Some(1));
    let admin_record = ra["records"][0]["id"].as_i64().expect("id");
    let user_record = ru["records"][0]["id"].as_i64().expect("id");
    // another user's record is "not found", for reading and deleting
    let r = get(&a, &format!("/api/record?id={admin_record}"), Some(&uc)).await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (404, MSG_RECORD_NOT_FOUND.to_string())
    );
    assert_eq!(
        get(&a, &format!("/api/record?id={user_record}"), Some(&uc))
            .await
            .status,
        200
    );
    let r = post(
        &a,
        "/api/records/delete",
        Some(&uc),
        &json!({"id": admin_record}).to_string(),
    )
    .await;
    assert_eq!(r.status, 404);
    assert_eq!(
        get(&a, &format!("/api/record?id={admin_record}"), Some(&c))
            .await
            .status,
        200
    );
    // settings
    post(
        &a,
        "/api/settings",
        Some(&uc),
        r#"{"theme": "dark", "uiLang": "ja"}"#,
    )
    .await;
    assert_eq!(
        get(&a, "/api/settings", Some(&c)).await.json()["theme"],
        "system"
    );
    assert_eq!(
        get(&a, "/api/settings", Some(&uc)).await.json()["theme"],
        "dark"
    );
    post(&a, "/api/settings/reset", Some(&c), "{}").await;
    assert_eq!(
        get(&a, "/api/settings", Some(&uc)).await.json()["uiLang"],
        "ja"
    );
    // delete all / reset touch only your own
    let r = post(
        &a,
        "/api/records/delete",
        Some(&uc),
        r#"{"all": true, "withStats": true}"#,
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["records"], json!([]));
    assert_eq!(
        get(&a, "/api/records", Some(&c)).await.json()["records"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    post(
        &a,
        "/api/attempt",
        Some(&uc),
        &attempt_body(&[("q3", true)]),
    )
    .await;
    let r = post(&a, "/api/reset", Some(&c), "{}").await;
    assert_eq!(r.json()["history"], json!([]));
    assert_eq!(
        get(&a, "/api/progress", Some(&uc)).await.json()["history"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    // retention: 500 per user
    {
        let mut db = raw(&a.db);
        let tx = db.transaction().expect("tx");
        for _ in 0..xexam_backend::database::RECORD_RETENTION {
            tx.execute(
                "INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, user_id) VALUES('t', '[]', 'all', 1, 1, 100, 1, ?1)",
                [uid],
            )
            .expect("attempt");
        }
        tx.commit().expect("commit");
    }
    a.store
        .record_attempt(1, &attempt(&[("q1", true)]))
        .expect("admin");
    let per_user = |id: i64| {
        count(
            &raw(&a.db),
            &format!("SELECT COUNT(*) FROM attempts WHERE user_id={id}"),
        )
    };
    assert_eq!(
        (per_user(1), per_user(uid)),
        (1, 501),
        "another user's exam does not prune yours"
    );
    a.store
        .record_attempt(uid, &attempt(&[("q1", true)]))
        .expect("user");
    assert_eq!((per_user(1), per_user(uid)), (1, 500));
}

#[tokio::test]
async fn deleting_a_bank_with_records_removes_the_study_data_of_every_user() {
    let a = app();
    let c = admin(&a).await;
    let (uid, uc) = user(&a, &c, "cook", "user").await;
    let s = &a.store;
    let (sh, qs) = (
        vec![dbc::sheet("Cooking", 2)],
        vec![
            dbc::single("Cooking", "k1", "Water boils at?", 1),
            dbc::single("Cooking", "k2", "Salt or sugar?", 2),
        ],
    );
    s.apply_import("legacy.xlsx", &sh, &qs)
        .expect("legacy bank");
    let (_, bank2) = s
        .apply_import_for(ImportTarget::New, "second.xlsx", &sh, &qs)
        .expect("bank 2")
        .expect("created");
    let ids2: Vec<String> = s.bank().expect("bank")["questions"]
        .as_array()
        .expect("q")
        .iter()
        .filter(|q| q["bank"] == bank2)
        .map(|q| q["id"].as_str().expect("id").to_string())
        .collect();
    for cookie in [&c, &uc] {
        let r = post(
            &a,
            "/api/attempt",
            Some(cookie),
            &attempt_body(&[(ids2[0].as_str(), false), (ids2[1].as_str(), true)]),
        )
        .await;
        assert_eq!(r.status, 200);
        post(
            &a,
            "/api/attempt",
            Some(cookie),
            &attempt_body(&[("k1", true)]),
        )
        .await;
    }
    // v0.6.5: the shared numbers are the administrator's (what their deletion removes); a plain user sees their own
    let banks = get(&a, "/api/banks", Some(&c)).await.json();
    assert_eq!(
        banks["banks"][1]["records"], 2,
        "shared numbers count every user"
    );
    assert_eq!(banks["banks"][1]["studied"], 2);
    let own = get(&a, "/api/banks", Some(&uc)).await.json();
    assert_eq!(own["banks"][1]["records"], 1, "the user's own records");
    assert_eq!(own["banks"][1]["studied"], 2, "the user's own statistics");
    let r = post(
        &a,
        "/api/banks/delete",
        Some(&c),
        &json!({"id": bank2, "withRecords": true}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let db = raw(&a.db);
    for user in [1, uid] {
        assert_eq!(
            count(
                &db,
                &format!("SELECT COUNT(*) FROM attempts WHERE user_id={user}")
            ),
            1,
            "user {user}"
        );
        assert_eq!(
            count(
                &db,
                &format!("SELECT COUNT(*) FROM user_stats WHERE user_id={user}")
            ),
            1,
            "user {user}: only k1 is left"
        );
    }
    assert_eq!(count(&db, "SELECT COUNT(*) FROM attempt_items"), 2);
    assert_eq!(
        get(&a, "/api/progress", Some(&uc)).await.json()["questions"]
            .as_object()
            .map(|m| m.len()),
        Some(1)
    );
}

// ---------------------------------------------------------------- sessions

#[tokio::test]
async fn sessions_expire_refresh_and_are_purged() {
    let a = app();
    let c = admin(&a).await;
    let hash = auth::token_hash(c.trim_start_matches("xexam_session="));
    let db = raw(&a.db);
    let now = auth::unix_now();
    let seen = |db: &Connection| -> (i64, i64) {
        db.query_row(
            "SELECT last_seen_at, expires_at FROM sessions WHERE token_hash=?1",
            [&hash],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("session")
    };
    let (last, exp) = seen(&db);
    assert!((last - now).abs() < 5 && exp - last == auth::SESSION_TTL_SECS);
    // within the hour: no refresh, no cookie
    let r = post(&a, "/api/settings", Some(&c), "{}").await;
    assert!(r.header("set-cookie").is_none());
    // two hours later: a GET (no Origin) does not refresh, a POST (with Origin) does and re-sends the cookie
    db.execute(
        "UPDATE sessions SET last_seen_at=last_seen_at-7200, expires_at=expires_at-7200 WHERE token_hash=?1",
        [&hash],
    )
    .expect("backdate");
    let r = get(&a, "/api/settings", Some(&c)).await;
    assert_eq!(r.status, 200);
    assert!(r.header("set-cookie").is_none());
    assert_eq!(seen(&db).0, last - 7200);
    let r = post(&a, "/api/settings", Some(&c), "{}").await;
    assert_eq!(r.status, 200);
    assert_eq!(
        r.header("set-cookie").as_deref(),
        Some(auth::session_cookie(c.trim_start_matches("xexam_session="), false).as_str())
    );
    let (last2, exp2) = seen(&db);
    assert!(last2 >= now && exp2 == last2 + auth::SESSION_TTL_SECS);
    // with XEXAM_COOKIE_SECURE=1 a GET refreshes too (the scheme is known), with a Secure cookie
    let forced = app_with(
        apic::config_with(&[("XEXAM_COOKIE_SECURE", "1")]),
        ThrottleLimits::default(),
    );
    let fc = admin(&forced).await;
    raw(&forced.db)
        .execute("UPDATE sessions SET last_seen_at=last_seen_at-7200", [])
        .expect("backdate");
    let r = get(&forced, "/api/bank", Some(&fc)).await;
    assert!(r
        .header("set-cookie")
        .expect("cookie")
        .ends_with("; Secure"));
    // expired: 401, and the row is gone
    db.execute(
        "UPDATE sessions SET expires_at=?1 WHERE token_hash=?2",
        rusqlite::params![now - 1, hash],
    )
    .expect("expire");
    let r = get(&a, "/api/bank", Some(&c)).await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (401, MSG_SIGN_IN_REQUIRED.to_string())
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM sessions WHERE token_hash=?1",
            [&hash],
            |r| r.get::<_, i64>(0)
        )
        .expect("count"),
        0
    );
    // the opportunistic purge removes other expired sessions (at most once an hour)
    let c2 = cookie_of(&login(&a, "admin", ADMIN_PW).await);
    db.execute(
        "INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at) VALUES('old', 1, 't', 1, 2)",
        [],
    )
    .expect("old session");
    a.state
        .0
        .last_session_purge
        .store(0, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(get(&a, "/api/bank", Some(&c2)).await.status, 200);
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM sessions WHERE token_hash='old'"),
        0
    );
    // the store purge at startup
    db.execute(
        "INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at) VALUES('old2', 1, 't', 1, 2)",
        [],
    )
    .expect("old session");
    assert_eq!(a.store.purge_expired_sessions(now).expect("purge"), 1);
}

#[tokio::test]
async fn logout_ends_only_that_session() {
    let a = app();
    let c = admin(&a).await;
    let other = cookie_of(&login(&a, "admin", ADMIN_PW).await);
    let r = post(&a, "/api/auth/logout", Some(&c), "").await;
    assert_eq!(r.status, 200);
    assert_eq!(get(&a, "/api/bank", Some(&c)).await.status, 401);
    assert_eq!(
        get(&a, "/api/auth/state", Some(&c)).await.json(),
        json!({"user": null, "defaultAdmin": null})
    );
    assert_eq!(get(&a, "/api/bank", Some(&other)).await.status, 200);
    let r = post(&a, "/api/auth/logout", Some(&c), "{}").await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (401, MSG_SIGN_IN_REQUIRED.to_string())
    );
}

// ---------------------------------------------------------------- reset-password command

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
fn reset_password_command() {
    let dir = dbc::tmp();
    let data = dir.path().to_str().expect("utf-8 path").to_string();
    let db = dir.path().join("xexam.db");
    {
        // an administrator who changed the password, then got disabled somehow, with a session
        let s = Store::open(&db).expect("open");
        let u = s.user_by_name("admin").expect("l").expect("admin");
        let forgotten = auth::hash_password("forgotten-pass").expect("h");
        s.change_own_password(
            u.id,
            &u.password_hash,
            &forgotten,
            &auth::token_hash(&auth::new_session_token()),
            auth::unix_now(),
        )
        .expect("change")
        .expect("changed");
        s.start_session(u.id, "somehash", &forgotten, auth::unix_now())
            .expect("session")
            .expect("started");
        raw(&db)
            .execute("UPDATE users SET disabled=1", [])
            .expect("disable");
    }
    let out = cli(&["reset-password", "ADMIN", "--data", &data]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let line = stdout
        .lines()
        .find(|l| l.starts_with("New password for 'admin': "))
        .expect("password line");
    let password = line.rsplit(": ").next().expect("password");
    assert_eq!(password.chars().count(), 16);
    let s = Store::open(&db).expect("open");
    let u = s.user_by_name("admin").expect("l").expect("admin");
    assert!(auth::verify_password(password, &u.password_hash));
    assert!(u.must_change_password && !u.disabled);
    assert_eq!(count(&raw(&db), "SELECT COUNT(*) FROM sessions"), 0);
    drop(s);
    // an unknown user: clear message, non-zero exit, nothing changed
    let before = dbc::db_bytes(&db);
    let out = cli(&["reset-password", "nobody", "--data", &data]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("There is no user named 'nobody'"));
    assert_eq!(dbc::db_bytes(&db), before);
    // help and usage errors
    let out = cli(&["reset-password", "--help"]);
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        help.contains("docker compose exec backend xexam-backend reset-password admin"),
        "{help}"
    );
    let out = cli(&["reset-password"]);
    assert_eq!(out.status.code(), Some(2));
}

// ---------------------------------------------------------------- races, limits and recovery (review fixes)

/// A sign-in request as a browser sends it (for tasks that run it in the background).
fn login_request(name: &str, password: &str) -> axum::http::Request<Body> {
    let body = json!({"username": name, "password": password}).to_string();
    let len = body.len().to_string();
    build(
        "POST",
        "/api/auth/login",
        Some(HOST),
        &[
            ("origin", ORIGIN.as_bytes()),
            ("content-type", b"application/json"),
            ("content-length", len.as_bytes()),
        ],
        Body::from(body),
    )
}

fn sessions_of(db: &Path, id: i64) -> i64 {
    raw(db)
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE user_id=?1",
            [id],
            |r| r.get(0),
        )
        .expect("count")
}

#[tokio::test]
async fn a_password_reset_while_a_sign_in_or_a_change_is_verifying_wins() {
    let a = app();
    let c = admin(&a).await;
    let (id, _) = user(&a, &c, "ann", "user").await;
    let before = sessions_of(&a.db, id);
    assert_eq!(before, 1);

    // 1. A sign-in with the old password has read the hash and waits for an Argon2 slot; the admin resets the password.
    let gate = a.state.0.hash_gate.clone();
    let permits = gate.clone().acquire_many_owned(2).await.expect("permits");
    let router = a.router.clone();
    let task =
        tokio::spawn(async move { send(&router, login_request("ann", "ann-own-pass")).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!task.is_finished(), "the sign-in waits for a hashing slot");
    let reset = auth::hash_password("reset-by-admin").expect("hash");
    assert_eq!(
        a.store.reset_user_password(1, id, &reset).expect("reset"),
        UserChange::Done("ann".to_string())
    );
    drop(permits);
    let r = task.await.expect("join");
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (401, MSG_BAD_LOGIN.to_string())
    );
    assert!(r.header("set-cookie").is_none());
    assert_eq!(
        sessions_of(&a.db, id),
        0,
        "no session with the old password"
    );

    // 2. A password change has read the hash and waits; the admin resets again. The reset is not silently overwritten.
    let r = login(&a, "ann", "reset-by-admin").await;
    assert_eq!(r.status, 200, "{}", r.text());
    let ac = cookie_of(&r);
    let permits = gate.clone().acquire_many_owned(2).await.expect("permits");
    let router = a.router.clone();
    let body = json!({"password": "ann-chosen-pass"}).to_string();
    let task = tokio::spawn(async move {
        let len = body.len().to_string();
        send(
            &router,
            build(
                "POST",
                "/api/auth/password",
                Some(HOST),
                &[
                    ("origin", ORIGIN.as_bytes()),
                    ("cookie", ac.as_bytes()),
                    ("content-type", b"application/json"),
                    ("content-length", len.as_bytes()),
                ],
                Body::from(body),
            ),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let second = auth::hash_password("reset-again-1").expect("hash");
    assert_eq!(
        a.store.reset_user_password(1, id, &second).expect("reset"),
        UserChange::Done("ann".to_string())
    );
    drop(permits);
    let r = task.await.expect("join");
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (401, MSG_SIGN_IN_REQUIRED.to_string())
    );
    let u = a.store.user_by_id(id).expect("l").expect("ann");
    assert_eq!(u.password_hash, second);
    assert!(u.must_change_password);
}

#[test]
fn the_store_refuses_a_session_or_a_password_change_for_a_stale_hash() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = Store::open(&db).expect("open");
    let u = s.user_by_name("admin").expect("l").expect("admin");
    let now = auth::unix_now();
    assert_eq!(
        s.start_session(u.id, "h1", "$argon2id$an-older-hash", now)
            .expect("start"),
        None
    );
    assert_eq!(count(&raw(&db), "SELECT COUNT(*) FROM sessions"), 0);
    let new_hash = auth::hash_password("brand-new-pass").expect("hash");
    assert_eq!(
        s.change_own_password(
            u.id,
            "$argon2id$an-older-hash",
            &new_hash,
            &auth::token_hash(&auth::new_session_token()),
            auth::unix_now()
        )
        .expect("change"),
        None
    );
    let after = s.user_by_id(u.id).expect("l").expect("admin");
    assert_eq!(after.password_hash, u.password_hash, "nothing was written");
    assert!(after.must_change_password);
    // a disabled user cannot change the password either
    raw(&db)
        .execute("UPDATE users SET disabled=1", [])
        .expect("disable");
    assert_eq!(
        s.change_own_password(
            u.id,
            &u.password_hash,
            &new_hash,
            &auth::token_hash(&auth::new_session_token()),
            auth::unix_now()
        )
        .expect("change"),
        None
    );
}

#[test]
fn sessions_are_capped_per_user() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = Store::open(&db).expect("open");
    let u = s.user_by_name("admin").expect("l").expect("admin");
    let now = auth::unix_now();
    let cap = auth::MAX_SESSIONS_PER_USER;
    for i in 0..cap + 10 {
        s.start_session(u.id, &format!("h{i}"), &u.password_hash, now)
            .expect("start")
            .expect("started");
    }
    let c = raw(&db);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM sessions"), cap);
    // the newest sessions stay, the oldest went first
    assert_eq!(
        count(
            &c,
            &format!(
                "SELECT COUNT(*) FROM sessions WHERE token_hash='h{}'",
                cap + 9
            )
        ),
        1
    );
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM sessions WHERE token_hash='h0'"),
        0
    );
}

#[tokio::test]
async fn a_flood_of_failed_sign_ins_never_locks_out_an_account_that_is_not_being_guessed() {
    let a = app_with(
        apic::default_config(),
        ThrottleLimits {
            global: 10,
            ..ThrottleLimits::default()
        },
    );
    let c = admin(&a).await;
    drop(c);
    for i in 0..10 {
        let r = login(&a, &format!("nobody{i}"), "made-up-pass").await;
        assert_eq!(r.status, 401, "{i}");
    }
    // under pressure: the administrator, who had no failure, still signs in with the right password
    let r = login(&a, "admin", ADMIN_PW).await;
    assert_eq!(r.status, 200, "{}", r.text());
    // (a success is taken back out of the count; one more made-up name keeps the pressure on)
    assert_eq!(login(&a, "nobody10", "made-up-pass").await.status, 401);
    // a name that was just guessed waits a minute
    let r = login(&a, "nobody0", "made-up-pass").await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (429, MSG_TOO_MANY_LOGINS.to_string())
    );
    // huge made-up names share one key and are slowed down together
    let huge = "x".repeat(500_000);
    assert_eq!(login(&a, &huge, "made-up-pass").await.status, 401);
    assert_eq!(
        login(&a, &format!("y{huge}"), "made-up-pass").await.status,
        429
    );
    let (_, longest) = a.state.0.throttle.key_stats();
    assert!(longest <= auth::USERNAME_MAX_CHARS, "{longest}");
}

#[tokio::test]
async fn huge_usernames_do_not_grow_the_throttle() {
    let a = app();
    for i in 0..20 {
        let name = format!("{i}{}", "z".repeat(100_000));
        let r = login(&a, &name, "made-up-pass").await;
        assert!(r.status == 401 || r.status == 429, "{i}: {}", r.status);
    }
    let (keys, longest) = a.state.0.throttle.key_stats();
    assert_eq!((keys, longest), (1, 0), "one shared key for invalid names");
}

#[tokio::test]
async fn a_wrong_current_password_is_throttled_like_a_sign_in() {
    let a = app();
    let c = admin(&a).await;
    let body = json!({"current": "wrong-pass-1", "password": "something-new"}).to_string();
    for i in 0..5 {
        let r = post(&a, "/api/auth/password", Some(&c), &body).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (403, MSG_WRONG_PASSWORD.to_string()),
            "{i}"
        );
    }
    let r = post(&a, "/api/auth/password", Some(&c), &body).await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (429, MSG_TOO_MANY_LOGINS.to_string())
    );
    let r = login(&a, "admin", ADMIN_PW).await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (429, MSG_TOO_MANY_LOGINS.to_string())
    );
}

#[tokio::test]
async fn an_admin_reset_lifts_the_login_lock_of_that_user() {
    let a = app();
    let c = admin(&a).await;
    let (id, _) = user(&a, &c, "bea", "user").await;
    for _ in 0..5 {
        assert_eq!(login(&a, "bea", "not-her-pass").await.status, 401);
    }
    assert_eq!(login(&a, "bea", "bea-own-pass").await.status, 429);
    let r = post(
        &a,
        "/api/users/password",
        Some(&c),
        &json!({"id": id, "password": "fresh-start-1"}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(login(&a, "bea", "fresh-start-1").await.status, 200);
}

/// The real engine, except that it deletes (or disables) the user `victim` just before a per-user write: exactly what happens
/// when an administrator deletes a user while one of their requests is between the session check and the write.
struct DeletesFirst {
    inner: RealEngine,
    store: Arc<Store>,
    victim: std::sync::Mutex<Option<(i64, bool)>>,
}

impl DeletesFirst {
    fn strike(&self) {
        let victim = self.victim.lock().expect("lock").take();
        if let Some((id, delete)) = victim {
            if delete {
                assert!(matches!(
                    self.store.delete_user(1, id).expect("delete"),
                    UserChange::Done(_)
                ));
            } else {
                let change = UserUpdate {
                    disabled: Some(true),
                    ..UserUpdate::default()
                };
                assert!(matches!(
                    self.store.update_user(1, id, &change).expect("disable"),
                    UserChange::Done(_)
                ));
            }
        }
    }
}

impl Engine for DeletesFirst {
    fn check_and_parse(
        &self,
        data: &[u8],
    ) -> Result<xexam_backend::models::ParsedWorkbook, xexam_backend::error::ImportError> {
        self.inner.check_and_parse(data)
    }
    fn count_active(&self) -> Result<i64, DbError> {
        self.inner.count_active()
    }
    fn bank(&self) -> Result<Value, DbError> {
        self.inner.bank()
    }
    fn progress(&self, user: i64) -> Result<Value, DbError> {
        self.inner.progress(user)
    }
    fn import_history(
        &self,
        limit: i64,
    ) -> Result<Vec<xexam_backend::models::ImportRecord>, DbError> {
        self.inner.import_history(limit)
    }
    fn plan_import(
        &self,
        sheets: &[xexam_backend::models::ParsedSheet],
        questions: &[xexam_backend::models::Question],
    ) -> Result<Value, DbError> {
        self.inner.plan_import(sheets, questions)
    }
    fn apply_import(
        &self,
        filename: &str,
        sheets: &[xexam_backend::models::ParsedSheet],
        questions: &[xexam_backend::models::Question],
    ) -> Result<Value, DbError> {
        self.inner.apply_import(filename, sheets, questions)
    }
    fn record_attempt(&self, user: i64, attempt: &Attempt) -> Result<Value, DbError> {
        self.strike();
        self.inner.record_attempt(user, attempt)
    }
    fn reset_progress(&self, user: i64) -> Result<Value, DbError> {
        self.inner.reset_progress(user)
    }
    fn record_attempt_with_answers(
        &self,
        user: i64,
        attempt: &Attempt,
        answers: &[Option<Value>],
    ) -> Result<Value, DbError> {
        self.strike();
        self.inner
            .record_attempt_with_answers(user, attempt, answers)
    }
    // v0.7: a plain user's scope is always their own list of banks, so the route calls this one
    fn record_attempt_as(
        &self,
        v: &xexam_backend::models::Viewer,
        attempt: &Attempt,
        answers: &[Option<Value>],
    ) -> Result<Value, DbError> {
        self.strike();
        self.inner.record_attempt_as(v, attempt, answers)
    }
    fn update_settings(&self, user: i64, changes: &[(String, Value)]) -> Result<Value, DbError> {
        self.strike();
        self.inner.update_settings(user, changes)
    }
    fn reset_settings(&self, user: i64) -> Result<Value, DbError> {
        self.strike();
        self.inner.reset_settings(user)
    }
}

#[tokio::test]
async fn writes_of_a_user_deleted_or_disabled_mid_request_are_refused() {
    let holder: Arc<std::sync::OnceLock<Arc<DeletesFirst>>> = Arc::new(std::sync::OnceLock::new());
    let slot = holder.clone();
    let a = app_with_engine(
        apic::default_config(),
        ThrottleLimits::default(),
        move |store| {
            let e = Arc::new(DeletesFirst {
                inner: RealEngine::new(store.clone()),
                store,
                victim: std::sync::Mutex::new(None),
            });
            let _ = slot.set(e.clone());
            e
        },
    );
    let engine = holder.get().expect("engine").clone();
    let c = admin(&a).await;
    let owned = |id: i64| {
        let db = raw(&a.db);
        let n = |t: &str| -> i64 {
            db.query_row(
                &format!("SELECT COUNT(*) FROM {t} WHERE user_id=?1"),
                [id],
                |r| r.get(0),
            )
            .expect("count")
        };
        n("attempts") + n("user_stats") + n("user_settings")
    };
    let cases: [(&str, &str, String, bool); 4] = [
        ("cara", "/api/attempt", attempt_body(&[("q1", true)]), true),
        (
            "dina",
            "/api/settings",
            r#"{"theme": "dark"}"#.to_string(),
            true,
        ),
        (
            "emil",
            "/api/attempt",
            attempt_body(&[("q1", false)]),
            false,
        ),
        (
            "fynn",
            "/api/settings",
            r#"{"theme": "dark"}"#.to_string(),
            false,
        ),
    ];
    for (name, path, body, delete) in cases {
        let (id, uc) = user(&a, &c, name, "user").await;
        *engine.victim.lock().expect("lock") = Some((id, delete));
        let r = post(&a, path, Some(&uc), &body).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (401, MSG_SIGN_IN_REQUIRED.to_string()),
            "{name} {path}"
        );
        assert_eq!(owned(id), 0, "{name}: nothing written under the id");
    }
    // the reset of the settings is refused the same way
    let (id, uc) = user(&a, &c, "gus", "user").await;
    *engine.victim.lock().expect("lock") = Some((id, true));
    let r = post(&a, "/api/settings/reset", Some(&uc), "{}").await;
    assert_eq!(r.status, 401);
    // The engine holds the store: let go of it before the app, so the app's temporary directory can be removed
    drop(engine);
    drop(holder);
}

#[test]
fn per_user_writes_for_a_deleted_or_disabled_user_write_nothing() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = Store::open(&db).expect("open");
    dbc::seed_questions(&db, &["q1"]);
    let hash = auth::hash_password("initial-pass").expect("hash");
    let UserChange::Done(gone) = s
        .create_user("gone", &hash, auth::Role::User)
        .expect("create")
    else {
        panic!("create");
    };
    let UserChange::Done(off) = s
        .create_user("off", &hash, auth::Role::User)
        .expect("create")
    else {
        panic!("create");
    };
    assert_eq!(
        s.delete_user(1, gone).expect("delete"),
        UserChange::Done("gone".to_string())
    );
    raw(&db)
        .execute("UPDATE users SET disabled=1 WHERE id=?1", [off])
        .expect("disable");
    for id in [gone, off, 999] {
        assert!(matches!(
            s.record_attempt(id, &attempt(&[("q1", true)])),
            Err(DbError::NoUser)
        ));
        assert!(matches!(
            s.update_settings(id, &[("theme".to_string(), json!("dark"))]),
            Err(DbError::NoUser)
        ));
        assert!(matches!(s.reset_settings(id), Err(DbError::NoUser)));
    }
    let c = raw(&db);
    for t in ["attempts", "user_stats", "user_settings"] {
        assert_eq!(
            count(&c, &format!("SELECT COUNT(*) FROM {t} WHERE user_id<>1")),
            0,
            "{t}"
        );
    }
    // the administrator still writes normally
    s.record_attempt(1, &attempt(&[("q1", true)]))
        .expect("admin");
}

#[test]
fn reset_password_never_creates_a_database() {
    let dir = dbc::tmp();
    let empty = dir.path().join("empty");
    std::fs::create_dir(&empty).expect("mkdir");
    let missing = dir.path().join("missing");
    for data in [&empty, &missing] {
        let out = cli(&[
            "reset-password",
            "admin",
            "--data",
            data.to_str().expect("utf-8"),
        ]);
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        assert_eq!(out.status.code(), Some(1), "{stdout}");
        assert!(stdout.contains("There is no database at"), "{stdout}");
        assert!(stdout.contains("xexam.db"), "{stdout}");
        assert!(!stdout.contains("New password"), "{stdout}");
        assert!(!data.join("xexam.db").exists());
    }
    assert_eq!(std::fs::read_dir(&empty).expect("read").count(), 0);
    assert!(!missing.exists(), "the data directory is not created");
}

#[tokio::test]
async fn every_get_route_has_its_own_handler() {
    let a = app();
    let c = admin(&a).await;
    for r in ROUTES.iter().filter(|r| r.method == "GET") {
        let resp = get(&a, r.path, Some(&c)).await;
        assert!(
            resp.status != 404 && resp.status != 500,
            "{} {}",
            r.path,
            resp.status
        );
        assert_eq!(
            resp.json().get("imports").is_some(),
            r.path == "/api/imports",
            "{}",
            r.path
        );
    }
}

#[test]
fn a_v2_database_whose_backup_cannot_be_written_is_left_unchanged() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    write_v2_database(&db);
    std::fs::write(dir.path().join("backups"), b"not a directory").expect("write");
    let before = dbc::db_bytes(&db);
    let err = Store::open(&db).err().expect("the open fails");
    assert!(matches!(err, DbError::Backup(_)), "{err}");
    assert_eq!(dbc::db_bytes(&db), before, "byte-identical");
    let c = raw(&db);
    assert_eq!(
        c.query_row("SELECT value FROM meta WHERE key='schema'", [], |r| r
            .get::<_, String>(0))
            .expect("schema"),
        "2"
    );
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('users', 'sessions', 'user_stats', 'user_settings')"
        ),
        0
    );
}

#[tokio::test]
async fn argon2_runs_at_most_two_at_a_time() {
    let a = app();
    assert_eq!(auth::HASH_CONCURRENCY, 2);
    assert_eq!(a.state.0.hash_gate.available_permits(), 2);
    let c = admin(&a).await;
    drop(c);
    // both slots taken: sign-ins wait (they are not refused and do not run a third hash)
    let permits = a
        .state
        .0
        .hash_gate
        .clone()
        .acquire_many_owned(2)
        .await
        .expect("permits");
    let mut tasks = Vec::new();
    for _ in 0..3 {
        let router = a.router.clone();
        tasks.push(tokio::spawn(async move {
            send(&router, login_request("admin", ADMIN_PW)).await
        }));
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(tasks.iter().all(|t| !t.is_finished()));
    drop(permits);
    for t in tasks {
        assert_eq!(t.await.expect("join").status, 200);
    }
    assert_eq!(a.state.0.hash_gate.available_permits(), 2);
}

#[tokio::test]
async fn admin_routes_answer_403_before_a_large_body_is_read() {
    let a = app();
    let c = admin(&a).await;
    let (_, uc) = user(&a, &c, "hugo", "user").await;
    let big = vec![b'x'; 2_000_000];
    let len = big.len().to_string();
    for r in ROUTES
        .iter()
        .filter(|r| r.access == Access::Admin && r.method == "POST")
    {
        let resp = send(
            &a.router,
            build(
                "POST",
                r.path,
                Some(HOST),
                &[
                    ("origin", ORIGIN.as_bytes()),
                    ("cookie", uc.as_bytes()),
                    ("content-type", b"application/json"),
                    ("content-length", len.as_bytes()),
                ],
                Body::from(big.clone()),
            ),
        )
        .await;
        assert_eq!(
            (resp.status.as_u16(), resp.error()),
            (403, MSG_ADMIN_ONLY.to_string()),
            "{}",
            r.path
        );
    }
}

#[tokio::test]
async fn a_session_of_a_user_disabled_in_the_database_is_refused() {
    let a = app();
    let c = admin(&a).await;
    let (id, uc) = user(&a, &c, "ivy", "user").await;
    assert_eq!(get(&a, "/api/bank", Some(&uc)).await.status, 200);
    raw(&a.db)
        .execute("UPDATE users SET disabled=1 WHERE id=?1", [id])
        .expect("disable");
    let r = get(&a, "/api/bank", Some(&uc)).await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (401, MSG_SIGN_IN_REQUIRED.to_string())
    );
    assert_eq!(
        get(&a, "/api/auth/state", Some(&uc)).await.json(),
        json!({"user": null, "defaultAdmin": null})
    );
    assert_eq!(
        sessions_of(&a.db, id),
        1,
        "the row stays; it is refused when read"
    );
}

// ---------------------------------------------------------------- review fixes (v0.6)

/// `name=value` of the `Set-Cookie` header for cookie `name`, if the response sets it.
fn set_cookie_pair(r: &Resp, name: &str) -> Option<String> {
    r.headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with(&format!("{name}=")))
        .and_then(|v| v.split(';').next())
        .map(str::to_string)
}

#[tokio::test]
async fn guesses_from_elsewhere_do_not_lock_the_owner_out_of_a_known_browser() {
    let a = app();
    let c = admin(&a).await;
    drop(c);
    // The owner signs in once in their browser: it gets the known-browser cookie (only for /api/auth, same rules as the session)
    let r = login(&a, "admin", ADMIN_PW).await;
    assert_eq!(r.status, 200);
    let device = set_cookie_pair(&r, auth::DEVICE_COOKIE).expect("device cookie");
    let sc: Vec<String> = r
        .headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok().map(str::to_string))
        .collect();
    let line = sc
        .iter()
        .find(|v| v.starts_with(auth::DEVICE_COOKIE))
        .expect("line");
    assert!(
        line.contains("HttpOnly; SameSite=Strict; Path=/api/auth; Max-Age="),
        "{line}"
    );
    assert!(!line.contains("Secure"), "plain HTTP: {line}");
    assert!(
        sc.iter().any(|v| v.starts_with("xexam_session=")),
        "the session cookie is still set"
    );
    // Somebody else keeps guessing: their attempts are locked, even with the right password
    for _ in 0..5 {
        assert_eq!(login(&a, "admin", "a-wrong-guess").await.status, 401);
    }
    assert_eq!(login(&a, "admin", ADMIN_PW).await.status, 429);
    // The owner's browser still signs in
    let body = json!({"username": "Admin", "password": ADMIN_PW}).to_string();
    let r = call(
        &a,
        "POST",
        "/api/auth/login",
        Some(&device),
        Some(ORIGIN),
        Some(&body),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let session = cookie_of(&r);
    // A forged or another user's value does not count
    let forged = format!("{}={}", auth::DEVICE_COOKIE, "A".repeat(auth::TOKEN_LEN));
    let r = call(
        &a,
        "POST",
        "/api/auth/login",
        Some(&forged),
        Some(ORIGIN),
        Some(&body),
    )
    .await;
    assert_eq!(r.status, 429);
    // Changing the password from Settings works in that browser too (same bucket) and replaces the cookie
    let both = format!("{session}; {device}");
    let r = call(
        &a,
        "POST",
        "/api/auth/password",
        Some(&both),
        Some(ORIGIN),
        Some(&json!({"current": ADMIN_PW, "password": "kitchen-admin-2"}).to_string()),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let fresh = set_cookie_pair(&r, auth::DEVICE_COOKIE).expect("new device cookie");
    assert_ne!(fresh, device, "a new password retires the old value");
    let body = json!({"username": "admin", "password": "kitchen-admin-2"}).to_string();
    let old = call(
        &a,
        "POST",
        "/api/auth/login",
        Some(&device),
        Some(ORIGIN),
        Some(&body),
    )
    .await;
    assert_eq!(old.status, 429, "the old value is no longer known");
    let r = call(
        &a,
        "POST",
        "/api/auth/login",
        Some(&fresh),
        Some(ORIGIN),
        Some(&body),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
}

#[tokio::test]
async fn the_sign_in_page_learns_whether_the_default_admin_password_is_still_set() {
    let a = app();
    let hashes = xexam_backend::api::auth::find_default_admin_hashes(&a.store).expect("hashes");
    assert_eq!(hashes.len(), 1, "the default administrator");
    a.state
        .0
        .default_admin_hashes
        .set(hashes)
        .expect("set once");
    let state = get(&a, "/api/auth/state", None).await.json();
    assert_eq!(state, json!({"user": null, "defaultAdmin": "admin"}));
    // Signing in alone does not change it (the password is still admin) …
    let r = login(&a, "admin", "admin").await;
    assert_eq!(r.status, 200);
    assert_eq!(
        get(&a, "/api/auth/state", None).await.json()["defaultAdmin"],
        "admin"
    );
    // … a rename in the database (the API refuses it before the new password) names the account as it is called now
    raw(&a.db)
        .execute(
            "UPDATE users SET username='owner' WHERE username='admin'",
            [],
        )
        .expect("rename");
    assert_eq!(
        get(&a, "/api/auth/state", None).await.json()["defaultAdmin"],
        "owner"
    );
    raw(&a.db)
        .execute(
            "UPDATE users SET username='admin' WHERE username='owner'",
            [],
        )
        .expect("rename back");
    // … choosing a new password does
    let r = post(
        &a,
        "/api/auth/password",
        Some(&cookie_of(&r)),
        &json!({ "password": ADMIN_PW }).to_string(),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(
        get(&a, "/api/auth/state", None).await.json(),
        json!({"user": null, "defaultAdmin": null})
    );
    // Without the startup check (unset) it is always null
    let b = app();
    assert_eq!(
        get(&b, "/api/auth/state", None).await.json()["defaultAdmin"],
        Value::Null
    );
}

#[tokio::test]
async fn the_library_stamp_changes_when_a_bank_is_imported_or_deleted() {
    let a = app();
    let c = admin(&a).await;
    let (_, uc) = user(&a, &c, "reader", "user").await;
    let stamp = |r: Resp| r.json()["library"].as_str().expect("library").to_string();
    let s0 = stamp(get(&a, "/api/auth/state", Some(&uc)).await);
    let admin0 = stamp(get(&a, "/api/auth/state", Some(&c)).await);
    // v0.7: a plain user's stamp is made of their own banks only (they have a list of banks, never "every bank")
    assert_ne!(admin0, s0);
    let sh = vec![dbc::sheet("Pantry", 2)];
    let qs = vec![
        dbc::single("Pantry", "p1", "Where does flour go?", 1),
        dbc::single("Pantry", "p2", "Rice or pasta?", 2),
    ];
    let (_, bank) = a
        .store
        .apply_import_for(ImportTarget::New, "pantry.xlsx", &sh, &qs)
        .expect("import")
        .expect("created");
    let s1 = stamp(get(&a, "/api/auth/state", Some(&uc)).await);
    assert_ne!(
        s1, s0,
        "a new bank (it reaches the reader: banks imported later)"
    );
    assert_ne!(stamp(get(&a, "/api/auth/state", Some(&c)).await), admin0);
    // the same file again into the same bank (nothing else changes but the import history)
    a.store
        .apply_import_for(ImportTarget::Bank(bank), "pantry.xlsx", &sh, &qs)
        .expect("replace")
        .expect("replaced");
    let s2 = stamp(get(&a, "/api/auth/state", Some(&uc)).await);
    assert_ne!(s2, s1, "a replaced bank");
    assert_eq!(
        stamp(get(&a, "/api/auth/state", Some(&uc)).await),
        s2,
        "stable"
    );
    let r = post(
        &a,
        "/api/banks/delete",
        Some(&c),
        &json!({"id": bank}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_ne!(
        stamp(get(&a, "/api/auth/state", Some(&uc)).await),
        s2,
        "a deleted bank"
    );
    // not shown before signing in
    assert!(get(&a, "/api/auth/state", None)
        .await
        .json()
        .get("library")
        .is_none());
}

// ---------------------------------------------------------------- escalating login lock (v0.6, owner's rule)

/// An app whose login throttle reads a clock the test moves forward with the returned handle (seconds).
fn app_with_moving_clock() -> (App, Arc<std::sync::atomic::AtomicU64>) {
    use std::sync::atomic::Ordering;
    let offset = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let o = offset.clone();
    let clock: auth::ThrottleClock =
        Arc::new(move || std::time::Instant::now() + Duration::from_secs(o.load(Ordering::SeqCst)));
    let a = app_with_throttle(
        apic::default_config(),
        LoginThrottle::with_clock(ThrottleLimits::default(), clock),
        |store| Arc::new(RealEngine::new(store)),
    );
    (a, offset)
}

fn advance(offset: &std::sync::atomic::AtomicU64, secs: u64) {
    offset.fetch_add(secs, std::sync::atomic::Ordering::SeqCst);
}

/// Asserts a 429 with `MSG_TOO_MANY_LOGINS` and the given wait in the body and the `Retry-After` header.
fn assert_wait(r: &Resp, secs: u64) {
    assert_eq!(r.status, 429, "{}", r.text());
    assert_eq!(
        r.json(),
        json!({"error": MSG_TOO_MANY_LOGINS, "retryAfter": secs}),
        "body"
    );
    assert_eq!(r.header("retry-after"), Some(secs.to_string()));
}

/// `"lock"` of `name` in `GET /api/users`.
async fn lock_of(a: &App, admin_cookie: &str, name: &str) -> Value {
    let r = get(a, "/api/users", Some(admin_cookie)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()["users"]
        .as_array()
        .expect("users")
        .iter()
        .find(|u| u["username"] == name)
        .map(|u| u["lock"].clone())
        .expect("user")
}

#[tokio::test]
async fn the_lock_escalates_by_rounds_and_tells_how_long_to_wait() {
    let (a, clock) = app_with_moving_clock();
    let c = admin(&a).await;
    let (id, _) = user(&a, &c, "bea", "user").await;
    assert_eq!(lock_of(&a, &c, "bea").await, Value::Null);
    // round 1: five failures, then 1 minute; even the right password waits and the wait does not grow
    for _ in 0..5 {
        assert_eq!(login(&a, "bea", "not-her-pass").await.status, 401);
    }
    assert_wait(&login(&a, "bea", "bea-own-pass").await, 60);
    assert_wait(&login(&a, "BEA", "not-her-pass").await, 60);
    let lock = lock_of(&a, &c, "bea").await;
    assert_eq!(
        (&lock["retryAfter"], &lock["round"], &lock["triesLeft"]),
        (&json!(60), &json!(1), &json!(0))
    );
    let until = lock["until"].as_str().expect("until");
    assert_eq!(until.len(), "2026-11-11 14:32:05".len(), "{until}");
    assert_eq!(lock_of(&a, &c, "admin").await, Value::Null);
    // rounds 2..: three failures each, counted from the end of the lock; 5 min, 15 min, 30 min, 1 hour, then 1 day (the maximum)
    let mut previous = 60;
    for (round, wait) in [
        (2, 300),
        (3, 900),
        (4, 1800),
        (5, 3600),
        (6, 86_400),
        (7, 86_400),
    ] {
        advance(&clock, previous);
        assert_eq!(
            lock_of(&a, &c, "bea").await,
            json!({"until": null, "retryAfter": 0, "round": round, "triesLeft": 3, "knownBrowser": false, "otherUntil": null, "otherRetryAfter": 0}),
            "round {round}"
        );
        for _ in 0..3 {
            assert_eq!(
                login(&a, "bea", "not-her-pass").await.status,
                401,
                "round {round}"
            );
        }
        assert_wait(&login(&a, "bea", "bea-own-pass").await, wait);
        assert_eq!(lock_of(&a, &c, "bea").await["round"], round);
        previous = wait;
    }
    // 24 hours after the last lock ended, with no new failure: five chances again
    advance(&clock, 86_400);
    assert_eq!(lock_of(&a, &c, "bea").await["round"], 8);
    advance(&clock, 86_400 - 1);
    assert_eq!(lock_of(&a, &c, "bea").await["round"], 8);
    advance(&clock, 1);
    assert_eq!(lock_of(&a, &c, "bea").await, Value::Null);
    for _ in 0..4 {
        assert_eq!(login(&a, "bea", "not-her-pass").await.status, 401);
    }
    let r = login(&a, "bea", "bea-own-pass").await;
    assert_eq!(r.status, 200, "{}", r.text());
    // a successful sign-in starts over too: the failures before it no longer count
    for _ in 0..4 {
        assert_eq!(login(&a, "bea", "not-her-pass").await.status, 401);
    }
    assert_eq!(lock_of(&a, &c, "bea").await, Value::Null);
    // an administrator's password reset starts over from any round
    assert_eq!(login(&a, "bea", "not-her-pass").await.status, 401);
    advance(&clock, 60);
    for _ in 0..3 {
        assert_eq!(login(&a, "bea", "not-her-pass").await.status, 401);
    }
    assert_eq!(lock_of(&a, &c, "bea").await["round"], 2);
    let r = post(
        &a,
        "/api/users/password",
        Some(&c),
        &json!({"id": id, "password": "fresh-start-1"}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let bea = r.json()["users"]
        .as_array()
        .expect("users")
        .iter()
        .find(|u| u["id"] == id)
        .cloned()
        .expect("bea");
    assert_eq!(
        bea["lock"],
        Value::Null,
        "the answer carries the cleared state"
    );
    assert_eq!(login(&a, "bea", "fresh-start-1").await.status, 200);
}

#[tokio::test]
async fn a_wrong_current_password_follows_the_rounds_with_a_wait() {
    let (a, clock) = app_with_moving_clock();
    let c = admin(&a).await;
    let body = json!({"current": "wrong-pass-1", "password": "something-new"}).to_string();
    for _ in 0..5 {
        assert_eq!(
            post(&a, "/api/auth/password", Some(&c), &body).await.status,
            403
        );
    }
    assert_wait(&post(&a, "/api/auth/password", Some(&c), &body).await, 60);
    advance(&clock, 60);
    for _ in 0..3 {
        assert_eq!(
            post(&a, "/api/auth/password", Some(&c), &body).await.status,
            403
        );
    }
    assert_wait(&post(&a, "/api/auth/password", Some(&c), &body).await, 300);
    // the same name: a sign-in from an unknown browser waits as well
    assert_wait(&login(&a, "admin", ADMIN_PW).await, 300);
}

#[tokio::test]
async fn unlock_clears_the_lock_for_admins_only() {
    let (a, clock) = app_with_moving_clock();
    let c = admin(&a).await;
    let (bea, bc) = user(&a, &c, "bea", "user").await;
    for _ in 0..5 {
        assert_eq!(login(&a, "bea", "not-her-pass").await.status, 401);
    }
    advance(&clock, 60);
    for _ in 0..3 {
        assert_eq!(login(&a, "bea", "not-her-pass").await.status, 401);
    }
    assert_wait(&login(&a, "bea", "bea-own-pass").await, 300);
    let body = json!({"id": bea}).to_string();
    // the route needs a session, and a plain user cannot unlock anyone (not even themselves)
    let r = post(&a, "/api/users/unlock", None, &body).await;
    assert_eq!(r.status, 401);
    let r = post(&a, "/api/users/unlock", Some(&bc), &body).await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (403, MSG_ADMIN_ONLY.to_string())
    );
    assert_eq!(lock_of(&a, &c, "bea").await["round"], 2);
    // unknown or malformed ids
    let r = post(&a, "/api/users/unlock", Some(&c), r#"{"id": 999}"#).await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (404, MSG_USER_NOT_FOUND.to_string())
    );
    let r = post(&a, "/api/users/unlock", Some(&c), r#"{"id": "1"}"#).await;
    assert_eq!(r.status, 400);
    let r = post(&a, "/api/users/unlock", Some(&c), "{}").await;
    assert_eq!(r.status, 400);
    // the administrator unlocks: back to round 1 with five chances, and the answer is the fresh list
    let r = post(&a, "/api/users/unlock", Some(&c), &body).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let users = r.json()["users"].as_array().cloned().expect("users");
    assert_eq!(users.len(), 2);
    assert!(users.iter().all(|u| u["lock"].is_null()), "{users:?}");
    for _ in 0..4 {
        assert_eq!(login(&a, "bea", "not-her-pass").await.status, 401);
    }
    assert_eq!(login(&a, "bea", "bea-own-pass").await.status, 200);
    // the administrator's own account: locked for unknown browsers while the admin is signed in, and unlocked by themselves
    for _ in 0..5 {
        assert_eq!(login(&a, "admin", "a-wrong-guess").await.status, 401);
    }
    assert_eq!(lock_of(&a, &c, "admin").await["retryAfter"], 60);
    let r = post(&a, "/api/users/unlock", Some(&c), r#"{"id": 1}"#).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(lock_of(&a, &c, "admin").await, Value::Null);
    assert_eq!(login(&a, "admin", ADMIN_PW).await.status, 200);
    // unlocking a user who is not locked is fine
    let r = post(&a, "/api/users/unlock", Some(&c), &body).await;
    assert_eq!(r.status, 200, "{}", r.text());
}

#[tokio::test]
async fn guesses_from_elsewhere_escalate_without_locking_the_known_browser() {
    let (a, clock) = app_with_moving_clock();
    let c = admin(&a).await;
    let r = login(&a, "admin", ADMIN_PW).await;
    let device = set_cookie_pair(&r, auth::DEVICE_COOKIE).expect("device cookie");
    let known_login = |password: &str| {
        let body = json!({"username": "admin", "password": password}).to_string();
        let device = device.clone();
        let a = &a;
        async move {
            call(
                a,
                "POST",
                "/api/auth/login",
                Some(&device),
                Some(ORIGIN),
                Some(&body),
            )
            .await
        }
    };
    // somebody elsewhere goes through rounds 1-3
    for _ in 0..5 {
        assert_eq!(login(&a, "admin", "a-wrong-guess").await.status, 401);
    }
    advance(&clock, 60);
    for _ in 0..3 {
        assert_eq!(login(&a, "admin", "a-wrong-guess").await.status, 401);
    }
    advance(&clock, 300);
    for _ in 0..3 {
        assert_eq!(login(&a, "admin", "a-wrong-guess").await.status, 401);
    }
    assert_wait(&login(&a, "admin", ADMIN_PW).await, 900);
    // the owner's known browser signs in all along, and its success does not start the guessed bucket over
    assert_eq!(known_login(ADMIN_PW).await.status, 200);
    assert_wait(&login(&a, "admin", ADMIN_PW).await, 900);
    assert_eq!(lock_of(&a, &c, "admin").await["round"], 3);
    // wrong passwords typed in the known browser follow the same rounds, in its own bucket
    for _ in 0..5 {
        assert_eq!(known_login("a-typo-of-mine").await.status, 401);
    }
    assert_wait(&known_login(ADMIN_PW).await, 60);
    // the list shows the lock that holds the user back most (the 15 minutes of the unknown browsers)
    let lock = lock_of(&a, &c, "admin").await;
    assert_eq!(
        (&lock["round"], &lock["retryAfter"]),
        (&json!(3), &json!(900))
    );
    // ... and says that the known browsers are locked at the same time (v0.6.5 re-review: the page used to say they still worked)
    assert_eq!(
        (
            &lock["knownBrowser"],
            &lock["otherRetryAfter"],
            lock["otherUntil"].as_str().map(str::len)
        ),
        (&json!(false), &json!(60), Some("2026-11-11 14:32:05".len()))
    );
    // Unlock clears both buckets
    let r = post(&a, "/api/users/unlock", Some(&c), r#"{"id": 1}"#).await;
    assert_eq!(r.status, 200);
    assert_eq!(known_login(ADMIN_PW).await.status, 200);
    assert_eq!(login(&a, "admin", ADMIN_PW).await.status, 200);
}

// ---------------------------------------------------------------- v0.6.5 fixes

const MSG_EXAM_QUESTIONS_GONE: &str = "这场考试的题目已经被删除，成绩没有保存。";

/// v0.6.5 review: the lock belongs to the account (its id), not to the typed name. A rename keeps a running lock where the
/// administrator can see and lift it; a new account that takes a deleted or renamed account's old name starts fresh.
#[tokio::test]
async fn the_lock_follows_the_account_through_a_rename_and_ends_with_its_deletion() {
    let (a, clock) = app_with_moving_clock();
    let c = admin(&a).await;
    let (carol, _) = user(&a, &c, "carol", "user").await;
    for _ in 0..5 {
        assert_eq!(login(&a, "carol", "wrong-pass!").await.status, 401);
    }
    let r = post(
        &a,
        "/api/users/update",
        Some(&c),
        &json!({"id": carol, "username": "carol2"}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    // the renamed row shows the running lock, and the account stays locked under its new name
    let lock = lock_of(&a, &c, "carol2").await;
    assert_eq!(
        (&lock["retryAfter"], &lock["round"], &lock["knownBrowser"]),
        (&json!(60), &json!(1), &json!(false))
    );
    assert_wait(&login(&a, "carol2", "carol-own-pass").await, 60);
    // the old name has no account any more: it is a fresh name of its own
    assert_eq!(login(&a, "carol", "carol-own-pass").await.status, 401);
    // after the lock: round 2 with three tries; Unlock (by id) also clears a state that is only past round 1
    advance(&clock, 60);
    let lock = lock_of(&a, &c, "carol2").await;
    assert_eq!(
        (&lock["retryAfter"], &lock["round"], &lock["triesLeft"]),
        (&json!(0), &json!(2), &json!(3))
    );
    let r = post(
        &a,
        "/api/users/unlock",
        Some(&c),
        &json!({"id": carol}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(lock_of(&a, &c, "carol2").await, Value::Null);
    // a new account under the name of a deleted, locked one starts at round 1 without a failure of its own
    let (alice, _) = user(&a, &c, "alice", "user").await;
    for _ in 0..5 {
        assert_eq!(login(&a, "alice", "wrong-pass!").await.status, 401);
    }
    let r = post(
        &a,
        "/api/users/delete",
        Some(&c),
        &json!({"id": alice}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let (alice2, _) = user(&a, &c, "alice", "user").await;
    assert_ne!(alice2, alice, "ids are never reused");
    assert_eq!(lock_of(&a, &c, "alice").await, Value::Null);
    // the same for a name that was guessed before any account had it
    for _ in 0..5 {
        assert_eq!(login(&a, "dora", "wrong-pass!").await.status, 401);
    }
    assert_eq!(login(&a, "dora", "wrong-pass!").await.status, 429);
    let (_, _) = user(&a, &c, "dora", "user").await;
    assert_eq!(lock_of(&a, &c, "dora").await, Value::Null);
}

/// v0.6.5 review: changing your own password ends every session of the account, the current one included; this browser
/// continues with a new session cookie, so a copied cookie stops working.
#[tokio::test]
async fn changing_your_own_password_rotates_the_session() {
    let a = app();
    let c = admin(&a).await;
    let (eve, uc) = user(&a, &c, "eve", "user").await;
    let copy = uc.clone();
    let r = post(
        &a,
        "/api/auth/password",
        Some(&uc),
        &json!({"current": "eve-own-pass", "password": "eve-new-pass"}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let fresh = cookie_of(&r);
    assert!(fresh.starts_with("xexam_session="), "{fresh}");
    assert_ne!(fresh, uc);
    assert_eq!(get(&a, "/api/records", Some(&copy)).await.status, 401);
    assert_eq!(get(&a, "/api/records", Some(&fresh)).await.status, 200);
    assert_eq!(
        count(
            &raw(&a.db),
            &format!("SELECT COUNT(*) FROM sessions WHERE user_id={eve}")
        ),
        1
    );
}

/// Known gap (a) of v0.6: an exam started before its bank was deleted used to be graded and stored with questions that no
/// longer exist. Now only the questions that still exist count; when none is left nothing is saved (409).
#[tokio::test]
async fn an_exam_counts_only_questions_that_still_exist() {
    let a = app();
    let c = admin(&a).await;
    let (uid, uc) = user(&a, &c, "eve", "user").await;
    let s = &a.store;
    let (sh, qs) = (
        vec![dbc::sheet("Cooking", 2)],
        vec![
            dbc::single("Cooking", "k1", "Water boils at?", 1),
            dbc::single("Cooking", "k2", "Salt or sugar?", 2),
        ],
    );
    s.apply_import("legacy.xlsx", &sh, &qs)
        .expect("legacy bank");
    let (_, bank2) = s
        .apply_import_for(ImportTarget::New, "second.xlsx", &sh, &qs)
        .expect("bank 2")
        .expect("created");
    let ids2: Vec<String> = s.bank().expect("bank")["questions"]
        .as_array()
        .expect("q")
        .iter()
        .filter(|q| q["bank"] == bank2)
        .map(|q| q["id"].as_str().expect("id").to_string())
        .collect();
    // the exam was started (the page holds these questions), then the bank is deleted
    let r = post(
        &a,
        "/api/banks/delete",
        Some(&c),
        &json!({"id": bank2, "withRecords": true}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = post(
        &a,
        "/api/attempt",
        Some(&uc),
        &attempt_body(&[(ids2[0].as_str(), false), (ids2[1].as_str(), true)]),
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (409, MSG_EXAM_QUESTIONS_GONE.to_string())
    );
    let db = raw(&a.db);
    for table in ["attempts", "user_stats"] {
        assert_eq!(
            count(
                &db,
                &format!("SELECT COUNT(*) FROM {table} WHERE user_id={uid}")
            ),
            0,
            "{table}"
        );
    }
    assert_eq!(count(&db, "SELECT COUNT(*) FROM attempt_items"), 0);
    // made-up ids are refused the same way
    let r = post(
        &a,
        "/api/attempt",
        Some(&uc),
        &attempt_body(&[("made-up-id", false)]),
    )
    .await;
    assert_eq!(r.status, 409);
    // a mixed exam: only the question that still exists counts, and the answer names the others
    let r = post(
        &a,
        "/api/attempt",
        Some(&uc),
        &attempt_body_with_sheets(
            &[
                (ids2[0].as_str(), false),
                ("k1", true),
                ("made-up-id", false),
            ],
            &["Deleted sheet", "Cooking", "Cooking"],
        ),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let p = r.json();
    assert_eq!(p["notCounted"], json!([ids2[0], "made-up-id"]));
    assert_eq!(
        p["questions"]
            .as_object()
            .expect("q")
            .keys()
            .collect::<Vec<_>>(),
        ["k1"]
    );
    let rec = &get(&a, "/api/records", Some(&uc)).await.json()["records"][0];
    assert_eq!(
        (
            &rec["total"],
            &rec["correct"],
            &rec["percent"],
            &rec["banks"][0]["id"]
        ),
        (&json!(1), &json!(1), &json!(100), &json!(1))
    );
    // the record keeps only the sheets of the counted questions (v0.6.5 re-review: the deleted bank's sheet used to stay)
    assert_eq!(rec["sheets"], json!(["Cooking"]));
    assert_eq!(count(&db, "SELECT COUNT(*) FROM attempt_items"), 1);
    // a client that sent no matching sheet name: the record names the sheets of the counted questions
    let r = post(
        &a,
        "/api/attempt",
        Some(&uc),
        &attempt_body_with_sheets(&[("k2", true), ("made-up-id", false)], &["Elsewhere"]),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let rec = &get(&a, "/api/records", Some(&uc)).await.json()["records"][0];
    assert_eq!(rec["sheets"], json!(["Cooking"]));
    // the progress of /api/progress never carries "notCounted"
    assert!(get(&a, "/api/progress", Some(&uc))
        .await
        .json()
        .get("notCounted")
        .is_none());
    // a retired question (left out of a re-import) still exists and is counted
    s.apply_import("legacy.xlsx", &sh, &qs[1..])
        .expect("retire k1");
    let r = post(
        &a,
        "/api/attempt",
        Some(&uc),
        &attempt_body(&[("k1", false)]),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(r.json().get("notCounted").is_none());
    // nothing left out: the sheets are stored as sent
    let r = post(
        &a,
        "/api/attempt",
        Some(&uc),
        &attempt_body_with_sheets(&[("k2", true)], &["Cooking", "Elsewhere"]),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let rec = &get(&a, "/api/records", Some(&uc)).await.json()["records"][0];
    assert_eq!(rec["sheets"], json!(["Cooking", "Elsewhere"]));
}
