//! v2.6: the display name (`users.display_name`, schema 12) — the name rule, case-insensitive uniqueness, the setup gate that
//! holds a session without a name like a pending password change, the password and the name in one request,
//! `POST /api/auth/name`, the AI-internal blocks, the activity row, and the migration from schema 11.
//!
//! These tests use the real router with the real engine and database and sign in through the API. Made-up data only.
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
use xexam_backend::api::auth::{Access, ROUTES};
use xexam_backend::api::internal::InternalToken;
use xexam_backend::api::{build_router, AppState};
use xexam_backend::auth;
use xexam_backend::database::Store;
use xexam_backend::services::RealEngine;

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";
const NGINX_HOST: &str = "backend:8765";
const TOKEN: &str = "fake-internal-token-for-tests-0123456789";

const MSG_PASSWORD_CHANGE_REQUIRED: &str = "请先设置新密码。";
const MSG_NAME_EMPTY: &str = "请输入用户名。";
const MSG_NAME_BAD: &str = "用户名最多 20 个字，不能包含 @ 或控制字符。";
const MSG_NAME_TAKEN: &str = "这个用户名已经有人用了。";
const MSG_NAME_REQUIRED: &str = "请先设置用户名。";
const MSG_BAD_REQUEST: &str = "请求的格式不对。";

const ADMIN_PW: &str = "kitchen-admin-1";
const INITIAL: &str = "initial-pass";

// ---------------------------------------------------------------- helpers

struct App {
    router: Router,
    store: Arc<Store>,
    state: AppState,
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
        Default::default(),
    );
    assert!(state
        .0
        .ai_link
        .set_token(InternalToken::new(TOKEN).expect("token")));
    App {
        router: build_router(state.clone()),
        store,
        state,
        db,
        _dir: dir,
    }
}

fn raw(p: &Path) -> Connection {
    Connection::open(p).expect("raw open")
}

