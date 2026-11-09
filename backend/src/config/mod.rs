//! 配置：环境变量 `XEXAM_*` 与命令行参数。依据 `server.py main()` 与 `docs/migration-plan.md` §3。
use crate::security::parse_allowed_hosts;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub host: String,
    pub port: u16,
    pub allowed_hosts: HashSet<String>,   // 已含 localhost / 127.0.0.1 / ::1
    pub allowed_origins: HashSet<String>, // 已含 allowed_hosts
    pub allow_any_host: bool,
    pub allow_shutdown: bool,
    pub parse_timeout_secs: u64,
    pub version: String,
}

pub const MAX_UPLOAD: usize = 20 * 1024 * 1024;
pub const MAX_JSON: usize = 1_000_000;
pub const PENDING_TTL_SECS: u64 = 30 * 60;
pub const SKIP_SHOWN: usize = 30;

impl Config {
    /// 从环境变量读取。`get` 注入环境（测试时不用改进程环境）。
    pub fn from_env_with(get: &dyn Fn(&str) -> Option<String>, version: String) -> Config {
        let extra = parse_allowed_hosts(get("XEXAM_ALLOWED_HOSTS").as_deref());
        let origins = parse_allowed_hosts(get("XEXAM_ALLOWED_ORIGINS").as_deref());
        let any = get("XEXAM_ALLOW_ANY_HOST")
            .map(|v| matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false);
        let mut hosts: HashSet<String> = ["localhost", "127.0.0.1", "::1"].iter().map(|s| s.to_string()).collect();
        hosts.extend(extra.iter().cloned());
        let mut allowed_origins = origins;
        allowed_origins.extend(extra);
        Config {
            data_dir: PathBuf::from(get("XEXAM_DATA").filter(|s| !s.is_empty()).unwrap_or_else(|| "data".to_string())),
            host: get("XEXAM_HOST").unwrap_or_else(|| "127.0.0.1".to_string()),
            port: get("XEXAM_PORT").and_then(|s| s.trim().parse().ok()).unwrap_or(8765),
            allowed_hosts: hosts,
            allowed_origins,
            allow_any_host: any,
            allow_shutdown: false,
            parse_timeout_secs: get("XEXAM_PARSE_TIMEOUT_SECS").and_then(|s| s.trim().parse().ok()).unwrap_or(30),
            version,
        }
    }
}
