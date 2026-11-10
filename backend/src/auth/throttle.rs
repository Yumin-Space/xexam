//! In-memory login throttle: escalating locks in rounds, per lower-cased username, plus a global pressure rule.
//!
//! Every sign-in attempt is counted as a failure the moment it starts ([`LoginThrottle::begin`]), before any hashing, so parallel
//! guesses are counted too; a successful sign-in takes it back and starts the name over ([`LoginThrottle::succeeded`]). Refused
//! (429) attempts are never counted, so hammering a locked name does not extend the lock. Nothing is stored on disk; a restart
//! forgets everything (the `reset-password` command runs in another process and cannot lift a lock; an administrator's password
//! reset or the Unlock action through the API does).
//!
//! ## Rounds (the owner's rule, `docs/en/security.md`)
//!
//! | Round | Failures allowed | Then locked for |
//! |---|---|---|
//! | 1 | 5 within 15 minutes (sliding) | 1 minute |
//! | 2 | 3, counted from the end of the previous lock | 5 minutes |
//! | 3 | 3 | 15 minutes |
//! | 4 | 3 | 30 minutes |
//! | 5 | 3 | 1 hour |
//! | 6 and later | 3 | 1 day (the maximum) |
//!
//! The failure that uses up a round's chances starts the lock at once; when the lock ends, the next round begins with a fresh
//! count. During a lock every attempt answers 429 (also with the right password), with the seconds left until the lock ends.
//! Back to round 1: a successful sign-in, an administrator's password reset or Unlock ([`LoginThrottle::forget`]), a restart, or
//! 24 hours of quiet (no lock running and no failure) after the last lock ended or the last failure, whichever is later.
//! A bucket is a small state machine: `locks` (locks so far in this streak; the round in progress is `locks + 1`), the
//! failures of the round in progress, the end of the latest lock and the time of the latest failure.
//!
//! ## Known browsers
//!
//! An attempt from a browser that carries the user's known-browser cookie ([`super::device_known`]) is counted in its own bucket
//! (the name plus a marker no username can contain), with exactly the same rounds. Guesses from elsewhere therefore escalate only
//! the bucket of unknown browsers; the owner keeps signing in from the browsers they used before, while wrong passwords typed in
//! such a browser escalate its own bucket. A success through a known browser starts only that bucket over (a success there must
//! not hand somebody guessing elsewhere five fresh chances); a success without the cookie starts both over (the password is
//! proven, and nothing remains to protect the known bucket from). [`LoginThrottle::forget`] clears both.
//!
//! ## Global pressure
//!
//! When all names together have at least `global` failures in the window, the throttle is under pressure. It then also refuses
//! every name whose own latest failure is less than `pressure_gap` (1 minute) old (one guess per name and minute), but it never
//! refuses a name that has no recent failure of its own. A flood of made-up names therefore slows sign-ins down (the Argon2
//! semaphore bounds the CPU and queues the rest) but cannot lock out an account that is not itself being guessed. Pressure
//! refusals are 429 like a lock but do not advance any round.
//!
//! ## Keys and memory
//!
//! Only a name that passes the username rule ([`super::clean_username`], at most 40 ASCII characters) becomes its own key. Every
//! other name can never match an account and shares one key, so the memory per key is bounded. The number of keys is capped at
//! [`MAX_KEYS`]; buckets that are back at round 1 with no failure in the window are swept out regularly, and when the map is still
//! full the least advanced bucket that is not locked is forgotten first.
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct ThrottleLimits {
    /// Failures allowed in round 1, within `window`
    pub first_round: usize,
    /// Failures allowed in every later round, counted from the end of the previous lock
    pub later_rounds: usize,
    /// The sliding window of round 1 (and of the global count)
    pub window: Duration,
    /// The lock after round 1, 2, …; the last entry also applies to every later round (the maximum)
    pub locks: [Duration; 6],
    /// Quiet time after the last lock and the last failure after which a name starts over at round 1
    pub forgive_after: Duration,
    /// Failures of all names together within `window` that put the throttle under pressure
    pub global: usize,
    /// Under pressure: the time a name waits after its own latest failure
    pub pressure_gap: Duration,
}

const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;

