//! v1.9 `XEXAM_FRONTEND_PEERS`: a peer that is not loopback and not listed gets the unknown-route answer (404 text) on
//! every route except `/healthz`; with no connect info the request is allowed. Everything is made up.
mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use common::*;
use std::net::SocketAddr;
use tower::ServiceExt;
use xexam_ai_service::peers::parse_peers;

fn with_peer(path: &str, peer: Option<&str>) -> Request<Body> {
    let mut req = Request::builder()
        .uri(path)
        .body(Body::empty())
        .expect("request");
    if let Some(p) = peer {
        let addr: SocketAddr = p.parse().expect("socket address");
        req.extensions_mut().insert(ConnectInfo(addr));
    }
    req
}

async fn status_and_text(t: &TestApp, path: &str, peer: Option<&str>) -> (StatusCode, String) {
    let res = t
        .router()
        .oneshot(with_peer(path, peer))
        .await
        .expect("answer");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).to_string())
}

#[tokio::test]
async fn an_unlisted_peer_gets_the_unknown_route_answer_but_healthz_stays_open() {
    let mut t = start().await;
    set_peers(&mut t, "10.0.0.5").await;
    // an unlisted peer: the same answer as a route that does not exist
    let (unknown_status, unknown_text) =
        status_and_text(&t, "/api/ai/no-such-route", Some("10.9.9.9:4000")).await;
    let (s, text) = status_and_text(&t, "/api/ai/status", Some("10.9.9.9:4000")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!((s, text.clone()), (unknown_status, unknown_text));
    // the health check is open to every peer
    let (s, _) = status_and_text(&t, "/healthz", Some("10.9.9.9:4000")).await;
    assert_eq!(s, StatusCode::OK);
    // a listed peer and loopback reach the route (no identity: not the 404 of the peer check)
    for peer in ["10.0.0.5:4000", "127.0.0.1:4000"] {
        let (s, text) = status_and_text(&t, "/api/ai/status", Some(peer)).await;
        assert_ne!(
            (s, text),
            (StatusCode::NOT_FOUND, "not found".to_string()),
            "{peer}"
        );
    }
    // no connect info (the in-process router of these tests): allowed
    let (s, _) = status_and_text(&t, "/healthz", None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, text) = status_and_text(&t, "/api/ai/status", None).await;
    assert_ne!((s, text), (StatusCode::NOT_FOUND, "not found".to_string()));
}

/// Reopens the service with `XEXAM_FRONTEND_PEERS` set to `list`.
async fn set_peers(t: &mut TestApp, list: &str) {
    let mut cfg = test_config(t.dir.path(), &t.backend.url, &Opts::default());
    let (peers, warnings) = parse_peers(Some(list));
    assert!(warnings.is_empty(), "{warnings:?}");
    cfg.frontend_peers = peers;
    t.app = xexam_ai_service::app::App::open(cfg).expect("reopen");
    t.app.0.heartbeat_now().await;
}
