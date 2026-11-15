//! Sign-in (v0.6): the route table with each route's access rule, the one place that enforces it ([`authorize`], called by the
//! dispatch function for every GET / HEAD / POST), and the handlers of `/api/auth/*` and `/api/users*`.
//!
//! Rules (contract: `docs/en/api.md`):
//! - Public: `GET /api/info`, `GET /api/auth/state`, `POST /api/auth/login`. Every other path, known or not, needs a valid
//!   session (401 `MSG_SIGN_IN_REQUIRED`); unknown paths then answer 404 as before.
//! - A user who must change their password reaches only `GET /api/auth/state`, `POST /api/auth/password`,
//!   `POST /api/auth/logout` and `GET /api/info`; everything else is 403 `MSG_PASSWORD_CHANGE_REQUIRED`.
//! - Admin routes answer 403 `MSG_ADMIN_ONLY` to other users.
//! - Bank management routes (v0.6.5: upload, import confirm / cancel, import history, bank delete) answer 403
//!   `MSG_NO_BANK_PERMISSION` to plain users without the "can manage question banks" permission. Which banks a bank manager may
//!   update or delete (only their own) is checked by the handlers, against the database, inside the write.
//! - The session is read from the `xexam_session` cookie only. v1.3: how long a session lives depends on the user's current
//!   role and the "keep me signed in" box ([`auth::session_deadline`]): administrators 24 h idle / 7 days hard, users 30 days
//!   sliding or 12 h. A request that counts as activity (every route but the ones with `activity: false`) touches the session
//!   in the database at most once a minute ([`auth::SessionGates`]); the cookie of a "remember" session is re-sent at most
//!   hourly, only on requests whose scheme is known (`XEXAM_COOKIE_SECURE=1`, or an `Origin`, which every browser POST has),
//!   so a re-send never drops `Secure`. An ended session is deleted, recorded (`auth.expired`) and answered with 401
//!   `auth.expired`.
//! - v0.7: the System log names users by their username, never by id (`user "admin" updated user "alex"`); the names come
//!   from the request's session (the actor) and from the same transaction as the change (the user changed).
//! - v0.8: [`introspect`] is the auth step of the AI routes (`GET /internal/ai/auth`, see `api::internal`): the same guard and
//!   session checks for a request that Nginx is about to pass to the AI service. `/internal/*` is not in [`ROUTES`] and
//!   never reaches [`authorize`]. Users carry the AI+ switch (`"ai"` in `/api/users*`, `"aiAllowed"` in the signed-in user).
//!   Whether AI+ is available comes from ai-service's last heartbeat (`api::internal::AiLink`, in memory):
//!   `"aiAvailable"` in `GET /api/auth/state` (signed in; administrators also get `"aiService"`) and in every `/api/users*`
//!   list answer (rows also carry `"aiSuspended"`); while AI+ is not available, a user's AI+ service cannot be switched
//!   (409 [`MSG_AI_SERVICE_OFF`](super::response::MSG_AI_SERVICE_OFF), nothing written).
use super::response::{
    json_resp, MSG_ADMIN_ONLY, MSG_BAD_LOGIN, MSG_BAD_PASSWORD, MSG_BAD_REQUEST, MSG_BAD_USERNAME,
    MSG_LAST_ADMIN, MSG_NOT_YOURSELF, MSG_NO_BANK_PERMISSION, MSG_PASSWORD_CHANGE_REQUIRED,
    MSG_SAME_PASSWORD, MSG_SIGN_IN_REQUIRED, MSG_UNKNOWN_BANKS, MSG_USERNAME_TAKEN,
    MSG_USER_NOT_FOUND, MSG_WRONG_PASSWORD,
};
use super::{blocking, body_flag, body_id, AppState, PyVal};
use crate::activity::{Actor, ClientInfo, Entry};
use crate::auth::throttle::{retry_after_secs, Begun, StartedLock};
use crate::auth::{self, Ended, LockStatus, Role, Rule, SessionTimes, Target, SESSION_PURGE_SECS};
use crate::database::{
    EndedSession, NewSession, SessionUser, SigninInfo, Touch, UserChange, UserUpdate,
};
use crate::error::{AppError, DbError};
use crate::logs::BurstReport;
use crate::models::{BankAccess, Viewer};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;

/// The 401 body text of an ended session (v1.3). The answer should carry `"code": "auth.expired"`, see [`expired`].
pub const MSG_SESSION_EXPIRED: &str = "登录已过期，请重新登录。";

/// Who may use a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// No session needed
    Public,
    /// Any signed-in user
    SignedIn,
    /// Signed-in administrators only
    Admin,
    /// Administrators and plain users with the "can manage question banks" permission (v0.6.5)
    BankManager,
}

/// One route of the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    /// `"GET"` (also used for HEAD) or `"POST"`
    pub method: &'static str,
    pub path: &'static str,
    pub access: Access,
    /// Reachable while the user must still choose a new password
    pub during_password_change: bool,
    /// v1.3: whether a request counts as activity (touches the session and `users.last_active_at`). `false` for polls and
    /// state reads, so a page that only polls never keeps a session alive. Such a route still authenticates.
    pub activity: bool,
}

const fn route(method: &'static str, path: &'static str, access: Access) -> Route {
    Route {
        method,
        path,
        access,
        during_password_change: false,
        activity: true,
    }
}

/// A route that does not count as activity.
const fn quiet(method: &'static str, path: &'static str, access: Access) -> Route {
    Route {
        activity: false,
        ..route(method, path, access)
    }
}

/// Every route the backend serves, with its access rule. The dispatch function answers 404 for anything else.
pub const ROUTES: &[Route] = &[
    quiet("GET", "/api/info", Access::Public),
    quiet("GET", "/api/auth/state", Access::Public),
    route("GET", "/api/bank", Access::SignedIn),
    route("GET", "/api/banks", Access::SignedIn),
    route("GET", "/api/progress", Access::SignedIn),
    route("GET", "/api/imports", Access::BankManager),
    route("GET", "/api/records", Access::SignedIn),
    route("GET", "/api/record", Access::SignedIn),
    route("GET", "/api/settings", Access::SignedIn),
    route("GET", "/api/users", Access::Admin),
    quiet("GET", "/api/logs", Access::Admin),
    quiet("GET", "/api/activity", Access::Admin),
    quiet("GET", "/api/activity/settings", Access::Admin),
    route("POST", "/api/auth/login", Access::Public),
    Route {
        during_password_change: true,
        ..route("POST", "/api/auth/logout", Access::SignedIn)
    },
    Route {
        during_password_change: true,
        ..route("POST", "/api/auth/password", Access::SignedIn)
    },
    route("POST", "/api/upload", Access::BankManager),
    route("POST", "/api/import/confirm", Access::BankManager),
    route("POST", "/api/import/cancel", Access::BankManager),
    route("POST", "/api/banks/delete", Access::BankManager),
    route("POST", "/api/banks/sheets/delete", Access::BankManager),
    route("POST", "/api/attempt", Access::SignedIn),
    route("POST", "/api/records/delete", Access::SignedIn),
    route("POST", "/api/reset", Access::SignedIn),
    route("POST", "/api/settings", Access::SignedIn),
    route("POST", "/api/settings/reset", Access::SignedIn),
    route("POST", "/api/shutdown", Access::Admin),
    route("POST", "/api/users/create", Access::Admin),
    route("POST", "/api/users/update", Access::Admin),
    route("POST", "/api/users/password", Access::Admin),
    route("POST", "/api/users/delete", Access::Admin),
    route("POST", "/api/users/unlock", Access::Admin),
    route("POST", "/api/users/signin-info", Access::Admin),
    route("POST", "/api/users/signin-info/clear", Access::Admin),
    route("POST", "/api/activity/settings", Access::Admin),
    route("POST", "/api/activity/visit", Access::SignedIn),
];

