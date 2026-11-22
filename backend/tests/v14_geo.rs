//! v1.4: where the place of a sign-in address comes from, and the last-activity facts of a user.
//!
//! Main database schema 9 (`users.last_active_ip / _country / _region / _city / _ua`), the place source (`geo.source`:
//! Cloudflare request headers or the external lookup that ai-service runs over the internal link), the three internal routes
//! (`/internal/ai/geo/next`, `/result`, `/source`) and the rules around them: the source rule, one lookup per new address at a
//! sign-in or at the once-a-minute activity write, nothing while the eye is closed, a cleared record never gets a late
//! result. The provider side is simulated by calling the internal routes in process: no test contacts any other host.
//! Made-up users and places only; the two public addresses used are the well-known resolver addresses (never looked up).
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
use std::time::Duration;
use xexam_backend::api::internal::InternalToken;
use xexam_backend::api::{build_router, AppState};
use xexam_backend::auth::{self, Role};
use xexam_backend::database::{ActivityFilter, FixedClock, Place, Store, UserChange};
use xexam_backend::geo::Target;
use xexam_backend::services::RealEngine;

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";
const NGINX_HOST: &str = "backend:8765";
const TOKEN: &str = "fake-internal-token-for-tests-0123456789";
const PW: &str = "kitchen-pass-1";
const IP_A: &str = "8.8.8.8";
const IP_B: &str = "1.1.1.1";

struct App {
    router: Router,
    store: Arc<Store>,
    state: AppState,
    db: PathBuf,
    _dir: tempfile::TempDir,
}

fn pw_hash() -> &'static str {
    static H: OnceLock<String> = OnceLock::new();
    H.get_or_init(|| auth::hash_password(PW).expect("hash"))
}

fn raw(p: &Path) -> Connection {
    Connection::open(p).expect("raw open")
}

/// A fresh app: the administrator `admin` and the users `kim` and `lee` all have the password [`PW`].
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
            "UPDATE users SET password_hash = ?1, must_change_password = 0, display_name = 'User ' || id",
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
    extra: &[(&str, &str)],
    body: Option<String>,
) -> Resp {
    let mut hs: Vec<(&str, Vec<u8>)> = Vec::new();
    if let Some(c) = cookie {
        hs.push(("cookie", c.as_bytes().to_vec()));
    }
    if method == "POST" {
        hs.push(("origin", ORIGIN.as_bytes().to_vec()));
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

fn visitor<'a>(ip: &'a str, ua: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("x-real-ip", ip),
        ("x-geo-country", "ZZ"),
        ("x-geo-region", "Cloud Region"),
        ("x-geo-city", "Cloud City"),
        ("user-agent", ua),
    ]
}

const UA_PHONE: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 Version/17.0 Mobile/15E148 Safari/604.1";
const UA_PC: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120.0 Safari/537.36 Edg/120.0";

async fn login_from(a: &App, name: &str, ip: &str, ua: &str) -> Resp {
    call(
        a,
        "POST",
        "/api/auth/login",
        None,
        &visitor(ip, ua),
        Some(json!({"username": name, "password": PW}).to_string()),
    )
    .await
}

fn cookie_of(r: &Resp) -> String {
    let sc = r.header("set-cookie").expect("set-cookie");
    sc.split(';').next().expect("pair").to_string()
}

async fn admin_cookie(a: &App) -> String {
    let r = login_from(a, "admin", "203.0.113.1", "Admin Browser").await;
    // the default administrator starts with the password "admin": give it PW and no pending change
    if r.status != 200 {
        raw(&a.db)
            .execute(
                "UPDATE users SET password_hash = ?1, must_change_password = 0, display_name = 'Admin' WHERE username = 'admin'",
                [pw_hash()],
            )
            .expect("admin");
        let r = login_from(a, "admin", "203.0.113.1", "Admin Browser").await;
        assert_eq!(r.status, 200, "{}", r.text());
        return cookie_of(&r);
    }
    cookie_of(&r)
}

async fn post(a: &App, path: &str, cookie: &str, body: Value) -> Resp {
    call(a, "POST", path, Some(cookie), &[], Some(body.to_string())).await
}

async fn get(a: &App, path: &str, cookie: &str) -> Resp {
    call(a, "GET", path, Some(cookie), &[], None).await
}

async fn internal(a: &App, path: &str, body: &Value, token: bool) -> Resp {
    let text = body.to_string();
    let len = text.len().to_string();
    let mut hs: Vec<(&str, &[u8])> = vec![
        ("content-type", b"application/json"),
        ("content-length", len.as_bytes()),
    ];
    if token {
        hs.push(("x-xexam-internal-token", TOKEN.as_bytes()));
    }
    send(
        &a.router,
        build("POST", path, Some(NGINX_HOST), &hs, Body::from(text)),
    )
    .await
}

async fn set_source(a: &App, source: &str) {
    let r = internal(
        a,
        "/internal/ai/geo/source",
        &json!({"source": source}),
        true,
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["source"], source);
}

async fn eye(a: &App, adm: &str, open: bool) {
    let r = post(a, "/api/users/signin-info", adm, json!({"enabled": open})).await;
    assert_eq!(r.status, 200, "{}", r.text());
}

