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
use serde_json::{json, Value};
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
    /// Reasoning text of the model: OpenAI-format `reasoning_content` / `reasoning`, a leading `<think>` /
    /// `<thinking>` / `<thought>` span of the content, Anthropic `thinking_delta`. Never empty. Only AI Chat keeps it
    /// (chat.rs, purpose None); every other consumer drops it. Never counts towards the answer cap, never logged.
    Reasoning(String),
    /// Reasoning happened but carries no text here (Anthropic thinking / redacted_thinking block start, entering an
    /// inline span).
    Thinking,
    Usage {
        input: Option<i64>,
        output: Option<i64>,
    },
    /// Input tokens served from the provider's prompt cache
    Cached(i64),
    /// Input tokens the provider wrote to its prompt cache
    CacheWrite(i64),
    Truncated,
    Done,
    Fail(UpCode),
}

/// What the upstream reader reports to a generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Up {
    Text(String),
    /// See [`Item::Reasoning`]
    Reasoning(String),
    /// Reasoning happened without text (see [`Item::Thinking`])
    Thinking,
    Usage {
        input: Option<i64>,
        output: Option<i64>,
    },
    /// Input tokens served from the provider's prompt cache
    Cached(i64),
    /// Input tokens the provider wrote to its prompt cache
    CacheWrite(i64),
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

/// How the answer-length limit is put into a request body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenField {
    /// OpenAI-compatible: `"max_tokens"` (Anthropic always uses this one)
    MaxTokens,
    /// OpenAI-compatible: `"max_completion_tokens"` (OpenAI's reasoning and newer models)
    MaxCompletionTokens,
    /// OpenAI-compatible: no token field at all (the body of v0.9.1)
    Omit,
}

impl TokenField {
    /// The JSON key, or `None` for [`TokenField::Omit`].
    pub fn key(self) -> Option<&'static str> {
        match self {
            TokenField::MaxTokens => Some("max_tokens"),
            TokenField::MaxCompletionTokens => Some("max_completion_tokens"),
            TokenField::Omit => None,
        }
    }
}

/// The field and the value of the answer-length limit of one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenLimit {
    pub field: TokenField,
    pub value: i64,
}

impl TokenLimit {
    /// No token field (the administrator's connection test with an OpenAI-compatible service).
    pub fn omit() -> TokenLimit {
        TokenLimit {
            field: TokenField::Omit,
            value: 0,
        }
    }

    pub fn max_tokens(value: i64) -> TokenLimit {
        TokenLimit {
            field: TokenField::MaxTokens,
            value,
        }
    }
}

/// The chat request body in the target's format. OpenAI-compatible: the token field is added when it is not `Omit`
/// and nothing else changes; Anthropic: `max_tokens` is always sent (the API requires it).
pub fn chat_body(
    format: Format,
    model: &str,
    system: &str,
    turns: &[Turn],
    usage_option: bool,
    token: TokenLimit,
) -> Value {
    match format {
        Format::OpenAi => openai::body(model, system, turns, usage_option, token),
        Format::Anthropic => anthropic::body(model, system, turns, token.value),
    }
}

/// Puts `prefix` in front of the first user turn of a built body. Anthropic: the turn becomes two text blocks, the prefix
/// with a `cache_control` breakpoint and the turn's own text after it; the `system` block carries its breakpoint
/// already, so the system text and the prefix are the cached part. OpenAI-compatible: the prefix, a blank line, the text.
pub fn put_prefix(format: Format, body: &mut Value, prefix: &str) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    let Some(first) = messages
        .iter_mut()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))
    else {
        return;
    };
    let own = match first.get("content") {
        Some(Value::String(s)) => s.clone(),
        _ => return,
    };
    first["content"] = match format {
        Format::Anthropic => json!([
            {"type": "text", "text": prefix, "cache_control": {"type": "ephemeral"}},
            {"type": "text", "text": own}
        ]),
        Format::OpenAi => Value::String(format!("{prefix}\n\n{own}")),
    };
}

/// Like [`put_prefix`], with cache markers for the OpenAI-compatible format too (content-part shape, which LiteLLM and
/// other Anthropic-backed gateways pass through): the first user message becomes `[{prefix + cache_control}, {own}]` and
/// the system message `[{system + cache_control}]`, two breakpoints in all. Anthropic: the same as `put_prefix`.
pub fn put_prefix_marked(format: Format, body: &mut Value, prefix: &str) {
    put_prefix(format, body, prefix);
    if format != Format::OpenAi {
        return;
    }
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    if let Some(sys) = messages
        .iter_mut()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("system"))
    {
        if let Some(Value::String(text)) = sys.get("content").cloned() {
            sys["content"] = json!([
                {"type": "text", "text": text, "cache_control": {"type": "ephemeral"}}
            ]);
        }
    }
    let Some(first) = messages
        .iter_mut()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))
    else {
        return;
    };
    // `put_prefix` joined "{prefix}\n\n{own}": split it again at the known prefix length
    let Some(Value::String(joined)) = first.get("content").cloned() else {
        return;
    };
    let Some(own) = joined
        .strip_prefix(prefix)
        .and_then(|r| r.strip_prefix("\n\n"))
    else {
        return;
    };
    first["content"] = json!([
        {"type": "text", "text": prefix, "cache_control": {"type": "ephemeral"}},
        {"type": "text", "text": own}
    ]);
}

/// What a settings fingerprint learned about the token limit (kept in memory only, see `App`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenMemo {
    pub field: TokenField,
    /// Anthropic: the ceiling the model accepted
    pub cap: Option<i64>,
}

/// The first request's limit: the answer length converted to tokens (`want`), in the field the remembered step of this
/// settings fingerprint says (OpenAI-compatible), or clamped to what the model is known to accept (Anthropic: the
/// remembered cap, else 4096 when the stored connection test found 8192 too large).
pub fn first_token_limit(
    format: Format,
    want: i64,
    stored_max: i64,
    memo: Option<TokenMemo>,
) -> TokenLimit {
    let unlimited = want >= crate::limits::UNLIMITED_TOKENS;
    match format {
        Format::OpenAi if unlimited => TokenLimit::omit(),
        Format::OpenAi => TokenLimit {
            field: memo.map_or(TokenField::MaxTokens, |m| m.field),
            value: want,
        },
        Format::Anthropic => {
            // the API requires a value: the largest one a model accepts, lowered by the ladder when refused
            let want = if unlimited { 64_000 } else { want };
            let cap = memo
                .and_then(|m| m.cap)
                .or((stored_max == 4096).then_some(4096));
            TokenLimit::max_tokens(cap.map_or(want, |c| want.min(c)))
        }
    }
}

