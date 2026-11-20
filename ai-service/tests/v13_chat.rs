//! v1.3 AI Chat: the chat is a tree, regenerate and edit add versions, switching versions costs nothing, an administrator
//! picks the model per chat, and the status carries the new keys. Everything is made up.
mod common;

use common::*;
use serde_json::{json, Value};
use xexam_ai_service::features::Feature;
use xexam_ai_service::store::{conn, data};

/// Admin + AI+ user + one working connection.
async fn setup() -> (TestApp, Who, Who) {
    let t = start().await;
    t.relax_caps();
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    (t, admin, sam)
}

async fn ask(t: &TestApp, who: &Who, path: &str, body: Value) -> Vec<(String, Value)> {
    t.open_stream(path, who, body)
        .await
        .unwrap_or_else(|e| panic!("{path}: {e:?}"))
        .all()
        .await
}

/// (chat id, user message id, answer id, events)
async fn send(
    t: &TestApp,
    who: &Who,
    chat: Option<i64>,
    text: &str,
) -> (i64, i64, i64, Vec<(String, Value)>) {
    let ev = ask(
        t,
        who,
        "/api/ai/send",
        json!({"chatId": chat, "text": text, "lang": "en"}),
    )
    .await;
    let s = &ev[0].1;
    (
        s["chatId"].as_i64().expect("chat"),
        s["userMessageId"].as_i64().expect("user"),
        s["assistantMessageId"].as_i64().expect("answer"),
        ev,
    )
}

async fn read(t: &TestApp, who: &Who, chat: i64) -> Value {
    let (code, v) = t.get(&format!("/api/ai/chat?id={chat}"), who).await;
    assert_eq!(code, 200, "{v}");
    v
}

fn ids(v: &Value) -> Vec<i64> {
    v["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .map(|m| m["id"].as_i64().expect("id"))
        .collect()
}

fn stored(t: &TestApp, chat: i64) -> i64 {
    t.db()
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE chat_id = ?1",
            [chat],
            |r| r.get(0),
        )
        .expect("count")
}

fn usage_rows(t: &TestApp) -> i64 {
    t.db()
        .query_row("SELECT COUNT(*) FROM usage", [], |r| r.get(0))
        .expect("usage")
}

fn upstream(t: &TestApp) -> Vec<Value> {
    t.stub.last().body["messages"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn content(v: &Value, id: i64) -> String {
    v["messages"]
        .as_array()
        .and_then(|a| a.iter().find(|m| m["id"] == json!(id)))
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string()
}

/// Sends q1 q2 q3 and returns (chat, [q1, a1, q2, a2, q3, a3]).
async fn three_pairs(t: &TestApp, who: &Who) -> (i64, Vec<i64>) {
    let (chat, q1, a1, _) = send(t, who, None, "first").await;
    let (_, q2, a2, _) = send(t, who, Some(chat), "second").await;
    let (_, q3, a3, _) = send(t, who, Some(chat), "third").await;
    (chat, vec![q1, a1, q2, a2, q3, a3])
}

// —— read and send ——————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn a_chat_is_one_line_until_it_branches_and_the_keys_follow_the_role() {
    let (t, admin, sam) = setup().await;
    let (chat, q1, a1, ev) = send(&t, &sam, None, "first").await;
    assert_eq!(ev[0].1["mode"], json!("send"));
    assert_eq!(ev[0].1["parentId"], json!(q1));
    assert_eq!(ev[0].1["userParentId"], Value::Null);
    assert!(ev[0].1.get("versions").is_none());
    assert!(ev[0].1.get("connection").is_none(), "users never see it");
    assert!(ev[0].1.get("model").is_none(), "users never see the model");
    let (_, q2, a2, ev) = send(&t, &sam, Some(chat), "second").await;
    assert_eq!(ev[0].1["userParentId"], json!(a1));
    let v = read(&t, &sam, chat).await;
    assert_eq!(ids(&v), vec![q1, a1, q2, a2]);
    assert_eq!(v["chat"]["leafId"], json!(a2));
    assert_eq!(v["chat"]["messageCount"], json!(4));
    let ms = v["messages"].as_array().expect("messages");
    assert_eq!(ms[0]["parentId"], Value::Null);
    assert_eq!(ms[1]["parentId"], json!(q1));
    assert_eq!(ms[3]["parentId"], json!(q2));
    for m in ms {
        assert!(m.get("versions").is_none() && m.get("connection").is_none());
        assert!(m.get("model").is_none());
    }
    assert!(v["chat"].get("pick").is_none());
    // an administrator also gets the pick and the name of the connection
    let (achat, _, _, ev) = send(&t, &admin, None, "hello").await;
    assert_eq!(ev[0].1["connection"], json!("main"));
    let av = read(&t, &admin, achat).await;
    assert_eq!(av["chat"]["pick"], Value::Null);
    assert_eq!(av["messages"][1]["connection"], json!("main"));
    assert_eq!(av["messages"][0]["connection"], Value::Null);
}

// —— regenerate ———————————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn regenerate_adds_a_version_and_hides_the_old_tail() {
    let (t, _admin, sam) = setup().await;
    let (chat, m) = three_pairs(&t, &sam).await;
    let before = read(&t, &sam, chat).await;
    let usage = usage_rows(&t);
    let ev = ask(
        &t,
        &sam,
        "/api/ai/regenerate",
        json!({"chatId": chat, "messageId": m[3], "lang": "en"}),
    )
    .await;
    assert_eq!(terminal(&ev).0, "done");
    let s = &ev[0].1;
    let new = s["assistantMessageId"].as_i64().expect("new answer");
    assert_ne!(new, m[3]);
    assert_eq!(s["mode"], json!("regenerate"));
    assert_eq!(s["parentId"], json!(m[2]));
    assert_eq!(s["userMessageId"], json!(m[2]));
    assert_eq!(s["versions"], json!([m[3], new]));
    assert_eq!(s["retry"], json!(false));
    let v = read(&t, &sam, chat).await;
    assert_eq!(ids(&v), vec![m[0], m[1], m[2], new]);
    assert_eq!(v["chat"]["leafId"], json!(new));
    assert_eq!(v["chat"]["messageCount"], json!(7), "every version counts");
    assert_eq!(v["messages"][3]["versions"], json!([m[3], new]));
    assert_eq!(usage_rows(&t), usage + 1, "one usage row");
    // the context holds the path before the question and the question; never the old answer or its tail
    let up = upstream(&t);
    assert_eq!(up.len(), 4, "{up:?}");
    assert_eq!(up[1]["content"], json!("first"));
    assert_eq!(up[2]["content"], json!(content(&before, m[1])));
    assert!(up[3]["content"]
        .as_str()
        .unwrap_or("")
        .starts_with("second"));
    assert!(!up
        .iter()
        .any(|x| x["content"].as_str().unwrap_or("").starts_with("third")));
}

