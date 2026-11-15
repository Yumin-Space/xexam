//! v1.3 sprint 2 (owner): an ordinary user never learns which AI answers. No model, connection, provider, host or
//! gateway field or value reaches a non-administrator in AI Chat (stream frames, chat reads, version switches, lists,
//! status) or in generation jobs. Administrators keep `connection` and `model`, in their own chats and in the admin chat
//! viewer. The stored rows are unchanged. Everything is made up.
mod common;

use common::genkit::*;
use common::*;
use serde_json::{json, Value};
use std::time::Duration;

const MODEL: &str = "demo-chat-small";
const CONN: &str = "main";

/// Nothing in `raw` names the AI: no hidden key and none of the stub's names.
fn assert_hidden(raw: &str, model: &str, conn: &str) {
    for k in [
        "\"model\"",
        "\"modelFallback\"",
        "\"connection\"",
        "\"connectionId\"",
        "\"connectionName\"",
        "\"pick\"",
        "\"chatDefault\"",
        "\"provider\"",
        "\"baseUrl\"",
        "\"host\"",
    ] {
        assert!(!raw.contains(k), "{k} in {raw}");
    }
    for v in [model, &format!("\"{conn}\""), "127.0.0.1"] {
        assert!(!raw.contains(v), "{v} in {raw}");
    }
}

fn hidden_json(v: &Value) {
    assert_hidden(&serde_json::to_string(v).expect("json"), MODEL, CONN);
}

async fn setup() -> (TestApp, Who, Who) {
    let t = start().await;
    t.relax_caps();
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, MODEL).await;
    (t, admin, sam)
}

/// (raw SSE text, events) of one stream.
async fn stream(t: &TestApp, who: &Who, path: &str, body: Value) -> (String, Vec<(String, Value)>) {
    let mut s = t
        .open_stream(path, who, body)
        .await
        .unwrap_or_else(|e| panic!("{path}: {e:?}"));
    let ev = s.all().await;
    (s.raw.clone(), ev)
}

async fn send(
    t: &TestApp,
    who: &Who,
    chat: Option<i64>,
    text: &str,
) -> (String, Vec<(String, Value)>) {
    stream(
        t,
        who,
        "/api/ai/send",
        json!({"chatId": chat, "text": text, "lang": "en"}),
    )
    .await
}

fn id_of(ev: &[(String, Value)], key: &str) -> i64 {
    ev[0].1[key].as_i64().expect(key)
}

async fn read(t: &TestApp, who: &Who, chat: i64) -> Value {
    let (code, v) = t.get(&format!("/api/ai/chat?id={chat}"), who).await;
    assert_eq!(code, 200, "{v}");
    v
}

#[tokio::test]
async fn a_user_never_sees_the_model_in_ai_chat() {
    let (t, _admin, sam) = setup().await;
    // send
    let (raw, ev) = send(&t, &sam, None, "first").await;
    assert_eq!(terminal(&ev).0, "done");
    assert_hidden(&raw, MODEL, CONN);
    let (chat, q1_, a1) = (
        id_of(&ev, "chatId"),
        id_of(&ev, "userMessageId"),
        id_of(&ev, "assistantMessageId"),
    );
    hidden_json(&read(&t, &sam, chat).await);
    // regenerate
    let (raw, ev) = stream(
        &t,
        &sam,
        "/api/ai/regenerate",
        json!({"chatId": chat, "messageId": a1, "lang": "en"}),
    )
    .await;
    assert_eq!(terminal(&ev).0, "done");
    assert_hidden(&raw, MODEL, CONN);
    // the version switch back to version 1
    let (c, v) = t
        .post(
            "/api/ai/branch",
            &sam,
            json!({"chatId": chat, "messageId": a1}),
        )
        .await;
    assert_eq!(c, 200, "{v}");
    hidden_json(&v);
    // edit and resend of the question
    let (raw, ev) = stream(
        &t,
        &sam,
        "/api/ai/edit",
        json!({"chatId": chat, "messageId": q1_, "text": "first, reworded"}),
    )
    .await;
    assert_eq!(terminal(&ev).0, "done");
    assert_hidden(&raw, MODEL, CONN);
    let v = read(&t, &sam, chat).await;
    hidden_json(&v);
    assert!(v["messages"].as_array().is_some_and(|m| m.len() >= 2));
    // an empty failed answer, then its in-place retry and the old retry route
    t.stub.force(Some((
        500,
        r#"{"error":{"message":"made-up failure","type":"server_error"}}"#,
    )));
    let (raw, ev) = send(&t, &sam, Some(chat), "will fail").await;
    assert_eq!(terminal(&ev).0, "error");
    assert_hidden(&raw, MODEL, CONN);
    let failed = id_of(&ev, "assistantMessageId");
    t.stub.force(None);
    let (raw, ev) = stream(
        &t,
        &sam,
        "/api/ai/regenerate",
        json!({"chatId": chat, "messageId": failed}),
    )
    .await;
    assert_eq!(ev[0].1["retry"], json!(true));
    assert_eq!(terminal(&ev).0, "done");
    assert_hidden(&raw, MODEL, CONN);
    let (raw, ev) = stream(&t, &sam, "/api/ai/retry", json!({"chatId": chat})).await;
    assert_eq!(terminal(&ev).0, "done");
    assert_hidden(&raw, MODEL, CONN);
    // lists and status
    let (c, v) = t.get("/api/ai/chats", &sam).await;
    assert_eq!(c, 200, "{v}");
    hidden_json(&v);
    let (c, v) = t.get("/api/ai/status", &sam).await;
    assert_eq!(c, 200, "{v}");
    hidden_json(&v);
    // the stored rows still record the model and the connection
    let rows: Vec<(String, String)> = {
        let db = t.db();
        let mut st = db
            .prepare("SELECT model, connection FROM messages WHERE role = 'assistant'")
            .expect("prepare");
        let rows = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("query")
            .map(|r| r.expect("row"))
            .collect();
        rows
    };
    assert!(rows.len() >= 3);
    for (m, c) in rows {
        assert_eq!((m.as_str(), c.as_str()), (MODEL, CONN));
    }
}

