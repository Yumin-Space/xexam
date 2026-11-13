//! The runner of an AI file import (spec IJ-3): extraction in the child process, recognising (classification and
//! segmentation, outside the per-user call caps), the confirm wait, then the writing loop of a generation with
//! the conversion or the material prompts, then the bank. The file's bytes and text live only in this task's memory;
//! nothing is written to disk and no file text or name reaches a log line.
use crate::app::{AppState, Knobs};
use crate::extract::child::{run_child_with, ChildArgs};
use crate::extract::{Doc, ExtractError, LangHint};
use crate::features::Feature;
use crate::gen::assemble;
use crate::gen::dedup::Deduper;
use crate::gen::plan::per_call;
use crate::gen::prompt::Prompt;
use crate::gen::run::{FailCode, JobLimits, Outcome, Run};
use crate::gen::runner::{
    create_bank, drive, fail_job, log_failed, random_bytes, run_detail, static_up_code,
    wait_for_place, CallResult, Ctx, Stop, StopWhy,
};
use crate::gen::Composition;
use crate::identity::Identity;
use crate::import::classify::{self, Verdict};
use crate::import::convert::{self, ConvertSpec};
use crate::import::count;
use crate::import::lines::{lines, total_chars, windows, Window};
use crate::import::material::{self, Excerpt};
use crate::import::segment::{self, Seg};
use crate::import::whole::{self, Whole};
use crate::import::{Mode, MAX_QUESTION_CHARS, QUESTIONS_TEXT_MAX, WINDOW_MAIN_CHARS};
use crate::provider::UpCode;
use crate::store::jobs::{self, End};
use crate::util::unix_now;
use futures_util::future::join_all;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{watch, OwnedSemaphorePermit};
use tokio::time::Instant;

/// Everything an import needs after the upload route wrote its row.
pub struct ImportJob {
    pub task: i64,
    pub key: String,
    pub who: Identity,
    /// The new bank's name (`<file stem>_<user>_<stamp>`)
    pub name: String,
    /// The cleaned file name (never logged) and its type word
    pub file_name: String,
    pub file_type: String,
    pub ext: String,
    pub lang: LangHint,
    pub bytes: Vec<u8>,
    pub comp: Composition,
    pub call_chars: usize,
    pub max_questions: usize,
    /// `YYYY-MM-DD HH:MM:SS` of the upload (the create route's `at`)
    pub at: String,
    /// The permit of `IMPORT_PENDING_MAX`, released when the extraction is over
    pub pending: Option<OwnedSemaphorePermit>,
}

/// Starts the runner (the row is queued, phase reading).
pub fn spawn_import(app: &AppState, job: ImportJob) {
    let (cancel, reg) = app.0.jobs.register(app, job.task, job.who.id);
    let app = app.clone();
    tokio::spawn(async move {
        let _reg = reg;
        run_import(app, job, cancel).await;
    });
}

/// The failure code of an extraction error and its `errorArg`.
pub fn extract_failure(e: ExtractError, max_chars: usize) -> (String, Option<Value>) {
    let code = format!("ai.import.{}", e.code());
    let arg = (e == ExtractError::TooMuchText).then(|| json!(max_chars));
    (code, arg)
}

fn static_code(code: &str) -> &'static str {
    for c in [
        "ai.import.empty",
        "ai.import.tooLarge",
        "ai.import.type",
        "ai.import.legacy",
        "ai.import.image",
        "ai.import.protected",
        "ai.import.corrupt",
        "ai.import.tooComplex",
        "ai.import.noText",
        "ai.import.tooMuchText",
        "ai.import.encoding",
        "ai.import.readTimeout",
        "ai.import.noContent",
        "ai.import.unclear",
        "ai.import.tooMany",
        "ai.import.tooLongQuestion",
        "ai.import.callLimit",
        "ai.gen.quality",
    ] {
        if c == code {
            return c;
        }
    }
    "ai.import.corrupt"
}

