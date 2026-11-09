//! 安全相关的小工具。**接口契约（不要改签名）**；行为依据 `docs/audit/security.md` 与 `server.py` 的
//! `host_name` / `origin_host` / `parse_allowed_hosts` / `_host_ok` / `_origin_ok` / `clean_filename`。
use std::collections::HashSet;

/// 从 Host 头（或 URL 的 netloc）取主机名：去端口，IPv6 保留方括号内的内容，转小写，去掉末尾的点。
pub fn host_name(_value: Option<&str>) -> String {
    unimplemented!()
}

/// Origin 形如 `https://example.com:8443`；返回主机名。不是 http/https（`null`、`file://`）或含 userinfo 时返回 None。
pub fn origin_host(_origin: Option<&str>) -> Option<String> {
    unimplemented!()
}

/// `XEXAM_ALLOWED_HOSTS` / `XEXAM_ALLOWED_ORIGINS`：逗号、空白、分号分隔；每项可带协议、端口、路径，统一成主机名。
pub fn parse_allowed_hosts(_text: Option<&str>) -> HashSet<String> {
    unimplemented!()
}

/// 清洗 `X-Filename`：百分号解码，取最后一段，去控制字符，最长 120 个字符，空则 `未命名.xlsx`。
pub fn clean_filename(_raw: Option<&str>) -> String {
    unimplemented!()
}

/// 16 字节 CSPRNG → base64url（无填充，22 个字符）。
pub fn new_token() -> String {
    unimplemented!()
}

/// 常量时间比较（长度不同时提前返回，与 Python `compare_digest` 一致）。
pub fn token_eq(_a: &str, _b: &str) -> bool {
    unimplemented!()
}
