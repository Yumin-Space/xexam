//! Sign-in (v0.6): the route table with each route's access rule, the one place that enforces it ([`authorize`], called by the
//! dispatch function for every GET / HEAD / POST), and the handlers of `/api/auth/*` and `/api/users*`.
//!
//! Rules (contract: `docs/en/api.md`):
//! - Public: `GET /api/info`, `GET /api/auth/state`, `POST /api/auth/login`. Every other path, known or not, needs a valid
//!   session (401 `MSG_SIGN_IN_REQUIRED`); unknown paths then answer 404 as before.
//! - A user who must change their password reaches only `GET /api/auth/state`, `POST /api/auth/password`,
//!   `POST /api/auth/logout` and `GET /api/info`; everything else is 403 `MSG_PASSWORD_CHANGE_REQUIRED`.
//! - Admin routes answer 403 `MSG_ADMIN_ONLY` to other users.
//! - The session is read from the `xexam_session` cookie only. Its sliding refresh (at most once per hour) re-sends the cookie,
//!   so it happens only on requests whose scheme is known: with `XEXAM_COOKIE_SECURE=1`, or when the request has an `Origin`
//!   (every browser POST does). That way a refresh never drops `Secure` from a cookie issued over HTTPS.
use super::response::{
    json_resp, MSG_ADMIN_ONLY, MSG_BAD_LOGIN, MSG_BAD_PASSWORD, MSG_BAD_REQUEST, MSG_BAD_USERNAME,
    MSG_LAST_ADMIN, MSG_NOT_YOURSELF, MSG_PASSWORD_CHANGE_REQUIRED, MSG_SAME_PASSWORD,
    MSG_SIGN_IN_REQUIRED, MSG_USERNAME_TAKEN, MSG_USER_NOT_FOUND, MSG_WRONG_PASSWORD,
};
use super::{blocking, body_id, AppState, PyVal};
use crate::auth::throttle::retry_after_secs;
use crate::auth::{self, LockStatus, Role, SESSION_PURGE_SECS};
use crate::database::{SessionUser, UserChange, UserUpdate};
use crate::error::{AppError, DbError};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;

/// Who may use a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// No session needed
    Public,
    /// Any signed-in user
    SignedIn,
    /// Signed-in administrators only
    Admin,
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
}

const fn route(method: &'static str, path: &'static str, access: Access) -> Route {
    Route {
        method,
        path,
        access,
        during_password_change: false,
    }
}

/// Every route the backend serves, with its access rule. The dispatch function answers 404 for anything else.
pub const ROUTES: &[Route] = &[
    route("GET", "/api/info", Access::Public),
    route("GET", "/api/auth/state", Access::Public),
    route("GET", "/api/bank", Access::SignedIn),
    route("GET", "/api/banks", Access::SignedIn),
    route("GET", "/api/progress", Access::SignedIn),
    route("GET", "/api/imports", Access::Admin),
    route("GET", "/api/records", Access::SignedIn),
    route("GET", "/api/record", Access::SignedIn),
    route("GET", "/api/settings", Access::SignedIn),
    route("GET", "/api/users", Access::Admin),
    route("POST", "/api/auth/login", Access::Public),
    Route {
        during_password_change: true,
        ..route("POST", "/api/auth/logout", Access::SignedIn)
    },
    Route {
        during_password_change: true,
        ..route("POST", "/api/auth/password", Access::SignedIn)
    },
    route("POST", "/api/upload", Access::Admin),
    route("POST", "/api/import/confirm", Access::Admin),
    route("POST", "/api/import/cancel", Access::Admin),
    route("POST", "/api/banks/delete", Access::Admin),
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
    /// The session expiry was just refreshed: the response re-sends the cookie
    refreshed: bool,
    /// Whether a re-sent cookie gets `Secure`
    secure: bool,
}

fn status(code: StatusCode, msg: &'static str) -> AppError {
    AppError::Status(code.as_u16(), msg)
}

fn bad_request() -> AppError {
    AppError::BadRequest(MSG_BAD_REQUEST.to_string())
}

