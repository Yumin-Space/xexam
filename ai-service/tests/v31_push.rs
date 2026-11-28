//! v3.1 web push courier: a mock main backend (`/internal/ai/push/*`) and a mock push service on 127.0.0.1, with the
//! courier's `push_target` hook sending every job to the mock instead of the (allow-listed) endpoint. Every endpoint,
//! key and message is made up.
mod common;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::response::Response;
use axum::Router;
use base64::Engine;
use common::*;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use xexam_ai_service::app::AppState;

const AUTH: &str =
    "vapid t=sentinel-jwt-header.sentinel-jwt-claims.sentinel-jwt-sig, k=demo-public-key";

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn answer(status: u16, headers: &[(&str, String)], body: &str) -> Response {
    let mut r = Response::new(Body::from(body.to_string()));
    *r.status_mut() = axum::http::StatusCode::from_u16(status).expect("status");
    for (k, v) in headers {
        r.headers_mut().insert(
            axum::http::HeaderName::from_bytes(k.as_bytes()).expect("name"),
            v.parse().expect("value"),
        );
    }
    r
}

async fn serve(router: Router) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = l.local_addr().expect("addr");
    let h = tokio::spawn(async move {
        let _ = axum::serve(l, router).await;
    });
    (addr, h)
}

// —— mock main backend ————————————————————————————————————————————————————————————————————————————————————————————————

#[derive(Default)]
struct BState {
    /// Batches to hand out, one per `push/next`
    queue: VecDeque<Vec<Value>>,
    next_bodies: Vec<Value>,
    results: Vec<Value>,
    batches: Vec<usize>,
    next_404: bool,
    /// Every `push/next` call that arrived with the right token, even a refused one
    next_calls: usize,
}

struct Backend {
    url: String,
    st: Arc<Mutex<BState>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Backend {
    fn st(&self) -> std::sync::MutexGuard<'_, BState> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn hand_out(&self, jobs: Vec<Value>) {
        self.st().queue.push_back(jobs);
    }

    fn results(&self) -> Vec<Value> {
        self.st().results.clone()
    }

    /// The result of one job id.
    fn result_of(&self, id: i64) -> Value {
        self.results()
            .into_iter()
            .find(|r| r["id"] == json!(id))
            .unwrap_or_else(|| panic!("no result for job {id}"))
    }

