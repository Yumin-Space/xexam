//! What reaches a log line (printed or forwarded to the main System log) and what reaches an answer: never a key, an
//! identity statement, a cookie, a prompt, an answer, a chat title, a memory or a provider URL.
mod common;

use common::*;
use serde_json::json;

#[tokio::test]
async fn no_secret_or_content_in_any_log_line() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    let secret = "sk-demo-SECRET-9999-abcd";
    let gw_secret = "gw-demo-SECRET-7777-wxyz";
    let question = "my private study question about matrices";
    let note = "private memory note text";

    // a full session: gateway with both keys (seeded: the connection routes have their own tests), chat, memory,
    // suggestion, admin views and clears, a failure
    t.seed_route(Seed {
        provider: "gateway",
        url: format!("{}/gw/openai", t.stub.url),
        api_key: Some(secret),
        gateway_key: Some(gw_secret),
        gateway_auth: "cf-aig-authorization",
        model: "demo-chat-small",
    });
    t.verify_and_enable(&admin).await;
    let ev = t
        .send_all(&sam, json!({"text": question}))
        .await
        .expect("stream");
    assert_eq!(text(&ev), ANSWER);
    let chat = ev[0].1["chatId"].as_i64().expect("id");
    let msg = ev[0].1["userMessageId"].as_i64().expect("id");
    t.post("/api/ai/memories/add", &sam, json!({"content": note}))
        .await;
    let (s, _) = t
        .post("/api/ai/memories/suggest", &sam, json!({"messageId": msg}))
        .await;
    assert_eq!(s, 200);
    t.stub.force(Some((401, r#"{"error":{"message":"Incorrect API key provided: sk-de****abcd","code":"invalid_api_key"}}"#)));
    let ev = t
        .send_all(&sam, json!({"text": question}))
        .await
        .expect("stream");
    assert_eq!(terminal(&ev).1["code"], json!("ai.up.auth"));
    t.stub.force(None);
    let (s, _) = t
        .get(&format!("/api/ai/admin/user?id={}", sam.id), &admin)
        .await;
    assert_eq!(s, 200);
    let (s, _) = t
        .get(
            &format!("/api/ai/admin/chat?user={}&id={chat}", sam.id),
            &admin,
        )
        .await;
    assert_eq!(s, 200);
    let (s, _) = t
        .post(
            "/api/ai/admin/clear",
            &admin,
            json!({"userId": sam.id, "what": "memories"}),
        )
        .await;
    assert_eq!(s, 200);
    let (s, v) = t.get("/api/ai/admin/settings", &admin).await;
    assert_eq!(s, 200);
    assert!(!v.to_string().contains(secret) && !v.to_string().contains(gw_secret));
    let (_, u) = t.get("/api/ai/admin/usage", &admin).await;
    assert!(!u.to_string().contains(&t.stub.url));

    // forward the queued lines to the fake backend
    xexam_ai_service::logs::flush_once(&t.app.0.link, xexam_ai_service::logs::queue()).await;
    let forwarded = t.backend.logs();
    assert!(forwarded
        .iter()
        .any(|l| l.contains("AI call failed for user \"sam\": ai.up.auth (HTTP 401)")));
    // audit lines arrive once (synchronously), never through the queue
    assert_eq!(
        forwarded
            .iter()
            .filter(|l| l.contains("viewed the AI chats and memories of user \"sam\""))
            .count(),
        1
    );
    assert_eq!(
        forwarded
            .iter()
            .filter(|l| l.contains("cleared all 1 AI memories of user \"sam\""))
            .count(),
        1
    );

    let printed = captured();
    let statement_prefix = t.ident(&sam).split('.').next().unwrap_or("x")[..12].to_string();
    let forbidden: Vec<String> = vec![
        secret.into(),
        gw_secret.into(),
        "sk-demo-NEVER".into(),
        "sk-de****".into(),
        statement_prefix,
        sam.sid.clone(),
        TOKEN.into(),
        question.into(),
        note.into(),
        "short answer".into(),
        t.stub.url.clone(),
        "/gw/openai".into(),
        "Reply with the single word OK.".into(),
        "You are the study assistant".into(),
    ];
    for line in printed.iter().chain(forwarded.iter()) {
        for f in &forbidden {
            assert!(
                !line.contains(f.as_str()),
                "a log line contains {f:?}: {line}"
            );
        }
    }
    // forwarded lines have levels the backend accepts
    for (level, _) in t.backend.st().logs.iter() {
        assert!(["INFO", "WARN", "ERROR"].contains(&level.as_str()));
    }
}
