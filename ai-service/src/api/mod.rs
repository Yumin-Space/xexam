//! HTTP routes of the AI service (spec 5.1–5.3, 5.5, 14.1).
//!
//! One `dispatch` fallback matches method and path (no axum `Json` extractor, no body-limit or timeout layer that could
//! cut the SSE stream); only a `CatchPanicLayer` that answers the JSON 500 `ai.internal`. Order of checks: method →
//! path → identity (`X-Xexam-Identity`, verified with the internal token) → Content-Type (POST) → body (≤ 128 KiB, a
//! JSON object, hand-validated) → identity-row check (stamp, username) → access level → handler.
//!
//! The identity is the only source of "who": user routes take the user id from it and every query filters by it; a
//! foreign id answers exactly like a missing one (404 `ai.notFound`). Only `/api/ai/admin/*` reads or deletes other
//! users' data, and every such read or delete is audited (also every Suspend / Resume of a user's AI+ function).
use crate::app::{AppState, Snapshot};
use crate::codes::ApiError;
use crate::identity::{self, Identity, VerifyError};
use crate::limits::MAX_BODY;
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use axum::Router;
use serde_json::{json, Map, Value};
use std::any::Any;

pub mod admin;
pub mod gen;
pub mod import;
pub mod study;
pub mod user;

/// The access level a route needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    Signed,
    Admin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Status,
    Chats,
    Chat,
    Send,
    Retry,
    Stop,
    ChatsDelete,
    Free,
    Memories,
    MemoriesAdd,
    MemoriesUpdate,
    MemoriesDelete,
    MemoriesSuggest,
    SettingsGet,
    SettingsPost,
    Models,
    Verify,
    Enable,
    Usage,
    Users,
    User,
    AdminChat,
    Clear,
    LimitsGet,
    LimitsPost,
    Suspend,
    Resume,
    Features,
    AdminFree,
    GenStart,
    GenJob,
    GenCancel,
    GenDismiss,
    AdminGen,
    ImportUpload,
    ImportConfirm,
    StudyAsk,
    StudySaved,
    SummaryAsk,
    SummarySaved,
}

/// (path, GET route, POST route)
const ROUTES: &[(&str, Option<Route>, Option<Route>)] = &[
    ("/api/ai/status", Some(Route::Status), None),
    ("/api/ai/chats", Some(Route::Chats), None),
    ("/api/ai/chat", Some(Route::Chat), None),
    ("/api/ai/send", None, Some(Route::Send)),
    ("/api/ai/retry", None, Some(Route::Retry)),
    ("/api/ai/stop", None, Some(Route::Stop)),
    ("/api/ai/chats/delete", None, Some(Route::ChatsDelete)),
    ("/api/ai/free", None, Some(Route::Free)),
    ("/api/ai/memories", Some(Route::Memories), None),
    ("/api/ai/memories/add", None, Some(Route::MemoriesAdd)),
    ("/api/ai/memories/update", None, Some(Route::MemoriesUpdate)),
    ("/api/ai/memories/delete", None, Some(Route::MemoriesDelete)),
    (
        "/api/ai/memories/suggest",
        None,
        Some(Route::MemoriesSuggest),
    ),
    (
        "/api/ai/admin/settings",
        Some(Route::SettingsGet),
        Some(Route::SettingsPost),
    ),
    ("/api/ai/admin/models", None, Some(Route::Models)),
    ("/api/ai/admin/verify", None, Some(Route::Verify)),
    ("/api/ai/admin/enable", None, Some(Route::Enable)),
    ("/api/ai/admin/usage", Some(Route::Usage), None),
    ("/api/ai/admin/users", Some(Route::Users), None),
    ("/api/ai/admin/user", Some(Route::User), None),
    ("/api/ai/admin/chat", Some(Route::AdminChat), None),
    ("/api/ai/admin/clear", None, Some(Route::Clear)),
    (
        "/api/ai/admin/limits",
        Some(Route::LimitsGet),
        Some(Route::LimitsPost),
    ),
    ("/api/ai/admin/suspend", None, Some(Route::Suspend)),
    ("/api/ai/admin/resume", None, Some(Route::Resume)),
    ("/api/ai/admin/features", None, Some(Route::Features)),
    ("/api/ai/admin/free", None, Some(Route::AdminFree)),
    ("/api/ai/gen/start", None, Some(Route::GenStart)),
    ("/api/ai/gen/job", Some(Route::GenJob), None),
    ("/api/ai/gen/cancel", None, Some(Route::GenCancel)),
    ("/api/ai/gen/dismiss", None, Some(Route::GenDismiss)),
    ("/api/ai/admin/gen", Some(Route::AdminGen), None),
    ("/api/ai/import/upload", None, Some(Route::ImportUpload)),
    ("/api/ai/import/confirm", None, Some(Route::ImportConfirm)),
    ("/api/ai/study/ask", None, Some(Route::StudyAsk)),
    ("/api/ai/study/saved", Some(Route::StudySaved), None),
    ("/api/ai/summary/ask", None, Some(Route::SummaryAsk)),
    ("/api/ai/summary/saved", Some(Route::SummarySaved), None),
];

