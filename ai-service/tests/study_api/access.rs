//! Access: the ask routes need the AI+ feature and a working AI service; the exam never gets any AI; a running answer
//! stops after a pause.
use super::util::*;
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn the_ask_routes_need_ai_on_the_feature_and_no_pause() {
    let e = env().await;
    // no AI+ feature
    let plain = e.t.user("plain", false, false);
    for path in [STUDY_ASK, SUMMARY_ASK] {
        let body = if path == STUDY_ASK {
            hint_body(K1, "q1", 1)
        } else {
            json!({"recordId": 1})
        };
        let (st, v) = refused(&e.t, &plain, path, body).await;
        assert_eq!((st, &v["code"]), (403, &json!("ai.notAllowed")), "{path}");
    }
    // the saved routes are Signed: an empty answer, not a refusal
    let (st, _) =
        e.t.get(
            &format!(
                "/api/ai/study/saved?mode=practice&key={K1}&source=practice&question=q1&kind=hint"
            ),
            &plain,
        )
        .await;
    assert_eq!(st, 200);
    // paused by an administrator
    let (st, v) =
        e.t.post(
            "/api/ai/admin/suspend",
            &e.admin,
            json!({"userId": e.user.id}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let (st, v) = refused(&e.t, &e.user, STUDY_ASK, hint_body(K1, "q1", 1)).await;
    assert_eq!((st, &v["code"]), (403, &json!("ai.suspended")));
    let (st, v) =
        e.t.post(
            "/api/ai/admin/resume",
            &e.admin,
            json!({"userId": e.user.id}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(
        terminal_name(&ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await),
        "done"
    );
    // AI switched off
    let (st, v) =
        e.t.post("/api/ai/admin/enable", &e.admin, json!({"enabled": false}))
            .await;
    assert_eq!(st, 200, "{v}");
    let (st, v) = refused(&e.t, &e.user, STUDY_ASK, hint_body(K1, "q1", 1)).await;
    assert_eq!((st, &v["code"]), (403, &json!("ai.disabled")));
    // administrators always have the feature
    let (st, v) = refused(&e.t, &e.admin, STUDY_ASK, hint_body(K1, "q1", 1)).await;
    assert_eq!((st, &v["code"]), (403, &json!("ai.disabled")));
}

#[tokio::test]
async fn an_administrator_may_ask() {
    let e = env().await;
    assert_eq!(
        terminal_name(&ask(&e.t, &e.admin, explain_body(K1, "q1", 1)).await),
        "done"
    );
}

#[tokio::test]
async fn the_backends_not_allowed_is_a_403_too() {
    let e = env().await;
    // the identity statement still says AI+, but the backend no longer does
    e.t.set_ai(&e.user, false);
    let (st, v) = refused(&e.t, &e.user, STUDY_ASK, hint_body(K1, "q1", 1)).await;
    assert_eq!((st, &v["code"]), (403, &json!("ai.notAllowed")));
    let n: i64 = q1(&e.t, "SELECT COUNT(*) FROM chats");
    assert_eq!(n, 0);
}

#[tokio::test]
async fn a_running_hint_stops_within_a_check_after_a_pause_and_keeps_nothing_unchecked() {
    let e = env().await;
    let mut s =
        e.t.open_stream(STUDY_ASK, &e.user, hint_body(K1, "q-leakslow", 1))
            .await
            .expect("stream");
    let (_, start) = s.until("start").await;
    let msg = start["assistantMessageId"].as_i64().expect("message");
    // the stub has sent the leaking sentence and holds: the guard holds it back
    stub_waiting(&e.t).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let (st, v) =
        e.t.post(
            "/api/ai/admin/suspend",
            &e.admin,
            json!({"userId": e.user.id}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let rest = tokio::time::timeout(Duration::from_secs(10), s.all())
        .await
        .expect("ended");
    let (name, data) = terminal(&rest);
    // the held-back leak is a leak, whatever ended the answer
    assert_eq!(
        (name.as_str(), &data["code"]),
        ("error", &json!("ai.hintLeak"))
    );
    let content: String =
        e.t.db()
            .query_row("SELECT content FROM messages WHERE id = ?1", [msg], |r| {
                r.get(0)
            })
            .expect("content");
    assert_eq!(content, "");
    e.t.stub.release();
}

#[tokio::test]
async fn a_running_explanation_ends_with_the_pause_code() {
    let e = env().await;
    let mut s =
        e.t.open_stream(STUDY_ASK, &e.user, explain_body(K1, "q-slow", 1))
            .await
            .expect("stream");
    let _ = s.until("start").await;
    let (st, v) =
        e.t.post(
            "/api/ai/admin/suspend",
            &e.admin,
            json!({"userId": e.user.id}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let rest = tokio::time::timeout(Duration::from_secs(10), s.all())
        .await
        .expect("ended");
    let (name, data) = terminal(&rest);
    assert_eq!(
        (name.as_str(), &data["code"]),
        ("error", &json!("ai.suspended"))
    );
    e.t.stub.release();
}
