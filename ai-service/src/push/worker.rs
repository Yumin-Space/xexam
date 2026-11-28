//! The background task that serves the main backend's web push queue.
//!
//! It long-polls `POST /internal/ai/push/next`, delivers each handed-out message to its push service (at most 4 requests
//! at a time and at most 2 per host) and posts one `push/result` call per batch; the backend decides what follows from
//! each outcome. It runs whether AI+ is on or off and never touches the AI quota or the AI usage statistics.
//!
//! Secrets: the endpoint, the authorization and the body of a job are never logged. A line names the job id, the host
//! and the status only, plus one summary line a minute.
use super::allow;
use super::client::{self, Delivery};
use crate::app::{AppState, Knobs};
use crate::link::{LinkError, PushJob, PushOutcome, PushResult, PUSH_BATCH_MAX};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{watch, Semaphore};
use tokio::task::JoinHandle;

/// Requests that run at the same time.
pub const PARALLEL: usize = 4;
/// Requests to one host that run at the same time.
pub const PER_HOST: usize = 2;
/// Pause after an older backend answered 404 (it has no `push` routes).
const REJECTED_PAUSE: Duration = Duration::from_secs(60);
/// Pause after any other failed call.
const FAILED_PAUSE: Duration = Duration::from_secs(5);
/// An empty answer that came back sooner than this is not a long poll: wait a little before asking again.
const QUICK_EMPTY: Duration = Duration::from_millis(200);
const QUICK_EMPTY_PAUSE: Duration = Duration::from_secs(1);
/// How often the summary line is written (when there was anything to report).
const SUMMARY_EVERY: Duration = Duration::from_secs(60);
/// The first wait after a 429 without `Retry-After`, in seconds; it doubles per host up to [`MAX_WAIT`].
const FIRST_WAIT: u32 = 60;
const MAX_WAIT: u32 = 3600;

/// Starts the courier. It ends when `stop` turns `true` (or its sender is gone).
pub fn spawn(app: AppState, stop: watch::Receiver<bool>) -> JoinHandle<()> {
    tokio::spawn(run(app, stop))
}

/// Counts of the last minute.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub sent: u32,
    pub failed: u32,
    pub retried: u32,
}

impl Stats {
    fn add(&mut self, o: PushOutcome) {
        match o {
            PushOutcome::Sent => self.sent += 1,
            PushOutcome::Gone | PushOutcome::Auth | PushOutcome::Rejected => self.failed += 1,
            PushOutcome::Retry | PushOutcome::Deferred => self.retried += 1,
        }
    }

    /// The summary line, or `None` when nothing happened.
    pub fn line(&self) -> Option<String> {
        (*self != Stats::default()).then(|| {
            format!(
                "push: sent {}, failed {}, retried {} in the last minute",
                self.sent, self.failed, self.retried
            )
        })
    }
}

/// What the courier remembers between batches (memory only).
pub struct Courier {
    client: reqwest::Client,
    /// Per host: the wait of the last 429 that came without `Retry-After` (it doubles while that goes on)
    escalation: Mutex<HashMap<String, u32>>,
    stats: Mutex<Stats>,
}

impl Courier {
    pub fn new() -> Result<Courier, String> {
        Ok(Courier {
            client: client::build_client()?,
            escalation: Mutex::new(HashMap::new()),
            stats: Mutex::new(Stats::default()),
        })
    }

    /// Takes the counts of the last minute.
    pub fn take_stats(&self) -> Stats {
        std::mem::take(&mut *self.stats.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// The wait for a 429 that carries no `Retry-After`: 60 s, doubling per host while it keeps happening.
    fn next_wait(&self, host: &str) -> u32 {
        let mut m = self.escalation.lock().unwrap_or_else(|e| e.into_inner());
        let next = m
            .get(host)
            .map_or(FIRST_WAIT, |w| w.saturating_mul(2).min(MAX_WAIT));
        m.insert(host.to_string(), next);
        next
    }

    fn calm(&self, host: &str) {
        self.escalation
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(host);
    }
}

/// Sleeps `d`; `false` when the service is stopping.
async fn nap(stop: &mut watch::Receiver<bool>, d: Duration) -> bool {
    if *stop.borrow() {
        return false;
    }
    tokio::select! {
        _ = tokio::time::sleep(d) => !*stop.borrow(),
        _ = stop.changed() => false,
    }
}

async fn run(app: AppState, mut stop: watch::Receiver<bool>) {
    let courier = match Courier::new() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("{e}");
            return;
        }
    };
    let mut last_summary = Instant::now();
    loop {
        if *stop.borrow() {
            return;
        }
        if last_summary.elapsed() >= SUMMARY_EVERY {
            last_summary = Instant::now();
            if let Some(line) = courier.take_stats().line() {
                tracing::info!("{line}");
            }
        }
        if !app.0.link.has_token() {
            app.0.link.wait_for_token(stop.clone()).await;
            if !app.0.link.has_token() && !nap(&mut stop, FAILED_PAUSE).await {
                return;
            }
            continue;
        }
        let started = Instant::now();
        let wait = Knobs::get(&app.0.knobs.push_wait_ms);
        let polled = tokio::select! {
            r = app.0.link.push_next(wait) => r,
            _ = stop.changed() => return,
        };
        let pause = match polled {
            Ok(jobs) if jobs.is_empty() => {
                (started.elapsed() < QUICK_EMPTY).then_some(QUICK_EMPTY_PAUSE)
            }
            Ok(jobs) if jobs.len() > PUSH_BATCH_MAX => {
                // Refused whole: nothing is sent and nothing is reported, the leases run out
                tracing::warn!(
                    "push: refused a batch of {} jobs (at most {PUSH_BATCH_MAX} are allowed)",
                    jobs.len()
                );
                Some(QUICK_EMPTY_PAUSE)
            }
            Ok(jobs) => {
                let target = *app
                    .0
                    .knobs
                    .push_target
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let results = serve_batch(&courier, jobs, target).await;
                // A result post that fails is dropped: the leases run out and the backend hands the jobs out again
                if !results.is_empty() {
                    let _ = app.0.link.push_result(&results).await;
                }
                None
            }
            Err(LinkError::Rejected) => Some(REJECTED_PAUSE),
            Err(_) => Some(FAILED_PAUSE),
        };
        if let Some(d) = pause {
            if !nap(&mut stop, d).await {
                return;
            }
        }
    }
}