/// The route for `method` (`GET` or `POST`) and `path`, if the backend serves it.
pub fn find_route(method: &str, path: &str) -> Option<&'static Route> {
    ROUTES.iter().find(|r| r.method == method && r.path == path)
}

/// A request with a valid session.
#[derive(Debug, Clone)]
pub struct Authed {
    pub user: SessionUser,
    /// SHA-256 of the cookie token (the key of the `sessions` row)
    pub token_hash: String,
    token: String,
    /// The cookie of this "remember" session is due for a re-send: the response carries it again
    refreshed: bool,
    /// Whether a re-sent cookie gets `Secure`
    secure: bool,
    /// v1.3: the lifetime rule of the session (by the user's current role and the stored "keep me signed in")
    pub rule: Rule,
    /// v1.3: the end of an administrator's hard limit, unix seconds (`None` for users)
    pub hard_ends_at: Option<i64>,
    /// v1.3: the stored "keep me signed in" of the session
    pub remember: bool,
    /// v1.3: the address, device and visitor location of this request (from the Nginx headers)
    pub client: ClientInfo,
}

fn status(code: StatusCode, msg: &'static str) -> AppError {
    AppError::Status(code.as_u16(), msg)
}

fn bad_request() -> AppError {
    AppError::BadRequest(MSG_BAD_REQUEST.to_string())
}

/// The error of a request whose session ended: 401, [`MSG_SESSION_EXPIRED`].
pub fn expired() -> AppError {
    AppError::SessionExpired
}

/// Unix seconds as `YYYY-MM-DD HH:MM:SS` in the local time zone (the format of the other times of the API).
pub fn local_second(secs: i64) -> String {
    jiff::Timestamp::from_second(secs)
        .map(|t| crate::database::timefmt::format_second(&t.to_zoned(jiff::tz::TimeZone::system())))
        .unwrap_or_default()
}

/// `{"rule", "idleSecs", "hardEndsAt"}`: what a session answers about itself (`POST /api/auth/login`, `GET /api/auth/state`).
fn session_json(rule: Rule, hard_ends_at: Option<i64>) -> Value {
    json!({
        "rule": rule.as_str(),
        "idleSecs": rule.idle_secs(),
        "hardEndsAt": hard_ends_at.map(local_second),
    })
}

fn client_of_row(ip: Option<&str>, ua: Option<&str>) -> ClientInfo {
    ClientInfo {
        ip: ip.and_then(|i| i.parse().ok()),
        ua: ua.map(str::to_string),
        ..ClientInfo::default()
    }
}

/// The `auth.expired` entry of a session that ended: the session's stored address and device, at the time it ended.
fn expired_entry(
    actor: Actor,
    rule: Rule,
    ended: Ended,
    deadline: i64,
    client: &ClientInfo,
) -> Entry {
    let e = Entry::new(actor, "auth.expired")
        .fail(ended.as_str())
        .detail(json!({ "rule": rule.as_str() }))
        .client(client);
    Entry {
        at: Some(deadline),
        ..e
    }
}

/// What the lookup of a session found.
enum Looked {
    /// No such session (or its user is disabled)
    None,
    /// The session ended: its row is deleted, the entry is recorded
    Ended,
    Found(Found),
}

struct Found {
    user: SessionUser,
    rule: Rule,
    hard_ends_at: Option<i64>,
    remember: bool,
    resend: bool,
}

/// Deletes every session that ended before `now` and records one `auth.expired` entry for each, at its deadline (the
/// entry names the session's stored address and device), plus one `system.sessionsPurged` row with the count. Returns how
/// many were deleted. Blocking (database).
pub fn purge_ended_sessions(st: &AppState, now: i64) -> Result<usize, DbError> {
    let store = &st.0.store;
    let ended = store.purge_expired_sessions_ended(now)?;
    let n = ended.len();
    let mut users: std::collections::HashMap<i64, Option<(String, Role)>> =
        std::collections::HashMap::new();
    for e in &ended {
        record_purged(st, e, &mut users);
    }
    if n > 0 {
        st.0.activity.record(Entry::system(
            "system.sessionsPurged",
            Some(json!({ "count": n })),
        ));
    }
    st.0.gates.prune(now);
    Ok(n)
}

fn record_purged(
    st: &AppState,
    e: &EndedSession,
    users: &mut std::collections::HashMap<i64, Option<(String, Role)>>,
) {
    let who = users
        .entry(e.user_id)
        .or_insert_with(|| {
            st.0.store
                .user_by_id(e.user_id)
                .ok()
                .flatten()
                .map(|u| (u.username, u.role))
        })
        .clone();
    let Some((username, role)) = who else { return };
    let times = SessionTimes {
        started_at: e.started_at.unwrap_or(e.last_seen_at),
        last_seen_at: e.last_seen_at,
        remember: e.remember,
    };
    let (deadline, ended) = auth::session_deadline(role, &times);
    let rule = auth::session_rule(role, e.remember);
    let client = client_of_row(e.ip.as_deref(), e.ua.as_deref());
    st.0.activity.record(expired_entry(
        Actor::User {
            id: e.user_id,
            username,
            role,
        },
        rule,
        ended,
        deadline,
        &client,
    ));
}

/// The session `token_hash` at `now` (v1.3). Purges ended sessions at most once an hour (whichever request comes first claims
/// it). An ended session (by the user's current role) is deleted, recorded and answered [`Looked::Ended`]. With `counts` the
/// request is activity: the session row and `users.last_active_at` are written at most once a minute per session / user
/// ([`auth::SessionGates`]); `may_resend` additionally allows the cookie re-send of a "remember" session (at most hourly).
async fn session_lookup(
    st: &AppState,
    token_hash: String,
    now: i64,
    counts: bool,
    may_resend: bool,
) -> Result<Looked, AppError> {
    let last = st.0.last_session_purge.load(Ordering::Relaxed);
    let purge = now.saturating_sub(last) >= SESSION_PURGE_SECS
        && st
            .0
            .last_session_purge
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok();
    let st = st.clone();
    blocking(move || {
        let store = &st.0.store;
        if purge {
            let n = purge_ended_sessions(&st, now)?;
            if n > 0 {
                tracing::info!("purged {n} expired sessions");
            }
        }
        let Some((user, row)) = store.session_user_ro(&token_hash)? else {
            return Ok(Looked::None);
        };
        let times = SessionTimes {
            started_at: row.started_at.unwrap_or(row.last_seen_at),
            last_seen_at: row.last_seen_at,
            remember: row.remember,
        };
        let (deadline, ended) = auth::session_deadline(user.role, &times);
        let (rule, hard_ends_at) = auth::session_info(user.role, &times);
        if now >= deadline {
            // Whoever finds it first deletes it and writes the one entry
            if st.0.gates.claim_end(&token_hash, now) {
                store.delete_session(&token_hash)?;
                st.0.gates.forget(&token_hash);
                st.0.activity.record(expired_entry(
                    Actor::of(&user),
                    rule,
                    ended,
                    deadline,
                    &client_of_row(row.ip.as_deref(), row.ua.as_deref()),
                ));
            }
            return Ok(Looked::Ended);
        }
        let mut resend = false;
        if counts {
            let plan = st.0.gates.plan(&token_hash, user.id, now);
            if plan.session || plan.active {
                let fresh = SessionTimes {
                    last_seen_at: now,
                    ..times
                };
                let touch = Touch {
                    session: plan.session,
                    active: plan.active,
                    expires_at: auth::session_deadline(user.role, &fresh).0,
                };
                // A failed touch must not fail the request that was only about to read something
                if let Err(e) = store.touch_session(&token_hash, user.id, now, &touch) {
                    tracing::warn!("could not record the session activity: {e}");
                }
            }
            resend =
                may_resend && rule == Rule::Remember && st.0.gates.cookie_due(&token_hash, now);
        }
        Ok(Looked::Found(Found {
            user,
            rule,
            hard_ends_at,
            remember: row.remember,
            resend,
        }))
    })
    .await
}

