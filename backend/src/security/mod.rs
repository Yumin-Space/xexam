//! 安全相关的小工具。**接口契约（不要改签名）**；行为依据 `docs/audit/security.md` 与 `server.py` 的
//! `host_name` / `origin_host` / `parse_allowed_hosts` / `_host_ok` / `_origin_ok` / `clean_filename`。
//!
//! 约定：凡是 Python 里按字符（码点）切片的地方，这里一律用 `chars()`，不按字节切。
use percent_encoding::percent_decode_str;
use std::collections::HashSet;
use subtle::ConstantTimeEq;

pub mod body;

/// Python `str.isspace()` / 正则 `\s`（Unicode）的码点集合。
pub fn is_py_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\u{0b}'
            | '\u{0c}'
            | '\r'
            | '\u{1c}'..='\u{1f}'
            | ' '
            | '\u{85}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

/// Python `str.strip()`。
pub fn py_strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

/// 去掉 `[\x00-\x1f\x7f]`。
pub fn strip_ctrl(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'))
        .collect()
}

/// 按码点取前 n 个字符（Python `s[:n]`）。
pub fn truncate_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// 回显到错误信息里的用户输入：去控制字符、最多 80 个字符（server.py `short`）。
pub fn short(s: &str) -> String {
    truncate_chars(&strip_ctrl(s), 80)
}

/// 请求头的原始字节按 latin-1 解码（与 Python http.client 一致：每个字节一个码点）。
pub fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// Python `int(str)` 的常见子集：首尾空白、正负号、十进制 ASCII 数字、数字之间的单个下划线。
/// 结果饱和在 ±10^30，调用方再夹取范围。
pub fn py_int_str(s: &str) -> Option<i128> {
    let t = py_strip(s);
    let (neg, digits) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
    {
        return None;
    }
    const CAP: i128 = 1_000_000_000_000_000_000_000_000_000_000;
    let mut v: i128 = 0;
    for c in digits.chars() {
        if c == '_' {
            continue;
        }
        let d = c.to_digit(10)? as i128;
        v = v.saturating_mul(10).saturating_add(d).min(CAP);
    }
    Some(if neg { -v } else { v })
}

/// 从 Host 头（或 URL 的 netloc）取主机名：去端口，IPv6 保留方括号内的内容，转小写，去掉末尾的点。
pub fn host_name(value: Option<&str>) -> String {
    let h = py_strip(value.unwrap_or("")).to_lowercase();
    let name: &str = if h.starts_with('[') {
        match h.find(']') {
            Some(end) if end > 0 => h.get(1..end).unwrap_or(&h),
            _ => &h,
        }
    } else {
        match h.rfind(':') {
            Some(i) => h.get(..i).unwrap_or(&h),
            None => &h,
        }
    };
    name.trim_end_matches('.').to_string()
}

/// Origin 形如 `https://example.com:8443`；返回主机名。不是 http/https（`null`、`file://`）或含 userinfo 时返回 None。
pub fn origin_host(origin: Option<&str>) -> Option<String> {
    let o = py_strip(origin.unwrap_or(""));
    let low = o.to_lowercase();
    for scheme in ["http://", "https://"] {
        if low.starts_with(scheme) {
            let rest_all: String = o.chars().skip(scheme.chars().count()).collect();
            let rest = rest_all.split('/').next().unwrap_or("");
            return if !rest.is_empty() && !rest.contains('@') {
                Some(host_name(Some(rest)))
            } else {
                None
            };
        }
    }
    None
}

/// `XEXAM_ALLOWED_HOSTS` / `XEXAM_ALLOWED_ORIGINS`：逗号、空白、分号分隔；每项可带协议、端口、路径，统一成主机名。
pub fn parse_allowed_hosts(text: Option<&str>) -> HashSet<String> {
    let mut out = HashSet::new();
    for item in text
        .unwrap_or("")
        .split(|c: char| c == ',' || c == ';' || is_py_space(c))
    {
        let item = py_strip(item);
        if item.is_empty() {
            continue;
        }
        let after = item.split_once("://").map(|(_, r)| r).unwrap_or(item);
        let hostpart = after.split('/').next().unwrap_or("");
        let n = host_name(Some(hostpart));
        if !n.is_empty() {
            out.insert(n);
        }
    }
    out
}

/// 清洗 `X-Filename`：百分号解码，取最后一段，去控制字符，最长 120 个字符，空则 `未命名.xlsx`。
pub fn clean_filename(raw: Option<&str>) -> String {
    let decoded = percent_decode_str(raw.unwrap_or(""))
        .decode_utf8_lossy()
        .replace('\\', "/");
    let last = decoded.rsplit('/').next().unwrap_or("");
    let cleaned = strip_ctrl(last);
    let name = truncate_chars(py_strip(&cleaned), 120);
    if name.is_empty() {
        "未命名.xlsx".to_string()
    } else {
        name
    }
}

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// base64url，无填充（RFC 4648 第 5 节）。
pub fn base64url_nopad(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk.first().copied().unwrap_or(0) as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        let idx = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        let take = chunk.len() + 1; // 1 字节 -> 2 个字符，2 字节 -> 3 个，3 字节 -> 4 个
        for &i in idx.iter().take(take) {
            out.push(B64URL.get(i as usize).copied().unwrap_or(b'A') as char);
        }
    }
    out
}

/// 16 字节 CSPRNG → base64url（无填充，22 个字符）。
pub fn new_token() -> String {
    let mut buf = [0u8; 16];
    if getrandom::fill(&mut buf).is_err() {
        // 系统随机源不可用时绝不退回可预测的令牌。
        panic!("operating system random source is unavailable");
    }
    base64url_nopad(&buf)
}

/// 常量时间比较（长度不同时提前返回，与 Python `compare_digest` 一致）。
pub fn token_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

#[cfg(test)]
mod tests;
