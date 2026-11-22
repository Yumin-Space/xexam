//! The v2.5 AI study assessment: the stream, the stored mapping (`summary` + `assess@` ref = wire kind `assess`), the
//! saved lookup, the refusals. All records and texts are made up.
use super::util::*;
use serde_json::{json, Value};

const ASK: &str = "/api/ai/assess/ask";

fn at_digits(at: &str) -> String {
    at.chars().filter(char::is_ascii_digit).collect()
}

/// Three records of the user (the minimum), the newest at 2026-11-21 10:30.
fn three_records(e: &Env) {
    for (id, kind, at) in [
        (1, "practice", "2026-11-19 08:00"),
        (2, "exam", "2026-11-20 19:15"),
        (3, "practice", "2026-11-21 10:30"),
    ] {
        e.t.add_record(record(
            &e.user,
            id,
            kind,
            None,
            at,
            vec![
                item("q1", json!(["B"]), true),
                item("q2", json!(["A"]), false),
            ],
        ));
    }
}

#[tokio::test]
async fn an_assessment_streams_and_lands_in_a_day_chat() {
    let e = env().await;
    three_records(&e);
    let ev = ask_to(&e.t, &e.user, ASK, json!({"lang": "en"})).await;
    assert_eq!(terminal_name(&ev), "done");
    let s = start_of(&ev);
    assert_eq!(s["chatKind"], "assess");
    assert_eq!(s["kind"], "assess");
    assert_eq!(s["recordId"], Value::Null);
    assert_eq!(s["sample"], Value::Null);
    assert_eq!(s["chatTitle"], "2026-11-22");
    assert_eq!(
        s["basis"],
        json!({"days": 30, "records": 3, "answered": 6, "latestAt": "2026-11-21 10:30"})
    );
    // stored as a summary chat with the day ref and no record; the user message is the machine line
    let (kind, rf, rec): (String, String, Option<i64>) =
        e.t.db()
            .query_row("SELECT kind, ref, record_id FROM chats", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .expect("chat");
    assert_eq!(
        (kind.as_str(), rf.as_str(), rec),
        ("summary", "assess@2026-11-22", None)
    );
    let line: String = q1(&e.t, "SELECT content FROM messages WHERE role = 'user'");
    assert_eq!(line, "[AI assessment] latest=2026-11-21 10:30 30d 3r 6q");
    let kinds: i64 = q1(&e.t, "SELECT COUNT(*) FROM messages WHERE kind = 'summary'");
    assert_eq!(kinds, 2);
    let purpose: Option<String> = q1(&e.t, "SELECT purpose FROM usage WHERE kind = 'chat'");
    assert_eq!(purpose.as_deref(), Some("summary"));
    // the prompt: numbers only, no question text
    let req = e.t.stub.last();
    let msgs = req.body["messages"].as_array().expect("messages");
    assert_eq!(msgs.len(), 2);
    let sys = msgs[0]["content"].as_str().expect("system");
    assert!(sys.contains("study coach inside Xexam"));
    let user = msgs[1]["content"].as_str().expect("user");
    assert!(user.starts_with("<stats>\n") && user.contains("answered: 6"));
    assert!(!user.contains("tool"));
    // the wire kind is `assess` in the list and in the chat
    let (_, list) = e.t.get("/api/ai/chats", &e.user).await;
    assert_eq!(list["chats"][0]["kind"], "assess");
    let (_, chat) =
        e.t.get(&format!("/api/ai/chat?id={}", s["chatId"]), &e.user)
            .await;
    assert_eq!(chat["chat"]["kind"], "assess");
    assert_eq!(chat["messages"][0]["kind"], "assess");
    assert_eq!(chat["messages"][1]["kind"], "assess");
    assert_eq!(chat["messages"][1]["recordId"], Value::Null);
    // the saved lookup: the newest record's time must match the stored line
    let url = format!("/api/ai/assess/saved?at={}", at_digits("2026-11-21 10:30"));
    let (st, v) = e.t.get(&url, &e.user).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["chatId"], s["chatId"]);
    assert_eq!(v["latest"]["status"], "done");
    assert_eq!(v["date"], "2026-11-22");
    assert_eq!(
        v["basis"],
        json!({"days": 30, "records": 3, "answered": 6, "latestAt": "2026-11-21 10:30"})
    );
    let (_, stale) =
        e.t.get(
            &format!("/api/ai/assess/saved?at={}", at_digits("2026-11-21 10:31")),
            &e.user,
        )
        .await;
    assert_eq!(
        stale,
        json!({"chatId": null, "chatTitle": null, "latest": null, "lastGood": null, "date": null, "basis": null})
    );
    for bad in [
        "/api/ai/assess/saved",
        "/api/ai/assess/saved?at=2026-11-21-10:30",
        "/api/ai/assess/saved?at=20261121103",
    ] {
        let (s, v) = e.t.get(bad, &e.user).await;
        assert_eq!((s, &v["code"]), (400, &json!("ai.badRequest")), "{bad}");
    }
    // the record summary route never sees this chat
    let (_, rs) =
        e.t.get("/api/ai/summary/saved?record=3&at=202611211030", &e.user)
            .await;
    assert_eq!(rs["chatId"], Value::Null);
    // another user has nothing saved
    let other = e.t.user("kim", false, true);
    let (_, none) = e.t.get(&url, &other).await;
    assert_eq!(none["chatId"], Value::Null);
}

