//! The async side of generation and import jobs (spec GJ-3): the job registry, the shared batch driver, the generation
//! runner, the restart repair and the quota / status helpers. The pure state machine is `gen::run`; everything here
//! talks to the AI database, the provider and the main backend.
//!
//! Rules kept here: a job holds one of `genRunning` job places for its whole writing phase; every model call also takes
//! a place of the global gate but never the last free one (`try_acquire_spare`); every sent call writes one `gen_calls`
//! row (never a `usage` row, so the per-user caps cannot see a generation); a cancel or a purge drops the in-flight
//! request; nothing is created unless every slot is valid; no failure is charged.
use crate::app::{AppState, Knobs};
use crate::chat::GatePermit;
use crate::features::Feature;
use crate::gen::assemble;
use crate::gen::dedup::Deduper;
use crate::gen::plan::{per_call, Plan};
use crate::gen::prompt::Prompt;
use crate::gen::run::{FailCode, JobLimits, Next, Outcome, Run};
use crate::gen::whole as whole_gen;
use crate::gen::{SeedFull, Tri};
use crate::identity::Identity;
use crate::pace::{self, PaceCfg};
use crate::provider::{self, Role, UpCode};
use crate::quota::Day;
use crate::store::jobs::{self, End};
use crate::store::stats::{self, GenCallRow};
use crate::util::unix_now;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::Instant;

/// Total timeout of one model call of a job (GJ-3).
pub const CALL_TOTAL_SECS: u64 = 300;

/// Why a job is being stopped from outside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopWhy {
    /// The user cancelled (the cancel route already wrote the end state)
    Cancel,
    /// The user's AI data is being removed: stop silently, write nothing more
    Purged,
    /// An import is cancelled with "keep what is finished" (A-OWNER-66.4): the cancel route already set the phase
    /// `saving`; the runner stops asking and saves the finished questions as a partial bank
    Keep,
}

struct Entry {
    user: i64,
    tx: watch::Sender<Option<StopWhy>>,
    /// An import waiting for its confirm is woken here
    confirm: std::sync::Arc<tokio::sync::Notify>,
    /// A model call of this job was sent (its row is written when it ends): a cancel then keeps the quota unit
    sent: std::sync::atomic::AtomicBool,
}

/// The jobs of this process (queued, waiting for a place, running): their cancel signals.
#[derive(Default)]
pub struct Jobs {
    map: Mutex<HashMap<i64, Entry>>,
}

/// Removes the registration when dropped.
pub struct Registration {
    app: AppState,
    task: i64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.app.0.jobs.remove(self.task);
    }
}

impl Jobs {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Entry>> {
        self.map.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Registers a job and gives its cancel receiver.
    pub fn register(
        &self,
        app: &AppState,
        task: i64,
        user: i64,
    ) -> (watch::Receiver<Option<StopWhy>>, Registration) {
        let (tx, rx) = watch::channel(None);
        self.lock().insert(
            task,
            Entry {
                user,
                tx,
                confirm: std::sync::Arc::new(tokio::sync::Notify::new()),
                sent: std::sync::atomic::AtomicBool::new(false),
            },
        );
        (
            rx,
            Registration {
                app: app.clone(),
                task,
            },
        )
    }