#[tokio::test]
async fn the_second_regenerate_of_a_question_is_attempt_three() {
    let (t, _admin, sam) = setup().await;
    let (chat, q, a, _) = send(&t, &sam, None, "first").await;
    let ev = ask(
        &t,
        &sam,
        "/api/ai/regenerate",
        json!({"chatId": chat, "messageId": a}),
    )
    .await;
    let a2 = ev[0].1["assistantMessageId"].as_i64().expect("a2");
    let last = upstream(&t).last().cloned().expect("last");
    assert!(last["content"]
        .as_str()
        .unwrap_or("")
        .contains("Regenerate request #2"));
    let ev = ask(
        &t,
        &sam,
        "/api/ai/regenerate",
        json!({"chatId": chat, "messageId": a2}),
    )
    .await;
    assert_eq!(ev[0].1["versions"].as_array().map(Vec::len), Some(3));
    let last = upstream(&t).last().cloned().expect("last");
    assert!(last["content"]
        .as_str()
        .unwrap_or("")
        .contains("Regenerate request #3"));
    let n: i64 = t
        .db()
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE parent_id = ?1 AND role = 'assistant'",
            [q],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(n, 3);
}

#[tokio::test]
async fn an_empty_failed_answer_is_reset_in_place_and_retry_is_a_regenerate_of_the_leaf() {
    let (t, _admin, sam) = setup().await;
    t.stub.force(Some((
        500,
        r#"{"error":{"message":"made-up failure","type":"server_error"}}"#,
    )));
    let (chat, _q, a, ev) = send(&t, &sam, None, "first").await;
    assert_eq!(terminal(&ev).0, "error");
    t.stub.force(None);
    let ev = ask(
        &t,
        &sam,
        "/api/ai/regenerate",
        json!({"chatId": chat, "messageId": a}),
    )
    .await;
    assert_eq!(ev[0].1["retry"], json!(true));
    assert_eq!(ev[0].1["assistantMessageId"], json!(a), "the same row");
    assert_eq!(terminal(&ev).0, "done");
    assert_eq!(stored(&t, chat), 2, "no version for an empty failure");
    // a done answer: the old route makes a new version of the leaf
    let ev = ask(&t, &sam, "/api/ai/retry", json!({"chatId": chat})).await;
    assert_eq!(ev[0].1["mode"], json!("regenerate"));
    assert_eq!(ev[0].1["retry"], json!(false));
    assert_eq!(stored(&t, chat), 3);
}

