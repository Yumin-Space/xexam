//! Provider adapters (spec 4): OpenAI-compatible APIs, Anthropic, and Cloudflare AI Gateway in either format.
//!
//! HTTP clients: rustls with the ring provider and the Mozilla roots compiled in, no proxy (environment proxies are
//! ignored, so keys and the internal token never go to a proxy), no redirects, HTTP/1.1 only. Every header that carries
//! a secret is marked sensitive. Upstream bodies are never logged or returned: only model ids, a success flag and the
//! fixed codes of [`errors`].
use crate::config::Timeouts;
use crate::limits::MAX_ANSWER_CHARS;
use crate::settings::{Format, GatewayAuth, Provider, ANTHROPIC_VERSION};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use zeroize::Zeroizing;

pub mod anthropic;
pub mod errors;
pub mod openai;
pub mod sse;

pub use errors::{UpCode, UpError};

/// Largest SSE event accepted.
pub const MAX_EVENT: usize = 1 << 20;
/// Error bodies are read up to this size (and only parsed for a type / code).
pub const MAX_ERROR_BODY: usize = 16 * 1024;
/// Largest models list body.
pub const MAX_MODELS_BODY: usize = 4 << 20;
/// Most model ids kept from a list.
pub const MAX_MODELS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

/// One message of the conversation sent upstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub role: Role,
    pub content: String,
}

/// What a stream handler produced from one event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Text(String),
    /// The model is reasoning before (or while) it answers. The reasoning text itself is dropped by the parsers at
    /// once: it is never stored, sent to the browser or logged, and does not count towards the answer cap.
    Thinking,
    Usage {
        input: Option<i64>,
        output: Option<i64>,
    },
    /// Input tokens served from the provider's prompt cache
    Cached(i64),
    Truncated,
    Done,
    Fail(UpCode),
}

/// What the upstream reader reports to a generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Up {
    Text(String),
    /// Reasoning arrived (no text; see [`Item::Thinking`])
    Thinking,
    Usage {
        input: Option<i64>,
        output: Option<i64>,
    },
    /// Input tokens served from the provider's prompt cache
    Cached(i64),
    Truncated,
    Done,
    Fail(UpError),
}

/// The rustls configuration: ring provider, Mozilla roots, TLS 1.2 and 1.3.
pub fn tls_config() -> Result<rustls::ClientConfig, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| format!("TLS setup failed: {e}"))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(cfg)
}

fn base_builder(version: &str) -> Result<reqwest::ClientBuilder, String> {
    Ok(reqwest::Client::builder()
        .tls_backend_preconfigured(tls_config()?)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .http1_only()
        .user_agent(format!("xexam-ai-service/{version}")))
}

/// The client for AI providers (per-request time limits are applied by the callers).
pub fn provider_client(version: &str, t: &Timeouts) -> Result<reqwest::Client, String> {
    base_builder(version)?
        .connect_timeout(t.connect)
        .build()
        .map_err(|_| "cannot build the HTTP client for AI providers".to_string())
}

/// The client for the main backend (plain http).
pub fn backend_client(version: &str, t: &Timeouts) -> Result<reqwest::Client, String> {
    base_builder(version)?
        .connect_timeout(t.backend_connect)
        .timeout(t.backend_total)
        .build()
        .map_err(|_| "cannot build the HTTP client for the main backend".to_string())
}

/// Where and how to call the provider: the saved settings plus the decrypted keys, alive only while a request is built.
/// No `Debug`.
pub struct Target {
    pub provider: Provider,
    pub format: Format,
    pub base: String,
    pub gateway_auth: GatewayAuth,
    pub api_key: Option<Zeroizing<String>>,
    pub gateway_key: Option<Zeroizing<String>>,
}

fn secret_value(text: &str) -> Option<HeaderValue> {
    let mut v = HeaderValue::from_str(text).ok()?;
    v.set_sensitive(true);
    Some(v)
}

fn bearer(key: &str) -> Option<HeaderValue> {
    let text = Zeroizing::new(format!("Bearer {key}"));
    secret_value(&text)
}

