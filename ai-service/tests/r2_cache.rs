//! Prompt caching and the shortening of old history messages: the Anthropic request carries `cache_control` on the
//! system block and on the last history message; cached input tokens are parsed for both formats, recorded in the usage
//! row and shown in the administrators' usage; old messages are cut the same way on every request.
mod common;

use common::*;
use serde_json::{json, Value};

fn cached_rows(t: &TestApp) -> Vec<Option<i64>> {
    let db = t.db();
    let mut st = db
        .prepare("SELECT tokens_cached FROM usage WHERE kind = 'chat' ORDER BY id")
        .expect("prepare");
    let rows = st.query_map([], |r| r.get(0)).expect("query");
    rows.map(|r| r.expect("row")).collect()
}

fn keys(v: &Value) -> Vec<&str> {
    v.as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect()
}

#[tokio::test]
async fn openai_cached_tokens_are_recorded_and_shown_in_the_usage() {
    let t = start().await;
    t.relax_caps();
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-cached").await;
    // the OpenAI request is unchanged: a plain system string, no cache markers
    let ev = t
        .send_all(&sam, json!({"chatId": null, "text": "Hello", "lang": "en"}))
        .await
        .expect("stream");
    assert_eq!(terminal(&ev).0, "done");
    let req = t.stub.last();
    assert!(req.body["messages"][0]["content"].is_string());
    assert!(!req.body.to_string().contains("cache_control"));
    assert_eq!(cached_rows(&t), vec![Some(8)]);
    let (code, u) = t.get("/api/ai/admin/usage?days=7", &admin).await;
    assert_eq!(code, 200, "{u}");
    assert_eq!(u["totals"]["tokensCached"], json!(8));
    // the connection test of `configure_openai` is the "other" row (empty model name), the chat is the cached model
    let model_row = u["byModel"]
        .as_array()
        .and_then(|a| a.iter().find(|r| r["model"] == json!("demo-cached")))
        .expect("model row");
    for row in [&u["totals"], &u["byUser"][0], model_row, &u["byPeriod"][6]] {
        assert_eq!(keys(row).last(), Some(&"tokensCacheWrite"), "{row}");
    }
    let sam_row = u["byUser"]
        .as_array()
        .and_then(|a| a.iter().find(|r| r["username"] == json!("sam")))
        .expect("sam");
    assert_eq!(sam_row["tokensCached"], json!(8));
    assert_eq!(model_row["tokensCached"], json!(8));
    assert_eq!(u["byPeriod"][6]["tokensCached"], json!(8));
    assert_eq!(u["byPeriod"][0]["tokensCached"], json!(0));
}

#[tokio::test]
async fn a_provider_that_reports_nothing_records_null_and_shows_zero() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    t.send_all(
        &admin,
        json!({"chatId": null, "text": "Hello", "lang": "en"}),
    )
    .await
    .expect("stream");
    assert_eq!(cached_rows(&t), vec![None]);
    let (_, u) = t.get("/api/ai/admin/usage?days=7", &admin).await;
    assert_eq!(u["totals"]["tokensCached"], json!(0));
}

#[tokio::test]
async fn the_anthropic_request_carries_two_cache_breakpoints() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.seed_route(Seed {
        provider: "anthropic",
        url: format!("{}/anthropic/v1", t.stub.url),
        api_key: Some(KEY),
        gateway_key: None,
        gateway_auth: "bearer",
        model: "demo-claude",
    });
    t.verify_and_enable(&admin).await;
    let ev = t
        .send_all(
            &sam,
            json!({"chatId": null, "text": "first-q", "lang": "en"}),
        )
        .await
        .expect("stream");
    let chat = ev[0].1["chatId"].as_i64().expect("chat");
    // no history yet: breakpoint on the system block only
    let b = t.stub.last().body;
    assert_eq!(
        b["system"][0]["cache_control"],
        json!({"type": "ephemeral"})
    );
    assert!(b["messages"][0]["content"].is_string());
    t.send_all(
        &sam,
        json!({"chatId": chat, "text": "second-q", "lang": "en"}),
    )
    .await
    .expect("stream");
    let b = t.stub.last().body;
    let msgs = b["messages"].as_array().expect("messages");
    assert_eq!(msgs.len(), 3);
    // the last history message (the first answer) carries the breakpoint, the new question does not
    assert_eq!(
        msgs[1]["content"][0]["cache_control"],
        json!({"type": "ephemeral"})
    );
    assert_eq!(msgs[2], json!({"role": "user", "content": "second-q"}));
    assert!(msgs[0]["content"].is_string());
    assert_eq!(b.to_string().matches("cache_control").count(), 2);
    // the stub reports 2 cache-read tokens per call
    assert_eq!(cached_rows(&t), vec![Some(2), Some(2)]);
}

#[tokio::test]
async fn the_history_only_grows_between_two_moves_of_the_shortening_line() {
    // A2: the shortening line moves once per 8 messages (and starts after 12), so a short chat is sent in full and every
    // request is the previous request plus the new messages: the provider's prompt cache keeps hitting
    let t = start().await;
    t.relax_caps();
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    let long = |c: char| c.to_string().repeat(2100);
    let mut chat = Value::Null;
    let mut seen: Vec<Vec<String>> = Vec::new();
    for (i, c) in ['a', 'b', 'c', 'd', 'e', 'f'].into_iter().enumerate() {
        let ev = t
            .send_all(
                &admin,
                json!({"chatId": chat, "text": long(c), "lang": "en"}),
            )
            .await
            .expect("stream");
        chat = ev[0].1["chatId"].clone();
        let msgs = t.stub.last().body["messages"]
            .as_array()
            .cloned()
            .expect("messages");
        seen.push(
            msgs.iter()
                .skip(1)
                .map(|m| m["content"].as_str().unwrap_or("").to_string())
                .collect(),
        );
        assert_eq!(msgs.len(), 2 + 2 * i);
    }
    // request 6: five questions and five short answers, all in full (fewer than 12 earlier messages)
    let last = &seen[5];
    assert_eq!(last.len(), 11);
    for (k, c) in ['a', 'b', 'c', 'd', 'e', 'f'].into_iter().enumerate() {
        assert_eq!(last[2 * k], long(c), "question {c}");
    }
    // every request, without its new question, is the start of the next one
    for w in seen.windows(2) {
        let old = &w[0][..w[0].len() - 1];
        assert_eq!(&w[1][..old.len()], old);
    }
}
