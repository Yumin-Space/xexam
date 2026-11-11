//! Both HTTP clients ignore proxy settings from the environment (keys and the internal token never go to a proxy).
//! Its own test binary: it changes the process environment.
mod common;

use common::*;
use serde_json::json;

#[tokio::test]
async fn proxy_variables_are_ignored() {
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        l.local_addr().expect("addr").port()
    };
    let proxy = format!("http://127.0.0.1:{closed}");
    for v in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        std::env::set_var(v, &proxy);
    }
    std::env::remove_var("NO_PROXY");
    std::env::remove_var("no_proxy");

    let t = start().await;
    // control: a client that honours the environment cannot reach the stub through the closed proxy port
    let honouring = reqwest::Client::builder()
        .tls_backend_preconfigured(xexam_ai_service::provider::tls_config().expect("tls"))
        .build()
        .expect("client");
    assert!(honouring
        .get(format!("{}/v1/models", t.stub.url))
        .send()
        .await
        .is_err());

    // the provider client goes direct
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let ev = t
        .send_all(&sam, json!({"text": "through no proxy"}))
        .await
        .expect("stream");
    assert_eq!(terminal(&ev).0, "done");
    // and so does the backend client (reconcile, audit lines)
    let (s, v) = t.get("/api/ai/admin/users", &admin).await;
    assert_eq!(s, 200, "{v}");
    let (s, _) = t
        .get(&format!("/api/ai/admin/user?id={}", sam.id), &admin)
        .await;
    assert_eq!(s, 200);
    assert!(t
        .backend
        .log_contains("viewed the AI chats and memories of user \"sam\""));
}