async fn run_import(
    app: AppState,
    mut job: ImportJob,
    mut cancel: watch::Receiver<Option<StopWhy>>,
) {
    // A-OWNER-45: with "unlimited" on, the recognising step (like the writing step) has no deadline
    let reading_deadline = if app.0.limits().import_unlimited == 1 {
        crate::gen::runner::no_deadline()
    } else {
        Instant::now() + Duration::from_secs(Knobs::get(&app.0.knobs.reading_secs))
    };
    tracing::info!(
        "user {:?} started an AI file import ({}, {})",
        job.who.username,
        job.file_type,
        human_size(job.bytes.len())
    );
    // 1. extraction (one at a time, server-wide)
    let max_bytes = u64::try_from(job.bytes.len()).unwrap_or(0);
    let doc = {
        let mut shutdown = app.0.shutdown.clone();
        let permit = tokio::select! {
            p = app.0.extract_gate.clone().acquire_owned() => p.ok(),
            _ = crate::gen::runner::cancelled(&mut cancel) => return,
            _ = crate::gen::runner::shut(&mut shutdown) => return,
        };
        let stop = Arc::new(AtomicBool::new(false));
        let args = ChildArgs::new(&job.ext, job.lang, max_bytes.max(1));
        let exe = app.0.import_exe();
        let kill = Duration::from_secs(Knobs::get(&app.0.knobs.kill_secs));
        let hang = app.0.knobs.hang_child.load(Ordering::Relaxed);
        let bytes = std::mem::take(&mut job.bytes);
        let stop2 = stop.clone();
        let work = tokio::task::spawn_blocking(move || {
            run_child_with(&exe, &args, bytes, kill, hang, &stop2)
        });
        tokio::pin!(work);
        let res = tokio::select! {
            r = &mut work => r.unwrap_or(Err(ExtractError::Corrupt)),
            _ = crate::gen::runner::cancelled(&mut cancel) => {
                stop.store(true, Ordering::SeqCst);
                let _ = work.await;
                return;
            }
        };
        drop(permit);
        // the upload's place is given back now that the bytes are gone
        job.pending.take();
        match res {
            Ok(d) => d,
            Err(e) => {
                let (code, arg) = extract_failure(e, crate::extract::Limits::default().max_chars);
                finish_failed(&app, &job, static_code(&code), arg, 0);
                return;
            }
        }
    };
    // 2. recognising
    let mut error_arg: Option<Value> = None;
    let recognised = match recognise(
        &app,
        &job,
        &doc,
        &mut cancel,
        reading_deadline,
        &mut error_arg,
    )
    .await
    {
        Ok(r) => r,
        Err(Stop::Cancelled(_)) | Err(Stop::Shutdown) => return,
        Err(Stop::Fail(code)) => {
            let arg = error_arg.or(match code {
                "ai.import.tooMany" => Some(json!(job.max_questions)),
                "ai.import.tooMuchText" => Some(json!(QUESTIONS_TEXT_MAX)),
                _ => None,
            });
            finish_failed(&app, &job, code, arg, calls_of(&app, job.task));
            return;
        }
    };
    drop(doc);
    let Recognised {
        mode,
        lines,
        segs,
        total: questions_total,
        approx,
        whole,
        at: recognised_at,
    } = recognised;
    // 3. the confirm wait
    let goal = match mode {
        Mode::Questions => segs.len(),
        Mode::Material => job.comp.single + job.comp.multi,
    };
    let confirm_secs = Knobs::get(&app.0.knobs.confirm_secs);
    let now = unix_now();
    let wrote = app
        .0
        .store
        .write(|tx| -> rusqlite::Result<usize> {
            let Some(row) = jobs::get(tx, job.task)? else {
                return Ok(0);
            };
            if !row.active() {
                return Ok(0);
            }
            let mut p = row.payload.clone();
            if let Some(o) = p.as_object_mut() {
                o.insert("mode".into(), json!(mode.as_str()));
                if mode == Mode::Questions {
                    // A-OWNER-47: the confirm says "N found, the first M are imported"
                    // `found` repeats `total` (the name the front end of v1.1 reads, A-OWNER-70)
                    o.insert("total".into(), json!(questions_total));
                    o.insert("found".into(), json!(questions_total));
                    o.insert("taken".into(), json!(segs.len()));
                    // "about N": the model and the local count of answer choices disagree
                    o.insert("approx".into(), json!(approx));
                }
                o.insert(
                    "confirmBy".into(),
                    json!(now + i64::try_from(confirm_secs).unwrap_or(1800)),
                );
            }
            jobs::update_payload(
                tx,
                job.task,
                &p,
                Some(i64::try_from(goal).unwrap_or(0)),
                now,
            )?;
            jobs::set_phase(tx, job.task, "confirm", now)
        })
        .unwrap_or(0);
    if wrote == 0 {
        return;
    }
    tracing::info!(
        "user {:?} AI file import recognised ({}, {})",
        job.who.username,
        mode.as_str(),
        goal
    );
    let Some(confirm) = app.0.jobs.confirm_signal(job.task) else {
        return;
    };
    let mut shutdown = app.0.shutdown.clone();
    let confirmed_at = tokio::select! {
        _ = confirm.notified() => Instant::now(),
        _ = crate::gen::runner::cancelled(&mut cancel) => return,
        _ = crate::gen::runner::shut(&mut shutdown) => return,
        _ = tokio::time::sleep(Duration::from_secs(confirm_secs)) => {
            // no confirm in time: cancelled, not charged, one unseen card (compare-and-set on the phase)
            let now = unix_now();
            let n = app
                .0
                .store
                .write(|tx| -> rusqlite::Result<usize> {
                    let phase: Option<String> = tx
                        .query_row("SELECT phase FROM tasks WHERE id = ?1", [job.task], |r| r.get(0))
                        .ok()
                        .flatten();
                    if phase.as_deref() != Some("confirm") {
                        return Ok(0);
                    }
                    jobs::finish(
                        tx,
                        job.task,
                        &End { state: "cancelled", error: Some("ai.import.expired"), charged: false, result: None, seen: false },
                        now,
                    )
                })
                .unwrap_or(0);
            if n > 0 {
                tracing::info!("user {:?} AI file import expired without a confirm", job.who.username);
            }
            return;
        }
    };
    // The name typed at the confirm step (A-OWNER-57) is in the payload by now
    if let Ok(Some(row)) = app.0.store.read(|c| jobs::get(c, job.task)) {
        if let Some(n) = row.payload.get("name").and_then(Value::as_str) {
            if !n.is_empty() {
                job.name = n.to_string();
            }
        }
    }
    // 4. writing: a job place (FIFO), then the batch loop
    let Some(place) = wait_for_place(&app, &mut cancel).await else {
        return;
    };
    let now = unix_now();
    let changed = app
        .0
        .store
        .write(|tx| jobs::mark_running(tx, job.task, now, false))
        .unwrap_or(0);
    if changed == 0 {
        return;
    }
    let unlimited = app.0.limits().import_unlimited == 1;
    let writing_deadline = if unlimited {
        crate::gen::runner::no_deadline()
    } else {
        confirmed_at + Duration::from_secs(Knobs::get(&app.0.knobs.writing_secs))
    };
    let ctx = Ctx {
        app: app.clone(),
        task: job.task,
        who: job.who.clone(),
        feature: Feature::Import,
        purpose: "import",
        cancel,
        deadline: writing_deadline,
        timeout_code: "ai.gen.timeout",
    };
    let seed = crate::gen::rng::Rng::seed_from_key(&job.key);
    // The per-job call ceiling of a file import, from the AI settings (A-OWNER-36b): the same for everyone
    let hard_cap = usize::try_from(app.0.limits().import_max_calls).unwrap_or(25);
    let (mut run, specs, excerpts): (Run, Vec<ConvertSpec>, Vec<Excerpt>) = match mode {
        Mode::Questions => {
            let specs: Vec<ConvertSpec> = segs
                .iter()
                .map(|s| convert::make_spec(&segment::region(&lines, s), s.kind))
                .collect();
            let plan = convert::convert_plan(&specs, job.call_chars, seed);
            let limits = JobLimits::for_convert(&plan);
            (
                Run::new_convert(plan, specs.clone(), limits, job.call_chars)
                    .with_hard_cap(hard_cap)
                    .with_unlimited(unlimited),
                specs,
                Vec::new(),
            )
        }
        Mode::Material => {
            let ex = material::excerpts(&lines);
            let want = crate::limits::chars_to_tokens(job.call_chars as i64);
            let ceiling = app
                .0
                .snapshot()
                .ok()
                .and_then(|s| app.token_ceiling(&s, want));
            let plan =
                match material::make_plan(&ex, job.comp, per_call(job.call_chars, ceiling), seed) {
                    Ok(p) => p,
                    Err(_) => {
                        finish_failed(&app, &job, "ai.gen.quality", None, calls_of(&app, job.task));
                        return;
                    }
                };
            let limits = JobLimits::for_plan(&plan);
            (
                Run::new(plan, Deduper::new(Vec::new(), Vec::new()), limits)
                    .with_hard_cap(hard_cap)
                    .with_unlimited(unlimited),
                Vec::new(),
                ex,
            )
        }
    };
    drop(lines);
    let by_id: HashMap<String, &Excerpt> = excerpts.iter().map(|e| (e.id.clone(), e)).collect();
    let driven = if let Some(w) = &whole {
        // A-OWNER-71: the writing calls start with the cached prefix and run up to four at a time
        let nonce = crate::util::hex(&random_bytes::<8>());
        let mut user_for = |run: &Run, batch: &[usize]| -> (String, Option<i64>) {
            let slots = run.slots(batch);
            match mode {
                Mode::Questions => (
                    whole::convert_user(&slots, &specs, &segs) + &run.retry_note(batch),
                    run.want_chars(batch)
                        .map(|c| crate::limits::chars_to_tokens(c as i64)),
                ),
                Mode::Material => (
                    whole::write_user(&slots, &by_id, &run.written(), &nonce)
                        + &run.retry_note(batch),
                    None,
                ),
            }
        };
        let cold = whole::cold(recognised_at, Instant::now());
        whole::drive(
            &ctx,
            w,
            &mut run,
            mode == Mode::Material,
            whole::WIDTH,
            cold,
            &mut user_for,
        )
        .await
    } else {
        let mut prompt_for = |run: &Run, batch: &[usize]| -> (Prompt, Option<i64>) {
            let slots = run.slots(batch);
            let nonce = crate::util::hex(&random_bytes::<8>());
            let note = run.retry_note(batch);
            match mode {
                Mode::Questions => (
                    with_note(convert::build_prompt(&slots, &specs, &nonce), &note),
                    run.want_chars(batch)
                        .map(|c| crate::limits::chars_to_tokens(c as i64)),
                ),
                Mode::Material => (
                    with_note(
                        material::build_prompt(&slots, &by_id, &run.written(), &nonce),
                        &note,
                    ),
                    None,
                ),
            }
        };
        drive(&ctx, &mut run, mode == Mode::Material, &mut prompt_for).await
    };
    drop(place);
    let calls = calls_of(&app, job.task) as usize;
    // A-OWNER-66.4: a cancel with "keep what is finished" ends the writing here and saves what is valid
    let kept = matches!(driven, Err(Stop::Cancelled(StopWhy::Keep)));
    match driven {
        Err(Stop::Cancelled(StopWhy::Keep)) => {}
        Err(Stop::Cancelled(_)) | Err(Stop::Shutdown) => return,
        Err(Stop::Fail(code)) => {
            finish_failed_with(&app, &job, code, None, calls as i64, &run_detail(&run));
            return;
        }
        Ok(()) => {}
    }
    if kept && run.made() == 0 {
        // nothing finished after all: an ordinary cancel (the unit stays charged, the job was writing)
        let now = unix_now();
        let _ = app.0.store.write(|tx| {
            jobs::finish(
                tx,
                job.task,
                &End {
                    state: "cancelled",
                    error: None,
                    charged: true,
                    result: None,
                    seen: true,
                },
                now,
            )
        });
        return;
    }
    let skipped: Option<Vec<usize>> = if kept {
        Some(run.skipped())
    } else {
        match run.outcome() {
            Outcome::Full => (mode == Mode::Questions).then(Vec::new),
            Outcome::Partial { skipped } => Some(skipped),
            Outcome::Failed(FailCode::Quality) => {
                finish_failed_with(
                    &app,
                    &job,
                    "ai.gen.quality",
                    None,
                    calls as i64,
                    &run_detail(&run),
                );
                return;
            }
            Outcome::Failed(FailCode::Provider(code)) => {
                finish_failed_with(
                    &app,
                    &job,
                    static_up_code(&code),
                    None,
                    calls as i64,
                    &run_detail(&run),
                );
                return;
            }
        }
    };
    // 6. saving (the compare-and-set on the phase decides against a cancel that came first)
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
    let payload = assemble::payload_file(
        &job.key,
        job.who.id,
        &job.who.stamp,
        &job.name,
        &job.file_name,
        &job.file_type,
        mode.as_str(),
        skipped.as_deref(),
        goal,
        &job.at,
        &questions,
    );
    match create_bank(&app, &ctx.cancel, &payload).await {
        Ok(reply) => {
            let mut result =
                json!({"bankId": reply.bank_id, "name": reply.name, "made": questions.len()});
            if let (Some(s), Some(o)) = (&skipped, result.as_object_mut()) {
                o.insert("skipped".into(), json!(s));
            }
            if let (Mode::Questions, Some(o)) = (mode, result.as_object_mut()) {
                // A-OWNER-47: "imported the first <taken> / <total>"
                o.insert("total".into(), json!(questions_total));
                o.insert("found".into(), json!(questions_total));
                o.insert("taken".into(), json!(goal));
                o.insert("approx".into(), json!(approx));
            }
            let now = unix_now();
            let n = app
                .0
                .store
                .write(|tx| -> rusqlite::Result<usize> {
                    tx.execute(
                        "UPDATE tasks SET made = ?2 WHERE id = ?1",
                        rusqlite::params![job.task, questions.len() as i64],
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
                    "user {:?} finished an AI file import ({} of {} questions, {} model calls)",
                    job.who.username,
                    questions.len(),
                    goal,
                    calls
                );
            }
        }
        Err(code) => finish_failed(&app, &job, code, None, calls as i64),
    }
}

fn human_size(n: usize) -> String {
    format!("{:.1} MB", n as f64 / 1_048_576.0)
}

fn calls_of(app: &AppState, task: i64) -> i64 {
    app.0
        .store
        .read(|c| jobs::get(c, task))
        .ok()
        .flatten()
        .map_or(0, |r| r.attempts)
}

fn finish_failed(
    app: &AppState,
    job: &ImportJob,
    code: &'static str,
    arg: Option<Value>,
    calls: i64,
) {
    finish_failed_with(app, job, code, arg, calls, "");
}

/// `finish_failed` with the run's facts (counts by reason code) for the System log line.
fn finish_failed_with(
    app: &AppState,
    job: &ImportJob,
    code: &'static str,
    arg: Option<Value>,
    calls: i64,
    detail: &str,
) {
    if fail_job(app, job.task, code, arg) {
        log_failed(
            app,
            &job.who,
            code,
            usize::try_from(calls).unwrap_or(0),
            detail,
        );
    }
}

/// What recognising found.
struct Recognised {
    mode: Mode,
    lines: Vec<String>,
    segs: Vec<Seg>,
    /// Questions in the file (>= `segs.len()`; more when the import takes only the first ones, A-OWNER-47)
    total: usize,
    /// The count is shown as "about N": the model and the local count of answer choices differ by more than a few
    approx: bool,
    /// The whole-file prefix (A-OWNER-71): every writing call starts with it; `None` = the windowed path ran
    whole: Option<Whole>,
    /// When recognising ended (the prompt cache of the prefix is that young)
    at: Instant,
}

/// Windows asked at the same time once the type is known (A-OWNER-38 item 2c).
const PARALLEL: usize = 3;
/// Windows asked first and together: the file's type is decided from them (item 2b).
const FIRST_WAVE: usize = 2;
/// Question-like numbered lines found locally from which the file is treated as a questions file without a
/// classification call.
const LOCAL_SIGNALS: usize = 3;
/// Calls kept back from the windows of a questions file: the classification and one retry.
const CEILING_SLACK: usize = 2;

/// The recognising step's shared state: the call ceiling, the provider-error streak and the first failure of a wave.
struct Rec<'a> {
    ctx: &'a Ctx,
    /// Calls sent so far / the ceiling (`recognizeMaxCalls`, everyone incl. administrators)
    used: AtomicUsize,
    max: usize,
    /// Provider errors in a row (three fail the job)
    fails: AtomicUsize,
    /// Set by the first failing window of a wave: its siblings start no new call
    abort: AtomicBool,
    first_err: Mutex<Option<Stop>>,
}

