//! Provider pacing for AI generation and import (v1.9, item 1h).
//!
//! One global setting (`paceOn`, `paceRpm`, `paceTpm`) describes the limits of the AI provider. The service uses them
//! exactly as entered, spreads the calls evenly over a sliding 60-second window, estimates the tokens of a call before it goes out
//! and shrinks the parallel width to what fits. [`Window`] is pure (the clock is passed in); [`Pacer`] is the shared
//! state of the service; [`WaitMark`] shows "waiting for quota" on the job while a wait lasts.

use crate::app::AppState;
use crate::gen::par::ANSWER_RESERVE_MAX;
use crate::limits::{estimate_tokens, Limits};
use crate::store::jobs;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;
use tokio::time::Instant;

/// The sliding window of the provider's per-minute limits.
pub const WINDOW_SECS: u64 = 60;
/// A wait of at least this many seconds is shown on the job ("waiting for quota").
pub const SHOW_WAIT_SECS: f64 = 8.0;
/// How often one call is retried after a 429 (not counted as an error).
pub const MAX_LIMITED_RETRIES: u32 = 10;
/// The longest single wait after a 429, in seconds.
pub const MAX_RETRY_WAIT_SECS: u64 = 300;
/// A wait the provider names above this many seconds is taken as a daily quota.
pub const DAILY_WAIT_SECS: u64 = 900;

/// The pacing settings in force (used exactly as entered).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaceCfg {
    pub on: bool,
    /// Requests per window
    pub rpm: i64,
    /// Tokens per window
    pub tpm: i64,
    pub window: Duration,
}

impl PaceCfg {
    /// From the limits; `scale_pct` shrinks the window (tests: 10 = a 6 s window, 0 = pacing off).
    pub fn of(l: &Limits, scale_pct: u64) -> PaceCfg {
        PaceCfg {
            on: l.pace_on != 0 && scale_pct > 0,
            rpm: l.pace_rpm,
            tpm: l.pace_tpm,
            window: Duration::from_secs_f64(WINDOW_SECS as f64 * scale_pct as f64 / 100.0),
        }
    }

    /// The even spacing between two call starts.
    pub fn interval(&self) -> Duration {
        self.window.div_f64(self.rpm.max(1) as f64)
    }
}

/// The tokens a call is expected to use: its input plus the answer it asks for (clamped, so that an owner with no
/// answer limit is still admitted).
pub fn estimate(system: &str, prefix: Option<&str>, user: &str, want: i64) -> i64 {
    estimate_tokens(system)
        + prefix.map_or(0, estimate_tokens)
        + estimate_tokens(user)
        + want.clamp(1, ANSWER_RESERVE_MAX)
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    id: u64,
    at: Instant,
    tokens: i64,
}

/// What a successful [`Window::admit`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admit {
    /// The handle for [`Window::stamp`] and [`Window::settle`]
    pub id: u64,
    /// The estimate alone is above the token limit: the call goes out alone (the caller logs one warning)
    pub oversize: bool,
}

/// The calls of the last window: start time and tokens of each.
#[derive(Debug, Default)]
pub struct Window {
    entries: VecDeque<Entry>,
    last_start: Option<Instant>,
    paused_until: Option<Instant>,
    next_id: u64,
}

impl Window {
    /// Tries to start a call of `est` tokens at `now`. `Err(t)` = not before `t`.
    pub fn admit(&mut self, cfg: &PaceCfg, est: i64, now: Instant) -> Result<Admit, Instant> {
        // 1. drop the calls that left the window
        while self
            .entries
            .front()
            .is_some_and(|e| e.at + cfg.window <= now)
        {
            self.entries.pop_front();
        }
        // 2. a provider asked us to wait
        if let Some(p) = self.paused_until {
            if p > now {
                return Err(p);
            }
            self.paused_until = None;
        }
        // 3. even spacing
        if let Some(last) = self.last_start {
            let next = last + cfg.interval();
            if next > now {
                return Err(next);
            }
        }
        // 4. the request limit
        let rpm = usize::try_from(cfg.rpm.max(1)).unwrap_or(usize::MAX);
        if self.entries.len() >= rpm {
            if let Some(first) = self.entries.front() {
                return Err(first.at + cfg.window);
            }
        }
        // 5. the token limit: walk from the oldest call until enough tokens are free
        let used: i64 = self.entries.iter().map(|e| e.tokens).sum();
        let mut oversize = false;
        if used.saturating_add(est) > cfg.tpm {
            if self.entries.is_empty() {
                // 6. nothing to wait for: an oversize call goes out alone
                oversize = true;
            } else {
                let mut left = used;
                for e in &self.entries {
                    left -= e.tokens;
                    if left.saturating_add(est) <= cfg.tpm {
                        return Err(e.at + cfg.window);
                    }
                }
                // even an empty window is too small: wait for it to empty, then go alone
                let newest = self.entries.back().map_or(now, |e| e.at);
                return Err(newest + cfg.window);
            }
        }
        // 7. admitted
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push_back(Entry {
            id,
            at: now,
            tokens: est,
        });
        self.last_start = Some(now);
        Ok(Admit { id, oversize })
    }

