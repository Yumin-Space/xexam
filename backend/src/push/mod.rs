//! Web push (v3.1). The backend owns the devices, the subscriptions, the queue, the triggers, the quiet hours, the texts, the
//! RFC 8291 encryption and the RFC 8292 VAPID signature; it never connects to the internet. The ai-service is a stateless
//! courier that long-polls `POST /internal/ai/push/next`, posts each job to the push service and reports the outcome.
//!
//! - [`vapid`]     the VAPID key (file / environment), key id, the JWT of the `Authorization` header
//! - [`ece`]       RFC 8291 `aes128gcm` encryption of one message
//! - [`allow`]     the allow-list of push-service endpoints
//! - [`texts`]     the notification texts (en / zh / ja) and the plaintext of a payload
//! - [`quiet`]     quiet hours (00:00 to 08:00 local time) and their release time
//! - [`scheduler`] start-up work, the periodic ticks and the housekeeping
//!
//! [`Push`] is the in-memory handle kept in `AppState`: the loaded key, courier presence, the wake-up of the courier's
//! long poll and the per-device throttle of test notifications.

pub mod allow;
pub mod ece;
pub mod quiet;
pub mod scheduler;
pub mod texts;
pub mod vapid;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// How long after its last `push/next` poll the courier counts as connected (seconds).
const COURIER_WINDOW_SECS: i64 = 60;
/// Minimum spacing of two test notifications of one device (seconds).
const TEST_SPACING_SECS: i64 = 10;
/// Test throttle entries older than this are dropped when the table grows (seconds).
const TEST_FORGET_SECS: i64 = 3600;
/// `system.pushAuth` is recorded at most once per host per this many seconds.
const AUTH_NOTE_SECS: i64 = 3600;

/// Decodes base64url, tolerating `=` padding and the standard alphabet (`+` and `/`); `None` when it is not base64.
pub fn b64_lenient(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    let t: String = s
        .trim()
        .trim_end_matches('=')
        .chars()
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            other => other,
        })
        .collect();
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(t.as_bytes())
        .ok()
}

#[derive(Default)]
struct KeyState {
    key: Option<Arc<vapid::Vapid>>,
    subject: String,
}

#[derive(Default)]
struct PushInner {
    key: Mutex<KeyState>,
    /// Unix seconds of the last courier poll (0 = never)
    last_poll: AtomicI64,
    wake: tokio::sync::Notify,
    closed: AtomicBool,
    /// device id -> unix seconds of its last accepted test notification
    tests: Mutex<HashMap<i64, i64>>,
    /// push-service host -> unix seconds of its last `system.pushAuth` record
    auth_noted: Mutex<HashMap<String, i64>>,
}

/// The in-memory push state shared by the handlers, the scheduler and the courier routes.
#[derive(Clone, Default)]
pub struct Push(Arc<PushInner>);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Push {
    pub fn new() -> Push {
        Push::default()
    }

    /// Installs the loaded VAPID key and the `sub` claim (`None`: push is off, `/api/push/config` says so).
    pub fn set_key(&self, key: Option<vapid::Vapid>, subject: String) {
        let mut g = lock(&self.0.key);
        g.key = key.map(Arc::new);
        g.subject = subject;
    }

    pub fn key(&self) -> Option<Arc<vapid::Vapid>> {
        lock(&self.0.key).key.clone()
    }

    pub fn subject(&self) -> String {
        lock(&self.0.key).subject.clone()
    }

    /// Records a courier poll (`POST /internal/ai/push/next`).
    pub fn note_poll(&self, now: i64) {
        self.0.last_poll.store(now, Ordering::Relaxed);
    }

    /// Whether a courier polled within the last 60 seconds.
    pub fn courier_connected(&self, now: i64) -> bool {
        let last = self.0.last_poll.load(Ordering::Relaxed);
        last > 0 && now.saturating_sub(last) <= COURIER_WINDOW_SECS
    }

    /// Wakes a waiting courier poll (something is due). A wake-up with nobody waiting is kept for the next wait.
    pub fn wake(&self) {
        self.0.wake.notify_one();
    }

