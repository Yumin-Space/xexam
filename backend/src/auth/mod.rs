//! Accounts and sign-in (v0.6): password hashing, session tokens, the session cookie, the login throttle and the input rules for
//! usernames and passwords. Contract: `docs/en/api.md` and `docs/en/security.md`.
//!
//! - Passwords: Argon2id with the `argon2` crate's default parameters and a random salt per hash, stored as a PHC string. The
//!   functions here are synchronous; the API layer runs them in `spawn_blocking` behind a semaphore of [`HASH_CONCURRENCY`].
//! - Sessions: 32 random bytes from the operating system, base64url (43 characters) in the cookie [`SESSION_COOKIE`]; the
//!   database stores only the SHA-256 (hex) of that cookie value, so a copy of the database does not contain usable tokens.
//! - Known browsers: a successful sign-in (and a password change) also sets the cookie [`DEVICE_COOKIE`], an HMAC of the user id
//!   keyed with the user's current password hash. A sign-in that presents it is throttled in its own bucket (same escalating
//!   rounds, [`throttle`]), so somebody who keeps guessing a username cannot lock its owner out of a browser they signed in with
//!   before. A new password invalidates
//!   every such cookie; nothing is stored for it.
//! - There is deliberately no switch (environment variable, flag, config file) that turns sign-in off.
use crate::security::base64url_nopad;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::sync::OnceLock;

pub mod throttle;

pub use throttle::{
    LockStatus, LoginThrottle, Target, ThrottleClock, ThrottleLimits, REFUSAL_REPORT_EVERY,
};

/// Name of the session cookie.
pub const SESSION_COOKIE: &str = "xexam_session";
/// Name of the cookie that marks a browser where the user signed in before (see [`device_tag`]).
pub const DEVICE_COOKIE: &str = "xexam_device";
/// Lifetime of the known-browser cookie: 400 days (the longest browsers keep a cookie).
pub const DEVICE_TTL_SECS: i64 = 400 * 24 * 3600;
/// Legacy (v1.2): session lifetime, 30 days. Still used by the old `Store::session_user` / `start_session`, which the v1.3
/// auth layer no longer calls; the store owner removes them together with this constant. Use [`USER_REMEMBER_SECS`].
pub const SESSION_TTL_SECS: i64 = 30 * 24 * 3600;
/// Legacy (v1.2), see [`SESSION_TTL_SECS`]. Use [`SESSION_COOKIE_RESEND_SECS`].
pub const SESSION_REFRESH_SECS: i64 = 3600;
/// v1.3: an administrator session ends after this long without activity (24 h)...
pub const ADMIN_IDLE_SECS: i64 = 86_400;
/// ...and this long after sign-in whatever the activity (7 days).
pub const ADMIN_HARD_SECS: i64 = 604_800;
/// v1.3: a user session with "keep me signed in": sliding idle limit, 30 days (also the cookie `Max-Age`).
pub const USER_REMEMBER_SECS: i64 = 2_592_000;
/// v1.3: a user session without it: idle limit, 12 h (the cookie is a browser-session cookie).
pub const USER_SESSION_IDLE_SECS: i64 = 43_200;
/// v1.3: a session row (and `users.last_active_at`) is written at most once per this many seconds per session.
pub const SESSION_TOUCH_SECS: i64 = 60;
/// v1.3: the session cookie of a "remember" session is re-sent at most this often.
pub const SESSION_COOKIE_RESEND_SECS: i64 = 3600;
/// Expired sessions are purged at startup and then at most once per this many seconds.
pub const SESSION_PURGE_SECS: i64 = 3600;
/// At most this many sessions per user; a new sign-in beyond it ends the sessions that would expire first.
pub const MAX_SESSIONS_PER_USER: i64 = 20;
/// At most this many Argon2 hash / verify operations run at the same time (the rest wait).
pub const HASH_CONCURRENCY: usize = 2;
/// Length of a session token in the cookie (32 bytes, base64url without padding).
pub const TOKEN_LEN: usize = 43;

pub const PASSWORD_MIN_CHARS: usize = 8;
pub const PASSWORD_MAX_CHARS: usize = 128;
pub const USERNAME_MAX_CHARS: usize = 40;