#[tokio::test]
async fn a_target_must_be_an_answer_on_the_path_and_the_refusals_write_nothing() {
    let (t, _admin, sam) = setup().await;
    let (chat, m) = three_pairs(&t, &sam).await;
    let ev = ask(
        &t,
        &sam,
        "/api/ai/regenerate",
        json!({"chatId": chat, "messageId": m[3]}),
    )
    .await;
    let new = ev[0].1["assistantMessageId"].as_i64().expect("new");
    let (rows, usage) = (stored(&t, chat), usage_rows(&t));
    for (path, body, status, code) in [
        // a user message cannot be regenerated, an answer cannot be edited
        (
            "/api/ai/regenerate",
            json!({"chatId": chat, "messageId": m[0]}),
            409,
            "ai.notRetryable",
        ),
        (
            "/api/ai/edit",
            json!({"chatId": chat, "messageId": m[1], "text": "x"}),
            409,
            "ai.notRetryable",
        ),
        // the old version is no longer on the viewed path
        (
            "/api/ai/regenerate",
            json!({"chatId": chat, "messageId": m[3]}),
            409,
            "ai.branchMoved",
        ),
        (
            "/api/ai/edit",
            json!({"chatId": chat, "messageId": m[4], "text": "x"}),
            409,
            "ai.branchMoved",
        ),
        // not in the chat
        (
            "/api/ai/regenerate",
            json!({"chatId": chat, "messageId": 99999}),
            404,
            "ai.notFound",
        ),
        (
            "/api/ai/regenerate",
            json!({"chatId": 99999, "messageId": new}),
            404,
            "ai.notFound",
        ),
        // bad bodies
        (
            "/api/ai/edit",
            json!({"chatId": chat, "messageId": m[0], "text": "  "}),
            400,
            "ai.badRequest",
        ),
        (
            "/api/ai/regenerate",
            json!({"chatId": chat, "messageId": m[2], "extra": 1}),
            400,
            "ai.badRequest",
        ),
    ] {
        let (c, v) = t.post(path, &sam, body).await;
        assert_eq!((c, v["code"].clone()), (status, json!(code)), "{path} {v}");
    }
    assert_eq!((stored(&t, chat), usage_rows(&t)), (rows, usage));
    // another user's chat and message
    let other = t.user("kim", false, true);
    let (c, v) = t
        .post(
            "/api/ai/regenerate",
            &other,
            json!({"chatId": chat, "messageId": new}),
        )
        .await;
    assert_eq!((c, v["code"].clone()), (404, json!("ai.notFound")));
    let (_, kq, ka, _) = send(&t, &other, None, "mine").await;
    let _ = kq;
    let (c, v) = t
        .post(
            "/api/ai/regenerate",
            &sam,
            json!({"chatId": chat, "messageId": ka}),
        )
        .await;
    assert_eq!((c, v["code"].clone()), (404, json!("ai.notFound")));
}

// —— edit ———————————————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn editing_the_first_message_makes_a_second_root() {
    let (t, _admin, sam) = setup().await;
    let (chat, m) = three_pairs(&t, &sam).await;
    let ev = ask(
        &t,
        &sam,
        "/api/ai/edit",
        json!({"chatId": chat, "messageId": m[0], "text": "  fresh start  ", "lang": "en"}),
    )
    .await;
    assert_eq!(terminal(&ev).0, "done");
    let s = &ev[0].1;
    let (nq, na) = (
        s["userMessageId"].as_i64().expect("q"),
        s["assistantMessageId"].as_i64().expect("a"),
    );
    assert_eq!(s["mode"], json!("edit"));
    assert_eq!(s["userParentId"], Value::Null);
    assert_eq!(s["parentId"], json!(nq));
    assert_eq!(s["userVersions"], json!([m[0], nq]));
    assert!(s.get("versions").is_none(), "one answer so far");
    let v = read(&t, &sam, chat).await;
    assert_eq!(ids(&v), vec![nq, na]);
    assert_eq!(v["messages"][0]["content"], json!("fresh start"));
    assert_eq!(v["messages"][0]["versions"], json!([m[0], nq]));
    assert_eq!(v["messages"][0]["parentId"], Value::Null);
    assert_eq!(stored(&t, chat), 8, "the old line stays stored");
    let up = upstream(&t);
    assert_eq!(up.len(), 2, "{up:?}");
    assert_eq!(up[1]["content"], json!("fresh start"));
}

