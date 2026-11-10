//! Configuration: `XEXAM_*` environment variables and command-line arguments. Based on `server.py main()` and `docs/migration-plan.md` §3.
use crate::security::parse_allowed_hosts;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub host: String,
    pub port: u16,
    pub allowed_hosts: HashSet<String>, // already includes localhost / 127.0.0.1 / ::1
    pub allowed_origins: HashSet<String>, // already includes allowed_hosts
    pub allow_any_host: bool,
    pub allow_shutdown: bool,
    pub parse_timeout_secs: u64,
    pub version: String,
}

pub const MAX_UPLOAD: usize = 20 * 1024 * 1024;
pub const MAX_JSON: usize = 1_000_000;
pub const PENDING_TTL_SECS: u64 = 30 * 60;
pub const SKIP_SHOWN: usize = 30;

/// The version in `/api/info` accepts only `[0-9A-Za-z.+-]{1,20}` (the rule of the old `read_version`), otherwise `dev`.
pub fn valid_version(v: &str) -> bool {
    let v = v.trim();
    !v.is_empty()
        && v.chars().count() <= 20
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
}

impl Config {
    /// Reads from the real process environment. The version comes from, in order:
    /// 1. `XEXAM_VERSION` (when valid);
    /// 2. the contents of the file pointed to by `XEXAM_VERSION_FILE` (`/VERSION` in the image, same as the old version reading the `VERSION` file);
    /// 3. the crate version compiled in.
    pub fn from_env() -> Config {
        let from_file = || {
            let path = std::env::var("XEXAM_VERSION_FILE").ok()?;
            let text = std::fs::read_to_string(path).ok()?;
            Some(text.trim().to_string()).filter(|v| valid_version(v))
        };
        let version = std::env::var("XEXAM_VERSION")
            .ok()
            .filter(|v| valid_version(v))
            .map(|v| v.trim().to_string())
            .or_else(from_file)
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
        Config::from_env_with(&|k| std::env::var(k).ok(), version)
    }

    /// Reads from environment variables. `get` injects the environment (tests need not change the process environment).
    pub fn from_env_with(get: &dyn Fn(&str) -> Option<String>, version: String) -> Config {
        let extra = parse_allowed_hosts(get("XEXAM_ALLOWED_HOSTS").as_deref());
        let origins = parse_allowed_hosts(get("XEXAM_ALLOWED_ORIGINS").as_deref());
        let any = get("XEXAM_ALLOW_ANY_HOST")
            .map(|v| {
                matches!(
                    v.trim().to_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        let mut hosts: HashSet<String> = ["localhost", "127.0.0.1", "::1"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        hosts.extend(extra.iter().cloned());
        let mut allowed_origins = origins;
        allowed_origins.extend(extra);
        Config {
            data_dir: PathBuf::from(
                get("XEXAM_DATA")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "data".to_string()),
            ),
            host: get("XEXAM_HOST").unwrap_or_else(|| "127.0.0.1".to_string()),
            port: get("XEXAM_PORT")
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(8765),
            allowed_hosts: hosts,
            allowed_origins,
            allow_any_host: any,
            allow_shutdown: false,
            parse_timeout_secs: get("XEXAM_PARSE_TIMEOUT_SECS")
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(30),
            version,
        }
    }
}
