//! HTTP routes. **Interface contract (do not change signatures).**
//!
//! `build_router(state)` returns the complete `axum::Router`, shared by `main.rs` and the integration tests (`tower::ServiceExt::oneshot`).
//! Behavior follows `docs/zh-CN/history/audit/api-contract.md` and `tests/baseline/golden/`.
//!
//! Guard order (identical to the old version, item by item): Host → (POST only) Origin → path / Content-Type → size → JSON parsing → routing.
//! That is why this does not use axum's `route()` but a single dispatch function: it gives us full control over the order of 404 / 405 / 415.
//!
//! v0.5 routes (same guards): `GET /api/banks`, `GET /api/records`, `GET /api/record?id=`, `GET /api/settings`,
//! `POST /api/banks/delete`, `POST /api/records/delete`, `POST /api/settings`, `POST /api/settings/reset`; `POST /api/upload`
//! takes the optional header `X-Bank-Target`. Contract: `docs/en/api.md`.
use crate::config::{Config, MAX_JSON, MAX_UPLOAD};
use crate::error::{AppError, DbError};
use crate::models::{setting_valid, ImportTarget, ParsedWorkbook, SETTING_KEYS};
use crate::security::body::{self, BodyError};
use crate::security::{clean_filename, latin1, py_int_str, py_strip, strip_ctrl, truncate_chars};
use crate::services::{self, Engine, PendingStore};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderName, Method, StatusCode};
use axum::response::Response;
use axum::Router;
use percent_encoding::percent_decode_str;
use serde_json::{json, Value};
use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

pub mod attempt;
pub mod guard;
pub mod json;
pub mod response;

use json::{parse_py_json, PyVal};
use response::{
    error_json, error_response, json_resp, method_not_allowed, text_resp, MSG_BAD_REQUEST,
    MSG_BAD_SETTINGS, MSG_BAD_TARGET, MSG_BANK_NOT_FOUND, MSG_RECORD_NOT_FOUND,
};

#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

pub struct Inner {
    pub cfg: Config,
    /// The real database handle. Handlers access the database only through `engine`; it is kept here for `main.rs` (shutdown, subcommands).
    pub store: Arc<crate::database::Store>,
    pub pending: PendingStore,
    /// Maximum number of uploads parsed at the same time (see `services::PARSE_CONCURRENCY`)
    pub parse_gate: Arc<tokio::sync::Semaphore>,
    /// Present only with `--allow-shutdown`
    pub shutdown: Option<tokio::sync::mpsc::Sender<()>>,
    /// Single entry point for the parser and the database. Production uses `RealEngine`; tests use a fake implementation.
    pub engine: Arc<dyn Engine>,
}

impl AppState {
    pub fn new(
        cfg: Config,
        store: Arc<crate::database::Store>,
        engine: Arc<dyn Engine>,
        shutdown: Option<tokio::sync::mpsc::Sender<()>>,
    ) -> AppState {
        AppState(Arc::new(Inner {
            cfg,
            store,
            pending: PendingStore::new(),
            parse_gate: Arc::new(tokio::sync::Semaphore::new(services::PARSE_CONCURRENCY)),
            shutdown,
            engine,
        }))
    }
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .fallback(dispatch)
        .layer(tower_http::catch_panic::CatchPanicLayer::custom(
            panic_response as fn(Box<dyn Any + Send>) -> Response,
        ))
        .with_state(state)
}

fn panic_response(_err: Box<dyn Any + Send>) -> Response {
    tracing::error!("a request handler panicked");
    error_json(
        StatusCode::INTERNAL_SERVER_ERROR,
        response::MSG_INTERNAL,
        false,
    )
}

const GET_PATHS: [&str; 8] = [
    "/api/info",
    "/api/bank",
    "/api/progress",
    "/api/imports",
    "/api/banks",
    "/api/records",
    "/api/record",
    "/api/settings",
];
/// Upper limit on the request body that is read and discarded on the client's behalf when rejecting early (403 / 415).
const DRAIN_CAP: usize = MAX_UPLOAD + 65536;

fn hdr(headers: &HeaderMap, name: &HeaderName) -> Option<String> {
    headers.get(name).map(|v| latin1(v.as_bytes()))
}

