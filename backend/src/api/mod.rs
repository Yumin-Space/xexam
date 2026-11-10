//! HTTP routes. **Interface contract (do not change signatures).**
//!
//! `build_router(state)` returns the complete `axum::Router`, shared by `main.rs` and the integration tests (`tower::ServiceExt::oneshot`).
//! Behavior follows `docs/audit/api-contract.md` and `tests/baseline/golden/`.
//!
//! Guard order (identical to the old version, item by item): Host → (POST only) Origin → path / Content-Type → size → JSON parsing → routing.
//! That is why this does not use axum's `route()` but a single dispatch function: it gives us full control over the order of 404 / 405 / 415.
use crate::config::{Config, MAX_JSON, MAX_UPLOAD};
use crate::error::{AppError, DbError};
use crate::models::ParsedWorkbook;
use crate::security::body::{self, BodyError};
use crate::security::{clean_filename, latin1, py_int_str, py_strip, strip_ctrl, truncate_chars};
use crate::services::{self, Engine, PendingStore};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderName, Method, StatusCode};
use axum::response::Response;
use axum::Router;
use serde_json::{json, Value};
use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

pub mod attempt;
pub mod guard;
pub mod json;
pub mod response;

use json::{parse_py_json, PyVal};
use response::{error_json, error_response, json_resp, method_not_allowed, text_resp};

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

const GET_PATHS: [&str; 4] = ["/api/info", "/api/bank", "/api/progress", "/api/imports"];
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
            match get_route(&st, &path, head_only).await {
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

async fn get_route(st: &AppState, path: &str, head_only: bool) -> Result<Response, AppError> {
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
        let filename =
            clean_filename(hdr(headers, &HeaderName::from_static("x-filename")).as_deref());
        return upload(st, filename, data.to_vec()).await;
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
            let attempt = attempt::clean_attempt(&data).map_err(AppError::BadRequest)?;
            let eng = st.0.engine.clone();
            let progress = blocking(move || eng.record_attempt(&attempt)).await?;
            Ok(json_resp(StatusCode::OK, &progress, false))
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

async fn upload(st: &AppState, filename: String, data: Vec<u8>) -> Result<Response, AppError> {
    let timeout = Duration::from_secs(st.0.cfg.parse_timeout_secs.max(1));
    let wb: ParsedWorkbook =
        services::parse_upload(st.0.engine.clone(), st.0.parse_gate.clone(), data, timeout).await?;
    let (plan, wb) = services::plan_import(st.0.engine.clone(), wb).await?;
    let payload = services::create_pending_import(&st.0.pending, filename, wb, plan);
    Ok(json_resp(StatusCode::OK, &payload, false))
}

async fn confirm(st: &AppState, data: &PyVal) -> Result<Response, AppError> {
    // A non-string token (integer, null, missing, ...) never matches: 409
    let token = match data.get("token") {
        Some(PyVal::Str(s)) => s.as_str(),
        _ => "",
    };
    let item = services::confirm_pending_import(&st.0.pending, token)?;
    // The database write after the token is taken runs in a separate task: even if the client disconnects mid-request and the request is dropped, the transaction still commits
    let eng = st.0.engine.clone();
    let joined = tokio::spawn(async move { services::apply_import(eng, item).await }).await;
    match joined {
        Ok(Ok((result, bank))) => Ok(json_resp(
            StatusCode::OK,
            &json!({ "result": result, "bank": bank }),
            false,
        )),
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
