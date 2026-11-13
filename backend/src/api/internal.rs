//! `/internal/*` (v0.8): the main service's side of the link with the AI service (ai-service).
//!
//! The AI service never sees a session cookie. Nginx (and the vite dev / preview server) first asks this backend whether a
//! request to `/api/ai/*` may go through: `GET /internal/ai/auth` runs the same Host / Origin guard and session checks as
//! `/api/*` and answers with a short-lived **identity statement** (header `X-Xexam-Identity`), signed with the internal
//! token, which is the only thing the AI service gets about the caller. The AI service calls back with the same token for
//! the token routes: `POST /internal/ai/introspect` (re-checks a session during a running answer),
//! `POST /internal/ai/users` (which users exist and who has AI+), `POST /internal/ai/log` (its lines for the System log),
//! `POST /internal/ai/heartbeat` (its state, [`ServiceReport`]) and, since v0.9, the three study-data routes
//! `POST /internal/ai/question`, `POST /internal/ai/record` and `POST /internal/ai/record-info` (see `internal_study`), and,
//! since v1.1, the four AI-bank routes `POST /internal/ai/gen/source`, `/gen/seeds`, `/gen/create` and `/gen/lookup` (see
//! `internal_gen`).
//!
//! - The backend never calls the AI service. ai-service reports its state with the heartbeat (at its start, at once after a
//!   change, every 25 s, and a last "stopping" report when it shuts down); the last report is kept in memory only
//!   ([`AiLink`]). AI+ is **available** while that report is younger than [`REPORT_STALE`], is not a stopping report and
//!   says the AI+ service is on ([`ServiceState::ai_available`]): only then may an administrator switch a user's AI+ service
//!   on or off (`POST /api/users/update` / `create`, else 409 `MSG_AI_SERVICE_OFF`). `GET /api/auth/state` and the
//!   `/api/users*` answers say whether AI+ is available, and which users' AI+ function is suspended.
//!
//! - The routes are answered by [`dispatch`], which the API's dispatch function calls for every path that starts with
//!   `/internal/`, before the Host guard and without the session layer (the auth step reads Host, Origin and method from
//!   the `X-Original-*` headers Nginx sets, never from the request itself). Nginx never proxies `/internal/` from outside.
//! - Anything else under `/internal/`, a wrong method, a missing or wrong token, or a token route while no token is loaded
//!   answers 404 plain text, exactly like an unknown path: the endpoints are not revealed. Refused token requests are
//!   logged through [`AiLink`]'s own [`BurstLog`], at most about one line a minute.
//! - The internal token ([`InternalToken`], never logged, never in `Config`, redacted `Debug`) comes from
//!   `XEXAM_INTERNAL_TOKEN` (at least 32 printable ASCII characters without spaces once the whitespace around the value is
//!   removed; shorter or with other characters: the link is off), else from the file `<link dir>/internal.token`
//!   (whitespace around the content ignored, like the AI service does), created by `serve` when it is missing (43
//!   base64url characters of 32 random bytes, mode 0600 in a 0700 directory) and only read by the AI service
//!   ([`load_link`]). Without a token the link is off: the auth step answers 503 and the token routes 404.
//! - The auth step reads the session cookie more strictly than the other routes: exactly one `xexam_session` cookie, as
//!   Nginx keys the per-session limit on starting AI answers by it (`auth::single_session_token`).
//! - Identity statement: `base64url(payload) "." base64url(HMAC-SHA-256(token, "xexam-identity-v1." base64url(payload)))`,
//!   the payload being compact JSON `{"v":1,"id","stamp","username","role","aiAllowed","sid","exp"}` (in that order;
//!   `stamp` = `users.uid`, `sid` = the session's token hash, `exp` = now + 60 s). See [`identity_statement`].
use super::response::{self, error_json, json_resp, text_resp, MSG_BAD_REQUEST};
use super::{auth, AppState};
use crate::auth::hmac_sha256;
use crate::database::SessionUser;
use crate::error::AppError;
use crate::logs::{BurstLog, BurstReport};
use crate::security::base64url_nopad;
use crate::security::body::{self, BodyError};
use crate::security::{py_strip, truncate_chars};
use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;

