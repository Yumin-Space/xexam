//! AI settings: providers, the outbound URL policy (4.2), the fingerprint (2.3) and the validation of an update (5.3).
use crate::secret::{self, SecretName};
use crate::util::hex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const DEFAULT_OPENAI: &str = "https://api.openai.com/v1";
pub const DEFAULT_ANTHROPIC: &str = "https://api.anthropic.com";
/// `anthropic-version` header value (one place)
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const MAX_URL_CHARS: usize = 300;
pub const MAX_MODEL_CHARS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenAi,
    Anthropic,
    Gateway,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::OpenAi => "openai",
            Provider::Anthropic => "anthropic",
            Provider::Gateway => "gateway",
        }
    }

    pub fn parse(s: &str) -> Option<Provider> {
        match s {
            "openai" => Some(Provider::OpenAi),
            "anthropic" => Some(Provider::Anthropic),
            "gateway" => Some(Provider::Gateway),
            _ => None,
        }
    }

    /// The name used in log lines.
    pub fn display(self) -> &'static str {
        match self {
            Provider::OpenAi => "OpenAI-compatible API",
            Provider::Anthropic => "Anthropic",
            Provider::Gateway => "Cloudflare AI Gateway",
        }
    }

    /// The key this provider cannot work without (`None`: no key required, e.g. a local OpenAI-compatible server).
    pub fn required_key(self) -> Option<SecretName> {
        match self {
            Provider::OpenAi => None,
            Provider::Anthropic => Some(SecretName::ApiKey),
            Provider::Gateway => Some(SecretName::GatewayKey),
        }
    }
}

/// Wire format of the requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    OpenAi,
    Anthropic,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Format::OpenAi => "openai",
            Format::Anthropic => "anthropic",
        }
    }

    pub fn parse(s: &str) -> Option<Format> {
        match s {
            "openai" => Some(Format::OpenAi),
            "anthropic" => Some(Format::Anthropic),
            _ => None,
        }
    }
}

/// How the gateway key is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayAuth {
    Bearer,
    XApiKey,
    CfAig,
}

impl GatewayAuth {
    pub fn as_str(self) -> &'static str {
        match self {
            GatewayAuth::Bearer => "bearer",
            GatewayAuth::XApiKey => "x-api-key",
            GatewayAuth::CfAig => "cf-aig-authorization",
        }
    }

    pub fn parse(s: &str) -> Option<GatewayAuth> {
        match s {
            "bearer" => Some(GatewayAuth::Bearer),
            "x-api-key" => Some(GatewayAuth::XApiKey),
            "cf-aig-authorization" => Some(GatewayAuth::CfAig),
            _ => None,
        }
    }
}

/// The `settings` row. Never holds a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub provider: Provider,
    pub base_url: String,
    /// Stored format column (used only for a gateway; see [`Settings::wire_format`])
    pub format: Format,
    pub gateway_auth: GatewayAuth,
    pub model: String,
    pub manual_model: bool,
    /// OpenAI format: send `stream_options.include_usage`
    pub usage_option: bool,
    /// Anthropic format: `max_tokens` (8192 or 4096)
    pub max_tokens: i64,
    pub enabled: bool,
    pub verified_at: Option<i64>,
    pub verified_fp: Option<String>,
    pub retention_days: i64,
    pub updated_at: i64,
    pub updated_by: String,
}

impl Settings {
    /// The wire format in effect.
    pub fn wire_format(&self) -> Format {
        match self.provider {
            Provider::OpenAi => Format::OpenAi,
            Provider::Anthropic => Format::Anthropic,
            Provider::Gateway => self.format,
        }
    }
}

/// What is known about the two saved keys (no key material).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyRevs {
    /// `Some(rev)` when the slot is set
    pub api: Option<i64>,
    pub gateway: Option<i64>,
}

impl KeyRevs {
    pub fn get(&self, name: SecretName) -> Option<i64> {
        match name {
            SecretName::ApiKey => self.api,
            SecretName::GatewayKey => self.gateway,
        }
    }
}

