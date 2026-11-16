//! v1.6 thinking strength per feature: the admin route, the stored value per slot, what every model call of a feature
//! puts into its request body, the one-time fallback when a provider refuses the parameter, and the added column.
//! Everything is made up.
mod common;

use common::genkit::*;
use common::*;
use serde_json::{json, Value};
use std::sync::atomic::Ordering::SeqCst;
use std::time::Duration;
use xexam_ai_service::config::JournalMode;
use xexam_ai_service::store::{Store, DB_FILE};

const EFFORT: &str = "/api/ai/admin/models/effort";

struct E {
    t: TestApp,
    admin: Who,
    user: Who,
}

async fn env() -> E {
    let t = start().await;
    let admin = t.user("root", true, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    t.post(
        "/api/ai/admin/limits",
        &admin,
        json!({"genUnlimited": 0, "importUnlimited": 0}),
    )
    .await;
    let user = t.user("lee", false, true);
    t.backend.st().gen = kitchen(2, 20);
    t.stub.st().script = Some(good_model());
    t.app.0.knobs.backoff_scale.store(0, SeqCst);
    t.app.0.knobs.place_poll_ms.store(20, SeqCst);
    t.app.0.knobs.import_context_tokens.store(1, SeqCst);
    *t.app.0.knobs.exe_override.lock().unwrap() =
        Some(env!("CARGO_BIN_EXE_xexam-ai-service").into());
    t.relax_caps();
    t.add_question(single_question("q1", "Which tool fits the made-up job?"));
    t.add_record(FakeRecord {
        user: user.id,
        id: 1,
        at: "2026-11-12 09:30".into(),
        kind: "practice".into(),
        key: None,
        started_at: None,
        detail: true,
        items: vec![FakeItem {
            qid: "q1".into(),
            answer: json!(["A"]),
            correct: false,
        }],
    });
    E { t, admin, user }
}

async fn settings(t: &TestApp, admin: &Who) -> Value {
    let (s, v) = t.get("/api/ai/admin/settings", admin).await;
    assert_eq!(s, 200, "{v}");
    v
}

async fn set(e: &E, slot: &str, family: &str, level: &str) {
    let (s, v) =
        e.t.post(
            EFFORT,
            &e.admin,
            json!({"slot": slot, "effort": {"family": family, "level": level}}),
        )
        .await;
    assert_eq!(s, 200, "{slot}: {v}");
}

/// One distinct level per slot, so a call that used the wrong slot's row shows.
const ROWS: [(&str, &str, &str); 6] = [
    ("default", "openai", "max"),
    ("chat", "openai", "high"),
    ("study", "google", "LOW"),
    ("summary", "openai", "xhigh"),
    ("generate", "openai", "medium"),
    ("import", "google", "HIGH"),
];

async fn set_all(e: &E) {
    for (slot, family, level) in ROWS {
        set(e, slot, family, level).await;
    }
}

fn level_of(slot: &str) -> &'static str {
    ROWS.iter().find(|r| r.0 == slot).expect("slot").2
}

/// The model calls the stub saw (not the model listings), from index `from`.
fn calls_from(t: &TestApp, from: usize) -> Vec<Value> {
    t.stub
        .requests()
        .into_iter()
        .skip(from)
        .filter(|r| !r.path.ends_with("/models"))
        .map(|r| r.body)
        .collect()
}

fn n_requests(t: &TestApp) -> usize {
    t.stub.count()
}

fn q1<T: rusqlite::types::FromSql>(t: &TestApp, sql: &str) -> T {
    t.db().query_row(sql, [], |r| r.get(0)).expect(sql)
}

fn all_carry(bodies: &[Value], level: &str) {
    assert!(!bodies.is_empty(), "no model call");
    for b in bodies {
        // OpenAI / Anthropic-style bodies carry reasoning_effort; Google / Gemma carry extra_body thinking_level
        let got = b["reasoning_effort"]
            .as_str()
            .or_else(|| b["reasoning"]["effort"].as_str())
            .or_else(|| b["extra_body"]["google"]["thinking_config"]["thinking_level"].as_str())
            .unwrap_or_else(|| panic!("no thinking parameter: {b}"));
        assert!(got.eq_ignore_ascii_case(level), "{got} vs {level}: {b}");
    }
}

