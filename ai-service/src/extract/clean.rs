//! Clean-up of extracted blocks (IX-5).
use crate::gen::is_hidden_format;

/// `\r\n` / `\r` → `\n`; C0 / C1 controls dropped except `\n` and `\t`; the hidden format characters of the names rule
/// and U+FFFD dropped; trailing spaces of each line trimmed; runs of more than one blank line collapsed; leading and
/// trailing blank lines dropped. An empty result means the block is dropped.
pub fn clean_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\n' | '\t' => out.push(c),
            c if c.is_control() => {}
            c if is_hidden_format(c) || c == '\u{FFFD}' => {}
            c => out.push(c),
        }
    }
    let mut lines: Vec<&str> = Vec::new();
    let mut blank_run = 0;
    for line in out.split('\n') {
        let line = line.trim_end_matches([' ', '\t']);
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
            lines.push("");
        } else {
            blank_run = 0;
            lines.push(line);
        }
    }
    lines.join("\n").trim_matches('\n').to_string()
}

/// Drops a single space between two CJK characters (the PDF library spaces glyphs): Han, kana and CJK punctuation.
pub fn drop_cjk_spaces(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == ' ' && i > 0 && i + 1 < chars.len() && is_cjk(chars[i - 1]) && is_cjk(chars[i + 1])
        {
            continue;
        }
        out.push(c);
    }
    out
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3000..=0x303F | 0x3040..=0x30FF | 0x31F0..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FA1F)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_ends_controls_and_hidden_characters() {
        assert_eq!(clean_text("a\r\nb\rc"), "a\nb\nc");
        assert_eq!(clean_text("a\u{0}b\u{7}c\u{85}d"), "abcd");
        // bidi override, zero-width space, BOM and the replacement character go; the joiner stays
        assert_eq!(clean_text("a\u{202E}b\u{200B}c\u{FEFF}d\u{FFFD}e"), "abcde");
        assert_eq!(clean_text("x\u{200D}y"), "x\u{200D}y");
        assert_eq!(clean_text("tab\there"), "tab\there");
    }

    #[test]
    fn spaces_and_blank_lines() {
        assert_eq!(clean_text("a  \n\n\n\nb \t\n"), "a\n\nb");
        assert_eq!(clean_text("\n\n  \n"), "");
        assert_eq!(clean_text("   "), "");
    }

    #[test]
    fn cjk_spaces_are_dropped_only_between_cjk_characters() {
        assert_eq!(drop_cjk_spaces("请 问 哪 种"), "请问哪种");
        assert_eq!(drop_cjk_spaces("请 abc 问"), "请 abc 问");
        assert_eq!(drop_cjk_spaces("a b"), "a b");
        assert_eq!(drop_cjk_spaces("ア イ。 ウ"), "アイ。ウ");
    }
}