/// The user of the session `token_hash` for the internal routes of the AI link (the answer being written re-checks it):
/// the same deadline rule, the same touch (an AI conversation keeps the session alive), never a cookie. `None` = no valid
/// session, an ended one included (deleted and recorded like any other).
pub async fn session_user_checked(
    st: &AppState,
    token_hash: &str,
) -> Result<Option<SessionUser>, AppError> {
    let found = session_lookup(st, token_hash.to_string(), auth::unix_now(), true, false).await?;
    Ok(match found {
        Looked::Found(f) => Some(f.user),
        _ => None,
    })
}

/// [`session_user_checked`] for the study and generation calls: the same deadline rule (an ended session is deleted and
/// recorded), but nothing is written for a live one (no touch, no refresh).
pub async fn session_user_untouched(
    st: &AppState,
    token_hash: &str,
) -> Result<Option<SessionUser>, AppError> {
    let found = session_lookup(st, token_hash.to_string(), auth::unix_now(), false, false).await?;
    Ok(match found {
        Looked::Found(f) => Some(f.user),
        _ => None,
    })
}

/// Looks up the session of the request (if it carries a token-shaped cookie). `counts`: the route counts as activity.
/// `Err` with [`expired`] when the session ended.
async fn session(
    st: &AppState,
    headers: &HeaderMap,
    origin: Option<&str>,
    counts: bool,
) -> Result<Option<Authed>, AppError> {
    let Some(token) = auth::session_token(headers) else {
        return Ok(None);
    };
    let force_secure = st.0.cfg.cookie_secure;
    let may_resend = force_secure || origin.is_some();
    let secure = auth::cookie_secure(force_secure, origin);
    let now = auth::unix_now();
    let token_hash = auth::token_hash(&token);
    let client = ClientInfo::from_headers(headers);
    let looked = session_lookup(st, token_hash.clone(), now, counts, may_resend).await?;
    match looked {
        Looked::None => Ok(None),
        Looked::Ended => Err(expired()),
        Looked::Found(f) => Ok(Some(Authed {
            user: f.user,
            token_hash,
            token,
            refreshed: f.resend,
            secure,
            rule: f.rule,
            hard_ends_at: f.hard_ends_at,
            remember: f.remember,
            client,
        })),
    }
}

/// The single enforcement point, run after the Host guard (GET) or the Host and Origin guards (POST) and before the body is read.
/// `Ok(None)`: a public route without a session. Never skips the check for a non-public route.
pub async fn authorize(
    st: &AppState,
    method: &str,
    path: &str,
    headers: &HeaderMap,
    origin: Option<&str>,
) -> Result<Option<Authed>, AppError> {
    let route = find_route(method, path);
    let access = route.map(|r| r.access).unwrap_or(Access::SignedIn);
    // The two public routes that never need to know who is asking: no database lookup at all
    if access == Access::Public && path != "/api/auth/state" {
        return Ok(None);
    }
    let counts = route.is_none_or(|r| r.activity);
    let authed = session(st, headers, origin, counts).await?;
    if access == Access::Public {
        return Ok(authed);
    }
    let Some(a) = authed else {
        return Err(status(StatusCode::UNAUTHORIZED, MSG_SIGN_IN_REQUIRED));
    };
    if a.user.must_change_password && !route.is_some_and(|r| r.during_password_change) {
        return Err(status(StatusCode::FORBIDDEN, MSG_PASSWORD_CHANGE_REQUIRED));
    }
    if access == Access::Admin && !a.user.is_admin() {
        return Err(status(StatusCode::FORBIDDEN, MSG_ADMIN_ONLY));
    }
    // `can_manage_banks` is true for every administrator (see `SessionUser`) and read from the database with the session
    if access == Access::BankManager && !a.user.can_manage_banks {
        return Err(status(StatusCode::FORBIDDEN, MSG_NO_BANK_PERMISSION));
    }
    Ok(Some(a))
}

/// What the auth step of an AI request found ([`introspect`], `GET /internal/ai/auth`, v0.8).
#[derive(Debug)]
pub enum IntrospectOutcome {
    /// No internal token is loaded (the AI link is off): 503 without a body
    LinkOff,
    /// The Host / Origin guard refused the request: 403, `X-Xexam-Deny: guard`
    Guard(super::guard::Denial),
    /// No valid session (none, expired, unknown, or the user is disabled): 401, `X-Xexam-Deny: signin`
    SignIn,
    /// The user must choose a new password first: 403, `X-Xexam-Deny: password`
    Password,
    /// The database could not be read: 500 (Nginx answers that like an unreachable backend)
    Failed,
    /// Signed in: 204 with this identity statement in `X-Xexam-Identity`
    Allowed(String),
}

/// The auth step of every `/api/ai/*` request (Nginx `auth_request`, the vite plugin), in the same order as [`authorize`]:
/// the AI link must be on; the Host / Origin guard runs on the `X-Original-Host` / `X-Original-Origin` headers (the
/// Origin only for a method other than GET / HEAD, `X-Original-Method`), never on this request's own `Host`; then the
/// session from the `Cookie` header, with the same hourly purge of expired sessions as every other request but **never a
/// refresh** (a refreshed cookie could not reach the browser through this path); then the pending password change. A
/// signed-in user gets a fresh identity statement for the AI service (see `api::internal`). Stricter than the other
/// routes about the cookie: a request with more than one `xexam_session` cookie (in any letter case, an empty one
/// included) or a padded or quoted value has no session here ([`auth::single_session_token`]).
pub async fn introspect(st: &AppState, headers: &HeaderMap) -> IntrospectOutcome {
    let Some(link_token) = st.0.ai_link.token().cloned() else {
        return IntrospectOutcome::LinkOff;
    };
    let (host, origin, method) = super::internal::original_request(headers);
    let checked_origin = if matches!(method.as_str(), "GET" | "HEAD") {
        None
    } else {
        origin.as_deref()
    };
    if let Some(denial) = super::guard::deny(&st.0.cfg, host.as_deref(), checked_origin) {
        let shown = crate::security::short(&method);
        super::note_rejection(
            st,
            format!(
                "{} AI request: {}",
                if shown.is_empty() { "an" } else { &shown },
                denial.log
            ),
        );
        return IntrospectOutcome::Guard(denial);
    }
    // Exactly one `xexam_session` cookie, written plainly: Nginx keys the per-session limit on starting answers by that
    // cookie, so its key is then always the session signed in here (see `auth::single_session_token`)
    let Some(token) = auth::single_session_token(headers) else {
        return IntrospectOutcome::SignIn;
    };
    let now = auth::unix_now();
    let sid = auth::token_hash(&token);
    // The same lookup as every other request: the same deadline rule, and the same touch (somebody who only talks to the AI
    // is active), but never a cookie (it could not reach the browser through this path)
    match session_lookup(st, sid.clone(), now, true, false).await {
        Err(_) => IntrospectOutcome::Failed,
        Ok(Looked::None | Looked::Ended) => IntrospectOutcome::SignIn,
        Ok(Looked::Found(f)) if f.user.must_change_password => IntrospectOutcome::Password,
        Ok(Looked::Found(f)) => IntrospectOutcome::Allowed(super::internal::identity_statement(
            &link_token,
            &f.user,
            &sid,
            now,
        )),
    }
}