impl Target {
    pub fn chat_url(&self) -> String {
        match self.format {
            Format::OpenAi => format!("{}/chat/completions", self.base),
            Format::Anthropic => format!("{}/v1/messages", self.base),
        }
    }

    pub fn models_url(&self) -> String {
        match self.format {
            Format::OpenAi => format!("{}/models", self.base),
            Format::Anthropic => format!("{}/v1/models?limit=1000", self.base),
        }
    }

    /// Authentication headers (4.3), every secret value marked sensitive.
    pub fn auth_headers(&self) -> Option<HeaderMap> {
        let mut h = HeaderMap::new();
        let x_api_key = HeaderName::from_static("x-api-key");
        if self.provider == Provider::Gateway {
            if let Some(g) = &self.gateway_key {
                match self.gateway_auth {
                    GatewayAuth::Bearer => {
                        h.insert(AUTHORIZATION, bearer(g)?);
                    }
                    GatewayAuth::XApiKey => {
                        h.insert(x_api_key.clone(), secret_value(g)?);
                    }
                    GatewayAuth::CfAig => {
                        h.insert(HeaderName::from_static("cf-aig-authorization"), bearer(g)?);
                    }
                }
            }
        }
        if let Some(k) = &self.api_key {
            match self.format {
                Format::OpenAi => {
                    h.insert(AUTHORIZATION, bearer(k)?);
                }
                Format::Anthropic => {
                    h.insert(x_api_key, secret_value(k)?);
                }
            }
        }
        if self.format == Format::Anthropic {
            h.insert(
                HeaderName::from_static("anthropic-version"),
                HeaderValue::from_static(ANTHROPIC_VERSION),
            );
        }
        Some(h)
    }

    /// The streaming chat request. `skip_cache` (a regenerate) asks Cloudflare AI Gateway to bypass its cache for this
    /// one request; it is sent only when the provider is the gateway.
    pub fn chat_request(
        &self,
        client: &reqwest::Client,
        body: &Value,
        skip_cache: bool,
    ) -> Result<reqwest::Request, UpError> {
        let mut headers = self
            .auth_headers()
            .ok_or(UpError::new(UpCode::BadRequest, None))?;
        if skip_cache && self.provider == Provider::Gateway {
            headers.insert(
                HeaderName::from_static("cf-aig-skip-cache"),
                HeaderValue::from_static("true"),
            );
        }
        let bytes = serde_json::to_vec(body).map_err(|_| UpError::new(UpCode::Protocol, None))?;
        client
            .post(self.chat_url())
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "text/event-stream")
            .body(bytes)
            .build()
            .map_err(|_| UpError::new(UpCode::Connect, None))
    }
}

/// The chat request body in the target's format.
pub fn chat_body(
    format: Format,
    model: &str,
    system: &str,
    turns: &[Turn],
    usage_option: bool,
    max_tokens: i64,
) -> Value {
    match format {
        Format::OpenAi => openai::body(model, system, turns, usage_option),
        Format::Anthropic => anthropic::body(model, system, turns, max_tokens),
    }
}

/// Reads at most `cap` bytes of a body (each read within `idle`); `None` when it is larger or the read fails.
async fn read_capped(resp: &mut reqwest::Response, cap: usize, idle: Duration) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    loop {
        match tokio::time::timeout(idle, resp.chunk()).await {
            Ok(Ok(Some(b))) => {
                if buf.len() + b.len() > cap {
                    return None;
                }
                buf.extend_from_slice(&b);
            }
            Ok(Ok(None)) => return Some(buf),
            _ => return None,
        }
    }
}

enum Handler {
    OpenAi(openai::State),
    Anthropic(anthropic::State),
}

impl Handler {
    fn on_event(&mut self, ev: &sse::SseEvent, out: &mut Vec<Item>) {
        match self {
            Handler::OpenAi(s) => s.on_data(&ev.data, out),
            Handler::Anthropic(s) => s.on_event(&ev.name, &ev.data, out),
        }
    }

    fn flush(&mut self, out: &mut Vec<Item>) {
        if let Handler::OpenAi(s) = self {
            s.flush(out);
        }
    }

    fn finish(&mut self) -> Item {
        match self {
            Handler::OpenAi(s) => s.finish(),
            Handler::Anthropic(s) => s.finish(),
        }
    }
}