/// Content-Length from the request headers: missing / empty / not a number → None (count the bytes actually received).
fn declared_length(headers: &HeaderMap) -> Option<i128> {
    let v = headers.get(header::CONTENT_LENGTH)?;
    py_int_str(&latin1(v.as_bytes()))
}

fn safe_path(p: &str) -> String {
    truncate_chars(&strip_ctrl(p), 80)
}

/// A bank / record id written as text (header `X-Bank-Target`, query `id=`): 1 to 18 ASCII digits.
fn id_from_text(s: &str) -> Option<i64> {
    if (1..=18).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) {
        s.parse().ok()
    } else {
        None
    }
}

/// `X-Bank-Target`: absent → the legacy bank; `new` (trimmed, case-insensitive) → a new bank; an id → that bank;
/// anything else → `None` (400).
fn bank_target(headers: &HeaderMap) -> Option<ImportTarget> {
    let Some(raw) = hdr(headers, &HeaderName::from_static("x-bank-target")) else {
        return Some(ImportTarget::Legacy);
    };
    let v = raw.trim_ascii();
    if v.eq_ignore_ascii_case("new") {
        return Some(ImportTarget::New);
    }
    id_from_text(v).map(ImportTarget::Bank)
}

/// The `id` of `/api/record?id=<int>`: exactly one `id` parameter (names and values percent-decoded), whose value is an id.
fn query_id(query: Option<&str>) -> Option<i64> {
    let mut found: Option<String> = None;
    for pair in query?.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if percent_decode_str(k).decode_utf8_lossy() == "id" {
            if found.is_some() {
                return None;
            }
            found = Some(percent_decode_str(v).decode_utf8_lossy().into_owned());
        }
    }
    id_from_text(&found?)
}

fn bad_request() -> AppError {
    AppError::BadRequest(MSG_BAD_REQUEST.to_string())
}

/// An id in a JSON body: must be an integer (a bool is not). `Ok(None)`: an integer outside the id range, which cannot exist.
fn body_id(v: Option<&PyVal>) -> Result<Option<i64>, AppError> {
    match v {
        Some(PyVal::Int(i)) => Ok(i64::try_from(*i).ok()),
        _ => Err(bad_request()),
    }
}

/// An optional bool flag in a JSON body: absent → false; present but not a bool → 400.
fn body_flag(v: Option<&PyVal>) -> Result<bool, AppError> {
    match v {
        None => Ok(false),
        Some(PyVal::Bool(b)) => Ok(*b),
        Some(_) => Err(bad_request()),
    }
}

/// Moves `value[key]` out (`null` when missing).
fn take_key(value: &mut Value, key: &str) -> Value {
    value.get_mut(key).map(Value::take).unwrap_or(Value::Null)
}

async fn blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, DbError> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => {
            tracing::error!("database error: {e}");
            Err(AppError::Db(e))
        }
        Err(e) => {
            tracing::error!("database task failed: {e}");
            Err(AppError::Internal)
        }
    }
}

async fn dispatch(State(st): State<AppState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let host = hdr(&parts.headers, &header::HOST);
    let cfg = &st.0.cfg;
    let path = parts.uri.path().to_string();
    let query = parts.uri.query().map(str::to_string);

    match parts.method {
        Method::GET | Method::HEAD => {
            let head_only = parts.method == Method::HEAD;
            // 1. Host
            if !guard::host_ok(cfg, host.as_deref()) {
                let seen = guard::seen_host(host.as_deref());
                tracing::warn!(
                    "rejected {} {}: Host {:?} is not in the allowed list",
                    parts.method,
                    safe_path(&path),
                    seen
                );
                return error_response(AppError::ForbiddenHost { seen }, head_only);
            }
            match get_route(&st, &path, query.as_deref(), head_only).await {
                Ok(r) => r,
                Err(e) => error_response(e, head_only),
            }
        }
        Method::POST => match post_route(&st, &parts.headers, host.as_deref(), &path, body).await {
            Ok(r) => r,
            Err(e) => error_response(e, false),
        },
        _ => {
            // Other methods: the Host guard still runs first (stricter than the old 501), then 405 + Allow
            if let Some(reason) = guard::deny_reason(cfg, host.as_deref(), None) {
                tracing::warn!("rejected {} {}: {}", parts.method, safe_path(&path), reason);
                return error_json(StatusCode::FORBIDDEN, &reason, false);
            }
            method_not_allowed("GET, HEAD, POST")
        }
    }
}

