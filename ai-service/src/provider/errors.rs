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
    /// How long the provider asked to wait before the next try (a 429 only)
    pub retry_ms: Option<u64>,
    /// The 429 is about a daily quota: waiting a few seconds will not help
    pub daily: bool,
}

impl UpError {
    pub fn new(code: UpCode, http: Option<u16>) -> UpError {
        UpError {
            code,
            http,
            retry_ms: None,
            daily: false,
        }
    }
}

/// A wait longer than this (milliseconds) is treated like a daily quota.
pub const DAILY_WAIT_MS: u64 = 900_000;

/// `"34s"`, `"1.5s"` or `"34"` (seconds) as milliseconds.
fn seconds_text_ms(t: &str) -> Option<u64> {
    let t = t.trim();
    let n = t.strip_suffix('s').unwrap_or(t).trim();
    let secs: f64 = n.parse().ok()?;
    (secs.is_finite() && (0.0..=1.0e9).contains(&secs)).then(|| (secs * 1000.0).round() as u64)
}

/// What a 429 body says: the first `retryDelay`, whether a quota id names a daily quota, and the error message.
fn scan_retry(v: &Value, depth: u8, delay: &mut Option<u64>, daily: &mut bool) {
    if depth > 8 {
        return;
    }
    match v {
        Value::Object(o) => {
            for (k, x) in o {
                match (k.as_str(), x) {
                    ("retryDelay", Value::String(t)) => {
                        if delay.is_none() {
                            *delay = seconds_text_ms(t);
                        }
                    }
                    ("quotaId" | "quotaMetric" | "quota_id", Value::String(t)) => {
                        let t = t.to_ascii_lowercase();
                        if t.contains("perday") || t.contains("per_day") || t.contains("daily") {
                            *daily = true;
                        }
                    }
                    _ => scan_retry(x, depth + 1, delay, daily),
                }
            }
        }
        Value::Array(a) => {
            for x in a {
                scan_retry(x, depth + 1, delay, daily);
            }
        }
        _ => {}
    }
}

/// `retry in 12.5s` in a message, as milliseconds.
fn message_delay_ms(msg: &str) -> Option<u64> {
    let lower = msg.to_ascii_lowercase();
    let rest = &lower[lower.find("retry in ")? + "retry in ".len()..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(rest.len());
    let secs: f64 = rest.get(..end)?.parse().ok()?;
    (secs.is_finite() && (0.0..=1.0e9).contains(&secs)).then(|| (secs * 1000.0).round() as u64)
}

/// For a 429: how long to wait (the response header first, then the body) and whether the quota is a daily one.
/// Any other status gives `(None, false)`. A wait over 15 minutes counts as daily. Never reads or keeps provider text
/// beyond these numbers.
pub fn retry_hint(status: u16, body: &[u8], header_ms: Option<u64>) -> (Option<u64>, bool) {
    if status != 429 {
        return (None, false);
    }
    let mut delay = None;
    let mut daily = false;
    if let Ok(v) = serde_json::from_slice::<Value>(body) {
        scan_retry(&v, 0, &mut delay, &mut daily);
        let msg = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .or_else(|| v.get("message").and_then(Value::as_str));
        if let Some(m) = msg {
            if delay.is_none() {
                delay = message_delay_ms(m);
            }
            let m = m.to_ascii_lowercase();
            if m.contains("per day") || m.contains("perday") || m.contains("daily quota") {
                daily = true;
            }
        }
    }
    let wait = header_ms.or(delay);
    if wait.is_some_and(|w| w > DAILY_WAIT_MS) {
        daily = true;
    }
    (wait, daily)
}

/// Maps an error type or code string found in a body.
pub fn from_type(s: &str) -> Option<UpCode> {
    Some(match s {
        "authentication_error" | "permission_error" | "invalid_api_key" => UpCode::Auth,
        "billing_error" | "insufficient_quota" | "credit_balance_exhausted" => UpCode::Quota,
        "not_found_error" | "model_not_found" => UpCode::NotFound,
        "rate_limit_error" | "rate_limit_exceeded" => UpCode::RateLimit,
        "overloaded_error" | "server_is_overloaded" | "service_unavailable_error" => {
            UpCode::Overloaded
        }
        "api_error" | "server_error" => UpCode::Server,
        "invalid_request_error" | "invalid_prompt" | "invalid_image" | "invalid_base64_image" => {
            UpCode::BadRequest
        }
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

    #[test]
    fn retry_hint_reads_the_wait_and_the_daily_flag_of_a_429() {
        // Google: an array body with RetryInfo
        let google = br#"[{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","details":[{"@type":"x.QuotaFailure","violations":[{"quotaId":"GenerateRequestsPerMinutePerProjectPerModel-FreeTier"}]},{"@type":"x.RetryInfo","retryDelay":"34s"}]}}]"#;
        assert_eq!(retry_hint(429, google, None), (Some(34_000), false));
        // fractional seconds
        let frac = br#"{"error":{"details":[{"retryDelay":"1.5s"}]}}"#;
        assert_eq!(retry_hint(429, frac, None), (Some(1_500), false));
        // a daily quota id
        let daily = br#"{"error":{"details":[{"violations":[{"quotaId":"GenerateRequestsPerDayPerProjectPerModel-FreeTier"}]},{"retryDelay":"20s"}]}}"#;
        assert_eq!(retry_hint(429, daily, None), (Some(20_000), true));
        // the header wins over the body
        assert_eq!(retry_hint(429, google, Some(2_000)), (Some(2_000), false));
        // text fallbacks
        let msg = br#"{"error":{"message":"Rate limited. Please retry in 12.5s."}}"#;
        assert_eq!(retry_hint(429, msg, None), (Some(12_500), false));
        let msg = br#"{"error":{"message":"You exceeded your current quota (requests per day)."}}"#;
        assert_eq!(retry_hint(429, msg, None), (None, true));
        let msg = br#"{"error":{"message":"Daily quota reached"}}"#;
        assert_eq!(retry_hint(429, msg, None), (None, true));
        // a very long wait is a daily quota
        let long = br#"{"error":{"details":[{"retryDelay":"1000s"}]}}"#;
        assert_eq!(retry_hint(429, long, None), (Some(1_000_000), true));
        assert_eq!(retry_hint(429, b"", Some(901_000)), (Some(901_000), true));
        // nothing known
        assert_eq!(retry_hint(429, b"not json", None), (None, false));
        // other statuses
        assert_eq!(retry_hint(500, google, Some(5)), (None, false));
        assert_eq!(retry_hint(503, daily, None), (None, false));
        // deeply nested: the scan stops after eight levels
        let mut deep = String::from(r#"{"retryDelay":"9s"}"#);
        for _ in 0..12 {
            deep = format!(r#"{{"a":{deep}}}"#);
        }
        assert_eq!(retry_hint(429, deep.as_bytes(), None), (None, false));
    }
}
