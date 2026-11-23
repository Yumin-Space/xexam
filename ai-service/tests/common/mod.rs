//! Shared test helpers: a fake main backend (`/internal/ai/*`), a stub AI provider (OpenAI-compatible, Anthropic and
//! gateway routes), the app under test, signed identities, and a log capture. All content is made up.
#![allow(dead_code)]

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::Response;
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use tower::ServiceExt;
use xexam_ai_service::app::{App, AppState};
use xexam_ai_service::config::{AiUserAgent, Config, JournalMode, Timeouts};
use xexam_ai_service::peers::PeerList;
use zeroize::Zeroizing;

pub mod genkit;
pub mod geokit;

/// The stub's answer (made-up text).
pub const ANSWER: &str =
    "Here is a **short answer** with a list:\n- one\n- two\n- three\nUse `x + 1` to check.";
/// Provider key used by the tests (obviously fake).
pub const KEY: &str = "sk-demo-0000-1234";
/// Gateway key used by the tests (obviously fake).
pub const GW_KEY: &str = "gw-demo-0000-5678";
/// Internal token of the tests.
pub const TOKEN: &str = "fake-internal-token-for-tests-0123456789abcdef";

// —— log capture ——————————————————————————————————————————————————————————————————————————————————————————————————————

static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let line = format!(
            "{} {}: {}",
            event.metadata().level(),
            event.metadata().target(),
            xexam_ai_service::logs::event_message(event)
        );
        LINES.lock().unwrap_or_else(|e| e.into_inner()).push(line);
    }
}

/// Installs the test subscriber once per test binary: the production filter at `trace` (so the forced `warn`
/// directives are exercised) feeding a capture, plus the production forward layer.
pub fn init_logs() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        use tracing_subscriber::filter::{filter_fn, FilterExt};
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        use tracing_subscriber::Layer;
        let (filter, _) = xexam_ai_service::logs::env_filter(Some("trace"));
        let _ = tracing_subscriber::registry()
            .with(Capture.with_filter(filter.and(filter_fn(xexam_ai_service::logs::not_quiet))))
            .with(
                xexam_ai_service::logs::ForwardLayer::global().with_filter(filter_fn(|m| {
                    xexam_ai_service::logs::forwarded(m.target(), m.level())
                })),
            )
            .try_init();
    });
}

/// Every captured line so far.
pub fn captured() -> Vec<String> {
    LINES.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn captured_contains(s: &str) -> bool {
    captured().iter().any(|l| l.contains(s))
}

pub fn captured_count(s: &str) -> usize {
    captured().iter().filter(|l| l.contains(s)).count()
}

async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = l.local_addr().expect("addr");
    let h = tokio::spawn(async move {
        let _ = axum::serve(l, router).await;
    });
    (format!("http://{addr}"), h)
}

async fn body_json(req: Request) -> Value {
    let bytes = axum::body::to_bytes(req.into_body(), 1 << 22)
        .await
        .unwrap_or_default();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn json_answer(status: u16, v: &Value) -> Response {
    let mut r = Response::new(Body::from(serde_json::to_vec(v).unwrap_or_default()));
    *r.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
    r.headers_mut()
        .insert("content-type", "application/json".parse().expect("hv"));
    r
}

fn text_answer(status: u16, s: &str) -> Response {
    let mut r = Response::new(Body::from(s.to_string()));
    *r.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
    r
}

// —— fake main backend ———————————————————————————————————————————————————————————————————————————————————————————————

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeUser {
    pub id: i64,
    pub stamp: String,
    pub username: String,
    pub role: String,
    pub disabled: bool,
    pub ai: bool,
}

impl FakeUser {
    pub fn ai_allowed(&self) -> bool {
        self.role == "admin" || self.ai
    }

    fn json(&self) -> Value {
        json!({"id": self.id, "stamp": self.stamp, "username": self.username, "role": self.role,
               "disabled": self.disabled, "ai": self.ai, "aiAllowed": self.ai_allowed()})
    }
}

/// How the fake backend answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Status500,
    NonJson,
    ExtraField,
    WrongType,
    Hang,
    Reject,
}

pub struct BackendState {
    pub users: BTreeMap<i64, FakeUser>,
    pub sessions: HashMap<String, i64>,
    pub mode: Mode,
    pub log_fail: bool,
    pub logs: Vec<(String, String)>,
    /// The events of the accepted `/internal/ai/activity` batches, oldest first
    pub activity: Vec<Value>,
    /// Batches received on that route (accepted or not)
    pub activity_batches: usize,
    /// Status the activity route answers with (200 accepts); 400 / 404 / 500 refuse the whole batch
    pub activity_status: u16,
    /// A batch that holds an event with this `action` is refused with 400 (a "bad event")
    pub activity_bad_action: Option<String>,
    pub calls: Vec<String>,
    pub token: String,
    /// Bodies of the accepted heartbeats (`POST /internal/ai/heartbeat`), oldest first
    pub heartbeats: Vec<Value>,
    /// Raw bodies of every heartbeat request that reached the handler (accepted or not)
    pub heartbeat_raw: Vec<String>,
    /// Answer heartbeats with 500
    pub heartbeat_fail: bool,
    /// Delay before a heartbeat is answered (ms)
    pub heartbeat_delay_ms: u64,
    /// Heartbeats being answered right now, and the most seen at once
    pub heartbeat_in_flight: usize,
    pub heartbeat_max_in_flight: usize,
    /// Made-up questions of `/internal/ai/question` (by id)
    pub questions: HashMap<String, FakeQuestion>,
    /// Made-up saved records of `/internal/ai/record` and `record-info` (by id)
    pub records: BTreeMap<i64, FakeRecord>,
    /// How the three study data routes answer
    pub study_switch: StudySwitch,
    /// The `uiLang` of every study answer
    pub ui_lang: String,
    /// The local day `/internal/ai/assess` reports as today (v2.5)
    pub assess_today: String,
    /// The generation and import routes
    pub gen: genkit::GenBackend,
    /// The external IP-location routes (v1.4)
    pub geo: geokit::GeoBackend,
}

/// A made-up question and its solution, as JSON of the wire (backend contracts section 3).
#[derive(Debug, Clone)]
pub struct FakeQuestion {
    pub question: Value,
    pub solution: Value,
}

/// One saved item of a made-up record.
#[derive(Debug, Clone)]
pub struct FakeItem {
    pub qid: String,
    /// `null` = not answered
    pub answer: Value,
    pub correct: bool,
}

/// A made-up saved record owned by a user.
#[derive(Debug, Clone)]
pub struct FakeRecord {
    pub user: i64,
    pub id: i64,
    pub at: String,
    pub kind: String,
    pub key: Option<String>,
    pub started_at: Option<i64>,
    pub detail: bool,
    pub items: Vec<FakeItem>,
}

/// How the study data routes of the fake backend answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StudySwitch {
    Normal,
    /// 404 plain text (a backend without the route)
    Http404,
    /// 200 with a body that is not JSON
    Unreadable,
    /// a good answer with one extra field
    ExtraField,
    NotFound,
    Session,
}

/// A single-choice question with four made-up options; the correct one is B ("Bravo tool").
pub fn single_question(id: &str, stem: &str) -> FakeQuestion {
    FakeQuestion {
        question: json!({"id": id, "kind": "single", "lang": "en", "stem": stem,
            "options": [{"key": "A", "text": "Alpha tool"}, {"key": "B", "text": "Bravo tool"},
                        {"key": "C", "text": "Charlie tool"}, {"key": "D", "text": "Delta tool"}],
            "items": [], "active": true, "cut": false}),
        solution: json!({"answer": ["B"], "explanation": "Bravo is the made-up tool for this job.",
            "mnemonic": "", "note": "", "explainLang": "en", "keyTexts": ["Bravo tool"]}),
    }
}

/// A statements question: three statements with Yes / No (the key is Yes, No, Yes).
pub fn hotspot_question(id: &str) -> FakeQuestion {
    FakeQuestion {
        question: json!({"id": id, "kind": "hotspot", "lang": "en", "stem": "Review the made-up statements.",
            "options": [],
            "items": [{"text": "The made-up gadget is red", "choices": ["Yes", "No"]},
                      {"text": "The made-up gadget is loud", "choices": ["Yes", "No"]},
                      {"text": "The made-up gadget is small", "choices": ["Yes", "No"]}],
            "active": true, "cut": false}),
        solution: json!({"answer": [0, 1, 0], "explanation": "", "mnemonic": "", "note": "",
            "explainLang": "", "keyTexts": ["Yes", "No", "Yes"]}),
    }
}