/// The environment variable that sets the internal token (the same value must reach both services).
pub const TOKEN_ENV: &str = "XEXAM_INTERNAL_TOKEN";
/// Name of the token file in the link directory.
pub const TOKEN_FILE: &str = "internal.token";
/// Request header of the AI service's calls to the token routes.
pub const TOKEN_HEADER: HeaderName = HeaderName::from_static("x-xexam-internal-token");
/// Response header of the auth step: the identity statement.
pub const IDENTITY_HEADER: HeaderName = HeaderName::from_static("x-xexam-identity");
/// Response header of a refused auth step: `signin`, `password` or `guard` (Nginx picks the error body by it).
pub const DENY_HEADER: HeaderName = HeaderName::from_static("x-xexam-deny");
/// The shortest internal token accepted.
pub const MIN_TOKEN_CHARS: usize = 32;
/// Lifetime of an identity statement (seconds).
pub const IDENTITY_TTL_SECS: i64 = 60;
/// Largest body of `POST /internal/ai/introspect`, `POST /internal/ai/users` and `POST /internal/ai/heartbeat` (and what is
/// drained of a refused request).
pub const MAX_BODY: usize = 64 * 1024;
/// Largest body of `POST /internal/ai/gen/create` (v1.1, the only route that gets it): a generated bank of 100 questions at
/// the generator's maximum lengths is about 4.6 MB of JSON, a worst-case file import (questions mode) about 8.1 MB; a typical
/// one is about 60 KB. Every other internal route keeps [`MAX_BODY`].
pub const MAX_GEN_BODY: usize = 10 * 1024 * 1024;
/// A heartbeat older than this means ai-service is not connected (it reports every 25 s).
pub const REPORT_STALE: Duration = Duration::from_secs(60);
/// Most user ids in `"suspended"` of one heartbeat.
pub const MAX_SUSPENDED_IDS: usize = 3000;
/// Most lines in one `POST /internal/ai/log`.
pub const MAX_LOG_LINES: usize = 50;
/// Longest message of one forwarded log line (characters); longer ones are cut and end with `…`.
pub const MAX_LOG_CHARS: usize = 2000;
/// Largest body of `POST /internal/ai/log`: a full batch the contract allows ([`MAX_LOG_LINES`] messages of
/// [`MAX_LOG_CHARS`] characters) even when every character takes the longest form the AI service's JSON writer
/// (`serde_json`) gives it (6 bytes, a `\u0001` escape; 4 bytes for a character outside the BMP, written as UTF-8), plus
/// the JSON around each line. About 590 KiB; a batch the AI service sends is never refused for its size.
pub const MAX_LOG_BODY: usize = MAX_LOG_LINES * (MAX_LOG_CHARS * 6 + 64) + 1024;
/// Most ids in one `POST /internal/ai/users`.
pub const MAX_USER_IDS: usize = 10_000;
/// Prefix of the signed message (domain separation from every other HMAC this program computes).
const IDENTITY_CONTEXT: &str = "xexam-identity-v1.";
/// Largest token file read (a valid one has 43 characters).
const MAX_TOKEN_FILE: u64 = 4096;

const ORIGINAL_HOST: HeaderName = HeaderName::from_static("x-original-host");
const ORIGINAL_ORIGIN: HeaderName = HeaderName::from_static("x-original-origin");
const ORIGINAL_METHOD: HeaderName = HeaderName::from_static("x-original-method");

/// The secret shared with the AI service: the HMAC key of the identity statement and the credential of its calls to the
/// token routes. Never logged; `Debug` prints no part of it.
#[derive(Clone)]
pub struct InternalToken(String);

/// Why a value cannot be the internal token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenProblem {
    /// Fewer than [`MIN_TOKEN_CHARS`] characters
    TooShort,
    /// A character outside printable ASCII without the space (`!` to `~`)
    NotPrintable,
}

impl InternalToken {
    /// A token: at least [`MIN_TOKEN_CHARS`] characters, each printable ASCII without the space (`0x21..=0x7E`).
    pub fn new(value: &str) -> Result<InternalToken, TokenProblem> {
        if !value.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
            return Err(TokenProblem::NotPrintable);
        }
        if value.len() < MIN_TOKEN_CHARS {
            return Err(TokenProblem::TooShort);
        }
        Ok(InternalToken(value.to_string()))
    }

    fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Whether `presented` is this token. Both sides are hashed first, so the comparison takes the same time whatever the
    /// lengths are (constant time, `subtle`).
    fn matches(&self, presented: &[u8]) -> bool {
        let a = Sha256::digest(self.as_bytes());
        let b = Sha256::digest(presented);
        a.as_slice().ct_eq(b.as_slice()).into()
    }
}

impl std::fmt::Debug for InternalToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InternalToken(<redacted>)")
    }
}

/// The last accepted heartbeat of ai-service (`POST /internal/ai/heartbeat`), kept in memory only: none after a restart
/// of the main service until the next report arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceReport {
    /// When it was accepted (monotonic clock)
    pub at: Instant,
    /// When it was accepted (unix seconds; `"lastSeen"` of `GET /api/auth/state`)
    pub at_unix: i64,
    /// ai-service's version (trimmed; the rule of [`crate::config::valid_version`])
    pub version: String,
    /// ai-service's effective AI+ service state (on in AI settings, the key verified, no key problem)
    pub enabled: bool,
    /// The users whose AI+ function an administrator suspended in AI settings
    pub suspended: BTreeSet<i64>,
    /// The last report ai-service sends while it shuts down
    pub stopping: bool,
}

impl ServiceReport {
    /// A report that is not a stopping report and is younger than [`REPORT_STALE`] at `now`.
    fn connected_at(&self, now: Instant) -> bool {
        !self.stopping && now.saturating_duration_since(self.at) < REPORT_STALE
    }
}

