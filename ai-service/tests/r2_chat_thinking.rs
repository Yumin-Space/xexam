//! Reasoning models (round 2, item 25): reasoning deltas (OpenAI `reasoning_content` / `reasoning`, Anthropic thinking
//! blocks) become one `thinking` SSE frame before the first `delta`; the reasoning text is never stored, sent, logged or
//! counted towards the answer cap. The test call and the memory suggestion ignore it.
mod common;

use common::*;
use serde_json::{json, Value};

/// Made-up reasoning words that must never show up anywhere.
const SECRET: &str = "zebra-thought-7781";

fn openai_sse(deltas: &[Value]) -> String {
    let mut s = String::new();
    for d in deltas {
        s.push_str(&format!(
            "data: {}\n\n",
            json!({"choices": [{"index": 0, "delta": d}]})
        ));
    }
    s.push_str(&format!(
        "data: {}\n\n",
        json!({"choices": [], "usage": {"prompt_tokens": 9, "completion_tokens": 4}})
    ));
    s.push_str("data: [DONE]\n\n");
    s
}

fn anthropic_sse(events: &[Value]) -> String {
    let mut s = String::new();
    for e in events {
        let name = e["type"].as_str().unwrap_or("message");
        s.push_str(&format!("event: {name}\ndata: {e}\n\n"));
    }
    s
}

/// Everything the AI service keeps on disk, as text.
fn disk_text(t: &TestApp) -> String {
    let mut all = String::new();
    for suffix in ["", "-wal"] {
        let p = format!("{}{suffix}", t.db_path().display());
        if let Ok(b) = std::fs::read(&p) {
            all.push_str(&String::from_utf8_lossy(&b));
        }
    }
    all
}

async fn stream_raw(t: &TestApp, who: &Who, body: Value) -> (Vec<(String, Value)>, String) {
    let mut s = t
        .open_stream("/api/ai/send", who, body)
        .await
        .expect("stream");
    let ev = s.all().await;
    (ev, s.raw.clone())
}

fn assert_never_kept(t: &TestApp, raw: &str) {
    assert!(
        !raw.contains(SECRET),
        "the SSE body carries reasoning: {raw}"
    );
    assert!(!disk_text(t).contains(SECRET), "reasoning was stored");
    assert!(!captured_contains(SECRET), "reasoning was logged");
}

#[tokio::test]
async fn openai_reasoning_gives_one_thinking_frame_before_the_text() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-reasoning").await;
    let body = openai_sse(&[
        json!({"role": "assistant", "content": ""}),
        json!({"reasoning_content": format!("{SECRET} step one")}),
        json!({"reasoning_content": format!("{SECRET} step two")}),
        json!({"reasoning": format!("{SECRET} step three")}),
        json!({"content": "Visible "}),
        json!({"reasoning_content": format!("{SECRET} late")}),
        json!({"content": "answer."}),
    ]);
    t.stub.force(Some((200, &body)));
    let (ev, raw) = stream_raw(&t, &sam, json!({"text": "Think first"})).await;
    assert_eq!(
        names(&ev),
        vec!["start", "thinking", "delta", "delta", "done"],
        "{ev:?}"
    );
    assert_eq!(ev[1].1, json!({}));
    assert!(raw.contains("event: thinking\ndata: {}\n\n"), "{raw}");
    assert_eq!(text(&ev), "Visible answer.");
    assert_eq!(
        terminal(&ev).1,
        json!({"status": "done", "tokensIn": 9, "tokensOut": 4, "truncated": false})
    );
    let chat = ev[0].1["chatId"].as_i64().expect("chat");
    let (_, v) = t.get(&format!("/api/ai/chat?id={chat}"), &sam).await;
    assert_eq!(v["messages"][1]["content"], json!("Visible answer."));
    assert_eq!(v["messages"][1]["status"], json!("done"));
    let (status, first): (String, Option<i64>) = t
        .db()
        .query_row(
            "SELECT status, first_byte_ms FROM usage WHERE kind = 'chat'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("usage");
    assert_eq!(status, "ok");
    assert!(first.is_some(), "time to the first text");
    assert_never_kept(&t, &raw);
}

#[tokio::test]
async fn reasoning_after_the_first_text_gives_no_frame() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-reasoning").await;
    let body = openai_sse(&[
        json!({"content": "Straight "}),
        json!({"reasoning_content": format!("{SECRET} afterthought")}),
        json!({"reasoning": format!("{SECRET} more")}),
        json!({"content": "to the point."}),
    ]);
    t.stub.force(Some((200, &body)));
    let (ev, raw) = stream_raw(&t, &sam, json!({"text": "Quick"})).await;
    assert_eq!(
        names(&ev),
        vec!["start", "delta", "delta", "done"],
        "{ev:?}"
    );
    assert_eq!(text(&ev), "Straight to the point.");
    // an ordinary model sends no `thinking` frame at all
    t.stub.force(None);
    t.set_model(&admin, "demo-chat-small").await;
    let ev = t
        .send_all(&sam, json!({"text": "Plain"}))
        .await
        .expect("stream");
    assert!(!names(&ev).iter().any(|n| n == "thinking"), "{ev:?}");
    assert_never_kept(&t, &raw);
}