/// Sends a streaming chat request and reports what arrives on `tx`, ending with exactly one `Done` or `Fail`.
/// Dropping the future (abort) drops the connection.
pub async fn stream_chat(
    client: &reqwest::Client,
    req: reqwest::Request,
    format: Format,
    t: Timeouts,
    tx: UnboundedSender<Up>,
) {
    let fail = |code: UpCode, http: Option<u16>| Up::Fail(UpError::new(code, http));
    let mut resp = match tokio::time::timeout(t.first_byte, client.execute(req)).await {
        Err(_) => {
            let _ = tx.send(fail(UpCode::Timeout, None));
            return;
        }
        Ok(Err(e)) => {
            let _ = tx.send(fail(errors::from_reqwest(&e), None));
            return;
        }
        Ok(Ok(r)) => r,
    };
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        let body = if resp.status().is_redirection() {
            Vec::new()
        } else {
            read_capped(&mut resp, MAX_ERROR_BODY, t.idle)
                .await
                .unwrap_or_default()
        };
        let _ = tx.send(fail(errors::from_body(status, &body), Some(status)));
        return;
    }
    let mut parser = sse::SseParser::new(MAX_EVENT);
    let mut handler = match format {
        Format::OpenAi => Handler::OpenAi(openai::State::default()),
        Format::Anthropic => Handler::Anthropic(anthropic::State::default()),
    };
    let mut chars = 0usize;
    loop {
        let chunk = match tokio::time::timeout(t.idle, resp.chunk()).await {
            Err(_) => {
                let _ = tx.send(fail(UpCode::Timeout, Some(status)));
                return;
            }
            Ok(Err(e)) => {
                let _ = tx.send(fail(errors::from_reqwest(&e), Some(status)));
                return;
            }
            Ok(Ok(c)) => c,
        };
        let events = match &chunk {
            Some(bytes) => parser.push(bytes),
            None => parser.finish(),
        };
        let events = match events {
            Ok(e) => e,
            Err(sse::TooLarge) => {
                let _ = tx.send(fail(UpCode::Protocol, Some(status)));
                return;
            }
        };
        let mut items = Vec::new();
        for ev in &events {
            handler.on_event(ev, &mut items);
            if matches!(items.last(), Some(Item::Done | Item::Fail(_))) {
                break;
            }
        }
        if chunk.is_none() && !matches!(items.last(), Some(Item::Done | Item::Fail(_))) {
            handler.flush(&mut items);
            items.push(handler.finish());
        }
        for item in items {
            let up = match item {
                Item::Text(s) => {
                    chars += s.chars().count();
                    if chars > MAX_ANSWER_CHARS {
                        let _ = tx.send(fail(UpCode::TooLong, Some(status)));
                        return;
                    }
                    Up::Text(s)
                }
                Item::Thinking => Up::Thinking,
                Item::Usage { input, output } => Up::Usage { input, output },
                Item::Cached(n) => Up::Cached(n),
                Item::Truncated => Up::Truncated,
                Item::Done => {
                    let _ = tx.send(Up::Done);
                    return;
                }
                Item::Fail(code) => {
                    let _ = tx.send(fail(code, Some(status)));
                    return;
                }
            };
            let _ = tx.send(up);
        }
        if chunk.is_none() {
            return;
        }
    }
}

/// A finished, collected answer (verify and memory suggestion).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Collected {
    pub text: String,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub truncated: bool,
    pub first_byte_ms: Option<i64>,
}

