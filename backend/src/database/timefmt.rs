//! 时间戳：本地时间、无时区标记、精确到分钟，格式 `%Y-%m-%d %H:%M`（旧版 `datetime.now().strftime(...)`）。
//!
//! 用 `jiff` 取系统时区，会读取 `TZ` 环境变量（`TZ=Asia/Tokyo`）。测试用 [`Clock`] 固定时间。
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

/// 时间来源。`Store` 每次写库前调用一次 `now()`。
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> String;
}

/// 真实的本地时间。
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> String {
        local_now()
    }
}

/// 固定时间（测试用）。
#[derive(Debug, Clone)]
pub struct FixedClock(pub String);

impl Clock for FixedClock {
    fn now(&self) -> String {
        self.0.clone()
    }
}

/// 用函数 / 闭包做时钟。
pub struct FnClock<F>(pub F);

impl<F> Clock for FnClock<F>
where
    F: Fn() -> String + Send + Sync + 'static,
{
    fn now(&self) -> String {
        (self.0)()
    }
}

/// 把一个带时区的时刻格式化成 `YYYY-MM-DD HH:MM`。不会 panic（不走 `Display`）。
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

/// 某个时刻在指定时区的 `YYYY-MM-DD HH:MM`。
pub fn format_timestamp_in(ts: Timestamp, tz: &TimeZone) -> String {
    format_minute(&ts.to_zoned(tz.clone()))
}

/// 现在的本地时间（系统时区；`TZ` 环境变量优先）。
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