pub struct FakeBackend {
    pub url: String,
    pub state: Arc<Mutex<BackendState>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeBackend {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeBackend {
    pub fn st(&self) -> std::sync::MutexGuard<'_, BackendState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_mode(&self, m: Mode) {
        self.st().mode = m;
    }

    pub fn logs(&self) -> Vec<String> {
        self.st().logs.iter().map(|(_, m)| m.clone()).collect()
    }

    pub fn log_contains(&self, s: &str) -> bool {
        self.logs().iter().any(|l| l.contains(s))
    }

    /// The accepted heartbeat bodies so far.
    pub fn heartbeats(&self) -> Vec<Value> {
        self.st().heartbeats.clone()
    }

    /// The last accepted heartbeat body.
    pub fn last_heartbeat(&self) -> Value {
        self.st().heartbeats.last().cloned().expect("a heartbeat")
    }
}

/// The heartbeat body rules of the main backend (round-2 contracts 2.2): exactly the four keys, in this order.
fn heartbeat_ok(v: &Value) -> bool {
    let Some(o) = v.as_object() else {
        return false;
    };
    let keys: Vec<&str> = o.keys().map(String::as_str).collect();
    keys == ["version", "enabled", "suspended", "stopping"]
        && o["version"].as_str().is_some_and(|s| !s.trim().is_empty())
        && o["enabled"].is_boolean()
        && o["stopping"].is_boolean()
        && o["suspended"]
            .as_array()
            .is_some_and(|a| a.len() <= 3000 && a.iter().all(|x| x.as_i64().is_some_and(|n| n > 0)))
}

async fn backend_handler(State(st): State<Arc<Mutex<BackendState>>>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    let token_ok = {
        let s = st.lock().unwrap_or_else(|e| e.into_inner());
        req.headers()
            .get("x-xexam-internal-token")
            .and_then(|v| v.to_str().ok())
            == Some(s.token.as_str())
    };
    let ct_ok = req
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        == Some("application/json");
    let raw = axum::body::to_bytes(req.into_body(), 1 << 22)
        .await
        .unwrap_or_default();
    let body: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    let mode = {
        let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
        s.calls.push(path.clone());
        s.mode
    };
    if !token_ok || !ct_ok || mode == Mode::Reject {
        return text_answer(404, "not found");
    }
    if path == "/internal/ai/heartbeat" {
        return heartbeat_answer(&st, mode, &raw, body).await;
    }
    if path == "/internal/ai/geo/next" && mode == Mode::Normal {
        return geokit::next_answer(&st, &body).await;
    }
    match mode {
        Mode::Status500 => return text_answer(500, "boom"),
        Mode::NonJson => return text_answer(200, "<html>nope</html>"),
        Mode::Hang => {
            tokio::time::sleep(Duration::from_secs(30)).await;
            return text_answer(200, "{}");
        }
        _ => {}
    }
    let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
    let mut answer = match path.as_str() {
        "/internal/ai/introspect" => {
            let sid = body.get("sid").and_then(Value::as_str).unwrap_or("");
            let id = body.get("id").and_then(Value::as_i64).unwrap_or(0);
            match s.sessions.get(sid).and_then(|uid| s.users.get(uid)) {
                Some(u) if u.id == id && !u.disabled => json!({"ok": true, "user": {
                    "id": u.id, "stamp": u.stamp, "username": u.username, "role": u.role, "aiAllowed": u.ai_allowed()}}),
                _ => json!({"ok": false}),
            }
        }
        "/internal/ai/users" => {
            let ids: Vec<i64> = body
                .get("ids")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default();
            let holders = body
                .get("holders")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let rows: Vec<Value> = s
                .users
                .values()
                .filter(|u| ids.contains(&u.id) || (holders && u.ai_allowed()))
                .map(FakeUser::json)
                .collect();
            json!({ "users": rows })
        }
        "/internal/ai/activity" => {
            s.activity_batches += 1;
            let events = body
                .get("events")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if s.activity_status != 200 {
                return text_answer(s.activity_status, "refused");
            }
            if let Some(bad) = &s.activity_bad_action {
                if events
                    .iter()
                    .any(|e| e.get("action").and_then(Value::as_str) == Some(bad.as_str()))
                {
                    return text_answer(400, "bad event");
                }
            }
            s.activity.extend(events);
            json!({"ok": true})
        }
        "/internal/ai/log" => {
            if s.log_fail {
                return text_answer(500, "log down");
            }
            for l in body
                .get("lines")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                let level = l
                    .get("level")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let msg = l
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                s.logs.push((level, msg));
            }
            json!({"ok": true})
        }
        "/internal/ai/question"
        | "/internal/ai/record"
        | "/internal/ai/record-info"
        | "/internal/ai/assess" => {
            match s.study_switch {
                StudySwitch::Http404 => return text_answer(404, "not found"),
                StudySwitch::Unreadable => return text_answer(200, "<html>nope</html>"),
                _ => {}
            }
            let mut a = study_answer(&s, &path, &body);
            if s.study_switch == StudySwitch::ExtraField {
                if let Some(o) = a.as_object_mut() {
                    o.insert("extra".into(), json!(1));
                }
            }
            a
        }
        p if p.starts_with("/internal/ai/geo/") => match geokit::answer(&mut s, p, &body) {
            Ok(v) => v,
            Err(code) => return text_answer(code, "no"),
        },
        p if p.starts_with("/internal/ai/gen/") || p == "/internal/ai/import/check" => {
            match genkit::answer(&mut s, p, &body) {
                Ok(v) => v,
                Err(code) => return text_answer(code, "no"),
            }
        }
        _ => return text_answer(404, "not found"),
    };
    match mode {
        Mode::ExtraField => {
            if let Some(o) = answer.as_object_mut() {
                o.insert("extra".into(), json!(1));
            }
        }
        Mode::WrongType => {
            if let Some(o) = answer.as_object_mut() {
                if o.contains_key("ok") {
                    o.insert("ok".into(), json!("yes"));
                }
                if o.contains_key("users") {
                    o.insert("users".into(), json!([{"id": "1"}]));
                }
            }
        }
        _ => {}
    }
    json_answer(200, &answer)
}

fn refusal(reason: &str) -> Value {
    json!({"ok": false, "reason": reason})
}

fn ident_json(r: &FakeRecord) -> Value {
    json!({"id": r.id, "at": r.at, "kind": r.kind, "key": r.key, "startedAt": r.started_at})
}