/// `hex(SHA-256(provider, base_url, format, gateway_auth, model, key revisions, key id))` (2.3). No key material.
pub fn fingerprint(s: &Settings, revs: KeyRevs, key_id: &str) -> String {
    let gw = s.provider == Provider::Gateway;
    let parts: [String; 8] = [
        s.provider.as_str().to_string(),
        s.base_url.clone(),
        if gw { s.format.as_str() } else { "" }.to_string(),
        if gw { s.gateway_auth.as_str() } else { "" }.to_string(),
        s.model.clone(),
        format!("api_key:{}", revs.api.unwrap_or(0)),
        if gw {
            format!("gateway_key:{}", revs.gateway.unwrap_or(0))
        } else {
            String::new()
        },
        key_id.to_string(),
    ];
    hex(&Sha256::digest(parts.join("\n").as_bytes()))
}

/// Normalises an admin-entered base URL (4.2): trimmed, trailing `/` removed, ≤ 300 characters, absolute http(s) URL
/// with a host and without user info, query or fragment.
pub fn normalize_url(raw: &str) -> Option<String> {
    let t = raw.trim().trim_end_matches('/');
    if t.is_empty() || t.chars().count() > MAX_URL_CHARS {
        return None;
    }
    if t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let u = reqwest::Url::parse(t).ok()?;
    let ok = matches!(u.scheme(), "http" | "https")
        && u.host_str().is_some_and(|h| !h.is_empty())
        && u.username().is_empty()
        && u.password().is_none()
        && u.query().is_none()
        && u.fragment().is_none()
        && !t.contains('?')
        && !t.contains('#');
    ok.then(|| t.to_string())
}

/// Removes one trailing `/v1` path segment (the Anthropic adapter adds `/v1/messages` itself).
pub fn strip_v1(url: &str) -> String {
    match url.strip_suffix("/v1") {
        Some(rest) if rest.contains("://") && !rest.ends_with(':') && !rest.ends_with('/') => {
            rest.to_string()
        }
        _ => url.to_string(),
    }
}

/// (scheme, lower-case host, port with the scheme default filled in)
pub fn origin(url: &str) -> Option<(String, String, u16)> {
    let u = reqwest::Url::parse(url).ok()?;
    Some((
        u.scheme().to_string(),
        u.host_str()?.to_ascii_lowercase(),
        u.port_or_known_default()?,
    ))
}

/// `true` when the key would travel unencrypted to a host that is not local (the UI warns; the server allows it).
pub fn insecure_http(url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else {
        return false;
    };
    if u.scheme() != "http" {
        return false;
    }
    let host = u.host_str().unwrap_or("");
    !(host == "localhost" || host == "127.0.0.1" || host == "[::1]" || !host.contains('.'))
}

/// The gateway format guessed from the last path segment of the URL.
pub fn infer_format(url: &str) -> Format {
    let last = url.rsplit('/').next().unwrap_or("");
    if last.eq_ignore_ascii_case("anthropic") {
        Format::Anthropic
    } else {
        Format::OpenAi
    }
}

/// `[A-Za-z0-9._:/@+-]`, at most 200 characters ("" clears the model).
pub fn valid_model(m: &str) -> bool {
    m.len() <= MAX_MODEL_CHARS
        && m.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'/' | b'@' | b'+' | b'-')
        })
}

/// A key change in a settings update. No `Debug`.
pub enum KeyChange {
    Keep,
    Remove,
    Set(Zeroizing<String>),
}

impl KeyChange {
    pub fn is_keep(&self) -> bool {
        matches!(self, KeyChange::Keep)
    }
}

/// Why a settings update was refused (5.1 codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateError {
    BadRequest,
    BadUrl,
    BadKey(SecretName),
    KeyReenter(SecretName),
    HeaderClash,
    BadRetention,
}

/// A validated update: the next settings row (fingerprint rules not applied yet) and the key changes. No `Debug`.
pub struct Plan {
    pub next: Settings,
    pub api_key: KeyChange,
    pub gateway_key: KeyChange,
    pub url_changed: bool,
    /// `true` when a field other than `retentionDays` was sent
    pub connection_fields: bool,
    pub retention_sent: bool,
}