/// Looks up the session of the request (if it carries a token-shaped cookie). Also purges expired sessions, at most once an hour.
async fn session(
    st: &AppState,
    headers: &HeaderMap,
    origin: Option<&str>,
) -> Result<Option<Authed>, AppError> {
    let Some(token) = auth::session_token(headers) else {
        return Ok(None);
    };
    let force_secure = st.0.cfg.cookie_secure;
    let may_refresh = force_secure || origin.is_some();
    let secure = auth::cookie_secure(force_secure, origin);
    let now = auth::unix_now();
    let last = st.0.last_session_purge.load(Ordering::Relaxed);
    let purge = now.saturating_sub(last) >= SESSION_PURGE_SECS
        && st
            .0
            .last_session_purge
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok();
    let store = st.0.store.clone();
    let token_hash = auth::token_hash(&token);
    let hash = token_hash.clone();
    let found = blocking(move || {
        if purge {
            let n = store.purge_expired_sessions(now)?;
            if n > 0 {
                tracing::info!("purged {n} expired sessions");
            }
        }
        store.session_user(&hash, now, may_refresh)
    })
    .await?;
    Ok(found.map(|(user, refreshed)| Authed {
        user,
        token_hash,
        token,
        refreshed,
        secure,
    }))
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
    let authed = session(st, headers, origin).await?;
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
    Ok(Some(a))
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
            set_cookie(resp, &auth::session_cookie(&a.token, a.secure));
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

/// `GET /api/auth/state` → `{"user": {...} | null, ...}`. Signed in: also `"library"`, an opaque stamp of the shared question
/// banks that changes whenever a bank is imported, replaced or deleted (an open page re-reads the banks when it changes).
/// Not signed in: also `"defaultAdmin"`, whether an enabled administrator still has the default password `admin` (the sign-in
/// page then tells the owner how to sign in for the first time; the default credentials are documented anyway).
pub async fn state(st: &AppState, authed: Option<&Authed>) -> Result<Value, AppError> {
    let store = st.0.store.clone();
    match authed {
        Some(a) => {
            let library = blocking(move || store.library_stamp()).await?;
            Ok(json!({ "user": a.user.to_json(), "library": library }))
        }
        None => {
            let hashes = st.0.default_admin_hashes.get().cloned().unwrap_or_default();
            let pending = if hashes.is_empty() {
                false
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

/// Starts a password check for `username` in the login throttle: a lock (or the pressure rule) answers 429 with the wait.
fn throttle_begin(st: &AppState, username: &str, known: bool) -> Result<(), AppError> {
    let t = &st.0.throttle;
    t.begin(username, known, t.now()).map_err(|wait| {
        tracing::warn!("password check throttled");
        AppError::TooManyLogins {
            retry_after: retry_after_secs(wait),
        }
    })
}

fn set_device_cookie(resp: &mut Response, user_id: i64, password_hash: &str, secure: bool) {
    set_cookie(
        resp,
        &auth::device_cookie(&auth::device_tag(user_id, password_hash), secure),
    );
}

/// `POST /api/auth/login` `{"username", "password"}` → `{"user": {...}}`, a new session cookie and the known-browser cookie.
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
    // The throttle keys only names that pass the username rule; every other name shares one key (bounded memory)
    throttle_begin(st, &name, known)?;
    let phc = user
        .as_ref()
        .map(|u| u.password_hash.clone())
        .unwrap_or_else(|| auth::dummy_hash().to_string());
    let matches = verify(st, password.to_string(), phc.clone()).await?;
    let user = match user {
        Some(u) if matches && !u.disabled => u,
        _ => {
            tracing::info!("failed sign-in");
            return Err(status(StatusCode::UNAUTHORIZED, MSG_BAD_LOGIN));
        }
    };
    let token = auth::new_session_token();
    let token_hash = auth::token_hash(&token);
    let store = st.0.store.clone();
    let id = user.id;
    let now = auth::unix_now();
    // start_session re-checks that the password verified above is still the user's password
    let verified = phc.clone();
    let Some(signed_in) =
        blocking(move || store.start_session(id, &token_hash, &verified, now)).await?
    else {
        // Disabled, deleted or given a new password while the password was being verified
        tracing::info!("failed sign-in: the account changed during the sign-in");
        return Err(status(StatusCode::UNAUTHORIZED, MSG_BAD_LOGIN));
    };
    st.0.throttle.succeeded(&name, known);
    tracing::info!("user {} signed in", signed_in.id);
    let secure = auth::cookie_secure(st.0.cfg.cookie_secure, origin);
    let mut resp = ok_json(&json!({ "user": signed_in.to_json() }));
    set_cookie(&mut resp, &auth::session_cookie(&token, secure));
    set_device_cookie(&mut resp, signed_in.id, &phc, secure);
    Ok(resp)
}

/// `POST /api/auth/logout` → `{"ok": true}` and an expired cookie; the session is deleted.
pub async fn logout(st: &AppState, a: &Authed, origin: Option<&str>) -> Result<Response, AppError> {
    let store = st.0.store.clone();
    let hash = a.token_hash.clone();
    blocking(move || store.end_session(&hash)).await?;
    let mut resp = ok_json(&json!({ "ok": true }));
    set_cookie(
        &mut resp,
        &auth::expired_cookie(auth::cookie_secure(st.0.cfg.cookie_secure, origin)),
    );
    Ok(resp)
}

/// `POST /api/auth/password` `{"current"?, "password"}` → `{"user": {...}}`. `current` is required unless the user must change
/// the password (then the session proves who they are). The new password must differ from the old one. Clears the flag and
/// deletes the user's other sessions; this browser gets the known-browser cookie of the new password.
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
        throttle_begin(st, &user.username, known)?;
        if !verify(st, current.to_string(), user.password_hash.clone()).await? {
            return Err(status(StatusCode::FORBIDDEN, MSG_WRONG_PASSWORD));
        }
        st.0.throttle.succeeded(&user.username, known);
    }
    if verify(st, password.to_string(), user.password_hash.clone()).await? {
        return Err(AppError::BadRequest(MSG_SAME_PASSWORD.to_string()));
    }
    let new_hash = hash(st, password.to_string()).await?;
    let store = st.0.store.clone();
    let keep = a.token_hash.clone();
    let expected = user.password_hash;
    let stored = new_hash.clone();
    // Refused (401) when the password was reset or changed elsewhere while this request ran: never overwrite that silently
    let updated = blocking(move || store.change_own_password(id, &expected, &stored, &keep))
        .await?
        .ok_or(status(StatusCode::UNAUTHORIZED, MSG_SIGN_IN_REQUIRED))?;
    tracing::info!("user {id} changed their password");
    let mut resp = ok_json(&json!({ "user": updated.to_json() }));
    set_device_cookie(
        &mut resp,
        id,
        &new_hash,
        auth::cookie_secure(st.0.cfg.cookie_secure, origin),
    );
    Ok(resp)
}

// ---------------------------------------------------------------- /api/users*

/// `"lock"` of a user in `GET /api/users`: `null` at round 1 and not locked; else `{"until": "<local time>" | null,
/// "retryAfter": <seconds, 0 when not locked>, "round": <n>, "triesLeft": <n>}` (see [`LockStatus`]).
fn lock_json(s: Option<LockStatus>) -> Value {
    match s {
        None => Value::Null,
        Some(s) => {
            let wait = s.wait.map(retry_after_secs);
            json!({
                "until": wait.map(crate::database::timefmt::local_in_secs),
                "retryAfter": wait.unwrap_or(0),
                "round": s.round,
                "triesLeft": s.tries_left,
            })
        }
    }
}

/// `{"users": [...]}` from the database, each with its current login-throttle state (`"lock"`, in memory only).
async fn users_json(st: &AppState) -> Result<Value, AppError> {
    let store = st.0.store.clone();
    let mut users = blocking(move || store.users()).await?;
    let t = &st.0.throttle;
    let now = t.now();
    if let Some(list) = users.get_mut("users").and_then(Value::as_array_mut) {
        for u in list.iter_mut() {
            let lock = lock_json(
                u.get("username")
                    .and_then(Value::as_str)
                    .and_then(|name| t.status(name, now)),
            );
            if let Some(obj) = u.as_object_mut() {
                obj.insert("lock".to_string(), lock);
            }
        }
    }
    Ok(users)
}

async fn users_body(st: &AppState) -> Result<Response, AppError> {
    Ok(ok_json(&users_json(st).await?))
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
    }
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

/// `POST /api/users/create` `{"username", "password", "role"}` → `{"users": [...]}`. The new user must change the password at
/// the first sign-in.
pub async fn create(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    let username =
        username_field(data)?.ok_or_else(|| AppError::BadRequest(MSG_BAD_USERNAME.to_string()))?;
    let password = new_password_field(data)?;
    let role = role_field(data)?.ok_or_else(bad_request)?;
    let phc = hash(st, password).await?;
    let store = st.0.store.clone();
    let id = refused(blocking(move || store.create_user(&username, &phc, role)).await?)?;
    tracing::info!("user {} created user {id}", a.user.id);
    users_body(st).await
}

/// `POST /api/users/update` `{"id", "username"?, "role"?, "disabled"?}` → `{"users": [...]}`.
pub async fn update(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    let id = user_id(data)?;
    let change = UserUpdate {
        username: username_field(data)?,
        role: role_field(data)?,
        disabled: match data.get("disabled") {
            None => None,
            Some(PyVal::Bool(b)) => Some(*b),
            Some(_) => return Err(bad_request()),
        },
    };
    let store = st.0.store.clone();
    let actor = a.user.id;
    refused(blocking(move || store.update_user(actor, id, &change)).await?)?;
    tracing::info!("user {actor} updated user {id}");
    users_body(st).await
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
    // The new password lifts the login lock of that name (the reset-password command, another process, cannot do this)
    st.0.throttle.forget(&name);
    tracing::info!("user {actor} reset the password of user {id}");
    users_body(st).await
}

/// `POST /api/users/unlock` `{"id"}` → `{"users": [...]}`: clears the login throttle of that user's name completely (every
/// bucket: back to round 1, five chances). Allowed for the administrator's own account too.
pub async fn unlock(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    let id = user_id(data)?;
    let store = st.0.store.clone();
    let user = blocking(move || store.user_by_id(id))
        .await?
        .ok_or(status(StatusCode::NOT_FOUND, MSG_USER_NOT_FOUND))?;
    st.0.throttle.forget(&user.username);
    tracing::info!("user {} unlocked the sign-in of user {id}", a.user.id);
    users_body(st).await
}

/// `POST /api/users/delete` `{"id"}` → `{"users": [...]}`.
pub async fn delete(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    let id = user_id(data)?;
    let store = st.0.store.clone();
    let actor = a.user.id;
    refused(blocking(move || store.delete_user(actor, id)).await?)?;
    tracing::info!("user {actor} deleted user {id}");
    users_body(st).await
}