/// The [`Viewer`] of a signed-in request: an administrator directly (no database read), anyone else as the database says now.
pub async fn viewer(st: &AppState, a: &Authed) -> Result<Viewer, AppError> {
    if a.user.is_admin() {
        return Ok(Viewer::admin(a.user.id));
    }
    let store = st.0.store.clone();
    let id = a.user.id;
    blocking(move || store.viewer(id)).await
}

fn set_cookie(resp: &mut Response, value: &str) {
    if let Ok(v) = HeaderValue::from_str(value) {
        resp.headers_mut().append(header::SET_COOKIE, v);
    }
}

/// Adds the refreshed session cookie to the response, if the session was refreshed by this request.
pub fn finish(resp: &mut Response, authed: Option<&Authed>) {
    if let Some(a) = authed.filter(|a| a.refreshed) {
        // A response that already sets the cookie (logout) keeps its own
        if !resp.headers().contains_key(header::SET_COOKIE) {
            set_cookie(
                resp,
                &auth::session_cookie_for(&a.token, a.secure, Rule::Remember),
            );
        }
    }
}

/// Runs a password hash / verify on a blocking thread, at most `HASH_CONCURRENCY` at a time. The permit is held by the blocking
/// task itself, so a client that disconnects cannot free it early.
async fn argon<T, F>(st: &AppState, f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let permit =
        st.0.hash_gate
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AppError::Internal)?;
    tokio::task::spawn_blocking(move || {
        let out = f();
        drop(permit);
        out
    })
    .await
    .map_err(|e| {
        tracing::error!("password hashing task failed: {e}");
        AppError::Internal
    })
}

async fn hash(st: &AppState, password: String) -> Result<String, AppError> {
    argon(st, move || auth::hash_password(&password))
        .await?
        .map_err(|e| {
            tracing::error!("{e}");
            AppError::Internal
        })
}

async fn verify(st: &AppState, password: String, phc: String) -> Result<bool, AppError> {
    argon(st, move || auth::verify_password(&password, &phc)).await
}

fn str_field<'a>(data: &'a PyVal, key: &str) -> Option<&'a str> {
    match data.get(key) {
        Some(PyVal::Str(s)) => Some(s.as_str()),
        _ => None,
    }
}

fn ok_json(v: &Value) -> Response {
    json_resp(StatusCode::OK, v, false)
}

// ---------------------------------------------------------------- /api/auth/*

/// `GET /api/auth/state` → `{"user": {...} | null, ...}`. Signed in: also `"library"`, an opaque stamp of the question banks
/// the user may see that changes whenever a bank is imported, replaced or deleted, and (v0.6.5) whenever the banks they may see
/// or manage change (an open page re-reads the banks when it changes).
/// v0.8, signed in: also `"aiAvailable"` (ai-service connected and its AI+ service on, from its last heartbeat), and for
/// administrators `"aiService"` (`{"connected", "stopped", "lastSeen", "version", "enabled", "link"}`, see
/// [`super::internal::AiLink::service_json`]), which the AI settings page shows when ai-service is not connected.
/// Not signed in: also `"defaultAdmin"`, the username of an enabled administrator that still has the default password `admin`,
/// or `null` (the sign-in page then tells the owner how to sign in for the first time, with that account's real name: it may
/// have been renamed; the default credentials are documented anyway).
pub async fn state(st: &AppState, authed: Option<&Authed>) -> Result<Value, AppError> {
    let store = st.0.store.clone();
    match authed {
        Some(a) => {
            let v = viewer(st, a).await?;
            let library = blocking(move || store.library_stamp_as(&v)).await?;
            let link = &st.0.ai_link;
            let service = link.state();
            let mut out = json!({
                "user": a.user.to_json(),
                "library": library,
                "aiAvailable": service.ai_available(),
                "session": session_json(a.rule, a.hard_ends_at),
                // Whether the page reports the pages a user opens: always for administrators, else the switch in the log settings
                "logVisits": a.user.is_admin() || st.0.activity.user_visits(),
            });
            if a.user.is_admin() {
                if let Some(o) = out.as_object_mut() {
                    o.insert("aiService".to_string(), link.service_json(&service));
                }
            }
            Ok(out)
        }
        None => {
            let hashes = st.0.default_admin_hashes.get().cloned().unwrap_or_default();
            let pending = if hashes.is_empty() {
                None
            } else {
                blocking(move || store.default_admin_pending(&hashes)).await?
            };
            Ok(json!({ "user": null, "defaultAdmin": pending }))
        }
    }
}

/// The password hashes in the database that verify against the default password `admin` (checked once, at startup; no API can
/// ever set a password that short, so the list cannot grow while the server runs). Feeds `"defaultAdmin"` of
/// `GET /api/auth/state` and the startup reminder.
pub fn find_default_admin_hashes(store: &crate::database::Store) -> Result<Vec<String>, DbError> {
    Ok(store
        .password_hashes_to_change()?
        .into_iter()
        .filter(|phc| auth::verify_password(auth::DEFAULT_ADMIN_PASSWORD, phc))
        .collect())
}

/// Starts a password check for `target` in the login throttle: a lock (or the pressure rule) answers 429 with the wait.
/// Refusals cost almost nothing, so they are not logged one by one: at most one summary line per minute.
fn throttle_begin(st: &AppState, target: Target<'_>, known: bool) -> Result<Begun, AppError> {
    let t = &st.0.throttle;
    t.begin(target, known, t.now()).map_err(|refused| {
        if let Some(n) = refused.report {
            log_refusals(n);
        }
        AppError::TooManyLogins {
            retry_after: retry_after_secs(refused.wait),
        }
    })
}

fn log_refusals(n: u64) {
    tracing::warn!(
        "refused {n} password check(s) of locked or throttled sign-ins since the last report"
    );
}

/// Reports, once a minute, the refusals that came after the last report line (the tail of a burst that no later attempt would
/// report), and in the same way the requests rejected by the Host / Origin guard, the failed sign-ins, the locks of usernames
/// without an account and (v0.8) the internal requests refused for a missing or wrong token. Runs until the server stops (the
/// caller aborts it).
pub fn spawn_refusal_reports(st: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut every = tokio::time::interval(crate::auth::REFUSAL_REPORT_EVERY);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            every.tick().await;
            let t = &st.0.throttle;
            if let Some(n) = t.flush_refusals(t.now()) {
                log_refusals(n);
            }
            let now = std::time::Instant::now();
            if let Some(report) = st.0.rejections.flush(now) {
                super::guard::log_rejections(&report);
                st.0.activity.record(crate::activity::guard_entry(&report));
            }
            if let Some(report) = st.0.failed_sign_ins.flush(now) {
                log_failed_sign_ins(&report);
            }
            if let Some(report) = st.0.name_locks.flush(now) {
                log_name_locks(&report);
            }
            if let Some(report) = st.0.ai_link.flush_refusals(now) {
                super::internal::log_refused(&report);
            }
        }
    })
}

