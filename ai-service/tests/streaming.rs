//! Streaming answers: both wire formats, persistence, Stop, disconnect, re-check, switch-off, deletion, busy, limits,
//! retry, error mapping, time limits, usage rows.
mod common;

use common::*;
use serde_json::{json, Value};
use std::time::Duration;

fn chat_of(ev: &[(String, Value)]) -> i64 {
    ev[0].1["chatId"].as_i64().expect("chatId")
}

#[tokio::test]
async fn openai_stream_is_saved_with_usage() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;

    let mut s = t
        .open_stream(
            "/api/ai/send",
            &sam,
            json!({"chatId": null, "text": "  What is a list?  "}),
        )
        .await
        .expect("stream");
    assert_eq!(
        s.headers.get("content-type").and_then(|v| v.to_str().ok()),
        Some("text/event-stream; charset=utf-8")
    );
    assert_eq!(
        s.headers.get("cache-control").and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(
        s.headers
            .get("x-accel-buffering")
            .and_then(|v| v.to_str().ok()),
        Some("no")
    );
    assert_eq!(
        s.headers
            .get("x-content-type-options")
            .and_then(|v| v.to_str().ok()),
        Some("nosniff")
    );
    assert!(s.headers.get("content-length").is_none());
    let ev = s.all().await;
    let n = names(&ev);
    assert_eq!(n.first().map(String::as_str), Some("start"));
    assert_eq!(n.last().map(String::as_str), Some("done"));
    assert!(n.iter().filter(|x| *x == "delta").count() >= 2);
    let start = &ev[0].1;
    assert_eq!(start["newChat"], json!(true));
    assert_eq!(start["retry"], json!(false));
    assert_eq!(start["chatTitle"], json!("What is a list?"));
    assert_eq!(start["model"], json!("demo-chat-small"));
    // the raw SSE text has the exact frame shape
    assert!(s.raw.starts_with("event: start\ndata: {\"chatId\":"));
    assert_eq!(text(&ev), ANSWER);
    let (_, done) = terminal(&ev);
    assert_eq!(
        done,
        json!({"status": "done", "tokensIn": 21, "tokensOut": 13, "truncated": false})
    );
    // the upstream request: system prompt first, the trimmed message, stream_options
    let req = t.stub.last();
    assert_eq!(req.path, "/v1/chat/completions");
    assert_eq!(
        req.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    assert_eq!(req.header("accept"), Some("text/event-stream"));
    assert_eq!(req.header("user-agent"), Some("xexam-ai-service/0.8"));
    assert_eq!(req.body["messages"][0]["role"], json!("system"));
    assert!(req.body["messages"][0]["content"]
        .as_str()
        .unwrap_or("")
        .starts_with("You are the study assistant inside Xexam"));
    assert_eq!(
        req.body["messages"][1],
        json!({"role": "user", "content": "What is a list?"})
    );
    assert_eq!(req.body["stream_options"], json!({"include_usage": true}));
    // the answer length of the limits (3000 tokens) is the max_tokens
    assert_eq!(req.body["max_tokens"], json!(3000));
    assert!(req.body.get("temperature").is_none());

    let chat = chat_of(&ev);
    let (st, v) = t.get(&format!("/api/ai/chat?id={chat}"), &sam).await;
    assert_eq!(st, 200, "{v}");
    let msgs = v["messages"].as_array().expect("messages");
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["role"], json!("user"));
    assert_eq!(msgs[1]["status"], json!("done"));
    assert_eq!(msgs[1]["content"], json!(ANSWER));
    assert_eq!(msgs[1]["model"], json!("demo-chat-small"));
    assert_eq!(v["chat"]["running"], json!(false));
    assert_eq!(v["chat"]["messageCount"], json!(2));
    let (_, list) = t.get("/api/ai/chats", &sam).await;
    assert_eq!(list["chats"].as_array().map(Vec::len), Some(1));

    // a second message in the same chat carries the history
    let ev2 = t
        .send_all(&sam, json!({"chatId": chat, "text": "And a second one?"}))
        .await
        .expect("stream");
    assert_eq!(terminal(&ev2).0, "done");
    assert_eq!(ev2[0].1["newChat"], json!(false));
    let req = t.stub.last();
    let roles: Vec<&str> = req.body["messages"]
        .as_array()
        .expect("msgs")
        .iter()
        .map(|m| m["role"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(roles, vec!["system", "user", "assistant", "user"]);

    // one usage row per call: 1 verify + 2 chats
    let chats: i64 = t
        .db()
        .query_row(
            "SELECT COUNT(*) FROM usage WHERE kind = 'chat' AND status = 'ok'",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(chats, 2);
}

#[tokio::test]
async fn anthropic_stream_with_ping_and_usage() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    let (s, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"provider": "anthropic", "baseUrl": format!("{}/anthropic/v1", t.stub.url), "apiKey": KEY, "model": "demo-claude"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        v["baseUrl"],
        json!(format!("{}/anthropic", t.stub.url)),
        "the trailing /v1 is removed"
    );
    t.verify_and_enable(&admin).await;
    let ev = t
        .send_all(&sam, json!({"text": "Explain sets"}))
        .await
        .expect("stream");
    assert_eq!(text(&ev), ANSWER);
    assert_eq!(
        terminal(&ev).1,
        json!({"status": "done", "tokensIn": 32, "tokensOut": 17, "truncated": false})
    );
    let req = t.stub.last();
    assert_eq!(req.path, "/anthropic/v1/messages");
    assert_eq!(req.header("x-api-key"), Some(KEY));
    assert_eq!(req.header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(req.body["max_tokens"], json!(3000));
    assert!(req.body["system"][0]["text"]
        .as_str()
        .unwrap_or("")
        .contains("Xexam"));
    assert_eq!(
        req.body["system"][0]["cache_control"],
        json!({"type": "ephemeral"})
    );
    // no history: the only message is the new question, without a breakpoint
    assert_eq!(
        req.body["messages"],
        json!([{"role": "user", "content": "Explain sets"}])
    );
    assert!(req.body.get("temperature").is_none());
}

#[tokio::test]
async fn an_anthropic_stream_cut_before_message_stop_is_an_error_and_can_be_retried() {
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
    // message_start and one text delta, then the body ends without message_stop
    t.stub.force(Some((
        200,
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5}}}\n\n\
         event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Half an ans\"}}\n\n",
    )));
    let ev = t
        .send_all(&sam, json!({"text": "Explain sets"}))
        .await
        .expect("stream");
    assert_eq!(text(&ev), "Half an ans");
    assert_eq!(terminal(&ev).0, "error", "{ev:?}");
    assert_eq!(terminal(&ev).1["code"], json!("ai.up.protocol"));
    let chat = chat_of(&ev);
    let (_, v) = t.get(&format!("/api/ai/chat?id={chat}"), &sam).await;
    assert_eq!(v["messages"][1]["status"], json!("error"));
    assert_eq!(v["messages"][1]["content"], json!("Half an ans"));
    let usage: (String, String) = t
        .db()
        .query_row(
            "SELECT status, error_code FROM usage WHERE kind = 'chat'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("usage");
    assert_eq!(usage, ("error".to_string(), "protocol".to_string()));
    // the cut answer can be retried
    t.stub.force(None);
    let ev = t
        .open_stream("/api/ai/retry", &sam, json!({"chatId": chat}))
        .await
        .expect("retry")
        .all()
        .await;
    assert_eq!(ev[0].1["retry"], json!(true));
    assert_eq!(terminal(&ev).0, "done");
    assert_eq!(text(&ev), ANSWER);
}

#[tokio::test]
async fn crlf_and_split_utf8_from_the_provider() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-crlf").await;
    let ev = t
        .send_all(&sam, json!({"text": "hi"}))
        .await
        .expect("stream");
    assert_eq!(text(&ev), "Grüße");
    assert_eq!(terminal(&ev).0, "done");
}

