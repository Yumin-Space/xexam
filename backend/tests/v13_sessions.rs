//! v1.3: sign-in lifetime and the activity facts of the auth layer — the three session rules (administrator / "keep me signed
//! in" / browser session) with their cookies, idle and hard expiry, role changes, the once-a-minute touch gates, the cookie
//! re-send, the AI auth step, the `auth.*` activity entries and the sign-in IP / location switch.
//!
//! Real router, real engine, real database; users are created straight in the store (made-up names) and sign in through the
//! API. Only made-up data is used.
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
use std::sync::{Arc, OnceLock};
use xexam_backend::api::auth::purge_ended_sessions;
use xexam_backend::api::internal::InternalToken;
use xexam_backend::api::{build_router, AppState};
use xexam_backend::auth::{self, Role};
use xexam_backend::database::{Store, UserChange, ACTIVITY_USER_VISITS};
use xexam_backend::services::RealEngine;

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";
const NGINX_HOST: &str = "backend:8765";
const TOKEN: &str = "fake-internal-token-for-tests-0123456789";
const PW: &str = "kitchen-pass-1";
const EXPIRED_MSG: &str = "登录已过期，请重新登录。";
const SIGN_IN_MSG: &str = "请先登录。";
const DAY: i64 = 86_400;

struct App {
    router: Router,
    store: Arc<Store>,
    state: AppState,
    db: PathBuf,
    _dir: tempfile::TempDir,
}

/// The Argon2 hash of [`PW`], made once (it is slow by design).
fn pw_hash() -> &'static str {
    static H: OnceLock<String> = OnceLock::new();
    H.get_or_init(|| auth::hash_password(PW).expect("hash"))
}

fn raw(p: &Path) -> Connection {
    Connection::open(p).expect("raw open")
}

/// A fresh app: the administrator `admin` and the users `kim` and `lee` all have the password [`PW`] and no pending change.
fn app() -> App {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let store = Arc::new(Store::open(&db).expect("store"));
    let engine = Arc::new(RealEngine::new(store.clone()));
    let state = AppState::new_with_logs(
        apic::default_config(),
        store.clone(),
        engine,
        None,
        Default::default(),
    );
    assert!(state
        .0
        .ai_link
        .set_token(InternalToken::new(TOKEN).expect("token")));
    for name in ["kim", "lee"] {
        match store
            .create_user(name, pw_hash(), Role::User)
            .expect("user")
        {
            UserChange::Done(_) => {}
            other => panic!("{other:?}"),
        }
    }
    raw(&db)
        .execute(
            "UPDATE users SET password_hash = ?1, must_change_password = 0",
            [pw_hash()],
        )
        .expect("prime");
    App {
        router: build_router(state.clone()),
        store,
        state,
        db,
        _dir: dir,
    }
}

async fn call(
    a: &App,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    origin: Option<&str>,
    extra: &[(&str, &str)],
    body: Option<String>,
) -> Resp {
    let mut hs: Vec<(&str, Vec<u8>)> = Vec::new();
    if let Some(c) = cookie {
        hs.push(("cookie", c.as_bytes().to_vec()));
    }
    if let Some(o) = origin {
        hs.push(("origin", o.as_bytes().to_vec()));
    }
    for (k, v) in extra {
        hs.push((k, v.as_bytes().to_vec()));
    }
    let body = match body {
        Some(b) => {
            hs.push(("content-type", b"application/json".to_vec()));
            hs.push(("content-length", b.len().to_string().into_bytes()));
            Body::from(b)
        }
        None => Body::empty(),
    };
    let hrefs: Vec<(&str, &[u8])> = hs.iter().map(|(k, v)| (*k, v.as_slice())).collect();
    send(&a.router, build(method, path, Some(HOST), &hrefs, body)).await
}

async fn get(a: &App, path: &str, cookie: &str) -> Resp {
    call(a, "GET", path, Some(cookie), None, &[], None).await
}

async fn post(a: &App, path: &str, cookie: &str, body: &str) -> Resp {
    call(
        a,
        "POST",
        path,
        Some(cookie),
        Some(ORIGIN),
        &[],
        Some(body.to_string()),
    )
    .await
}

async fn login_body(a: &App, body: Value, extra: &[(&str, &str)]) -> Resp {
    call(
        a,
        "POST",
        "/api/auth/login",
        None,
        Some(ORIGIN),
        extra,
        Some(body.to_string()),
    )
    .await
}

async fn login(a: &App, name: &str) -> Resp {
    login_body(a, json!({"username": name, "password": PW}), &[]).await
}

/// `xexam_session=<token>` of a sign-in answer.
fn cookie_of(r: &Resp) -> String {
    let sc = r.header("set-cookie").expect("set-cookie");
    sc.split(';').next().expect("pair").to_string()
}

fn hash_of(cookie: &str) -> String {
    auth::token_hash(cookie.trim_start_matches("xexam_session="))
}

async fn signed_in(a: &App, name: &str) -> String {
    let r = login(a, name).await;
    assert_eq!(r.status, 200, "{}", r.text());
    cookie_of(&r)
}