impl Default for ThrottleLimits {
    fn default() -> Self {
        ThrottleLimits {
            first_round: 5,
            later_rounds: 3,
            window: Duration::from_secs(15 * MINUTE),
            locks: [
                Duration::from_secs(MINUTE),
                Duration::from_secs(5 * MINUTE),
                Duration::from_secs(15 * MINUTE),
                Duration::from_secs(30 * MINUTE),
                Duration::from_secs(HOUR),
                Duration::from_secs(24 * HOUR),
            ],
            forgive_after: Duration::from_secs(24 * HOUR),
            global: 100,
            pressure_gap: Duration::from_secs(MINUTE),
        }
    }
}

impl ThrottleLimits {
    /// Failures allowed in the round that follows `locks` locks.
    fn allowed(&self, locks: u32) -> usize {
        if locks == 0 {
            self.first_round
        } else {
            self.later_rounds
        }
    }

    /// The lock that ends the round after `locks` locks (round `locks + 1`).
    pub fn lock_for(&self, locks: u32) -> Duration {
        let last = self.locks.len() - 1;
        let i = usize::try_from(locks).unwrap_or(last).min(last);
        self.locks.get(i).copied().unwrap_or_default()
    }
}

/// Upper bound on the number of usernames tracked at once (each key is at most 40 bytes, so this is about a megabyte or two).
pub const MAX_KEYS: usize = 10_000;

/// The shared key of every name that breaks the username rule (no account can have an empty name).
const INVALID_KEY: &str = "";

/// Appended to the key of attempts from a known browser (a control character, which no username can contain).
const KNOWN_MARK: &str = "\u{1}known";

/// The throttle key of a typed username: the lower-cased name when it passes the username rule, else the shared invalid key;
/// attempts from a known browser of a valid name get their own key.
fn key_for(username: &str, known: bool) -> String {
    match super::clean_username(username) {
        Some(n) if known => format!("{}{KNOWN_MARK}", n.to_ascii_lowercase()),
        Some(n) => n.to_ascii_lowercase(),
        None => INVALID_KEY.to_string(),
    }
}

/// Seconds to put in `Retry-After` / `"retryAfter"`: `wait` rounded up, at least 1.
pub fn retry_after_secs(wait: Duration) -> u64 {
    (wait.as_secs() + u64::from(wait.subsec_nanos() > 0)).max(1)
}

/// The state of one key.
#[derive(Debug, Clone, Default)]
struct Bucket {
    /// Locks so far in this streak: the round in progress is `locks + 1` (while a lock runs, the round that ended in it is `locks`)
    locks: u32,
    /// Failures of the round in progress (round 1: only those within the window)
    failures: VecDeque<Instant>,
    /// End of the latest lock (in the future while it runs)
    lock_end: Option<Instant>,
    /// The latest failure (also kept across a lock, for the pressure rule and the 24-hour reset)
    last_failure: Option<Instant>,
}

impl Bucket {
    /// Brings the bucket up to `now`: 24 hours of quiet start it over; round 1 forgets failures that left the window.
    fn settle(&mut self, now: Instant, l: &ThrottleLimits) {
        if self.locks > 0 {
            let quiet_since = self.lock_end.max(self.last_failure);
            if quiet_since.is_some_and(|t| now.saturating_duration_since(t) >= l.forgive_after)
                && self.locked(now).is_none()
            {
                *self = Bucket::default();
            }
        }
        if self.locks == 0 {
            while self
                .failures
                .front()
                .is_some_and(|t| now.saturating_duration_since(*t) >= l.window)
            {
                self.failures.pop_front();
            }
        }
    }

    /// Time left of the running lock, if any.
    fn locked(&self, now: Instant) -> Option<Duration> {
        self.lock_end
            .filter(|end| *end > now)
            .map(|end| end.saturating_duration_since(now))
    }

    /// Counts one failure; the one that uses up the round's chances starts the lock of that round.
    fn fail(&mut self, now: Instant, l: &ThrottleLimits) {
        self.failures.push_back(now);
        self.last_failure = Some(now);
        if self.failures.len() >= l.allowed(self.locks) {
            self.lock_end = now.checked_add(l.lock_for(self.locks));
            self.locks = self.locks.saturating_add(1);
            self.failures.clear();
        }
    }

    /// Back at round 1 with nothing in the window: the bucket can be dropped.
    fn idle(&self) -> bool {
        self.locks == 0 && self.failures.is_empty()
    }

    fn status(&self, now: Instant, l: &ThrottleLimits) -> Option<LockStatus> {
        if self.locks == 0 {
            return None;
        }
        Some(match self.locked(now) {
            Some(wait) => LockStatus {
                wait: Some(wait),
                round: self.locks,
                tries_left: 0,
            },
            None => LockStatus {
                wait: None,
                round: self.locks.saturating_add(1),
                tries_left: u32::try_from(
                    l.allowed(self.locks).saturating_sub(self.failures.len()),
                )
                .unwrap_or(0),
            },
        })
    }
}

