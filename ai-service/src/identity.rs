//! The identity statement (`X-Xexam-Identity`, spec 5.4.2): who is asking, signed by the main backend.
//!
//! `header = base64url(payload) + "." + base64url(HMAC-SHA256(internal token, "xexam-identity-v1." + base64url(payload)))`.
//! Nginx (or the vite plugin) asks the backend for it on every request and overwrites any value the client sent. The AI
//! service never reads identity from anything else. The statement is never logged.
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ring::hmac;
use serde::Deserialize;
use std::fmt;

/// Longest accepted header value.
pub const MAX_LEN: usize = 2048;
/// Prefix of the signed message.
pub const SIGN_PREFIX: &str = "xexam-identity-v1.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Admin,
    User,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::User => "user",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "admin" => Some(Role::Admin),
            "user" => Some(Role::User),
            _ => None,
        }
    }
}

/// A verified identity.
#[derive(Clone, PartialEq, Eq)]
pub struct Identity {
    pub id: i64,
    /// `users.uid` of the main database (32 lower-case hex digits)
    pub stamp: String,
    pub username: String,
    pub role: Role,
    /// AI+ (always true for administrators)
    pub ai_allowed: bool,
    /// The session's token hash, used only for the re-check while an answer runs (kept in memory)
    pub sid: String,
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("id", &self.id)
            .field("username", &self.username)
            .field("role", &self.role)
            .field("ai_allowed", &self.ai_allowed)
            .finish_non_exhaustive()
    }
}

impl Identity {
    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }
}

/// Why a statement was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyError {
    /// No header
    Missing,
    /// The signature does not verify with the current token (may mean the token was rotated)
    BadSignature,
    /// Wrong shape, wrong field, or expired
    Invalid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    v: i64,
    id: i64,
    stamp: String,
    username: String,
    role: String,
    #[serde(rename = "aiAllowed")]
    ai_allowed: bool,
    sid: String,
    exp: i64,
}

/// Verifies a statement against `token` at time `now` (unix seconds).
pub fn verify(header: Option<&[u8]>, token: &[u8], now: i64) -> Result<Identity, VerifyError> {
    let raw = header.ok_or(VerifyError::Missing)?;
    if raw.is_empty() {
        return Err(VerifyError::Missing);
    }
    if raw.len() > MAX_LEN || !raw.is_ascii() {
        return Err(VerifyError::Invalid);
    }
    let text = std::str::from_utf8(raw).map_err(|_| VerifyError::Invalid)?;
    if text.matches('.').count() != 1 {
        return Err(VerifyError::Invalid);
    }
    let (p64, s64) = text.split_once('.').ok_or(VerifyError::Invalid)?;
    let sig = URL_SAFE_NO_PAD
        .decode(s64)
        .map_err(|_| VerifyError::Invalid)?;
    let key = hmac::Key::new(hmac::HMAC_SHA256, token);
    let msg = format!("{SIGN_PREFIX}{p64}");
    hmac::verify(&key, msg.as_bytes(), &sig).map_err(|_| VerifyError::BadSignature)?;
    let payload = URL_SAFE_NO_PAD
        .decode(p64)
        .map_err(|_| VerifyError::Invalid)?;
    let p: Payload = serde_json::from_slice(&payload).map_err(|_| VerifyError::Invalid)?;
    let role = Role::parse(&p.role).ok_or(VerifyError::Invalid)?;
    let name_len = p.username.chars().count();
    let ok = p.v == 1
        && now <= p.exp
        && p.exp <= now.saturating_add(120)
        && p.id > 0
        && crate::util::is_lower_hex(&p.stamp, 32)
        && crate::util::is_lower_hex(&p.sid, 64)
        && (1..=80).contains(&name_len);
    if !ok {
        return Err(VerifyError::Invalid);
    }
    Ok(Identity {
        id: p.id,
        stamp: p.stamp,
        username: p.username,
        role,
        ai_allowed: p.ai_allowed,
        sid: p.sid,
    })
}

/// Signs a payload the way the backend does (used by tests and the documentation of the format).
pub fn sign(payload_json: &str, token: &[u8]) -> String {
    let p64 = URL_SAFE_NO_PAD.encode(payload_json.as_bytes());
    let key = hmac::Key::new(hmac::HMAC_SHA256, token);
    let tag = hmac::sign(&key, format!("{SIGN_PREFIX}{p64}").as_bytes());
    format!("{p64}.{}", URL_SAFE_NO_PAD.encode(tag.as_ref()))
}

/// What the proxy told us about the caller (v1.3): the connecting address and the browser, as Nginx set them. Used only
/// for the activity entries of administrator actions; never for access decisions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientInfo {
    /// `X-Real-IP`, only when it parses as an address
    pub ip: Option<String>,
    /// `User-Agent` without control characters, at most 256 characters
    pub ua: Option<String>,
}

impl ClientInfo {
    pub fn from_headers(h: &axum::http::HeaderMap) -> ClientInfo {
        let ip = h
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<std::net::IpAddr>().ok())
            .map(|ip| ip.to_string());
        let ua = h
            .get(axum::http::header::USER_AGENT)
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .map(|v| {
                let clean: String = v.chars().filter(|c| !c.is_control()).collect();
                crate::util::cut_chars(clean.trim(), 256)
            })
            .filter(|v| !v.is_empty());
        ClientInfo { ip, ua }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &[u8] = b"fake-internal-token-for-tests-0123456789";
    const STAMP: &str = "0123456789abcdef0123456789abcdef";