/// Runs a chat request to the end within `total`, collecting the text.
pub async fn collect_chat(
    client: &reqwest::Client,
    req: reqwest::Request,
    format: Format,
    t: Timeouts,
    total: Duration,
) -> Result<Collected, UpError> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let started = std::time::Instant::now();
    let client = client.clone();
    let reader = tokio::spawn(async move { stream_chat(&client, req, format, t, tx).await });
    let mut out = Collected::default();
    let deadline = tokio::time::sleep(total);
    tokio::pin!(deadline);
    let result = loop {
        tokio::select! {
            _ = &mut deadline => break Err(UpError::new(UpCode::Timeout, None)),
            item = rx.recv() => match item {
                Some(Up::Text(s)) => {
                    if out.first_byte_ms.is_none() {
                        out.first_byte_ms = Some(ms(started.elapsed()));
                    }
                    out.text.push_str(&s);
                }
                // the test call and the memory suggestion wait for the text only
                Some(Up::Thinking) => {}
                Some(Up::Usage { input, output }) => {
                    if input.is_some() { out.tokens_in = input; }
                    if output.is_some() { out.tokens_out = output; }
                }
                Some(Up::Cached(_)) => {}
                Some(Up::Truncated) => out.truncated = true,
                Some(Up::Done) => break Ok(()),
                Some(Up::Fail(e)) => break Err(e),
                None => break Err(UpError::new(UpCode::Protocol, None)),
            }
        }
    };
    reader.abort();
    result.map(|_| out)
}

/// Milliseconds of a duration (saturating).
pub fn ms(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

/// Result of a models listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelList {
    pub models: Vec<String>,
    /// `false`: the service does not list models (manual entry)
    pub listed: bool,
}

/// Lists the models (4.9). 404 / 405 / 501, an unparsable or empty answer → `listed: false`.
pub async fn list_models(
    client: &reqwest::Client,
    target: &Target,
    t: Timeouts,
) -> Result<ModelList, UpError> {
    let unlisted = ModelList {
        models: Vec::new(),
        listed: false,
    };
    let headers = target
        .auth_headers()
        .ok_or(UpError::new(UpCode::BadRequest, None))?;
    let req = client
        .get(target.models_url())
        .headers(headers)
        .header(ACCEPT, "application/json")
        .build()
        .map_err(|_| UpError::new(UpCode::Connect, None))?;
    let work = async {
        let mut resp = client
            .execute(req)
            .await
            .map_err(|e| UpError::new(errors::from_reqwest(&e), None))?;
        let status = resp.status().as_u16();
        if matches!(status, 404 | 405 | 501) {
            return Ok(unlisted.clone());
        }
        if !resp.status().is_success() {
            let body = if resp.status().is_redirection() {
                Vec::new()
            } else {
                read_capped(&mut resp, MAX_ERROR_BODY, t.short_total)
                    .await
                    .unwrap_or_default()
            };
            return Err(UpError::new(errors::from_body(status, &body), Some(status)));
        }
        let Some(body) = read_capped(&mut resp, MAX_MODELS_BODY, t.short_total).await else {
            return Ok(unlisted.clone());
        };
        Ok(parse_models(&body, target.format).unwrap_or(unlisted.clone()))
    };
    match tokio::time::timeout(t.short_total, work).await {
        Ok(r) => r,
        Err(_) => Err(UpError::new(UpCode::Timeout, None)),
    }
}