// ---------------------------------------------------------------- GET / HEAD

async fn get_route(
    st: &AppState,
    path: &str,
    query: Option<&str>,
    head_only: bool,
) -> Result<Response, AppError> {
    if !GET_PATHS.contains(&path) {
        // Static files are served by Nginx; keep the old plain-text 404 here
        return Ok(text_resp(StatusCode::NOT_FOUND, "not found", head_only));
    }
    let eng = st.0.engine.clone();
    let value: Value = match path {
        "/api/info" => {
            let count = blocking(move || eng.count_active()).await?;
            json!({
                "version": st.0.cfg.version,
                "arch": machine_arch(),
                "canShutdown": st.0.cfg.allow_shutdown,
                "bankCount": count,
            })
        }
        "/api/bank" => blocking(move || eng.bank()).await?,
        "/api/progress" => blocking(move || eng.progress()).await?,
        "/api/banks" => blocking(move || eng.banks()).await?,
        "/api/records" => blocking(move || eng.records()).await?,
        "/api/record" => {
            let id = query_id(query).ok_or_else(bad_request)?;
            blocking(move || eng.record(id))
                .await?
                .ok_or(AppError::NotFoundMsg(MSG_RECORD_NOT_FOUND))?
        }
        "/api/settings" => blocking(move || eng.settings()).await?,
        _ => {
            let list = blocking(move || eng.import_history(12)).await?;
            let items = serde_json::to_value(&list).map_err(|_| AppError::Internal)?;
            json!({ "imports": items })
        }
    };
    Ok(json_resp(StatusCode::OK, &value, head_only))
}

/// Approximates Python's `platform.machine()` (display only, a volatile field).
fn machine_arch() -> &'static str {
    match (cfg!(windows), std::env::consts::ARCH) {
        (true, "x86_64") => "AMD64",
        (true, "aarch64") => "ARM64",
        (_, "") => "unknown",
        (_, a) => a,
    }
}

// ---------------------------------------------------------------- POST

