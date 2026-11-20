//! The parallel writing driver shared by the whole-file import and the whole-source generation (v1.3): the calls of a job
//! start with a cached prefix that is byte-identical for every call of its chunk, so a provider with prompt caching
//! reads it from the cache after the first call; up to `width` calls are in flight together for the whole job, each
//! taking a place of the global gate.
//!
//! A batch is never mixed over chunks (the `Run` groups its slots), a second or later send of a batch ends with an
//! `attempt <k> <hex>` line after the cached prefix and asks the gateway to bypass its response cache, and a prefix below
//! the model's cache minimum goes out without cache markers (and without the alone-first wait).
use crate::gen::run::{attempt_line, Next, Run};
use crate::gen::runner::{
    log_failed_slots, random_bytes, CallOpts, CallResult, Ctx, Stop, StopWhy,
};
use crate::pace::{PaceCfg, Window};
use futures_util::stream::{FuturesUnordered, StreamExt};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use tokio::time::Instant;

/// The context size assumed when the provider's model listing names none.
pub const DEFAULT_CONTEXT_TOKENS: usize = 128_000;
/// The most output tokens kept back for an answer when the fit is checked, whatever the answer setting says.
pub const ANSWER_RESERVE_MAX: i64 = 16_000;
/// Tokens kept back for the task text after the prefix.
pub const TASK_SLACK_TOKENS: i64 = 2_000;
/// Writing calls in flight together.
pub const WIDTH: usize = 4;
/// Seconds after which the provider's prompt cache is taken as gone (it lives 5 minutes): the first writing call then goes
/// alone and the others follow when it has rewritten the cache.
pub const CACHE_WARM_SECS: u64 = 240;

/// What every call of a job (or of a chunk of it) starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Whole {
    /// The rules (the system text)
    pub system: String,
    /// The text inside the nonce fence (the first part of the user turn); the cache breakpoint sits after it
    pub prefix: String,
    /// The estimated tokens of both
    pub tokens: i64,
}

/// Does a prefix of `tokens` fit the model with the answer and the task text? `context` = the model's context in
/// tokens; `answer` = the answer setting of the job in tokens.
pub fn fits_tokens(tokens: i64, context: usize, answer: i64) -> bool {
    let context = i64::try_from(context).unwrap_or(i64::MAX);
    tokens.saturating_add(reserve(answer)) <= context
}

/// The tokens kept back for the answer and the task text.
pub fn reserve(answer: i64) -> i64 {
    answer.clamp(1, ANSWER_RESERVE_MAX) + TASK_SLACK_TOKENS
}

/// The model's context in tokens: the listing's figure for the route's model when the cached listing has one, else
/// [`DEFAULT_CONTEXT_TOKENS`]. A test hook can force a figure for the import (the generation reads the route only, see
/// [`route_context`]).
pub fn model_context(app: &crate::app::AppState, route: &crate::route::Route) -> usize {
    let forced = crate::app::Knobs::get(&app.0.knobs.import_context_tokens);
    if forced > 0 {
        return usize::try_from(forced).unwrap_or(usize::MAX);
    }
    route_context(route)
}

/// The context of the route's model: the model listing's figure, else [`DEFAULT_CONTEXT_TOKENS`].
pub fn route_context(route: &crate::route::Route) -> usize {
    route
        .context
        .and_then(|c| usize::try_from(c).ok())
        .filter(|c| *c > 0)
        .unwrap_or(DEFAULT_CONTEXT_TOKENS)
}

/// `true` when the prompt cache the first call made has probably expired by `now`.
pub fn cold(recognised: Instant, now: Instant) -> bool {
    now.saturating_duration_since(recognised) > Duration::from_secs(CACHE_WARM_SECS)
}

/// The settings of one driven job.
pub struct Par<'a> {
    /// The prefixes of the job: one per chunk (one for an import)
    pub wholes: &'a [Whole],
    /// Lower the batch size to what the provider's token ceiling allows (generation, material)
    pub cap_batches: bool,
    /// Calls in flight together
    pub width: usize,
    /// The cache has probably expired: the first call goes alone
    pub cold: bool,
    /// The answer size setting of the job's calls is the generation's (else the import's)
    pub generation: bool,
    /// Write the accepted questions of every call to `gen_items`
    pub items: bool,
    /// The JSON Schema every writing call asks the answer to follow (`None` = free text)
    pub schema: Option<&'static serde_json::Value>,
}

/// One writing call: which prefix, the user text after it (the rejection note included) and the answer size.
pub struct Step {
    pub whole: usize,
    pub user: String,
    /// `None` = the default of the limits
    pub want: Option<i64>,
}

/// How a writing call comes about.
pub type StepFn<'a> = dyn FnMut(&Run, &[usize]) -> Step + Send + 'a;

type Flight<'a> = Pin<Box<dyn Future<Output = (Vec<usize>, Result<CallResult, Stop>)> + Send + 'a>>;

/// The writing loop: like `runner::drive`, but up to `width` calls are in flight together. With `cold` (the cache has
/// probably expired) the first call goes alone. The safety stops of the run (call ceiling, errors in a row) are
/// honoured with the calls in flight counted.
pub async fn drive(
    ctx: &Ctx,
    par: &Par<'_>,
    run: &mut Run,
    step_for: &mut StepFn<'_>,
) -> Result<(), Stop> {
    let mut flying: FuturesUnordered<Flight<'_>> = FuturesUnordered::new();
    let r = drive_inner(ctx, par, run, step_for, &mut flying).await;
    if matches!(r, Err(Stop::Cancelled(_)) | Err(Stop::Shutdown)) {
        // the calls still in flight see the same signal and write their stopped rows
        let _ = tokio::time::timeout(Duration::from_secs(3), async {
            while flying.next().await.is_some() {}
        })
        .await;
    }
    r
}

