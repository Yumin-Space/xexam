//! v1.3: the activity log (write side and routes): the entry of every POST route, the visit rules, `GET /api/activity`, the
//! sign-in info switch (display only) and the explicit clearing of addresses and locations, the events of the AI service, flood guards, retention
//! and the marker-secret check. Made-up users, example addresses and a fake internal token only.
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
use std::sync::Arc;
use xexam_backend::activity::{Actor, Entry, Sink};
use xexam_backend::api::internal::InternalToken;
use xexam_backend::api::{build_router, AppState};
use xexam_backend::auth::{self, Role};
use xexam_backend::database::{
    ActivityFilter, ActivityRow, ActivitySource, Merge, NewSession, Store, UserChange,
};
use xexam_backend::logs::{BufferLayer, LogBuffer};
use xexam_backend::models::BankAccess;
use xexam_backend::services::RealEngine;

/// Every test of this binary runs alone. Two of them capture log lines with a per-thread subscriber (`set_default`), and
/// tracing's callsite interest cache races when other tests' subscribers and log lines come and go on other threads: the
/// handler's line was not captured on CI (as in v08_ai_link.rs). A tokio Mutex has no poisoning (a failed test does not
/// fail the others) and may be held across `.await`.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";
const TOKEN: &str = "fake-internal-token-for-tests-0123456789";
const MARKER: &str = "MARKER-SECRET-7731";

struct App {
    router: Router,
    state: AppState,
    store: Arc<Store>,
    db: PathBuf,
    _dir: tempfile::TempDir,
}

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
        Arc::new(LogBuffer::default()),
    );
    assert!(state
        .0
        .ai_link
        .set_token(InternalToken::new(TOKEN).expect("token")));
    App {
        router: build_router(state.clone()),
        state,
        store,
        db,
        _dir: dir,
    }
}

fn raw(p: &Path) -> Connection {
    Connection::open(p).expect("raw open")
}

/// Creates a made-up user (never signs in through the API) and returns its id.
fn add_user(a: &App, name: &str, role: Role) -> i64 {
    match a
        .store
        .create_user_opts(
            name,
            "placeholder-hash",
            role,
            &BankAccess::Only(Vec::new()),
            false,
            false,
            false,
        )
        .expect("create")
    {
        UserChange::Done(id) => id,
        other => panic!("{other:?}"),
    }
}

/// A session for user `id` (as the sign-in route starts one); returns the `Cookie` value and the token.
fn session(a: &App, id: i64) -> (String, String) {
    let u = a.store.user_by_id(id).expect("lookup").expect("user");
    // clears a pending password change (the default administrator starts with one)
    a.store
        .change_own_password(
            id,
            &u.password_hash,
            &u.password_hash,
            &auth::token_hash(&auth::new_session_token()),
            auth::unix_now(),
        )
        .expect("flag")
        .expect("user");
    a.store
        .set_display_name(id, &format!("User {}", id))
        .expect("name");
    let token = auth::new_session_token();
    let now = auth::unix_now();
    a.store
        .create_session(&NewSession {
            token_hash: &auth::token_hash(&token),
            user_id: id,
            now,
            expires_at: now + 3600,
            remember: false,
            ip: None,
            ua: None,
        })
        .expect("session");
    (format!("{}={token}", auth::SESSION_COOKIE), token)
}

fn admin(a: &App) -> String {
    session(a, 1).0
}

async fn get(a: &App, path: &str, cookie: &str) -> Resp {
    // Two tests capture log lines with a per-thread subscriber; rebuilding the callsite interest cache before every
    // request keeps another thread's subscriber from leaving a cached "never" (see v08_ai_link.rs)
    tracing::callsite::rebuild_interest_cache();
    send(
        &a.router,
        build(
            "GET",
            path,
            Some(HOST),
            &[("cookie", cookie.as_bytes())],
            Body::empty(),
        ),
    )
    .await
}

async fn post_h(a: &App, path: &str, cookie: &str, body: &Value, extra: &[(&str, &[u8])]) -> Resp {
    // Two tests capture log lines with a per-thread subscriber; rebuilding the callsite interest cache before every
    // request keeps another thread's subscriber from leaving a cached "never" (see v08_ai_link.rs)
    tracing::callsite::rebuild_interest_cache();
    let text = body.to_string();
    let len = text.len().to_string();
    let mut hs: Vec<(&str, &[u8])> = vec![
        ("cookie", cookie.as_bytes()),
        ("origin", ORIGIN.as_bytes()),
        ("content-type", b"application/json"),
        ("content-length", len.as_bytes()),
    ];
    hs.extend_from_slice(extra);
    send(
        &a.router,
        build("POST", path, Some(HOST), &hs, Body::from(text)),
    )
    .await
}

async fn post(a: &App, path: &str, cookie: &str, body: &Value) -> Resp {
    post_h(a, path, cookie, body, &[]).await
}

async fn internal(a: &App, body: &Value) -> Resp {
    let text = body.to_string();
    let len = text.len().to_string();
    send(
        &a.router,
        build(
            "POST",
            "/internal/ai/activity",
            Some("backend:8765"),
            &[
                ("content-type", b"application/json"),
                ("content-length", len.as_bytes()),
                ("x-xexam-internal-token", TOKEN.as_bytes()),
            ],
            Body::from(text),
        ),
    )
    .await
}

