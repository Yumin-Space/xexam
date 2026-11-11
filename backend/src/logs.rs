//! System log (v0.7): the most recent log events, kept in memory for the administrators' "System log" page.
//!
//! [`BufferLayer`] is a `tracing` layer installed next to the stderr formatter in `main.rs`, behind the same `XEXAM_LOG`
//! filter, so the page shows exactly what `docker compose logs backend` shows (plus the plain status lines that
//! `main.rs` prints, see [`record`]). Every event becomes a [`LogLine`] with a sequence number that grows by one per event
//! (starting at 1 in each process), the local time with seconds, the level, the target (module path) and the message
//! followed by its fields.
//!
//! The ring buffer keeps the last [`CAPACITY`] lines; older ones are dropped. It lives only in memory: it is empty again
//! after a restart and nothing is written to the database. `GET /api/logs?after=<seq>&boot=<id>` ([`LogBuffer::since_boot`])
//! returns the lines after `after`. Each buffer has a random `boot` id that every answer carries: the sequence numbers start
//! again at 1 after a restart, so a client that sends the `boot` it last saw gets every kept line when it differs (the
//! backend restarted in between), instead of only the lines above a number that belonged to the previous process.
//!
//! Lines that anyone can cause without signing in (requests rejected by the Host / Origin guard, failed sign-ins, locks of
//! usernames without an account) go through a [`BurstLog`], at most about one line a minute each, so a flood of them cannot
//! push the other lines out of the buffer.
//!
//! What is logged must never contain a password, a session token, a cookie value or question text: the log calls in this
//! crate only log usernames (never a typed name that matches no account), bank and file names, ids, counts, paths and error
//! kinds.
use serde::Serialize;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::{Arc, LazyLock, Mutex};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

/// Number of lines the buffer keeps.
pub const CAPACITY: usize = 2000;
/// Longest message kept per line (in characters); longer messages are cut and end with `…`.
pub const MAX_MESSAGE_CHARS: usize = 4000;

/// One log event as `GET /api/logs` returns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogLine {
    /// Sequence number: 1 for the first event of this process, then one more for each event
    pub seq: u64,
    /// Local time `YYYY-MM-DD HH:MM:SS` (the host's time zone, like every other timestamp)
    pub time: String,
    /// `error`, `warn`, `info`, `debug` or `trace`
    pub level: &'static str,
    /// The module that logged it, for example `xexam_backend::api`
    pub target: String,
    /// The message, then its fields as ` key=value`
    pub message: String,
}

/// The answer of [`LogBuffer::since`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogPage {
    /// The lines after `after`, oldest first
    pub lines: Vec<LogLine>,
    /// The sequence number of the newest line in the buffer (0 when nothing has been logged yet): the `after` of the next poll
    pub last: u64,
    /// Lines newer than `after` were dropped from the buffer before they could be returned (older lines are gone)
    pub dropped: bool,
    /// The id of this buffer (a new random one in each process): the `boot` of the next poll
    pub boot: String,
}

struct Ring {
    lines: VecDeque<LogLine>,
    /// Sequence number of the next line
    next: u64,
}

/// A ring buffer of the most recent log lines.
pub struct LogBuffer {
    ring: Mutex<Ring>,
    capacity: usize,
    /// Random id of this buffer, so a client can tell a restarted backend from the one it polled before
    boot: String,
}

impl Default for LogBuffer {
    fn default() -> Self {
        LogBuffer::new(CAPACITY)
    }
}

/// `level` as the API spells it.
pub fn level_name(level: Level) -> &'static str {
    match level {
        Level::ERROR => "error",
        Level::WARN => "warn",
        Level::INFO => "info",
        Level::DEBUG => "debug",
        Level::TRACE => "trace",
    }
}

fn cut(mut message: String) -> String {
    if let Some((i, _)) = message.char_indices().nth(MAX_MESSAGE_CHARS) {
        message.truncate(i);
        message.push('…');
    }
    message
}

impl LogBuffer {
    /// An empty buffer that keeps `capacity` lines (at least one).
    pub fn new(capacity: usize) -> LogBuffer {
        let capacity = capacity.max(1);
        LogBuffer {
            ring: Mutex::new(Ring {
                lines: VecDeque::with_capacity(capacity.min(CAPACITY)),
                next: 1,
            }),
            capacity,
            boot: crate::security::new_token().chars().take(12).collect(),
        }
    }

    /// The random id of this buffer (see [`LogPage::boot`]).
    pub fn boot(&self) -> &str {
        &self.boot
    }

