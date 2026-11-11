//! The AI summary of a record: the stream, the sample numbers, the chat, the refusals and the saved lookup.
use super::util::*;
use serde_json::{json, Value};

fn at_digits(at: &str) -> String {
    at.chars().filter(char::is_ascii_digit).collect()
}

fn sum_body(record: i64) -> Value {
    json!({"recordId": record, "lang": "en"})
}

fn wrong_record(user: &Who, id: i64, n: usize) -> FakeRecord {
    let mut items = vec![
        item("q1", json!(["B"]), true),
        item("q2", Value::Null, false),
    ];
    for i in 0..n {
        items.push(item(
            if i % 2 == 0 { "q1" } else { "qh" },
            if i % 2 == 0 {
                json!(["A"])
            } else {
                json!([1, 0, 1])
            },
            false,
        ));
    }
    record(user, id, "exam", Some(K2), "2026-11-11 21:58", items)
}

#[tokio::test]
async fn a_summary_streams_with_its_sample_and_lands_in_the_record_chat() {
    let e = env().await;
    e.t.add_record(wrong_record(&e.user, 42, 3));
    let ev = ask_to(&e.t, &e.user, SUMMARY_ASK, sum_body(42)).await;
    assert_eq!(terminal_name(&ev), "done");
    let s = start_of(&ev);
    assert_eq!(s["chatKind"], "summary");
    assert_eq!(s["kind"], "summary");
    assert_eq!(s["questionId"], Value::Null);
    assert_eq!(s["questionNo"], Value::Null);
    assert_eq!(s["source"], Value::Null);
    assert_eq!(s["recordId"], 42);
    assert_eq!(s["sample"], json!({"used": 3, "usable": 3, "wrong": 3}));
    assert_eq!(s["chatTitle"], "2026-11-11 21:58");
    assert!(text(&ev).contains("Items: 3"));
    assert!(text(&ev).contains("## Weak knowledge points"));
    let (kind, rf, title, rec): (String, String, String, Option<i64>) =
        e.t.db()
            .query_row("SELECT kind, ref, title, record_id FROM chats", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .expect("chat");
    assert_eq!(
        (kind.as_str(), rf.as_str(), title.as_str(), rec),
        (
            "summary",
            "r42@2026-11-11 21:58",
            "2026-11-11 21:58",
            Some(42)
        )
    );
    let (k, qid, src, used, usable, wrong, content): (String, Option<String>, Option<String>, i64, i64, i64, String) = e
        .t
        .db()
        .query_row(
            "SELECT kind, question_id, source, sample_used, sample_usable, sample_wrong, content FROM messages
             WHERE role = 'user'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .expect("message");
    assert_eq!(
        (k.as_str(), qid, src, used, usable, wrong),
        ("summary", None, None, 3, 3, 3)
    );
    assert_eq!(content, "[AI summary] 2026-11-11 21:58 exam 3/3/3");
    let purpose: Option<String> = q1(&e.t, "SELECT purpose FROM usage WHERE kind = 'chat'");
    assert_eq!(purpose.as_deref(), Some("summary"));
    // the prompt: one system prompt, the items as the one user turn, no guard, nothing about "random"
    let req = e.t.stub.last();
    let msgs = req.body["messages"].as_array().expect("messages");
    assert_eq!(msgs.len(), 2);
    assert!(msgs[0]["content"]
        .as_str()
        .expect("system")
        .contains("3 of their 3 wrong answers"));
    assert!(!msgs[0]["content"]
        .as_str()
        .expect("system")
        .contains("chosen at random"));
    // the saved lookup by record id and the 12 digits of its time
    let url = format!(
        "/api/ai/summary/saved?record=42&at={}",
        at_digits("2026-11-11 21:58")
    );
    let (st, v) = e.t.get(&url, &e.user).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["chatId"], s["chatId"]);
    assert_eq!(v["latest"]["status"], "done");
    assert_eq!(v["latest"]["recordId"], 42);
    assert_eq!(
        v["latest"]["sample"],
        json!({"used": 3, "usable": 3, "wrong": 3})
    );
    assert_eq!(v["lastGood"], Value::Null);
    // another time (a reused record id) is another chat; a space or %20 in the time is refused
    let (_, other) =
        e.t.get("/api/ai/summary/saved?record=42&at=202611112159", &e.user)
            .await;
    assert_eq!(
        other,
        json!({"chatId": null, "latest": null, "lastGood": null})
    );
    for bad in [
        "/api/ai/summary/saved?record=42&at=2026-11-11-21:58",
        "/api/ai/summary/saved?record=42&at=2026-11-11%2021:58",
        "/api/ai/summary/saved?record=42&at=20261111215",
        "/api/ai/summary/saved?at=202611112158",
        "/api/ai/summary/saved?record=0&at=202611112158",
    ] {
        let (s, v) = e.t.get(bad, &e.user).await;
        assert_eq!((s, &v["code"]), (400, &json!("ai.badRequest")), "{bad}");
    }
    // a second summary of the record is a new pair in the same chat
    let s2 = start_of(&ask_to(&e.t, &e.user, SUMMARY_ASK, sum_body(42)).await);
    assert_eq!(s2["chatId"], s["chatId"]);
    assert_eq!(s2["newChat"], false);
    let (_, v2) = e.t.get(&url, &e.user).await;
    assert_eq!(v2["latest"]["messageId"], s2["assistantMessageId"]);
}

#[tokio::test]
async fn a_summary_sends_at_most_seventy_and_says_it_was_random() {
    let e = env().await;
    e.t.add_record(wrong_record(&e.user, 43, 100));
    let ev = ask_to(&e.t, &e.user, SUMMARY_ASK, sum_body(43)).await;
    let s = start_of(&ev);
    assert_eq!(
        s["sample"],
        json!({"used": 70, "usable": 100, "wrong": 100})
    );
    assert!(text(&ev).contains("Items: 70"));
    let sys = e.t.stub.last().body["messages"][0]["content"]
        .as_str()
        .expect("system")
        .to_string();
    assert!(sys.contains("70 of their 100 wrong answers, chosen at random"));
}

#[tokio::test]
async fn the_summary_refusals() {
    let e = env().await;
    // no per-question records
    let mut r = wrong_record(&e.user, 50, 2);
    r.detail = false;
    e.t.add_record(r);
    let (s, v) = refused(&e.t, &e.user, SUMMARY_ASK, sum_body(50)).await;
    assert_eq!((s, &v["code"]), (409, &json!("ai.noDetail")));
    assert_eq!(v["error"], "这条记录没有逐题记录，无法总结。");
    // nothing answered wrong (a correct and an unanswered item)
    e.t.add_record(record(
        &e.user,
        51,
        "practice",
        Some(K1),
        "2026-11-11 20:00",
        vec![
            item("q1", json!(["B"]), true),
            item("q2", Value::Null, false),
        ],
    ));
    let (s, v) = refused(&e.t, &e.user, SUMMARY_ASK, sum_body(51)).await;
    assert_eq!((s, &v["code"]), (409, &json!("ai.noWrong")));
    assert_eq!(v["error"], "这次没有答错的题（未作答的题不计入 AI 总结）。");
    // wrong answers whose questions are gone
    e.t.add_record(record(
        &e.user,
        52,
        "practice",
        Some(K3),
        "2026-11-11 20:10",
        vec![
            item("gone-1", json!(["A"]), false),
            item("gone-2", json!(["A"]), false),
            item("gone-3", json!(["A"]), false),
        ],
    ));
    let (s, v) = refused(&e.t, &e.user, SUMMARY_ASK, sum_body(52)).await;
    assert_eq!((s, &v["code"]), (409, &json!("ai.noUsable")));
    assert_eq!(
        v["error"],
        "这些答错的题已经不可用（题库已删除或没有访问权限），无法总结。"
    );
    // a missing record
    let (s, v) = refused(&e.t, &e.user, SUMMARY_ASK, sum_body(999)).await;
    assert_eq!((s, &v["code"]), (404, &json!("ai.notFound")));
    // nothing was created by any refusal
    let n: i64 = q1(&e.t, "SELECT COUNT(*) FROM chats");
    assert_eq!(n, 0);
    // bad bodies
    for b in [
        json!({}),
        json!({"recordId": 0}),
        json!({"recordId": "1"}),
        json!({"recordId": 1, "x": 1}),
        json!({"recordId": 1, "lang": "fr"}),
    ] {
        let (s, v) = refused(&e.t, &e.user, SUMMARY_ASK, b.clone()).await;
        assert_eq!((s, &v["code"]), (400, &json!("ai.badRequest")), "{b}");
    }
}

#[tokio::test]
async fn the_summary_chat_stays_when_the_record_is_gone() {
    let e = env().await;
    e.t.add_record(wrong_record(&e.user, 42, 2));
    let s = start_of(&ask_to(&e.t, &e.user, SUMMARY_ASK, sum_body(42)).await);
    // the record is deleted in the main app
    e.t.backend.st().records.remove(&42);
    let (st, v) =
        e.t.get("/api/ai/summary/saved?record=42&at=202611112158", &e.user)
            .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["chatId"], s["chatId"]);
    assert_eq!(v["latest"]["status"], "done");
    let (_, chat) =
        e.t.get(&format!("/api/ai/chat?id={}", s["chatId"]), &e.user)
            .await;
    assert_eq!(chat["chat"]["kind"], "summary");
    assert_eq!(chat["messages"][0]["sample"]["wrong"], 2);
}
