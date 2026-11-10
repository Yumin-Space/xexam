//! 测试公用：假的 `Engine`（不依赖解析器和数据库是否已经实现）、造请求、发请求。
#![allow(dead_code)]
use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower::ServiceExt;
use xexam_backend::api::{build_router, AppState, Inner};
use xexam_backend::config::Config;
use xexam_backend::database::Store;
use xexam_backend::error::{DbError, ImportError};
use xexam_backend::models::{
    Answer, Attempt, ImportRecord, Kind, ParsedSheet, ParsedWorkbook, Question,
};
use xexam_backend::services::{Engine, PendingStore};

/// tests/baseline 的位置（由 backend/ 的位置推出，不写死机器路径）。
pub fn fixtures() -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join("tests/baseline")
        .to_string_lossy()
        .into_owned()
}

#[derive(Default)]
pub struct Fake {
    pub parse: Option<Result<ParsedWorkbook, ImportError>>,
    pub parse_delay: Option<Duration>,
    pub plan: Value,
    pub count: i64,
    pub bank: Value,
    pub progress: Value,
    pub imports: Vec<ImportRecord>,
    pub apply: Option<Result<Value, DbError>>,
    pub attempt_result: Value,
    pub last_attempt: Option<Attempt>,
    pub last_parse_len: Option<usize>,
    pub apply_calls: usize,
    pub reset_calls: usize,
    pub fail_all_db: bool,
}

pub struct FakeEngine(pub Mutex<Fake>);