// —— the admin route ——————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn effort_save_round_trip() {
    let e = env().await;
    let v = settings(&e.t, &e.admin).await;
    assert_eq!(
        v["effortLevels"],
        json!({
            "anthropic": ["low", "medium", "high", "xhigh", "max"],
            "openai": ["low", "medium", "high", "xhigh", "max"],
            "google": ["MINIMAL", "LOW", "MEDIUM", "HIGH", "XHIGH"],
            "responses": ["none", "minimal", "low", "medium", "high", "xhigh", "max"],
            "gemma": ["HIGH", "MINIMAL"],
        })
    );
    for slot in ["default", "chat", "study", "summary", "generate", "import"] {
        assert_eq!(v["slots"][slot]["effort"], Value::Null, "{slot}");
    }
    let (s, v) =
        e.t.post(
            EFFORT,
            &e.admin,
            json!({"slot": "chat", "effort": {"family": "openai", "level": "high"}}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        v["settings"]["slots"]["chat"]["effort"],
        json!({"family": "openai", "level": "high"})
    );
    let v = settings(&e.t, &e.admin).await;
    assert_eq!(
        v["slots"]["chat"]["effort"],
        json!({"family": "openai", "level": "high"})
    );
    for slot in ["default", "study", "summary", "generate", "import"] {
        assert_eq!(v["slots"][slot]["effort"], Value::Null, "{slot}");
    }
    // a family change and the reset to the default
    set(&e, "chat", "anthropic", "max").await;
    let v = settings(&e.t, &e.admin).await;
    assert_eq!(
        v["slots"]["chat"]["effort"],
        json!({"family": "anthropic", "level": "max"})
    );
    let (s, v) =
        e.t.post(EFFORT, &e.admin, json!({"slot": "chat", "effort": null}))
            .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["settings"]["slots"]["chat"]["effort"], Value::Null);
}

#[tokio::test]
async fn effort_save_rejects_bad_input() {
    let e = env().await;
    let bad = [
        json!({"slot": "chat", "effort": {"family": "mistral", "level": "high"}}),
        json!({"slot": "chat", "effort": {"family": "anthropic", "level": "none"}}),
        json!({"slot": "chat", "effort": {"family": "google", "level": "max"}}),
        json!({"slot": "chat", "effort": {"family": "responses", "level": "turbo"}}),
        json!({"slot": "chat", "effort": {"family": "responses", "level": "HIGH"}}),
        json!({"slot": "chat", "effort": {"family": "deepseek", "level": "high"}}),
        json!({"slot": "chat", "effort": {"family": "gemma", "level": "high"}}),
        json!({"slot": "chat", "effort": {"family": "gemma", "level": "true"}}),
        json!({"slot": "chat", "effort": {"family": "gemma", "level": "false"}}),
        json!({"slot": "chat", "effort": {"family": "google", "level": "low"}}),
        json!({"slot": "chat", "effort": {"family": "google", "level": "none"}}),
        json!({"slot": "chat", "effort": {"family": "openai", "level": "none"}}),
        json!({"slot": "chat", "effort": {"family": "openai", "level": "minimal"}}),
        json!({"slot": "chat", "effort": {"family": "openai", "level": "High"}}),
        json!({"slot": "chat", "effort": {"family": "openai", "level": " high"}}),
        json!({"slot": "chat", "effort": {"family": "openai", "level": 3}}),
        json!({"slot": "chat", "effort": {"family": "openai"}}),
        json!({"slot": "chat", "effort": {"family": "openai", "level": "high", "x": 1}}),
        json!({"slot": "chat", "effort": {"family": "openai", "level": "high"}, "extra": 1}),
        json!({"slot": "nope", "effort": null}),
        json!({"slot": 7, "effort": null}),
        json!({"effort": null}),
        json!({"slot": "chat"}),
        json!({"slot": "chat", "effort": "high"}),
        json!({"slot": "chat", "effort": "openai:high"}),
    ];
    for b in bad {
        let (s, v) = e.t.post(EFFORT, &e.admin, b.clone()).await;
        assert_eq!(s, 400, "{b}: {v}");
    }
    let stored: i64 = q1(
        &e.t,
        "SELECT COUNT(*) FROM model_slots WHERE effort IS NOT NULL",
    );
    assert_eq!(stored, 0, "nothing was stored");
}

#[tokio::test]
async fn effort_save_ignores_connection_format() {
    let e = env().await;
    // the connection is OpenAI-compatible: the Anthropic family is still accepted and stored
    set(&e, "chat", "anthropic", "xhigh").await;
    let v = settings(&e.t, &e.admin).await;
    assert_eq!(
        v["slots"]["chat"]["effort"],
        json!({"family": "anthropic", "level": "xhigh"})
    );
    // and an Anthropic-format connection takes the OpenAI family
    let e2 = env().await;
    e2.t.configure_anthropic(&e2.admin, "demo-claude").await;
    set(&e2, "chat", "openai", "low").await;
    let v = settings(&e2.t, &e2.admin).await;
    assert_eq!(
        v["slots"]["chat"]["effort"],
        json!({"family": "openai", "level": "low"})
    );
}

