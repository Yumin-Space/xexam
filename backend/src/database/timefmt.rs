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
    fn minute_is_zero_padded() {
        let ts = Timestamp::from_second(0).expect("ts");
        assert_eq!(format_timestamp_in(ts, &TimeZone::UTC), "1970-01-01 00:00");
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