/// Counts a failed sign-in. Every attempt with an acceptable Host and Origin reaches the password check unless the throttle
/// refuses it, and a flood of made-up names is never refused (each name has its own bucket), so one line each would let anyone
/// push the other lines out of the System log: the first one after a quiet minute is logged at once, the rest summed up.
fn note_failed_sign_in(st: &AppState, what: &str) {
    let now = std::time::Instant::now();
    if let Some(report) = st.0.failed_sign_ins.note(now, what.to_string()) {
        log_failed_sign_ins(&report);
    }
}

fn log_failed_sign_ins(report: &BurstReport) {
    match report {
        BurstReport::One(what) => tracing::info!("{what}"),
        BurstReport::Many { count, .. } => {
            tracing::info!("{count} failed sign-in(s) since the last report")
        }
    }
}

fn log_name_locks(report: &BurstReport) {
    match report {
        BurstReport::One(what) => tracing::warn!("{what}"),
        BurstReport::Many { count, last } => tracing::warn!(
            "{count} sign-in lock(s) of usernames without an account started since the last report; the last one: {last}"
        ),
    }
}

/// Logs a lock once, when it starts: the attempt that used up the round's last chance turned out wrong. Locks of accounts are
/// logged one by one (one account locks at most once per lock length), with the account's username (`username`, the name
/// of the account the target belongs to); locks of usernames without an account go through the state's `name_locks`,
/// because anyone can start one per made-up name (see [`note_failed_sign_in`]), and never repeat the typed name.
fn log_lock(
    st: &AppState,
    lock: Option<StartedLock>,
    target: Target<'_>,
    known: bool,
    username: &str,
) {
    let Some(l) = lock else { return };
    let from = if known {
        "the browsers where it signed in before"
    } else {
        "other browsers"
    };
    let line = |whom: &str| {
        format!(
            "sign-in of {whom} from {from} locked for {} s (lock {} in a row) after repeated failed attempts",
            l.length.as_secs(),
            l.round
        )
    };
    match target {
        Target::Account(_) => {
            tracing::warn!("{}", line(&format!("user {username:?}")));
            // Locks of accounts are bounded by the accounts; locks of made-up names are not recorded one by one
            st.0.activity.record(
                Entry::system(
                    "auth.lock",
                    Some(json!({ "round": l.round, "minutes": l.length.as_secs().div_ceil(60) })),
                )
                .object(username),
            );
        }
        Target::Name(_) => {
            let what = line("a username without an account");
            if let Some(report) = st.0.name_locks.note(std::time::Instant::now(), what) {
                log_name_locks(&report);
            }
        }
    }
}

fn set_device_cookie(resp: &mut Response, user_id: i64, password_hash: &str, secure: bool) {
    set_cookie(
        resp,
        &auth::device_cookie(&auth::device_tag(user_id, password_hash), secure),
    );
}

/// `POST /api/auth/login` `{"username", "password", "remember"?}` → `{"user": {...}, "session": {...}}`, a new session cookie
/// (administrator: 7 days once; user with `remember` (default): 30 days; else a browser-session cookie) and the known-browser
/// cookie.
/// A wrong password, an unknown user and a disabled user all get the same 401 after the same amount of work.
pub async fn login(
    st: &AppState,
    data: &PyVal,
    headers: &HeaderMap,
    origin: Option<&str>,
) -> Result<Response, AppError> {
    let (Some(raw_name), Some(password)) =
        (str_field(data, "username"), str_field(data, "password"))
    else {
        return Err(bad_request());
    };
    // "Keep me signed in": missing = true (an older client keeps the 30 days), anything but a boolean is a bad request
    let remember = match data.get("remember") {
        None => true,
        Some(PyVal::Bool(b)) => *b,
        Some(_) => return Err(bad_request()),
    };
    let client = ClientInfo::from_headers(headers);
    let name = raw_name.trim().to_string();
    // A plain lookup (no hashing) comes first: a browser where this user signed in before is throttled in its own bucket
    let store = st.0.store.clone();
    let lookup = name.clone();
    let user = if auth::clean_username(&name).is_some() {
        blocking(move || store.user_by_name(&lookup)).await?
    } else {
        None
    };
    let known = user
        .as_ref()
        .is_some_and(|u| auth::device_known(headers, u.id, &u.password_hash));
    // An existing account is throttled by its id (its state follows the account, and a flood of made-up names can never evict
    // it); any other name by the name itself, with the same rounds, so the answers do not tell whether the account exists
    let target = user
        .as_ref()
        .map_or(Target::Name(&name), |u| Target::Account(u.id));
    let begun = match throttle_begin(st, target, known) {
        Ok(b) => b,
        Err(e) => {
            // Refused while locked: the account when it exists (never the typed name of an unknown one)
            let actor = user.as_ref().map_or(Actor::Anon, |u| Actor::User {
                id: u.id,
                username: u.username.clone(),
                role: u.role,
            });
            st.0.activity.record(
                Entry::new(actor, "auth.signin")
                    .fail("locked")
                    .client(&client),
            );
            return Err(e);
        }
    };
    let phc = user
        .as_ref()
        .map(|u| u.password_hash.clone())
        .unwrap_or_else(|| auth::dummy_hash().to_string());
    let matches = verify(st, password.to_string(), phc.clone()).await?;
    let user = match user {
        Some(u) if matches && !u.disabled => u,
        Some(u) => {
            // The account exists: its name is safe to log. A disabled account is named as the reason only when the
            // password was right (otherwise the wrong password is why)
            let what = if matches {
                format!(
                    "failed sign-in of user {:?} (the account is disabled)",
                    u.username
                )
            } else {
                format!("failed sign-in of user {:?}", u.username)
            };
            note_failed_sign_in(st, &what);
            st.0.activity.record(
                Entry::new(Actor::of(&SessionUser::from(&u)), "auth.signin")
                    .fail(if matches { "disabled" } else { "badPassword" })
                    .client(&client),
            );
            log_lock(st, begun.lock, target, known, &u.username);
            return Err(status(StatusCode::UNAUTHORIZED, MSG_BAD_LOGIN));
        }
        None => {
            // Never the typed name: it may be a password typed into the wrong field
            note_failed_sign_in(st, "failed sign-in with a username that has no account");
            st.0.activity.record(
                Entry::new(Actor::Anon, "auth.signin")
                    .fail("unknownUser")
                    .client(&client),
            );
            log_lock(st, begun.lock, target, known, "");
            return Err(status(StatusCode::UNAUTHORIZED, MSG_BAD_LOGIN));
        }
    };
    let token = auth::new_session_token();
    let token_hash = auth::token_hash(&token);
    let store = st.0.store.clone();
    let id = user.id;
    let now = auth::unix_now();
    let times = SessionTimes {
        started_at: now,
        last_seen_at: now,
        remember,
    };
    let (deadline, _) = auth::session_deadline(user.role, &times);
    let rule = auth::session_rule(user.role, remember);
    // The address and the visitor location are stored only while the eye in User management is open; while it is closed
    // nothing new is recorded (the stored values stay, hidden)
    let keep = st.0.activity.signin_info();
    let ip = client.ip.filter(|_| keep).map(|i| i.to_string());
    let (ua, country, region, city) = (
        client.ua.clone(),
        client.country.clone().filter(|_| keep),
        client.region.clone().filter(|_| keep),
        client.city.clone().filter(|_| keep),
    );
    let verified = phc.clone();
    let hash_for_row = token_hash.clone();
    let stamp = local_second(now);
    let signed_in = blocking(move || {
        let created = store.create_session(&NewSession {
            token_hash: &hash_for_row,
            user_id: id,
            now,
            expires_at: deadline,
            remember,
            ip: ip.as_deref(),
            ua: ua.as_deref(),
        });
        match created {
            Ok(()) => {}
            // Deleted or disabled while the password was being verified
            Err(DbError::NoUser) => return Ok(None),
            Err(e) => return Err(e),
        }
        // The password verified above must still be the user's: a password changed or reset while the (slow) verify ran
        // must not leave a session signed in with the old password. (A change after this check ends every session.)
        let Some(u) = store
            .user_by_id(id)?
            .filter(|u| !u.disabled && u.password_hash == verified)
        else {
            store.delete_session(&hash_for_row)?;
            return Ok(None);
        };
        // Closed eye: `None` leaves the stored sign-in info untouched (only the times move)
        let info = keep.then_some(SigninInfo {
            ip: ip.as_deref(),
            country: country.as_deref(),
            region: region.as_deref(),
            city: city.as_deref(),
        });
        store.note_signin(id, &stamp, info.as_ref())?;
        Ok(Some(SessionUser::from(&u)))
    })
    .await?;
    let Some(signed_in) = signed_in else {
        note_failed_sign_in(
            st,
            &format!(
                "failed sign-in of user {:?}: the account changed during the sign-in",
                user.username
            ),
        );
        st.0.activity.record(
            Entry::new(Actor::of(&SessionUser::from(&user)), "auth.signin")
                .fail("changed")
                .client(&client),
        );
        return Err(status(StatusCode::UNAUTHORIZED, MSG_BAD_LOGIN));
    };
    st.0.throttle.succeeded(id, known);
    st.0.gates.started(&token_hash, id, now);
    tracing::info!("user {:?} signed in", signed_in.username);
    st.0.activity.record(
        Entry::new(Actor::of(&signed_in), "auth.signin")
            .detail(json!({ "rule": rule.as_str() }))
            .client(&client),
    );
    let hard = (rule == Rule::Admin).then(|| now.saturating_add(auth::ADMIN_HARD_SECS));
    let secure = auth::cookie_secure(st.0.cfg.cookie_secure, origin);
    let mut resp = ok_json(&json!({
        "user": signed_in.to_json(),
        "session": session_json(rule, hard),
    }));
    set_cookie(&mut resp, &auth::session_cookie_for(&token, secure, rule));
    set_device_cookie(&mut resp, signed_in.id, &phc, secure);
    Ok(resp)
}