/// The throttle state of a username, for the administrator's user list (`"lock"` of `GET /api/users`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockStatus {
    /// Time left of the running lock; `None` = not locked right now
    pub wait: Option<Duration>,
    /// While locked: the round that ended in this lock (1 = the first lock of one minute). Otherwise the round in progress (≥ 2).
    pub round: u32,
    /// Failures left in the round in progress before the next lock (0 while locked)
    pub tries_left: u32,
}

impl LockStatus {
    /// Orders two statuses by how much they hold the user back: a running lock (the longer first), then the higher round.
    fn weight(&self) -> (bool, Duration, u32, std::cmp::Reverse<u32>) {
        (
            self.wait.is_some(),
            self.wait.unwrap_or_default(),
            self.round,
            std::cmp::Reverse(self.tries_left),
        )
    }
}

#[derive(Default)]
struct State {
    users: HashMap<String, Bucket>,
    /// Every counted failure of the window (all names), at most `global` entries
    global: VecDeque<Instant>,
    last_sweep: Option<Instant>,
}

impl State {
    fn sweep(&mut self, now: Instant, l: &ThrottleLimits) {
        self.users.retain(|_, b| {
            b.settle(now, l);
            !b.idle()
        });
        self.last_sweep = Some(now);
    }

    /// Makes room for one new key: first drop idle buckets, then forget the least advanced bucket that is not locked (a locked
    /// name is evicted only when every tracked name is locked).
    fn make_room(&mut self, now: Instant, l: &ThrottleLimits) {
        if self.users.len() < MAX_KEYS {
            return;
        }
        self.sweep(now, l);
        while self.users.len() >= MAX_KEYS {
            let victim = self
                .users
                .iter()
                .min_by_key(|(_, b)| {
                    (
                        b.locked(now).is_some(),
                        b.locks,
                        b.failures.len(),
                        b.last_failure,
                    )
                })
                .map(|(k, _)| k.clone());
            match victim {
                Some(k) => {
                    self.users.remove(&k);
                }
                None => break,
            }
        }
    }
}

/// Where the throttle reads the time. Production: [`Instant::now`].
pub type ThrottleClock = Arc<dyn Fn() -> Instant + Send + Sync>;

pub struct LoginThrottle {
    limits: ThrottleLimits,
    state: Mutex<State>,
    clock: ThrottleClock,
}

impl Default for LoginThrottle {
    fn default() -> Self {
        Self::new(ThrottleLimits::default())
    }
}

impl LoginThrottle {
    pub fn new(limits: ThrottleLimits) -> Self {
        Self::with_clock(limits, Arc::new(Instant::now))
    }

    /// A throttle that reads the time from `clock`. Test hook: the API tests move this clock forward to check the long locks
    /// (5 minutes … 1 day) and the 24-hour reset without waiting; the server always uses [`LoginThrottle::new`].
    pub fn with_clock(limits: ThrottleLimits, clock: ThrottleClock) -> Self {
        LoginThrottle {
            limits,
            state: Mutex::new(State::default()),
            clock,
        }
    }

    /// The throttle's current time (pass it to [`begin`](Self::begin) and [`status`](Self::status)).
    pub fn now(&self) -> Instant {
        (self.clock)()
    }

    pub fn limits(&self) -> &ThrottleLimits {
        &self.limits
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts an attempt for `username` (as typed); `known`: it comes from a known browser of that user. `Err(wait)` = locked
    /// (or held back by the pressure rule): answer 429 with `wait` without checking the password. `Ok` = the attempt is counted
    /// as a failure until [`succeeded`](Self::succeeded) is called.
    pub fn begin(&self, username: &str, known: bool, now: Instant) -> Result<(), Duration> {
        let key = key_for(username, known);
        let l = self.limits;
        let mut st = self.state();
        if st
            .last_sweep
            .is_none_or(|t| now.saturating_duration_since(t) >= l.window)
        {
            st.sweep(now, &l);
        }
        while st
            .global
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= l.window)
        {
            st.global.pop_front();
        }
        let pressure = st.global.len() >= l.global;
        if let Some(b) = st.users.get_mut(&key) {
            b.settle(now, &l);
            if let Some(wait) = b.locked(now) {
                return Err(wait);
            }
            if pressure {
                if let Some(t) = b.last_failure {
                    let since = now.saturating_duration_since(t);
                    if since < l.pressure_gap {
                        return Err(l.pressure_gap.saturating_sub(since));
                    }
                }
            }
        }
        if !st.users.contains_key(&key) {
            st.make_room(now, &l);
        }
        st.users.entry(key).or_default().fail(now, &l);
        st.global.push_back(now);
        while st.global.len() > l.global {
            st.global.pop_front();
        }
        Ok(())
    }