#[tokio::test]
async fn editing_a_middle_message_cuts_the_history_there() {
    let (t, _admin, sam) = setup().await;
    let (chat, m) = three_pairs(&t, &sam).await;
    let before = read(&t, &sam, chat).await;
    let ev = ask(
        &t,
        &sam,
        "/api/ai/edit",
        json!({"chatId": chat, "messageId": m[2], "text": "second, reworded"}),
    )
    .await;
    let nq = ev[0].1["userMessageId"].as_i64().expect("q");
    assert_eq!(ev[0].1["userParentId"], json!(m[1]));
    let up = upstream(&t);
    assert_eq!(up.len(), 4, "{up:?}");
    assert_eq!(up[1]["content"], json!("first"));
    assert_eq!(up[2]["content"], json!(content(&before, m[1])));
    assert_eq!(up[3]["content"], json!("second, reworded"));
    let v = read(&t, &sam, chat).await;
    assert_eq!(ids(&v)[..3], [m[0], m[1], nq]);
    assert_eq!(v["messages"][2]["versions"], json!([m[2], nq]));
}

// —— switching versions ——————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn switching_versions_restores_each_tail_and_costs_nothing() {
    let (t, _admin, sam) = setup().await;
    let (chat, m) = three_pairs(&t, &sam).await;
    let ev = ask(
        &t,
        &sam,
        "/api/ai/regenerate",
        json!({"chatId": chat, "messageId": m[3]}),
    )
    .await;
    let new = ev[0].1["assistantMessageId"].as_i64().expect("new");
    t.db()
        .execute("UPDATE chats SET updated_at = 1", [])
        .expect("old time");
    let (calls, usage) = (t.stub.count(), usage_rows(&t));
    // back to the old version: its whole tail is shown again
    let (c, v) = t
        .post(
            "/api/ai/branch",
            &sam,
            json!({"chatId": chat, "messageId": m[3]}),
        )
        .await;
    assert_eq!(c, 200, "{v}");
    assert_eq!(ids(&v), m);
    assert_eq!(v["chat"]["leafId"], json!(m[5]));
    assert_eq!(v["messages"][3]["versions"], json!([m[3], new]));
    // and forth
    let (_, v) = t
        .post(
            "/api/ai/branch",
            &sam,
            json!({"chatId": chat, "messageId": new}),
        )
        .await;
    assert_eq!(ids(&v), vec![m[0], m[1], m[2], new]);
    assert_eq!(v["chat"]["leafId"], json!(new));
    // the choice is stored: a fresh read shows the same line
    let v = read(&t, &sam, chat).await;
    assert_eq!(ids(&v), vec![m[0], m[1], m[2], new]);
    assert_eq!((t.stub.count(), usage_rows(&t)), (calls, usage));
    let updated: i64 = t
        .db()
        .query_row("SELECT updated_at FROM chats", [], |r| r.get(0))
        .expect("updated");
    assert_eq!(updated, 1, "switching never reorders the list");
    // a send after switching continues the shown branch
    let (_, nq, _, _) = send(&t, &sam, Some(chat), "fourth").await;
    let parent: Option<i64> = t
        .db()
        .query_row("SELECT parent_id FROM messages WHERE id = ?1", [nq], |r| {
            r.get(0)
        })
        .expect("parent");
    assert_eq!(parent, Some(new));
    // a message of another chat or user is a 404
    let other = t.user("kim", false, true);
    let (_, _, ka, _) = send(&t, &other, None, "mine").await;
    for (who, body) in [
        (&sam, json!({"chatId": chat, "messageId": ka})),
        (&other, json!({"chatId": chat, "messageId": new})),
    ] {
        let (c, v) = t.post("/api/ai/branch", who, body).await;
        assert_eq!((c, v["code"].clone()), (404, json!("ai.notFound")));
    }
}

#[tokio::test]
async fn a_running_chat_refuses_branch_regenerate_and_edit() {
    let (t, admin, sam) = setup().await;
    let (chat, q, a, _) = send(&t, &sam, None, "first").await;
    t.set_model(&admin, "demo-hold").await;
    let mut s = t
        .open_stream(
            "/api/ai/send",
            &sam,
            json!({"chatId": chat, "text": "slow one"}),
        )
        .await
        .expect("stream");
    s.until("start").await;
    for (path, body) in [
        ("/api/ai/branch", json!({"chatId": chat, "messageId": a})),
        (
            "/api/ai/regenerate",
            json!({"chatId": chat, "messageId": a}),
        ),
        (
            "/api/ai/edit",
            json!({"chatId": chat, "messageId": q, "text": "x"}),
        ),
    ] {
        let (c, v) = t.post(path, &sam, body).await;
        assert_eq!((c, v["code"].clone()), (409, json!("ai.busy")), "{path}");
    }
    t.stub.release();
    s.all().await;
    // idle again: the same switch works
    let (c, _) = t
        .post(
            "/api/ai/branch",
            &sam,
            json!({"chatId": chat, "messageId": a}),
        )
        .await;
    assert_eq!(c, 200);
}