    /// The call really starts at `now` (it waited for a place of the gate after it was admitted).
    pub fn stamp(&mut self, id: u64, now: Instant) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.at = now;
        }
        self.last_start = Some(self.last_start.map_or(now, |l| l.max(now)));
        // keep the list ordered by start time
        self.entries.make_contiguous().sort_by_key(|e| e.at);
    }

    /// The call is over: its real tokens replace the estimate (`None` = the call produced none; the request itself
    /// still counts against the request limit until it leaves the window).
    pub fn settle(&mut self, id: u64, tokens: Option<i64>) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.tokens = tokens.unwrap_or(0).max(0);
        }
    }

    /// No call may start before `until` (the provider answered 429 and named a wait).
    pub fn pause_until(&mut self, until: Instant) {
        self.paused_until = Some(self.paused_until.map_or(until, |p| p.max(until)));
    }

    /// How many calls may be in flight together for calls of about `est` tokens.
    pub fn width_for(cfg: &PaceCfg, est: i64) -> usize {
        if !cfg.on {
            return usize::MAX;
        }
        let rpm = usize::try_from(cfg.rpm.max(1)).unwrap_or(usize::MAX);
        let by_tokens = usize::try_from((cfg.tpm / est.max(1)).max(1)).unwrap_or(usize::MAX);
        rpm.min(by_tokens)
    }
}

/// The shared pacer of the service.
#[derive(Default)]
pub struct Pacer {
    pub(crate) win: Mutex<Window>,
    /// Calls queue here in order; the one at the front waits for its turn in the window
    pub(crate) turn: tokio::sync::Mutex<()>,
    /// Jobs that currently show "waiting": task id -> waiting calls
    waiting: Mutex<HashMap<i64, usize>>,
}

impl Pacer {
    /// Runs `f` on the window.
    pub fn with_window<R>(&self, f: impl FnOnce(&mut Window) -> R) -> R {
        let mut w = self.win.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut w)
    }

    /// Marks `task` as waiting for quota until the returned guard is dropped.
    pub fn mark(&self, app: &AppState, task: i64) -> WaitMark {
        let first = {
            let mut m = self.waiting.lock().unwrap_or_else(|e| e.into_inner());
            let n = m.entry(task).or_insert(0);
            *n += 1;
            *n == 1
        };
        if first {
            set_flag(app, task, true);
        }
        WaitMark {
            app: app.clone(),
            task,
        }
    }
}

/// Shows "waiting for quota" on a job while it lives (0 -> 1 and 1 -> 0 writes the flag).
pub struct WaitMark {
    app: AppState,
    task: i64,
}

impl Drop for WaitMark {
    fn drop(&mut self) {
        let last = {
            let mut m = self
                .app
                .0
                .pacer
                .waiting
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            match m.get_mut(&self.task) {
                Some(n) if *n > 1 => {
                    *n -= 1;
                    false
                }
                Some(_) => {
                    m.remove(&self.task);
                    true
                }
                None => false,
            }
        };
        if last {
            set_flag(&self.app, self.task, false);
        }
    }
}

