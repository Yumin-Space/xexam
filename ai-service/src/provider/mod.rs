//! Provider adapters (spec 4): OpenAI-compatible APIs, Anthropic, and Cloudflare AI Gateway in either format.
//!
//! HTTP clients: rustls with the ring provider and the Mozilla roots compiled in, no proxy (environment proxies are
//! ignored, so keys and the internal token never go to a proxy), no redirects, HTTP/1.1 only. Every header that carries
//! a secret is marked sensitive. Upstream bodies are never logged or returned: only model ids, a success flag and the
//! fixed codes of [`errors`].
use crate::config::Timeouts;
use crate::effort::{Effort, EffortFamily};
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
pub mod responses;
pub mod sse;

pub use errors::{UpCode, UpError};

/// Largest SSE event accepted.
pub const MAX_EVENT: usize = 1 << 20;
/// Largest SSE event accepted for the Responses format: `response.completed` repeats the whole output and the
/// instructions, so it can be far larger than any other event.
pub const RESPONSES_MAX_EVENT: usize = 8 << 20;
/// Largest whole (non-streamed) answer body.
pub const MAX_WHOLE_BODY: usize = 8 << 20;
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

pub(crate) fn base_builder() -> Result<reqwest::ClientBuilder, String> {
    Ok(reqwest::Client::builder()
        .tls_backend_preconfigured(tls_config()?)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .http1_only())
}

/// The client for AI providers (per-request time limits are applied by the callers).
/// `user_agent` is what `Config::provider_user_agent` resolved from `XEXAM_AI_USER_AGENT_NAME`: `Some` is sent as is (the
/// default is `xexam-ai-service/<version>`); with `None` (`none`) it sends no `User-Agent` header at all (reqwest adds
/// none unless one is set), so gateways and providers do not learn the app name or its version.
pub fn provider_client(user_agent: Option<&str>, t: &Timeouts) -> Result<reqwest::Client, String> {
    let b = base_builder()?;
    let b = match user_agent {
        Some(ua) => b.user_agent(ua),
        None => b,
    };
    b.connect_timeout(t.connect)
        .build()
        .map_err(|_| "cannot build the HTTP client for AI providers".to_string())
}

/// The client for the main backend (plain http).
pub fn backend_client(version: &str, t: &Timeouts) -> Result<reqwest::Client, String> {
    base_builder()?
        .user_agent(format!("xexam-ai-service/{version}"))
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
            Format::Responses => format!("{}/responses", self.base),
        }
    }

    pub fn models_url(&self) -> String {
        match self.format {
            Format::OpenAi | Format::Responses => format!("{}/models", self.base),
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
                Format::OpenAi | Format::Responses => {
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
        let accept = if body.get("stream") == Some(&Value::Bool(false)) {
            "application/json"
        } else {
            "text/event-stream"
        };
        client
            .post(self.chat_url())
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, accept)
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
    /// Responses API: `"max_output_tokens"`
    MaxOutputTokens,
    /// OpenAI-compatible: no token field at all (the body of v0.9.1)
    Omit,
}

impl TokenField {
    /// The JSON key, or `None` for [`TokenField::Omit`].
    pub fn key(self) -> Option<&'static str> {
        match self {
            TokenField::MaxTokens => Some("max_tokens"),
            TokenField::MaxCompletionTokens => Some("max_completion_tokens"),
            TokenField::MaxOutputTokens => Some("max_output_tokens"),
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
        // the Responses API always reports usage in its terminal event: no usage option
        Format::Responses => responses::body(model, system, turns, token),
    }
}

/// Puts `prefix` in front of the first user turn of a built body. Anthropic: the turn becomes two text blocks, the prefix
/// with a `cache_control` breakpoint and the turn's own text after it; the `system` block carries its breakpoint
/// already, so the system text and the prefix are the cached part. OpenAI-compatible: the prefix, a blank line, the text.
pub fn put_prefix(format: Format, body: &mut Value, prefix: &str) {
    // the Responses API keeps the turns in `input`
    let key = if format == Format::Responses {
        "input"
    } else {
        "messages"
    };
    let Some(messages) = body.get_mut(key).and_then(Value::as_array_mut) else {
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
        Format::OpenAi | Format::Responses => Value::String(format!("{prefix}\n\n{own}")),
    };
}

/// Like [`put_prefix`], with cache markers for the OpenAI-compatible format too (content-part shape, which LiteLLM and
/// other Anthropic-backed gateways pass through): the first user message becomes `[{prefix + cache_control}, {own}]` and
/// the system message `[{system + cache_control}]`, two breakpoints in all. Anthropic: the same as `put_prefix`.
/// Responses: the same as `put_prefix` too, never any marker (OpenAI caches automatically and its `input_text` parts
/// do not take `cache_control`).
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

/// What a route (connection, model and format) learned about the token limit (kept in memory only, see `App`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenMemo {
    pub field: TokenField,
    /// Anthropic: the ceiling the model accepted
    pub cap: Option<i64>,
}