#[tokio::test]
async fn stop_needs_no_id_and_keeps_the_text() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-hold").await;
    let mut s = t
        .open_stream("/api/ai/send", &sam, json!({"text": "wait"}))
        .await
        .expect("stream");
    let (_, start) = s.until("start").await;
    s.until("delta").await;
    // while it runs, the status names it and the chat is marked running
    let (_, st) = t.get("/api/ai/status", &sam).await;
    assert_eq!(st["running"]["chatId"], start["chatId"]);
    assert_eq!(st["running"]["messageId"], start["assistantMessageId"]);
    let (code, v) = t.post("/api/ai/stop", &sam, json!({})).await;
    assert_eq!((code, v), (200, json!({"ok": true, "stopped": true})));
    let (name, data) = s.until("stopped").await;
    assert_eq!(
        (name.as_str(), data),
        ("stopped", json!({"status": "stopped"}))
    );
    assert!(
        s.next().await.is_none(),
        "the stream ends after the terminal event"
    );
    let chat = start["chatId"].as_i64().expect("id");
    let (_, v) = t.get(&format!("/api/ai/chat?id={chat}"), &sam).await;
    assert_eq!(v["messages"][1]["status"], json!("stopped"));
    assert_eq!(v["messages"][1]["content"], json!("Hold on. "));
    let (_, v) = t.post("/api/ai/stop", &sam, json!({})).await;
    assert_eq!(v["stopped"], json!(false));
    let status: String = t
        .db()
        .query_row("SELECT status FROM usage WHERE kind = 'chat'", [], |r| {
            r.get(0)
        })
        .expect("usage");
    assert_eq!(status, "stopped");
    t.stub.release();
}