/// The administrator created when the database has no users (the owner's requirement); `must_change_password` is set.
pub const DEFAULT_ADMIN_USERNAME: &str = "admin";
pub const DEFAULT_ADMIN_PASSWORD: &str = "admin";

pub const ROLE_ADMIN: &str = "admin";
pub const ROLE_USER: &str = "user";

/// A role as stored in `users.role`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Admin,
    User,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => ROLE_ADMIN,
            Role::User => ROLE_USER,
        }
    }
    /// Exactly `admin` or `user` (the CHECK constraint of the table); anything else is `None`.
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            ROLE_ADMIN => Some(Role::Admin),
            ROLE_USER => Some(Role::User),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------- session lifetime (v1.3)

/// The stored times of a session, unix seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionTimes {
    /// The sign-in; a session without one (made by a v1.2 binary) passes its `last_seen_at`
    pub started_at: i64,
    pub last_seen_at: i64,
    pub remember: bool,
}

/// Why a session ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    Idle,
    Hard,
}

impl Ended {
    pub fn as_str(self) -> &'static str {
        match self {
            Ended::Idle => "idle",
            Ended::Hard => "hard",
        }
    }
}

/// Which lifetime rule a session follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    Admin,
    Remember,
    Browser,
}

impl Rule {
    pub fn as_str(self) -> &'static str {
        match self {
            Rule::Admin => "admin",
            Rule::Remember => "remember",
            Rule::Browser => "browser",
        }
    }
    /// The idle limit of the rule, seconds (`session.idleSecs`).
    pub fn idle_secs(self) -> i64 {
        match self {
            Rule::Admin => ADMIN_IDLE_SECS,
            Rule::Remember => USER_REMEMBER_SECS,
            Rule::Browser => USER_SESSION_IDLE_SECS,
        }
    }
}

/// The rule for a user of `role`; `remember` is the box ticked at sign-in (ignored for administrators).
pub fn session_rule(role: Role, remember: bool) -> Rule {
    match (role, remember) {
        (Role::Admin, _) => Rule::Admin,
        (Role::User, true) => Rule::Remember,
        (Role::User, false) => Rule::Browser,
    }
}

/// The only place that knows how long a session lives. The first second (unix) at which the session at its current
/// `last_seen_at` counts as ended, and why. Follows the user's CURRENT role, so a promotion or demotion applies at the next
/// request. An administrator's hard limit wins a tie with the idle limit.
pub fn session_deadline(role: Role, t: &SessionTimes) -> (i64, Ended) {
    let idle = t
        .last_seen_at
        .saturating_add(session_rule(role, t.remember).idle_secs());
    if role == Role::Admin {
        let hard = t.started_at.saturating_add(ADMIN_HARD_SECS);
        if hard <= idle {
            return (hard, Ended::Hard);
        }
    }
    (idle, Ended::Idle)
}

/// The rule and the end of the hard limit (unix seconds; administrators only) of a session, as `session` in the answers.
pub fn session_info(role: Role, t: &SessionTimes) -> (Rule, Option<i64>) {
    let rule = session_rule(role, t.remember);
    let hard = (rule == Rule::Admin).then(|| t.started_at.saturating_add(ADMIN_HARD_SECS));
    (rule, hard)
}

/// What one request should write ([`SessionGates::plan`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchPlan {
    pub session: bool,
    pub active: bool,
}

#[derive(Default)]
struct GateInner {
    /// session token hash -> (last touch, last cookie send); `i64::MIN` = never
    sessions: std::collections::HashMap<String, (i64, i64)>,
    /// user id -> the minute (`now / 60`) of the last `last_active_at` write
    users: std::collections::HashMap<i64, i64>,
    /// session token hash -> when its end was claimed (so two requests that find it ended record it once)
    ending: std::collections::HashMap<String, i64>,
}

/// In-memory gates that keep an untouched request read-only: at most one session write per session per
/// [`SESSION_TOUCH_SECS`], one `last_active_at` write per user per minute, one cookie re-send per session per
/// [`SESSION_COOKIE_RESEND_SECS`]. Empty after a restart (the first request then writes once, which is harmless).
#[derive(Default)]
pub struct SessionGates(std::sync::Mutex<GateInner>);