const UPDATE_KEYS: [&str; 9] = [
    "provider",
    "baseUrl",
    "format",
    "gatewayAuth",
    "apiKey",
    "gatewayKey",
    "model",
    "manualModel",
    "retentionDays",
];

fn enum_field<T>(
    body: &Map<String, Value>,
    key: &str,
    parse: fn(&str) -> Option<T>,
) -> Result<Option<T>, UpdateError> {
    match body.get(key) {
        None => Ok(None),
        Some(Value::String(s)) => parse(s).map(Some).ok_or(UpdateError::BadRequest),
        Some(_) => Err(UpdateError::BadRequest),
    }
}

fn key_field(body: &Map<String, Value>, name: SecretName) -> Result<KeyChange, UpdateError> {
    match body.get(name.json()) {
        None => Ok(KeyChange::Keep),
        Some(Value::Null) => Ok(KeyChange::Remove),
        Some(Value::String(s)) => {
            if secret::valid_key(s) {
                Ok(KeyChange::Set(Zeroizing::new(s.clone())))
            } else {
                Err(UpdateError::BadKey(name))
            }
        }
        Some(_) => Err(UpdateError::BadRequest),
    }
}

/// Validates an update against the current settings and saved keys, in the order of 5.3.
pub fn plan_update(
    cur: &Settings,
    revs: KeyRevs,
    body: &Map<String, Value>,
) -> Result<Plan, UpdateError> {
    if body.keys().any(|k| !UPDATE_KEYS.contains(&k.as_str())) {
        return Err(UpdateError::BadRequest);
    }
    let provider = enum_field(body, "provider", Provider::parse)?;
    let base_url = match body.get("baseUrl") {
        None => None,
        Some(Value::String(s)) => Some(normalize_url(s).ok_or(UpdateError::BadUrl)?),
        Some(_) => return Err(UpdateError::BadRequest),
    };
    let format = enum_field(body, "format", Format::parse)?;
    let gateway_auth = enum_field(body, "gatewayAuth", GatewayAuth::parse)?;
    let api_key = key_field(body, SecretName::ApiKey)?;
    let gateway_key = key_field(body, SecretName::GatewayKey)?;

    let mut next = cur.clone();
    next.provider = provider.unwrap_or(cur.provider);
    let url = base_url.clone().unwrap_or_else(|| cur.base_url.clone());
    next.format = match (next.provider, format) {
        (_, Some(f)) => f,
        (Provider::Gateway, None) if cur.provider != Provider::Gateway => infer_format(&url),
        (_, None) => cur.format,
    };
    if let Some(a) = gateway_auth {
        next.gateway_auth = a;
    }
    next.base_url = if next.wire_format() == Format::Anthropic {
        strip_v1(&url)
    } else {
        url
    };

    // Key re-entry: a saved key is never sent to a new provider or origin without being typed again
    let moved = next.provider != cur.provider || origin(&next.base_url) != origin(&cur.base_url);
    if moved {
        for (name, change) in [
            (SecretName::ApiKey, &api_key),
            (SecretName::GatewayKey, &gateway_key),
        ] {
            if revs.get(name).is_some() && change.is_keep() {
                return Err(UpdateError::KeyReenter(name));
            }
        }
    }

    // Header clash, on the state after the update (saved keys included)
    let api_set = match &api_key {
        KeyChange::Keep => revs.api.is_some(),
        KeyChange::Remove => false,
        KeyChange::Set(_) => true,
    };
    if next.provider == Provider::Gateway && api_set {
        let clash = matches!(
            (next.gateway_auth, next.format),
            (GatewayAuth::Bearer, Format::OpenAi) | (GatewayAuth::XApiKey, Format::Anthropic)
        );
        if clash {
            return Err(UpdateError::HeaderClash);
        }
    }

    match body.get("model") {
        None => {}
        Some(Value::String(m)) => {
            let m = m.trim();
            if !valid_model(m) {
                return Err(UpdateError::BadRequest);
            }
            next.model = m.to_string();
        }
        Some(_) => return Err(UpdateError::BadRequest),
    }
    match body.get("manualModel") {
        None => {}
        Some(Value::Bool(b)) => next.manual_model = *b,
        Some(_) => return Err(UpdateError::BadRequest),
    }
    let retention_sent = body.contains_key("retentionDays");
    if let Some(v) = body.get("retentionDays") {
        match v.as_i64() {
            Some(n) if v.is_i64() && (1..=3650).contains(&n) => next.retention_days = n,
            _ => return Err(UpdateError::BadRetention),
        }
    }
    Ok(Plan {
        url_changed: next.base_url != cur.base_url,
        next,
        api_key,
        gateway_key,
        connection_fields: body.keys().any(|k| k != "retentionDays"),
        retention_sent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base() -> Settings {
        Settings {
            provider: Provider::OpenAi,
            base_url: DEFAULT_OPENAI.into(),
            format: Format::OpenAi,
            gateway_auth: GatewayAuth::Bearer,
            model: String::new(),
            manual_model: false,
            usage_option: true,
            max_tokens: 8192,
            enabled: false,
            verified_at: None,
            verified_fp: None,
            retention_days: 90,
            updated_at: 0,
            updated_by: String::new(),
        }
    }

    fn plan(cur: &Settings, revs: KeyRevs, v: Value) -> Result<Plan, UpdateError> {
        plan_update(cur, revs, v.as_object().expect("object"))
    }

    #[test]
    fn url_rules() {
        assert_eq!(
            normalize_url(" https://example.com/v1/ ").as_deref(),
            Some("https://example.com/v1")
        );
        assert_eq!(
            normalize_url("http://ai-stub:9000/v1").as_deref(),
            Some("http://ai-stub:9000/v1")
        );
        for bad in [
            "",
            "ftp://example.com",
            "example.com/v1",
            "https://user:pw@example.com",
            "https://user@example.com",
            "https://example.com/v1?x=1",
            "https://example.com/v1#frag",
            "https://example.com/v1?",
            "https://exa mple.com",
            "https://",
        ] {
            assert!(normalize_url(bad).is_none(), "{bad}");
        }
        let long = format!("https://example.com/{}", "a".repeat(300));
        assert!(normalize_url(&long).is_none());
        assert_eq!(
            strip_v1("https://api.anthropic.com/v1"),
            "https://api.anthropic.com"
        );
        assert_eq!(
            strip_v1("https://example.com/gw/anthropic"),
            "https://example.com/gw/anthropic"
        );
        assert_eq!(
            strip_v1("https://example.com/v1/v1"),
            "https://example.com/v1"
        );
        assert_eq!(
            origin("https://Example.com/a"),
            Some(("https".into(), "example.com".into(), 443))
        );
        assert_eq!(
            origin("https://example.com:443/b"),
            origin("https://example.com/c")
        );
        assert_ne!(
            origin("http://example.com/a"),
            origin("https://example.com/a")
        );
        assert!(insecure_http("http://example.com/v1"));
        assert!(!insecure_http("http://ai-stub:9000/v1"));
        assert!(!insecure_http("http://127.0.0.1:9000/v1"));
        assert!(!insecure_http("https://example.com/v1"));
        assert_eq!(
            infer_format("https://ai-gateway.example.com/anthropic"),
            Format::Anthropic
        );
        assert_eq!(
            infer_format("https://ai-gateway.example.com/openai"),
            Format::OpenAi
        );
        assert!(valid_model("demo-chat-small"));
        assert!(valid_model("org/model:1.5@x+y_z"));
        assert!(!valid_model("bad model"));
        assert!(!valid_model(&"m".repeat(201)));
    }

    #[test]
    fn fingerprint_covers_what_it_must() {
        let s = base();
        let r = KeyRevs {
            api: Some(1),
            gateway: None,
        };
        let fp = fingerprint(&s, r, "k1");
        let mut t = s.clone();
        t.retention_days = 5;
        t.manual_model = true;
        t.usage_option = false;
        t.max_tokens = 4096;
        t.enabled = true;
        assert_eq!(fingerprint(&t, r, "k1"), fp, "not part of the fingerprint");
        let mut m = s.clone();
        m.model = "x".into();
        assert_ne!(fingerprint(&m, r, "k1"), fp);
        let mut u = s.clone();
        u.base_url = "https://example.com/v1".into();
        assert_ne!(fingerprint(&u, r, "k1"), fp);
        assert_ne!(
            fingerprint(
                &s,
                KeyRevs {
                    api: Some(2),
                    gateway: None
                },
                "k1"
            ),
            fp
        );
        assert_ne!(fingerprint(&s, r, "k2"), fp);
        // format, gateway auth and the gateway key matter only for a gateway
        let mut f = s.clone();
        f.format = Format::Anthropic;
        f.gateway_auth = GatewayAuth::CfAig;
        assert_eq!(
            fingerprint(
                &f,
                KeyRevs {
                    api: Some(1),
                    gateway: Some(9)
                },
                "k1"
            ),
            fp
        );
        let mut g = s.clone();
        g.provider = Provider::Gateway;
        let gfp = fingerprint(&g, r, "k1");
        g.format = Format::Anthropic;
        assert_ne!(fingerprint(&g, r, "k1"), gfp);
    }

    #[test]
    fn validation_order_and_codes() {
        let s = base();
        let none = KeyRevs::default();
        assert_eq!(
            plan(&s, none, json!({"provider": "x"})).err(),
            Some(UpdateError::BadRequest)
        );
        assert_eq!(
            plan(&s, none, json!({"bogus": 1})).err(),
            Some(UpdateError::BadRequest)
        );
        assert_eq!(
            plan(&s, none, json!({"baseUrl": "nope"})).err(),
            Some(UpdateError::BadUrl)
        );
        assert_eq!(
            plan(&s, none, json!({"baseUrl": 5})).err(),
            Some(UpdateError::BadRequest)
        );
        assert_eq!(
            plan(&s, none, json!({"apiKey": "has space"})).err(),
            Some(UpdateError::BadKey(SecretName::ApiKey))
        );
        assert_eq!(
            plan(&s, none, json!({"gatewayKey": ""})).err(),
            Some(UpdateError::BadKey(SecretName::GatewayKey))
        );
        assert_eq!(
            plan(&s, none, json!({"apiKey": 12345})).err(),
            Some(UpdateError::BadRequest)
        );
        assert_eq!(
            plan(&s, none, json!({"model": "a b"})).err(),
            Some(UpdateError::BadRequest)
        );
        assert_eq!(
            plan(&s, none, json!({"retentionDays": 0})).err(),
            Some(UpdateError::BadRetention)
        );
        assert_eq!(
            plan(&s, none, json!({"retentionDays": 3651})).err(),
            Some(UpdateError::BadRetention)
        );
        assert_eq!(
            plan(&s, none, json!({"retentionDays": "9"})).err(),
            Some(UpdateError::BadRetention)
        );
        assert_eq!(
            plan(&s, none, json!({"retentionDays": 9.5})).err(),
            Some(UpdateError::BadRetention)
        );
        // URL error comes before the key error
        assert_eq!(
            plan(&s, none, json!({"baseUrl": "nope", "apiKey": "x y"})).err(),
            Some(UpdateError::BadUrl)
        );
        let p = plan(&s, none, json!({"retentionDays": 30})).expect("ok");
        assert!(!p.connection_fields);
        assert!(p.retention_sent);
        assert_eq!(p.next.retention_days, 30);
    }

    #[test]
    fn anthropic_v1_is_stripped_and_gateway_format_inferred() {
        let s = base();
        let p = plan(
            &s,
            KeyRevs::default(),
            json!({"provider": "anthropic", "baseUrl": "https://api.anthropic.com/v1"}),
        )
        .expect("ok");
        assert_eq!(p.next.base_url, "https://api.anthropic.com");
        let p = plan(
            &s,
            KeyRevs::default(),
            json!({"provider": "gateway", "baseUrl": "https://ai-gateway.example.com/anthropic/v1"}),
        )
        .expect("ok");
        // inferred from the URL as sent (last segment v1 → openai), so nothing is stripped
        assert_eq!(p.next.format, Format::OpenAi);
        let p = plan(
            &s,
            KeyRevs::default(),
            json!({"provider": "gateway", "baseUrl": "https://ai-gateway.example.com/anthropic"}),
        )
        .expect("ok");
        assert_eq!(p.next.format, Format::Anthropic);
        let p = plan(
            &s,
            KeyRevs::default(),
            json!({"provider": "gateway", "format": "anthropic", "baseUrl": "https://ai-gateway.example.com/x/v1"}),
        )
        .expect("ok");
        assert_eq!(p.next.base_url, "https://ai-gateway.example.com/x");
    }

    #[test]
    fn key_reentry_rule() {
        let s = base();
        let saved = KeyRevs {
            api: Some(1),
            gateway: None,
        };
        // origin change without the key
        assert_eq!(
            plan(&s, saved, json!({"baseUrl": "https://example.com/v1"})).err(),
            Some(UpdateError::KeyReenter(SecretName::ApiKey))
        );
        // with the key, or removing it
        assert!(plan(
            &s,
            saved,
            json!({"baseUrl": "https://example.com/v1", "apiKey": "sk-new-key-1234"})
        )
        .is_ok());
        assert!(plan(
            &s,
            saved,
            json!({"baseUrl": "https://example.com/v1", "apiKey": null})
        )
        .is_ok());
        // default port written out: same origin
        assert!(plan(
            &s,
            saved,
            json!({"baseUrl": "https://api.openai.com:443/v1"})
        )
        .is_ok());
        // path-only change keeps the keys
        assert!(plan(&s, saved, json!({"baseUrl": "https://api.openai.com/v2"})).is_ok());
        // provider change
        assert_eq!(
            plan(
                &s,
                saved,
                json!({"provider": "anthropic", "baseUrl": DEFAULT_ANTHROPIC})
            )
            .err(),
            Some(UpdateError::KeyReenter(SecretName::ApiKey))
        );
        // both slots set: apiKey is named first, then gatewayKey
        let mut g = base();
        g.provider = Provider::Gateway;
        g.gateway_auth = GatewayAuth::CfAig;
        g.base_url = "https://ai-gateway.example.com/openai".into();
        let both = KeyRevs {
            api: Some(1),
            gateway: Some(1),
        };
        assert_eq!(
            plan(
                &g,
                both,
                json!({"baseUrl": "https://other.example.com/openai"})
            )
            .err(),
            Some(UpdateError::KeyReenter(SecretName::ApiKey))
        );
        assert_eq!(
            plan(
                &g,
                both,
                json!({"baseUrl": "https://other.example.com/openai", "apiKey": null})
            )
            .err(),
            Some(UpdateError::KeyReenter(SecretName::GatewayKey))
        );
        assert!(plan(
            &g,
            both,
            json!({"baseUrl": "https://ai-gateway.example.com/anthropic"})
        )
        .is_ok());
    }

    #[test]
    fn header_clash_checks_saved_keys() {
        let mut g = base();
        g.provider = Provider::Gateway;
        g.base_url = "https://ai-gateway.example.com/openai".into();
        g.gateway_auth = GatewayAuth::CfAig;
        let api_saved = KeyRevs {
            api: Some(1),
            gateway: Some(1),
        };
        assert_eq!(
            plan(&g, api_saved, json!({"gatewayAuth": "bearer"})).err(),
            Some(UpdateError::HeaderClash)
        );
        assert!(plan(
            &g,
            api_saved,
            json!({"gatewayAuth": "bearer", "apiKey": null})
        )
        .is_ok());
        assert!(plan(&g, api_saved, json!({"gatewayAuth": "x-api-key"})).is_ok());
        assert_eq!(
            plan(
                &g,
                api_saved,
                json!({"gatewayAuth": "x-api-key", "format": "anthropic"})
            )
            .err(),
            Some(UpdateError::HeaderClash)
        );
        let no_api = KeyRevs {
            api: None,
            gateway: Some(1),
        };
        assert!(plan(&g, no_api, json!({"gatewayAuth": "bearer"})).is_ok());
        assert_eq!(
            plan(
                &g,
                no_api,
                json!({"gatewayAuth": "bearer", "apiKey": "sk-demo-0000-1234"})
            )
            .err(),
            Some(UpdateError::HeaderClash)
        );
    }
}