#[tokio::test]
async fn a_closed_browser_does_not_stop_the_answer() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-hold").await;
    let mut s = t
        .open_stream("/api/ai/send", &sam, json!({"text": "keep going"}))
        .await
        .expect("stream");
    let (_, start) = s.until("start").await;
    s.until("delta").await;
    drop(s);
    let chat = start["chatId"].as_i64().expect("id");
    // the flushed text is visible while it runs
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (_, v) = t.get(&format!("/api/ai/chat?id={chat}"), &sam).await;
    assert_eq!(v["chat"]["running"], json!(true));
    assert_eq!(v["messages"][1]["status"], json!("streaming"));
    assert_eq!(v["messages"][1]["content"], json!("Hold on. "));
    t.stub.release();
    let mut done = false;
    for _ in 0..100 {
        let (_, v) = t.get(&format!("/api/ai/chat?id={chat}"), &sam).await;
        if v["chat"]["running"] == json!(false) {
            assert_eq!(v["messages"][1]["status"], json!("done"));
            assert_eq!(v["messages"][1]["content"], json!("Hold on. Done now."));
            done = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(done, "the generation finished in the background");
}

#[tokio::test]
async fn revoking_ai_plus_stops_the_answer_within_one_tick() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-hold").await;
    let mut s = t
        .open_stream("/api/ai/send", &sam, json!({"text": "slow"}))
        .await
        .expect("stream");
    s.until("delta").await;
    t.set_ai(&sam, false);
    let started = std::time::Instant::now();
    let (_, e) = s.until("error").await;
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(e["code"], json!("ai.notAllowed"));
    assert_eq!(e["error"], json!("你的账号没有开通 AI+ 功能。"));
    let (_, msg): (String, String) = t
        .db()
        .query_row(
            "SELECT status, error_code FROM messages WHERE role = 'assistant'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row");
    assert_eq!(msg, "ai.notAllowed");
    let usage: String = t
        .db()
        .query_row("SELECT status FROM usage WHERE kind = 'chat'", [], |r| {
            r.get(0)
        })
        .expect("usage");
    assert_eq!(usage, "stopped");
    // the chats stay readable, sending is refused (the identity now says no AI+)
    let sam_off = Who {
        ai: false,
        ..sam.clone()
    };
    let (code, v) = t
        .post("/api/ai/send", &sam_off, json!({"text": "again"}))
        .await;
    assert_eq!((code, v["code"].clone()), (403, json!("ai.notAllowed")));
    let (code, v) = t.get("/api/ai/chats", &sam_off).await;
    assert_eq!(code, 200);
    assert_eq!(v["chats"].as_array().map(Vec::len), Some(1));
    t.stub.release();
}

#[tokio::test]
async fn three_failed_rechecks_end_with_backend_down() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-hold").await;
    let mut s = t
        .open_stream("/api/ai/send", &sam, json!({"text": "slow"}))
        .await
        .expect("stream");
    s.until("delta").await;
    t.backend.set_mode(Mode::Status500);
    let (_, e) = s.until("error").await;
    assert_eq!(e["code"], json!("ai.backendDown"));
    t.backend.set_mode(Mode::Normal);
    t.stub.release();
}

#[tokio::test]
async fn switching_ai_off_stops_every_answer() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    let kim = t.user("kim", false, true);
    t.configure_openai(&admin, "demo-hold").await;
    let mut a = t
        .open_stream("/api/ai/send", &sam, json!({"text": "one"}))
        .await
        .expect("a");
    let mut b = t
        .open_stream("/api/ai/send", &kim, json!({"text": "two"}))
        .await
        .expect("b");
    a.until("delta").await;
    b.until("delta").await;
    let (code, v) = t
        .post("/api/ai/admin/enable", &admin, json!({"enabled": false}))
        .await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["enabled"], json!(false));
    assert_eq!(a.until("error").await.1["code"], json!("ai.disabled"));
    assert_eq!(b.until("error").await.1["code"], json!("ai.disabled"));
    assert!(captured_contains(
        "AI was switched off: 2 running answers were stopped"
    ));
    assert!(captured_contains("user \"admin\" disabled AI"));
    let (code, v) = t.post("/api/ai/send", &sam, json!({"text": "three"})).await;
    assert_eq!((code, v["code"].clone()), (403, json!("ai.disabled")));
    t.stub.release();
}

#[tokio::test]
async fn deleting_a_chat_during_an_answer_waits_for_it() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-hold").await;
    let mut s = t
        .open_stream("/api/ai/send", &sam, json!({"text": "x"}))
        .await
        .expect("s");
    let (_, start) = s.until("start").await;
    s.until("delta").await;
    let chat = start["chatId"].as_i64().expect("id");
    let (code, v) = t
        .post("/api/ai/chats/delete", &sam, json!({"id": chat}))
        .await;
    assert_eq!((code, v), (200, json!({"deleted": 1})));
    let (_, e) = s.until("error").await;
    assert_eq!(e["code"], json!("ai.notFound"));
    let db = t.db();
    let msgs: i64 = db
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .expect("n");
    let chats: i64 = db
        .query_row("SELECT COUNT(*) FROM chats", [], |r| r.get(0))
        .expect("n");
    assert_eq!((msgs, chats), (0, 0), "no row written for a deleted chat");
    let usage: i64 = db
        .query_row("SELECT COUNT(*) FROM usage WHERE kind = 'chat'", [], |r| {
            r.get(0)
        })
        .expect("n");
    assert_eq!(usage, 1);
    let (code, _) = t
        .post("/api/ai/chats/delete", &sam, json!({"id": chat}))
        .await;
    assert_eq!(code, 404);
    t.stub.release();
}