    /// The attempt started with [`begin`](Self::begin) succeeded: the bucket it came through starts over at round 1 (a success
    /// without the known-browser cookie starts the known-browser bucket over too), and the attempt leaves the global count.
    pub fn succeeded(&self, username: &str, known: bool) {
        let mut st = self.state();
        st.users.remove(&key_for(username, known));
        if !known {
            st.users.remove(&key_for(username, true));
        }
        st.global.pop_back();
    }

    /// Forgets everything about `username`, from any browser (an administrator set a new password for it, or unlocked it).
    pub fn forget(&self, username: &str) {
        let mut st = self.state();
        st.users.remove(&key_for(username, false));
        st.users.remove(&key_for(username, true));
    }

    /// The state of `username` for the user list: `None` when it is at round 1 and not locked in every bucket; otherwise the
    /// bucket that holds the user back most (a running lock first, the longer one; then the higher round).
    pub fn status(&self, username: &str, now: Instant) -> Option<LockStatus> {
        let l = self.limits;
        let mut st = self.state();
        [key_for(username, false), key_for(username, true)]
            .iter()
            .filter_map(|k| {
                let b = st.users.get_mut(k)?;
                b.settle(now, &l);
                b.status(now, &l)
            })
            .max_by_key(LockStatus::weight)
    }