#[tokio::test]
async fn the_same_day_appends_and_a_new_day_starts_a_chat() {
    let e = env().await;
    three_records(&e);
    let s1 = start_of(&ask_to(&e.t, &e.user, ASK, json!({})).await);
    let s2 = start_of(&ask_to(&e.t, &e.user, ASK, json!({})).await);
    assert_eq!(s2["chatId"], s1["chatId"]);
    assert_eq!(s2["newChat"], false);
    let n: i64 = q1(&e.t, "SELECT COUNT(*) FROM messages");
    assert_eq!(n, 4);
    e.t.backend.st().assess_today = "2026-11-23".to_string();
    let s3 = start_of(&ask_to(&e.t, &e.user, ASK, json!({"lang": "ja"})).await);
    assert_ne!(s3["chatId"], s1["chatId"]);
    assert_eq!(s3["newChat"], true);
    assert_eq!(s3["lang"], "ja");
    // the newest matching attempt decides; its line still matches
    let url = format!("/api/ai/assess/saved?at={}", at_digits("2026-11-21 10:30"));
    let (_, v) = e.t.get(&url, &e.user).await;
    assert_eq!(v["chatId"], s3["chatId"]);
    assert_eq!(v["date"], "2026-11-23");
    // a follow-up that touches the older chat does not move the lookup there
    e.t.db()
        .execute(
            "UPDATE chats SET updated_at = updated_at + 86400 WHERE id = ?1",
            [s1["chatId"].as_i64().expect("id")],
        )
        .expect("touch");
    let (_, v) = e.t.get(&url, &e.user).await;
    assert_eq!(v["chatId"], s3["chatId"]);
    assert_eq!(v["latest"]["messageId"], s3["assistantMessageId"]);
    // a stopped newest attempt is returned as it is (never asked again on its own), beside the last good one
    e.t.db()
        .execute(
            "UPDATE messages SET status = 'stopped' WHERE id = ?1",
            [s3["assistantMessageId"].as_i64().expect("id")],
        )
        .expect("stop");
    let (_, v) = e.t.get(&url, &e.user).await;
    assert_eq!(v["chatId"], s3["chatId"]);
    assert_eq!(v["latest"]["status"], "stopped");
    assert_eq!(v["lastGood"]["messageId"], s2["assistantMessageId"]);
    assert_eq!(v["date"], "2026-11-23");
}

#[tokio::test]
async fn too_few_records_and_other_refusals() {
    let e = env().await;
    e.t.add_record(record(
        &e.user,
        1,
        "practice",
        None,
        "2026-11-19 08:00",
        vec![item("q1", json!(["B"]), true)],
    ));
    let (s, v) = refused(&e.t, &e.user, ASK, json!({})).await;
    assert_eq!((s, &v["code"]), (409, &json!("ai.assessFew")));
    assert_eq!(v["error"], "学习记录太少，至少需要 3 条。");
    let n: i64 = q1(&e.t, "SELECT COUNT(*) FROM chats");
    assert_eq!(n, 0);
    // bad bodies
    for b in [
        json!({"x": 1}),
        json!({"lang": "fr"}),
        json!({"lang": 1}),
        json!({"recordId": 1}),
    ] {
        let (s, v) = refused(&e.t, &e.user, ASK, b.clone()).await;
        assert_eq!((s, &v["code"]), (400, &json!("ai.badRequest")), "{b}");
    }
    // a user without AI+
    let plain = e.t.user("ann", false, false);
    let (s, v) = refused(&e.t, &plain, ASK, json!({})).await;
    assert_eq!((s, &v["code"]), (403, &json!("ai.notAllowed")));
}

#[tokio::test]
async fn an_unreadable_backend_answer_fails_only_this_request() {
    let e = env().await;
    three_records(&e);
    e.t.backend.st().study_switch = StudySwitch::ExtraField;
    let (s, v) = refused(&e.t, &e.user, ASK, json!({})).await;
    assert_eq!(s, 503, "{v}");
    e.t.backend.st().study_switch = StudySwitch::Normal;
    assert_eq!(
        terminal_name(&ask_to(&e.t, &e.user, ASK, json!({})).await),
        "done"
    );
}

#[tokio::test]
async fn an_ordinary_user_never_sees_the_model() {
    let e = env().await;
    three_records(&e);
    let ev = ask_to(&e.t, &e.user, ASK, json!({})).await;
    assert!(start_of(&ev).get("model").is_none());
    let url = format!("/api/ai/assess/saved?at={}", at_digits("2026-11-21 10:30"));
    let (_, v) = e.t.get(&url, &e.user).await;
    assert!(!v.to_string().contains("\"model\""), "{v}");
}
