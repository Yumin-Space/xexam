//! v2.7: an administrator reads another user's study records (`GET /api/users/records`, `GET /api/users/record`): the answers,
//! who may ask, the refused queries and the activity entry. Real router, real engine and database; made-up data only.
#![allow(dead_code, unused_imports)] // the helper block is shared in shape with v06 and not all of it is used here

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
const MSG_BAD_LOGIN: &str = "邮箱或密码不对。";
const MSG_TOO_MANY_LOGINS: &str = "登录失败次数太多，请稍后再试。";
const MSG_PASSWORD_CHANGE_REQUIRED: &str = "请先设置新密码。";
const MSG_SAME_PASSWORD: &str = "新密码不能和原来的一样。";
const MSG_BAD_USERNAME: &str = "邮箱不对：1–40 个字符，只能用字母、数字和 . _ - @。";
const MSG_BAD_PASSWORD: &str = "密码需要 8–128 个字符。";
const MSG_USERNAME_TAKEN: &str = "这个邮箱已经有人用了。";
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
        ai_link: Default::default(),
        activity: Default::default(),
        gates: Default::default(),
        push: Default::default(),
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
        &json!({ "password": ADMIN_PW, "displayName": "Admin" }).to_string(),
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
        &json!({ "password": format!("{name}-own-pass"), "displayName": format!("{name} display") }).to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    (id, cookie_of(&r))
}

fn attempt_body(ids: &[(&str, bool)]) -> String {
    let results: Vec<Value> = ids
        .iter()
        .map(|(id, c)| json!({"id": id, "correct": c, "answer": ["A"]}))
        .collect();
    json!({"results": results, "sheets": ["Cooking"], "mode": "all", "seconds": 12}).to_string()
}

type Row = (String, Option<String>, Option<String>, Option<String>);

fn activity_rows(a: &App) -> Vec<Row> {
    raw(&a.db)
        .prepare("SELECT action, username, object, detail FROM activity_log WHERE action='users.viewRecords' ORDER BY id")
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

#[tokio::test]
async fn an_administrator_reads_another_users_records() {
    let a = app();
    seed_banked(&a.db, &["q1", "q2", "q3"]);
    let c = admin(&a).await;
    let (uid, uc) = user(&a, &c, "cook", "user").await;
    let (oid, oc) = user(&a, &c, "baker", "user").await;
    let r = post(
        &a,
        "/api/attempt",
        Some(&uc),
        &attempt_body(&[("q1", false), ("q2", true)]),
    )
    .await;
    assert_eq!(r.status, 200);
    post(
        &a,
        "/api/attempt",
        Some(&oc),
        &attempt_body(&[("q3", true)]),
    )
    .await;
    let own = get(&a, "/api/records", Some(&uc)).await.json();
    let rid = own["records"][0]["id"].as_i64().expect("id");
    let other_rid = get(&a, "/api/records", Some(&oc)).await.json()["records"][0]["id"]
        .as_i64()
        .expect("id");

    // the list: the user's own answer plus the header
    let r = get(&a, &format!("/api/users/records?user={uid}"), Some(&c)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let j = r.json();
    assert_eq!(j["records"], own["records"]);
    assert_eq!(j["retention"], own["retention"]);
    assert_eq!(j["user"]["id"], uid);
    assert_eq!(j["user"]["username"], "cook");
    assert_eq!(j["user"]["displayName"], "cook display");

    // the detail: like /api/record, with the header, without the AI keys
    let r = get(
        &a,
        &format!("/api/users/record?user={uid}&id={rid}"),
        Some(&c),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let j = r.json();
    let mut want = get(&a, &format!("/api/record?id={rid}"), Some(&uc))
        .await
        .json();
    // the AI keys sit inside "record"; the own route still shows them, the admin route must not
    assert!(want["record"].get("sessionKey").is_some());
    want["record"]
        .as_object_mut()
        .expect("obj")
        .remove("sessionKey");
    want["record"]
        .as_object_mut()
        .expect("obj")
        .remove("startedAt");
    let mut got = j.clone();
    assert_eq!(
        got.as_object_mut()
            .expect("obj")
            .remove("user")
            .expect("user")["id"],
        uid
    );
    assert_eq!(got, want);
    assert!(j.get("sessionKey").is_none() && j.get("startedAt").is_none());
    assert!(!r.text().contains("sessionKey") && !r.text().contains("startedAt"));

    // a record of another user under this uid, and a missing one: 404 alike
    for id in [other_rid, 999_999] {
        let r = get(
            &a,
            &format!("/api/users/record?user={uid}&id={id}"),
            Some(&c),
        )
        .await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (404, MSG_RECORD_NOT_FOUND.to_string())
        );
    }
    // another administrator's records, and the own ones, work too
    let own_admin = get(&a, "/api/records", Some(&c)).await.json();
    assert_eq!(own_admin["records"].as_array().map(Vec::len), Some(0));
    assert_eq!(
        get(&a, "/api/users/records?user=1", Some(&c)).await.status,
        200
    );
    let _ = oid;

    // the own-only routes did not change
    let r = get(&a, &format!("/api/record?id={other_rid}"), Some(&c)).await;
    assert_eq!(r.status, 404);
}

#[tokio::test]
async fn the_records_routes_refuse_plain_users_and_bad_queries() {
    let a = app();
    seed_banked(&a.db, &["q1"]);
    let c = admin(&a).await;
    let (uid, uc) = user(&a, &c, "cook", "user").await;
    for p in [
        format!("/api/users/records?user={uid}"),
        format!("/api/users/record?user={uid}&id=1"),
    ] {
        let r = get(&a, &p, Some(&uc)).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (403, MSG_ADMIN_ONLY.to_string())
        );
        assert_eq!(get(&a, &p, None).await.status, 401);
    }
    for p in [
        "/api/users/records",
        "/api/users/records?user=",
        "/api/users/records?user=abc",
        "/api/users/records?user=1&user=2",
        "/api/users/record",
        "/api/users/record?user=1",
        "/api/users/record?id=1",
        "/api/users/record?user=1&id=x",
    ] {
        let r = get(&a, p, Some(&c)).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (400, MSG_BAD_REQUEST.to_string()),
            "{p}"
        );
    }
    for p in [
        "/api/users/records?user=99999",
        "/api/users/record?user=99999&id=1",
    ] {
        let r = get(&a, p, Some(&c)).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (404, MSG_USER_NOT_FOUND.to_string()),
            "{p}"
        );
    }
}

