//! v1.1 AI bank generation (GJ-1 … GJ-6, API-U1 … U5, API-A1, GP-7 in the running service): the in-process app, the fake
//! backend's generation routes and a scripted provider. Every text is made up.
mod common;

use common::genkit::*;
use common::*;
use serde_json::{json, Value};
use std::time::Duration;

struct E {
    t: TestApp,
    admin: Who,
    user: Who,
}

fn fast(t: &TestApp) {
    use std::sync::atomic::Ordering::SeqCst;
    t.app.0.knobs.backoff_scale.store(0, SeqCst);
    t.app.0.knobs.place_poll_ms.store(20, SeqCst);
    t.relax_caps();
}

async fn env() -> E {
    let t = start().await;
    let admin = t.user("root", true, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    // these tests exercise the call ceilings, so the v1.1 default (unlimited on) is switched off
    t.post(
        "/api/ai/admin/limits",
        &admin,
        json!({"genUnlimited": 0, "importUnlimited": 0}),
    )
    .await;
    let user = t.user("alexandra_k", false, true);
    t.backend.st().gen = kitchen(2, 20);
    t.stub.st().script = Some(good_model());
    fast(&t);
    E { t, admin, user }
}

const BANK: fn() -> Value = || json!({"source": "bank", "bankId": 3});

async fn start_job(e: &E, who: &Who, body: Value) -> (u16, Value) {
    e.t.post("/api/ai/gen/start", who, body).await
}

/// Polls the job until it is in one of the states.
async fn wait_state(t: &TestApp, who: &Who, id: i64, states: &[&str]) -> Value {
    for _ in 0..600 {
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

async fn run_to_end(e: &E, who: &Who, body: Value) -> Value {
    let (s, v) = start_job(e, who, body).await;
    assert_eq!(s, 202, "{v}");
    let id = v["job"]["id"].as_i64().expect("id");
    wait_state(&e.t, who, id, &["done", "failed", "cancelled"]).await
}

fn q1<T: rusqlite::types::FromSql>(t: &TestApp, sql: &str) -> T {
    t.db().query_row(sql, [], |r| r.get(0)).expect(sql)
}

#[tokio::test]
async fn a_bank_generation_makes_exactly_twenty_valid_questions_and_creates_the_bank() {
    let e = env().await;
    let usage_before = e.t.usage_rows();
    let (s, v) = start_job(&e, &e.user, BANK()).await;
    assert_eq!(s, 202, "{v}");
    let job = &v["job"];
    assert_eq!(job["source"], "bank");
    assert_eq!(job["goal"], 20);
    assert_eq!(job["charged"], true);
    assert!(job["name"]
        .as_str()
        .unwrap_or("")
        .starts_with("Kitchen basics_alexandr_"));
    assert_eq!(v["gen"]["used"], 1, "queued jobs count at once");
    let id = job["id"].as_i64().expect("id");
    let done = wait_state(&e.t, &e.user, id, &["done", "failed"]).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(
        (done["made"].clone(), done["bankId"].clone()),
        (json!(20), json!(41))
    );
    assert_eq!(done["charged"], true);
    // the create body of API-I3
    let st = e.t.backend.st();
    let body = st.gen.created.first().expect("a create call");
    let keys: Vec<&str> = body
        .as_object()
        .expect("obj")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "job",
            "user",
            "stamp",
            "name",
            "source",
            "from",
            "goal",
            "at",
            "sheet",
            "questions"
        ]
    );
    assert_eq!(body["source"], "bank");
    assert_eq!(body["from"], json!([3]));
    assert_eq!(body["sheet"], "AI");
    assert_eq!(body["goal"], 20);
    assert_eq!(body["user"], e.user.id);
    let qs = body["questions"].as_array().expect("questions");
    assert_eq!(qs.len(), 20);
    let multi = qs.iter().filter(|q| q["kind"] == "multi").count();
    assert_eq!(
        (qs.len() - multi, multi),
        (14, 6),
        "14 single + 6 multiple choice"
    );
    for q in qs {
        let opts = q["options"].as_array().expect("options").len();
        assert!((3..=6).contains(&opts));
        let ans = q["answer"].as_array().expect("answer");
        assert_eq!(ans.len(), if q["kind"] == "multi" { 2 } else { 1 });
    }
    assert_eq!(body["at"].as_str().map(str::len), Some(19));
    drop(st);
    // calls are gen_calls rows, never usage rows
    assert_eq!(
        e.t.usage_rows(),
        usage_before,
        "a generation writes no usage row"
    );
    let calls: i64 = q1(
        &e.t,
        "SELECT COUNT(*) FROM gen_calls WHERE purpose = 'generate' AND status = 'ok'",
    );
    assert_eq!(calls, 4, "20 questions at 5 per call");
    let accepted: i64 = q1(&e.t, "SELECT SUM(accepted) FROM gen_calls");
    assert_eq!(accepted, 20);
    assert_eq!(done["calls"], 4);
    // the quota unit is charged today
    let charged_at: i64 = q1(&e.t, "SELECT charged_at FROM tasks");
    assert!(charged_at > 0);
    assert!(captured_contains(
        "started an AI bank generation (bank mode, 20 questions)"
    ));
    assert!(captured_contains(
        "finished an AI bank generation (20 questions, 4 model calls)"
    ));
}

#[tokio::test]
async fn the_status_reports_the_quota_the_job_and_the_unseen_card() {
    let e = env().await;
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(v["why"], Value::Null);
    assert_eq!(
        v["features"],
        json!({"chat": true, "hint": true, "explain": true, "summary": true, "generate": true, "import": true})
    );
    let g = &v["gen"];
    assert_eq!(
        (
            g["perDay"].clone(),
            g["used"].clone(),
            g["left"].clone(),
            g["unlimited"].clone()
        ),
        (json!(3), json!(0), json!(3), json!(false))
    );
    assert_eq!(
        (g["minSource"].clone(), g["minWrong"].clone()),
        (json!(30), json!(10))
    );
    assert_eq!(g["bank"], json!({"single": 14, "multi": 6}));
    assert_eq!(g["wrong"], json!({"single": 14, "multi": 6}));
    assert_eq!(
        g["import"],
        json!({"maxMb": 20, "maxQuestions": 50, "types": ["pdf", "docx", "xlsx", "pptx", "md", "txt", "csv"]})
    );
    assert_eq!(g["job"], Value::Null);
    assert!(g["resetAt"].as_i64().unwrap_or(0) > xexam_ai_service::util::unix_now());
    let done = run_to_end(&e, &e.user, BANK()).await;
    assert_eq!(done["state"], "done");
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(v["gen"]["used"], 1);
    assert_eq!(v["gen"]["left"], 2);
    assert_eq!(v["gen"]["unseen"].as_array().map(Vec::len), Some(1));
    // dismissing clears it (idempotent)
    let id = done["id"].as_i64().expect("id");
    for _ in 0..2 {
        let (s, v) =
            e.t.post("/api/ai/gen/dismiss", &e.user, json!({"id": id}))
                .await;
        assert_eq!((s, v), (200, json!({"ok": true})));
    }
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(v["gen"]["unseen"], json!([]));
    // an administrator is unlimited by default, a user without AI+ gets neither features nor gen
    let (_, v) = e.t.get("/api/ai/status", &e.admin).await;
    assert_eq!(v["gen"]["unlimited"], true);
    assert_eq!(v["gen"]["perDay"], Value::Null);
    assert_eq!(v["gen"]["left"], Value::Null);
    let plain = e.t.user("plain", false, false);
    let (_, v) = e.t.get("/api/ai/status", &plain).await;
    assert!(v.get("features").is_none() && v.get("gen").is_none());
    assert_eq!(v["why"], Value::Null);
}

#[tokio::test]
async fn the_refusals_of_start_in_their_order() {
    let e = env().await;
    // a user without AI+
    let plain = e.t.user("plain", false, false);
    let (s, v) = start_job(&e, &plain, BANK()).await;
    assert_eq!((s, &v["code"]), (403, &json!("ai.notAllowed")));
    // a bad body
    let (s, v) = start_job(&e, &e.user, json!({"source": "bank"})).await;
    assert_eq!((s, &v["code"]), (400, &json!("ai.badRequest")));
    // the Home name
    let (s, v) = start_job(
        &e,
        &e.user,
        json!({"source": "scope", "sheets": [{"bank": 3, "sheet": "Ovens"}], "name": "  "}),
    )
    .await;
    assert_eq!(
        (s, &v["code"], &v["max"]),
        (400, &json!("ai.gen.badName"), &json!(60))
    );
    // the backend's refusals
    for (reason, status, code) in [
        ("notAllowed", 403, "ai.gen.forbidden"),
        ("notFound", 404, "ai.gen.notFound"),
        ("aiSource", 422, "ai.gen.aiSource"),
        ("session", 403, "ai.notAllowed"),
    ] {
        e.t.backend.st().gen.source_refusal = Some(reason.to_string());
        let (s, v) = start_job(&e, &e.user, BANK()).await;
        assert_eq!((s, &v["code"]), (status, &json!(code)), "{reason}");
    }
    e.t.backend.st().gen.source_refusal = None;
    // too few questions
    e.t.backend.st().gen = kitchen(1, 12);
    let (s, v) = start_job(&e, &e.user, BANK()).await;
    assert_eq!(
        (s, &v["code"], &v["have"], &v["need"], &v["what"]),
        (
            422,
            &json!("ai.gen.tooFew"),
            &json!(12),
            &json!(30),
            &json!("source")
        )
    );
    // wrong mode names the other minimum
    let (s, v) = start_job(
        &e,
        &e.user,
        json!({"source": "wrong", "sheets": [{"bank": 3, "sheet": "Ovens"}], "name": "Wrong ones"}),
    )
    .await;
    assert_eq!(s, 202, "12 wrong questions are enough (minimum 10): {v}");
    // nothing refused wrote a row except the accepted one
    let rows: i64 = q1(&e.t, "SELECT COUNT(*) FROM tasks");
    assert_eq!(rows, 1);
    // the backend cannot be reached
    e.t.backend.set_mode(Mode::Status500);
    let (s, v) = start_job(&e, &e.admin, BANK()).await;
    assert_eq!((s, &v["code"]), (503, &json!("ai.backendDown")));
    e.t.backend.set_mode(Mode::Normal);
}

#[tokio::test]
async fn one_job_per_user_and_the_quota_with_its_retry_header() {
    let e = env().await;
    // hold the provider so the first job stays active
    e.t.stub.st().head_delay_ms = 600;
    let (s, v) = start_job(&e, &e.user, BANK()).await;
    assert_eq!(s, 202, "{v}");
    let first = v["job"]["id"].as_i64().expect("id");
    let (s, v) = start_job(&e, &e.user, BANK()).await;
    assert_eq!((s, &v["code"]), (409, &json!("ai.gen.busy")));
    assert_eq!(
        v["job"]["id"], first,
        "the busy answer carries the running job"
    );
    e.t.stub.st().head_delay_ms = 0;
    wait_state(&e.t, &e.user, first, &["done"]).await;
    // genPerDay 1: the unit is used up
    let (s, _) =
        e.t.post("/api/ai/admin/limits", &e.admin, json!({"genPerDay": 1}))
            .await;
    assert_eq!(s, 200);
    let r =
        e.t.raw(
            axum::http::Method::POST,
            "/api/ai/gen/start",
            Some(e.t.ident(&e.user)),
            Some(BANK().to_string().into_bytes()),
            Some("application/json"),
        )
        .await;
    assert_eq!(r.status().as_u16(), 429);
    let retry = r
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let (_, v) = read_json(r).await;
    assert_eq!(v["code"], "ai.gen.quota");
    assert_eq!(
        (v["used"].clone(), v["limit"].clone()),
        (json!(1), json!(1))
    );
    assert!(v["resetAt"].as_i64().unwrap_or(0) > 0 && v["retryAfter"].as_i64().unwrap_or(0) >= 1);
    assert_eq!(retry, Some(v["retryAfter"].to_string()));
    // an administrator is exempt; without the exemption the limit applies to them too
    let done = run_to_end(&e, &e.admin, BANK()).await;
    assert_eq!(done["state"], "done");
    let (s, _) =
        e.t.post(
            "/api/ai/admin/limits",
            &e.admin,
            json!({"genAdminsExempt": 0}),
        )
        .await;
    assert_eq!(s, 200);
    let (s, v) = start_job(&e, &e.admin, BANK()).await;
    assert_eq!((s, &v["code"]), (429, &json!("ai.gen.quota")));
}

#[tokio::test]
async fn a_truncated_answer_never_fails_the_job() {
    let e = env().await;
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = calls.clone();
    let inner = good_model();
    e.t.stub.st().script = Some(std::sync::Arc::new(move |sys: &str, user: &str| {
        let n = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        match inner(sys, user) {
            // the first answer is cut in the middle of its JSON
            Reply::Text(t) if n == 0 => Reply::Cut(t[..t.len() / 2].to_string()),
            other => other,
        }
    }));
    let done = run_to_end(&e, &e.user, BANK()).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["made"], 20);
    let cut: i64 = q1(&e.t, "SELECT COUNT(*) FROM gen_calls WHERE truncated = 1");
    assert_eq!(cut, 1);
    let calls_made = done["calls"].as_i64().unwrap_or(0);
    assert!(calls_made > 4 && calls_made <= 20, "{calls_made}");
}