// —— limits, caps, switches ——————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn every_version_counts_against_the_messages_per_chat_limit() {
    let (t, _admin, sam) = setup().await;
    let mut l = t.app.0.limits();
    l.messages_per_chat = 4;
    t.app.0.set_limits(l);
    let (chat, q, a, _) = send(&t, &sam, None, "first").await;
    let regen = |id: i64| json!({"chatId": chat, "messageId": id});
    let ev = ask(&t, &sam, "/api/ai/regenerate", regen(a)).await;
    let a2 = ev[0].1["assistantMessageId"].as_i64().expect("a2");
    let ev = ask(&t, &sam, "/api/ai/regenerate", regen(a2)).await;
    let a3 = ev[0].1["assistantMessageId"].as_i64().expect("a3");
    assert_eq!(stored(&t, chat), 4);
    for (path, body) in [
        ("/api/ai/regenerate", regen(a3)),
        (
            "/api/ai/edit",
            json!({"chatId": chat, "messageId": q, "text": "x"}),
        ),
        ("/api/ai/send", json!({"chatId": chat, "text": "x"})),
    ] {
        let (c, v) = t.post(path, &sam, body).await;
        assert_eq!(
            (c, v["code"].clone()),
            (409, json!("ai.messageLimit")),
            "{path}"
        );
    }
    assert_eq!(stored(&t, chat), 4);
}

#[tokio::test]
async fn regenerate_and_edit_count_as_calls_and_a_capped_one_writes_nothing() {
    let (t, _admin, sam) = setup().await;
    let mut l = t.app.0.limits();
    l.calls_per_minute = 3;
    t.app.0.set_limits(l);
    let (chat, q, a, _) = send(&t, &sam, None, "first").await;
    ask(
        &t,
        &sam,
        "/api/ai/regenerate",
        json!({"chatId": chat, "messageId": a}),
    )
    .await;
    ask(
        &t,
        &sam,
        "/api/ai/edit",
        json!({"chatId": chat, "messageId": q, "text": "again"}),
    )
    .await;
    assert_eq!(usage_rows(&t), 3);
    let rows = stored(&t, chat);
    let (c, v) = t
        .post(
            "/api/ai/regenerate",
            &sam,
            json!({"chatId": chat, "messageId": a}),
        )
        .await;
    assert_eq!(c, 429, "{v}");
    assert_eq!((stored(&t, chat), usage_rows(&t)), (rows, 3));
    // switching a version is free even now
    let (c, _) = t
        .post(
            "/api/ai/branch",
            &sam,
            json!({"chatId": chat, "messageId": q}),
        )
        .await;
    assert_eq!(c, 200);
}

#[tokio::test]
async fn an_exempt_admin_writes_usage_without_a_cap_refusal() {
    let (t, admin, sam) = setup().await;
    let mut l = t.app.0.limits();
    l.calls_per_minute = 1;
    l.calls_admins_exempt = 1;
    t.app.0.set_limits(l);
    let mut chat = None;
    for i in 0..3 {
        let (c, _, _, ev) = send(&t, &admin, chat, &format!("q{i}")).await;
        assert_eq!(terminal(&ev).0, "done");
        chat = Some(c);
    }
    assert_eq!(usage_rows(&t), 3);
    let (_, st) = t.get("/api/ai/status", &admin).await;
    assert_eq!(st["calls"]["exempt"], json!(true));
    assert_eq!(st["calls"]["used"]["minute"], json!(3));
    // a plain user is not exempt and gets no such key
    send(&t, &sam, None, "one").await;
    let (_, st) = t.get("/api/ai/status", &sam).await;
    assert!(st["calls"].get("exempt").is_none(), "{st}");
    let (c, _) = t.post("/api/ai/send", &sam, json!({"text": "two"})).await;
    assert_eq!(c, 429);
}

#[tokio::test]
async fn a_feature_off_user_is_refused_before_any_write() {
    let (t, _admin, sam) = setup().await;
    let (chat, q, a, _) = send(&t, &sam, None, "first").await;
    t.app
        .0
        .store
        .write(|tx| {
            data::set_features_off(
                tx,
                sam.id,
                &sam.stamp,
                &sam.username,
                Feature::Chat.bit(),
                1,
            )
        })
        .expect("bit");
    let (rows, usage) = (stored(&t, chat), usage_rows(&t));
    for (path, body) in [
        (
            "/api/ai/regenerate",
            json!({"chatId": chat, "messageId": a}),
        ),
        (
            "/api/ai/edit",
            json!({"chatId": chat, "messageId": q, "text": "x"}),
        ),
    ] {
        let (c, v) = t.post(path, &sam, body).await;
        assert_eq!(
            (c, v["code"].clone()),
            (403, json!("ai.featureOff")),
            "{path}"
        );
    }
    assert_eq!((stored(&t, chat), usage_rows(&t)), (rows, usage));
    // reading and switching versions stay possible
    let (c, _) = t
        .post(
            "/api/ai/branch",
            &sam,
            json!({"chatId": chat, "messageId": q}),
        )
        .await;
    assert_eq!(c, 200);
}