impl SessionGates {
    fn lock(&self) -> std::sync::MutexGuard<'_, GateInner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Claims the writes of a request that counts as activity: what to write now (both false = stay read-only).
    pub fn plan(&self, token_hash: &str, user_id: i64, now: i64) -> TouchPlan {
        let mut g = self.lock();
        let e = g
            .sessions
            .entry(token_hash.to_string())
            .or_insert((i64::MIN, i64::MIN));
        let session = e.0 == i64::MIN || now.saturating_sub(e.0) >= SESSION_TOUCH_SECS;
        if session {
            e.0 = now;
        }
        let minute = now.div_euclid(60);
        let active = g.users.get(&user_id) != Some(&minute);
        if active {
            g.users.insert(user_id, minute);
        }
        TouchPlan { session, active }
    }

    /// Claims the cookie re-send of a "remember" session: true when none was sent in the last hour (or since the restart).
    pub fn cookie_due(&self, token_hash: &str, now: i64) -> bool {
        let mut g = self.lock();
        let e = g
            .sessions
            .entry(token_hash.to_string())
            .or_insert((i64::MIN, i64::MIN));
        let due = e.1 == i64::MIN || now.saturating_sub(e.1) >= SESSION_COOKIE_RESEND_SECS;
        if due {
            e.1 = now;
        }
        due
    }

    /// A session was just created: its touch and cookie are fresh.
    pub fn started(&self, token_hash: &str, user_id: i64, now: i64) {
        let mut g = self.lock();
        g.sessions.insert(token_hash.to_string(), (now, now));
        g.users.insert(user_id, now.div_euclid(60));
    }

    /// Claims the deletion and the log entry of an ended session: true for the first caller only.
    pub fn claim_end(&self, token_hash: &str, now: i64) -> bool {
        let mut g = self.lock();
        g.ending.retain(|_, t| now.saturating_sub(*t) < 120);
        g.ending.insert(token_hash.to_string(), now).is_none()
    }

    /// The session ended (sign-out, expiry, rotation).
    pub fn forget(&self, token_hash: &str) {
        self.lock().sessions.remove(token_hash);
    }

    /// Drops entries nobody needs any more: sessions untouched for 2 h (past the cookie gate), users older than 2 minutes.
    pub fn prune(&self, now: i64) {
        let mut g = self.lock();
        g.sessions
            .retain(|_, (t, c)| now.saturating_sub((*t).max(*c)) < 2 * SESSION_COOKIE_RESEND_SECS);
        let minute = now.div_euclid(60);
        g.users.retain(|_, m| minute.saturating_sub(*m) < 2);
    }