/// What the main service knows about ai-service right now, derived from the last report alone (no I/O).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceState {
    /// A report younger than [`REPORT_STALE`] that is not a stopping report
    pub connected: bool,
    /// The last report was a stopping report
    pub stopped: bool,
    /// Unix seconds of the last report (`None`: no report since the main service started)
    pub last_seen: Option<i64>,
    /// Version of the last report
    pub version: Option<String>,
    /// `enabled` of the last report while connected; false when not connected
    pub enabled: bool,
    /// The suspended users of the last report (kept while it is stale; empty when there is none)
    pub suspended: BTreeSet<i64>,
}

impl ServiceState {
    /// AI+ is available: ai-service is connected and its AI+ service is on.
    pub fn ai_available(&self) -> bool {
        self.connected && self.enabled
    }
}

/// The state of the AI link in [`super::Inner`]: the internal token once `serve` loaded it (none in tests that do not set
/// one, and none when the link is off), the rate limit of the log line about refused token requests, and the last
/// heartbeat of ai-service.
#[derive(Default)]
pub struct AiLink {
    token: OnceLock<InternalToken>,
    refused: BurstLog,
    report: Mutex<Option<ServiceReport>>,
}

impl std::fmt::Debug for AiLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiLink")
            .field("on", &self.is_on())
            .finish_non_exhaustive()
    }
}

impl AiLink {
    fn last_report(&self) -> std::sync::MutexGuard<'_, Option<ServiceReport>> {
        self.report.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps `report` as the last heartbeat (replacing the one before).
    pub fn record(&self, report: ServiceReport) {
        *self.last_report() = Some(report);
    }

    /// What the last heartbeat says now (see [`ServiceState`]).
    pub fn state(&self) -> ServiceState {
        let now = Instant::now();
        match self.last_report().as_ref() {
            None => ServiceState::default(),
            Some(r) => {
                let connected = r.connected_at(now);
                ServiceState {
                    connected,
                    stopped: r.stopping,
                    last_seen: Some(r.at_unix),
                    version: Some(r.version.clone()),
                    enabled: connected && r.enabled,
                    suspended: r.suspended.clone(),
                }
            }
        }
    }

    /// Whether AI+ is available now: ai-service connected (a fresh, non-stopping report) and its AI+ service on.
    pub fn ai_available(&self) -> bool {
        let now = Instant::now();
        self.last_report()
            .as_ref()
            .is_some_and(|r| r.connected_at(now) && r.enabled)
    }

    /// Whether the last report names user `id` as suspended (also while it is stale; false without a report).
    pub fn suspended(&self, id: i64) -> bool {
        self.last_report()
            .as_ref()
            .is_some_and(|r| r.suspended.contains(&id))
    }

    /// `"aiService"` of `GET /api/auth/state` (administrators only): `{"connected", "stopped", "lastSeen", "version",
    /// "enabled", "link"}` (`link`: an internal token is loaded).
    pub fn service_json(&self, s: &ServiceState) -> Value {
        json!({
            "connected": s.connected,
            "stopped": s.stopped,
            "lastSeen": s.last_seen,
            "version": s.version,
            "enabled": s.enabled,
            "link": self.is_on(),
        })
    }

    /// Makes the last report `by` older (tests of a stale report only; production code never calls it).
    #[doc(hidden)]
    pub fn age_report_for_tests(&self, by: Duration) {
        if let Some(r) = self.last_report().as_mut() {
            if let Some(at) = r.at.checked_sub(by) {
                r.at = at;
            }
            r.at_unix = r
                .at_unix
                .saturating_sub(i64::try_from(by.as_secs()).unwrap_or(i64::MAX));
        }
    }

    /// Sets the token (once; `false` when one was set already, which is then kept).
    pub fn set_token(&self, token: InternalToken) -> bool {
        self.token.set(token).is_ok()
    }

    /// The loaded token, if any.
    pub fn token(&self) -> Option<&InternalToken> {
        self.token.get()
    }

    /// Whether a token is loaded (the link is on).
    pub fn is_on(&self) -> bool {
        self.token.get().is_some()
    }

    fn accepts(&self, headers: &HeaderMap) -> bool {
        match (self.token.get(), headers.get(&TOKEN_HEADER)) {
            (Some(t), Some(v)) => t.matches(v.as_bytes()),
            _ => false,
        }
    }

    /// The refused token requests counted since the last report line, when one is due (the server's once-a-minute timer).
    pub fn flush_refusals(&self, now: std::time::Instant) -> Option<BurstReport> {
        self.refused.flush(now)
    }
}

/// Logs a [`BurstReport`] of refused token requests.
pub fn log_refused(report: &BurstReport) {
    match report {
        BurstReport::One(what) => {
            tracing::warn!("refused an internal request without a valid token: {what}")
        }
        BurstReport::Many { count, last } => tracing::warn!(
            "refused {count} internal request(s) without a valid token since the last report; the last one: {last}"
        ),
    }
}

// ---------------------------------------------------------------- the token at startup

