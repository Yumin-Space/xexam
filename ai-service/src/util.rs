//! Small helpers shared by the modules: time, hex, text clean-up.

/// Unix time in seconds.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Lower-case hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    out
}

/// `true` when `s` is exactly `len` lower-case hex digits.
pub fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The local calendar date (`YYYY-MM-DD`) of a unix time, in the container's time zone (`TZ` or `/etc/localtime`).
pub fn local_date(ts: i64) -> String {
    match jiff::Timestamp::from_second(ts) {
        Ok(t) => t.to_zoned(jiff::tz::TimeZone::system()).date().to_string(),
        Err(_) => "1970-01-01".to_string(),
    }
}

/// The local date and time to the minute (`YYYY-MM-DD HH:MM`) of a unix time, in the container's time zone.
pub fn local_minute(ts: i64) -> String {
    match jiff::Timestamp::from_second(ts) {
        Ok(t) => t
            .to_zoned(jiff::tz::TimeZone::system())
            .strftime("%Y-%m-%d %H:%M")
            .to_string(),
        Err(_) => "1970-01-01 00:00".to_string(),
    }
}

/// Today's local date.
pub fn local_today() -> jiff::civil::Date {
    jiff::Zoned::now().date()
}

/// Collapses every run of whitespace (newlines included) into one space and trims the ends.
pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The first `max` characters of `s`.
pub fn cut_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Number of characters (Unicode scalar values) in `s`.
pub fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// Chat title from the first user message: whitespace collapsed, at most 60 characters (a longer text ends with `…`).
pub fn chat_title(text: &str) -> String {
    let t = collapse_ws(text);
    if char_len(&t) <= 60 {
        t
    } else {
        let mut s = cut_chars(&t, 59);
        s.push('…');
        s
    }
}

/// Removes control characters (keeps nothing below U+0020 and no U+007F..U+009F).
pub fn strip_ctrl(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_checks() {
        assert_eq!(hex(&[0x00, 0xab, 0xff]), "00abff");
        assert!(is_lower_hex("00abff", 6));
        assert!(!is_lower_hex("00ABff", 6));
        assert!(!is_lower_hex("00abf", 6));
    }

    #[test]
    fn titles_are_collapsed_and_cut() {
        assert_eq!(
            chat_title("  What is\n\n a   matrix? "),
            "What is a matrix?"
        );
        let long = "x".repeat(80);
        let t = chat_title(&long);
        assert_eq!(char_len(&t), 60);
        assert!(t.ends_with('…'));
        assert_eq!(chat_title(&"y".repeat(60)), "y".repeat(60));
    }

    #[test]
    fn local_minute_has_the_minute_shape() {
        let d = local_minute(1_700_000_000);
        assert_eq!(d.len(), 16);
        assert_eq!(d.as_bytes()[10], b' ');
        assert_eq!(d.as_bytes()[13], b':');
        assert_eq!(local_minute(i64::MAX), "1970-01-01 00:00");
    }

    #[test]
    fn local_date_has_the_iso_shape() {
        let d = local_date(1_700_000_000);
        assert_eq!(d.len(), 10);
        assert_eq!(d.as_bytes()[4], b'-');
    }
}