impl Route {
    fn access(self) -> Access {
        match self {
            Route::SettingsGet
            | Route::SettingsPost
            | Route::Models
            | Route::Verify
            | Route::Enable
            | Route::Usage
            | Route::Users
            | Route::User
            | Route::AdminChat
            | Route::Clear
            | Route::LimitsGet
            | Route::LimitsPost
            | Route::Suspend
            | Route::Resume
            | Route::Features
            | Route::AdminFree
            | Route::AdminGen
            | Route::Free => Access::Admin,
            _ => Access::Signed,
        }
    }
}

/// What a handler gets.
pub struct Ctx {
    pub app: AppState,
    pub who: Identity,
    pub body: Map<String, Value>,
    pub query: Option<String>,
}

pub type Answer = Result<Response, ApiError>;

/// The router (shared by `main` and the tests).
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .fallback(dispatch)
        .layer(tower_http::catch_panic::CatchPanicLayer::custom(
            panic_response as fn(Box<dyn Any + Send>) -> Response,
        ))
        .with_state(state)
}

fn panic_response(_err: Box<dyn Any + Send>) -> Response {
    tracing::error!("an AI request handler panicked");
    error_response(&ApiError::internal())
}

fn json_headers(h: &mut HeaderMap) {
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
}

/// A JSON answer (compact, keys in insertion order).
pub fn json_response(status: StatusCode, v: &Value) -> Response {
    let body = serde_json::to_vec(v).unwrap_or_else(|_| b"{}".to_vec());
    let mut r = Response::new(Body::from(body));
    *r.status_mut() = status;
    json_headers(r.headers_mut());
    r
}

pub fn ok(v: Value) -> Answer {
    Ok(json_response(StatusCode::OK, &v))
}

pub fn error_response(e: &ApiError) -> Response {
    let mut r = json_response(e.status, &e.to_json());
    if e.status == StatusCode::METHOD_NOT_ALLOWED {
        r.headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, POST"));
    }
    if let Some(c) = &e.call {
        if let Ok(v) = HeaderValue::from_str(&c.retry_after.to_string()) {
            r.headers_mut().insert(header::RETRY_AFTER, v);
        }
    }
    if let Some(n) = e.retry_header() {
        if let Ok(v) = HeaderValue::from_str(&n.to_string()) {
            r.headers_mut().insert(header::RETRY_AFTER, v);
        }
    }
    r
}

fn healthz(app: &AppState) -> Response {
    json_response(
        StatusCode::OK,
        &json!({"ok": true, "version": app.0.version, "arch": std::env::consts::ARCH}),
    )
}

fn json_content_type(h: &HeaderMap) -> bool {
    let Some(v) = h.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let v = v.to_ascii_lowercase();
    let mut parts = v.split(';').map(str::trim);
    if parts.next() != Some("application/json") {
        return false;
    }
    matches!(
        (parts.next(), parts.next()),
        (None, _) | (Some("charset=utf-8"), None)
    )
}

/// Verifies the identity statement; an HMAC failure re-reads the token once (the token may have been rotated).
fn identify(app: &AppState, headers: &HeaderMap) -> Result<Identity, ApiError> {
    let link = &app.0.link;
    let Some(token) = link.token() else {
        link.note_waiting();
        return Err(ApiError::backend_down());
    };
    let raw = headers.get("x-xexam-identity").map(HeaderValue::as_bytes);
    let now = crate::util::unix_now();
    let mut res = identity::verify(raw, token.as_bytes(), now);
    drop(token);
    if res == Err(VerifyError::BadSignature) && link.note_rejected() {
        if let Some(t) = link.token() {
            res = identity::verify(raw, t.as_bytes(), now);
        }
    }
    match res {
        Ok(who) => {
            link.note_verified();
            Ok(who)
        }
        Err(_) => {
            if app.0.identity_log.allow() {
                tracing::warn!("refused a request without a valid identity (requests must come through Nginx; XEXAM_INTERNAL_TOKEN must be the same for both services)");
            }
            Err(ApiError::backend_down())
        }
    }
}