/// The three study data routes of the fake backend (the contracts' section 3), from the made-up data.
fn study_answer(s: &BackendState, path: &str, body: &Value) -> Value {
    let sid = body.get("sid").and_then(Value::as_str).unwrap_or("");
    let id = body.get("id").and_then(Value::as_i64).unwrap_or(0);
    let user = s
        .sessions
        .get(sid)
        .and_then(|uid| s.users.get(uid))
        .filter(|u| u.id == id && !u.disabled);
    let Some(u) = user else {
        return refusal("session");
    };
    match s.study_switch {
        StudySwitch::NotFound => return refusal("notFound"),
        StudySwitch::Session => return refusal("session"),
        _ => {}
    }
    let owned = |rid: i64| s.records.get(&rid).filter(|r| r.user == u.id);
    match path {
        "/internal/ai/record-info" => {
            let rid = body.get("recordId").and_then(Value::as_i64).unwrap_or(0);
            match owned(rid) {
                Some(r) => json!({"ok": true, "uiLang": s.ui_lang, "record": ident_json(r)}),
                None => refusal("notFound"),
            }
        }
        "/internal/ai/question" => {
            if !u.ai_allowed() {
                return refusal("notAllowed");
            }
            let qid = body.get("questionId").and_then(Value::as_str).unwrap_or("");
            let Some(q) = s.questions.get(qid) else {
                return refusal("notFound");
            };
            let mut question = q.question.clone();
            if let Some(l) = body.get("qlang").and_then(Value::as_str) {
                question["lang"] = json!(l);
            }
            let (item, record) = match body.get("record") {
                Some(Value::Object(r)) => {
                    let rid = r.get("id").and_then(Value::as_i64).unwrap_or(0);
                    let kind = r.get("kind").and_then(Value::as_str).unwrap_or("");
                    let Some(rec) = owned(rid).filter(|x| x.kind == kind) else {
                        return refusal("notFound");
                    };
                    let Some((seq, it)) = rec.items.iter().enumerate().find(|(_, i)| i.qid == qid)
                    else {
                        return refusal("notFound");
                    };
                    (
                        json!({"seq": seq, "answer": it.answer, "correct": it.correct}),
                        ident_json(rec),
                    )
                }
                _ => (Value::Null, Value::Null),
            };
            json!({"ok": true, "uiLang": s.ui_lang, "question": question, "solution": q.solution,
                   "item": item, "record": record})
        }
        "/internal/ai/assess" => {
            if !u.ai_allowed() {
                return refusal("notAllowed");
            }
            let mine: Vec<&FakeRecord> = s.records.values().filter(|r| r.user == u.id).collect();
            if mine.len() < 3 {
                return refusal("few");
            }
            let latest = mine.iter().map(|r| r.at.as_str()).max().unwrap_or("");
            let answered: usize = mine
                .iter()
                .map(|r| r.items.iter().filter(|i| !i.answer.is_null()).count())
                .sum();
            let correct: usize = mine
                .iter()
                .map(|r| r.items.iter().filter(|i| i.correct).count())
                .sum();
            let exams = mine.iter().filter(|r| r.kind == "exam").count();
            json!({"ok": true, "uiLang": s.ui_lang, "today": s.assess_today, "latestAt": latest, "basis": {
                "days": 30, "records": mine.len(), "practice": mine.len() - exams, "exams": exams,
                "answered": answered, "correct": correct, "unanswered": 0,
                "activeDays": 3, "daysSinceLast": 0, "longestGapDays": 4, "avgSecPerItem": 30, "medianMinutes": 10,
                "examAvgPercent": null, "timedExams": 0, "timeUps": 0,
                "weekdays": [1, 1, 1, 0, 0, 0, 0], "dayparts": [0, 1, 1, 1],
                "weeks": [{"answered": 0, "correct": 0}, {"answered": 0, "correct": 0},
                          {"answered": 0, "correct": 0}, {"answered": answered, "correct": correct}],
                "modes": {"all": mine.len()},
                "allTime": {"records": mine.len(), "seen": answered, "answers": answered, "correct": correct,
                            "notebookOpen": 0, "mastered": 0}}})
        }
        _ => {
            if !u.ai_allowed() {
                return refusal("notAllowed");
            }
            let rid = body.get("recordId").and_then(Value::as_i64).unwrap_or(0);
            let max = body.get("max").and_then(Value::as_u64).unwrap_or(70) as usize;
            let Some(rec) = owned(rid) else {
                return refusal("notFound");
            };
            let wrong_items: Vec<(usize, &FakeItem)> = rec
                .items
                .iter()
                .enumerate()
                .filter(|(_, i)| !i.answer.is_null() && !i.correct)
                .collect();
            let usable: Vec<&(usize, &FakeItem)> = wrong_items
                .iter()
                .filter(|(_, i)| s.questions.contains_key(&i.qid))
                .collect();
            let (wrong, usable_n) = if rec.detail {
                (wrong_items.len(), usable.len())
            } else {
                (0, 0)
            };
            let items: Vec<Value> = if rec.detail {
                usable
                    .iter()
                    .take(max)
                    .filter_map(|(seq, i)| {
                        s.questions.get(&i.qid).map(|q| {
                            json!({"seq": seq, "answer": i.answer, "question": q.question,
                                   "solution": {"answer": q.solution["answer"], "explanation": q.solution["explanation"],
                                                "mnemonic": "", "note": "", "explainLang": q.solution["explainLang"],
                                                "keyTexts": []}})
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let answered = rec.items.iter().filter(|i| !i.answer.is_null()).count();
            let correct = rec.items.iter().filter(|i| i.correct).count();
            json!({"ok": true, "uiLang": s.ui_lang,
                   "record": {"id": rec.id, "at": rec.at, "kind": rec.kind, "mode": "all", "total": rec.items.len(),
                              "correct": correct, "answered": answered, "detail": rec.detail,
                              "key": rec.key, "startedAt": rec.started_at},
                   "wrong": wrong, "usable": usable_n, "items": items, "budgetCut": false})
        }
    }
}

/// `POST /internal/ai/heartbeat` of the fake backend: records the body, answers `{"ok": true}` (or fails as told).
async fn heartbeat_answer(
    st: &Arc<Mutex<BackendState>>,
    mode: Mode,
    raw: &[u8],
    body: Value,
) -> Response {
    let delay = {
        let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
        s.heartbeat_raw
            .push(String::from_utf8_lossy(raw).into_owned());
        s.heartbeat_in_flight += 1;
        s.heartbeat_max_in_flight = s.heartbeat_max_in_flight.max(s.heartbeat_in_flight);
        s.heartbeat_delay_ms
    };
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    if mode == Mode::Hang {
        st.lock()
            .unwrap_or_else(|e| e.into_inner())
            .heartbeat_in_flight -= 1;
        tokio::time::sleep(Duration::from_secs(30)).await;
        return text_answer(200, "{}");
    }
    let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
    s.heartbeat_in_flight -= 1;
    match mode {
        Mode::Status500 => return text_answer(500, "boom"),
        Mode::NonJson => return text_answer(200, "<html>nope</html>"),
        _ => {}
    }
    if s.heartbeat_fail {
        return text_answer(500, "heartbeat down");
    }
    if !heartbeat_ok(&body) {
        return json_answer(400, &json!({"error": "请求无效。"}));
    }
    s.heartbeats.push(body);
    let answer = match mode {
        Mode::ExtraField => json!({"ok": true, "extra": 1}),
        Mode::WrongType => json!({"ok": "yes"}),
        _ => json!({"ok": true}),
    };
    json_answer(200, &answer)
}

pub async fn fake_backend() -> FakeBackend {
    let state = Arc::new(Mutex::new(BackendState {
        users: BTreeMap::new(),
        sessions: HashMap::new(),
        mode: Mode::Normal,
        log_fail: false,
        logs: Vec::new(),
        activity: Vec::new(),
        activity_batches: 0,
        activity_status: 200,
        activity_bad_action: None,
        calls: Vec::new(),
        token: TOKEN.to_string(),
        heartbeats: Vec::new(),
        heartbeat_raw: Vec::new(),
        heartbeat_fail: false,
        heartbeat_delay_ms: 0,
        heartbeat_in_flight: 0,
        heartbeat_max_in_flight: 0,
        questions: HashMap::new(),
        records: BTreeMap::new(),
        study_switch: StudySwitch::Normal,
        ui_lang: "en".to_string(),
        assess_today: "2026-11-22".to_string(),
        gen: genkit::GenBackend::default(),
        geo: geokit::GeoBackend::default(),
    }));
    let router = Router::new()
        .fallback(backend_handler)
        .with_state(state.clone());
    let (url, task) = serve(router).await;
    FakeBackend { url, state, task }
}

// —— stub AI provider —————————————————————————————————————————————————————————————————————————————————————————————————

#[derive(Debug, Clone)]
pub struct StubRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
    /// When the stub received the request
    pub at: Instant,
}

impl StubRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Default)]
pub struct StubState {
    pub requests: Vec<StubRequest>,
    /// Answer every chat request with this status and body
    pub force: Option<(u16, String)>,
    /// v1.9: answer the next chat requests (one entry each, front first) with this status, body and optional
    /// `retry-after` header (seconds), before `force` and the stub's own behaviour
    pub fail_next: std::collections::VecDeque<(u16, String, Option<String>)>,
    /// Refuse a chat request whose JSON body text contains the needle, with this status and body (v1.6)
    pub refuse_if_body_has: Option<(String, u16, String)>,
    /// Delay before the response headers (ms)
    pub head_delay_ms: u64,
    /// A scripted provider (generation and import tests): consulted before the stub's own behaviour
    pub script: Option<genkit::Script>,
    /// Input tokens the scripted answers report as read from the provider's prompt cache (`None` = no figure)
    pub cached_tokens: Option<i64>,
    /// Anthropic stub: cache-write tokens every answer reports
    pub cache_write_tokens: Option<i64>,
    /// v1.3: reasoning text every normal, study and scripted answer sends before its text (OpenAI: one
    /// `reasoning_content` delta, or an inline span with `reasoning_tag`; Anthropic: a thinking block)
    pub reasoning: Option<String>,
    /// v1.3: `"think"`, `"thinking"` or `"thought"`: with `reasoning`, the OpenAI content starts with
    /// `<tag>{reasoning}</tag>\n\n` instead of a reasoning field
    pub reasoning_tag: Option<&'static str>,
}

pub struct Stub {
    pub url: String,
    pub state: Arc<Mutex<StubState>>,
    pub hold: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Stub {
    pub fn st(&self) -> std::sync::MutexGuard<'_, StubState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn requests(&self) -> Vec<StubRequest> {
        self.st().requests.clone()
    }

    pub fn count(&self) -> usize {
        self.st().requests.len()
    }

    pub fn last(&self) -> StubRequest {
        self.st().requests.last().cloned().expect("a stub request")
    }

    /// Refuses every chat request whose body contains `needle` (`None` = stop refusing).
    pub fn refuse_if_body_has(&self, r: Option<(&str, u16, &str)>) {
        self.st().refuse_if_body_has = r.map(|(n, s, b)| (n.to_string(), s, b.to_string()));
    }

    pub fn force(&self, f: Option<(u16, &str)>) {
        self.st().force = f.map(|(s, b)| (s, b.to_string()));
    }

    /// Lets every held answer continue.
    pub fn release(&self) {
        self.hold.notify_waiters();
    }
}

#[derive(Clone)]
struct StubShared {
    state: Arc<Mutex<StubState>>,
    hold: Arc<Notify>,
}

const WRONG_KEY_BODY: &str = r#"{"error":{"message":"Incorrect API key provided: sk-de****1234","type":"invalid_request_error","code":"invalid_api_key"}}"#;

fn sse_body(rx: tokio::sync::mpsc::UnboundedReceiver<Bytes>) -> Response {
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|b| (Ok::<Bytes, std::convert::Infallible>(b), rx))
    });
    let mut r = Response::new(Body::from_stream(stream));
    r.headers_mut()
        .insert("content-type", "text/event-stream".parse().expect("hv"));
    r
}

fn chunks_of(text: &str, n: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let size = chars.len().div_ceil(n).max(1);
    chars.chunks(size).map(|c| c.iter().collect()).collect()
}

/// The configured reasoning of the stub as OpenAI chunks (none when no reasoning is set).
fn reasoning_chunks(sh: &StubShared) -> Vec<Bytes> {
    let (r, tag) = {
        let st = sh.state.lock().unwrap_or_else(|e| e.into_inner());
        (st.reasoning.clone(), st.reasoning_tag)
    };
    match (r, tag) {
        (None, _) => vec![],
        (Some(r), Some(tag)) => vec![openai_chunk(&format!("<{tag}>{r}</{tag}>\n\n"))],
        (Some(r), None) => vec![Bytes::from(format!(
            "data: {}\n\n",
            json!({"choices": [{"index": 0, "delta": {"reasoning_content": r}}]})
        ))],
    }
}

fn openai_chunk(content: &str) -> Bytes {
    Bytes::from(format!(
        "data: {}\n\n",
        json!({"choices": [{"index": 0, "delta": {"content": content}}]})
    ))
}

/// How often a held answer sends an SSE comment.
const HOLD_PING: Duration = Duration::from_millis(500);