/// `POST /api/auth/logout` → `{"ok": true}` and an expired cookie; the session is deleted.
pub async fn logout(st: &AppState, a: &Authed, origin: Option<&str>) -> Result<Response, AppError> {
    let store = st.0.store.clone();
    let hash = a.token_hash.clone();
    blocking(move || store.delete_session(&hash)).await?;
    st.0.gates.forget(&a.token_hash);
    st.0.activity
        .record(Entry::new(Actor::of(&a.user), "auth.signout").client(&a.client));
    let mut resp = ok_json(&json!({ "ok": true }));
    set_cookie(
        &mut resp,
        &auth::expired_cookie(auth::cookie_secure(st.0.cfg.cookie_secure, origin)),
    );
    Ok(resp)
}

/// `POST /api/auth/password` `{"current"?, "password"}` → `{"user": {...}}`. `current` is required unless the user must change
/// the password (then the session proves who they are). The new password must differ from the old one. Clears the flag and
/// ends every session of the user, this one included: this browser gets a **new** session cookie (and the known-browser cookie
/// of the new password), so a copy of the old cookie (read off a plain-HTTP network, say) stops working too.
pub async fn change_password(
    st: &AppState,
    a: &Authed,
    data: &PyVal,
    headers: &HeaderMap,
    origin: Option<&str>,
) -> Result<Response, AppError> {
    let Some(password) = str_field(data, "password") else {
        return Err(bad_request());
    };
    if !auth::password_ok(password) {
        return Err(AppError::BadRequest(MSG_BAD_PASSWORD.to_string()));
    }
    let store = st.0.store.clone();
    let id = a.user.id;
    let user = blocking(move || store.user_by_id(id))
        .await?
        .ok_or(status(StatusCode::UNAUTHORIZED, MSG_SIGN_IN_REQUIRED))?;
    if !a.user.must_change_password {
        let Some(current) = str_field(data, "current") else {
            return Err(bad_request());
        };
        // Guessing the current password through a stolen session is throttled like a sign-in (same buckets)
        let known = auth::device_known(headers, user.id, &user.password_hash);
        let target = Target::Account(user.id);
        let begun = throttle_begin(st, target, known)?;
        if !verify(st, current.to_string(), user.password_hash.clone()).await? {
            log_lock(st, begun.lock, target, known, &user.username);
            return Err(status(StatusCode::FORBIDDEN, MSG_WRONG_PASSWORD));
        }
        st.0.throttle.succeeded(user.id, known);
    }
    if verify(st, password.to_string(), user.password_hash.clone()).await? {
        return Err(AppError::BadRequest(MSG_SAME_PASSWORD.to_string()));
    }
    let new_hash = hash(st, password.to_string()).await?;
    let store = st.0.store.clone();
    let expected = user.password_hash;
    let stored = new_hash.clone();
    // The session is rotated: one new token replaces every session of the user, the one of this request included
    let token = auth::new_session_token();
    let token_hash = auth::token_hash(&token);
    let now = auth::unix_now();
    // Refused (401) when the password was reset or changed elsewhere while this request ran: never overwrite that silently
    let new_hash_of_token = token_hash.clone();
    // The new session keeps `remember`, gets the deadline of the user's role and the address / device of this request
    let remember = a.remember;
    let times = SessionTimes {
        started_at: now,
        last_seen_at: now,
        remember,
    };
    let (deadline, _) = auth::session_deadline(a.user.role, &times);
    let ip = a
        .client
        .ip
        .filter(|_| st.0.activity.signin_info())
        .map(|i| i.to_string());
    let ua = a.client.ua.clone();
    let updated = blocking(move || {
        store.change_own_password_as(
            id,
            &expected,
            &stored,
            &NewSession {
                token_hash: &token_hash,
                user_id: id,
                now,
                expires_at: deadline,
                remember,
                ip: ip.as_deref(),
                ua: ua.as_deref(),
            },
        )
    })
    .await?
    .ok_or(status(StatusCode::UNAUTHORIZED, MSG_SIGN_IN_REQUIRED))?;
    st.0.gates.forget(&a.token_hash);
    st.0.gates.started(&new_hash_of_token, id, now);
    tracing::info!("user {:?} changed their password", updated.username);
    let secure = auth::cookie_secure(st.0.cfg.cookie_secure, origin);
    let mut resp = ok_json(&json!({ "user": updated.to_json() }));
    // The new session keeps the rule of the old one (administrator / remember / browser)
    set_cookie(&mut resp, &auth::session_cookie_for(&token, secure, a.rule));
    set_device_cookie(&mut resp, id, &new_hash, secure);
    Ok(resp)
}

// ---------------------------------------------------------------- /api/users*