/// What [`load_link`] found.
#[derive(Debug)]
pub enum LinkLoad {
    /// From `XEXAM_INTERNAL_TOKEN`
    Env(InternalToken),
    /// From the token file (`created`: this start created it)
    File {
        token: InternalToken,
        path: PathBuf,
        created: bool,
    },
    /// No token: the AI link is off. The line to log says why.
    Off(String),
}

impl LinkLoad {
    /// The startup log line (English; never contains the token).
    pub fn log_line(&self) -> String {
        match self {
            LinkLoad::Env(_) => format!("AI link: using {TOKEN_ENV}"),
            LinkLoad::File { path, .. } => format!("AI link: {}", path.display()),
            LinkLoad::Off(why) => why.clone(),
        }
    }

    /// The token, when the link is on.
    pub fn into_token(self) -> Option<InternalToken> {
        match self {
            LinkLoad::Env(t) | LinkLoad::File { token: t, .. } => Some(t),
            LinkLoad::Off(_) => None,
        }
    }
}

/// Finds the internal token when `serve` starts (never in the command-line subcommands): `env_value` (the value of
/// `XEXAM_INTERNAL_TOKEN`) when it is set and not blank, else the file `<link_dir>/internal.token`, created when it is
/// missing. Whitespace around the value and around the file's content is ignored, exactly as the AI service does
/// (`str::trim` on both), so both services read the same token from the same source. A value or file that is not a
/// valid token turns the link off (no fallback: a mistyped token must not silently become another one).
pub fn load_link(env_value: Option<&str>, link_dir: &Path) -> LinkLoad {
    if let Some(v) = env_value.map(str::trim).filter(|v| !v.is_empty()) {
        return match InternalToken::new(v) {
            Ok(t) => LinkLoad::Env(t),
            Err(TokenProblem::TooShort) => LinkLoad::Off(format!(
                "{TOKEN_ENV} is too short (at least {MIN_TOKEN_CHARS} characters); AI link off"
            )),
            Err(TokenProblem::NotPrintable) => LinkLoad::Off(format!(
                "{TOKEN_ENV} may contain only printable ASCII characters without spaces; AI link off"
            )),
        };
    }
    let path = std::path::absolute(link_dir.join(TOKEN_FILE))
        .unwrap_or_else(|_| link_dir.join(TOKEN_FILE));
    // At most twice: a missing file is created; when someone else created it first, it is read again
    for _ in 0..2 {
        match read_token_file(&path) {
            Ok(text) => return token_from_file(&text, path, false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return LinkLoad::Off(format!(
                    "AI link off: cannot read {} ({})",
                    path.display(),
                    e.kind()
                ))
            }
        }
        match create_token_file(&path) {
            Ok(Some(token)) => {
                return LinkLoad::File {
                    token,
                    path,
                    created: true,
                }
            }
            Ok(None) => {}
            Err(e) => {
                return LinkLoad::Off(format!(
                    "AI link off: cannot create {} ({})",
                    path.display(),
                    e.kind()
                ))
            }
        }
    }
    LinkLoad::Off(format!("AI link off: cannot read {}", path.display()))
}

/// The token in the file's `text`: the whole content without the whitespace around it (a hand-written file may end with
/// a newline or two, `\r\n` or spaces), as the AI service reads it.
fn token_from_file(text: &str, path: PathBuf, created: bool) -> LinkLoad {
    match InternalToken::new(text.trim()) {
        Ok(token) => LinkLoad::File {
            token,
            path,
            created,
        },
        Err(_) => LinkLoad::Off(format!(
            "AI link off: {} does not hold a valid token (at least {MIN_TOKEN_CHARS} printable ASCII characters without \
             spaces); move it out of the way to have a new one created at the next start",
            path.display()
        )),
    }
}

fn read_token_file(path: &Path) -> std::io::Result<String> {
    let mut buf = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_TOKEN_FILE + 1)
        .read_to_end(&mut buf)?;
    // Invalid UTF-8 becomes U+FFFD, which no token contains
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Writes `value` into a new file at `path` (`create_new`, mode 0600 on unix) and syncs it; a file that could not be
/// written completely is removed again. `AlreadyExists` when `path` exists.
fn write_new_file(path: &Path, value: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    if let Err(e) = file
        .write_all(value.as_bytes())
        .and_then(|()| file.sync_all())
    {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(e);
    }
    Ok(())
}

/// Creates the directory (mode 0700 on unix) when it is missing, then the token file (mode 0600 on unix): 32 random bytes
/// as 43 base64url characters, synced to disk. The value is written to a temporary file first and then linked under the
/// final name, which fails when that name exists: whoever reads the file (the AI service) never sees it half written, and
/// two creators cannot overwrite each other. Where the file system has no hard links, the file is created in place with
/// `create_new`. `Ok(None)` when the file exists already (someone else created it in the meantime).
fn create_token_file(path: &Path) -> std::io::Result<Option<InternalToken>> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    if !dir.is_dir() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(dir)?;
    }
    let value = crate::auth::new_session_token();
    let tmp = dir.join(format!(
        ".{TOKEN_FILE}.{}.tmp",
        crate::security::new_token()
    ));
    write_new_file(&tmp, &value)?;
    let linked = std::fs::hard_link(&tmp, path);
    let _ = std::fs::remove_file(&tmp);
    match linked {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(None),
        // No hard links here: create it in place
        Err(_) => match write_new_file(path, &value) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(None),
            Err(e) => return Err(e),
        },
    }
    // The new directory entry survives a crash too (unix; best effort)
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(InternalToken::new(&value).ok())
}

