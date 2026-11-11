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
//! - The session is read from the `xexam_session` cookie only. Its sliding refresh (at most once per hour) re-sends the cookie,
//!   so it happens only on requests whose scheme is known: with `XEXAM_COOKIE_SECURE=1`, or when the request has an `Origin`
//!   (every browser POST does). That way a refresh never drops `Secure` from a cookie issued over HTTPS.
//! - v0.7: the System log names users by their username, never by id (`user "admin" updated user "alex"`); the names come
//!   from the request's session (the actor) and from the same transaction as the change (the user changed).
use super::response::{
    json_resp, MSG_ADMIN_ONLY, MSG_BAD_LOGIN, MSG_BAD_PASSWORD, MSG_BAD_REQUEST, MSG_BAD_USERNAME,
    MSG_LAST_ADMIN, MSG_NOT_YOURSELF, MSG_NO_BANK_PERMISSION, MSG_PASSWORD_CHANGE_REQUIRED,
    MSG_SAME_PASSWORD, MSG_SIGN_IN_REQUIRED, MSG_UNKNOWN_BANKS, MSG_USERNAME_TAKEN,
    MSG_USER_NOT_FOUND, MSG_WRONG_PASSWORD,
};
use super::{blocking, body_flag, body_id, AppState, PyVal};
use crate::auth::throttle::{retry_after_secs, Begun, StartedLock};
use crate::auth::{self, LockStatus, Role, Target, SESSION_PURGE_SECS};
use crate::database::{SessionUser, UserChange, UserUpdate};
use crate::error::{AppError, DbError};
use crate::logs::BurstReport;
use crate::models::{BankAccess, Viewer};
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
    route("GET", "/api/imports", Access::BankManager),
    route("GET", "/api/records", Access::SignedIn),
    route("GET", "/api/record", Access::SignedIn),
    route("GET", "/api/settings", Access::SignedIn),
    route("GET", "/api/users", Access::Admin),
    route("GET", "/api/logs", Access::Admin),
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
    // `can_manage_banks` is true for every administrator (see `SessionUser`) and read from the database with the session
    if access == Access::BankManager && !a.user.can_manage_banks {
        return Err(status(StatusCode::FORBIDDEN, MSG_NO_BANK_PERMISSION));
    }
    Ok(Some(a))
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

