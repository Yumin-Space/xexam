//! The one outbound HTTP call of the external IP-location lookup (spec 4.6). The client has the same TLS guard as the AI
//! provider client (rustls with the ring provider and the Mozilla roots), no proxy, no redirects, HTTP/1.1, no cookies.
//!
//! Errors are mapped to failure codes only: a `reqwest::Error`, the URL, a header or the body is never formatted into a
//! log line, an event or an answer (the URL can hold the key).
use super::template::Built;
use super::{Failure, FailureCode, GEO_MAX_BODY};
use serde_json::Value;
use std::time::Duration;

/// The client of the lookups. `total` is the connect time limit; each request also gets it as its total limit.
pub fn build_client(version: &str, total: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .tls_backend_preconfigured(crate::provider::tls_config()?)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .http1_only()
        .user_agent(format!("xexam-ai-service/{version}"))
        .connect_timeout(total)
        .build()
        .map_err(|_| "cannot build the HTTP client for the location lookup".to_string())
}

fn transport(e: &reqwest::Error) -> Failure {
    Failure::code(if e.is_timeout() {
        FailureCode::Timeout
    } else {
        FailureCode::Unreachable
    })
}

/// `GET`s the filled URL: the whole exchange (connect, answer headers, body) has to end within `total`. A 3xx is a
/// `redirect` (never followed), another non-2xx status an `httpStatus`, more than 64 KiB of body `tooLarge`, and a body
/// that is not a JSON object `notJson`.
pub async fn fetch(
    client: &reqwest::Client,
    total: Duration,
    built: Built,
) -> Result<Value, Failure> {
    let mut req = client
        .get(built.url)
        .timeout(total)
        .header(reqwest::header::ACCEPT, "application/json");
    if let Some((name, value)) = built.header {
        req = req.header(name, value);
    }
    let mut resp = req.send().await.map_err(|e| transport(&e))?;
    let status = resp.status().as_u16();
    if (300..400).contains(&status) {
        return Err(Failure::code(FailureCode::Redirect));
    }
    if !(200..300).contains(&status) {
        return Err(Failure {
            code: FailureCode::HttpStatus,
            status: Some(status),
            message: None,
        });
    }
    if resp
        .content_length()
        .is_some_and(|n| n > GEO_MAX_BODY as u64)
    {
        return Err(Failure::code(FailureCode::TooLarge));
    }
    let mut buf: Vec<u8> = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(b)) => {
                if buf.len() + b.len() > GEO_MAX_BODY {
                    return Err(Failure::code(FailureCode::TooLarge));
                }
                buf.extend_from_slice(&b);
            }
            Ok(None) => break,
            Err(e) => return Err(transport(&e)),
        }
    }
    match serde_json::from_slice::<Value>(&buf) {
        Ok(v @ Value::Object(_)) => Ok(v),
        _ => Err(Failure::code(FailureCode::NotJson)),
    }
}
