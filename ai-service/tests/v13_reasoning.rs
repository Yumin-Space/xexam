//! v1.3 AI Chat reasoning: the model's reasoning streams as `reasoning` frames, is closed by one `reasoningDone`, is
//! stored per answer version (trimmed, capped at 32 KiB, head kept) and shown the same way to the owner and to an
//! administrator. It is never sent back to the model, never logged, and goes with its chat. Generation and import drop
//! it. (The study routes have their own cases in `study_api/reasoning.rs`.) Every text here is made up.
mod common;

use common::genkit::*;
use common::*;
use serde_json::{json, Value};
use std::sync::atomic::Ordering::SeqCst;
use std::time::Duration;
use xexam_ai_service::extract::testgen::{self, DocxSpec};

/// Made-up reasoning words that AI Chat keeps and nothing else may ever show.
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

/// Everything the AI service keeps on disk, as text (database and WAL).
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

/// Admin + AI+ user + one working OpenAI-format connection.
async fn setup() -> (TestApp, Who, Who) {
    let t = start().await;
    t.relax_caps();
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    (t, admin, sam)
}

async fn ask(t: &TestApp, who: &Who, path: &str, body: Value) -> (Vec<(String, Value)>, String) {
    let mut s = t
        .open_stream(path, who, body)
        .await
        .unwrap_or_else(|e| panic!("{path}: {e:?}"));
    let ev = s.all().await;
    (ev, s.raw.clone())
}

async fn send(
    t: &TestApp,
    who: &Who,
    chat: Option<i64>,
    text: &str,
) -> (Vec<(String, Value)>, String) {
    ask(
        t,
        who,
        "/api/ai/send",
        json!({"chatId": chat, "text": text, "lang": "en"}),
    )
    .await
}

fn chat_of(ev: &[(String, Value)]) -> i64 {
    ev[0].1["chatId"].as_i64().expect("chat")
}

async fn read(t: &TestApp, who: &Who, chat: i64) -> Value {
    let (code, v) = t.get(&format!("/api/ai/chat?id={chat}"), who).await;
    assert_eq!(code, 200, "{v}");
    v
}

/// The three reasoning fields of a message.
fn fields(m: &Value) -> (Value, Value, Value) {
    (
        m["reasoning"].clone(),
        m["reasoningMs"].clone(),
        m["reasoningTruncated"].clone(),
    )
}

fn set_reasoning(t: &TestApp, text: Option<&str>, tag: Option<&'static str>) {
    let mut st = t.stub.st();
    st.reasoning = text.map(str::to_string);
    st.reasoning_tag = tag;
}

fn frames(ev: &[(String, Value)], name: &str) -> Vec<Value> {
    ev.iter()
        .filter(|(n, _)| n == name)
        .map(|(_, v)| v.clone())
        .collect()
}

fn reasoning_text(ev: &[(String, Value)]) -> String {
    frames(ev, "reasoning")
        .iter()
        .filter_map(|v| v["text"].as_str())
        .collect()
}

fn not_logged() {
    assert!(!captured_contains(SECRET), "reasoning was logged");
}

