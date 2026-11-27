//! Timestamp: local time, no time-zone marker, minute precision, format `%Y-%m-%d %H:%M` (the old `datetime.now().strftime(...)`).
//!
//! Uses `jiff` to get the system time zone, which honors the `TZ` environment variable and otherwise reads `/etc/localtime` (in Docker: the host's, mounted read only). Tests pin the time with [`Clock`].
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

/// Time source. `Store` calls `now()` once before each database write.
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> String;

    /// The same instant as Unix seconds, for the request path (session gates). The real clock reads the system time; a
    /// pinned test clock derives it from its own string (read as UTC), so no real minute boundary can reach the tests.
    fn unix(&self) -> i64 {
        crate::auth::unix_now()
    }
}

/// `YYYY-MM-DD HH:MM` (or with seconds) read as UTC Unix seconds; `None` when it does not parse.
fn unix_of_minute(s: &str) -> Option<i64> {
    let fmt = if s.len() > 16 {
        "%Y-%m-%d %H:%M:%S"
    } else {
        "%Y-%m-%d %H:%M"
    };
    let dt = jiff::civil::DateTime::strptime(fmt, s).ok()?;
    Some(dt.to_zoned(TimeZone::UTC).ok()?.timestamp().as_second())
}

/// The real local time.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> String {
        local_now()
    }
}

/// A fixed time (for tests).
#[derive(Debug, Clone)]
pub struct FixedClock(pub String);

impl Clock for FixedClock {
    fn now(&self) -> String {
        self.0.clone()
    }

    fn unix(&self) -> i64 {
        unix_of_minute(&self.0).unwrap_or_else(crate::auth::unix_now)
    }
}

/// A clock backed by a function / closure.
pub struct FnClock<F>(pub F);

impl<F> Clock for FnClock<F>
where
    F: Fn() -> String + Send + Sync + 'static,
{
    fn now(&self) -> String {
        (self.0)()
    }

    fn unix(&self) -> i64 {
        unix_of_minute(&(self.0)()).unwrap_or_else(crate::auth::unix_now)
    }
}

/// Formats a zoned instant as `YYYY-MM-DD HH:MM`. Never panics (does not go through `Display`).
pub fn format_minute(z: &Zoned) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        z.year(),
        z.month(),
        z.day(),
        z.hour(),
        z.minute()
    )
}

/// An instant as `YYYY-MM-DD HH:MM` in the given time zone.
pub fn format_timestamp_in(ts: Timestamp, tz: &TimeZone) -> String {
    format_minute(&ts.to_zoned(tz.clone()))
}

/// The current local time (system time zone; the `TZ` environment variable takes precedence).
pub fn local_now() -> String {
    format_minute(&Zoned::now())
}

/// Formats a zoned instant as `YYYY-MM-DD HH:MM:SS`.
pub fn format_second(z: &Zoned) -> String {
    format!("{}:{:02}", format_minute(z), z.second())
}

/// The local time `secs` seconds from now as `YYYY-MM-DD HH:MM:SS` (when a login lock ends; seconds matter for short locks).
pub fn local_in_secs(secs: u64) -> String {
    let now = Zoned::now();
    let later = i64::try_from(secs)
        .ok()
        .and_then(|s| now.checked_add(jiff::SignedDuration::from_secs(s)).ok())
        .unwrap_or(now);
    format_second(&later)
}

/// Formats a zoned instant as `YYYYmmdd-HHMMSS` (file-name safe; used for backup names).
pub fn format_file_stamp(z: &Zoned) -> String {
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        z.year(),
        z.month(),
        z.day(),
        z.hour(),
        z.minute(),
        z.second()
    )
}

/// The current local time as `YYYYmmdd-HHMMSS`.
pub fn local_file_stamp() -> String {
    format_file_stamp(&Zoned::now())
}

/// `now` (`YYYY-MM-DD HH:MM`) moved back by `days` calendar days, in the same format. `None` when `now` does not parse or the
/// result is out of range.
pub fn minus_days(now: &str, days: i64) -> Option<String> {
    // Exactly `YYYY-MM-DD HH:MM`: a date alone is not a timestamp
    let (d, t) = now.trim().split_once(' ')?;
    let dt = format!("{d}T{t}").parse::<jiff::civil::DateTime>().ok()?;
    let earlier = dt
        .checked_sub(jiff::Span::new().try_days(days).ok()?)
        .ok()?;
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        earlier.year(),
        earlier.month(),
        earlier.day(),
        earlier.hour(),
        earlier.minute()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_in_named_zones() {
        // 2026-01-07 08:00:30 UTC
        let ts = Timestamp::from_second(1_767_772_830).expect("ts");
        assert_eq!(format_timestamp_in(ts, &TimeZone::UTC), "2026-01-07 08:00");
        // A named zone that every tz database has (the "etcetera" file): Etc/GMT+5 is UTC-5 (POSIX inverts the sign)
        let minus_five = TimeZone::get("Etc/GMT+5").expect("named zone");
        assert_eq!(format_timestamp_in(ts, &minus_five), "2026-01-07 03:00");
    }

    #[test]
    fn seconds_and_offsets() {
        // 2026-01-07 08:00:05 UTC
        let ts = Timestamp::from_second(1_767_772_805).expect("ts");
        assert_eq!(
            format_second(&ts.to_zoned(TimeZone::UTC)),
            "2026-01-07 08:00:05"
        );
        let s = local_in_secs(90);
        assert_eq!(s.len(), 19, "{s}");
        assert!(s > format_second(&Zoned::now()), "{s}");
    }

    #[test]
    fn minute_is_zero_padded() {
        let ts = Timestamp::from_second(0).expect("ts");
        assert_eq!(format_timestamp_in(ts, &TimeZone::UTC), "1970-01-01 00:00");
    }

    #[test]
    fn file_stamp_has_seconds_and_no_separators_that_paths_dislike() {
        // 2026-01-07 08:00:30 UTC
        let ts = Timestamp::from_second(1_767_772_830).expect("ts");
        assert_eq!(
            format_file_stamp(&ts.to_zoned(TimeZone::UTC)),
            "20260107-080030"
        );
        let s = local_file_stamp();
        assert_eq!(s.len(), 15, "{s}");
        assert!(s.chars().all(|c| c.is_ascii_digit() || c == '-'), "{s}");
    }

    #[test]
    fn clocks() {
        assert_eq!(
            FixedClock("2026-02-03 04:05".into()).now(),
            "2026-02-03 04:05"
        );
        assert_eq!(FnClock(|| "x".to_string()).now(), "x");
        let n = SystemClock.now();
        assert_eq!(n.len(), 16, "{n}");
        assert_eq!(n.as_bytes().get(10), Some(&b' '));
    }

    #[test]
    fn minus_days_handles_edges() {
        assert_eq!(
            minus_days("2026-11-22 14:05", 30).as_deref(),
            Some("2026-10-23 14:05")
        );
        assert_eq!(
            minus_days("2026-03-01 00:00", 1).as_deref(),
            Some("2026-02-28 00:00")
        );
        assert_eq!(
            minus_days("2028-03-01 09:30", 1).as_deref(),
            Some("2028-02-29 09:30")
        );
        assert_eq!(
            minus_days("2026-01-10 23:59", 30).as_deref(),
            Some("2025-12-11 23:59")
        );
        assert_eq!(
            minus_days("2026-11-22 14:05", 0).as_deref(),
            Some("2026-11-22 14:05")
        );
        for bad in ["", "nope", "2026-13-01 10:00", "2026-11-22"] {
            assert_eq!(minus_days(bad, 3), None, "{bad}");
        }
    }
}
