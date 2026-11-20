//! Configuration of the AI service: `XEXAM_AI_*` and the shared `XEXAM_*` environment variables.
//!
//! The struct holds the internal token and the AI encryption key while the service starts, so it has a hand-written,
//! redacted `Debug` (never derive it). The encryption key is taken out of it as soon as it was read (`secret`).
use crate::peers::PeerList;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;
use zeroize::Zeroizing;

/// The AI database's journal mode (`XEXAM_AI_SQLITE_JOURNAL`, the same rule as the backend's `XEXAM_SQLITE_JOURNAL`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JournalMode {
    #[default]
    Wal,
    Delete,
}

impl JournalMode {
    /// `delete` (trimmed, any case) is the opt-out; anything else, or nothing, is WAL.
    pub fn from_env_value(value: Option<&str>) -> JournalMode {
        match value {
            Some(v) if v.trim().eq_ignore_ascii_case("delete") => JournalMode::Delete,
            _ => JournalMode::Wal,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            JournalMode::Wal => "wal",
            JournalMode::Delete => "delete",
        }
    }
}

/// Every time limit of the service. Production uses [`Timeouts::default`]; tests shorten them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// TCP / TLS connect to a provider (`ai.up.connect`)
    pub connect: Duration,
    /// From sending the request to the response headers (`ai.up.timeout`)
    pub first_byte: Duration,
    /// Between two body chunks of a streamed answer (`ai.up.timeout`)
    pub idle: Duration,
    /// A whole chat answer (`ai.up.timeout`, the text so far is kept)
    pub total: Duration,
    /// Models list and memory suggestion, in total
    pub short_total: Duration,
    /// The test call of `POST /api/ai/admin/verify`, in total (the chat limits apply as well)
    pub verify_total: Duration,
    /// Connect to the main backend
    pub backend_connect: Duration,
    /// A whole call to the main backend
    pub backend_total: Duration,
    /// Re-check of the caller's access while an answer is generated
    pub access_interval: Duration,
    /// How often the text of a running answer is written to the database
    pub flush_interval: Duration,
    /// Keep-alive comment of the SSE stream when nothing else was sent
    pub keep_alive: Duration,
    /// Time between two state reports to the main backend after an accepted one (contracts 2.6)
    pub heartbeat_every: Duration,
    /// Time before the next state report after a failed one
    pub heartbeat_retry: Duration,
    /// One external IP-location lookup, connect and answer in total (v1.4, default 3 s)
    pub geo_total: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts {
            connect: Duration::from_secs(10),
            first_byte: Duration::from_secs(120),
            idle: Duration::from_secs(120),
            total: Duration::from_secs(600),
            short_total: Duration::from_secs(30),
            verify_total: Duration::from_secs(120),
            backend_connect: Duration::from_secs(2),
            backend_total: Duration::from_secs(5),
            access_interval: Duration::from_secs(2),
            flush_interval: Duration::from_secs(2),
            keep_alive: Duration::from_secs(15),
            heartbeat_every: Duration::from_secs(25),
            heartbeat_retry: Duration::from_secs(5),
            geo_total: Duration::from_secs(3),
        }
    }
}

/// What `XEXAM_AI_USER_AGENT_NAME` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiUserAgent {
    /// Unset, blank or unusable: send `xexam-ai-service/<version>`
    Default,
    /// Printable ASCII of at most [`MAX_USER_AGENT`] characters, sent as is
    Value(String),
    /// `none` (any case): send no `User-Agent` header at all
    Off,
}

