//! In-memory login throttle: escalating locks in rounds, per account (or per typed name that matches no account), plus a global
//! pressure rule.
//!
//! Every sign-in attempt is counted as a failure the moment it starts ([`LoginThrottle::begin`]), before any hashing, so parallel
//! guesses are counted too; a successful sign-in takes it back and starts the account over ([`LoginThrottle::succeeded`]).
//! Refused (429) attempts are never counted, so hammering a locked account does not extend the lock. Nothing is stored on disk;
//! a restart forgets everything (the `reset-password` command runs in another process and cannot lift a lock; an
//! administrator's password reset or the Unlock action through the API does).
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
//! ## Accounts and other names
//!
//! The caller looks the typed name up first (a cheap query, no hashing) and passes a [`Target`]:
//! - [`Target::Account`]: an existing account, enabled or not. Its buckets are keyed by the **user id**, so the state belongs to
//!   the account and not to the name: a rename keeps a running lock (and Unlock still finds it), and a new account that later
//!   takes a deleted or renamed account's old name starts fresh. These buckets live in their own map, which is bounded by the
//!   number of accounts (the API forgets an account's buckets when the account is deleted) and is **never** evicted to make room:
//!   a flood of made-up names cannot push a guessed account back to round 1.
//! - [`Target::Name`]: a name that matches no account. It still gets its own bucket with exactly the same rounds, so the answers
//!   never tell whether a username exists. Only a name that passes the username rule ([`super::clean_username`], at most 40
//!   ASCII characters) becomes its own key; every other name shares one key. This map is capped at [`MAX_KEYS`]: buckets back at
//!   round 1 with nothing in the window are swept out regularly, and when it is still full the least advanced bucket that is not
//!   locked is forgotten first (a locked one only when every tracked name is locked). Forgetting such a bucket protects nothing,
//!   because no account is behind it. The one thing an attacker learns under such a flood (about ten thousand requests per
//!   probe) is whether a name's state can be evicted, that is whether the name has an account; `docs/en/security.md` says so.
//!
//! ## Known browsers
//!
//! An attempt from a browser that carries the account's known-browser cookie ([`super::device_known`]) is counted in its own
//! bucket of that account, with exactly the same rounds. Guesses from elsewhere therefore escalate only the bucket of unknown
//! browsers; the owner keeps signing in from the browsers they used before, while wrong passwords typed in such a browser
//! escalate its own bucket. A success through a known browser starts only that bucket over (a success there must not hand
//! somebody guessing elsewhere five fresh chances); a success without the cookie starts both over (the password is proven, and
//! nothing remains to protect the known bucket from). [`LoginThrottle::forget`] clears both. A name without an account has no
//! known browsers.
//!
//! ## Global pressure
//!
//! When all buckets together have at least `global` failures in the window, the throttle is under pressure. It then also refuses
//! every bucket whose own latest failure is less than `pressure_gap` (1 minute) old (one guess per bucket and minute), but it
//! never refuses one that has no recent failure of its own. A flood of made-up names therefore slows sign-ins down (the Argon2
//! semaphore bounds the CPU and queues the rest) but cannot lock out an account that is not itself being guessed. Pressure
//! refusals are 429 like a lock but do not advance any round.
//!
//! ## Logging
//!
//! A refused attempt costs the server almost nothing, so it must not cost a log line either: the API logs when a lock starts
//! (see [`Begun::lock`]) and, at most once per [`REFUSAL_REPORT_EVERY`], how many attempts were refused since the last report.
//! The first refusal after a quiet minute is reported at once ([`Refused::report`]); the refusals that follow it within the
//! minute are counted and reported by the server's once-a-minute timer ([`LoginThrottle::flush_refusals`]), so the end of a
//! burst is logged within about a minute even when no further attempt arrives.
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

/// Upper bound on the number of names without an account tracked at once (each key is at most 40 bytes, so this is about a
/// megabyte or two). The buckets of real accounts are not counted here and never evicted.
pub const MAX_KEYS: usize = 10_000;

/// Refused attempts are summed up in the log at most this often.
pub const REFUSAL_REPORT_EVERY: Duration = Duration::from_secs(MINUTE);