    fn ring(&self) -> std::sync::MutexGuard<'_, Ring> {
        // A panic while holding the lock cannot leave the ring inconsistent (each push is one push_back / pop_front)
        self.ring.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds a line stamped with the current local time; returns its sequence number.
    pub fn push(&self, level: Level, target: &str, message: String) -> u64 {
        let time = crate::database::timefmt::format_second(&jiff::Zoned::now());
        self.push_at(time, level, target, message)
    }

    /// Adds a line with the given time (for tests); returns its sequence number.
    pub fn push_at(&self, time: String, level: Level, target: &str, message: String) -> u64 {
        let message = cut(message);
        let mut r = self.ring();
        let seq = r.next;
        r.next += 1;
        if r.lines.len() >= self.capacity {
            r.lines.pop_front();
        }
        r.lines.push_back(LogLine {
            seq,
            time,
            level: level_name(level),
            target: target.to_string(),
            message,
        });
        seq
    }

    /// The lines with a sequence number greater than `after`, oldest first (all kept lines when `after` is `None`).
    ///
    /// An `after` greater than the newest sequence number comes from before a restart of the backend (the numbers start
    /// at 1 again): it is answered like no `after`, with every kept line. `dropped` is true when the oldest kept line is
    /// not the one right after `after` (or, without `after`, not line 1): lines in between were dropped.
    pub fn since(&self, after: Option<u64>) -> LogPage {
        self.since_boot(after, None)
    }

    /// [`LogBuffer::since`] for a client that last saw the buffer `boot`: when it is not this buffer's id, the client's
    /// `after` belongs to an earlier process (the backend restarted, and may already have logged more lines than `after`),
    /// so it is answered like no `after`, with every kept line.
    pub fn since_boot(&self, after: Option<u64>, boot: Option<&str>) -> LogPage {
        let after = if boot.is_some_and(|b| b != self.boot) {
            None
        } else {
            after
        };
        let r = self.ring();
        let last = r.next - 1;
        let after = after.filter(|a| *a <= last).unwrap_or(0);
        let oldest = r.lines.front().map_or(r.next, |l| l.seq);
        // The kept lines are consecutive, so the first one to return is found by subtraction
        let skip = usize::try_from((after + 1).saturating_sub(oldest)).unwrap_or(usize::MAX);
        LogPage {
            lines: r.lines.iter().skip(skip).cloned().collect(),
            last,
            dropped: oldest > after + 1,
            boot: self.boot.clone(),
        }
    }
}

static GLOBAL: LazyLock<Arc<LogBuffer>> = LazyLock::new(|| Arc::new(LogBuffer::default()));

/// The buffer of this process: [`BufferLayer::global`] writes into it and the server reads it.
pub fn global() -> Arc<LogBuffer> {
    GLOBAL.clone()
}

/// Adds a plain status line (one that `main.rs` prints to stdout instead of logging it) to the buffer of this process,
/// so the System log page shows the startup and shutdown lines too. Never call it with a secret.
pub fn record(level: Level, target: &str, message: &str) {
    GLOBAL.push(level, target, message.to_string());
}

/// The `tracing` layer that feeds a [`LogBuffer`].
pub struct BufferLayer {
    buffer: Arc<LogBuffer>,
}

impl BufferLayer {
    /// A layer writing into `buffer`.
    pub fn new(buffer: Arc<LogBuffer>) -> BufferLayer {
        BufferLayer { buffer }
    }

    /// A layer writing into the buffer of this process ([`global`]).
    pub fn global() -> BufferLayer {
        BufferLayer::new(global())
    }
}

/// Collects the `message` field and the other fields of an event as `message key=value key=value`.
#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            let _ = write!(self.fields, " {}={value:?}", field.name());
        }
    }
}

impl<S: Subscriber> Layer<S> for BufferLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut v = MessageVisitor::default();
        event.record(&mut v);
        let mut message = v.message;
        if message.is_empty() {
            message = v.fields.trim_start().to_string();
        } else {
            message.push_str(&v.fields);
        }
        self.buffer.push(*meta.level(), meta.target(), message);
    }
}

/// How often a [`BurstLog`] sums up the events it held back (the same pace as the refused sign-ins of the login throttle).
pub const BURST_REPORT_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// What a [`BurstLog`] says should be logged now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BurstReport {
    /// One event since the last report line (the first one after a quiet period): log it on its own, as it is
    One(String),
    /// `count` events since the last report line; `last` describes the most recent one
    Many { count: u64, last: String },
}

#[derive(Default)]
struct BurstState {
    /// Events not reported yet, and the description of the most recent of them
    pending: u64,
    last: String,
    /// When the last report line was written
    last_report: Option<std::time::Instant>,
}