/// `(remember, started_at, last_seen_at, expires_at)` of the session.
fn row(a: &App, cookie: &str) -> (i64, i64, i64, i64) {
    raw(&a.db)
        .query_row(
            "SELECT remember, started_at, last_seen_at, expires_at FROM sessions WHERE token_hash = ?1",
            [hash_of(cookie)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("session row")
}

fn set_times(a: &App, cookie: &str, started: i64, last: i64) {
    raw(&a.db)
        .execute(
            "UPDATE sessions SET started_at = ?1, last_seen_at = ?2 WHERE token_hash = ?3",
            rusqlite::params![started, last, hash_of(cookie)],
        )
        .expect("set times");
}

fn session_count(a: &App, cookie: &str) -> i64 {
    raw(&a.db)
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE token_hash = ?1",
            [hash_of(cookie)],
            |r| r.get(0),
        )
        .expect("count")
}

fn now() -> i64 {
    auth::unix_now()
}

/// The activity rows of `action` (flushed first): `(username, result, reason, ip, ua, detail, ts)`.
type Entry = (
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
);

async fn entries(a: &App, action: &str) -> Vec<Entry> {
    a.state.0.activity.flush().await;
    let c = raw(&a.db);
    let mut st = c
        .prepare(
            "SELECT username, result, reason, ip, ua, detail, ts FROM activity_log WHERE action = ?1 ORDER BY id",
        )
        .expect("prepare");
    let rows = st
        .query_map([action], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })
        .expect("query");
    rows.map(|r| r.expect("row")).collect()
}

fn cookie_attrs(r: &Resp) -> Vec<String> {
    r.header("set-cookie")
        .expect("set-cookie")
        .split("; ")
        .skip(1)
        .map(str::to_string)
        .collect()
}

// ---------------------------------------------------------------- sign-in and the three rules