// —— AI Chat: frames, storage ————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn reasoning_streams_as_frames_and_is_stored_per_answer() {
    let (t, _admin, sam) = setup().await;
    let body = openai_sse(&[
        json!({"role": "assistant", "content": ""}),
        json!({"reasoning_content": format!("\n{SECRET} one ")}),
        json!({"reasoning_content": "two "}),
        json!({"reasoning": "three\n\n"}),
        json!({"content": "Visible "}),
        json!({"reasoning_content": "LATE-words"}),
        json!({"content": "answer."}),
    ]);
    t.stub.force(Some((200, &body)));
    let (ev, raw) = send(&t, &sam, None, "Think first").await;
    assert_eq!(
        names(&ev),
        vec![
            "start",
            "thinking",
            "reasoning",
            "reasoning",
            "reasoning",
            "reasoningDone",
            "delta",
            "delta",
            "done"
        ],
        "{ev:?}"
    );
    // the frames carry the raw pieces; the late reasoning is in no frame
    assert_eq!(reasoning_text(&ev), format!("\n{SECRET} one two three\n\n"));
    assert!(!raw.contains("LATE-words"), "{raw}");
    let done = &frames(&ev, "reasoningDone")[0];
    let ms = done["ms"].as_i64().expect("ms");
    assert!(ms >= 0);
    assert_eq!(done["truncated"], json!(false));
    assert_eq!(text(&ev), "Visible answer.");
    // stored trimmed, per answer, with the duration of the frame
    let v = read(&t, &sam, chat_of(&ev)).await;
    assert_eq!(v["messages"][1]["content"], json!("Visible answer."));
    assert_eq!(
        fields(&v["messages"][1]),
        (
            json!(format!("{SECRET} one two three")),
            json!(ms),
            json!(false)
        )
    );
    assert!(!disk_text(&t).contains("LATE-words"), "late text stored");
    // the usage row is as without reasoning
    let (tin, tout, first): (Option<i64>, Option<i64>, Option<i64>) = t
        .db()
        .query_row(
            "SELECT tokens_in, tokens_out, first_byte_ms FROM usage WHERE kind = 'chat'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("usage");
    assert_eq!((tin, tout), (Some(9), Some(4)));
    assert!(first.is_some(), "time to the first answer text");
    not_logged();
}

#[tokio::test]
async fn think_thinking_and_thought_spans_are_reasoning_in_ai_chat() {
    for tag in ["think", "thinking", "thought"] {
        let (t, _admin, sam) = setup().await;
        set_reasoning(&t, Some(SECRET), Some(tag));
        let (ev, raw) = send(&t, &sam, None, "Hello").await;
        assert_eq!(
            names(&ev)[..4],
            ["start", "thinking", "reasoning", "reasoningDone"]
        );
        assert_eq!(terminal(&ev).0, "done", "{tag}: {ev:?}");
        assert_eq!(reasoning_text(&ev), SECRET, "{tag}");
        assert_eq!(
            text(&ev),
            ANSWER,
            "{tag}: the span is not part of the answer"
        );
        assert!(!raw.contains(&format!("<{tag}>")), "{tag}: {raw}");
        let v = read(&t, &sam, chat_of(&ev)).await;
        assert_eq!(v["messages"][1]["content"], json!(ANSWER), "{tag}");
        assert_eq!(v["messages"][1]["reasoning"], json!(SECRET), "{tag}");
    }
    not_logged();
}

#[tokio::test]
async fn an_unterminated_span_is_reasoning_and_the_answer_is_empty() {
    let (t, _admin, sam) = setup().await;
    let body = openai_sse(&[
        json!({"content": "<thought>the answer is A"}),
        json!({"content": format!(" and more {SECRET}")}),
    ]);
    t.stub.force(Some((200, &body)));
    let (ev, _) = send(&t, &sam, None, "Hello").await;
    let n = names(&ev);
    assert_eq!(n.first().map(String::as_str), Some("start"), "{ev:?}");
    assert_eq!(n.last().map(String::as_str), Some("done"), "{ev:?}");
    assert!(!n.iter().any(|x| x == "delta"), "{ev:?}");
    let done_at = n.iter().position(|x| x == "reasoningDone").expect("done");
    let last_reasoning = n.iter().rposition(|x| x == "reasoning").expect("frames");
    assert!(last_reasoning < done_at, "{ev:?}");
    assert_eq!(
        reasoning_text(&ev),
        format!("the answer is A and more {SECRET}")
    );
    let v = read(&t, &sam, chat_of(&ev)).await;
    let m = &v["messages"][1];
    assert_eq!(m["status"], json!("done"));
    assert_eq!(m["content"], json!(""));
    assert_eq!(
        m["reasoning"],
        json!(format!("the answer is A and more {SECRET}"))
    );
    not_logged();
}