#[tokio::test]
async fn busy_server_busy_and_a_resized_gate() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    let kim = t.user("kim", false, true);
    t.configure_openai(&admin, "demo-hold").await;
    let (code, _) = t.post("/api/ai/admin/models", &admin, json!({})).await;
    assert_eq!(code, 200);
    let mut a = t
        .open_stream("/api/ai/send", &sam, json!({"text": "one"}))
        .await
        .expect("a");
    let (_, start) = a.until("start").await;
    a.until("delta").await;
    // one running call per user (chat, retry, memory suggest)
    let (code, v) = t.post("/api/ai/send", &sam, json!({"text": "two"})).await;
    assert_eq!((code, v["code"].clone()), (409, json!("ai.busy")));
    let msg = start["userMessageId"].as_i64().expect("id");
    let (code, v) = t
        .post("/api/ai/memories/suggest", &sam, json!({"messageId": msg}))
        .await;
    assert_eq!((code, v["code"].clone()), (409, json!("ai.busy")));
    // the instance-wide gate, lowered to one place while a call runs
    let (code, _) = t
        .post("/api/ai/admin/limits", &admin, json!({"globalRunning": 1}))
        .await;
    assert_eq!(code, 200);
    let (code, v) = t.post("/api/ai/send", &kim, json!({"text": "three"})).await;
    assert_eq!((code, v["code"].clone()), (503, json!("ai.serverBusy")));
    let (code, v) = t.post("/api/ai/admin/models", &admin, json!({})).await;
    assert_eq!(code, 200, "a cached list needs no place: {v}");
    // raised again: kim gets a place at once, nothing running was cancelled
    let (code, _) = t
        .post("/api/ai/admin/limits", &admin, json!({"globalRunning": 2}))
        .await;
    assert_eq!(code, 200);
    let mut b = t
        .open_stream("/api/ai/send", &kim, json!({"text": "four"}))
        .await
        .expect("b");
    b.until("delta").await;
    // two per user allowed now: sam may start a second chat (but never two in the same chat)
    let (code, _) = t
        .post(
            "/api/ai/admin/limits",
            &admin,
            json!({"perUserRunning": 2, "globalRunning": 3}),
        )
        .await;
    assert_eq!(code, 200);
    let chat = start["chatId"].as_i64().expect("chat");
    let (code, v) = t
        .post(
            "/api/ai/send",
            &sam,
            json!({"chatId": chat, "text": "same chat"}),
        )
        .await;
    assert_eq!((code, v["code"].clone()), (409, json!("ai.busy")));
    let mut c = t
        .open_stream("/api/ai/send", &sam, json!({"text": "other chat"}))
        .await
        .expect("c");
    c.until("delta").await;
    t.stub.release();
    assert_eq!(a.until("done").await.0, "done");
    assert_eq!(b.until("done").await.0, "done");
    assert_eq!(c.until("done").await.0, "done");
}

#[tokio::test]
async fn sixteen_calls_at_once_then_server_busy() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-hold").await;
    let mut streams = Vec::new();
    for i in 0..16 {
        let u = t.user(&format!("u{i}"), false, true);
        let mut s = t
            .open_stream("/api/ai/send", &u, json!({"text": "hi"}))
            .await
            .expect("stream");
        s.until("delta").await;
        streams.push(s);
    }
    let late = t.user("late", false, true);
    let (code, v) = t.post("/api/ai/send", &late, json!({"text": "hi"})).await;
    assert_eq!((code, v["code"].clone()), (503, json!("ai.serverBusy")));
    // nothing was saved for the refused call
    let chats: i64 = t
        .db()
        .query_row(
            "SELECT COUNT(*) FROM chats WHERE user_id = ?1",
            [late.id],
            |r| r.get(0),
        )
        .expect("n");
    assert_eq!(chats, 0);
    t.stub.release();
    for mut s in streams {
        assert_eq!(s.until("done").await.0, "done");
    }
}

#[tokio::test]
async fn a_held_answer_with_pings_outlives_the_idle_limit() {
    // The hold pattern of the tests above relies on this: SSE comments count against the idle limit (4.7), so held
    // calls stay open until they are released, however slow the runner is
    let mut timeouts = fast_timeouts();
    timeouts.idle = Duration::from_secs(2);
    let t = start_with(Opts {
        timeouts,
        ..Opts::default()
    })
    .await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-hold").await;
    let mut s = t
        .open_stream("/api/ai/send", &sam, json!({"text": "hi"}))
        .await
        .expect("stream");
    s.until("delta").await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    t.stub.release();
    let ev = s.all().await;
    assert_eq!(terminal(&ev).0, "done", "{ev:?}");
    assert_eq!(text(&ev), "Done now.");
}

