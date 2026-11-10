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

pub use throttle::{LockStatus, LoginThrottle, ThrottleClock, ThrottleLimits};

/// Name of the session cookie.
pub const SESSION_COOKIE: &str = "xexam_session";
/// Name of the cookie that marks a browser where the user signed in before (see [`device_tag`]).
pub const DEVICE_COOKIE: &str = "xexam_device";
/// Lifetime of the known-browser cookie: 400 days (the longest browsers keep a cookie).
pub const DEVICE_TTL_SECS: i64 = 400 * 24 * 3600;
/// Session lifetime (sliding): 30 days.
pub const SESSION_TTL_SECS: i64 = 30 * 24 * 3600;
/// `last_seen_at` / `expires_at` of a session are written at most once per this many seconds.
pub const SESSION_REFRESH_SECS: i64 = 3600;
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

/// HMAC-SHA-256 (RFC 2104).
fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
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

/// `Set-Cookie` value for a session token.
pub fn session_cookie(token: &str, secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={SESSION_TTL_SECS}{}",
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
            "日本",
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
}