async fn read_body(req: Request) -> Result<Map<String, Value>, ApiError> {
    let declared = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    if declared.is_some_and(|n| n > MAX_BODY as u64) {
        return Err(ApiError::too_large());
    }
    let bytes: Bytes = axum::body::to_bytes(req.into_body(), MAX_BODY)
        .await
        .map_err(|_| ApiError::too_large())?;
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Map::new());
    }
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(ApiError::bad_request()),
    }
}

async fn dispatch(State(app): State<AppState>, req: Request) -> Response {
    match handle(app, req).await {
        Ok(r) => r,
        Err(e) => error_response(&e),
    }
}

async fn handle(app: AppState, req: Request) -> Answer {
    let method = req.method().clone();
    if method != Method::GET && method != Method::POST {
        return Err(ApiError::method_not_allowed());
    }
    let path = req.uri().path().to_string();
    if path == "/healthz" {
        return if method == Method::GET {
            Ok(healthz(&app))
        } else {
            Err(ApiError::method_not_allowed())
        };
    }
    let Some((_, get, post)) = ROUTES.iter().find(|(p, _, _)| *p == path) else {
        return Err(ApiError::not_found());
    };
    let route = if method == Method::GET { *get } else { *post };
    let Some(route) = route else {
        return Err(ApiError::method_not_allowed());
    };
    let who = identify(&app, req.headers())?;
    // The raw upload replaces only the body step of the guard order (method, path, identity, then its own Content-Type
    // and body rules): the file is not a JSON body and is read by its own reader with the limit of the setting
    if route == Route::ImportUpload {
        app.0.reconcile_identity(&who).await?;
        return import::upload(app, who, req).await;
    }
    let query = req.uri().query().map(str::to_string);
    let body = if method == Method::POST {
        if !json_content_type(req.headers()) {
            return Err(ApiError::bad_media_type());
        }
        read_body(req).await?
    } else {
        Map::new()
    };
    app.0.reconcile_identity(&who).await?;
    if route.access() == Access::Admin && !who.is_admin() {
        return Err(ApiError::admin_only());
    }
    let ctx = Ctx {
        app,
        who,
        body,
        query,
    };
    match route {
        Route::Status => user::status(ctx).await,
        Route::Chats => user::chats(ctx).await,
        Route::Chat => user::chat(ctx).await,
        Route::Send => user::send(ctx).await,
        Route::Retry => user::retry(ctx).await,
        Route::Stop => user::stop(ctx).await,
        Route::ChatsDelete => user::delete_chats(ctx).await,
        Route::Free => user::set_free_mode(ctx).await,
        Route::Memories => user::memories(ctx).await,
        Route::MemoriesAdd => user::memory_add(ctx).await,
        Route::MemoriesUpdate => user::memory_update(ctx).await,
        Route::MemoriesDelete => user::memory_delete(ctx).await,
        Route::MemoriesSuggest => user::memory_suggest(ctx).await,
        Route::SettingsGet => admin::settings_get(ctx).await,
        Route::SettingsPost => admin::settings_post(ctx).await,
        Route::Models => admin::models(ctx).await,
        Route::Verify => admin::verify(ctx).await,
        Route::Enable => admin::enable(ctx).await,
        Route::Usage => admin::usage(ctx).await,
        Route::Users => admin::users(ctx).await,
        Route::User => admin::user(ctx).await,
        Route::AdminChat => admin::chat(ctx).await,
        Route::Clear => admin::clear(ctx).await,
        Route::LimitsGet => admin::limits_get(ctx).await,
        Route::LimitsPost => admin::limits_post(ctx).await,
        Route::Suspend => admin::set_function(ctx, false).await,
        Route::Resume => admin::set_function(ctx, true).await,
        Route::Features => admin::set_features(ctx).await,
        Route::AdminFree => admin::set_free(ctx).await,
        Route::GenStart => gen::start(ctx).await,
        Route::GenJob => gen::job(ctx).await,
        Route::GenCancel => gen::cancel(ctx).await,
        Route::GenDismiss => gen::dismiss(ctx).await,
        Route::AdminGen => gen::admin_list(ctx).await,
        Route::ImportUpload => Err(ApiError::bad_request()),
        Route::ImportConfirm => import::confirm(ctx).await,
        Route::StudyAsk => study::ask(ctx).await,
        Route::StudySaved => study::saved(ctx).await,
        Route::SummaryAsk => study::summary_ask(ctx).await,
        Route::SummarySaved => study::summary_saved(ctx).await,
    }
}

