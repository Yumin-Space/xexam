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
//!
//! v0.6: sign-in. The guard order becomes Host → (POST only) Origin → **session** → path / Content-Type → size → JSON → routing.
//! The session check is [`auth::authorize`], called right here in the dispatch function for every GET / HEAD / POST before any
//! body is read, against the route table [`auth::ROUTES`]; a path that is not in the table needs a session too. Study records,
//! statistics and settings are read and written for the signed-in user only (their id comes from the session, never from the
//! request).
//!
//! v0.6.5: every route that reads shared bank data or writes into a bank builds the request's [`Viewer`] ([`auth::viewer`])
//! and calls the engine's `*_as` methods: a plain user reads only the banks they may see (`/api/bank`, `/api/banks`, the
//! statistics of `/api/progress`, the questions `/api/attempt` may count, the question text of `/api/record`), and a bank
//! manager imports, updates and deletes only the banks they own. Each uploader has their own pending import.
//!
//! v0.7: `GET /api/logs?after=<seq>&boot=<id>` (administrators only) returns the recent log lines kept in memory (see `crate::logs`).
//! Only the six-column layout may be imported: `POST /api/upload` (a new bank or an update) and `POST /api/import/confirm`
//! refuse a 3- or 4-column workbook with 400 `MSG_OLD_LAYOUT` before anything is staged or written
//! ([`services::check_layout`]; [`Inner::old_layouts`] is the test-only way around it).
//!
//! v0.8: every path that starts with `/internal/` goes to [`internal::dispatch`] first, before the Host guard and without the
//! session layer: the auth step of the AI routes and the token routes the AI service calls (see `api::internal`). The
//! `/api/*` routing above is unchanged.
use crate::config::{Config, MAX_JSON, MAX_UPLOAD};
use crate::error::{AppError, DbError};
use crate::models::{setting_valid, ImportTarget, ParsedWorkbook, Viewer, SETTING_KEYS};
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
pub mod auth;
pub mod guard;
pub mod internal;
pub mod json;
pub mod response;

use auth::Authed;
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
    /// (Accounts and sessions always use `store` directly.)
    pub engine: Arc<dyn Engine>,
    /// At most `auth::HASH_CONCURRENCY` Argon2 operations at a time
    pub hash_gate: Arc<tokio::sync::Semaphore>,
    /// Failed sign-ins (in memory)
    pub throttle: crate::auth::LoginThrottle,
    /// Unix time of the last purge of expired sessions (0 = not yet in this process)
    pub last_session_purge: std::sync::atomic::AtomicI64,
    /// Password hashes that still verify against the default password `admin`, found once at startup
    /// ([`auth::find_default_admin_hashes`]); unset = none known (tests that do not set it)
    pub default_admin_hashes: std::sync::OnceLock<Vec<String>>,
    /// The recent log lines served by `GET /api/logs` (the process-wide buffer fed by the tracing layer, or a test's own)
    pub logs: Arc<crate::logs::LogBuffer>,
    /// Requests rejected by the Host / Origin guard, logged at most about one line a minute (see [`crate::logs::BurstLog`])
    pub rejections: crate::logs::BurstLog,
    /// Failed sign-ins (wrong password, unknown or disabled user), logged in the same way
    pub failed_sign_ins: crate::logs::BurstLog,
    /// Locks of usernames without an account (anyone can start one per made-up name), logged in the same way; locks of
    /// accounts are logged one by one (their number is bounded by the accounts)
    pub name_locks: crate::logs::BurstLog,
    /// v0.7: whether uploads of the old 3- / 4-column layouts are refused. Always [`services::OldLayouts::Refuse`] in
    /// production ([`AppState::new`]); only test code builds a state with `AllowForTests` (the golden suites replay the
    /// reference implementation's 3-column imports).
    pub old_layouts: services::OldLayouts,
    /// v0.8: the link with the AI service: the internal token once `serve` loaded it (none by default: the link is off) and
    /// the rate limit of the log line about refused internal requests (see [`internal::AiLink`])
    pub ai_link: internal::AiLink,
}