/// The step after `sent` on the ladder, whatever the refusal said: OpenAI-compatible `MaxTokens` ->
/// `MaxCompletionTokens` (same value) -> `Omit` -> none; Anthropic `v` -> `min(v, stored_max)` when that is smaller.
pub fn ladder_after(format: Format, sent: &TokenLimit, stored_max: i64) -> Option<TokenLimit> {
    match format {
        Format::OpenAi => match sent.field {
            TokenField::MaxTokens => Some(TokenLimit {
                field: TokenField::MaxCompletionTokens,
                value: sent.value,
            }),
            TokenField::MaxCompletionTokens => Some(TokenLimit {
                field: TokenField::Omit,
                value: sent.value,
            }),
            TokenField::Omit => None,
        },
        Format::Anthropic => (stored_max < sent.value).then(|| TokenLimit::max_tokens(stored_max)),
    }
}

/// `true` when the error body (JSON, as OpenAI or Anthropic write it) names `field` as `error.param` or inside
/// `error.message` / `message`.
fn body_names(error_body: &str, field: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(error_body) else {
        return false;
    };
    let err = v.get("error");
    let candidates = [
        err.and_then(|e| e.get("param")).and_then(Value::as_str),
        err.and_then(|e| e.get("message")).and_then(Value::as_str),
        err.and_then(Value::as_str),
        v.get("message").and_then(Value::as_str),
    ];
    candidates.iter().flatten().any(|text| text.contains(field))
}

/// The next step of the ladder after an HTTP refusal answered before any stream event: only for status 400 or 422
/// whose error body names the field that was sent; otherwise `None` (the refusal stays the ordinary upstream error).
pub fn next_token_limit(
    format: Format,
    sent: &TokenLimit,
    status: u16,
    error_body: &str,
    stored_max: i64,
) -> Option<TokenLimit> {
    if !matches!(status, 400 | 422) {
        return None;
    }
    let field = match (format, sent.field.key()) {
        (_, Some(k)) => k,
        (Format::Anthropic, None) => "max_tokens",
        (Format::OpenAi, None) => return None,
    };
    if !body_names(error_body, field) {
        return None;
    }
    ladder_after(format, sent, stored_max)
}

/// Called with the step that was accepted after a refusal.
pub type Remember = Arc<dyn Fn(&TokenLimit) + Send + Sync>;

/// The requests of one model call, in ladder order: the first request and, behind it, each fallback that can follow a
/// refused token field. Built together while the key is decrypted; the reader sends a next one only on such a refusal.
/// No `Debug` (the requests carry keys).
pub struct Plan {
    pub format: Format,
    /// The stored result of the connection test (the Anthropic ceiling)
    pub stored_max: i64,
    pub steps: Vec<(TokenLimit, reqwest::Request)>,
    /// The app remembers the accepted step per settings fingerprint
    pub remember: Option<Remember>,
    /// OpenAI-compatible request with cache markers: the same ladder without markers (empty when the steps carry none).
    /// Used once when a 400 / 422 refuses the marked request for another reason than the token field.
    pub plain: Vec<(TokenLimit, reqwest::Request)>,
    /// Called when the unmarked retry succeeded: the connection must not get markers
    pub remember_plain: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Request with a structured output field: the same ladder without that field and without cache markers (empty when
    /// the steps carry none). Used once when a 400 / 422 still refuses the request after the plain ladder (or without
    /// one).
    pub unformatted: Vec<(TokenLimit, reqwest::Request)>,
    /// Called when the request without the structured output field succeeded: the connection must not get it
    pub remember_unformatted: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Plan {
    /// A call without a ladder (the connection test).
    pub fn single(format: Format, req: reqwest::Request) -> Plan {
        Plan {
            format,
            stored_max: 0,
            steps: vec![(TokenLimit::omit(), req)],
            remember: None,
            plain: Vec::new(),
            remember_plain: None,
            unformatted: Vec::new(),
            remember_unformatted: None,
        }
    }
}

/// The parts of a chat request besides the token limit.
pub struct ChatSpec<'a> {
    pub model: &'a str,
    pub system: &'a str,
    pub turns: &'a [Turn],
    pub usage_option: bool,
    pub skip_cache: bool,
    /// A text that opens the first user turn and is byte-identical across the calls of a job (the whole file of an AI
    /// import, A-OWNER-71): Anthropic gets it as its own text block with a `cache_control` breakpoint, an
    /// OpenAI-compatible service gets the same text first in the user message (providers that cache automatically hit it)
    pub prefix: Option<&'a str>,
    /// `false`: no cache markers in the request (a call that cannot profit from a cache, or whose prefix is below the
    /// model's cache minimum, see [`cache_floor_ok`])
    pub cache: bool,
    /// A JSON schema the answer must follow (structured output); `None` = free text. Anthropic gets
    /// `output_config.format`, an OpenAI-compatible service `response_format` (see [`put_schema`]).
    pub schema: Option<&'a serde_json::Value>,
}

/// Adds the structured output field of `schema` to a request body.
fn put_schema(format: Format, body: &mut Value, schema: &Value) {
    match format {
        Format::Anthropic => {
            body["output_config"] = json!({"format": {"type": "json_schema", "schema": schema}});
        }
        Format::OpenAi => {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": {"name": "xexam_answer", "strict": true, "schema": schema}
            });
        }
    }
}

/// Admin connection tests (90-second rule): one model listing, one test attempt, the attempts of one model, and the
/// deadline of a whole test request, in seconds.
pub const TEST_LIST_SECS: u64 = 15;
pub const TEST_ATTEMPT_SECS: u64 = 30;
pub const TEST_ATTEMPTS: u32 = 2;
pub const TEST_TOTAL_SECS: u64 = 90;

/// The fewest input tokens a provider caches for `model` (the caching starts at this prompt size). An unknown model gets
/// the largest figure, so a marker is never sent where it cannot work.
pub fn cache_min_tokens(format: Format, model: &str) -> i64 {
    let m = model.to_ascii_lowercase();
    match format {
        Format::Anthropic => {
            if m.contains("haiku-4-5")
                || m.contains("haiku-4.5")
                || m.contains("opus-4-5")
                || m.contains("opus-4.5")
            {
                4096
            } else if m.contains("haiku") {
                2048
            } else if m.contains("sonnet") || m.contains("opus") {
                1024
            } else {
                4096
            }
        }
        Format::OpenAi => {
            if m.starts_with("gpt-")
                || m.starts_with("o1")
                || m.starts_with("o3")
                || m.starts_with("o4")
            {
                1024
            } else {
                4096
            }
        }
    }
}

/// `true` when a request whose cached part is `system` + `prefix` is long enough for the model's cache: the estimate of
/// its tokens, with a 30 % margin, reaches [`cache_min_tokens`]. A caller sets `ChatSpec.cache` from this.
pub fn cache_floor_ok(format: Format, model: &str, system: &str, prefix: Option<&str>) -> bool {
    let tokens =
        crate::limits::estimate_tokens(system) + prefix.map_or(0, crate::limits::estimate_tokens);
    tokens.saturating_mul(13) / 10 >= cache_min_tokens(format, model)
}

/// Removes every `cache_control` marker of a request body.
pub fn strip_cache_markers(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.remove("cache_control");
            for x in m.values_mut() {
                strip_cache_markers(x);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(strip_cache_markers),
        _ => {}
    }
}