#[tokio::test]
async fn chat_and_message_limits_refuse_new_items_only() {
    let t = start().await;
    t.relax_caps();
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let (code, _) = t
        .post(
            "/api/ai/admin/limits",
            &admin,
            json!({"chatsPerUser": 10, "messagesPerChat": 20}),
        )
        .await;
    assert_eq!(code, 200);
    let mut first = 0;
    for i in 0..10 {
        let ev = t
            .send_all(&sam, json!({"text": format!("q{i}")}))
            .await
            .expect("ok");
        if i == 0 {
            first = chat_of(&ev);
        }
    }
    let (code, v) = t.post("/api/ai/send", &sam, json!({"text": "q11"})).await;
    assert_eq!(code, 409);
    assert_eq!(v["code"], json!("ai.chatLimit"));
    assert_eq!(v["limit"], json!(10));
    assert_eq!(
        v["error"],
        json!("对话太多了（最多 10 个），请先删除一些旧对话。")
    );
    // a full chat: 20 messages = 10 exchanges
    for i in 1..10 {
        t.send_all(&sam, json!({"chatId": first, "text": format!("more {i}")}))
            .await
            .expect("ok");
    }
    let (code, v) = t
        .post(
            "/api/ai/send",
            &sam,
            json!({"chatId": first, "text": "one too many"}),
        )
        .await;
    assert_eq!(
        (code, v["code"].clone(), v["limit"].clone()),
        (409, json!("ai.messageLimit"), json!(20))
    );
    // old data stays; deleting one chat makes room again
    let (_, list) = t.get("/api/ai/chats", &sam).await;
    assert_eq!(list["chats"].as_array().map(Vec::len), Some(10));
    let (code, _) = t
        .post("/api/ai/chats/delete", &sam, json!({"id": first}))
        .await;
    assert_eq!(code, 200);
    assert!(t
        .send_all(&sam, json!({"text": "fits again"}))
        .await
        .is_ok());
    // message length: a fixed internal cap of 16000 characters (not an administrator's setting)
    let (code, v) = t
        .post("/api/ai/send", &sam, json!({"text": "z".repeat(16001)}))
        .await;
    assert_eq!(
        (code, v["code"].clone(), v["limit"].clone()),
        (400, json!("ai.tooLong"), json!(16000))
    );
    let (code, v) = t
        .post(
            "/api/ai/admin/limits",
            &admin,
            json!({"messageChars": 1000}),
        )
        .await;
    assert_eq!((code, v["code"].clone()), (400, json!("ai.badRequest")));
    let (_, st) = t.get("/api/ai/status", &sam).await;
    assert_eq!(st["messageChars"], json!(16000));
    assert_eq!(st["maxMessageChars"], json!(16000));
    let (code, v) = t.post("/api/ai/send", &sam, json!({"text": "   "})).await;
    assert_eq!((code, v["code"].clone()), (400, json!("ai.badRequest")));
}

#[tokio::test]
async fn retry_replaces_a_failed_answer() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    t.stub.force(Some((
        500,
        r#"{"error":{"type":"api_error","message":"internal detail"}}"#,
    )));
    let ev = t
        .send_all(&sam, json!({"text": "first try"}))
        .await
        .expect("stream");
    let (name, e) = terminal(&ev);
    assert_eq!(name, "error");
    assert_eq!(
        e,
        json!({"status": "error", "code": "ai.up.server", "error": "AI 服务商内部出错了。"})
    );
    let chat = chat_of(&ev);
    let answer = ev[0].1["assistantMessageId"].clone();
    t.stub.force(None);
    let mut s = t
        .open_stream("/api/ai/retry", &sam, json!({"chatId": chat}))
        .await
        .expect("retry");
    let ev = s.all().await;
    assert_eq!(ev[0].1["retry"], json!(true));
    assert_eq!(ev[0].1["assistantMessageId"], answer);
    assert_eq!(terminal(&ev).0, "done");
    let (_, v) = t.get(&format!("/api/ai/chat?id={chat}"), &sam).await;
    assert_eq!(
        v["messages"].as_array().map(Vec::len),
        Some(2),
        "no new user message"
    );
    assert_eq!(v["messages"][1]["status"], json!("done"));
    assert_eq!(v["messages"][1]["error"], Value::Null);
    let (code, v) = t.post("/api/ai/retry", &sam, json!({"chatId": chat})).await;
    assert_eq!((code, v["code"].clone()), (409, json!("ai.notRetryable")));
    let other = t.user("kim", false, true);
    let (code, v) = t
        .post("/api/ai/retry", &other, json!({"chatId": chat}))
        .await;
    assert_eq!((code, v["code"].clone()), (404, json!("ai.notFound")));
    // the upstream body was never echoed
    assert!(!captured_contains("internal detail"));
}