    /// `(number of tracked names, length in bytes of the longest one)`: a diagnostic for the memory bound.
    pub fn key_stats(&self) -> (usize, usize) {
        let st = self.state();
        let longest = st.users.keys().map(String::len).max().unwrap_or(0);
        (st.users.len(), longest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);
    const SEC: Duration = Duration::from_secs(1);

    fn limits(global: usize) -> ThrottleLimits {
        ThrottleLimits {
            global,
            ..ThrottleLimits::default()
        }
    }

    /// Fails `n` times at `at` and asserts each one was let through.
    fn fail(t: &LoginThrottle, name: &str, known: bool, n: usize, at: Instant) {
        for i in 0..n {
            assert!(t.begin(name, known, at).is_ok(), "{name} attempt {i}");
        }
    }

    #[test]
    fn lock_durations_escalate_and_stop_at_one_day() {
        let l = ThrottleLimits::default();
        let want = [1, 5, 15, 30, 60, 1440, 1440, 1440];
        for (locks, minutes) in want.iter().enumerate() {
            assert_eq!(
                l.lock_for(locks as u32),
                MIN * *minutes,
                "lock after round {}",
                locks + 1
            );
        }
        assert_eq!(l.lock_for(u32::MAX), MIN * 1440);
        assert_eq!(retry_after_secs(Duration::from_millis(1)), 1);
        assert_eq!(retry_after_secs(Duration::from_millis(60_001)), 61);
        assert_eq!(retry_after_secs(MIN), 60);
        assert_eq!(retry_after_secs(Duration::ZERO), 1);
    }

    #[test]
    fn each_round_allows_its_failures_then_locks_for_its_time() {
        let t = LoginThrottle::default();
        let mut now = Instant::now();
        // round 1: five failures, then one minute
        fail(&t, "Ann", false, 5, now);
        assert_eq!(t.begin("ann", false, now), Err(MIN), "case-insensitive");
        assert_eq!(
            t.status("ANN", now),
            Some(LockStatus {
                wait: Some(MIN),
                round: 1,
                tries_left: 0
            })
        );
        assert!(
            t.begin("bob", false, now).is_ok(),
            "other names are not affected"
        );
        // rounds 2..: three failures each, counted from the end of the previous lock
        let expected = [5, 15, 30, 60, 1440, 1440, 1440];
        for (i, minutes) in expected.iter().enumerate() {
            let round = i as u32 + 2;
            now += t.limits().lock_for(round - 2);
            assert_eq!(
                t.status("ann", now),
                Some(LockStatus {
                    wait: None,
                    round,
                    tries_left: 3
                }),
                "round {round} begins when the lock ends"
            );
            fail(&t, "ann", false, 2, now);
            assert_eq!(t.status("ann", now).map(|s| s.tries_left), Some(1));
            now += SEC;
            assert!(
                t.begin(" ANN ", false, now).is_ok(),
                "third failure of round {round}"
            );
            let lock = MIN * *minutes;
            assert_eq!(t.begin("ann", false, now), Err(lock), "round {round}");
            // the lock does not grow while it is hammered, and even the right password waits (the caller never gets to check it)
            assert_eq!(t.begin("ann", false, now + lock - SEC), Err(SEC));
            assert_eq!(
                t.status("ann", now).map(|s| (s.round, s.wait)),
                Some((round, Some(lock)))
            );
        }
    }

    #[test]
    fn round_one_counts_failures_within_fifteen_minutes() {
        let t = LoginThrottle::default();
        let t0 = Instant::now();
        // four failures, the oldest ages out of the window: a fifth and sixth are still allowed
        fail(&t, "ann", false, 1, t0);
        fail(&t, "ann", false, 3, t0 + 10 * MIN);
        let later = t0 + 15 * MIN;
        fail(&t, "ann", false, 1, later);
        assert!(t.status("ann", later).is_none(), "still round 1");
        fail(&t, "ann", false, 1, later);
        // now five within the window: locked for one minute from the last failure
        assert_eq!(t.begin("ann", false, later + SEC), Err(MIN - SEC));
        // later rounds have no window: three failures far apart still lock
        let r2 = later + MIN;
        fail(&t, "ann", false, 1, r2);
        fail(&t, "ann", false, 1, r2 + 5 * 60 * MIN);
        fail(&t, "ann", false, 1, r2 + 10 * 60 * MIN);
        assert_eq!(t.begin("ann", false, r2 + 10 * 60 * MIN), Err(5 * MIN));
    }

    #[test]
    fn resets_to_round_one() {
        let day = 24 * 60 * MIN;
        // a success, through an unknown browser
        let t = LoginThrottle::default();
        let now = Instant::now();
        fail(&t, "ann", false, 5, now);
        let now = now + MIN;
        fail(&t, "ann", false, 1, now);
        t.succeeded("ann", false);
        assert!(t.status("ann", now).is_none());
        fail(&t, "ann", false, 5, now);
        assert!(t.begin("ann", false, now).is_err(), "five chances again");
        // an administrator's reset / unlock, even during a lock
        t.forget("ANN");
        assert!(t.status("ann", now).is_none());
        fail(&t, "ann", false, 5, now);
        assert!(t.begin("ann", false, now).is_err());
        // 24 hours after the last lock ended, with no new failure
        let t = LoginThrottle::default();
        let t0 = Instant::now();
        fail(&t, "ann", false, 5, t0);
        let end = t0 + MIN;
        assert_eq!(t.status("ann", end + day - SEC).map(|s| s.round), Some(2));
        assert!(t.status("ann", end + day).is_none(), "forgiven");
        fail(&t, "ann", false, 4, end + day);
        assert!(t.status("ann", end + day).is_none(), "five chances again");
        // a failure after the lock pushes the 24 hours back; a running lock is never forgiven
        let t = LoginThrottle::default();
        fail(&t, "ann", false, 5, t0);
        fail(&t, "ann", false, 1, end + 12 * 60 * MIN);
        assert_eq!(t.status("ann", end + day).map(|s| s.tries_left), Some(2));
        let quiet = end + 12 * 60 * MIN + day;
        assert!(t.status("ann", quiet).is_none());
        let t = LoginThrottle::default();
        let mut now = t0;
        for r in 0..6u32 {
            fail(&t, "ann", false, if r == 0 { 5 } else { 3 }, now);
            now += t.limits().lock_for(r);
        }
        // the sixth lock (one day) is running until `now`
        assert_eq!(t.status("ann", now - SEC).map(|s| s.round), Some(6));
        assert_eq!(t.status("ann", now).map(|s| s.round), Some(7));
        assert!(t.status("ann", now + day).is_none());
    }

    #[test]
    fn global_pressure_slows_guessed_names_but_never_locks_out_a_fresh_one() {
        let t = LoginThrottle::new(limits(10));
        let t0 = Instant::now();
        for i in 0..10 {
            assert!(t.begin(&format!("user{i}"), false, t0).is_ok());
        }
        // under pressure: a name with a failure in the last minute waits, a name without one gets through
        assert_eq!(t.begin("user0", false, t0 + SEC), Err(MIN - SEC));
        assert!(t.begin("admin", false, t0 + SEC).is_ok());
        assert!(t.begin("admin", false, t0 + 2 * SEC).is_err());
        // made-up names that break the username rule share one key, so they are slowed down together
        assert!(t.begin("not a name", false, t0 + 3 * SEC).is_ok());
        assert!(t.begin("also not a name!", false, t0 + 4 * SEC).is_err());
        // a flood can keep the pressure on for ever, yet every name still gets one try per minute
        for i in 0..20 {
            assert!(t.begin(&format!("flood{i}"), false, t0 + 10 * SEC).is_ok());
        }
        assert!(t.begin("admin", false, t0 + 62 * SEC).is_ok());
        assert!(t.begin("user0", false, t0 + 62 * SEC).is_ok());
        // pressure refusals do not advance the round
        assert!(t.status("user0", t0 + 62 * SEC).is_none());
    }

    #[test]
    fn a_known_browser_has_its_own_rounds() {
        let t = LoginThrottle::default();
        let t0 = Instant::now();
        fail(&t, "admin", false, 5, t0);
        assert!(t.begin("admin", false, t0).is_err());
        // the owner's browser has its own count and signs in
        assert!(t.begin("Admin", true, t0).is_ok());
        t.succeeded("admin", true);
        // a success there does not start the guessed bucket over
        assert_eq!(t.begin("admin", false, t0), Err(MIN));
        // guesses from elsewhere escalate through the rounds, the known browser is never locked by them
        let mut now = t0;
        for r in 0..8u32 {
            if r > 0 {
                fail(&t, "admin", false, 3, now);
            }
            assert!(t.begin("admin", false, now).is_err(), "round {}", r + 1);
            assert!(t.begin("admin", true, now).is_ok());
            t.succeeded("admin", true);
            now += t.limits().lock_for(r);
        }
        assert_eq!(t.status("admin", now).map(|s| s.round), Some(9));
        // the known browser follows the same rounds when it guesses wrong itself
        fail(&t, "admin", true, 5, now);
        assert_eq!(t.begin("admin", true, now), Err(MIN));
        fail(&t, "admin", true, 3, now + MIN);
        assert_eq!(t.begin("admin", true, now + MIN), Err(5 * MIN));
        // the status shows the bucket that holds the user back most: here the unknown browsers' (higher round, no lock)
        // versus the known browser's 5-minute lock: a running lock wins
        assert_eq!(
            t.status("admin", now + MIN).map(|s| (s.round, s.wait)),
            Some((2, Some(5 * MIN)))
        );
        // a success without the cookie starts both buckets over
        t.succeeded("admin", false);
        assert!(t.status("admin", now + MIN).is_none());
        // Unlock clears both
        fail(&t, "admin", false, 5, now);
        fail(&t, "admin", true, 5, now);
        t.forget("ADMIN");
        assert!(t.begin("admin", true, now).is_ok());
        assert!(t.begin("admin", false, now).is_ok());
        // an invalid name never gets a key of its own, known or not
        assert!(t.begin("not a name", true, now).is_ok());
        assert_eq!(t.key_stats().1, "admin".len() + KNOWN_MARK.len());
    }

    #[test]
    fn keys_are_bounded_in_length_and_number() {
        let t = LoginThrottle::new(limits(usize::MAX));
        let t0 = Instant::now();
        for i in 0..20 {
            let huge = format!("{i}{}", "x".repeat(1_000_000));
            assert!(t.begin(&huge, false, t0).is_ok() || i >= 5);
        }
        assert_eq!(t.key_stats(), (1, 0), "every invalid name shares one key");
        let long = "y".repeat(super::super::USERNAME_MAX_CHARS);
        assert!(t.begin(&long, false, t0).is_ok());
        assert_eq!(t.key_stats(), (2, super::super::USERNAME_MAX_CHARS));
        // the map never grows past MAX_KEYS; the least advanced names are forgotten first, never a locked one
        fail(&t, "target", false, 5, t0);
        for i in 0..MAX_KEYS + 50 {
            assert!(t.begin(&format!("n{i}"), false, t0).is_ok());
        }
        assert!(t.key_stats().0 <= MAX_KEYS);
        assert!(
            t.begin("target", false, t0 + SEC).is_err(),
            "a locked name is not evicted by a flood of new ones"
        );
        // idle buckets are swept out once a window has passed; names past round 1 are kept for the 24-hour rule (here
        // "target" and the shared key of invalid names, both locked once)
        assert!(t.begin("fresh", false, t0 + 15 * MIN + SEC).is_ok());
        assert_eq!(t.key_stats().0, 3);
    }
}
