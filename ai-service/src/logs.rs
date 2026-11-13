//! Logging (spec 3.5, 6).
//!
//! Every event goes to stderr (the container log) through the operator's filter `XEXAM_AI_LOG` (default `info`), to
//! which `hyper`, `hyper_util`, `reqwest`, `h2` and `rustls` are always added at `warn` (and a second filter refuses
//! their lower levels whatever the operator wrote), so no transport detail can be logged.
//!
//! Events of this crate (target `xexam_ai_service*`) at info and above are also queued for the main System log and forwarded in
//! batches by [`run_forwarder`] (`POST /internal/ai/log`), whatever the operator's filter says. The queue keeps at most
//! 500 lines; when the backend is unreachable the oldest are dropped and counted. Admin audit lines never use this queue
//! (see `api::admin`): they are logged locally with the target `audit` and delivered synchronously or through the
//! persisted outbox.
use crate::link::{Link, LogLine};
use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

/// Lines kept in the forward queue.
pub const QUEUE_CAP: usize = 500;
/// Lines per forwarded batch.
pub const BATCH: usize = 50;
/// Longest forwarded message (characters).
pub const MAX_LINE_CHARS: usize = 2000;
/// Targets whose events below `warn` are never printed.
pub const QUIET_TARGETS: [&str; 5] = ["hyper", "hyper_util", "reqwest", "h2", "rustls"];

#[derive(Default)]
struct QueueState {
    lines: VecDeque<LogLine>,
    dropped: u64,
}

/// The queue of lines waiting to be forwarded to the main System log.
#[derive(Default)]
pub struct ForwardQueue {
    state: Mutex<QueueState>,
    notify: Notify,
}

impl ForwardQueue {
    fn lock(&self) -> std::sync::MutexGuard<'_, QueueState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds a line (the oldest is dropped when the queue is full).
    pub fn push(&self, level: &'static str, message: String) {
        let full = {
            let mut st = self.lock();
            if st.lines.len() >= QUEUE_CAP {
                st.lines.pop_front();
                st.dropped += 1;
            }
            st.lines.push_back(LogLine {
                level,
                message: crate::util::cut_chars(&crate::util::strip_ctrl(&message), MAX_LINE_CHARS),
            });
            st.lines.len() >= BATCH
        };
        if full {
            self.notify.notify_one();
        }
    }

    /// The next batch (without removing it): the dropped-lines notice first when lines were dropped.
    pub fn peek_batch(&self) -> (Vec<LogLine>, usize, u64) {
        let st = self.lock();
        let mut out = Vec::new();
        let dropped = st.dropped;
        if dropped > 0 {
            out.push(LogLine {
                level: "WARN",
                message: format!(
                    "{dropped} AI log lines were dropped while the main service was unreachable"
                ),
            });
        }
        let take = BATCH - out.len();
        let n = st.lines.len().min(take);
        out.extend(st.lines.iter().take(n).cloned());
        (out, n, dropped)
    }

    /// Removes `n` delivered lines and the reported drop count.
    pub fn confirm(&self, n: usize, dropped: u64) {
        let mut st = self.lock();
        for _ in 0..n {
            st.lines.pop_front();
        }
        st.dropped = st.dropped.saturating_sub(dropped);
    }

    pub fn len(&self) -> usize {
        self.lock().lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Waits until a batch is full.
    pub async fn wait_full(&self) {
        self.notify.notified().await
    }
}

/// The process-wide forward queue.
pub fn queue() -> &'static ForwardQueue {
    static Q: LazyLock<ForwardQueue> = LazyLock::new(ForwardQueue::default);
    &Q
}

struct MessageVisitor<'a>(&'a mut String);

impl Visit for MessageVisitor<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0.push_str(value);
        } else {
            let _ = write!(self.0, " {}={value}", field.name());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        } else {
            let _ = write!(self.0, " {}={value:?}", field.name());
        }
    }
}

/// Formats the message of an event (message, then other fields as ` key=value`).
pub fn event_message(event: &Event<'_>) -> String {
    let mut s = String::new();
    event.record(&mut MessageVisitor(&mut s));
    s
}

/// `true` for events forwarded to the main System log.
pub fn forwarded(target: &str, level: &Level) -> bool {
    target.starts_with("xexam_ai_service") && *level <= Level::INFO
}

/// Layer feeding [`queue`] (or another queue in tests).
pub struct ForwardLayer {
    queue: &'static ForwardQueue,
}

impl ForwardLayer {
    pub fn global() -> ForwardLayer {
        ForwardLayer { queue: queue() }
    }
}