#[tokio::test]
async fn an_admin_still_sees_connection_and_model() {
    let (t, admin, sam) = setup().await;
    let (_, ev) = send(&t, &admin, None, "first").await;
    assert_eq!(ev[0].1["model"], json!(MODEL));
    assert_eq!(ev[0].1["connection"], json!(CONN));
    let (chat, q1_, a1) = (
        id_of(&ev, "chatId"),
        id_of(&ev, "userMessageId"),
        id_of(&ev, "assistantMessageId"),
    );
    let v = read(&t, &admin, chat).await;
    assert_eq!(v["messages"][1]["model"], json!(MODEL));
    assert_eq!(v["messages"][1]["connection"], json!(CONN));
    let (_, ev) = stream(
        &t,
        &admin,
        "/api/ai/regenerate",
        json!({"chatId": chat, "messageId": a1}),
    )
    .await;
    assert_eq!(ev[0].1["model"], json!(MODEL));
    assert_eq!(ev[0].1["connection"], json!(CONN));
    let (_, v) = t
        .post(
            "/api/ai/branch",
            &admin,
            json!({"chatId": chat, "messageId": a1}),
        )
        .await;
    assert_eq!(v["messages"][1]["model"], json!(MODEL));
    assert_eq!(v["messages"][1]["connection"], json!(CONN));
    let (_, ev) = stream(
        &t,
        &admin,
        "/api/ai/edit",
        json!({"chatId": chat, "messageId": q1_, "text": "reworded"}),
    )
    .await;
    assert_eq!(ev[0].1["model"], json!(MODEL));
    assert_eq!(ev[0].1["connection"], json!(CONN));
    // the admin viewer shows both on sam's answers
    let (_, sev) = send(&t, &sam, None, "mine").await;
    let schat = id_of(&sev, "chatId");
    let (c, v) = t
        .get(
            &format!("/api/ai/admin/chat?user={}&id={schat}", sam.id),
            &admin,
        )
        .await;
    assert_eq!(c, 200, "{v}");
    let msgs = v["messages"].as_array().expect("messages");
    let answer = msgs
        .iter()
        .find(|m| m["role"] == "assistant")
        .expect("answer");
    assert_eq!(answer["model"], json!(MODEL));
    assert_eq!(answer["connection"], json!(CONN));
}

#[tokio::test]
async fn a_user_sees_no_model_in_frames_after_start() {
    let (t, _admin, sam) = setup().await;
    t.stub.st().reasoning = Some("Made-up reasoning about the question.".to_string());
    let (raw, ev) = send(&t, &sam, None, "think first").await;
    assert_eq!(terminal(&ev).0, "done");
    assert!(names(&ev).iter().any(|n| n == "reasoning"), "{raw}");
    assert_hidden(&raw, MODEL, CONN);
    // an error terminal frame
    t.stub.force(Some((
        500,
        r#"{"error":{"message":"made-up failure","type":"server_error"}}"#,
    )));
    let (raw, ev) = send(&t, &sam, None, "will fail").await;
    assert_eq!(terminal(&ev).0, "error");
    assert_hidden(&raw, MODEL, CONN);
}

#[tokio::test]
async fn a_users_generation_job_names_no_model() {
    use std::sync::atomic::Ordering::SeqCst;
    let t = start().await;
    let admin = t.user("root", true, true);
    t.configure_openai(&admin, MODEL).await;
    t.post(
        "/api/ai/admin/limits",
        &admin,
        json!({"genUnlimited": 0, "importUnlimited": 0}),
    )
    .await;
    let sam = t.user("sam", false, true);
    t.backend.st().gen = kitchen(2, 20);
    t.stub.st().script = Some(good_model());
    t.app.0.knobs.backoff_scale.store(0, SeqCst);
    t.app.0.knobs.place_poll_ms.store(20, SeqCst);
    t.relax_caps();
    let (s, v) = t
        .post(
            "/api/ai/gen/start",
            &sam,
            json!({"source": "bank", "bankId": 3}),
        )
        .await;
    assert_eq!(s, 202, "{v}");
    hidden_json(&v);
    let id = v["job"]["id"].as_i64().expect("id");
    for _ in 0..800 {
        let (s, v) = t.get(&format!("/api/ai/gen/job?id={id}"), &sam).await;
        assert_eq!(s, 200, "{v}");
        hidden_json(&v);
        let state = v["job"]["state"].as_str().unwrap_or("").to_string();
        if state == "done" {
            return;
        }
        assert!(state != "failed" && state != "cancelled", "{v}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the job never finished");
}