/// The shared key of every name that breaks the username rule (no account can have an empty name).
const INVALID_KEY: &str = "";

/// Who a password check is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target<'a> {
    /// An existing account (enabled or not), by user id
    Account(i64),
    /// A typed name that matches no account (as typed; it is trimmed and lower-cased here)
    Name(&'a str),
}

/// The bucket of one attempt.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    /// An account's bucket: `known` = attempts from a browser where the account signed in before
    Account { id: i64, known: bool },
    /// A name without an account: the lower-cased name when it passes the username rule, else [`INVALID_KEY`]
    Name(String),
}

impl Key {
    fn of(target: Target<'_>, known: bool) -> Key {
        match target {
            Target::Account(id) => Key::Account { id, known },
            Target::Name(typed) => Key::Name(match super::clean_username(typed) {
                Some(n) => n.to_ascii_lowercase(),
                None => INVALID_KEY.to_string(),
            }),
        }
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

    /// Counts one failure; the one that uses up the round's chances starts the lock of that round, which is returned.
    fn fail(&mut self, now: Instant, l: &ThrottleLimits) -> Option<StartedLock> {
        self.failures.push_back(now);
        self.last_failure = Some(now);
        if self.failures.len() < l.allowed(self.locks) {
            return None;
        }
        let length = l.lock_for(self.locks);
        self.lock_end = now.checked_add(length);
        self.locks = self.locks.saturating_add(1);
        self.failures.clear();
        Some(StartedLock {
            round: self.locks,
            length,
        })
    }

    /// Back at round 1 with nothing in the window: the bucket can be dropped.
    fn idle(&self) -> bool {
        self.locks == 0 && self.failures.is_empty()
    }

    fn status(&self, now: Instant, l: &ThrottleLimits, known_browser: bool) -> Option<LockStatus> {
        if self.locks == 0 {
            return None;
        }
        Some(match self.locked(now) {
            Some(wait) => LockStatus {
                wait: Some(wait),
                round: self.locks,
                tries_left: 0,
                known_browser,
                other_wait: None,
            },
            None => LockStatus {
                wait: None,
                round: self.locks.saturating_add(1),
                tries_left: u32::try_from(
                    l.allowed(self.locks).saturating_sub(self.failures.len()),
                )
                .unwrap_or(0),
                known_browser,
                other_wait: None,
            },
        })
    }

    /// The order in which full maps forget buckets: not locked before locked, then the least advanced first.
    fn eviction_rank(&self, now: Instant) -> (bool, u32, usize, Option<Instant>) {
        (
            self.locked(now).is_some(),
            self.locks,
            self.failures.len(),
            self.last_failure,
        )
    }
}

/// A lock that an attempt started (when that attempt then fails).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartedLock {
    /// The lock's number in this streak (1 = the first lock, of one minute)
    pub round: u32,
    pub length: Duration,
}

/// An attempt that may go on to check the password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Begun {
    /// The lock this attempt started by using up the round's last chance: it holds if the password turns out wrong (a success
    /// takes it back). The API logs it only then, so a lock is logged once, when it starts.
    pub lock: Option<StartedLock>,
}

/// An attempt refused without checking the password (a running lock, or the pressure rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused {
    /// How long the caller should wait (the `Retry-After`)
    pub wait: Duration,
    /// `Some(n)`: time to log one line that `n` attempts were refused since the last such line (at most one line per
    /// [`REFUSAL_REPORT_EVERY`]); `None`: counted, nothing to log now.
    pub report: Option<u64>,
}