#[tokio::test]
async fn effort_save_is_admin_only() {
    let e = env().await;
    set(&e, "chat", "openai", "high").await;
    let (s, _) =
        e.t.post(
            EFFORT,
            &e.user,
            json!({"slot": "chat", "effort": {"family": "openai", "level": "low"}}),
        )
        .await;
    assert_eq!(s, 403);
    let (s, _) = e.t.get("/api/ai/admin/settings", &e.user).await;
    assert_eq!(s, 403);
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert!(
        !v.to_string().contains("effort"),
        "the status of a user never names it: {v}"
    );
    let (_, v) = e.t.get("/api/ai/status", &e.admin).await;
    assert!(!v.to_string().contains("effort"), "{v}");
    let tag: String = q1(&e.t, "SELECT effort FROM model_slots WHERE slot = 'chat'");
    assert_eq!(tag, "openai:high", "the refused save changed nothing");
}

#[tokio::test]
async fn responses_new_levels_round_trip() {
    let e = env().await;
    for lvl in ["minimal", "xhigh"] {
        set(&e, "chat", "responses", lvl).await;
        let tag: String = q1(&e.t, "SELECT effort FROM model_slots WHERE slot = 'chat'");
        assert_eq!(tag, format!("responses:{lvl}"));
        let v = settings(&e.t, &e.admin).await;
        assert_eq!(
            v["slots"]["chat"]["effort"],
            json!({"family": "responses", "level": lvl})
        );
    }
}

#[tokio::test]
async fn responses_effort_round_trip_and_wire() {
    let e = env().await;
    set(&e, "chat", "responses", "max").await;
    let tag: String = q1(&e.t, "SELECT effort FROM model_slots WHERE slot = 'chat'");
    assert_eq!(tag, "responses:max");
    let v = settings(&e.t, &e.admin).await;
    assert_eq!(
        v["slots"]["chat"]["effort"],
        json!({"family": "responses", "level": "max"})
    );
    let from = n_requests(&e.t);
    let ev =
        e.t.send_all(&e.user, json!({"text": "hello", "lang": "en"}))
            .await
            .expect("send");
    assert_eq!(terminal(&ev).0, "done", "{ev:?}");
    let bodies = calls_from(&e.t, from);
    all_carry(&bodies, "max");
    for b in &bodies {
        assert!(!b.to_string().contains("\"thinking\""), "{b}");
    }
}

#[tokio::test]
async fn google_and_gemma_effort_round_trip_and_wire() {
    let e = env().await;
    for (fam, lvl) in [
        ("gemma", "HIGH"),
        ("gemma", "MINIMAL"),
        ("google", "MEDIUM"),
        ("google", "XHIGH"),
    ] {
        set(&e, "chat", fam, lvl).await;
        let tag: String = q1(&e.t, "SELECT effort FROM model_slots WHERE slot = 'chat'");
        assert_eq!(tag, format!("{fam}:{lvl}"));
        let v = settings(&e.t, &e.admin).await;
        assert_eq!(
            v["slots"]["chat"]["effort"],
            json!({"family": fam, "level": lvl})
        );
        let from = n_requests(&e.t);
        let ev =
            e.t.send_all(&e.user, json!({"text": "hello", "lang": "en"}))
                .await
                .expect("send");
        assert_eq!(terminal(&ev).0, "done", "{ev:?}");
        let bodies = calls_from(&e.t, from);
        assert!(!bodies.is_empty());
        for b in &bodies {
            assert_eq!(
                b["extra_body"],
                json!({"google": {"thinking_config": {"thinking_level": lvl}}}),
                "{b}"
            );
            assert!(b.get("reasoning_effort").is_none(), "{b}");
            assert!(b.get("thinking_level").is_none(), "{b}");
            let s = b.to_string();
            assert!(
                !s.contains("chat_template_kwargs") && !s.contains("enable_thinking"),
                "{b}"
            );
        }
    }
}

#[tokio::test]
async fn legacy_stored_levels_read_in_the_new_form() {
    let e = env().await;
    set(&e, "chat", "openai", "high").await;
    for (old, want) in [
        ("google:low", json!({"family": "google", "level": "LOW"})),
        ("gemma:true", json!({"family": "gemma", "level": "HIGH"})),
        (
            "gemma:false",
            json!({"family": "gemma", "level": "MINIMAL"}),
        ),
        (
            "deepseek:max",
            json!({"family": "responses", "level": "max"}),
        ),
        (
            "deepseek:medium",
            json!({"family": "responses", "level": "medium"}),
        ),
        ("deepseek:turbo", Value::Null),
        ("google:none", Value::Null),
        ("openai:minimal", Value::Null),
        ("openai:none", Value::Null),
    ] {
        e.t.db()
            .execute(
                "UPDATE model_slots SET effort = ?1 WHERE slot = 'chat'",
                [old],
            )
            .expect("update");
        let v = settings(&e.t, &e.admin).await;
        assert_eq!(v["slots"]["chat"]["effort"], want, "{old}");
    }
}

