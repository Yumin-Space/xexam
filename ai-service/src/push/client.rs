//! The one outbound HTTP call of the push courier: `POST <endpoint>` with the encrypted body the backend prepared.
//!
//! The client has the same TLS guard as the AI provider client (rustls with the ring provider and the Mozilla roots), no
//! proxy, no redirects, HTTP/1.1 and no `User-Agent`. Names are resolved by a guard that keeps only public addresses
//! (the rules of `geo::filter::is_public`), so a DNS answer that points into a private network can never be reached.
//!
//! Errors are mapped to outcomes only: a `reqwest::Error` (its text holds the URL), the endpoint, the authorization and
//! the body are never formatted into a log line or forwarded.
use super::allow;
use crate::link::{PushJob, PushOutcome};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::net::SocketAddr;
use std::time::Duration;

/// Connect time limit.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// A whole exchange (connect, headers, the part of the answer that is read).
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(15);
/// Most bytes of the answer that are read (and discarded).
pub const MAX_ANSWER_READ: usize = 2048;
/// Largest decoded body (an RFC 8291 record).
pub const MAX_BODY: usize = 4096;
/// Largest `TTL` (28 days).
pub const MAX_TTL: i64 = 2_419_200;
/// Longest `Authorization` value.
pub const MAX_AUTHORIZATION: usize = 1024;

const URGENCIES: [&str; 4] = ["very-low", "low", "normal", "high"];

/// The DNS answer held no public address.
#[derive(Debug)]
pub struct NoPublicAddress;

impl std::fmt::Display for NoPublicAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no public address")
    }
}

impl std::error::Error for NoPublicAddress {}

/// The addresses that may be connected to (the shared `is_public` rules).
pub fn keep_public(addrs: impl IntoIterator<Item = SocketAddr>) -> Vec<SocketAddr> {
    addrs
        .into_iter()
        .filter(|a| crate::geo::filter::is_public(a.ip()))
        .collect()
}

/// The resolver of the courier client.
struct PublicOnly;

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let found = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let kept = keep_public(found);
            if kept.is_empty() {
                let e: Box<dyn std::error::Error + Send + Sync> = Box::new(NoPublicAddress);
                return Err(e);
            }
            Ok(Box::new(kept.into_iter()) as Addrs)
        })
    }
}

/// The client of the courier.
pub fn build_client() -> Result<reqwest::Client, String> {
    crate::provider::base_builder()?
        .dns_resolver(PublicOnly)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        .build()
        .map_err(|_| "cannot build the HTTP client for web push".to_string())
}

/// What came of one job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delivery {
    pub outcome: PushOutcome,
    pub status: Option<u16>,
    /// Seconds from a `Retry-After` header of a 429 (clamped 1..=3600); `None` when absent or unreadable
    pub retry_after: Option<u32>,
}

impl Delivery {
    /// An outcome that has no answer of a push service behind it.
    pub fn from_outcome(outcome: PushOutcome) -> Delivery {
        Delivery {
            outcome,
            status: None,
            retry_after: None,
        }
    }

    fn rejected(status: Option<u16>) -> Delivery {
        Delivery {
            outcome: PushOutcome::Rejected,
            status,
            retry_after: None,
        }
    }
}

/// Checks a job's fields (the courier does not trust the backend blindly) and returns the decoded body.
pub fn validate(job: &PushJob) -> Option<Vec<u8>> {
    use base64::Engine;
    if !allow::endpoint_allowed(&job.endpoint) {
        return None;
    }
    let auth = job.authorization.as_str();
    if !auth.starts_with("vapid t=")
        || auth.len() > MAX_AUTHORIZATION
        || auth.bytes().any(|b| b == b'\r' || b == b'\n')
    {
        return None;
    }
    if !(0..=MAX_TTL).contains(&job.ttl) || !URGENCIES.contains(&job.urgency.as_str()) {
        return None;
    }
    if let Some(t) = &job.topic {
        let ok = (1..=32).contains(&t.len())
            && t.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !ok {
            return None;
        }
    }
    // 4096 raw bytes are 5462 base64 characters: refuse a longer text before decoding it
    if job.body.len() > MAX_BODY.div_ceil(3) * 4 {
        return None;
    }
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(job.body.as_bytes())
        .ok()?;
    (!raw.is_empty() && raw.len() <= MAX_BODY).then_some(raw)
}

/// Clamps a wait to 1..=3600 seconds.
pub fn clamp_retry(secs: i64) -> u32 {
    secs.clamp(1, 3600) as u32
}