async fn call(
    a: &App,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> Resp {
    let mut hs: Vec<(&str, Vec<u8>)> = Vec::new();
    if let Some(c) = cookie {
        hs.push(("cookie", c.as_bytes().to_vec()));
    }
    if method == "POST" {
        hs.push(("origin", ORIGIN.as_bytes().to_vec()));
    }
    let body = match body {
        Some(b) => {
            let b = b.to_string();
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
    call(a, "GET", path, Some(cookie), None).await
}

async fn post(a: &App, path: &str, cookie: &str, body: Value) -> Resp {
    call(a, "POST", path, Some(cookie), Some(body)).await
}

fn cookie_of(r: &Resp) -> String {
    let sc = r.header("set-cookie").expect("set-cookie");
    sc.split(';').next().expect("pair").to_string()
}

async fn login(a: &App, name: &str, password: &str) -> Resp {
    call(
        a,
        "POST",
        "/api/auth/login",
        None,
        Some(json!({"username": name, "password": password})),
    )
    .await
}

/// The default administrator, signed in with the forced flow: new password and a display name together.
async fn admin(a: &App) -> String {
    let r = login(a, "admin", "admin").await;
    assert_eq!(r.status, 200, "{}", r.text());
    let c = cookie_of(&r);
    let r = post(
        a,
        "/api/auth/password",
        &c,
        json!({"password": ADMIN_PW, "displayName": "Chef"}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    cookie_of(&r)
}

/// Creates a plain user as the administrator (they must change the password, and have no display name).
async fn create(a: &App, admin_cookie: &str, name: &str) -> i64 {
    let r = post(
        a,
        "/api/users/create",
        admin_cookie,
        json!({"username": name, "password": INITIAL, "role": "user"}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()["users"]
        .as_array()
        .expect("users")
        .iter()
        .find(|u| u["username"] == name)
        .expect("row")["id"]
        .as_i64()
        .expect("id")
}

/// A user whose password change is done but who has no display name yet (an existing user of an older version):
/// created through the API, then the flag is cleared in the database. Signed in; returns the cookie.
async fn nameless(a: &App, admin_cookie: &str, name: &str) -> String {
    create(a, admin_cookie, name).await;
    raw(&a.db)
        .execute(
            "UPDATE users SET must_change_password = 0 WHERE username = ?1",
            [name],
        )
        .expect("clear the flag");
    let r = login(a, name, INITIAL).await;
    assert_eq!(r.status, 200, "{}", r.text());
    cookie_of(&r)
}

async fn set_name(a: &App, cookie: &str, name: Value) -> Resp {
    post(a, "/api/auth/name", cookie, json!({ "displayName": name })).await
}

fn err(r: &Resp) -> (u16, String) {
    (r.status.as_u16(), r.error())
}

fn display_name_in_db(a: &App, username: &str) -> Option<String> {
    raw(&a.db)
        .query_row(
            "SELECT display_name FROM users WHERE username = ?1",
            [username],
            |r| r.get(0),
        )
        .expect("row")
}

// ---------------------------------------------------------------- the rule

#[tokio::test]
async fn the_name_route_cleans_and_checks_the_rule() {
    let a = app();
    let adm = admin(&a).await;
    let c = nameless(&a, &adm, "kim").await;
    // refused: nothing, blanks, an @, too long, a control character, wrong types
    for (body, want) in [
        (json!(""), MSG_NAME_EMPTY),
        (json!("   \t "), MSG_NAME_EMPTY),
        (json!("kim@example.com"), MSG_NAME_BAD),
        (json!("x".repeat(21)), MSG_NAME_BAD),
        (json!("a\u{7}b"), MSG_NAME_BAD),
    ] {
        let r = set_name(&a, &c, body.clone()).await;
        assert_eq!(err(&r), (400, want.to_string()), "{body}");
    }
    let r = post(&a, "/api/auth/name", &c, json!({})).await;
    assert_eq!(err(&r), (400, MSG_NAME_EMPTY.to_string()));
    let r = post(&a, "/api/auth/name", &c, json!({"displayName": null})).await;
    assert_eq!(err(&r), (400, MSG_NAME_EMPTY.to_string()));
    let r = post(&a, "/api/auth/name", &c, json!({"displayName": 5})).await;
    assert_eq!(err(&r), (400, MSG_BAD_REQUEST.to_string()));
    assert_eq!(display_name_in_db(&a, "kim"), None, "nothing was written");
    // accepted: trimmed, inner whitespace collapsed, any script, exactly 20 characters
    let r = set_name(&a, &c, json!("  Kim   the  Baker ")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["user"]["displayName"], "Kim the Baker");
    assert_eq!(r.json()["user"]["needsName"], false);
    assert_eq!(
        display_name_in_db(&a, "kim").as_deref(),
        Some("Kim the Baker")
    );
    let r = set_name(&a, &c, json!("字".repeat(20))).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["user"]["displayName"], "字".repeat(20));
}

#[tokio::test]
async fn names_are_unique_ignoring_letter_case() {
    let a = app();
    let adm = admin(&a).await;
    let kim = nameless(&a, &adm, "kim").await;
    let lee = nameless(&a, &adm, "lee").await;
    assert_eq!(set_name(&a, &kim, json!("Sunny")).await.status, 200);
    for taken in ["sunny", "SUNNY", " Sunny "] {
        let r = set_name(&a, &lee, json!(taken)).await;
        assert_eq!(err(&r), (409, MSG_NAME_TAKEN.to_string()), "{taken}");
    }
    // the administrator's own name is taken as well
    let r = set_name(&a, &lee, json!("chef")).await;
    assert_eq!(err(&r), (409, MSG_NAME_TAKEN.to_string()));
    assert_eq!(display_name_in_db(&a, "lee"), None);
    // another user's login is taken too (logins and display names are exclusive across users), your own login is not
    let r = set_name(&a, &lee, json!("KIM")).await;
    assert_eq!(err(&r), (409, MSG_NAME_TAKEN.to_string()));
    assert_eq!(set_name(&a, &lee, json!("Lee")).await.status, 200);
    assert_eq!(set_name(&a, &lee, json!("lee2")).await.status, 200);
    // and a new login may not be somebody's display name
    let r = post(
        &a,
        "/api/users/create",
        &adm,
        json!({"username": "SUNNY", "password": INITIAL, "role": "user"}),
    )
    .await;
    assert_eq!(r.status, 409, "{}", r.text());
    // your own name in other capitals is allowed (it changes the capitals), and a free name works
    let r = set_name(&a, &kim, json!("SUNNY")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(set_name(&a, &lee, json!("Moon")).await.status, 200);
    // the database holds the same rule
    let dup = raw(&a.db).execute(
        "UPDATE users SET display_name = 'moon' WHERE username = 'kim'",
        [],
    );
    assert!(dup.is_err(), "the unique index refuses a duplicate");
}

// ---------------------------------------------------------------- the gate

#[tokio::test]
async fn a_session_without_a_name_reaches_only_the_setup_routes() {
    let a = app();
    let adm = admin(&a).await;
    let c = nameless(&a, &adm, "kim").await;
    // the state says what is missing
    let me = get(&a, "/api/auth/state", &c).await.json();
    assert_eq!(me["user"]["needsName"], true);
    assert_eq!(me["user"]["displayName"], Value::Null);
    assert_eq!(me["user"]["mustChangePassword"], false);
    // every route outside the allow-list is 403 with the name message
    for r in ROUTES
        .iter()
        .filter(|r| r.access != Access::Public && !r.during_password_change)
    {
        let resp = if r.method == "GET" {
            get(&a, r.path, &c).await
        } else {
            post(&a, r.path, &c, json!({})).await
        };
        assert_eq!(
            err(&resp),
            (403, MSG_NAME_REQUIRED.to_string()),
            "{} {}",
            r.method,
            r.path
        );
    }
    assert_eq!(get(&a, "/api/info", &c).await.status, 200);
    // the allow-list: logout, password and name (the name route is the way out)
    let allowed: Vec<&str> = ROUTES
        .iter()
        .filter(|r| r.during_password_change)
        .map(|r| r.path)
        .collect();
    assert_eq!(
        allowed,
        ["/api/auth/logout", "/api/auth/password", "/api/auth/name"]
    );
    assert_eq!(set_name(&a, &c, json!("Kim")).await.status, 200);
    // the same session is open now, no new sign-in needed
    assert_eq!(get(&a, "/api/bank", &c).await.status, 200);
    assert_eq!(get(&a, "/api/settings", &c).await.status, 200);
}

#[tokio::test]
async fn a_pending_password_change_comes_first_and_keeps_its_message() {
    let a = app();
    let adm = admin(&a).await;
    create(&a, &adm, "kim").await;
    let c = cookie_of(&login(&a, "kim", INITIAL).await);
    // other routes: the password message (not the name one) while both are missing
    let r = get(&a, "/api/bank", &c).await;
    assert_eq!(err(&r), (403, MSG_PASSWORD_CHANGE_REQUIRED.to_string()));
    // the name route cannot be used to go around the password change
    let r = set_name(&a, &c, json!("Kim")).await;
    assert_eq!(err(&r), (403, MSG_PASSWORD_CHANGE_REQUIRED.to_string()));
    assert_eq!(display_name_in_db(&a, "kim"), None);
}

#[tokio::test]
async fn the_ai_internal_checks_hold_a_session_without_a_name() {
    let a = app();
    let adm = admin(&a).await;
    let c = nameless(&a, &adm, "kim").await;
    let step = |cookie: String| {
        let a = &a;
        async move {
            send(
                &a.router,
                build(
                    "GET",
                    "/internal/ai/auth",
                    Some(NGINX_HOST),
                    &[
                        ("cookie", cookie.as_bytes()),
                        ("x-original-method", b"GET"),
                        ("x-original-host", HOST.as_bytes()),
                    ],
                    Body::empty(),
                ),
            )
            .await
        }
    };
    let r = step(c.clone()).await;
    assert_eq!(r.status, 403, "{}", r.text());
    assert_eq!(r.header("x-xexam-deny").as_deref(), Some("name"));
    // the token route that binds a call to a session
    let sid = auth::token_hash(c.split('=').nth(1).expect("token"));
    let id = a.store.user_by_name("kim").expect("q").expect("kim").id;
    let ask = || {
        let body = json!({"sid": sid, "id": id}).to_string();
        let a = &a;
        async move {
            send(
                &a.router,
                build(
                    "POST",
                    "/internal/ai/introspect",
                    Some(NGINX_HOST),
                    &[
                        ("content-type", b"application/json"),
                        ("content-length", body.len().to_string().as_bytes()),
                        ("x-xexam-internal-token", TOKEN.as_bytes()),
                    ],
                    Body::from(body),
                ),
            )
            .await
        }
    };
    assert_eq!(ask().await.json()["ok"], false);
    // once the name is set both open
    assert_eq!(set_name(&a, &c, json!("Kim")).await.status, 200);
    assert_eq!(step(c.clone()).await.status, 204);
    assert_eq!(ask().await.json()["ok"], true);
}

// ---------------------------------------------------------------- password and name together

#[tokio::test]
async fn the_forced_password_change_takes_the_name_in_the_same_request() {
    let a = app();
    let adm = admin(&a).await;
    create(&a, &adm, "kim").await;
    create(&a, &adm, "lee").await;
    let c = cookie_of(&login(&a, "kim", INITIAL).await);
    // a name is required while there is none; nothing changes when it is missing, bad or taken
    for (body, want) in [
        (json!({"password": "kim-own-pass-1"}), (400, MSG_NAME_EMPTY)),
        (
            json!({"password": "kim-own-pass-1", "displayName": ""}),
            (400, MSG_NAME_EMPTY),
        ),
        (
            json!({"password": "kim-own-pass-1", "displayName": "a@b"}),
            (400, MSG_NAME_BAD),
        ),
        (
            json!({"password": "kim-own-pass-1", "displayName": "chef"}),
            (409, MSG_NAME_TAKEN),
        ),
    ] {
        let r = post(&a, "/api/auth/password", &c, body.clone()).await;
        assert_eq!(err(&r), (want.0, want.1.to_string()), "{body}");
    }
    assert_eq!(login(&a, "kim", INITIAL).await.status, 200, "old password");
    assert_eq!(display_name_in_db(&a, "kim"), None);
    // both in one go: the flag is cleared, the name is set, the session is rotated and open
    let r = post(
        &a,
        "/api/auth/password",
        &c,
        json!({"password": "kim-own-pass-1", "displayName": "  Kim  K "}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let user = &r.json()["user"];
    assert_eq!(user["mustChangePassword"], false);
    assert_eq!(user["needsName"], false);
    assert_eq!(user["displayName"], "Kim K");
    let fresh = cookie_of(&r);
    assert_ne!(fresh, c);
    assert_eq!(get(&a, "/api/bank", &fresh).await.status, 200);
    assert_eq!(login(&a, "kim", "kim-own-pass-1").await.status, 200);
    assert_eq!(login(&a, "kim", INITIAL).await.status, 401);
    // a user who already has a name may leave the field out, or change the name along with the password
    let r = post(
        &a,
        "/api/auth/password",
        &fresh,
        json!({"current": "kim-own-pass-1", "password": "kim-own-pass-2"}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(display_name_in_db(&a, "kim").as_deref(), Some("Kim K"));
    let fresh = cookie_of(&r);
    let r = post(
        &a,
        "/api/auth/password",
        &fresh,
        json!({"current": "kim-own-pass-2", "password": "kim-own-pass-3", "displayName": "Kimmy"}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(display_name_in_db(&a, "kim").as_deref(), Some("Kimmy"));
}

// ---------------------------------------------------------------- existing users, the lists, the activity row

#[tokio::test]
async fn an_existing_user_has_no_name_and_the_lists_show_it() {
    let a = app();
    let adm = admin(&a).await;
    let c = nameless(&a, &adm, "kim").await;
    // sign-in answers and the state carry the new fields; the password-flag stays false
    let r = login(&a, "kim", INITIAL).await;
    assert_eq!(r.json()["user"]["needsName"], true);
    assert_eq!(r.json()["user"]["displayName"], Value::Null);
    assert_eq!(r.json()["user"]["mustChangePassword"], false);
    // the administrator's list: null for kim, the chosen name for the administrator
    let list = get(&a, "/api/users", &adm).await.json();
    let row = |name: &str| -> Value {
        list["users"]
            .as_array()
            .expect("users")
            .iter()
            .find(|u| u["username"] == name)
            .expect("row")
            .clone()
    };
    assert_eq!(row("kim")["displayName"], Value::Null);
    assert_eq!(row("admin")["displayName"], "Chef");
    assert_eq!(set_name(&a, &c, json!("Kim")).await.status, 200);
    let list = get(&a, "/api/users", &adm).await.json();
    assert_eq!(
        list["users"]
            .as_array()
            .expect("users")
            .iter()
            .find(|u| u["username"] == "kim")
            .expect("row")["displayName"],
        "Kim"
    );
    // the login stays the login: signing in is by username, never by display name
    assert_eq!(
        login(&a, "Kim", INITIAL).await.status,
        200,
        "case-insensitive login"
    );
    assert_eq!(login(&a, "Kim K", INITIAL).await.status, 401);
}

#[tokio::test]
async fn setting_a_name_is_recorded_with_the_old_and_the_new_name() {
    let a = app();
    let adm = admin(&a).await;
    let c = nameless(&a, &adm, "kim").await;
    assert_eq!(set_name(&a, &c, json!("Kim")).await.status, 200);
    assert_eq!(set_name(&a, &c, json!("Kimmy")).await.status, 200);
    // a refused change leaves no success row
    assert_eq!(set_name(&a, &c, json!("chef")).await.status, 409);
    a.state.0.activity.flush().await;
    let rows: Vec<(String, String, Option<String>)> = {
        let c = raw(&a.db);
        let mut st = c
            .prepare(
                "SELECT username, result, detail FROM activity_log WHERE action = 'auth.name' ORDER BY id",
            )
            .expect("prepare");
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .expect("query")
            .map(|r| r.expect("row"))
            .collect()
    };
    assert_eq!(rows.len(), 3, "{rows:?}");
    let detail = |i: usize| -> Value {
        serde_json::from_str(rows[i].2.as_deref().expect("detail")).expect("json")
    };
    assert_eq!(
        (rows[0].0.as_str(), rows[0].1.as_str()),
        ("kim", "ok"),
        "the login is logged, not the name"
    );
    assert_eq!(detail(0), json!({"oldName": null, "newName": "Kim"}));
    assert_eq!(detail(1), json!({"oldName": "Kim", "newName": "Kimmy"}));
    assert_eq!(rows[2].1, "fail");
}

// ---------------------------------------------------------------- the migration

#[test]
fn a_schema_11_file_gets_the_column_with_null_and_keeps_every_user() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    {
        let s = Store::open(&db).expect("fresh");
        let hash = auth::hash_password("somepassword").expect("hash");
        for n in ["kim", "lee"] {
            s.create_user(n, &hash, auth::Role::User).expect("user");
        }
    }
    // turn it back into a schema-11 file: no column, no index
    {
        let c = raw(&db);
        c.execute_batch(
            "DROP INDEX idx_users_display_name;
             ALTER TABLE users DROP COLUMN display_name;
             UPDATE meta SET value = '11' WHERE key = 'schema';",
        )
        .expect("downgrade");
    }
    let s = Store::open(&db).expect("upgrade");
    drop(s);
    let c = raw(&db);
    let one = |sql: &str| -> String {
        c.query_row(&format!("SELECT CAST(({sql}) AS TEXT)"), [], |r| r.get(0))
            .expect("value")
    };
    assert_eq!(one("SELECT value FROM meta WHERE key = 'schema'"), "13");
    assert_eq!(one("SELECT COUNT(*) FROM users"), "3");
    assert_eq!(
        one("SELECT group_concat(username, ',') FROM (SELECT username FROM users ORDER BY id)"),
        "admin,kim,lee"
    );
    assert_eq!(
        one("SELECT COUNT(*) FROM users WHERE display_name IS NULL"),
        "3",
        "everybody is asked for a name at the next sign-in"
    );
    assert_eq!(
        one("SELECT COUNT(*) FROM sqlite_master WHERE name = 'idx_users_display_name'"),
        "1"
    );
    // one backup, named after the schema the file had, and the one migrate row
    let backups: Vec<_> = std::fs::read_dir(dir.path().join("backups"))
        .expect("backups")
        .map(|e| e.expect("entry").file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert!(backups[0].starts_with("xexam-schema11-"), "{backups:?}");
    assert_eq!(
        one("SELECT detail FROM activity_log WHERE action = 'system.migrate'"),
        r#"{"from":11,"to":13}"#
    );
    // opening again changes nothing and writes no second backup
    drop(Store::open(&db).expect("reopen"));
    assert_eq!(
        std::fs::read_dir(dir.path().join("backups"))
            .expect("backups")
            .count(),
        1
    );
}