/// All rows, oldest first, as (action, object, result, reason).
async fn rows(a: &App) -> Vec<(String, Option<String>, bool, Option<String>)> {
    a.state.0.activity.flush().await;
    let mut p = a
        .store
        .activity_page(&ActivityFilter {
            limit: Some(200),
            ..Default::default()
        })
        .expect("page")
        .entries;
    p.reverse();
    p.into_iter()
        .map(|e| (e.action, e.object, e.ok, e.reason))
        .collect()
}

fn count(a: &App, sql: &str) -> i64 {
    raw(&a.db).query_row(sql, [], |r| r.get(0)).expect("count")
}

fn ai_event(id: &str, extra: Value) -> Value {
    let mut base = json!({
        "id": id,
        "at": auth::unix_now(),
        "userId": 1,
        "stamp": "x",
        "username": "admin",
        "action": "ai.settings.limits",
        "object": null,
        "detail": {"keys": ["messagesPerChat"]},
        "result": "ok",
        "reason": null,
        "ip": "203.0.113.7",
        "ua": "Agent/1.0"
    });
    if let (Some(b), Value::Object(x)) = (base.as_object_mut(), extra) {
        for (k, v) in x {
            b.insert(k, v);
        }
    }
    base
}

const ID1: &str = "00000000000000000000000000000001";
const ID2: &str = "00000000000000000000000000000002";
const ID3: &str = "00000000000000000000000000000003";

// ---------------------------------------------------------------- the POST hook

