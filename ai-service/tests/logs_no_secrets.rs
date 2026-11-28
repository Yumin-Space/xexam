//! What reaches a log line (printed or forwarded to the main System log) and what reaches an answer: never a key, an
//! identity statement, a cookie, a prompt, an answer, a chat title, a memory or a provider URL.
mod common;

use common::*;
use serde_json::json;

#[tokio::test]
async fn no_secret_or_content_in_any_log_line() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    let secret = "sk-demo-SECRET-9999-abcd";
    let gw_secret = "gw-demo-SECRET-7777-wxyz";
    let question = "my private study question about matrices";
    let note = "private memory note text";

    // a full session: gateway with both keys (seeded: the connection routes have their own tests), chat, memory,
    // suggestion, admin views and clears, a failure
    t.seed_route(Seed {
        provider: "gateway",
        url: format!("{}/gw/openai", t.stub.url),
        api_key: Some(secret),
        gateway_key: Some(gw_secret),
        gateway_auth: "cf-aig-authorization",
        model: "demo-chat-small",
    });
    t.verify_and_enable(&admin).await;
    let ev = t
        .send_all(&sam, json!({"text": question}))
        .await
        .expect("stream");
    assert_eq!(text(&ev), ANSWER);
    let chat = ev[0].1["chatId"].as_i64().expect("id");
    let msg = ev[0].1["userMessageId"].as_i64().expect("id");
    t.post("/api/ai/memories/add", &sam, json!({"content": note}))
        .await;
    let (s, _) = t
        .post("/api/ai/memories/suggest", &sam, json!({"messageId": msg}))
        .await;
    assert_eq!(s, 200);
    t.stub.force(Some((401, r#"{"error":{"message":"Incorrect API key provided: sk-de****abcd","code":"invalid_api_key"}}"#)));
    let ev = t
        .send_all(&sam, json!({"text": question}))
        .await
        .expect("stream");
    assert_eq!(terminal(&ev).1["code"], json!("ai.up.auth"));
    t.stub.force(None);
    let (s, _) = t
        .get(&format!("/api/ai/admin/user?id={}", sam.id), &admin)
        .await;
    assert_eq!(s, 200);
    let (s, _) = t
        .get(
            &format!("/api/ai/admin/chat?user={}&id={chat}", sam.id),
            &admin,
        )
        .await;
    assert_eq!(s, 200);
    let (s, _) = t
        .post(
            "/api/ai/admin/clear",
            &admin,
            json!({"userId": sam.id, "what": "memories"}),
        )
        .await;
    assert_eq!(s, 200);
    let (s, v) = t.get("/api/ai/admin/settings", &admin).await;
    assert_eq!(s, 200);
    assert!(!v.to_string().contains(secret) && !v.to_string().contains(gw_secret));
    let (_, u) = t.get("/api/ai/admin/usage", &admin).await;
    assert!(!u.to_string().contains(&t.stub.url));

    // forward the queued lines to the fake backend
    xexam_ai_service::logs::flush_once(&t.app.0.link, xexam_ai_service::logs::queue()).await;
    let forwarded = t.backend.logs();
    assert!(forwarded
        .iter()
        .any(|l| l.contains("AI call failed for user \"sam\": ai.up.auth (HTTP 401)")));
    // audit lines arrive once (synchronously), never through the queue
    assert_eq!(
        forwarded
            .iter()
            .filter(|l| l.contains("viewed the AI chats and memories of user \"sam\""))
            .count(),
        1
    );
    assert_eq!(
        forwarded
            .iter()
            .filter(|l| l.contains("cleared all 1 AI memories of user \"sam\""))
            .count(),
        1
    );

    let printed = captured();
    let statement_prefix = t.ident(&sam).split('.').next().unwrap_or("x")[..12].to_string();
    let forbidden: Vec<String> = vec![
        secret.into(),
        gw_secret.into(),
        "sk-demo-NEVER".into(),
        "sk-de****".into(),
        statement_prefix,
        sam.sid.clone(),
        TOKEN.into(),
        question.into(),
        note.into(),
        "short answer".into(),
        t.stub.url.clone(),
        "/gw/openai".into(),
        "Reply with the single word OK.".into(),
        "You are the study assistant".into(),
    ];
    for line in printed.iter().chain(forwarded.iter()) {
        for f in &forbidden {
            assert!(
                !line.contains(f.as_str()),
                "a log line contains {f:?}: {line}"
            );
        }
    }
    // forwarded lines have levels the backend accepts
    for (level, _) in t.backend.st().logs.iter() {
        assert!(["INFO", "WARN", "ERROR"].contains(&level.as_str()));
    }
}

// —— v3.1: the web push courier ——————————————————————————————————————————————————————————————————————————————————————————

/// One local server plays the main backend (`/internal/ai/push/*`, `/internal/ai/log`) and the push service (every other
/// path), so a single address serves both roles.
#[derive(Default)]
struct PushMock {
    jobs: Vec<serde_json::Value>,
    results: usize,
    lines: Vec<String>,
}

#[tokio::test]
async fn the_push_courier_logs_no_endpoint_token_or_body() {
    use axum::extract::{Request, State};
    use axum::response::Response;
    use std::sync::{Arc, Mutex};

    const ENDPOINT_SECRET: &str = "SENTINEL-ENDPOINT-PATH-7421";
    const JWT_SECRET: &str = "SENTINEL-JWT-SIGNATURE-5530";
    const USERINFO_SECRET: &str = "SENTINEL-USERINFO-9917";

    fn reply(status: u16, body: &str) -> Response {
        let mut r = Response::new(axum::body::Body::from(body.to_string()));
        *r.status_mut() = axum::http::StatusCode::from_u16(status).expect("status");
        r
    }

    async fn handler(State(st): State<Arc<Mutex<PushMock>>>, req: Request) -> Response {
        let path = req.uri().path().to_string();
        let raw = axum::body::to_bytes(req.into_body(), 1 << 20)
            .await
            .unwrap_or_default();
        let body: serde_json::Value = serde_json::from_slice(&raw).unwrap_or_default();
        let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
        match path.as_str() {
            "/internal/ai/push/next" => {
                let jobs = std::mem::take(&mut s.jobs);
                reply(200, &json!({ "jobs": jobs }).to_string())
            }
            "/internal/ai/push/result" => {
                s.results += body["results"].as_array().map_or(0, Vec::len);
                reply(200, r#"{"ok":true}"#)
            }
            "/internal/ai/log" => {
                for l in body["lines"].as_array().cloned().unwrap_or_default() {
                    s.lines
                        .push(l["message"].as_str().unwrap_or("").to_string());
                }
                reply(200, r#"{"ok":true}"#)
            }
            // the push service: the last path segment is the status; the answer repeats the secrets back
            p => {
                let code: u16 = p
                    .rsplit('/')
                    .next()
                    .and_then(|c| c.parse().ok())
                    .unwrap_or(201);
                reply(code, &format!("echo {ENDPOINT_SECRET} {JWT_SECRET}"))
            }
        }
    }

    init_logs();
    let st = Arc::new(Mutex::new(PushMock::default()));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = l.local_addr().expect("addr");
    let router = axum::Router::new().fallback(handler).with_state(st.clone());
    let server = tokio::spawn(async move {
        let _ = axum::serve(l, router).await;
    });
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("link")).expect("link dir");
    let url = format!("http://{addr}");
    let app = xexam_ai_service::app::App::open(test_config(dir.path(), &url, &Opts::default()))
        .expect("open");
    app.0
        .knobs
        .push_wait_ms
        .store(50, std::sync::atomic::Ordering::SeqCst);
    *app.0.knobs.push_target.lock().expect("lock") = Some(addr);

    let job = |id: i64, endpoint: String| {
        json!({"id": id, "tries": 1, "endpoint": endpoint, "ttl": 60, "urgency": "normal", "topic": "remind",
               "authorization": format!("vapid t=a.b.{JWT_SECRET}, k=demo"), "body": "AAECAwQFBgcICQ"})
    };
    st.lock().expect("lock").jobs = vec![
        job(
            1,
            format!("https://fcm.googleapis.com/fcm/send/{ENDPOINT_SECRET}/201"),
        ),
        job(
            2,
            format!("https://fcm.googleapis.com/fcm/send/{ENDPOINT_SECRET}/410"),
        ),
        job(
            3,
            format!("https://web.push.apple.com/{ENDPOINT_SECRET}/500"),
        ),
        job(
            4,
            format!("https://web.push.apple.com/{ENDPOINT_SECRET}/401"),
        ),
        job(
            5,
            format!("https://{USERINFO_SECRET}@fcm.googleapis.com/{ENDPOINT_SECRET}/201"),
        ),
        job(6, "https://127.0.0.1/x".to_string()),
    ];
    // the Debug form of a job names the id and the host only
    let shown = {
        let j = xexam_ai_service::link::PushJob {
            id: 9,
            tries: 1,
            endpoint: format!("https://fcm.googleapis.com/fcm/send/{ENDPOINT_SECRET}"),
            ttl: 1,
            urgency: "normal".into(),
            topic: None,
            authorization: format!("vapid t=a.b.{JWT_SECRET}"),
            body: "AAEC".into(),
        };
        format!("{j:?}")
    };
    assert_eq!(shown, "PushJob { id: 9, host: \"fcm.googleapis.com\" }");

    xexam_ai_service::push::worker::spawn(app.clone(), app.0.shutdown.clone());
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while st.lock().expect("lock").results < 6 {
        assert!(std::time::Instant::now() < until, "no results");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    xexam_ai_service::logs::flush_once(&app.0.link, xexam_ai_service::logs::queue()).await;
    let forwarded = st.lock().expect("lock").lines.clone();
    for line in captured().iter().chain(forwarded.iter()) {
        for f in [
            ENDPOINT_SECRET,
            JWT_SECRET,
            USERINFO_SECRET,
            "AAECAwQFBgcICQ",
            "fcm/send",
            &url,
        ] {
            assert!(!line.contains(f), "a log line contains {f:?}: {line}");
        }
    }
    app.0.begin_shutdown();
    app.0.store.release_file();
    server.abort();
}