#[tokio::test]
async fn anthropic_thinking_is_reasoning_and_redacted_alone_shows_nothing() {
    let (t, admin, sam) = setup().await;
    t.configure_anthropic(&admin, "demo-claude").await;
    let body = anthropic_sse(&[
        json!({"type": "message_start", "message": {"usage": {"input_tokens": 6}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": format!("{SECRET} idea")}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": "c2lnLWRlbW8="}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "Claude-style answer."}}),
        json!({"type": "content_block_stop", "index": 1}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 5}}),
        json!({"type": "message_stop"}),
    ]);
    t.stub.force(Some((200, &body)));
    let (ev, raw) = send(&t, &sam, None, "Think").await;
    assert_eq!(
        names(&ev),
        vec![
            "start",
            "thinking",
            "reasoning",
            "reasoningDone",
            "delta",
            "done"
        ],
        "{ev:?}"
    );
    assert_eq!(reasoning_text(&ev), format!("{SECRET} idea"));
    assert!(
        !raw.contains("c2lnLWRlbW8="),
        "the signature is never shown"
    );
    let v = read(&t, &sam, chat_of(&ev)).await;
    assert_eq!(
        v["messages"][1]["reasoning"],
        json!(format!("{SECRET} idea"))
    );
    // a redacted block alone: the fact frame only, nothing stored
    let body = anthropic_sse(&[
        json!({"type": "message_start", "message": {"usage": {"input_tokens": 2}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "redacted_thinking", "data": "cmVkYWN0ZWQtZGVtbw=="}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "Done."}}),
        json!({"type": "message_stop"}),
    ]);
    t.stub.force(Some((200, &body)));
    let (ev, raw) = send(&t, &sam, None, "Again").await;
    assert_eq!(
        names(&ev),
        vec!["start", "thinking", "delta", "done"],
        "{ev:?}"
    );
    assert!(!raw.contains("cmVkYWN0ZWQtZGVtbw=="));
    let v = read(&t, &sam, chat_of(&ev)).await;
    assert_eq!(
        fields(&v["messages"][1]),
        (Value::Null, Value::Null, json!(false))
    );
    not_logged();
}

#[tokio::test]
async fn reasoning_is_capped_at_32_kib_keeping_the_head() {
    let (t, _admin, sam) = setup().await;
    // 50 000 bytes of two- and three-byte characters, sent in ten deltas of 5 000 bytes
    let all = "é思".repeat(10_000);
    assert_eq!(all.len(), 50_000);
    let pieces: Vec<Value> = all
        .chars()
        .collect::<Vec<_>>()
        .chunks(2_000)
        .map(|c| json!({"reasoning_content": c.iter().collect::<String>()}))
        .collect();
    assert_eq!(pieces.len(), 10);
    let mut deltas = pieces;
    deltas.push(json!({"content": "A complete answer."}));
    let body = openai_sse(&deltas);
    t.stub.force(Some((200, &body)));
    let (ev, _) = send(&t, &sam, None, "Long think").await;
    assert_eq!(terminal(&ev).0, "done");
    assert_eq!(text(&ev), "A complete answer.");
    let sent = reasoning_text(&ev);
    assert!(sent.len() <= 32_768, "{}", sent.len());
    assert!(sent.len() > 32_768 - 4, "{}", sent.len());
    assert!(all.starts_with(&sent), "the head is what was sent");
    let done = &frames(&ev, "reasoningDone")[0];
    assert_eq!(done["truncated"], json!(true));
    let v = read(&t, &sam, chat_of(&ev)).await;
    let m = &v["messages"][1];
    let stored = m["reasoning"].as_str().expect("stored");
    assert!(stored.len() <= 32_768);
    assert!(all.starts_with(stored), "a prefix of the sent text");
    assert_eq!(stored, sent.trim());
    assert_eq!(m["reasoningTruncated"], json!(true));
    assert_eq!(m["content"], json!("A complete answer."));
    assert_eq!(m["status"], json!("done"));
}

