//! Which chat an ask belongs to: the session key, the backend's record, the legacy chat of a record without a key; the
//! answers of the backend's data routes and what they do (and do not do) to the link.
use super::util::*;
use serde_json::{json, Value};
use xexam_ai_service::util::local_minute;

#[tokio::test]
async fn titles_come_from_the_start_time_and_an_absurd_one_becomes_now() {
    let e = env().await;
    let started = now() - 7200;
    let mut b = hint_body(K1, "q1", 1);
    b["startedAt"] = json!(started);
    let s = start_of(&ask(&e.t, &e.user, b).await);
    assert_eq!(s["chatTitle"], local_minute(started));
    // later than now + 300 s, and older than 400 days: now is used instead
    for (key, at) in [(K2, now() + 100_000), (K3, now() - 401 * 86_400)] {
        let mut b = hint_body(key, "q1", 1);
        b["startedAt"] = json!(at);
        let s = start_of(&ask(&e.t, &e.user, b).await);
        let shown = s["chatTitle"].as_str().expect("title").to_string();
        let ok = [now(), now() - 1, now() - 2, now() - 3]
            .iter()
            .any(|t| local_minute(*t) == shown);
        assert!(ok, "{shown}");
    }
}

#[tokio::test]
async fn an_exam_result_ask_lands_in_the_exam_chat_of_its_session() {
    let e = env().await;
    e.t.add_record(record(
        &e.user,
        7,
        "exam",
        Some(K2),
        "2026-11-11 21:58",
        vec![
            item("q1", json!(["A"]), false),
            item("q2", json!(["B"]), true),
        ],
    ));
    let ev = ask(&e.t, &e.user, result_body("exam", 7, "q1", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    let s = start_of(&ev);
    assert_eq!(s["chatKind"], "exam");
    assert_eq!(s["recordId"], 7);
    assert_eq!(s["source"], "result");
    let (kind, rf, rec): (String, String, Option<i64>) =
        e.t.db()
            .query_row("SELECT kind, ref, record_id FROM chats", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .expect("chat");
    assert_eq!((kind.as_str(), rf.as_str(), rec), ("exam", K2, Some(7)));
    // the result page discusses the user's answer
    let req = e.t.stub.last();
    let sys = req.body["messages"][0]["content"].as_str().expect("system");
    assert!(sys.contains("The user answered A."));
    assert!(req.body["messages"][1]["content"]
        .as_str()
        .expect("user")
        .contains("<user_answer>A</user_answer>"));
    // the message carries the record id (R5) and the source result
    let (_, v) =
        e.t.get(&format!("/api/ai/chat?id={}", s["chatId"]), &e.user)
            .await;
    assert_eq!(v["messages"][1]["recordId"], 7);
    assert_eq!(v["chat"]["recordId"], 7);
    // the title is the record's start time
    let started = e.t.backend.st().records[&7].started_at.expect("started");
    assert_eq!(s["chatTitle"], local_minute(started));
    // a second question of the same exam: the same chat
    let s2 = start_of(&ask(&e.t, &e.user, result_body("exam", 7, "q2", 2)).await);
    assert_eq!(s2["chatId"], s["chatId"]);
}

#[tokio::test]
async fn an_unanswered_item_is_explained_without_an_answer_to_discuss() {
    let e = env().await;
    e.t.add_record(record(
        &e.user,
        8,
        "practice",
        Some(K1),
        "2026-11-11 21:58",
        vec![item("q1", Value::Null, false)],
    ));
    let ev = ask(&e.t, &e.user, result_body("practice", 8, "q1", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    let req = e.t.stub.last();
    assert!(req.body["messages"][0]["content"]
        .as_str()
        .expect("system")
        .contains("The user did not answer this question."));
    // the record is a practice record: the detail page shares the chat of the practice session
    let practice = start_of(&ask(&e.t, &e.user, hint_body(K1, "q2", 1)).await);
    assert_eq!(practice["chatId"], start_of(&ev)["chatId"]);
}

#[tokio::test]
async fn a_key_that_is_not_the_records_key_is_not_found() {
    let e = env().await;
    e.t.add_record(record(
        &e.user,
        7,
        "exam",
        Some(K2),
        "2026-11-11 21:58",
        vec![item("q1", json!(["A"]), false)],
    ));
    let mut b = result_body("exam", 7, "q1", 1);
    b["key"] = json!(K1);
    b["startedAt"] = json!(now() - 100);
    let (s, v) = refused(&e.t, &e.user, STUDY_ASK, b).await;
    assert_eq!((s, &v["code"]), (404, &json!("ai.notFound")));
    // the same key is accepted
    let mut b = result_body("exam", 7, "q1", 1);
    b["key"] = json!(K2);
    assert_eq!(terminal_name(&ask(&e.t, &e.user, b).await), "done");
}

#[tokio::test]
async fn a_record_without_a_key_gets_its_own_legacy_chat() {
    let e = env().await;
    e.t.add_record(record(
        &e.user,
        9,
        "practice",
        None,
        "2026-10-01 08:30",
        vec![item("q1", json!(["A"]), false)],
    ));
    e.t.add_record(record(
        &e.user,
        10,
        "exam",
        None,
        "2026-10-02 09:45",
        vec![item("q1", json!(["A"]), false)],
    ));
    let a = start_of(&ask(&e.t, &e.user, result_body("practice", 9, "q1", 1)).await);
    let b = start_of(&ask(&e.t, &e.user, result_body("exam", 10, "q1", 1)).await);
    assert_eq!(a["chatKind"], "practice");
    assert_eq!(b["chatKind"], "exam");
    assert_eq!(a["chatTitle"], "2026-10-01 08:30");
    assert_eq!(b["chatTitle"], "2026-10-02 09:45");
    let rows: Vec<(String, String)> =
        e.t.db()
            .prepare("SELECT kind, ref FROM chats ORDER BY id")
            .expect("prepare")
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("query")
            .map(|x| x.expect("row"))
            .collect();
    assert_eq!(
        rows,
        vec![
            ("practice".to_string(), "r9@2026-10-01 08:30".to_string()),
            ("exam".to_string(), "r10@2026-10-02 09:45".to_string())
        ]
    );
    // asking again lands in the same chat
    let again = start_of(&ask(&e.t, &e.user, result_body("practice", 9, "q1", 1)).await);
    assert_eq!(again["chatId"], a["chatId"]);
}

#[tokio::test]
async fn a_foreign_or_missing_record_or_question_is_not_found() {
    let e = env().await;
    let other = e.t.user("kim", false, true);
    e.t.add_record(record(
        &other,
        20,
        "exam",
        Some(K2),
        "2026-11-11 21:58",
        vec![item("q1", json!(["A"]), false)],
    ));
    for body in [
        result_body("exam", 20, "q1", 1),
        result_body("exam", 99, "q1", 1),
        result_body("practice", 20, "q1", 1),
    ] {
        let (s, v) = refused(&e.t, &e.user, STUDY_ASK, body).await;
        assert_eq!((s, &v["code"]), (404, &json!("ai.notFound")));
    }
    // an unknown question
    let (s, v) = refused(&e.t, &e.user, STUDY_ASK, hint_body(K1, "nope", 1)).await;
    assert_eq!((s, &v["code"]), (404, &json!("ai.notFound")));
    let n: i64 = q1(&e.t, "SELECT COUNT(*) FROM chats");
    assert_eq!(n, 0);
}

#[tokio::test]
async fn the_backends_refusals_become_api_errors() {
    let e = env().await;
    e.t.backend.st().study_switch = StudySwitch::Session;
    let (s, v) = refused(&e.t, &e.user, STUDY_ASK, hint_body(K1, "q1", 1)).await;
    assert_eq!((s, &v["code"]), (403, &json!("ai.notAllowed")));
    e.t.backend.st().study_switch = StudySwitch::NotFound;
    let (s, v) = refused(&e.t, &e.user, STUDY_ASK, hint_body(K1, "q1", 1)).await;
    assert_eq!((s, &v["code"]), (404, &json!("ai.notFound")));
    let n: i64 = q1(&e.t, "SELECT COUNT(*) FROM chats");
    assert_eq!(n, 0);
}

#[tokio::test]
async fn a_bad_answer_of_a_data_route_fails_only_that_request() {
    let e = env().await;
    assert!(e.t.app.0.link.is_connected());
    for sw in [
        StudySwitch::Http404,
        StudySwitch::Unreadable,
        StudySwitch::ExtraField,
    ] {
        e.t.backend.st().study_switch = sw;
        let (s, v) = refused(&e.t, &e.user, STUDY_ASK, hint_body(K1, "q1", 1)).await;
        assert_eq!((s, &v["code"]), (503, &json!("ai.backendDown")), "{sw:?}");
        // the link is not lost, AI stays on for everybody
        assert!(e.t.app.0.link.is_connected(), "{sw:?}");
        let (st, status) = e.t.get("/api/ai/status", &e.user).await;
        assert_eq!(st, 200);
        assert_eq!(status["enabled"], true, "{sw:?}");
        assert_eq!(status["allowed"], true, "{sw:?}");
    }
    assert!(captured_contains(
        "AI link: an answer of /internal/ai/question could not be read"
    ));
    // the same user is served as soon as the backend answers well
    e.t.backend.st().study_switch = StudySwitch::Normal;
    assert_eq!(
        terminal_name(&ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await),
        "done"
    );
    let n: i64 = q1(&e.t, "SELECT COUNT(*) FROM chats");
    assert_eq!(n, 1);
}

#[tokio::test]
async fn a_good_data_answer_does_not_make_the_link_connected() {
    let mut e = env().await;
    // a service that has not talked to the backend yet: a good data answer is no "connected" event
    e.t.restart_without_report(Opts::default());
    assert!(!e.t.app.0.link.is_connected());
    e.t.add_record(record(
        &e.user,
        7,
        "exam",
        Some(K2),
        "2026-11-11 21:58",
        vec![item("q1", json!(["A"]), false)],
    ));
    let (s, v) =
        e.t.get(
            "/api/ai/study/saved?mode=exam&record=7&source=result&question=q1&kind=explain",
            &e.user,
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(e.t.backend_calls("/internal/ai/record-info"), 1);
    assert!(!e.t.app.0.link.is_connected());
}
