//! The chat side of a suspended AI+ function (round 2, items 7 / 16): Status `suspended` / `allowed`, the running answer
//! ends with `ai.suspended` (also when the flag is found by the 2-s re-check), a running memory suggestion is refused
//! with 403 `ai.suspended`. The suspend / resume routes themselves are tested with the admin routes.
mod common;

use common::*;
use serde_json::{json, Value};
use std::time::Duration;
use xexam_ai_service::chat::{Kind, StopReason};

/// Writes the flag directly, as an administrator's Suspend would (without the route's immediate cancel).
fn set_function(t: &TestApp, who: &Who, on: bool) {
    let n = t
        .db()
        .execute(
            "UPDATE users SET function_enabled = ?2 WHERE id = ?1",
            rusqlite::params![who.id, i64::from(on)],
        )
        .expect("update");
    assert_eq!(n, 1, "the user has a row");
}

#[tokio::test]
async fn status_reports_the_suspension_of_plain_users_only() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    for who in [&admin, &sam] {
        let ev = t
            .send_all(who, json!({"text": "hello"}))
            .await
            .expect("stream");
        assert_eq!(terminal(&ev).0, "done");
    }
    let (_, v) = t.get("/api/ai/status", &sam).await;
    let keys: Vec<&str> = v
        .as_object()
        .expect("obj")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(&keys[..4], &["enabled", "allowed", "suspended", "hasData"]);
    assert_eq!(
        (&v["enabled"], &v["allowed"], &v["suspended"]),
        (&json!(true), &json!(true), &json!(false))
    );
    set_function(&t, &sam, false);
    let (_, v) = t.get("/api/ai/status", &sam).await;
    assert_eq!(
        (&v["enabled"], &v["allowed"], &v["suspended"], &v["hasData"]),
        (&json!(true), &json!(false), &json!(true), &json!(true))
    );
    // an administrator is never suspended, whatever the row says
    set_function(&t, &admin, false);
    let (_, v) = t.get("/api/ai/status", &admin).await;
    assert_eq!(
        (&v["allowed"], &v["suspended"]),
        (&json!(true), &json!(false))
    );
    let ev = t
        .send_all(&admin, json!({"text": "still fine"}))
        .await
        .expect("stream");
    assert_eq!(terminal(&ev).0, "done");
    // the suspended user can still read and delete
    let (code, list) = t.get("/api/ai/chats", &sam).await;
    assert_eq!(code, 200);
    let chat = list["chats"][0]["id"].as_i64().expect("chat");
    let (code, _) = t.get(&format!("/api/ai/chat?id={chat}"), &sam).await;
    assert_eq!(code, 200);
    let (code, v) = t.post("/api/ai/send", &sam, json!({"text": "no"})).await;
    assert_eq!((code, &v["code"]), (403, &json!("ai.suspended")), "{v}");
    // resumed
    set_function(&t, &sam, true);
    let (_, v) = t.get("/api/ai/status", &sam).await;
    assert_eq!(
        (&v["allowed"], &v["suspended"]),
        (&json!(true), &json!(false))
    );
    let (code, v) = t
        .post("/api/ai/chats/delete", &sam, json!({"id": chat}))
        .await;
    assert_eq!(code, 200, "{v}");
}

#[tokio::test]
async fn the_access_recheck_stops_a_running_answer_of_a_suspended_user() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-hold").await;
    let mut s = t
        .open_stream(
            "/api/ai/send",
            &sam,
            json!({"text": "slow one", "lang": "en"}),
        )
        .await
        .expect("stream");
    let (_, start) = s.until("start").await;
    let (_, first) = s.until("delta").await;
    assert_eq!(first["text"], json!("Hold on. "));
    let at = std::time::Instant::now();
    set_function(&t, &sam, false);
    let (name, end) = s.until("error").await;
    assert_eq!(name, "error");
    assert!(
        at.elapsed() < Duration::from_secs(3),
        "stopped by the re-check"
    );
    assert_eq!(
        end,
        json!({"status": "error", "code": "ai.suspended", "error": "管理员已暂停你的 AI+ 功能。"})
    );
    assert!(s.next().await.is_none(), "the stream ends after the error");
    let chat = start["chatId"].as_i64().expect("chat");
    let (_, v) = t.get(&format!("/api/ai/chat?id={chat}"), &sam).await;
    let answer = &v["messages"][1];
    assert_eq!(answer["status"], json!("error"));
    assert_eq!(answer["error"], json!("ai.suspended"));
    assert_eq!(
        answer["content"],
        json!("Hold on. "),
        "the text so far is kept"
    );
    let usage: (String, Option<String>) = t
        .db()
        .query_row(
            "SELECT status, error_code FROM usage WHERE kind = 'chat'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("usage");
    assert_eq!(usage, ("stopped".to_string(), None));
    assert_eq!(t.app.0.registry.chat_count(), 0, "the place is free again");
}

#[tokio::test]
async fn a_running_suggestion_is_refused_when_the_user_is_suspended() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let ev = t
        .send_all(&sam, json!({"text": "Remember that I study at night"}))
        .await
        .expect("stream");
    let msg = ev[0].1["userMessageId"].as_i64().expect("message");
    t.set_model(&admin, "demo-hold").await;
    let suggest = t.post(
        "/api/ai/memories/suggest",
        &sam,
        json!({"messageId": msg, "lang": "en"}),
    );
    let cancel = async {
        // as the suspend route does once the flag is written
        for _ in 0..400 {
            let asked = t
                .app
                .0
                .registry
                .cancel_where(StopReason::Suspended, |u, k, _| {
                    u == sam.id && k == Kind::Suggest
                });
            if !asked.is_empty() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        false
    };
    let ((code, v), asked): ((u16, Value), bool) = tokio::join!(suggest, cancel);
    assert!(asked, "the suggestion was running");
    assert_eq!(code, 403, "{v}");
    assert_eq!(
        v,
        json!({"error": "管理员已暂停你的 AI+ 功能。", "code": "ai.suspended"})
    );
    let status: String = t
        .db()
        .query_row("SELECT status FROM usage WHERE kind = 'memory'", [], |r| {
            r.get(0)
        })
        .expect("usage");
    assert_eq!(status, "stopped");
    t.stub.release();
}