/// `AiUser` (5.1, round-2 contracts 0.3 / 2.7), checked in this order: the AI+ service effectively on (else 403
/// `ai.disabled`); the main backend accepted a state report less than 60 s ago (else 503 `ai.backendDown`); the
/// caller's AI+ service (else 403 `ai.notAllowed`); for a plain user, the AI+ function not suspended (else 403
/// `ai.suspended`).
pub fn require_ai_user(app: &AppState, who: &Identity) -> Result<Snapshot, ApiError> {
    let snap = app.0.snapshot()?;
    if !snap.effective() {
        return Err(ApiError::disabled());
    }
    if !app.0.link_fresh() {
        return Err(ApiError::backend_down());
    }
    if !who.ai_allowed {
        return Err(ApiError::not_allowed());
    }
    if app.0.suspended(who) {
        return Err(ApiError::suspended());
    }
    Ok(snap)
}

fn positive_id(text: &str) -> Option<i64> {
    if (1..=18).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit()) {
        text.parse::<i64>().ok().filter(|n| *n > 0)
    } else {
        None
    }
}

/// A positive integer query parameter that appears exactly once.
pub fn query_id(query: Option<&str>, name: &str) -> Result<i64, ApiError> {
    let mut found = None;
    for pair in query.unwrap_or("").split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == name {
            if found.is_some() {
                return Err(ApiError::bad_request());
            }
            found = Some(positive_id(v).ok_or_else(ApiError::bad_request)?);
        }
    }
    found.ok_or_else(ApiError::bad_request)
}

/// An optional query parameter (raw text) that appears at most once.
pub fn query_text<'a>(query: Option<&'a str>, name: &str) -> Result<Option<&'a str>, ApiError> {
    let mut found = None;
    for pair in query.unwrap_or("").split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == name {
            if found.is_some() {
                return Err(ApiError::bad_request());
            }
            found = Some(v);
        }
    }
    Ok(found)
}

/// A positive integer in the body (`None` when absent).
pub fn body_id(body: &Map<String, Value>, name: &str) -> Result<Option<i64>, ApiError> {
    match body.get(name) {
        None => Ok(None),
        Some(Value::Number(n)) => match n.as_i64() {
            Some(v) if v > 0 => Ok(Some(v)),
            _ => Err(ApiError::bad_request()),
        },
        Some(_) => Err(ApiError::bad_request()),
    }
}

/// Refuses keys other than `allowed`.
pub fn only_keys(body: &Map<String, Value>, allowed: &[&str]) -> Result<(), ApiError> {
    if body.keys().all(|k| allowed.contains(&k.as_str())) {
        Ok(())
    } else {
        Err(ApiError::bad_request())
    }
}

/// `{"id": N}` or `{"all": true}` (exactly one of them).
pub fn id_or_all(body: &Map<String, Value>) -> Result<Option<i64>, ApiError> {
    only_keys(body, &["id", "all"])?;
    match (body_id(body, "id")?, body.get("all")) {
        (Some(id), None) => Ok(Some(id)),
        (None, Some(Value::Bool(true))) => Ok(None),
        _ => Err(ApiError::bad_request()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_types() {
        let mut h = HeaderMap::new();
        assert!(!json_content_type(&h));
        for (v, ok) in [
            ("application/json", true),
            ("Application/JSON; charset=UTF-8", true),
            ("application/json;charset=utf-8", true),
            ("application/json; charset=latin1", false),
            ("text/plain", false),
            ("application/jsonx", false),
            ("application/json; charset=utf-8; x=1", false),
        ] {
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static(v));
            assert_eq!(json_content_type(&h), ok, "{v}");
        }
    }

    #[test]
    fn ids() {
        assert_eq!(query_id(Some("id=5"), "id").ok(), Some(5));
        assert!(query_id(Some("id=5&id=6"), "id").is_err());
        assert!(query_id(Some("id=0"), "id").is_err());
        assert!(query_id(Some("id=-1"), "id").is_err());
        assert!(query_id(Some("id=1e3"), "id").is_err());
        assert!(query_id(None, "id").is_err());
        assert_eq!(query_id(Some("user=2&id=9&_=1"), "id").ok(), Some(9));
        let m = json!({"id": 3}).as_object().cloned().unwrap_or_default();
        assert_eq!(id_or_all(&m).ok(), Some(Some(3)));
        let m = json!({"all": true})
            .as_object()
            .cloned()
            .unwrap_or_default();
        assert_eq!(id_or_all(&m).ok(), Some(None));
        for bad in [
            json!({"all": false}),
            json!({"id": 1, "all": true}),
            json!({}),
            json!({"id": "1"}),
            json!({"id": 1.5}),
        ] {
            let m = bad.as_object().cloned().unwrap_or_default();
            assert!(id_or_all(&m).is_err(), "{bad}");
        }
    }
}