/// Configuration. No `Debug` derive: it holds secrets while the service starts.
pub struct Config {
    /// `XEXAM_AI_DATA` (default `data-ai`): the AI database and the key file
    pub data_dir: PathBuf,
    /// `XEXAM_AI_HOST` (default `127.0.0.1`)
    pub host: String,
    /// `XEXAM_AI_PORT` (default 8766)
    pub port: u16,
    /// `XEXAM_AI_BACKEND_URL` (default `http://127.0.0.1:8765`): `http://` or `https://` plus host[:port], one trailing
    /// `/` trimmed (the link checks the rest)
    pub backend_url: String,
    /// `XEXAM_LINK_DIR` (default `data/link`): where the backend writes `internal.token`
    pub link_dir: PathBuf,
    /// `XEXAM_INTERNAL_TOKEN` (empty = unset)
    pub internal_token: Option<Zeroizing<String>>,
    /// `XEXAM_AI_SECRET_KEY` (empty = unset); taken out by the key loader right at start
    pub secret_key: Option<Zeroizing<String>>,
    /// `XEXAM_AI_LOG`
    pub log_filter: Option<String>,
    /// `XEXAM_AI_SQLITE_JOURNAL`
    pub journal: JournalMode,
    /// `XEXAM_AI_USER_AGENT_NAME` (v1.9: unset or blank = the default, `none` = no header, else sent as is)
    pub ai_user_agent: AiUserAgent,
    /// `XEXAM_AI_USER_AGENT_NAME` was set but not usable (logged once at start; the default is sent)
    pub ai_user_agent_invalid: bool,
    /// `XEXAM_FRONTEND_PEERS` (unset = any peer)
    pub frontend_peers: PeerList,
    /// Problems found in `XEXAM_FRONTEND_PEERS` (logged once at start)
    pub peer_warnings: Vec<String>,
    /// `XEXAM_INTERNAL_TOKEN_REQUIRED`: start only with a valid `XEXAM_INTERNAL_TOKEN`
    pub internal_token_required: bool,
    /// `XEXAM_VERSION`, else `CODE_VERSION` in `XEXAM_VERSION_FILE`, else the crate version
    pub version: String,
    /// Time limits (not configurable by the environment)
    pub timeouts: Timeouts,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("data_dir", &self.data_dir)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("link_dir", &self.link_dir)
            .field(
                "internal_token",
                &self.internal_token.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "secret_key",
                &self.secret_key.as_ref().map(|_| "<redacted>"),
            )
            .field("journal", &self.journal)
            .field("ai_user_agent", &self.ai_user_agent)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// The text of `xexam-ai-service --help`.
pub const USAGE: &str = "Usage:
  xexam-ai-service [serve]      start the AI service (default)
  xexam-ai-service healthcheck  exit 0 when GET http://127.0.0.1:$XEXAM_AI_PORT/healthz answers 200
  xexam-ai-service --help       show this text
Environment variables:
  XEXAM_AI_DATA            AI data directory: xexam-ai.db and secret.key (default data-ai)
  XEXAM_AI_HOST            listen address (default 127.0.0.1)
  XEXAM_AI_PORT            listen port (default 8766)
  XEXAM_AI_BACKEND_URL     the main backend, http:// or https:// host[:port] (default http://127.0.0.1:8765)
  XEXAM_LINK_DIR           directory of internal.token written by the backend (default data/link)
  XEXAM_INTERNAL_TOKEN     shared token instead of the file (at least 32 characters; the same for both services)
  XEXAM_AI_SECRET_KEY      AI encryption key: 32 bytes as 64 hex digits or base64 (default: the key file secret.key)
  XEXAM_AI_LOG             log filter (default info)
  XEXAM_AI_SQLITE_JOURNAL  delete = rollback journal instead of WAL
  XEXAM_AI_USER_AGENT_NAME User-Agent sent to AI providers (default xexam-ai-service/<version>; none = no User-Agent; else printable ASCII, at most 256 characters)
  XEXAM_FRONTEND_PEERS     addresses / CIDR ranges allowed to call this service (default any; loopback always may)
  XEXAM_INTERNAL_TOKEN_REQUIRED  1 = refuse to start without a valid XEXAM_INTERNAL_TOKEN
  XEXAM_VERSION            version reported by /healthz
  XEXAM_VERSION_FILE       file with CODE_VERSION=... (the image sets /VERSION)
  TZ                       time zone for the per-day usage statistics (for example TZ=UTC)";

/// The version accepts only `[0-9A-Za-z.+-]{1,20}` (the backend's rule).
pub fn valid_version(v: &str) -> bool {
    let v = v.trim();
    !v.is_empty()
        && v.chars().count() <= 20
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
}

/// `CODE_VERSION` from the text of the `VERSION` file (same rules as the backend's `code_version_from_file`).
pub fn code_version_from_file(text: &str) -> Option<String> {
    let mut saw_key = false;
    let mut found: Option<&str> = None;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            saw_key = true;
            if key.trim() == "CODE_VERSION" {
                if found.is_some() {
                    return None;
                }
                found = Some(value.trim());
            }
        }
    }
    if let Some(value) = found {
        return valid_version(value).then(|| value.to_string());
    }
    let whole = text.trim();
    (!saw_key && valid_version(whole)).then(|| whole.to_string())
}

fn non_empty(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.trim().is_empty())
}

/// The longest `XEXAM_AI_USER_AGENT_NAME` accepted.
pub const MAX_USER_AGENT: usize = 256;