#[tokio::test]
async fn admin_posts_are_recorded_users_only_when_important() {
    let _serial = SERIAL.lock().await;
    let a = app();
    let adm = admin(&a);
    let bob = add_user(&a, "bob", Role::User);
    let (bc, _) = session(&a, bob);
    // admin: a settings change names its keys; a bad value is a failed entry
    let r = post(&a, "/api/settings", &adm, &json!({"theme": "dark"})).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = post(&a, "/api/settings", &adm, &json!({"theme": 5})).await;
    assert_eq!(r.status, 400);
    // user: settings are not important, a progress reset is
    assert_eq!(
        post(&a, "/api/settings", &bc, &json!({"theme": "dark"}))
            .await
            .status,
        200
    );
    assert_eq!(post(&a, "/api/reset", &bc, &json!({})).await.status, 200);
    // an unlisted admin POST
    assert_eq!(post(&a, "/api/nothing", &adm, &json!({})).await.status, 404);
    let got = rows(&a).await;
    assert_eq!(
        got,
        vec![
            ("settings.update".to_string(), None, true, None),
            (
                "settings.update".to_string(),
                None,
                false,
                Some("badRequest".to_string())
            ),
            ("progress.reset".to_string(), None, true, None),
            (
                "admin.request".to_string(),
                Some("/api/nothing".to_string()),
                false,
                Some("notFound".to_string())
            ),
        ]
    );
    let c = raw(&a.db);
    let who: String = c
        .query_row(
            "SELECT group_concat(username || ':' || role) FROM activity_log WHERE action='progress.reset'",
            [],
            |r| r.get(0),
        )
        .expect("who");
    assert_eq!(who, "bob:user");
    let detail: String = c
        .query_row(
            "SELECT detail FROM activity_log WHERE action='settings.update' AND result='ok'",
            [],
            |r| r.get(0),
        )
        .expect("detail");
    assert_eq!(detail, r#"{"keys":["theme"]}"#);
}

#[tokio::test]
async fn user_management_entries_name_the_target_and_the_change() {
    let _serial = SERIAL.lock().await;
    let a = app();
    let adm = admin(&a);
    let r = post(
        &a,
        "/api/users/create",
        &adm,
        &json!({"username": "cora", "password": "made-up-pass-1", "role": "user"}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let id = a.store.user_by_name("cora").expect("db").expect("cora").id;
    let r = post(
        &a,
        "/api/users/update",
        &adm,
        &json!({"id": id, "username": "cora2", "canManageBanks": true}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = post(&a, "/api/users/delete", &adm, &json!({"id": id})).await;
    assert_eq!(r.status, 200, "{}", r.text());
    a.state.0.activity.flush().await;
    let c = raw(&a.db);
    let row = |action: &str| -> (Option<String>, Option<String>) {
        c.query_row(
            "SELECT object, detail FROM activity_log WHERE action=?1",
            [action],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row")
    };
    assert_eq!(row("users.create").0.as_deref(), Some("cora"));
    assert!(row("users.create")
        .1
        .expect("detail")
        .contains("\"role\":\"user\""));
    let (obj, detail) = row("users.update");
    assert_eq!(obj.as_deref(), Some("cora2"));
    let d = detail.expect("detail");
    assert!(
        d.contains("\"oldName\":\"cora\"") && d.contains("canManageBanks"),
        "{d}"
    );
    assert_eq!(row("users.delete").0.as_deref(), Some("cora2"));
}

#[tokio::test]
async fn bank_and_study_entries_carry_their_object_and_counts() {
    let _serial = SERIAL.lock().await;
    let a = app();
    let adm = admin(&a);
    let r = post(
        &a,
        "/api/records/delete",
        &adm,
        &json!({"all": true, "withStats": false}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = post(&a, "/api/import/cancel", &adm, &json!({})).await;
    assert_eq!(r.status, 200);
    let got = rows(&a).await;
    assert_eq!(got[0].0, "records.delete");
    assert_eq!(got[1].0, "bank.importCancel");
    let d: String = raw(&a.db)
        .query_row(
            "SELECT detail FROM activity_log WHERE action='records.delete'",
            [],
            |r| r.get(0),
        )
        .expect("detail");
    assert_eq!(d, r#"{"count":0}"#);
}

// ---------------------------------------------------------------- visits

#[tokio::test]
async fn visits_follow_role_switch_merge_and_guard() {
    let _serial = SERIAL.lock().await;
    let a = app();
    let adm = admin(&a);
    let bob = add_user(&a, "bob", Role::User);
    let (bc, _) = session(&a, bob);
    // unknown page: 400, nothing written
    assert_eq!(
        post(&a, "/api/activity/visit", &adm, &json!({"page": "nope"}))
            .await
            .status,
        400
    );
    assert_eq!(
        post(&a, "/api/activity/visit", &adm, &json!({"page": 3}))
            .await
            .status,
        400
    );
    // admin visits are logged and merge (same page within 30 minutes)
    for _ in 0..3 {
        let r = post(&a, "/api/activity/visit", &adm, &json!({"page": "home"})).await;
        assert_eq!(r.json(), json!({"ok": true}));
    }
    post(&a, "/api/activity/visit", &adm, &json!({"page": "logs"})).await;
    // a plain user's visit: 200 and nothing while the switch is off
    assert_eq!(
        post(&a, "/api/activity/visit", &bc, &json!({"page": "home"}))
            .await
            .status,
        200
    );
    let _ = rows(&a).await;
    assert_eq!(
        count(&a, "SELECT COUNT(*) FROM activity_log WHERE username='bob'"),
        0
    );
    assert_eq!(
        count(&a, "SELECT count FROM activity_log WHERE object='home'"),
        3
    );
    // the switch on: the user's visits are logged
    let r = post(
        &a,
        "/api/activity/settings",
        &adm,
        &json!({"userVisits": true}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["userVisits"], json!(true));
    post(&a, "/api/activity/visit", &bc, &json!({"page": "banks"})).await;
    let _ = rows(&a).await;
    assert_eq!(
        count(&a, "SELECT COUNT(*) FROM activity_log WHERE username='bob'"),
        1
    );
    // the guard: at most 30 writes per user per minute (all distinct pages would not merge; the count stops at the cap)
    let pages = [
        "home",
        "banks",
        "users",
        "logs",
        "records",
        "record",
        "wrong",
        "settings",
        "about",
        "quiz",
        "result",
        "ai",
        "aiSettings",
    ];
    for i in 0..60 {
        post(
            &a,
            "/api/activity/visit",
            &bc,
            &json!({"page": pages[i % pages.len()]}),
        )
        .await;
    }
    let _ = rows(&a).await;
    let total: i64 = count(
        &a,
        "SELECT SUM(count) FROM activity_log WHERE username='bob' AND action='page.view'",
    );
    // 1 before the loop + at most 29 more within the same minute (a minute boundary may allow a few more)
    assert!((30..=60).contains(&total), "{total}");
    assert!(total < 61);
}

// v2.5: `study` is a visit page (the exam setup moved off `home`); `dashboard` is an API name, not a page
#[tokio::test]
async fn visit_pages_include_study_but_not_dashboard() {
    let _serial = SERIAL.lock().await;
    let a = app();
    let adm = admin(&a);
    assert_eq!(
        post(&a, "/api/activity/visit", &adm, &json!({"page": "study"}))
            .await
            .status,
        200
    );
    assert_eq!(
        post(
            &a,
            "/api/activity/visit",
            &adm,
            &json!({"page": "dashboard"})
        )
        .await
        .status,
        400
    );
}

// ---------------------------------------------------------------- reading

#[tokio::test]
async fn activity_read_filters_pages_and_facets() {
    let _serial = SERIAL.lock().await;
    let a = app();
    let adm = admin(&a);
    let bob = add_user(&a, "bob", Role::User);
    let (bc, _) = session(&a, bob);
    for i in 0..5 {
        post(
            &a,
            "/api/settings",
            &adm,
            &json!({"theme": if i % 2 == 0 { "dark" } else { "light" }}),
        )
        .await;
    }
    post(&a, "/api/settings", &adm, &json!({"theme": 9})).await;
    post(&a, "/api/reset", &bc, &json!({})).await;
    post(&a, "/api/activity/visit", &adm, &json!({"page": "logs"})).await;
    a.state.0.activity.flush().await;
    // non-admin: refused
    assert_eq!(get(&a, "/api/activity", &bc).await.status, 403);
    assert_eq!(get(&a, "/api/activity/settings", &bc).await.status, 403);
    // full page: visits are hidden without the filter
    let r = get(&a, "/api/activity?facets=1", &adm).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let j = r.json();
    let entries = j["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 7);
    assert!(entries.iter().all(|e| e["action"] != "page.view"));
    assert_eq!(entries[0]["action"], "progress.reset");
    assert_eq!(entries[0]["username"], "bob");
    assert_eq!(entries[0]["source"], "backend");
    assert_eq!(j["settings"]["retentionDays"], json!(90));
    assert_eq!(j["settings"]["userVisits"], json!(false));
    assert_eq!(j["settings"]["signinInfo"], json!(false));
    let names: Vec<&str> = j["facets"]["users"]
        .as_array()
        .expect("facets")
        .iter()
        .map(|u| u["username"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(names, vec!["admin", "bob"]);
    // filters
    let r = get(&a, "/api/activity?group=visit", &adm).await.json();
    assert_eq!(r["entries"].as_array().map(Vec::len), Some(1));
    let r = get(&a, "/api/activity?result=fail", &adm).await.json();
    assert_eq!(r["entries"].as_array().map(Vec::len), Some(1));
    let r = get(&a, &format!("/api/activity?user={bob}"), &adm)
        .await
        .json();
    assert_eq!(r["entries"].as_array().map(Vec::len), Some(1));
    let r = get(&a, "/api/activity?action=settings.update&limit=2", &adm)
        .await
        .json();
    assert_eq!(r["entries"].as_array().map(Vec::len), Some(2));
    let next = r["next"].as_i64().expect("next");
    let r2 = get(
        &a,
        &format!("/api/activity?action=settings.update&limit=10&before={next}"),
        &adm,
    )
    .await
    .json();
    assert_eq!(r2["entries"].as_array().map(Vec::len), Some(4));
    assert!(r2["next"].is_null());
    // newer than what was seen
    let newest = j["newest"].as_i64().expect("newest");
    let none = get(&a, &format!("/api/activity?after={newest}"), &adm)
        .await
        .json();
    assert_eq!(none["entries"].as_array().map(Vec::len), Some(0));
    assert_eq!(none["newest"].as_i64(), Some(newest));
    // bad queries
    for bad in [
        "limit=0",
        "limit=500",
        "group=x",
        "result=maybe",
        "user=bob",
        "before=x",
    ] {
        assert_eq!(
            get(&a, &format!("/api/activity?{bad}"), &adm).await.status,
            400,
            "{bad}"
        );
    }
    // the settings route
    let s = get(&a, "/api/activity/settings", &adm).await.json();
    assert_eq!(s["retentionDays"], json!(90));
}

#[tokio::test]
async fn reading_the_log_is_not_logged_and_settings_are_checked() {
    let _serial = SERIAL.lock().await;
    let a = app();
    let adm = admin(&a);
    get(&a, "/api/activity", &adm).await;
    get(&a, "/api/activity/settings", &adm).await;
    assert!(rows(&a).await.is_empty());
    for bad in [
        json!({"retentionDays": 6}),
        json!({"retentionDays": 3651}),
        json!({"retentionDays": "90"}),
        json!({"userVisits": 1}),
    ] {
        assert_eq!(
            post(&a, "/api/activity/settings", &adm, &bad).await.status,
            400,
            "{bad}"
        );
    }
    let r = post(
        &a,
        "/api/activity/settings",
        &adm,
        &json!({"retentionDays": 30}),
    )
    .await;
    assert_eq!(r.json()["retentionDays"], json!(30));
    assert_eq!(a.store.activity_retention_days().expect("days"), 30);
    let got = rows(&a).await;
    assert_eq!(
        got.iter().filter(|r| r.0 == "logs.settings" && r.2).count(),
        1
    );
    assert_eq!(got.iter().filter(|r| !r.2).count(), 4);
    let d: String = raw(&a.db)
        .query_row(
            "SELECT detail FROM activity_log WHERE action='logs.settings' AND result='ok'",
            [],
            |r| r.get(0),
        )
        .expect("detail");
    assert_eq!(d, r#"{"retentionDays":30,"userVisits":false}"#);
}

// ---------------------------------------------------------------- the sign-in info switch

#[tokio::test]
async fn sign_in_info_switch_stops_recording_hides_and_the_clear_wipes_them() {
    let _serial = SERIAL.lock().await;
    let a = app();
    let adm = admin(&a);
    let geo: Vec<(&str, &[u8])> = vec![
        ("x-real-ip", b"203.0.113.7"),
        ("x-geo-country", b"zz"),
        ("x-geo-region", b"Example Region"),
        ("x-geo-city", b"Example City"),
        ("user-agent", b"Agent/1.0"),
    ];
    const WITH_INFO: &str = "SELECT COUNT(*) FROM activity_log WHERE ip IS NOT NULL OR country IS NOT NULL OR region IS NOT NULL OR city IS NOT NULL";
    // closed (default): no address and no location is recorded, the agent is
    post_h(&a, "/api/settings", &adm, &json!({"theme": "dark"}), &geo).await;
    let _ = rows(&a).await;
    assert_eq!(count(&a, WITH_INFO), 0);
    assert_eq!(
        count(&a, "SELECT COUNT(*) FROM activity_log WHERE ua='Agent/1.0'"),
        1
    );
    // an AI event while closed: its address is not stored either
    let r = internal(&a, &json!({"events": [ai_event(ID1, json!({}))]})).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(count(&a, WITH_INFO), 0);
    // no answer shows anything while the switch is closed
    let j = get(&a, "/api/activity", &adm).await.json();
    assert_eq!(j["settings"]["signinInfo"], json!(false));
    assert!(j["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .all(|e| e["ip"].is_null() && e["location"].is_null()));
    let j = get(&a, "/api/users", &adm).await.json();
    assert!(j["users"]
        .as_array()
        .expect("users")
        .iter()
        .all(|u| u["lastLoginIp"].is_null() && u["lastLoginLocation"].is_null()));
    // open: recording resumes; what was recorded while closed simply has no address
    let r = post(
        &a,
        "/api/users/signin-info",
        &adm,
        &json!({"enabled": true}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["signinInfo"], json!(true));
    let j = get(&a, "/api/activity", &adm).await.json();
    let shown = j["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .filter(|e| e["action"] == "settings.update" && e["ip"] == "203.0.113.7")
        .count();
    assert_eq!(
        shown, 0,
        "the row written while the switch was closed has none"
    );
    post_h(&a, "/api/settings", &adm, &json!({"theme": "light"}), &geo).await;
    // The event carries the Cloudflare place of its request, cleaned like the headers (XX = no country)
    let ev = ai_event(
        ID2,
        json!({"country": "zz", "region": "Example Region", "city": "Example\u{7} City"}),
    );
    let bad = ai_event(
        ID3,
        json!({"country": "XX", "region": "  ", "ip": "::ffff:203.0.113.8"}),
    );
    let r = internal(&a, &json!({"events": [ev, bad]})).await;
    assert_eq!(r.status, 200);
    let _ = rows(&a).await;
    let c = raw(&a.db);
    let (ip, country, region, city): (String, String, String, String) = c
        .query_row(
            "SELECT ip, country, region, city FROM activity_log WHERE action='settings.update' AND ip IS NOT NULL ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("geo row");
    assert_eq!(
        (
            ip.as_str(),
            country.as_str(),
            region.as_str(),
            city.as_str()
        ),
        ("203.0.113.7", "ZZ", "Example Region", "Example City")
    );
    // In Cloudflare mode an AI event gets the place it carries, with the same cleaning as the request headers
    let (ip, country, region, city): (String, Option<String>, Option<String>, Option<String>) = c
        .query_row(
            "SELECT ip, country, region, city FROM activity_log WHERE action='ai.settings.limits' AND src_id=?1",
            [ID2],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("ai row");
    assert_eq!(
        (
            ip.as_str(),
            country.as_deref(),
            region.as_deref(),
            city.as_deref()
        ),
        (
            "203.0.113.7",
            Some("ZZ"),
            Some("Example Region"),
            Some("Example City")
        )
    );
    // a placeless event stays placeless; the address is stored in its plain IPv4 form
    let (ip, country, region): (String, Option<String>, Option<String>) = c
        .query_row(
            "SELECT ip, country, region FROM activity_log WHERE action='ai.settings.limits' AND src_id=?1",
            [ID3],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("ai row 3");
    assert_eq!((ip.as_str(), country, region), ("203.0.113.8", None, None));
    // the read answer shows them while on
    let j = get(&a, "/api/activity", &adm).await.json();
    assert_eq!(j["entries"][0]["ip"], json!("203.0.113.7"));
    // closed again: nothing stored is cleared, the answers hide it, nothing new is recorded
    let before = count(&a, WITH_INFO);
    assert!(before >= 2, "{before}");
    let r = post(
        &a,
        "/api/users/signin-info",
        &adm,
        &json!({"enabled": false}),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["signinInfo"], json!(false));
    post_h(&a, "/api/settings", &adm, &json!({"theme": "dark"}), &geo).await;
    let _ = rows(&a).await;
    assert_eq!(
        count(&a, WITH_INFO),
        before,
        "kept, and nothing new while closed"
    );
    let j = get(&a, "/api/activity", &adm).await.json();
    assert!(j["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .all(|e| e["ip"].is_null() && e["location"].is_null()));
    assert!(!a.state.0.activity.signin_info());
    // the switch is an entry, with its value
    let _ = rows(&a).await;
    let d: Vec<String> = {
        let mut st = c
            .prepare("SELECT detail FROM activity_log WHERE action='users.signinInfo' ORDER BY id")
            .expect("prepare");
        st.query_map([], |r| r.get(0))
            .expect("rows")
            .map(|x| x.expect("detail"))
            .collect()
    };
    assert_eq!(d, vec![r#"{"enabled":true}"#, r#"{"enabled":false}"#]);
    // the explicit clear works while the switch is off: users, sessions and the log hold no address or location
    {
        let c = raw(&a.db);
        c.execute(
            "UPDATE users SET last_login_ip='203.0.113.7', last_login_country='ZZ', last_login_region='Example Region', last_login_city='Example City' WHERE id=1",
            [],
        )
        .expect("user info");
        c.execute("UPDATE sessions SET ip='203.0.113.7'", [])
            .expect("session ip");
    }
    let stored = count(&a, WITH_INFO);
    let r = post(&a, "/api/users/signin-info/clear", &adm, &json!({})).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["signinInfo"], json!(false));
    assert!(r.json()["users"].is_array());
    let _ = rows(&a).await;
    assert_eq!(
        count(&a, "SELECT COUNT(*) FROM users WHERE last_login_ip IS NOT NULL OR last_login_country IS NOT NULL OR last_login_region IS NOT NULL OR last_login_city IS NOT NULL"),
        0
    );
    assert_eq!(
        count(&a, "SELECT COUNT(*) FROM sessions WHERE ip IS NOT NULL"),
        0
    );
    // only the entry of the clear itself may be newer than the clear (this test client sends no address)
    assert_eq!(count(&a, WITH_INFO), 0);
    let d: String = raw(&a.db)
        .query_row(
            "SELECT detail FROM activity_log WHERE action='users.signinInfoClear' AND result='ok'",
            [],
            |r| r.get(0),
        )
        .expect("clear entry");
    let rows_cleared = serde_json::from_str::<Value>(&d).expect("json")["rows"]
        .as_i64()
        .expect("rows");
    let sessions = count(&a, "SELECT COUNT(*) FROM sessions");
    assert_eq!(rows_cleared, stored + 1 + sessions);
    // on again after the clear: nothing old comes back, a second clear changes nothing
    let r = post(
        &a,
        "/api/users/signin-info",
        &adm,
        &json!({"enabled": true}),
    )
    .await;
    assert_eq!(r.status, 200);
    let j = get(&a, "/api/activity", &adm).await.json();
    assert!(j["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .all(|e| e["ip"].is_null() && e["location"].is_null()));
    let r = post(&a, "/api/users/signin-info/clear", &adm, &json!({})).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["signinInfo"], json!(true));
    let _ = rows(&a).await;
    assert_eq!(
        count(&a, "SELECT COUNT(*) FROM activity_log WHERE action='users.signinInfoClear' AND detail='{\"rows\":0}'"),
        1
    );
    // validation: not a bool, and a non-admin
    assert_eq!(
        post(
            &a,
            "/api/users/signin-info",
            &adm,
            &json!({"enabled": "yes"})
        )
        .await
        .status,
        400
    );
    let bob = add_user(&a, "bob", Role::User);
    let (bc, _) = session(&a, bob);
    assert_eq!(
        post(&a, "/api/users/signin-info", &bc, &json!({"enabled": true}))
            .await
            .status,
        403
    );
    // the clear has the same guards: administrators only, a session, the Origin check
    assert_eq!(
        post(&a, "/api/users/signin-info/clear", &bc, &json!({}))
            .await
            .status,
        403
    );
    assert_eq!(
        post(&a, "/api/users/signin-info/clear", "", &json!({}))
            .await
            .status,
        401
    );
    let r = send(
        &a.router,
        build(
            "POST",
            "/api/users/signin-info/clear",
            Some(HOST),
            &[
                ("cookie", adm.as_bytes()),
                ("origin", b"http://evil.example"),
                ("content-type", b"application/json"),
                ("content-length", b"2"),
            ],
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(r.status, 403, "{}", r.text());
    let r = get(&a, "/api/users/signin-info/clear", &adm).await;
    assert!(r.status == 404 || r.status == 405, "{}", r.status);
}

// ---------------------------------------------------------------- AI events

#[tokio::test]
async fn ai_events_are_checked_stored_once_and_attributed() {
    let _serial = SERIAL.lock().await;
    let a = app();
    let uid = a.store.user_by_id(1).expect("u").expect("admin").uid;
    // the whole batch is refused on the first bad event: nothing is stored
    let bad = json!({"events": [
        ai_event(ID1, json!({})),
        ai_event(ID2, json!({"action": "auth.signin"})),
    ]});
    assert_eq!(internal(&a, &bad).await.status, 400);
    assert_eq!(count(&a, "SELECT COUNT(*) FROM activity_log"), 0);
    // more than 50 events
    let many: Vec<Value> = (0..51)
        .map(|i| ai_event(&format!("{i:032x}"), json!({})))
        .collect();
    assert_eq!(internal(&a, &json!({"events": many})).await.status, 400);
    assert_eq!(internal(&a, &json!({})).await.status, 400);
    // a good batch: matching stamp = the backend's own account; mismatch = the sent name
    let good = json!({"events": [
        ai_event(ID1, json!({"stamp": uid, "username": "someone-else"})),
        ai_event(ID2, json!({"stamp": "wrong", "username": "old-name", "userId": 77})),
        ai_event(ID3, json!({"userId": null, "result": "fail", "reason": "ai.up.timeout"})),
    ]});
    let r = internal(&a, &good).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json(), json!({"ok": true}));
    // delivered again (outbox retry): nothing new
    assert_eq!(internal(&a, &good).await.status, 200);
    assert_eq!(count(&a, "SELECT COUNT(*) FROM activity_log"), 3);
    let c = raw(&a.db);
    let row = |id: &str| -> (String, Option<i64>, Option<String>, String, String) {
        c.query_row(
            "SELECT actor, user_id, username, source, result FROM activity_log WHERE src_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .expect("row")
    };
    assert_eq!(
        row(ID1),
        (
            "user".into(),
            Some(1),
            Some("admin".into()),
            "ai".into(),
            "ok".into()
        )
    );
    assert_eq!(
        row(ID2),
        (
            "user".into(),
            Some(77),
            Some("old-name".into()),
            "ai".into(),
            "ok".into()
        )
    );
    assert_eq!(
        row(ID3),
        ("system".into(), None, None, "ai".into(), "fail".into())
    );
    // the read side shows the deleted account's snapshot
    let adm = admin(&a);
    let j = get(&a, "/api/activity?group=ai", &adm).await.json();
    let gone = j["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|e| e["userId"] == json!(77))
        .expect("entry of the gone account");
    assert_eq!(gone["deleted"], json!(true));
    assert_eq!(gone["username"], json!("old-name"));
    // the route needs the token
    let r = send(
        &a.router,
        build(
            "POST",
            "/internal/ai/activity",
            Some("backend:8765"),
            &[("content-type", b"application/json")],
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(r.status, 404);
}

// ---------------------------------------------------------------- guards and retention

#[tokio::test]
async fn anonymous_flood_writes_at_most_120_rows() {
    let _serial = SERIAL.lock().await;
    let dir = dbc::tmp();
    let store = Arc::new(Store::open(&dir.path().join("x.db")).expect("store"));
    let sink = Sink::new();
    sink.start(store.clone());
    for _ in 0..300 {
        sink.record(Entry::new(Actor::Anon, "auth.signin").fail("unknownUser"));
    }
    sink.flush().await;
    let n: i64 = store
        .activity_page(&ActivityFilter {
            limit: Some(200),
            action: Some("auth.signin".into()),
            ..Default::default()
        })
        .expect("page")
        .entries
        .iter()
        .map(|e| e.count)
        .sum();
    // identical refused entries from one source merge into rows, but the cap is on what is written at all: at most 120
    assert!(n <= 120, "{n}");
    sink.stop().await;
}

#[tokio::test]
async fn writer_applies_retention_and_reports_it() {
    let _serial = SERIAL.lock().await;
    let dir = dbc::tmp();
    let store = Arc::new(Store::open(&dir.path().join("x.db")).expect("store"));
    let now = auth::unix_now();
    let old = ActivityRow {
        ts: now - 100 * 86_400,
        actor: "system".into(),
        user_id: None,
        username: None,
        role: None,
        action: "system.start".into(),
        object: None,
        detail: None,
        ok: true,
        reason: None,
        ip: None,
        country: None,
        region: None,
        city: None,
        ua: None,
        source: ActivitySource::Backend,
        src_id: None,
        merge: Merge::None,
    };
    let fresh = ActivityRow {
        ts: now - 86_400,
        ..old.clone()
    };
    store
        .activity_insert(&[old.clone(), old, fresh])
        .expect("insert");
    let sink = Sink::new();
    sink.start(store.clone());
    sink.flush().await;
    let p = store
        .activity_page(&ActivityFilter::default())
        .expect("page")
        .entries;
    let actions: Vec<(&str, i64)> = p.iter().map(|e| (e.action.as_str(), e.count)).collect();
    assert!(actions.contains(&("system.logPurge", 1)), "{actions:?}");
    assert_eq!(
        p.iter().filter(|e| e.action == "system.start").count(),
        1,
        "only the day-old row is kept"
    );
    let purge = p
        .iter()
        .find(|e| e.action == "system.logPurge")
        .expect("purge row");
    assert_eq!(purge.detail, Some(json!({"deleted": 2})));
    sink.stop().await;
}

#[test]
fn retention_never_deletes_sign_in_rows_and_the_cap_converges() {
    let _serial = SERIAL.blocking_lock();
    let dir = dbc::tmp();
    let store = Store::open(&dir.path().join("x.db")).expect("store");
    let now = auth::unix_now();
    let row = |action: &str, ts: i64, ok: bool| ActivityRow {
        ts,
        actor: "system".into(),
        user_id: None,
        username: None,
        role: None,
        action: action.to_string(),
        object: None,
        detail: None,
        ok,
        reason: None,
        ip: Some("203.0.113.5".into()),
        country: Some("ZZ".into()),
        region: None,
        city: None,
        ua: None,
        source: ActivitySource::Backend,
        src_id: None,
        merge: Merge::None,
    };
    let old = now - 400 * 86_400;
    let mut batch = Vec::new();
    for _ in 0..3 {
        batch.push(row("auth.signin", old, true));
        batch.push(row("auth.signin", old, false));
    }
    for _ in 0..4 {
        batch.push(row("settings.update", old, true));
        batch.push(row("page.view", now, true));
    }
    store.activity_insert(&batch).expect("insert");
    let before: i64 = raw(&dir.path().join("x.db"))
        .query_row("SELECT COUNT(*) FROM activity_log", [], |r| r.get(0))
        .expect("count");
    // days = 7 removes the 4 old non-sign-in rows; a tiny cap of 2 then trims the fresh ones, never the sign-ins
    let deleted = store.activity_retention(now, 7, 2).expect("retention");
    let c = raw(&dir.path().join("x.db"));
    let n = |sql: &str| -> i64 { c.query_row(sql, [], |r| r.get(0)).expect("count") };
    assert_eq!(
        n("SELECT COUNT(*) FROM activity_log WHERE action='auth.signin'"),
        6
    );
    assert_eq!(
        n("SELECT COUNT(*) FROM activity_log WHERE action='auth.signin' AND ip IS NOT NULL"),
        6
    );
    assert_eq!(
        n("SELECT COUNT(*) FROM activity_log WHERE action<>'auth.signin'"),
        2
    );
    assert_eq!(deleted, before - 8);
    // a second run has nothing left to do
    assert_eq!(store.activity_retention(now, 7, 2).expect("again"), 0);
    // cap 0: every non-sign-in row goes, the sign-ins stay, and the run ends
    store.activity_retention(now, 7, 0).expect("cap 0");
    assert_eq!(n("SELECT COUNT(*) FROM activity_log"), 6);
}

// ---------------------------------------------------------------- marker secret

#[tokio::test]
async fn secrets_never_reach_rows_or_log_lines() {
    let _serial = SERIAL.lock().await;
    use tracing_subscriber::layer::SubscriberExt;
    let lines = Arc::new(LogBuffer::new(1000));
    let _guard = tracing::subscriber::set_default(
        tracing_subscriber::registry().with(BufferLayer::new(lines.clone())),
    );
    let a = app();
    // tracing caches each callsite's interest when it is first hit; a hit racing with another test's subscriber on
    // another thread can cache "never", and the line is not captured (seen on CI, as in v08_ai_link.rs)
    tracing::callsite::rebuild_interest_cache();
    let adm = admin(&a);
    let token = adm.split('=').nth(1).expect("token").to_string();
    let secret_headers: Vec<(&str, &[u8])> = vec![
        ("authorization", MARKER.as_bytes()),
        ("x-api-key", MARKER.as_bytes()),
        ("x-real-ip", b"203.0.113.9"),
    ];
    // requests with the marker in a body field, in headers and in an invalid value
    post_h(
        &a,
        "/api/users/signin-info",
        &adm,
        &json!({"enabled": true, "password": MARKER, "apiKey": MARKER}),
        &secret_headers,
    )
    .await;
    post_h(
        &a,
        "/api/settings",
        &adm,
        &json!({"theme": MARKER}),
        &secret_headers,
    )
    .await;
    post_h(
        &a,
        "/api/activity/settings",
        &adm,
        &json!({"retentionDays": MARKER, "token": MARKER}),
        &secret_headers,
    )
    .await;
    post_h(
        &a,
        "/api/activity/visit",
        &adm,
        &json!({"page": MARKER}),
        &secret_headers,
    )
    .await;
    get(&a, &format!("/api/activity?action={MARKER}"), &adm).await;
    a.state.0.activity.flush().await;
    let c = raw(&a.db);
    let mut st = c
        .prepare(
            "SELECT COALESCE(username,'')||'|'||COALESCE(object,'')||'|'||COALESCE(detail,'')||'|'||COALESCE(reason,'')||'|'||COALESCE(ua,'')||'|'||COALESCE(ip,'') FROM activity_log",
        )
        .expect("prepare");
    let dump: Vec<String> = st
        .query_map([], |r| r.get(0))
        .expect("rows")
        .map(|x| x.expect("row"))
        .collect();
    assert!(!dump.is_empty());
    for line in &dump {
        assert!(!line.contains(MARKER), "{line}");
        assert!(!line.contains(&token), "{line}");
    }
    for l in lines.since(None).lines {
        assert!(!l.message.contains(MARKER), "{}", l.message);
        assert!(!l.message.contains(&token), "{}", l.message);
    }
    // the sign-in info switch is on (the first request worked), so the line may carry the address, and only that
    let logged: Vec<String> = lines
        .since(None)
        .lines
        .into_iter()
        .map(|l| l.message)
        .filter(|m| m.starts_with("activity "))
        .collect();
    assert!(logged.iter().any(|m| m.contains("action=users.signinInfo")));
}

#[tokio::test]
async fn log_line_has_no_address_while_the_switch_is_off() {
    let _serial = SERIAL.lock().await;
    use tracing_subscriber::layer::SubscriberExt;
    let lines = Arc::new(LogBuffer::new(1000));
    let _guard = tracing::subscriber::set_default(
        tracing_subscriber::registry().with(BufferLayer::new(lines.clone())),
    );
    let a = app();
    // tracing caches each callsite's interest when it is first hit; a hit racing with another test's subscriber on
    // another thread can cache "never", and the line is not captured (seen on CI, as in v08_ai_link.rs)
    tracing::callsite::rebuild_interest_cache();
    let adm = admin(&a);
    post_h(
        &a,
        "/api/settings",
        &adm,
        &json!({"theme": "dark"}),
        &[("x-real-ip", b"203.0.113.9")],
    )
    .await;
    let logged: Vec<String> = lines
        .since(None)
        .lines
        .into_iter()
        .map(|l| l.message)
        .filter(|m| m.starts_with("activity "))
        .collect();
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert!(logged[0].contains("user=\"admin\""), "{}", logged[0]);
    assert!(logged[0].contains("action=settings.update"));
    assert!(logged[0].contains("result=ok"));
    assert!(!logged[0].contains("203.0.113.9"), "{}", logged[0]);
}
