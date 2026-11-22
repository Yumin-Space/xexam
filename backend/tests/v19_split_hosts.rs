//! v1.9 item 12: the backend on its own machine. `XEXAM_FRONTEND_PEERS` (the peer allow-list: an unlisted peer sees only
//! the unknown-route answer, except the token routes; forwarded headers are believed only from listed peers),
//! `XEXAM_INTERNAL_TOKEN_REQUIRED` (`require_env_token`). Real router and database; made-up users and addresses only.
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
use std::sync::{Arc, OnceLock};
use xexam_backend::api::internal::{
    load_link, require_env_token, InternalToken, LinkLoad, TOKEN_ENV,
};
use xexam_backend::api::{build_router, AppState};
use xexam_backend::auth::{self, Role};
use xexam_backend::config::Config;
use xexam_backend::database::{Store, UserChange};
use xexam_backend::services::RealEngine;

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";
const NGINX_HOST: &str = "backend:8765";
const TOKEN: &str = "fake-internal-token-for-tests-0123456789";
const PW: &str = "kitchen-pass-1";

struct App {
    router: Router,
    store: Arc<Store>,
    db: PathBuf,
    state: AppState,
    _dir: tempfile::TempDir,
}

fn pw_hash() -> &'static str {
    static H: OnceLock<String> = OnceLock::new();
    H.get_or_init(|| auth::hash_password(PW).expect("hash"))
}

fn raw(p: &Path) -> Connection {
    Connection::open(p).expect("raw open")
}

fn app(env: &[(&str, &str)]) -> App {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let store = Arc::new(Store::open(&db).expect("store"));
    let engine = Arc::new(RealEngine::new(store.clone()));
    let cfg: Config = apic::config_with(env);
    let state = AppState::new_with_logs(cfg, store.clone(), engine, None, Default::default());
    assert!(state
        .0
        .ai_link
        .set_token(InternalToken::new(TOKEN).expect("token")));
    match store
        .create_user("kim", pw_hash(), Role::User)
        .expect("user")
    {
        UserChange::Done(_) => {}
        other => panic!("{other:?}"),
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
        db,
        state,
        _dir: dir,
    }
}

/// A request as if it came from `peer` (`None` = no connect info).
async fn call(
    a: &App,
    peer: Option<&str>,
    method: &str,
    path: &str,
    host: &str,
    headers: &[(&str, &str)],
    body: Option<String>,
) -> Resp {
    let mut hs: Vec<(&str, Vec<u8>)> = Vec::new();
    for (k, v) in headers {
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
    let mut req = build(method, path, Some(host), &hrefs, body);
    if let Some(p) = peer {
        let addr: std::net::SocketAddr = p.parse().expect("peer");
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(addr));
    }
    send(&a.router, req).await
}

async fn info(a: &App, peer: Option<&str>) -> Resp {
    call(a, peer, "GET", "/api/info", HOST, &[], None).await
}

async fn auth_step(a: &App, peer: Option<&str>) -> Resp {
    call(
        a,
        peer,
        "GET",
        "/internal/ai/auth",
        NGINX_HOST,
        &[("x-original-method", "GET"), ("x-original-host", HOST)],
        None,
    )
    .await
}

async fn heartbeat(a: &App, peer: Option<&str>, token: &str) -> Resp {
    let body =
        json!({"version": "1.9", "enabled": false, "suspended": [], "stopping": false}).to_string();
    call(
        a,
        peer,
        "POST",
        "/internal/ai/heartbeat",
        NGINX_HOST,
        &[("x-xexam-internal-token", token)],
        Some(body),
    )
    .await
}

async fn sign_in(a: &App, peer: Option<&str>, headers: &[(&str, &str)]) -> Resp {
    let mut hs: Vec<(&str, &str)> = vec![("origin", ORIGIN)];
    hs.extend_from_slice(headers);
    call(
        a,
        peer,
        "POST",
        "/api/auth/login",
        HOST,
        &hs,
        Some(json!({"username": "kim", "password": PW}).to_string()),
    )
    .await
}

fn last_ip(a: &App) -> Option<String> {
    raw(&a.db)
        .query_row(
            "SELECT last_login_ip FROM users WHERE username = 'kim'",
            [],
            |r| r.get(0),
        )
        .expect("user")
}

// ---------------------------------------------------------------- the peer allow-list

#[tokio::test]
async fn an_unlisted_peer_sees_only_the_unknown_route_answer() {
    let a = app(&[
        ("XEXAM_FRONTEND_PEERS", "10.9.9.9/32"),
        ("XEXAM_BEHIND_PROXY", "1"),
    ]);
    assert!(a.state.0.cfg.peer_warnings.is_empty());
    let r = info(&a, Some("10.1.1.1:40000")).await;
    assert_eq!(r.status, 404);
    assert_eq!(r.text(), "not found");
    // the auth step is for the frontend only
    assert_eq!(auth_step(&a, Some("10.1.1.1:40000")).await.status, 404);
    // a sign-in attempt does not reach the handler either
    let r = sign_in(&a, Some("10.1.1.1:40000"), &[]).await;
    assert_eq!(r.status, 404);
    assert_eq!(last_ip(&a), None);
    // the AI service's own routes stay reachable: they prove themselves with the token
    let r = heartbeat(&a, Some("10.1.1.1:40000"), TOKEN).await;
    assert_eq!(r.status, 200, "{}", r.text());
    // and a wrong token is refused by the token check, not by the peer list
    let r = heartbeat(
        &a,
        Some("10.1.1.1:40000"),
        "wrong-token-wrong-token-wrong-token-0",
    )
    .await;
    assert_ne!(r.status, 200);
}

