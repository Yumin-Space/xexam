//! Host / Origin guard (server.py `_host_ok` / `_origin_ok` / `_deny_reason`, here [`deny`]).
//!
//! Only reads the real `Host` header and **never** looks at `X-Forwarded-*` / `Forwarded`. Origin is compared by hostname only (scheme and port are not compared).
use crate::config::Config;
use crate::logs::BurstReport;
use crate::security::{host_name, origin_host, short, strip_ctrl, truncate_chars};

pub fn host_ok(cfg: &Config, host: Option<&str>) -> bool {
    if cfg.allow_any_host {
        return true;
    }
    cfg.allowed_hosts.contains(&host_name(host))
}

/// No Origin header -> allowed (command-line tools are not cross-site requests from a browser).
pub fn origin_ok(cfg: &Config, host: Option<&str>, origin: Option<&str>) -> bool {
    let Some(origin) = origin else { return true };
    let Some(oh) = origin_host(Some(origin)) else {
        return false;
    };
    oh == host_name(host) || cfg.allowed_origins.contains(&oh)
}

/// Hostname echoed in the 403 for GET: normalized, control characters removed, at most 80 characters.
pub fn seen_host(host: Option<&str>) -> String {
    truncate_chars(&strip_ctrl(&host_name(host)), 80)
}

/// A request the guard refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denial {
    /// The actionable Chinese reason sent in the 403 (the frontend translates it): which check failed and how to let it through
    pub message: String,
    /// The same reason in English for the log (log lines stay English; the System log page shows them to administrators)
    pub log: String,
}

/// Checks the Host, then the Origin; `None` means no problem. With no Origin only the Host is checked.
pub fn deny(cfg: &Config, host: Option<&str>, origin: Option<&str>) -> Option<Denial> {
    if !host_ok(cfg, host) {
        return Some(Denial {
            message: format!(
                "被拒绝：这个请求的 Host 是「{}」，不在允许的列表里。请把它加进 XEXAM_ALLOWED_HOSTS（或设 XEXAM_ALLOW_ANY_HOST=1，只在前面有身份验证时）。",
                short(host.unwrap_or(""))
            ),
            log: format!("Host {:?} is not in the allowed list", seen_host(host)),
        });
    }
    if !origin_ok(cfg, host, origin) {
        return Some(match origin_host(origin) {
            None => {
                let seen = short(origin.unwrap_or(""));
                Denial {
                    message: format!("被拒绝：这个请求的 Origin 是「{seen}」，不是一个网址。"),
                    log: format!("Origin {seen:?} is not a URL"),
                }
            }
            Some(o) => Denial {
                message: format!(
                    "被拒绝：这个请求来自「{}」，和服务看到的 Host「{}」不是同一个主机。如果你在 Cloudflare Tunnel 等反向代理后面且代理改写了 Host，请把「{}」加进 XEXAM_ALLOWED_ORIGINS。",
                    short(&o),
                    short(&host_name(host)),
                    short(&o)
                ),
                log: format!(
                    "Origin host {:?} differs from the Host {:?} and is not in the allowed origins",
                    short(&o),
                    short(&host_name(host))
                ),
            },
        });
    }
    None
}

/// Logs a [`BurstReport`] of requests rejected by the Host / Origin guard (see [`crate::logs::BurstLog`]): the guard runs
/// before sign-in, so anyone who can reach the server can make it reject requests as fast as they can send them, and one line
/// each would push every other line out of the System log.
pub fn log_rejections(report: &BurstReport) {
    match report {
        BurstReport::One(what) => tracing::warn!("rejected {what}"),
        BurstReport::Many { count, last } => tracing::warn!(
            "rejected {count} request(s) by the Host / Origin check since the last report; the last one: {last}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::from_env_with(&|_| None, "0.0.0".to_string())
    }

    #[test]
    fn a_denial_gives_a_chinese_message_and_an_english_log_text() {
        let c = cfg();
        assert_eq!(deny(&c, Some("127.0.0.1:8765"), None), None);
        assert_eq!(
            deny(&c, Some("127.0.0.1:8765"), Some("http://127.0.0.1:5173")),
            None
        );
        let cases = [
            (
                Some("evil.example:80"),
                None,
                "Host \"evil.example\" is not in the allowed list",
            ),
            (
                Some("127.0.0.1:8765"),
                Some("http://evil.example"),
                "Origin host \"evil.example\" differs from the Host \"127.0.0.1\" and is not in the allowed origins",
            ),
            (
                Some("127.0.0.1:8765"),
                Some("null"),
                "Origin \"null\" is not a URL",
            ),
        ];
        for (host, origin, log) in cases {
            let d = deny(&c, host, origin).expect("denied");
            assert_eq!(d.log, log);
            assert!(d.message.starts_with("被拒绝："), "{}", d.message);
            assert!(d.log.is_ascii(), "{}", d.log);
        }
    }
}
