//! Per-user caps on AI model calls: the pure counting function (no I/O). Minute and hour are rolling windows (the last
//! 60 s / 3600 s); the day is the calendar day of the server's time zone and resets at its 0:00.

/// One of the three caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    Minute,
    Hour,
    Day,
}

impl Cap {
    /// The name used in the JSON answers.
    pub fn key(self) -> &'static str {
        match self {
            Cap::Minute => "minute",
            Cap::Hour => "hour",
            Cap::Day => "day",
        }
    }

    /// Length of the rolling window in seconds; `None` for the calendar day.
    pub fn window(self) -> Option<i64> {
        match self {
            Cap::Minute => Some(60),
            Cap::Hour => Some(3600),
            Cap::Day => None,
        }
    }

    /// Tie-break rank: the longer cap is the larger.
    fn rank(self) -> u8 {
        match self {
            Cap::Minute => 0,
            Cap::Hour => 1,
            Cap::Day => 2,
        }
    }
}

/// The three limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub minute: i64,
    pub hour: i64,
    pub day: i64,
}

/// Calls counted in each window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Used {
    pub minute: i64,
    pub hour: i64,
    pub day: i64,
}

/// A cap that is reached: which one, its limit, and when one more call is possible (unix seconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub cap: Cap,
    pub limit: i64,
    pub retry_at: i64,
}

/// Today in the server's time zone (unix s): `start` = today's 0:00, `next` = the next day's 0:00; `start <= now < next`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Day {
    pub start: i64,
    pub next: i64,
}

/// The starts inside the rolling window (`start > now - window`), sorted ascending.
fn rolling(starts: &[i64], now: i64, window: i64) -> Vec<i64> {
    let mut v: Vec<i64> = starts
        .iter()
        .copied()
        .filter(|&s| s > now.saturating_sub(window))
        .collect();
    v.sort_unstable();
    v
}