    /// Notes that a model call of the job was sent.
    pub fn note_sent(&self, task: i64) {
        if let Some(e) = self.lock().get(&task) {
            e.sent.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// `true` when a model call of the job was sent (finished or still in flight).
    pub fn has_sent(&self, task: i64) -> bool {
        self.lock()
            .get(&task)
            .is_some_and(|e| e.sent.load(std::sync::atomic::Ordering::SeqCst))
    }

    /// The confirm signal of a registered import.
    pub fn confirm_signal(&self, task: i64) -> Option<std::sync::Arc<tokio::sync::Notify>> {
        self.lock().get(&task).map(|e| e.confirm.clone())
    }

    /// Wakes an import that waits for its confirm; `false` when this process does not run it.
    pub fn notify_confirm(&self, task: i64) -> bool {
        match self.confirm_signal(task) {
            Some(n) => {
                n.notify_one();
                true
            }
            None => false,
        }
    }

    fn remove(&self, task: i64) {
        self.lock().remove(&task);
    }

    pub fn is_registered(&self, task: i64) -> bool {
        self.lock().contains_key(&task)
    }

    /// Signals one job; `false` when this process does not run it.
    pub fn cancel(&self, task: i64, why: StopWhy) -> bool {
        match self.lock().get(&task) {
            Some(e) => {
                let _ = e.tx.send(Some(why));
                true
            }
            None => false,
        }
    }

    /// Signals every job of a user and waits (at most 10 s) until their runners are gone.
    pub async fn cancel_user(&self, user: i64, why: StopWhy) {
        let ids: Vec<i64> = {
            let m = self.lock();
            m.iter()
                .filter(|(_, e)| e.user == user)
                .map(|(id, e)| {
                    let _ = e.tx.send(Some(why));
                    *id
                })
                .collect()
        };
        for _ in 0..500 {
            if ids.iter().all(|id| !self.is_registered(*id)) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }
}

/// Resolves when a stop signal arrives (the guard of the watch is dropped here, so the caller's future stays `Send`).
pub async fn cancelled(rx: &mut watch::Receiver<Option<StopWhy>>) {
    let _ = rx.wait_for(|v| v.is_some()).await;
}

/// Resolves when the process starts shutting down.
pub async fn shut(rx: &mut watch::Receiver<bool>) {
    let _ = rx.wait_for(|v| *v).await;
}

// —— quota and status ————————————————————————————————————————————————————————————————————————————————————————————————

/// What the quota says about a user right now (GJ-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quota {
    pub used: i64,
    pub limit: i64,
    pub unlimited: bool,
    pub day: Day,
}

impl Quota {
    pub fn left(&self) -> i64 {
        (self.limit - self.used).max(0)
    }

    pub fn exhausted(&self) -> bool {
        !self.unlimited && self.used >= self.limit
    }
}

/// The user's units today (the day a unit was charged), with the limits as they are now.
pub fn quota_of(
    c: &rusqlite::Connection,
    app: &crate::app::App,
    who: &Identity,
    now: i64,
) -> rusqlite::Result<Quota> {
    let l = app.limits();
    let day = crate::util::day_bounds(now);
    Ok(Quota {
        used: jobs::quota_used(c, who.id, day.start)?,
        limit: l.gen_per_day,
        unlimited: who.is_admin() && l.gen_admins_exempt == 1,
        day,
    })
}

/// The `gen` object of `GET /api/ai/status` (API-U1, P-30): present whenever the user has the AI+ service.
pub fn gen_status(app: &AppState, who: &Identity, now: i64) -> rusqlite::Result<Value> {
    let l = app.0.limits();
    let (q, job, unseen) = app.0.store.read(|c| {
        Ok((
            quota_of(c, &app.0, who, now)?,
            jobs::active_for_user(c, who.id)?,
            jobs::unseen_for_user(c, who.id, 3)?,
        ))
    })?;
    Ok(json!({
        "perDay": if q.unlimited { Value::Null } else { json!(q.limit) },
        "used": q.used,
        "left": if q.unlimited { Value::Null } else { json!(q.left()) },
        "unlimited": q.unlimited,
        "resetAt": q.day.next,
        "minSource": l.gen_min_source,
        "minWrong": l.gen_min_wrong,
        "bank": {"single": l.gen_bank_single, "multi": l.gen_bank_multi},
        "wrong": {"single": l.gen_wrong_single, "multi": l.gen_wrong_multi},
        "job": job.map_or(Value::Null, |j| j.to_json()),
        "unseen": unseen.iter().map(|j| j.to_json()).collect::<Vec<_>>(),
        "import": {
            "maxMb": l.import_max_mb,
            "maxQuestions": l.import_max_questions,
            "types": ["pdf", "docx", "xlsx", "pptx", "md", "txt", "csv"],
        },
    }))
}

// —— the shared driver ———————————————————————————————————————————————————————————————————————————————————————————————

/// Why a driver or a runner step stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// Stopped from outside (cancel, purge, shutdown): write nothing more
    Cancelled(StopWhy),
    /// The process is shutting down: leave the row for the restart repair
    Shutdown,
    /// The job fails with this code
    Fail(&'static str),
}

/// What a job's driver needs.
pub struct Ctx {
    pub app: AppState,
    pub task: i64,
    pub who: Identity,
    pub feature: Feature,
    /// `generate`, `recognise` or `import` (the `gen_calls.purpose`)
    pub purpose: &'static str,
    /// Where the job's calls go: resolved once at the start of the job and kept (the prompt cache is per provider)
    pub route: crate::route::Route,
    pub cancel: watch::Receiver<Option<StopWhy>>,
    pub deadline: Instant,
    /// The failure code when the deadline passes (`ai.gen.timeout` or `ai.import.readTimeout`)
    pub timeout_code: &'static str,
}

/// Options of one model call of a job (all default: today's behaviour).
#[derive(Debug, Clone, Copy, Default)]
pub struct CallOpts {
    /// Ask the Cloudflare AI Gateway to bypass its response cache (a repeated send)
    pub skip_cache: bool,
    /// `Some(false)` = no cache markers; `None` = markers when a prefix is long enough for the model's cache
    pub cache: Option<bool>,
    /// The `gen_calls.purpose` of this call when it is not the job's (`plan`)
    pub purpose: Option<&'static str>,
    /// The JSON Schema of the answer (structured output); `None` = free text
    pub schema: Option<&'static Value>,
}

impl CallOpts {
    /// The options of the `attempt`-th send of a batch (1 = the first).
    pub fn for_attempt(attempt: u32) -> CallOpts {
        CallOpts {
            skip_cache: attempt >= 2,
            schema: None,
            ..CallOpts::default()
        }
    }
}

/// The answer of one model call.
pub enum CallResult {
    Answer {
        text: String,
        truncated: bool,
        row: GenCallRow,
    },
    Error {
        code: UpCode,
        row: GenCallRow,
        hint: RetryHint,
    },
}

/// What a provider error says about trying again (a 429 only).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetryHint {
    /// How long the provider asked to wait, in milliseconds
    pub wait_ms: Option<u64>,
    /// The quota is a daily one: waiting does not help
    pub daily: bool,
}

impl Ctx {
    /// Cancel, shutdown and the deadline, in this order.
    pub fn check(&self) -> Result<(), Stop> {
        if let Some(why) = *self.cancel.borrow() {
            return Err(Stop::Cancelled(why));
        }
        if *self.app.0.shutdown.borrow() {
            return Err(Stop::Shutdown);
        }
        if Instant::now() >= self.deadline {
            return Err(Stop::Fail(self.timeout_code));
        }
        Ok(())
    }

    /// Sleeps `secs` (fractions allowed), checking cancel and the deadline every second.
    pub async fn sleep_checked(&self, secs: f64) -> Result<(), Stop> {
        let end = Instant::now() + Duration::from_secs_f64(secs.max(0.0));
        loop {
            self.check()?;
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(());
            }
            tokio::time::sleep(left.min(Duration::from_secs(1))).await;
        }
    }

    /// AI effective, the user not paused, the feature still on, the account still valid (GJ-3 step 2; M2). A lookup
    /// that fails does not stop the job (the create step decides).
    pub async fn recheck(&self) -> Result<(), Stop> {
        let app = &self.app;
        let on = app.0.effective().unwrap_or(false);
        if !on || app.0.suspended(&self.who) || !app.0.feature_allowed(&self.who, self.feature) {
            return Err(Stop::Fail("ai.gen.stopped"));
        }
        // The job keeps its route; when that connection turned broken or was removed the job stops (no mid-job switch)
        let route_ok = app.0.routes().is_ok_and(|v| {
            v.conn(self.route.connection_id)
                .is_some_and(|c| v.conn_usable(c))
        });
        if !route_ok {
            return Err(Stop::Fail("ai.gen.stopped"));
        }
        if let Ok(list) = app.0.link.users(&[self.who.id], false).await {
            match list.iter().find(|u| u.id == self.who.id) {
                Some(u) if u.stamp == self.who.stamp && !u.disabled && u.ai_allowed => {}
                _ => return Err(Stop::Fail("ai.gen.forbidden")),
            }
        }
        Ok(())
    }