/// Builds the plan: the request with `first`, then one request per later step of the ladder. OpenAI-compatible with a
/// prefix and `spec.cache` (and `no_markers` false): the requests carry cache markers ([`put_prefix_marked`]) and the
/// plan also holds the unmarked ladder for the one fallback; `on_no_markers` is called when that fallback worked.
/// With `spec.schema` (and `no_schema` false) every request carries the structured output field and the plan also holds
/// the ladder without it (`unformatted`); `on_no_schema` is called when that last fallback worked.
#[allow(clippy::too_many_arguments)]
pub fn build_plan(
    target: &Target,
    client: &reqwest::Client,
    spec: &ChatSpec<'_>,
    first: TokenLimit,
    stored_max: i64,
    remember: Option<Remember>,
    no_markers: bool,
    on_no_markers: Option<Arc<dyn Fn() + Send + Sync>>,
    no_schema: bool,
    on_no_schema: Option<Arc<dyn Fn() + Send + Sync>>,
) -> Result<Plan, UpError> {
    let marked =
        target.format == Format::OpenAi && spec.prefix.is_some() && spec.cache && !no_markers;
    let mut steps = Vec::new();
    let mut plain = Vec::new();
    let mut unformatted = Vec::new();
    let schema = spec.schema.filter(|_| !no_schema);
    let mut limit = Some(first);
    while let Some(l) = limit {
        let mut body = chat_body(
            target.format,
            spec.model,
            spec.system,
            spec.turns,
            spec.usage_option,
            l,
        );
        let mut fallback = None;
        if let Some(p) = spec.prefix {
            if marked {
                let mut plain_body = body.clone();
                put_prefix(target.format, &mut plain_body, p);
                fallback = Some(plain_body);
                put_prefix_marked(target.format, &mut body, p);
            } else {
                put_prefix(target.format, &mut body, p);
            }
        }
        if !spec.cache {
            strip_cache_markers(&mut body);
        }
        let mut bare = None;
        if let Some(sc) = schema {
            // the last fallback carries neither the structured output field nor cache markers, so a server that refuses
            // both still answers once (the next call then finds its markers fallback by itself)
            bare = Some(fallback.clone().unwrap_or_else(|| body.clone()));
            put_schema(target.format, &mut body, sc);
            if let Some(b) = fallback.as_mut() {
                put_schema(target.format, b, sc);
            }
        }
        steps.push((l, target.chat_request(client, &body, spec.skip_cache)?));
        if let Some(b) = fallback {
            plain.push((l, target.chat_request(client, &b, spec.skip_cache)?));
        }
        if let Some(b) = bare {
            unformatted.push((l, target.chat_request(client, &b, spec.skip_cache)?));
        }
        limit = ladder_after(target.format, &l, stored_max);
    }
    Ok(Plan {
        format: target.format,
        stored_max,
        steps,
        remember,
        plain,
        remember_plain: on_no_markers,
        unformatted,
        remember_unformatted: on_no_schema,
    })
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

/// What one request ended in: it ran (its result went to the channel), or the service refused it by HTTP status before
/// any stream event (nothing was sent to the channel yet).
enum Attempt {
    /// `ok`: a 2xx response whose stream ended in `Done`
    Ran {
        ok: bool,
    },
    Refused {
        status: u16,
        body: Vec<u8>,
    },
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
    if let Attempt::Refused { status, body } = stream_once(client, req, format, t, &tx).await {
        let _ = tx.send(Up::Fail(UpError::new(
            errors::from_body(status, &body),
            Some(status),
        )));
    }
}

/// Runs a plan: the first request, and after an HTTP 400 / 422 that names the token field it sent (before any stream
/// event) the next step of the ladder. A marked OpenAI-compatible request that is refused with 400 / 422 for any other
/// reason is retried once, unmarked (same token step), and a success is remembered (`remember_plain`); a request with a structured output field that is still refused with
/// 400 / 422 is retried without it (`remember_unformatted`). Several requests,
/// but one call: the caller counts, times and records it once. Ends with exactly one `Done` or `Fail` on `tx`.
pub async fn stream_plan(
    client: &reqwest::Client,
    plan: Plan,
    t: Timeouts,
    tx: UnboundedSender<Up>,
) {
    let Plan {
        format,
        stored_max,
        steps,
        remember,
        plain,
        remember_plain,
        unformatted,
        remember_unformatted,
    } = plan;
    let args = (client, format, stored_max, t, &tx, &remember);
    let (mut status, mut body, mut at) = match run_ladder(args, steps, 0).await {
        Ok(_) => return,
        Err(r) => r,
    };
    if !plain.is_empty() && matches!(status, 400 | 422) {
        match run_ladder(args, plain, at).await {
            Ok(ok) => {
                // Only a clean 2xx answer proves the unmarked form works; a timeout or a broken stream proves nothing
                if ok {
                    if let Some(r) = &remember_plain {
                        r();
                    }
                }
                return;
            }
            // the unformatted ladder starts at the token step the plain ladder reached
            Err((s, b, i)) => (status, body, at) = (s, b, i),
        }
    }
    if !unformatted.is_empty() && matches!(status, 400 | 422) {
        match run_ladder(args, unformatted, at).await {
            Ok(ok) => {
                if ok {
                    if let Some(r) = &remember_unformatted {
                        r();
                    }
                }
                return;
            }
            Err((s, b, _)) => (status, body) = (s, b),
        }
    }
    let _ = tx.send(Up::Fail(UpError::new(
        errors::from_body(status, &body),
        Some(status),
    )));
}

type LadderArgs<'a> = (
    &'a reqwest::Client,
    Format,
    i64,
    Timeouts,
    &'a UnboundedSender<Up>,
    &'a Option<Remember>,
);

/// Runs the steps of a ladder from index `skip`; `Ok(clean)` when a request ran (its result went to the channel;
/// `clean` = 2xx and finished with `Done`), else the
/// final refusal (status, body, index of the step).
async fn run_ladder(
    (client, format, stored_max, t, tx, remember): LadderArgs<'_>,
    steps: Vec<(TokenLimit, reqwest::Request)>,
    skip: usize,
) -> Result<bool, (u16, Vec<u8>, usize)> {
    let total = steps.len();
    for (i, (sent, req)) in steps.into_iter().enumerate().skip(skip) {
        match stream_once(client, req, format, t, tx).await {
            Attempt::Ran { ok } => {
                if i > 0 {
                    if let Some(r) = remember {
                        r(&sent);
                    }
                }
                return Ok(ok);
            }
            Attempt::Refused { status, body } => {
                let text = String::from_utf8_lossy(&body);
                let retry = i + 1 < total
                    && next_token_limit(format, &sent, status, &text, stored_max).is_some();
                if !retry {
                    return Err((status, body, i));
                }
            }
        }
    }
    Err((500, Vec::new(), total))
}