#[tokio::test]
async fn viewing_records_is_written_to_the_activity_log() {
    let a = app();
    a.state.0.activity.start(a.store.clone());
    seed_banked(&a.db, &["q1"]);
    let c = admin(&a).await;
    let (uid, uc) = user(&a, &c, "cook", "user").await;
    post(
        &a,
        "/api/attempt",
        Some(&uc),
        &attempt_body(&[("q1", true)]),
    )
    .await;
    let rid = get(&a, "/api/records", Some(&uc)).await.json()["records"][0]["id"]
        .as_i64()
        .expect("id");
    assert_eq!(
        get(&a, &format!("/api/users/records?user={uid}"), Some(&c))
            .await
            .status,
        200
    );
    assert_eq!(
        get(
            &a,
            &format!("/api/users/record?user={uid}&id={rid}"),
            Some(&c)
        )
        .await
        .status,
        200
    );
    // a later page of the list is the same visit: nothing more is written
    for q in ["offset=10", "page=2"] {
        assert_eq!(
            get(&a, &format!("/api/users/records?user={uid}&{q}"), Some(&c))
                .await
                .status,
            200
        );
    }
    // the first page again is logged
    assert_eq!(
        get(
            &a,
            &format!("/api/users/records?user={uid}&offset=0"),
            Some(&c)
        )
        .await
        .status,
        200
    );
    // HEAD and refused requests write nothing
    let r = call(
        &a,
        "HEAD",
        &format!("/api/users/records?user={uid}"),
        Some(&c),
        None,
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(get(&a, "/api/users/records", Some(&c)).await.status, 400);
    assert_eq!(
        get(&a, &format!("/api/users/records?user={uid}"), Some(&uc))
            .await
            .status,
        403
    );
    a.state.0.activity.flush().await;
    let rows = activity_rows(&a);
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!(rows[0].1.as_deref(), Some("admin"));
    assert_eq!(rows[0].2.as_deref(), Some("cook"));
    assert_eq!(rows[0].3, None);
    let d: Value = serde_json::from_str(rows[1].3.as_deref().expect("detail")).expect("json");
    assert_eq!(d, json!({"record": rid}));
}