fn count_of(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// Counts the calls in each window. `starts` are the start times (unix s) of the user's counted calls, finished and
/// running, not including the call being admitted. Minute / hour: a start counts when `start > now - window`; day: when
/// `start >= day.start`.
pub fn used(starts: &[i64], now: i64, day: Day) -> Used {
    Used {
        minute: count_of(rolling(starts, now, 60).len()),
        hour: count_of(rolling(starts, now, 3600).len()),
        day: count_of(starts.iter().filter(|&&s| s >= day.start).count()),
    }
}

/// Admits one more call or names the cap that refuses it. When several caps are hit the one with the latest `retry_at`
/// wins (ties: the longer window; the day counts as the longest). `retry_at` is at least `now + 1`.
pub fn check(starts: &[i64], now: i64, day: Day, caps: Caps) -> Result<Used, Hit> {
    let u = used(starts, now, day);
    let mut hits: Vec<Hit> = Vec::new();
    for (cap, limit) in [(Cap::Minute, caps.minute), (Cap::Hour, caps.hour)] {
        let w = cap.window().unwrap_or(0);
        let inside = rolling(starts, now, w);
        let n = usize::try_from(limit.max(0)).unwrap_or(usize::MAX);
        if inside.len() >= n {
            // enough of the oldest calls must leave the window for one more to fit
            let oldest_needed = inside.get(inside.len() - n).copied().unwrap_or(now);
            hits.push(Hit {
                cap,
                limit,
                retry_at: oldest_needed + w,
            });
        }
    }
    if u.day >= caps.day {
        hits.push(Hit {
            cap: Cap::Day,
            limit: caps.day,
            retry_at: day.next,
        });
    }
    match hits.into_iter().max_by_key(|h| (h.retry_at, h.cap.rank())) {
        Some(mut h) => {
            h.retry_at = h.retry_at.max(now + 1);
            Err(h)
        }
        None => Ok(u),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Day = Day {
        start: 1_000_000,
        next: 1_086_400,
    };
    const CAPS: Caps = Caps {
        minute: 2,
        hour: 5,
        day: 8,
    };

    fn now() -> i64 {
        DAY.start + 40_000
    }

    #[test]
    fn empty_and_under_every_cap() {
        let r = check(&[], now(), DAY, CAPS).expect("ok");
        assert_eq!((r.minute, r.hour, r.day), (0, 0, 0));
        let n = now();
        let r = check(&[n - 10, n - 500, n - 5000], n, DAY, CAPS).expect("ok");
        assert_eq!((r.minute, r.hour, r.day), (1, 2, 3));
    }

    #[test]
    fn minute_cap_is_hit_at_the_limit_and_frees_when_the_oldest_leaves() {
        let n = now();
        let h = check(&[n - 50, n - 10], n, DAY, CAPS).expect_err("hit");
        assert_eq!(h.cap, Cap::Minute);
        assert_eq!(h.limit, 2);
        assert_eq!(h.retry_at, n - 50 + 60);
        // three inside with a limit of two: the second-oldest must leave first
        let h = check(&[n - 10, n - 50, n - 30], n, DAY, CAPS).expect_err("hit");
        assert_eq!(h.retry_at, n - 30 + 60);
    }

    #[test]
    fn a_start_at_exactly_now_minus_window_is_outside() {
        let n = now();
        assert!(check(&[n - 60, n - 10], n, DAY, CAPS).is_ok());
        assert_eq!(used(&[n - 60, n - 59], n, DAY).minute, 1);
        assert_eq!(used(&[n - 3600, n - 3599], n, DAY).hour, 1);
    }

    #[test]
    fn hour_cap_arithmetic() {
        let n = now();
        let starts = [n - 3000, n - 2000, n - 1500, n - 1000, n - 700];
        let h = check(&starts, n, DAY, CAPS).expect_err("hit");
        assert_eq!(h.cap, Cap::Hour);
        assert_eq!(h.limit, 5);
        assert_eq!(h.retry_at, n - 3000 + 3600);
    }

    #[test]
    fn the_day_window_starts_at_midnight() {
        let n = DAY.start + 40_000;
        // exactly at day.start counts, one second before does not
        assert_eq!(used(&[DAY.start], n, DAY).day, 1);
        assert_eq!(used(&[DAY.start - 1], n, DAY).day, 0);
        let caps = Caps {
            minute: 100,
            hour: 100,
            day: 3,
        };
        let h = check(&[DAY.start, DAY.start + 1, DAY.start + 2], n, DAY, caps).expect_err("hit");
        assert_eq!(h.cap, Cap::Day);
        assert_eq!(h.limit, 3);
        assert_eq!(h.retry_at, DAY.next);
    }

    #[test]
    fn just_after_midnight_yesterdays_calls_count_for_the_hour_only() {
        let n = DAY.start + 600;
        let caps = Caps {
            minute: 100,
            hour: 3,
            day: 3,
        };
        // 23:30 yesterday: inside the hour window, outside today
        let starts = [n - 2400, n - 2300, n - 2200];
        let u = used(&starts, n, DAY);
        assert_eq!((u.hour, u.day), (3, 0));
        let h = check(&starts, n, DAY, caps).expect_err("hour cap");
        assert_eq!(h.cap, Cap::Hour);
        assert_eq!(h.retry_at, n - 2400 + 3600);
    }

    #[test]
    fn a_full_day_before_midnight_does_not_hit_the_day_cap() {
        let n = DAY.start + 7200;
        let caps = Caps {
            minute: 100,
            hour: 100,
            day: 3,
        };
        let starts: Vec<i64> = (1..=10).map(|i| DAY.start - i * 100).collect();
        assert!(check(&starts, n, DAY, caps).is_ok());
    }

    #[test]
    fn the_latest_retry_wins_among_several_hits() {
        let caps = Caps {
            minute: 100,
            hour: 2,
            day: 3,
        };
        // late in the day: the hour cap frees long before 0:00, so the day cap wins
        let n = DAY.next - 1000;
        let starts = [DAY.start + 10, n - 3000, n - 100];
        let h = check(&starts, n, DAY, caps).expect_err("hit");
        assert_eq!(h.cap, Cap::Day);
        assert_eq!(h.retry_at, DAY.next);
        // an hour cap whose oldest call leaves after day.next beats the day cap
        let n = DAY.next - 100;
        let starts = [n - 90, n - 50, n - 20];
        let h = check(&starts, n, DAY, caps).expect_err("hit");
        assert_eq!(h.cap, Cap::Hour);
        assert_eq!(h.retry_at, n - 50 + 3600);
        assert!(h.retry_at > DAY.next);
    }

    #[test]
    fn a_tie_goes_to_the_longer_cap() {
        let n = now();
        let s = n - 10;
        // the hour cap frees at s + 3600, which is also day.next here
        let day = Day {
            start: DAY.start,
            next: s + 3600,
        };
        let caps = Caps {
            minute: 5,
            hour: 1,
            day: 1,
        };
        let h = check(&[s], n, day, caps).expect_err("hit");
        assert_eq!(h.cap, Cap::Day);
        let caps = Caps {
            minute: 1,
            hour: 1,
            day: 100,
        };
        let h = check(&[s], n, day, caps).expect_err("hit");
        assert_eq!(h.cap, Cap::Hour);
    }

    #[test]
    fn retry_is_at_least_now_plus_one() {
        let n = now();
        let day = Day {
            start: DAY.start,
            next: n,
        };
        let caps = Caps {
            minute: 5,
            hour: 5,
            day: 1,
        };
        let h = check(&[n - 5], n, day, caps).expect_err("hit");
        assert_eq!(h.retry_at, n + 1);
    }

    #[test]
    fn running_calls_are_just_starts() {
        let n = now();
        // a call that started 200 s ago and still runs counts like any other start
        let caps = Caps {
            minute: 5,
            hour: 1,
            day: 8,
        };
        let h = check(&[n - 200], n, DAY, caps).expect_err("hit");
        assert_eq!(h.cap, Cap::Hour);
        assert_eq!(h.retry_at, n - 200 + 3600);
    }

    #[test]
    fn cap_keys_and_windows() {
        assert_eq!(Cap::Minute.key(), "minute");
        assert_eq!(Cap::Hour.key(), "hour");
        assert_eq!(Cap::Day.key(), "day");
        assert_eq!(Cap::Minute.window(), Some(60));
        assert_eq!(Cap::Hour.window(), Some(3600));
        assert_eq!(Cap::Day.window(), None);
    }
}
