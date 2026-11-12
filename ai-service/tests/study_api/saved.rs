//! The saved lookups: newest answer per (kind, question, place), last good answer, another user's key, the record
//! route and its identity cache, query validation.
use super::util::*;
use serde_json::{json, Value};

fn practice_q(key: &str, question: &str, kind: &str, source: &str) -> String {
    format!("/api/ai/study/saved?mode=practice&key={key}&source={source}&question={question}&kind={kind}")
}

#[tokio::test]
async fn nothing_saved_is_a_normal_empty_answer() {
    let e = env().await;
    let (s, v) =
        e.t.get(&practice_q(K1, "q1", "hint", "practice"), &e.user)
            .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        v,
        json!({"chatId": null, "chatTitle": null, "latest": null, "lastGood": null})
    );
}

#[tokio::test]
async fn the_newest_answer_wins_and_the_places_are_kept_apart() {
    let e = env().await;
    let first = start_of(&ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await);
    let second = start_of(&ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await);
    assert_eq!(first["chatId"], second["chatId"]);
    let (_, v) =
        e.t.get(&practice_q(K1, "q1", "hint", "practice"), &e.user)
            .await;
    assert_eq!(v["chatId"], first["chatId"]);
    // the saved route returns the chat's real title (the same one the start event named)
    assert!(first["chatTitle"].as_str().is_some_and(|t| !t.is_empty()));
    assert_eq!(v["chatTitle"], first["chatTitle"]);
    assert_eq!(v["latest"]["messageId"], second["assistantMessageId"]);
    assert_eq!(v["latest"]["status"], "done");
    assert_eq!(v["latest"]["content"], CLEAN_HINT);
    assert_eq!(v["latest"]["kind"], "hint");
    assert_eq!(v["latest"]["questionId"], "q1");
    assert_eq!(v["latest"]["source"], "practice");
    assert_eq!(v["latest"]["lang"], "en");
    // the newest is done: nothing is sent twice
    assert_eq!(v["lastGood"], Value::Null);
    // an explanation of the same question is another key; the practice and the result place are kept apart
    let _ = ask(&e.t, &e.user, explain_body(K1, "q1", 1)).await;
    let (_, ex) =
        e.t.get(&practice_q(K1, "q1", "explain", "practice"), &e.user)
            .await;
    assert_eq!(ex["latest"]["kind"], "explain");
    let (_, res) =
        e.t.get(&practice_q(K1, "q1", "explain", "result"), &e.user)
            .await;
    assert_eq!(res["latest"], Value::Null);
    assert_eq!(res["chatId"], first["chatId"]);
    let mut b = explain_body(K1, "q1", 1);
    b["source"] = json!("result");
    let _ = ask(&e.t, &e.user, b).await;
    let (_, res) =
        e.t.get(&practice_q(K1, "q1", "explain", "result"), &e.user)
            .await;
    assert_eq!(res["latest"]["source"], "result");
    // the practice one is unchanged
    let (_, ex2) =
        e.t.get(&practice_q(K1, "q1", "explain", "practice"), &e.user)
            .await;
    assert_eq!(ex2["latest"]["messageId"], ex["latest"]["messageId"]);
}

#[tokio::test]
async fn last_good_is_the_previous_done_answer_when_the_newest_is_not_done() {
    let e = env().await;
    // a good answer of q-err is not possible with the stub (it always fails), so write the first one by hand: ask a
    // clean question, then rename its question id in the database
    let a = start_of(&ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await);
    e.t.db()
        .execute("UPDATE messages SET question_id = 'q-err'", [])
        .expect("update");
    let b = start_of(&ask(&e.t, &e.user, hint_body(K1, "q-err", 1)).await);
    assert_eq!(a["chatId"], b["chatId"]);
    let (_, v) =
        e.t.get(&practice_q(K1, "q-err", "hint", "practice"), &e.user)
            .await;
    assert_eq!(v["latest"]["status"], "error");
    assert_eq!(v["latest"]["messageId"], b["assistantMessageId"]);
    assert_eq!(v["lastGood"]["status"], "done");
    assert_eq!(v["lastGood"]["messageId"], a["assistantMessageId"]);
    assert_eq!(v["lastGood"]["content"], CLEAN_HINT);
}

#[tokio::test]
async fn another_users_key_finds_nothing_and_the_lookup_works_with_ai_off() {
    let e = env().await;
    let _ = ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await;
    let other = e.t.user("kim", false, true);
    let (s, v) =
        e.t.get(&practice_q(K1, "q1", "hint", "practice"), &other)
            .await;
    assert_eq!(s, 200);
    assert_eq!(
        v,
        json!({"chatId": null, "chatTitle": null, "latest": null, "lastGood": null})
    );
    // AI switched off: a Signed route still answers
    let (s, v) =
        e.t.post("/api/ai/admin/enable", &e.admin, json!({"enabled": false}))
            .await;
    assert_eq!(s, 200, "{v}");
    let (s, v) =
        e.t.get(&practice_q(K1, "q1", "hint", "practice"), &e.user)
            .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["latest"]["status"], "done");
}