/// `XEXAM_AI_USER_AGENT_NAME`: unset or blank = `(Default, false)`; `none` (any case, trimmed) = `(Off, false)`;
/// printable ASCII (space to `~`) of at most [`MAX_USER_AGENT`] characters = `(Value, false)`; anything else =
/// `(Default, true)` (the caller warns once and sends the default).
pub fn user_agent_value(v: Option<String>) -> (AiUserAgent, bool) {
    let Some(v) = non_empty(v) else {
        return (AiUserAgent::Default, false);
    };
    let v = v.trim();
    if v.eq_ignore_ascii_case("none") {
        (AiUserAgent::Off, false)
    } else if v.len() <= MAX_USER_AGENT && v.chars().all(|c| (' '..='~').contains(&c)) {
        (AiUserAgent::Value(v.to_string()), false)
    } else {
        (AiUserAgent::Default, true)
    }
}

/// `1`, `true`, `yes` or `on` (trimmed, any case).
fn flag(v: Option<String>) -> bool {
    v.is_some_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

impl Config {
    /// Reads the real process environment.
    pub fn from_env() -> Config {
        Config::from_env_with(&|k| std::env::var(k).ok())
    }

    /// Reads from `get` (tests inject the environment).
    pub fn from_env_with(get: &dyn Fn(&str) -> Option<String>) -> Config {
        let from_file = || {
            let path = get("XEXAM_VERSION_FILE")?;
            let text = std::fs::read_to_string(path).ok()?;
            code_version_from_file(&text)
        };
        let version = get("XEXAM_VERSION")
            .filter(|v| valid_version(v))
            .map(|v| v.trim().to_string())
            .or_else(from_file)
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
        let (ai_user_agent, ai_user_agent_invalid) =
            user_agent_value(get("XEXAM_AI_USER_AGENT_NAME"));
        let (frontend_peers, peer_warnings) =
            crate::peers::parse_peers(get("XEXAM_FRONTEND_PEERS").as_deref());
        Config {
            data_dir: PathBuf::from(
                non_empty(get("XEXAM_AI_DATA")).unwrap_or_else(|| "data-ai".to_string()),
            ),
            host: non_empty(get("XEXAM_AI_HOST")).unwrap_or_else(|| "127.0.0.1".to_string()),
            port: get("XEXAM_AI_PORT")
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(8766),
            backend_url: non_empty(get("XEXAM_AI_BACKEND_URL"))
                .map(|s| s.trim().trim_end_matches('/').to_string())
                .unwrap_or_else(|| "http://127.0.0.1:8765".to_string()),
            link_dir: PathBuf::from(
                non_empty(get("XEXAM_LINK_DIR")).unwrap_or_else(|| "data/link".to_string()),
            ),
            internal_token: non_empty(get("XEXAM_INTERNAL_TOKEN")).map(Zeroizing::new),
            secret_key: non_empty(get("XEXAM_AI_SECRET_KEY")).map(Zeroizing::new),
            log_filter: get("XEXAM_AI_LOG"),
            journal: JournalMode::from_env_value(get("XEXAM_AI_SQLITE_JOURNAL").as_deref()),
            ai_user_agent,
            ai_user_agent_invalid,
            frontend_peers,
            peer_warnings,
            internal_token_required: flag(get("XEXAM_INTERNAL_TOKEN_REQUIRED")),
            version,
            timeouts: Timeouts::default(),
        }
    }

    /// The `User-Agent` sent to AI providers: the service name by default, nothing for `none`.
    pub fn provider_user_agent(&self) -> Option<String> {
        match &self.ai_user_agent {
            AiUserAgent::Default => Some(format!("xexam-ai-service/{}", self.version)),
            AiUserAgent::Value(v) => Some(v.clone()),
            AiUserAgent::Off => None,
        }
    }

    /// The port `xexam-ai-service healthcheck` checks.
    pub fn healthcheck_port() -> u16 {
        std::env::var("XEXAM_AI_PORT")
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(8766)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg(pairs: &[(&str, &str)]) -> Config {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_env_with(&|k| map.get(k).cloned())
    }

    #[test]
    fn defaults() {
        let c = cfg(&[]);
        assert_eq!(c.data_dir, PathBuf::from("data-ai"));
        assert_eq!(c.host, "127.0.0.1");
        assert_eq!(c.port, 8766);
        assert_eq!(c.backend_url, "http://127.0.0.1:8765");
        assert_eq!(c.link_dir, PathBuf::from("data/link"));
        assert!(c.internal_token.is_none());
        assert!(c.secret_key.is_none());
        assert_eq!(c.journal, JournalMode::Wal);
        assert_eq!(c.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(c.ai_user_agent, AiUserAgent::Default);
        assert!(!c.ai_user_agent_invalid);
        assert_eq!(
            c.provider_user_agent().as_deref(),
            Some(format!("xexam-ai-service/{}", env!("CARGO_PKG_VERSION")).as_str())
        );
        assert_eq!(c.frontend_peers, PeerList::Any);
        assert!(c.peer_warnings.is_empty());
        assert!(!c.internal_token_required);
    }

    #[test]
    fn the_ai_user_agent_default_value_none_and_invalid() {
        let ua = |v: &str| cfg(&[("XEXAM_AI_USER_AGENT_NAME", v)]);
        let c = ua("  ");
        assert!(c.ai_user_agent == AiUserAgent::Default && !c.ai_user_agent_invalid);
        let c = ua(" MyGateway-Client/2.0 (made up) ");
        assert_eq!(
            c.ai_user_agent,
            AiUserAgent::Value("MyGateway-Client/2.0 (made up)".to_string())
        );
        assert!(!c.ai_user_agent_invalid);
        for off in ["none", "NONE", "None", " none "] {
            let c = ua(off);
            assert!(
                c.ai_user_agent == AiUserAgent::Off && !c.ai_user_agent_invalid,
                "{off:?}"
            );
            assert_eq!(c.provider_user_agent(), None);
        }
        assert_eq!(
            ua("None/1.0").ai_user_agent,
            AiUserAgent::Value("None/1.0".to_string())
        );
        for bad in ["line\nbreak", "tab\there", "non-ascii \u{00e9}"] {
            let c = ua(bad);
            assert!(
                c.ai_user_agent == AiUserAgent::Default && c.ai_user_agent_invalid,
                "{bad:?}"
            );
        }
        let long = "a".repeat(MAX_USER_AGENT + 1);
        let c = ua(&long);
        assert!(c.ai_user_agent == AiUserAgent::Default && c.ai_user_agent_invalid);
        let max = "a".repeat(MAX_USER_AGENT);
        let c = ua(&max);
        assert_eq!(c.ai_user_agent, AiUserAgent::Value(max));
        let c = cfg(&[("XEXAM_VERSION", "9.9")]);
        assert_eq!(
            c.provider_user_agent().as_deref(),
            Some("xexam-ai-service/9.9")
        );
    }

    #[test]
    fn backend_url_and_new_switches() {
        let c = cfg(&[("XEXAM_AI_BACKEND_URL", "https://10.0.0.2:8765/")]);
        assert_eq!(c.backend_url, "https://10.0.0.2:8765");
        let c = cfg(&[
            ("XEXAM_INTERNAL_TOKEN_REQUIRED", "1"),
            ("XEXAM_FRONTEND_PEERS", "10.0.0.5, nope"),
        ]);
        assert!(c.internal_token_required);
        assert_eq!(c.peer_warnings.len(), 1);
        assert!(c.frontend_peers.allows(Some("10.0.0.5:1".parse().unwrap())));
        assert!(!c.frontend_peers.allows(Some("10.0.0.6:1".parse().unwrap())));
        assert!(!cfg(&[("XEXAM_INTERNAL_TOKEN_REQUIRED", "0")]).internal_token_required);
    }

    #[test]
    fn empty_values_are_unset_and_debug_is_redacted() {
        let c = cfg(&[
            ("XEXAM_INTERNAL_TOKEN", ""),
            ("XEXAM_AI_SECRET_KEY", "  "),
            ("XEXAM_AI_SQLITE_JOURNAL", "DELETE"),
            ("XEXAM_VERSION", "0.8"),
        ]);
        assert!(c.internal_token.is_none());
        assert!(c.secret_key.is_none());
        assert_eq!(c.journal, JournalMode::Delete);
        assert_eq!(c.version, "0.8");
        let c = cfg(&[
            (
                "XEXAM_INTERNAL_TOKEN",
                "fake-token-for-tests-0123456789abcdef",
            ),
            ("XEXAM_AI_SECRET_KEY", "fake-secret"),
        ]);
        let dbg = format!("{c:?}");
        assert!(!dbg.contains("fake-token"));
        assert!(!dbg.contains("fake-secret"));
        assert!(dbg.contains("<redacted>"));
    }

    #[test]
    fn version_file_rules() {
        assert_eq!(
            code_version_from_file("# c\nCODE_VERSION=0.8\nRELEASE_VERSION=0.7.5\n").as_deref(),
            Some("0.8")
        );
        assert_eq!(code_version_from_file("0.8\n").as_deref(), Some("0.8"));
        assert_eq!(code_version_from_file("RELEASE_VERSION=0.7\n"), None);
        assert_eq!(
            code_version_from_file("CODE_VERSION=0.8\nCODE_VERSION=0.8\n"),
            None
        );
    }
}
