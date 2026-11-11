//! Question id: the first 12 hex digits of `sha1(sheet + "\n" + first 300 code points of stem with all whitespace removed + "|" + first 200 code points of first with all whitespace removed)`.
//! No Unicode normalization / case folding; whitespace follows Python `\s`; truncation is by code point.
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
                "一週は一年の部分です。一年の部分はどれですか？",
                "三年"
            ),
            "f3886e9f5948"
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