#[tokio::test]
async fn three_provider_errors_in_a_row_fail_the_job_without_a_bank_or_a_charge() {
    let e = env().await;
    e.t.stub.st().script = Some(std::sync::Arc::new(|_, _| {
        Reply::Status(500, "boom".into())
    }));
    let done = run_to_end(&e, &e.user, BANK()).await;
    assert_eq!(
        (done["state"].clone(), done["error"].clone()),
        (json!("failed"), json!("ai.up.server"))
    );
    assert_eq!(
        (done["charged"].clone(), done["calls"].clone()),
        (json!(false), json!(3))
    );
    assert!(e.t.backend.st().gen.created.is_empty());
    let errors: i64 = q1(
        &e.t,
        "SELECT COUNT(*) FROM gen_calls WHERE status = 'error' AND error_code = 'server'",
    );
    assert_eq!(errors, 3);
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(v["gen"]["used"], 0, "a failure is never counted");
    assert_eq!(
        v["gen"]["unseen"].as_array().map(Vec::len),
        Some(1),
        "the failure shows as a card"
    );
    assert!(captured_contains(
        "AI job failed: 0/20 questions passed the checks; rejected for: none (ai.up.server, 3 model calls, user"
    ));
}

#[tokio::test]
async fn a_model_that_never_delivers_valid_questions_stops_at_the_settings_call_ceiling() {
    let e = env().await;
    e.t.stub.st().script = Some(std::sync::Arc::new(|_, _| {
        Reply::Text("I cannot help with that.".into())
    }));
    let done = run_to_end(&e, &e.user, BANK()).await;
    assert_eq!(
        (done["state"].clone(), done["error"].clone()),
        (json!("failed"), json!("ai.gen.quality"))
    );
    assert_eq!(done["calls"], 15, "the default ceiling of 15 calls per job");
    assert_eq!(done["charged"], false);
    assert!(e.t.backend.st().gen.created.is_empty());
}