/// Reads a `Retry-After` value: delta seconds or an HTTP date (`now` = unix seconds). `None` when unreadable.
pub fn parse_retry_after(value: &str, now: i64) -> Option<u32> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    if v.bytes().all(|b| b.is_ascii_digit()) {
        // A number too large for i64 is still "a long time"
        return Some(clamp_retry(v.parse::<i64>().unwrap_or(i64::MAX)));
    }
    let at = jiff::fmt::rfc2822::DateTimeParser::new()
        .parse_timestamp(v)
        .ok()?;
    Some(clamp_retry(at.as_second().saturating_sub(now)))
}

/// The outcome table of the push service's HTTP status.
pub fn map_status(code: u16, retry_after: Option<u32>) -> Delivery {
    let (outcome, retry_after) = match code {
        200..=299 => (PushOutcome::Sent, None),
        404 | 410 => (PushOutcome::Gone, None),
        401 | 403 => (PushOutcome::Auth, None),
        429 => (PushOutcome::Retry, retry_after),
        408 | 500..=599 => (PushOutcome::Retry, None),
        // 400, 413, 3xx and any other status: this message is refused, the subscription may be fine
        _ => (PushOutcome::Rejected, None),
    };
    Delivery {
        outcome,
        status: Some(code),
        retry_after,
    }
}