/// The throttle state of an account, for the administrator's user list (`"lock"` of `GET /api/users`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockStatus {
    /// Time left of the running lock; `None` = not locked right now
    pub wait: Option<Duration>,
    /// While locked: the round that ended in this lock (1 = the first lock of one minute). Otherwise the round in progress (≥ 2).
    pub round: u32,
    /// Failures left in the round in progress before the next lock (0 while locked)
    pub tries_left: u32,
    /// The state belongs to the browsers where the account signed in before (else to every other browser)
    pub known_browser: bool,
    /// Time left of a lock that runs at the same time in the account's **other** bucket (the browsers this status is not
    /// about); `None` = the other browsers are not locked. Only set when this status is a running lock too (a running lock
    /// always outweighs a bucket without one), so `Some` means the account is locked in every browser.
    pub other_wait: Option<Duration>,
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
    /// Buckets of real accounts, by (user id, known browser): bounded by the number of accounts, never evicted
    accounts: HashMap<(i64, bool), Bucket>,
    /// Buckets of names without an account: at most [`MAX_KEYS`]
    names: HashMap<String, Bucket>,
    /// Every counted failure of the window (all buckets), at most `global` entries
    global: VecDeque<Instant>,
    last_sweep: Option<Instant>,
    /// Refused attempts since the last report line, and when that line was written
    refused: u64,
    last_report: Option<Instant>,
}

impl State {
    fn sweep(&mut self, now: Instant, l: &ThrottleLimits) {
        let keep = |b: &mut Bucket| {
            b.settle(now, l);
            !b.idle()
        };
        self.accounts.retain(|_, b| keep(b));
        self.names.retain(|_, b| keep(b));
        self.last_sweep = Some(now);
    }

    fn bucket(&mut self, key: &Key) -> Option<&mut Bucket> {
        match key {
            Key::Account { id, known } => self.accounts.get_mut(&(*id, *known)),
            Key::Name(n) => self.names.get_mut(n),
        }
    }

    /// The bucket of `key`, created when missing (a new name may first make room in the map of names).
    fn bucket_or_new(&mut self, key: Key, now: Instant, l: &ThrottleLimits) -> &mut Bucket {
        match key {
            Key::Account { id, known } => self.accounts.entry((id, known)).or_default(),
            Key::Name(n) => {
                if !self.names.contains_key(&n) {
                    self.make_room(now, l);
                }
                self.names.entry(n).or_default()
            }
        }
    }

    /// Makes room for one new name without an account: first drop idle buckets, then forget the least advanced name that is
    /// not locked (a locked one only when every tracked name is locked). Accounts are never touched here.
    fn make_room(&mut self, now: Instant, l: &ThrottleLimits) {
        if self.names.len() < MAX_KEYS {
            return;
        }
        self.sweep(now, l);
        while self.names.len() >= MAX_KEYS {
            let victim = self
                .names
                .iter()
                .min_by_key(|(_, b)| b.eviction_rank(now))
                .map(|(k, _)| k.clone());
            match victim {
                Some(k) => {
                    self.names.remove(&k);
                }
                None => break,
            }
        }
    }

    /// Counts a refused attempt; `Some(n)` when it is time for the next report line.
    fn note_refusal(&mut self, now: Instant) -> Option<u64> {
        self.refused = self.refused.saturating_add(1);
        self.report_due(now)
    }

