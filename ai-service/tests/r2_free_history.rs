//! Free mode and the history: the full capped history is sent in both modes; the system prompt names the current mode
//! (normal: study prompt + memories + NORMAL block; free: no study prompt, no memories, FREE block). Every message
//! carries `free` in the chat JSON.
mod common;

use common::*;
use serde_json::{json, Value};

fn sent(t: &TestApp) -> String {
    t.stub.last().body["messages"].to_string()
}

fn system(t: &TestApp) -> String {
    t.stub.last().body["messages"][0]["content"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

async fn set_free(t: &TestApp, who: &Who, on: bool) {
    let (code, v) = t.post("/api/ai/free", who, json!({"free": on})).await;
    assert_eq!(code, 200, "{v}");
}

async fn say(t: &TestApp, who: &Who, chat: Value, text: &str) -> i64 {
    let ev = t
        .send_all(who, json!({"chatId": chat, "text": text, "lang": "en"}))
        .await
        .expect("stream");
    ev[0].1["chatId"].as_i64().expect("chat")
}

const NORMAL_BLOCK: &str =
    "Current mode: normal (Xexam study assistant); earlier turns may be free mode.";
const FREE_BLOCK: &str =
    "Current mode: free (general assistant, not the study-assistant role); earlier turns may be normal mode.";

fn assert_normal(sys: &str, block: bool) {
    assert!(
        sys.starts_with("You are the study assistant inside Xexam"),
        "{sys}"
    );
    assert!(
        sys.contains(
            "
- Prefers metric units."
        ),
        "{sys}"
    );
    assert_eq!(sys.contains(NORMAL_BLOCK), block, "{sys}");
    assert!(!sys.contains("Current mode: free"), "{sys}");
    assert!(sys.contains("Always reply in English"));
    if !block {
        assert!(sys.ends_with("in this message."), "{sys}");
    }
    if block {
        assert!(sys.ends_with(NORMAL_BLOCK));
    }
}

fn assert_free(sys: &str, block: bool) {
    assert_eq!(sys.contains(FREE_BLOCK), block, "{sys}");
    assert!(!sys.contains("study assistant inside Xexam"), "{sys}");
    assert!(!sys.contains("metric"), "{sys}");
    assert!(sys.starts_with("Always reply in English"), "{sys}");
    if block {
        assert!(sys.ends_with(FREE_BLOCK) && sys.len() < 400, "short: {sys}");
    } else {
        assert!(sys.ends_with("in this message."), "{sys}");
    }
}

#[tokio::test]
async fn the_full_history_is_sent_and_the_prompt_names_the_mode() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    let (code, v) = t
        .post(
            "/api/ai/memories/add",
            &admin,
            json!({"content": "Prefers metric units."}),
        )
        .await;
    assert_eq!(code, 200, "{v}");

    // one mode only: exactly today's prompts, no block
    let chat = say(&t, &admin, Value::Null, "normal-one-zq").await;
    assert_normal(&system(&t), false);
    say(&t, &admin, json!(chat), "normal-two-zq").await;
    assert_normal(&system(&t), false);
    let lone = {
        set_free(&t, &admin, true).await;
        let c = say(&t, &admin, Value::Null, "free-lone-zq").await;
        assert_free(&system(&t), false);
        say(&t, &admin, json!(c), "free-lone-two-zq").await;
        assert_free(&system(&t), false);
        c
    };
    assert_ne!(lone, chat);
    // switch on in the old chat: the history holds normal turns -> free block, full history
    say(&t, &admin, json!(chat), "free-one-zq").await;
    assert_free(&system(&t), true);
    assert!(
        sent(&t).contains("normal-one-zq"),
        "full history in free mode"
    );
    // switch off: the history holds a free turn -> normal block
    set_free(&t, &admin, false).await;
    say(&t, &admin, json!(chat), "normal-three-zq").await;
    assert_normal(&system(&t), true);
    let s = sent(&t);
    assert!(
        s.contains("normal-one-zq") && s.contains("free-one-zq"),
        "{s}"
    );
    // the other chat now holds free turns and is asked in normal mode: block
    say(&t, &admin, json!(lone), "normal-lone-zq").await;
    assert_normal(&system(&t), true);

    let (code, v) = t.get(&format!("/api/ai/chat?id={chat}"), &admin).await;
    assert_eq!(code, 200, "{v}");
    let flags: Vec<bool> = v["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .map(|m| m["free"].as_bool().expect("free"))
        .collect();
    assert_eq!(
        flags,
        [false, false, false, false, true, true, false, false]
    );
    let keys: Vec<&String> = v["messages"][0].as_object().expect("obj").keys().collect();
    // `free` is followed by the seven keys of v0.9 (study messages)
    let pos = keys
        .iter()
        .position(|k| k.as_str() == "free")
        .expect("free");
    let tail: Vec<&str> = keys[pos + 1..].iter().map(|k| k.as_str()).collect();
    assert_eq!(
        tail,
        [
            "kind",
            "questionId",
            "questionNo",
            "source",
            "lang",
            "recordId",
            "sample"
        ]
    );
}

#[tokio::test]
async fn a_retry_uses_the_mode_at_retry_time() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    let chat = say(&t, &admin, Value::Null, "normal-one-zq").await;
    set_free(&t, &admin, true).await;
    say(&t, &admin, json!(chat), "free-one-zq").await;
    t.stub.force(Some((
        500,
        r#"{"error":{"message":"made-up failure","type":"server_error"}}"#,
    )));
    let ev = t
        .send_all(
            &admin,
            json!({"chatId": chat, "text": "free-two-zq", "lang": "en"}),
        )
        .await
        .expect("stream");
    assert_eq!(terminal(&ev).0, "error");
    t.stub.force(None);
    set_free(&t, &admin, false).await;
    let ev = t
        .open_stream(
            "/api/ai/retry",
            &admin,
            json!({"chatId": chat, "lang": "en"}),
        )
        .await
        .expect("retry")
        .all()
        .await;
    assert_eq!(ev[0].1["free"], json!(false));
    let sys = system(&t);
    assert!(
        sys.ends_with(NORMAL_BLOCK) || sys.contains(NORMAL_BLOCK),
        "{sys}"
    );
    let all = sent(&t);
    assert!(
        all.contains("normal-one-zq") && all.contains("free-one-zq"),
        "{all}"
    );
    let (_, v) = t.get(&format!("/api/ai/chat?id={chat}"), &admin).await;
    let m = v["messages"].as_array().expect("messages");
    let n = m.len();
    assert_eq!(m[n - 1]["free"], json!(false));
    assert_eq!(m[n - 2]["free"], json!(false));
    assert_eq!(m[0]["free"], json!(false));
    assert_eq!(m[n - 3]["free"], json!(true));
}