/// Maps a failed request without ever formatting the error.
fn map_error(e: &reqwest::Error) -> Delivery {
    use std::error::Error;
    let mut cur: Option<&(dyn Error + 'static)> = Some(e);
    while let Some(err) = cur {
        if err.is::<NoPublicAddress>() {
            return Delivery::rejected(None);
        }
        cur = err.source();
    }
    Delivery {
        outcome: PushOutcome::Retry,
        status: Some(0),
        retry_after: None,
    }
}

/// Where a job goes: its endpoint, or (tests) the target address with the endpoint's path.
fn url_for(job: &PushJob, target: Option<SocketAddr>) -> Option<String> {
    match target {
        None => Some(job.endpoint.clone()),
        Some(addr) => Some(format!(
            "http://{addr}{}",
            allow::path_and_query(&job.endpoint)?
        )),
    }
}

/// Delivers one job. Never fails: every problem is an outcome.
pub async fn deliver(
    client: &reqwest::Client,
    job: &PushJob,
    target: Option<SocketAddr>,
) -> Delivery {
    let Some(body) = validate(job) else {
        return Delivery::rejected(None);
    };
    let Some(url) = url_for(job, target) else {
        return Delivery::rejected(None);
    };
    send_to(client, &url, job, body).await
}

/// The request itself. `validate` has run.
pub async fn send_to(
    client: &reqwest::Client,
    url: &str,
    job: &PushJob,
    body: Vec<u8>,
) -> Delivery {
    let Ok(mut auth) = reqwest::header::HeaderValue::from_str(&job.authorization) else {
        return Delivery::rejected(None);
    };
    auth.set_sensitive(true);
    let mut req = client
        .post(url)
        .header("TTL", job.ttl.to_string())
        .header("Urgency", job.urgency.as_str())
        .header("Content-Encoding", "aes128gcm")
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .header(reqwest::header::AUTHORIZATION, auth)
        .body(body);
    if let Some(t) = &job.topic {
        req = req.header("Topic", t.as_str());
    }
    let mut resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return map_error(&e),
    };
    let code = resp.status().as_u16();
    let retry_after = if code == 429 {
        resp.headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| parse_retry_after(v, crate::util::unix_now()))
    } else {
        None
    };
    // Read a little of the answer and drop it
    let mut read = 0usize;
    while read < MAX_ANSWER_READ {
        match resp.chunk().await {
            Ok(Some(b)) => read += b.len(),
            _ => break,
        }
    }
    map_status(code, retry_after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn job() -> PushJob {
        PushJob {
            id: 7,
            tries: 1,
            endpoint: "https://fcm.googleapis.com/fcm/send/demo".into(),
            ttl: 3600,
            urgency: "normal".into(),
            topic: Some("remind".into()),
            authorization: "vapid t=a.b.c, k=demo".into(),
            body: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1u8; 40]),
        }
    }

    #[test]
    fn a_good_job_validates_and_each_field_is_checked() {
        assert_eq!(validate(&job()), Some(vec![1u8; 40]));
        let mut j = job();
        j.topic = None;
        assert!(validate(&j).is_some());
        type Break = Box<dyn Fn(&mut PushJob)>;
        let bads: Vec<Break> = vec![
            Box::new(|j| j.endpoint = "https://127.0.0.1/x".into()),
            Box::new(|j| j.authorization = "Bearer x".into()),
            Box::new(|j| j.authorization = "vapid t=a\r\nX: y".into()),
            Box::new(|j| j.authorization = format!("vapid t={}", "a".repeat(1100))),
            Box::new(|j| j.ttl = -1),
            Box::new(|j| j.ttl = MAX_TTL + 1),
            Box::new(|j| j.urgency = "urgent".into()),
            Box::new(|j| j.topic = Some(String::new())),
            Box::new(|j| j.topic = Some("a".repeat(33))),
            Box::new(|j| j.topic = Some("bad topic".into())),
            Box::new(|j| j.body = "not base64!!".into()),
            Box::new(|j| j.body = String::new()),
            Box::new(|j| {
                j.body = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0u8; 4097])
            }),
        ];
        for (i, f) in bads.iter().enumerate() {
            let mut j = job();
            f(&mut j);
            assert!(validate(&j).is_none(), "case {i}");
        }
        let mut j = job();
        j.ttl = MAX_TTL;
        j.urgency = "very-low".into();
        j.topic = Some("a".repeat(32));
        j.body = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([9u8; 4096]);
        assert_eq!(validate(&j).map(|b| b.len()), Some(4096));
    }

    #[test]
    fn statuses_map_to_the_table() {
        let o = |c, r| map_status(c, r).outcome;
        for c in [200, 201, 202, 204] {
            assert_eq!(o(c, None), PushOutcome::Sent);
        }
        for c in [404, 410] {
            assert_eq!(o(c, None), PushOutcome::Gone);
        }
        for c in [401, 403] {
            assert_eq!(o(c, None), PushOutcome::Auth);
        }
        for c in [408, 500, 502, 503, 599] {
            assert_eq!(o(c, None), PushOutcome::Retry);
        }
        for c in [400, 413, 301, 302, 307, 405, 422, 100] {
            assert_eq!(o(c, None), PushOutcome::Rejected, "{c}");
        }
        let d = map_status(429, Some(30));
        assert_eq!(
            (d.outcome, d.status, d.retry_after),
            (PushOutcome::Retry, Some(429), Some(30))
        );
        // Retry-After matters only for a 429
        assert_eq!(map_status(503, Some(30)).retry_after, None);
    }

    #[test]
    fn retry_after_in_both_forms() {
        let now = 1_700_000_000;
        assert_eq!(parse_retry_after("120", now), Some(120));
        assert_eq!(parse_retry_after(" 7 ", now), Some(7));
        assert_eq!(parse_retry_after("0", now), Some(1));
        assert_eq!(parse_retry_after("99999", now), Some(3600));
        assert_eq!(
            parse_retry_after("99999999999999999999999", now),
            Some(3600)
        );
        assert_eq!(parse_retry_after("", now), None);
        assert_eq!(parse_retry_after("soon", now), None);
        assert_eq!(parse_retry_after("-5", now), None);
        let date = |ts: i64| {
            jiff::fmt::rfc2822::DateTimePrinter::new()
                .timestamp_to_string(&jiff::Timestamp::from_second(ts).expect("ts"))
                .expect("date")
        };
        assert_eq!(parse_retry_after(&date(now + 90), now), Some(90));
        assert_eq!(parse_retry_after(&date(now - 90), now), Some(1));
        assert_eq!(parse_retry_after(&date(now + 90_000), now), Some(3600));
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 08:49:37 GMT", 784_111_777 - 30),
            Some(30)
        );
    }

    #[test]
    fn the_dns_guard_keeps_public_addresses_only() {
        let a = |s: &str| -> SocketAddr { s.parse().expect("addr") };
        let kept = keep_public([
            a("127.0.0.1:0"),
            a("10.1.2.3:0"),
            a("192.168.0.9:0"),
            a("169.254.169.254:0"),
            a("[::1]:0"),
            a("[fe80::1]:0"),
            a("[::ffff:127.0.0.1]:0"),
            a("8.8.8.8:0"),
            a("[2001:4860:4860::8888]:0"),
        ]);
        assert_eq!(kept, vec![a("8.8.8.8:0"), a("[2001:4860:4860::8888]:0")]);
    }

    /// A name that resolves only to loopback never gets a connection: the job is rejected, the listener sees nothing.
    #[tokio::test]
    async fn a_private_dns_answer_is_rejected_without_connecting() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = l.local_addr().expect("addr").port();
        let client = build_client().expect("client");
        let url = format!("http://localhost:{port}/x");
        let d = send_to(&client, &url, &job(), vec![1; 10]).await;
        assert_eq!(d.outcome, PushOutcome::Rejected);
        assert_eq!(d.status, None);
        let waited = tokio::time::timeout(Duration::from_millis(200), l.accept()).await;
        assert!(waited.is_err(), "the listener must not be connected to");
    }
}