/// `"lock"` of a user in `GET /api/users`: `null` at round 1 and not locked; else `{"until": "<local time>" | null,
/// "retryAfter": <seconds, 0 when not locked>, "round": <n>, "triesLeft": <n>, "knownBrowser": <bool>,
/// "otherUntil": "<local time>" | null, "otherRetryAfter": <seconds, 0 when null>}` (see [`LockStatus`]; `knownBrowser`: the
/// state belongs to the browsers where the user signed in before, else to every other browser; `otherUntil` /
/// `otherRetryAfter`: a lock that runs at the same time for the other browsers, so the account is locked in every browser).
fn lock_json(s: Option<LockStatus>) -> Value {
    use crate::database::timefmt::local_in_secs;
    match s {
        None => Value::Null,
        Some(s) => {
            let wait = s.wait.map(retry_after_secs);
            let other = s.other_wait.map(retry_after_secs);
            json!({
                "until": wait.map(local_in_secs),
                "retryAfter": wait.unwrap_or(0),
                "round": s.round,
                "triesLeft": s.tries_left,
                "knownBrowser": s.known_browser,
                "otherUntil": other.map(local_in_secs),
                "otherRetryAfter": other.unwrap_or(0),
            })
        }
    }
}

/// `{"users": [...], "aiAvailable": bool}` from the database, each user with its current login-throttle state (`"lock"`, in
/// memory only) and (v0.8) `"aiSuspended"`: whether ai-service's last heartbeat names the user's AI+ function as suspended
/// (always false for administrators, who cannot be suspended). `"aiAvailable"`: see [`state`].
async fn users_json(st: &AppState) -> Result<Value, AppError> {
    let store = st.0.store.clone();
    let mut users = blocking(move || store.users()).await?;
    let t = &st.0.throttle;
    let now = t.now();
    let service = st.0.ai_link.state();
    if let Some(list) = users.get_mut("users").and_then(Value::as_array_mut) {
        for u in list.iter_mut() {
            let id = u.get("id").and_then(Value::as_i64);
            let lock = lock_json(id.and_then(|id| t.status(id, now)));
            let admin = u.get("role").and_then(Value::as_str) == Some(Role::Admin.as_str());
            let suspended = !admin && id.is_some_and(|id| service.suspended.contains(&id));
            if let Some(obj) = u.as_object_mut() {
                obj.insert("lock".to_string(), lock);
                obj.insert("aiSuspended".to_string(), Value::Bool(suspended));
            }
        }
    }
    if let Some(obj) = users.as_object_mut() {
        obj.insert(
            "aiAvailable".to_string(),
            Value::Bool(service.ai_available()),
        );
    }
    Ok(users)
}

/// v0.8: a user's AI+ service may be switched (either way) only while AI+ is available; else 409 `MSG_AI_SERVICE_OFF`.
fn require_ai_available(st: &AppState) -> Result<(), AppError> {
    if st.0.ai_link.ai_available() {
        Ok(())
    } else {
        Err(AppError::AiServiceOff)
    }
}

async fn users_body(st: &AppState) -> Result<Response, AppError> {
    Ok(ok_json(&users_json(st).await?))
}

/// [`users_body`] with the activity note of the route: `object` = the target username, `detail` as the contract says.
async fn users_body_noted(
    st: &AppState,
    object: &str,
    detail: Value,
) -> Result<Response, AppError> {
    let resp = users_body(st).await?;
    Ok(crate::activity::with_note(
        resp,
        Some(object.to_string()),
        Some(detail),
    ))
}

/// `GET /api/users`
pub async fn list(st: &AppState) -> Result<Value, AppError> {
    users_json(st).await
}

fn refused<T>(change: UserChange<T>) -> Result<T, AppError> {
    match change {
        UserChange::Done(v) => Ok(v),
        UserChange::NotFound => Err(status(StatusCode::NOT_FOUND, MSG_USER_NOT_FOUND)),
        UserChange::Taken => Err(status(StatusCode::CONFLICT, MSG_USERNAME_TAKEN)),
        UserChange::NotYourself => Err(status(StatusCode::CONFLICT, MSG_NOT_YOURSELF)),
        UserChange::LastAdmin => Err(status(StatusCode::CONFLICT, MSG_LAST_ADMIN)),
        UserChange::UnknownBanks => Err(AppError::BadRequest(MSG_UNKNOWN_BANKS.to_string())),
    }
}

/// `"banks"` of `/api/users/create` and `/api/users/update`: the existing banks the user sees, as a list of bank ids
/// (integers; duplicates are ignored), or `"all"` (every bank that exists now; a shorthand that also turns "include banks
/// imported later" on, see [`new_banks_of`]). Anything else is 400 `MSG_BAD_REQUEST`; an integer that cannot be a bank id
/// is an unknown bank (400 `MSG_UNKNOWN_BANKS`); whether the ids exist is checked by the store, inside the write.
fn banks_field(data: &PyVal) -> Result<Option<BankAccess>, AppError> {
    match data.get("banks") {
        None => Ok(None),
        Some(PyVal::Str(s)) if s == "all" => Ok(Some(BankAccess::All)),
        Some(PyVal::List(items)) => {
            let mut ids = Vec::with_capacity(items.len());
            for item in items {
                let PyVal::Int(i) = item else {
                    return Err(bad_request());
                };
                let id = i64::try_from(*i)
                    .ok()
                    .filter(|id| *id > 0)
                    .ok_or_else(|| AppError::BadRequest(MSG_UNKNOWN_BANKS.to_string()))?;
                ids.push(id);
            }
            ids.sort_unstable();
            ids.dedup();
            Ok(Some(BankAccess::Only(ids)))
        }
        Some(_) => Err(bad_request()),
    }
}

/// `"newBanks"` ("include banks imported later"): absent → `None`, a bool → that, anything else → 400. When it is absent
/// and `"banks"` is the shorthand `"all"`, it is `Some(true)` (every bank now and every bank later).
fn new_banks_of(data: &PyVal, banks: Option<&BankAccess>) -> Result<Option<bool>, AppError> {
    match data.get("newBanks") {
        None => Ok(matches!(banks, Some(BankAccess::All)).then_some(true)),
        some => body_flag(some).map(Some),
    }
}

/// `"canManageBanks"`: absent → `None`; a bool → that; anything else → 400.
fn can_manage_field(data: &PyVal) -> Result<Option<bool>, AppError> {
    match data.get("canManageBanks") {
        None => Ok(None),
        some => body_flag(some).map(Some),
    }
}

/// `"ai"` (v0.8, the AI+ switch): absent → `None`; a bool → that; anything else → 400.
fn ai_field(data: &PyVal) -> Result<Option<bool>, AppError> {
    match data.get("ai") {
        None => Ok(None),
        some => body_flag(some).map(Some),
    }
}

/// The System log line of an AI+ switch (v0.8): `user "admin" turned AI+ on for user "sam"`.
fn log_ai_switch(admin: &str, on: bool, user: &str) {
    tracing::info!(
        "user {admin:?} turned AI+ {} for user {user:?}",
        if on { "on" } else { "off" }
    );
}

fn username_field(data: &PyVal) -> Result<Option<String>, AppError> {
    match data.get("username") {
        None => Ok(None),
        Some(PyVal::Str(s)) => auth::clean_username(s)
            .map(Some)
            .ok_or_else(|| AppError::BadRequest(MSG_BAD_USERNAME.to_string())),
        Some(_) => Err(AppError::BadRequest(MSG_BAD_USERNAME.to_string())),
    }
}

fn role_field(data: &PyVal) -> Result<Option<Role>, AppError> {
    match data.get("role") {
        None => Ok(None),
        Some(PyVal::Str(s)) => Role::parse(s).map(Some).ok_or_else(bad_request),
        Some(_) => Err(bad_request()),
    }
}