async fn next_jobs(a: &App, wait_ms: u64) -> Vec<(String, String)> {
    let r = internal(
        a,
        "/internal/ai/geo/next",
        &json!({"waitMs": wait_ms}),
        true,
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()["lookups"]
        .as_array()
        .expect("lookups")
        .iter()
        .map(|j| {
            (
                j["id"].as_str().expect("id").to_string(),
                j["ip"].as_str().expect("ip").to_string(),
            )
        })
        .collect()
}

fn place_ok(id: &str, city: &str) -> Value {
    json!({"id": id, "ok": true, "place": {
        "country": "Example Country", "countryCode": "zz", "region": "Example Region", "city": city}})
}

async fn send_results(a: &App, results: Vec<Value>) -> Resp {
    internal(
        a,
        "/internal/ai/geo/result",
        &json!({"results": results}),
        true,
    )
    .await
}

/// The user row of `name` in the users answer.
async fn user_row(a: &App, adm: &str, name: &str) -> Value {
    let r = get(a, "/api/users", adm).await;
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()["users"]
        .as_array()
        .expect("users")
        .iter()
        .find(|u| u["username"] == name)
        .cloned()
        .expect("row")
}

async fn signin_row(a: &App, name: &str) -> (Option<String>, Option<String>, Option<String>) {
    a.state.0.activity.flush().await;
    raw(&a.db)
        .query_row(
            "SELECT ip, country, city FROM activity_log WHERE action = 'auth.signin' AND username = ?1 ORDER BY id DESC LIMIT 1",
            [name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("sign-in row")
}

/// Makes the once-a-minute last-active gate of `name` claimable again.
fn reopen_gate(a: &App, name: &str) {
    let id = a
        .store
        .user_by_name(name)
        .expect("lookup")
        .expect("user")
        .id;
    a.state
        .0
        .gates
        .plan("older-minute", id, auth::unix_now() - 120);
}

fn here() -> Place {
    Place {
        country: Some("ZZ".into()),
        region: Some("Example Region".into()),
        city: Some("Example City".into()),
    }
}

// ---------------------------------------------------------------- the source rule

#[tokio::test]
async fn cloudflare_mode_keeps_the_header_place_and_queues_nothing() {
    let a = app();
    let adm = admin_cookie(&a).await;
    eye(&a, &adm, true).await;
    let r = login_from(&a, "kim", IP_A, UA_PHONE).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let kim = user_row(&a, &adm, "kim").await;
    assert_eq!(kim["lastLoginIp"], IP_A);
    assert_eq!(kim["lastLoginLocation"]["city"], "Cloud City");
    assert_eq!(kim["lastLoginLocation"]["country"], "ZZ");
    assert_eq!(kim["lastActiveIp"], IP_A);
    assert_eq!(kim["lastActiveLocation"]["city"], "Cloud City");
    assert_eq!(kim["lastActiveUa"], UA_PHONE);
    let row = signin_row(&a, "kim").await;
    assert_eq!(
        row,
        (
            Some(IP_A.to_string()),
            Some("ZZ".to_string()),
            Some("Cloud City".to_string())
        )
    );
    assert_eq!(a.state.0.activity.geo().queue_len(), 0);
    // a later request's own headers move the last-active place
    let c = cookie_of(&r);
    reopen_gate(&a, "kim");
    let r = call(
        &a,
        "GET",
        "/api/banks",
        Some(&c),
        &[
            ("x-real-ip", IP_B),
            ("x-geo-city", "Other City"),
            ("x-geo-country", "YY"),
            ("user-agent", UA_PC),
        ],
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    let kim = user_row(&a, &adm, "kim").await;
    assert_eq!(kim["lastActiveIp"], IP_B);
    assert_eq!(kim["lastActiveLocation"]["city"], "Other City");
    assert_eq!(kim["lastActiveUa"], UA_PC);
    assert_eq!(
        kim["lastLoginLocation"]["city"], "Cloud City",
        "the sign-in place is not touched"
    );
    assert_eq!(kim["lastLoginIp"], IP_A);
}

#[tokio::test]
async fn api_mode_ignores_the_header_place_and_asks_once_per_new_address() {
    let a = app();
    let adm = admin_cookie(&a).await;
    eye(&a, &adm, true).await;
    set_source(&a, "api").await;
    let users = get(&a, "/api/users", &adm).await.json();
    assert_eq!(users["geoSource"], "api");
    let r = login_from(&a, "kim", IP_A, UA_PHONE).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let kim = user_row(&a, &adm, "kim").await;
    assert_eq!(kim["lastLoginIp"], IP_A);
    assert!(kim["lastLoginLocation"].is_null(), "{kim}");
    assert!(kim["lastActiveLocation"].is_null());
    let (ip, country, city) = signin_row(&a, "kim").await;
    assert_eq!((ip.as_deref(), country, city), (Some(IP_A), None, None));
    // one job, with two targets (the sign-in and the activity)
    assert_eq!(a.state.0.activity.geo().queue_len(), 1);
    // a second sign-in from the same address while the job waits adds nothing
    let r2 = login_from(&a, "lee", IP_A, UA_PC).await;
    assert_eq!(r2.status, 200);
    assert_eq!(a.state.0.activity.geo().queue_len(), 1);
    // a failed sign-in never queues
    let bad = call(
        &a,
        "POST",
        "/api/auth/login",
        None,
        &visitor(IP_B, UA_PC),
        Some(json!({"username": "kim", "password": "wrong-password"}).to_string()),
    )
    .await;
    assert_eq!(bad.status, 401);
    assert_eq!(a.state.0.activity.geo().queue_len(), 1);
    // a reserved address is never queued
    let r3 = login_from(&a, "kim", "203.0.113.50", UA_PC).await;
    assert_eq!(r3.status, 200);
    assert_eq!(a.state.0.activity.geo().queue_len(), 1);
}

#[tokio::test]
async fn a_cache_hit_fills_the_place_at_once_and_queues_nothing() {
    let a = app();
    let adm = admin_cookie(&a).await;
    eye(&a, &adm, true).await;
    set_source(&a, "api").await;
    let ip: std::net::IpAddr = IP_A.parse().expect("ip");
    a.state
        .0
        .activity
        .geo()
        .cache_place(ip, here(), auth::unix_now());
    let r = login_from(&a, "kim", IP_A, UA_PHONE).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let kim = user_row(&a, &adm, "kim").await;
    assert_eq!(kim["lastLoginLocation"]["city"], "Example City");
    assert_eq!(kim["lastActiveLocation"]["city"], "Example City");
    assert_eq!(a.state.0.activity.geo().queue_len(), 0);
    let (_, country, city) = signin_row(&a, "kim").await;
    assert_eq!(
        (country.as_deref(), city.as_deref()),
        (Some("ZZ"), Some("Example City"))
    );
    // an AI event row reads the cache too, and never starts a lookup
    let ev = json!({"events": [{
        "id": "00000000000000000000000000000001", "at": auth::unix_now(), "userId": 1, "stamp": "x",
        "username": "admin", "action": "ai.settings.limits", "object": null, "detail": {"keys": ["messagesPerChat"]},
        "result": "ok", "reason": null, "ip": IP_A, "ua": "Agent/1.0"}]});
    let r = internal(&a, "/internal/ai/activity", &ev, true).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let c = raw(&a.db);
    let (country, city): (Option<String>, Option<String>) = c
        .query_row(
            "SELECT country, city FROM activity_log WHERE source = 'ai'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("ai row");
    assert_eq!(
        (country.as_deref(), city.as_deref()),
        (Some("ZZ"), Some("Example City"))
    );
    assert_eq!(a.state.0.activity.geo().queue_len(), 0);
}

#[tokio::test]
async fn switching_back_to_cloudflare_forgets_the_queue_and_uses_the_headers_again() {
    let a = app();
    let adm = admin_cookie(&a).await;
    eye(&a, &adm, true).await;
    set_source(&a, "api").await;
    login_from(&a, "kim", IP_A, UA_PHONE).await;
    assert_eq!(a.state.0.activity.geo().queue_len(), 1);
    let r = post(
        &a,
        "/api/users/geo-source",
        &adm,
        json!({"source": "cloudflare"}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["geoSource"], "cloudflare");
    assert_eq!(a.state.0.activity.geo().queue_len(), 0);
    assert!(next_jobs(&a, 0).await.is_empty());
    login_from(&a, "lee", IP_B, UA_PC).await;
    let lee = user_row(&a, &adm, "lee").await;
    assert_eq!(lee["lastLoginLocation"]["city"], "Cloud City");
    // the entry of the route
    a.state.0.activity.flush().await;
    let page = a
        .store
        .activity_page(&ActivityFilter {
            action: Some("users.geoSource".to_string()),
            ..Default::default()
        })
        .expect("page");
    assert_eq!(page.entries.len(), 1);
    assert_eq!(
        page.entries[0].detail,
        Some(json!({"source": "cloudflare"}))
    );
}

#[tokio::test]
async fn the_geo_source_route_takes_cloudflare_only_and_only_from_an_administrator() {
    let a = app();
    let adm = admin_cookie(&a).await;
    for bad in [
        json!({"source": "api"}),
        json!({"source": "other"}),
        json!({"source": 1}),
        json!({}),
        json!({"source": "cloudflare", "extra": 1}),
    ] {
        let r = post(&a, "/api/users/geo-source", &adm, bad.clone()).await;
        assert_eq!(r.status, 400, "{bad}");
    }
    let kim = cookie_of(&login_from(&a, "kim", IP_A, UA_PHONE).await);
    let r = post(
        &a,
        "/api/users/geo-source",
        &kim,
        json!({"source": "cloudflare"}),
    )
    .await;
    assert_eq!(r.status, 403);
    let r = post(
        &a,
        "/api/users/geo-source",
        &adm,
        json!({"source": "cloudflare"}),
    )
    .await;
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(v["geoSource"], "cloudflare");
    // no heartbeat in this test: ai-service is not connected
    assert_eq!(v["aiServiceUp"], false);
    let keys: Vec<&str> = v
        .as_object()
        .expect("o")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "users",
            "signinInfo",
            "aiAvailable",
            "geoSource",
            "aiServiceUp"
        ]
    );
    let u = &v["users"][0];
    for k in ["lastActiveIp", "lastActiveLocation", "lastActiveUa"] {
        assert!(u.get(k).is_some(), "{k}");
    }
}

// ---------------------------------------------------------------- the long poll

#[tokio::test]
async fn next_answers_at_once_waits_and_wakes() {
    let a = app();
    let adm = admin_cookie(&a).await;
    // Cloudflare mode: nothing is handed out
    assert!(next_jobs(&a, 0).await.is_empty());
    eye(&a, &adm, true).await;
    set_source(&a, "api").await;
    // nothing waiting: empty after the wait
    let t = std::time::Instant::now();
    assert!(next_jobs(&a, 150).await.is_empty());
    assert!(t.elapsed() >= Duration::from_millis(120));
    // a sign-in wakes a waiting poll
    let waiter = {
        let router = a.router.clone();
        tokio::spawn(async move {
            let body = json!({"waitMs": 5000}).to_string();
            let len = body.len().to_string();
            send(
                &router,
                build(
                    "POST",
                    "/internal/ai/geo/next",
                    Some(NGINX_HOST),
                    &[
                        ("content-type", b"application/json"),
                        ("content-length", len.as_bytes()),
                        ("x-xexam-internal-token", TOKEN.as_bytes()),
                    ],
                    Body::from(body),
                ),
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    login_from(&a, "kim", IP_A, UA_PHONE).await;
    let r = tokio::time::timeout(Duration::from_secs(3), waiter)
        .await
        .expect("woken")
        .expect("join");
    let jobs = r.json()["lookups"].as_array().expect("l").clone();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["ip"], IP_A);
    let id = jobs[0]["id"].as_str().expect("id");
    assert_eq!(id.len(), 16);
    assert!(id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
    // in flight: not handed out twice
    assert!(next_jobs(&a, 0).await.is_empty());
    // the eye closed: empty, and everything is forgotten
    login_from(&a, "lee", IP_B, UA_PC).await;
    assert_eq!(a.state.0.activity.geo().queue_len(), 1);
    eye(&a, &adm, false).await;
    assert_eq!(a.state.0.activity.geo().queue_len(), 0);
    assert!(next_jobs(&a, 0).await.is_empty());
}

#[tokio::test]
async fn a_shutdown_ends_a_waiting_poll() {
    let a = app();
    let adm = admin_cookie(&a).await;
    eye(&a, &adm, true).await;
    set_source(&a, "api").await;
    let router = a.router.clone();
    let waiter = tokio::spawn(async move {
        let body = json!({"waitMs": 20000}).to_string();
        let len = body.len().to_string();
        send(
            &router,
            build(
                "POST",
                "/internal/ai/geo/next",
                Some(NGINX_HOST),
                &[
                    ("content-type", b"application/json"),
                    ("content-length", len.as_bytes()),
                    ("x-xexam-internal-token", TOKEN.as_bytes()),
                ],
                Body::from(body),
            ),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    a.state.0.activity.geo().close();
    let r = tokio::time::timeout(Duration::from_secs(3), waiter)
        .await
        .expect("ended")
        .expect("join");
    assert_eq!(r.json()["lookups"], json!([]));
}

// ---------------------------------------------------------------- results

#[tokio::test]
async fn a_result_puts_the_place_on_the_signin_the_activity_and_the_log_rows() {
    let a = app();
    let adm = admin_cookie(&a).await;
    eye(&a, &adm, true).await;
    set_source(&a, "api").await;
    login_from(&a, "kim", IP_A, UA_PHONE).await;
    let jobs = next_jobs(&a, 0).await;
    assert_eq!(jobs.len(), 1);
    let r = send_results(&a, vec![place_ok(&jobs[0].0, "Example City")]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json(), json!({"ok": true}));
    let kim = user_row(&a, &adm, "kim").await;
    assert_eq!(
        kim["lastLoginLocation"],
        json!({"country": "ZZ", "region": "Example Region", "city": "Example City"})
    );
    assert_eq!(kim["lastActiveLocation"]["city"], "Example City");
    let (ip, country, city) = signin_row(&a, "kim").await;
    assert_eq!(
        (ip.as_deref(), country.as_deref(), city.as_deref()),
        (Some(IP_A), Some("ZZ"), Some("Example City"))
    );
    // the cache holds it now: a second sign-in gets it at once and queues nothing
    login_from(&a, "lee", IP_A, UA_PC).await;
    let lee = user_row(&a, &adm, "lee").await;
    assert_eq!(lee["lastLoginLocation"]["city"], "Example City");
    assert_eq!(a.state.0.activity.geo().queue_len(), 0);
    // a repeated or unknown id changes nothing and is still answered 200
    let r = send_results(&a, vec![place_ok(&jobs[0].0, "Another City")]).await;
    assert_eq!(r.status, 200);
    let kim = user_row(&a, &adm, "kim").await;
    assert_eq!(kim["lastLoginLocation"]["city"], "Example City");
}

#[tokio::test]
async fn a_place_is_written_only_where_none_is_set_and_the_address_still_matches() {
    let a = app();
    let adm = admin_cookie(&a).await;
    eye(&a, &adm, true).await;
    set_source(&a, "api").await;
    let c = cookie_of(&login_from(&a, "kim", IP_A, UA_PHONE).await);
    let jobs = next_jobs(&a, 0).await;
    // the user moves to another address before the answer arrives: the activity place is not overwritten, the sign-in's is
    // filled
    reopen_gate(&a, "kim");
    let r = call(
        &a,
        "GET",
        "/api/banks",
        Some(&c),
        &visitor(IP_B, UA_PC),
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    let r = send_results(&a, vec![place_ok(&jobs[0].0, "Example City")]).await;
    assert_eq!(r.status, 200);
    let kim = user_row(&a, &adm, "kim").await;
    assert_eq!(kim["lastLoginLocation"]["city"], "Example City");
    assert_eq!(kim["lastActiveIp"], IP_B);
    assert!(kim["lastActiveLocation"].is_null(), "{kim}");
    // the activity write of the new address queued its own lookup
    let jobs = next_jobs(&a, 0).await;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].1, IP_B);
    // failures and empty places are cached but change nothing
    let r = send_results(
        &a,
        vec![json!({"id": jobs[0].0, "ok": false, "code": "timeout"})],
    )
    .await;
    assert_eq!(r.status, 200);
    let kim = user_row(&a, &adm, "kim").await;
    assert!(kim["lastActiveLocation"].is_null());
    assert!(a.state.0.activity.geo().cache_len() >= 2);
}

#[tokio::test]
async fn a_late_result_after_clear_or_after_the_eye_closed_changes_nothing() {
    let a = app();
    let adm = admin_cookie(&a).await;
    eye(&a, &adm, true).await;
    set_source(&a, "api").await;
    login_from(&a, "kim", IP_A, UA_PHONE).await;
    let jobs = next_jobs(&a, 0).await;
    let r = post(&a, "/api/users/signin-info/clear", &adm, json!({})).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = send_results(&a, vec![place_ok(&jobs[0].0, "Example City")]).await;
    assert_eq!(r.status, 200);
    let kim = user_row(&a, &adm, "kim").await;
    assert!(kim["lastLoginIp"].is_null() && kim["lastLoginLocation"].is_null());
    assert!(kim["lastActiveIp"].is_null() && kim["lastActiveLocation"].is_null());
    assert_eq!(
        a.state.0.activity.geo().cache_len(),
        0,
        "a late result is not cached either"
    );
    // the eye closed after the hand-out
    login_from(&a, "lee", IP_B, UA_PC).await;
    let jobs = next_jobs(&a, 0).await;
    assert_eq!(jobs.len(), 1);
    eye(&a, &adm, false).await;
    let r = send_results(&a, vec![place_ok(&jobs[0].0, "Example City")]).await;
    assert_eq!(r.status, 200);
    eye(&a, &adm, true).await;
    let lee = user_row(&a, &adm, "lee").await;
    assert!(lee["lastLoginLocation"].is_null(), "{lee}");
    assert_eq!(a.state.0.activity.geo().cache_len(), 0);
}

// ---------------------------------------------------------------- eye closed, last activity, clear

#[tokio::test]
async fn eye_closed_records_no_address_no_place_no_lookup_but_keeps_the_device() {
    let a = app();
    let adm = admin_cookie(&a).await;
    set_source(&a, "api").await;
    let c = cookie_of(&login_from(&a, "kim", IP_A, UA_PHONE).await);
    assert_eq!(a.state.0.activity.geo().queue_len(), 0);
    let kim = user_row(&a, &adm, "kim").await;
    assert!(kim["lastLoginIp"].is_null() && kim["lastActiveIp"].is_null());
    assert!(kim["lastActiveLocation"].is_null());
    assert_eq!(
        kim["lastActiveUa"], UA_PHONE,
        "the device is not an address"
    );
    reopen_gate(&a, "kim");
    let r = call(
        &a,
        "GET",
        "/api/banks",
        Some(&c),
        &visitor(IP_B, UA_PC),
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(a.state.0.activity.geo().queue_len(), 0);
    let kim = user_row(&a, &adm, "kim").await;
    assert!(kim["lastActiveIp"].is_null());
    assert_eq!(kim["lastActiveUa"], UA_PC);
    let n: i64 = raw(&a.db)
        .query_row(
            "SELECT COUNT(*) FROM users WHERE last_active_ip IS NOT NULL OR last_active_city IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(n, 0, "nothing stored while the eye is closed");
    // opening the eye shows what was stored while it was open earlier, never what was skipped
    eye(&a, &adm, true).await;
    let kim = user_row(&a, &adm, "kim").await;
    assert!(kim["lastActiveIp"].is_null());
}

#[tokio::test]
async fn the_last_active_write_follows_the_minute_gate_and_never_runs_for_anonymous_requests() {
    let a = app();
    let adm = admin_cookie(&a).await;
    eye(&a, &adm, true).await;
    let c = cookie_of(&login_from(&a, "kim", IP_A, UA_PHONE).await);
    // inside the same minute: nothing is written
    let r = call(
        &a,
        "GET",
        "/api/banks",
        Some(&c),
        &visitor(IP_B, UA_PC),
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(user_row(&a, &adm, "kim").await["lastActiveIp"], IP_A);
    // an anonymous request writes nothing, even with the gate open
    reopen_gate(&a, "kim");
    let r = call(
        &a,
        "GET",
        "/api/auth/state",
        None,
        &visitor(IP_B, UA_PC),
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(user_row(&a, &adm, "kim").await["lastActiveIp"], IP_A);
    // the next minute: ip, place and device follow the request
    reopen_gate(&a, "kim");
    let r = call(
        &a,
        "GET",
        "/api/banks",
        Some(&c),
        &visitor(IP_B, UA_PC),
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    let kim = user_row(&a, &adm, "kim").await;
    assert_eq!(kim["lastActiveIp"], IP_B);
    assert_eq!(kim["lastActiveUa"], UA_PC);
    assert_eq!(kim["lastActiveLocation"]["city"], "Cloud City");
}

/// ip, country, region, city, user agent
type Facts5 = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Owner rule "one source": for one request, the System log row and the user's last-activity facts carry the same IP,
/// place and device (the same ClientInfo, the same source rule), in both place-source modes.
#[tokio::test]
async fn one_request_gives_the_log_row_and_the_last_active_facts_the_same_ip_place_and_device() {
    for api in [false, true] {
        let a = app();
        let adm = admin_cookie(&a).await;
        eye(&a, &adm, true).await;
        if api {
            set_source(&a, "api").await;
            let ip: std::net::IpAddr = IP_B.parse().expect("ip");
            a.state
                .0
                .activity
                .geo()
                .cache_place(ip, here(), auth::unix_now());
        }
        reopen_gate(&a, "admin");
        let r = call(
            &a,
            "POST",
            "/api/activity/settings",
            Some(&adm),
            &visitor(IP_B, UA_PC),
            Some(json!({"userVisits": true}).to_string()),
        )
        .await;
        assert_eq!(r.status, 200, "{}", r.text());
        a.state.0.activity.flush().await;
        let row: Facts5 = raw(&a.db)
            .query_row(
                "SELECT ip, country, region, city, ua FROM activity_log WHERE action = 'logs.settings' ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .expect("log row");
        let me = user_row(&a, &adm, "admin").await;
        let loc = &me["lastActiveLocation"];
        let facts = (
            me["lastActiveIp"].as_str().map(str::to_string),
            loc["country"].as_str().map(str::to_string),
            loc["region"].as_str().map(str::to_string),
            loc["city"].as_str().map(str::to_string),
            me["lastActiveUa"].as_str().map(str::to_string),
        );
        assert_eq!(row, facts, "api mode: {api}");
        let want_city = if api { "Example City" } else { "Cloud City" };
        assert_eq!(row.3.as_deref(), Some(want_city), "api mode: {api}");
        assert_eq!(row.0.as_deref(), Some(IP_B));
        assert_eq!(row.4.as_deref(), Some(UA_PC));
    }
}

#[tokio::test]
async fn the_ai_auth_step_passes_the_client_to_the_last_active_write() {
    let a = app();
    let adm = admin_cookie(&a).await;
    eye(&a, &adm, true).await;
    let c = cookie_of(&login_from(&a, "kim", IP_A, UA_PHONE).await);
    reopen_gate(&a, "kim");
    let mut headers = visitor(IP_B, UA_PC);
    headers.push(("x-original-host", HOST));
    headers.push(("x-original-method", "POST"));
    headers.push(("x-original-origin", ORIGIN));
    let r = call(&a, "GET", "/internal/ai/auth", Some(&c), &headers, None).await;
    assert_eq!(r.status, 204, "{}", r.text());
    let kim = user_row(&a, &adm, "kim").await;
    assert_eq!(kim["lastActiveIp"], IP_B);
    assert_eq!(kim["lastActiveUa"], UA_PC);
}

#[tokio::test]
async fn clear_wipes_the_activity_address_and_place_keeps_the_device_and_empties_the_cache() {
    let a = app();
    let adm = admin_cookie(&a).await;
    eye(&a, &adm, true).await;
    set_source(&a, "api").await;
    let ip: std::net::IpAddr = IP_B.parse().expect("ip");
    a.state
        .0
        .activity
        .geo()
        .cache_place(ip, here(), auth::unix_now());
    login_from(&a, "kim", IP_A, UA_PHONE).await;
    login_from(&a, "lee", IP_B, UA_PC).await;
    let lee = user_row(&a, &adm, "lee").await;
    assert_eq!(lee["lastActiveLocation"]["city"], "Example City");
    assert_eq!(a.state.0.activity.geo().queue_len(), 1);
    let r = post(&a, "/api/users/signin-info/clear", &adm, json!({})).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = r.json();
    for u in v["users"].as_array().expect("users") {
        assert!(
            u["lastActiveIp"].is_null() && u["lastActiveLocation"].is_null(),
            "{u}"
        );
        assert!(u["lastLoginIp"].is_null() && u["lastLoginLocation"].is_null());
    }
    let kim = user_row(&a, &adm, "kim").await;
    assert_eq!(kim["lastActiveUa"], UA_PHONE, "the device stays");
    let g = a.state.0.activity.geo();
    assert_eq!((g.cache_len(), g.queue_len(), g.inflight_len()), (0, 0, 0));
    let n: i64 = raw(&a.db)
        .query_row(
            "SELECT COUNT(*) FROM users WHERE last_active_ip IS NOT NULL OR last_active_country IS NOT NULL \
             OR last_active_region IS NOT NULL OR last_active_city IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(n, 0);
}

// ---------------------------------------------------------------- the internal routes

#[tokio::test]
async fn the_internal_routes_need_the_token_and_validate_their_bodies() {
    let a = app();
    for path in [
        "/internal/ai/geo/next",
        "/internal/ai/geo/result",
        "/internal/ai/geo/source",
    ] {
        let r = internal(&a, path, &json!({}), false).await;
        assert_eq!(r.status, 404, "{path}");
    }
    for bad in [
        json!({}),
        json!({"waitMs": -1}),
        json!({"waitMs": 20001}),
        json!({"waitMs": "5"}),
        json!({"waitMs": 0, "extra": 1}),
    ] {
        let r = internal(&a, "/internal/ai/geo/next", &bad, true).await;
        assert_eq!(r.status, 400, "{bad}");
    }
    let id = "0123456789abcdef";
    let ok = place_ok(id, "Example City");
    let many: Vec<Value> = (0..11).map(|_| ok.clone()).collect();
    for bad in [
        json!({}),
        json!({"results": []}),
        json!({"results": many}),
        json!({"results": [{"id": "XYZ", "ok": false, "code": "timeout"}]}),
        json!({"results": [{"id": "0123456789ABCDEF", "ok": false, "code": "timeout"}]}),
        json!({"results": [{"id": id, "ok": false, "code": "nope"}]}),
        json!({"results": [{"id": id, "ok": false}]}),
        json!({"results": [{"id": id, "ok": true}]}),
        json!({"results": [{"id": id, "ok": true, "place": {"zip": "x"}}]}),
        json!({"results": [{"id": id, "ok": true, "place": {"city": 5}}]}),
        json!({"results": [{"id": id, "ok": "yes", "code": "timeout"}]}),
        json!({"results": [{"id": id, "ok": false, "code": "timeout", "extra": 1}]}),
        json!({"results": [ok, {"id": id, "ok": false, "code": "nope"}]}),
        json!({"results": [ok], "extra": 1}),
    ] {
        let r = internal(&a, "/internal/ai/geo/result", &bad, true).await;
        assert_eq!(r.status, 400, "{bad}");
    }
    // every failure code of the vocabulary is accepted
    for code in [
        "timeout",
        "unreachable",
        "redirect",
        "httpStatus",
        "notJson",
        "tooLarge",
        "providerFail",
        "noFields",
        "keyProblem",
        "rateLimited",
        "notConfigured",
        "refused",
    ] {
        let r = send_results(&a, vec![json!({"id": id, "ok": false, "code": code})]).await;
        assert_eq!(r.status, 200, "{code}");
    }
    // the source route: read, set, validate
    let r = internal(&a, "/internal/ai/geo/source", &json!({}), true).await;
    assert_eq!(r.json(), json!({"source": "cloudflare"}));
    for bad in [
        json!({"source": "other"}),
        json!({"source": 1}),
        json!({"source": "api", "x": 1}),
    ] {
        let r = internal(&a, "/internal/ai/geo/source", &bad, true).await;
        assert_eq!(r.status, 400, "{bad}");
    }
    set_source(&a, "api").await;
    let r = internal(&a, "/internal/ai/geo/source", &json!({}), true).await;
    assert_eq!(r.json(), json!({"source": "api"}));
    // stored in the main database, and nothing is recorded in the activity log by the route itself
    let stored: String = raw(&a.db)
        .query_row(
            "SELECT value FROM system_settings WHERE key='geo.source'",
            [],
            |r| r.get(0),
        )
        .expect("setting");
    assert_eq!(stored, "api");
}

#[tokio::test]
async fn the_cap_keeps_a_flood_of_sign_ins_from_hammering_the_provider() {
    let a = app();
    let adm = admin_cookie(&a).await;
    eye(&a, &adm, true).await;
    set_source(&a, "api").await;
    // 40 distinct public addresses queue up; hand-outs stop at the per-minute cap
    let geo = a.state.0.activity.geo();
    let now = auth::unix_now();
    for i in 0..40u32 {
        let ip = std::net::IpAddr::V4(std::net::Ipv4Addr::from(0x0900_0000u32 + i));
        assert!(geo.request(ip, Target::Signin(1), now));
    }
    let mut got = 0;
    for _ in 0..5 {
        got += next_jobs(&a, 0).await.len();
    }
    // a minute boundary may fall in between: the cap is per wall-clock minute
    assert!((30..=40).contains(&got), "{got}");
}

// ---------------------------------------------------------------- the migration to schema 9

fn open_fixed(db: &Path) -> Store {
    Store::open_with_clock(db, Arc::new(FixedClock("2026-11-15 07:20".to_string()))).expect("open")
}

fn master(p: &Path) -> Vec<(String, String, Option<String>)> {
    let c = raw(p);
    let mut st = c
        .prepare("SELECT type, name, sql FROM sqlite_master ORDER BY rowid")
        .expect("prepare");
    let rows = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query");
    rows.map(|r| r.expect("row")).collect()
}

#[test]
fn a_schema_8_file_with_data_migrates_to_9_with_one_backup_and_one_row() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let fresh = dir.path().join("fresh").join("xexam.db");
    drop(open_fixed(&fresh));
    // A genuine schema-8 file (v2.0: built from the history constants, not by taking columns off a fresh file)
    dbc::historic(&db, 8);
    raw(&db)
        .execute_batch(
            "INSERT INTO users(id, username, password_hash, role, created_at, password_changed_at, new_banks, last_login_at) VALUES                (1, 'admin', 'placeholder-hash', 'admin', '2026-11-01 07:00', '2026-11-01 07:00', 1, NULL),                (2, 'ann', 'placeholder-hash', 'user', '2026-11-01 07:00', '2026-11-01 07:00', 1, '2026-11-01 08:00');",
        )
        .expect("schema-8 users");
    let s = open_fixed(&db);
    let backup = s
        .migration_backup()
        .expect("a backup was written")
        .to_path_buf();
    let name = backup.file_name().and_then(|n| n.to_str()).expect("name");
    assert!(name.starts_with("xexam-schema8-"), "{name}");
    drop(s);
    let c = raw(&db);
    let schema: String = c
        .query_row("SELECT value FROM meta WHERE key='schema'", [], |r| {
            r.get(0)
        })
        .expect("schema");
    assert_eq!(schema, xexam_backend::database::schema::SCHEMA_VERSION);
    // data intact, the new columns empty
    let (login, ip, ua): (Option<String>, Option<String>, Option<String>) = c
        .query_row(
            "SELECT last_login_at, last_active_ip, last_active_ua FROM users WHERE username='ann'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("ann");
    assert_eq!(login.as_deref(), Some("2026-11-01 08:00"));
    assert_eq!((ip, ua), (None, None));
    // exactly one migrate row, from 8 to 9
    let rows: Vec<String> = c
        .prepare("SELECT detail FROM activity_log WHERE action = 'system.migrate'")
        .expect("prepare")
        .query_map([], |r| r.get(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(rows, vec![r#"{"from":8,"to":12}"#.to_string()]);
    // fresh == migrated, and the next open changes nothing
    assert_eq!(master(&db), master(&fresh));
    drop(c);
    let before = dbc::db_bytes(&db);
    let s = open_fixed(&db);
    assert!(s.migration_backup().is_none());
    drop(s);
    assert_eq!(dbc::db_bytes(&db), before, "byte-identical");
    assert_eq!(
        std::fs::read_dir(dir.path().join("backups"))
            .expect("dir")
            .count(),
        1
    );
}

#[test]
fn a_fresh_database_is_schema_9_without_a_migrate_row() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = open_fixed(&db);
    assert!(s.migration_backup().is_none());
    drop(s);
    let c = raw(&db);
    let schema: String = c
        .query_row("SELECT value FROM meta WHERE key='schema'", [], |r| {
            r.get(0)
        })
        .expect("schema");
    assert_eq!(schema, xexam_backend::database::schema::SCHEMA_VERSION);
    let n: i64 = c
        .query_row("SELECT COUNT(*) FROM activity_log", [], |r| r.get(0))
        .expect("count");
    assert_eq!(n, 0);
    let cols: Vec<String> = c
        .prepare("SELECT name FROM pragma_table_info('users') ORDER BY cid")
        .expect("prepare")
        .query_map([], |r| r.get(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(
        cols[cols.len() - 7..cols.len() - 2],
        [
            "last_active_ip",
            "last_active_country",
            "last_active_region",
            "last_active_city",
            "last_active_ua"
        ]
    );
}

// ---------------------------------------------------------------- the backfill (v1.6)

/// Cloudflare mode, the eye open: kim and lee signed in from `IP_A`, then every place is wiped so the rows are placeless.
/// Extra rows: an old placeless row and a mapped-form row of `IP_A`, a placeless and a placed row of `IP_B`, and a placed
/// row of `IP_A` whose place must survive. Returns the administrator's cookie.
async fn placeless_world(a: &App) -> String {
    let adm = admin_cookie(a).await;
    eye(a, &adm, true).await;
    assert_eq!(login_from(a, "kim", IP_A, UA_PHONE).await.status, 200);
    assert_eq!(login_from(a, "lee", IP_A, UA_PC).await.status, 200);
    a.state.0.activity.flush().await;
    let c = raw(&a.db);
    c.execute_batch(
        "UPDATE users SET last_login_country=NULL, last_login_region=NULL, last_login_city=NULL, \
         last_active_country=NULL, last_active_region=NULL, last_active_city=NULL; \
         UPDATE activity_log SET country=NULL, region=NULL, city=NULL;",
    )
    .expect("wipe");
    let now = auth::unix_now();
    let old = now - 90 * 86_400;
    for (ts, ip, city) in [
        (old, IP_A, None),
        (old, "::ffff:8.8.8.8", None),
        (now - 100, IP_B, None),
        (old, IP_B, Some("Kept City")),
        (old, IP_A, Some("Kept City")),
        (old, "10.1.2.3", None),
    ] {
        c.execute(
            "INSERT INTO activity_log (ts, last_ts, actor, action, result, ip, country, city) \
             VALUES (?1, ?1, 'system', 'test.row', 'ok', ?2, ?3, ?4)",
            rusqlite::params![ts, ip, city.map(|_| "ZZ"), city],
        )
        .expect("row");
    }
    adm
}

fn place_count(a: &App, where_: &str) -> i64 {
    raw(&a.db)
        .query_row(
            &format!("SELECT COUNT(*) FROM activity_log WHERE {where_}"),
            [],
            |r| r.get(0),
        )
        .expect("count")
}

#[tokio::test]
async fn backfill_looks_up_each_address_once_and_fills_every_placeless_row() {
    let a = app();
    let adm = placeless_world(&a).await;
    set_source(&a, "api").await;
    let g = a.state.0.activity.geo();
    // IP_A (in log rows, mapped form, two users' sign-in and activity) and IP_B: two distinct public addresses
    assert_eq!(g.backfill_left(), 2);
    let jobs = next_jobs(&a, 0).await;
    let mut ips: Vec<&str> = jobs.iter().map(|j| j.1.as_str()).collect();
    ips.sort_unstable();
    assert_eq!(ips, vec![IP_B, IP_A], "one lookup per distinct address");
    assert!(
        next_jobs(&a, 0).await.is_empty(),
        "nothing is looked up twice"
    );
    let results: Vec<Value> = jobs
        .iter()
        .map(|(id, ip)| place_ok(id, if ip == IP_A { "City of A" } else { "City of B" }))
        .collect();
    let r = send_results(&a, results).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(g.backfill_left(), 0);
    // every placeless row of IP_A (old, mapped form) and of IP_B has the place; placed rows keep theirs; private stays empty
    assert_eq!(
        place_count(
            &a,
            "ip IN ('8.8.8.8', '::ffff:8.8.8.8') AND city = 'City of A'"
        ),
        place_count(
            &a,
            "ip IN ('8.8.8.8', '::ffff:8.8.8.8') AND city IS NOT 'Kept City'"
        )
    );
    assert!(
        place_count(
            &a,
            "ip IN ('8.8.8.8', '::ffff:8.8.8.8') AND city = 'City of A'"
        ) >= 4
    );
    assert_eq!(place_count(&a, "ip = '8.8.8.8' AND city = 'Kept City'"), 1);
    assert_eq!(place_count(&a, "ip = '1.1.1.1' AND city = 'Kept City'"), 1);
    assert_eq!(place_count(&a, "ip = '1.1.1.1' AND city = 'City of B'"), 1);
    assert_eq!(place_count(&a, "ip = '10.1.2.3' AND city IS NOT NULL"), 0);
    assert_eq!(
        place_count(
            &a,
            "ts < strftime('%s','now') - 80*86400 AND city = 'City of A'"
        ),
        2,
        "rows older than the live window are filled"
    );
    for name in ["kim", "lee"] {
        let u = user_row(&a, &adm, name).await;
        assert_eq!(u["lastLoginLocation"]["city"], "City of A", "{name}");
        assert_eq!(u["lastActiveLocation"]["city"], "City of A", "{name}");
        assert_eq!(u["lastLoginLocation"]["country"], "ZZ", "{name}");
    }
    assert!(next_jobs(&a, 0).await.is_empty());
}

#[tokio::test]
async fn backfill_never_overwrites_a_place_and_live_jobs_go_first() {
    let a = app();
    let adm = placeless_world(&a).await;
    // kim already has a place again before the lookups come back
    raw(&a.db)
        .execute(
            "UPDATE users SET last_login_country='QQ', last_login_city='Own City' WHERE username='kim'",
            [],
        )
        .expect("kim");
    set_source(&a, "api").await;
    // a sign-in now queues a live lookup for a new address: it is handed out before the backfill ones
    assert_eq!(login_from(&a, "lee", "9.9.9.9", UA_PC).await.status, 200);
    let jobs = next_jobs(&a, 0).await;
    assert_eq!(jobs.len(), 3);
    assert_eq!(jobs[0].1, "9.9.9.9", "live first");
    let results: Vec<Value> = jobs
        .iter()
        .map(|(id, _)| place_ok(id, "Looked Up"))
        .collect();
    assert_eq!(send_results(&a, results).await.status, 200);
    let kim = user_row(&a, &adm, "kim").await;
    assert_eq!(
        kim["lastLoginLocation"]["city"], "Own City",
        "never overwritten"
    );
    assert_eq!(kim["lastActiveLocation"]["city"], "Looked Up");
    assert_eq!(place_count(&a, "ip = '8.8.8.8' AND city = 'Kept City'"), 1);
}

#[tokio::test]
async fn backfill_rate_limit_pauses_it_and_reposting_api_restarts_it() {
    let a = app();
    let _adm = placeless_world(&a).await;
    set_source(&a, "api").await;
    let g = a.state.0.activity.geo();
    let jobs = next_jobs(&a, 0).await;
    assert_eq!(jobs.len(), 2);
    let (limited, other) = (&jobs[0], &jobs[1]);
    let r = send_results(
        &a,
        vec![
            json!({"id": limited.0, "ok": false, "code": "rateLimited"}),
            place_ok(&other.0, "Other City"),
        ],
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    // paused: the limited address waits, nothing is handed out, nothing is written for it
    assert_eq!(g.backfill_left(), 1);
    assert!(next_jobs(&a, 0).await.is_empty());
    assert!(place_count(&a, &format!("ip = '{}' AND city IS NULL", limited.1)) > 0);
    // posting "api" again (new provider settings saved) restarts it: the one address that is still missing comes back at once
    set_source(&a, "api").await;
    assert_eq!(g.backfill_left(), 1);
    let again = next_jobs(&a, 0).await;
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].1, limited.1);
}

#[tokio::test]
async fn backfill_stops_when_the_eye_closes_the_source_goes_back_or_records_are_cleared() {
    let a = app();
    let adm = placeless_world(&a).await;
    let g = a.state.0.activity.geo();
    set_source(&a, "api").await;
    assert_eq!(g.backfill_left(), 2);
    eye(&a, &adm, false).await;
    assert_eq!(g.backfill_left(), 0, "eye closed");
    assert!(next_jobs(&a, 0).await.is_empty());
    // eye closed: posting api starts nothing
    set_source(&a, "api").await;
    assert_eq!(g.backfill_left(), 0);
    eye(&a, &adm, true).await;
    set_source(&a, "api").await;
    assert_eq!(g.backfill_left(), 2);
    let r = post(
        &a,
        "/api/users/geo-source",
        &adm,
        json!({"source": "cloudflare"}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(g.backfill_left(), 0, "source back to Cloudflare");
    assert!(next_jobs(&a, 0).await.is_empty());
    set_source(&a, "api").await;
    assert_eq!(g.backfill_left(), 2);
    let r = post(&a, "/api/users/signin-info/clear", &adm, json!({})).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(g.backfill_left(), 0, "clear records");
    assert!(next_jobs(&a, 0).await.is_empty());
    // nothing left to look up after the clear
    set_source(&a, "api").await;
    assert_eq!(g.backfill_left(), 0);
}

#[tokio::test]
async fn cloudflare_mode_never_backfills() {
    let a = app();
    let _adm = placeless_world(&a).await;
    set_source(&a, "cloudflare").await;
    assert_eq!(a.state.0.activity.geo().backfill_left(), 0);
    assert!(next_jobs(&a, 0).await.is_empty());
}
