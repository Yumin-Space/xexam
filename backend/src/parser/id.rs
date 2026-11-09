//! 题目 id：`sha1(sheet + "\n" + stem 去尽空白后取前 300 个码点 + "|" + first 去尽空白后取前 200 个码点)` 的前 12 位十六进制。
//! 不做任何 Unicode 规范化 / 大小写折叠；空白按 Python `\s`；截断按码点。
use super::py_space::{remove_py_space, take_chars};
use sha1::{Digest, Sha1};
use std::fmt::Write as _;

pub fn make_id(sheet: &str, stem: &str, first: &str) -> String {
    let mut raw = String::with_capacity(sheet.len() + 32);
    raw.push_str(sheet);
    raw.push('\n');
    raw.push_str(&take_chars(&remove_py_space(stem), 300));
    raw.push('|');
    raw.push_str(&take_chars(&remove_py_space(first), 200));
    let digest = Sha1::digest(raw.as_bytes());
    let mut out = String::with_capacity(12);
    for b in digest.iter().take(6) {
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::make_id;

    #[test]
    fn known_vector() {
        assert_eq!(
            make_id(
                "demo",
                "東京は日本の首都です。日本の首都はどこですか？",
                "大阪"
            ),
            "94e4bfe00e8d"
        );
    }

    #[test]
    fn whitespace_and_truncation() {
        let a = make_id("s", "a b\u{3000}c\n", "x\ty");
        let b = make_id("s", "abc", "xy");
        assert_eq!(a, b);
        let long = "あ".repeat(300);
        assert_eq!(
            make_id("s", &format!("{long}いう"), "f"),
            make_id("s", &long, "f")
        );
        let spaced: String = "あ ".repeat(400);
        assert_eq!(make_id("s", &spaced, "f"), make_id("s", &long, "f"));
        assert_ne!(make_id("s", "\u{e9}", "f"), make_id("s", "e\u{301}", "f"));
    }
}