#[tokio::test]
async fn upstream_errors_are_mapped_and_never_echoed() {
    let t = start().await;
    t.relax_caps();
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let cases: &[(u16, &str, &str)] = &[
        (
            401,
            r#"{"error":{"code":"invalid_api_key","message":"secret-ish"}}"#,
            "ai.up.auth",
        ),
        (403, "nope", "ai.up.auth"),
        (402, "{}", "ai.up.quota"),
        (
            429,
            r#"{"error":{"code":"insufficient_quota"}}"#,
            "ai.up.quota",
        ),
        (
            404,
            r#"{"error":{"code":"model_not_found"}}"#,
            "ai.up.notFound",
        ),
        (429, "slow down", "ai.up.rateLimit"),
        (529, "{}", "ai.up.overloaded"),
        (503, "{}", "ai.up.overloaded"),
        (500, "{}", "ai.up.server"),
        (422, "{}", "ai.up.badRequest"),
        (
            400,
            r#"{"type":"error","error":{"type":"overloaded_error"}}"#,
            "ai.up.overloaded",
        ),
        (302, "", "ai.up.protocol"),
    ];
    for (status, body, code) in cases {
        t.stub.force(Some((*status, body)));
        let ev = t
            .send_all(&sam, json!({"text": "x"}))
            .await
            .expect("stream");
        let (name, e) = terminal(&ev);
        assert_eq!(name, "error", "{status}");
        assert_eq!(e["code"], json!(code), "{status} {body}");
    }
    t.stub.force(None);
    assert!(captured_contains(
        "AI call failed for user \"sam\": ai.up.auth (HTTP 401)"
    ));
    assert!(!captured_contains("secret-ish"));
    let n: i64 = t
        .db()
        .query_row(
            "SELECT COUNT(*) FROM usage WHERE kind = 'chat' AND status = 'error'",
            [],
            |r| r.get(0),
        )
        .expect("n");
    assert_eq!(n, cases.len() as i64, "exactly one usage row per call");
    let rl: String = t
        .db()
        .query_row(
            "SELECT error_code FROM usage WHERE error_code = 'rateLimit' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("short code");
    assert_eq!(rl, "rateLimit");
}

#[tokio::test]
async fn in_band_errors_length_refusal_and_too_long() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-error").await;
    let ev = t.send_all(&sam, json!({"text": "x"})).await.expect("s");
    assert_eq!(terminal(&ev).1["code"], json!("ai.up.server"));
    assert_eq!(text(&ev), "Part", "the text that arrived is kept");

    t.set_model(&admin, "demo-length").await;
    let ev = t.send_all(&sam, json!({"text": "x"})).await.expect("s");
    assert_eq!(terminal(&ev).1["truncated"], json!(true));
    let trunc: i64 = t
        .db()
        .query_row(
            "SELECT truncated FROM messages WHERE id = ?1",
            [ev[0].1["assistantMessageId"].as_i64().unwrap_or(0)],
            |r| r.get(0),
        )
        .expect("row");
    assert_eq!(trunc, 1);

    t.set_model(&admin, "demo-big").await;
    let ev = t.send_all(&sam, json!({"text": "x"})).await.expect("s");
    assert_eq!(terminal(&ev).1["code"], json!("ai.up.tooLong"));
    let len: i64 = t
        .db()
        .query_row(
            "SELECT length(content) FROM messages WHERE id = ?1",
            [ev[0].1["assistantMessageId"].as_i64().unwrap_or(0)],
            |r| r.get(0),
        )
        .expect("row");
    assert!((999_000..=1_000_000).contains(&len), "{len}");
}

#[tokio::test]
async fn refusal_and_empty_streams() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    // verify of a model that refuses everything fails (config class), so check the chat path with a forced body
    t.configure_openai(&admin, "demo-chat-small").await;
    t.stub.force(Some((
        200,
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"content_filter\"}]}\n\ndata: [DONE]\n\n",
    )));
    let ev = t.send_all(&sam, json!({"text": "x"})).await.expect("s");
    assert_eq!(terminal(&ev).1["code"], json!("ai.up.refused"));
    t.stub.force(Some((200, "")));
    let ev = t.send_all(&sam, json!({"text": "x"})).await.expect("s");
    assert_eq!(terminal(&ev).1["code"], json!("ai.up.protocol"));
    t.stub.force(None);
}

#[tokio::test]
async fn numeric_error_codes_inside_a_200_stream() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    // OpenRouter sends `error.code` as the HTTP status number after a 200
    for (n, code) in [
        (429, "ai.up.rateLimit"),
        (402, "ai.up.quota"),
        (401, "ai.up.auth"),
    ] {
        let body = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"Part\"}}}}]}}\n\ndata: {{\"error\":{{\"code\":{n},\"message\":\"m\"}}}}\n\n"
        );
        t.stub.force(Some((200, &body)));
        let ev = t.send_all(&sam, json!({"text": "x"})).await.expect("s");
        assert_eq!(terminal(&ev).0, "error", "{n}");
        assert_eq!(terminal(&ev).1["code"], json!(code), "{n}");
        assert_eq!(text(&ev), "Part", "the text that arrived is kept");
    }
    // the test call treats such an auth problem as a configuration error: AI off, no longer verified
    let (s, v) = t.post("/api/ai/admin/verify", &admin, json!({})).await;
    assert_eq!(s, 200);
    assert_eq!(v["ok"], json!(false), "{v}");
    assert_eq!(v["code"], json!("ai.up.auth"));
    assert_eq!(v["switchedOff"], json!(true));
    assert_eq!(v["verifiedAt"], Value::Null);
    t.stub.force(None);
}