/// Delivers one batch (at most 20 jobs) and returns one result per job, in order.
pub async fn serve_batch(
    courier: &Courier,
    jobs: Vec<PushJob>,
    target: Option<std::net::SocketAddr>,
) -> Vec<PushResult> {
    let global = Arc::new(Semaphore::new(PARALLEL));
    let mut per_host: HashMap<String, Arc<Semaphore>> = HashMap::new();
    for j in &jobs {
        if let Some(h) = allow::allowed_host(&j.endpoint) {
            per_host
                .entry(h)
                .or_insert_with(|| Arc::new(Semaphore::new(PER_HOST)));
        }
    }
    // Hosts that answered 429 in this batch: the rest of their jobs wait
    let held: Mutex<HashMap<String, u32>> = Mutex::new(HashMap::new());
    let results = futures_util::future::join_all(jobs.iter().map(|job| {
        let global = global.clone();
        let host_sem = allow::allowed_host(&job.endpoint)
            .and_then(|h| per_host.get(&h).cloned().map(|s| (h, s)));
        let held = &held;
        async move {
            let Some((host, host_sem)) = host_sem else {
                return result(job, Delivery::from_outcome(PushOutcome::Rejected));
            };
            // The host's place first, then a global one, so a slow host never takes places it cannot use
            let _h = host_sem.acquire().await;
            let _g = global.acquire().await;
            let wait = held
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&host)
                .copied();
            if let Some(secs) = wait {
                return result(
                    job,
                    Delivery {
                        outcome: PushOutcome::Deferred,
                        status: None,
                        retry_after: Some(secs),
                    },
                );
            }
            let mut d = client::deliver(&courier.client, job, target).await;
            match d.outcome {
                PushOutcome::Sent => courier.calm(&host),
                PushOutcome::Retry if d.status == Some(429) => {
                    let secs = match d.retry_after {
                        Some(s) => {
                            courier.calm(&host);
                            s
                        }
                        None => courier.next_wait(&host),
                    };
                    d.retry_after = Some(secs);
                    held.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(host.clone(), secs);
                }
                _ => {}
            }
            tracing::debug!(
                "push: job {} to {host}: {} (status {})",
                job.id,
                d.outcome.as_str(),
                d.status.map_or("none".to_string(), |s| s.to_string())
            );
            result(job, d)
        }
    }))
    .await;
    let mut stats = courier.stats.lock().unwrap_or_else(|e| e.into_inner());
    for r in &results {
        stats.add(r.outcome);
    }
    results
}

fn result(job: &PushJob, d: Delivery) -> PushResult {
    PushResult {
        id: job.id,
        tries: job.tries,
        outcome: d.outcome,
        status: d.status,
        retry_after: d.retry_after,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_line_and_the_counts() {
        let mut s = Stats::default();
        assert_eq!(s.line(), None);
        for o in [
            PushOutcome::Sent,
            PushOutcome::Sent,
            PushOutcome::Gone,
            PushOutcome::Auth,
            PushOutcome::Rejected,
            PushOutcome::Retry,
            PushOutcome::Deferred,
        ] {
            s.add(o);
        }
        assert_eq!(
            s.line().as_deref(),
            Some("push: sent 2, failed 3, retried 2 in the last minute")
        );
    }

    #[test]
    fn a_429_without_retry_after_doubles_per_host() {
        let c = Courier::new().expect("client");
        assert_eq!(c.next_wait("a.example"), 60);
        assert_eq!(c.next_wait("a.example"), 120);
        assert_eq!(c.next_wait("b.example"), 60);
        for _ in 0..10 {
            c.next_wait("a.example");
        }
        assert_eq!(c.next_wait("a.example"), 3600);
        c.calm("a.example");
        assert_eq!(c.next_wait("a.example"), 60);
    }
}