    /// Number of remembered sessions.
    pub fn len(&self) -> usize {
        self.lock().sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---------------------------------------------------------------- passwords

/// Argon2id PHC string of `password` (default parameters, random salt). Synchronous and CPU-heavy by design.
pub fn hash_password(password: &str) -> Result<String, String> {
    use argon2::password_hash::PasswordHasher;
    argon2::Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| format!("password hashing failed: {e}"))
}

/// Whether `password` matches the PHC string `phc`. A malformed stored hash never matches.
pub fn verify_password(password: &str, phc: &str) -> bool {
    use argon2::password_hash::phc::PasswordHash;
    use argon2::password_hash::PasswordVerifier;
    match PasswordHash::new(phc) {
        Ok(parsed) => argon2::Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// A valid hash of a random password nobody knows. Unknown usernames are verified against it, so a failed sign-in for an unknown
/// user costs the same time as one for a known user (the response never says whether the username exists).
pub fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| {
        let mut secret = [0u8; 32];
        fill_random(&mut secret);
        // A fixed fallback would still be a valid PHC string; it is only ever used for failing comparisons
        hash_password(&base64url_nopad(&secret)).unwrap_or_else(|_| {
            "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
                .to_string()
        })
    })
}

/// Password rule: 8–128 characters (code points), any characters.
pub fn password_ok(password: &str) -> bool {
    (PASSWORD_MIN_CHARS..=PASSWORD_MAX_CHARS).contains(&password.chars().count())
}

/// Username rule: trimmed, 1–40 characters, ASCII letters, digits and `. _ - @`. Returns the trimmed name, or `None` when it
/// breaks the rule. ASCII only on purpose: uniqueness is case-insensitive through SQLite's `COLLATE NOCASE`, which folds ASCII
/// letters only, and it rules out look-alike names.
pub fn clean_username(raw: &str) -> Option<String> {
    let name = raw.trim();
    let n = name.chars().count();
    let ok = (1..=USERNAME_MAX_CHARS).contains(&n)
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@'));
    ok.then(|| name.to_string())
}

/// A random password for `reset-password` (16 characters from an unambiguous alphabet).
pub fn random_password() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut out = String::with_capacity(16);
    while out.len() < 16 {
        let mut buf = [0u8; 32];
        fill_random(&mut buf);
        // Rejection sampling keeps every character equally likely
        let limit = 256 - (256 % ALPHABET.len());
        for b in buf {
            if out.len() == 16 {
                break;
            }
            if usize::from(b) < limit {
                if let Some(&c) = ALPHABET.get(usize::from(b) % ALPHABET.len()) {
                    out.push(char::from(c));
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------- session tokens and the cookie

fn fill_random(buf: &mut [u8]) {
    if getrandom::fill(buf).is_err() {
        // Never fall back to predictable values when the operating system's randomness is unavailable
        panic!("operating system random source is unavailable");
    }
}

/// A new session token: 32 random bytes, base64url without padding (43 characters). Every sign-in gets a new one.
pub fn new_session_token() -> String {
    let mut buf = [0u8; 32];
    fill_random(&mut buf);
    base64url_nopad(&buf)
}

/// What the database stores for a token: the SHA-256 of the cookie value, as 64 lower-case hex digits.
pub fn token_hash(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut out = String::with_capacity(64);
    for b in digest.iter() {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Whether `v` looks like a token this server issued (exactly 43 base64url characters). Anything else is never looked up.
fn token_shaped(v: &str) -> bool {
    v.len() == TOKEN_LEN
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The first value of cookie `cookie` in the `Cookie` header(s) that has the shape of a token.
fn token_cookie(headers: &axum::http::HeaderMap, cookie: &str) -> Option<String> {
    for value in headers.get_all(axum::http::header::COOKIE) {
        let Ok(text) = value.to_str() else { continue };
        for pair in text.split(';') {
            let Some((name, v)) = pair.split_once('=') else {
                continue;
            };
            if name.trim() == cookie {
                let v = v.trim().trim_matches('"');
                if token_shaped(v) {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// The session token from the `Cookie` header(s): the first `xexam_session` value that has the shape of a token.
pub fn session_token(headers: &axum::http::HeaderMap) -> Option<String> {
    token_cookie(headers, SESSION_COOKIE)
}

/// The session token of an AI request (v0.8, the auth step `GET /internal/ai/auth`), read so that it is always the key
/// Nginx gives the per-session limit on starting AI answers (`limit_req_zone $cookie_xexam_session`). Nginx takes the
/// first cookie whose name matches in any letter case (a new pair starts after a `;` or a `,`, then spaces; the value
/// runs to the next `;`), with its value as written, and does not count an empty value; so an empty `xexam_session=` in
/// front, a second cookie of that name in another case or behind a comma or a tab, or a padded or quoted value would put
/// a signed-in request under another key, or under none. Therefore:
/// - exactly one cookie may be named `xexam_session` in any letter case, counted over every `Cookie` header, with pairs
///   split at `;` and also at `,`, names trimmed of whitespace and headers read as bytes (a header that is not ASCII is
///   counted too);
/// - that cookie must be written `xexam_session=<token>` exactly: lower-case name, `=` right after it, a value with the
///   shape of a token and nothing else up to the next `;` (only spaces may come before the name).
///
/// Anything else is no session. Browsers send one `xexam_session=<token>` pair, so only hand-made requests are refused.
/// The other routes keep reading the cookie with [`session_token`] (no limit is keyed on it there).
pub fn single_session_token(headers: &axum::http::HeaderMap) -> Option<String> {
    let name = SESSION_COOKIE.as_bytes();
    let mut named = 0usize;
    let mut found = None;
    for value in headers.get_all(axum::http::header::COOKIE) {
        for pair in value.as_bytes().split(|&b| b == b';') {
            named += pair
                .split(|&b| b == b',')
                .filter(|part| {
                    part.iter()
                        .position(|&b| b == b'=')
                        .is_some_and(|i| part[..i].trim_ascii().eq_ignore_ascii_case(name))
                })
                .count();
            let start = pair.iter().position(|&b| b != b' ').unwrap_or(pair.len());
            let token = pair[start..]
                .strip_prefix(name)
                .and_then(|rest| rest.strip_prefix(b"="))
                .and_then(|v| std::str::from_utf8(v).ok())
                .filter(|v| token_shaped(v));
            if let Some(t) = token {
                found = Some(t.to_string());
            }
        }
    }
    if named == 1 {
        found
    } else {
        None
    }
}

/// HMAC-SHA-256 (RFC 2104). Also signs the identity statement for the AI service (`api::internal`, v0.8).
pub(crate) fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > block.len() {
        let digest = Sha256::digest(key);
        block[..digest.len()].copy_from_slice(&digest);
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|b| b ^ 0x36));
    inner.update(msg);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(block.map(|b| b ^ 0x5c));
    outer.update(inner);
    let mut out = [0u8; 32];
    out.copy_from_slice(&outer.finalize());
    out
}

/// The known-browser value for user `user_id` whose password hash is `password_hash`: HMAC-SHA-256 keyed with the stored hash,
/// base64url (43 characters). It changes whenever the password changes, so a new password retires every old value.
pub fn device_tag(user_id: i64, password_hash: &str) -> String {
    base64url_nopad(&hmac_sha256(
        password_hash.as_bytes(),
        format!("xexam-device\0{user_id}").as_bytes(),
    ))
}

/// Whether the request carries the known-browser cookie of this user (and of their current password).
pub fn device_known(headers: &axum::http::HeaderMap, user_id: i64, password_hash: &str) -> bool {
    use subtle::ConstantTimeEq;
    token_cookie(headers, DEVICE_COOKIE).is_some_and(|v| {
        v.as_bytes()
            .ct_eq(device_tag(user_id, password_hash).as_bytes())
            .into()
    })
}

/// `Set-Cookie` value of the known-browser cookie. Sent only to the sign-in and password routes (`Path=/api/auth`).
pub fn device_cookie(tag: &str, secure: bool) -> String {
    format!(
        "{DEVICE_COOKIE}={tag}; HttpOnly; SameSite=Strict; Path=/api/auth; Max-Age={DEVICE_TTL_SECS}{}",
        if secure { "; Secure" } else { "" }
    )
}

/// Whether the cookie gets `Secure`: the request's `Origin` is `https://…`, or `XEXAM_COOKIE_SECURE=1`. Plain HTTP (the
/// default LAN deployment) must keep working, so `Secure` is never set just because.
pub fn cookie_secure(force: bool, origin: Option<&str>) -> bool {
    force
        || origin.is_some_and(|o| {
            o.trim()
                .get(..8)
                .is_some_and(|p| p.eq_ignore_ascii_case("https://"))
        })
}

/// `Set-Cookie` value for a session token of a "remember" session (30 days). See [`session_cookie_for`].
pub fn session_cookie(token: &str, secure: bool) -> String {
    session_cookie_for(token, secure, Rule::Remember)
}

/// `Set-Cookie` value for a session token under `rule`: administrator `Max-Age=604800` (set once at sign-in), remember
/// `Max-Age=2592000`, browser session: neither `Max-Age` nor `Expires`.
pub fn session_cookie_for(token: &str, secure: bool, rule: Rule) -> String {
    let age = match rule {
        Rule::Admin => format!("; Max-Age={ADMIN_HARD_SECS}"),
        Rule::Remember => format!("; Max-Age={USER_REMEMBER_SECS}"),
        Rule::Browser => String::new(),
    };
    format!(
        "{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/{age}{}",
        if secure { "; Secure" } else { "" }
    )
}

/// `Set-Cookie` value that removes the session cookie.
pub fn expired_cookie(secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    )
}

/// Seconds since the Unix epoch (sessions use unix seconds; everything else uses the local `%Y-%m-%d %H:%M` strings).
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn hashes_verify_and_are_salted() {
        let a = hash_password("correct horse").expect("hash");
        let b = hash_password("correct horse").expect("hash");
        assert!(a.starts_with("$argon2id$"), "{a}");
        assert_ne!(a, b, "random salt per hash");
        assert!(verify_password("correct horse", &a));
        assert!(!verify_password("correct horsE", &a));
        assert!(!verify_password("x", "not a phc string"));
        assert!(!verify_password("x", dummy_hash()));
    }

    #[test]
    fn usernames_and_passwords() {
        assert_eq!(
            clean_username("  ann.lee@x_y-z "),
            Some("ann.lee@x_y-z".into())
        );
        assert_eq!(clean_username(&"a".repeat(40)), Some("a".repeat(40)));
        for bad in [
            "",
            "   ",
            "a b",
            "ann!",
            "一年",
            "é",
            &"a".repeat(41),
            "a\tb",
        ] {
            assert_eq!(clean_username(bad), None, "{bad:?}");
        }
        assert!(!password_ok("1234567"));
        assert!(password_ok("12345678"));
        assert!(password_ok("日本語のパスワード"));
        assert!(password_ok(&"x".repeat(128)));
        assert!(!password_ok(&"x".repeat(129)));
        let p = random_password();
        assert_eq!(p.chars().count(), 16);
        assert!(password_ok(&p));
        assert_ne!(p, random_password());
    }

    #[test]
    fn tokens_and_cookies() {
        let t = new_session_token();
        assert_eq!(t.len(), TOKEN_LEN);
        assert_ne!(t, new_session_token());
        let h = token_hash(&t);
        assert_eq!(h.len(), 64);
        assert_ne!(h, t);
        let mut headers = HeaderMap::new();
        headers.append(
            "cookie",
            HeaderValue::from_str(&format!("a=1; {SESSION_COOKIE}=short; b=2")).expect("v"),
        );
        assert_eq!(
            session_token(&headers),
            None,
            "malformed values are ignored"
        );
        headers.append(
            "cookie",
            HeaderValue::from_str(&format!("{SESSION_COOKIE}={t}")).expect("v"),
        );
        assert_eq!(session_token(&headers), Some(t.clone()));
        assert!(session_cookie(&t, false).ends_with("Max-Age=2592000"));
        assert!(session_cookie(&t, true).ends_with("; Secure"));
        assert!(cookie_secure(false, Some("HTTPS://example.com")));
        assert!(!cookie_secure(false, Some("http://192.168.1.2:8765")));
        assert!(!cookie_secure(false, None));
        assert!(cookie_secure(true, None));
        assert_eq!(
            expired_cookie(false),
            "xexam_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"
        );
    }

    #[test]
    fn the_ai_auth_step_reads_exactly_one_session_cookie() {
        let t = new_session_token();
        let u = new_session_token();
        let of = |cookies: &[&[u8]]| {
            let mut headers = HeaderMap::new();
            for c in cookies {
                headers.append("cookie", HeaderValue::from_bytes(c).expect("v"));
            }
            single_session_token(&headers)
        };
        let s = |v: String| v.into_bytes();
        // one pair, also among other cookies and after extra spaces
        for ok in [
            s(format!("xexam_session={t}")),
            s(format!("theme=dark; xexam_session={t}; other=a,b")),
            s(format!("a=1;  xexam_session={t};b=2")),
            s(format!(
                "xexam_session_x=1; x_xexam_session=2; xexam_session={t}"
            )),
        ] {
            assert_eq!(
                of(&[&ok]),
                Some(t.clone()),
                "{}",
                String::from_utf8_lossy(&ok)
            );
        }
        assert_eq!(
            of(&[b"theme=dark", t.as_bytes()]),
            None,
            "no pair, no token"
        );
        assert_eq!(
            of(&[b"theme=dark", format!("xexam_session={t}").as_bytes()]),
            Some(t.clone())
        );
        // anything Nginx would key differently, or not at all
        for bad in [
            s(format!("xexam_session=; xexam_session={t}")),
            s(format!("xexam_session={t}; xexam_session=")),
            s(format!("XEXAM_SESSION=junk; xexam_session={t}")),
            s(format!("xexam_session={t}; Xexam_Session={u}")),
            s(format!("xexam_session={t}; xexam_session={t}")),
            s(format!("xexam_session={t},xexam_session=x")),
            s(format!("a=b, XEXAM_session=x; xexam_session={t}")),
            s(format!("xexam_session ={t}")),
            s(format!("xexam_session= {t}")),
            s(format!("xexam_session={t} ; a=1")),
            s(format!("xexam_session=\"{t}\"")),
            s(format!("\txexam_session={t}")),
            s(format!("XEXAM_SESSION={t}")),
            s(format!("xexam_session={t}x")),
            s("xexam_session=short".to_string()),
        ] {
            assert_eq!(of(&[&bad]), None, "{}", String::from_utf8_lossy(&bad));
        }
        // one pair per header, two headers: two cookies of that name
        assert_eq!(
            of(&[
                format!("xexam_session={t}").as_bytes(),
                format!("xexam_session={u}").as_bytes()
            ]),
            None
        );
        assert_eq!(
            of(&[
                b"xexam_session=\xff",
                format!("xexam_session={t}").as_bytes()
            ]),
            None,
            "a header that is not ASCII is counted too"
        );
        // bytes outside ASCII in another cookie do not matter
        assert_eq!(
            of(&[&[
                b"a=\xff; ".as_slice(),
                format!("xexam_session={t}").as_bytes()
            ]
            .concat()]),
            Some(t.clone())
        );
        // the other routes stay as lenient as before
        let mut headers = HeaderMap::new();
        headers.append(
            "cookie",
            HeaderValue::from_str(&format!("xexam_session=; xexam_session={t}")).expect("v"),
        );
        assert_eq!(session_token(&headers), Some(t));
    }

    #[test]
    fn hmac_matches_rfc_4231_and_device_tags_follow_the_password() {
        // RFC 4231, test case 2
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // RFC 4231, test case 6 (a key longer than the block)
        let mac = hmac_sha256(
            &[0xaa; 131],
            b"Test Using Larger Than Block-Size Key - Hash Key First",
        );
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
        let tag = device_tag(7, "$argon2id$one");
        assert_eq!(tag.len(), TOKEN_LEN);
        assert_ne!(tag, device_tag(8, "$argon2id$one"), "per user");
        assert_ne!(tag, device_tag(7, "$argon2id$two"), "per password");
        let mut headers = HeaderMap::new();
        headers.append(
            "cookie",
            HeaderValue::from_str(&format!("{DEVICE_COOKIE}={tag}")).expect("v"),
        );
        assert!(device_known(&headers, 7, "$argon2id$one"));
        assert!(!device_known(&headers, 7, "$argon2id$two"));
        assert!(!device_known(&headers, 8, "$argon2id$one"));
        assert!(!device_known(&HeaderMap::new(), 7, "$argon2id$one"));
        assert!(device_cookie(&tag, false).contains("; Path=/api/auth; "));
        assert!(device_cookie(&tag, true).ends_with("; Secure"));
    }

    fn times(started: i64, last: i64, remember: bool) -> SessionTimes {
        SessionTimes {
            started_at: started,
            last_seen_at: last,
            remember,
        }
    }

    #[test]
    fn deadline_rule_matrix() {
        // administrator: 24 h idle, 7 days hard, whatever `remember` says
        for remember in [true, false] {
            let t = times(1_000, 1_000, remember);
            assert_eq!(
                session_deadline(Role::Admin, &t),
                (1_000 + ADMIN_IDLE_SECS, Ended::Idle)
            );
            // active every minute for 7 days: the hard limit still wins
            let t = times(1_000, 1_000 + ADMIN_HARD_SECS - 60, remember);
            assert_eq!(
                session_deadline(Role::Admin, &t),
                (1_000 + ADMIN_HARD_SECS, Ended::Hard)
            );
        }
        // a tie goes to the hard limit
        let t = times(0, ADMIN_HARD_SECS - ADMIN_IDLE_SECS, true);
        assert_eq!(
            session_deadline(Role::Admin, &t),
            (ADMIN_HARD_SECS, Ended::Hard)
        );
        // user, remember: 30 days sliding, no hard limit
        let t = times(0, 100 * 86_400, true);
        assert_eq!(
            session_deadline(Role::User, &t),
            (100 * 86_400 + USER_REMEMBER_SECS, Ended::Idle)
        );
        // user, browser session: 12 h
        let t = times(0, 500, false);
        assert_eq!(
            session_deadline(Role::User, &t),
            (500 + USER_SESSION_IDLE_SECS, Ended::Idle)
        );
    }

    #[test]
    fn role_change_applies_at_the_next_lookup() {
        // a remember user active for 10 days, promoted to admin: 24 h idle and 7 days hard from the sign-in
        let t = times(0, 10 * 86_400, true);
        assert_eq!(session_deadline(Role::User, &t).1, Ended::Idle);
        assert_eq!(
            session_deadline(Role::Admin, &t),
            (ADMIN_HARD_SECS, Ended::Hard)
        );
        // a demoted administrator gets the user rule with the stored `remember`
        let t = times(0, 1_000, false);
        assert_eq!(
            session_deadline(Role::User, &t).0,
            1_000 + USER_SESSION_IDLE_SECS
        );
    }

    #[test]
    fn rule_info_and_names() {
        assert_eq!(session_rule(Role::Admin, false), Rule::Admin);
        assert_eq!(session_rule(Role::User, true), Rule::Remember);
        assert_eq!(session_rule(Role::User, false), Rule::Browser);
        let t = times(50, 60, true);
        assert_eq!(
            session_info(Role::Admin, &t),
            (Rule::Admin, Some(50 + ADMIN_HARD_SECS))
        );
        assert_eq!(session_info(Role::User, &t), (Rule::Remember, None));
        assert_eq!(Rule::Browser.idle_secs(), 43_200);
        assert_eq!(
            [
                Rule::Admin.as_str(),
                Rule::Remember.as_str(),
                Rule::Browser.as_str()
            ],
            ["admin", "remember", "browser"]
        );
        assert_eq!(
            [Ended::Idle.as_str(), Ended::Hard.as_str()],
            ["idle", "hard"]
        );
    }

    #[test]
    fn cookie_attributes_per_rule() {
        let c = session_cookie_for("tok", false, Rule::Admin);
        assert!(
            c.contains("; Max-Age=604800") && !c.contains("Secure"),
            "{c}"
        );
        let c = session_cookie_for("tok", true, Rule::Remember);
        assert!(
            c.contains("; Max-Age=2592000") && c.ends_with("; Secure"),
            "{c}"
        );
        let c = session_cookie_for("tok", false, Rule::Browser);
        assert!(!c.contains("Max-Age") && !c.contains("Expires"), "{c}");
        assert_eq!(
            session_cookie("tok", false),
            session_cookie_for("tok", false, Rule::Remember)
        );
    }

    #[test]
    fn gates_allow_one_write_per_minute() {
        let g = SessionGates::default();
        let mut sessions = 0;
        let mut actives = 0;
        for i in 0..100 {
            let p = g.plan("a", 1, 6_000 + i % 60);
            sessions += usize::from(p.session);
            actives += usize::from(p.active);
        }
        assert_eq!((sessions, actives), (1, 1), "100 requests in one minute");
        // the next minute writes again, once
        let p = g.plan("a", 1, 6_060);
        assert_eq!((p.session, p.active), (true, true));
        let p = g.plan("a", 1, 6_061);
        assert_eq!((p.session, p.active), (false, false));
        // two browsers of one user: two session writes, one user write
        let g = SessionGates::default();
        let a = g.plan("a", 7, 120);
        let b = g.plan("b", 7, 121);
        assert_eq!((a.session, a.active), (true, true));
        assert_eq!((b.session, b.active), (true, false));
        // a session is written again 60 s after its last write, not before
        assert!(g.plan("a", 7, 180).session);
        assert!(!g.plan("a", 7, 239).session);
        assert!(g.plan("a", 7, 240).session);
    }

    #[test]
    fn gates_cookie_resend_hourly_and_prune() {
        let g = SessionGates::default();
        assert!(g.cookie_due("a", 10_000), "once after a restart");
        assert!(!g.cookie_due("a", 10_000 + SESSION_COOKIE_RESEND_SECS - 1));
        assert!(g.cookie_due("a", 10_000 + SESSION_COOKIE_RESEND_SECS));
        g.started("b", 2, 20_000);
        assert!(
            !g.cookie_due("b", 20_100),
            "a fresh sign-in just sent its cookie"
        );
        g.forget("b");
        assert!(g.cookie_due("b", 20_200));
        g.prune(100_000);
        assert!(g.is_empty(), "idle entries are pruned");
        g.plan("c", 3, 200_000);
        g.prune(200_010);
        assert_eq!(g.len(), 1);
    }
}