#[tokio::test]
async fn with_unlimited_on_a_slot_that_never_arrives_still_ends_the_job_by_the_per_slot_rule() {
    // A-OWNER-66: unlimited lifts the call ceiling only; unreadable answers end after the slots' own tries
    let e = env().await;
    e.t.post("/api/ai/admin/limits", &e.admin, json!({"genUnlimited": 1}))
        .await;
    e.t.stub.st().script = Some(std::sync::Arc::new(|_, _| {
        Reply::Text("I cannot help with that.".into())
    }));
    let done = run_to_end(&e, &e.user, BANK()).await;
    assert_eq!(
        (done["state"].clone(), done["error"].clone()),
        (json!("failed"), json!("ai.gen.quality"))
    );
    let calls = done["calls"].as_i64().unwrap_or(0);
    assert!((1..=100).contains(&calls), "{calls} calls");
    assert!(e.t.backend.st().gen.created.is_empty());
}

#[tokio::test]
async fn cancel_while_a_call_is_in_flight_keeps_the_unit_and_drops_the_request() {
    let e = env().await;
    e.t.stub.st().head_delay_ms = 3000;
    let (_, v) = start_job(&e, &e.user, BANK()).await;
    let id = v["job"]["id"].as_i64().expect("id");
    // wait until the call is sent
    for _ in 0..200 {
        if e.t.stub.count() > 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let (s, v) =
        e.t.post("/api/ai/gen/cancel", &e.user, json!({"id": id}))
            .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["job"]["state"], "cancelled");
    assert_eq!(v["job"]["charged"], true, "a call was sent");
    // the runner records the dropped call as stopped and ends
    for _ in 0..200 {
        let n: i64 = q1(
            &e.t,
            "SELECT COUNT(*) FROM gen_calls WHERE status = 'stopped'",
        );
        if n == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let stopped: i64 = q1(
        &e.t,
        "SELECT COUNT(*) FROM gen_calls WHERE status = 'stopped'",
    );
    assert_eq!(stopped, 1);
    assert!(e.t.backend.st().gen.created.is_empty());
    // cancelling again is too late
    let (s, v) =
        e.t.post("/api/ai/gen/cancel", &e.user, json!({"id": id}))
            .await;
    assert_eq!((s, &v["code"]), (409, &json!("ai.gen.tooLate")));
    // a foreign job answers like a missing one
    let (s, v) =
        e.t.post("/api/ai/gen/cancel", &e.admin, json!({"id": id}))
            .await;
    assert_eq!((s, &v["code"]), (404, &json!("ai.gen.notFound")));
    // the cancelled card is never unseen
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(v["gen"]["unseen"], json!([]));
}

#[tokio::test]
async fn cancel_while_queued_is_not_counted() {
    let e = env().await;
    // one place for everyone: the second user waits
    let (s, _) =
        e.t.post("/api/ai/admin/limits", &e.admin, json!({"genRunning": 1}))
            .await;
    assert_eq!(s, 200);
    e.t.stub.st().head_delay_ms = 400;
    let other = e.t.user("blake", false, true);
    let (s, a) = start_job(&e, &e.user, BANK()).await;
    assert_eq!(s, 202, "{a}");
    let (s, b) = start_job(&e, &other, BANK()).await;
    assert_eq!(s, 202, "{b}");
    let b_id = b["job"]["id"].as_i64().expect("id");
    tokio::time::sleep(Duration::from_millis(150)).await;
    let (_, v) = e.t.get(&format!("/api/ai/gen/job?id={b_id}"), &other).await;
    assert_eq!(
        v["job"]["state"], "queued",
        "the second job waits for a place"
    );
    let (s, v) =
        e.t.post("/api/ai/gen/cancel", &other, json!({"id": b_id}))
            .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        (v["job"]["state"].clone(), v["job"]["charged"].clone()),
        (json!("cancelled"), json!(false))
    );
    e.t.stub.st().head_delay_ms = 0;
    wait_state(
        &e.t,
        &e.user,
        a["job"]["id"].as_i64().expect("id"),
        &["done"],
    )
    .await;
    let (_, v) = e.t.get("/api/ai/status", &other).await;
    assert_eq!(v["gen"]["used"], 0);
}