async fn stream_once(
    client: &reqwest::Client,
    req: reqwest::Request,
    format: Format,
    t: Timeouts,
    tx: &UnboundedSender<Up>,
) -> Attempt {
    let fail = |code: UpCode, http: Option<u16>| Up::Fail(UpError::new(code, http));
    let mut resp = match tokio::time::timeout(t.first_byte, client.execute(req)).await {
        Err(_) => {
            let _ = tx.send(fail(UpCode::Timeout, None));
            return Attempt::Ran { ok: false };
        }
        Ok(Err(e)) => {
            let _ = tx.send(fail(errors::from_reqwest(&e), None));
            return Attempt::Ran { ok: false };
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
        return Attempt::Refused { status, body };
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
                return Attempt::Ran { ok: false };
            }
            Ok(Err(e)) => {
                let _ = tx.send(fail(errors::from_reqwest(&e), Some(status)));
                return Attempt::Ran { ok: false };
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
                return Attempt::Ran { ok: false };
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
                        return Attempt::Ran { ok: false };
                    }
                    Up::Text(s)
                }
                Item::Reasoning(s) => Up::Reasoning(s),
                Item::Thinking => Up::Thinking,
                Item::Usage { input, output } => Up::Usage { input, output },
                Item::Cached(n) => Up::Cached(n),
                Item::CacheWrite(n) => Up::CacheWrite(n),
                Item::Truncated => Up::Truncated,
                Item::Done => {
                    let _ = tx.send(Up::Done);
                    return Attempt::Ran { ok: true };
                }
                Item::Fail(code) => {
                    let _ = tx.send(fail(code, Some(status)));
                    return Attempt::Ran { ok: false };
                }
            };
            let _ = tx.send(up);
        }
        if chunk.is_none() {
            return Attempt::Ran { ok: false };
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
    /// Input tokens the provider read from its prompt cache (when it says so)
    pub tokens_cached: Option<i64>,
    /// Input tokens the provider wrote to its prompt cache (when it says so)
    pub tokens_cache_write: Option<i64>,
}