impl AppState {
    pub fn new(
        cfg: Config,
        store: Arc<crate::database::Store>,
        engine: Arc<dyn Engine>,
        shutdown: Option<tokio::sync::mpsc::Sender<()>>,
    ) -> AppState {
        Self::new_with_logs(cfg, store, engine, shutdown, crate::logs::global())
    }

    /// Same as [`AppState::new`], with the log buffer that `GET /api/logs` reads (tests pass their own).
    pub fn new_with_logs(
        cfg: Config,
        store: Arc<crate::database::Store>,
        engine: Arc<dyn Engine>,
        shutdown: Option<tokio::sync::mpsc::Sender<()>>,
        logs: Arc<crate::logs::LogBuffer>,
    ) -> AppState {
        AppState(Arc::new(Inner {
            cfg,
            store,
            pending: PendingStore::new(),
            parse_gate: Arc::new(tokio::sync::Semaphore::new(services::PARSE_CONCURRENCY)),
            shutdown,
            engine,
            hash_gate: Arc::new(tokio::sync::Semaphore::new(crate::auth::HASH_CONCURRENCY)),
            throttle: crate::auth::LoginThrottle::default(),
            last_session_purge: std::sync::atomic::AtomicI64::new(0),
            default_admin_hashes: std::sync::OnceLock::new(),
            logs,
            rejections: Default::default(),
            failed_sign_ins: Default::default(),
            name_locks: Default::default(),
            old_layouts: services::OldLayouts::Refuse,
            ai_link: Default::default(),
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

/// The `after` of `/api/logs?after=<seq>`: absent → `None`; present exactly once with 1 to 19 ASCII digits (a `u64`) →
/// that number; anything else (empty, not a number, repeated) → 400.
fn query_after(query: Option<&str>) -> Result<Option<u64>, AppError> {
    match query_once(query, "after")? {
        None => Ok(None),
        Some(s) if (1..=19).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) => {
            s.parse().map(Some).map_err(|_| bad_request())
        }
        Some(_) => Err(bad_request()),
    }
}

/// The value of `key` in `query` (percent-decoded): `Ok(None)` when absent, 400 when it appears more than once.
fn query_once(query: Option<&str>, key: &str) -> Result<Option<String>, AppError> {
    let mut found: Option<String> = None;
    for pair in query.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if percent_decode_str(k).decode_utf8_lossy() == key {
            if found.is_some() {
                return Err(bad_request());
            }
            found = Some(percent_decode_str(v).decode_utf8_lossy().into_owned());
        }
    }
    Ok(found)
}

/// The `boot` of `/api/logs?after=<seq>&boot=<id>` (the log buffer id the client last saw): absent → `None`; present once
/// with 1 to 64 characters of `A-Z a-z 0-9 _ -` → that id; anything else → 400.
fn query_boot(query: Option<&str>) -> Result<Option<String>, AppError> {
    match query_once(query, "boot")? {
        None => Ok(None),
        Some(s)
            if (1..=64).contains(&s.len())
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') =>
        {
            Ok(Some(s))
        }
        Some(_) => Err(bad_request()),
    }
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
        Ok(Err(DbError::NoUser)) => {
            // Deleted or disabled while this request was running: answered like a missing session
            tracing::info!("write refused: the user no longer exists or is disabled");
            Err(AppError::Status(401, response::MSG_SIGN_IN_REQUIRED))
        }
        Ok(Err(DbError::QuestionsGone)) => {
            // An exam started before its question bank was deleted: nothing was saved
            tracing::info!("exam not saved: none of its questions exist any more");
            Err(AppError::Status(409, response::MSG_EXAM_QUESTIONS_GONE))
        }
        Ok(Err(DbError::NotBankOwner)) => {
            // A bank manager aimed at a bank they do not own: nothing was written
            tracing::info!("refused: the user may not manage that question bank");
            Err(AppError::Status(403, response::MSG_NOT_BANK_OWNER))
        }
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

/// Logs a request rejected by the Host / Origin guard: the first one after a quiet minute at once, the rest summed up (see
/// [`crate::logs::BurstLog`]), so a flood of rejected requests cannot push the other lines out of the System log. `what` is
/// English (method, path and [`guard::Denial::log`]), never the Chinese text of the response.
fn note_rejection(st: &AppState, what: String) {
    if let Some(report) = st.0.rejections.note(std::time::Instant::now(), what) {
        guard::log_rejections(&report);
    }
}

async fn dispatch(State(st): State<AppState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    // v0.8: the internal routes of the AI link, before the Host guard (see `internal`)
    if parts.uri.path().starts_with("/internal/") {
        return internal::dispatch(&st, &parts.method, parts.uri.path(), &parts.headers, body)
            .await;
    }
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
                note_rejection(
                    &st,
                    format!(
                        "{} {}: Host {:?} is not in the allowed list",
                        parts.method,
                        safe_path(&path),
                        seen
                    ),
                );
                return error_response(AppError::ForbiddenHost { seen }, head_only);
            }
            // 2. Session (the origin is passed for the cookie refresh rule; GETs from a browser normally have none)
            let origin = hdr(&parts.headers, &header::ORIGIN);
            let authed =
                match auth::authorize(&st, "GET", &path, &parts.headers, origin.as_deref()).await {
                    Ok(a) => a,
                    Err(e) => return error_response(e, head_only),
                };
            let mut resp =
                match get_route(&st, &path, query.as_deref(), head_only, authed.as_ref()).await {
                    Ok(r) => r,
                    Err(e) => error_response(e, head_only),
                };
            auth::finish(&mut resp, authed.as_ref());
            resp
        }
        Method::POST => {
            let (mut resp, authed) =
                post_route(&st, &parts.headers, host.as_deref(), &path, body).await;
            auth::finish(&mut resp, authed.as_ref());
            resp
        }
        _ => {
            // Other methods: the Host guard still runs first (stricter than the old 501), then 405 + Allow
            if let Some(denial) = guard::deny(cfg, host.as_deref(), None) {
                note_rejection(
                    &st,
                    format!("{} {}: {}", parts.method, safe_path(&path), denial.log),
                );
                return error_json(StatusCode::FORBIDDEN, &denial.message, false);
            }
            method_not_allowed("GET, HEAD, POST")
        }
    }
}

// ---------------------------------------------------------------- GET / HEAD

/// The signed-in user's id. `authorize` guarantees a session for every non-public route, so `None` here is a bug: refuse.
fn user_of(authed: Option<&Authed>) -> Result<i64, AppError> {
    authed
        .map(|a| a.user.id)
        .ok_or(AppError::Status(401, response::MSG_SIGN_IN_REQUIRED))
}

/// The signed-in request's [`Viewer`] (same rule as [`user_of`] for a missing session).
async fn viewer_of(st: &AppState, authed: Option<&Authed>) -> Result<Viewer, AppError> {
    match authed {
        Some(a) => auth::viewer(st, a).await,
        None => Err(AppError::Status(401, response::MSG_SIGN_IN_REQUIRED)),
    }
}

async fn get_route(
    st: &AppState,
    path: &str,
    query: Option<&str>,
    head_only: bool,
    authed: Option<&Authed>,
) -> Result<Response, AppError> {
    if auth::find_route("GET", path).is_none() {
        // Static files are served by Nginx; keep the old plain-text 404 here
        return Ok(text_resp(StatusCode::NOT_FOUND, "not found", head_only));
    }
    if path == "/api/auth/state" {
        return Ok(json_resp(
            StatusCode::OK,
            &auth::state(st, authed).await?,
            head_only,
        ));
    }
    if path == "/api/users" {
        let users = auth::list(st).await?;
        return Ok(json_resp(StatusCode::OK, &users, head_only));
    }
    if path == "/api/logs" {
        // Administrators only (`auth::ROUTES`); the buffer is in memory, so no blocking task is needed
        let after = query_after(query)?;
        let boot = query_boot(query)?;
        let page = st.0.logs.since_boot(after, boot.as_deref());
        let value = serde_json::to_value(&page).map_err(|_| AppError::Internal)?;
        return Ok(json_resp(StatusCode::OK, &value, head_only));
    }
    if path == "/api/info" {
        // Public (before sign-in, and the container health check): only what the sign-in page needs. Since v0.7.5 it no longer
        // says how many questions exist (signed-in pages count them from GET /api/bank and GET /api/banks). The database is
        // still read, and the count dropped, so a broken database still answers 500 and the health check reports it.
        let eng = st.0.engine.clone();
        blocking(move || eng.count_active()).await?;
        let value = json!({
            "version": st.0.cfg.version,
            "arch": machine_arch(),
            "canShutdown": st.0.cfg.allow_shutdown,
        });
        return Ok(json_resp(StatusCode::OK, &value, head_only));
    }
    let user = user_of(authed)?;
    let eng = st.0.engine.clone();
    let value: Value = match path {
        "/api/bank" => {
            let v = viewer_of(st, authed).await?;
            blocking(move || eng.bank_as(&v)).await?
        }
        "/api/progress" => {
            let v = viewer_of(st, authed).await?;
            blocking(move || eng.progress_as(&v)).await?
        }
        "/api/banks" => {
            let v = viewer_of(st, authed).await?;
            blocking(move || eng.banks_as(&v)).await?
        }
        "/api/records" => blocking(move || eng.records(user)).await?,
        "/api/record" => {
            let id = query_id(query).ok_or_else(bad_request)?;
            let v = viewer_of(st, authed).await?;
            // Another user's record answers exactly like a record that does not exist
            blocking(move || eng.record_as(&v, id))
                .await?
                .ok_or(AppError::NotFoundMsg(MSG_RECORD_NOT_FOUND))?
        }
        "/api/settings" => blocking(move || eng.settings(user)).await?,
        "/api/imports" => {
            let v = viewer_of(st, authed).await?;
            let list = blocking(move || eng.import_history_as(&v, 12)).await?;
            let items = serde_json::to_value(&list).map_err(|_| AppError::Internal)?;
            json!({ "imports": items })
        }
        _ => {
            // A GET route in auth::ROUTES without a handler here: a bug, never fall through to another route's data
            debug_assert!(false, "GET route without a handler: {path}");
            tracing::error!("GET route without a handler: {}", safe_path(path));
            return Err(AppError::Internal);
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

/// A POST: the response, and the session it was made with (for the cookie refresh).
async fn post_route(
    st: &AppState,
    headers: &HeaderMap,
    host: Option<&str>,
    path: &str,
    mut body: Body,
) -> (Response, Option<Authed>) {
    let cfg = &st.0.cfg;
    let origin = hdr(headers, &header::ORIGIN);
    // 1. Host, Origin
    if let Some(denial) = guard::deny(cfg, host, origin.as_deref()) {
        body::drain(&mut body, 0, DRAIN_CAP).await;
        note_rejection(st, format!("POST {}: {}", safe_path(path), denial.log));
        return (
            error_response(AppError::Denied(denial.message), false),
            None,
        );
    }
    // 2. Session, before anything of the body is read
    let authed = match auth::authorize(st, "POST", path, headers, origin.as_deref()).await {
        Ok(a) => a,
        Err(e) => {
            body::drain(&mut body, 0, DRAIN_CAP).await;
            return (error_response(e, false), None);
        }
    };
    let resp = match post_body(st, headers, path, origin.as_deref(), authed.as_ref(), body).await {
        Ok(r) => r,
        Err(e) => error_response(e, false),
    };
    (resp, authed)
}

/// The rest of a POST after the guards and the session check: Content-Type → size → JSON → routing.
async fn post_body(
    st: &AppState,
    headers: &HeaderMap,
    path: &str,
    origin: Option<&str>,
    authed: Option<&Authed>,
    mut body: Body,
) -> Result<Response, AppError> {
    let cfg = &st.0.cfg;
    // 3. Path / Content-Type
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
        let v = viewer_of(st, authed).await?;
        let username = authed.map(|a| a.user.username.as_str()).unwrap_or("");
        return upload(st, &v, username, target, filename, data.to_vec()).await;
    }

    if py_strip(ctype.split(';').next().unwrap_or("")).to_lowercase() != "application/json" {
        body::drain(&mut body, 0, DRAIN_CAP).await;
        return Err(AppError::UnsupportedMediaType);
    }
    // 4. Size
    let raw = match body::read_limited(body, declared, MAX_JSON).await {
        Ok(r) => r,
        Err(BodyError::TooLarge) => return Err(AppError::JsonTooLarge),
        Err(BodyError::Broken) => return Err(AppError::BadJson),
    };
    // 5. JSON parsing (an empty body is treated as {})
    let data: PyVal = if raw.is_empty() {
        PyVal::Dict(Vec::new())
    } else {
        parse_py_json(&raw).map_err(|_| AppError::BadJson)?
    };
    if !data.is_dict() {
        return Err(AppError::BadJson);
    }
    // 6. Routing
    if path == "/api/auth/login" {
        return auth::login(st, &data, headers, origin).await;
    }
    let Some(a) = authed else {
        // `authorize` lets only the public routes through without a session, and login was handled above
        return Err(AppError::Status(401, response::MSG_SIGN_IN_REQUIRED));
    };
    let user = a.user.id;
    match path {
        "/api/auth/logout" => auth::logout(st, a, origin).await,
        "/api/auth/password" => auth::change_password(st, a, &data, headers, origin).await,
        "/api/users/create" => auth::create(st, a, &data).await,
        "/api/users/update" => auth::update(st, a, &data).await,
        "/api/users/password" => auth::reset_password(st, a, &data).await,
        "/api/users/delete" => auth::delete(st, a, &data).await,
        "/api/users/unlock" => auth::unlock(st, a, &data).await,
        "/api/import/confirm" => confirm(st, a, &data).await,
        "/api/import/cancel" => {
            // Only the caller's own staged upload: another user's preview is never touched
            services::cancel_pending_import_as(&st.0.pending, user);
            Ok(json_resp(StatusCode::OK, &json!({ "ok": true }), false))
        }
        "/api/attempt" => {
            let (attempt, answers) =
                attempt::clean_attempt_with_answers(&data).map_err(AppError::BadRequest)?;
            let v = auth::viewer(st, a).await?;
            let eng = st.0.engine.clone();
            let progress = blocking(move || eng.record_attempt_as(&v, &attempt, &answers)).await?;
            Ok(json_resp(StatusCode::OK, &progress, false))
        }
        "/api/banks/delete" => delete_bank(st, a, &data).await,
        "/api/records/delete" => delete_records(st, a, &data).await,
        "/api/settings" => update_settings(st, user, &data).await,
        "/api/settings/reset" => {
            let eng = st.0.engine.clone();
            let settings = blocking(move || eng.reset_settings(user)).await?;
            Ok(json_resp(StatusCode::OK, &settings, false))
        }
        "/api/reset" => {
            let eng = st.0.engine.clone();
            let progress = blocking(move || eng.reset_progress(user)).await?;
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
    v: &Viewer,
    username: &str,
    target: ImportTarget,
    filename: String,
    data: Vec<u8>,
) -> Result<Response, AppError> {
    // An unknown bank id, or a bank this user may not update, is rejected before spending a parse on it (the write checks
    // again, inside its transaction)
    let eng = st.0.engine.clone();
    let check = v.clone();
    if !blocking(move || eng.check_import_target(&check, target)).await? {
        return Err(AppError::NotFoundMsg(MSG_BANK_NOT_FOUND));
    }
    let timeout = Duration::from_secs(st.0.cfg.parse_timeout_secs.max(1));
    let wb: ParsedWorkbook =
        services::parse_upload(st.0.engine.clone(), st.0.parse_gate.clone(), data, timeout).await?;
    // v0.7: a 3- or 4-column workbook is refused here, before anything is planned or staged (a new bank or an update alike)
    services::check_layout(&wb.questions, st.0.old_layouts, &filename)?;
    let (plan, wb) = services::plan_import_for(st.0.engine.clone(), target, wb).await?;
    let plan = plan.ok_or(AppError::NotFoundMsg(MSG_BANK_NOT_FOUND))?;
    let payload = services::create_pending_import_named(
        &st.0.pending,
        v.user,
        username,
        target,
        filename,
        wb,
        plan,
    );
    Ok(json_resp(StatusCode::OK, &payload, false))
}

async fn confirm(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    // A non-string token (integer, null, missing, ...) never matches: 409
    let token = match data.get("token") {
        Some(PyVal::Str(s)) => s.as_str(),
        _ => "",
    };
    // Read before the token is taken: a failure here leaves the staged upload in place
    let viewer = auth::viewer(st, a).await?;
    let item = services::confirm_pending_import_as(&st.0.pending, a.user.id, token)?;
    // The upload route already refused an old layout; checked again so nothing of that layout can ever be written (the
    // token is consumed, nothing is written)
    services::check_layout(&item.questions, st.0.old_layouts, &item.filename)?;
    // Only an upload that named its target gets `bankId` (the v0.4 response stays byte-identical)
    let explicit = item.target.is_explicit();
    // The database write after the token is taken runs in a separate task: even if the client disconnects mid-request and the request is dropped, the transaction still commits
    let eng = st.0.engine.clone();
    let joined =
        tokio::spawn(async move { services::apply_import_as(eng, viewer, item).await }).await;
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
        // The bank changed owner after the preview (or the permission is gone): nothing was written
        Ok(Err(DbError::NotBankOwner)) => Err(AppError::Status(403, response::MSG_NOT_BANK_OWNER)),
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
/// `{"banks": [...], "bank": <GET /api/bank>, "progress": <GET /api/progress>}`, all three as the signed-in user sees them.
/// An administrator deletes any bank, and `withRecords` removes the study data of every user; a bank manager deletes only the
/// banks they own (403 `MSG_NOT_BANK_OWNER` otherwise), and `withRecords` removes only their own study data.
async fn delete_bank(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    let id = body_id(data.get("id"))?;
    let with_records = body_flag(data.get("withRecords"))?;
    let id = id.ok_or(AppError::NotFoundMsg(MSG_BANK_NOT_FOUND))?;
    let v = auth::viewer(st, a).await?;
    let eng = st.0.engine.clone();
    let actor = v.clone();
    let deleted = blocking(move || eng.delete_bank_as(&actor, id, with_records))
        .await?
        .ok_or(AppError::NotFoundMsg(MSG_BANK_NOT_FOUND))?;
    // The staged upload for that bank was previewed against data that no longer exists
    if st.0.pending.drop_for_deleted_bank(&deleted) {
        tracing::info!(
            "dropped the pending import of deleted bank {:?}",
            deleted.name
        );
    }
    let eng = st.0.engine.clone();
    let body = blocking(move || {
        Ok(json!({
            "banks": take_key(&mut eng.banks_as(&v)?, "banks"),
            "bank": eng.bank_as(&v)?,
            "progress": eng.progress_as(&v)?,
        }))
    })
    .await?;
    Ok(json_resp(StatusCode::OK, &body, false))
}

/// `POST /api/records/delete`: `{"id": <int>}` (that record and its detail; stats unchanged) or
/// `{"all": true, "withStats": <bool, default false>}` → `{"records": [...], "retention": 500, "progress": {...}}`.
/// Only the signed-in user's own records are ever deleted; `progress` is as they see it.
async fn delete_records(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    let user = a.user.id;
    let eng = st.0.engine.clone();
    match (data.get("id"), data.get("all")) {
        (Some(id), None) => {
            if data.get("withStats").is_some() {
                return Err(bad_request());
            }
            let id = body_id(Some(id))?.ok_or(AppError::NotFoundMsg(MSG_RECORD_NOT_FOUND))?;
            if !blocking(move || eng.delete_record(user, id)).await? {
                return Err(AppError::NotFoundMsg(MSG_RECORD_NOT_FOUND));
            }
        }
        (None, Some(PyVal::Bool(true))) => {
            let with_stats = body_flag(data.get("withStats"))?;
            blocking(move || eng.delete_all_records(user, with_stats)).await?;
        }
        _ => return Err(bad_request()),
    }
    let v = auth::viewer(st, a).await?;
    let eng = st.0.engine.clone();
    let body = blocking(move || {
        let mut records = eng.records(user)?;
        Ok(json!({
            "records": take_key(&mut records, "records"),
            "retention": take_key(&mut records, "retention"),
            "progress": eng.progress_as(&v)?,
        }))
    })
    .await?;
    Ok(json_resp(StatusCode::OK, &body, false))
}

/// `POST /api/settings`: any subset of the known keys; unknown keys are ignored. One invalid value → 400, nothing written.
async fn update_settings(st: &AppState, user: i64, data: &PyVal) -> Result<Response, AppError> {
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
    let settings = blocking(move || eng.update_settings(user, &changes)).await?;
    Ok(json_resp(StatusCode::OK, &settings, false))
}
