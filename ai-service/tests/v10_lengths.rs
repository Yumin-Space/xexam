//! v1.0 answer and memory length (spec 3.10, FB-5 revised): the answer length in characters (`replyMaxChars`, default
//! 1000) becomes the provider's `max_tokens` by the reply language, with the fallback ladder inside one counted call;
//! the memory length is a plain character count (`memoryChars`, default 200); the user message has a fixed cap of 16000
//! characters. All names, texts and keys are made up.
mod common;

#[path = "study_api/util.rs"]
mod util;

use serde_json::{json, Value};
use util::*;

const SUGGEST: &str = "/api/ai/memories/suggest";

/// The stub's chat requests (the administrator's test calls left out).
fn chat_requests(t: &TestApp) -> Vec<StubRequest> {
    t.stub
        .requests()
        .into_iter()
        .filter(|r| r.path.ends_with("/chat/completions") || r.path.ends_with("/messages"))
        .filter(|r| {
            !r.body
                .to_string()
                .contains("Reply with the single word OK.")
        })
        .collect()
}

fn hint_in(lang: &str, q: &str, no: i64) -> Value {
    let mut b = hint_body(K1, q, no);
    b["lang"] = json!(lang);
    b
}

async fn set_reply_chars(e: &Env, n: i64) {
    set_limit(&e.t, &e.admin, "replyMaxChars", n).await;
}