// —— never sent back, versions, admin ———————————————————————————————————————————————————————————————————————————————

async fn never_sent_back(anthropic: bool) {
    let (t, admin, sam) = setup().await;
    if anthropic {
        t.configure_anthropic(&admin, "demo-claude").await;
    }
    set_reasoning(&t, Some(SECRET), None);
    let (ev, _) = send(&t, &sam, None, "first").await;
    let chat = chat_of(&ev);
    let v = read(&t, &sam, chat).await;
    assert_eq!(
        v["messages"][1]["reasoning"],
        json!(SECRET),
        "stored at all"
    );
    let q1 = v["messages"][0]["id"].as_i64().expect("q1");
    // later turns, regenerate and edit-and-resend: the model gets answer texts only
    let (ev, _) = send(&t, &sam, Some(chat), "second").await;
    assert_eq!(terminal(&ev).0, "done");
    let body = t.stub.last().body.to_string();
    assert!(!body.contains(SECRET), "send: {body}");
    assert!(body.contains("first"), "the context is there: {body}");
    let a2 = ev[0].1["assistantMessageId"].as_i64().expect("a2");
    let (ev, _) = ask(
        &t,
        &sam,
        "/api/ai/regenerate",
        json!({"chatId": chat, "messageId": a2, "lang": "en"}),
    )
    .await;
    assert_eq!(terminal(&ev).0, "done");
    let body = t.stub.last().body.to_string();
    assert!(!body.contains(SECRET), "regenerate: {body}");
    assert!(body.contains("first"), "{body}");
    let (ev, _) = ask(
        &t,
        &sam,
        "/api/ai/edit",
        json!({"chatId": chat, "messageId": q1, "text": "first, reworded", "lang": "en"}),
    )
    .await;
    assert_eq!(terminal(&ev).0, "done");
    let body = t.stub.last().body.to_string();
    assert!(!body.contains(SECRET), "edit: {body}");
    assert!(body.contains("reworded"), "{body}");
    not_logged();
}

#[tokio::test]
async fn reasoning_is_never_sent_back_to_the_model() {
    never_sent_back(false).await;
    never_sent_back(true).await;
}