/// Aborts a spawned task when dropped.
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Runs a chat call (its whole plan) to the end within `total`, collecting the text.
pub async fn collect_chat(
    client: &reqwest::Client,
    plan: Plan,
    t: Timeouts,
    total: Duration,
) -> Result<Collected, UpError> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let started = std::time::Instant::now();
    let client = client.clone();
    let reader = tokio::spawn(async move { stream_plan(&client, plan, t, tx).await });
    // Dropping this future (a cancel, a stop, a deadline) drops the in-flight request too
    let _abort_on_drop = AbortOnDrop(reader.abort_handle());
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
                Some(Up::Reasoning(_)) => {}
                Some(Up::Thinking) => {}
                Some(Up::Usage { input, output }) => {
                    if input.is_some() { out.tokens_in = input; }
                    if output.is_some() { out.tokens_out = output; }
                }
                Some(Up::Cached(n)) => out.tokens_cached = Some(n),
                Some(Up::CacheWrite(n)) => out.tokens_cache_write = Some(n),
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
    /// Context sizes in tokens the listing names (`context_length`, `context_window`, `max_input_tokens`), by model id
    pub contexts: std::collections::HashMap<String, usize>,
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
        contexts: Default::default(),
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
    let mut contexts = std::collections::HashMap::new();
    for item in data {
        if let Some(id) = item.get("id").and_then(Value::as_str) {
            if !id.is_empty() && crate::settings::valid_model(id) && !ids.iter().any(|x| x == id) {
                ids.push(id.to_string());
                let ctx = ["context_length", "context_window", "max_input_tokens"]
                    .iter()
                    .find_map(|k| item.get(*k).and_then(Value::as_u64))
                    .filter(|n| *n > 0);
                if let Some(n) = ctx {
                    contexts.insert(id.to_string(), usize::try_from(n).unwrap_or(usize::MAX));
                }
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
        contexts,
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

    fn turns() -> Vec<Turn> {
        vec![Turn {
            role: Role::User,
            content: "hi".into(),
        }]
    }

    #[test]
    fn chat_body_per_token_field() {
        let t = turns();
        let tl = |field, value| TokenLimit { field, value };
        let plain = chat_body(Format::OpenAi, "m", "s", &t, true, TokenLimit::omit());
        assert!(plain.get("max_tokens").is_none());
        assert!(plain.get("max_completion_tokens").is_none());
        let a = chat_body(
            Format::OpenAi,
            "m",
            "s",
            &t,
            true,
            tl(TokenField::MaxTokens, 1500),
        );
        assert_eq!(a["max_tokens"], serde_json::json!(1500));
        let b = chat_body(
            Format::OpenAi,
            "m",
            "s",
            &t,
            true,
            tl(TokenField::MaxCompletionTokens, 1500),
        );
        assert_eq!(b["max_completion_tokens"], serde_json::json!(1500));
        assert!(b.get("max_tokens").is_none());
        // nothing else changed
        for (k, v) in plain.as_object().expect("object") {
            assert_eq!(&a[k], v);
            assert_eq!(&b[k], v);
        }
        assert_eq!(a.as_object().map(|m| m.len()), Some(5));
        // Anthropic: always max_tokens
        let c = chat_body(
            Format::Anthropic,
            "m",
            "s",
            &t,
            true,
            TokenLimit::max_tokens(1500),
        );
        assert_eq!(c["max_tokens"], serde_json::json!(1500));
    }

    #[test]
    fn unlimited_sends_no_token_field_or_the_largest_anthropic_value() {
        let u = crate::limits::UNLIMITED_TOKENS;
        assert_eq!(
            first_token_limit(Format::OpenAi, u, 8192, None),
            TokenLimit::omit()
        );
        assert_eq!(
            first_token_limit(Format::Anthropic, u, 8192, None),
            TokenLimit::max_tokens(64_000)
        );
        assert_eq!(
            first_token_limit(Format::Anthropic, u, 4096, None),
            TokenLimit::max_tokens(4096)
        );
    }

    #[test]
    fn first_limit_uses_the_memo_and_the_stored_ceiling() {
        let want = 24000;
        assert_eq!(
            first_token_limit(Format::OpenAi, want, 8192, None),
            TokenLimit::max_tokens(want)
        );
        let memo = TokenMemo {
            field: TokenField::MaxCompletionTokens,
            cap: None,
        };
        assert_eq!(
            first_token_limit(Format::OpenAi, 1500, 8192, Some(memo)),
            TokenLimit {
                field: TokenField::MaxCompletionTokens,
                value: 1500
            }
        );
        // Anthropic: the stored 4096 clamps at once; 8192 stored does not
        assert_eq!(
            first_token_limit(Format::Anthropic, 24000, 4096, None),
            TokenLimit::max_tokens(4096)
        );
        assert_eq!(
            first_token_limit(Format::Anthropic, 1500, 4096, None),
            TokenLimit::max_tokens(1500)
        );
        assert_eq!(
            first_token_limit(Format::Anthropic, 24000, 8192, None),
            TokenLimit::max_tokens(24000)
        );
        let memo = TokenMemo {
            field: TokenField::MaxTokens,
            cap: Some(8192),
        };
        assert_eq!(
            first_token_limit(Format::Anthropic, 24000, 8192, Some(memo)),
            TokenLimit::max_tokens(8192)
        );
    }

    #[test]
    fn the_token_ladder() {
        let mt = TokenLimit::max_tokens(1500);
        let mct = TokenLimit {
            field: TokenField::MaxCompletionTokens,
            value: 1500,
        };
        let omit = TokenLimit {
            field: TokenField::Omit,
            value: 1500,
        };
        let unsupported_mt = r#"{"error":{"message":"Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead.","type":"invalid_request_error","param":"max_tokens","code":"unsupported_parameter"}}"#;
        let by_param = r#"{"error":{"message":"bad","param":"max_completion_tokens"}}"#;
        let by_message = r#"{"message":"unknown field max_completion_tokens"}"#;
        let other = r#"{"error":{"message":"model not found","type":"invalid_request_error"}}"#;
        let f = Format::OpenAi;
        assert_eq!(
            next_token_limit(f, &mt, 400, unsupported_mt, 8192),
            Some(mct)
        );
        assert_eq!(
            next_token_limit(f, &mt, 422, unsupported_mt, 8192),
            Some(mct)
        );
        assert_eq!(next_token_limit(f, &mct, 400, by_param, 8192), Some(omit));
        assert_eq!(next_token_limit(f, &mct, 400, by_message, 8192), Some(omit));
        // the end of the ladder
        assert_eq!(next_token_limit(f, &omit, 400, unsupported_mt, 8192), None);
        // a 400 that does not name the sent field, a body that is not JSON, other statuses
        assert_eq!(next_token_limit(f, &mt, 400, other, 8192), None);
        assert_eq!(next_token_limit(f, &mt, 400, by_param, 8192), None);
        assert_eq!(next_token_limit(f, &mt, 400, "max_tokens", 8192), None);
        assert_eq!(next_token_limit(f, &mt, 401, unsupported_mt, 8192), None);
        assert_eq!(next_token_limit(f, &mt, 429, unsupported_mt, 8192), None);
        assert_eq!(next_token_limit(f, &mt, 500, unsupported_mt, 8192), None);
        // Anthropic: the stored ceiling, once
        let a = Format::Anthropic;
        let big = TokenLimit::max_tokens(24000);
        let refused = r#"{"type":"error","error":{"type":"invalid_request_error","message":"max_tokens: 24000 > 8192, which is the maximum allowed number of output tokens for demo"}}"#;
        assert_eq!(
            next_token_limit(a, &big, 400, refused, 8192),
            Some(TokenLimit::max_tokens(8192))
        );
        assert_eq!(
            next_token_limit(a, &TokenLimit::max_tokens(8192), 400, refused, 8192),
            None
        );
        assert_eq!(
            next_token_limit(a, &TokenLimit::max_tokens(4096), 400, refused, 4096),
            None
        );
        assert_eq!(next_token_limit(a, &big, 400, other, 8192), None);
        assert_eq!(
            next_token_limit(a, &big, 422, refused, 4096),
            Some(TokenLimit::max_tokens(4096))
        );
    }

    #[test]
    fn clients_build_without_a_process_crypto_provider() {
        let t = Timeouts::default();
        assert!(provider_client("0.8", &t).is_ok());
        assert!(backend_client("0.8", &t).is_ok());
    }

    fn one_turn(text: &str) -> Vec<Turn> {
        vec![Turn {
            role: Role::User,
            content: text.into(),
        }]
    }

    #[test]
    fn an_anthropic_prefix_is_its_own_block_with_the_breakpoint_and_the_text_follows() {
        let turns = one_turn("TASK CONVERT now");
        let mut body = chat_body(
            Format::Anthropic,
            "m",
            "rules",
            &turns,
            true,
            TokenLimit::max_tokens(900),
        );
        put_prefix(Format::Anthropic, &mut body, "FILE\nL1: made-up line");
        let content = &body["messages"][0]["content"];
        assert_eq!(
            content,
            &json!([
                {"type": "text", "text": "FILE\nL1: made-up line", "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": "TASK CONVERT now"}
            ])
        );
        // the rules block keeps its own breakpoint, so the cached part is rules + prefix
        assert_eq!(
            body["system"][0]["cache_control"],
            json!({"type": "ephemeral"})
        );
        // the varying text carries no breakpoint
        assert!(content[1].get("cache_control").is_none());
    }

    #[test]
    fn an_openai_prefix_comes_first_in_the_user_message_and_nothing_else_changes() {
        let turns = one_turn("TASK CONVERT now");
        let plain = chat_body(
            Format::OpenAi,
            "m",
            "rules",
            &turns,
            true,
            TokenLimit::omit(),
        );
        let mut body = plain.clone();
        put_prefix(Format::OpenAi, &mut body, "FILE\nL1: made-up line");
        assert_eq!(
            body["messages"][1]["content"],
            json!("FILE\nL1: made-up line\n\nTASK CONVERT now")
        );
        body["messages"][1]["content"] = plain["messages"][1]["content"].clone();
        assert_eq!(body, plain, "only the user message changed");
        assert!(body.to_string().find("cache_control").is_none());
    }

    #[test]
    fn the_prefix_bytes_are_identical_for_two_different_tasks() {
        let a = one_turn("TASK RECOGNISE\nRecognise the file now.");
        let b = one_turn("TASK CONVERT\nSLOTS\n[]\n\nConvert the 3 questions now.");
        for f in [Format::Anthropic, Format::OpenAi] {
            let (mut x, mut y) = (
                chat_body(f, "m", "rules", &a, true, TokenLimit::max_tokens(900)),
                chat_body(f, "m", "rules", &b, true, TokenLimit::max_tokens(900)),
            );
            put_prefix(f, &mut x, "PREFIX-BYTES");
            put_prefix(f, &mut y, "PREFIX-BYTES");
            let head = |v: &Value| match f {
                Format::Anthropic => v["messages"][0]["content"][0].to_string(),
                Format::OpenAi => v["messages"][1]["content"].as_str().unwrap()[..12].to_string(),
            };
            assert_eq!(head(&x), head(&y));
            assert_eq!(x["system"], y["system"]);
        }
    }

    #[test]
    fn the_model_listing_keeps_the_context_figures_it_names() {
        let body = br#"{"data":[{"id":"demo-a","context_length":200000},{"id":"demo-b","max_input_tokens":32000},{"id":"demo-c"}]}"#;
        let l = parse_models(body, Format::OpenAi).expect("list");
        assert_eq!(l.contexts.get("demo-a"), Some(&200_000));
        assert_eq!(l.contexts.get("demo-b"), Some(&32_000));
        assert!(!l.contexts.contains_key("demo-c"));
    }
    #[test]
    fn cache_minimums_and_the_floor() {
        assert_eq!(cache_min_tokens(Format::Anthropic, "demo-sonnet-9"), 1024);
        assert_eq!(cache_min_tokens(Format::Anthropic, "Demo-Haiku-3"), 2048);
        assert_eq!(cache_min_tokens(Format::Anthropic, "demo-haiku-4-5"), 4096);
        assert_eq!(cache_min_tokens(Format::Anthropic, "demo-unknown"), 4096);
        assert_eq!(cache_min_tokens(Format::OpenAi, "gpt-demo"), 1024);
        assert_eq!(cache_min_tokens(Format::OpenAi, "vendor/demo"), 4096);
        // a short prompt is below every minimum, a long one reaches the 1024 of a known model but not the unknown 4096
        assert!(!cache_floor_ok(
            Format::Anthropic,
            "demo-sonnet-9",
            "rules",
            Some("short")
        ));
        let long = "word ".repeat(1000);
        assert!(cache_floor_ok(
            Format::Anthropic,
            "demo-sonnet-9",
            "rules",
            Some(&long)
        ));
        assert!(!cache_floor_ok(
            Format::Anthropic,
            "demo-unknown",
            "rules",
            Some(&long)
        ));
        // the margin: 800 estimated tokens x 1.3 = 1040 reaches 1024
        let eight = "a".repeat(3200);
        assert!(cache_floor_ok(
            Format::Anthropic,
            "demo-sonnet-9",
            &eight,
            None
        ));
        assert!(!cache_floor_ok(
            Format::Anthropic,
            "demo-sonnet-9",
            &"a".repeat(3000),
            None
        ));
    }

    #[test]
    fn no_cache_means_no_markers_anywhere() {
        let mut turns = one_turn("first question");
        turns.push(Turn {
            role: Role::Assistant,
            content: "an answer".into(),
        });
        turns.push(Turn {
            role: Role::User,
            content: "second question".into(),
        });
        let mut body = chat_body(
            Format::Anthropic,
            "m",
            "rules",
            &turns,
            true,
            TokenLimit::max_tokens(900),
        );
        put_prefix(Format::Anthropic, &mut body, "PREFIX-BYTES");
        assert!(body.to_string().matches("cache_control").count() >= 2);
        strip_cache_markers(&mut body);
        assert!(!body.to_string().contains("cache_control"));
        // the text itself is untouched
        assert_eq!(body["system"][0]["text"], "rules");
        assert_eq!(body["messages"][0]["content"][0]["text"], "PREFIX-BYTES");
    }

    #[test]
    fn the_test_budget_constants() {
        assert_eq!(
            (
                TEST_LIST_SECS,
                TEST_ATTEMPT_SECS,
                TEST_ATTEMPTS,
                TEST_TOTAL_SECS
            ),
            (15, 30, 2, 90)
        );
    }

    fn spec<'a>(turns: &'a [Turn], prefix: Option<&'a str>, cache: bool) -> ChatSpec<'a> {
        ChatSpec {
            model: "m",
            system: "rules",
            turns,
            usage_option: true,
            skip_cache: false,
            prefix,
            cache,
            schema: None,
        }
    }

    fn body_of(req: &reqwest::Request) -> Value {
        serde_json::from_slice(req.body().and_then(|b| b.as_bytes()).expect("bytes")).expect("json")
    }

    fn openai_target(base: &str) -> Target {
        let mut t = target(
            Provider::OpenAi,
            Format::OpenAi,
            GatewayAuth::Bearer,
            true,
            false,
        );
        t.base = base.to_string();
        t
    }

    #[test]
    fn an_openai_prefix_with_cache_is_a_content_part_array_with_two_breakpoints() {
        let turns = one_turn("TASK now");
        let mut body = chat_body(
            Format::OpenAi,
            "m",
            "rules",
            &turns,
            true,
            TokenLimit::omit(),
        );
        put_prefix_marked(Format::OpenAi, &mut body, "FILE\nL1: made-up line");
        assert_eq!(
            body["messages"][0]["content"],
            json!([{"type": "text", "text": "rules", "cache_control": {"type": "ephemeral"}}])
        );
        assert_eq!(
            body["messages"][1]["content"],
            json!([
                {"type": "text", "text": "FILE\nL1: made-up line", "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": "TASK now"}
            ])
        );
        assert_eq!(body.to_string().matches("cache_control").count(), 2);
        strip_cache_markers(&mut body);
        assert!(!body.to_string().contains("cache_control"));
    }

    #[tokio::test]
    async fn a_schema_adds_the_structured_output_field_and_an_unformatted_ladder() {
        let turns = one_turn("TASK");
        let client = provider_client("test", &Timeouts::default()).expect("client");
        let schema = json!({"type": "object"});
        let first = TokenLimit::omit();
        let build = |t: &Target, with: bool, no: bool| {
            let mut sp = spec(&turns, None, false);
            sp.schema = with.then_some(&schema);
            build_plan(t, &client, &sp, first, 8192, None, false, None, no, None).expect("plan")
        };
        let oa = openai_target("http://127.0.0.1:9");
        let an = target(
            Provider::Anthropic,
            Format::Anthropic,
            GatewayAuth::Bearer,
            true,
            false,
        );
        let p = build(&oa, true, false);
        for (_, r) in &p.steps {
            let b = body_of(r);
            assert_eq!(b["response_format"]["type"], "json_schema");
            assert_eq!(b["response_format"]["json_schema"]["name"], "xexam_answer");
            assert_eq!(b["response_format"]["json_schema"]["strict"], true);
            assert_eq!(b["response_format"]["json_schema"]["schema"], schema);
        }
        assert_eq!(p.unformatted.len(), p.steps.len());
        assert!(body_of(&p.unformatted[0].1)
            .get("response_format")
            .is_none());
        let p = build(&an, true, false);
        let b = body_of(&p.steps[0].1);
        assert_eq!(b["output_config"]["format"]["type"], "json_schema");
        assert_eq!(b["output_config"]["format"]["schema"], schema);
        assert_eq!(p.unformatted.len(), p.steps.len());
        assert!(body_of(&p.unformatted[0].1).get("output_config").is_none());
        // marked steps: the plain ladder keeps the field, the unformatted ladder has neither field nor markers
        let mut sp = spec(&turns, Some("PFX"), true);
        sp.schema = Some(&schema);
        let p = build_plan(
            &oa, &client, &sp, first, 8192, None, false, None, false, None,
        )
        .expect("plan");
        let (s, pl, un) = (
            body_of(&p.steps[0].1),
            body_of(&p.plain[0].1),
            body_of(&p.unformatted[0].1),
        );
        assert!(s.to_string().contains("cache_control") && s.get("response_format").is_some());
        assert!(!pl.to_string().contains("cache_control") && pl.get("response_format").is_some());
        assert!(!un.to_string().contains("cache_control") && un.get("response_format").is_none());
        assert_eq!(p.unformatted.len(), p.steps.len());
        for t in [&oa, &an] {
            for (with, no) in [(false, false), (true, true)] {
                let p = build(t, with, no);
                let b = body_of(&p.steps[0].1);
                assert!(b.get("output_config").is_none() && b.get("response_format").is_none());
                assert!(p.unformatted.is_empty());
            }
        }
    }

    #[tokio::test]
    async fn build_plan_marks_only_openai_with_prefix_and_cache() {
        let client = provider_client("test", &Timeouts::default()).expect("client");
        let turns = one_turn("TASK now");
        let t = openai_target("http://127.0.0.1:9/v1");
        let first = TokenLimit::omit();
        let plan = |t: &Target, sp: &ChatSpec<'_>, no: bool| {
            build_plan(t, &client, sp, first, 8192, None, no, None, false, None).expect("plan")
        };
        // prefix + cache: marked steps and the plain ladder
        let p = plan(&t, &spec(&turns, Some("PFX"), true), false);
        let b = body_of(&p.steps[0].1);
        assert!(b["messages"][1]["content"].is_array());
        assert_eq!(p.plain.len(), p.steps.len());
        let pb = body_of(&p.plain[0].1);
        assert_eq!(pb["messages"][0]["content"], json!("rules"));
        assert_eq!(pb["messages"][1]["content"], json!("PFX\n\nTASK now"));
        // cache off or a memo: plain strings, the same as put_prefix, no fallback ladder
        for (sp, no) in [
            (spec(&turns, Some("PFX"), false), false),
            (spec(&turns, Some("PFX"), true), true),
        ] {
            let p = plan(&t, &sp, no);
            assert!(p.plain.is_empty());
            assert_eq!(body_of(&p.steps[0].1), pb);
        }
        // no prefix: the plain chat body, byte-identical to today
        let p = plan(&t, &spec(&turns, None, true), false);
        assert!(p.plain.is_empty());
        let golden = chat_body(Format::OpenAi, "m", "rules", &turns, true, first);
        assert_eq!(body_of(&p.steps[0].1), golden);
        // Anthropic is unchanged: its own breakpoints, no plain ladder
        let mut a = openai_target("http://127.0.0.1:9/v1");
        a.format = Format::Anthropic;
        a.provider = Provider::Anthropic;
        let p = plan(&a, &spec(&turns, Some("PFX"), true), false);
        assert!(p.plain.is_empty());
        let mut want = chat_body(Format::Anthropic, "m", "rules", &turns, true, first);
        put_prefix(Format::Anthropic, &mut want, "PFX");
        assert_eq!(body_of(&p.steps[0].1), want);
    }

    type Seen = std::sync::Arc<std::sync::Mutex<Vec<Value>>>;

    /// A stub server: refuses a body that holds `cache_control` with 400 (when `refuse`) and answers everything else
    /// with a one-event stream; returns its base URL and the bodies it saw.
    async fn marker_stub(refuse: bool) -> (String, Seen) {
        marker_stub_with(refuse, false).await
    }

    /// Like `marker_stub`; with `broken_plain` the unmarked retry answers 200 but the stream carries an error event.
    async fn marker_stub_with(refuse: bool, broken_plain: bool) -> (String, Seen) {
        use axum::{body::Bytes, http::StatusCode, response::IntoResponse, Router};
        let seen: Seen = Default::default();
        let log = seen.clone();
        let app = Router::new().fallback(move |body: Bytes| {
            let log = log.clone();
            async move {
                let v: Value = serde_json::from_slice(&body).expect("json");
                let marked = v.to_string().contains("cache_control");
                log.lock().expect("log").push(v);
                if marked && refuse {
                    return (
                        StatusCode::BAD_REQUEST,
                        r#"{"error":{"message":"unknown field cache_control"}}"#,
                    )
                        .into_response();
                }
                if broken_plain {
                    return (
                        [("content-type", "text/event-stream")],
                        "data: {\"error\":{\"message\":\"upstream broke\"}}

",
                    )
                        .into_response();
                }
                (
                    [("content-type", "text/event-stream")],
                    "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
                )
                    .into_response()
            }
        });
        let l = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = l.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(l, app).await;
        });
        (format!("http://{addr}"), seen)
    }

    async fn run(plan: Plan) -> Vec<Up> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let client = provider_client("test", &Timeouts::default()).expect("client");
        stream_plan(&client, plan, Timeouts::default(), tx).await;
        let mut out = Vec::new();
        while let Ok(u) = rx.try_recv() {
            out.push(u);
        }
        out
    }

    #[tokio::test]
    async fn a_server_that_refuses_markers_gets_one_unmarked_retry_and_is_remembered() {
        let (base, seen) = marker_stub(true).await;
        let client = provider_client("test", &Timeouts::default()).expect("client");
        let turns = one_turn("TASK now");
        let t = openai_target(&format!("{base}/v1"));
        let memo = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let build = |no: bool| {
            let m = memo.clone();
            let cb: Arc<dyn Fn() + Send + Sync> =
                Arc::new(move || m.store(true, std::sync::atomic::Ordering::SeqCst));
            build_plan(
                &t,
                &client,
                &spec(&turns, Some("PFX"), true),
                TokenLimit::omit(),
                8192,
                None,
                no,
                Some(cb),
                false,
                None,
            )
            .expect("plan")
        };
        let ups = run(build(false)).await;
        assert!(matches!(ups.last(), Some(Up::Done)), "{ups:?}");
        assert!(ups.iter().any(|u| matches!(u, Up::Text(s) if s == "ok")));
        assert!(memo.load(std::sync::atomic::Ordering::SeqCst));
        {
            let s = seen.lock().expect("seen");
            assert_eq!(s.len(), 2, "one marked request, one retry");
            assert!(s[0].to_string().contains("cache_control"));
            assert!(!s[1].to_string().contains("cache_control"));
            assert_eq!(s[1]["messages"][1]["content"], json!("PFX\n\nTASK now"));
        }
        // the next call is built with the memo: no markers at all, one request
        let ups = run(build(true)).await;
        assert!(matches!(ups.last(), Some(Up::Done)), "{ups:?}");
        let s = seen.lock().expect("seen");
        assert_eq!(s.len(), 3);
        assert!(!s[2].to_string().contains("cache_control"));
    }

    #[tokio::test]
    async fn a_server_that_accepts_markers_is_asked_once_and_never_flagged() {
        let (base, seen) = marker_stub(false).await;
        let client = provider_client("test", &Timeouts::default()).expect("client");
        let turns = one_turn("TASK now");
        let t = openai_target(&format!("{base}/v1"));
        let plan = build_plan(
            &t,
            &client,
            &spec(&turns, Some("PFX"), true),
            TokenLimit::omit(),
            8192,
            None,
            false,
            Some(Arc::new(|| panic!("must not be remembered"))),
            false,
            None,
        )
        .expect("plan");
        let ups = run(plan).await;
        assert!(matches!(ups.last(), Some(Up::Done)), "{ups:?}");
        assert_eq!(seen.lock().expect("seen").len(), 1);
    }

    #[tokio::test]
    async fn a_broken_unmarked_retry_is_not_remembered() {
        // marked request: 400; unmarked retry: 200 with an in-stream error, so nothing proves the plain form works
        let (base, seen) = marker_stub_with(true, true).await;
        let client = provider_client("test", &Timeouts::default()).expect("client");
        let turns = one_turn("TASK now");
        let t = openai_target(&format!("{base}/v1"));
        let plan = build_plan(
            &t,
            &client,
            &spec(&turns, Some("PFX"), true),
            TokenLimit::omit(),
            8192,
            None,
            false,
            Some(Arc::new(|| panic!("must not be remembered"))),
            false,
            None,
        )
        .expect("plan");
        let ups = run(plan).await;
        assert!(matches!(ups.last(), Some(Up::Fail(_))), "{ups:?}");
        assert_eq!(seen.lock().expect("seen").len(), 2);
    }

    /// A stub server that refuses (400) a request with cache markers (`markers`) or with `response_format` (`schema`).
    async fn refusing_stub(markers: bool, schema: bool) -> (String, Seen) {
        use axum::{body::Bytes, http::StatusCode, response::IntoResponse, Router};
        let seen: Seen = Default::default();
        let log = seen.clone();
        let app = Router::new().fallback(move |body: Bytes| {
            let log = log.clone();
            async move {
                let v: Value = serde_json::from_slice(&body).expect("json");
                let refuse = (markers && v.to_string().contains("cache_control"))
                    || (schema && v.get("response_format").is_some());
                log.lock().expect("log").push(v);
                if refuse {
                    return (
                        StatusCode::BAD_REQUEST,
                        r#"{"error":{"message":"unsupported field"}}"#,
                    )
                        .into_response();
                }
                (
                    [("content-type", "text/event-stream")],
                    "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
                )
                    .into_response()
            }
        });
        let l = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = l.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(l, app).await;
        });
        (format!("http://{addr}"), seen)
    }

    #[tokio::test]
    async fn a_server_that_refuses_structured_output_gets_one_unformatted_retry() {
        use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
        let schema = json!({"type": "object"});
        let turns = one_turn("TASK now");
        let client = provider_client("test", &Timeouts::default()).expect("client");
        // (refuse markers, refuse schema, requests of the first call, markers remembered)
        for (markers, refuse_schema, sent, plain_memo) in
            [(false, true, 3, false), (true, true, 3, false)]
        {
            let (base, seen) = refusing_stub(markers, refuse_schema).await;
            let t = openai_target(&format!("{base}/v1"));
            let no_markers = Arc::new(AtomicBool::new(false));
            let no_schema = Arc::new(AtomicBool::new(false));
            let build = |nm: bool, ns: bool| {
                let (m, s) = (no_markers.clone(), no_schema.clone());
                let mut sp = spec(&turns, Some("PFX"), true);
                sp.schema = Some(&schema);
                build_plan(
                    &t,
                    &client,
                    &sp,
                    TokenLimit::omit(),
                    8192,
                    None,
                    nm,
                    Some(Arc::new(move || m.store(true, SeqCst))),
                    ns,
                    Some(Arc::new(move || s.store(true, SeqCst))),
                )
                .expect("plan")
            };
            let ups = run(build(false, false)).await;
            assert!(matches!(ups.last(), Some(Up::Done)), "{ups:?}");
            assert_eq!(seen.lock().expect("seen").len(), sent);
            assert!(no_schema.load(SeqCst));
            assert_eq!(no_markers.load(SeqCst), plain_memo);
            {
                let s = seen.lock().expect("seen");
                let last = s.last().expect("last");
                assert!(last.get("response_format").is_none());
                assert!(!last.to_string().contains("cache_control"));
            }
            // the next call goes out without the schema; markers are found out (or kept) by the markers fallback
            let ups = run(build(false, true)).await;
            assert!(matches!(ups.last(), Some(Up::Done)), "{ups:?}");
            assert_eq!(no_markers.load(SeqCst), markers);
            let s = seen.lock().expect("seen");
            assert_eq!(s.len(), sent + if markers { 2 } else { 1 });
            assert!(s
                .iter()
                .skip(sent)
                .all(|v| v.get("response_format").is_none()));
        }
    }

    /// A stub server that answers every request with the given event stream body.
    async fn fixed_stub(body: String) -> String {
        use axum::{response::IntoResponse, Router};
        let app = Router::new().fallback(move || {
            let body = body.clone();
            async move { ([("content-type", "text/event-stream")], body).into_response() }
        });
        let l = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = l.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(l, app).await;
        });
        format!("http://{addr}/v1")
    }

    fn sse_of(deltas: &[Value]) -> String {
        let mut out = String::new();
        for d in deltas {
            out.push_str(&format!("data: {}\n\n", json!({"choices": [{"delta": d}]})));
        }
        out.push_str("data: [DONE]\n\n");
        out
    }

    async fn plan_for(base: &str) -> (reqwest::Client, Plan) {
        let client = provider_client("test", &Timeouts::default()).expect("client");
        let turns = one_turn("TASK now");
        let t = openai_target(base);
        let plan = build_plan(
            &t,
            &client,
            &spec(&turns, None, false),
            TokenLimit::omit(),
            8192,
            None,
            false,
            None,
            false,
            None,
        )
        .expect("plan");
        (client, plan)
    }

    #[tokio::test]
    async fn reasoning_is_forwarded_and_not_counted_towards_the_answer_cap() {
        let mut deltas: Vec<Value> = (0..150)
            .map(|_| json!({"reasoning_content": "r".repeat(1000)}))
            .collect();
        deltas.push(json!({"content": "Short."}));
        let base = fixed_stub(sse_of(&deltas)).await;
        let (_client, plan) = plan_for(&base).await;
        let ups = run(plan).await;
        let reasoning: usize = ups
            .iter()
            .map(|u| match u {
                Up::Reasoning(s) => s.chars().count(),
                _ => 0,
            })
            .sum();
        assert_eq!(reasoning, 150_000);
        assert!(ups.contains(&Up::Text("Short.".into())), "{ups:?}");
        assert!(matches!(ups.last(), Some(Up::Done)), "{ups:?}");
        assert!(!ups.iter().any(|u| matches!(u, Up::Fail(_))), "{ups:?}");
    }

    #[tokio::test]
    async fn collect_chat_drops_reasoning() {
        let client = provider_client("test", &Timeouts::default()).expect("client");
        let base = fixed_stub(sse_of(&[
            json!({"reasoning_content": "zebra-secret-1 "}),
            json!({"content": "<thought>zebra-secret-2</thought>\n\nThe answer."}),
        ]))
        .await;
        let (_c, plan) = plan_for(&base).await;
        let got = collect_chat(&client, plan, Timeouts::default(), Duration::from_secs(10))
            .await
            .expect("collected");
        assert_eq!(got.text, "The answer.");
        assert!(got.first_byte_ms.is_some());
        // reasoning only: no text, no first byte
        let base = fixed_stub(sse_of(&[json!({"reasoning": "zebra-secret-3"})])).await;
        let (_c, plan) = plan_for(&base).await;
        let got = collect_chat(&client, plan, Timeouts::default(), Duration::from_secs(10))
            .await
            .expect("collected");
        assert_eq!(got.text, "");
        assert!(got.first_byte_ms.is_none());
    }
}
