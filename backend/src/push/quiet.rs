//! Quiet hours (contracts C.7): no notification between 00:00 (inclusive) and 08:00 (exclusive) local time.

use jiff::tz::TimeZone;
use jiff::Timestamp;

/// Start of the quiet window (inclusive local hour).
pub const QUIET_START_HOUR: i8 = 0;
/// End of the quiet window (exclusive local hour).
pub const QUIET_END_HOUR: i8 = 8;

/// The zone for a name that is a valid IANA zone; `auto`, empty and unknown names give `None`.
fn named(name: Option<&str>) -> Option<TimeZone> {
    let name = name?.trim();
    if name.is_empty() || name.len() > 64 || name.eq_ignore_ascii_case("auto") {
        return None;
    }
    TimeZone::get(name).ok()
}

/// The zone for a user: their `tz` setting when it is an IANA zone, else the device's zone, else the system zone.
pub fn zone_for(user_tz: Option<&str>, device_tz: Option<&str>) -> TimeZone {
    named(user_tz)
        .or_else(|| named(device_tz))
        .unwrap_or_else(TimeZone::system)
}

/// `now` when it is outside the window, else 08:00:00 local time the same day (unix seconds).
pub fn release_at(now: i64, zone: &TimeZone) -> i64 {
    let Ok(ts) = Timestamp::from_second(now) else {
        return now;
    };
    let local = ts.to_zoned(zone.clone());
    let hour = local.hour();
    if !(QUIET_START_HOUR..QUIET_END_HOUR).contains(&hour) {
        return now;
    }
    // A DST gap at 08:00 resolves with the compatible disambiguation (the first valid instant after it)
    local
        .date()
        .at(QUIET_END_HOUR, 0, 0, 0)
        .to_zoned(zone.clone())
        .map(|z| z.timestamp().as_second())
        .unwrap_or(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(name: &str) -> TimeZone {
        TimeZone::get(name).expect("zone")
    }

    /// Unix seconds of an RFC 3339 instant.
    fn at(text: &str) -> i64 {
        text.parse::<Timestamp>().expect("timestamp").as_second()
    }

    fn local(ts: i64, zone: &TimeZone) -> String {
        Timestamp::from_second(ts)
            .expect("ts")
            .to_zoned(zone.clone())
            .strftime("%Y-%m-%d %H:%M")
            .to_string()
    }

    #[test]
    fn utc_window_edges() {
        let utc = TimeZone::UTC;
        let day = at("2026-03-10T00:00:00Z");
        let eight = at("2026-03-10T08:00:00Z");
        assert_eq!(
            release_at(at("2026-03-09T23:59:00Z"), &utc),
            at("2026-03-09T23:59:00Z")
        );
        assert_eq!(release_at(day, &utc), eight);
        assert_eq!(release_at(at("2026-03-10T03:30:00Z"), &utc), eight);
        assert_eq!(release_at(at("2026-03-10T07:59:59Z"), &utc), eight);
        assert_eq!(release_at(eight, &utc), eight);
        assert_eq!(
            release_at(at("2026-03-10T12:00:00Z"), &utc),
            at("2026-03-10T12:00:00Z")
        );
    }

    #[test]
    fn release_is_never_the_next_day() {
        let utc = TimeZone::UTC;
        let start = at("2026-03-10T00:00:00Z");
        for step in 0..(8 * 60) {
            let now = start + step * 60;
            let released = release_at(now, &utc);
            assert_eq!(released, start + 8 * 3600, "{now}");
        }
        for step in (8 * 60)..(24 * 60) {
            let now = start + step * 60;
            assert_eq!(release_at(now, &utc), now);
        }
    }

    #[test]
    fn new_york_spring_forward() {
        // 2026-03-08: clocks jump from 02:00 to 03:00 local; 08:00 local is EDT (UTC-4)
        let ny = zone("America/New_York");
        let now = at("2026-03-08T05:30:00Z"); // 00:30 EST
        assert_eq!(local(now, &ny), "2026-03-08 00:30");
        let released = release_at(now, &ny);
        assert_eq!(local(released, &ny), "2026-03-08 08:00");
        assert_eq!(released, at("2026-03-08T12:00:00Z"));
        // 07:59 EDT is still quiet, 08:00 EDT is not
        let late = at("2026-03-08T11:59:00Z");
        assert_eq!(release_at(late, &ny), at("2026-03-08T12:00:00Z"));
        assert_eq!(
            release_at(at("2026-03-08T12:00:00Z"), &ny),
            at("2026-03-08T12:00:00Z")
        );
    }

    #[test]
    fn new_york_fall_back() {
        // 2026-11-01: 01:00-02:00 happens twice; 08:00 local is EST (UTC-5)
        let ny = zone("America/New_York");
        let now = at("2026-11-01T06:30:00Z"); // 01:30 EST (second pass)
        let released = release_at(now, &ny);
        assert_eq!(local(released, &ny), "2026-11-01 08:00");
        assert_eq!(released, at("2026-11-01T13:00:00Z"));
        let first_pass = at("2026-11-01T05:30:00Z"); // 01:30 EDT
        assert_eq!(release_at(first_pass, &ny), at("2026-11-01T13:00:00Z"));
    }

    #[test]
    fn berlin_dst_dates() {
        let berlin = zone("Europe/Berlin");
        // 2026-03-29: 02:00 -> 03:00 (CEST, UTC+2)
        let now = at("2026-03-28T23:30:00Z"); // 00:30 CET on the 29th
        assert_eq!(local(now, &berlin), "2026-03-29 00:30");
        assert_eq!(release_at(now, &berlin), at("2026-03-29T06:00:00Z"));
        // 2026-10-25: 03:00 -> 02:00 (CET, UTC+1)
        let now = at("2026-10-24T22:30:00Z"); // 00:30 CEST on the 25th
        assert_eq!(release_at(now, &berlin), at("2026-10-25T07:00:00Z"));
    }

    #[test]
    fn auckland_dst_dates() {
        let auckland = zone("Pacific/Auckland");
        // 2026-09-27: 02:00 -> 03:00 (NZDT, UTC+13)
        let now = at("2026-09-26T12:30:00Z"); // 00:30 NZST on the 27th
        assert_eq!(local(now, &auckland), "2026-09-27 00:30");
        assert_eq!(release_at(now, &auckland), at("2026-09-26T19:00:00Z"));
        // 2026-04-05: 03:00 -> 02:00 (NZST, UTC+12)
        let now = at("2026-04-04T11:30:00Z"); // 00:30 NZDT on the 5th
        assert_eq!(release_at(now, &auckland), at("2026-04-04T20:00:00Z"));
    }

    #[test]
    fn zone_chain() {
        let name = |z: &TimeZone| z.iana_name().map(str::to_string);
        // The user's zone wins
        assert_eq!(
            name(&zone_for(Some("Europe/Berlin"), Some("Pacific/Auckland"))).as_deref(),
            Some("Europe/Berlin")
        );
        // auto -> the device's zone
        assert_eq!(
            name(&zone_for(Some("auto"), Some("Europe/Berlin"))).as_deref(),
            Some("Europe/Berlin")
        );
        assert_eq!(
            name(&zone_for(None, Some("Europe/Berlin"))).as_deref(),
            Some("Europe/Berlin")
        );
        // An invalid user zone is skipped
        assert_eq!(
            name(&zone_for(Some("Not/AZone"), Some("Europe/Berlin"))).as_deref(),
            Some("Europe/Berlin")
        );
        // Both invalid -> the system zone
        let system = TimeZone::system();
        let fallback = zone_for(Some("Not/AZone"), Some(""));
        assert_eq!(name(&fallback), name(&system));
        assert_eq!(name(&zone_for(None, None)), name(&system));
        // Hostile text never panics
        let _ = zone_for(Some(&"x".repeat(500)), Some("\u{0}\u{7f}"));
    }

    #[test]
    fn auto_with_device_zone_shifts_the_release() {
        // 23:30 UTC is 00:30 in Berlin (CET): quiet there, not in UTC
        let now = at("2026-02-10T23:30:00Z");
        let berlin = zone_for(Some("auto"), Some("Europe/Berlin"));
        assert_eq!(release_at(now, &berlin), at("2026-02-11T07:00:00Z"));
        assert_eq!(release_at(now, &TimeZone::UTC), now);
    }

    #[test]
    fn out_of_range_instant_is_returned_unchanged() {
        assert_eq!(release_at(i64::MAX, &TimeZone::UTC), i64::MAX);
        assert_eq!(release_at(i64::MIN, &TimeZone::UTC), i64::MIN);
    }
}
