//! Configuration: `XEXAM_*` environment variables and command-line arguments. Based on `server.py main()` and `docs/zh-CN/history/migration-plan.md` §3.
use crate::database::journal::{JournalMode, JOURNAL_ENV};
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
    /// `XEXAM_COOKIE_SECURE=1`: the session cookie always gets `Secure` (otherwise only for an `https://` Origin)
    pub cookie_secure: bool,
    pub parse_timeout_secs: u64,
    pub version: String,
    /// `XEXAM_SQLITE_JOURNAL`: WAL unless it says `delete` (v0.7; see `database::journal`)
    pub sqlite_journal: JournalMode,
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

/// Reads the version of the code from the text of the `VERSION` file.
///
/// Since v0.6.5 the file holds two `KEY=VALUE` lines: `CODE_VERSION` (the version of the code that was built; this is
/// what `/api/info` reports) and `RELEASE_VERSION` (the last published release, written by CI after the release job;
/// not used here). Empty lines and lines starting with `#` are comments. A file without any `KEY=VALUE` line is the
/// pre-0.6.5 format, where the whole file is the version. Returns `None` when no valid version is found, and also when
/// `CODE_VERSION` appears more than once (a malformed file: which one would be meant? The CI check rejects it too).
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
    // Old single-line format (for example "0.6"): only when the file has no KEY=VALUE line at all
    let whole = text.trim();
    (!saw_key && valid_version(whole)).then(|| whole.to_string())
}

impl Config {
    /// Reads from the real process environment. The version comes from, in order:
    /// 1. `XEXAM_VERSION` (when valid);
    /// 2. `CODE_VERSION` in the file pointed to by `XEXAM_VERSION_FILE` (`/VERSION` in the image; see [`code_version_from_file`]);
    /// 3. the crate version compiled in.
    pub fn from_env() -> Config {
        let from_file = || {
            let path = std::env::var("XEXAM_VERSION_FILE").ok()?;
            let text = std::fs::read_to_string(path).ok()?;
            code_version_from_file(&text)
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
        let flag = |name: &str| {
            get(name)
                .map(|v| {
                    matches!(
                        v.trim().to_lowercase().as_str(),
                        "1" | "true" | "yes" | "on"
                    )
                })
                .unwrap_or(false)
        };
        let any = flag("XEXAM_ALLOW_ANY_HOST");
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
            cookie_secure: flag("XEXAM_COOKIE_SECURE"),
            parse_timeout_secs: get("XEXAM_PARSE_TIMEOUT_SECS")
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(30),
            version,
            sqlite_journal: JournalMode::from_env_value(get(JOURNAL_ENV).as_deref()),
        }
    }
}

#[cfg(test)]
mod version_file_tests {
    use super::code_version_from_file;

    #[test]
    fn reads_code_version_and_ignores_release_version_and_comments() {
        let text = "# comment
CODE_VERSION=0.6.5

# another comment
RELEASE_VERSION=0.6
";
        assert_eq!(code_version_from_file(text).as_deref(), Some("0.6.5"));
        // The order of the lines does not matter
        let text = "RELEASE_VERSION=0.6
CODE_VERSION = 0.7 
";
        assert_eq!(code_version_from_file(text).as_deref(), Some("0.7"));
    }

    #[test]
    fn the_old_single_line_format_still_works() {
        assert_eq!(
            code_version_from_file(
                "0.6
"
            )
            .as_deref(),
            Some("0.6")
        );
    }

    #[test]
    fn invalid_or_missing_code_version_gives_none() {
        // KEY=VALUE lines but no CODE_VERSION: not the old format either
        assert_eq!(
            code_version_from_file(
                "RELEASE_VERSION=0.6
"
            ),
            None
        );
        assert_eq!(
            code_version_from_file(
                "CODE_VERSION=not a version!
"
            ),
            None
        );
        assert_eq!(code_version_from_file(""), None);
        // A duplicated CODE_VERSION is malformed, even when both lines agree
        assert_eq!(
            code_version_from_file(
                "CODE_VERSION=0.6.5
CODE_VERSION=0.6.5
"
            ),
            None
        );
        assert_eq!(
            code_version_from_file(
                "CODE_VERSION=0.6.5
CODE_VERSION=garbage
"
            ),
            None
        );
    }
}