/// Waits until `release()` (`held` must be enabled before the first text is sent), sending an SSE comment every
/// [`HOLD_PING`]: any body chunk resets the idle limit (spec 4.7), so a held answer stays open however slow the runner
/// is. `false` when the client went away.
async fn hold_with_pings(
    mut held: std::pin::Pin<&mut tokio::sync::futures::Notified<'_>>,
    tx: &tokio::sync::mpsc::UnboundedSender<Bytes>,
) -> bool {
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + HOLD_PING, HOLD_PING);
    loop {
        tokio::select! {
            _ = held.as_mut() => return true,
            _ = ping.tick() => {
                if tx.send(Bytes::from_static(b": ping\n\n")).is_err() {
                    return false;
                }
            }
        }
    }
}

async fn stub_handler(State(sh): State<StubShared>, req: Request) -> Response {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body = body_json(req).await;
    let (force, delay) = {
        let mut s = sh.state.lock().unwrap_or_else(|e| e.into_inner());
        s.requests.push(StubRequest {
            method: method.clone(),
            path: path.clone(),
            headers: headers.clone(),
            body: body.clone(),
            at: Instant::now(),
        });
        (s.force.clone(), s.head_delay_ms)
    };
    let h = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };
    if path.starts_with("/redirect") {
        let mut r = text_answer(302, "");
        r.headers_mut()
            .insert("location", "/v1/models".parse().expect("hv"));
        return r;
    }
    if path == "/nolist/v1/models" {
        return text_answer(404, "no such route");
    }
    let gw = path.starts_with("/gw/");
    let anthropic = path.starts_with("/anthropic/") || path.starts_with("/gw/anthropic/");
    // authentication
    let bearer = h("authorization").unwrap_or_default();
    let x_api = h("x-api-key").unwrap_or_default();
    let cf = h("cf-aig-authorization").unwrap_or_default();
    let auth_ok = if gw {
        cf.starts_with("Bearer gw-demo-")
            || bearer.starts_with("Bearer gw-demo-")
            || x_api.starts_with("gw-demo-")
    } else if anthropic {
        x_api.starts_with("sk-demo-") && h("anthropic-version").is_some()
    } else if path.starts_with("/open/") {
        true
    } else {
        bearer.starts_with("Bearer sk-demo-")
    };
    if !auth_ok {
        return json_answer(
            401,
            &serde_json::from_str(WRONG_KEY_BODY).unwrap_or(Value::Null),
        );
    }
    let is_models = path.ends_with("/models");
    if is_models {
        return if anthropic {
            json_answer(
                200,
                &json!({"data": [{"id": "demo-claude"}, {"id": "demo-4k"}, {"id": "demo-hold"}], "has_more": false}),
            )
        } else {
            json_answer(
                200,
                &json!({"object": "list", "data": [
                {"id": "demo-chat-small"}, {"id": "demo-chat-large"}, {"id": "demo-nousage"},
                {"id": "demo-error"}, {"id": "demo-length"}, {"id": "demo-chat-small"}]}),
            )
        };
    }
    let scripted_failure = sh
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .fail_next
        .pop_front();
    if let Some((status, b, retry_after)) = scripted_failure {
        let mut r = text_answer(status, &b);
        if let Some(v) = retry_after {
            r.headers_mut()
                .insert("retry-after", v.parse().expect("retry-after value"));
        }
        return r;
    }
    if let Some((status, b)) = force {
        return text_answer(status, &b);
    }
    let refusal = sh
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .refuse_if_body_has
        .clone();
    if let Some((needle, status, b)) = refusal {
        if body.to_string().contains(&needle) {
            return text_answer(status, &b);
        }
    }
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if anthropic {
        return anthropic_stream(sh, &model, &body);
    }
    if path.ends_with("/responses") {
        return responses_stream(sh, &model, &body);
    }
    openai_stream(sh, &model, &body)
}

/// One Responses API event (`event:` line plus a JSON `data:` line carrying the same `type`).
fn responses_event(data: Value) -> Bytes {
    Bytes::from(format!(
        "event: {}
data: {data}

",
        data["type"].as_str().unwrap_or("")
    ))
}

/// The last user item of a Responses body (`input` items with plain string content).
fn responses_user(body: &Value) -> String {
    body["input"]
        .as_array()
        .and_then(|m| m.iter().rev().find(|x| x["role"] == "user"))
        .and_then(|m| m["content"].as_str())
        .unwrap_or("")
        .to_string()
}

