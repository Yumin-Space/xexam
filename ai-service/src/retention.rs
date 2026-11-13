//! Background work: retention sweep (7.1), reconcile with the main backend (7.3), audit outbox delivery (6.2), the
//! coalesced WAL checkpoint (2.2), log forwarding (6.1), the token watch (3.6) and the state reports to the main
//! backend (round-2 contracts 2.6).
use crate::app::AppState;
use crate::link::{BackendUser, LinkError};
use crate::store::{data, stats, tables};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// Chats deleted per write transaction by the retention sweep. Study chats can hold hundreds of messages each, so a
/// batch of 20 stays near 0.2 s in the worst case (measured).
pub const SWEEP_BATCH: i64 = 20;
/// Pause between two full batches: the store mutex is not fair, so without it a burst sweep would keep every other
/// writer (answer flushes, asks, finishes) waiting for the whole sweep.
pub const SWEEP_PAUSE: Duration = Duration::from_millis(20);

/// Deletes the chats last updated before `cutoff` (except the running ones) in batches of [`SWEEP_BATCH`], releasing the
/// store and sleeping `pause` after each full batch; returns the number deleted. Blocking: call it off the async
/// workers.
pub fn sweep_batches(
    store: &crate::store::Store,
    cutoff: i64,
    running: &[i64],
    pause: Duration,
) -> usize {
    let mut chats = 0usize;
    loop {
        match store.write(|tx| stats::sweep_chats(tx, cutoff, running, SWEEP_BATCH)) {
            Ok(n) => {
                chats += n;
                if n < SWEEP_BATCH as usize {
                    break;
                }
                std::thread::sleep(pause);
            }
            Err(e) => {
                tracing::warn!("AI retention failed: {e}");
                break;
            }
        }
    }
    chats
}

/// Deletes chats older than the retention period (except running ones) and old finished tasks; returns the number of
/// chats deleted.
pub fn sweep_once(app: &AppState) -> usize {
    let now = crate::util::unix_now();
    let days = match app.0.store.read(tables::load_settings) {
        Ok(s) => s.retention_days,
        Err(e) => {
            tracing::warn!("AI retention: cannot read the settings: {e}");
            return 0;
        }
    };
    let running = app.0.registry.running_chats();
    let cutoff = now - days * 86_400;
    // Batches with a pause between them, so one sweep never keeps the write lock for long
    let chats = sweep_batches(&app.0.store, cutoff, &running, SWEEP_PAUSE);
    let tasks = match app
        .0
        .store
        .write(|tx| stats::sweep_tasks(tx, now - 30 * 86_400))
    {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("AI retention failed: {e}");
            0
        }
    };
    if chats > 0 {
        tracing::info!("AI retention: deleted {chats} chats older than {days} days");
    }
    if chats > 0 || tasks > 0 {
        app.0.store.schedule_checkpoint();
    }
    chats
}

/// Compares the AI users with the main database: users that no longer exist or whose stamp differs lose their AI data
/// (running answers are stopped and awaited first); renamed users are refreshed. `extra` ids are asked for too (their
/// rows are returned but nothing is deleted for them). Returns the backend's answer.
pub async fn reconcile_once(
    app: &AppState,
    holders: bool,
    extra: &[i64],
) -> Result<Vec<BackendUser>, LinkError> {
    let known = app
        .0
        .store
        .read(data::all_users)
        .map_err(|_| LinkError::Failed)?;
    let mut ids: Vec<i64> = known.iter().map(|(id, _, _)| *id).collect();
    for id in extra {
        if !ids.contains(id) {
            ids.push(*id);
        }
    }
    let answer = app.0.link.users(&ids, holders).await?;
    let by_id: HashMap<i64, &BackendUser> = answer.iter().map(|u| (u.id, u)).collect();
    let mut gone = 0usize;
    for (id, stamp, name) in &known {
        match by_id.get(id) {
            None => {
                if app.0.purge_user(*id).await {
                    gone += 1;
                }
            }
            Some(u) if u.stamp != *stamp => {
                if app.0.purge_user(*id).await {
                    tracing::info!(
                        "Removed the AI data of a user id that now belongs to another account (id {id})"
                    );
                }
            }
            Some(u) if u.username != *name => {
                let _ = app
                    .0
                    .store
                    .write(|tx| data::rename_user(tx, *id, &u.username));
            }
            Some(_) => {}
        }
    }
    if gone > 0 {
        tracing::info!("Removed the AI data of {gone} deleted users");
    }
    Ok(answer)
}