#[tokio::test]
async fn unnamed_google_refusal_retries_without_thinking_level() {
    let e = env().await;
    set(&e, "chat", "gemma", "MINIMAL").await;
    e.t.stub.refuse_if_body_has(Some((
        "thinking_level",
        400,
        r#"{"errors":[{"message":"AiError: AiError: Invalid input (0b1f)","code":8001}]}"#,
    )));
    let from = n_requests(&e.t);
    let ev =
        e.t.send_all(&e.user, json!({"text": "hello", "lang": "en"}))
            .await
            .expect("send");
    assert_eq!(terminal(&ev).0, "done", "{ev:?}");
    let bodies = calls_from(&e.t, from);
    assert_eq!(bodies.len(), 2, "the refused request and one retry");
    assert!(bodies[0].get("extra_body").is_some());
    assert!(bodies[1].get("extra_body").is_none());
}

#[tokio::test]
async fn effort_save_writes_activity_event() {
    let e = env().await;
    set(&e, "summary", "google", "MINIMAL").await;
    let (s, _) =
        e.t.post(EFFORT, &e.admin, json!({"slot": "summary", "effort": null}))
            .await;
    assert_eq!(s, 200);
    e.t.app.0.deliver_outbox().await;
    let evs: Vec<Value> =
        e.t.backend
            .st()
            .activity
            .iter()
            .filter(|v| v["action"] == json!("ai.settings.effort"))
            .cloned()
            .collect();
    assert_eq!(evs.len(), 2, "{evs:?}");
    assert_eq!(evs[0]["object"], "summary");
    assert_eq!(evs[0]["userId"], json!(e.admin.id));
    assert_eq!(
        evs[0]["detail"],
        json!({"family": "google", "level": "MINIMAL"})
    );
    assert_eq!(evs[1]["object"], "summary");
    assert_eq!(evs[1]["detail"], json!({"family": "default"}));
}

#[tokio::test]
async fn effort_and_model_pick_are_independent() {
    let e = env().await;
    set(&e, "chat", "openai", "high").await;
    // a pick of a model keeps the effort
    let (s, v) =
        e.t.post(
            "/api/ai/admin/models/pick",
            &e.admin,
            json!({"slot": "chat", "connectionId": 1, "model": "demo-chat-large"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], true, "{v}");
    let v = settings(&e.t, &e.admin).await;
    assert_eq!(
        v["slots"]["chat"]["effort"],
        json!({"family": "openai", "level": "high"})
    );
    let model_id = v["slots"]["chat"]["modelId"].clone();
    assert!(model_id.is_i64(), "{v}");
    // an effort save keeps the model
    set(&e, "chat", "openai", "low").await;
    let v = settings(&e.t, &e.admin).await;
    assert_eq!(v["slots"]["chat"]["modelId"], model_id);
    // clearing the slot keeps the effort
    let (s, _) =
        e.t.post(
            "/api/ai/admin/models/pick",
            &e.admin,
            json!({"slot": "chat", "clear": true}),
        )
        .await;
    assert_eq!(s, 200);
    let v = settings(&e.t, &e.admin).await;
    assert_eq!(v["slots"]["chat"]["modelId"], Value::Null);
    assert_eq!(
        v["slots"]["chat"]["effort"],
        json!({"family": "openai", "level": "low"})
    );
}

// —— what the calls of a feature send —————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn chat_sends_chat_slot_effort() {
    let e = env().await;
    set_all(&e).await;
    // the chat slot has no model of its own: the Default slot's model is used, with the Chat row's strength
    let from = n_requests(&e.t);
    let ev =
        e.t.send_all(&e.user, json!({"text": "hello", "lang": "en"}))
            .await
            .expect("send");
    assert_eq!(terminal(&ev).0, "done");
    all_carry(&calls_from(&e.t, from), level_of("chat"));
    let start = &ev[0].1;
    let chat = start["chatId"].as_i64().expect("chat");
    let answer = start["assistantMessageId"].as_i64().expect("answer");
    // regenerate
    let from = n_requests(&e.t);
    let mut s =
        e.t.open_stream(
            "/api/ai/regenerate",
            &e.user,
            json!({"chatId": chat, "messageId": answer, "lang": "en"}),
        )
        .await
        .expect("regenerate");
    let ev = s.all().await;
    assert_eq!(terminal(&ev).0, "done");
    all_carry(&calls_from(&e.t, from), level_of("chat"));
    // an administrator's model pick (free mode) still carries the Chat row's strength
    let (c, v) =
        e.t.post("/api/ai/free", &e.admin, json!({"free": true}))
            .await;
    assert_eq!(c, 200, "{v}");
    let from = n_requests(&e.t);
    let ev =
        e.t.send_all(
            &e.admin,
            json!({"text": "hello", "lang": "en",
                   "model": {"connectionId": 1, "model": "demo-chat-large"}}),
        )
        .await
        .expect("send with a pick");
    assert_eq!(terminal(&ev).0, "done", "{ev:?}");
    assert!(ev[0].1.get("modelFallback").is_none(), "{:?}", ev[0].1);
    let bodies = calls_from(&e.t, from);
    assert!(bodies.iter().all(|b| b["model"] == "demo-chat-large"));
    all_carry(&bodies, level_of("chat"));
}