#[tokio::test]
async fn the_saved_answer_of_a_result_page_is_found_by_record_and_by_key() {
    let e = env().await;
    e.t.add_record(record(
        &e.user,
        7,
        "exam",
        Some(K2),
        "2026-11-11 21:58",
        vec![item("q1", json!(["A"]), false)],
    ));
    let s = start_of(&ask(&e.t, &e.user, result_body("exam", 7, "q1", 1)).await);
    // the detail page asks by record id; the result page by key
    let by_record = "/api/ai/study/saved?mode=exam&record=7&source=result&question=q1&kind=explain";
    let by_key =
        format!("/api/ai/study/saved?mode=exam&key={K2}&source=result&question=q1&kind=explain");
    let (_, a) = e.t.get(by_record, &e.user).await;
    let (_, b) = e.t.get(&by_key, &e.user).await;
    assert_eq!(a["chatId"], s["chatId"]);
    assert_eq!(a, b);
    assert_eq!(a["latest"]["status"], "done");
    // the ask put the record in the identity cache: no call to record-info at all
    assert_eq!(e.t.backend_calls("/internal/ai/record-info"), 0);
    // a record of the wrong kind finds nothing
    let (_, c) =
        e.t.get(
            "/api/ai/study/saved?mode=practice&record=7&source=result&question=q1&kind=explain",
            &e.user,
        )
        .await;
    assert_eq!(
        c,
        json!({"chatId": null, "chatTitle": null, "latest": null, "lastGood": null})
    );
}

#[tokio::test]
async fn the_record_identity_is_cached_and_not_found_is_not() {
    let e = env().await;
    e.t.add_record(record(
        &e.user,
        7,
        "practice",
        Some(K1),
        "2026-11-11 21:58",
        vec![item("q1", json!(["A"]), false)],
    ));
    let _ = ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await;
    let q = "/api/ai/study/saved?mode=practice&record=7&source=result&question=q1&kind=explain";
    for _ in 0..3 {
        let (s, _) = e.t.get(q, &e.user).await;
        assert_eq!(s, 200);
    }
    assert_eq!(e.t.backend_calls("/internal/ai/record-info"), 1);
    // a record that is not found is asked again every time
    let nf = "/api/ai/study/saved?mode=practice&record=99&source=result&question=q1&kind=explain";
    for _ in 0..2 {
        let (s, v) = e.t.get(nf, &e.user).await;
        assert_eq!((s, &v["latest"]), (200, &Value::Null));
    }
    assert_eq!(e.t.backend_calls("/internal/ai/record-info"), 3);
    // a backend that cannot be read: 503, nothing cached
    e.t.backend.st().study_switch = StudySwitch::Unreadable;
    let other = e.t.user("kim", false, true);
    let (s, v) = e.t.get(q, &other).await;
    assert_eq!((s, &v["code"]), (503, &json!("ai.backendDown")));
}

#[tokio::test]
async fn a_pre_v09_record_is_found_in_its_legacy_chat() {
    let e = env().await;
    e.t.add_record(record(
        &e.user,
        9,
        "practice",
        None,
        "2026-10-01 08:30",
        vec![item("q1", json!(["A"]), false)],
    ));
    let s = start_of(&ask(&e.t, &e.user, result_body("practice", 9, "q1", 1)).await);
    let (_, v) =
        e.t.get(
            "/api/ai/study/saved?mode=practice&record=9&source=result&question=q1&kind=explain",
            &e.user,
        )
        .await;
    assert_eq!(v["chatId"], s["chatId"]);
    assert_eq!(v["latest"]["status"], "done");
}

#[tokio::test]
async fn saved_queries_are_validated() {
    let e = env().await;
    let ok = practice_q(K1, "q1", "hint", "practice");
    assert_eq!(e.t.get(&ok, &e.user).await.0, 200);
    for bad in [
        // no mode / source / question / kind
        format!("/api/ai/study/saved?key={K1}&source=practice&question=q1&kind=hint"),
        format!("/api/ai/study/saved?mode=practice&key={K1}&question=q1&kind=hint"),
        format!("/api/ai/study/saved?mode=practice&key={K1}&source=practice&kind=hint"),
        format!("/api/ai/study/saved?mode=practice&key={K1}&source=practice&question=q1"),
        // neither / both of key and record
        "/api/ai/study/saved?mode=practice&source=practice&question=q1&kind=hint".to_string(),
        format!("/api/ai/study/saved?mode=practice&key={K1}&record=7&source=result&question=q1&kind=explain"),
        // bad values
        "/api/ai/study/saved?mode=practice&key=ABC&source=practice&question=q1&kind=hint".to_string(),
        format!("/api/ai/study/saved?mode=practice&key={K1}&source=practice&question=q%201&kind=hint"),
        format!("/api/ai/study/saved?mode=practice&key={K1}&source=practice&question=q1&kind=other"),
        "/api/ai/study/saved?mode=practice&record=0&source=result&question=q1&kind=explain".to_string(),
        // combinations: a hint is practice + practice; practice source needs practice mode and a key; an exam is a
        // result; a record is a result
        format!("/api/ai/study/saved?mode=practice&key={K1}&source=result&question=q1&kind=hint"),
        format!("/api/ai/study/saved?mode=exam&key={K1}&source=practice&question=q1&kind=explain"),
        "/api/ai/study/saved?mode=practice&record=7&source=practice&question=q1&kind=explain".to_string(),
        // a repeated name
        format!("/api/ai/study/saved?mode=practice&mode=exam&key={K1}&source=practice&question=q1&kind=hint"),
    ] {
        let (s, v) = e.t.get(&bad, &e.user).await;
        assert_eq!((s, &v["code"]), (400, &json!("ai.badRequest")), "{bad}");
    }
    // an unknown name is ignored
    assert_eq!(e.t.get(&format!("{ok}&x=1"), &e.user).await.0, 200);
}