    async fn wait_results(&self, n: usize) {
        let until = Instant::now() + Duration::from_secs(10);
        while self.results().len() < n {
            assert!(
                Instant::now() < until,
                "only {} results",
                self.results().len()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

async fn backend_handler(State(st): State<Arc<Mutex<BState>>>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    let token_ok = req
        .headers()
        .get("x-xexam-internal-token")
        .and_then(|v| v.to_str().ok())
        == Some(TOKEN);
    let raw = axum::body::to_bytes(req.into_body(), 1 << 22)
        .await
        .unwrap_or_default();
    let body: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    {
        let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
        if token_ok && path == "/internal/ai/push/next" {
            s.next_calls += 1;
        }
        if !token_ok || s.next_404 {
            return answer(404, &[], "not found");
        }
    }
    match path.as_str() {
        "/internal/ai/push/next" => {
            let wait = body["waitMs"].as_u64().unwrap_or(0).min(100);
            let until = Instant::now() + Duration::from_millis(wait);
            st.lock()
                .unwrap_or_else(|e| e.into_inner())
                .next_bodies
                .push(body);
            loop {
                {
                    let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(jobs) = s.queue.pop_front() {
                        return answer(
                            200,
                            &[("content-type", "application/json".into())],
                            &json!({ "jobs": jobs }).to_string(),
                        );
                    }
                }
                if Instant::now() >= until {
                    return answer(
                        200,
                        &[("content-type", "application/json".into())],
                        r#"{"jobs":[]}"#,
                    );
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        "/internal/ai/push/result" => {
            let list = body["results"].as_array().cloned().unwrap_or_default();
            if list.is_empty() || list.len() > 20 {
                return answer(400, &[], "bad");
            }
            let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
            s.batches.push(list.len());
            s.results.extend(list);
            answer(
                200,
                &[("content-type", "application/json".into())],
                r#"{"ok":true}"#,
            )
        }
        _ => answer(404, &[], "not found"),
    }
}

async fn backend() -> Backend {
    let st = Arc::new(Mutex::new(BState::default()));
    let router = Router::new()
        .fallback(backend_handler)
        .with_state(st.clone());
    let (addr, task) = serve(router).await;
    Backend {
        url: format!("http://{addr}"),
        st,
        task,
    }
}

// —— mock push service ————————————————————————————————————————————————————————————————————————————————————————————————

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Default)]
struct PState {
    seen: Vec<Seen>,
    in_flight: usize,
    max_in_flight: usize,
}

struct PushSvc {
    addr: SocketAddr,
    st: Arc<Mutex<PState>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for PushSvc {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl PushSvc {
    fn seen(&self) -> Vec<Seen> {
        self.st
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .seen
            .clone()
    }

    fn max_in_flight(&self) -> usize {
        self.st
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .max_in_flight
    }
}

/// The last path segment picks the answer: `s<code>`, `s429-<seconds>`, `s429-date`, `s429-none`, `slow<code>` (the
/// answer comes after 300 ms), `s302` (with a `Location` header).
async fn push_handler(State(st): State<Arc<Mutex<PState>>>, req: Request) -> Response {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body = axum::body::to_bytes(req.into_body(), 1 << 20)
        .await
        .unwrap_or_default()
        .to_vec();
    {
        let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
        s.seen.push(Seen {
            method,
            path: path.clone(),
            headers,
            body,
        });
        s.in_flight += 1;
        s.max_in_flight = s.max_in_flight.max(s.in_flight);
    }
    let kind = path.rsplit('/').next().unwrap_or("").to_string();
    let r = if let Some(code) = kind.strip_prefix("slow") {
        tokio::time::sleep(Duration::from_millis(300)).await;
        answer(code.parse().unwrap_or(201), &[], "slow")
    } else if kind == "s429-date" {
        let at =
            jiff::Timestamp::from_second(xexam_ai_service::util::unix_now() + 120).expect("ts");
        let date = jiff::fmt::rfc2822::DateTimePrinter::new()
            .timestamp_to_string(&at)
            .expect("date");
        answer(429, &[("retry-after", date)], "slow down")
    } else if kind == "s429-none" {
        answer(429, &[], "slow down")
    } else if let Some(secs) = kind.strip_prefix("s429-") {
        answer(429, &[("retry-after", secs.to_string())], "slow down")
    } else if kind == "s302" {
        answer(
            302,
            &[("location", "http://127.0.0.1:9/elsewhere".into())],
            "",
        )
    } else if let Some(code) = kind.strip_prefix('s') {
        answer(code.parse().unwrap_or(201), &[], "{\"detail\":\"made up\"}")
    } else {
        answer(201, &[], "")
    };
    st.lock().unwrap_or_else(|e| e.into_inner()).in_flight -= 1;
    r
}

async fn push_svc() -> PushSvc {
    let st = Arc::new(Mutex::new(PState::default()));
    let router = Router::new().fallback(push_handler).with_state(st.clone());
    let (addr, task) = serve(router).await;
    PushSvc { addr, st, task }
}

// —— the app under test ———————————————————————————————————————————————————————————————————————————————————————————————

struct Rig {
    app: AppState,
    backend: Backend,
    push: PushSvc,
    _dir: tempfile::TempDir,
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.app.0.begin_shutdown();
        self.app.0.store.release_file();
    }
}

/// The service with a mock backend and a mock push service; the courier is NOT started yet.
async fn rig() -> Rig {
    init_logs();
    let backend = backend().await;
    let push = push_svc().await;
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("link")).expect("link dir");
    let app =
        xexam_ai_service::app::App::open(test_config(dir.path(), &backend.url, &Opts::default()))
            .expect("open");
    app.0
        .knobs
        .push_wait_ms
        .store(50, std::sync::atomic::Ordering::SeqCst);
    *app.0.knobs.push_target.lock().expect("lock") = Some(push.addr);
    Rig {
        app,
        backend,
        push,
        _dir: dir,
    }
}

impl Rig {
    fn start_courier(&self) {
        xexam_ai_service::push::worker::spawn(self.app.clone(), self.app.0.shutdown.clone());
    }
}

/// A job as the backend hands it out; the body is `len` bytes of `id`.
fn job(id: i64, kind: &str) -> Value {
    job_at(id, "fcm.googleapis.com", kind)
}

fn job_at(id: i64, host: &str, kind: &str) -> Value {
    json!({
        "id": id, "tries": 1,
        "endpoint": format!("https://{host}/fcm/send/{kind}"),
        "ttl": 3600, "urgency": "normal", "topic": "remind",
        "authorization": AUTH,
        "body": b64(&[id as u8; 64]),
    })
}

// —— tests ————————————————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn a_job_is_posted_with_exact_headers_and_the_body_byte_for_byte() {
    let t = rig().await;
    t.start_courier();
    let body: Vec<u8> = (0..=255u8).cycle().take(1500).collect();
    t.backend.hand_out(vec![
        json!({"id": 4711, "tries": 3, "endpoint": "https://fcm.googleapis.com/fcm/send/ok",
               "ttl": 21600, "urgency": "high", "topic": "bank-12",
               "authorization": AUTH, "body": b64(&body)}),
        json!({"id": 4712, "tries": 1, "endpoint": "https://web.push.apple.com/QAbc?x=1",
               "ttl": 0, "urgency": "very-low", "topic": null,
               "authorization": AUTH, "body": b64(&body[..40])}),
    ]);
    t.backend.wait_results(2).await;
    let seen = t.push.seen();
    assert_eq!(seen.len(), 2);
    let a = seen.iter().find(|s| s.path == "/fcm/send/ok").expect("a");
    assert_eq!(a.method, "POST");
    assert_eq!(a.header("ttl"), Some("21600"));
    assert_eq!(a.header("urgency"), Some("high"));
    assert_eq!(a.header("topic"), Some("bank-12"));
    assert_eq!(a.header("content-encoding"), Some("aes128gcm"));
    assert_eq!(a.header("content-type"), Some("application/octet-stream"));
    assert_eq!(a.header("authorization"), Some(AUTH));
    assert_eq!(a.header("user-agent"), None);
    assert_eq!(a.header("x-xexam-internal-token"), None);
    assert_eq!(a.body, body);
    // the path and the query of the endpoint are kept; no topic header without a topic
    let b = seen.iter().find(|s| s.path == "/QAbc").expect("b");
    assert_eq!(b.header("topic"), None);
    assert_eq!(b.header("ttl"), Some("0"));
    assert_eq!(b.header("urgency"), Some("very-low"));
    assert_eq!(b.body, &body[..40]);
    // results: exact keys, the tries of the job
    let r = t.backend.result_of(4711);
    assert_eq!(
        r,
        json!({"id": 4711, "tries": 3, "outcome": "sent", "status": 201, "retryAfter": null})
    );
    let keys: Vec<&str> = r
        .as_object()
        .expect("o")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["id", "tries", "outcome", "status", "retryAfter"]);
    // every poll carries a waitMs of at most 20000
    assert!(t
        .backend
        .st()
        .next_bodies
        .iter()
        .all(|b| b["waitMs"].as_u64().is_some_and(|w| w <= 20_000)));
}

#[tokio::test]
async fn each_status_maps_to_its_outcome() {
    let t = rig().await;
    t.start_courier();
    let table: [(&str, &str, Option<u64>); 12] = [
        ("s201", "sent", Some(201)),
        ("s202", "sent", Some(202)),
        ("s404", "gone", Some(404)),
        ("s410", "gone", Some(410)),
        ("s401", "auth", Some(401)),
        ("s403", "auth", Some(403)),
        ("s500", "retry", Some(500)),
        ("s503", "retry", Some(503)),
        ("s400", "rejected", Some(400)),
        ("s413", "rejected", Some(413)),
        ("s302", "rejected", Some(302)),
        ("s429-none", "retry", Some(429)),
    ];
    // distinct hosts, so a 429 does not hold the others
    let hosts = [
        "fcm.googleapis.com",
        "updates.push.services.mozilla.com",
        "web.push.apple.com",
        "a.push.apple.com",
        "b.push.apple.com",
        "c.notify.windows.com",
    ];
    for (i, chunk) in table.chunks(6).enumerate() {
        let jobs: Vec<Value> = chunk
            .iter()
            .enumerate()
            .map(|(k, (kind, _, _))| job_at((i * 6 + k) as i64 + 1, hosts[k], kind))
            .collect();
        t.backend.hand_out(jobs);
    }
    t.backend.wait_results(12).await;
    for (i, (kind, outcome, status)) in table.iter().enumerate() {
        let r = t.backend.result_of(i as i64 + 1);
        assert_eq!(r["outcome"], json!(outcome), "{kind}");
        assert_eq!(r["status"].as_u64(), *status, "{kind}");
    }
    // a 429 without Retry-After waits 60 s
    assert_eq!(t.backend.result_of(12)["retryAfter"], json!(60));
    assert_eq!(t.backend.result_of(1)["retryAfter"], json!(null));
}

#[tokio::test]
async fn retry_after_seconds_and_http_date_and_the_rest_of_the_host_wait() {
    let t = rig().await;
    t.start_courier();
    // seconds, clamped to 3600
    t.backend.hand_out(vec![job(1, "s429-45")]);
    t.backend
        .hand_out(vec![job_at(2, "web.push.apple.com", "s429-99999")]);
    // an HTTP date about two minutes ahead
    t.backend.hand_out(vec![job_at(
        3,
        "updates.push.services.mozilla.com",
        "s429-date",
    )]);
    t.backend.wait_results(3).await;
    assert_eq!(t.backend.result_of(1)["retryAfter"], json!(45));
    assert_eq!(t.backend.result_of(2)["retryAfter"], json!(3600));
    let d = t.backend.result_of(3)["retryAfter"].as_u64().expect("secs");
    assert!((110..=120).contains(&d), "{d}");

    // one batch on one host: the first job is refused with a 429, the second is already on its way (two per host), the
    // third waits for a place and is reported `deferred` with the same wait, without being sent
    t.backend.hand_out(vec![
        job_at(11, "a.push.apple.com", "s429-45"),
        job_at(12, "a.push.apple.com", "slow201"),
        job_at(13, "a.push.apple.com", "s201"),
    ]);
    t.backend.wait_results(6).await;
    let r11 = t.backend.result_of(11);
    assert_eq!(
        (
            r11["outcome"].clone(),
            r11["status"].clone(),
            r11["retryAfter"].clone()
        ),
        (json!("retry"), json!(429), json!(45))
    );
    assert_eq!(t.backend.result_of(12)["outcome"], json!("sent"));
    let r13 = t.backend.result_of(13);
    assert_eq!(
        (
            r13["outcome"].clone(),
            r13["status"].clone(),
            r13["retryAfter"].clone()
        ),
        (json!("deferred"), json!(null), json!(45))
    );
    assert!(!t.push.seen().iter().any(|s| s.path.ends_with("/s201")));
}

#[tokio::test]
async fn at_most_two_requests_per_host_and_four_in_all() {
    let t = rig().await;
    t.start_courier();
    // five jobs, one host: two at a time
    t.backend
        .hand_out((1..=5).map(|i| job(i, "slow201")).collect());
    t.backend.wait_results(5).await;
    assert_eq!(t.push.max_in_flight(), 2);
    let t2 = rig().await;
    t2.start_courier();
    // eight jobs on five hosts: four at a time (and never more than two per host)
    let hosts = [
        "fcm.googleapis.com",
        "updates.push.services.mozilla.com",
        "web.push.apple.com",
        "a.push.apple.com",
        "b.push.apple.com",
    ];
    t2.backend.hand_out(
        (0..8)
            .map(|i| job_at(i as i64 + 1, hosts[i % 5], "slow201"))
            .collect(),
    );
    t2.backend.wait_results(8).await;
    assert_eq!(t2.push.max_in_flight(), 4);
}

#[tokio::test]
async fn a_batch_of_21_jobs_is_refused_whole() {
    let t = rig().await;
    t.start_courier();
    t.backend
        .hand_out((1..=21).map(|i| job(i, "s201")).collect());
    let until = Instant::now() + Duration::from_secs(5);
    while !captured_contains("push: refused a batch of 21 jobs") {
        assert!(Instant::now() < until, "no refusal line");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(t.push.seen().is_empty(), "nothing is sent");
    assert!(t.backend.results().is_empty(), "nothing is reported");
    // 20 jobs are fine and go in ONE result call
    t.backend
        .hand_out((1..=20).map(|i| job(i, "s201")).collect());
    t.backend.wait_results(20).await;
    assert_eq!(t.backend.st().batches, vec![20]);
}

#[tokio::test]
async fn a_404_on_push_next_pauses_the_courier() {
    let t = rig().await;
    t.backend.st().next_404 = true;
    t.start_courier();
    tokio::time::sleep(Duration::from_millis(900)).await;
    // an older backend has no routes: the courier waits 60 s instead of asking again
    assert_eq!(t.backend.st().next_calls, 1);
    assert!(t.push.seen().is_empty());
}

#[tokio::test]
async fn a_job_the_courier_does_not_trust_is_rejected_and_never_sent() {
    let t = rig().await;
    t.start_courier();
    let mut bad: Vec<Value> = Vec::new();
    let mut add = |id: i64, f: &dyn Fn(&mut Value)| {
        let mut j = job(id, "s201");
        f(&mut j);
        bad.push(j);
    };
    add(1, &|j| j["endpoint"] = json!("https://127.0.0.1/x"));
    add(2, &|j| {
        j["endpoint"] = json!("http://fcm.googleapis.com/fcm/send/x")
    });
    add(3, &|j| {
        j["endpoint"] = json!("https://user@fcm.googleapis.com/fcm/send/x")
    });
    add(4, &|j| j["authorization"] = json!("Bearer abc"));
    add(5, &|j| j["authorization"] = json!("vapid t=a\r\nX-Evil: 1"));
    add(6, &|j| j["urgency"] = json!("urgent"));
    add(7, &|j| j["topic"] = json!("not a topic!"));
    add(8, &|j| j["ttl"] = json!(2_419_201));
    add(9, &|j| j["ttl"] = json!(-1));
    add(10, &|j| j["body"] = json!(b64(&[0u8; 4097])));
    add(11, &|j| j["body"] = json!("***"));
    add(12, &|j| j["topic"] = json!("a".repeat(33)));
    t.backend.hand_out(bad[..6].to_vec());
    t.backend.hand_out(bad[6..].to_vec());
    t.backend.wait_results(12).await;
    for id in 1..=12 {
        let r = t.backend.result_of(id);
        assert_eq!(r["outcome"], json!("rejected"), "{id}");
        assert_eq!(r["status"], json!(null), "{id}");
    }
    assert!(t.push.seen().is_empty());
}

#[tokio::test]
async fn an_unreachable_push_service_is_a_retry_with_status_zero() {
    let t = rig().await;
    // a port nobody listens on
    let l = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let dead = l.local_addr().expect("addr");
    drop(l);
    *t.app.0.knobs.push_target.lock().expect("lock") = Some(dead);
    t.start_courier();
    t.backend.hand_out(vec![job(1, "s201")]);
    t.backend.wait_results(1).await;
    let r = t.backend.result_of(1);
    assert_eq!(
        (r["outcome"].clone(), r["status"].clone()),
        (json!("retry"), json!(0))
    );
}

#[tokio::test]
async fn nothing_secret_reaches_a_log_line() {
    let t = rig().await;
    t.start_courier();
    t.backend.hand_out(vec![
        job(1, "s201"),
        job(2, "s410"),
        job(3, "s500"),
        job_at(4, "web.push.apple.com", "s429-none"),
    ]);
    t.backend.wait_results(4).await;
    xexam_ai_service::logs::flush_once(&t.app.0.link, xexam_ai_service::logs::queue()).await;
    let b = b64(&[1u8; 64]);
    for line in captured() {
        for f in [
            "sentinel-jwt",
            AUTH,
            "/fcm/send/",
            b.as_str(),
            &t.push.addr.to_string(),
        ] {
            assert!(!line.contains(f), "a log line contains {f:?}: {line}");
        }
    }
}