#[tokio::test]
async fn each_version_keeps_its_own_reasoning() {
    let (t, admin, sam) = setup().await;
    set_reasoning(&t, Some("reasoning-A"), None);
    let (ev, _) = send(&t, &sam, None, "first").await;
    let chat = chat_of(&ev);
    let a1 = ev[0].1["assistantMessageId"].as_i64().expect("a1");
    set_reasoning(&t, Some("reasoning-B"), None);
    let (ev, _) = ask(
        &t,
        &sam,
        "/api/ai/regenerate",
        json!({"chatId": chat, "messageId": a1, "lang": "en"}),
    )
    .await;
    let a2 = ev[0].1["assistantMessageId"].as_i64().expect("a2");
    assert_ne!(a1, a2);
    let owner = read(&t, &sam, chat).await;
    assert_eq!(owner["messages"][1]["reasoning"], json!("reasoning-B"));
    for (at, want) in [(a1, "reasoning-A"), (a2, "reasoning-B")] {
        let (c, v) = t
            .post(
                "/api/ai/branch",
                &sam,
                json!({"chatId": chat, "messageId": at}),
            )
            .await;
        assert_eq!(c, 200, "{v}");
        assert_eq!(v["messages"][1]["id"], json!(at));
        assert_eq!(v["messages"][1]["reasoning"], json!(want));
        let (c, av) = t
            .get(
                &format!("/api/ai/admin/chat?user={}&id={chat}&at={at}", sam.id),
                &admin,
            )
            .await;
        assert_eq!(c, 200, "{av}");
        assert_eq!(av["messages"][1]["id"], json!(at));
        assert_eq!(av["messages"][1]["reasoning"], json!(want), "admin at={at}");
    }
    // an in-place retry of an empty failed answer clears what that attempt left behind
    let (t, _admin, sam) = {
        drop((t, admin, sam));
        setup().await
    };
    let body = openai_sse(&[
        json!({"reasoning_content": format!("{SECRET} half a thought")}),
        json!({"refusal": "no"}),
    ]);
    t.stub.force(Some((200, &body)));
    let (ev, _) = send(&t, &sam, None, "first").await;
    assert_eq!(terminal(&ev).0, "error", "{ev:?}");
    let chat = chat_of(&ev);
    let a = ev[0].1["assistantMessageId"].as_i64().expect("a");
    let v = read(&t, &sam, chat).await;
    assert_eq!(v["messages"][1]["content"], json!(""));
    assert_eq!(
        v["messages"][1]["reasoning"],
        json!(format!("{SECRET} half a thought")),
        "the failed attempt kept its reasoning"
    );
    t.stub.force(None);
    set_reasoning(&t, None, None);
    let (ev, _) = ask(
        &t,
        &sam,
        "/api/ai/regenerate",
        json!({"chatId": chat, "messageId": a}),
    )
    .await;
    assert_eq!(ev[0].1["retry"], json!(true));
    assert_eq!(ev[0].1["assistantMessageId"], json!(a));
    assert_eq!(terminal(&ev).0, "done");
    let v = read(&t, &sam, chat).await;
    assert_eq!(
        fields(&v["messages"][1]),
        (Value::Null, Value::Null, json!(false))
    );
    t.app.0.store.checkpoint_now().expect("checkpoint");
    assert!(
        !disk_text(&t).contains(SECRET),
        "old reasoning left on disk"
    );
    not_logged();
}

#[tokio::test]
async fn admin_view_shows_the_owners_reasoning() {
    let (t, admin, sam) = setup().await;
    set_reasoning(&t, Some(SECRET), None);
    let (ev, _) = send(&t, &sam, None, "first").await;
    let chat = chat_of(&ev);
    let owner = read(&t, &sam, chat).await;
    let (c, av) = t
        .get(
            &format!("/api/ai/admin/chat?user={}&id={chat}", sam.id),
            &admin,
        )
        .await;
    assert_eq!(c, 200, "{av}");
    for i in 0..2 {
        assert_eq!(
            fields(&av["messages"][i]),
            fields(&owner["messages"][i]),
            "{i}"
        );
    }
    assert_eq!(av["messages"][1]["reasoning"], json!(SECRET));
    assert!(av["messages"][1]["reasoningMs"].as_i64().is_some());
    not_logged();
}

#[tokio::test]
async fn answers_without_reasoning_read_as_null() {
    let (t, admin, sam) = setup().await;
    let (ev, _) = send(&t, &sam, None, "plain").await;
    assert!(!names(&ev).iter().any(|n| n.starts_with("reasoning")));
    let chat = chat_of(&ev);
    let owner = read(&t, &sam, chat).await;
    let (_, av) = t
        .get(
            &format!("/api/ai/admin/chat?user={}&id={chat}", sam.id),
            &admin,
        )
        .await;
    for v in [&owner, &av] {
        for i in 0..2 {
            let m = &v["messages"][i];
            for k in ["reasoning", "reasoningMs", "reasoningTruncated"] {
                assert!(m.get(k).is_some(), "{k} is always present");
            }
            assert_eq!(
                fields(m),
                (Value::Null, Value::Null, json!(false)),
                "message {i}"
            );
        }
    }
}

// —— deletion, titles, memory, usage ————————————————————————————————————————————————————————————————————————————————

fn with_reasoning(t: &TestApp) -> i64 {
    t.db()
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE reasoning IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .expect("count")
}

