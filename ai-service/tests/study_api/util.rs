//! Shared set-up of the study tests: an app with AI on, an administrator, a user with the AI+ feature, and made-up
//! questions whose stems carry the markers the stub understands (`[leak]`, `[err]`, `[slow]`, `[long]`).
#![allow(dead_code)]

pub use crate::common::*;
use serde_json::{json, Value};
use std::time::Duration;

pub const K1: &str = "0123456789abcdef0123456789abcdef";
pub const K2: &str = "fedcba9876543210fedcba9876543210";
pub const K3: &str = "aaaaaaaabbbbbbbbccccccccdddddddd";

pub fn now() -> i64 {
    xexam_ai_service::util::unix_now()
}

pub struct Env {
    pub t: TestApp,
    pub admin: Who,
    pub user: Who,
}

/// `TZ=UTC` for the whole binary, before the first time zone lookup.
fn utc() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| std::env::set_var("TZ", "UTC"));
}

pub async fn env() -> Env {
    env_with(Opts::default()).await
}

pub async fn env_with(opts: Opts) -> Env {
    utc();
    let t = start_with(opts).await;
    let admin = t.user("root", true, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let user = t.user("lee", false, true);
    t.add_question(single_question("q1", "Which tool fits the made-up job?"));
    t.add_question(single_question(
        "q2",
        "Which tool fits the second made-up job?",
    ));
    t.add_question(hotspot_question("qh"));
    for (id, marker) in [
        ("q-leak", "[leak]"),
        ("q-err", "[err]"),
        ("q-slow", "[slow]"),
        ("q-long", "[long]"),
        ("q-think", "[think]"),
        ("q-think-late", "[think-late]"),
        ("q-leakerr", "[leak][err]"),
        ("q-leakslow", "[leak][slow]"),
    ] {
        t.add_question(single_question(
            id,
            &format!("{marker} Which tool fits the job?"),
        ));
    }
    let mut hl = hotspot_question("qh-leak");
    hl.question["stem"] = json!("[leak] Review the made-up statements.");
    t.add_question(hl);
    Env { t, admin, user }
}

/// A practice-page hint body.
pub fn hint_body(key: &str, qid: &str, no: i64) -> Value {
    json!({"kind": "hint", "mode": "practice", "key": key, "startedAt": now() - 3600, "questionId": qid, "no": no,
           "qlang": "en", "source": "practice", "lang": "en"})
}

/// A practice-page explanation body.
pub fn explain_body(key: &str, qid: &str, no: i64) -> Value {
    json!({"kind": "explain", "mode": "practice", "key": key, "startedAt": now() - 3600, "questionId": qid,
           "no": no, "qlang": "en", "source": "practice", "lang": "en"})
}

/// A result-page / detail-page explanation body for a saved record (`mode` = the record's kind).
pub fn result_body(mode: &str, record: i64, qid: &str, no: i64) -> Value {
    json!({"kind": "explain", "mode": mode, "questionId": qid, "no": no, "qlang": "en", "source": "result",
           "recordId": record, "lang": "en"})
}

pub const STUDY_ASK: &str = "/api/ai/study/ask";
pub const SUMMARY_ASK: &str = "/api/ai/summary/ask";

/// Opens an ask and reads the whole stream; panics on a JSON error.
pub async fn ask_to(t: &TestApp, who: &Who, path: &str, body: Value) -> Vec<(String, Value)> {
    let mut s = t
        .open_stream(path, who, body)
        .await
        .unwrap_or_else(|(s, v)| panic!("expected a stream, got {s} {v}"));
    let ev = s.all().await;
    settle(t).await;
    ev
}

pub async fn ask(t: &TestApp, who: &Who, body: Value) -> Vec<(String, Value)> {
    ask_to(t, who, STUDY_ASK, body).await
}

/// An ask that is refused with a JSON error.
pub async fn refused(t: &TestApp, who: &Who, path: &str, body: Value) -> (u16, Value) {
    match t.open_stream(path, who, body).await {
        Ok(_) => panic!("expected a refusal"),
        Err(e) => e,
    }
}

/// Waits until the stub has an upstream chat request in hand and had time to hold it (before `release()`).
pub async fn stub_waiting(t: &TestApp) {
    eventually(|| {
        t.stub
            .requests()
            .iter()
            .any(|r| r.body["messages"].to_string().contains("[slow]"))
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
}

/// Waits until no generation holds a place any more (the end of a stream is sent just before the place is released).
pub async fn settle(t: &TestApp) {
    for _ in 0..200 {
        if t.app.0.registry.chat_count() == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("a generation is still running");
}

pub fn start_of(ev: &[(String, Value)]) -> Value {
    ev.iter()
        .find(|(n, _)| n == "start")
        .map(|(_, v)| v.clone())
        .expect("a start event")
}

pub fn terminal_name(ev: &[(String, Value)]) -> String {
    terminal(ev).0
}

/// A one-value query of the AI database.
pub fn q1<T: rusqlite::types::FromSql>(t: &TestApp, sql: &str) -> T {
    t.db().query_row(sql, [], |r| r.get(0)).expect(sql)
}

/// Setting a limit through the admin route.
pub async fn set_limit(t: &TestApp, admin: &Who, key: &str, n: i64) {
    let (s, v) = t
        .post("/api/ai/admin/limits", admin, json!({ key: n }))
        .await;
    assert_eq!(s, 200, "{v}");
}

/// A made-up saved record.
pub fn record(
    user: &Who,
    id: i64,
    kind: &str,
    key: Option<&str>,
    at: &str,
    items: Vec<FakeItem>,
) -> FakeRecord {
    FakeRecord {
        user: user.id,
        id,
        at: at.to_string(),
        kind: kind.to_string(),
        key: key.map(str::to_string),
        started_at: key.map(|_| now() - 7200),
        detail: true,
        items,
    }
}

pub fn item(qid: &str, answer: Value, correct: bool) -> FakeItem {
    FakeItem {
        qid: qid.to_string(),
        answer,
        correct,
    }
}
