//! v3.1: devices — one row per browser per user (`xexam_did`), the sign-in facts of a device, the device list and the
//! sign-out routes, the linking of sessions that have no device, the cookie order, the delete paths (A.7), the eye / clear /
//! place rules for device rows and the backfill lane.
//!
//! Real router, real engine, real database; users are created straight in the store (made-up names) and sign in through the
//! API. Only made-up data and documentation addresses are used.
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
use xexam_backend::api::{build_router, AppState};
use xexam_backend::auth::{self, Role};
use xexam_backend::config::Config;
use xexam_backend::database::{NewSession, Place, Store, UserChange};
use xexam_backend::services::RealEngine;

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";
const PW: &str = "kitchen-pass-1";
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) TestBrowser/1.0";
const IP: &str = "203.0.113.7";
const DAY: i64 = 86_400;

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

/// A fresh app: `admin`, `kim` and `lee` all have the password [`PW`], a display name and no pending change.
fn app() -> App {
    app_with(apic::default_config())
}

fn app_with(cfg: Config) -> App {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let store = Arc::new(Store::open(&db).expect("store"));
    let engine = Arc::new(RealEngine::new(store.clone()));
    let state = AppState::new_with_logs(cfg, store.clone(), engine, None, Default::default());
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

async fn post(a: &App, path: &str, cookie: &str, body: Value) -> Resp {
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

/// Every `Set-Cookie` of an answer, in order.
fn cookies(r: &Resp) -> Vec<String> {
    r.headers
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().expect("cookie").to_string())
        .collect()
}

/// The `name=value` pair of the cookie `name` in an answer.
fn pair(r: &Resp, name: &str) -> Option<String> {
    cookies(r)
        .iter()
        .find(|c| c.starts_with(&format!("{name}=")))
        .map(|c| c.split(';').next().expect("pair").to_string())
}

/// One sign-in: the answer and the cookie header a browser would send afterwards (`xexam_session` and `xexam_did`).
struct Browser {
    session: String,
    did: String,
}

impl Browser {
    fn header(&self) -> String {
        format!("{}; {}", self.session, self.did)
    }
}

async fn login_with(a: &App, name: &str, did: Option<&str>, extra: &[(&str, &str)]) -> Resp {
    call(
        a,
        "POST",
        "/api/auth/login",
        did,
        Some(ORIGIN),
        extra,
        Some(json!({"username": name, "password": PW}).to_string()),
    )
    .await
}

/// Signs in as `name` from a new browser (no `xexam_did` yet).
async fn browser(a: &App, name: &str) -> Browser {
    let r = login_with(a, name, None, &[("user-agent", UA)]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    Browser {
        session: pair(&r, "xexam_session").expect("session cookie"),
        did: pair(&r, "xexam_did").expect("did cookie"),
    }
}

/// Signs in as `name` again from the browser that holds `did`.
async fn sign_in_again(a: &App, name: &str, did: &str) -> Browser {
    let r = login_with(a, name, Some(did), &[("user-agent", UA)]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    Browser {
        session: pair(&r, "xexam_session").expect("session cookie"),
        did: pair(&r, "xexam_did").expect("did cookie"),
    }
}

/// Opens or closes the sign-in info eye (database and the cache the request path reads).
fn eye(a: &App, open: bool) {
    a.store.set_signin_info(open, None).expect("eye");
    a.state.0.activity.reload_settings(&a.store);
}

/// Sets the place source: the external lookup (`true`) or the Cloudflare headers.
fn source(a: &App, api: bool) {
    a.store.set_geo_source(api, None).expect("source");
    a.state.0.activity.reload_settings(&a.store);
}

fn user_id(a: &App, name: &str) -> i64 {
    a.store
        .user_by_name(name)
        .expect("lookup")
        .expect("user")
        .id
}

fn did_hash(did_pair: &str) -> String {
    auth::token_hash(did_pair.trim_start_matches("xexam_did="))
}

fn session_hash(session_pair: &str) -> String {
    auth::token_hash(session_pair.trim_start_matches("xexam_session="))
}

fn count(a: &App, sql: &str) -> i64 {
    raw(&a.db)
        .query_row(sql, [], |r| r.get(0))
        .expect("count query")
}

fn device_of_session(a: &App, session_pair: &str) -> Option<i64> {
    raw(&a.db)
        .query_row(
            "SELECT device_id FROM sessions WHERE token_hash = ?1",
            [session_hash(session_pair)],
            |r| r.get(0),
        )
        .expect("session row")
}

fn device_by_did(a: &App, user: &str, did_pair: &str) -> Option<i64> {
    raw(&a.db)
        .query_row(
            "SELECT id FROM devices WHERE user_id = ?1 AND cookie_hash = ?2",
            rusqlite::params![user_id(a, user), did_hash(did_pair)],
            |r| r.get(0),
        )
        .ok()
}

fn session_exists(a: &App, session_pair: &str) -> bool {
    count(
        a,
        &format!(
            "SELECT COUNT(*) FROM sessions WHERE token_hash = '{}'",
            session_hash(session_pair)
        ),
    ) == 1
}

fn signed_out_at(a: &App, device: i64) -> Option<i64> {
    raw(&a.db)
        .query_row(
            "SELECT signed_out_at FROM devices WHERE id = ?1",
            [device],
            |r| r.get(0),
        )
        .expect("device row")
}

async fn entries(a: &App, action: &str) -> Vec<(Option<String>, Option<String>)> {
    a.state.0.activity.flush().await;
    let c = raw(&a.db);
    let mut st = c
        .prepare("SELECT object, detail FROM activity_log WHERE action = ?1 AND result = 'ok' ORDER BY id")
        .expect("prepare");
    let rows = st
        .query_map([action], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query");
    rows.map(|r| r.expect("row")).collect()
}

fn keys(v: &Value) -> Vec<String> {
    v.as_object()
        .expect("object")
        .keys()
        .cloned()
        .collect::<Vec<_>>()
}

// ---------------------------------------------------------------- sign-in: cookies and the device row

#[tokio::test]
async fn sign_in_sets_the_session_cookie_first_then_the_known_browser_then_the_device_id() {
    let a = app();
    let r = login_with(&a, "kim", None, &[("user-agent", UA)]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let all = cookies(&r);
    assert_eq!(all.len(), 3, "{all:?}");
    assert!(all[0].starts_with("xexam_session="), "{all:?}");
    assert!(all[1].starts_with("xexam_device="), "{all:?}");
    let did = &all[2];
    assert!(did.starts_with("xexam_did="), "{all:?}");
    let value = did
        .split(';')
        .next()
        .expect("pair")
        .trim_start_matches("xexam_did=");
    assert_eq!(value.len(), 43, "{value}");
    assert!(did.contains("; HttpOnly"), "{did}");
    assert!(did.contains("; SameSite=Strict"), "{did}");
    assert!(did.contains("; Path=/"), "{did}");
    assert!(did.contains("; Max-Age=34560000"), "{did}");
    assert!(!did.contains("Secure"), "{did}");
    // the database holds only the hash of the cookie
    let (hash, ua, app_flag): (String, Option<String>, i64) = raw(&a.db)
        .query_row(
            "SELECT cookie_hash, ua, app FROM devices WHERE user_id = ?1",
            [user_id(&a, "kim")],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("device");
    assert_eq!(hash, auth::token_hash(value));
    assert_eq!(ua.as_deref(), Some(UA));
    assert_eq!(app_flag, 0);
    assert_eq!(
        device_of_session(&a, &pair(&r, "xexam_session").expect("s")),
        device_by_did(&a, "kim", &pair(&r, "xexam_did").expect("d"))
    );
}

#[tokio::test]
async fn the_device_id_cookie_is_secure_when_the_cookie_is_forced_secure() {
    let a = app_with(apic::config_with(&[("XEXAM_COOKIE_SECURE", "1")]));
    let r = login_with(&a, "kim", None, &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let all = cookies(&r);
    assert!(all[0].ends_with("; Secure"), "{all:?}");
    assert!(
        all[2].starts_with("xexam_did=") && all[2].ends_with("; Secure"),
        "{all:?}"
    );
    // an old session's first request of any kind may create the device and its cookie when the scheme is known
    let token = auth::new_session_token();
    let now = auth::unix_now();
    a.store
        .create_session(&NewSession {
            token_hash: &auth::token_hash(&token),
            user_id: user_id(&a, "lee"),
            now,
            expires_at: now + 30 * DAY,
            remember: true,
            ip: None,
            ua: None,
            device_id: None,
        })
        .expect("session");
    let r = get(&a, "/api/devices", &format!("xexam_session={token}")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let did = cookies(&r);
    let last = did.last().expect("cookie");
    assert!(
        last.starts_with("xexam_did=") && last.ends_with("; Secure"),
        "{did:?}"
    );
    assert_eq!(
        did.iter().filter(|c| c.starts_with("xexam_did=")).count(),
        1
    );
}

#[tokio::test]
async fn the_same_browser_reuses_its_device_and_a_new_browser_makes_another() {
    let a = app();
    let first = browser(&a, "kim").await;
    let again = sign_in_again(&a, "kim", &first.did).await;
    // the cookie value did not change and no second device was made
    assert_eq!(first.did, again.did);
    assert_eq!(
        count(&a, "SELECT COUNT(*) FROM devices"),
        1,
        "one browser, one device"
    );
    assert_eq!(
        device_of_session(&a, &first.session),
        device_of_session(&a, &again.session)
    );
    // a cookie that is not token-shaped is replaced by a new one
    let r = login_with(&a, "kim", Some("xexam_did=short"), &[]).await;
    let fresh = pair(&r, "xexam_did").expect("did");
    assert_ne!(fresh, first.did);
    assert_eq!(count(&a, "SELECT COUNT(*) FROM devices"), 2);
    // the same browser for another user is another device row (one row per user and cookie)
    let lee = sign_in_again(&a, "lee", &first.did).await;
    assert_eq!(lee.did, first.did);
    assert_eq!(count(&a, "SELECT COUNT(*) FROM devices"), 3);
    assert_ne!(
        device_by_did(&a, "kim", &first.did),
        device_by_did(&a, "lee", &first.did)
    );
}

#[tokio::test]
async fn a_new_sign_in_clears_the_signed_out_mark_of_its_device() {
    let a = app();
    let b = browser(&a, "kim").await;
    let device = device_by_did(&a, "kim", &b.did).expect("device");
    let r = post(&a, "/api/auth/logout", &b.header(), json!({})).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(signed_out_at(&a, device).is_some());
    let again = sign_in_again(&a, "kim", &b.did).await;
    assert_eq!(signed_out_at(&a, device), None);
    assert_eq!(device_of_session(&a, &again.session), Some(device));
}

/// `(signin_ip, signin_country, active_ip, active_city, ua)` of a device row.
type Facts = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

#[tokio::test]
async fn sign_in_facts_follow_the_eye() {
    let a = app();
    let place_headers = [
        ("user-agent", UA),
        ("x-real-ip", IP),
        ("x-geo-country", "ZZ"),
        ("x-geo-region", "Region One"),
        ("x-geo-city", "Town"),
    ];
    // eye closed (the default): the address and place are not stored, the device is
    let r = login_with(&a, "kim", None, &place_headers).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let row = |a: &App| -> Facts {
        raw(&a.db)
            .query_row(
                "SELECT signin_ip, signin_country, active_ip, active_city, ua FROM devices WHERE user_id = ?1",
                [user_id(a, "kim")],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .expect("device")
    };
    assert_eq!(row(&a), (None, None, None, None, Some(UA.to_string())));
    // eye open: sign-in and last-activity facts (the three place parts together)
    eye(&a, true);
    let did = pair(&r, "xexam_did").expect("did");
    let r = login_with(&a, "kim", Some(&did), &place_headers).await;
    assert_eq!(r.status, 200);
    let (ip, country, active_ip, city, _) = row(&a);
    assert_eq!(ip.as_deref(), Some(IP));
    assert_eq!(country.as_deref(), Some("ZZ"));
    assert_eq!(active_ip.as_deref(), Some(IP));
    assert_eq!(city.as_deref(), Some("Town"));
    let (region,): (Option<String>,) = raw(&a.db)
        .query_row(
            "SELECT signin_region FROM devices WHERE user_id = ?1",
            [user_id(&a, "kim")],
            |r| Ok((r.get(0)?,)),
        )
        .expect("region");
    assert_eq!(region.as_deref(), Some("Region One"));
}

#[tokio::test]
async fn the_cap_of_twenty_sessions_never_signs_a_device_out() {
    let a = app();
    let mut did: Option<String> = None;
    let mut first_device: Option<i64> = None;
    for _ in 0..21 {
        let b = match &did {
            None => browser(&a, "kim").await,
            Some(d) => sign_in_again(&a, "kim", d).await,
        };
        first_device = first_device.or(device_of_session(&a, &b.session));
        did = Some(b.did);
    }
    assert_eq!(
        count(&a, "SELECT COUNT(*) FROM sessions"),
        20,
        "the 20-session cap"
    );
    assert_eq!(count(&a, "SELECT COUNT(*) FROM devices"), 1);
    assert_eq!(signed_out_at(&a, first_device.expect("device")), None);
}

#[tokio::test]
async fn at_most_twenty_devices_per_user_and_never_the_one_just_used() {
    let a = app();
    let kim = user_id(&a, "kim");
    // 20 older devices without a session
    let c = raw(&a.db);
    for i in 0..20 {
        c.execute(
            "INSERT INTO devices(user_id, cookie_hash, created_at, active_at) VALUES(?1, ?2, ?3, ?3)",
            rusqlite::params![kim, format!("{i:064x}"), 1_000 + i],
        )
        .expect("device");
    }
    drop(c);
    let b = browser(&a, "kim").await;
    assert_eq!(
        count(&a, "SELECT COUNT(*) FROM devices WHERE user_id = 2"),
        20,
        "the oldest unused one went"
    );
    assert!(device_by_did(&a, "kim", &b.did).is_some());
    assert_eq!(
        count(&a, "SELECT COUNT(*) FROM devices WHERE active_at = 1000"),
        0
    );
}

// ---------------------------------------------------------------- the list

#[tokio::test]
async fn the_device_list_has_the_contract_shape_and_hides_addresses_while_the_eye_is_closed() {
    let a = app();
    let kim = browser(&a, "kim").await;
    let other = browser(&a, "kim").await;
    let _lee = browser(&a, "lee").await;
    let r = get(&a, "/api/devices", &kim.header()).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = r.json();
    assert_eq!(keys(&v), ["devices", "signinInfo"]);
    assert_eq!(v["signinInfo"], false);
    let list = v["devices"].as_array().expect("devices");
    assert_eq!(list.len(), 2, "only the caller's devices");
    assert_eq!(
        keys(&list[0]),
        [
            "id",
            "current",
            "state",
            "ua",
            "app",
            "createdAt",
            "signinAt",
            "signinIp",
            "signinLocation",
            "activeAt",
            "activeIp",
            "activeLocation",
            "push"
        ]
    );
    // the current device first
    assert_eq!(list[0]["current"], true);
    assert_eq!(list[0]["id"].as_i64(), device_of_session(&a, &kim.session));
    assert_eq!(list[1]["current"], false);
    assert_eq!(
        list[1]["id"].as_i64(),
        device_of_session(&a, &other.session)
    );
    assert_eq!(list[0]["state"], "active");
    assert_eq!(list[0]["ua"], UA);
    assert_eq!(list[0]["push"], false);
    for d in list {
        assert!(d["signinIp"].is_null() && d["activeIp"].is_null());
        assert!(d["signinLocation"].is_null() && d["activeLocation"].is_null());
        // local server time, `YYYY-MM-DD HH:MM:SS`
        assert_eq!(d["createdAt"].as_str().map(str::len), Some(19));
        assert_eq!(d["activeAt"].as_str().map(str::len), Some(19));
    }
}

#[tokio::test]
async fn the_list_shows_addresses_and_places_while_the_eye_is_open() {
    let a = app();
    eye(&a, true);
    let r = login_with(
        &a,
        "kim",
        None,
        &[
            ("user-agent", UA),
            ("x-real-ip", IP),
            ("x-geo-country", "ZZ"),
        ],
    )
    .await;
    let session = pair(&r, "xexam_session").expect("session");
    let v = get(&a, "/api/devices", &session).await.json();
    assert_eq!(v["signinInfo"], true);
    let d = &v["devices"][0];
    assert_eq!(d["signinIp"], IP);
    assert_eq!(d["activeIp"], IP);
    assert_eq!(
        d["signinLocation"],
        json!({"country": "ZZ", "region": null, "city": null})
    );
}

#[tokio::test]
async fn device_states_active_expired_and_signed_out() {
    let a = app();
    let first = browser(&a, "kim").await;
    let second = browser(&a, "kim").await;
    let state_of = |v: &Value, id: i64| -> String {
        v["devices"]
            .as_array()
            .expect("devices")
            .iter()
            .find(|d| d["id"] == id)
            .expect("device")["state"]
            .as_str()
            .expect("state")
            .to_string()
    };
    let d1 = device_of_session(&a, &first.session).expect("d1");
    let d2 = device_of_session(&a, &second.session).expect("d2");
    // the first one's session ran past its 30-day idle limit but the purge has not run: expired, not active
    raw(&a.db)
        .execute(
            "UPDATE sessions SET last_seen_at = ?1, started_at = ?1 WHERE token_hash = ?2",
            rusqlite::params![auth::unix_now() - 40 * DAY, session_hash(&first.session)],
        )
        .expect("age one session");
    let v = get(&a, "/api/devices", &second.header()).await.json();
    assert_eq!(state_of(&v, d1), "expired");
    assert_eq!(state_of(&v, d2), "active");
    // an explicit sign-out of the expired row (the "Stop" button): signedOut
    let r = post(
        &a,
        "/api/devices/signout",
        &second.header(),
        json!({"id": d1}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(state_of(&r.json(), d1), "signedOut");
    assert_eq!(state_of(&r.json(), d2), "active");
}

// ---------------------------------------------------------------- hello

#[tokio::test]
async fn hello_validates_stores_the_zone_and_the_app_flag() {
    let a = app();
    let b = browser(&a, "kim").await;
    for bad in [
        json!({}),
        json!({"tz": null, "app": true}),
        json!({"tz": null, "app": "yes", "permission": "granted"}),
        json!({"tz": 5, "app": true, "permission": "granted"}),
        json!({"tz": null, "app": true, "permission": "maybe"}),
        json!({"app": true, "permission": "granted"}),
    ] {
        let r = post(&a, "/api/devices/hello", &b.header(), bad.clone()).await;
        assert_eq!(r.status, 400, "{bad}: {}", r.text());
    }
    let r = post(
        &a,
        "/api/devices/hello",
        &b.header(),
        json!({"tz": "UTC", "app": true, "permission": "default"}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = r.json();
    assert_eq!(keys(&v), ["device"]);
    assert_eq!(v["device"]["app"], true);
    assert_eq!(v["device"]["current"], true);
    let device = device_by_did(&a, "kim", &b.did).expect("device");
    let tz: Option<String> = raw(&a.db)
        .query_row("SELECT tz FROM devices WHERE id = ?1", [device], |r| {
            r.get(0)
        })
        .expect("tz");
    assert_eq!(tz.as_deref(), Some("UTC"));
    // an invalid zone is stored as NULL, never refused
    let r = post(
        &a,
        "/api/devices/hello",
        &b.header(),
        json!({"tz": "../x", "app": false, "permission": "granted"}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let (tz, app_flag): (Option<String>, i64) = raw(&a.db)
        .query_row("SELECT tz, app FROM devices WHERE id = ?1", [device], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("row");
    assert_eq!((tz, app_flag), (None, 0));
    // hello is not logged (route action "")
    assert!(entries(&a, "settings.pushOff").await.is_empty());
}

// ---------------------------------------------------------------- sign-out routes

#[tokio::test]
async fn signing_out_another_device_ends_its_sessions_and_keeps_the_row() {
    let a = app();
    let mine = browser(&a, "kim").await;
    let theirs = browser(&a, "kim").await;
    let device = device_of_session(&a, &theirs.session).expect("device");
    let r = post(
        &a,
        "/api/devices/signout",
        &mine.header(),
        json!({"id": device}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(keys(&r.json()), ["devices", "signinInfo"]);
    assert!(!session_exists(&a, &theirs.session));
    assert!(session_exists(&a, &mine.session));
    assert!(
        signed_out_at(&a, device).is_some(),
        "the row stays, signed out"
    );
    // the other browser is signed out at its next request
    assert_eq!(get(&a, "/api/devices", &theirs.header()).await.status, 401);
    // activity: `auth.signoutDevice` with the device id, no label
    let rows = entries(&a, "auth.signoutDevice").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        serde_json::from_str::<Value>(rows[0].1.as_deref().expect("detail")).expect("json"),
        json!({"device": device})
    );
    // again: still a device of the user, no error
    let r = post(
        &a,
        "/api/devices/signout",
        &mine.header(),
        json!({"id": device}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
}

#[tokio::test]
async fn the_current_device_a_missing_device_and_another_users_device_are_refused() {
    let a = app();
    let mine = browser(&a, "kim").await;
    let lee = browser(&a, "lee").await;
    let current = device_of_session(&a, &mine.session).expect("current");
    let r = post(
        &a,
        "/api/devices/signout",
        &mine.header(),
        json!({"id": current}),
    )
    .await;
    assert_eq!(r.status, 409, "{}", r.text());
    assert_eq!(r.json()["code"], "devices.current");
    for id in [
        json!(9999),
        json!(device_of_session(&a, &lee.session).expect("lee")),
    ] {
        let r = post(
            &a,
            "/api/devices/signout",
            &mine.header(),
            json!({"id": id}),
        )
        .await;
        assert_eq!(r.status, 404, "{}", r.text());
        assert_eq!(r.json()["code"], "devices.notFound");
    }
    for bad in [
        json!({}),
        json!({"id": "1"}),
        json!({"id": 1.5}),
        json!({"id": null}),
    ] {
        let r = post(&a, "/api/devices/signout", &mine.header(), bad).await;
        assert_eq!(r.status, 400, "{}", r.text());
    }
    assert!(session_exists(&a, &lee.session), "lee was never touched");
}

#[tokio::test]
async fn signing_out_the_others_keeps_the_current_device_and_counts() {
    let a = app();
    let mine = browser(&a, "kim").await;
    let b = browser(&a, "kim").await;
    let c = browser(&a, "kim").await;
    let lee = browser(&a, "lee").await;
    let r = post(&a, "/api/devices/signout-others", &mine.header(), json!({})).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = r.json();
    assert_eq!(keys(&v), ["devices", "signinInfo", "count"]);
    assert_eq!(v["count"], 2);
    assert!(session_exists(&a, &mine.session));
    assert!(!session_exists(&a, &b.session) && !session_exists(&a, &c.session));
    assert!(session_exists(&a, &lee.session));
    let states: Vec<_> = v["devices"]
        .as_array()
        .expect("devices")
        .iter()
        .map(|d| {
            (
                d["current"].as_bool().expect("c"),
                d["state"].as_str().expect("s").to_string(),
            )
        })
        .collect();
    assert_eq!(states[0], (true, "active".to_string()));
    assert!(states[1..]
        .iter()
        .all(|s| s == &(false, "signedOut".to_string())));
    let rows = entries(&a, "auth.signoutOthers").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        serde_json::from_str::<Value>(rows[0].1.as_deref().expect("detail")).expect("json"),
        json!({"count": 2})
    );
    // nothing left to sign out: count 0
    let r = post(&a, "/api/devices/signout-others", &mine.header(), json!({})).await;
    assert_eq!(r.json()["count"], 0);
}

#[tokio::test]
async fn logging_out_signs_the_device_out_and_keeps_the_browser_id() {
    let a = app();
    let b = browser(&a, "kim").await;
    // a second session on the same device (signed in again from the same browser)
    let again = sign_in_again(&a, "kim", &b.did).await;
    let device = device_of_session(&a, &b.session).expect("device");
    let r = post(&a, "/api/auth/logout", &again.header(), json!({})).await;
    assert_eq!(r.status, 200, "{}", r.text());
    // only the session cookie is expired; the browser id is not touched
    let all = cookies(&r);
    assert_eq!(all.len(), 1, "{all:?}");
    assert!(all[0].starts_with("xexam_session=;"), "{all:?}");
    assert!(!session_exists(&a, &again.session));
    assert!(
        !session_exists(&a, &b.session),
        "the device's other sessions go too"
    );
    assert!(signed_out_at(&a, device).is_some());
    assert_eq!(
        count(&a, "SELECT COUNT(*) FROM devices"),
        1,
        "the row is kept for the list"
    );
}

#[tokio::test]
async fn an_own_password_change_signs_every_other_device_out() {
    let a = app();
    let mine = browser(&a, "kim").await;
    let other = browser(&a, "kim").await;
    let my_device = device_of_session(&a, &mine.session).expect("mine");
    let other_device = device_of_session(&a, &other.session).expect("other");
    let r = post(
        &a,
        "/api/auth/password",
        &mine.header(),
        json!({"current": PW, "password": "a-brand-new-pass-9"}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let new_session = pair(&r, "xexam_session").expect("new session");
    assert!(!session_exists(&a, &mine.session), "the old session ended");
    assert!(!session_exists(&a, &other.session));
    assert_eq!(device_of_session(&a, &new_session), Some(my_device));
    assert_eq!(
        signed_out_at(&a, my_device),
        None,
        "this device stays signed in"
    );
    assert!(signed_out_at(&a, other_device).is_some());
    assert_eq!(
        count(&a, "SELECT COUNT(*) FROM sessions WHERE user_id = 2"),
        1
    );
}

#[tokio::test]
async fn an_administrator_reset_disable_and_role_change_sign_every_device_out() {
    let a = app();
    let admin = browser(&a, "admin").await;
    let kim_id = user_id(&a, "kim");
    let signed_out_devices = |a: &App| -> i64 {
        count(
            a,
            &format!(
                "SELECT COUNT(*) FROM devices WHERE user_id = {kim_id} AND signed_out_at IS NOT NULL"
            ),
        )
    };
    // password reset
    let k1 = browser(&a, "kim").await;
    let k2 = browser(&a, "kim").await;
    let r = post(
        &a,
        "/api/users/password",
        &admin.header(),
        json!({"id": kim_id, "password": "reset-by-admin-9"}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(!session_exists(&a, &k1.session) && !session_exists(&a, &k2.session));
    assert_eq!(signed_out_devices(&a), 2);
    // disable
    let k3 = sign_in_again_with(&a, "kim", "reset-by-admin-9").await;
    assert_eq!(
        signed_out_devices(&a),
        2,
        "the new browser is a third, signed-in device"
    );
    let r = post(
        &a,
        "/api/users/update",
        &admin.header(),
        json!({"id": kim_id, "disabled": true}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(!session_exists(&a, &k3));
    assert_eq!(signed_out_devices(&a), 3);
    // role change
    raw(&a.db)
        .execute("UPDATE users SET disabled = 0 WHERE id = ?1", [kim_id])
        .expect("enable");
    let k4 = sign_in_again_with(&a, "kim", "reset-by-admin-9").await;
    let r = post(
        &a,
        "/api/users/update",
        &admin.header(),
        json!({"id": kim_id, "role": "admin"}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(!session_exists(&a, &k4));
    assert_eq!(signed_out_devices(&a), 4);
}

/// Signs `kim` in with another password from a new browser; returns the session pair.
async fn sign_in_again_with(a: &App, name: &str, password: &str) -> String {
    let r = call(
        a,
        "POST",
        "/api/auth/login",
        None,
        Some(ORIGIN),
        &[],
        Some(json!({"username": name, "password": password}).to_string()),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    pair(&r, "xexam_session").expect("session")
}

#[tokio::test]
async fn deleting_a_user_deletes_the_devices_child_first() {
    let a = app();
    let admin = browser(&a, "admin").await;
    let _k = browser(&a, "kim").await;
    let _k2 = browser(&a, "kim").await;
    let kim_id = user_id(&a, "kim");
    assert_eq!(
        count(
            &a,
            &format!("SELECT COUNT(*) FROM devices WHERE user_id = {kim_id}")
        ),
        2
    );
    let r = post(
        &a,
        "/api/users/delete",
        &admin.header(),
        json!({"id": kim_id}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(
        count(
            &a,
            &format!("SELECT COUNT(*) FROM devices WHERE user_id = {kim_id}")
        ),
        0
    );
    assert_eq!(
        count(&a, "SELECT COUNT(*) FROM pragma_foreign_key_check"),
        0
    );
}

// ---------------------------------------------------------------- linking a session without a device

/// A session made the old way (no device), as a v3.0 binary or `create_session` does: its cookie pair.
fn old_session(a: &App, name: &str) -> String {
    let token = auth::new_session_token();
    let now = auth::unix_now();
    a.store
        .create_session(&NewSession {
            token_hash: &auth::token_hash(&token),
            user_id: user_id(a, name),
            now,
            expires_at: now + 30 * DAY,
            remember: true,
            ip: None,
            ua: None,
            device_id: None,
        })
        .expect("session");
    format!("xexam_session={token}")
}

#[tokio::test]
async fn a_get_without_origin_never_creates_a_device_or_a_cookie() {
    let a = app();
    let s = old_session(&a, "kim");
    let r = get(&a, "/api/devices", &s).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(cookies(&r).is_empty(), "{:?}", cookies(&r));
    assert_eq!(device_of_session(&a, &s), None);
    assert_eq!(count(&a, "SELECT COUNT(*) FROM devices"), 0);
    // no row is marked current
    assert!(r.json()["devices"].as_array().expect("devices").is_empty());
}

#[tokio::test]
async fn a_cookie_that_names_a_device_of_the_user_links_without_sending_a_cookie() {
    let a = app();
    let b = browser(&a, "kim").await;
    let device = device_by_did(&a, "kim", &b.did).expect("device");
    let s = old_session(&a, "kim");
    // a plain GET (no Origin) carrying the browser id links the session (rule 1)
    let r = get(&a, "/api/devices", &format!("{s}; {}", b.did)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(cookies(&r).is_empty(), "{:?}", cookies(&r));
    assert_eq!(device_of_session(&a, &s), Some(device));
    // the list marks it current
    assert_eq!(r.json()["devices"][0]["current"], true);
    // another user's browser id is no device of this user: nothing is linked
    let s2 = old_session(&a, "lee");
    let r = get(&a, "/api/devices", &format!("{s2}; {}", b.did)).await;
    assert_eq!(r.status, 200);
    assert_eq!(device_of_session(&a, &s2), None);
}

#[tokio::test]
async fn a_post_with_an_origin_creates_the_device_and_sends_the_cookie_once() {
    let a = app();
    let s = old_session(&a, "kim");
    let r = post(
        &a,
        "/api/devices/hello",
        &s,
        json!({"tz": "UTC", "app": false, "permission": "default"}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    // the session cookie of a "remember" session may be re-sent (its first request), the browser id comes after it
    let all = cookies(&r);
    assert_eq!(
        all.iter().filter(|c| c.starts_with("xexam_did=")).count(),
        1,
        "{all:?}"
    );
    assert!(
        all.last().expect("cookie").starts_with("xexam_did="),
        "{all:?}"
    );
    let did = pair(&r, "xexam_did").expect("did");
    assert_eq!(device_of_session(&a, &s), device_by_did(&a, "kim", &did));
    assert_eq!(r.json()["device"]["current"], true);
    // the next POST finds the session linked and sets nothing
    let r = post(
        &a,
        "/api/devices/hello",
        &s,
        json!({"tz": "UTC", "app": false, "permission": "default"}),
    )
    .await;
    assert!(cookies(&r).is_empty());
    assert_eq!(count(&a, "SELECT COUNT(*) FROM devices"), 1);
}

#[tokio::test]
async fn a_post_that_carries_an_existing_browser_id_reuses_it_for_the_device() {
    let a = app();
    let lee = browser(&a, "lee").await;
    let s = old_session(&a, "kim");
    // the browser id of lee's browser, presented with kim's old session: a device for (kim, that cookie)
    let r = post(
        &a,
        "/api/devices/hello",
        &format!("{s}; {}", lee.did),
        json!({"tz": null, "app": false, "permission": "default"}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(device_by_did(&a, "kim", &lee.did).is_some());
    assert_eq!(
        pair(&r, "xexam_did").as_deref(),
        Some(lee.did.as_str()),
        "the same value is sent again, refreshing its 400 days"
    );
}

#[tokio::test]
async fn hello_on_a_session_without_a_device_and_without_an_origin_is_a_bad_request() {
    let a = app();
    let s = old_session(&a, "kim");
    let r = call(
        &a,
        "POST",
        "/api/devices/hello",
        Some(&s),
        None,
        &[],
        Some(json!({"tz": null, "app": false, "permission": "default"}).to_string()),
    )
    .await;
    // the Origin guard refuses a POST without an Origin before the route runs
    assert!(r.status == 400 || r.status == 403, "{}", r.status);
    assert_eq!(device_of_session(&a, &s), None);
}

// ---------------------------------------------------------------- the activity touch

#[tokio::test]
async fn the_activity_touch_writes_the_device_facts() {
    let a = app();
    eye(&a, true);
    let b = browser(&a, "kim").await;
    let device = device_by_did(&a, "kim", &b.did).expect("device");
    raw(&a.db)
        .execute(
            "UPDATE devices SET active_at = active_at - 3600, active_ip = NULL, active_country = NULL WHERE id = ?1",
            [device],
        )
        .expect("old facts");
    // the sign-in just touched the session in memory: forget that so the next request may write
    a.state.0.gates.forget(&session_hash(&b.session));
    let again = Browser {
        session: b.session.clone(),
        did: b.did.clone(),
    };
    let r = call(
        &a,
        "GET",
        "/api/devices",
        Some(&again.header()),
        None,
        &[
            (
                "user-agent",
                "Mozilla/5.0 (Windows NT 10.0) TestBrowser/2.0",
            ),
            ("x-real-ip", "198.51.100.9"),
            ("x-geo-country", "ZZ"),
        ],
        None,
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let (at, ua): (i64, Option<String>) = raw(&a.db)
        .query_row(
            "SELECT active_at, ua FROM devices WHERE id = ?1",
            [device],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("device");
    assert!(at >= auth::unix_now() - 5, "{at}");
    assert_eq!(
        ua.as_deref(),
        Some("Mozilla/5.0 (Windows NT 10.0) TestBrowser/2.0")
    );
    let (ip, country): (Option<String>, Option<String>) = raw(&a.db)
        .query_row(
            "SELECT active_ip, active_country FROM devices WHERE id = ?1",
            [device],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("device");
    assert_eq!(ip.as_deref(), Some("198.51.100.9"));
    assert_eq!(country.as_deref(), Some("ZZ"));
}

// ---------------------------------------------------------------- administrators

#[tokio::test]
async fn administrators_list_and_sign_out_the_devices_of_a_user() {
    let a = app();
    let admin = browser(&a, "admin").await;
    let k1 = browser(&a, "kim").await;
    let k2 = browser(&a, "kim").await;
    let kim_id = user_id(&a, "kim");
    // a plain user is refused
    let r = get(
        &a,
        &format!("/api/users/devices?user={kim_id}"),
        &k1.header(),
    )
    .await;
    assert_eq!(r.status, 403);
    let r = post(
        &a,
        "/api/users/devices/signout",
        &k1.header(),
        json!({"user": kim_id, "id": null}),
    )
    .await;
    assert_eq!(r.status, 403);
    // the list
    let r = get(
        &a,
        &format!("/api/users/devices?user={kim_id}"),
        &admin.header(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = r.json();
    assert_eq!(keys(&v), ["user", "devices", "signinInfo"]);
    assert_eq!(
        v["user"],
        json!({"id": kim_id, "username": "kim", "displayName": format!("User {kim_id}")})
    );
    assert_eq!(v["devices"].as_array().map(Vec::len), Some(2));
    assert!(
        v["devices"]
            .as_array()
            .expect("devices")
            .iter()
            .all(|d| d["current"] == false),
        "no row is current in somebody else's list"
    );
    // bad queries and unknown users
    for q in [
        "",
        "?user=",
        "?user=abc",
        "?user=1&user=2",
        "?user=1234567890123456789",
        "?user=-1",
    ] {
        let r = get(&a, &format!("/api/users/devices{q}"), &admin.header()).await;
        assert_eq!(r.status, 400, "{q}: {}", r.text());
    }
    assert_eq!(
        get(&a, "/api/users/devices?user=9999", &admin.header())
            .await
            .status,
        404
    );
    // one device
    let d1 = device_of_session(&a, &k1.session).expect("d1");
    let r = post(
        &a,
        "/api/users/devices/signout",
        &admin.header(),
        json!({"user": kim_id, "id": d1}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(keys(&r.json()), ["user", "devices", "signinInfo"]);
    assert!(!session_exists(&a, &k1.session) && session_exists(&a, &k2.session));
    // a device of another user, an unknown user, a bad body
    let admin_device = device_of_session(&a, &admin.session).expect("admin device");
    let r = post(
        &a,
        "/api/users/devices/signout",
        &admin.header(),
        json!({"user": kim_id, "id": admin_device}),
    )
    .await;
    assert_eq!(r.status, 404);
    assert_eq!(r.json()["code"], "devices.notFound");
    let r = post(
        &a,
        "/api/users/devices/signout",
        &admin.header(),
        json!({"user": 9999, "id": null}),
    )
    .await;
    assert_eq!(r.status, 404);
    for bad in [
        json!({"user": kim_id}),
        json!({"user": "2", "id": null}),
        json!({"user": kim_id, "id": "1"}),
    ] {
        let r = post(&a, "/api/users/devices/signout", &admin.header(), bad).await;
        assert_eq!(r.status, 400, "{}", r.text());
    }
    // every device
    let r = post(
        &a,
        "/api/users/devices/signout",
        &admin.header(),
        json!({"user": kim_id, "id": null}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(!session_exists(&a, &k2.session));
    let rows = entries(&a, "users.signoutDevice").await;
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.0.as_deref() == Some("kim")));
    assert_eq!(
        serde_json::from_str::<Value>(rows[1].1.as_deref().expect("detail")).expect("json"),
        json!({"all": true, "count": 1})
    );
}

#[tokio::test]
async fn an_administrator_never_signs_out_their_own_current_device_here() {
    let a = app();
    let mine = browser(&a, "admin").await;
    let other = browser(&a, "admin").await;
    let admin_id = user_id(&a, "admin");
    let current = device_of_session(&a, &mine.session).expect("current");
    let r = post(
        &a,
        "/api/users/devices/signout",
        &mine.header(),
        json!({"user": admin_id, "id": current}),
    )
    .await;
    assert_eq!(r.status, 409, "{}", r.text());
    assert_eq!(r.json()["code"], "devices.current");
    // "every device" of their own account keeps the current one
    let r = post(
        &a,
        "/api/users/devices/signout",
        &mine.header(),
        json!({"user": admin_id, "id": null}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(session_exists(&a, &mine.session));
    assert!(!session_exists(&a, &other.session));
    // and their own list marks the current device
    let r = get(
        &a,
        &format!("/api/users/devices?user={admin_id}"),
        &mine.header(),
    )
    .await;
    assert_eq!(r.json()["devices"][0]["current"], true);
}

#[tokio::test]
async fn the_users_list_counts_the_devices_as_its_last_key() {
    let a = app();
    let admin = browser(&a, "admin").await;
    let _k1 = browser(&a, "kim").await;
    let _k2 = browser(&a, "kim").await;
    let v = get(&a, "/api/users", &admin.header()).await.json();
    let row = |name: &str| -> Value {
        v["users"]
            .as_array()
            .expect("users")
            .iter()
            .find(|u| u["username"] == name)
            .cloned()
            .expect("row")
    };
    assert_eq!(row("kim")["devices"], 2);
    assert_eq!(row("lee")["devices"], 0);
    assert_eq!(row("admin")["devices"], 1);
    assert_eq!(
        keys(&row("kim")).last().map(String::as_str),
        Some("devices")
    );
}

// ---------------------------------------------------------------- the eye, clear, place

#[tokio::test]
async fn clearing_the_saved_addresses_clears_the_device_records_and_keeps_the_device_text() {
    let a = app();
    eye(&a, true);
    let admin = login_with(
        &a,
        "admin",
        None,
        &[
            ("user-agent", UA),
            ("x-real-ip", IP),
            ("x-geo-country", "ZZ"),
        ],
    )
    .await;
    let admin_session = pair(&admin, "xexam_session").expect("session");
    let (ip, ua): (Option<String>, Option<String>) = raw(&a.db)
        .query_row(
            "SELECT signin_ip, ua FROM devices WHERE user_id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row");
    assert_eq!(ip.as_deref(), Some(IP));
    assert_eq!(ua.as_deref(), Some(UA));
    let r = post(
        &a,
        "/api/users/signin-info/clear",
        &admin_session,
        json!({}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let (a_ip, s_country, ua): (Option<String>, Option<String>, Option<String>) = raw(&a.db)
        .query_row(
            "SELECT active_ip, signin_country, ua FROM devices WHERE user_id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("row");
    assert_eq!((a_ip, s_country), (None, None));
    assert_eq!(ua.as_deref(), Some(UA), "the device text is not an address");
}

fn device_with_address(a: &App, ip: &str, signin: bool) -> i64 {
    let c = raw(&a.db);
    let n: i64 = c
        .query_row("SELECT COUNT(*) FROM devices", [], |r| r.get(0))
        .expect("count");
    let (col_ip, col_at) = if signin {
        ("signin_ip", "signin_at")
    } else {
        ("active_ip", "signin_at")
    };
    c.execute(
        &format!(
            "INSERT INTO devices(user_id, cookie_hash, created_at, active_at, {col_ip}, {col_at}) VALUES(2, ?1, 1000, 1000, ?2, 1000)"
        ),
        rusqlite::params![format!("{:064x}", n + 100), ip],
    )
    .expect("device");
    c.last_insert_rowid()
}

fn place() -> Place {
    Place {
        country: Some("ZZ".into()),
        region: Some("Region One".into()),
        city: None,
    }
}

#[test]
fn geo_apply_devices_fills_only_placeless_devices_under_the_eye_and_the_api_source() {
    let a = app();
    eye(&a, true);
    source(&a, true);
    let sign = device_with_address(&a, IP, true);
    let act = device_with_address(&a, IP, false);
    // an empty place or no ids writes nothing
    assert!(!a
        .store
        .geo_apply_devices(IP, &Place::default(), 0, &[sign], &[act])
        .expect("empty"));
    assert!(!a
        .store
        .geo_apply_devices(IP, &place(), 0, &[], &[])
        .expect("no ids"));
    // a different address than the stored one never matches
    assert!(!a
        .store
        .geo_apply_devices("198.51.100.1", &place(), 0, &[sign], &[act])
        .expect("other ip"));
    assert!(a
        .store
        .geo_apply_devices(IP, &place(), 0, &[sign], &[act])
        .expect("apply"));
    let read = |a: &App,
                id: i64|
     -> (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        raw(&a.db)
            .query_row(
                "SELECT signin_country, signin_region, active_country, active_region FROM devices WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .expect("row")
    };
    // `signin` ids fill the signin_* columns, `active` ids the active_* columns
    assert_eq!(read(&a, sign).0.as_deref(), Some("ZZ"));
    assert_eq!(read(&a, sign).2, None);
    assert_eq!(read(&a, act).2.as_deref(), Some("ZZ"));
    // never overwritten
    let other = Place {
        country: Some("YY".into()),
        ..Place::default()
    };
    assert!(!a
        .store
        .geo_apply_devices(IP, &other, 0, &[sign], &[act])
        .expect("again"));
    assert_eq!(read(&a, sign).0.as_deref(), Some("ZZ"));
}

#[test]
fn geo_apply_devices_writes_nothing_with_the_eye_closed_or_the_cloudflare_source() {
    let a = app();
    let sign = device_with_address(&a, IP, true);
    // the eye is closed by default; the source is `cloudflare` by default
    source(&a, true);
    assert!(!a
        .store
        .geo_apply_devices(IP, &place(), 0, &[sign], &[])
        .expect("eye closed"));
    eye(&a, true);
    source(&a, false);
    assert!(!a
        .store
        .geo_apply_devices(IP, &place(), 0, &[sign], &[])
        .expect("cloudflare"));
    assert_eq!(
        count(
            &a,
            "SELECT COUNT(*) FROM devices WHERE signin_country IS NOT NULL"
        ),
        0
    );
}

#[test]
fn the_backfill_lane_lists_and_fills_device_records_without_a_place() {
    let a = app();
    eye(&a, true);
    source(&a, true);
    let _sign = device_with_address(&a, "203.0.113.50", true);
    let _act = device_with_address(&a, "203.0.113.51", false);
    // a device that already has its place is not listed
    let placed = device_with_address(&a, "203.0.113.52", true);
    raw(&a.db)
        .execute(
            "UPDATE devices SET signin_country = 'ZZ' WHERE id = ?1",
            [placed],
        )
        .expect("place");
    let missing = a.store.geo_missing_addresses().expect("missing");
    assert!(missing.contains(&"203.0.113.50".to_string()), "{missing:?}");
    assert!(missing.contains(&"203.0.113.51".to_string()), "{missing:?}");
    assert!(
        !missing.contains(&"203.0.113.52".to_string()),
        "{missing:?}"
    );
    assert!(a
        .store
        .geo_fill_all("203.0.113.50", &place())
        .expect("fill"));
    assert!(a
        .store
        .geo_fill_all("203.0.113.51", &place())
        .expect("fill"));
    assert!(!a
        .store
        .geo_missing_addresses()
        .expect("missing")
        .contains(&"203.0.113.50".to_string()));
    // an existing place is never overwritten
    let other = Place {
        country: Some("YY".into()),
        ..Place::default()
    };
    assert!(!a.store.geo_fill_all("203.0.113.50", &other).expect("again"));
    assert_eq!(
        count(
            &a,
            "SELECT COUNT(*) FROM devices WHERE signin_country = 'ZZ'"
        ),
        2
    );
}

#[test]
fn the_backfill_matches_the_ipv4_mapped_form() {
    let a = app();
    eye(&a, true);
    source(&a, true);
    let id = device_with_address(&a, "::ffff:203.0.113.60", true);
    assert!(a
        .store
        .geo_fill_all("203.0.113.60", &place())
        .expect("fill"));
    let country: Option<String> = raw(&a.db)
        .query_row(
            "SELECT signin_country FROM devices WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .expect("row");
    assert_eq!(country.as_deref(), Some("ZZ"));
}

#[test]
fn the_backfill_touches_nothing_with_the_eye_closed_or_the_cloudflare_source() {
    let a = app();
    let _id = device_with_address(&a, "203.0.113.70", true);
    source(&a, true);
    assert!(!a
        .store
        .geo_fill_all("203.0.113.70", &place())
        .expect("eye closed"));
    eye(&a, true);
    source(&a, false);
    assert!(!a
        .store
        .geo_fill_all("203.0.113.70", &place())
        .expect("cloudflare"));
    assert_eq!(
        count(
            &a,
            "SELECT COUNT(*) FROM devices WHERE signin_country IS NOT NULL"
        ),
        0
    );
}

#[tokio::test]
async fn a_new_sign_in_on_a_shared_browser_drops_the_previous_accounts_push_subscription() {
    use xexam_backend::database::store::PushSubscribe;
    let a = app();
    let kim = browser(&a, "kim").await;
    let kim_id = user_id(&a, "kim");
    let dev = device_of_session(&a, &kim.session).expect("device");
    let req = PushSubscribe {
        endpoint: "https://push.example.com/route-shared".to_string(),
        p256dh: "p".repeat(87),
        auth: "a".repeat(22),
        key_id: "0123456789abcdef".to_string(),
        expires_at: None,
    };
    a.store
        .push_subscribe(kim_id, dev, &req, 1_000)
        .expect("subscribe");
    assert_eq!(count(&a, "SELECT COUNT(*) FROM push_subscriptions"), 1);
    // The session of kim ends without a sign-out; lee signs in on the same browser
    raw(&a.db)
        .execute("DELETE FROM sessions", [])
        .expect("expire");
    let r = login_with(&a, "lee", Some(&kim.did), &[("user-agent", UA)]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(count(&a, "SELECT COUNT(*) FROM push_subscriptions"), 0);
    assert_eq!(count(&a, "SELECT COUNT(*) FROM push_outbox"), 0);
}