/// `data[].id`: sorted and deduplicated for the OpenAI format, in API order for Anthropic; at most 2 000. `None` when
/// the body is not such a list or the list is empty.
pub fn parse_models(body: &[u8], format: Format) -> Option<ModelList> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let data = v.get("data")?.as_array()?;
    let mut ids: Vec<String> = Vec::new();
    for item in data {
        if let Some(id) = item.get("id").and_then(Value::as_str) {
            if !id.is_empty() && crate::settings::valid_model(id) && !ids.iter().any(|x| x == id) {
                ids.push(id.to_string());
            }
        }
    }
    if format == Format::OpenAi {
        ids.sort();
    }
    ids.truncate(MAX_MODELS);
    (!ids.is_empty()).then_some(ModelList {
        models: ids,
        listed: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(provider: Provider, format: Format, auth: GatewayAuth, k: bool, g: bool) -> Target {
        Target {
            provider,
            format,
            base: "http://127.0.0.1:9/x".into(),
            gateway_auth: auth,
            api_key: k.then(|| Zeroizing::new("sk-demo-0000-1234".to_string())),
            gateway_key: g.then(|| Zeroizing::new("gw-demo-0000-5678".to_string())),
        }
    }

    #[test]
    fn headers_per_provider_and_scheme() {
        let h = target(
            Provider::OpenAi,
            Format::OpenAi,
            GatewayAuth::Bearer,
            true,
            false,
        )
        .auth_headers()
        .expect("h");
        assert_eq!(h.get("authorization").map(|v| v.is_sensitive()), Some(true));
        assert_eq!(h.len(), 1);
        let h = target(
            Provider::OpenAi,
            Format::OpenAi,
            GatewayAuth::Bearer,
            false,
            false,
        )
        .auth_headers()
        .expect("h");
        assert!(h.is_empty());
        let h = target(
            Provider::Anthropic,
            Format::Anthropic,
            GatewayAuth::Bearer,
            true,
            false,
        )
        .auth_headers()
        .expect("h");
        assert!(h.get("x-api-key").is_some_and(|v| v.is_sensitive()));
        assert_eq!(
            h.get("anthropic-version").and_then(|v| v.to_str().ok()),
            Some("2023-06-01")
        );
        let h = target(
            Provider::Gateway,
            Format::OpenAi,
            GatewayAuth::CfAig,
            true,
            true,
        )
        .auth_headers()
        .expect("h");
        assert!(h
            .get("cf-aig-authorization")
            .is_some_and(|v| v.is_sensitive()));
        assert!(h.get("authorization").is_some_and(|v| v.is_sensitive()));
        let h = target(
            Provider::Gateway,
            Format::Anthropic,
            GatewayAuth::Bearer,
            true,
            true,
        )
        .auth_headers()
        .expect("h");
        assert!(h.get("authorization").is_some());
        assert!(h.get("x-api-key").is_some());
        let h = target(
            Provider::Gateway,
            Format::Anthropic,
            GatewayAuth::XApiKey,
            false,
            true,
        )
        .auth_headers()
        .expect("h");
        assert!(h.get("x-api-key").is_some_and(|v| v.is_sensitive()));
        assert!(h.get("authorization").is_none());
        let t = target(
            Provider::Gateway,
            Format::Anthropic,
            GatewayAuth::XApiKey,
            false,
            true,
        );
        assert_eq!(t.chat_url(), "http://127.0.0.1:9/x/v1/messages");
        assert_eq!(t.models_url(), "http://127.0.0.1:9/x/v1/models?limit=1000");
    }

    #[test]
    fn skip_cache_header_only_for_the_gateway_and_only_when_asked() {
        let client = provider_client("0.9", &Timeouts::default()).expect("client");
        let body = serde_json::json!({"a": 1});
        let header = |t: &Target, skip: bool| {
            t.chat_request(&client, &body, skip)
                .expect("request")
                .headers()
                .get("cf-aig-skip-cache")
                .and_then(|v| v.to_str().ok().map(str::to_string))
        };
        let gw = target(
            Provider::Gateway,
            Format::OpenAi,
            GatewayAuth::CfAig,
            true,
            true,
        );
        assert_eq!(header(&gw, true).as_deref(), Some("true"));
        assert_eq!(header(&gw, false), None);
        let oa = target(
            Provider::OpenAi,
            Format::OpenAi,
            GatewayAuth::Bearer,
            true,
            false,
        );
        assert_eq!(header(&oa, true), None);
        let an = target(
            Provider::Anthropic,
            Format::Anthropic,
            GatewayAuth::Bearer,
            true,
            false,
        );
        assert_eq!(header(&an, true), None);
    }

    #[test]
    fn models_parsing() {
        let b = br#"{"data":[{"id":"b"},{"id":"a"},{"id":"b"},{"x":1},{"id":"bad id"}]}"#;
        assert_eq!(
            parse_models(b, Format::OpenAi).map(|m| m.models),
            Some(vec!["a".to_string(), "b".to_string()])
        );
        assert_eq!(
            parse_models(b, Format::Anthropic).map(|m| m.models),
            Some(vec!["b".to_string(), "a".to_string()])
        );
        assert_eq!(parse_models(br#"{"data":[]}"#, Format::OpenAi), None);
        assert_eq!(parse_models(b"oops", Format::OpenAi), None);
    }

    #[test]
    fn clients_build_without_a_process_crypto_provider() {
        let t = Timeouts::default();
        assert!(provider_client("0.8", &t).is_ok());
        assert!(backend_client("0.8", &t).is_ok());
    }
}