#[tokio::test]
async fn time_limits() {
    let mut timeouts = fast_timeouts();
    timeouts.first_byte = Duration::from_millis(300);
    timeouts.idle = Duration::from_millis(400);
    let t = start_with(Opts {
        timeouts,
        ..Opts::default()
    })
    .await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    t.stub.st().head_delay_ms = 1500;
    let ev = t.send_all(&sam, json!({"text": "x"})).await.expect("s");
    assert_eq!(terminal(&ev).1["code"], json!("ai.up.timeout"));
    t.stub.st().head_delay_ms = 0;
    // idle limit: the provider sends headers and a comment, then nothing
    let (code, _) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"model": "demo-silent"}),
        )
        .await;
    assert_eq!(code, 200);
    // verify would time out too; enable directly through the store for this check
    let fp = t.app.0.snapshot().expect("snap").fp;
    t.db()
        .execute(
            "UPDATE settings SET enabled = 1, verified_at = 1, verified_fp = ?1",
            [fp],
        )
        .expect("update");
    let started = std::time::Instant::now();
    let ev = t.send_all(&sam, json!({"text": "x"})).await.expect("s");
    assert_eq!(terminal(&ev).1["code"], json!("ai.up.timeout"));
    assert!(started.elapsed() < Duration::from_secs(5));

    // total limit
    let mut timeouts = fast_timeouts();
    timeouts.total = Duration::from_millis(500);
    let t2 = start_with(Opts {
        timeouts,
        ..Opts::default()
    })
    .await;
    let admin = t2.user("admin", true, false);
    let sam = t2.user("sam", false, true);
    t2.configure_openai(&admin, "demo-hold").await;
    let ev = t2.send_all(&sam, json!({"text": "x"})).await.expect("s");
    assert_eq!(terminal(&ev).1["code"], json!("ai.up.timeout"));
    assert_eq!(text(&ev), "Hold on. ");
    t2.stub.release();
}

#[tokio::test]
async fn shutdown_ends_answers_as_stopped_and_saves_them() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-hold").await;
    let mut s = t
        .open_stream("/api/ai/send", &sam, json!({"text": "x"}))
        .await
        .expect("s");
    let (_, start) = s.until("start").await;
    s.until("delta").await;
    assert_eq!(t.app.0.begin_shutdown(), 1);
    assert_eq!(s.until("stopped").await.1, json!({"status": "stopped"}));
    let id = start["assistantMessageId"].as_i64().expect("id");
    let (status, content): (String, String) = t
        .db()
        .query_row(
            "SELECT status, content FROM messages WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row");
    assert_eq!(
        (status.as_str(), content.as_str()),
        ("stopped", "Hold on. ")
    );
    t.stub.release();
}