    /// `Some(n)`: `n > 0` refusals are waiting and the last report line is at least [`REFUSAL_REPORT_EVERY`] old (or there was
    /// none yet); the count starts over.
    fn report_due(&mut self, now: Instant) -> Option<u64> {
        let due = self
            .last_report
            .is_none_or(|t| now.saturating_duration_since(t) >= REFUSAL_REPORT_EVERY);
        if self.refused == 0 || !due {
            return None;
        }
        self.last_report = Some(now);
        Some(std::mem::take(&mut self.refused))
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

    /// Starts a password check for `target`; `known`: it comes from a known browser of that account (ignored for a name without
    /// an account). `Err` = locked (or held back by the pressure rule): answer 429 with the wait without checking the password.
    /// `Ok` = the attempt is counted as a failure until [`succeeded`](Self::succeeded) is called.
    pub fn begin(&self, target: Target<'_>, known: bool, now: Instant) -> Result<Begun, Refused> {
        let key = Key::of(target, known);
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
        let mut refuse = None;
        if let Some(b) = st.bucket(&key) {
            b.settle(now, &l);
            refuse = b.locked(now);
            if refuse.is_none() && pressure {
                if let Some(t) = b.last_failure {
                    let since = now.saturating_duration_since(t);
                    if since < l.pressure_gap {
                        refuse = Some(l.pressure_gap.saturating_sub(since));
                    }
                }
            }
        }
        if let Some(wait) = refuse {
            let report = st.note_refusal(now);
            return Err(Refused { wait, report });
        }
        let lock = st.bucket_or_new(key, now, &l).fail(now, &l);
        st.global.push_back(now);
        while st.global.len() > l.global {
            st.global.pop_front();
        }
        Ok(Begun { lock })
    }

    /// The attempt started with [`begin`](Self::begin) for account `id` succeeded: the bucket it came through starts over at
    /// round 1 (a success without the known-browser cookie starts the known-browser bucket over too), and the attempt leaves
    /// the global count.
    pub fn succeeded(&self, id: i64, known: bool) {
        let mut st = self.state();
        st.accounts.remove(&(id, known));
        if !known {
            st.accounts.remove(&(id, true));
        }
        st.global.pop_back();
    }

    /// Forgets everything about account `id`, from any browser (an administrator set a new password for it or unlocked it, or
    /// the account was deleted).
    pub fn forget(&self, id: i64) {
        let mut st = self.state();
        st.accounts.remove(&(id, false));
        st.accounts.remove(&(id, true));
    }

    /// The refused attempts counted since the last report line, when that line is at least [`REFUSAL_REPORT_EVERY`] old and
    /// there are any: the server's timer calls this once a minute, so the tail of a burst is reported even when no later attempt
    /// comes in (a refusal reports only itself and what came before it, see [`Refused::report`]).
    pub fn flush_refusals(&self, now: Instant) -> Option<u64> {
        self.state().report_due(now)
    }

    /// The state of account `id` for the user list: `None` when it is at round 1 and not locked in every bucket; otherwise the
    /// bucket that holds the user back most (a running lock first, the longer one; then the higher round), with the running
    /// lock of the other bucket when both are locked ([`LockStatus::other_wait`]).
    pub fn status(&self, id: i64, now: Instant) -> Option<LockStatus> {
        let l = self.limits;
        let mut st = self.state();
        let mut both = [false, true].into_iter().filter_map(|known| {
            let b = st.accounts.get_mut(&(id, known))?;
            b.settle(now, &l);
            b.status(now, &l, known)
        });
        let (first, second) = (both.next(), both.next());
        let (mut shown, other) = match (first, second) {
            (Some(a), Some(b)) if b.weight() > a.weight() => (b, Some(a)),
            (Some(a), b) => (a, b),
            (None, b) => (b?, None),
        };
        if shown.wait.is_some() {
            shown.other_wait = other.and_then(|o| o.wait);
        }
        Some(shown)
    }

    /// `(number of tracked names without an account, length in bytes of the longest one)`: a diagnostic for the memory bound.
    pub fn key_stats(&self) -> (usize, usize) {
        let st = self.state();
        let longest = st.names.keys().map(String::len).max().unwrap_or(0);
        (st.names.len(), longest)
    }

    /// Number of account buckets held (diagnostic; at most two per account).
    pub fn account_buckets(&self) -> usize {
        self.state().accounts.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);
    const SEC: Duration = Duration::from_secs(1);
    const ANN: Target<'static> = Target::Account(1);
    const BOB: Target<'static> = Target::Account(2);
    const ADMIN: Target<'static> = Target::Account(3);

    fn limits(global: usize) -> ThrottleLimits {
        ThrottleLimits {
            global,
            ..ThrottleLimits::default()
        }
    }

    /// Fails `n` times at `at` and asserts each one was let through.
    fn fail(t: &LoginThrottle, who: Target<'_>, known: bool, n: usize, at: Instant) {
        for i in 0..n {
            assert!(t.begin(who, known, at).is_ok(), "{who:?} attempt {i}");
        }
    }

    /// The wait of a refused attempt (panics when it was let through).
    fn wait(t: &LoginThrottle, who: Target<'_>, known: bool, at: Instant) -> Duration {
        match t.begin(who, known, at) {
            Err(r) => r.wait,
            Ok(b) => panic!("{who:?} was let through: {b:?}"),
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
        // round 1: five failures, then one minute; only the fifth reports the lock it starts
        fail(&t, ANN, false, 4, now);
        assert_eq!(
            t.begin(ANN, false, now).map(|b| b.lock),
            Ok(Some(StartedLock {
                round: 1,
                length: MIN
            }))
        );
        assert_eq!(wait(&t, ANN, false, now), MIN);
        assert_eq!(
            t.status(1, now),
            Some(LockStatus {
                wait: Some(MIN),
                round: 1,
                tries_left: 0,
                known_browser: false,
                other_wait: None,
            })
        );
        assert!(
            t.begin(BOB, false, now).is_ok(),
            "other accounts are not affected"
        );
        // rounds 2..: three failures each, counted from the end of the previous lock
        let expected = [5, 15, 30, 60, 1440, 1440, 1440];
        for (i, minutes) in expected.iter().enumerate() {
            let round = i as u32 + 2;
            now += t.limits().lock_for(round - 2);
            assert_eq!(
                t.status(1, now),
                Some(LockStatus {
                    wait: None,
                    round,
                    tries_left: 3,
                    known_browser: false,
                    other_wait: None,
                }),
                "round {round} begins when the lock ends"
            );
            fail(&t, ANN, false, 2, now);
            assert_eq!(t.status(1, now).map(|s| s.tries_left), Some(1));
            now += SEC;
            let lock = MIN * *minutes;
            assert_eq!(
                t.begin(ANN, false, now).map(|b| b.lock),
                Ok(Some(StartedLock {
                    round,
                    length: lock
                })),
                "third failure of round {round}"
            );
            assert_eq!(wait(&t, ANN, false, now), lock, "round {round}");
            // the lock does not grow while it is hammered, and even the right password waits (the caller never gets to check it)
            assert_eq!(wait(&t, ANN, false, now + lock - SEC), SEC);
            assert_eq!(
                t.status(1, now).map(|s| (s.round, s.wait)),
                Some((round, Some(lock)))
            );
        }
    }

    #[test]
    fn names_without_an_account_follow_the_same_rounds_case_insensitively() {
        let t = LoginThrottle::default();
        let now = Instant::now();
        fail(&t, Target::Name("Ghost"), false, 5, now);
        assert_eq!(wait(&t, Target::Name(" GHOST "), false, now), MIN);
        // the known flag means nothing for a name without an account
        assert_eq!(wait(&t, Target::Name("ghost"), true, now), MIN);
        fail(&t, Target::Name("ghost"), false, 3, now + MIN);
        assert_eq!(wait(&t, Target::Name("ghost"), false, now + MIN), 5 * MIN);
        // a name's state is not an account's: account 1 is fresh
        assert!(t.status(1, now).is_none());
        assert!(t.begin(ANN, false, now + MIN).is_ok());
    }

    #[test]
    fn round_one_counts_failures_within_fifteen_minutes() {
        let t = LoginThrottle::default();
        let t0 = Instant::now();
        // four failures, the oldest ages out of the window: a fifth and sixth are still allowed
        fail(&t, ANN, false, 1, t0);
        fail(&t, ANN, false, 3, t0 + 10 * MIN);
        let later = t0 + 15 * MIN;
        fail(&t, ANN, false, 1, later);
        assert!(t.status(1, later).is_none(), "still round 1");
        fail(&t, ANN, false, 1, later);
        // now five within the window: locked for one minute from the last failure
        assert_eq!(wait(&t, ANN, false, later + SEC), MIN - SEC);
        // later rounds have no window: three failures far apart still lock
        let r2 = later + MIN;
        fail(&t, ANN, false, 1, r2);
        fail(&t, ANN, false, 1, r2 + 5 * 60 * MIN);
        fail(&t, ANN, false, 1, r2 + 10 * 60 * MIN);
        assert_eq!(wait(&t, ANN, false, r2 + 10 * 60 * MIN), 5 * MIN);
    }

    #[test]
    fn resets_to_round_one() {
        let day = 24 * 60 * MIN;
        // a success, through an unknown browser
        let t = LoginThrottle::default();
        let now = Instant::now();
        fail(&t, ANN, false, 5, now);
        let now = now + MIN;
        fail(&t, ANN, false, 1, now);
        t.succeeded(1, false);
        assert!(t.status(1, now).is_none());
        fail(&t, ANN, false, 5, now);
        assert!(t.begin(ANN, false, now).is_err(), "five chances again");
        // an administrator's reset / unlock, even during a lock
        t.forget(1);
        assert!(t.status(1, now).is_none());
        fail(&t, ANN, false, 5, now);
        assert!(t.begin(ANN, false, now).is_err());
        // 24 hours after the last lock ended, with no new failure
        let t = LoginThrottle::default();
        let t0 = Instant::now();
        fail(&t, ANN, false, 5, t0);
        let end = t0 + MIN;
        assert_eq!(t.status(1, end + day - SEC).map(|s| s.round), Some(2));
        assert!(t.status(1, end + day).is_none(), "forgiven");
        fail(&t, ANN, false, 4, end + day);
        assert!(t.status(1, end + day).is_none(), "five chances again");
        // a failure after the lock pushes the 24 hours back; a running lock is never forgiven
        let t = LoginThrottle::default();
        fail(&t, ANN, false, 5, t0);
        fail(&t, ANN, false, 1, end + 12 * 60 * MIN);
        assert_eq!(t.status(1, end + day).map(|s| s.tries_left), Some(2));
        let quiet = end + 12 * 60 * MIN + day;
        assert!(t.status(1, quiet).is_none());
        let t = LoginThrottle::default();
        let mut now = t0;
        for r in 0..6u32 {
            fail(&t, ANN, false, if r == 0 { 5 } else { 3 }, now);
            now += t.limits().lock_for(r);
        }
        // the sixth lock (one day) is running until `now`
        assert_eq!(t.status(1, now - SEC).map(|s| s.round), Some(6));
        assert_eq!(t.status(1, now).map(|s| s.round), Some(7));
        assert!(t.status(1, now + day).is_none());
    }

    #[test]
    fn global_pressure_slows_guessed_names_but_never_locks_out_a_fresh_one() {
        let t = LoginThrottle::new(limits(10));
        let t0 = Instant::now();
        for i in 0..10 {
            assert!(t.begin(Target::Account(100 + i), false, t0).is_ok());
        }
        // under pressure: a bucket with a failure in the last minute waits, one without gets through
        assert_eq!(wait(&t, Target::Account(100), false, t0 + SEC), MIN - SEC);
        assert!(t.begin(ADMIN, false, t0 + SEC).is_ok());
        assert!(t.begin(ADMIN, false, t0 + 2 * SEC).is_err());
        // made-up names that break the username rule share one key, so they are slowed down together
        assert!(t
            .begin(Target::Name("not a name"), false, t0 + 3 * SEC)
            .is_ok());
        assert!(t
            .begin(Target::Name("also not a name!"), false, t0 + 4 * SEC)
            .is_err());
        // a flood can keep the pressure on for ever, yet every bucket still gets one try per minute
        for i in 0..20 {
            let name = format!("flood{i}");
            assert!(t.begin(Target::Name(&name), false, t0 + 10 * SEC).is_ok());
        }
        assert!(t.begin(ADMIN, false, t0 + 62 * SEC).is_ok());
        assert!(t.begin(Target::Account(100), false, t0 + 62 * SEC).is_ok());
        // pressure refusals do not advance the round
        assert!(t.status(100, t0 + 62 * SEC).is_none());
    }

    #[test]
    fn a_known_browser_has_its_own_rounds() {
        let t = LoginThrottle::default();
        let t0 = Instant::now();
        fail(&t, ADMIN, false, 5, t0);
        assert!(t.begin(ADMIN, false, t0).is_err());
        // the owner's browser has its own count and signs in
        assert!(t.begin(ADMIN, true, t0).is_ok());
        t.succeeded(3, true);
        // a success there does not start the guessed bucket over
        assert_eq!(wait(&t, ADMIN, false, t0), MIN);
        // guesses from elsewhere escalate through the rounds, the known browser is never locked by them
        let mut now = t0;
        for r in 0..8u32 {
            if r > 0 {
                fail(&t, ADMIN, false, 3, now);
            }
            assert!(t.begin(ADMIN, false, now).is_err(), "round {}", r + 1);
            assert!(t.begin(ADMIN, true, now).is_ok());
            t.succeeded(3, true);
            now += t.limits().lock_for(r);
        }
        assert_eq!(
            t.status(3, now).map(|s| (s.round, s.known_browser)),
            Some((9, false))
        );
        // the known browser follows the same rounds when it guesses wrong itself
        fail(&t, ADMIN, true, 5, now);
        assert_eq!(wait(&t, ADMIN, true, now), MIN);
        fail(&t, ADMIN, true, 3, now + MIN);
        assert_eq!(wait(&t, ADMIN, true, now + MIN), 5 * MIN);
        // the status shows the bucket that holds the user back most: here the unknown browsers' (higher round, no lock)
        // versus the known browser's 5-minute lock: a running lock wins, and it says whose it is
        assert_eq!(
            t.status(3, now + MIN)
                .map(|s| (s.round, s.wait, s.known_browser)),
            Some((2, Some(5 * MIN), true))
        );
        // a success without the cookie starts both buckets over
        t.succeeded(3, false);
        assert!(t.status(3, now + MIN).is_none());
        // Unlock clears both
        fail(&t, ADMIN, false, 5, now);
        fail(&t, ADMIN, true, 5, now);
        t.forget(3);
        assert!(t.begin(ADMIN, true, now).is_ok());
        assert!(t.begin(ADMIN, false, now).is_ok());
        assert_eq!(t.account_buckets(), 2);
        // an invalid name never gets a key of its own
        assert!(t.begin(Target::Name("not a name"), true, now).is_ok());
        assert_eq!(t.key_stats(), (1, 0));
    }

    #[test]
    fn keys_of_names_are_bounded_in_length_and_number() {
        let t = LoginThrottle::new(limits(usize::MAX));
        let t0 = Instant::now();
        for i in 0..20 {
            let huge = format!("{i}{}", "x".repeat(1_000_000));
            assert!(t.begin(Target::Name(&huge), false, t0).is_ok() || i >= 5);
        }
        assert_eq!(t.key_stats(), (1, 0), "every invalid name shares one key");
        let long = "y".repeat(super::super::USERNAME_MAX_CHARS);
        assert!(t.begin(Target::Name(&long), false, t0).is_ok());
        assert_eq!(t.key_stats(), (2, super::super::USERNAME_MAX_CHARS));
        // the map never grows past MAX_KEYS; the least advanced names are forgotten first, never a locked one
        fail(&t, Target::Name("target"), false, 5, t0);
        for i in 0..MAX_KEYS + 50 {
            let name = format!("n{i}");
            assert!(t.begin(Target::Name(&name), false, t0).is_ok());
        }
        assert!(t.key_stats().0 <= MAX_KEYS);
        assert!(
            t.begin(Target::Name("target"), false, t0 + SEC).is_err(),
            "a locked name is not evicted by a flood of new ones"
        );
        // idle buckets are swept out once a window has passed; names past round 1 are kept for the 24-hour rule (here
        // "target" and the shared key of invalid names, both locked once)
        assert!(t
            .begin(Target::Name("fresh"), false, t0 + 15 * MIN + SEC)
            .is_ok());
        assert_eq!(t.key_stats().0, 3);
    }

    /// Regression (v0.6.5 review): a full map of names in round 3 used to evict an account's round-2 bucket for every new name,
    /// which gave a guesser five tries per minute for ever instead of the escalating locks.
    #[test]
    fn a_flood_of_names_never_evicts_an_account() {
        let t = LoginThrottle::new(limits(usize::MAX));
        let t0 = Instant::now();
        // "admin" (account 3) is in round 2, not locked right now
        fail(&t, ADMIN, false, 5, t0);
        let r2 = t0 + MIN;
        fail(&t, ADMIN, false, 1, r2);
        // the map of names is filled with names in round 3 (more advanced than admin), none locked right now
        for i in 0..MAX_KEYS {
            let name = format!("f{i}");
            fail(&t, Target::Name(&name), false, 5, t0);
            fail(&t, Target::Name(&name), false, 3, r2);
        }
        let r3 = r2 + 5 * MIN;
        assert_eq!(t.key_stats().0, MAX_KEYS);
        // brand-new names keep arriving: each one evicts a name, never the account
        for i in 0..50 {
            let name = format!("evict{i}");
            assert!(t.begin(Target::Name(&name), false, r3).is_ok());
            assert!(t.key_stats().0 <= MAX_KEYS);
        }
        assert_eq!(
            t.status(3, r3).map(|s| (s.round, s.tries_left)),
            Some((2, 2)),
            "admin is still in round 2 with two tries left"
        );
        fail(&t, ADMIN, false, 2, r3);
        assert_eq!(wait(&t, ADMIN, false, r3), 5 * MIN, "the second lock");
        // and a locked account is not evicted either, even when every tracked name is locked
        for i in 0..50 {
            let name = format!("locked{i}");
            fail(&t, Target::Name(&name), false, 5, r3);
        }
        assert_eq!(wait(&t, ADMIN, false, r3 + SEC), 5 * MIN - SEC);
    }

    /// Regression (v0.6.5 re-review): with both buckets locked the user list showed only the longer lock and said the other
    /// browsers still worked.
    #[test]
    fn the_status_says_when_every_browser_is_locked() {
        let t = LoginThrottle::default();
        let t0 = Instant::now();
        // other browsers: round 2 lock (5 minutes); then the known browsers: round 1 lock (1 minute)
        fail(&t, ADMIN, false, 5, t0);
        fail(&t, ADMIN, false, 3, t0 + MIN);
        let now = t0 + MIN + 10 * SEC;
        assert_eq!(
            t.status(3, now).map(|s| (s.known_browser, s.other_wait)),
            Some((false, None)),
            "only the other browsers are locked"
        );
        fail(&t, ADMIN, true, 5, now);
        let s = t.status(3, now).expect("locked");
        assert_eq!(
            (s.known_browser, s.wait, s.other_wait),
            (false, Some(5 * MIN - 10 * SEC), Some(MIN)),
            "the longer lock is shown, with the other one"
        );
        // once the known browsers' lock ends, only one lock is left
        assert_eq!(t.status(3, now + MIN).and_then(|s| s.other_wait), None);
        // a bucket without a running lock never sets other_wait (the locked one is shown instead)
        let t = LoginThrottle::default();
        fail(&t, ADMIN, false, 5, t0);
        fail(&t, ADMIN, true, 5, t0 + 2 * MIN);
        let s = t.status(3, t0 + 2 * MIN).expect("status");
        assert_eq!(
            (s.known_browser, s.wait, s.other_wait),
            (true, Some(MIN), None)
        );
    }

    #[test]
    fn refusals_are_reported_at_most_once_a_minute() {
        let t = LoginThrottle::default();
        let t0 = Instant::now();
        fail(&t, ANN, false, 5, t0);
        let reports: Vec<Option<u64>> = (0..5)
            .map(|i| {
                t.begin(ANN, false, t0 + i * SEC)
                    .err()
                    .and_then(|r| r.report)
            })
            .collect();
        assert_eq!(reports, vec![Some(1), None, None, None, None]);
        // a minute after the last report: one line for the four refusals since, plus this one
        let later = t0 + MIN;
        fail(&t, BOB, false, 5, later);
        assert_eq!(
            t.begin(BOB, false, later).err().and_then(|r| r.report),
            Some(5)
        );
        // the tail of a burst is reported by the timer even when no later attempt comes in
        assert_eq!(t.flush_refusals(later + 30 * SEC), None, "nothing waiting");
        for i in 1..=3 {
            assert!(t.begin(BOB, false, later + i * SEC).is_err());
        }
        assert_eq!(t.flush_refusals(later + 30 * SEC), None, "not a minute yet");
        assert_eq!(t.flush_refusals(later + MIN), Some(3));
        assert_eq!(t.flush_refusals(later + 3 * MIN), None, "reported once");
        // the next refusal after a quiet minute is reported at once again
        let ann = later + 2 * MIN;
        fail(&t, ANN, false, 3, ann + 5 * MIN); // round 2 (the first lock is long over)
        assert_eq!(
            t.begin(ANN, false, ann + 5 * MIN)
                .err()
                .and_then(|r| r.report),
            Some(1)
        );
    }
}