fn new_password_field(data: &PyVal) -> Result<String, AppError> {
    match str_field(data, "password") {
        Some(p) if auth::password_ok(p) => Ok(p.to_string()),
        Some(_) => Err(AppError::BadRequest(MSG_BAD_PASSWORD.to_string())),
        None => Err(bad_request()),
    }
}

/// The `id` of a `/api/users*` body: an integer (else 400); an integer that cannot be an id is an unknown user (404).
fn user_id(data: &PyVal) -> Result<i64, AppError> {
    body_id(data.get("id"))?.ok_or(status(StatusCode::NOT_FOUND, MSG_USER_NOT_FOUND))
}

/// `POST /api/users/create` `{"username", "password", "role", "banks"?, "newBanks"?, "canManageBanks"?, "ai"?}` →
/// `{"users": [...]}`. The new user must change the password at the first sign-in. `banks` (the existing banks they see)
/// defaults to `[]` (no bank until an administrator chooses some), `newBanks` ("include banks imported later") to false
/// (true with the shorthand `"banks": "all"`), `canManageBanks` to false, `ai` (v0.8, AI+; stored for administrators too,
/// who always have it) to false. `"ai": true` while AI+ is not available is refused (409 `MSG_AI_SERVICE_OFF`) after the
/// body is checked and before anything else (the password is not hashed, the user not created, nothing logged).
pub async fn create(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    let username =
        username_field(data)?.ok_or_else(|| AppError::BadRequest(MSG_BAD_USERNAME.to_string()))?;
    let password = new_password_field(data)?;
    let role = role_field(data)?.ok_or_else(bad_request)?;
    let banks = banks_field(data)?;
    let new_banks = new_banks_of(data, banks.as_ref())?.unwrap_or(false);
    let banks = banks.unwrap_or(BankAccess::Only(Vec::new()));
    let can_manage = can_manage_field(data)?.unwrap_or(false);
    let ai = ai_field(data)?.unwrap_or(false);
    if ai {
        require_ai_available(st)?;
    }
    let phc = hash(st, password).await?;
    let store = st.0.store.clone();
    let name = username.clone();
    refused(
        blocking(move || {
            store.create_user_opts(&name, &phc, role, &banks, new_banks, can_manage, ai)
        })
        .await?,
    )?;
    let note = json!({ "role": role.as_str() });
    tracing::info!("user {:?} created user {:?}", a.user.username, username);
    if ai {
        log_ai_switch(&a.user.username, true, &username);
    }
    users_body_noted(st, &username, note).await
}

/// `POST /api/users/update` `{"id", "username"?, "role"?, "disabled"?, "banks"?, "newBanks"?, "canManageBanks"?, "ai"?}`
/// → `{"users": [...]}`; only the fields that are given change (`"banks": "all"` also sets `newBanks` unless it is given).
/// A role change keeps `banks`, `newBanks`, `canManageBanks` and `ai` (they matter only while the user is a plain user).
/// Switching AI+ on or off is logged on its own line (`user "admin" turned AI+ off for user "sam"`); a request that only
/// switches AI+ logs just that line. A request with `"ai"` (true or false, changed or not, alone or with other fields)
/// while AI+ is not available is refused as a whole (409 `MSG_AI_SERVICE_OFF`, nothing written, nothing logged).
pub async fn update(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    let id = user_id(data)?;
    let banks = banks_field(data)?;
    let change = UserUpdate {
        username: username_field(data)?,
        role: role_field(data)?,
        disabled: match data.get("disabled") {
            None => None,
            Some(PyVal::Bool(b)) => Some(*b),
            Some(_) => return Err(bad_request()),
        },
        new_banks: new_banks_of(data, banks.as_ref())?,
        banks,
        can_manage_banks: can_manage_field(data)?,
        ai: ai_field(data)?,
    };
    if change.ai.is_some() {
        require_ai_available(st)?;
    }
    let only_ai = change.ai.is_some()
        && UserUpdate {
            ai: None,
            ..change.clone()
        } == UserUpdate::default();
    let mut changed: Vec<&str> = Vec::new();
    if change.username.is_some() {
        changed.push("username");
    }
    if change.role.is_some() {
        changed.push("role");
    }
    if change.disabled.is_some() {
        changed.push("disabled");
    }
    if change.banks.is_some() {
        changed.push("banks");
    }
    if change.new_banks.is_some() {
        changed.push("newBanks");
    }
    if change.can_manage_banks.is_some() {
        changed.push("canManageBanks");
    }
    if change.ai.is_some() {
        changed.push("ai");
    }
    let role_after = change.role;
    let store = st.0.store.clone();
    let actor = a.user.id;
    let done = refused(blocking(move || store.update_user(actor, id, &change)).await?)?;
    if !(only_ai && done.ai_changed.is_some()) {
        match &done.renamed_from {
            Some(old) => tracing::info!(
                "user {:?} updated user {:?} (renamed from {:?})",
                a.user.username,
                done.username,
                old
            ),
            None => tracing::info!(
                "user {:?} updated user {:?}",
                a.user.username,
                done.username
            ),
        }
    }
    if let Some(on) = done.ai_changed {
        log_ai_switch(&a.user.username, on, &done.username);
    }
    let mut note = json!({ "changed": changed });
    if let Some(r) = role_after {
        note["role"] = json!(r.as_str());
    }
    if let Some(old) = &done.renamed_from {
        note["oldName"] = json!(old);
    }
    users_body_noted(st, &done.username, note).await
}

/// `POST /api/users/password` `{"id", "password"}` → `{"users": [...]}`: sets a new initial password for another user.
pub async fn reset_password(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    let id = user_id(data)?;
    let password = new_password_field(data)?;
    if id == a.user.id {
        // Your own password goes through POST /api/auth/password (it asks for the current one)
        return Err(status(StatusCode::CONFLICT, MSG_NOT_YOURSELF));
    }
    let phc = hash(st, password).await?;
    let store = st.0.store.clone();
    let actor = a.user.id;
    let name = refused(blocking(move || store.reset_user_password(actor, id, &phc)).await?)?;
    // The new password lifts the login lock of that account (the reset-password command, another process, cannot do this)
    st.0.throttle.forget(id);
    tracing::info!(
        "user {:?} reset the password of user {:?}",
        a.user.username,
        name
    );
    users_body_noted(st, &name, json!({})).await
}

/// `POST /api/users/unlock` `{"id"}` → `{"users": [...]}`: clears the login throttle of that account completely (every bucket:
/// back to round 1, five chances), whether a lock runs or the account is only past round 1. Allowed for the administrator's own
/// account too.
pub async fn unlock(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    let id = user_id(data)?;
    let store = st.0.store.clone();
    let user = blocking(move || store.user_by_id(id))
        .await?
        .ok_or(status(StatusCode::NOT_FOUND, MSG_USER_NOT_FOUND))?;
    st.0.throttle.forget(id);
    tracing::info!(
        "user {:?} unlocked the sign-in of user {:?}",
        a.user.username,
        user.username
    );
    users_body_noted(st, &user.username, json!({})).await
}

/// `POST /api/users/delete` `{"id"}` → `{"users": [...]}`. The banks the deleted user owned now belong to the administrator
/// who deleted them.
pub async fn delete(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    let id = user_id(data)?;
    let store = st.0.store.clone();
    let actor = a.user.id;
    let name = refused(blocking(move || store.delete_user(actor, id)).await?)?;
    // Ids are never reused (AUTOINCREMENT); this frees the memory of the deleted account's buckets
    st.0.throttle.forget(id);
    tracing::info!("user {:?} deleted user {:?}", a.user.username, name);
    users_body_noted(st, &name, json!({})).await
}