/// `GET /api/auth/state` → `{"user": {...} | null, ...}`. Signed in: also `"library"`, an opaque stamp of the question banks
/// the user may see that changes whenever a bank is imported, replaced or deleted, and (v0.6.5) whenever the banks they may see
/// or manage change (an open page re-reads the banks when it changes).
/// Not signed in: also `"defaultAdmin"`, the username of an enabled administrator that still has the default password `admin`,
/// or `null` (the sign-in page then tells the owner how to sign in for the first time, with that account's real name: it may
/// have been renamed; the default credentials are documented anyway).
pub async fn state(st: &AppState, authed: Option<&Authed>) -> Result<Value, AppError> {
    let store = st.0.store.clone();
    match authed {
        Some(a) => {
            let v = viewer(st, a).await?;
            let library = blocking(move || store.library_stamp_as(&v)).await?;
            Ok(json!({ "user": a.user.to_json(), "library": library }))
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
/// report), and in the same way the requests rejected by the Host / Origin guard, the failed sign-ins and the locks of usernames
/// without an account. Runs until the server stops (the caller aborts it).
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
            }
            if let Some(report) = st.0.failed_sign_ins.flush(now) {
                log_failed_sign_ins(&report);
            }
            if let Some(report) = st.0.name_locks.flush(now) {
                log_name_locks(&report);
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
        Target::Account(_) => tracing::warn!("{}", line(&format!("user {username:?}"))),
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
    // An existing account is throttled by its id (its state follows the account, and a flood of made-up names can never evict
    // it); any other name by the name itself, with the same rounds, so the answers do not tell whether the account exists
    let target = user
        .as_ref()
        .map_or(Target::Name(&name), |u| Target::Account(u.id));
    let begun = throttle_begin(st, target, known)?;
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
            log_lock(st, begun.lock, target, known, &u.username);
            return Err(status(StatusCode::UNAUTHORIZED, MSG_BAD_LOGIN));
        }
        None => {
            // Never the typed name: it may be a password typed into the wrong field
            note_failed_sign_in(st, "failed sign-in with a username that has no account");
            log_lock(st, begun.lock, target, known, "");
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
        note_failed_sign_in(
            st,
            &format!(
                "failed sign-in of user {:?}: the account changed during the sign-in",
                user.username
            ),
        );
        return Err(status(StatusCode::UNAUTHORIZED, MSG_BAD_LOGIN));
    };
    st.0.throttle.succeeded(id, known);
    tracing::info!("user {:?} signed in", signed_in.username);
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
    let updated =
        blocking(move || store.change_own_password(id, &expected, &stored, &token_hash, now))
            .await?
            .ok_or(status(StatusCode::UNAUTHORIZED, MSG_SIGN_IN_REQUIRED))?;
    tracing::info!("user {:?} changed their password", updated.username);
    let secure = auth::cookie_secure(st.0.cfg.cookie_secure, origin);
    let mut resp = ok_json(&json!({ "user": updated.to_json() }));
    set_cookie(&mut resp, &auth::session_cookie(&token, secure));
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

/// `{"users": [...]}` from the database, each with its current login-throttle state (`"lock"`, in memory only).
async fn users_json(st: &AppState) -> Result<Value, AppError> {
    let store = st.0.store.clone();
    let mut users = blocking(move || store.users()).await?;
    let t = &st.0.throttle;
    let now = t.now();
    if let Some(list) = users.get_mut("users").and_then(Value::as_array_mut) {
        for u in list.iter_mut() {
            let lock = lock_json(
                u.get("id")
                    .and_then(Value::as_i64)
                    .and_then(|id| t.status(id, now)),
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

/// `POST /api/users/create` `{"username", "password", "role", "banks"?, "newBanks"?, "canManageBanks"?}` →
/// `{"users": [...]}`. The new user must change the password at the first sign-in. `banks` (the existing banks they see)
/// defaults to `[]` (no bank until an administrator chooses some), `newBanks` ("include banks imported later") to false
/// (true with the shorthand `"banks": "all"`), `canManageBanks` to false.
pub async fn create(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    let username =
        username_field(data)?.ok_or_else(|| AppError::BadRequest(MSG_BAD_USERNAME.to_string()))?;
    let password = new_password_field(data)?;
    let role = role_field(data)?.ok_or_else(bad_request)?;
    let banks = banks_field(data)?;
    let new_banks = new_banks_of(data, banks.as_ref())?.unwrap_or(false);
    let banks = banks.unwrap_or(BankAccess::Only(Vec::new()));
    let can_manage = can_manage_field(data)?.unwrap_or(false);
    let phc = hash(st, password).await?;
    let store = st.0.store.clone();
    let name = username.clone();
    refused(
        blocking(move || store.create_user_with(&name, &phc, role, &banks, new_banks, can_manage))
            .await?,
    )?;
    tracing::info!("user {:?} created user {:?}", a.user.username, username);
    users_body(st).await
}

/// `POST /api/users/update` `{"id", "username"?, "role"?, "disabled"?, "banks"?, "newBanks"?, "canManageBanks"?}` →
/// `{"users": [...]}`; only the fields that are given change (`"banks": "all"` also sets `newBanks` unless it is given).
/// A role change keeps `banks`, `newBanks` and `canManageBanks` (they matter only while the user is a plain user).
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
    };
    let store = st.0.store.clone();
    let actor = a.user.id;
    let done = refused(blocking(move || store.update_user(actor, id, &change)).await?)?;
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
    // The new password lifts the login lock of that account (the reset-password command, another process, cannot do this)
    st.0.throttle.forget(id);
    tracing::info!(
        "user {:?} reset the password of user {:?}",
        a.user.username,
        name
    );
    users_body(st).await
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
    users_body(st).await
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
    users_body(st).await
}
