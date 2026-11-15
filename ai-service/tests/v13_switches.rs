//! v1.3 global feature switches while a call is running: turning a feature off ends the running calls of normal users
//! (they are not completed), while the calls of administrators go on to their end (the owner's rule: administrators are
//! not affected by the switches). Made-up texts only. The tests run with `TZ=UTC`.
mod common;

#[path = "study_api/util.rs"]
mod util;

use serde_json::json;
use util::*;

fn slow_requests(t: &TestApp) -> usize {
    t.stub
        .requests()
        .iter()
        .filter(|r| r.body["messages"].to_string().contains("[slow]"))
        .count()
}

type Events = Vec<(String, serde_json::Value)>;

async fn two_slow_hints(e: &Env) -> (Events, Events, i64, i64) {
    let mut user_stream =
        e.t.open_stream(STUDY_ASK, &e.user, hint_body(K1, "q-slow", 1))
            .await
            .expect("the user's stream");
    let mut admin_stream =
        e.t.open_stream(STUDY_ASK, &e.admin, hint_body(K2, "q-slow", 1))
            .await
            .expect("the administrator's stream");
    let (_, user_start) = user_stream.until("start").await;
    let (_, admin_start) = admin_stream.until("start").await;
    let user_msg = user_start["assistantMessageId"].as_i64().expect("message");
    let admin_msg = admin_start["assistantMessageId"].as_i64().expect("message");
    eventually(|| slow_requests(&e.t) >= 2).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let (s, v) =
        e.t.post(
            "/api/ai/admin/feature-switches",
            &e.admin,
            json!({"hint": false}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    e.t.stub.release();
    let user_events = user_stream.all().await;
    let admin_events = admin_stream.all().await;
    settle(&e.t).await;
    (user_events, admin_events, user_msg, admin_msg)
}

fn status(e: &Env, id: i64) -> String {
    e.t.db()
        .query_row("SELECT status FROM messages WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .expect("status")
}

#[tokio::test]
async fn switching_a_feature_off_does_not_touch_the_running_call_of_an_administrator() {
    let e = env().await;
    let (_, admin_events, _, admin_msg) = two_slow_hints(&e).await;
    assert_eq!(
        terminal_name(&admin_events),
        "done",
        "{:?}",
        names(&admin_events)
    );
    assert_eq!(status(&e, admin_msg), "done");
    // from now on the user is refused, the administrator is not
    let (s, v) = refused(&e.t, &e.user, STUDY_ASK, hint_body(K1, "q1", 2)).await;
    assert_eq!((s, v["code"].clone()), (403, json!("ai.featureOff")), "{v}");
    let ev = ask(&e.t, &e.admin, hint_body(K2, "q1", 2)).await;
    assert_eq!(terminal_name(&ev), "done");
}

#[tokio::test]
async fn switching_a_feature_off_ends_the_running_call_of_a_normal_user() {
    let e = env().await;
    let (user_events, _, user_msg, _) = two_slow_hints(&e).await;
    assert_ne!(
        terminal_name(&user_events),
        "done",
        "the user's call must end: {:?}",
        names(&user_events)
    );
    assert_ne!(status(&e, user_msg), "done");
    assert_ne!(status(&e, user_msg), "streaming", "nothing is left running");
}