impl<S: Subscriber> Layer<S> for ForwardLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        if !forwarded(meta.target(), meta.level()) {
            return;
        }
        let level = match *meta.level() {
            Level::ERROR => "ERROR",
            Level::WARN => "WARN",
            _ => "INFO",
        };
        self.queue.push(level, event_message(event));
    }
}

/// The operator's filter plus the forced `warn` directives. Unset, blank or invalid → `info` (invalid also returns
/// the reason, to be logged once).
pub fn env_filter(value: Option<&str>) -> (tracing_subscriber::EnvFilter, bool) {
    use tracing_subscriber::EnvFilter;
    let (mut f, invalid) = match value.map(str::trim).filter(|v| !v.is_empty()) {
        None => (EnvFilter::new("info"), false),
        Some(v) => match EnvFilter::try_new(v) {
            Ok(f) => (f, false),
            Err(_) => (EnvFilter::new("info"), true),
        },
    };
    for t in QUIET_TARGETS {
        if let Ok(d) = format!("{t}=warn").parse() {
            f = f.add_directive(d);
        }
    }
    (f, invalid)
}

/// `false` for transport-library events below `warn` (a second guard besides the directives).
pub fn not_quiet(meta: &tracing::Metadata<'_>) -> bool {
    let t = meta.target();
    let quiet = QUIET_TARGETS
        .iter()
        .any(|q| t == *q || t.strip_prefix(q).is_some_and(|r| r.starts_with("::")));
    !quiet || *meta.level() <= Level::WARN
}

/// Installs the global subscriber: stderr through the filters, plus the forward layer.
pub fn init(value: Option<&str>) {
    use tracing_subscriber::filter::{filter_fn, FilterExt};
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let (filter, invalid) = env_filter(value);
    let quiet = filter_fn(not_quiet);
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(std::io::stderr)
                .with_filter(filter.and(quiet)),
        )
        .with(ForwardLayer::global().with_filter(filter_fn(|m| forwarded(m.target(), m.level()))))
        .try_init();
    if invalid {
        tracing::warn!("XEXAM_AI_LOG is not a valid log filter; using the default \"info\"");
    }
}

/// Forwards queued lines to the main System log every 2 s (sooner when a batch is full) until `stop` turns true.
pub async fn run_forwarder(
    link: Arc<Link>,
    q: &'static ForwardQueue,
    every: Duration,
    mut stop: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(every) => {}
            _ = q.wait_full() => {}
            _ = stop.changed() => {}
        }
        flush_once(&link, q).await;
        if *stop.borrow() {
            return;
        }
    }
}

/// Sends the queued lines in batches while the backend accepts them.
pub async fn flush_once(link: &Link, q: &ForwardQueue) {
    loop {
        let (batch, n, dropped) = q.peek_batch();
        if batch.is_empty() {
            return;
        }
        if link.log(&batch).await.is_err() {
            return;
        }
        q.confirm(n, dropped);
        if n == 0 {
            return;
        }
    }
}

/// At most one line a minute (the refused-identity line).
pub struct Throttle {
    last: Mutex<Option<Instant>>,
    every: Duration,
}

impl Throttle {
    pub fn new(every: Duration) -> Throttle {
        Throttle {
            last: Mutex::new(None),
            every,
        }
    }

    /// `true` when a line may be written now.
    pub fn allow(&self) -> bool {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if last.is_some_and(|t| now.saturating_duration_since(t) < self.every) {
            return false;
        }
        *last = Some(now);
        true
    }
}

/// Chat failure lines: at most one per code per minute; the others are counted and summed up later.
pub struct FailureLog {
    state: Mutex<(HashMap<&'static str, Instant>, u64)>,
    /// Withheld hints: when the last line was written, and how many were withheld since
    leaks: Mutex<(Option<Instant>, u64)>,
}

impl Default for FailureLog {
    fn default() -> Self {
        FailureLog {
            state: Mutex::new((HashMap::new(), 0)),
            leaks: Mutex::new((None, 0)),
        }
    }
}

impl FailureLog {
    /// Logs or counts one failure.
    pub fn note(&self, username: &str, code: &'static str, http: Option<u16>) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let due =
            st.0.get(code)
                .is_none_or(|t| now.saturating_duration_since(*t) >= Duration::from_secs(60));
        if due {
            st.0.insert(code, now);
            let status = http.map_or_else(|| "-".to_string(), |s| s.to_string());
            tracing::error!("AI call failed for user {username:?}: {code} (HTTP {status})");
        } else {
            st.1 += 1;
        }
    }