#[tokio::test]
async fn defaults_by_language_for_every_kind_of_call() {
    let e = env().await;
    // zh study ask: ceil(1000 x 1.5)
    let ev = ask(&e.t, &e.user, hint_in("zh", "q1", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    assert_eq!(e.t.stub.last().body["max_tokens"], json!(1500));
    assert!(e.t.stub.last().body.get("max_completion_tokens").is_none());
    // en AI Chat send: 1 character = 1 token
    let ev =
        e.t.send_all(
            &e.user,
            json!({"chatId": null, "text": "I like short answers", "lang": "en"}),
        )
        .await
        .expect("send");
    assert_eq!(terminal(&ev).0, "done");
    assert_eq!(e.t.stub.last().body["max_tokens"], json!(1000));
    let msg = ev[0].1["userMessageId"].as_i64().expect("message");
    // zh memory suggestion
    let (s, v) =
        e.t.post(SUGGEST, &e.user, json!({"messageId": msg, "lang": "zh"}))
            .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(e.t.stub.last().body["max_tokens"], json!(1500));
    // ja summary ask
    let items = vec![
        item("q1", json!(["A"]), false),
        item("q2", Value::Null, false),
    ];
    e.t.add_record(record(
        &e.user,
        7,
        "exam",
        Some(K2),
        "2026-11-11 21:58",
        items,
    ));
    let ev = ask_to(
        &e.t,
        &e.user,
        SUMMARY_ASK,
        json!({"recordId": 7, "lang": "ja"}),
    )
    .await;
    assert_eq!(terminal_name(&ev), "done");
    assert_eq!(e.t.stub.last().body["max_tokens"], json!(1500));
    // another question in English uses the same rule
    let _ = ask(&e.t, &e.user, hint_in("en", "q2", 2)).await;
    assert_eq!(e.t.stub.last().body["max_tokens"], json!(1000));
    // the administrator's connection test sent no token field (the body of v0.9.1)
    let verify =
        e.t.stub
            .requests()
            .into_iter()
            .find(|r| {
                r.body
                    .to_string()
                    .contains("Reply with the single word OK.")
            })
            .expect("the test call");
    assert!(verify.body.get("max_tokens").is_none());
    assert!(verify.body.get("max_completion_tokens").is_none());
}

#[tokio::test]
async fn a_changed_answer_length_applies_to_the_next_ask() {
    let e = env().await;
    set_reply_chars(&e, 300).await;
    let _ = ask(&e.t, &e.user, hint_in("zh", "q1", 1)).await;
    assert_eq!(e.t.stub.last().body["max_tokens"], json!(450));
    set_reply_chars(&e, 16000).await;
    let _ = ask(&e.t, &e.user, hint_in("zh", "q2", 2)).await;
    assert_eq!(e.t.stub.last().body["max_tokens"], json!(24000));
    let _ = ask(&e.t, &e.user, hint_in("en", "q1", 3)).await;
    assert_eq!(e.t.stub.last().body["max_tokens"], json!(16000));
}

#[tokio::test]
async fn a_refused_max_tokens_is_retried_inside_one_counted_call_and_remembered() {
    let e = env().await;
    set_limit(&e.t, &e.admin, "callsPerMinute", 2).await;
    e.t.set_model(&e.admin, "demo-newparam").await;
    let before = chat_requests(&e.t).len();
    let rows_before: i64 = q1(&e.t, "SELECT COUNT(*) FROM usage WHERE kind = 'chat'");
    let ev = ask(&e.t, &e.user, hint_in("en", "q1", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    let reqs = chat_requests(&e.t);
    assert_eq!(
        reqs.len() - before,
        2,
        "first max_tokens, then the fallback"
    );
    assert_eq!(reqs[before].body["max_tokens"], json!(1000));
    assert!(reqs[before].body.get("max_completion_tokens").is_none());
    assert_eq!(reqs[before + 1].body["max_completion_tokens"], json!(1000));
    assert!(reqs[before + 1].body.get("max_tokens").is_none());
    // one INFO line for the step, without any key, URL or content
    assert!(captured_contains(
        "the AI provider refused max_tokens; using max_completion_tokens"
    ));
    // one usage row for the call
    let rows: i64 = q1(&e.t, "SELECT COUNT(*) FROM usage WHERE kind = 'chat'");
    assert_eq!(rows - rows_before, 1);
    // the next call starts with the step that worked
    let ev = ask(&e.t, &e.user, hint_in("en", "q2", 2)).await;
    assert_eq!(terminal_name(&ev), "done");
    let reqs = chat_requests(&e.t);
    assert_eq!(reqs.len() - before, 3, "no refused request this time");
    assert_eq!(reqs[before + 2].body["max_completion_tokens"], json!(1000));
    // two counted calls with a cap of 2: the retry did not count, the third ask is refused
    let (s, v) = refused(&e.t, &e.user, STUDY_ASK, hint_in("en", "q1", 3)).await;
    assert_eq!((s, v["code"].clone()), (429, json!("ai.callLimit")), "{v}");
    // a model change starts again from max_tokens
    e.t.set_model(&e.admin, "demo-chat-small").await;
    e.t.relax_caps();
    let _ = ask(&e.t, &e.user, hint_in("en", "q1", 4)).await;
    assert_eq!(e.t.stub.last().body["max_tokens"], json!(1000));
    assert!(e.t.stub.last().body.get("max_completion_tokens").is_none());
}

#[tokio::test]
async fn a_service_that_refuses_both_fields_gets_no_token_limit() {
    let e = env().await;
    e.t.set_model(&e.admin, "demo-notokens").await;
    let before = chat_requests(&e.t).len();
    let ev = ask(&e.t, &e.user, hint_in("zh", "q1", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    let reqs = chat_requests(&e.t);
    assert_eq!(reqs.len() - before, 3);
    assert_eq!(reqs[before].body["max_tokens"], json!(1500));
    assert_eq!(reqs[before + 1].body["max_completion_tokens"], json!(1500));
    assert!(reqs[before + 2].body.get("max_tokens").is_none());
    assert!(reqs[before + 2].body.get("max_completion_tokens").is_none());
    let rows: i64 = q1(&e.t, "SELECT COUNT(*) FROM usage WHERE kind = 'chat'");
    assert_eq!(rows, 1);
    let status: String = q1(&e.t, "SELECT status FROM usage WHERE kind = 'chat'");
    assert_eq!(status, "ok");
}

#[tokio::test]
async fn a_changed_answer_length_forgets_the_remembered_step() {
    let e = env().await;
    e.t.set_model(&e.admin, "demo-notokens").await;
    let before = chat_requests(&e.t).len();
    let _ = ask(&e.t, &e.user, hint_in("zh", "q1", 1)).await;
    assert_eq!(chat_requests(&e.t).len() - before, 3);
    // remembered: the next call sends no limit at once
    let _ = ask(&e.t, &e.user, hint_in("zh", "q2", 2)).await;
    let reqs = chat_requests(&e.t);
    assert_eq!(reqs.len() - before, 4);
    assert!(reqs[before + 3].body.get("max_tokens").is_none());
    // the admin changes the answer length: the next call starts again from max_tokens with the new value
    set_reply_chars(&e, 300).await;
    let _ = ask(&e.t, &e.user, hint_in("zh", "q1", 3)).await;
    let reqs = chat_requests(&e.t);
    assert_eq!(reqs.len() - before, 7);
    assert_eq!(reqs[before + 4].body["max_tokens"], json!(450));
    // an unchanged length (another limit saved) keeps the remembered step
    set_limit(&e.t, &e.admin, "memoryChars", 150).await;
    let _ = ask(&e.t, &e.user, hint_in("zh", "q2", 4)).await;
    let reqs = chat_requests(&e.t);
    assert_eq!(reqs.len() - before, 8);
    assert!(reqs[before + 7].body.get("max_tokens").is_none());
}

#[tokio::test]
async fn an_unrelated_400_is_not_retried() {
    let e = env().await;
    e.t.stub.force(Some((
        400,
        r#"{"error":{"type":"invalid_request_error","message":"model not found"}}"#,
    )));
    let before = chat_requests(&e.t).len();
    let ev = ask(&e.t, &e.user, hint_in("en", "q1", 1)).await;
    assert_eq!(terminal_name(&ev), "error");
    assert_eq!(chat_requests(&e.t).len() - before, 1);
}

async fn use_anthropic(e: &Env, model: &str) {
    let (s, v) = e
        .t
        .post(
            "/api/ai/admin/settings",
            &e.admin,
            json!({"provider": "anthropic", "baseUrl": format!("{}/anthropic", e.t.stub.url), "apiKey": KEY, "model": model}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    e.t.verify_and_enable(&e.admin).await;
}

#[tokio::test]
async fn anthropic_is_clamped_to_the_stored_test_result() {
    let e = env().await;
    set_reply_chars(&e, 16000).await;
    // demo-4k: the test finds that 8192 is too large and stores 4096, so the first request is already at most 4096
    use_anthropic(&e, "demo-4k").await;
    let stored: i64 = q1(&e.t, "SELECT max_tokens FROM settings");
    assert_eq!(stored, 4096);
    let before = chat_requests(&e.t).len();
    let ev = ask(&e.t, &e.user, hint_in("zh", "q1", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    let reqs = chat_requests(&e.t);
    assert_eq!(reqs.len() - before, 1);
    assert_eq!(reqs[before].body["max_tokens"], json!(4096));
    // a short answer length stays as it is
    set_reply_chars(&e, 1000).await;
    let _ = ask(&e.t, &e.user, hint_in("en", "q2", 2)).await;
    assert_eq!(e.t.stub.last().body["max_tokens"], json!(1000));
}

#[tokio::test]
async fn anthropic_refusing_the_value_gets_the_stored_ceiling_once() {
    let e = env().await;
    set_reply_chars(&e, 16000).await;
    // demo-8k accepts the test's 8192 (stored) but refuses 24000
    use_anthropic(&e, "demo-8k").await;
    let stored: i64 = q1(&e.t, "SELECT max_tokens FROM settings");
    assert_eq!(stored, 8192);
    let before = chat_requests(&e.t).len();
    let ev = ask(&e.t, &e.user, hint_in("zh", "q1", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    let reqs = chat_requests(&e.t);
    assert_eq!(reqs.len() - before, 2);
    assert_eq!(reqs[before].body["max_tokens"], json!(24000));
    assert_eq!(reqs[before + 1].body["max_tokens"], json!(8192));
    let rows: i64 = q1(&e.t, "SELECT COUNT(*) FROM usage WHERE kind = 'chat'");
    assert_eq!(rows, 1);
    // remembered: the next call starts at 8192
    let _ = ask(&e.t, &e.user, hint_in("zh", "q2", 2)).await;
    let reqs = chat_requests(&e.t);
    assert_eq!(reqs.len() - before, 3);
    assert_eq!(reqs[before + 2].body["max_tokens"], json!(8192));
}

#[tokio::test]
async fn the_test_call_keeps_its_v091_body() {
    let e = env().await;
    // OpenAI-compatible: no token field
    let verify = e.t.stub.requests().into_iter().next().expect("test call");
    assert!(verify.body.get("max_tokens").is_none());
    assert!(verify.body.get("max_completion_tokens").is_none());
    // Anthropic: the stored value (8192, then 4096 after a refusal), independent of the answer length
    set_reply_chars(&e, 100).await;
    use_anthropic(&e, "demo-4k").await;
    let tests: Vec<i64> =
        e.t.stub
            .requests()
            .into_iter()
            .filter(|r| r.path.ends_with("/messages"))
            .filter(|r| {
                r.body
                    .to_string()
                    .contains("Reply with the single word OK.")
            })
            .map(|r| r.body["max_tokens"].as_i64().unwrap_or(0))
            .collect();
    assert_eq!(tests, vec![8192, 4096]);
}

#[tokio::test]
async fn message_and_memory_lengths() {
    let e = env().await;
    // a user message: a fixed cap of 16000 characters
    let (s, v) =
        e.t.post(
            "/api/ai/send",
            &e.user,
            json!({"text": "z".repeat(16001), "lang": "en"}),
        )
        .await;
    assert_eq!(
        (s, v["code"].clone(), v["limit"].clone()),
        (400, json!("ai.tooLong"), json!(16000))
    );
    let ev =
        e.t.send_all(&e.user, json!({"text": "z".repeat(16000), "lang": "en"}))
            .await
            .expect("16000 characters pass");
    assert_eq!(terminal(&ev).0, "done");
    // the status answer keeps its fields
    let (_, st) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(st["messageChars"], json!(16000));
    assert_eq!(st["maxMessageChars"], json!(16000));
    assert_eq!(st["memoryChars"], json!(200));
    assert_eq!(st["memoryMaxChars"], json!(200));
    // a memory: 201 characters are too long on a fresh database, 200 pass
    let (s, v) =
        e.t.post(
            "/api/ai/memories/add",
            &e.user,
            json!({"content": "x".repeat(201)}),
        )
        .await;
    assert_eq!(
        (s, v["code"].clone(), v["limit"].clone()),
        (400, json!("ai.memoryTooLong"), json!(200))
    );
    let (s, v) =
        e.t.post(
            "/api/ai/memories/add",
            &e.user,
            json!({"content": "x".repeat(200)}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    // the answer length of the limits answer
    let (_, v) = e.t.get("/api/ai/admin/limits", &e.admin).await;
    assert_eq!(v["limits"]["replyMaxChars"], json!(1000));
    assert_eq!(v["limits"]["memoryChars"], json!(200));
}