#[tokio::test]
async fn reasoning_goes_with_its_chat() {
    let (t, admin, sam) = setup().await;
    set_reasoning(&t, Some(SECRET), None);
    // the owner deletes the chat
    let (ev, _) = send(&t, &sam, None, "to be deleted").await;
    let chat = chat_of(&ev);
    assert_eq!(with_reasoning(&t), 1);
    assert!(disk_text(&t).contains(SECRET), "stored before the delete");
    let (c, v) = t
        .post("/api/ai/chats/delete", &sam, json!({"id": chat}))
        .await;
    assert_eq!((c, v), (200, json!({"deleted": 1})));
    t.app.0.store.checkpoint_now().expect("checkpoint");
    assert_eq!(with_reasoning(&t), 0);
    assert!(!disk_text(&t).contains(SECRET), "owner delete");
    // an administrator clears one
    let (ev, _) = send(&t, &sam, None, "to be cleared").await;
    let chat = chat_of(&ev);
    assert_eq!(with_reasoning(&t), 1);
    let (c, v) = t
        .post(
            "/api/ai/admin/clear",
            &admin,
            json!({"userId": sam.id, "what": "chat", "chatId": chat}),
        )
        .await;
    assert_eq!((c, v), (200, json!({"deleted": 1})));
    t.app.0.store.checkpoint_now().expect("checkpoint");
    assert_eq!(with_reasoning(&t), 0);
    assert!(!disk_text(&t).contains(SECRET), "admin clear");
    not_logged();
}