// ---------------------------------------------------------------- the identity statement

/// The identity statement of `user`, signed in with the session whose token hash is `sid`, valid until `now` + 60 s (see
/// the module documentation). Never logged.
pub fn identity_statement(
    token: &InternalToken,
    user: &SessionUser,
    sid: &str,
    now: i64,
) -> String {
    let payload = json!({
        "v": 1,
        "id": user.id,
        "stamp": user.uid,
        "username": user.username,
        "role": user.role.as_str(),
        "aiAllowed": user.ai_allowed,
        "sid": sid,
        "exp": now.saturating_add(IDENTITY_TTL_SECS),
    });
    // Compact JSON (serde_json's default writer), keys in the order above (`preserve_order`)
    let p64 = base64url_nopad(payload.to_string().as_bytes());
    let sig = hmac_sha256(
        token.as_bytes(),
        format!("{IDENTITY_CONTEXT}{p64}").as_bytes(),
    );
    format!("{p64}.{}", base64url_nopad(&sig))
}

// ---------------------------------------------------------------- dispatch

/// 404 plain text, like any unknown path.
fn not_found(head_only: bool) -> Response {
    text_resp(StatusCode::NOT_FOUND, "not found", head_only)
}

/// Every request whose path starts with `/internal/` (called by the API's dispatch function before anything else).
pub(super) async fn dispatch(
    st: &AppState,
    method: &Method,
    path: &str,
    headers: &HeaderMap,
    mut body: Body,
) -> Response {
    let head_only = method == Method::HEAD;
    match (method.as_str(), path) {
        ("GET", "/internal/ai/auth") => auth_step(st, headers).await,
        (
            "POST",
            "/internal/ai/introspect"
            | "/internal/ai/users"
            | "/internal/ai/log"
            | "/internal/ai/heartbeat"
            | "/internal/ai/question"
            | "/internal/ai/record"
            | "/internal/ai/record-info"
            | "/internal/ai/gen/source"
            | "/internal/ai/gen/seeds"
            | "/internal/ai/gen/create"
            | "/internal/ai/gen/lookup"
            | "/internal/ai/import/check",
        ) => {
            if !st.0.ai_link.accepts(headers) {
                body::drain(&mut body, 0, MAX_BODY.saturating_mul(4)).await;
                let what = if st.0.ai_link.is_on() {
                    format!("POST {path}")
                } else {
                    format!("POST {path} (the AI link is off)")
                };
                if let Some(report) = st.0.ai_link.refused.note(std::time::Instant::now(), what) {
                    log_refused(&report);
                }
                return not_found(false);
            }
            let max = match path {
                "/internal/ai/log" => MAX_LOG_BODY,
                "/internal/ai/gen/create" => MAX_GEN_BODY,
                _ => MAX_BODY,
            };
            let data = match read_object(headers, body, max).await {
                Ok(d) => d,
                Err(e) => return response::error_response(e, false),
            };
            let answer = match path {
                "/internal/ai/introspect" => introspect_session(st, &data).await,
                "/internal/ai/users" => users(st, &data).await,
                "/internal/ai/heartbeat" => heartbeat(st, &data),
                "/internal/ai/question" => super::internal_study::question(st, &data).await,
                "/internal/ai/record" => super::internal_study::record(st, &data).await,
                "/internal/ai/record-info" => super::internal_study::record_info(st, &data).await,
                "/internal/ai/gen/source" => super::internal_gen::source(st, &data).await,
                "/internal/ai/gen/seeds" => super::internal_gen::seeds(st, &data).await,
                "/internal/ai/gen/create" => super::internal_gen::create(st, &data).await,
                "/internal/ai/gen/lookup" => super::internal_gen::lookup(st, &data).await,
                "/internal/ai/import/check" => super::internal_gen::import_check(st, &data).await,
                _ => log_lines(st, &data),
            };
            match answer {
                Ok(v) => json_resp(StatusCode::OK, &v, false),
                Err(e) => response::error_response(e, false),
            }
        }
        _ => {
            body::drain(&mut body, 0, MAX_BODY.saturating_mul(4)).await;
            not_found(head_only)
        }
    }
}