impl FakeEngine {
    pub fn new() -> Arc<FakeEngine> {
        Arc::new(FakeEngine(Mutex::new(Fake {
            plan: json!({}),
            bank: json!({"file": "", "mtime": "", "error": "", "questions": [], "sheets": []}),
            progress: json!({"version": 1, "questions": {}, "history": [], "masterStreak": 2}),
            attempt_result: json!({"version": 1, "questions": {}, "history": [], "masterStreak": 2}),
            ..Fake::default()
        })))
    }
    pub fn with<R>(&self, f: impl FnOnce(&mut Fake) -> R) -> R {
        f(&mut self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl Engine for FakeEngine {
    fn check_and_parse(&self, data: &[u8]) -> Result<ParsedWorkbook, ImportError> {
        let (delay, r) = self.with(|f| {
            f.last_parse_len = Some(data.len());
            (f.parse_delay, f.parse.take())
        });
        if let Some(d) = delay {
            std::thread::sleep(d);
        }
        r.unwrap_or(Err(ImportError::Empty))
    }
    fn count_active(&self) -> Result<i64, DbError> {
        self.with(|f| {
            if f.fail_all_db {
                Err(DbError::Join)
            } else {
                Ok(f.count)
            }
        })
    }
    fn bank(&self) -> Result<Value, DbError> {
        self.with(|f| {
            if f.fail_all_db {
                Err(DbError::Join)
            } else {
                Ok(f.bank.clone())
            }
        })
    }
    fn progress(&self) -> Result<Value, DbError> {
        self.with(|f| {
            if f.fail_all_db {
                Err(DbError::Join)
            } else {
                Ok(f.progress.clone())
            }
        })
    }
    fn import_history(&self, _limit: i64) -> Result<Vec<ImportRecord>, DbError> {
        self.with(|f| {
            if f.fail_all_db {
                Err(DbError::Join)
            } else {
                Ok(f.imports.clone())
            }
        })
    }
    fn plan_import(&self, _s: &[ParsedSheet], _q: &[Question]) -> Result<Value, DbError> {
        self.with(|f| {
            if f.fail_all_db {
                Err(DbError::Join)
            } else {
                Ok(f.plan.clone())
            }
        })
    }
    fn apply_import(
        &self,
        _filename: &str,
        _s: &[ParsedSheet],
        _q: &[Question],
    ) -> Result<Value, DbError> {
        self.with(|f| {
            f.apply_calls += 1;
            f.apply.take().unwrap_or(Ok(json!({"added": 0})))
        })
    }
    fn record_attempt(&self, a: &Attempt) -> Result<Value, DbError> {
        self.with(|f| {
            f.last_attempt = Some(a.clone());
            if f.fail_all_db {
                Err(DbError::Join)
            } else {
                Ok(f.attempt_result.clone())
            }
        })
    }
    fn reset_progress(&self) -> Result<Value, DbError> {
        self.with(|f| {
            f.reset_calls += 1;
            Ok(f.progress.clone())
        })
    }
}

pub fn dummy_question(i: usize) -> Question {
    Question {
        kind: Kind::Single,
        tag: String::new(),
        stem: format!("q{i}"),
        options: vec![],
        items: vec![],
        answer: Answer::Letters(vec!["A".into()]),
        translation: String::new(),
        english: String::new(),
        en: None,
        zh: None,
        explanation: String::new(),
        mnemonic: String::new(),
        note: String::new(),
        id: format!("id{i}"),
        sheet: "s".into(),
        row: i as i64,
    }
}

pub fn workbook(n: usize) -> ParsedWorkbook {
    ParsedWorkbook {
        sheets: vec![ParsedSheet {
            name: "s".into(),
            total: n as i64,
            usable: n as i64,
            skipped: vec![],
        }],
        questions: (0..n).map(dummy_question).collect(),
    }
}

pub fn default_config() -> Config {
    Config::from_env_with(&|_| None, "0.3.0".to_string())
}

pub fn config_with(env: &[(&str, &str)]) -> Config {
    let env: Vec<(String, String)> = env
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    Config::from_env_with(
        &|k| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()),
        "0.3.0".to_string(),
    )
}

pub struct TestApp {
    pub router: Router,
    pub state: AppState,
    pub engine: Arc<FakeEngine>,
    pub shutdown_rx: Option<tokio::sync::mpsc::Receiver<()>>,
}

static NEXT_DB: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn app_with(cfg: Config, ttl: Option<Duration>) -> TestApp {
    let engine = FakeEngine::new();
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let allow = cfg.allow_shutdown;
    // 测试里 API 层只通过 FakeEngine 访问数据，这个 Store 不会被用到；放在临时目录里，不在当前目录留文件
    let dir = std::env::temp_dir().join(format!(
        "xexam_api_test_{}_{}",
        std::process::id(),
        NEXT_DB.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let store = Arc::new(Store::open(&dir.join("unused.db")).expect("store"));
    let state = AppState(Arc::new(Inner {
        cfg,
        store,
        pending: ttl.map(PendingStore::with_ttl).unwrap_or_default(),
        parse_gate: Arc::new(tokio::sync::Semaphore::new(
            xexam_backend::services::PARSE_CONCURRENCY,
        )),
        shutdown: if allow { Some(tx) } else { None },
        engine: engine.clone(),
    }));
    TestApp {
        router: build_router(state.clone()),
        state,
        engine,
        shutdown_rx: Some(rx),
    }
}

pub fn app() -> TestApp {
    app_with(default_config(), None)
}

pub struct Resp {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    pub fn error(&self) -> String {
        self.json()
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }
}

/// 构造请求。`host`：`None` 不带 Host 头。`headers` 里的值按原样（字节）放进去。
pub fn build(
    method: &str,
    path: &str,
    host: Option<&str>,
    headers: &[(&str, &[u8])],
    body: Body,
) -> Request<Body> {
    let mut b = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).expect("method"))
        .uri(path);
    if let Some(h) = host {
        b = b.header(
            "host",
            HeaderValue::from_bytes(h.as_bytes()).expect("host header"),
        );
    }
    let mut req = b.body(body).expect("request");
    for (k, v) in headers {
        req.headers_mut().append(
            HeaderName::from_bytes(k.to_lowercase().as_bytes()).expect("header name"),
            HeaderValue::from_bytes(v).expect("header value"),
        );
    }
    req
}

pub async fn send(router: &Router, req: Request<Body>) -> Resp {
    let resp = router.clone().oneshot(req).await.expect("infallible");
    let (parts, body) = resp.into_parts();
    let bytes = body
        .collect()
        .await
        .map(|c| c.to_bytes().to_vec())
        .unwrap_or_default();
    Resp {
        status: parts.status,
        headers: parts.headers,
        body: bytes,
    }
}

pub async fn get(app: &TestApp, path: &str) -> Resp {
    send(
        &app.router,
        build("GET", path, Some("127.0.0.1:8765"), &[], Body::empty()),
    )
    .await
}

pub async fn post_json(app: &TestApp, path: &str, body: &str) -> Resp {
    post_raw(
        app,
        path,
        body.as_bytes().to_vec(),
        &[("content-type", b"application/json")],
    )
    .await
}

pub async fn post_raw(app: &TestApp, path: &str, body: Vec<u8>, headers: &[(&str, &[u8])]) -> Resp {
    let len = body.len().to_string();
    let mut hs: Vec<(&str, &[u8])> = headers.to_vec();
    hs.push(("content-length", len.as_bytes()));
    send(
        &app.router,
        build("POST", path, Some("127.0.0.1:8765"), &hs, Body::from(body)),
    )
    .await
}

pub async fn upload(app: &TestApp, data: Vec<u8>, filename: Option<&str>) -> Resp {
    let mut hs: Vec<(&str, &[u8])> = vec![("content-type", b"application/octet-stream")];
    if let Some(f) = filename {
        hs.push(("x-filename", f.as_bytes()));
    }
    post_raw(app, "/api/upload", data, &hs).await
}