// —— the administrator's model pick ——————————————————————————————————————————————————————————————————————————————————

fn big() -> Value {
    json!({"connectionId": 1, "model": "demo-chat-large"})
}

#[tokio::test]
async fn an_administrator_picks_the_model_per_chat_and_a_user_may_not() {
    let (t, admin, sam) = setup().await;
    // a plain user: any non-null model is refused, null is the default
    let (c, v) = t
        .post("/api/ai/send", &sam, json!({"text": "x", "model": big()}))
        .await;
    assert_eq!((c, v["code"].clone()), (403, json!("ai.notAllowed")));
    let ev = ask(
        &t,
        &sam,
        "/api/ai/send",
        json!({"text": "x", "model": Value::Null}),
    )
    .await;
    assert_eq!(terminal(&ev).0, "done");
    // outside free mode an administrator's named model is refused too, and nothing is written
    let rows = usage_rows(&t);
    let (c, v) = t
        .post("/api/ai/send", &admin, json!({"text": "x", "model": big()}))
        .await;
    assert_eq!((c, v["code"].clone()), (400, json!("ai.badModel")), "{v}");
    assert_eq!(usage_rows(&t), rows);
    // the model menu belongs to free mode
    let (c, _) = t.post("/api/ai/free", &admin, json!({"free": true})).await;
    assert_eq!(c, 200);
    // an administrator: bad references are 400 and write nothing
    for bad in [
        json!({"connectionId": 99, "model": "demo-chat-large"}),
        json!({"connectionId": 1}),
        json!({"connectionId": 1, "model": ""}),
        json!("1:demo"),
    ] {
        let (c, v) = t
            .post("/api/ai/send", &admin, json!({"text": "x", "model": bad}))
            .await;
        assert_eq!((c, v["code"].clone()), (400, json!("ai.badModel")), "{v}");
    }
    assert_eq!(usage_rows(&t), rows);
    // a valid pick is used, stored, and shown
    let (chat, _, _, ev) = {
        let ev = ask(
            &t,
            &admin,
            "/api/ai/send",
            json!({"text": "hello", "model": big()}),
        )
        .await;
        let c = ev[0].1["chatId"].as_i64().expect("chat");
        (c, 0, 0, ev)
    };
    assert_eq!(ev[0].1["model"], json!("demo-chat-large"));
    assert_eq!(t.stub.last().body["model"], json!("demo-chat-large"));
    let v = read(&t, &admin, chat).await;
    assert_eq!(v["chat"]["pick"], big());
    assert_eq!(v["messages"][1]["model"], json!("demo-chat-large"));
    // no `model` key: the stored pick is used again
    ask(
        &t,
        &admin,
        "/api/ai/send",
        json!({"chatId": chat, "text": "more"}),
    )
    .await;
    assert_eq!(t.stub.last().body["model"], json!("demo-chat-large"));
    // null clears it
    ask(
        &t,
        &admin,
        "/api/ai/send",
        json!({"chatId": chat, "text": "default", "model": Value::Null}),
    )
    .await;
    assert_eq!(t.stub.last().body["model"], json!("demo-chat-small"));
    assert_eq!(read(&t, &admin, chat).await["chat"]["pick"], Value::Null);
    // a regenerate may carry a pick too
    let v = read(&t, &admin, chat).await;
    let last = ids(&v).pop().expect("last");
    ask(
        &t,
        &admin,
        "/api/ai/regenerate",
        json!({"chatId": chat, "messageId": last, "model": big()}),
    )
    .await;
    assert_eq!(t.stub.last().body["model"], json!("demo-chat-large"));
    assert_eq!(read(&t, &admin, chat).await["chat"]["pick"], big());
}

/// A second connection that is broken: it exists, and it cannot serve calls.
fn broken_connection(t: &TestApp) -> i64 {
    let id = t
        .app
        .0
        .store
        .write(|tx| {
            conn::insert_connection(
                tx,
                &conn::NewConn {
                    name: "spare",
                    provider: "openai",
                    base_url: "https://api.example.com/v1",
                    format: "openai",
                    gateway_auth: "bearer",
                    lists_models: true,
                    by: "admin",
                    now: 1_800_000_000,
                },
            )
        })
        .expect("connection");
    t.db()
        .execute(
            "UPDATE connections SET status = 'broken', error_code = 'ai.up.auth' WHERE id = ?1",
            [id],
        )
        .expect("broken");
    id
}