#[tokio::test]
async fn jobs_run_first_in_first_out_within_the_job_places() {
    let e = env().await;
    let (s, _) =
        e.t.post("/api/ai/admin/limits", &e.admin, json!({"genRunning": 1}))
            .await;
    assert_eq!(s, 200);
    e.t.stub.st().head_delay_ms = 80;
    let users: Vec<Who> = (0..3)
        .map(|i| e.t.user(&format!("cook{i}"), false, true))
        .collect();
    let mut ids = Vec::new();
    for u in &users {
        let (s, v) = start_job(&e, u, BANK()).await;
        assert_eq!(s, 202, "{v}");
        ids.push(v["job"]["id"].as_i64().expect("id"));
    }
    for (u, id) in users.iter().zip(&ids) {
        let j = wait_state(&e.t, u, *id, &["done"]).await;
        assert_eq!(j["state"], "done");
    }
    let started: Vec<i64> = {
        let c = e.t.db();
        let mut st = c
            .prepare("SELECT id FROM tasks ORDER BY started_at, id")
            .expect("prepare");
        st.query_map([], |r| r.get(0))
            .expect("query")
            .map(|r| r.expect("row"))
            .collect()
    };
    assert_eq!(started, ids, "FIFO by job id");
}

#[tokio::test]
async fn the_feature_taken_away_mid_job_stops_it() {
    let e = env().await;
    e.t.stub.st().head_delay_ms = 300;
    let (_, v) = start_job(&e, &e.user, BANK()).await;
    let id = v["job"]["id"].as_i64().expect("id");
    let (s, v) = e
        .t
        .post(
            "/api/ai/admin/features",
            &e.admin,
            json!({"userId": e.user.id, "features": {"chat": true, "hint": true, "explain": true, "summary": true, "generate": false, "import": true}}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let j = wait_state(&e.t, &e.user, id, &["failed", "done"]).await;
    assert_eq!(
        (j["state"].clone(), j["error"].clone()),
        (json!("failed"), json!("ai.gen.stopped"))
    );
    assert_eq!(j["charged"], false);
    // and the start route refuses it now
    let (s, v) = start_job(&e, &e.user, BANK()).await;
    assert_eq!(
        (s, &v["code"], &v["feature"]),
        (403, &json!("ai.featureOff"), &json!("generate"))
    );
}

#[tokio::test]
async fn the_account_checked_by_the_main_service_each_batch() {
    let e = env().await;
    e.t.stub.st().head_delay_ms = 300;
    let (_, v) = start_job(&e, &e.user, BANK()).await;
    let id = v["job"]["id"].as_i64().expect("id");
    e.t.set_ai(&e.user, false);
    let j = wait_state(&e.t, &e.user, id, &["failed", "done"]).await;
    assert_eq!(
        (j["state"].clone(), j["error"].clone()),
        (json!("failed"), json!("ai.gen.forbidden"))
    );
}

#[tokio::test]
async fn missing_seeds_are_replaced_and_the_goal_never_shrinks() {
    let e = env().await;
    // the first twenty source questions vanish between the source and the seeds calls
    let gone: Vec<String> = (1..=10).map(|n| format!("s{n:03}")).collect();
    e.t.backend.st().gen.seeds_missing = gone.clone();
    let done = run_to_end(&e, &e.user, BANK()).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["goal"], 20);
    assert_eq!(
        e.t.backend.st().gen.created[0]["questions"]
            .as_array()
            .map(Vec::len),
        Some(20)
    );
    // every seed missing: nothing to learn from
    let e2 = env().await;
    let all: Vec<String> = e2.t.backend.st().gen.seeds.keys().cloned().collect();
    e2.t.backend.st().gen.seeds_missing = all;
    let (s, v) = start_job(&e2, &e2.user, BANK()).await;
    assert_eq!((s, &v["code"]), (404, &json!("ai.gen.notFound")));
    let rows: i64 = q1(&e2.t, "SELECT COUNT(*) FROM tasks");
    assert_eq!(rows, 0, "nothing written");
}

#[tokio::test]
async fn a_create_that_cannot_be_saved_fails_the_job() {
    let e = env().await;
    e.t.backend.st().gen.create_refusal = Some("notAllowed".into());
    let done = run_to_end(&e, &e.user, BANK()).await;
    assert_eq!(
        (done["state"].clone(), done["error"].clone()),
        (json!("failed"), json!("ai.gen.forbidden"))
    );
    assert_eq!(done["charged"], false);
}

#[tokio::test]
async fn the_admin_list_shows_every_users_jobs_with_tokens() {
    let e = env().await;
    let done = run_to_end(&e, &e.user, BANK()).await;
    assert_eq!(done["state"], "done");
    let (s, v) = e.t.get("/api/ai/admin/gen?limit=10", &e.admin).await;
    assert_eq!(s, 200, "{v}");
    let row = &v["jobs"][0];
    assert_eq!(
        (row["username"].clone(), row["userId"].clone()),
        (json!("alexandra_k"), json!(e.user.id))
    );
    assert_eq!(row["calls"], 4);
    assert_eq!(
        (row["tokensIn"].clone(), row["tokensOut"].clone()),
        (json!(84), json!(52))
    );
    // administrators only, and a bad limit
    let (s, v) = e.t.get("/api/ai/admin/gen", &e.user).await;
    assert_eq!((s, &v["code"]), (403, &json!("ai.adminOnly")));
    let (s, _) = e.t.get("/api/ai/admin/gen?limit=0", &e.admin).await;
    assert_eq!(s, 400);
    let (s, _) = e.t.get("/api/ai/admin/gen?limit=201", &e.admin).await;
    assert_eq!(s, 400);
    // the usage totals include the generation calls, byPurpose has the generate row
    let (s, v) = e.t.get("/api/ai/admin/usage?days=7", &e.admin).await;
    assert_eq!(s, 200, "{v}");
    let gen_row = v["byPurpose"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["purpose"] == "generate")
        .expect("generate row")
        .clone();
    assert_eq!(gen_row["calls"], 4);
    let by_user = v["byUser"]
        .as_array()
        .expect("byUser")
        .iter()
        .find(|r| r["username"] == "alexandra_k")
        .expect("user row")
        .clone();
    assert_eq!(by_user["calls"], 4);
    assert_eq!(
        by_user["today"], 0,
        "generation calls never count against the call caps"
    );
}

#[tokio::test]
async fn a_restart_fails_active_jobs_and_a_saved_bank_is_found_by_its_job_key() {
    let mut e = env().await;
    let now = xexam_ai_service::util::unix_now();
    {
        let c = e.t.db();
        for (id, uid, key, phase) in [
            (1, e.user.id, "aa", "writing"),
            (2, e.admin.id, "bb", "saving"),
        ] {
            c.execute(
                "INSERT OR IGNORE INTO users (id, stamp, username, first_seen_at, last_seen_at) VALUES (?1, 'x', 'u', 1, 1)",
                [uid],
            )
            .expect("user");
            c.execute(
                "INSERT INTO tasks (id, user_id, kind, state, payload, attempts, run_after, created_at, updated_at, job_key, goal, made, phase, charged, charged_at)
                 VALUES (?1, ?2, 'bank_gen', 'running', '{\"source\":\"bank\"}', 2, ?3, ?3, ?3, ?4, 20, 5, ?5, 1, ?3)",
                rusqlite::params![id, uid, now, key, phase],
            )
            .expect("task");
        }
    }
    e.t.backend
        .st()
        .gen
        .banks_by_job
        .insert("bb".into(), (77, "Saved bank".into()));
    e.t.restart(Opts::default()).await;
    let row = |id: i64| -> (String, Option<String>, i64) {
        e.t.db()
            .query_row(
                "SELECT state, error_code, charged FROM tasks WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("row")
    };
    assert_eq!(
        row(1),
        ("failed".into(), Some("ai.gen.interrupted".into()), 0)
    );
    assert_eq!(row(2).0, "running", "a saving job waits for the lookup");
    xexam_ai_service::gen::runner::settle_saving(&e.t.app).await;
    let (state, err, charged) = row(2);
    assert_eq!((state.as_str(), err, charged), ("done", None, 1));
    let result: String = q1(&e.t, "SELECT result FROM tasks WHERE id = 2");
    assert!(result.contains("\"bankId\":77"), "{result}");
}

#[tokio::test]
async fn purging_a_user_stops_the_running_job_and_removes_its_rows() {
    let e = env().await;
    e.t.stub.st().head_delay_ms = 400;
    let (_, v) = start_job(&e, &e.user, BANK()).await;
    assert!(v["job"]["id"].as_i64().is_some());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(e.t.app.0.purge_user(e.user.id).await);
    assert!(e.t.app.0.jobs.is_empty(), "the runner is gone");
    let rows: i64 = q1(&e.t, "SELECT COUNT(*) FROM tasks");
    assert_eq!(rows, 0);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        e.t.backend.st().gen.created.is_empty(),
        "nothing is created after a purge"
    );
}

#[tokio::test]
async fn the_wrong_and_scope_modes_use_the_typed_name_and_their_own_composition() {
    let e = env().await;
    let (s, _) =
        e.t.post(
            "/api/ai/admin/limits",
            &e.admin,
            json!({"genWrongSingle": 5, "genWrongMulti": 5}),
        )
        .await;
    assert_eq!(s, 200);
    let done = run_to_end(
        &e,
        &e.user,
        json!({"source": "wrong", "sheets": [{"bank": 3, "sheet": "Ovens"}, {"bank": 3, "sheet": "Knives"}], "name": "My wrong ones"}),
    )
    .await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["goal"], 10);
    assert_eq!(done["name"], "My wrong ones");
    let st = e.t.backend.st();
    let body = st.gen.created.last().expect("create");
    assert_eq!(body["name"], "My wrong ones");
    assert_eq!(body["source"], "wrong");
    let qs = body["questions"].as_array().expect("qs");
    assert_eq!(qs.iter().filter(|q| q["kind"] == "multi").count(), 5);
    // the source call carried the sheets and the wrong-mode minimum
    let req = st.gen.source_requests.last().expect("request");
    assert_eq!(req["source"], "wrong");
    assert_eq!(req["min"], 10);
    assert_eq!(
        req["sheets"],
        json!([{"bank": 3, "sheet": "Ovens"}, {"bank": 3, "sheet": "Knives"}])
    );
}

