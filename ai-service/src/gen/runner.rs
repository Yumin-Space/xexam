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
use crate::gen::prompt::{build_prompt, Prompt};
use crate::gen::run::{FailCode, JobLimits, Next, Outcome, Run};
use crate::gen::{SeedFull, Tri};
use crate::identity::Identity;
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
    pub cancel: watch::Receiver<Option<StopWhy>>,
    pub deadline: Instant,
    /// The failure code when the deadline passes (`ai.gen.timeout` or `ai.import.readTimeout`)
    pub timeout_code: &'static str,
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
    },
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
        let on = app.0.snapshot().map(|s| s.effective()).unwrap_or(false);
        if !on || app.0.suspended(&self.who) || !app.0.feature_allowed(&self.who, self.feature) {
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
    /// whole file of an AI import).
    pub async fn call_with(
        &self,
        system: &str,
        prefix: Option<&str>,
        user: &str,
        want: i64,
        asked: usize,
    ) -> Result<CallResult, Stop> {
        let app = &self.app;
        let snap = app.0.snapshot().map_err(|_| Stop::Fail("ai.gen.stopped"))?;
        let target = app
            .0
            .target(&snap)
            .map_err(|_| Stop::Fail("ai.gen.stopped"))?;
        let s = &snap.settings;
        let turns = [provider::Turn {
            role: Role::User,
            content: user.to_string(),
        }];
        let spec = provider::ChatSpec {
            model: &s.model,
            system,
            turns: &turns,
            usage_option: s.usage_option,
            skip_cache: false,
            prefix,
        };
        let planned = app.token_plan(&snap, &target, &spec, want);
        drop(target);
        let provider_name = s.provider.as_str();
        let model = s.model.clone();
        let started = std::time::Instant::now();
        let base_row = |status: &'static str| GenCallRow {
            task_id: self.task,
            user_id: self.who.id,
            purpose: self.purpose,
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
                app.0
                    .failures
                    .note(&self.who.username, e.code.code(), e.http);
                let mut row = base_row("error");
                row.error_code = Some(e.code.short().to_string());
                Ok(CallResult::Error { code: e.code, row })
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
            if let Ok(snap) = app.0.snapshot() {
                let ceiling = app.token_ceiling(&snap, default_want);
                let mut n = per_call(call_chars, ceiling);
                // what an earlier job learned about this provider's real output limit (L-44)
                if let Some(m) = app.0.gen_per_call_cap(&snap.fp) {
                    n = n.min(m);
                }
                run.cap_per_call(n);
                fp = Some(snap.fp.clone());
            }
        }
        let batch = match run.next() {
            Next::Finish => return Ok(()),
            Next::Ask(b) => b,
        };
        ctx.sleep_checked(backoff(app, run.backoff_secs())).await?;
        let permit = ctx.place().await?;
        let (prompt, want) = prompt_for(run, &batch);
        let result = ctx
            .call(
                &prompt.system,
                &prompt.user,
                want.unwrap_or(default_want),
                batch.len(),
            )
            .await;
        drop(permit);
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
            CallResult::Error { code, row } => {
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
    pub seeds: HashMap<String, SeedFull>,
    pub source_stems: Vec<Tri>,
    pub previous: Vec<Tri>,
    /// `YYYY-MM-DD HH:MM:SS` of the job's start (the create route's `at`)
    pub at: String,
}

/// Starts the runner of a generation job (the row is queued and registered here).
pub fn spawn_generation(app: &AppState, job: GenJob) {
    let (cancel, reg) = app.0.jobs.register(app, job.task, job.who.id);
    let app = app.clone();
    tokio::spawn(async move {
        let _reg = reg;
        run_generation(app, job, cancel).await;
    });
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
        .write(|tx| jobs::mark_running(tx, job.task, now, true))
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
    let writing = Knobs::get(&app.0.knobs.writing_secs);
    let (hard_cap, unlimited) = {
        let l = app.0.limits();
        (
            usize::try_from(l.gen_max_calls).unwrap_or(15),
            l.gen_unlimited == 1,
        )
    };
    let ctx = Ctx {
        app: app.clone(),
        task: job.task,
        who: job.who.clone(),
        feature: Feature::Generate,
        purpose: "generate",
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
        Deduper::new(job.source_stems.clone(), job.previous.clone())
            .relaxed(job.source_stems.len() < job.plan.slots.len()),
        limits,
    )
    .with_hard_cap(hard_cap)
    .with_unlimited(unlimited);
    let seeds = &job.seeds;
    let driven = {
        let mut prompt_for = |run: &Run, batch: &[usize]| -> (Prompt, Option<i64>) {
            let slots = run.slots(batch);
            let nonce = crate::util::hex(&random_bytes::<8>());
            let mut prompt = build_prompt(&slots, seeds, &run.written(), &nonce);
            prompt.user.push_str(&run.retry_note(batch));
            (prompt, None)
        };
        drive(&ctx, &mut run, true, &mut prompt_for).await
    };
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
        Err(Stop::Fail(code)) => {
            if fail_job(&app, job.task, code, None) {
                log_failed(&app, &job.who, code, calls, &run_detail(&run));
            }
        }
        Ok(()) => match run.outcome() {
            // a generation never ends partial (A-OWNER-13c); the arm only keeps the match total
            Outcome::Failed(FailCode::Quality) | Outcome::Partial { .. } => {
                if fail_job(&app, job.task, "ai.gen.quality", None) {
                    log_failed(&app, &job.who, "ai.gen.quality", calls, &run_detail(&run));
                }
            }
            Outcome::Failed(FailCode::Provider(code)) => {
                let code = static_up_code(&code);
                if fail_job(&app, job.task, code, None) {
                    log_failed(&app, &job.who, code, calls, &run_detail(&run));
                }
            }
            Outcome::Full => {
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
                    run.goal(),
                    &job.at,
                    &questions,
                );
                match create_bank(&app, &ctx.cancel, &payload).await {
                    Ok(reply) => {
                        let result = json!({"bankId": reply.bank_id, "name": reply.name, "made": run.goal()});
                        let now = unix_now();
                        let n = app
                            .0
                            .store
                            .write(|tx| -> rusqlite::Result<usize> {
                                tx.execute(
                                    "UPDATE tasks SET made = ?2 WHERE id = ?1",
                                    rusqlite::params![job.task, run.goal() as i64],
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
                            tracing::info!(
                                "user {:?} finished an AI bank generation ({} questions, {} model calls)",
                                job.who.username,
                                run.goal(),
                                calls
                            );
                        }
                    }
                    Err(code) => {
                        if fail_job(&app, job.task, code, None) {
                            log_failed(&app, &job.who, code, calls, &run_detail(&run));
                        }
                    }
                }
            }
        },
    }
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
    match app
        .0
        .store
        .write(|tx| jobs::repair_interrupted(tx, now, true))
    {
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