async fn post_route(
    st: &AppState,
    headers: &HeaderMap,
    host: Option<&str>,
    path: &str,
    mut body: Body,
) -> Result<Response, AppError> {
    let cfg = &st.0.cfg;
    let origin = hdr(headers, &header::ORIGIN);
    // 1. Host, Origin
    if let Some(reason) = guard::deny_reason(cfg, host, origin.as_deref()) {
        body::drain(&mut body, 0, DRAIN_CAP).await;
        tracing::warn!("rejected POST {}: {}", safe_path(path), reason);
        return Err(AppError::Denied(reason));
    }
    // 2. Path / Content-Type
    let ctype = hdr(headers, &header::CONTENT_TYPE).unwrap_or_default();
    let declared = declared_length(headers);

    if path == "/api/upload" {
        if !ctype.contains("application/octet-stream") {
            body::drain(&mut body, 0, DRAIN_CAP).await;
            return Err(AppError::UnsupportedMediaType);
        }
        let data = match body::read_limited(body, declared, MAX_UPLOAD).await {
            Ok(d) => d,
            Err(BodyError::TooLarge) => {
                tracing::warn!("upload rejected: larger than {} bytes", MAX_UPLOAD);
                return Err(AppError::UploadTooLarge {
                    max_mb: (MAX_UPLOAD / 1024 / 1024) as u64,
                });
            }
            Err(BodyError::Broken) => {
                return Err(AppError::BadRequest(
                    response::MSG_UPLOAD_INCOMPLETE.to_string(),
                ))
            }
        };
        let target =
            bank_target(headers).ok_or_else(|| AppError::BadRequest(MSG_BAD_TARGET.to_string()))?;
        let filename =
            clean_filename(hdr(headers, &HeaderName::from_static("x-filename")).as_deref());
        return upload(st, target, filename, data.to_vec()).await;
    }

    if py_strip(ctype.split(';').next().unwrap_or("")).to_lowercase() != "application/json" {
        body::drain(&mut body, 0, DRAIN_CAP).await;
        return Err(AppError::UnsupportedMediaType);
    }
    // 3. Size
    let raw = match body::read_limited(body, declared, MAX_JSON).await {
        Ok(r) => r,
        Err(BodyError::TooLarge) => return Err(AppError::JsonTooLarge),
        Err(BodyError::Broken) => return Err(AppError::BadJson),
    };
    // 4. JSON parsing (an empty body is treated as {})
    let data: PyVal = if raw.is_empty() {
        PyVal::Dict(Vec::new())
    } else {
        parse_py_json(&raw).map_err(|_| AppError::BadJson)?
    };
    if !data.is_dict() {
        return Err(AppError::BadJson);
    }
    // 5. Routing
    match path {
        "/api/import/confirm" => confirm(st, &data).await,
        "/api/import/cancel" => {
            services::cancel_pending_import(&st.0.pending);
            Ok(json_resp(StatusCode::OK, &json!({ "ok": true }), false))
        }
        "/api/attempt" => {
            let (attempt, answers) =
                attempt::clean_attempt_with_answers(&data).map_err(AppError::BadRequest)?;
            let eng = st.0.engine.clone();
            let progress =
                blocking(move || eng.record_attempt_with_answers(&attempt, &answers)).await?;
            Ok(json_resp(StatusCode::OK, &progress, false))
        }
        "/api/banks/delete" => delete_bank(st, &data).await,
        "/api/records/delete" => delete_records(st, &data).await,
        "/api/settings" => update_settings(st, &data).await,
        "/api/settings/reset" => {
            let eng = st.0.engine.clone();
            let settings = blocking(move || eng.reset_settings()).await?;
            Ok(json_resp(StatusCode::OK, &settings, false))
        }
        "/api/reset" => {
            let eng = st.0.engine.clone();
            let progress = blocking(move || eng.reset_progress()).await?;
            Ok(json_resp(StatusCode::OK, &progress, false))
        }
        "/api/shutdown" => {
            if !cfg.allow_shutdown {
                return Err(AppError::ShutdownDisabled);
            }
            let resp = json_resp(StatusCode::OK, &json!({ "ok": true }), false);
            if let Some(tx) = &st.0.shutdown {
                let _ = tx.try_send(());
            }
            Ok(resp)
        }
        _ => Err(AppError::NotFound),
    }
}

async fn upload(
    st: &AppState,
    target: ImportTarget,
    filename: String,
    data: Vec<u8>,
) -> Result<Response, AppError> {
    // An unknown bank id is rejected before spending a parse on it (and checked again by the plan)
    if let ImportTarget::Bank(id) = target {
        let eng = st.0.engine.clone();
        if !blocking(move || eng.bank_exists(id)).await? {
            return Err(AppError::NotFoundMsg(MSG_BANK_NOT_FOUND));
        }
    }
    let timeout = Duration::from_secs(st.0.cfg.parse_timeout_secs.max(1));
    let wb: ParsedWorkbook =
        services::parse_upload(st.0.engine.clone(), st.0.parse_gate.clone(), data, timeout).await?;
    let (plan, wb) = services::plan_import_for(st.0.engine.clone(), target, wb).await?;
    let plan = plan.ok_or(AppError::NotFoundMsg(MSG_BANK_NOT_FOUND))?;
    let payload = services::create_pending_import_for(&st.0.pending, target, filename, wb, plan);
    Ok(json_resp(StatusCode::OK, &payload, false))
}

async fn confirm(st: &AppState, data: &PyVal) -> Result<Response, AppError> {
    // A non-string token (integer, null, missing, ...) never matches: 409
    let token = match data.get("token") {
        Some(PyVal::Str(s)) => s.as_str(),
        _ => "",
    };
    let item = services::confirm_pending_import(&st.0.pending, token)?;
    // Only an upload that named its target gets `bankId` (the v0.4 response stays byte-identical)
    let explicit = item.target.is_explicit();
    // The database write after the token is taken runs in a separate task: even if the client disconnects mid-request and the request is dropped, the transaction still commits
    let eng = st.0.engine.clone();
    let joined = tokio::spawn(async move { services::apply_import(eng, item).await }).await;
    match joined {
        Ok(Ok(Some(done))) => {
            let body = if explicit {
                json!({ "result": done.result, "bank": done.bank, "bankId": done.bank_id })
            } else {
                json!({ "result": done.result, "bank": done.bank })
            };
            Ok(json_resp(StatusCode::OK, &body, false))
        }
        // The target bank was deleted after the preview: nothing was written (the token is consumed)
        Ok(Ok(None)) => Err(AppError::NotFoundMsg(MSG_BANK_NOT_FOUND)),
        Ok(Err(e)) => {
            tracing::error!("apply_import failed: {e}");
            Ok(error_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                services::MSG_APPLY_FAILED,
                false,
            ))
        }
        Err(e) => {
            tracing::error!("apply_import task failed: {e}");
            Ok(error_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                services::MSG_APPLY_FAILED,
                false,
            ))
        }
    }
}