#[tokio::test]
async fn login_follows_the_three_rules_and_validates_remember() {
    let a = app();
    // a user, `remember` missing = true: 30 days, sliding
    let r = login(&a, "kim").await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(
        r.json()["session"],
        json!({"rule": "remember", "idleSecs": 2_592_000, "hardEndsAt": null})
    );
    assert_eq!(
        cookie_attrs(&r),
        ["HttpOnly", "SameSite=Strict", "Path=/", "Max-Age=2592000"]
    );
    let (remember, started, last, expires) = row(&a, &cookie_of(&r));
    assert_eq!((remember, started == last), (1, true));
    assert_eq!(expires, last + 2_592_000);
    // explicit true
    let r = login_body(
        &a,
        json!({"username": "kim", "password": PW, "remember": true}),
        &[],
    )
    .await;
    assert_eq!(r.json()["session"]["rule"], "remember");
    // a user without the box: a browser-session cookie (no Max-Age, no Expires), 12 h
    let r = login_body(
        &a,
        json!({"username": "kim", "password": PW, "remember": false}),
        &[],
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(
        r.json()["session"],
        json!({"rule": "browser", "idleSecs": 43_200, "hardEndsAt": null})
    );
    assert_eq!(cookie_attrs(&r), ["HttpOnly", "SameSite=Strict", "Path=/"]);
    let (remember, _, last, expires) = row(&a, &cookie_of(&r));
    assert_eq!((remember, expires), (0, last + 43_200));
    // an administrator follows the admin rule whatever the box says (and the box is stored as sent)
    for remember in [Some(false), Some(true), None] {
        let mut body = json!({"username": "admin", "password": PW});
        if let Some(b) = remember {
            body["remember"] = json!(b);
        }
        let r = login_body(&a, body, &[]).await;
        assert_eq!(r.status, 200, "{}", r.text());
        let s = &r.json()["session"];
        assert_eq!(
            (s["rule"].as_str(), s["idleSecs"].as_i64()),
            (Some("admin"), Some(86_400))
        );
        assert!(s["hardEndsAt"].is_string(), "{s}");
        assert_eq!(
            cookie_attrs(&r),
            ["HttpOnly", "SameSite=Strict", "Path=/", "Max-Age=604800"]
        );
        let (stored, _, last, expires) = row(&a, &cookie_of(&r));
        assert_eq!(stored, i64::from(remember.unwrap_or(true)));
        assert_eq!(expires, last + 86_400);
    }
    // anything but a boolean is a bad request
    for bad in [json!("yes"), json!(1), json!(null), json!([true])] {
        let r = login_body(
            &a,
            json!({"username": "kim", "password": PW, "remember": bad}),
            &[],
        )
        .await;
        assert_eq!(r.status, 400, "{bad}: {}", r.text());
    }
}

#[tokio::test]
async fn state_carries_the_session_and_the_visit_switch() {
    let a = app();
    let ac = signed_in(&a, "admin").await;
    let kc = signed_in(&a, "kim").await;
    let v = get(&a, "/api/auth/state", &ac).await.json();
    assert_eq!(v["session"]["rule"], "admin");
    assert_eq!(v["logVisits"], true, "an administrator is always reported");
    let v = get(&a, "/api/auth/state", &kc).await.json();
    assert_eq!(v["session"]["rule"], "remember");
    assert_eq!(v["logVisits"], false);
    a.store
        .set_system_setting(ACTIVITY_USER_VISITS, "1", None)
        .expect("switch");
    a.state.0.activity.reload_settings(&a.store);
    assert_eq!(
        get(&a, "/api/auth/state", &kc).await.json()["logVisits"],
        true
    );
}

// ---------------------------------------------------------------- expiry

#[tokio::test]
async fn an_idle_browser_session_ends_after_12_hours_and_is_recorded_once() {
    let a = app();
    let c = signed_in_with(&a, "kim", false).await;
    assert_eq!(get(&a, "/api/bank", &c).await.status, 200);
    set_times(&a, &c, now() - 20 * 3600, now() - 13 * 3600);
    a.state.0.gates.forget(&hash_of(&c));
    let r = get(&a, "/api/bank", &c).await;
    assert_eq!(r.status, 401);
    assert_eq!(r.error(), EXPIRED_MSG);
    assert_eq!(r.json()["code"], "auth.expired");
    assert_eq!(session_count(&a, &c), 0, "the row is deleted");
    // the next request has no session at all: the plain answer, and no second entry
    let r = get(&a, "/api/bank", &c).await;
    assert_eq!((r.status.as_u16(), r.error().as_str()), (401, SIGN_IN_MSG));
    let rows = entries(&a, "auth.expired").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let (user, result, reason, _, _, detail, ts) = &rows[0];
    assert_eq!(user.as_deref(), Some("kim"));
    assert_eq!((result.as_str(), reason.as_deref()), ("fail", Some("idle")));
    assert_eq!(detail.as_deref(), Some(r#"{"rule":"browser"}"#));
    // the entry time is the moment the session ended (last seen + 12 h)
    assert!((*ts - (now() - 13 * 3600 + 43_200)).abs() <= 5, "{ts}");
}

async fn signed_in_with(a: &App, name: &str, remember: bool) -> String {
    let r = login_body(
        a,
        json!({"username": name, "password": PW, "remember": remember}),
        &[],
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    cookie_of(&r)
}

#[tokio::test]
async fn an_admin_session_ends_7_days_after_sign_in_however_active() {
    let a = app();
    let c = signed_in(&a, "admin").await;
    // active a minute ago, signed in 7 days and a minute ago
    set_times(&a, &c, now() - 7 * DAY - 60, now() - 60);
    a.state.0.gates.forget(&hash_of(&c));
    let r = get(&a, "/api/bank", &c).await;
    assert_eq!((r.status.as_u16(), r.error().as_str()), (401, EXPIRED_MSG));
    let rows = entries(&a, "auth.expired").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].2.as_deref(), Some("hard"));
    assert_eq!(rows[0].5.as_deref(), Some(r#"{"rule":"admin"}"#));
    // still inside both limits: fine, and the touch keeps the hard end
    let c = signed_in(&a, "admin").await;
    set_times(&a, &c, now() - 6 * DAY, now() - 3600);
    a.state.0.gates.forget(&hash_of(&c));
    assert_eq!(get(&a, "/api/bank", &c).await.status, 200);
    let (_, started, last, expires) = row(&a, &c);
    assert!(last >= now() - 5);
    assert_eq!(expires, started + 7 * DAY, "the hard limit comes first");
    // an administrator idle for more than 24 h
    let c = signed_in(&a, "admin").await;
    set_times(&a, &c, now() - 2 * DAY, now() - 25 * 3600);
    a.state.0.gates.forget(&hash_of(&c));
    assert_eq!(get(&a, "/api/bank", &c).await.status, 401);
    assert_eq!(
        entries(&a, "auth.expired").await[1].2.as_deref(),
        Some("idle")
    );
}

#[tokio::test]
async fn a_remember_user_lives_30_days_sliding() {
    let a = app();
    let c = signed_in(&a, "kim").await;
    set_times(&a, &c, now() - 100 * DAY, now() - 29 * DAY);
    a.state.0.gates.forget(&hash_of(&c));
    assert_eq!(
        get(&a, "/api/bank", &c).await.status,
        200,
        "no hard limit for users"
    );
    set_times(&a, &c, now() - 100 * DAY, now() - 31 * DAY);
    a.state.0.gates.forget(&hash_of(&c));
    assert_eq!(get(&a, "/api/bank", &c).await.status, 401);
}

#[tokio::test]
async fn a_role_change_applies_at_the_next_request() {
    let a = app();
    // a remember user idle for 2 days is fine, but promoted to administrator (24 h idle) the session has ended
    let c = signed_in(&a, "kim").await;
    set_times(&a, &c, now() - 3 * DAY, now() - 2 * DAY);
    a.state.0.gates.forget(&hash_of(&c));
    assert_eq!(get(&a, "/api/auth/state", &c).await.status, 200);
    raw(&a.db)
        .execute("UPDATE users SET role = 'admin' WHERE username = 'kim'", [])
        .expect("promote");
    let r = get(&a, "/api/auth/state", &c).await;
    assert_eq!((r.status.as_u16(), r.error().as_str()), (401, EXPIRED_MSG));
    // a demoted administrator gets the user rule with the stored box: browser session, 13 h idle = ended
    let c = signed_in_with(&a, "lee", false).await;
    raw(&a.db)
        .execute("UPDATE users SET role = 'admin' WHERE username = 'lee'", [])
        .expect("promote");
    set_times(&a, &c, now() - DAY, now() - 13 * 3600);
    a.state.0.gates.forget(&hash_of(&c));
    assert_eq!(
        get(&a, "/api/bank", &c).await.status,
        200,
        "an administrator: 24 h"
    );
    raw(&a.db)
        .execute("UPDATE users SET role = 'user' WHERE username = 'lee'", [])
        .expect("demote");
    a.state.0.gates.forget(&hash_of(&c));
    set_times(&a, &c, now() - DAY, now() - 13 * 3600);
    assert_eq!(get(&a, "/api/bank", &c).await.status, 401);
}

#[tokio::test]
async fn the_purge_records_each_ended_session_once_at_its_deadline() {
    let a = app();
    let kim = a.store.user_by_name("kim").expect("q").expect("kim");
    let t = now();
    {
        let c = raw(&a.db);
        // ended 10 days ago (remember: 30 days after the last activity 40 days ago)
        c.execute(
            "INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at, started_at, remember, ip, ua) \
             VALUES('purged', ?1, 't', ?2, ?3, ?4, 1, '203.0.113.9', 'Made-up UA')",
            rusqlite::params![kim.id, t - 40 * DAY, t - 10 * DAY, t - 50 * DAY],
        )
        .expect("old session");
        // still valid
        c.execute(
            "INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at, started_at, remember) \
             VALUES('alive', ?1, 't', ?2, ?3, ?2, 1)",
            rusqlite::params![kim.id, t, t + 30 * DAY],
        )
        .expect("live session");
    }
    let state = a.state.clone();
    let n = tokio::task::spawn_blocking(move || purge_ended_sessions(&state, t).expect("purge"))
        .await
        .expect("join");
    assert_eq!(n, 1);
    let rows = entries(&a, "auth.expired").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let (user, _, reason, ip, ua, detail, ts) = &rows[0];
    assert_eq!(user.as_deref(), Some("kim"));
    assert_eq!(reason.as_deref(), Some("idle"));
    assert_eq!(
        ts,
        &(t - 40 * DAY + 30 * DAY),
        "the entry time is the deadline"
    );
    assert_eq!(detail.as_deref(), Some(r#"{"rule":"remember"}"#));
    assert_eq!(
        ip.as_deref(),
        None,
        "the eye is closed: the entry records no address (the stored one stays hidden in the session)"
    );
    assert_eq!(
        ua.as_deref(),
        Some("Made-up UA"),
        "the device is always kept"
    );
    let sys = entries(&a, "system.sessionsPurged").await;
    assert_eq!(sys.len(), 1);
    assert_eq!(sys[0].5.as_deref(), Some(r#"{"count":1}"#));
    // a second purge finds nothing and records nothing
    let state = a.state.clone();
    let n = tokio::task::spawn_blocking(move || purge_ended_sessions(&state, t).expect("purge"))
        .await
        .expect("join");
    assert_eq!(n, 0);
    assert_eq!(entries(&a, "auth.expired").await.len(), 1);
    // a session the lookup ended is not logged again by the purge
    let c = signed_in(&a, "lee").await;
    set_times(&a, &c, now() - 40 * DAY, now() - 31 * DAY);
    a.state.0.gates.forget(&hash_of(&c));
    assert_eq!(get(&a, "/api/bank", &c).await.status, 401);
    let state = a.state.clone();
    let n = tokio::task::spawn_blocking(move || {
        purge_ended_sessions(&state, now() + 1).expect("purge")
    })
    .await
    .expect("join");
    assert_eq!(n, 0);
    assert_eq!(entries(&a, "auth.expired").await.len(), 2);
}

// ---------------------------------------------------------------- touch gates

#[tokio::test]
async fn a_request_touches_at_most_once_a_minute_and_a_poll_never() {
    let a = app();
    let c = signed_in(&a, "kim").await;
    let h = hash_of(&c);
    let old = now() - 5_000;
    set_times(&a, &c, old - 100, old);
    // the sign-in just touched it: 100 requests write nothing
    for _ in 0..100 {
        assert_eq!(get(&a, "/api/bank", &c).await.status, 200);
    }
    assert_eq!(row(&a, &c).2, old, "within the minute: read only");
    // the gate lets one request through, the rest of the minute stays read-only
    a.state.0.gates.forget(&h);
    assert_eq!(get(&a, "/api/bank", &c).await.status, 200);
    let (_, _, last, expires) = row(&a, &c);
    assert!(last >= now() - 5 && expires == last + 2_592_000);
    set_times(&a, &c, old - 100, old);
    for _ in 0..20 {
        get(&a, "/api/bank", &c).await;
    }
    assert_eq!(row(&a, &c).2, old);
    // routes with `activity: false` authenticate but never touch
    a.state.0.gates.forget(&h);
    for path in ["/api/auth/state", "/api/info"] {
        assert_eq!(get(&a, path, &c).await.status, 200, "{path}");
    }
    assert_eq!(row(&a, &c).2, old, "polls do not keep a session alive");
    let ac = signed_in(&a, "admin").await;
    set_times(&a, &ac, old - 100, old);
    a.state.0.gates.forget(&hash_of(&ac));
    for path in ["/api/logs", "/api/activity", "/api/activity/settings"] {
        assert_eq!(get(&a, path, &ac).await.status, 200, "{path}");
    }
    assert_eq!(row(&a, &ac).2, old);
    assert_eq!(get(&a, "/api/users", &ac).await.status, 200);
    assert!(row(&a, &ac).2 >= now() - 5, "an ordinary route touches");
}

#[tokio::test]
async fn last_active_is_written_once_a_minute_per_user_even_with_two_browsers() {
    let a = app();
    let c1 = signed_in(&a, "kim").await;
    let c2 = signed_in(&a, "kim").await;
    let sentinel = "2001-01-01 00:00:00";
    let set = |a: &App| {
        raw(&a.db)
            .execute(
                "UPDATE users SET last_active_at = ?1 WHERE username = 'kim'",
                [sentinel],
            )
            .expect("sentinel");
    };
    let active = |a: &App| -> String {
        raw(&a.db)
            .query_row(
                "SELECT last_active_at FROM users WHERE username = 'kim'",
                [],
                |r| r.get(0),
            )
            .expect("active")
    };
    // the sign-in set it; nothing writes it again within the minute, whichever browser asks
    set(&a);
    for c in [&c1, &c2, &c1, &c2] {
        assert_eq!(get(&a, "/api/bank", c).await.status, 200);
    }
    assert_eq!(active(&a), sentinel);
    // a new minute (the user's gate remembers an older one): both sessions are due, the user is written once
    let kim = a.store.user_by_name("kim").expect("q").expect("kim").id;
    for c in [&c1, &c2] {
        a.state.0.gates.forget(&hash_of(c));
    }
    a.state.0.gates.plan("older-minute", kim, now() - 120);
    assert_eq!(get(&a, "/api/bank", &c1).await.status, 200);
    assert_ne!(
        active(&a),
        sentinel,
        "the first request of the minute writes it"
    );
    set(&a);
    assert_eq!(get(&a, "/api/bank", &c2).await.status, 200);
    assert!(
        row(&a, &c2).2 >= now() - 5,
        "the other browser's session was written"
    );
    assert_eq!(active(&a), sentinel, "but the user only once a minute");
}

#[tokio::test]
async fn the_ai_auth_step_touches_the_session_but_sets_no_cookie() {
    let a = app();
    let c = signed_in(&a, "kim").await;
    let old = now() - 5_000;
    set_times(&a, &c, old - 100, old);
    a.state.0.gates.forget(&hash_of(&c));
    let step = |a: &App, cookie: String| {
        let router = a.router.clone();
        async move {
            let hs: Vec<(&str, &[u8])> = vec![
                ("cookie", cookie.as_bytes()),
                ("x-original-method", b"POST"),
                ("x-original-host", HOST.as_bytes()),
                ("x-original-origin", ORIGIN.as_bytes()),
            ];
            send(
                &router,
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
    };
    let r = step(&a, c.clone()).await;
    assert_eq!(r.status, 204, "{}", r.text());
    assert!(r.header("set-cookie").is_none());
    assert!(
        row(&a, &c).2 >= now() - 5,
        "somebody who only chats is active"
    );
    // an ended session is refused like no session, and recorded
    set_times(&a, &c, now() - 40 * DAY, now() - 31 * DAY);
    a.state.0.gates.forget(&hash_of(&c));
    let r = step(&a, c.clone()).await;
    assert_eq!(r.status, 401);
    assert_eq!(r.header("x-xexam-deny").as_deref(), Some("signin"));
    assert_eq!(session_count(&a, &c), 0);
    assert_eq!(entries(&a, "auth.expired").await.len(), 1);
    // the helper for the internal routes follows the same rule
    let c = signed_in(&a, "lee").await;
    let h = hash_of(&c);
    let state = a.state.clone();
    let hh = h.clone();
    let user = xexam_backend::api::auth::session_user_checked(&state, &hh)
        .await
        .expect("lookup");
    assert_eq!(user.map(|u| u.username), Some("lee".to_string()));
    set_times(&a, &c, now() - 40 * DAY, now() - 31 * DAY);
    state.0.gates.forget(&h);
    let user = xexam_backend::api::auth::session_user_checked(&state, &h)
        .await
        .expect("lookup");
    assert!(user.is_none());
    assert_eq!(entries(&a, "auth.expired").await.len(), 2);
}

// ---------------------------------------------------------------- cookies

#[tokio::test]
async fn only_a_remember_session_gets_its_cookie_again_and_at_most_hourly() {
    let a = app();
    let kc = signed_in(&a, "kim").await;
    let bc = signed_in_with(&a, "lee", false).await;
    let ac = signed_in(&a, "admin").await;
    // the sign-in just sent it: not again
    assert!(post(&a, "/api/settings", &kc, "{}")
        .await
        .header("set-cookie")
        .is_none());
    // an hour later (the gate forgotten = an old entry): a POST with an Origin re-sends it once
    a.state.0.gates.forget(&hash_of(&kc));
    let r = post(&a, "/api/settings", &kc, "{}").await;
    assert_eq!(r.status, 200, "{}", r.text());
    let sc = r.header("set-cookie").expect("re-sent");
    assert!(
        sc.starts_with(&format!("{kc};")) && sc.contains("; Max-Age=2592000"),
        "{sc}"
    );
    assert!(post(&a, "/api/settings", &kc, "{}")
        .await
        .header("set-cookie")
        .is_none());
    // a GET without an Origin never re-sends (the scheme is unknown)
    a.state.0.gates.forget(&hash_of(&kc));
    assert!(get(&a, "/api/bank", &kc)
        .await
        .header("set-cookie")
        .is_none());
    // a poll that is not activity does not either
    assert!(get(&a, "/api/auth/state", &kc)
        .await
        .header("set-cookie")
        .is_none());
    // browser-session and administrator cookies are never re-sent
    for c in [&bc, &ac] {
        a.state.0.gates.forget(&hash_of(c));
        let r = post(&a, "/api/settings", c, "{}").await;
        assert_eq!(r.status, 200, "{}", r.text());
        assert!(r.header("set-cookie").is_none());
    }
}

#[tokio::test]
async fn a_password_change_keeps_the_rule_of_the_session() {
    let a = app();
    for (name, remember, want) in [
        (
            "admin",
            true,
            vec!["HttpOnly", "SameSite=Strict", "Path=/", "Max-Age=604800"],
        ),
        (
            "kim",
            true,
            vec!["HttpOnly", "SameSite=Strict", "Path=/", "Max-Age=2592000"],
        ),
        ("lee", false, vec!["HttpOnly", "SameSite=Strict", "Path=/"]),
    ] {
        let c = signed_in_with(&a, name, remember).await;
        let r = post(
            &a,
            "/api/auth/password",
            &c,
            &json!({"current": PW, "password": format!("{name}-new-pass-1")}).to_string(),
        )
        .await;
        assert_eq!(r.status, 200, "{name}: {}", r.text());
        assert_eq!(cookie_attrs(&r), want, "{name}");
        let fresh = cookie_of(&r);
        assert_ne!(fresh, c);
        assert_eq!(
            get(&a, "/api/bank", &c).await.status,
            401,
            "the old cookie is dead"
        );
        let r = get(&a, "/api/bank", &fresh).await;
        assert_eq!(r.status, 200);
        // the stored rule survived: a browser session is still a browser session (no 30-day cookie is sent later)
        let stored: i64 = raw(&a.db)
            .query_row(
                "SELECT remember FROM sessions WHERE token_hash=?1",
                [hash_of(&fresh)],
                |row| row.get(0),
            )
            .expect("row");
        assert_eq!(stored != 0, remember, "{name}: stored remember");
        if !remember {
            assert!(
                r.header("set-cookie").is_none(),
                "{name}: no re-sent cookie"
            );
        }
    }
}

// ---------------------------------------------------------------- activity entries

#[tokio::test]
async fn sign_in_sign_out_and_failures_are_recorded() {
    let a = app();
    let ua = ("user-agent", "Made-up Browser 1.0");
    // ok
    let r = login_body(&a, json!({"username": "kim", "password": PW}), &[ua]).await;
    let c = cookie_of(&r);
    let rows = entries(&a, "auth.signin").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0.as_deref(), Some("kim"));
    assert_eq!((rows[0].1.as_str(), rows[0].2.as_deref()), ("ok", None));
    assert_eq!(rows[0].4.as_deref(), Some("Made-up Browser 1.0"));
    assert_eq!(rows[0].5.as_deref(), Some(r#"{"rule":"remember"}"#));
    // wrong password: the account as the actor
    let r = login_body(
        &a,
        json!({"username": "kim", "password": "wrong-pass-1"}),
        &[],
    )
    .await;
    assert_eq!(r.status, 401);
    // a disabled account with the right password
    raw(&a.db)
        .execute("UPDATE users SET disabled = 1 WHERE username = 'lee'", [])
        .expect("disable");
    let r = login(&a, "lee").await;
    assert_eq!(r.status, 401);
    // an unknown name: anonymous, and the typed name is nowhere
    let typed = "marker-name-xyz";
    let r = login_body(
        &a,
        json!({"username": typed, "password": "whatever-pass-1"}),
        &[],
    )
    .await;
    assert_eq!(r.status, 401);
    let rows = entries(&a, "auth.signin").await;
    let fails: Vec<(Option<&str>, Option<&str>)> = rows[1..]
        .iter()
        .map(|r| (r.0.as_deref(), r.2.as_deref()))
        .collect();
    assert_eq!(
        fails,
        [
            (Some("kim"), Some("badPassword")),
            (Some("lee"), Some("disabled")),
            (None, Some("unknownUser")),
        ]
    );
    assert!(rows[1..].iter().all(|r| r.1 == "fail"));
    let all: Vec<String> = raw(&a.db)
        .prepare("SELECT COALESCE(username,'') || COALESCE(object,'') || COALESCE(detail,'') || COALESCE(reason,'') FROM activity_log")
        .expect("p")
        .query_map([], |r| r.get(0))
        .expect("q")
        .map(|r| r.expect("r"))
        .collect();
    assert!(
        all.iter().all(|s| !s.contains(typed)),
        "the typed name is never stored"
    );
    // sign-out
    let r = post(&a, "/api/auth/logout", &c, "{}").await;
    assert_eq!(r.status, 200);
    let rows = entries(&a, "auth.signout").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0.as_deref(), Some("kim"));
    assert_eq!(session_count(&a, &c), 0);
}

#[tokio::test]
async fn a_lock_and_a_refused_attempt_while_locked_are_recorded() {
    let a = app();
    // five wrong passwords start the first lock (round 1)
    for _ in 0..5 {
        let r = login_body(
            &a,
            json!({"username": "kim", "password": "wrong-pass-1"}),
            &[],
        )
        .await;
        assert_eq!(r.status, 401);
    }
    let r = login(&a, "kim").await;
    assert_eq!(r.status, 429, "{}", r.text());
    let locks = entries(&a, "auth.lock").await;
    assert_eq!(locks.len(), 1, "{locks:?}");
    let detail: Value = serde_json::from_str(locks[0].5.as_deref().expect("detail")).expect("json");
    assert_eq!(detail, json!({"round": 1, "minutes": 1}));
    assert_eq!(locks[0].0, None, "a system row");
    let rows = entries(&a, "auth.signin").await;
    assert_eq!(rows.last().and_then(|r| r.2.as_deref()), Some("locked"));
    assert_eq!(rows.last().and_then(|r| r.0.as_deref()), Some("kim"));
}

// ---------------------------------------------------------------- sign-in info (IP and location)

const GEO: [(&str, &str); 5] = [
    ("x-real-ip", "203.0.113.7"),
    ("x-geo-country", "ZZ"),
    ("x-geo-region", "Example Region"),
    ("x-geo-city", "Example City"),
    ("user-agent", "Made-up Browser 2.0"),
];

fn user_info(
    a: &App,
    name: &str,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    raw(&a.db)
        .query_row(
            "SELECT last_login_ip, last_login_country, last_login_region, last_login_city FROM users WHERE username = ?1",
            [name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("user")
}

fn session_ip_ua(a: &App, cookie: &str) -> (Option<String>, Option<String>) {
    raw(&a.db)
        .query_row(
            "SELECT ip, ua FROM sessions WHERE token_hash = ?1",
            [hash_of(cookie)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("session")
}

#[tokio::test]
async fn the_sign_in_address_and_location_are_stored_only_while_the_eye_is_open() {
    let a = app();
    let full = (
        Some("203.0.113.7".to_string()),
        Some("ZZ".to_string()),
        Some("Example Region".to_string()),
        Some("Example City".to_string()),
    );
    let none = (None, None, None, None);
    // closed (the default): no address and no location is stored anywhere; the device still is
    let r = login_body(&a, json!({"username": "kim", "password": PW}), &GEO).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(user_info(&a, "kim"), none);
    let (ip, ua) = session_ip_ua(&a, &cookie_of(&r));
    assert_eq!(
        (ip.as_deref(), ua.as_deref()),
        (None, Some("Made-up Browser 2.0"))
    );
    let rows = entries(&a, "auth.signin").await;
    assert_eq!(
        (rows[0].3.as_deref(), rows[0].4.as_deref()),
        (None, Some("Made-up Browser 2.0"))
    );
    let v = a.store.users().expect("users");
    assert_eq!(v["signinInfo"], false);
    // open: recording starts; the sign-in made while closed simply has none
    a.store.set_signin_info(true, Some(1)).expect("on");
    a.state.0.activity.reload_settings(&a.store);
    let r = login_body(&a, json!({"username": "kim", "password": PW}), &GEO).await;
    assert_eq!(user_info(&a, "kim"), full);
    assert_eq!(
        session_ip_ua(&a, &cookie_of(&r)).0.as_deref(),
        Some("203.0.113.7")
    );
    let rows = entries(&a, "auth.signin").await;
    assert_eq!(rows[0].3, None);
    assert_eq!(
        rows.last().and_then(|r| r.3.as_deref()),
        Some("203.0.113.7")
    );
    // closed again: the old values stay (hidden), a new sign-in does not overwrite them, the times still move
    let (at_before,): (Option<String>,) = raw(&a.db)
        .query_row(
            "SELECT last_login_at FROM users WHERE username='kim'",
            [],
            |r| Ok((r.get(0)?,)),
        )
        .expect("at");
    a.store.set_signin_info(false, Some(1)).expect("off");
    a.state.0.activity.reload_settings(&a.store);
    let other = [("x-real-ip", "198.51.100.9"), ("x-geo-country", "YY")];
    let r = login_body(&a, json!({"username": "kim", "password": PW}), &other).await;
    assert_eq!(r.status, 200);
    assert_eq!(user_info(&a, "kim"), full);
    assert_eq!(session_ip_ua(&a, &cookie_of(&r)).0, None);
    let (at_after,): (Option<String>,) = raw(&a.db)
        .query_row(
            "SELECT last_login_at FROM users WHERE username='kim'",
            [],
            |r| Ok((r.get(0)?,)),
        )
        .expect("at");
    assert!(at_after.is_some() && at_after >= at_before);
    let v = a.store.users().expect("users");
    assert!(v["users"]
        .as_array()
        .expect("users")
        .iter()
        .all(|u| u["lastLoginIp"].is_null() && u["lastLoginLocation"].is_null()));
    let rows = entries(&a, "auth.signin").await;
    assert_eq!(rows.last().and_then(|r| r.3.as_deref()), None);
    // clear works while closed, too
    a.store.clear_signin_info().expect("clear");
    assert_eq!(user_info(&a, "kim"), none);
    // reopened: the history shows again
    a.store.set_signin_info(true, Some(1)).expect("on");
    a.state.0.activity.reload_settings(&a.store);
    let r = login_body(&a, json!({"username": "kim", "password": PW}), &GEO).await;
    assert_eq!(r.status, 200);
    assert_eq!(user_info(&a, "kim"), full);
    // a sign-in without the location headers does not keep the old location (the record never mixes two sign-ins)
    let r = login_body(
        &a,
        json!({"username": "kim", "password": PW}),
        &[("x-real-ip", "2001:db8::1")],
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(
        user_info(&a, "kim"),
        (Some("2001:db8::1".into()), None, None, None)
    );
    // garbage in the address header is no address
    let r = login_body(
        &a,
        json!({"username": "kim", "password": PW}),
        &[("x-real-ip", "not-an-address"), ("x-geo-country", "T1")],
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(user_info(&a, "kim"), (None, None, None, None));
}

#[tokio::test]
async fn reopening_the_eye_returns_the_old_stored_values_after_sign_ins_made_while_closed() {
    let a = app();
    let full = (
        Some("203.0.113.7".to_string()),
        Some("ZZ".to_string()),
        Some("Example Region".to_string()),
        Some("Example City".to_string()),
    );
    // open: the sign-in stores the address and the location
    a.store.set_signin_info(true, Some(1)).expect("on");
    a.state.0.activity.reload_settings(&a.store);
    let r = login_body(&a, json!({"username": "kim", "password": PW}), &GEO).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(user_info(&a, "kim"), full);
    // closed: a new sign-in from elsewhere neither overwrites the stored values nor shows them
    a.store.set_signin_info(false, Some(1)).expect("off");
    a.state.0.activity.reload_settings(&a.store);
    let other = [("x-real-ip", "198.51.100.9"), ("x-geo-country", "YY")];
    let r = login_body(&a, json!({"username": "kim", "password": PW}), &other).await;
    assert_eq!(r.status, 200);
    assert_eq!(user_info(&a, "kim"), full);
    let v = a.store.users().expect("users");
    assert!(v["users"]
        .as_array()
        .expect("users")
        .iter()
        .all(|u| u["lastLoginIp"].is_null() && u["lastLoginLocation"].is_null()));
    // reopened without another sign-in: the OLD values are returned again
    a.store.set_signin_info(true, Some(1)).expect("on");
    a.state.0.activity.reload_settings(&a.store);
    assert_eq!(user_info(&a, "kim"), full);
    let v = a.store.users().expect("users");
    let kim = v["users"]
        .as_array()
        .expect("users")
        .iter()
        .find(|u| u["username"] == "kim")
        .expect("kim")
        .clone();
    assert_eq!(kim["lastLoginIp"], "203.0.113.7");
    assert!(!kim["lastLoginLocation"].is_null());
}

#[tokio::test]
async fn a_lan_peer_without_the_proxy_flag_gets_its_own_address_stored() {
    let a = app();
    a.store.set_signin_info(true, Some(1)).expect("on");
    a.state.0.activity.reload_settings(&a.store);
    let body = json!({"username": "kim", "password": PW}).to_string();
    let sign_in = |peer: &str, headers: &[(&str, &[u8])]| {
        let mut hs: Vec<(&str, &[u8])> = vec![
            ("origin", ORIGIN.as_bytes()),
            ("content-type", b"application/json"),
        ];
        hs.extend_from_slice(headers);
        let mut req = build(
            "POST",
            "/api/auth/login",
            Some(HOST),
            &hs,
            Body::from(body.clone()),
        );
        let addr: std::net::SocketAddr = peer.parse().expect("peer");
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(addr));
        req
    };
    // a LAN client that forges the headers: its own address is stored, no location
    let r = send(
        &a.router,
        sign_in(
            "192.0.2.50:40000",
            &[("x-real-ip", b"198.51.100.9"), ("x-geo-country", b"ZZ")],
        ),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(
        user_info(&a, "kim"),
        (Some("192.0.2.50".into()), None, None, None)
    );
    // a loopback peer (a local reverse proxy) is believed
    let r = send(
        &a.router,
        sign_in(
            "127.0.0.1:40001",
            &[("x-real-ip", b"203.0.113.7"), ("x-geo-country", b"ZZ")],
        ),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(
        user_info(&a, "kim"),
        (Some("203.0.113.7".into()), Some("ZZ".into()), None, None)
    );
}