    /// A model-call place of the global gate that leaves one free for an interactive call (when the gate has two or
    /// more places); polls every 2 s, checking cancel and the deadline.
    pub async fn place(&self) -> Result<GatePermit, Stop> {
        let keep = usize::from(self.app.0.gate.size() >= 2);
        loop {
            self.check()?;
            if let Some(p) = self.app.0.gate.try_acquire_spare(keep) {
                return Ok(p);
            }
            let ms = Knobs::get(&self.app.0.knobs.place_poll_ms).max(1);
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
    }

    /// Waits for the provider-rate pacing (v1.9): the calls of all jobs queue in order and each starts when the
    /// sliding window has room for it. `Ok(None)` = pacing is off. Holds no gate place while waiting; cancel, shutdown
    /// and the deadline end the wait.
    pub async fn pace(&self, est: i64) -> Result<Option<u64>, Stop> {
        let app = &self.app;
        let pacer = &app.0.pacer;
        let scale = Knobs::get(&app.0.knobs.pace_scale);
        if !PaceCfg::of(&app.0.limits(), scale).on {
            return Ok(None);
        }
        let mut cancel = self.cancel.clone();
        let mut shutdown = app.0.shutdown.clone();
        let turn = tokio::select! {
            g = pacer.turn.lock() => Some(g),
            _ = cancelled(&mut cancel) => None,
            _ = shut(&mut shutdown) => None,
            _ = tokio::time::sleep_until(self.deadline) => None,
        };
        let Some(_turn) = turn else {
            self.check()?;
            return Err(Stop::Fail(self.timeout_code));
        };
        let began = Instant::now();
        let mut mark: Option<pace::WaitMark> = None;
        loop {
            self.check()?;
            let scale = Knobs::get(&app.0.knobs.pace_scale);
            let cfg = PaceCfg::of(&app.0.limits(), scale);
            if !cfg.on {
                return Ok(None);
            }
            let now = Instant::now();
            match pacer.with_window(|w| w.admit(&cfg, est, now)) {
                Ok(a) => {
                    if a.oversize {
                        tracing::warn!(
                            "an AI call of about {est} tokens is above the paced token limit of {} a minute; it goes out alone",
                            cfg.tpm
                        );
                    }
                    return Ok(Some(a.id));
                }
                Err(at) => {
                    let left = at.saturating_duration_since(now);
                    let total = now.saturating_duration_since(began) + left;
                    if mark.is_none() && total.as_secs_f64() >= pace::SHOW_WAIT_SECS {
                        mark = Some(pacer.mark(app, self.task));
                    }
                    self.sleep_checked(left.min(Duration::from_secs(1)).as_secs_f64())
                        .await?;
                }
            }
        }
    }

    /// One paced model call of a job: wait for the pacing, take a place of the gate, send, give the place back. A 429
    /// waits the time the provider names and sends again (up to [`pace::MAX_LIMITED_RETRIES`] times, not counted as an
    /// error); a daily quota stops the job with `ai.gen.providerDaily`.
    pub async fn send(
        &self,
        system: &str,
        prefix: Option<&str>,
        user: &str,
        want: i64,
        asked: usize,
        opts: CallOpts,
    ) -> Result<CallResult, Stop> {
        let app = &self.app;
        let pacer = &app.0.pacer;
        let est = pace::estimate(system, prefix, user, want);
        let mut limited = 0u32;
        loop {
            let stamp = self.pace(est).await?;
            let permit = self.place().await?;
            if let Some(id) = stamp {
                pacer.with_window(|w| w.stamp(id, Instant::now()));
            }
            let r = self
                .call_opts(system, prefix, user, want, asked, opts)
                .await;
            drop(permit);
            let settle = |tokens: Option<i64>| {
                if let Some(id) = stamp {
                    pacer.with_window(|w| w.settle(id, tokens));
                }
            };
            let r = match r {
                Ok(r) => r,
                Err(e) => {
                    settle(None);
                    return Err(e);
                }
            };
            match r {
                CallResult::Answer { ref row, .. } => {
                    settle(Some(
                        row.tokens_in.unwrap_or(0) + row.tokens_out.unwrap_or(0),
                    ));
                    return Ok(r);
                }
                CallResult::Error {
                    code: UpCode::RateLimit,
                    row,
                    hint,
                } => {
                    settle(None);
                    if hint.daily {
                        if !self.write_call(&row, None, None) {
                            return Err(Stop::Cancelled(StopWhy::Purged));
                        }
                        return Err(Stop::Fail("ai.gen.providerDaily"));
                    }
                    if limited >= pace::MAX_LIMITED_RETRIES {
                        return Ok(CallResult::Error {
                            code: UpCode::RateLimit,
                            row,
                            hint,
                        });
                    }
                    limited += 1;
                    let secs = hint
                        .wait_ms
                        .map(|ms| ms.div_ceil(1000))
                        .unwrap_or_else(|| u64::from((20 * limited).min(60)))
                        .clamp(1, pace::MAX_RETRY_WAIT_SECS);
                    let scale = Knobs::get(&app.0.knobs.pace_scale);
                    let wait = secs as f64 * scale as f64 / 100.0;
                    if PaceCfg::of(&app.0.limits(), scale).on {
                        let until = Instant::now() + Duration::from_secs_f64(wait);
                        pacer.with_window(|w| w.pause_until(until));
                    }
                    if !self.write_call(&row, None, None) {
                        return Err(Stop::Cancelled(StopWhy::Purged));
                    }
                    tracing::info!(
                        "an AI job call hit the provider rate limit; waiting {secs} s (try {limited})"
                    );
                    let _mark = (wait >= pace::SHOW_WAIT_SECS).then(|| pacer.mark(app, self.task));
                    self.sleep_checked(wait).await?;
                }
                other => {
                    settle(None);
                    return Ok(other);
                }
            }
        }
    }

    /// Writes the `gen_calls` row and updates the task; `false` when the task is gone (a purge).
    pub fn write_call(&self, row: &GenCallRow, made: Option<i64>, phase: Option<&str>) -> bool {
        let now = unix_now();
        let r = self.app.0.store.write(|tx| -> rusqlite::Result<usize> {
            stats::insert_gen_call(tx, row)?;
            jobs::after_call(tx, self.task, made, phase, now)
        });
        match r {
            Ok(n) => n > 0,
            Err(e) => {
                tracing::warn!("could not write an AI job call row: {e}");
                true
            }
        }
    }

    /// Like [`Ctx::write_call`], and the accepted questions of the call (`(slot number, data)`) go to `gen_items` in the
    /// same transaction, so the job's owner can read them while the job runs.
    pub fn write_call_items(
        &self,
        row: &GenCallRow,
        made: Option<i64>,
        phase: Option<&str>,
        items: &[(i64, String)],
    ) -> bool {
        let now = unix_now();
        let r = self.app.0.store.write(|tx| -> rusqlite::Result<usize> {
            stats::insert_gen_call(tx, row)?;
            let n = jobs::after_call(tx, self.task, made, phase, now)?;
            if n > 0 {
                for (slot, data) in items {
                    crate::store::items::items_add(tx, self.task, *slot, data, now)?;
                }
            }
            Ok(n)
        });
        match r {
            Ok(n) => n > 0,
            Err(e) => {
                tracing::warn!("could not write an AI job call row: {e}");
                true
            }
        }
    }

    /// Sends one model call: `system` / `user` text and `want` output tokens; asks `asked` questions. A stop while the
    /// call is in flight drops the request and writes its `stopped` row.
    pub async fn call(
        &self,
        system: &str,
        user: &str,
        want: i64,
        asked: usize,
    ) -> Result<CallResult, Stop> {
        self.call_with(system, None, user, want, asked).await
    }

    /// `call` with a cached prefix: a text that opens the user turn and is the same for every call of the job (the
    /// whole file of an AI import, the whole source of a generation).
    pub async fn call_with(
        &self,
        system: &str,
        prefix: Option<&str>,
        user: &str,
        want: i64,
        asked: usize,
    ) -> Result<CallResult, Stop> {
        self.call_opts(system, prefix, user, want, asked, CallOpts::default())
            .await
    }

    /// `call_with` and the v1.3 options: a send that repeats an earlier one bypasses the gateway's response cache, a
    /// prefix below the model's cache minimum goes out without cache markers, a plan call writes its own purpose.
    pub async fn call_opts(
        &self,
        system: &str,
        prefix: Option<&str>,
        user: &str,
        want: i64,
        asked: usize,
        opts: CallOpts,
    ) -> Result<CallResult, Stop> {
        let app = &self.app;
        let route = &self.route;
        let target = app
            .0
            .target(route)
            .map_err(|_| Stop::Fail("ai.gen.stopped"))?;
        let turns = [provider::Turn {
            role: Role::User,
            content: user.to_string(),
        }];
        let spec = provider::ChatSpec {
            model: &route.model,
            system,
            turns: &turns,
            usage_option: route.usage_option != 0,
            skip_cache: opts.skip_cache,
            prefix,
            cache: opts.cache.unwrap_or_else(|| {
                prefix.is_none()
                    || provider::cache_floor_ok(route.format, &route.model, system, prefix)
            }),
            schema: opts.schema,
            effort: route.effort,
            // background job: the whole answer at once (streams of some services stop mid-answer), unless a whole answer
            // of this route timed out before (a proxy that cuts a quiet response): then it streams
            whole: !app.0.no_whole_of(&route.key),
        };
        let whole = spec.whole && route.format == crate::settings::Format::OpenAi;
        let planned = app.token_plan(route, &target, &spec, want);
        drop(target);
        let provider_name = route.provider.as_str();
        let model = route.model.clone();
        let started = std::time::Instant::now();
        let base_row = |status: &'static str| GenCallRow {
            connection_id: Some(route.connection_id),
            connection: Some(route.connection_name.clone()),
            task_id: self.task,
            user_id: self.who.id,
            purpose: opts.purpose.unwrap_or(self.purpose),
            at: unix_now(),
            provider: provider_name,
            model: model.clone(),
            tokens_in: None,
            tokens_out: None,
            tokens_cached: None,
            tokens_cache_write: None,
            latency_ms: provider::ms(started.elapsed()),
            first_byte_ms: None,
            status,
            error_code: None,
            asked: i64::try_from(asked).unwrap_or(0),
            accepted: 0,
            truncated: false,
        };
        let (plan, _first) = match planned {
            Ok(p) => p,
            Err(e) => {
                // the request could not even be built: nothing was sent, nothing is written
                tracing::warn!("an AI job could not build its request: {}", e.code.code());
                return Err(Stop::Fail("ai.gen.stopped"));
            }
        };
        let t = app.0.timeouts;
        app.0.jobs.note_sent(self.task);
        let mut cancel = self.cancel.clone();
        let mut shutdown = app.0.shutdown.clone();
        let deadline = self.deadline;
        let outcome = tokio::select! {
            r = provider::collect_chat(&app.0.client, plan, t, Duration::from_secs(CALL_TOTAL_SECS)) => Ok(r),
            _ = cancelled(&mut cancel) => Err(None),
            _ = shut(&mut shutdown) => Err(Some(Stop::Shutdown)),
            _ = tokio::time::sleep_until(deadline) => Err(Some(Stop::Fail(self.timeout_code))),
        };
        match outcome {
            Ok(Ok(c)) => {
                let mut row = base_row("ok");
                row.tokens_in = c.tokens_in;
                row.tokens_out = c.tokens_out;
                row.tokens_cached = c.tokens_cached;
                row.tokens_cache_write = c.tokens_cache_write;
                row.first_byte_ms = c.first_byte_ms;
                row.truncated = c.truncated;
                Ok(CallResult::Answer {
                    text: c.text,
                    truncated: c.truncated,
                    row,
                })
            }
            Ok(Err(e)) => {
                if whole
                    && (e.code == provider::UpCode::Timeout || matches!(e.http, Some(504 | 524)))
                {
                    app.0.remember_no_whole(&route.key);
                }
                app.0
                    .failures
                    .note(&self.who.username, e.code.code(), e.http);
                let mut row = base_row("error");
                row.error_code = Some(e.code.short().to_string());
                let hint = RetryHint {
                    wait_ms: e.retry_ms,
                    daily: e.daily,
                };
                Ok(CallResult::Error {
                    code: e.code,
                    row,
                    hint,
                })
            }
            Err(stop) => {
                let row = base_row("stopped");
                let alive = self.write_call(&row, None, None);
                match stop {
                    Some(s) => Err(s),
                    None => {
                        let why = (*self.cancel.borrow()).unwrap_or(StopWhy::Cancel);
                        Err(Stop::Cancelled(if alive { why } else { StopWhy::Purged }))
                    }
                }
            }
        }
    }
}

/// The deadline of a job without one (A-OWNER-45, "unlimited"): far enough away never to matter.
pub fn no_deadline() -> Instant {
    Instant::now() + Duration::from_secs(365 * 24 * 3600)
}

/// The seconds of backoff after provider errors, scaled by the test knob (100 = the binding 15 s steps).
fn backoff(app: &AppState, secs: u64) -> f64 {
    secs as f64 * Knobs::get(&app.0.knobs.backoff_scale) as f64 / 100.0
}

/// How a batch's prompt is built and how many output tokens it asks for (`None` = the default of the limits).
pub type PromptFn<'a> = dyn FnMut(&Run, &[usize]) -> (Prompt, Option<i64>) + Send + 'a;

/// The batch loop of GJ-3: runs `run` to `Finish`. `cap_batches`: lower the batch size to what the provider's token
/// ceiling allows (generation); an import's conversion has its own size rules.
pub async fn drive(
    ctx: &Ctx,
    run: &mut Run,
    cap_batches: bool,
    prompt_for: &mut PromptFn<'_>,
) -> Result<(), Stop> {
    let app = &ctx.app;
    loop {
        ctx.check()?;
        ctx.recheck().await?;
        let l = app.0.limits();
        let want_tokens = if ctx.feature == Feature::Generate {
            l.gen_tokens()
        } else {
            l.import_tokens()
        };
        let call_chars =
            crate::limits::Limits::usize_of(crate::limits::tokens_to_chars(want_tokens));
        let default_want = want_tokens;
        let mut fp: Option<String> = None;
        if cap_batches {
            {
                let ceiling = app.token_ceiling(&ctx.route, default_want);
                let mut n = per_call(call_chars, ceiling);
                // what an earlier job learned about this provider's real output limit (L-44)
                if let Some(m) = app.0.gen_per_call_cap(&ctx.route.key) {
                    n = n.min(m);
                }
                run.cap_per_call(n);
                fp = Some(ctx.route.key.clone());
            }
        }
        let batch = match run.next() {
            Next::Finish => return Ok(()),
            Next::Ask(b) => b,
        };
        ctx.sleep_checked(backoff(app, run.backoff_secs())).await?;
        let (prompt, want) = prompt_for(run, &batch);
        let attempt = run.attempt_of(&batch);
        let mut user = prompt.user;
        if attempt >= 2 {
            user.push_str(&crate::gen::run::attempt_line(
                attempt,
                &crate::util::hex(&random_bytes::<4>()),
            ));
        }
        let result = ctx
            .send(
                &prompt.system,
                None,
                &user,
                want.unwrap_or(default_want),
                batch.len(),
                CallOpts {
                    schema: Some(crate::gen::schema::questions()),
                    ..CallOpts::for_attempt(attempt)
                },
            )
            .await;
        match result? {
            CallResult::Answer {
                text,
                truncated,
                mut row,
            } => {
                let before = run.per_call();
                let report = run.on_answer(&batch, &text, truncated);
                if let Some(fp) = &fp {
                    if run.per_call() < before {
                        app.0.note_gen_per_call(fp, run.per_call());
                    }
                }
                row.accepted = i64::try_from(report.accepted).unwrap_or(0);
                log_failed_slots(ctx.task, run.calls(), &report);
                let made = i64::try_from(run.made()).unwrap_or(0);
                if !ctx.write_call(&row, Some(made), Some("writing")) {
                    return Err(Stop::Cancelled(StopWhy::Purged));
                }
            }
            CallResult::Error { code, row, .. } => {
                run.on_provider_error(&batch, code.code());
                let made = i64::try_from(run.made()).unwrap_or(0);
                if !ctx.write_call(&row, Some(made), Some("writing")) {
                    return Err(Stop::Cancelled(StopWhy::Purged));
                }
            }
        }
    }
}

/// The per-call INFO line of a call that lost slots (A-OWNER-66.3): slot numbers and reason codes, never the text.
pub fn log_failed_slots(task: i64, call: usize, report: &crate::gen::run::BatchReport) {
    if report.failed.is_empty() {
        return;
    }
    let list = report
        .failed
        .iter()
        .map(|(no, why)| format!("{no}:{why}"))
        .collect::<Vec<_>>()
        .join(" ");
    let gave = if report.given_up.is_empty() {
        String::new()
    } else {
        format!(
            "; given up after the last try: {}",
            report
                .given_up
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    tracing::info!(
        "AI job {task} call {call}: {} accepted, failed slots {list}{gave}",
        report.accepted
    );
}

// —— ending a job ————————————————————————————————————————————————————————————————————————————————————————————————————

/// Ends a job as failed (not charged). Returns `false` when it was no longer active.
pub fn fail_job(app: &AppState, task: i64, code: &str, error_arg: Option<Value>) -> bool {
    let now = unix_now();
    app.0
        .store
        .write(|tx| -> rusqlite::Result<bool> {
            if let Some(arg) = error_arg {
                if let Some(row) = jobs::get(tx, task)? {
                    let mut p = row.payload.clone();
                    if let Some(o) = p.as_object_mut() {
                        o.insert("errorArg".into(), arg);
                    }
                    jobs::update_payload(tx, task, &p, None, now)?;
                }
            }
            Ok(jobs::finish(
                tx,
                task,
                &End {
                    state: "failed",
                    error: Some(code),
                    charged: false,
                    result: None,
                    seen: false,
                },
                now,
            )? > 0)
        })
        .unwrap_or(false)
}

/// Waits until the user's job place frees (FIFO), or the job is stopped meanwhile.
pub async fn wait_for_place(
    app: &AppState,
    cancel: &mut watch::Receiver<Option<StopWhy>>,
) -> Option<GatePermit> {
    let mut shutdown = app.0.shutdown.clone();
    tokio::select! {
        p = app.0.gen_gate.acquire() => Some(p),
        _ = cancelled(cancel) => None,
        _ = shut(&mut shutdown) => None,
    }
}

/// The create call with 3 tries 5 s apart (GJ-3): `Ok(reply)`, or the failure code.
pub async fn create_bank(
    app: &AppState,
    cancel: &watch::Receiver<Option<StopWhy>>,
    payload: &Value,
) -> Result<crate::gen::wire::CreateReply, &'static str> {
    use crate::gen::wire::{parse_create, Reason, WireError};
    let mut last = "ai.gen.save";
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        if *app.0.shutdown.borrow()
            || matches!(*cancel.borrow(), Some(StopWhy::Cancel | StopWhy::Purged))
        {
            return Err("ai.gen.save");
        }
        match app
            .0
            .link
            .data_call("/internal/ai/gen/create", payload.clone())
            .await
        {
            Ok(bytes) => match parse_create(&bytes) {
                Ok(r) => return Ok(r),
                Err(WireError::Refused(Reason::NotAllowed | Reason::User)) => {
                    return Err("ai.gen.forbidden")
                }
                Err(_) => last = "ai.gen.save",
            },
            Err(_) => last = "ai.gen.save",
        }
    }
    Err(last)
}

// —— the generation runner ———————————————————————————————————————————————————————————————————————————————————————————

/// Everything a generation job needs after the start route wrote its row.
pub struct GenJob {
    pub task: i64,
    pub key: String,
    pub who: Identity,
    pub source: String,
    pub name: String,
    pub from_ids: Vec<i64>,
    pub plan: Plan,
    /// The chunk (index into `ranges`) of each slot of the plan
    pub groups: Vec<usize>,
    /// Every question of the source, in source order (dropped when the job ends)
    pub corpus: Vec<SeedFull>,
    /// The corpus range of each chunk (one chunk in the normal case)
    pub ranges: Vec<std::ops::Range<usize>>,
    pub previous: Vec<Tri>,
    /// `YYYY-MM-DD HH:MM:SS` of the job's start (the create route's `at`)
    pub at: String,
    /// The caller's address and browser (for the activity event only)
    pub client: crate::identity::ClientInfo,
}

/// Starts the runner of a generation job (the row is queued and registered here).
pub fn spawn_generation(app: &AppState, job: GenJob) {
    let (cancel, reg) = app.0.jobs.register(app, job.task, job.who.id);
    let app = app.clone();
    tokio::spawn(async move {
        let _reg = reg;
        let task = job.task;
        run_generation(app.clone(), job, cancel).await;
        // the questions shown while the job ran go with it (a process that is shutting down leaves them to the repair)
        if !*app.0.shutdown.borrow() {
            let _ = app
                .0
                .store
                .write(|tx| crate::store::items::items_clear(tx, task));
        }
    });
}

/// A job that ends in failure: the row, the System log line and the activity event.
fn gen_failed(app: &AppState, job: &GenJob, code: &'static str, calls: usize, detail: &str) {
    if fail_job(app, job.task, code, None) {
        log_failed(app, &job.who, code, calls, detail);
        app.0.activity(
            crate::activity::Event::new(Some(&job.who), "ai.gen.create", &job.client)
                .object(job.name.clone())
                .failed(code),
        );
    }
}

/// One plan-type call (a plan call, or the global plan): `Ok(Some(text))` for an answer, `Ok(None)` for a provider error
/// (a plan failure never fails the job), `Err` only for a stop. The row is written with the purpose `plan`. From the
/// second send on the text differs and the gateway's response cache is bypassed. `schema` = the structured answer shape.
#[allow(clippy::too_many_arguments)]
async fn plan_call(
    ctx: &Ctx,
    system: &str,
    prefix: Option<&str>,
    user: &str,
    want: i64,
    cache: Option<bool>,
    attempt: u32,
    schema: &'static Value,
) -> Result<Option<String>, Stop> {
    ctx.check()?;
    ctx.recheck().await?;
    let user = if attempt >= 2 {
        format!(
            "{user}{}",
            crate::gen::run::attempt_line(attempt, &crate::util::hex(&random_bytes::<4>()))
        )
    } else {
        user.to_string()
    };
    let opts = CallOpts {
        skip_cache: attempt >= 2,
        cache,
        purpose: Some("plan"),
        schema: Some(schema),
    };
    let r = ctx.send(system, prefix, &user, want, 0, opts).await;
    let (row, text) = match r? {
        CallResult::Answer { text, row, .. } => (row, Some(text)),
        CallResult::Error { row, .. } => (row, None),
    };
    if !ctx.write_call(&row, None, Some("planning")) {
        return Err(Stop::Cancelled(StopWhy::Purged));
    }
    Ok(text)
}

/// The plan call of one chunk: the first request with the chunk's prefix (it writes the cache). One differing retry,
/// then `None`: the job goes on with the local plan.
async fn plan_chunk(
    ctx: &Ctx,
    whole: &crate::gen::par::Whole,
    slots: &[&crate::gen::plan::Slot],
    valid: &std::ops::Range<usize>,
    note: Option<&whole_gen::ChunkNote>,
) -> Result<Option<whole_gen::ModelPlan>, Stop> {
    let user = whole_gen::plan_user(slots, note);
    let want = whole_gen::plan_want(slots.len());
    let nos: Vec<usize> = slots.iter().map(|s| s.no).collect();
    for attempt in 1..=2u32 {
        if let Some(text) = plan_call(
            ctx,
            &whole.system,
            Some(&whole.prefix),
            &user,
            want,
            None,
            attempt,
            crate::gen::schema::gen_plan(),
        )
        .await?
        {
            if let Some(p) = whole_gen::parse_plan(&text, &nos, valid) {
                return Ok(Some(p));
            }
        }
    }
    Ok(None)
}

async fn run_generation(app: AppState, job: GenJob, mut cancel: watch::Receiver<Option<StopWhy>>) {
    // 1. a job place (FIFO); a cancel while waiting ends the job unseen by the runner (the route wrote the end state)
    let Some(place) = wait_for_place(&app, &mut cancel).await else {
        return;
    };
    // 2. running; no row changed = cancelled or purged meanwhile
    let now = unix_now();
    let changed = app
        .0
        .store
        .write(|tx| -> rusqlite::Result<usize> {
            let n = jobs::mark_running(tx, job.task, now, true)?;
            if n > 0 {
                jobs::set_phase(tx, job.task, "planning", now)?;
            }
            Ok(n)
        })
        .unwrap_or(0);
    if changed == 0 {
        return;
    }
    tracing::info!(
        "user {:?} started an AI bank generation ({} mode, {} questions)",
        job.who.username,
        job.source,
        job.plan.slots.len()
    );
    if job.ranges.len() > 1 {
        tracing::info!(
            "AI bank generation of user {:?} is cut into {} chunks",
            job.who.username,
            job.ranges.len()
        );
    }
    let writing = Knobs::get(&app.0.knobs.writing_secs);
    let (hard_cap, unlimited) = {
        let l = app.0.limits();
        (
            usize::try_from(l.gen_max_calls).unwrap_or(15),
            l.gen_unlimited == 1,
        )
    };
    // the route is resolved once, here, and kept for the whole job (the prompt cache is per connection)
    let route = match app.0.route(crate::route::Slot::Generate, None) {
        Ok(r) => r,
        Err(_) => {
            drop(place);
            fail_job(&app, job.task, "ai.gen.stopped", None);
            return;
        }
    };
    let ctx = Ctx {
        app: app.clone(),
        task: job.task,
        who: job.who.clone(),
        feature: Feature::Generate,
        purpose: "generate",
        route,
        cancel,
        deadline: if unlimited {
            no_deadline()
        } else {
            Instant::now() + Duration::from_secs(writing)
        },
        timeout_code: "ai.gen.timeout",
    };
    let limits = JobLimits::for_plan(&job.plan);
    let mut run = Run::new(
        job.plan.clone(),
        Deduper::new(
            job.corpus.iter().map(|c| c.stem.clone()).collect(),
            job.previous.clone(),
        )
        .relaxed(job.corpus.len() < job.plan.slots.len()),
        limits,
    )
    .with_hard_cap(hard_cap)
    .with_unlimited(unlimited)
    .with_groups(job.groups.clone())
    // v2.4: a stuck question is left out; the bank is made with at least half of the questions
    .with_partial(true);
    let driven = drive_generation(&ctx, &job, &mut run).await;
    drop(place);
    let calls = run.calls();
    tracing::info!(
        "AI bank generation of user {:?}: {} of {} questions valid after {} model calls; rejections: {}",
        job.who.username,
        run.made(),
        run.goal(),
        calls,
        run.reason_counts()
    );
    match driven {
        Err(Stop::Cancelled(_)) | Err(Stop::Shutdown) => {}
        Err(Stop::Fail(code)) => gen_failed(&app, &job, code, calls, &run_detail(&run)),
        Ok(()) => match run.outcome() {
            Outcome::Failed(FailCode::Quality) => {
                gen_failed(&app, &job, "ai.gen.quality", calls, &run_detail(&run))
            }
            Outcome::Failed(FailCode::Provider(code)) => {
                gen_failed(&app, &job, static_up_code(&code), calls, &run_detail(&run))
            }
            Outcome::Full | Outcome::Partial { .. } => {
                // a partial run (some stuck slots were left out) saves what is valid: `made` questions
                let made = run.made();
                // saving: the compare-and-set on the phase decides against a cancel that came first
                let now = unix_now();
                let set = app
                    .0
                    .store
                    .write(|tx| jobs::set_phase(tx, job.task, "saving", now))
                    .unwrap_or(0);
                if set == 0 {
                    return;
                }
                let questions = run.questions();
                let payload = assemble::payload(
                    &job.key,
                    job.who.id,
                    &job.who.stamp,
                    &job.name,
                    &job.source,
                    &job.from_ids,
                    made,
                    &job.at,
                    &questions,
                );
                match create_bank(&app, &ctx.cancel, &payload).await {
                    Ok(reply) => {
                        let result =
                            json!({"bankId": reply.bank_id, "name": reply.name, "made": made});
                        let now = unix_now();
                        let n = app
                            .0
                            .store
                            .write(|tx| -> rusqlite::Result<usize> {
                                tx.execute(
                                    "UPDATE tasks SET made = ?2 WHERE id = ?1",
                                    rusqlite::params![job.task, made as i64],
                                )?;
                                jobs::finish(
                                    tx,
                                    job.task,
                                    &End {
                                        state: "done",
                                        error: None,
                                        charged: true,
                                        result: Some(&result),
                                        seen: false,
                                    },
                                    now,
                                )
                            })
                            .unwrap_or(0);
                        if n > 0 {
                            app.0.activity(
                                crate::activity::Event::new(
                                    Some(&job.who),
                                    "ai.gen.create",
                                    &job.client,
                                )
                                .object(reply.name.clone())
                                .detail(json!({"questions": made})),
                            );
                            tracing::info!(
                                "user {:?} finished an AI bank generation ({} questions, {} model calls)",
                                job.who.username,
                                made,
                                calls
                            );
                        }
                    }
                    Err(code) => gen_failed(&app, &job, code, calls, &run_detail(&run)),
                }
            }
        },
    }
}

/// Planning and writing of a generation: the prefix of every chunk, the global plan of a chunked job, the plan call of
/// each chunk (the chunks at the same time, up to the job's width), then the writing calls of the whole job.
async fn drive_generation(ctx: &Ctx, job: &GenJob, run: &mut Run) -> Result<(), Stop> {
    use crate::gen::par::{self, Par, Step, WIDTH};
    let app = &ctx.app;
    // the chunks that have slots, each with its prefix (a nonce per chunk)
    let mut chunk_ids: Vec<usize> = job.groups.clone();
    chunk_ids.sort_unstable();
    chunk_ids.dedup();
    let mut whole_of: HashMap<usize, usize> = HashMap::new();
    let mut wholes: Vec<par::Whole> = Vec::new();
    for &c in &chunk_ids {
        let Some(r) = job.ranges.get(c) else {
            return Err(Stop::Fail("ai.gen.stopped"));
        };
        let nonce = crate::util::hex(&random_bytes::<8>());
        whole_of.insert(c, wholes.len());
        wholes.push(whole_gen::build(
            &job.corpus[r.clone()],
            r.start + 1,
            &nonce,
        ));
    }
    let number_of: HashMap<String, usize> = job
        .corpus
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id.clone(), i + 1))
        .collect();
    let slots_of = |c: usize| -> Vec<&crate::gen::plan::Slot> {
        job.plan
            .slots
            .iter()
            .zip(&job.groups)
            .filter(|(_, g)| **g == c)
            .map(|(s, _)| s)
            .collect()
    };
    // 1. the global plan of a chunked job (no cached prefix); a failure only means the chunks plan on their own
    let notes: Vec<whole_gen::ChunkNote> = if chunk_ids.len() > 1 {
        let l = app.0.limits();
        let context = par::route_context(&ctx.route);
        let room = i64::try_from(context).unwrap_or(i64::MAX)
            - par::reserve(l.gen_tokens())
            - crate::limits::estimate_tokens(whole_gen::GEN_WHOLE_SYSTEM_PROMPT);
        let index = whole_gen::index(&job.corpus, &job.ranges, room.max(1000));
        let per_chunk: Vec<usize> = (0..job.ranges.len()).map(|c| slots_of(c).len()).collect();
        let nonce = crate::util::hex(&random_bytes::<8>());
        let user = whole_gen::global_user(&index, &per_chunk, &nonce);
        let mut got: Option<Vec<whole_gen::ChunkNote>> = None;
        for attempt in 1..=2u32 {
            if let Some(text) = plan_call(
                ctx,
                whole_gen::GEN_WHOLE_SYSTEM_PROMPT,
                None,
                &user,
                // the answer (about 4 000 tokens at most) plus room for the model's own thinking (v1.6, owner: 8 000)
                8_000,
                Some(false),
                attempt,
                crate::gen::schema::global_plan(),
            )
            .await?
            {
                got = whole_gen::parse_global(&text, job.ranges.len());
                if got.is_some() {
                    break;
                }
            }
        }
        got.unwrap_or_default()
    } else {
        Vec::new()
    };
    // 2. the plan call of every chunk, up to WIDTH at the same time
    let plans: Vec<Option<(whole_gen::ModelPlan, Instant)>> = {
        use futures_util::stream::{self, StreamExt};
        let mut out: Vec<Option<(whole_gen::ModelPlan, Instant)>> =
            (0..job.ranges.len()).map(|_| None).collect();
        let mut results = stream::iter(chunk_ids.iter().copied().map(|c| {
            let slots = slots_of(c);
            let whole = &wholes[whole_of[&c]];
            let valid = job.ranges[c].start + 1..job.ranges[c].end + 1;
            let note = notes.get(c);
            async move {
                let r = plan_chunk(ctx, whole, &slots, &valid, note).await;
                (c, r.map(|p| p.map(|p| (p, Instant::now()))))
            }
        }))
        .buffer_unordered(WIDTH);
        while let Some((c, r)) = results.next().await {
            out[c] = r?;
        }
        out
    };
    // 3. writing
    let now = unix_now();
    let set = app
        .0
        .store
        .write(|tx| jobs::set_phase(tx, job.task, "writing", now))
        .unwrap_or(0);
    if set == 0 {
        return Err(Stop::Cancelled(StopWhy::Purged));
    }
    let first = wholes.first().ok_or(Stop::Fail("ai.gen.stopped"))?;
    let cache_on = crate::provider::cache_floor_ok(
        ctx.route.format,
        &ctx.route.model,
        &first.system,
        Some(&first.prefix),
    );
    // the cache is probably gone when a plan is missing (its call did not write it) or the oldest one is too old
    let cold = cache_on
        && (chunk_ids.iter().any(|&c| plans[c].is_none())
            || plans
                .iter()
                .flatten()
                .any(|(_, at)| par::cold(*at, Instant::now())));
    let models: Vec<Option<&whole_gen::ModelPlan>> =
        plans.iter().map(|p| p.as_ref().map(|(m, _)| m)).collect();
    let mut step_for = |run: &Run, batch: &[usize]| -> Step {
        let slots = run.slots(batch);
        let c = batch.first().map_or(0, |&i| run.group_of(i));
        let planned = whole_gen::Planned {
            model: models.get(c).copied().flatten(),
            number_of: &number_of,
        };
        let nonce = crate::util::hex(&random_bytes::<8>());
        Step {
            whole: whole_of.get(&c).copied().unwrap_or(0),
            user: whole_gen::write_user(&slots, &planned, &run.written(), &nonce)
                + &run.retry_note(batch),
            want: None,
        }
    };
    let par = Par {
        wholes: &wholes,
        cap_batches: true,
        width: WIDTH,
        cold,
        generation: true,
        items: true,
        schema: Some(crate::gen::schema::questions()),
    };
    par::drive(ctx, &par, run, &mut step_for).await
}