impl<'a> Rec<'a> {
    fn new(ctx: &'a Ctx, max: usize) -> Rec<'a> {
        Rec {
            ctx,
            used: AtomicUsize::new(0),
            max,
            fails: AtomicUsize::new(0),
            abort: AtomicBool::new(false),
            first_err: Mutex::new(None),
        }
    }

    /// One more call against the ceiling; a call over it is never sent.
    fn take(&self) -> Result<(), Stop> {
        if self.used.fetch_add(1, Ordering::SeqCst) >= self.max {
            self.used.fetch_sub(1, Ordering::SeqCst);
            return Err(Stop::Fail("ai.import.unclear"));
        }
        Ok(())
    }

    fn fail(&self, e: Stop) {
        let mut g = self.first_err.lock().unwrap_or_else(|x| x.into_inner());
        if g.is_none() {
            *g = Some(e);
        }
        self.abort.store(true, Ordering::SeqCst);
    }

    fn failed(&self) -> Option<Stop> {
        *self.first_err.lock().unwrap_or_else(|x| x.into_inner())
    }

    /// One recognising call with the provider-error rules: three errors in a row fail the job with the provider's code,
    /// the backoff grows by 15 s per error.
    async fn call(&self, prompt: &Prompt, want: i64) -> Result<String, Stop> {
        self.call_with(&prompt.system, None, &prompt.user, want)
            .await
    }

    /// `call` with a cached prefix (the whole-file prompt).
    async fn call_with(
        &self,
        system: &str,
        prefix: Option<&str>,
        user: &str,
        want: i64,
    ) -> Result<String, Stop> {
        let ctx = self.ctx;
        loop {
            ctx.check()?;
            if self.abort.load(Ordering::SeqCst) {
                return Err(self.failed().unwrap_or(Stop::Fail("ai.import.unclear")));
            }
            ctx.recheck().await?;
            let fails = self.fails.load(Ordering::SeqCst);
            if fails > 0 {
                let secs = (15 * fails as u64).min(60) as f64
                    * Knobs::get(&ctx.app.0.knobs.backoff_scale) as f64
                    / 100.0;
                ctx.sleep_checked(secs).await?;
            }
            let permit = ctx.place().await?;
            self.take()?;
            let r = ctx.call_with(system, prefix, user, want, 0).await;
            drop(permit);
            let r = r?;
            let (row, outcome) = match r {
                CallResult::Answer { text, row, .. } => {
                    self.fails.store(0, Ordering::SeqCst);
                    (row, Ok(text))
                }
                CallResult::Error { code, row } => {
                    let n = self.fails.fetch_add(1, Ordering::SeqCst) + 1;
                    let c: UpCode = code;
                    (
                        row,
                        Err((n >= 3).then(|| Stop::Fail(static_up_code(c.code())))),
                    )
                }
            };
            let alive = ctx.write_call(&row, None, None);
            if !alive {
                return Err(Stop::Cancelled(StopWhy::Purged));
            }
            match outcome {
                Ok(text) => return Ok(text),
                Err(Some(stop)) => return Err(stop),
                Err(None) => {}
            }
        }
    }

    /// Segments one window: one retry when the answer is unreadable or its ranges overlap; a second overlapping answer
    /// is resolved greedily.
    async fn segment_window(&self, lines: &[String], w: &Window) -> Result<Vec<Seg>, Stop> {
        for attempt in 0..2 {
            let nonce = crate::util::hex(&random_bytes::<8>());
            let prompt = segment::build_prompt(lines, w, &nonce);
            let want = crate::limits::chars_to_tokens(3000);
            let text = self.call(&prompt, want).await?;
            match segment::parse_answer(&text, w) {
                Some(a) if !a.overlaps => return Ok(a.segs),
                Some(a) if attempt == 1 => return Ok(segment::resolve_greedy(&a.segs)),
                Some(_) | None => {}
            }
        }
        Err(Stop::Fail("ai.import.unclear"))
    }

    /// Segments `wins` at the same time; the answers come back in window order. The first failure wins.
    async fn wave(
        &self,
        lines: &[String],
        wins: &[Window],
        progress: &Progress<'_>,
    ) -> Result<Vec<Vec<Seg>>, Stop> {
        let all = join_all(wins.iter().map(|w| async move {
            let r = self.segment_window(lines, w).await;
            match &r {
                Ok(_) => progress.bump(),
                Err(e) => self.fail(*e),
            }
            r
        }))
        .await;
        if let Some(e) = self.failed() {
            return Err(e);
        }
        all.into_iter().collect()
    }
}

/// The progress the card shows ("reading 3 / 9"): windows answered of all windows, one small payload update each.
struct Progress<'a> {
    app: &'a AppState,
    task: i64,
    done: AtomicUsize,
    total: usize,
}