    fn payload(exp: i64) -> String {
        format!(
            r#"{{"v":1,"id":7,"stamp":"{STAMP}","username":"sam","role":"user","aiAllowed":true,"sid":"{}","exp":{exp}}}"#,
            "ab".repeat(32)
        )
    }

    #[test]
    fn a_valid_statement_verifies() {
        let now = 1_700_000_000;
        let h = sign(&payload(now + 60), TOKEN);
        let id = verify(Some(h.as_bytes()), TOKEN, now).expect("valid");
        assert_eq!(id.id, 7);
        assert_eq!(id.username, "sam");
        assert_eq!(id.role, Role::User);
        assert!(id.ai_allowed);
        assert!(!format!("{id:?}").contains(&"ab".repeat(32)));
    }

    #[test]
    fn refusals() {
        let now = 1_700_000_000;
        let good = sign(&payload(now + 60), TOKEN);
        assert_eq!(verify(None, TOKEN, now), Err(VerifyError::Missing));
        assert_eq!(
            verify(
                Some(good.as_bytes()),
                b"another-token-0000000000000000000000",
                now
            ),
            Err(VerifyError::BadSignature)
        );
        // Expired, and too far ahead
        let old = sign(&payload(now - 1), TOKEN);
        assert_eq!(
            verify(Some(old.as_bytes()), TOKEN, now),
            Err(VerifyError::Invalid)
        );
        let ahead = sign(&payload(now + 121), TOKEN);
        assert_eq!(
            verify(Some(ahead.as_bytes()), TOKEN, now),
            Err(VerifyError::Invalid)
        );
        // Unknown field, wrong role, bad stamp, bad sid, wrong version, empty name
        for p in [
            payload(now + 60).replace("\"v\":1,", "\"v\":1,\"x\":1,"),
            payload(now + 60).replace("\"user\"", "\"owner\""),
            payload(now + 60).replace(STAMP, "0123"),
            payload(now + 60).replace(&"ab".repeat(32), &"AB".repeat(32)),
            payload(now + 60).replace("\"v\":1", "\"v\":2"),
            payload(now + 60).replace("\"sam\"", "\"\""),
            payload(now + 60).replace("\"id\":7", "\"id\":0"),
            payload(now + 60).replace("\"aiAllowed\":true", "\"aiAllowed\":1"),
        ] {
            let h = sign(&p, TOKEN);
            assert_eq!(
                verify(Some(h.as_bytes()), TOKEN, now),
                Err(VerifyError::Invalid),
                "{p}"
            );
        }
        // Shape problems
        assert_eq!(verify(Some(b"abc"), TOKEN, now), Err(VerifyError::Invalid));
        assert_eq!(
            verify(Some(b"a.b.c"), TOKEN, now),
            Err(VerifyError::Invalid)
        );
        let long = "a".repeat(MAX_LEN + 1);
        assert_eq!(
            verify(Some(long.as_bytes()), TOKEN, now),
            Err(VerifyError::Invalid)
        );
    }
    #[test]
    fn client_info_reads_only_a_valid_address_and_a_clean_agent() {
        use axum::http::{HeaderMap, HeaderValue};
        let mut h = HeaderMap::new();
        assert_eq!(ClientInfo::from_headers(&h), ClientInfo::default());
        h.insert("x-real-ip", HeaderValue::from_static("203.0.113.7"));
        h.insert("user-agent", HeaderValue::from_static("Demo/1.0 (made up)"));
        let c = ClientInfo::from_headers(&h);
        assert_eq!(c.ip.as_deref(), Some("203.0.113.7"));
        assert_eq!(c.ua.as_deref(), Some("Demo/1.0 (made up)"));
        h.insert("x-real-ip", HeaderValue::from_static("2001:db8::1"));
        assert_eq!(
            ClientInfo::from_headers(&h).ip.as_deref(),
            Some("2001:db8::1")
        );
        for bad in [
            "not an address",
            "203.0.113.7, 198.51.100.1",
            "203.0.113.7:80",
            "",
        ] {
            h.insert("x-real-ip", HeaderValue::from_str(bad).expect("value"));
            assert_eq!(ClientInfo::from_headers(&h).ip, None, "{bad:?}");
        }
        // control characters are removed, the agent is cut at 256 characters, an empty agent is none
        h.insert(
            "user-agent",
            HeaderValue::from_bytes(b"A	B	C").expect("value"),
        );
        assert_eq!(ClientInfo::from_headers(&h).ua.as_deref(), Some("ABC"));
        h.insert(
            "user-agent",
            HeaderValue::from_str(&"u".repeat(400)).expect("value"),
        );
        assert_eq!(
            ClientInfo::from_headers(&h).ua.map(|u| u.chars().count()),
            Some(256)
        );
        h.insert("user-agent", HeaderValue::from_static("  "));
        assert_eq!(ClientInfo::from_headers(&h).ua, None);
    }
}