/// The Responses API stub: `instructions` + `input` in, `response.output_text.delta` events out, usage in the terminal
/// event. Models: `demo-cached` (8 cached tokens), `demo-length` (incomplete: max_output_tokens), `demo-nomax` (refuses
/// `max_output_tokens`); the scripted provider (`StubState::script`) answers like the other formats.
fn responses_stream(sh: StubShared, model: &str, body: &Value) -> Response {
    let system = body["instructions"].as_str().unwrap_or("").to_string();
    let user = responses_user(body);
    let verify = system == "Reply with the single word OK.";
    let (script, cached, reasoning) = {
        let st = sh.state.lock().unwrap_or_else(|e| e.into_inner());
        (st.script.clone(), st.cached_tokens, st.reasoning.clone())
    };
    let mut text = ANSWER.to_string();
    let mut cut = model == "demo-length";
    let mut cached = cached.or((model == "demo-cached").then_some(8));
    if let (Some(f), false) = (script, verify) {
        match f(&system, &user) {
            genkit::Reply::Text(t) => text = t,
            genkit::Reply::Cut(t) => {
                text = t;
                cut = true;
            }
            genkit::Reply::Status(code, b) => return text_answer(code, &b),
            genkit::Reply::Fall => {}
        }
    }
    if model == "demo-nomax" && body.get("max_output_tokens").is_some() && !verify {
        return json_answer(
            400,
            &json!({"error": {"message": "Unsupported parameter: 'max_output_tokens' is not supported with this model.",
                              "type": "invalid_request_error", "param": "max_output_tokens", "code": "unsupported_parameter"}}),
        );
    }
    if verify {
        cached = None;
    }
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();
    let _ = tx.send(responses_event(
        json!({"type": "response.created", "response": {"id": "resp_demo", "status": "in_progress"}}),
    ));
    if let Some(r) = reasoning {
        let _ = tx.send(responses_event(
            json!({"type": "response.output_item.added", "output_index": 0, "item": {"type": "reasoning", "id": "rs_demo"}}),
        ));
        let _ = tx.send(responses_event(
            json!({"type": "response.reasoning_summary_text.delta", "item_id": "rs_demo", "delta": r}),
        ));
    }
    for c in chunks_of(&text, 6) {
        let _ = tx.send(responses_event(
            json!({"type": "response.output_text.delta", "item_id": "msg_demo", "output_index": 1, "content_index": 0, "delta": c}),
        ));
    }
    let _ = tx.send(responses_event(
        json!({"type": "response.output_text.done", "item_id": "msg_demo", "text": text}),
    ));
    let mut usage = json!({"input_tokens": 21, "output_tokens": 13, "total_tokens": 34});
    if let Some(n) = cached {
        usage["input_tokens_details"] = json!({"cached_tokens": n});
    }
    let output = json!([{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": text}]}]);
    let end = if cut {
        json!({"type": "response.incomplete", "response": {"status": "incomplete", "incomplete_details": {"reason": "max_output_tokens"}, "output": output, "usage": usage}})
    } else {
        json!({"type": "response.completed", "response": {"status": "completed", "output": output, "usage": usage}})
    };
    let _ = tx.send(responses_event(end));
    sse_body(rx)
}

/// The test call of `POST /api/ai/admin/verify` (system prompt "Reply with the single word OK.").
fn is_verify(body: &Value) -> bool {
    let sys = body["messages"][0]["content"]
        .as_str()
        .or(body["system"].as_str())
        .unwrap_or("");
    sys == "Reply with the single word OK."
}

/// The text of the system prompt (a string for OpenAI-compatible bodies, text blocks for Anthropic bodies).
fn system_text(body: &Value) -> String {
    let sys = if body["messages"][0]["role"] == "system" {
        &body["messages"][0]["content"]
    } else {
        &body["system"]
    };
    match sys {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The last user turn of an OpenAI-compatible body.
fn user_turn(body: &Value) -> String {
    body["messages"]
        .as_array()
        .and_then(|m| m.iter().rev().find(|x| x["role"] == "user"))
        .and_then(|m| match &m["content"] {
            Value::String(s) => Some(s.clone()),
            // Anthropic: the text blocks of the turn (a cached prefix, then the task), joined like the OpenAI form
            Value::Array(a) => Some(
                a.iter()
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Study {
    Hint,
    Explain,
    Summary,
    /// The study assessment, in the language its system prompt asks for
    Assess(&'static str),
}

/// A clean made-up hint: no option letter, no option text, no verdict.
pub const CLEAN_HINT: &str = "First work out what the question is really asking for, then compare the ideas it mentions one by one.\n\nKnowledge points\n- recall the definitions that the topic relies on\n- note which rules apply in which situation\n";
/// The sentence a leaking stub hint ends with (a single-choice question / a statements question).
pub const LEAK_SINGLE: &str = " The answer is B.";
pub const LEAK_STATEMENT: &str = " Statement 2 is true.";
/// A clean hint longer than the hint limit (`[long]`).
pub fn long_hint() -> String {
    "Consider the context. ".repeat(400)
}

/// What the stub answers to a study prompt (by its system prompt): the text of the answer.
fn study_kind(body: &Value) -> Option<Study> {
    let sys = system_text(body);
    if sys.contains("You are a study coach inside Xexam") {
        Some(Study::Assess(if sys.contains("Chinese") {
            "zh"
        } else if sys.contains("Japanese") {
            "ja"
        } else {
            "en"
        }))
    } else if sys.contains("asked for a HINT") {
        Some(Study::Hint)
    } else if sys.contains("asked for an EXPLANATION") {
        Some(Study::Explain)
    } else if sys.contains("answered wrong") {
        Some(Study::Summary)
    } else {
        None
    }
}

fn study_text(kind: Study, user: &str) -> String {
    match kind {
        Study::Hint => {
            let mut t = if user.contains("[long]") {
                long_hint()
            } else {
                CLEAN_HINT.to_string()
            };
            if user.contains("[think]") {
                // inline reasoning of a server without a reasoning parser: it names the answer
                t = format!(
                    "<think>The first option is the right one, but I must not say so.</think>

{t}"
                );
            }
            if user.contains("[think-late]") {
                // reasoning after the text: the provider keeps it as text, so the hint guard must catch it by the tag
                // alone (the reasoning names no answer, so no other rule of the guard fires)
                t.push_str(" <think>Made-up reasoning.</think>");
            }
            if user.contains("[leak]") {
                t.push_str(if user.contains("statements, each") {
                    LEAK_STATEMENT
                } else {
                    LEAK_SINGLE
                });
            }
            t
        }
        Study::Explain => {
            let letters = user
                .split("<answer_key>")
                .nth(1)
                .and_then(|r| r.split("</answer_key>").next())
                .unwrap_or("?");
            let mut t = format!("**Answer: {letters}**\n\nThis question tests a made-up fact.\n\nThe other choices fail for made-up reasons.");
            if user.contains("[think-late]") {
                t.push_str("\n\n<think>Made-up reasoning.</think>");
            }
            t
        }
        // made-up, plain, 3 to 4 sentences: it passes the assessment checks (no counting, no drafts, ends with a mark)
        Study::Assess("zh") => "一滴水每天落进池塘，不知不觉就汇成了一片安静的湖。你最近完成了不少题目，大部分都答对了，节奏很稳。接下来可以抽一小段时间，把做错的题再看一遍。慢慢来，每一步都算数。".to_string(),
        Study::Assess("ja") => "毎日落ちる一滴の水が、いつか静かな湖になります。最近はたくさんの問題に取り組み、ほとんど正解できています。次は短い時間で、間違えた問題をもう一度見直してみましょう。無理のないペースで続けてください。".to_string(),
        Study::Assess(_) => "A small step taken each day adds up like drops filling a quiet pond. You have answered a good number of questions lately and most of them were right, which is a steady sign. Next week, spend a short session on the questions you missed. Go at your own pace and be kind to yourself.".to_string(),
        Study::Summary => {
            let n = user.matches("<item").count();
            format!("## Weak knowledge points\nMade-up topics.\n\n## What to watch out for\nMade-up traps.\n\n## How to approach such questions\nMade-up steps.\n\n## Summary of the knowledge points\nMade-up notes.\n\nItems: {n}")
        }
    }
}

/// The answer to a study prompt. Markers in the user turn: `[leak]` (the hint ends with a verdict), `[err]` (an
/// upstream error after the text), `[slow]` (waits for `release()` after the text), `[long]` (a hint over the limit), `[think]` (an inline <think> span first), `[think-late]` (a <think> span after the text).
fn study_openai(sh: StubShared, kind: Study, user: String, usage: bool) -> Response {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();
    tokio::spawn(async move {
        let text = study_text(kind, &user);
        let n = sh.hold.notified();
        tokio::pin!(n);
        n.as_mut().enable();
        let _ = tx.send(Bytes::from(format!(
            "data: {}\n\n",
            json!({"choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}}]})
        )));
        for c in reasoning_chunks(&sh) {
            let _ = tx.send(c);
        }
        let chars: Vec<char> = text.chars().collect();
        let size = if chars.len() > 1000 { 200 } else { 8 };
        for c in chars.chunks(size) {
            let piece: String = c.iter().collect();
            if tx.send(openai_chunk(&piece)).is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        if user.contains("[err]") {
            let _ = tx.send(Bytes::from(
                "data: {\"error\":{\"message\":\"boom\",\"type\":\"server_error\"}}\n\n",
            ));
            return;
        }
        // an assessment of exactly 4 saved sessions also waits (it cannot carry a marker)
        let slow = user.contains("[slow]") || user.contains("saved in the last 30 days: 4 (");
        if slow && !hold_with_pings(n, &tx).await {
            return;
        }
        let _ = tx.send(Bytes::from(format!(
            "data: {}\n\n",
            json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]})
        )));
        if usage {
            let _ = tx.send(Bytes::from(format!(
                "data: {}\n\n",
                json!({"choices": [], "usage": {"prompt_tokens": 21, "completion_tokens": 13}})
            )));
        }
        let _ = tx.send(Bytes::from("data: [DONE]\n\n"));
    });
    sse_body(rx)
}

/// A scripted answer as an OpenAI-compatible stream (usage included).
fn scripted_stream(
    sh: &StubShared,
    text: &str,
    cut: bool,
    cached: Option<i64>,
    written: Option<i64>,
) -> Response {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();
    for c in reasoning_chunks(sh) {
        let _ = tx.send(c);
    }
    let _ = tx.send(openai_chunk(text));
    let reason = if cut { "length" } else { "stop" };
    let _ = tx.send(Bytes::from(format!(
        "data: {}\n\n",
        json!({"choices": [{"index": 0, "delta": {}, "finish_reason": reason}]})
    )));
    let _ = tx.send(Bytes::from(format!(
        "data: {}\n\n",
        match (cached, written) {
            (Some(n), w) => json!({"choices": [], "usage": {"prompt_tokens": 21, "completion_tokens": 13, "prompt_tokens_details": {"cached_tokens": n, "cache_creation_tokens": w.unwrap_or(0)}}}),
            (None, _) => json!({"choices": [], "usage": {"prompt_tokens": 21, "completion_tokens": 13}}),
        }
    )));
    let _ = tx.send(Bytes::from("data: [DONE]\n\n"));
    sse_body(rx)
}

fn openai_stream(sh: StubShared, model: &str, body: &Value) -> Response {
    let script = sh
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .script
        .clone();
    let (cached, written) = {
        let st = sh.state.lock().unwrap_or_else(|e| e.into_inner());
        (st.cached_tokens, st.cache_write_tokens)
    };
    if let (Some(f), false) = (script, is_verify(body)) {
        match f(&system_text(body), &user_turn(body)) {
            genkit::Reply::Text(t) => return scripted_stream(&sh, &t, false, cached, written),
            genkit::Reply::Cut(t) => return scripted_stream(&sh, &t, true, cached, written),
            genkit::Reply::Status(code, b) => return text_answer(code, &b),
            genkit::Reply::Fall => {}
        }
    }
    let usage = body
        .get("stream_options")
        .and_then(|o| o.get("include_usage"))
        .and_then(Value::as_bool)
        == Some(true);
    if model == "demo-nousage" && body.get("stream_options").is_some() {
        return json_answer(
            400,
            &json!({"error": {"type": "invalid_request_error", "message": "unsupported"}}),
        );
    }
    // Models that refuse a token field (the token-limit ladder of C-8b)
    if !is_verify(body) {
        let refuse = |field: &str| {
            json_answer(
                400,
                &json!({"error": {"message": format!("Unsupported parameter: '{field}' is not supported with this model."),
                                  "type": "invalid_request_error", "param": field, "code": "unsupported_parameter"}}),
            )
        };
        if model == "demo-newparam" && body.get("max_tokens").is_some() {
            return refuse("max_tokens");
        }
        if model == "demo-notokens" {
            for f in ["max_tokens", "max_completion_tokens"] {
                if body.get(f).is_some() {
                    return refuse(f);
                }
            }
        }
    }
    if let Some(kind) = study_kind(body) {
        return study_openai(sh, kind, user_turn(body), usage);
    }
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();
    // The test call of verify is answered normally, whatever the model (except the two models above)
    let model = if is_verify(body) {
        "demo-chat-small".to_string()
    } else {
        model.to_string()
    };
    tokio::spawn(async move {
        let finish = |reason: &str| {
            Bytes::from(format!(
                "data: {}\n\n",
                json!({"choices": [{"index": 0, "delta": {}, "finish_reason": reason}]})
            ))
        };
        let usage_chunk = Bytes::from(format!(
            "data: {}\n\n",
            if model == "demo-cached" {
                json!({"choices": [], "usage": {"prompt_tokens": 21, "completion_tokens": 13, "prompt_tokens_details": {"cached_tokens": 8}}})
            } else {
                json!({"choices": [], "usage": {"prompt_tokens": 21, "completion_tokens": 13}})
            }
        ));
        match model.as_str() {
            "demo-error" => {
                let _ = tx.send(openai_chunk("Part"));
                let _ = tx.send(Bytes::from(
                    "data: {\"error\":{\"message\":\"boom\",\"type\":\"server_error\"}}\n\n",
                ));
                return;
            }
            "demo-silent" => {
                let _ = tx.send(Bytes::from(": waiting\n\n"));
                tokio::time::sleep(Duration::from_secs(60)).await;
                return;
            }
            "demo-big" => {
                let piece = "y".repeat(1000);
                for _ in 0..1005 {
                    if tx.send(openai_chunk(&piece)).is_err() {
                        return;
                    }
                }
                let _ = tx.send(Bytes::from("data: [DONE]\n\n"));
                return;
            }
            "demo-refuse" => {
                let _ = tx.send(finish("content_filter"));
                let _ = tx.send(Bytes::from("data: [DONE]\n\n"));
                return;
            }
            "demo-empty" => {
                let _ = tx.send(Bytes::from("data: [DONE]\n\n"));
                return;
            }
            "demo-crlf" => {
                // CRLF line ends, a multi-byte character split across two chunks
                let text = "Grüße";
                let line = format!(
                    "data: {}\r\n\r\n",
                    json!({"choices": [{"index": 0, "delta": {"content": text}}]})
                );
                let bytes = line.into_bytes();
                let cut = bytes.iter().position(|b| *b >= 0x80).map_or(5, |p| p + 1);
                let _ = tx.send(Bytes::copy_from_slice(&bytes[..cut]));
                tokio::time::sleep(Duration::from_millis(5)).await;
                let _ = tx.send(Bytes::copy_from_slice(&bytes[cut..]));
                let _ = tx.send(Bytes::from("data: [DONE]\r\n\r\n"));
                return;
            }
            "demo-hold" => {
                let n = sh.hold.notified();
                tokio::pin!(n);
                n.as_mut().enable();
                let _ = tx.send(openai_chunk("Hold on. "));
                if !hold_with_pings(n, &tx).await {
                    return;
                }
                if tx.send(openai_chunk("Done now.")).is_err() {
                    return;
                }
                if usage {
                    let _ = tx.send(usage_chunk);
                }
                let _ = tx.send(Bytes::from("data: [DONE]\n\n"));
                return;
            }
            _ => {}
        }
        let _ = tx.send(Bytes::from(format!(
            "data: {}\n\n",
            json!({"choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}}]})
        )));
        for c in reasoning_chunks(&sh) {
            let _ = tx.send(c);
        }
        for c in chunks_of(ANSWER, 6) {
            if tx.send(openai_chunk(&c)).is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        if model == "demo-length" {
            let _ = tx.send(finish("length"));
        } else {
            let _ = tx.send(finish("stop"));
        }
        if usage {
            let _ = tx.send(usage_chunk);
        }
        let _ = tx.send(Bytes::from("data: [DONE]\n\n"));
    });
    sse_body(rx)
}

fn anthropic_event(name: &str, data: Value) -> Bytes {
    Bytes::from(format!("event: {name}\ndata: {data}\n\n"))
}

/// The configured reasoning of the stub as an Anthropic thinking block at index 0 (none when no reasoning is set); the
/// text block then uses index 1. Returns the text block's index.
fn anthropic_reasoning(sh: &StubShared, tx: &tokio::sync::mpsc::UnboundedSender<Bytes>) -> u64 {
    let Some(r) = sh
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .reasoning
        .clone()
    else {
        return 0;
    };
    let _ = tx.send(anthropic_event(
        "content_block_start",
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": ""}}),
    ));
    let _ = tx.send(anthropic_event(
        "content_block_delta",
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": r}}),
    ));
    let _ = tx.send(anthropic_event(
        "content_block_delta",
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": "c2ln"}}),
    ));
    let _ = tx.send(anthropic_event(
        "content_block_stop",
        json!({"type": "content_block_stop", "index": 0}),
    ));
    1
}

fn anthropic_stream(sh: StubShared, model: &str, body: &Value) -> Response {
    let max = body.get("max_tokens").and_then(Value::as_i64).unwrap_or(0);
    if model == "demo-8k" && max > 8192 {
        return json_answer(
            400,
            &json!({"type": "error", "error": {"type": "invalid_request_error", "message": format!("max_tokens: {max} > 8192, which is the maximum allowed number of output tokens for demo-8k")}}),
        );
    }
    if model == "demo-4k" && max > 4096 {
        return json_answer(
            400,
            &json!({"type": "error", "error": {"type": "invalid_request_error", "message": "max_tokens: too large"}}),
        );
    }
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();
    let (script, cached, written) = {
        let st = sh.state.lock().unwrap_or_else(|e| e.into_inner());
        (st.script.clone(), st.cached_tokens, st.cache_write_tokens)
    };
    if let (Some(f), false) = (script, is_verify(body)) {
        if let genkit::Reply::Text(t) = f(&system_text(body), &user_turn(body)) {
            let _ = tx.send(anthropic_event(
                "message_start",
                json!({"type": "message_start", "message": {"usage": {"input_tokens": 21, "cache_read_input_tokens": cached.unwrap_or(0), "cache_creation_input_tokens": written.unwrap_or(0), "output_tokens": 1}}}),
            ));
            let idx = anthropic_reasoning(&sh, &tx);
            let _ = tx.send(anthropic_event(
                "content_block_start",
                json!({"type": "content_block_start", "index": idx, "content_block": {"type": "text", "text": ""}}),
            ));
            let _ = tx.send(anthropic_event(
                "content_block_delta",
                json!({"type": "content_block_delta", "index": idx, "delta": {"type": "text_delta", "text": t}}),
            ));
            let _ = tx.send(anthropic_event(
                "message_delta",
                json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 13}}),
            ));
            let _ = tx.send(anthropic_event(
                "message_stop",
                json!({"type": "message_stop"}),
            ));
            return sse_body(rx);
        }
    }
    let model = if is_verify(body) {
        "demo-claude".to_string()
    } else {
        model.to_string()
    };
    tokio::spawn(async move {
        let _ = tx.send(anthropic_event(
            "message_start",
            json!({"type": "message_start", "message": {"usage": {"input_tokens": 30, "cache_read_input_tokens": 2, "output_tokens": 1}}}),
        ));
        let _ = tx.send(anthropic_event("ping", json!({"type": "ping"})));
        let idx = anthropic_reasoning(&sh, &tx);
        let _ = tx.send(anthropic_event(
            "content_block_start",
            json!({"type": "content_block_start", "index": idx, "content_block": {"type": "text", "text": ""}}),
        ));
        let delta = |t: &str| {
            anthropic_event(
                "content_block_delta",
                json!({"type": "content_block_delta", "index": idx, "delta": {"type": "text_delta", "text": t}}),
            )
        };
        if model == "demo-hold" {
            let n = sh.hold.notified();
            tokio::pin!(n);
            n.as_mut().enable();
            let _ = tx.send(delta("Hold on. "));
            if !hold_with_pings(n, &tx).await {
                return;
            }
        }
        for c in chunks_of(ANSWER, 5) {
            if tx.send(delta(&c)).is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let _ = tx.send(anthropic_event(
            "content_block_stop",
            json!({"type": "content_block_stop", "index": idx}),
        ));
        let _ = tx.send(anthropic_event(
            "message_delta",
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 17}}),
        ));
        let _ = tx.send(anthropic_event(
            "message_stop",
            json!({"type": "message_stop"}),
        ));
    });
    sse_body(rx)
}

pub async fn stub() -> Stub {
    let state = Arc::new(Mutex::new(StubState::default()));
    let hold = Arc::new(Notify::new());
    let router = Router::new().fallback(stub_handler).with_state(StubShared {
        state: state.clone(),
        hold: hold.clone(),
    });
    let (url, task) = serve(router).await;
    Stub {
        url,
        state,
        hold,
        task,
    }
}

// —— the app under test ——————————————————————————————————————————————————————————————————————————————————————————————

/// A user of the tests.
#[derive(Debug, Clone)]
pub struct Who {
    pub id: i64,
    pub stamp: String,
    pub username: String,
    pub admin: bool,
    pub ai: bool,
    pub sid: String,
}

/// Time limits short enough for tests.
pub fn fast_timeouts() -> Timeouts {
    Timeouts {
        connect: Duration::from_secs(2),
        first_byte: Duration::from_secs(5),
        idle: Duration::from_secs(5),
        total: Duration::from_secs(30),
        short_total: Duration::from_secs(5),
        verify_total: Duration::from_secs(10),
        backend_connect: Duration::from_secs(1),
        backend_total: Duration::from_secs(2),
        access_interval: Duration::from_millis(200),
        flush_interval: Duration::from_millis(100),
        keep_alive: Duration::from_secs(15),
        heartbeat_every: Duration::from_secs(25),
        heartbeat_retry: Duration::from_millis(300),
        geo_total: Duration::from_secs(3),
    }
}

pub struct Opts {
    pub secret_key: Option<String>,
    pub timeouts: Timeouts,
    /// Token in `XEXAM_INTERNAL_TOKEN` (true) or in the link file (false)
    pub token_env: bool,
    /// What `XEXAM_AI_USER_AGENT_NAME` asks for (v1.9)
    pub ai_user_agent: AiUserAgent,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            secret_key: None,
            timeouts: fast_timeouts(),
            token_env: true,
            ai_user_agent: AiUserAgent::Default,
        }
    }
}

/// What `TestApp::seed_route` writes for connection 1.
pub struct Seed<'a> {
    pub provider: &'a str,
    pub url: String,
    pub api_key: Option<&'a str>,
    pub gateway_key: Option<&'a str>,
    pub gateway_auth: &'a str,
    pub model: &'a str,
}

pub struct TestApp {
    pub app: AppState,
    pub backend: FakeBackend,
    pub stub: Stub,
    pub dir: tempfile::TempDir,
    pub token: String,
    next_user: Mutex<i64>,
}

fn config(dir: &std::path::Path, backend: &str, opts: &Opts) -> Config {
    Config {
        data_dir: dir.join("ai"),
        host: "127.0.0.1".into(),
        port: 0,
        backend_url: backend.to_string(),
        link_dir: dir.join("link"),
        internal_token: opts.token_env.then(|| Zeroizing::new(TOKEN.to_string())),
        secret_key: opts.secret_key.clone().map(Zeroizing::new),
        log_filter: None,
        journal: JournalMode::Wal,
        ai_user_agent: opts.ai_user_agent.clone(),
        ai_user_agent_invalid: false,
        frontend_peers: PeerList::Any,
        peer_warnings: vec![],
        internal_token_required: false,
        version: "0.8".into(),
        timeouts: opts.timeouts,
    }
}

/// The configuration `start_with` uses for a data directory (tests that open `App` themselves).
pub fn test_config(dir: &std::path::Path, backend: &str, opts: &Opts) -> Config {
    config(dir, backend, opts)
}

pub async fn start() -> TestApp {
    start_with(Opts::default()).await
}

pub async fn start_with(opts: Opts) -> TestApp {
    init_logs();
    let backend = fake_backend().await;
    let stub = stub().await;
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("link")).expect("link dir");
    if !opts.token_env {
        std::fs::write(
            dir.path().join("link").join("internal.token"),
            format!("{TOKEN}\n"),
        )
        .expect("token");
    }
    let app = App::open(config(dir.path(), &backend.url, &opts)).expect("open");
    // v1.9: the tests do not pace generation / import calls (the pacer has its own tests)
    app.0
        .knobs
        .pace_scale
        .store(0, std::sync::atomic::Ordering::SeqCst);
    // The start report of the service (the heartbeat task is not running in these tests): the link is fresh
    app.0.heartbeat_now().await;
    TestApp {
        app,
        backend,
        stub,
        dir,
        token: TOKEN.to_string(),
        next_user: Mutex::new(100),
    }
}

/// Closes the AI database file before the temporary directory is removed, even while a clone of the service still
/// lives in a spawned task (on Windows an open file would make `TempDir` leave the directory behind silently).
impl Drop for TestApp {
    fn drop(&mut self) {
        self.app.0.begin_shutdown();
        self.app.0.store.release_file();
    }
}

impl TestApp {
    /// Reopens the service on the same data directory (another process start). The new instance reports to the
    /// fake backend at once, like a starting service.
    pub async fn restart(&mut self, opts: Opts) {
        let cfg = config(self.dir.path(), &self.backend.url, &opts);
        self.app = App::open(cfg).expect("reopen");
        self.app
            .0
            .knobs
            .pace_scale
            .store(0, std::sync::atomic::Ordering::SeqCst);
        self.app.0.heartbeat_now().await;
    }

    /// Reopens the service without a start report (the link is not fresh until a report is accepted).
    pub fn restart_without_report(&mut self, opts: Opts) {
        let cfg = config(self.dir.path(), &self.backend.url, &opts);
        self.app = App::open(cfg).expect("reopen");
        self.app
            .0
            .knobs
            .pace_scale
            .store(0, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn router(&self) -> Router {
        xexam_ai_service::api::build_router(self.app.clone())
    }

    /// Raises the per-user call caps to their maximum (in memory only), for tests that send many messages in a row.
    pub fn relax_caps(&self) {
        let mut l = self.app.0.limits();
        l.calls_per_minute = 100;
        l.calls_per_hour = 2000;
        l.calls_per_day = 20_000;
        self.app.0.set_limits(l);
    }

    /// A second connection to the AI database.
    pub fn db(&self) -> rusqlite::Connection {
        let c =
            rusqlite::Connection::open(self.dir.path().join("ai").join("xexam-ai.db")).expect("db");
        c.busy_timeout(Duration::from_secs(5))
            .expect("busy timeout");
        c
    }

    pub fn db_path(&self) -> PathBuf {
        self.dir.path().join("ai").join("xexam-ai.db")
    }

    /// Registers a user in the fake backend.
    pub fn user(&self, name: &str, admin: bool, ai: bool) -> Who {
        let id = {
            let mut n = self.next_user.lock().unwrap_or_else(|e| e.into_inner());
            *n += 1;
            *n
        };
        self.user_with_id(id, name, admin, ai)
    }

    pub fn user_with_id(&self, id: i64, name: &str, admin: bool, ai: bool) -> Who {
        let stamp = format!("{:032x}", 0xabc0_0000_u64 + id as u64 * 7919);
        let sid = format!("{:064x}", 0x5100_0000_u64 + id as u64);
        let who = Who {
            id,
            stamp: stamp.clone(),
            username: name.to_string(),
            admin,
            ai,
            sid: sid.clone(),
        };
        let mut st = self.backend.st();
        st.users.insert(
            id,
            FakeUser {
                id,
                stamp,
                username: name.to_string(),
                role: if admin { "admin" } else { "user" }.into(),
                disabled: false,
                ai,
            },
        );
        st.sessions.insert(sid, id);
        who
    }

    /// Changes AI+ of a user in the fake backend (as the backend would after `POST /api/users/update`).
    pub fn set_ai(&self, who: &Who, ai: bool) {
        if let Some(u) = self.backend.st().users.get_mut(&who.id) {
            u.ai = ai;
        }
    }

    pub fn delete_backend_user(&self, who: &Who) {
        self.backend.st().users.remove(&who.id);
    }

    pub fn ident(&self, who: &Who) -> String {
        self.ident_with(who, xexam_ai_service::util::unix_now() + 60, &self.token)
    }

    pub fn ident_with(&self, who: &Who, exp: i64, token: &str) -> String {
        let payload = json!({
            "v": 1, "id": who.id, "stamp": who.stamp, "username": who.username,
            "role": if who.admin { "admin" } else { "user" },
            "aiAllowed": who.admin || who.ai, "sid": who.sid, "exp": exp,
        });
        xexam_ai_service::identity::sign(&payload.to_string(), token.as_bytes())
    }

    /// Sends a request through the router.
    pub async fn raw(
        &self,
        method: Method,
        path: &str,
        ident: Option<String>,
        body: Option<Vec<u8>>,
        content_type: Option<&str>,
    ) -> Response {
        let mut b = Request::builder().method(method).uri(path);
        if let Some(i) = ident {
            b = b.header("x-xexam-identity", i);
        }
        if let Some(ct) = content_type {
            b = b.header("content-type", ct);
        }
        let req = b
            .body(Body::from(body.unwrap_or_default()))
            .expect("request");
        self.router().oneshot(req).await.expect("response")
    }

    pub async fn get(&self, path: &str, who: &Who) -> (u16, Value) {
        let r = self
            .raw(Method::GET, path, Some(self.ident(who)), None, None)
            .await;
        read_json(r).await
    }

    pub async fn post(&self, path: &str, who: &Who, body: Value) -> (u16, Value) {
        let r = self
            .raw(
                Method::POST,
                path,
                Some(self.ident(who)),
                Some(body.to_string().into_bytes()),
                Some("application/json"),
            )
            .await;
        read_json(r).await
    }

    /// Opens a streaming answer; returns the status and the body (or the JSON error).
    pub async fn open_stream(
        &self,
        path: &str,
        who: &Who,
        body: Value,
    ) -> Result<SseReader, (u16, Value)> {
        let r = self
            .raw(
                Method::POST,
                path,
                Some(self.ident(who)),
                Some(body.to_string().into_bytes()),
                Some("application/json"),
            )
            .await;
        let ct = r
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if r.status() != StatusCode::OK || !ct.starts_with("text/event-stream") {
            return Err(read_json(r).await);
        }
        Ok(SseReader::new(r))
    }

    /// Sends and reads the whole stream.
    pub async fn send_all(
        &self,
        who: &Who,
        body: Value,
    ) -> Result<Vec<(String, Value)>, (u16, Value)> {
        let mut s = self.open_stream("/api/ai/send", who, body).await?;
        Ok(s.all().await)
    }

    /// Writes connection 1 (the one connection of the tests), its sealed keys, the pool model and the default slot straight
    /// into the AI database and turns AI+ on. v1.3 has no single-settings route any more; the connection routes have
    /// their own tests. `url` is normalised like an administrator's input (an Anthropic address loses a trailing `/v1`).
    pub fn seed_route(&self, seed: Seed<'_>) {
        self.seed_route_as(seed, None);
    }

    /// `seed_route` with the connection format given by hand (`Some("responses")`; a URL never infers it).
    pub fn seed_route_as(&self, seed: Seed<'_>, format_override: Option<&str>) {
        use xexam_ai_service::secret::{SecretName, SecretRef};
        use xexam_ai_service::settings::{infer_format, normalize_url, strip_v1, Provider};
        use xexam_ai_service::store::conn::{self, ConnChange, NewConn, NewConnSecret, NewPool};
        let app = &self.app.0;
        let master = app.master().expect("the tests run with an encryption key");
        let mut url = normalize_url(&seed.url).expect("a valid address");
        let provider = Provider::parse(seed.provider).expect("provider");
        if provider == Provider::Anthropic {
            url = strip_v1(&url);
        }
        let format = match provider {
            Provider::OpenAi => "openai",
            Provider::Anthropic => "anthropic",
            Provider::Gateway => format_override.unwrap_or_else(|| infer_format(&url).as_str()),
        };
        let now = xexam_ai_service::util::unix_now();
        // the thinking-strength family follows the provider and format like the server's inference on create
        let family = conn::infer_family(seed.provider, format, &[seed.model]);
        app.store
            .write(|tx| -> rusqlite::Result<()> {
                let id = match conn::connection(tx, 1)? {
                    Some(c) => {
                        conn::update_connection(
                            tx,
                            1,
                            c.rev,
                            &ConnChange {
                                name: &c.name,
                                provider: seed.provider,
                                base_url: &url,
                                format,
                                gateway_auth: seed.gateway_auth,
                                lists_models: true,
                                effort_family: family.as_str(),
                                by: "test",
                                now,
                            },
                        )?;
                        conn::set_connection_status(tx, 1, "ok", None, Some(now))?;
                        1
                    }
                    None => conn::insert_connection(
                        tx,
                        &NewConn {
                            name: "main",
                            provider: seed.provider,
                            base_url: &url,
                            format,
                            gateway_auth: seed.gateway_auth,
                            lists_models: true,
                            effort_family: family.as_str(),
                            by: "test",
                            now,
                        },
                    )?,
                };
                for (name, value) in [
                    (SecretName::ApiKey, seed.api_key),
                    (SecretName::GatewayKey, seed.gateway_key),
                ] {
                    match value {
                        Some(v) => {
                            let sealed = master.seal(&SecretRef::new(id, name), v.as_bytes());
                            conn::put_conn_secret(
                                tx,
                                id,
                                &NewConnSecret {
                                    name,
                                    key_id: master.key_id(),
                                    sealed: &sealed,
                                    hint: &xexam_ai_service::secret::hint(v),
                                    aad_ver: 2,
                                    now,
                                },
                            )?;
                        }
                        None => {
                            conn::delete_conn_secret(tx, id, name)?;
                        }
                    }
                }
                let pool = conn::upsert_pool(
                    tx,
                    &NewPool {
                        connection_id: id,
                        model: seed.model,
                        manual: false,
                        status: "ok",
                        error_code: None,
                        usage_option: true,
                        // the stub's models that cap their answers at 4096 are named `*-4k` (the ladder ceiling a
                        // connection test would have found)
                        max_tokens: if seed.model.ends_with("-4k") {
                            4096
                        } else {
                            8192
                        },
                        context: None,
                        latency_ms: Some(10),
                        tested_at: Some(now),
                    },
                )?;
                conn::set_slot(tx, "default", Some(pool), "test")?;
                tx.execute("UPDATE settings SET enabled = 1 WHERE id = 1", [])?;
                Ok(())
            })
            .expect("seed the route");
    }

    /// Admin: OpenAI-compatible connection against the stub, a model, AI+ on (seeded; the report goes out at once).
    pub async fn configure_openai(&self, _admin: &Who, model: &str) {
        self.seed_route(Seed {
            provider: "openai",
            url: format!("{}/v1", self.stub.url),
            api_key: Some(KEY),
            gateway_key: None,
            gateway_auth: "bearer",
            model,
        });
        self.app.0.heartbeat_now().await;
    }

    /// v1.8: gateway connection in the Responses API format against the stub (`/gw/v1/responses`), a model, AI+ on.
    /// The gateway key goes in `Authorization: Bearer` (a provider key would clash with it).
    pub async fn configure_responses(&self, _admin: &Who, model: &str) {
        self.seed_route_as(
            Seed {
                provider: "gateway",
                url: format!("{}/gw/v1", self.stub.url),
                api_key: None,
                gateway_key: Some(GW_KEY),
                gateway_auth: "bearer",
                model,
            },
            Some("responses"),
        );
        self.app.0.heartbeat_now().await;
    }

    /// Kept for the callers of v1.2: the connection is already seeded and enabled, so this only reports.
    pub async fn verify_and_enable(&self, _admin: &Who) {
        self.app.0.heartbeat_now().await;
    }

    /// Admin: Anthropic-format connection against the stub, a model, AI+ on.
    pub async fn configure_anthropic(&self, _admin: &Who, model: &str) {
        self.seed_route(Seed {
            provider: "anthropic",
            url: format!("{}/anthropic/v1", self.stub.url),
            api_key: Some(KEY),
            gateway_key: None,
            gateway_auth: "bearer",
            model,
        });
        self.app.0.heartbeat_now().await;
    }

    /// Admin: only the default model (the same connection; the pool gets the model, the default slot points at it).
    pub async fn set_model(&self, _admin: &Who, model: &str) {
        use xexam_ai_service::store::conn::{self, NewPool};
        let now = xexam_ai_service::util::unix_now();
        self.app
            .0
            .store
            .write(|tx| -> rusqlite::Result<()> {
                let pool = conn::upsert_pool(
                    tx,
                    &NewPool {
                        connection_id: 1,
                        model,
                        manual: false,
                        status: "ok",
                        error_code: None,
                        usage_option: true,
                        max_tokens: if model.ends_with("-4k") { 4096 } else { 8192 },
                        context: None,
                        latency_ms: Some(10),
                        tested_at: Some(now),
                    },
                )?;
                conn::set_slot(tx, "default", Some(pool), "test")
            })
            .expect("set the default model");
        self.app.0.heartbeat_now().await;
    }

    /// Adds a made-up question to the fake backend.
    pub fn add_question(&self, q: FakeQuestion) {
        let id = q.question["id"].as_str().unwrap_or("").to_string();
        self.backend.st().questions.insert(id, q);
    }

    /// Adds a made-up saved record to the fake backend.
    pub fn add_record(&self, r: FakeRecord) {
        self.backend.st().records.insert(r.id, r);
    }

    /// How many times the fake backend was called on `path`.
    pub fn backend_calls(&self, path: &str) -> usize {
        self.backend
            .st()
            .calls
            .iter()
            .filter(|p| *p == path)
            .count()
    }

    /// Uploads a file to the AI import (raw octet-stream body, `X-Filename` percent-encoded) and answers like `post`.
    pub async fn upload(&self, who: &Who, name: &str, bytes: Vec<u8>) -> (u16, Value) {
        self.upload_with(who, name, bytes, "application/octet-stream", &[])
            .await
    }

    pub async fn upload_with(
        &self,
        who: &Who,
        name: &str,
        bytes: Vec<u8>,
        content_type: &str,
        extra: &[(&str, &str)],
    ) -> (u16, Value) {
        let mut enc = String::new();
        for b in name.bytes() {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                enc.push(b as char);
            } else {
                enc.push_str(&format!("%{b:02X}"));
            }
        }
        let mut b = Request::builder()
            .method(Method::POST)
            .uri("/api/ai/import/upload")
            .header("x-xexam-identity", self.ident(who))
            .header("content-type", content_type)
            .header("content-length", bytes.len())
            .header("x-filename", enc);
        for (k, v) in extra {
            b = b.header(*k, *v);
        }
        let req = b.body(Body::from(bytes)).expect("request");
        let resp = self.router().oneshot(req).await.expect("response");
        read_json(resp).await
    }

    pub fn usage_rows(&self) -> i64 {
        self.db()
            .query_row("SELECT COUNT(*) FROM usage", [], |r| r.get(0))
            .expect("count")
    }
}

pub async fn read_json(r: Response) -> (u16, Value) {
    let status = r.status().as_u16();
    let bytes = r
        .into_body()
        .collect()
        .await
        .map(|c| c.to_bytes())
        .unwrap_or_default();
    let v = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, v)
}

/// Reads SSE events from a response body.
pub struct SseReader {
    pub headers: HeaderMap,
    body: Body,
    parser: xexam_ai_service::provider::sse::SseParser,
    pending: std::collections::VecDeque<(String, Value)>,
    ended: bool,
    pub raw: String,
}

impl SseReader {
    fn new(r: Response) -> SseReader {
        let headers = r.headers().clone();
        SseReader {
            headers,
            body: r.into_body(),
            parser: xexam_ai_service::provider::sse::SseParser::new(1 << 22),
            pending: Default::default(),
            ended: false,
            raw: String::new(),
        }
    }

    /// The next event, or `None` at the end of the body.
    pub async fn next(&mut self) -> Option<(String, Value)> {
        loop {
            if let Some(e) = self.pending.pop_front() {
                return Some(e);
            }
            if self.ended {
                return None;
            }
            let frame = tokio::time::timeout(Duration::from_secs(20), self.body.frame())
                .await
                .expect("no stream event within 20 s");
            let events = match frame {
                Some(Ok(f)) => match f.into_data() {
                    Ok(d) => {
                        self.raw.push_str(&String::from_utf8_lossy(&d));
                        self.parser.push(&d).expect("sse")
                    }
                    Err(_) => Vec::new(),
                },
                _ => {
                    self.ended = true;
                    self.parser.finish().expect("sse")
                }
            };
            for e in events {
                let v = serde_json::from_str(&e.data).unwrap_or(Value::Null);
                self.pending.push_back((e.name, v));
            }
        }
    }

    pub async fn all(&mut self) -> Vec<(String, Value)> {
        let mut out = Vec::new();
        while let Some(e) = self.next().await {
            out.push(e);
        }
        out
    }

    /// Reads until an event with this name arrives.
    pub async fn until(&mut self, name: &str) -> (String, Value) {
        loop {
            let e = self.next().await.expect("stream ended early");
            if e.0 == name {
                return e;
            }
        }
    }
}

/// Names of the events.
pub fn names(ev: &[(String, Value)]) -> Vec<String> {
    ev.iter().map(|(n, _)| n.clone()).collect()
}

/// The terminal event.
pub fn terminal(ev: &[(String, Value)]) -> (String, Value) {
    ev.last().cloned().expect("events")
}

/// Joined delta texts.
pub fn text(ev: &[(String, Value)]) -> String {
    ev.iter()
        .filter(|(n, _)| n == "delta")
        .filter_map(|(_, v)| v["text"].as_str().map(str::to_string))
        .collect()
}

/// Polls until `f` is true (up to 10 s).
pub async fn eventually(mut f: impl FnMut() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("condition not reached within 10 s");
}