    /// A hint was withheld because it gave the answer away: one line a minute, the others are counted. Never any text.
    pub fn note_leak(&self, username: &str) {
        let mut st = self.leaks.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if st
            .0
            .is_none_or(|t| now.saturating_duration_since(t) >= Duration::from_secs(60))
        {
            st.0 = Some(now);
            tracing::warn!("AI hint withheld because it gave the answer away (user {username:?})");
        } else {
            st.1 += 1;
        }
    }

    /// Writes the summary line for the failures counted since the last report (called once a minute).
    pub fn flush(&self) {
        {
            let mut st = self.leaks.lock().unwrap_or_else(|e| e.into_inner());
            let n = std::mem::take(&mut st.1);
            if n > 0 {
                tracing::warn!("{n} more AI hints were withheld since the last report");
            }
        }
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let n = std::mem::take(&mut st.1);
        if n > 0 {
            tracing::error!("{n} more AI call failures since the last report");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_drops_oldest_and_reports() {
        let q = ForwardQueue::default();
        for i in 0..(QUEUE_CAP + 3) {
            q.push("INFO", format!("line {i}"));
        }
        assert_eq!(q.len(), QUEUE_CAP);
        let (batch, n, dropped) = q.peek_batch();
        assert_eq!(dropped, 3);
        assert_eq!(n, BATCH - 1);
        assert_eq!(batch.len(), BATCH);
        assert_eq!(
            batch[0].message,
            "3 AI log lines were dropped while the main service was unreachable"
        );
        assert_eq!(batch[1].message, "line 3");
        q.confirm(n, dropped);
        let (batch, _, dropped) = q.peek_batch();
        assert_eq!(dropped, 0);
        assert_eq!(batch[0].message, format!("line {}", 3 + BATCH - 1));
    }

    #[test]
    fn control_characters_are_stripped() {
        let q = ForwardQueue::default();
        q.push("INFO", "a\u{1b}[31mb\nc".to_string());
        assert_eq!(q.peek_batch().0[0].message, "a[31mbc");
    }

    #[test]
    fn transport_targets_stay_at_warn_even_with_trace() {
        use tracing_subscriber::layer::SubscriberExt;
        let (f, invalid) = env_filter(Some("trace"));
        assert!(!invalid);
        let sub = tracing_subscriber::registry().with(f);
        tracing::subscriber::with_default(sub, || {
            assert!(!tracing::enabled!(target: "hyper", tracing::Level::DEBUG));
            assert!(!tracing::enabled!(target: "reqwest", tracing::Level::INFO));
            assert!(!tracing::enabled!(target: "rustls", tracing::Level::TRACE));
            assert!(!tracing::enabled!(target: "h2", tracing::Level::DEBUG));
            assert!(!tracing::enabled!(target: "hyper_util", tracing::Level::DEBUG));
            assert!(tracing::enabled!(target: "hyper", tracing::Level::WARN));
            assert!(tracing::enabled!(target: "xexam_ai_service::chat", tracing::Level::TRACE));
        });
        let (f, _) = env_filter(Some("hyper=trace,reqwest=debug"));
        let sub = tracing_subscriber::registry().with(f);
        tracing::subscriber::with_default(sub, || {
            assert!(!tracing::enabled!(target: "hyper", tracing::Level::DEBUG));
            assert!(!tracing::enabled!(target: "reqwest", tracing::Level::DEBUG));
        });
        let (_, invalid) = env_filter(Some("=[bad"));
        assert!(invalid);
    }

    #[test]
    fn quiet_guard() {
        static META_FILTER_CHECKS: &[(&str, bool)] =
            &[("hyper", true), ("hyper::client", true), ("hyperx", false)];
        for (t, quiet) in META_FILTER_CHECKS {
            let is_quiet = QUIET_TARGETS
                .iter()
                .any(|q| *t == *q || t.strip_prefix(q).is_some_and(|r| r.starts_with("::")));
            assert_eq!(is_quiet, *quiet, "{t}");
        }
        assert!(forwarded("xexam_ai_service::api", &Level::INFO));
        assert!(forwarded("xexam_ai_service", &Level::ERROR));
        assert!(!forwarded("xexam_ai_service::api", &Level::DEBUG));
        assert!(!forwarded("audit", &Level::INFO));
        assert!(!forwarded("hyper", &Level::ERROR));
    }

    #[test]
    fn throttle_once_a_minute() {
        let t = Throttle::new(Duration::from_secs(60));
        assert!(t.allow());
        assert!(!t.allow());
    }
}
