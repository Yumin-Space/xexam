//! Small security utilities. **Interface contract (do not change signatures)**; behavior follows `docs/zh-CN/history/audit/security.md` and `server.py`'s
//! `host_name` / `origin_host` / `parse_allowed_hosts` / `_host_ok` / `_origin_ok` / `clean_filename`.
//!
//! Convention: wherever Python slices by character (code point), use `chars()` here, never byte slicing.
use percent_encoding::percent_decode_str;
use std::collections::HashSet;
use subtle::ConstantTimeEq;

pub mod body;

/// The set of code points for Python `str.isspace()` / regex `\s` (Unicode).
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

/// Python `str.strip()`.
pub fn py_strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

/// Strips `[\x00-\x1f\x7f]`.
pub fn strip_ctrl(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'))
        .collect()
}

/// Takes the first n characters by code point (Python `s[:n]`).
pub fn truncate_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// User input echoed into error messages: control characters removed, at most 80 characters (server.py `short`).
pub fn short(s: &str) -> String {
    truncate_chars(&strip_ctrl(s), 80)
}

/// Raw request-header bytes are decoded as latin-1 (same as Python http.client: one code point per byte).
pub fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// A common subset of Python `int(str)`: leading/trailing whitespace, sign, decimal ASCII digits, a single underscore between digits.
/// The result saturates at ±10^30; the caller clamps it to a range.
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

/// Takes the host name from a Host header (or a URL's netloc): strips the port, keeps the content inside IPv6 brackets, lowercases, strips the trailing dot.
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

/// An Origin looks like `https://example.com:8443`; returns the host name. Returns None if it is not http/https (`null`, `file://`) or contains userinfo.
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

/// `XEXAM_ALLOWED_HOSTS` / `XEXAM_ALLOWED_ORIGINS`: separated by commas, whitespace or semicolons; each item may carry a scheme, port and path, and is normalized to a host name.
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

/// Sanitizes `X-Filename`: percent-decodes, takes the last segment, strips control characters, at most 120 characters; if empty, uses the default name `未命名.xlsx`.
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

/// base64url, unpadded (RFC 4648 section 5).
pub fn base64url_nopad(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk.first().copied().unwrap_or(0) as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        let idx = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        let take = chunk.len() + 1; // 1 byte -> 2 characters, 2 bytes -> 3, 3 bytes -> 4
        for &i in idx.iter().take(take) {
            out.push(B64URL.get(i as usize).copied().unwrap_or(b'A') as char);
        }
    }
    out
}

/// 16 bytes of CSPRNG → base64url (unpadded, 22 characters).
pub fn new_token() -> String {
    let mut buf = [0u8; 16];
    if getrandom::fill(&mut buf).is_err() {
        // Never fall back to a predictable token when the system randomness source is unavailable.
        panic!("operating system random source is unavailable");
    }
    base64url_nopad(&buf)
}

/// Constant-time comparison (returns early when the lengths differ, same as Python `compare_digest`).
pub fn token_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

#[cfg(test)]
mod tests;