/// `POST /api/banks/delete` `{"id": <int>, "withRecords": <bool, default false>}` →
/// `{"banks": [...], "bank": <GET /api/bank>, "progress": <GET /api/progress>}`.
async fn delete_bank(st: &AppState, data: &PyVal) -> Result<Response, AppError> {
    let id = body_id(data.get("id"))?;
    let with_records = body_flag(data.get("withRecords"))?;
    let id = id.ok_or(AppError::NotFoundMsg(MSG_BANK_NOT_FOUND))?;
    let eng = st.0.engine.clone();
    let deleted = blocking(move || eng.delete_bank(id, with_records))
        .await?
        .ok_or(AppError::NotFoundMsg(MSG_BANK_NOT_FOUND))?;
    // The staged upload for that bank was previewed against data that no longer exists
    if st.0.pending.drop_for_deleted_bank(deleted) {
        tracing::info!("dropped the pending import of deleted bank {}", deleted.id);
    }
    let eng = st.0.engine.clone();
    let body = blocking(move || {
        Ok(json!({
            "banks": take_key(&mut eng.banks()?, "banks"),
            "bank": eng.bank()?,
            "progress": eng.progress()?,
        }))
    })
    .await?;
    Ok(json_resp(StatusCode::OK, &body, false))
}

/// `POST /api/records/delete`: `{"id": <int>}` (that record and its detail; stats unchanged) or
/// `{"all": true, "withStats": <bool, default false>}` → `{"records": [...], "retention": 500, "progress": {...}}`.
async fn delete_records(st: &AppState, data: &PyVal) -> Result<Response, AppError> {
    let eng = st.0.engine.clone();
    match (data.get("id"), data.get("all")) {
        (Some(id), None) => {
            if data.get("withStats").is_some() {
                return Err(bad_request());
            }
            let id = body_id(Some(id))?.ok_or(AppError::NotFoundMsg(MSG_RECORD_NOT_FOUND))?;
            if !blocking(move || eng.delete_record(id)).await? {
                return Err(AppError::NotFoundMsg(MSG_RECORD_NOT_FOUND));
            }
        }
        (None, Some(PyVal::Bool(true))) => {
            let with_stats = body_flag(data.get("withStats"))?;
            blocking(move || eng.delete_all_records(with_stats)).await?;
        }
        _ => return Err(bad_request()),
    }
    let eng = st.0.engine.clone();
    let body = blocking(move || {
        let mut records = eng.records()?;
        Ok(json!({
            "records": take_key(&mut records, "records"),
            "retention": take_key(&mut records, "retention"),
            "progress": eng.progress()?,
        }))
    })
    .await?;
    Ok(json_resp(StatusCode::OK, &body, false))
}

/// `POST /api/settings`: any subset of the known keys; unknown keys are ignored. One invalid value → 400, nothing written.
async fn update_settings(st: &AppState, data: &PyVal) -> Result<Response, AppError> {
    let mut changes: Vec<(String, Value)> = Vec::new();
    for key in SETTING_KEYS {
        let Some(v) = data.get(key) else { continue };
        let value = match v {
            PyVal::Str(s) => Value::from(s.as_str()),
            PyVal::Bool(b) => Value::Bool(*b),
            _ => Value::Null,
        };
        if !setting_valid(key, &value) {
            return Err(AppError::BadRequest(MSG_BAD_SETTINGS.to_string()));
        }
        changes.push((key.to_string(), value));
    }
    let eng = st.0.engine.clone();
    let settings = blocking(move || eng.update_settings(&changes)).await?;
    Ok(json_resp(StatusCode::OK, &settings, false))
}
