//! AI settings: providers, wire formats, the outbound URL policy (4.2) and the global settings row.
use crate::secret::SecretName;
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
    /// OpenAI Responses API (`POST {base}/responses`); picked by hand for a gateway, never inferred from a URL
    Responses,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Format::OpenAi => "openai",
            Format::Anthropic => "anthropic",
            Format::Responses => "responses",
        }
    }

    pub fn parse(s: &str) -> Option<Format> {
        match s {
            "openai" => Some(Format::OpenAi),
            "anthropic" => Some(Format::Anthropic),
            "responses" => Some(Format::Responses),
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

/// The `settings` row (AI switch and retention). Never holds a secret; the connections live in `connections`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub retention_days: i64,
    pub updated_at: i64,
    pub updated_by: String,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