/// `GET /internal/ai/auth`, the auth step of every `/api/ai/*` request (see [`auth::introspect`]).
async fn auth_step(st: &AppState, headers: &HeaderMap) -> Response {
    match auth::introspect(st, headers).await {
        auth::IntrospectOutcome::LinkOff => response::make(
            StatusCode::SERVICE_UNAVAILABLE,
            response::TEXT_CT,
            Vec::new(),
            false,
        ),
        auth::IntrospectOutcome::Failed => error_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            response::MSG_INTERNAL,
            false,
        ),
        auth::IntrospectOutcome::Guard(denial) => denied(
            StatusCode::FORBIDDEN,
            "guard",
            &denial.message,
            "auth.guard",
        ),
        auth::IntrospectOutcome::SignIn => denied(
            StatusCode::UNAUTHORIZED,
            "signin",
            response::MSG_SIGN_IN_REQUIRED,
            "auth.signin",
        ),
        auth::IntrospectOutcome::Password => denied(
            StatusCode::FORBIDDEN,
            "password",
            response::MSG_PASSWORD_CHANGE_REQUIRED,
            "auth.password",
        ),
        auth::IntrospectOutcome::Allowed(statement) => {
            let mut resp = Response::new(Body::empty());
            *resp.status_mut() = StatusCode::NO_CONTENT;
            let h = resp.headers_mut();
            if let Ok(mut v) = HeaderValue::from_str(&statement) {
                v.set_sensitive(true);
                h.insert(IDENTITY_HEADER, v);
            }
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            h.insert(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            );
            resp
        }
    }
}

/// A refused auth step: `{"error": <message>, "code": <code>}` and the reason in `X-Xexam-Deny`.
fn denied(status: StatusCode, reason: &'static str, message: &str, code: &str) -> Response {
    let mut resp = json_resp(status, &json!({ "error": message, "code": code }), false);
    resp.headers_mut()
        .insert(DENY_HEADER, HeaderValue::from_static(reason));
    resp
}

/// The `X-Original-*` header `name` (decoded like every other header, latin-1).
fn original(headers: &HeaderMap, name: &HeaderName) -> Option<String> {
    super::hdr(headers, name)
}

/// Host, Origin and method of the request being checked, from the `X-Original-*` headers only. An empty Origin counts as
/// none (browsers never send an empty one; a proxy may pass an empty value for a missing header).
pub(super) fn original_request(headers: &HeaderMap) -> (Option<String>, Option<String>, String) {
    let host = original(headers, &ORIGINAL_HOST);
    let origin = original(headers, &ORIGINAL_ORIGIN).filter(|o| !py_strip(o).is_empty());
    let method = original(headers, &ORIGINAL_METHOD).unwrap_or_default();
    (host, origin, method)
}

/// A token route's body: `Content-Type: application/json`, at most `max` bytes ([`MAX_BODY`], or [`MAX_LOG_BODY`] for the
/// log route; more: 413), a JSON object (an empty body is `{}`).
async fn read_object(
    headers: &HeaderMap,
    body: Body,
    max: usize,
) -> Result<serde_json::Map<String, Value>, AppError> {
    let ctype = super::hdr(headers, &header::CONTENT_TYPE).unwrap_or_default();
    if py_strip(ctype.split(';').next().unwrap_or("")).to_lowercase() != "application/json" {
        let mut body = body;
        body::drain(&mut body, 0, max.saturating_mul(4)).await;
        return Err(AppError::UnsupportedMediaType);
    }
    let raw = match body::read_limited(body, super::declared_length(headers), max).await {
        Ok(r) => r,
        Err(BodyError::TooLarge) => return Err(AppError::JsonTooLarge),
        Err(BodyError::Broken) => return Err(AppError::BadJson),
    };
    if raw.is_empty() {
        return Ok(serde_json::Map::new());
    }
    match serde_json::from_slice::<Value>(&raw) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(AppError::BadJson),
    }
}

fn bad_request() -> AppError {
    AppError::BadRequest(MSG_BAD_REQUEST.to_string())
}

fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `POST /internal/ai/introspect {"sid": "<64 hex>", "id": n}` → `{"ok": true, "user": {...}}` while that session is
/// valid, belongs to user `id` and no password change is pending; `{"ok": false}` otherwise. Never refreshes the session.
async fn introspect_session(
    st: &AppState,
    data: &serde_json::Map<String, Value>,
) -> Result<Value, AppError> {
    let sid = match data.get("sid") {
        Some(Value::String(s)) if is_lower_hex(s, 64) => s.clone(),
        _ => return Err(bad_request()),
    };
    let id = match data.get("id").and_then(Value::as_i64) {
        Some(i) if i > 0 => i,
        _ => return Err(bad_request()),
    };
    let store = st.0.store.clone();
    let now = crate::auth::unix_now();
    let found = super::blocking(move || store.session_user(&sid, now, false)).await?;
    Ok(match found {
        Some((u, _)) if u.id == id && !u.must_change_password => json!({
            "ok": true,
            "user": {
                "id": u.id,
                "stamp": u.uid,
                "username": u.username,
                "role": u.role.as_str(),
                "aiAllowed": u.ai_allowed,
            },
        }),
        _ => json!({ "ok": false }),
    })
}