/// One-line facts of a run for the failure log: `made/goal`, the model calls and the rejection counts by reason code
/// (never question text).
pub fn run_detail(run: &Run) -> String {
    format!(
        "{}/{} questions passed the checks; rejected for: {}",
        run.made(),
        run.goal(),
        run.reason_counts()
    )
}

/// A job that ends in failure goes to the System log at ERROR with its reason code (A-OWNER-34); a refusal by a per-user
/// cap stays at WARN. `detail` is `run_detail` or empty. Never any question text, prompt, key or URL.
pub fn log_failed(app: &AppState, who: &Identity, code: &str, calls: usize, detail: &str) {
    let _ = app;
    let detail = if detail.is_empty() {
        String::new()
    } else {
        format!(": {detail}")
    };
    // only a per-user cap refusal is a warning; a job that hit its own call ceiling (ai.import.callLimit) failed
    if code == "ai.callLimit" {
        tracing::warn!(
            "AI job refused by a call limit{detail} ({code}, {calls} model calls, user {:?})",
            who.username
        );
    } else {
        tracing::error!(
            "AI job failed{detail} ({code}, {calls} model calls, user {:?})",
            who.username
        );
    }
}

/// Maps a provider code string (`ai.up.rateLimit`, ...) to its `'static` text.
pub fn static_up_code(code: &str) -> &'static str {
    use UpCode::*;
    for c in [
        Auth, Quota, NotFound, RateLimit, Overloaded, Server, BadRequest, Timeout, Connect,
        Protocol, Refused, TooLong,
    ] {
        if c.code() == code {
            return c.code();
        }
    }
    "ai.up.server"
}

