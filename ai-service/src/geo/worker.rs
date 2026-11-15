//! The background task that serves the main backend's lookup queue (spec 4.9).
//!
//! The backend never calls this service; this task pulls: it long-polls `POST /internal/ai/geo/next`, looks the handed-out
//! addresses up (up to 4 at a time) with the saved settings, and posts one `geo/result` call per batch. It runs whether
//! AI+ is on or off and never touches the AI quota or the AI usage statistics.
use super::{lookup_saved, template, FailureCode, GeoPlace};
use crate::app::{AppState, Knobs};
use crate::link::{GeoJob, GeoOutcome, LinkError};
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// Lookups that run at the same time.
const PARALLEL: usize = 4;
/// Pause after an older backend answered 404 (it has no `geo` routes).
const REJECTED_PAUSE: Duration = Duration::from_secs(60);
/// Pause after any other failed call.
const FAILED_PAUSE: Duration = Duration::from_secs(5);
/// An empty answer that came back sooner than this is not a long poll: wait a little before asking again.
const QUICK_EMPTY: Duration = Duration::from_millis(200);
const QUICK_EMPTY_PAUSE: Duration = Duration::from_secs(1);

/// Starts the worker. It ends when `stop` turns `true` (or its sender is gone).
pub fn spawn(app: AppState, stop: watch::Receiver<bool>) -> JoinHandle<()> {
    tokio::spawn(run(app, stop))
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
    loop {
        if *stop.borrow() {
            return;
        }
        app.0.geo.flush_log(crate::util::unix_now());
        if !app.0.link.has_token() {
            app.0.link.wait_for_token(stop.clone()).await;
            if !app.0.link.has_token() && !nap(&mut stop, FAILED_PAUSE).await {
                return;
            }
            continue;
        }
        let started = Instant::now();
        let wait = Knobs::get(&app.0.knobs.geo_wait_ms);
        let polled = tokio::select! {
            r = app.0.link.geo_next(wait) => r,
            _ = stop.changed() => return,
        };
        let pause = match polled {
            Ok(jobs) if jobs.is_empty() => {
                (started.elapsed() < QUICK_EMPTY).then_some(QUICK_EMPTY_PAUSE)
            }
            Ok(jobs) => {
                serve_batch(&app, jobs).await;
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

/// The host to name in a failure line: the saved template's host, or `(none)`.
fn host_for_log(app: &AppState) -> String {
    app.0
        .geo
        .config()
        .map(|row| template::host_of(&row.cfg.url_template))
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "(none)".to_string())
}

/// Looks one batch up and posts the results.
async fn serve_batch(app: &AppState, jobs: Vec<GeoJob>) {
    let mut outcomes: Vec<GeoOutcome> = Vec::with_capacity(jobs.len());
    for chunk in jobs.chunks(PARALLEL) {
        let done = futures_util::future::join_all(chunk.iter().map(|job| async move {
            let r = lookup_saved(&app.0, job.ip).await;
            (job.id.clone(), r)
        }))
        .await;
        for (id, r) in done {
            let result: Result<GeoPlace, FailureCode> = match r {
                Ok(place) => Ok(place),
                Err(f) => {
                    app.0
                        .geo
                        .log_failure(&host_for_log(app), f.code, crate::util::unix_now());
                    Err(f.code)
                }
            };
            outcomes.push(GeoOutcome { id, result });
        }
    }
    if outcomes.is_empty() {
        return;
    }
    // A result post that fails is dropped: the backend forgets the job after 60 s
    let _ = app.0.link.geo_result(&outcomes).await;
}