#[tokio::test]
async fn a_listed_peer_is_served_and_its_forwarded_ip_is_believed() {
    let a = app(&[
        ("XEXAM_FRONTEND_PEERS", "10.9.9.9/32"),
        ("XEXAM_BEHIND_PROXY", "1"),
    ]);
    a.store.set_signin_info(true, Some(1)).expect("on");
    a.state.0.activity.reload_settings(&a.store);
    assert_eq!(info(&a, Some("10.9.9.9:40000")).await.status, 200);
    let r = sign_in(&a, Some("10.9.9.9:40001"), &[("x-real-ip", "203.0.113.7")]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(last_ip(&a).as_deref(), Some("203.0.113.7"));
}

#[tokio::test]
async fn loopback_and_a_missing_peer_are_always_allowed() {
    let a = app(&[("XEXAM_FRONTEND_PEERS", "10.9.9.9/32")]);
    assert_eq!(info(&a, Some("127.0.0.1:40000")).await.status, 200);
    assert_eq!(info(&a, Some("[::1]:40000")).await.status, 200);
    assert_eq!(info(&a, Some("[::ffff:127.0.0.1]:40000")).await.status, 200);
    assert_eq!(info(&a, None).await.status, 200);
    assert_eq!(auth_step(&a, Some("127.0.0.1:40000")).await.status, 401);
}

#[tokio::test]
async fn without_the_variable_every_peer_is_served_as_before() {
    let a = app(&[]);
    assert!(!a.state.0.cfg.frontend_peers.is_restricted());
    assert_eq!(info(&a, Some("10.1.1.1:40000")).await.status, 200);
    assert_ne!(auth_step(&a, Some("10.1.1.1:40000")).await.status, 404);
}

#[tokio::test]
async fn a_value_without_a_valid_entry_means_loopback_only() {
    let a = app(&[("XEXAM_FRONTEND_PEERS", "0.0.0.0/0, nope")]);
    assert!(a
        .state
        .0
        .cfg
        .peer_warnings
        .iter()
        .any(|w| w.contains("no valid entry")));
    assert_eq!(info(&a, Some("10.9.9.9:40000")).await.status, 404);
    assert_eq!(info(&a, Some("127.0.0.1:40000")).await.status, 200);
}

#[tokio::test]
async fn forwarded_headers_from_an_unlisted_peer_are_never_used() {
    // A peer that is listed but the proxy flag is off: the forwarded address is still not believed (not loopback)
    let a = app(&[("XEXAM_FRONTEND_PEERS", "10.9.9.9")]);
    a.store.set_signin_info(true, Some(1)).expect("on");
    a.state.0.activity.reload_settings(&a.store);
    let r = sign_in(&a, Some("10.9.9.9:40001"), &[("x-real-ip", "203.0.113.7")]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(last_ip(&a).as_deref(), Some("10.9.9.9"));
}

// ---------------------------------------------------------------- XEXAM_INTERNAL_TOKEN_REQUIRED

fn long_token() -> String {
    "k".repeat(40)
}

#[test]
fn require_env_token_matrix() {
    let dir = dbc::tmp();
    // not required: always fine, whatever was loaded
    for load in [
        LinkLoad::Off("anything".into()),
        load_link(None, dir.path()),
    ] {
        assert_eq!(require_env_token(false, &load), Ok(()));
    }
    // required and set to a valid value
    let ok = load_link(Some(&long_token()), dir.path());
    assert!(matches!(ok, LinkLoad::Env(_)));
    assert_eq!(require_env_token(true, &ok), Ok(()));
    // required but only the token file exists (a file cannot be shared between machines)
    let file = load_link(None, dir.path());
    assert!(matches!(file, LinkLoad::File { .. }));
    let e = require_env_token(true, &file).expect_err("file");
    assert!(
        e.starts_with("XEXAM_INTERNAL_TOKEN_REQUIRED=1 but XEXAM_INTERNAL_TOKEN is not set"),
        "{e}"
    );
    assert!(e.contains("openssl rand -hex 32"), "{e}");
    // required and nothing at all
    let e = require_env_token(true, &LinkLoad::Off(String::new())).expect_err("off");
    assert!(e.contains("is not set"), "{e}");
    // required and the value is too short / not printable
    let short = load_link(Some("short"), dir.path());
    let e = require_env_token(true, &short).expect_err("short");
    assert_eq!(
        e,
        format!(
            "{TOKEN_ENV} is too short (at least 32 characters) (XEXAM_INTERNAL_TOKEN_REQUIRED=1)"
        )
    );
    let bad = format!("{}\u{e9}", "k".repeat(40));
    let e = require_env_token(true, &load_link(Some(&bad), dir.path())).expect_err("print");
    assert!(
        e.starts_with("XEXAM_INTERNAL_TOKEN may contain only printable ASCII")
            && e.ends_with("(XEXAM_INTERNAL_TOKEN_REQUIRED=1)"),
        "{e}"
    );
}

#[test]
fn the_messages_never_contain_the_token() {
    let dir = dbc::tmp();
    let secret = format!("{}\u{e9}secret-part", "z".repeat(40));
    let secret_short = "tiny-secret";
    for value in [secret.as_str(), secret_short, ""] {
        let load = load_link(Some(value), dir.path());
        if let Err(e) = require_env_token(true, &load) {
            assert!(!e.contains(value) || value.is_empty(), "{e}");
        }
        assert!(!load.log_line().contains(value) || value.is_empty());
    }
}

#[test]
fn the_flag_is_read_from_the_environment() {
    assert!(!apic::config_with(&[]).internal_token_required);
    assert!(apic::config_with(&[("XEXAM_INTERNAL_TOKEN_REQUIRED", "1")]).internal_token_required);
    assert!(!apic::config_with(&[("XEXAM_INTERNAL_TOKEN_REQUIRED", "0")]).internal_token_required);
}