/// `N` random bytes.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    if getrandom::fill(&mut b).is_err() {
        // never expected; fall back to the clock so a nonce is at least different per call
        let t = unix_now().to_le_bytes();
        for (i, x) in b.iter_mut().enumerate() {
            *x = t[i % 8] ^ (i as u8);
        }
    }
    b
}

// —— restart ————————————————————————————————————————————————————————————————————————————————————————————————————————

/// At start-up (before the service listens): every queued or running job except those in the phase `saving` becomes
/// failed `ai.gen.interrupted`, not charged. Returns the number repaired.
pub fn repair_at_start(app: &AppState) -> usize {
    let now = unix_now();
    match app.0.store.write(|tx| -> rusqlite::Result<usize> {
        let n = jobs::repair_interrupted(tx, now, true)?;
        // the questions a cut-off job showed go with it
        crate::store::items::items_sweep(tx)?;
        Ok(n)
    }) {
        Ok(n) => {
            if n > 0 {
                tracing::info!("marked {n} AI bank jobs cut off by a restart as interrupted");
            }
            n
        }
        Err(e) => {
            tracing::warn!("could not repair the AI jobs after a restart: {e}");
            0
        }
    }
}

/// The jobs that were saving when the service stopped: the backend is asked for a bank of the job key (API-I4); one
/// found → done, otherwise (or the backend unreachable after three tries) interrupted. Jobs of this process are left
/// alone.
pub async fn settle_saving(app: &AppState) {
    use crate::gen::wire::parse_lookup;
    let rows = app.0.store.read(jobs::saving_jobs).unwrap_or_default();
    for row in rows {
        if app.0.jobs.is_registered(row.id) {
            continue;
        }
        let Some(key) = row.job_key.clone() else {
            continue;
        };
        let mut found = None;
        let mut reached = false;
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            if let Ok(bytes) = app
                .0
                .link
                .data_call("/internal/ai/gen/lookup", json!({"job": key}))
                .await
            {
                if let Ok(b) = parse_lookup(&bytes) {
                    found = b;
                    reached = true;
                    break;
                }
            }
        }
        let now = unix_now();
        let _ = app.0.store.write(|tx| -> rusqlite::Result<()> {
            match (&found, reached) {
                (Some(b), _) => {
                    let result = json!({"bankId": b.bank_id, "name": b.name, "made": row.goal});
                    tx.execute(
                        "UPDATE tasks SET made = goal WHERE id = ?1",
                        rusqlite::params![row.id],
                    )?;
                    jobs::finish(
                        tx,
                        row.id,
                        &End {
                            state: "done",
                            error: None,
                            charged: true,
                            result: Some(&result),
                            seen: false,
                        },
                        now,
                    )?;
                }
                _ => {
                    jobs::finish(
                        tx,
                        row.id,
                        &End {
                            state: "failed",
                            error: Some("ai.gen.interrupted"),
                            charged: false,
                            result: None,
                            seen: false,
                        },
                        now,
                    )?;
                }
            }
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_codes_map_to_static_strings() {
        assert_eq!(static_up_code("ai.up.rateLimit"), "ai.up.rateLimit");
        assert_eq!(static_up_code("ai.up.tooLong"), "ai.up.tooLong");
        assert_eq!(static_up_code("nonsense"), "ai.up.server");
    }

    #[test]
    fn random_bytes_differ() {
        let a = random_bytes::<16>();
        let b = random_bytes::<16>();
        assert_ne!(a, b);
    }
}