/// Rate limit for a kind of log line that anyone who can reach the server can cause without signing in (requests rejected by
/// the Host / Origin guard, failed sign-ins). One line each would let a flood push every other line out of the buffer (it keeps
/// the last [`CAPACITY`]). Instead, the first event after a quiet [`BURST_REPORT_EVERY`] is logged at once, and the ones that
/// follow within it are counted and reported in one line: by a later event once the minute is over, or by
/// [`BurstLog::flush`], which the server's once-a-minute timer calls so the tail of a burst is reported even when nothing
/// follows it. The caller formats and writes the line.
#[derive(Default)]
pub struct BurstLog {
    state: Mutex<BurstState>,
}

impl BurstLog {
    fn state(&self) -> std::sync::MutexGuard<'_, BurstState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Counts an event described by `what`; `Some` when a line is due now.
    pub fn note(&self, now: std::time::Instant, what: String) -> Option<BurstReport> {
        let mut st = self.state();
        st.pending = st.pending.saturating_add(1);
        st.last = what;
        Self::due(&mut st, now)
    }

    /// The events counted since the last report line, when that line is at least [`BURST_REPORT_EVERY`] old.
    pub fn flush(&self, now: std::time::Instant) -> Option<BurstReport> {
        Self::due(&mut self.state(), now)
    }

