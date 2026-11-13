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

/// `YYYYMMDDHHMMSS` of a unix time in the server's time zone (the time stamp of a generated bank's name).
pub fn local_stamp(ts: i64) -> String {
    match jiff::Timestamp::from_second(ts) {
        Ok(t) => t
            .to_zoned(jiff::tz::TimeZone::system())
            .strftime("%Y%m%d%H%M%S")
            .to_string(),
        Err(_) => "19700101000000".to_string(),
    }
}

/// `YYYY-MM-DD HH:MM:SS` of a unix time in the server's time zone (the `at` of the create route).
pub fn local_second(ts: i64) -> String {
    match jiff::Timestamp::from_second(ts) {
        Ok(t) => t
            .to_zoned(jiff::tz::TimeZone::system())
            .strftime("%Y-%m-%d %H:%M:%S")
            .to_string(),
        Err(_) => "1970-01-01 00:00:00".to_string(),
    }
}

/// Today's local date.
pub fn local_today() -> jiff::civil::Date {
    jiff::Zoned::now().date()
}

/// Today's bounds in the server's time zone (`TZ`, else `/etc/localtime`, else UTC): see [`day_bounds_in`].
pub fn day_bounds(now: i64) -> crate::quota::Day {
    day_bounds_in(now, &jiff::tz::TimeZone::system())
}

/// The calendar day of `now` in `tz`: `start` = that day's first moment, `next` = the next day's first moment (unix s).
/// Days are 23 or 25 hours long around a daylight-saving change, so `next` is never `start + 86400` (except in the UTC
/// fallback used when jiff cannot compute the bounds).
pub fn day_bounds_in(now: i64, tz: &jiff::tz::TimeZone) -> crate::quota::Day {
    let bounds = || -> Result<crate::quota::Day, jiff::Error> {
        let z = jiff::Timestamp::from_second(now)?.to_zoned(tz.clone());
        let start = z.start_of_day()?;
        let next = z.tomorrow()?.start_of_day()?;
        Ok(crate::quota::Day {
            start: start.timestamp().as_second(),
            next: next.timestamp().as_second(),
        })
    };
    bounds().unwrap_or_else(|_| {
        let start = now - now.rem_euclid(86400);
        crate::quota::Day {
            start,
            next: start + 86400,
        }
    })
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

    #[test]
    fn day_bounds_utc_and_fixed_offset() {
        use jiff::tz::{Offset, TimeZone};
        let utc = TimeZone::UTC;
        let now = 1_700_000_000;
        let d = day_bounds_in(now, &utc);
        assert!(d.start <= now && now < d.next);
        assert_eq!(d.next - d.start, 86400);
        assert_eq!(d.start.rem_euclid(86400), 0);
        // exactly at 0:00 the day starts now; one second before belongs to the previous day
        assert_eq!(day_bounds_in(d.start, &utc).start, d.start);
        let prev = day_bounds_in(d.start - 1, &utc);
        assert_eq!(prev.next, d.start);
        assert_eq!(d.start - prev.start, 86400);
        // a fixed non-zero offset: the bounds are that zone's civil midnight
        let tz = TimeZone::fixed(Offset::from_seconds(5 * 3600 + 1800).expect("offset"));
        let d = day_bounds_in(now, &tz);
        assert!(d.start <= now && now < d.next);
        assert_eq!(d.next - d.start, 86400);
        let civil = jiff::Timestamp::from_second(d.start)
            .expect("ts")
            .to_zoned(tz.clone());
        assert_eq!(civil.strftime("%H:%M:%S").to_string(), "00:00:00");
        assert_eq!(day_bounds_in(d.start, &tz).start, d.start);
        assert_eq!(day_bounds_in(d.start - 1, &tz).next, d.start);
        // the system-zone function holds the same invariants
        let d = day_bounds(now);
        assert!(d.start <= now && now < d.next);
    }

    #[test]
    fn day_bounds_follow_a_skipped_midnight_and_a_repeated_hour() {
        use jiff::tz::TimeZone;
        // a nameless POSIX rule: offsets only, no real zone
        let tz = TimeZone::posix("<+0130>-1:30<+0230>,M3.2.0/0,M11.1.0/0").expect("rule");
        let at = |s: &str| -> i64 {
            s.parse::<jiff::civil::DateTime>()
                .expect("civil")
                .to_zoned(tz.clone())
                .expect("zoned")
                .timestamp()
                .as_second()
        };
        // 2026-03-08: 0:00 does not exist; the day starts at 01:00 local and lasts 23 hours
        let now = at("2026-03-08T12:00:00");
        let d = day_bounds_in(now, &tz);
        let start = jiff::Timestamp::from_second(d.start)
            .expect("ts")
            .to_zoned(tz.clone());
        assert_eq!(
            start.strftime("%Y-%m-%d %H:%M").to_string(),
            "2026-03-08 01:00"
        );
        assert_eq!(d.next - d.start, 23 * 3600);
        assert!(d.start <= now && now < d.next);
        // 2026-10-31: the clocks go back, 23:00-24:00 repeats; the day lasts 25 hours
        for s in [
            "2026-10-31T00:30:00",
            "2026-10-31T12:00:00",
            "2026-10-31T23:30:00",
        ] {
            let now = at(s);
            let d = day_bounds_in(now, &tz);
            assert_eq!(d.next - d.start, 25 * 3600, "{s}");
            assert!(d.start <= now && now < d.next, "{s}");
        }
    }
}