    /// Ends every wait for good (shutdown): a waiting `push/next` returns before the graceful wait.
    pub fn close(&self) {
        self.0.closed.store(true, Ordering::SeqCst);
        self.0.wake.notify_waiters();
        self.0.wake.notify_one();
    }

    pub fn is_closed(&self) -> bool {
        self.0.closed.load(Ordering::SeqCst)
    }

    /// Returns on a wake-up, on [`Push::close`] or after `max`, whichever comes first.
    pub async fn wait(&self, max: Duration) {
        if self.is_closed() {
            return;
        }
        let _ = tokio::time::timeout(max, self.0.wake.notified()).await;
    }

    /// Whether a `system.pushAuth` record for `host` may be written now (at most once an hour per host; claims the slot).
    pub fn auth_note_allowed(&self, host: &str, now: i64) -> bool {
        let mut g = lock(&self.0.auth_noted);
        if g.len() > 256 {
            g.retain(|_, at| now.saturating_sub(*at) < AUTH_NOTE_SECS);
        }
        match g.get(host) {
            Some(at) if now.saturating_sub(*at) < AUTH_NOTE_SECS => false,
            _ => {
                g.insert(host.to_string(), now);
                true
            }
        }
    }

    /// One test notification per device per 10 seconds: `true` claims the slot.
    pub fn test_allowed(&self, device_id: i64, now: i64) -> bool {
        let mut g = lock(&self.0.tests);
        if g.len() > 1024 {
            g.retain(|_, at| now.saturating_sub(*at) < TEST_FORGET_SECS);
        }
        match g.get(&device_id) {
            Some(at) if now.saturating_sub(*at) < TEST_SPACING_SECS => false,
            _ => {
                g.insert(device_id, now);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn courier_presence_follows_the_last_poll() {
        let p = Push::new();
        assert!(!p.courier_connected(1_000));
        p.note_poll(1_000);
        assert!(p.courier_connected(1_000));
        assert!(p.courier_connected(1_060));
        assert!(!p.courier_connected(1_061));
    }

    #[test]
    fn a_device_gets_one_test_per_ten_seconds() {
        let p = Push::new();
        assert!(p.test_allowed(7, 100));
        assert!(!p.test_allowed(7, 109));
        assert!(p.test_allowed(8, 109));
        assert!(p.test_allowed(7, 110));
    }

    #[test]
    fn auth_records_are_limited_to_one_an_hour_per_host() {
        let p = Push::new();
        assert!(p.auth_note_allowed("push.example", 100));
        assert!(!p.auth_note_allowed("push.example", 3_699));
        assert!(p.auth_note_allowed("other.example", 3_699));
        assert!(p.auth_note_allowed("push.example", 3_700));
    }

    #[test]
    fn base64_is_read_leniently() {
        assert_eq!(b64_lenient("aGk").as_deref(), Some(&b"hi"[..]));
        assert_eq!(b64_lenient("aGk=").as_deref(), Some(&b"hi"[..]));
        assert_eq!(b64_lenient("-_8").as_deref(), Some(&[0xfb, 0xff][..]));
        assert_eq!(b64_lenient("+/8=").as_deref(), Some(&[0xfb, 0xff][..]));
        assert_eq!(b64_lenient("a b"), None);
    }

    #[test]
    fn the_key_and_subject_are_kept() {
        let p = Push::new();
        assert!(p.key().is_none());
        p.set_key(None, "mailto:admin@localhost".to_string());
        assert_eq!(p.subject(), "mailto:admin@localhost");
    }

    #[tokio::test]
    async fn wait_returns_on_wake_close_and_timeout() {
        let p = Push::new();
        p.wake();
        p.wait(Duration::from_secs(30)).await;
        let started = std::time::Instant::now();
        p.wait(Duration::from_millis(20)).await;
        assert!(started.elapsed() >= Duration::from_millis(15));
        p.close();
        assert!(p.is_closed());
        p.wait(Duration::from_secs(30)).await;
    }
}