impl Progress<'_> {
    fn write(&self) {
        let (done, total) = (self.done.load(Ordering::SeqCst), self.total);
        let now = unix_now();
        let _ = self.app.0.store.write(|tx| -> rusqlite::Result<()> {
            if let Some(row) = jobs::get(tx, self.task)? {
                let mut p = row.payload.clone();
                if let Some(o) = p.as_object_mut() {
                    o.insert("read".into(), json!({"done": done, "total": total}));
                }
                jobs::update_payload(tx, self.task, &p, None, now)?;
            }
            Ok(())
        });
    }

    fn bump(&self) {
        self.done.fetch_add(1, Ordering::SeqCst);
        self.write();
    }
}

/// Call 1 of the whole-file path: kind, language and every question's line range in one call. `Ok(None)` = the answer
/// stayed unreadable after one more try (the windowed path takes over).
async fn recognise_whole(
    rec: &Rec<'_>,
    whole: &Whole,
    lines: &[String],
    max_questions: usize,
    error_arg: &mut Option<Value>,
) -> Result<Option<Recognised>, Stop> {
    let user = whole::recognise_user();
    let mut want = whole::recognise_want(lines.len());
    let mut best: Option<whole::Recognition> = None;
    for _ in 0..2 {
        let text = rec
            .call_with(&whole.system, Some(&whole.prefix), &user, want)
            .await?;
        match whole::parse_recognition(&text, lines.len()) {
            Some(r) if !r.overlaps => {
                best = Some(r);
                break;
            }
            // overlapping ranges: one more try, then resolved greedily
            Some(r) => best = Some(r),
            None => {}
        }
        // a longer answer for the second try (the first may have been cut)
        want = (want * 2).min(32_000);
    }
    let Some(r) = best else {
        return Ok(None);
    };
    let at = Instant::now();
    let material = |at| Recognised {
        mode: Mode::Material,
        lines: lines.to_vec(),
        segs: Vec::new(),
        total: 0,
        approx: false,
        whole: Some(whole.clone()),
        at,
    };
    match r.class.verdict {
        Verdict::None => return Err(Stop::Fail("ai.import.noContent")),
        Verdict::Material => return Ok(Some(material(at))),
        Verdict::Questions => {}
    }
    let mut segs = if r.overlaps {
        segment::resolve_greedy(&r.segs)
    } else {
        r.segs
    };
    if segs.is_empty() {
        // the verdict said questions, the file has none: material mode, no extra call
        return Ok(Some(material(at)));
    }
    // the file's own count is exact (A-OWNER-70); the import takes the first `max_questions` (A-OWNER-47)
    let total = segs.len();
    segs.truncate(max_questions);
    if let Some(n) = segment::too_long(lines, &segs, MAX_QUESTION_CHARS) {
        *error_arg = Some(json!(n));
        return Err(Stop::Fail("ai.import.tooLongQuestion"));
    }
    Ok(Some(Recognised {
        mode: Mode::Questions,
        lines: lines.to_vec(),
        segs,
        total,
        approx: count::uncertain(total, lines),
        whole: Some(whole.clone()),
        at,
    }))
}

