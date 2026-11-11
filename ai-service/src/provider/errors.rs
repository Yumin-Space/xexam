//! Upstream error codes (4.8). Upstream text is never echoed or logged: only a status and a type / code string are read.
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UpCode {
    Auth,
    Quota,
    NotFound,
    RateLimit,
    Overloaded,
    Server,
    BadRequest,
    Timeout,
    Connect,
    Protocol,
    Refused,
    TooLong,
}

impl UpCode {
    /// The full code, e.g. `ai.up.rateLimit`.
    pub fn code(self) -> &'static str {
        match self {
            UpCode::Auth => "ai.up.auth",
            UpCode::Quota => "ai.up.quota",
            UpCode::NotFound => "ai.up.notFound",
            UpCode::RateLimit => "ai.up.rateLimit",
            UpCode::Overloaded => "ai.up.overloaded",
            UpCode::Server => "ai.up.server",
            UpCode::BadRequest => "ai.up.badRequest",
            UpCode::Timeout => "ai.up.timeout",
            UpCode::Connect => "ai.up.connect",
            UpCode::Protocol => "ai.up.protocol",
            UpCode::Refused => "ai.up.refused",
            UpCode::TooLong => "ai.up.tooLong",
        }
    }

    /// The code without `ai.up.` (stored in `usage.error_code`).
    pub fn short(self) -> &'static str {
        self.code().trim_start_matches("ai.up.")
    }

    /// Configuration-class codes clear the verified state when a test fails (2.3 rule 2); the others are transient.
    pub fn is_config(self) -> bool {
        matches!(
            self,
            UpCode::Auth
                | UpCode::Quota
                | UpCode::NotFound
                | UpCode::BadRequest
                | UpCode::Protocol
                | UpCode::Refused
        )
    }
}

/// An upstream failure: the code and the HTTP status when there was one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpError {
    pub code: UpCode,
    pub http: Option<u16>,
}

impl UpError {
    pub fn new(code: UpCode, http: Option<u16>) -> UpError {
        UpError { code, http }
    }
}

/// Maps an error type or code string found in a body.
pub fn from_type(s: &str) -> Option<UpCode> {
    Some(match s {
        "authentication_error" | "permission_error" | "invalid_api_key" => UpCode::Auth,
        "billing_error" | "insufficient_quota" | "credit_balance_exhausted" => UpCode::Quota,
        "not_found_error" | "model_not_found" => UpCode::NotFound,
        "rate_limit_error" => UpCode::RateLimit,
        "overloaded_error" | "server_is_overloaded" | "service_unavailable_error" => {
            UpCode::Overloaded
        }
        "api_error" => UpCode::Server,
        "invalid_request_error" => UpCode::BadRequest,
        _ => return None,
    })
}

/// Maps an HTTP status.
pub fn from_status(status: u16) -> UpCode {
    match status {
        300..=399 => UpCode::Protocol,
        401 | 403 => UpCode::Auth,
        402 => UpCode::Quota,
        404 => UpCode::NotFound,
        429 => UpCode::RateLimit,
        503 | 529 => UpCode::Overloaded,
        500..=599 => UpCode::Server,
        400..=499 => UpCode::BadRequest,
        _ => UpCode::Protocol,
    }
}

/// The type / code of an error object: `error.code` first, then `error.type` (OpenAI and Anthropic shapes), then a
/// top-level `type` that is not the envelope's `"error"`. When none of these strings is known, an integer `error.code`
/// from 300 to 599 (OpenRouter's shape, an HTTP status) is mapped like that status.
pub fn from_error_value(v: &Value) -> Option<UpCode> {
    let err = v.get("error");
    let candidates = [
        err.and_then(|e| e.get("code")).and_then(Value::as_str),
        err.and_then(|e| e.get("type")).and_then(Value::as_str),
        err.and_then(Value::as_str),
        v.get("type")
            .and_then(Value::as_str)
            .filter(|t| *t != "error"),
    ];
    candidates
        .into_iter()
        .flatten()
        .find_map(from_type)
        .or_else(|| {
            err.and_then(|e| e.get("code"))
                .and_then(Value::as_u64)
                .and_then(|c| u16::try_from(c).ok())
                .filter(|c| (300..=599).contains(c))
                .map(from_status)
        })
}

/// Maps an error answer: the type / code in the body first (when parsable), else the status.
pub fn from_body(status: u16, body: &[u8]) -> UpCode {
    if status / 100 == 3 {
        return UpCode::Protocol;
    }
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| from_error_value(&v))
        .unwrap_or_else(|| from_status(status))
}

