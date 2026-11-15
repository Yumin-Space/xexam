//! v1.3 sprint 2 (owner): study frames and saved answers name no model to a user; an administrator keeps it.
use super::util::*;
use serde_json::{json, Value};

const MODEL: &str = "demo-chat-small";
const CONN: &str = "main";

/// Nothing in `raw` names the AI: no hidden key and none of the stub's names.
fn assert_hidden(raw: &str, model: &str, conn: &str) {
    for k in [
        "\"model\"",
        "\"modelFallback\"",
        "\"connection\"",
        "\"connectionId\"",
        "\"connectionName\"",
        "\"pick\"",
        "\"chatDefault\"",
        "\"provider\"",
        "\"baseUrl\"",
        "\"host\"",
    ] {
        assert!(!raw.contains(k), "{k} in {raw}");
    }
    for v in [model, &format!("\"{conn}\""), "127.0.0.1"] {
        assert!(!raw.contains(v), "{v} in {raw}");
    }
}

fn hidden_json(v: &Value) {
    assert_hidden(&serde_json::to_string(v).expect("json"), MODEL, CONN);
}

/// The raw SSE text and the events of one ask.
async fn raw_ask(
    t: &TestApp,
    who: &Who,
    path: &str,
    body: Value,
) -> (String, Vec<(String, Value)>) {
    let mut s = t
        .open_stream(path, who, body)
        .await
        .unwrap_or_else(|(s, v)| panic!("expected a stream, got {s} {v}"));
    let ev = s.all().await;
    let raw = s.raw.clone();
    settle(t).await;
    (raw, ev)
}

fn saved_url() -> String {
    format!("/api/ai/study/saved?mode=practice&key={K1}&source=practice&question=q1&kind=hint")
}

#[tokio::test]
async fn study_frames_and_saved_answers_hide_the_model_from_users() {
    let e = env().await;
    // a hint in practice
    let (raw, ev) = raw_ask(&e.t, &e.user, STUDY_ASK, hint_body(K1, "q1", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    assert_hidden(&raw, MODEL, CONN);
    // an explanation on a result page
    let mut items = vec![
        item("q1", json!(["A"]), false),
        item("q2", Value::Null, false),
    ];
    items.extend((0..3).map(|_| item("q1", json!(["A"]), false)));
    e.t.add_record(record(
        &e.user,
        7,
        "exam",
        Some(K2),
        "2026-11-11 21:58",
        items,
    ));
    let (raw, ev) = raw_ask(&e.t, &e.user, STUDY_ASK, result_body("exam", 7, "q1", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    assert_hidden(&raw, MODEL, CONN);
    // a summary
    let (raw, ev) = raw_ask(
        &e.t,
        &e.user,
        SUMMARY_ASK,
        json!({"recordId": 7, "lang": "en"}),
    )
    .await;
    assert_eq!(terminal_name(&ev), "done");
    assert_hidden(&raw, MODEL, CONN);
    // the saved lookups
    let (s, v) = e.t.get(&saved_url(), &e.user).await;
    assert_eq!(s, 200, "{v}");
    assert!(v["latest"].is_object(), "{v}");
    hidden_json(&v);
    let (s, v) =
        e.t.get("/api/ai/summary/saved?record=7&at=202611112158", &e.user)
            .await;
    assert_eq!(s, 200, "{v}");
    assert!(v["latest"].is_object(), "{v}");
    hidden_json(&v);
    // a second user's shared-cache hit
    let mut l = e.t.app.0.limits();
    l.study_share = 1;
    e.t.app.0.set_limits(l);
    let kim = e.t.user("kim", false, true);
    let (_, first) = raw_ask(&e.t, &e.user, STUDY_ASK, hint_body(K1, "q2", 2)).await;
    assert_eq!(start_of(&first)["shared"], false);
    eventually(|| q1::<i64>(&e.t, "SELECT COUNT(*) FROM study_shared") >= 1).await;
    let (raw, ev) = raw_ask(&e.t, &kim, STUDY_ASK, hint_body(K2, "q2", 2)).await;
    assert_eq!(start_of(&ev)["shared"], true);
    assert_hidden(&raw, MODEL, CONN);
}

#[tokio::test]
async fn an_admin_sees_the_model_in_study_frames() {
    let e = env().await;
    let (raw, ev) = raw_ask(&e.t, &e.admin, STUDY_ASK, hint_body(K1, "q1", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    assert_eq!(start_of(&ev)["model"], json!(MODEL), "{raw}");
    let (s, v) = e.t.get(&saved_url(), &e.admin).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["latest"]["model"], json!(MODEL));
}
