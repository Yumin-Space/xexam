//! Host / Origin guard (server.py `_host_ok` / `_origin_ok` / `_deny_reason`).
//!
//! Only reads the real `Host` header and **never** looks at `X-Forwarded-*` / `Forwarded`. Origin is compared by hostname only (scheme and port are not compared).
use crate::config::Config;
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

/// When a POST is rejected, gives an actionable reason; `None` means no problem.
pub fn deny_reason(cfg: &Config, host: Option<&str>, origin: Option<&str>) -> Option<String> {
    if !host_ok(cfg, host) {
        return Some(format!(
            "被拒绝：这个请求的 Host 是「{}」，不在允许的列表里。请把它加进 XEXAM_ALLOWED_HOSTS（或设 XEXAM_ALLOW_ANY_HOST=1，只在前面有身份验证时）。",
            short(host.unwrap_or(""))
        ));
    }
    if !origin_ok(cfg, host, origin) {
        return Some(match origin_host(origin) {
            None => format!("被拒绝：这个请求的 Origin 是「{}」，不是一个网址。", short(origin.unwrap_or(""))),
            Some(o) => format!(
                "被拒绝：这个请求来自「{}」，和服务看到的 Host「{}」不是同一个主机。如果你在 Cloudflare Tunnel 等反向代理后面且代理改写了 Host，请把「{}」加进 XEXAM_ALLOWED_ORIGINS。",
                short(&o),
                short(&host_name(host)),
                short(&o)
            ),
        });
    }
    None
}