#[tokio::test]
async fn the_composition_rule_and_a_lowered_gen_running_apply_at_runtime() {
    let e = env().await;
    let (s, v) =
        e.t.post(
            "/api/ai/admin/limits",
            &e.admin,
            json!({"genBankSingle": 0, "genBankMulti": 0}),
        )
        .await;
    assert_eq!(
        (s, &v["code"], &v["field"]),
        (400, &json!("ai.genCount"), &json!("genBankMulti"))
    );
    let (s, v) =
        e.t.post(
            "/api/ai/admin/limits",
            &e.admin,
            json!({"genWrongSingle": 90, "genWrongMulti": 90}),
        )
        .await;
    assert_eq!(
        (s, &v["code"], &v["field"]),
        (400, &json!("ai.genCount"), &json!("genWrongMulti"))
    );
    // the limits were not changed by the refusals
    let (_, v) = e.t.get("/api/ai/admin/limits", &e.admin).await;
    assert_eq!(v["limits"]["genBankSingle"], 14);
    assert_eq!(e.t.app.0.gen_gate.size(), 2);
    let (s, _) =
        e.t.post("/api/ai/admin/limits", &e.admin, json!({"genRunning": 4}))
            .await;
    assert_eq!(s, 200);
    assert_eq!(e.t.app.0.gen_gate.size(), 4);
}