/// The state reports: once the internal token is loaded (this task also waits for a missing token), a report at once,
/// then the next one after `heartbeat_every` (after `heartbeat_retry` when it failed) or at once when notified; ends
/// on shutdown (the caller sends the stopping report). No log lines.
pub async fn heartbeat_loop(app: AppState, mut stop: watch::Receiver<bool>) {
    app.0.link.wait_for_token(stop.clone()).await;
    let signal = app.0.heartbeat_signal();
    let t = app.0.timeouts;
    loop {
        if *stop.borrow() {
            return;
        }
        let wait = if app.0.heartbeat_now().await {
            t.heartbeat_every
        } else {
            t.heartbeat_retry
        };
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = signal.notified() => {}
            _ = stop.wait_for(|v| *v) => return,
        }
    }
}

/// Starts every background task; they end when the shutdown signal is given (the caller aborts them as well).
pub fn spawn_all(app: &AppState) -> Vec<JoinHandle<()>> {
    let mut tasks = Vec::new();

    // Coalesced checkpoint: at most one run per 10 s
    let a = app.clone();
    tasks.push(tokio::spawn(async move {
        let signal = a.0.store.checkpoint_signal();
        loop {
            signal.notified().await;
            match a.0.store.checkpoint_now() {
                Ok(true) => signal.notify_one(),
                Ok(false) => {}
                Err(e) => tracing::warn!("AI database checkpoint failed: {e}"),
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }));

    // Jobs that were saving a bank when the service stopped: the backend is asked whether the bank exists
    let a = app.clone();
    tasks.push(tokio::spawn(async move {
        a.0.link.wait_for_token(a.0.shutdown.clone()).await;
        crate::gen::runner::settle_saving(&a).await;
    }));

    // Retention: 60 s after start, then every hour
    let a = app.clone();
    tasks.push(tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let mut every = tokio::time::interval(Duration::from_secs(3600));
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            every.tick().await;
            let b = a.clone();
            let _ = tokio::task::spawn_blocking(move || sweep_once(&b)).await;
        }
    }));

    // Reconcile: at start (every 30 s until it works), then every 10 minutes
    let a = app.clone();
    tasks.push(tokio::spawn(async move {
        loop {
            if reconcile_once(&a, false, &[]).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
        loop {
            tokio::time::sleep(Duration::from_secs(600)).await;
            let _ = reconcile_once(&a, false, &[]).await;
        }
    }));

    // Audit outbox: at start, then every 30 s
    let a = app.clone();
    tasks.push(tokio::spawn(async move {
        loop {
            a.0.deliver_outbox().await;
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }));

    // Summary line of rate-limited chat failures, once a minute
    let a = app.clone();
    tasks.push(tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_secs(60));
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            every.tick().await;
            a.0.failures.flush();
        }
    }));

    // The internal token while it is missing, then the state reports to the main backend
    let a = app.clone();
    let stop = app.0.shutdown.clone();
    tasks.push(tokio::spawn(heartbeat_loop(a, stop)));

    // Log forwarding
    let link = app.0.link.clone();
    let stop = app.0.shutdown.clone();
    let every = app.0.timeouts.flush_interval;
    tasks.push(tokio::spawn(async move {
        crate::logs::run_forwarder(link, crate::logs::queue(), every, stop).await;
    }));
    tasks
}
