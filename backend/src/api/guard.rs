//! Host / Origin 守卫（server.py `_host_ok` / `_origin_ok` / `_deny_reason`）。
//!
//! 只读真实的 `Host` 头，**绝不**看 `X-Forwarded-*` / `Forwarded`。Origin 只比较主机名（协议、端口不比）。
use crate::config::Config;
use crate::security::{host_name, origin_host, short, strip_ctrl, truncate_chars};

pub fn host_ok(cfg: &Config, host: Option<&str>) -> bool {
    if cfg.allow_any_host {
        return true;
    }
    cfg.allowed_hosts.contains(&host_name(host))
}

/// 没有 Origin 头 -> 放行（命令行工具不是浏览器里的跨站请求）。
pub fn origin_ok(cfg: &Config, host: Option<&str>, origin: Option<&str>) -> bool {
    let Some(origin) = origin else { return true };
    let Some(oh) = origin_host(Some(origin)) else {
        return false;
    };
    oh == host_name(host) || cfg.allowed_origins.contains(&oh)
}

/// GET 的 403 里回显的主机名：已归一化、去控制字符、最多 80 个字符。
pub fn seen_host(host: Option<&str>) -> String {
    truncate_chars(&strip_ctrl(&host_name(host)), 80)
}

/// POST 被拒绝时给出能照着做的原因；`None` 表示没问题。
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
