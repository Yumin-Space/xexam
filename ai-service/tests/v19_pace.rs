//! v1.9 pacing of AI generation and import (item 1h): the three limit keys, the 429 wait-and-retry, the daily quota stop
//! and the even spacing of the calls. Every text is made up.
mod common;

use common::genkit::*;
use common::*;
use serde_json::{json, Value};
use std::sync::atomic::Ordering::SeqCst;
use std::time::Duration;
use xexam_ai_service::extract::testgen::{self, DocxSpec};

struct E {
    t: TestApp,
    admin: Who,
    user: Who,
}

async fn env() -> E {
    let t = start().await;
    let admin = t.user("root", true, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let user = t.user("alexandra_k", false, true);
    t.backend.st().gen = kitchen(2, 20);
    t.stub.st().script = Some(good_model());
    t.app.0.knobs.backoff_scale.store(0, SeqCst);
    t.app.0.knobs.place_poll_ms.store(20, SeqCst);
    t.relax_caps();
    E { t, admin, user }
}

const GOOGLE_WAIT: &str =
    r#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","details":[{"retryDelay":"2s"}]}}"#;
const GOOGLE_DAILY: &str = r#"{"error":{"code":429,"details":[{"violations":[{"quotaId":"GenerateRequestsPerDayPerProjectPerModel-FreeTier"}]},{"retryDelay":"20s"}]}}"#;

async fn wait_state(t: &TestApp, who: &Who, id: i64, states: &[&str]) -> Value {
    for _ in 0..800 {
        let (s, v) = t.get(&format!("/api/ai/gen/job?id={id}"), who).await;
        assert_eq!(s, 200, "{v}");
        let state = v["job"]["state"].as_str().unwrap_or("");
        if states.contains(&state) {
            return v["job"].clone();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the job never reached {states:?}");
}

async fn run_gen(e: &E) -> Value {
    let (s, v) =
        e.t.post(
            "/api/ai/gen/start",
            &e.user,
            json!({"source": "bank", "bankId": 3}),
        )
        .await;
    assert_eq!(s, 202, "{v}");
    let id = v["job"]["id"].as_i64().expect("id");
    wait_state(&e.t, &e.user, id, &["done", "failed", "cancelled"]).await
}

fn count(t: &TestApp, sql: &str) -> i64 {
    t.db().query_row(sql, [], |r| r.get(0)).expect(sql)
}

fn limited(n: usize, body: &str) -> std::collections::VecDeque<(u16, String, Option<String>)> {
    (0..n).map(|_| (429u16, body.to_string(), None)).collect()
}

#[tokio::test]
async fn the_pacing_keys_have_defaults_ranges_and_stay_apart_from_the_unlimited_switches() {
    let e = env().await;
    let (s, v) = e.t.get("/api/ai/admin/limits", &e.admin).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        (
            v["limits"]["paceOn"].clone(),
            v["limits"]["paceRpm"].clone(),
            v["limits"]["paceTpm"].clone()
        ),
        (json!(1), json!(15), json!(250_000))
    );
    assert_eq!(v["ranges"]["paceOn"], json!([0, 1]));
    assert_eq!(v["ranges"]["paceRpm"], json!([1, 10_000]));
    assert_eq!(v["ranges"]["paceTpm"], json!([1000, 100_000_000]));
    let (s, v) =
        e.t.post("/api/ai/admin/limits", &e.admin, json!({"paceRpm": 0}))
            .await;
    assert_eq!(
        (s, v["code"].clone(), v["field"].clone()),
        (400, json!("ai.limitRange"), json!("paceRpm"))
    );
    let (s, _) =
        e.t.post("/api/ai/admin/limits", &e.admin, json!({"paceTpm": 999}))
            .await;
    assert_eq!(s, 400);
    let (s, v) =
        e.t.post(
            "/api/ai/admin/limits",
            &e.admin,
            json!({"paceOn": 0, "paceRpm": 30, "paceTpm": 90_000}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let (_, v) = e.t.get("/api/ai/admin/limits", &e.admin).await;
    assert_eq!(
        (
            v["limits"]["paceOn"].clone(),
            v["limits"]["paceRpm"].clone(),
            v["limits"]["paceTpm"].clone()
        ),
        (json!(0), json!(30), json!(90_000))
    );
    // the unlimited switches never touch the pacing keys
    let (s, _) =
        e.t.post(
            "/api/ai/admin/limits",
            &e.admin,
            json!({"genUnlimited": 0, "importUnlimited": 1}),
        )
        .await;
    assert_eq!(s, 200);
    let (_, v) = e.t.get("/api/ai/admin/limits", &e.admin).await;
    assert_eq!(
        (
            v["limits"]["paceOn"].clone(),
            v["limits"]["paceRpm"].clone(),
            v["limits"]["paceTpm"].clone()
        ),
        (json!(0), json!(30), json!(90_000))
    );
}

#[tokio::test]
async fn three_rate_limit_answers_are_waited_out_and_the_job_still_finishes() {
    let e = env().await;
    // a short wait: the provider names 2 s, one hundredth of it is slept
    e.t.app.0.knobs.pace_scale.store(1, SeqCst);
    e.t.stub.st().fail_next = limited(3, GOOGLE_WAIT);
    let done = run_gen(&e).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["waiting"], json!(false));
    assert_eq!(
        count(
            &e.t,
            "SELECT COUNT(*) FROM gen_calls WHERE error_code = 'rateLimit'"
        ),
        3
    );
}

#[tokio::test]
async fn a_rate_limit_is_waited_out_even_with_pacing_off() {
    let e = env().await;
    e.t.post("/api/ai/admin/limits", &e.admin, json!({"paceOn": 0}))
        .await;
    e.t.app.0.knobs.pace_scale.store(1, SeqCst);
    e.t.stub.st().fail_next = limited(1, GOOGLE_WAIT);
    let done = run_gen(&e).await;
    assert_eq!(done["state"], "done", "{done}");
}

#[tokio::test]
async fn a_daily_quota_stops_a_generation_job_without_a_charge() {
    let e = env().await;
    e.t.app.0.knobs.pace_scale.store(1, SeqCst);
    e.t.stub.st().fail_next = limited(1, GOOGLE_DAILY);
    let done = run_gen(&e).await;
    assert_eq!(
        (done["state"].clone(), done["error"].clone()),
        (json!("failed"), json!("ai.gen.providerDaily")),
        "{done}"
    );
    assert_eq!(done["charged"], json!(false));
}

#[tokio::test]
async fn a_daily_quota_stops_an_import_job() {
    let e = env().await;
    e.t.app.0.knobs.pace_scale.store(1, SeqCst);
    e.t.app.0.knobs.import_context_tokens.store(0, SeqCst);
    *e.t.app.0.knobs.exe_override.lock().unwrap() =
        Some(env!("CARGO_BIN_EXE_xexam-ai-service").into());
    e.t.stub.st().fail_next = limited(1, GOOGLE_DAILY);
    let lines = questions_lines(6);
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let bytes = testgen::docx(&DocxSpec::paragraphs(&refs));
    let (s, v) = e.t.upload(&e.user, "workbook.docx", bytes).await;
    assert_eq!(s, 202, "{v}");
    let id = v["job"]["id"].as_i64().expect("id");
    let j = wait_state(&e.t, &e.user, id, &["failed", "done", "cancelled"]).await;
    assert_eq!(
        (j["state"].clone(), j["error"].clone()),
        (json!("failed"), json!("ai.gen.providerDaily")),
        "{j}"
    );
}

#[tokio::test]
async fn the_calls_of_a_job_are_spread_over_the_window() {
    let e = env().await;
    // 10 requests a minute = 9 with the margin; the window is shrunk to 6 s: one call every 667 ms
    e.t.post("/api/ai/admin/limits", &e.admin, json!({"paceRpm": 10}))
        .await;
    e.t.app.0.knobs.pace_scale.store(10, SeqCst);
    let before = e.t.stub.count();
    let done = run_gen(&e).await;
    assert_eq!(done["state"], "done", "{done}");
    let at: Vec<_> = e.t.stub.requests()[before..]
        .iter()
        .filter(|r| r.path.ends_with("/chat/completions"))
        .map(|r| r.at)
        .collect();
    assert!(at.len() >= 2, "only {} calls", at.len());
    for w in at.windows(2) {
        let gap = w[1].duration_since(w[0]);
        assert!(
            gap >= Duration::from_millis(600),
            "calls only {gap:?} apart"
        );
    }
}