/// The first request's limit: the answer length converted to tokens (`want`), in the field the remembered step of this
/// route says (OpenAI-compatible), or clamped to what the model is known to accept (Anthropic: the
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
            // a memo of another format's field must never leak into this one
            field: memo
                .map(|m| m.field)
                .filter(|f| *f != TokenField::MaxOutputTokens)
                .unwrap_or(TokenField::MaxTokens),
            value: want,
        },
        Format::Responses if unlimited => TokenLimit::omit(),
        Format::Responses => TokenLimit {
            field: memo
                .map(|m| m.field)
                .filter(|f| matches!(f, TokenField::MaxOutputTokens | TokenField::Omit))
                .unwrap_or(TokenField::MaxOutputTokens),
            // the API refuses a limit below 16
            value: want.max(16),
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
            // never sent on Chat Completions: nothing left to try
            TokenField::MaxOutputTokens | TokenField::Omit => None,
        },
        Format::Responses => match sent.field {
            TokenField::Omit => None,
            _ => Some(TokenLimit {
                field: TokenField::Omit,
                value: sent.value,
            }),
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

/// `true` for any 400 / 422: when a thinking parameter was sent, every such refusal gets the one retry without it
/// (a generic body such as Workers AI's "Invalid input" names no parameter). The caller only asks when a thinking
/// parameter was sent; a structured-output refusal still reaches the schema ladders afterwards.
pub fn effort_refused(status: u16, _error_body: &str) -> bool {
    matches!(status, 400 | 422)
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
        (Format::OpenAi | Format::Responses, None) => return None,
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
    /// The app remembers the accepted step per route
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
    /// The same ladder without the thinking-strength parameter (empty when none is sent). Used once after any 400 / 422
    /// (see [`effort_refused`]).
    pub uneffort: Vec<(TokenLimit, reqwest::Request)>,
    /// The plain (unmarked) ladder without the parameter; replaces `plain` after the parameter was refused in this call.
    pub plain_uneffort: Vec<(TokenLimit, reqwest::Request)>,
    /// Called when the retry without the parameter (the `uneffort` ladder) succeeded cleanly: the route must not get it
    pub remember_uneffort: Option<Arc<dyn Fn() + Send + Sync>>,
    /// The requests ask for the whole answer at once (see [`ChatSpec::whole`])
    pub whole: bool,
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
            uneffort: Vec::new(),
            plain_uneffort: Vec::new(),
            remember_uneffort: None,
            whole: false,
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
    /// Thinking strength to send (`None` = nothing). Put into the body by [`put_effort`] regardless of the wire format.
    pub effort: Option<Effort>,
    /// Ask for the whole answer at once instead of a stream (OpenAI-compatible format only; the other formats always
    /// stream, their streams end with an explicit terminal event). The background jobs set it.
    pub whole: bool,
}

/// The `output_config` object of a body (created, or replaced when it is not an object).
fn output_config(body: &mut Value) -> &mut serde_json::Map<String, Value> {
    if !body.get("output_config").is_some_and(Value::is_object) {
        body["output_config"] = json!({});
    }
    match body.get_mut("output_config").and_then(Value::as_object_mut) {
        Some(m) => m,
        None => unreachable!("output_config was just set to an object"),
    }
}

/// The `text` object of a Responses body (created, or replaced when it is not an object).
fn text_object(body: &mut Value) -> &mut serde_json::Map<String, Value> {
    if !body.get("text").is_some_and(Value::is_object) {
        body["text"] = json!({});
    }
    match body.get_mut("text").and_then(Value::as_object_mut) {
        Some(m) => m,
        None => unreachable!("text was just set to an object"),
    }
}

/// Adds the thinking-strength parameter of `effort` to a body. Anthropic family: merges `"effort"` into
/// `output_config` (next to a structured-output `format`); OpenAI family: `"reasoning_effort": <level>`; Responses family: `"reasoning": {"effort": <level>}` (merged into an existing `reasoning` object);
/// Google family: `"extra_body": {"google": {"thinking_config": {"thinking_level": <LEVEL>}}}` (merged into
/// whatever of that path already exists; a top-level `thinking_level` is rejected by the gateway).
/// The connection's wire format does not matter (owner: the choice only changes the parameter).
pub fn put_effort(body: &mut Value, effort: Effort) {
    match effort.family() {
        EffortFamily::Anthropic => {
            output_config(body).insert("effort".into(), json!(effort.level()));
        }
        EffortFamily::OpenAi => {
            body["reasoning_effort"] = json!(effort.level());
        }
        EffortFamily::Responses => {
            if !body["reasoning"].is_object() {
                body["reasoning"] = json!({});
            }
            body["reasoning"]["effort"] = json!(effort.level());
        }
        EffortFamily::Google => {
            let mut node = &mut *body;
            for key in ["extra_body", "google", "thinking_config"] {
                if !node.is_object() {
                    *node = json!({});
                }
                node = match node {
                    Value::Object(o) => o.entry(key).or_insert_with(|| json!({})),
                    _ => unreachable!("node was just made an object"),
                };
            }
            if !node.is_object() {
                *node = json!({});
            }
            node["thinking_level"] = json!(effort.level());
        }
    }
}

/// Adds the structured output field of `schema` to a request body.
fn put_schema(format: Format, body: &mut Value, schema: &Value) {
    match format {
        Format::Anthropic => {
            output_config(body).insert(
                "format".into(),
                json!({"type": "json_schema", "schema": schema}),
            );
        }
        Format::OpenAi => {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": {"name": "answer", "strict": true, "schema": schema}
            });
        }
        Format::Responses => {
            // flat under `text.format`, no `json_schema` wrapper
            text_object(body).insert(
                "format".into(),
                json!({"type": "json_schema", "name": "answer", "strict": true, "schema": schema}),
            );
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
        Format::OpenAi | Format::Responses => {
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
    no_effort: bool,
    on_no_effort: Option<Arc<dyn Fn() + Send + Sync>>,
) -> Result<Plan, UpError> {
    let marked =
        target.format == Format::OpenAi && spec.prefix.is_some() && spec.cache && !no_markers;
    let mut steps = Vec::new();
    let mut plain = Vec::new();
    let mut unformatted = Vec::new();
    let mut uneffort = Vec::new();
    let mut plain_uneffort = Vec::new();
    let schema = spec.schema.filter(|_| !no_schema);
    let whole = spec.whole && target.format == Format::OpenAi;
    let effort = spec.effort.filter(|_| !no_effort);
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
        if whole {
            openai::make_whole(&mut body);
        }
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
        // the ladders without the thinking strength are copies taken before it is put in; `bare` never carries it
        if let Some(e) = effort {
            uneffort.push((l, target.chat_request(client, &body, spec.skip_cache)?));
            put_effort(&mut body, e);
            if let Some(b) = fallback.as_mut() {
                plain_uneffort.push((l, target.chat_request(client, b, spec.skip_cache)?));
                put_effort(b, e);
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
        uneffort,
        plain_uneffort,
        remember_uneffort: on_no_effort,
        whole,
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
    Responses(responses::State),
}

impl Handler {
    fn on_event(&mut self, ev: &sse::SseEvent, out: &mut Vec<Item>) {
        match self {
            Handler::OpenAi(s) => s.on_data(&ev.data, out),
            Handler::Anthropic(s) => s.on_event(&ev.name, &ev.data, out),
            Handler::Responses(s) => s.on_event(&ev.name, &ev.data, out),
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
            Handler::Responses(s) => s.finish(),
        }
    }
}

/// What one request ended in: it ran (its result went to the channel), or the service refused it by HTTP status before
/// any stream event (nothing was sent to the channel yet).
enum Attempt {
    /// `ok`: a 2xx response whose stream ended in `Done`
    Ran { ok: bool },
    Refused {
        status: u16,
        body: Vec<u8>,
        /// `retry-after-ms` / `retry-after` of a 429, in milliseconds
        retry_ms: Option<u64>,
    },
}

/// The final refusal of a ladder: the status, the error body and the wait the provider named in a header (429 only).
struct Refusal {
    status: u16,
    body: Vec<u8>,
    retry_ms: Option<u64>,
}

/// The failure to report for a refusal: the code of the answer, plus the wait and the daily flag of a 429.
fn refusal_error(r: &Refusal) -> UpError {
    let (retry_ms, daily) = errors::retry_hint(r.status, &r.body, r.retry_ms);
    UpError {
        retry_ms,
        daily,
        ..UpError::new(errors::from_body(r.status, &r.body), Some(r.status))
    }
}

/// `retry-after-ms` (float milliseconds), else `retry-after` (integer or float seconds; an HTTP date is ignored).
fn header_retry_ms(h: &reqwest::header::HeaderMap) -> Option<u64> {
    let num = |name: &str| -> Option<f64> {
        let v: f64 = h.get(name)?.to_str().ok()?.trim().parse().ok()?;
        (v.is_finite() && (0.0..=1.0e9).contains(&v)).then_some(v)
    };
    num("retry-after-ms")
        .map(|ms| ms.round() as u64)
        .or_else(|| num("retry-after").map(|s| (s * 1000.0).round() as u64))
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
    if let Attempt::Refused {
        status,
        body,
        retry_ms,
    } = stream_once(client, req, format, false, t, &tx).await
    {
        let _ = tx.send(Up::Fail(refusal_error(&Refusal {
            status,
            body,
            retry_ms,
        })));
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
        uneffort,
        plain_uneffort,
        remember_uneffort,
        whole,
    } = plan;
    let args = (client, format, whole, stored_max, t, &tx, &remember);
    let (mut last, mut at) = match run_ladder(args, steps, 0).await {
        Ok(_) => return,
        Err(r) => r,
    };
    let remember_dropped = || {
        if let Some(r) = &remember_uneffort {
            r();
        }
    };
    // any 400 / 422 while a thinking strength was sent: once without it, same token step. Only a clean answer of this
    // retry is remembered: the plain / unformatted ladders below also leave the parameter out, so their success says
    // nothing about it (a generic refusal may have been about the markers or the schema)
    let mut dropped = false;
    if !uneffort.is_empty() && effort_refused(last.status, &String::from_utf8_lossy(&last.body)) {
        match run_ladder(args, uneffort, at).await {
            Ok(ok) => {
                if ok {
                    remember_dropped();
                }
                return;
            }
            Err((r, i)) => {
                (last, at) = (r, i);
                dropped = true;
            }
        }
    }
    let plain = if dropped { plain_uneffort } else { plain };
    if !plain.is_empty() && matches!(last.status, 400 | 422) {
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
            Err((r, i)) => (last, at) = (r, i),
        }
    }
    if !unformatted.is_empty() && matches!(last.status, 400 | 422) {
        match run_ladder(args, unformatted, at).await {
            Ok(ok) => {
                if ok {
                    if let Some(r) = &remember_unformatted {
                        r();
                    }
                }
                return;
            }
            Err((r, _)) => last = r,
        }
    }
    let _ = tx.send(Up::Fail(refusal_error(&last)));
}

type LadderArgs<'a> = (
    &'a reqwest::Client,
    Format,
    bool,
    i64,
    Timeouts,
    &'a UnboundedSender<Up>,
    &'a Option<Remember>,
);

/// Runs the steps of a ladder from index `skip`; `Ok(clean)` when a request ran (its result went to the channel;
/// `clean` = 2xx and finished with `Done`), else the
/// final refusal (status, body, index of the step).
async fn run_ladder(
    (client, format, whole, stored_max, t, tx, remember): LadderArgs<'_>,
    steps: Vec<(TokenLimit, reqwest::Request)>,
    skip: usize,
) -> Result<bool, (Refusal, usize)> {
    let total = steps.len();
    for (i, (sent, req)) in steps.into_iter().enumerate().skip(skip) {
        match stream_once(client, req, format, whole, t, tx).await {
            Attempt::Ran { ok } => {
                if i > 0 {
                    if let Some(r) = remember {
                        r(&sent);
                    }
                }
                return Ok(ok);
            }
            Attempt::Refused {
                status,
                body,
                retry_ms,
            } => {
                let text = String::from_utf8_lossy(&body);
                let retry = i + 1 < total
                    && next_token_limit(format, &sent, status, &text, stored_max).is_some();
                if !retry {
                    return Err((
                        Refusal {
                            status,
                            body,
                            retry_ms,
                        },
                        i,
                    ));
                }
            }
        }
    }
    Err((
        Refusal {
            status: 500,
            body: Vec::new(),
            retry_ms: None,
        },
        total,
    ))
}

async fn stream_once(
    client: &reqwest::Client,
    req: reqwest::Request,
    format: Format,
    whole: bool,
    t: Timeouts,
    tx: &UnboundedSender<Up>,
) -> Attempt {
    let fail = |code: UpCode, http: Option<u16>| Up::Fail(UpError::new(code, http));
    // a whole answer sends its headers only when the model has finished: wait as long as a whole chat answer may take
    let wait = if whole {
        t.total.max(t.first_byte)
    } else {
        t.first_byte
    };
    let mut resp = match tokio::time::timeout(wait, client.execute(req)).await {
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
        let retry_ms = if status == 429 {
            header_retry_ms(resp.headers())
        } else {
            None
        };
        let body = if resp.status().is_redirection() {
            Vec::new()
        } else {
            read_capped(&mut resp, MAX_ERROR_BODY, t.idle)
                .await
                .unwrap_or_default()
        };
        return Attempt::Refused {
            status,
            body,
            retry_ms,
        };
    }
    let mut parser = sse::SseParser::new(if format == Format::Responses {
        RESPONSES_MAX_EVENT
    } else {
        MAX_EVENT
    });
    let mut handler = match format {
        Format::OpenAi => Handler::OpenAi(openai::State::default()),
        Format::Anthropic => Handler::Anthropic(anthropic::State::default()),
        Format::Responses => Handler::Responses(responses::State::default()),
    };
    let mut chars = 0usize;
    // a whole answer (a JSON body, not an event stream: a server that ignores `"stream": false` still streams) is read
    // in one piece and handed to the stream state as one chunk
    let is_stream = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("text/event-stream"));
    if whole && format == Format::OpenAi && !is_stream {
        let body = match read_whole(&mut resp, t.idle).await {
            Ok(b) => b,
            Err(code) => {
                let _ = tx.send(fail(code, Some(status)));
                return Attempt::Ran { ok: false };
            }
        };
        let mut items = Vec::new();
        let mut state = openai::State::default();
        match serde_json::from_slice::<Value>(&body) {
            Ok(v) => {
                state.on_data(&openai::whole_as_chunk(&v).to_string(), &mut items);
                if !matches!(items.last(), Some(Item::Done | Item::Fail(_))) {
                    state.on_data("[DONE]", &mut items);
                }
            }
            Err(_) => items.push(Item::Fail(UpCode::Protocol)),
        }
        return send_items(items, tx, status, &mut chars).unwrap_or(Attempt::Ran { ok: false });
    }
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

/// Reads a whole answer body (each read within `idle`): a stall is `Timeout`, a broken connection the code of the
/// network error, a body over [`MAX_WHOLE_BODY`] `TooLong`.
async fn read_whole(resp: &mut reqwest::Response, idle: Duration) -> Result<Vec<u8>, UpCode> {
    let mut buf = Vec::new();
    loop {
        match tokio::time::timeout(idle, resp.chunk()).await {
            Err(_) => return Err(UpCode::Timeout),
            Ok(Err(e)) => return Err(errors::from_reqwest(&e)),
            Ok(Ok(None)) => return Ok(buf),
            Ok(Ok(Some(b))) => {
                if buf.len() + b.len() > MAX_WHOLE_BODY {
                    return Err(UpCode::TooLong);
                }
                buf.extend_from_slice(&b);
            }
        }
    }
}

/// Sends the items of a whole answer to the channel (the same rules as the stream loop); `Some` once it ended.
fn send_items(
    items: Vec<Item>,
    tx: &UnboundedSender<Up>,
    status: u16,
    chars: &mut usize,
) -> Option<Attempt> {
    let fail = |code: UpCode, http: Option<u16>| Up::Fail(UpError::new(code, http));
    for item in items {
        let up = match item {
            Item::Text(s) => {
                *chars += s.chars().count();
                if *chars > MAX_ANSWER_CHARS {
                    let _ = tx.send(fail(UpCode::TooLong, Some(status)));
                    return Some(Attempt::Ran { ok: false });
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
                return Some(Attempt::Ran { ok: true });
            }
            Item::Fail(code) => {
                let _ = tx.send(fail(code, Some(status)));
                return Some(Attempt::Ran { ok: false });
            }
        };
        let _ = tx.send(up);
    }
    None
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
    if format != Format::Anthropic {
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
        let client = provider_client(None, &Timeouts::default()).expect("client");
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
        assert!(provider_client(None, &t).is_ok());
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
        for f in [Format::Anthropic, Format::OpenAi, Format::Responses] {
            let (mut x, mut y) = (
                chat_body(f, "m", "rules", &a, true, TokenLimit::max_tokens(900)),
                chat_body(f, "m", "rules", &b, true, TokenLimit::max_tokens(900)),
            );
            put_prefix(f, &mut x, "PREFIX-BYTES");
            put_prefix(f, &mut y, "PREFIX-BYTES");
            let head = |v: &Value| match f {
                Format::Anthropic => v["messages"][0]["content"][0].to_string(),
                Format::OpenAi => v["messages"][1]["content"].as_str().unwrap()[..12].to_string(),
                Format::Responses => v["input"][0]["content"].as_str().unwrap()[..12].to_string(),
            };
            assert_eq!(head(&x), head(&y));
            assert_eq!(x["system"], y["system"]);
            assert_eq!(x["instructions"], y["instructions"]);
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
            effort: None,
            whole: false,
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

    /// Reads one request head from a local listener and answers 200, so the test sees the headers that really
    /// go over the wire (reqwest merges client-wide headers only when a request is sent).
    async fn wire_head(client: reqwest::Client) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().expect("addr")
        );
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept");
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = sock.read(&mut chunk).expect("read");
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                .expect("write");
            String::from_utf8_lossy(&buf).to_ascii_lowercase()
        });
        client.post(url).body("{}").send().await.expect("send");
        server.join().expect("server")
    }

    #[tokio::test]
    async fn none_sends_no_user_agent() {
        let head = wire_head(provider_client(None, &Timeouts::default()).expect("client")).await;
        assert!(head.starts_with("post /v1/chat/completions"), "{head}");
        assert!(!head.contains("user-agent"), "{head}");
        assert!(!head.contains("xexam"), "{head}");
    }

    #[tokio::test]
    async fn the_default_user_agent_names_the_service() {
        let mut cfg = crate::config::Config::from_env_with(&|k| {
            (k == "XEXAM_VERSION").then(|| "9.9".to_string())
        });
        assert_eq!(
            cfg.provider_user_agent().as_deref(),
            Some("xexam-ai-service/9.9")
        );
        let ua = cfg.provider_user_agent();
        let head =
            wire_head(provider_client(ua.as_deref(), &Timeouts::default()).expect("client")).await;
        assert!(head.contains("user-agent: xexam-ai-service/9.9"), "{head}");
        cfg.ai_user_agent = crate::config::AiUserAgent::Off;
        assert_eq!(cfg.provider_user_agent(), None);
    }

    #[tokio::test]
    async fn a_configured_user_agent_is_sent_as_is() {
        let ua = Some("MyGateway-Client/2.0 (made up)");
        let head = wire_head(provider_client(ua, &Timeouts::default()).expect("client")).await;
        assert!(
            head.contains("user-agent: mygateway-client/2.0 (made up)\r\n"),
            "{head}"
        );
        assert!(!head.contains("xexam"), "{head}");
    }

    #[test]
    fn retry_headers_of_a_429_are_read_in_milliseconds() {
        let map = |pairs: &[(&str, &str)]| {
            let mut h = reqwest::header::HeaderMap::new();
            for (k, v) in pairs {
                h.insert(
                    reqwest::header::HeaderName::from_bytes(k.as_bytes()).expect("name"),
                    v.parse().expect("value"),
                );
            }
            h
        };
        assert_eq!(header_retry_ms(&map(&[("retry-after", "3")])), Some(3000));
        assert_eq!(header_retry_ms(&map(&[("retry-after", "1.5")])), Some(1500));
        assert_eq!(
            header_retry_ms(&map(&[("retry-after-ms", "250.4"), ("retry-after", "9")])),
            Some(250)
        );
        // an HTTP date or junk is ignored
        assert_eq!(
            header_retry_ms(&map(&[("retry-after", "Wed, 21 Oct 2026 07:28:00 GMT")])),
            None
        );
        assert_eq!(header_retry_ms(&map(&[("retry-after", "-4")])), None);
        assert_eq!(header_retry_ms(&map(&[])), None);
    }

    #[tokio::test]
    async fn backend_requests_still_name_the_ai_service() {
        let head = wire_head(backend_client("9.9", &Timeouts::default()).expect("client")).await;
        assert!(head.contains("user-agent: xexam-ai-service/9.9"), "{head}");
    }

    #[tokio::test]
    async fn a_schema_adds_the_structured_output_field_and_an_unformatted_ladder() {
        let turns = one_turn("TASK");
        let client = provider_client(None, &Timeouts::default()).expect("client");
        let schema = json!({"type": "object"});
        let first = TokenLimit::omit();
        let build = |t: &Target, with: bool, no: bool| {
            let mut sp = spec(&turns, None, false);
            sp.schema = with.then_some(&schema);
            build_plan(
                t, &client, &sp, first, 8192, None, false, None, no, None, false, None,
            )
            .expect("plan")
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
            assert_eq!(b["response_format"]["json_schema"]["name"], "answer");
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
            &oa, &client, &sp, first, 8192, None, false, None, false, None, false, None,
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
        let client = provider_client(None, &Timeouts::default()).expect("client");
        let turns = one_turn("TASK now");
        let t = openai_target("http://127.0.0.1:9/v1");
        let first = TokenLimit::omit();
        let plan = |t: &Target, sp: &ChatSpec<'_>, no: bool| {
            build_plan(
                t, &client, sp, first, 8192, None, no, None, false, None, false, None,
            )
            .expect("plan")
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
        let client = provider_client(None, &Timeouts::default()).expect("client");
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
        let client = provider_client(None, &Timeouts::default()).expect("client");
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
        let client = provider_client(None, &Timeouts::default()).expect("client");
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
        let client = provider_client(None, &Timeouts::default()).expect("client");
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
        let client = provider_client(None, &Timeouts::default()).expect("client");
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
                    false,
                    None,
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
        let client = provider_client(None, &Timeouts::default()).expect("client");
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
        let client = provider_client(None, &Timeouts::default()).expect("client");
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

    fn eff(f: EffortFamily, l: &str) -> Effort {
        Effort::new(f, l).expect("level")
    }

    fn plain_body(format: Format) -> Value {
        let turns = one_turn("T");
        chat_body(format, "m", "rules", &turns, true, TokenLimit::omit())
    }

    #[test]
    fn anthropic_family_puts_output_config_effort() {
        let mut b = plain_body(Format::Anthropic);
        put_effort(&mut b, eff(EffortFamily::Anthropic, "xhigh"));
        assert_eq!(b["output_config"], json!({"effort": "xhigh"}));
        assert!(b.get("reasoning_effort").is_none());
    }

    #[test]
    fn openai_family_puts_reasoning_effort() {
        let mut b = plain_body(Format::OpenAi);
        put_effort(&mut b, eff(EffortFamily::OpenAi, "low"));
        assert_eq!(b["reasoning_effort"], "low");
        assert!(b.get("output_config").is_none());
    }

    #[test]
    fn responses_family_puts_nested_reasoning_effort() {
        let mut b = plain_body(Format::OpenAi);
        put_effort(&mut b, eff(EffortFamily::Responses, "low"));
        assert_eq!(b["reasoning"], json!({"effort": "low"}));
        assert!(b.get("reasoning_effort").is_none());
        assert!(b.get("output_config").is_none());
        // merged into an existing reasoning object
        let mut c = plain_body(Format::OpenAi);
        c["reasoning"] = json!({"summary": "auto", "effort": "high"});
        put_effort(&mut c, eff(EffortFamily::Responses, "max"));
        assert_eq!(c["reasoning"], json!({"summary": "auto", "effort": "max"}));
        // a non-object value is replaced
        let mut d = plain_body(Format::OpenAi);
        d["reasoning"] = json!("x");
        put_effort(&mut d, eff(EffortFamily::Responses, "none"));
        assert_eq!(d["reasoning"], json!({"effort": "none"}));
    }

    #[test]
    fn google_family_puts_extra_body_thinking_level_only() {
        let mut b = plain_body(Format::OpenAi);
        put_effort(&mut b, eff(EffortFamily::Google, "MINIMAL"));
        assert_eq!(
            b["extra_body"],
            json!({"google": {"thinking_config": {"thinking_level": "MINIMAL"}}})
        );
        for k in ["reasoning_effort", "thinking_level", "output_config"] {
            assert!(b.get(k).is_none(), "{k}");
        }
    }

    #[test]
    fn google_family_merges_into_existing_extra_body() {
        let mut b = plain_body(Format::OpenAi);
        b["extra_body"] =
            json!({"x": 1, "google": {"a": 2, "thinking_config": {"include_thoughts": true}}});
        put_effort(&mut b, eff(EffortFamily::Google, "XHIGH"));
        assert_eq!(
            b["extra_body"],
            json!({"x": 1, "google": {"a": 2, "thinking_config": {"include_thoughts": true, "thinking_level": "XHIGH"}}})
        );
        put_effort(&mut b, eff(EffortFamily::Google, "LOW"));
        assert_eq!(
            b["extra_body"]["google"]["thinking_config"]["thinking_level"],
            "LOW"
        );
        // a non-object in the way is replaced
        let mut c = plain_body(Format::OpenAi);
        c["extra_body"] = json!("junk");
        put_effort(&mut c, eff(EffortFamily::Google, "HIGH"));
        assert_eq!(
            c["extra_body"]["google"]["thinking_config"]["thinking_level"],
            "HIGH"
        );
    }

    #[test]
    fn google_high_and_minimal_send_thinking_level_only() {
        for l in ["HIGH", "MINIMAL"] {
            let mut b = plain_body(Format::OpenAi);
            put_effort(&mut b, eff(EffortFamily::Google, l));
            assert_eq!(
                b["extra_body"],
                json!({"google": {"thinking_config": {"thinking_level": l}}})
            );
            let s = b.to_string();
            assert!(!s.contains("chat_template_kwargs") && !s.contains("enable_thinking"));
            assert!(b.get("reasoning_effort").is_none());
        }
    }

    #[test]
    fn effort_merges_with_schema_format_in_output_config() {
        let schema = json!({"type": "object"});
        let e = eff(EffortFamily::Anthropic, "high");
        let mut a = plain_body(Format::Anthropic);
        put_schema(Format::Anthropic, &mut a, &schema);
        put_effort(&mut a, e);
        let mut b = plain_body(Format::Anthropic);
        put_effort(&mut b, e);
        put_schema(Format::Anthropic, &mut b, &schema);
        for x in [a, b] {
            assert_eq!(x["output_config"]["effort"], "high");
            assert_eq!(x["output_config"]["format"]["type"], "json_schema");
            assert_eq!(x["output_config"].as_object().expect("obj").len(), 2);
        }
    }

    #[test]
    fn no_effort_sends_no_parameter() {
        let turns = one_turn("T");
        let client = provider_client(None, &Timeouts::default()).expect("client");
        let sp = spec(&turns, None, false);
        let oa = openai_target("http://127.0.0.1:9");
        let p = build_plan(
            &oa,
            &client,
            &sp,
            TokenLimit::omit(),
            8192,
            None,
            false,
            None,
            false,
            None,
            false,
            None,
        )
        .expect("plan");
        for (_, r) in &p.steps {
            let b = body_of(r);
            assert!(b.get("reasoning_effort").is_none() && b.get("output_config").is_none());
        }
        assert!(p.uneffort.is_empty() && p.plain_uneffort.is_empty());
    }

    #[test]
    fn mismatched_family_still_sends_its_parameter() {
        let mut oa = plain_body(Format::OpenAi);
        put_effort(&mut oa, eff(EffortFamily::Anthropic, "low"));
        assert_eq!(oa["output_config"]["effort"], "low");
        let mut an = plain_body(Format::Anthropic);
        put_effort(&mut an, eff(EffortFamily::OpenAi, "low"));
        assert_eq!(an["reasoning_effort"], "low");
    }

    #[test]
    fn build_plan_uneffort_ladders() {
        let turns = one_turn("TASK");
        let client = provider_client(None, &Timeouts::default()).expect("client");
        let schema = json!({"type": "object"});
        let oa = openai_target("http://127.0.0.1:9");
        let an = target(
            Provider::Anthropic,
            Format::Anthropic,
            GatewayAuth::Bearer,
            true,
            false,
        );
        let build = |t: &Target, e: Option<Effort>, no: bool, marks: bool| {
            let mut sp = spec(&turns, marks.then_some("PFX"), marks);
            sp.schema = Some(&schema);
            sp.effort = e;
            // the 8192 ceiling gives the Anthropic ladder more than one step
            build_plan(
                t,
                &client,
                &sp,
                TokenLimit::max_tokens(8192),
                4096,
                None,
                false,
                None,
                false,
                None,
                no,
                None,
            )
            .expect("plan")
        };
        let has = |r: &reqwest::Request| {
            let s = body_of(r).to_string();
            s.contains("reasoning_effort") || s.contains("\"effort\"")
        };
        // OpenAI with markers: steps and plain carry it; uneffort, plain_uneffort and unformatted do not
        let e = eff(EffortFamily::OpenAi, "high");
        let p = build(&oa, Some(e), false, true);
        assert!(!p.steps.is_empty() && !p.plain.is_empty());
        assert!(p.steps.iter().chain(&p.plain).all(|(_, r)| has(r)));
        assert_eq!(p.uneffort.len(), p.steps.len());
        assert_eq!(p.plain_uneffort.len(), p.plain.len());
        assert!(!p.unformatted.is_empty());
        assert!(p
            .uneffort
            .iter()
            .chain(&p.plain_uneffort)
            .chain(&p.unformatted)
            .all(|(_, r)| !has(r)));
        // the plain-uneffort ladder is unmarked, the uneffort one still marked
        assert!(!body_of(&p.plain_uneffort[0].1)
            .to_string()
            .contains("cache_control"));
        assert!(body_of(&p.uneffort[0].1)
            .to_string()
            .contains("cache_control"));
        assert!(body_of(&p.uneffort[0].1).get("response_format").is_some());
        // Anthropic: output_config holds format and effort; the uneffort copy keeps the format only
        let e = eff(EffortFamily::Anthropic, "max");
        let p = build(&an, Some(e), false, false);
        let b = body_of(&p.steps[0].1);
        assert_eq!(b["output_config"]["effort"], "max");
        assert!(b["output_config"]["format"].is_object());
        let u = body_of(&p.uneffort[0].1);
        assert!(
            u["output_config"].get("effort").is_none() && u["output_config"]["format"].is_object()
        );
        assert!(p.unformatted.iter().all(|(_, r)| !has(r)));
        // none / no_effort: everything empty and nothing sent
        for p in [
            build(&oa, None, false, true),
            build(&oa, Some(e), true, true),
        ] {
            assert!(p.uneffort.is_empty() && p.plain_uneffort.is_empty());
            assert!(p.steps.iter().chain(&p.plain).all(|(_, r)| !has(r)));
        }
    }

    #[test]
    fn effort_refused_detection() {
        let param = r#"{"error":{"message":"Unknown parameter","param":"reasoning_effort"}}"#;
        assert!(effort_refused(400, param));
        assert!(effort_refused(422, "{}"));
        // a generic Workers AI body names no parameter and still counts
        let workers =
            r#"{"errors":[{"message":"AiError: AiError: Invalid input (abc)","code":8001}]}"#;
        assert!(effort_refused(400, workers));
        assert!(effort_refused(400, "not json"));
        let google = r#"[{"error":{"code":400,"message":"Unknown name \"thinking_level\": Cannot find field."}}]"#;
        assert!(effort_refused(400, google));
        for st in [200, 401, 404, 429, 500, 503] {
            assert!(!effort_refused(st, param), "{st}");
        }
    }

    #[test]
    fn chat_body_has_no_effort() {
        for f in [Format::OpenAi, Format::Anthropic, Format::Responses] {
            let b = plain_body(f);
            assert!(b.get("reasoning_effort").is_none() && b.get("output_config").is_none());
            assert!(b.get("reasoning").is_none() && b.get("text").is_none());
        }
    }

    fn responses_target(base: &str) -> Target {
        let mut t = target(
            Provider::Gateway,
            Format::Responses,
            GatewayAuth::Bearer,
            true,
            false,
        );
        t.base = base.to_string();
        t
    }

    #[test]
    fn responses_urls_and_headers() {
        let t = responses_target("https://gw.example.com/v1");
        assert_eq!(t.chat_url(), "https://gw.example.com/v1/responses");
        assert_eq!(t.models_url(), "https://gw.example.com/v1/models");
        let h = t.auth_headers().expect("headers");
        assert_eq!(h.get("authorization").map(|v| v.is_sensitive()), Some(true));
        assert!(h.get("anthropic-version").is_none() && h.get("x-api-key").is_none());
        assert_eq!(h.len(), 1);
        // a gateway key as well
        let mut g = target(
            Provider::Gateway,
            Format::Responses,
            GatewayAuth::CfAig,
            true,
            true,
        );
        g.base = "https://gw.example.com/v1".into();
        let h = g.auth_headers().expect("headers");
        assert!(h.get("authorization").is_some() && h.get("cf-aig-authorization").is_some());
        let body = br#"{"data":[{"id":"demo-b"},{"id":"demo-a"}]}"#;
        let l = parse_models(body, Format::Responses).expect("list");
        assert_eq!(l.models, vec!["demo-a".to_string(), "demo-b".to_string()]);
    }

    #[test]
    fn responses_token_limits_and_ladder() {
        let f = Format::Responses;
        let mot = |v| TokenLimit {
            field: TokenField::MaxOutputTokens,
            value: v,
        };
        let omit = TokenLimit {
            field: TokenField::Omit,
            value: 1500,
        };
        // unlimited sends nothing; a small value is raised to 16; the memo is honoured for the Responses fields only
        assert_eq!(
            first_token_limit(f, crate::limits::UNLIMITED_TOKENS, 8192, None),
            TokenLimit::omit()
        );
        assert_eq!(first_token_limit(f, 1500, 8192, None), mot(1500));
        assert_eq!(first_token_limit(f, 3, 8192, None), mot(16));
        let memo = |field| Some(TokenMemo { field, cap: None });
        assert_eq!(
            first_token_limit(f, 1500, 8192, memo(TokenField::Omit)).field,
            TokenField::Omit
        );
        for stale in [TokenField::MaxTokens, TokenField::MaxCompletionTokens] {
            assert_eq!(
                first_token_limit(f, 1500, 8192, memo(stale)).field,
                TokenField::MaxOutputTokens
            );
        }
        // a Responses memo never reaches Chat Completions
        assert_eq!(
            first_token_limit(
                Format::OpenAi,
                1500,
                8192,
                memo(TokenField::MaxOutputTokens)
            )
            .field,
            TokenField::MaxTokens
        );
        // the ladder: max_output_tokens, then nothing
        assert_eq!(ladder_after(f, &mot(1500), 8192), Some(omit));
        assert_eq!(ladder_after(f, &omit, 8192), None);
        assert_eq!(ladder_after(Format::OpenAi, &mot(5), 8192), None);
        let refused = r#"{"error":{"message":"Unsupported parameter: 'max_output_tokens'","type":"invalid_request_error","param":"max_output_tokens","code":"unsupported_parameter"}}"#;
        assert_eq!(
            next_token_limit(f, &mot(1500), 400, refused, 8192),
            Some(omit)
        );
        assert_eq!(next_token_limit(f, &omit, 400, refused, 8192), None);
        assert_eq!(next_token_limit(f, &mot(1500), 500, refused, 8192), None);
        assert_eq!(
            next_token_limit(
                f,
                &mot(1500),
                400,
                r#"{"error":{"param":"max_tokens"}}"#,
                8192
            ),
            None
        );
    }

    #[test]
    fn responses_body_carries_only_its_token_field() {
        let turns = one_turn("T");
        for (field, sent) in [
            (TokenField::MaxOutputTokens, true),
            (TokenField::Omit, false),
        ] {
            let b = chat_body(
                Format::Responses,
                "m",
                "rules",
                &turns,
                true,
                TokenLimit { field, value: 1500 },
            );
            assert_eq!(b.get("max_output_tokens").is_some(), sent);
            for k in ["max_tokens", "max_completion_tokens", "stream_options"] {
                assert!(b.get(k).is_none(), "{k}");
            }
            assert_eq!(b["stream"], json!(true));
            assert_eq!(b["store"], json!(false));
        }
    }

    #[test]
    fn responses_schema_goes_to_text_format() {
        let schema = json!({"type": "object"});
        let mut b = plain_body(Format::Responses);
        put_schema(Format::Responses, &mut b, &schema);
        assert_eq!(
            b["text"],
            json!({"format": {"type": "json_schema", "name": "answer", "strict": true, "schema": schema}})
        );
        assert!(b.get("response_format").is_none() && b.get("output_config").is_none());
        // a non-object `text` is replaced, an existing one is kept and extended
        let mut c = plain_body(Format::Responses);
        c["text"] = json!("x");
        put_schema(Format::Responses, &mut c, &schema);
        assert_eq!(c["text"]["format"]["name"], "answer");
        let mut d = plain_body(Format::Responses);
        d["text"] = json!({"verbosity": "low"});
        put_schema(Format::Responses, &mut d, &schema);
        assert_eq!(d["text"]["verbosity"], "low");
        assert_eq!(d["text"]["format"]["strict"], true);
    }

    #[test]
    fn responses_effort_families_are_not_translated() {
        let mut b = plain_body(Format::Responses);
        put_effort(&mut b, eff(EffortFamily::Responses, "high"));
        assert_eq!(b["reasoning"], json!({"effort": "high"}));
        assert!(b["reasoning"].get("summary").is_none());
        let mut c = plain_body(Format::Responses);
        put_effort(&mut c, eff(EffortFamily::OpenAi, "low"));
        assert_eq!(c["reasoning_effort"], "low");
        assert!(c.get("reasoning").is_none());
        let mut d = plain_body(Format::Responses);
        put_effort(&mut d, eff(EffortFamily::Anthropic, "low"));
        assert_eq!(d["output_config"], json!({"effort": "low"}));
    }

    #[test]
    fn responses_prefix_goes_into_the_first_input_item_without_markers() {
        let turns = one_turn("TASK now");
        let plain = chat_body(
            Format::Responses,
            "m",
            "rules",
            &turns,
            true,
            TokenLimit::omit(),
        );
        for marked in [false, true] {
            let mut b = plain.clone();
            if marked {
                put_prefix_marked(Format::Responses, &mut b, "FILE\nL1: made-up line");
            } else {
                put_prefix(Format::Responses, &mut b, "FILE\nL1: made-up line");
            }
            assert_eq!(
                b["input"][0]["content"],
                json!("FILE\nL1: made-up line\n\nTASK now")
            );
            assert!(b.get("messages").is_none());
            assert_eq!(b["instructions"], json!("rules"));
            assert!(!b.to_string().contains("cache_control"));
        }
        let mut s = plain.clone();
        put_prefix(Format::Responses, &mut s, "P");
        strip_cache_markers(&mut s);
        assert_eq!(s["input"][0]["content"], json!("P\n\nTASK now"));
    }

    #[tokio::test]
    async fn a_responses_plan_has_a_two_step_ladder_and_no_marker_ladder() {
        let turns = one_turn("TASK");
        let client = provider_client(None, &Timeouts::default()).expect("client");
        let schema = json!({"type": "object"});
        let t = responses_target("http://127.0.0.1:9/x");
        let mut sp = spec(&turns, Some("PFX"), true);
        sp.schema = Some(&schema);
        sp.effort = Some(eff(EffortFamily::Responses, "low"));
        let first = TokenLimit {
            field: TokenField::MaxOutputTokens,
            value: 800,
        };
        let p = build_plan(
            &t, &client, &sp, first, 8192, None, false, None, false, None, false, None,
        )
        .expect("plan");
        assert_eq!(p.steps.len(), 2);
        assert!(p.plain.is_empty() && p.plain_uneffort.is_empty());
        assert_eq!(p.uneffort.len(), 2);
        assert_eq!(p.unformatted.len(), 2);
        let b = body_of(&p.steps[0].1);
        assert_eq!(b["max_output_tokens"], json!(800));
        assert_eq!(b["reasoning"], json!({"effort": "low"}));
        assert_eq!(b["text"]["format"]["name"], "answer");
        assert_eq!(b["input"][0]["content"], json!("PFX\n\nTASK"));
        assert!(!b.to_string().contains("cache_control"));
        let b = body_of(&p.steps[1].1);
        assert!(b.get("max_output_tokens").is_none());
        let b = body_of(&p.uneffort[0].1);
        assert!(b.get("reasoning").is_none() && b.get("text").is_some());
        let b = body_of(&p.unformatted[0].1);
        assert!(b.get("text").is_none() && b.get("reasoning").is_none());
        assert_eq!(p.steps[0].1.url().path(), "/x/responses");
    }

    #[tokio::test]
    async fn a_responses_stream_with_a_huge_completed_event_ends_in_done() {
        let big = "x".repeat(2 << 20);
        let completed = json!({"type": "response.completed", "response": {
            "instructions": big,
            "output": [{"type": "message", "content": [{"type": "output_text", "text": "Hello"}]}],
            "usage": {"input_tokens": 10, "output_tokens": 2, "input_tokens_details": {"cached_tokens": 4}}
        }});
        let mut sse = String::new();
        for (n, d) in [
            ("response.created", json!({"type": "response.created"})),
            (
                "response.output_text.delta",
                json!({"type": "response.output_text.delta", "delta": "Hello"}),
            ),
            ("response.completed", completed),
        ] {
            sse.push_str(&format!("event: {n}\ndata: {d}\n\n"));
        }
        let base = fixed_stub(sse).await;
        let client = provider_client(None, &Timeouts::default()).expect("client");
        let turns = one_turn("TASK now");
        let t = responses_target(&base);
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
            false,
            None,
        )
        .expect("plan");
        let ups = run(plan).await;
        assert_eq!(
            ups,
            vec![
                Up::Text("Hello".into()),
                Up::Usage {
                    input: Some(10),
                    output: Some(2)
                },
                Up::Cached(4),
                Up::Done
            ]
        );
    }
}