async fn recognise(
    app: &AppState,
    job: &ImportJob,
    doc: &Doc,
    cancel: &mut watch::Receiver<Option<StopWhy>>,
    deadline: Instant,
    error_arg: &mut Option<Value>,
) -> Result<Recognised, Stop> {
    let mut full = lines(doc);
    let local = count::count(&full);
    // The whole file as the cached prefix of every call, when it fits the model (A-OWNER-71)
    let candidate = {
        let nonce = crate::util::hex(&random_bytes::<8>());
        let w = whole::build(&full, &nonce);
        whole::fits(
            &w,
            whole::model_context(app),
            app.0.limits().import_tokens(),
        )
        .then_some(w)
    };
    // The windowed path (the file does not fit, or the one-call answer stayed unreadable). A-OWNER-47: a file with more
    // numbered questions than the limit is cut after the limit (no AI call for the rest)
    let keep = count::keep_lines(&full, job.max_questions).unwrap_or(full.len());
    let wl = &full[..keep];
    let total = total_chars(wl);
    // The ceiling of this step: the windows of a questions file must fit it (with the classification and one retry spare)
    let unlimited = app.0.limits().import_unlimited == 1;
    // unlimited: no call ceiling (the windows are then bounded by the text limit only)
    let ceiling = if unlimited {
        1000
    } else {
        usize::try_from(app.0.limits().recognize_max_calls)
            .unwrap_or(20)
            .max(1)
    };
    let cap_windows = ceiling.saturating_sub(CEILING_SLACK).max(1);
    let max_text = QUESTIONS_TEXT_MAX.min(cap_windows * WINDOW_MAIN_CHARS);
    let wins: Option<Vec<Window>> = if total <= max_text {
        windows(wl).ok().filter(|w| w.len() <= cap_windows)
    } else {
        None
    };
    let signals = local.question_like >= LOCAL_SIGNALS;
    if candidate.is_none() && signals && wins.is_none() {
        *error_arg = Some(json!(max_text));
        return Err(Stop::Fail("ai.import.tooMuchText"));
    }
    // a job place for the AI steps
    let Some(place) = wait_for_place(app, cancel).await else {
        return Err(Stop::Cancelled(StopWhy::Cancel));
    };
    let now = unix_now();
    let changed = app
        .0
        .store
        .write(|tx| jobs::mark_running(tx, job.task, now, false))
        .unwrap_or(0);
    if changed == 0 {
        return Err(Stop::Cancelled(StopWhy::Cancel));
    }
    let ctx = Ctx {
        app: app.clone(),
        task: job.task,
        who: job.who.clone(),
        feature: Feature::Import,
        purpose: "recognise",
        cancel: cancel.clone(),
        deadline,
        timeout_code: "ai.import.readTimeout",
    };
    let rec = Rec::new(&ctx, if unlimited { usize::MAX } else { ceiling });
    if let Some(w) = candidate {
        let one = Progress {
            app,
            task: job.task,
            done: AtomicUsize::new(0),
            total: 1,
        };
        one.write();
        if let Some(r) = recognise_whole(&rec, &w, &full, job.max_questions, error_arg).await? {
            one.bump();
            drop(place);
            return Ok(r);
        }
        // unreadable twice: the windows try
    }
    let mut found: Vec<Seg> = Vec::new();
    let mut seen = 0usize;
    let mut next = 0usize;
    let progress = Progress {
        app,
        task: job.task,
        done: AtomicUsize::new(0),
        total: wins.as_ref().map_or(0, Vec::len),
    };
    // 2b: the type is decided from the first windows when the numbered lines say "questions"
    if let (true, Some(w)) = (signals, wins.as_ref()) {
        progress.write();
        let first = FIRST_WAVE.min(w.len());
        for segs in rec.wave(wl, &w[..first], &progress).await? {
            seen = segment::merge(&mut found, &segs, job.max_questions);
        }
        next = if seen >= job.max_questions {
            w.len()
        } else {
            first
        };
    }
    if found.is_empty() {
        // classification: one call, one more when unreadable
        let sample = classify::sample(wl);
        let mut verdict = None;
        for _ in 0..2 {
            let nonce = crate::util::hex(&random_bytes::<8>());
            let prompt = classify::build_prompt(&sample, &nonce);
            let want = crate::limits::chars_to_tokens(800);
            let text = rec.call(&prompt, want).await?;
            if let Some(c) = classify::parse_answer(&text) {
                verdict = Some(c);
                break;
            }
        }
        let Some(verdict) = verdict else {
            return Err(Stop::Fail("ai.import.unclear"));
        };
        match verdict.verdict {
            Verdict::None => return Err(Stop::Fail("ai.import.noContent")),
            Verdict::Material => {
                drop(place);
                full.truncate(keep);
                return Ok(Recognised {
                    mode: Mode::Material,
                    lines: full,
                    segs: Vec::new(),
                    total: 0,
                    approx: false,
                    whole: None,
                    at: Instant::now(),
                });
            }
            Verdict::Questions => {}
        }
        if wins.is_none() {
            *error_arg = Some(json!(max_text));
            return Err(Stop::Fail("ai.import.tooMuchText"));
        }
        if !signals {
            progress.write();
        }
    }
    // the remaining windows, a few at a time (item 2c); never more calls than the ceiling
    if let Some(w) = wins.as_ref() {
        while next < w.len() {
            let end = (next + PARALLEL).min(w.len());
            for segs in rec.wave(wl, &w[next..end], &progress).await? {
                seen = segment::merge(&mut found, &segs, job.max_questions);
            }
            next = if seen >= job.max_questions {
                w.len()
            } else {
                end
            };
        }
    }
    drop(place);
    // the count check compares against the whole file, before the cut
    let uncut = full.clone();
    full.truncate(keep);
    if found.is_empty() {
        // the classification said questions, the file has none: material mode, no extra call
        return Ok(Recognised {
            mode: Mode::Material,
            lines: full,
            segs: Vec::new(),
            total: 0,
            approx: false,
            whole: None,
            at: Instant::now(),
        });
    }
    if let Some(n) = segment::too_long(&full, &found, MAX_QUESTION_CHARS) {
        *error_arg = Some(json!(n));
        return Err(Stop::Fail("ai.import.tooLongQuestion"));
    }
    // the number of questions in the file: exact when every window was read, else what the local count and the model saw
    let total_found = if seen >= job.max_questions {
        seen.max(local.question_like)
    } else {
        found.len()
    };
    let approx = count::uncertain(total_found, &uncut);
    Ok(Recognised {
        mode: Mode::Questions,
        lines: full,
        segs: found,
        total: total_found,
        approx,
        whole: None,
        at: Instant::now(),
    })
}

/// A prompt with the retry note appended to its user text (after everything that can be cached).
fn with_note(mut p: Prompt, note: &str) -> Prompt {
    p.user.push_str(note);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extraction_errors_map_to_import_codes() {
        for (e, code) in [
            (ExtractError::NoText, "ai.import.noText"),
            (ExtractError::Protected, "ai.import.protected"),
            (ExtractError::Corrupt, "ai.import.corrupt"),
            (ExtractError::TooComplex, "ai.import.tooComplex"),
            (ExtractError::BadEncoding, "ai.import.encoding"),
            (ExtractError::Timeout, "ai.import.readTimeout"),
            (ExtractError::Unsupported, "ai.import.type"),
            (ExtractError::Empty, "ai.import.empty"),
        ] {
            let (c, arg) = extract_failure(e, 400_000);
            assert_eq!(c, code);
            assert_eq!(static_code(&c), code);
            assert!(arg.is_none());
        }
        let (c, arg) = extract_failure(ExtractError::TooMuchText, 400_000);
        assert_eq!(
            (c.as_str(), arg),
            ("ai.import.tooMuchText", Some(json!(400_000)))
        );
    }
}