/// Maps a transport error of reqwest (only its kind is looked at).
pub fn from_reqwest(e: &reqwest::Error) -> UpCode {
    if e.is_timeout() && !e.is_connect() {
        UpCode::Timeout
    } else if e.is_connect() {
        UpCode::Connect
    } else if e.is_redirect() || e.is_decode() {
        UpCode::Protocol
    } else {
        UpCode::Connect
    }
}

/// The kind of a reqwest error for a log line (never its text or URL).
pub fn reqwest_kind(e: &reqwest::Error) -> &'static str {
    if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connect"
    } else if e.is_body() {
        "body"
    } else if e.is_decode() {
        "decode"
    } else {
        "request"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table() {
        assert_eq!(from_status(401), UpCode::Auth);
        assert_eq!(from_status(403), UpCode::Auth);
        assert_eq!(from_status(402), UpCode::Quota);
        assert_eq!(from_status(404), UpCode::NotFound);
        assert_eq!(from_status(429), UpCode::RateLimit);
        assert_eq!(from_status(529), UpCode::Overloaded);
        assert_eq!(from_status(503), UpCode::Overloaded);
        assert_eq!(from_status(500), UpCode::Server);
        assert_eq!(from_status(502), UpCode::Server);
        for s in [400, 409, 413, 422] {
            assert_eq!(from_status(s), UpCode::BadRequest);
        }
        assert_eq!(from_status(302), UpCode::Protocol);
        // the body wins over the status
        assert_eq!(
            from_body(
                400,
                br#"{"error":{"type":"invalid_request_error","code":"invalid_api_key"}}"#
            ),
            UpCode::Auth
        );
        assert_eq!(
            from_body(
                400,
                br#"{"type":"error","error":{"type":"overloaded_error"}}"#
            ),
            UpCode::Overloaded
        );
        assert_eq!(
            from_body(429, br#"{"error":{"code":"insufficient_quota"}}"#),
            UpCode::Quota
        );
        assert_eq!(
            from_body(404, br#"{"error":{"code":"model_not_found"}}"#),
            UpCode::NotFound
        );
        assert_eq!(
            from_body(500, br#"{"error":{"type":"api_error"}}"#),
            UpCode::Server
        );
        assert_eq!(
            from_body(402, br#"{"error":{"type":"credit_balance_exhausted"}}"#),
            UpCode::Quota
        );
        assert_eq!(from_body(400, b"not json"), UpCode::BadRequest);
        assert_eq!(
            from_body(401, br#"{"error":{"type":"unknown"}}"#),
            UpCode::Auth
        );
        assert_eq!(
            from_body(301, br#"{"error":{"type":"api_error"}}"#),
            UpCode::Protocol
        );
        // an integer error.code is an HTTP status (OpenRouter), also inside a 200 stream
        let num = |b: &str| from_error_value(&serde_json::from_str(b).expect("json"));
        assert_eq!(
            num(r#"{"error":{"code":429,"message":"m"}}"#),
            Some(UpCode::RateLimit)
        );
        assert_eq!(
            num(r#"{"error":{"code":402,"message":"m"}}"#),
            Some(UpCode::Quota)
        );
        assert_eq!(
            num(r#"{"error":{"code":401,"message":"m"}}"#),
            Some(UpCode::Auth)
        );
        assert_eq!(num(r#"{"error":{"code":503}}"#), Some(UpCode::Overloaded));
        assert_eq!(num(r#"{"error":{"code":302}}"#), Some(UpCode::Protocol));
        // outside 300..=599, negative or not an integer: unknown
        for b in [
            r#"{"error":{"code":200}}"#,
            r#"{"error":{"code":600}}"#,
            r#"{"error":{"code":70000}}"#,
            r#"{"error":{"code":-429}}"#,
            r#"{"error":{"code":429.5}}"#,
        ] {
            assert_eq!(num(b), None, "{b}");
        }
        // a known string code or type keeps precedence over the number
        assert_eq!(
            num(r#"{"error":{"code":429,"type":"invalid_request_error"}}"#),
            Some(UpCode::BadRequest)
        );
        assert_eq!(from_body(500, br#"{"error":{"code":402}}"#), UpCode::Quota);
        assert_eq!(UpCode::RateLimit.short(), "rateLimit");
        assert!(UpCode::Auth.is_config());
        assert!(!UpCode::Timeout.is_config());
        assert!(!UpCode::RateLimit.is_config());
    }
}
