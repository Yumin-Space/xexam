//! Timestamp: local time, no time-zone marker, minute precision, format `%Y-%m-%d %H:%M` (the old `datetime.now().strftime(...)`).
//!
//! Uses `jiff` to get the system time zone, which honors the `TZ` environment variable (`TZ=Asia/Tokyo`). Tests pin the time with [`Clock`].
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

/// Time source. `Store` calls `now()` once before each database write.
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> String;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_in_named_zones() {
        // 2026-01-07 08:00:30 UTC
        let ts = Timestamp::from_second(1_767_772_830).expect("ts");
        assert_eq!(format_timestamp_in(ts, &TimeZone::UTC), "2026-01-07 08:00");
        let tokyo = TimeZone::get("Asia/Tokyo").expect("tokyo zone");
        assert_eq!(format_timestamp_in(ts, &tokyo), "2026-01-07 17:00");
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
}