fn set_flag(app: &AppState, task: i64, on: bool) {
    if let Err(e) = app.0.store.write(|tx| jobs::set_rate_wait(tx, task, on)) {
        tracing::warn!("could not update the waiting flag of an AI job: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(rpm: i64, tpm: i64) -> PaceCfg {
        PaceCfg {
            on: true,
            rpm,
            tpm,
            window: Duration::from_secs(60),
        }
    }

    #[test]
    fn the_calls_are_spread_evenly() {
        let c = cfg(13, 1_000_000);
        let t0 = Instant::now();
        let mut w = Window::default();
        assert!(w.admit(&c, 100, t0).is_ok());
        let next = w.admit(&c, 100, t0).expect_err("too soon");
        let gap = next - t0;
        let want = Duration::from_secs_f64(60.0 / 13.0);
        assert!(gap >= want - Duration::from_millis(1) && gap <= want + Duration::from_millis(1));
        assert!(w.admit(&c, 100, next).is_ok());
    }

    #[test]
    fn a_full_request_window_waits_for_the_oldest_call() {
        let c = cfg(2, 1_000_000);
        let t0 = Instant::now();
        let mut w = Window::default();
        assert!(w.admit(&c, 10, t0).is_ok());
        assert!(w.admit(&c, 10, t0 + Duration::from_secs(30)).is_ok());
        let e = w
            .admit(&c, 10, t0 + Duration::from_secs(59))
            .expect_err("full");
        assert_eq!(e, t0 + Duration::from_secs(60));
        assert!(w.admit(&c, 10, e).is_ok());
    }

    #[test]
    fn the_token_limit_waits_until_enough_tokens_are_free() {
        let c = cfg(100, 225_000);
        let t0 = Instant::now();
        let mut w = Window::default();
        for i in 0..2 {
            assert!(w
                .admit(&c, 78_000, t0 + Duration::from_secs(i * 10))
                .is_ok());
        }
        let e = w
            .admit(&c, 78_000, t0 + Duration::from_secs(25))
            .expect_err("tokens");
        assert_eq!(e, t0 + Duration::from_secs(60));
    }

    #[test]
    fn an_oversize_call_goes_out_alone() {
        let c = cfg(100, 225_000);
        let t0 = Instant::now();
        let mut w = Window::default();
        let a = w.admit(&c, 300_000, t0).expect("alone");
        assert!(a.oversize);
        // nothing else while it is in the window
        let e = w
            .admit(&c, 10, t0 + Duration::from_secs(10))
            .expect_err("full of tokens");
        assert_eq!(e, t0 + Duration::from_secs(60));
        // a second oversize call waits for the window to empty
        let e = w
            .admit(&c, 300_000, t0 + Duration::from_secs(10))
            .expect_err("wait");
        assert_eq!(e, t0 + Duration::from_secs(60));
    }

    #[test]
    fn settle_frees_tokens() {
        let c = cfg(100, 225_000);
        let t0 = Instant::now();
        let mut w = Window::default();
        let a = w.admit(&c, 200_000, t0).expect("first");
        let t1 = t0 + Duration::from_secs(5);
        assert!(w.admit(&c, 100_000, t1).is_err());
        w.settle(a.id, Some(20_000));
        assert!(w.admit(&c, 100_000, t1).is_ok());
        let b = w.admit(&c, 10, t1 + Duration::from_secs(5)).expect("b");
        w.settle(b.id, None);
        assert!(w.admit(&c, 100_000, t1 + Duration::from_secs(10)).is_ok());
    }

    #[test]
    fn stamp_moves_the_start() {
        let c = cfg(1, 1_000_000);
        let t0 = Instant::now();
        let mut w = Window::default();
        let a = w.admit(&c, 10, t0).expect("first");
        w.stamp(a.id, t0 + Duration::from_secs(20));
        let e = w
            .admit(&c, 10, t0 + Duration::from_secs(61))
            .expect_err("still in the window");
        assert_eq!(e, t0 + Duration::from_secs(80));
    }

    #[test]
    fn a_pause_is_honoured() {
        let c = cfg(100, 1_000_000);
        let t0 = Instant::now();
        let mut w = Window::default();
        w.pause_until(t0 + Duration::from_secs(30));
        w.pause_until(t0 + Duration::from_secs(10));
        assert_eq!(
            w.admit(&c, 10, t0).expect_err("paused"),
            t0 + Duration::from_secs(30)
        );
        assert!(w.admit(&c, 10, t0 + Duration::from_secs(30)).is_ok());
    }

    #[test]
    fn the_width_follows_the_limits() {
        assert_eq!(Window::width_for(&cfg(13, 225_000), 80_000), 2);
        assert_eq!(Window::width_for(&cfg(13, 225_000), 1_000), 13);
        assert_eq!(Window::width_for(&cfg(13, 225_000), 900_000), 1);
        let off = PaceCfg {
            on: false,
            ..cfg(13, 225_000)
        };
        assert_eq!(Window::width_for(&off, 80_000), usize::MAX);
    }

    #[test]
    fn the_estimate_clamps_the_answer() {
        let e = estimate("", None, "", 250_000);
        assert_eq!(e, 16_000);
        assert_eq!(estimate("abcd", Some("abcdabcd"), "abcd", 0), 4 + 1);
    }

    #[test]
    fn the_config_follows_the_limits_and_the_scale() {
        let l = Limits::default();
        let c = PaceCfg::of(&l, 100);
        assert_eq!((c.on, c.rpm, c.tpm), (true, 13, 230_000));
        assert_eq!(c.window, Duration::from_secs(60));
        assert_eq!(PaceCfg::of(&l, 10).window, Duration::from_secs(6));
        assert!(!PaceCfg::of(&l, 0).on);
    }
}