/// `POST /internal/ai/users {"ids": [n, …], "holders": bool}` → `{"users": [...]}`: the requested users that exist, plus
/// (with `holders`) everyone who has AI+, each once, by id. A missing id is a deleted user.
async fn users(st: &AppState, data: &serde_json::Map<String, Value>) -> Result<Value, AppError> {
    let ids: Vec<i64> = match data.get("ids") {
        None => Vec::new(),
        Some(Value::Array(items)) if items.len() <= MAX_USER_IDS => items
            .iter()
            .map(|v| v.as_i64().filter(|i| *i > 0))
            .collect::<Option<Vec<i64>>>()
            .ok_or_else(bad_request)?,
        Some(_) => return Err(bad_request()),
    };
    let holders = match data.get("holders") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad_request()),
    };
    let store = st.0.store.clone();
    let rows = super::blocking(move || store.ai_users(&ids, holders)).await?;
    Ok(json!({ "users": rows.iter().map(|u| u.to_json()).collect::<Vec<_>>() }))
}

/// `POST /internal/ai/heartbeat {"version": str, "enabled": bool, "suspended": [n, …], "stopping": bool}` → `{"ok": true}`:
/// ai-service's state, kept as the last report ([`AiLink::record`]). Exactly these four keys; `version` follows the rule of
/// [`crate::config::valid_version`] (kept trimmed), `suspended` holds at most [`MAX_SUSPENDED_IDS`] positive integers
/// (duplicates are ignored). Anything else is 400 and the last report stays as it was. Never logged (it comes every 25 s).
fn heartbeat(st: &AppState, data: &serde_json::Map<String, Value>) -> Result<Value, AppError> {
    const KEYS: [&str; 4] = ["version", "enabled", "suspended", "stopping"];
    if data.len() != KEYS.len() || !KEYS.iter().all(|k| data.contains_key(*k)) {
        return Err(bad_request());
    }
    let version = match data.get("version") {
        Some(Value::String(s)) if crate::config::valid_version(s) => s.trim().to_string(),
        _ => return Err(bad_request()),
    };
    let flag = |key: &str| match data.get(key) {
        Some(Value::Bool(b)) => Ok(*b),
        _ => Err(bad_request()),
    };
    let enabled = flag("enabled")?;
    let stopping = flag("stopping")?;
    let suspended: BTreeSet<i64> = match data.get("suspended") {
        Some(Value::Array(items)) if items.len() <= MAX_SUSPENDED_IDS => items
            .iter()
            .map(|v| v.as_i64().filter(|i| *i > 0))
            .collect::<Option<BTreeSet<i64>>>()
            .ok_or_else(bad_request)?,
        _ => return Err(bad_request()),
    };
    st.0.ai_link.record(ServiceReport {
        at: Instant::now(),
        at_unix: crate::auth::unix_now(),
        version,
        enabled,
        suspended,
        stopping,
    });
    Ok(json!({ "ok": true }))
}