    fn due(st: &mut BurstState, now: std::time::Instant) -> Option<BurstReport> {
        let due = st
            .last_report
            .is_none_or(|t| now.saturating_duration_since(t) >= BURST_REPORT_EVERY);
        if st.pending == 0 || !due {
            return None;
        }
        st.last_report = Some(now);
        let count = std::mem::take(&mut st.pending);
        let last = std::mem::take(&mut st.last);
        Some(if count == 1 {
            BurstReport::One(last)
        } else {
            BurstReport::Many { count, last }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    #[test]
    fn a_burst_gives_one_line_then_one_summary_a_minute() {
        use std::time::{Duration, Instant};
        let log = BurstLog::default();
        let t0 = Instant::now();
        assert_eq!(
            log.note(t0, "GET /a: x".into()),
            Some(BurstReport::One("GET /a: x".into()))
        );
        for i in 0..5000u64 {
            assert_eq!(
                log.note(t0 + Duration::from_millis(i), format!("GET /{i}")),
                None
            );
        }
        assert_eq!(
            log.flush(t0 + Duration::from_secs(30)),
            None,
            "not a minute yet"
        );
        assert_eq!(
            log.flush(t0 + Duration::from_secs(60)),
            Some(BurstReport::Many {
                count: 5000,
                last: "GET /4999".into()
            })
        );
        assert_eq!(
            log.flush(t0 + Duration::from_secs(200)),
            None,
            "nothing new"
        );
        // after a quiet minute the next event is logged on its own again
        assert_eq!(
            log.note(t0 + Duration::from_secs(200), "POST /b: y".into()),
            Some(BurstReport::One("POST /b: y".into()))
        );
        // a single event within the minute after a report is reported on its own by the timer, not as a "summary of 1"
        assert_eq!(log.note(t0 + Duration::from_secs(210), "c".into()), None);
        assert_eq!(
            log.flush(t0 + Duration::from_secs(260)),
            Some(BurstReport::One("c".into()))
        );
    }

    fn fill(b: &LogBuffer, n: u64) {
        for i in 1..=n {
            b.push_at(
                "2026-01-02 03:04:05".to_string(),
                Level::INFO,
                "t",
                format!("line {i}"),
            );
        }
    }

    fn seqs(p: &LogPage) -> Vec<u64> {
        p.lines.iter().map(|l| l.seq).collect()
    }

    #[test]
    fn an_empty_buffer() {
        let b = LogBuffer::new(3);
        let p = b.since(None);
        assert!(p.lines.is_empty());
        assert_eq!((p.last, p.dropped), (0, false));
        let p = b.since(Some(0));
        assert_eq!((p.lines.len(), p.last, p.dropped), (0, 0, false));
    }

    #[test]
    fn sequence_numbers_start_at_one_and_grow_by_one() {
        let b = LogBuffer::new(10);
        fill(&b, 4);
        let p = b.since(None);
        assert_eq!(seqs(&p), [1, 2, 3, 4]);
        assert_eq!((p.last, p.dropped), (4, false));
        assert_eq!(p.lines[0].message, "line 1");
        assert_eq!(p.lines[0].level, "info");
        assert_eq!(p.lines[0].time, "2026-01-02 03:04:05");
    }

    #[test]
    fn after_returns_only_newer_lines() {
        let b = LogBuffer::new(10);
        fill(&b, 5);
        assert_eq!(seqs(&b.since(Some(0))), [1, 2, 3, 4, 5]);
        assert_eq!(seqs(&b.since(Some(3))), [4, 5]);
        let p = b.since(Some(5));
        assert!(p.lines.is_empty());
        assert_eq!((p.last, p.dropped), (5, false));
    }

    #[test]
    fn a_client_of_an_earlier_process_gets_every_kept_line() {
        // The client polled the previous process up to line 50; the restarted backend has already logged 60 lines
        let before = LogBuffer::new(100);
        let after = LogBuffer::new(100);
        assert_ne!(before.boot(), after.boot(), "each buffer has its own id");
        assert_eq!(after.boot().len(), 12);
        fill(&after, 60);
        let p = after.since_boot(Some(50), Some(before.boot()));
        assert_eq!(
            p.lines.len(),
            60,
            "lines 1 to 50 of the new process are not skipped"
        );
        assert_eq!((p.last, p.dropped), (60, false));
        assert_eq!(p.boot, after.boot());
        // The same client after that answer: only newer lines again
        assert_eq!(
            seqs(&after.since_boot(Some(58), Some(after.boot()))),
            [59, 60]
        );
        // Without a boot (older clients) the number alone decides, as before
        assert_eq!(seqs(&after.since_boot(Some(58), None)), [59, 60]);
    }

    #[test]
    fn the_buffer_keeps_only_the_newest_lines() {
        let b = LogBuffer::new(3);
        fill(&b, 7);
        let p = b.since(None);
        assert_eq!(seqs(&p), [5, 6, 7]);
        assert_eq!(p.last, 7);
        assert!(p.dropped, "lines 1 to 4 are gone");
        // A client that saw line 2: lines 3 and 4 were dropped in between
        let p = b.since(Some(2));
        assert_eq!(seqs(&p), [5, 6, 7]);
        assert!(p.dropped);
        // A client that saw line 4: nothing was missed
        let p = b.since(Some(4));
        assert_eq!(seqs(&p), [5, 6, 7]);
        assert!(!p.dropped);
        let p = b.since(Some(6));
        assert_eq!(seqs(&p), [7]);
        assert!(!p.dropped);
    }

    #[test]
    fn the_default_capacity_is_2000() {
        let b = LogBuffer::default();
        fill(&b, 2005);
        let p = b.since(None);
        assert_eq!(p.lines.len(), CAPACITY);
        assert_eq!(CAPACITY, 2000);
        assert_eq!(p.lines.first().map(|l| l.seq), Some(6));
        assert_eq!(p.last, 2005);
        assert!(p.dropped);
    }

    #[test]
    fn an_after_from_before_a_restart_gets_every_kept_line() {
        let b = LogBuffer::new(10);
        fill(&b, 3);
        let p = b.since(Some(500));
        assert_eq!(seqs(&p), [1, 2, 3]);
        assert_eq!((p.last, p.dropped), (3, false));
    }

    #[test]
    fn long_messages_are_cut() {
        let b = LogBuffer::new(2);
        b.push(Level::WARN, "t", "é".repeat(MAX_MESSAGE_CHARS + 10));
        let p = b.since(None);
        let m = &p.lines[0].message;
        assert_eq!(m.chars().count(), MAX_MESSAGE_CHARS + 1);
        assert!(m.ends_with('…'));
        assert_eq!(p.lines[0].level, "warn");
        // The real clock: `YYYY-MM-DD HH:MM:SS`
        assert_eq!(p.lines[0].time.len(), 19);
    }

    #[test]
    fn the_layer_records_level_target_message_and_fields() {
        let buffer = Arc::new(LogBuffer::new(10));
        let subscriber = tracing_subscriber::registry().with(BufferLayer::new(buffer.clone()));
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("hello {}", 42);
            tracing::warn!(count = 3, name = "x", "purged");
            tracing::error!(target: "custom", "broken");
            tracing::debug!(only = true);
        });
        let p = buffer.since(None);
        let got: Vec<(&str, &str, &str)> = p
            .lines
            .iter()
            .map(|l| (l.level, l.target.as_str(), l.message.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("info", "xexam_backend::logs::tests", "hello 42"),
                (
                    "warn",
                    "xexam_backend::logs::tests",
                    "purged count=3 name=\"x\""
                ),
                ("error", "custom", "broken"),
                ("debug", "xexam_backend::logs::tests", "only=true"),
            ]
        );
    }

    #[test]
    fn the_json_shape() {
        let b = LogBuffer::new(5);
        b.push_at(
            "2026-01-02 03:04:05".to_string(),
            Level::ERROR,
            "xexam_backend::api",
            "database error: disk full".to_string(),
        );
        let v = serde_json::to_value(b.since(Some(0))).expect("json");
        assert_eq!(
            v,
            serde_json::json!({
                "lines": [{
                    "seq": 1,
                    "time": "2026-01-02 03:04:05",
                    "level": "error",
                    "target": "xexam_backend::api",
                    "message": "database error: disk full"
                }],
                "last": 1,
                "dropped": false,
                "boot": b.boot()
            })
        );
    }
}