fn stored_pick(t: &TestApp, chat: i64) -> (Option<i64>, Option<String>) {
    t.db()
        .query_row(
            "SELECT pick_conn, pick_model FROM chats WHERE id = ?1",
            [chat],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("pick")
}

#[tokio::test]
async fn a_stored_pick_that_no_longer_resolves_falls_back_and_is_cleared() {
    let (t, admin, _sam) = setup().await;
    let (c, _) = t.post("/api/ai/free", &admin, json!({"free": true})).await;
    assert_eq!(c, 200);
    let (chat, _, _, _) = send(&t, &admin, None, "hello").await;
    // the connection of the pick exists but cannot serve calls
    let spare = broken_connection(&t);
    t.db()
        .execute(
            "UPDATE chats SET pick_conn = ?2, pick_model = 'demo-gone' WHERE id = ?1",
            [chat, spare],
        )
        .expect("unusable pick");
    assert_eq!(
        read(&t, &admin, chat).await["chat"]["pick"],
        json!({"connectionId": spare, "model": "demo-gone"})
    );
    let (_, _, _, ev) = send(&t, &admin, Some(chat), "again").await;
    assert_eq!(ev[0].1["modelFallback"], json!(true));
    assert_eq!(ev[0].1["model"], json!("demo-chat-small"));
    assert_eq!(terminal(&ev).0, "done");
    assert_eq!(read(&t, &admin, chat).await["chat"]["pick"], Value::Null);
    assert_eq!(stored_pick(&t, chat), (None, None));
    // the next call has nothing to report
    let (_, _, _, ev) = send(&t, &admin, Some(chat), "and again").await;
    assert!(ev[0].1.get("modelFallback").is_none());
}

#[tokio::test]
async fn a_stored_pick_of_a_missing_connection_is_no_pick() {
    let (mut t, admin, _sam) = setup().await;
    let (c, _) = t.post("/api/ai/free", &admin, json!({"free": true})).await;
    assert_eq!(c, 200);
    let (chat, _, _, _) = send(&t, &admin, None, "hello").await;
    t.db()
        .execute(
            "UPDATE chats SET pick_conn = 99, pick_model = 'demo-gone' WHERE id = ?1",
            [chat],
        )
        .expect("stale pick");
    // neither the chat nor the list reports it
    assert_eq!(read(&t, &admin, chat).await["chat"]["pick"], Value::Null);
    let (c, v) = t.get("/api/ai/chats", &admin).await;
    assert_eq!(c, 200, "{v}");
    assert!(!v.to_string().contains("demo-gone"), "{v}");
    // the chat default answers, and there is no fallback to report: the chat has no pick
    let (_, _, _, ev) = send(&t, &admin, Some(chat), "again").await;
    assert!(ev[0].1.get("modelFallback").is_none(), "{:?}", ev[0]);
    assert_eq!(ev[0].1["model"], json!("demo-chat-small"));
    assert_eq!(t.stub.last().body["model"], json!("demo-chat-small"));
    assert_eq!(terminal(&ev).0, "done");
    // the next start clears the stored value, and one more start changes nothing
    t.restart(Opts::default()).await;
    assert_eq!(stored_pick(&t, chat), (None, None));
    t.restart(Opts::default()).await;
    assert_eq!(stored_pick(&t, chat), (None, None));
}

#[tokio::test]
async fn deleting_a_connection_clears_the_chat_picks_of_it() {
    let (t, admin, _sam) = setup().await;
    let (c, _) = t.post("/api/ai/free", &admin, json!({"free": true})).await;
    assert_eq!(c, 200);
    let (gone, _, _, _) = send(&t, &admin, None, "hello").await;
    let (kept, _, _, _) = send(&t, &admin, None, "hello again").await;
    let spare = broken_connection(&t);
    t.db()
        .execute(
            "UPDATE chats SET pick_conn = ?2, pick_model = 'demo-spare' WHERE id = ?1",
            [gone, spare],
        )
        .expect("pick of the spare connection");
    ask(
        &t,
        &admin,
        "/api/ai/send",
        json!({"chatId": kept, "text": "big", "model": big()}),
    )
    .await;
    let (c, v) = t
        .post(
            "/api/ai/admin/connections/delete",
            &admin,
            json!({"id": spare}),
        )
        .await;
    assert_eq!(c, 200, "{v}");
    assert_eq!(stored_pick(&t, gone), (None, None));
    assert_eq!(read(&t, &admin, gone).await["chat"]["pick"], Value::Null);
    // a pick of another connection stays
    assert_eq!(read(&t, &admin, kept).await["chat"]["pick"], big());
    // the chat answers with the default, with nothing to report
    let (_, _, _, ev) = send(&t, &admin, Some(gone), "again").await;
    assert!(ev[0].1.get("modelFallback").is_none(), "{:?}", ev[0]);
    assert_eq!(ev[0].1["model"], json!("demo-chat-small"));
}

#[tokio::test]
async fn the_chat_model_route_sets_and_clears_the_pick_without_a_call() {
    let (t, admin, sam) = setup().await;
    let (chat, _, _, _) = send(&t, &admin, None, "hello").await;
    // outside free mode a named model is refused
    let (c, v) = t
        .post(
            "/api/ai/chat/model",
            &admin,
            json!({"chatId": chat, "model": big()}),
        )
        .await;
    assert_eq!((c, v["code"].clone()), (400, json!("ai.badModel")), "{v}");
    let (c, _) = t.post("/api/ai/free", &admin, json!({"free": true})).await;
    assert_eq!(c, 200);
    t.db()
        .execute("UPDATE chats SET updated_at = 1", [])
        .expect("old time");
    let (calls, usage) = (t.stub.count(), usage_rows(&t));
    let (c, v) = t
        .post(
            "/api/ai/chat/model",
            &admin,
            json!({"chatId": chat, "model": big()}),
        )
        .await;
    assert_eq!(c, 200, "{v}");
    assert_eq!(v["chat"]["pick"], big());
    assert_eq!(v["chat"]["id"], json!(chat));
    assert!(v.get("messages").is_none());
    let (_, v) = t
        .post(
            "/api/ai/chat/model",
            &admin,
            json!({"chatId": chat, "model": Value::Null}),
        )
        .await;
    assert_eq!(v["chat"]["pick"], Value::Null);
    assert_eq!((t.stub.count(), usage_rows(&t)), (calls, usage));
    let updated: i64 = t
        .db()
        .query_row("SELECT updated_at FROM chats", [], |r| r.get(0))
        .expect("updated");
    assert_eq!(updated, 1);
    // refusals
    let (c, v) = t
        .post(
            "/api/ai/chat/model",
            &admin,
            json!({"chatId": chat, "model": {"connectionId": 7, "model": "m"}}),
        )
        .await;
    assert_eq!((c, v["code"].clone()), (400, json!("ai.badModel")));
    let (c, _) = t
        .post("/api/ai/chat/model", &admin, json!({"chatId": chat}))
        .await;
    assert_eq!(c, 400);
    let (c, v) = t
        .post(
            "/api/ai/chat/model",
            &admin,
            json!({"chatId": 9999, "model": big()}),
        )
        .await;
    assert_eq!((c, v["code"].clone()), (404, json!("ai.notFound")));
    let (c, v) = t
        .post(
            "/api/ai/chat/model",
            &sam,
            json!({"chatId": chat, "model": big()}),
        )
        .await;
    assert_eq!((c, v["code"].clone()), (403, json!("ai.notAllowed")));
}

// —— status ———————————————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn the_status_keys_follow_the_role_the_switches_and_the_slots() {
    let (t, admin, sam) = setup().await;
    let a = t.get("/api/ai/status", &admin).await.1;
    assert_eq!(
        a["chatDefault"],
        json!({"connectionId": 1, "connectionName": "main", "model": "demo-chat-small"})
    );
    assert!(a.get("chatModels").is_none());
    assert_eq!(a["configured"], json!(true));
    let (_, s) = t.get("/api/ai/status", &sam).await;
    assert!(s.get("chatDefault").is_none() && s.get("chatModels").is_none());
    assert_eq!(s["features"]["chat"], json!(true));
    // a global switch turns a feature off for everyone, administrators included (v1.9)
    t.app
        .0
        .store
        .write(|tx| conn::set_features_off(tx, Feature::Hint.bit()))
        .expect("bits");
    let (_, s) = t.get("/api/ai/status", &sam).await;
    assert_eq!(s["features"]["hint"], json!(false));
    assert_eq!(s["features"]["chat"], json!(true));
    let (_, a) = t.get("/api/ai/status", &admin).await;
    assert_eq!(a["features"]["hint"], json!(false));
    assert_eq!(a["features"]["explain"], json!(true));
    // the per-user bit applies to the user only
    t.app
        .0
        .store
        .write(|tx| {
            data::set_features_off(
                tx,
                sam.id,
                &sam.stamp,
                &sam.username,
                Feature::Chat.bit(),
                1,
            )
        })
        .expect("bit");
    let (_, s) = t.get("/api/ai/status", &sam).await;
    assert_eq!(s["features"]["chat"], json!(false));
    // a slot that does not resolve switches its feature off (the default does not answer for a failed model)
    t.db()
        .execute("UPDATE models SET status = 'failed'", [])
        .expect("fail");
    let (_, a) = t.get("/api/ai/status", &admin).await;
    assert_eq!(a["features"]["chat"], json!(false));
    assert_eq!(a["chatDefault"], Value::Null);
}