/// `POST /internal/ai/log {"lines": [{"level": "INFO"|"WARN"|"ERROR", "message": str}, …]}` → `{"ok": true}`: each line
/// goes into the System log as `AI: <message>` (target `xexam_ai_service`), control characters removed, cut at
/// [`MAX_LOG_CHARS`] characters. All lines are checked first: one invalid line (or more than [`MAX_LOG_LINES`]) refuses
/// the whole batch with 400. The lines are not printed to stderr (the AI container prints its own).
fn log_lines(st: &AppState, data: &serde_json::Map<String, Value>) -> Result<Value, AppError> {
    let Some(Value::Array(lines)) = data.get("lines") else {
        return Err(bad_request());
    };
    if lines.len() > MAX_LOG_LINES {
        return Err(bad_request());
    }
    let mut checked = Vec::with_capacity(lines.len());
    for line in lines {
        let level = match line.get("level").and_then(Value::as_str) {
            Some("INFO") => tracing::Level::INFO,
            Some("WARN") => tracing::Level::WARN,
            Some("ERROR") => tracing::Level::ERROR,
            _ => return Err(bad_request()),
        };
        let Some(message) = line.get("message").and_then(Value::as_str) else {
            return Err(bad_request());
        };
        let clean: String = message.chars().filter(|c| !c.is_control()).collect();
        let mut text = truncate_chars(&clean, MAX_LOG_CHARS);
        if text.len() < clean.len() {
            text.push('…');
        }
        checked.push((level, text));
    }
    for (level, text) in checked {
        st.0.logs
            .push(level, "xexam_ai_service", format!("AI: {text}"));
    }
    Ok(json!({ "ok": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_need_32_printable_characters_and_never_show_in_debug() {
        let good = "abcdefghijklmnopqrstuvwxyz012345";
        assert!(InternalToken::new(good).is_ok());
        assert_eq!(
            InternalToken::new(&good[1..]).err(),
            Some(TokenProblem::TooShort)
        );
        let spaced = format!("{good} x");
        assert_eq!(
            InternalToken::new(&spaced).err(),
            Some(TokenProblem::NotPrintable)
        );
        assert_eq!(
            InternalToken::new(&format!("{good}\u{e9}")).err(),
            Some(TokenProblem::NotPrintable)
        );
        let t = InternalToken::new(good).expect("token");
        assert!(!format!("{t:?}").contains("abc"));
        assert!(t.matches(good.as_bytes()));
        assert!(!t.matches(&good.as_bytes()[1..]));
        assert!(!t.matches(b""));
        let link = AiLink::default();
        assert!(!link.is_on());
        assert!(link.set_token(t.clone()));
        assert!(!link.set_token(t), "set once");
        assert!(link.is_on());
        assert!(!format!("{link:?}").contains("abc"));
    }

    #[test]
    fn the_token_comes_from_the_environment_or_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let link = dir.path().join("nested").join("link");
        // An environment value wins and never touches the file
        let env = "env-token-env-token-env-token-0000";
        match load_link(Some(env), &link) {
            LinkLoad::Env(t) => assert!(t.matches(env.as_bytes())),
            other => panic!("{other:?}"),
        }
        assert!(!link.exists(), "no file when the variable is set");
        // Too short or with a space: the link is off (no fallback to the file)
        let off = load_link(Some("short"), &link);
        assert_eq!(
            off.log_line(),
            "XEXAM_INTERNAL_TOKEN is too short (at least 32 characters); AI link off"
        );
        assert!(off.into_token().is_none());
        assert!(load_link(Some("with space with space with space x"), &link)
            .into_token()
            .is_none());
        assert!(!link.exists());
        // Unset or blank: the file, created with 43 base64url characters
        let first = load_link(Some("  "), &link);
        let LinkLoad::File {
            token,
            path,
            created,
        } = &first
        else {
            panic!("{first:?}")
        };
        assert!(*created);
        assert_eq!(path.file_name().and_then(|n| n.to_str()), Some(TOKEN_FILE));
        let text = std::fs::read_to_string(path).expect("token file");
        assert_eq!(text.len(), 43);
        assert!(text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        assert!(token.matches(text.as_bytes()));
        assert_eq!(first.log_line(), format!("AI link: {}", path.display()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path).expect("meta").permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            let mode = std::fs::metadata(&link).expect("meta").permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        // The next start reads the same token (a trailing newline written by hand is fine)
        match load_link(None, &link) {
            LinkLoad::File { token, created, .. } => {
                assert!(!created);
                assert!(token.matches(text.as_bytes()));
            }
            other => panic!("{other:?}"),
        }
        // Whitespace around the file's content is ignored, as the AI service does (`str::trim`)
        for padded in [
            format!("{text}\r\n"),
            format!("{text}\n\n"),
            format!("  {text} \t\r\n"),
        ] {
            std::fs::write(link.join(TOKEN_FILE), &padded).expect("write");
            assert!(
                load_link(None, &link)
                    .into_token()
                    .is_some_and(|t| t.matches(text.as_bytes())),
                "{padded:?}"
            );
        }
        // ... and around the environment value: the trimmed value is the token
        for padded in [
            format!("  {env} "),
            format!("{env}\n"),
            format!("\t{env}\r\n"),
        ] {
            match load_link(Some(&padded), &link) {
                LinkLoad::Env(t) => {
                    assert!(t.matches(env.as_bytes()), "{padded:?}");
                    assert!(!t.matches(padded.as_bytes()), "{padded:?}");
                }
                other => panic!("{padded:?}: {other:?}"),
            }
        }
        // whitespace inside still turns the link off
        assert_eq!(
            load_link(Some(&format!(" {env} x ")), &link).log_line(),
            "XEXAM_INTERNAL_TOKEN may contain only printable ASCII characters without spaces; AI link off"
        );
        // A file that holds no valid token turns the link off and is left alone
        std::fs::write(link.join(TOKEN_FILE), "short\n").expect("write");
        let off = load_link(None, &link);
        assert!(off.log_line().starts_with("AI link off: "), "{off:?}");
        assert!(off.into_token().is_none());
        assert_eq!(
            std::fs::read_to_string(link.join(TOKEN_FILE)).expect("read"),
            "short\n"
        );
    }

    #[test]
    fn a_link_dir_that_cannot_be_created_turns_the_link_off() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A file where the directory should be
        let blocker = dir.path().join("link");
        std::fs::write(&blocker, b"x").expect("write");
        let off = load_link(None, &blocker);
        let line = off.log_line();
        assert!(line.starts_with("AI link off: cannot "), "{line}");
        assert!(off.into_token().is_none());
    }

    #[test]
    fn two_starts_at_once_share_one_token_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let link = dir.path().join("link");
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let link = link.clone();
                std::thread::spawn(move || load_link(None, &link).into_token())
            })
            .collect();
        let text = {
            let tokens: Vec<InternalToken> = handles
                .into_iter()
                .map(|h| h.join().expect("thread").expect("token"))
                .collect();
            let text = std::fs::read_to_string(link.join(TOKEN_FILE)).expect("read");
            for t in &tokens {
                assert!(t.matches(text.as_bytes()));
            }
            text
        };
        assert_eq!(text.len(), 43);
    }
}