#[tokio::test]
async fn title_memory_and_usage_ignore_reasoning() {
    let (t, _admin, sam) = setup().await;
    let body = openai_sse(&[
        json!({"reasoning_content": format!("{SECRET} planning")}),
        json!({"content": "Plan answer."}),
    ]);
    t.stub.force(Some((200, &body)));
    let (ev, _) = send(&t, &sam, None, "Plan my week").await;
    let chat = chat_of(&ev);
    let v = read(&t, &sam, chat).await;
    assert_eq!(v["chat"]["title"], json!("Plan my week"));
    assert!(!v["chat"].to_string().contains(SECRET));
    let (tin, tout): (Option<i64>, Option<i64>) = t
        .db()
        .query_row(
            "SELECT tokens_in, tokens_out FROM usage WHERE kind = 'chat'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("usage");
    assert_eq!(
        (tin, tout),
        (Some(9), Some(4)),
        "tokens as the provider says"
    );
    let chats: Value = t.get("/api/ai/chats", &sam).await.1;
    assert!(!chats.to_string().contains(SECRET), "the chat list");
    // the memory suggestion uses the user's message and returns the answer text only
    let msg = ev[0].1["userMessageId"].as_i64().expect("message");
    let body = openai_sse(&[
        json!({"reasoning_content": format!("{SECRET} ponder")}),
        json!({"content": "Likes plans."}),
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
    assert_eq!(v, json!({"suggestion": "Likes plans."}));
    // and the model call of the suggestion carried no reasoning either
    assert!(!t.stub.last().body.to_string().contains(SECRET));
    not_logged();
}

// —— generation and import ——————————————————————————————————————————————————————————————————————————————————————————

struct E {
    t: TestApp,
    user: Who,
}

async fn env() -> E {
    let t = start().await;
    let admin = t.user("root", true, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let user = t.user("alexandra_k", false, true);
    t.backend.st().gen = kitchen(2, 20);
    t.stub.st().script = Some(good_model());
    t.app.0.knobs.backoff_scale.store(0, SeqCst);
    t.app.0.knobs.place_poll_ms.store(20, SeqCst);
    // the whole-file import path
    t.app.0.knobs.import_context_tokens.store(0, SeqCst);
    *t.app.0.knobs.exe_override.lock().unwrap() =
        Some(env!("CARGO_BIN_EXE_xexam-ai-service").into());
    t.relax_caps();
    E { t, user }
}

async fn wait_job(e: &E, path: &str, id: i64, phase: Option<&str>, states: &[&str]) -> Value {
    for _ in 0..800 {
        let (s, v) = e.t.get(&format!("{path}?id={id}"), &e.user).await;
        assert_eq!(s, 200, "{v}");
        let j = v["job"].clone();
        if states.contains(&j["state"].as_str().unwrap_or(""))
            || (phase.is_some() && j["phase"].as_str() == phase)
        {
            return j;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the job never reached {phase:?} / {states:?}");
}

async fn run_generation(e: &E) {
    let (s, v) =
        e.t.post(
            "/api/ai/gen/start",
            &e.user,
            json!({"source": "bank", "bankId": 3}),
        )
        .await;
    assert_eq!(s, 202, "{v}");
    let id = v["job"]["id"].as_i64().expect("id");
    let j = wait_job(
        e,
        "/api/ai/gen/job",
        id,
        None,
        &["done", "failed", "cancelled"],
    )
    .await;
    assert_eq!(j["state"], json!("done"), "{j}");
}

fn six() -> Vec<String> {
    let mut v = vec!["KIND:QUESTIONS zebrafishmarker notes".to_string()];
    for i in 1..=4 {
        v.push(format!(
            "Q{i}. Which kitchen device {} keeps a stock pot at a steady simmer?",
            en_text(300 + i, 3)
        ));
        for (k, l) in ["A", "B", "C", "D"].iter().enumerate() {
            v.push(format!(
                "{l}. {} {}",
                pw(500 + i * 10 + k),
                pw(600 + i * 10 + k)
            ));
        }
        v.push("Answer: B".to_string());
    }
    v
}

async fn run_import(e: &E) {
    let refs: Vec<String> = six();
    let lines: Vec<&str> = refs.iter().map(String::as_str).collect();
    let docx = testgen::docx(&DocxSpec::paragraphs(&lines));
    let (s, v) = e.t.upload(&e.user, "Kitchen notes.docx", docx).await;
    assert_eq!(s, 202, "{v}");
    let id = v["job"]["id"].as_i64().expect("id");
    wait_job(
        e,
        "/api/ai/gen/job",
        id,
        Some("confirm"),
        &["failed", "cancelled"],
    )
    .await;
    let (s, v) =
        e.t.post(
            "/api/ai/import/confirm",
            &e.user,
            json!({"id": id, "name": "Kitchen notes (typed)"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let j = wait_job(e, "/api/ai/gen/job", id, None, &["done", "failed"]).await;
    assert_eq!(j["state"], json!("done"), "{j}");
}

/// Everything the fake backend was sent, as text.
fn backend_text(t: &TestApp) -> String {
    let logs = t.backend.logs().join("\n");
    let st = t.backend.st();
    let mut all = String::new();
    for part in [
        &st.gen.created,
        &st.gen.check_requests,
        &st.gen.source_requests,
        &st.gen.corpus_requests,
    ] {
        all.push_str(&serde_json::to_string(part).expect("json"));
    }
    all.push_str(&logs);
    all
}

#[tokio::test]
async fn generation_and_import_never_see_reasoning() {
    for tag in [None, Some("thought")] {
        let e = env().await;
        set_reasoning(&e.t, Some(SECRET), tag);
        run_generation(&e).await;
        run_import(&e).await;
        assert!(e.t.stub.count() > 0);
        assert!(!backend_text(&e.t).contains(SECRET), "{tag:?}: the backend");
        let results: String =
            e.t.db()
                .query_row(
                    "SELECT COALESCE(group_concat(result), '') FROM tasks",
                    [],
                    |r| r.get(0),
                )
                .expect("tasks");
        assert!(!results.contains(SECRET), "{tag:?}: tasks.result");
        assert_eq!(with_reasoning(&e.t), 0, "{tag:?}");
        // gen_calls, gen_items, tasks and every other table: the database file as text
        assert!(!disk_text(&e.t).contains(SECRET), "{tag:?}: on disk");
    }
    not_logged();
}