#[tokio::test]
async fn reasoning_does_not_count_towards_the_answer_cap() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-reasoning").await;
    // 150 000 characters of reasoning (more than the 100 000-character answer cap), then a short answer
    let long = format!("{SECRET} {}", "r".repeat(50_000));
    let body = openai_sse(&[
        json!({"reasoning_content": long}),
        json!({"reasoning_content": long}),
        json!({"reasoning": long}),
        json!({"content": "Short."}),
    ]);
    t.stub.force(Some((200, &body)));
    let (ev, raw) = stream_raw(&t, &sam, json!({"text": "Long think"})).await;
    assert_eq!(names(&ev), vec!["start", "thinking", "delta", "done"]);
    assert_eq!(terminal(&ev).1["truncated"], json!(false));
    assert_eq!(text(&ev), "Short.");
    assert_never_kept(&t, &raw);
}

#[tokio::test]
async fn anthropic_thinking_blocks_give_one_thinking_frame() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    let (s, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"provider": "anthropic", "baseUrl": format!("{}/anthropic", t.stub.url), "apiKey": KEY, "model": "demo-claude"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    t.verify_and_enable(&admin).await;
    let body = anthropic_sse(&[
        json!({"type": "message_start", "message": {"usage": {"input_tokens": 6}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": format!("{SECRET} idea")}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": "c2lnbmF0dXJlLWRlbW8="}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "content_block_start", "index": 1, "content_block": {"type": "redacted_thinking", "data": format!("{SECRET}-redacted")}}),
        json!({"type": "content_block_stop", "index": 1}),
        json!({"type": "content_block_start", "index": 2, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 2, "delta": {"type": "text_delta", "text": "Claude-style answer."}}),
        json!({"type": "content_block_stop", "index": 2}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 5}}),
        json!({"type": "message_stop"}),
    ]);
    t.stub.force(Some((200, &body)));
    let (ev, raw) = stream_raw(&t, &sam, json!({"text": "Think, Claude"})).await;
    assert_eq!(
        names(&ev),
        vec!["start", "thinking", "delta", "done"],
        "{ev:?}"
    );
    assert_eq!(text(&ev), "Claude-style answer.");
    assert_eq!(
        terminal(&ev).1,
        json!({"status": "done", "tokensIn": 6, "tokensOut": 5, "truncated": false})
    );
    // a redacted block alone (no thinking delta) also counts
    let body = anthropic_sse(&[
        json!({"type": "message_start", "message": {"usage": {"input_tokens": 2}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "redacted_thinking", "data": SECRET}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "Done."}}),
        json!({"type": "message_stop"}),
    ]);
    t.stub.force(Some((200, &body)));
    let (ev, raw2) = stream_raw(&t, &sam, json!({"text": "Again"})).await;
    assert_eq!(
        names(&ev),
        vec!["start", "thinking", "delta", "done"],
        "{ev:?}"
    );
    assert_never_kept(&t, &raw);
    assert_never_kept(&t, &raw2);
}

#[tokio::test]
async fn the_test_call_and_the_memory_suggestion_ignore_reasoning() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-reasoning").await;
    let ev = t
        .send_all(&sam, json!({"text": "Remember that I like tables"}))
        .await
        .expect("stream");
    let msg = ev[0].1["userMessageId"].as_i64().expect("message");
    let body = openai_sse(&[
        json!({"reasoning_content": format!("{SECRET} ponder")}),
        json!({"content": "Likes tables."}),
    ]);
    t.stub.force(Some((200, &body)));
    let (code, v) = t
        .post(
            "/api/ai/memories/suggest",
            &sam,
            json!({"messageId": msg, "lang": "en"}),
        )
        .await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(v, json!({"suggestion": "Likes tables."}));
    let (code, v) = t.post("/api/ai/admin/verify", &admin, json!({})).await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["ok"], json!(true), "{v}");
    assert!(!v.to_string().contains(SECRET));
    // reasoning without any text is no suggestion
    let body = openai_sse(&[json!({"reasoning": format!("{SECRET} only")})]);
    t.stub.force(Some((200, &body)));
    let (code, v) = t
        .post(
            "/api/ai/memories/suggest",
            &sam,
            json!({"messageId": msg, "lang": "en"}),
        )
        .await;
    assert_eq!(code, 502, "{v}");
    assert_eq!(v["code"], json!("ai.up.protocol"));
    assert_never_kept(&t, "");
}
