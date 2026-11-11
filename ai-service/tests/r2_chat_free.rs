//! Free mode (round 2, item 27, reworked): a state of the administrator, not of a chat. While it is on, every message in
//! any chat (old or new) is answered with the language line alone and no memories; while it is off, every message is
//! normal. Plain users never have it. `POST /api/ai/free`, `freeMode` in the status.
mod common;

use common::*;
use serde_json::{json, Value};

fn line(language: &str) -> String {
    format!(
        "Always reply in {language} (the language chosen in the user's Xexam settings), unless the user explicitly asks for another language in this message."
    )
}

fn messages_of(req: &StubRequest) -> Vec<Value> {
    req.body["messages"].as_array().cloned().unwrap_or_default()
}

fn system_of(req: &StubRequest) -> String {
    messages_of(req)[0]["content"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

async fn set_free(t: &TestApp, who: &Who, on: bool) {
    let (code, v) = t.post("/api/ai/free", who, json!({"free": on})).await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(v, json!({"free": on}));
}

#[tokio::test]
async fn the_mode_follows_the_administrator_into_old_and_new_chats() {
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
    let (_, st) = t.get("/api/ai/status", &admin).await;
    assert_eq!(st["freeMode"], json!(false));

    // an old chat, written in normal mode
    let ev = t
        .send_all(&admin, json!({"chatId": null, "text": "One", "lang": "en"}))
        .await
        .expect("stream");
    assert_eq!(ev[0].1["free"], json!(false));
    let old = ev[0].1["chatId"].as_i64().expect("chat");
    let sys = system_of(&t.stub.last());
    assert!(sys.starts_with("You are the study assistant inside Xexam"));
    assert!(sys.contains("\n- Prefers metric units."), "{sys}");

    // on: a send in the OLD chat is free (language line, no memories); the history stays
    set_free(&t, &admin, true).await;
    let (_, st) = t.get("/api/ai/status", &admin).await;
    assert_eq!(st["freeMode"], json!(true));
    let ev = t
        .send_all(&admin, json!({"chatId": old, "text": "Two", "lang": "ja"}))
        .await
        .expect("stream");
    assert_eq!(ev[0].1["free"], json!(true));
    assert_eq!(ev[0].1["newChat"], json!(false));
    let msgs = messages_of(&t.stub.last());
    assert_eq!(msgs[0]["role"], json!("system"));
    assert!(msgs[0]["content"]
        .as_str()
        .unwrap_or("")
        .starts_with(&line("Japanese")));
    assert_eq!(msgs.len(), 4, "system + the earlier two + the new question");
    assert!(!msgs
        .iter()
        .any(|m| m["content"].as_str().unwrap_or("").contains("metric")));

    // a NEW chat is free too
    let ev = t
        .send_all(
            &admin,
            json!({"chatId": null, "text": "Fresh", "lang": "en"}),
        )
        .await
        .expect("stream");
    assert_eq!(ev[0].1["free"], json!(true));
    assert!(system_of(&t.stub.last()).ends_with(&line("English")));

    // a retry uses the mode at the time of the retry
    t.stub.force(Some((
        500,
        r#"{"error":{"message":"made-up failure","type":"server_error"}}"#,
    )));
    let ev = t
        .send_all(
            &admin,
            json!({"chatId": old, "text": "Three", "lang": "en"}),
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
            json!({"chatId": old, "lang": "en"}),
        )
        .await
        .expect("retry")
        .all()
        .await;
    assert_eq!(ev[0].1["retry"], json!(true));
    assert_eq!(ev[0].1["free"], json!(false));
    assert!(system_of(&t.stub.last()).contains("Prefers metric units."));

    // off: normal again, in the same old chat
    let ev = t
        .send_all(&admin, json!({"chatId": old, "text": "Four"}))
        .await
        .expect("stream");
    assert_eq!(ev[0].1["free"], json!(false));
    let sys = system_of(&t.stub.last());
    assert!(sys.starts_with("You are the study assistant inside Xexam"));
    assert!(sys.contains("\n- Prefers metric units."));
    let (_, st) = t.get("/api/ai/status", &admin).await;
    assert_eq!(st["freeMode"], json!(false));

    // a `free` in the send body is ignored (no error, no effect); chats carry no mode
    let ev = t
        .send_all(&admin, json!({"chatId": old, "text": "Five", "free": true}))
        .await
        .expect("stream");
    assert_eq!(ev[0].1["free"], json!(false));
    let (_, list) = t.get("/api/ai/chats", &admin).await;
    assert!(list["chats"][0].get("free").is_none(), "{list}");
}

#[tokio::test]
async fn only_administrators_have_free_mode() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let (code, v) = t.post("/api/ai/free", &sam, json!({"free": true})).await;
    assert_eq!(code, 403, "{v}");
    assert_eq!(v["code"], json!("ai.adminOnly"));
    let (_, st) = t.get("/api/ai/status", &sam).await;
    assert_eq!(st["freeMode"], json!(false));
    // a stored flag never counts for a plain user (e.g. after a demotion)
    set_free(&t, &admin, true).await;
    t.db()
        .execute("UPDATE users SET free_mode = 1", [])
        .expect("flag");
    let (_, st) = t.get("/api/ai/status", &sam).await;
    assert_eq!(st["freeMode"], json!(false));
    let ev = t
        .send_all(&sam, json!({"chatId": null, "text": "hi", "free": true}))
        .await
        .expect("stream");
    assert_eq!(ev[0].1["free"], json!(false));
    assert!(system_of(&t.stub.last()).starts_with("You are the study assistant inside Xexam"));
    // the mode of one administrator is not the mode of another
    let other = t.user("other", true, true);
    let (_, st) = t.get("/api/ai/status", &other).await;
    assert_eq!(st["freeMode"], json!(false));
}

#[tokio::test]
async fn the_free_route_validates_its_body() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let bad: Vec<Value> = vec![
        json!({}),
        json!({"free": "true"}),
        json!({"free": 1}),
        json!({"free": null}),
        json!({"free": true, "extra": 1}),
        json!({"id": 3, "free": true}),
    ];
    for body in bad {
        let (code, v) = t.post("/api/ai/free", &admin, body.clone()).await;
        assert_eq!(code, 400, "{body}: {v}");
        assert_eq!(v["code"], json!("ai.badRequest"));
    }
    let (code, _) = t.get("/api/ai/free", &admin).await;
    assert_eq!(code, 405);
    let (code, _) = t
        .post("/api/ai/chats/free", &admin, json!({"id": 1, "free": true}))
        .await;
    assert_eq!(code, 404, "the per-chat route is gone");
    // the switch needs no chat and no AI+ configuration
    set_free(&t, &admin, true).await;
    set_free(&t, &admin, true).await;
    set_free(&t, &admin, false).await;
}