#[tokio::test]
async fn memories_and_the_model_suggestion() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    // memories before any chat (the user row is created on the fly)
    let (code, v) = t
        .post(
            "/api/ai/memories/add",
            &sam,
            json!({"content": "  Prefers\nshort answers  "}),
        )
        .await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["memory"]["content"], json!("Prefers short answers"));
    assert_eq!(v["memory"]["source"], json!("manual"));
    let id = v["memory"]["id"].as_i64().expect("id");
    let (code, v) = t
        .post(
            "/api/ai/memories/update",
            &sam,
            json!({"id": id, "content": "Prefers lists"}),
        )
        .await;
    assert_eq!(
        (code, v["memory"]["content"].clone()),
        (200, json!("Prefers lists"))
    );
    let (code, v) = t
        .post(
            "/api/ai/memories/add",
            &sam,
            json!({"content": "x".repeat(4001)}),
        )
        .await;
    assert_eq!(
        (code, v["code"].clone(), v["limit"].clone()),
        (400, json!("ai.memoryTooLong"), json!(1000))
    );
    let (code, v) = t
        .post(
            "/api/ai/memories/add",
            &sam,
            json!({"content": "Uses metric units", "source": "chat"}),
        )
        .await;
    assert_eq!((code, v["memory"]["source"].clone()), (200, json!("chat")));
    let (_, v) = t.get("/api/ai/memories", &sam).await;
    assert_eq!(v["limit"], json!(50));
    assert_eq!(v["maxTokens"], json!(1000));
    assert_eq!(v["memories"].as_array().map(Vec::len), Some(2));
    // the memories go into the system prompt, newest first
    let ev = t
        .send_all(
            &sam,
            json!({"text": "Please remember that I like examples"}),
        )
        .await
        .expect("s");
    let sys = t.stub.last().body["messages"][0]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert!(sys.contains("The user asked you to remember these notes"));
    let a = sys.find("- Uses metric units").expect("newest");
    let b = sys.find("- Prefers lists").expect("older");
    assert!(a < b);
    // suggestion: the model proposes, nothing is saved
    let msg = ev[0].1["userMessageId"].as_i64().expect("id");
    let (code, v) = t
        .post("/api/ai/memories/suggest", &sam, json!({"messageId": msg}))
        .await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(
        v["suggestion"],
        json!(xexam_ai_service::util::collapse_ws(ANSWER))
    );
    let req = t.stub.last();
    assert!(req.body["messages"][0]["content"]
        .as_str()
        .unwrap_or("")
        .starts_with("Rewrite the user's request"));
    let (_, v) = t.get("/api/ai/memories", &sam).await;
    assert_eq!(v["memories"].as_array().map(Vec::len), Some(2));
    let kinds: i64 = t
        .db()
        .query_row(
            "SELECT COUNT(*) FROM usage WHERE kind = 'memory'",
            [],
            |r| r.get(0),
        )
        .expect("n");
    assert_eq!(kinds, 1);
    // an assistant message, or another user's message, is not found
    let answer = ev[0].1["assistantMessageId"].as_i64().expect("id");
    let (code, _) = t
        .post(
            "/api/ai/memories/suggest",
            &sam,
            json!({"messageId": answer}),
        )
        .await;
    assert_eq!(code, 404);
    let kim = t.user("kim", false, true);
    let (code, _) = t
        .post("/api/ai/memories/suggest", &kim, json!({"messageId": msg}))
        .await;
    assert_eq!(code, 404);
    // the memory limit is adjustable; lowering it keeps the existing notes
    let (code, _) = t
        .post(
            "/api/ai/admin/limits",
            &admin,
            json!({"memoriesPerUser": 2, "memoryTokens": 50}),
        )
        .await;
    assert_eq!(code, 200);
    let (code, v) = t
        .post("/api/ai/memories/add", &sam, json!({"content": "third"}))
        .await;
    assert_eq!(
        (code, v["code"].clone(), v["limit"].clone()),
        (409, json!("ai.memoryLimit"), json!(2))
    );
    let (code, v) = t
        .post(
            "/api/ai/memories/update",
            &sam,
            json!({"id": id, "content": "y".repeat(201)}),
        )
        .await;
    assert_eq!((code, v["limit"].clone()), (400, json!(50)));
    let (_, st) = t.get("/api/ai/status", &sam).await;
    assert_eq!(
        (st["memoryTokens"].clone(), st["memoryLimit"].clone()),
        (json!(50), json!(2))
    );
    let (code, v) = t
        .post("/api/ai/memories/delete", &sam, json!({"id": id}))
        .await;
    assert_eq!((code, v), (200, json!({"deleted": 1})));
    let (code, v) = t
        .post("/api/ai/memories/delete", &sam, json!({"all": true}))
        .await;
    assert_eq!((code, v), (200, json!({"deleted": 1})));
}

#[tokio::test]
async fn keep_alive_comments_while_the_provider_is_silent() {
    let mut timeouts = fast_timeouts();
    timeouts.keep_alive = Duration::from_millis(150);
    let t = start_with(Opts {
        timeouts,
        ..Opts::default()
    })
    .await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-hold").await;
    let mut s = t
        .open_stream("/api/ai/send", &sam, json!({"text": "x"}))
        .await
        .expect("s");
    s.until("delta").await;
    // release later, while the body is being read (a real connection polls it all the time)
    let hold = t.stub.hold.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        hold.notify_waiters();
    });
    s.until("done").await;
    assert!(s.raw.contains(": keep-alive\n\n"), "{}", s.raw);
}

#[tokio::test]
async fn the_default_chat_bound_is_one_thousand() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    t.send_all(&sam, json!({"text": "first"})).await.expect("s");
    let mut db = t.db();
    let tx = db.transaction().expect("tx");
    for i in 0..999 {
        tx.execute(
            "INSERT INTO chats (user_id, title, created_at, updated_at) VALUES (?1, ?2, 1, 1)",
            rusqlite::params![sam.id, format!("c{i}")],
        )
        .expect("insert");
    }
    tx.commit().expect("commit");
    let (code, v) = t
        .post("/api/ai/send", &sam, json!({"text": "one more"}))
        .await;
    assert_eq!(code, 409);
    assert_eq!(
        (v["code"].clone(), v["limit"].clone()),
        (json!("ai.chatLimit"), json!(1000))
    );
    // an existing chat still takes messages
    let (_, list) = t.get("/api/ai/chats", &sam).await;
    let newest = list["chats"][0]["id"].as_i64().expect("id");
    assert!(t
        .send_all(&sam, json!({"chatId": newest, "text": "fine"}))
        .await
        .is_ok());
}