#[tokio::test]
async fn memory_suggest_sends_chat_slot_effort() {
    let e = env().await;
    set_all(&e).await;
    let ev =
        e.t.send_all(
            &e.user,
            json!({"text": "I like short answers", "lang": "en"}),
        )
        .await
        .expect("send");
    let msg = ev[0].1["userMessageId"].as_i64().expect("message");
    let from = n_requests(&e.t);
    let (s, v) =
        e.t.post(
            "/api/ai/memories/suggest",
            &e.user,
            json!({"messageId": msg, "lang": "en"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    all_carry(&calls_from(&e.t, from), level_of("chat"));
}

fn hint_body(kind: &str) -> Value {
    json!({"kind": kind, "mode": "practice", "key": "0123456789abcdef0123456789abcdef",
           "startedAt": xexam_ai_service::util::unix_now() - 3600, "questionId": "q1", "no": 1,
           "qlang": "en", "source": "practice", "lang": "en"})
}

#[tokio::test]
async fn hint_and_explain_send_study_slot_effort() {
    let e = env().await;
    set_all(&e).await;
    for kind in ["hint", "explain"] {
        let from = n_requests(&e.t);
        let ev =
            e.t.open_stream("/api/ai/study/ask", &e.user, hint_body(kind))
                .await
                .expect(kind)
                .all()
                .await;
        assert_eq!(terminal(&ev).0, "done", "{kind}");
        all_carry(&calls_from(&e.t, from), level_of("study"));
    }
}

#[tokio::test]
async fn summary_sends_summary_slot_effort() {
    let e = env().await;
    set_all(&e).await;
    let from = n_requests(&e.t);
    let ev =
        e.t.open_stream(
            "/api/ai/summary/ask",
            &e.user,
            json!({"recordId": 1, "lang": "en"}),
        )
        .await
        .expect("summary")
        .all()
        .await;
    assert_eq!(terminal(&ev).0, "done");
    all_carry(&calls_from(&e.t, from), level_of("summary"));
}

async fn run_generation(e: &E) -> Value {
    let (s, v) =
        e.t.post(
            "/api/ai/gen/start",
            &e.user,
            json!({"source": "bank", "bankId": 3}),
        )
        .await;
    assert_eq!(s, 202, "{v}");
    let id = v["job"]["id"].as_i64().expect("id");
    for _ in 0..800 {
        let (s, v) = e.t.get(&format!("/api/ai/gen/job?id={id}"), &e.user).await;
        assert_eq!(s, 200, "{v}");
        let state = v["job"]["state"].as_str().unwrap_or("");
        if ["done", "failed", "cancelled"].contains(&state) {
            return v["job"].clone();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the job never ended");
}

#[tokio::test]
async fn generation_plan_and_writing_send_generate_slot_effort() {
    let e = env().await;
    set_all(&e).await;
    let from = n_requests(&e.t);
    let done = run_generation(&e).await;
    assert_eq!(done["state"], "done", "{done}");
    let bodies = calls_from(&e.t, from);
    assert!(bodies.len() >= 5, "the plan call and the writing calls");
    all_carry(&bodies, level_of("generate"));
    let plans: i64 = q1(
        &e.t,
        "SELECT COUNT(*) FROM gen_calls WHERE purpose = 'plan'",
    );
    assert_eq!(plans, 1);
}

#[tokio::test]
async fn import_calls_send_import_slot_effort() {
    use xexam_ai_service::extract::testgen::{self, DocxSpec};
    let e = env().await;
    set_all(&e).await;
    let mut lines = vec!["KIND:QUESTIONS zebrafishmarker notes".to_string()];
    for i in 1..=4 {
        lines.push(format!(
            "Q{i}. Which kitchen device {} keeps a stock pot at a steady simmer?",
            en_text(300 + i, 3)
        ));
        for (k, l) in ["A", "B", "C", "D"].iter().enumerate() {
            lines.push(format!(
                "{l}. {} {}",
                pw(500 + i * 10 + k),
                pw(600 + i * 10 + k)
            ));
        }
        lines.push("Answer: B".to_string());
    }
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let from = n_requests(&e.t);
    let (s, v) =
        e.t.upload(
            &e.user,
            "notes.docx",
            testgen::docx(&DocxSpec::paragraphs(&refs)),
        )
        .await;
    assert_eq!(s, 202, "{v}");
    let id = v["job"]["id"].as_i64().expect("id");
    let mut phase = Value::Null;
    for _ in 0..800 {
        let (_, v) = e.t.get(&format!("/api/ai/gen/job?id={id}"), &e.user).await;
        phase = v["job"]["phase"].clone();
        let state = v["job"]["state"].as_str().unwrap_or("");
        if phase == "confirm" || ["failed", "cancelled"].contains(&state) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(phase, "confirm");
    all_carry(&calls_from(&e.t, from), level_of("import"));
    // the confirmed import runs its writing calls with the same row
    let from = n_requests(&e.t);
    let (s, v) =
        e.t.post(
            "/api/ai/import/confirm",
            &e.user,
            json!({"id": id, "name": "Kitchen notes (typed)"}),
        )
        .await;
    assert!(s == 200 || s == 202, "{s} {v}");
    for _ in 0..800 {
        let (_, v) = e.t.get(&format!("/api/ai/gen/job?id={id}"), &e.user).await;
        if ["done", "failed", "cancelled"].contains(&v["job"]["state"].as_str().unwrap_or("")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    all_carry(&calls_from(&e.t, from), level_of("import"));
}

#[tokio::test]
async fn default_row_sends_nothing() {
    let e = env().await;
    // rows set and reset: back to the default sends nothing; an untouched install never sent anything
    let from = n_requests(&e.t);
    let ev =
        e.t.send_all(&e.user, json!({"text": "hello", "lang": "en"}))
            .await
            .expect("send");
    assert_eq!(terminal(&ev).0, "done");
    set(&e, "chat", "anthropic", "high").await;
    let (s, _) =
        e.t.post(EFFORT, &e.admin, json!({"slot": "chat", "effort": null}))
            .await;
    assert_eq!(s, 200);
    let ev =
        e.t.send_all(&e.user, json!({"text": "again", "lang": "en"}))
            .await
            .expect("send");
    assert_eq!(terminal(&ev).0, "done");
    let bodies = calls_from(&e.t, from);
    assert!(bodies.len() >= 2);
    for b in bodies {
        let s = b.to_string();
        assert!(
            !s.contains("reasoning_effort")
                && !s.contains("output_config")
                && !s.contains("effort"),
            "{b}"
        );
    }
}

#[tokio::test]
async fn connection_and_model_test_send_no_effort() {
    let e = env().await;
    set_all(&e).await;
    let from = n_requests(&e.t);
    let (s, v) =
        e.t.post("/api/ai/admin/connections/test", &e.admin, json!({"id": 1}))
            .await;
    assert_eq!(s, 200, "{v}");
    let (s, v) =
        e.t.post(
            "/api/ai/admin/models/test",
            &e.admin,
            json!({"connectionId": 1, "model": "demo-chat-small"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let bodies = calls_from(&e.t, from);
    assert!(!bodies.is_empty(), "the tests call the model");
    for b in bodies {
        assert!(!b.to_string().contains("effort"), "{b}");
    }
}

#[tokio::test]
async fn anthropic_effort_and_schema_share_output_config() {
    let e = env().await;
    e.t.configure_anthropic(&e.admin, "demo-claude").await;
    set(&e, "generate", "anthropic", "xhigh").await;
    let from = n_requests(&e.t);
    let done = run_generation(&e).await;
    assert_eq!(done["state"], "done", "{done}");
    let bodies = calls_from(&e.t, from);
    assert!(!bodies.is_empty());
    for b in &bodies {
        assert_eq!(b["output_config"]["effort"], "xhigh", "{b}");
        assert!(b.get("reasoning_effort").is_none(), "{b}");
    }
    assert!(
        bodies
            .iter()
            .any(|b| b["output_config"]["format"]["type"] == "json_schema"),
        "a structured-output call keeps its format next to the effort"
    );
}

// —— the fallback —————————————————————————————————————————————————————————————————————————————————————————————————————

const NAMED: &str = r#"{"error":{"message":"Unrecognized request argument supplied: reasoning_effort","type":"invalid_request_error","param":"reasoning_effort"}}"#;

#[tokio::test]
async fn effort_refusal_retries_once_without_and_remembers() {
    let e = env().await;
    set(&e, "chat", "openai", "high").await;
    e.t.stub
        .refuse_if_body_has(Some(("reasoning_effort", 400, NAMED)));
    let before = captured_count("refused reasoning_effort");
    let from = n_requests(&e.t);
    let ev =
        e.t.send_all(&e.user, json!({"text": "hello", "lang": "en"}))
            .await
            .expect("send");
    assert_eq!(terminal(&ev).0, "done", "{ev:?}");
    let bodies = calls_from(&e.t, from);
    assert_eq!(bodies.len(), 2, "the refused request and one retry");
    assert_eq!(bodies[0]["reasoning_effort"], "high");
    assert!(bodies[1].get("reasoning_effort").is_none());
    assert_eq!(
        captured_count("refused reasoning_effort"),
        before + 1,
        "one INFO line"
    );
    // remembered: the next call sends none and needs no retry
    let from = n_requests(&e.t);
    let ev =
        e.t.send_all(&e.user, json!({"text": "again", "lang": "en"}))
            .await
            .expect("send");
    assert_eq!(terminal(&ev).0, "done");
    let bodies = calls_from(&e.t, from);
    assert_eq!(bodies.len(), 1);
    assert!(bodies[0].get("reasoning_effort").is_none());
    assert_eq!(captured_count("refused reasoning_effort"), before + 1);
    // the line holds no key, URL, model or text
    for l in captured()
        .iter()
        .filter(|l| l.contains("refused reasoning_effort"))
    {
        for bad in ["sk-demo", "127.0.0.1", "demo-chat", "http"] {
            assert!(!l.contains(bad), "{l}");
        }
    }
    // a new level is tried again (and refused again, once)
    set(&e, "chat", "openai", "low").await;
    let from = n_requests(&e.t);
    let ev =
        e.t.send_all(&e.user, json!({"text": "third", "lang": "en"}))
            .await
            .expect("send");
    assert_eq!(terminal(&ev).0, "done");
    let bodies = calls_from(&e.t, from);
    assert_eq!(bodies.len(), 2, "tried again after a save");
    assert_eq!(bodies[0]["reasoning_effort"], "low");
    assert!(bodies[1].get("reasoning_effort").is_none());
    // when the provider accepts it again, a save lets the parameter through
    e.t.stub.refuse_if_body_has(None);
    set(&e, "chat", "openai", "medium").await;
    let from = n_requests(&e.t);
    let ev =
        e.t.send_all(&e.user, json!({"text": "fourth", "lang": "en"}))
            .await
            .expect("send");
    assert_eq!(terminal(&ev).0, "done");
    let bodies = calls_from(&e.t, from);
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0]["reasoning_effort"], "medium");
}

#[tokio::test]
async fn unnamed_refusal_is_retried_without_effort() {
    let e = env().await;
    set(&e, "chat", "openai", "high").await;
    e.t.stub.refuse_if_body_has(Some((
        "reasoning_effort",
        400,
        r#"{"error":{"message":"The request could not be processed."}}"#,
    )));
    let from = n_requests(&e.t);
    let ev =
        e.t.send_all(&e.user, json!({"text": "hello", "lang": "en"}))
            .await
            .expect("send");
    assert_eq!(terminal(&ev).0, "done", "{ev:?}");
    let bodies = calls_from(&e.t, from);
    assert_eq!(bodies.len(), 2, "one retry without the parameter");
    assert!(bodies[1].get("reasoning_effort").is_none());
}

#[tokio::test]
async fn responses_refusal_retries_without_reasoning_object() {
    let e = env().await;
    set(&e, "chat", "responses", "low").await;
    e.t.stub.refuse_if_body_has(Some((
        "reasoning",
        400,
        r#"{"error":{"message":"Unknown parameter: 'reasoning'.","type":"invalid_request_error","param":"reasoning"}}"#,
    )));
    let from = n_requests(&e.t);
    let ev =
        e.t.send_all(&e.user, json!({"text": "hello", "lang": "en"}))
            .await
            .expect("send");
    assert_eq!(terminal(&ev).0, "done", "{ev:?}");
    let bodies = calls_from(&e.t, from);
    assert_eq!(bodies.len(), 2, "the refused request and one retry");
    assert_eq!(bodies[0]["reasoning"], json!({"effort": "low"}));
    assert!(bodies[1].get("reasoning").is_none(), "{}", bodies[1]);
    assert!(bodies[1].get("reasoning_effort").is_none());
}

#[tokio::test]
async fn generic_schema_refusal_does_not_drop_thinking_strength() {
    // a 400 that is about the structured output (not the thinking strength) is answered by the unformatted ladder; that
    // success must not turn the thinking strength off for the route
    let e = env().await;
    set(&e, "generate", "openai", "high").await;
    e.t.stub.refuse_if_body_has(Some((
        "response_format",
        400,
        r#"{"errors":[{"message":"AiError: AiError: Invalid input (7c2e)","code":8001}]}"#,
    )));
    let done = run_generation(&e).await;
    assert_eq!(done["state"], "done", "{done}");
    // the next job: no schema any more (remembered), and the thinking strength still goes out
    let from = n_requests(&e.t);
    let done = run_generation(&e).await;
    assert_eq!(done["state"], "done", "{done}");
    let bodies = calls_from(&e.t, from);
    assert!(!bodies.is_empty());
    for b in &bodies {
        assert!(b.get("response_format").is_none(), "{b}");
    }
    all_carry(&bodies, "high");
}

// —— the column ———————————————————————————————————————————————————————————————————————————————————————————————————————

fn has_effort_column(path: &std::path::Path) -> bool {
    let c = rusqlite::Connection::open(path).expect("open");
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('model_slots') WHERE name = 'effort')",
        [],
        |r| r.get(0),
    )
    .expect("pragma")
}

fn schema_of(path: &std::path::Path) -> String {
    let c = rusqlite::Connection::open(path).expect("open");
    c.query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
        r.get(0)
    })
    .expect("schema")
}

#[test]
fn new_file_has_model_slots_effort() {
    let dir = tempfile::tempdir().expect("dir");
    let store = Store::open(dir.path(), JournalMode::Delete).expect("open");
    drop(store);
    let path = dir.path().join(DB_FILE);
    assert!(has_effort_column(&path));
    assert_eq!(schema_of(&path), "8");
    let c = rusqlite::Connection::open(&path).expect("open");
    let n: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM model_slots WHERE effort IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(n, 6, "every seeded slot starts at the default");
}

#[test]
fn schema_8_file_without_effort_column_gains_it() {
    let dir = tempfile::tempdir().expect("dir");
    drop(Store::open(dir.path(), JournalMode::Delete).expect("open"));
    let path = dir.path().join(DB_FILE);
    {
        let c = rusqlite::Connection::open(&path).expect("open");
        c.execute_batch("ALTER TABLE model_slots DROP COLUMN effort;")
            .expect("drop");
    }
    assert!(!has_effort_column(&path));
    drop(Store::open(dir.path(), JournalMode::Delete).expect("reopen"));
    assert!(has_effort_column(&path));
    assert_eq!(schema_of(&path), "8", "no version change");
    let c = rusqlite::Connection::open(&path).expect("open");
    let slots: i64 = c
        .query_row("SELECT COUNT(*) FROM model_slots", [], |r| r.get(0))
        .expect("count");
    let nulls: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM model_slots WHERE effort IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!((slots, nulls), (6, 6), "the slot rows read as the default");
}

#[test]
fn upgrade_from_7_adds_effort_column() {
    let dir = tempfile::tempdir().expect("dir");
    drop(Store::open(dir.path(), JournalMode::Delete).expect("open"));
    let path = dir.path().join(DB_FILE);
    {
        let c = rusqlite::Connection::open(&path).expect("open");
        c.execute_batch("ALTER TABLE model_slots DROP COLUMN effort;")
            .expect("drop");
        c.execute("UPDATE meta SET value = '7' WHERE key = 'schema'", [])
            .expect("downgrade the marker");
    }
    drop(Store::open(dir.path(), JournalMode::Delete).expect("upgrade"));
    assert!(has_effort_column(&path));
    assert_eq!(schema_of(&path), "8");
}

#[test]
fn new_and_repaired_files_have_the_same_sqlite_master() {
    let master = |path: &std::path::Path| -> Vec<(String, String, Option<String>)> {
        let c = rusqlite::Connection::open(path).expect("open");
        let mut st = c
            .prepare("SELECT type, name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name")
            .expect("prepare");
        let rows = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .expect("query");
        rows.map(|r| r.expect("row")).collect()
    };
    let a = tempfile::tempdir().expect("dir");
    drop(Store::open(a.path(), JournalMode::Delete).expect("open"));
    let b = tempfile::tempdir().expect("dir");
    drop(Store::open(b.path(), JournalMode::Delete).expect("open"));
    let pb = b.path().join(DB_FILE);
    {
        let c = rusqlite::Connection::open(&pb).expect("open");
        c.execute_batch(
            "ALTER TABLE model_slots DROP COLUMN effort; ALTER TABLE connections DROP COLUMN responses;",
        )
        .expect("drop");
    }
    drop(Store::open(b.path(), JournalMode::Delete).expect("repair"));
    assert_eq!(master(&a.path().join(DB_FILE)), master(&pb));
}