async fn drive_inner<'a>(
    ctx: &'a Ctx,
    par: &'a Par<'a>,
    run: &mut Run,
    step_for: &mut StepFn<'_>,
    flying: &mut FuturesUnordered<Flight<'a>>,
) -> Result<(), Stop> {
    let app = &ctx.app;
    let mut finished = 0usize;
    let mut fp: Option<String> = None;
    loop {
        ctx.check()?;
        // fill the places
        loop {
            // the width shrinks to what the provider's per-minute limits allow (v1.9 pacing)
            let limit = if par.cold && finished == 0 {
                1
            } else {
                let l = app.0.limits();
                let want = if par.generation {
                    l.gen_tokens()
                } else {
                    l.import_tokens()
                };
                let est_one = par.wholes.iter().map(|w| w.tokens).max().unwrap_or(0)
                    + TASK_SLACK_TOKENS
                    + want.clamp(1, ANSWER_RESERVE_MAX);
                let cfg = PaceCfg::of(&l, crate::app::Knobs::get(&app.0.knobs.pace_scale));
                par.width.min(Window::width_for(&cfg, est_one)).max(1)
            };
            if flying.len() >= limit {
                break;
            }
            if !flying.is_empty() && (run.room() <= flying.len() || run.backoff_secs() > 0) {
                break;
            }
            ctx.recheck().await?;
            let l = app.0.limits();
            let default_want = if par.generation {
                l.gen_tokens()
            } else {
                l.import_tokens()
            };
            if par.cap_batches {
                let call_chars =
                    crate::limits::Limits::usize_of(crate::limits::tokens_to_chars(default_want));
                {
                    let ceiling = app.token_ceiling(&ctx.route, default_want);
                    let mut n = crate::gen::plan::per_call(call_chars, ceiling);
                    if let Some(m) = app.0.gen_per_call_cap(&ctx.route.key) {
                        n = n.min(m);
                    }
                    run.cap_per_call(n);
                    fp = Some(ctx.route.key.clone());
                }
            }
            let batch = match run.next() {
                Next::Finish => break,
                Next::Ask(b) => b,
            };
            if flying.is_empty() {
                let secs = run.backoff_secs() as f64
                    * crate::app::Knobs::get(&app.0.knobs.backoff_scale) as f64
                    / 100.0;
                ctx.sleep_checked(secs).await?;
            }
            let step = step_for(run, &batch);
            let whole = par
                .wholes
                .get(step.whole)
                .ok_or(Stop::Fail("ai.gen.stopped"))?;
            let attempt = run.attempt_of(&batch);
            let mut user = step.user;
            if attempt >= 2 {
                user.push_str(&attempt_line(
                    attempt,
                    &crate::util::hex(&random_bytes::<4>()),
                ));
            }
            let want = step.want.unwrap_or(default_want);
            let asked = batch.len();
            let schema = par.schema;
            flying.push(Box::pin(async move {
                let r = async {
                    ctx.send(
                        &whole.system,
                        Some(&whole.prefix),
                        &user,
                        want,
                        asked,
                        CallOpts {
                            schema,
                            ..CallOpts::for_attempt(attempt)
                        },
                    )
                    .await
                }
                .await;
                (batch, r)
            }));
        }
        let Some((batch, result)) = flying.next().await else {
            return Ok(());
        };
        finished += 1;
        match result? {
            CallResult::Answer {
                text,
                truncated,
                mut row,
            } => {
                let before = run.per_call();
                let was: Vec<bool> = batch.iter().map(|&i| run.is_filled(i)).collect();
                let report = run.on_answer(&batch, &text, truncated);
                if let Some(fp) = &fp {
                    if run.per_call() < before {
                        app.0.note_gen_per_call(fp, run.per_call());
                    }
                }
                row.accepted = i64::try_from(report.accepted).unwrap_or(0);
                log_failed_slots(ctx.task, run.calls(), &report);
                let made = i64::try_from(run.made()).unwrap_or(0);
                let items: Vec<(i64, String)> = if par.items {
                    batch
                        .iter()
                        .zip(&was)
                        .filter(|(_, w)| !**w)
                        .filter_map(|(&i, _)| {
                            let q = run.question_of(i)?;
                            let no = run.plan().slots.get(i)?.no;
                            Some((
                                i64::try_from(no).unwrap_or(0),
                                crate::gen::assemble::item_json(q).to_string(),
                            ))
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                if !ctx.write_call_items(&row, Some(made), Some("writing"), &items) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fit_check_keeps_the_answer_and_the_task_back() {
        assert!(fits_tokens(100_000, 128_000, 20_000));
        // the reserve is at most 16 000 + 2 000, whatever the answer setting says
        assert!(fits_tokens(110_000, 128_000, 20_000));
        assert!(!fits_tokens(110_001, 128_000, 20_000));
        assert_eq!(reserve(3_000), 5_000);
        assert_eq!(reserve(0), 2_001);
    }

    #[test]
    fn the_cache_is_warm_for_four_minutes() {
        let t0 = Instant::now();
        assert!(!cold(t0, t0 + Duration::from_secs(239)));
        assert!(cold(t0, t0 + Duration::from_secs(241)));
    }
}
