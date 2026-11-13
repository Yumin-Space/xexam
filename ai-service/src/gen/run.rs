//! The pure state machine of one generation job (GP-7). The runner (async, in ai-service) asks `next()` for a batch,
//! builds the prompt, calls the model and feeds the result back with `on_answer` or `on_provider_error`; this file
//! never does I/O and keeps no clock.
//!
//! Owner rule (A-OWNER-13c): a job always delivers exactly `goal` valid questions. Missing or invalid slots are asked
//! again in smaller batches until every slot is valid; the only way out without a bank is the safety stop (twice the
//! planned number of calls at the current batch size, or three provider errors in a row). There is no partial result.

use std::collections::{HashMap, VecDeque};

use crate::gen::assemble::{place, FinalQuestion};
use crate::gen::check::{check, Reject};
use crate::gen::dedup::Deduper;
use crate::gen::parse::{parse_answer, RawQuestion};
use crate::gen::plan::{Plan, Slot};
use crate::gen::rng::Rng;
use crate::import::convert::{
    batches as convert_batches, check_convert, place_convert, ConvertSpec,
};

/// The overall ceiling of a generation, in multiples of the planned calls (A-OWNER-31).
pub const GEN_CEILING: usize = 5;
/// Rejections a generation slot may collect: the first try and 4 retries.
pub const SLOT_REJECTS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobLimits {
    /// The ceiling on ANSWERED calls (provider errors never use it)
    pub max_calls: usize,
    pub max_fails_in_row: usize,
    /// Quality rejections one slot may have used up before the job stops (a generation: the first try plus 4 retries
    /// = 5 rejections; 0 = no per-slot budget, the conversion's own rule applies)
    pub max_slot_rejects: usize,
}

impl JobLimits {
    /// The safety stop at the start of a job (A-OWNER-31): provider errors stop it after three in a row; quality
    /// rejections have their own budget (up to 4 retries per slot, each with a fresh seed) and an overall ceiling of 5x
    /// the planned calls (the writing deadline is the third bound). The planned calls follow the batch size: whenever
    /// `per_call` is lowered (truncation, `Run::cap_per_call`), the run raises `max_calls` to `5 * ceil(goal / per_call)`
    /// if that is more (coordinator A-v11-Q5: one truncated answer must never fail a job).
    pub fn for_plan(plan: &Plan) -> JobLimits {
        JobLimits {
            max_calls: GEN_CEILING * plan.batches.len(),
            max_fails_in_row: 3,
            max_slot_rejects: SLOT_REJECTS,
        }
    }

    /// The safety stop of a conversion: three times the planned calls (IA-6), three provider errors in a row.
    pub fn for_convert(plan: &Plan) -> JobLimits {
        JobLimits {
            max_calls: 3 * plan.batches.len(),
            max_fails_in_row: 3,
            max_slot_rejects: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Slot indexes (into `Plan::slots`) to write in one call.
    Ask(Vec<usize>),
    Finish,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailCode {
    /// The safety stop came before every slot was valid.
    Quality,
    /// The last `ai.up.*` code, when the provider errors in a row were the stop.
    Provider(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Full,
    /// Convert policy only: some questions could not be converted (their numbers, in the file's order); at least one was
    Partial {
        skipped: Vec<usize>,
    },
    Failed(FailCode),
}

/// What one answer did. Numbers are slot numbers (`Slot::no`, 1-based), as the model wrote them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BatchReport {
    pub accepted: usize,
    pub rejected: Vec<(usize, Reject)>,
    /// Slots of the batch the answer did not deliver (absent, or an element that could not be read).
    pub missing: Vec<usize>,
    /// Every failed slot with its reason code (`missing`, `unreadable` or a check reason), for the per-call INFO line
    pub failed: Vec<(usize, &'static str)>,
    /// Slots given up by this answer (their third failed try; a generation: its own per-slot rule)
    pub given_up: Vec<usize>,
}

/// A rejection code in plain words, for the retry note.
fn plain_reason(code: &str) -> &'static str {
    match code {
        "unknownSlot" | "duplicateSlot" => "answer each listed slot exactly once, with its own slot number.",
        "optionCount" => "the number of options was not the one asked for; give exactly that many.",
        "correctCount" => "the number of correct options was not the one asked for; mark exactly that many.",
        "empty" => "a text was empty; fill every field.",
        "tooShort" => "a text was too short; write it out fully.",
        "tooLong" => "a text was too long; shorten it.",
        "language" => "a text was not in its language; write each language field in its own language.",
        "untranslated" => "the three languages were not really translated; give three different texts.",
        "dupOption" => "two options were the same; make every option distinct.",
        "positionalOption" => "an option pointed at a position (like \"all of the above\"); make each option stand alone.",
        "letterRef" => "a text referred to an option by its letter; describe the option instead.",
        "copy" => "the question copied its source too closely; write it in new words.",
        "duplicate" => "the question repeated another one; write a different question.",
        "notVerbatim" => "the text was not word for word from the file; copy the source's own words exactly.",
        "optionOrder" => "the options were not in the source's order; keep them exactly in the file's order.",
        "answerMismatch" => "the marked answer did not match the answer stated in the file; follow the file.",
        "trueFalse" => "a true/false question must have exactly the two fixed options; follow that form.",
        "missing" => "the slot was missing from the answer; answer it.",
        "unreadable" => "the answer could not be read; reply with the JSON object only.",
        _ => "the answer failed the checks; write it again carefully.",
    }
}

pub struct Run {
    plan: Plan,
    filled: Vec<Option<FinalQuestion>>,
    asked: Vec<bool>,
    in_queue: Vec<bool>,
    queue: VecDeque<usize>,
    rejects: Vec<u32>,
    /// The reason code of each slot's last failed try (for the retry note of its next request)
    last_why: Vec<Option<&'static str>>,
    calls: usize,
    /// Answered calls (a provider error is not one)
    answers: usize,
    /// Rejections by reason code, for the job's INFO line
    reasons: std::collections::BTreeMap<&'static str, usize>,
    fails_in_row: usize,
    per_call: usize,
    deduper: Deduper,
    limits: JobLimits,
    /// The hard per-job ceiling on answered calls from the AI settings (A-OWNER-36); never raised by the run
    hard_cap: usize,
    /// A-OWNER-45: no call ceiling and no per-slot retry cap (only 3 provider errors in a row and a cancel stop the job)
    unlimited: bool,
    last_error: Option<String>,
    written: Vec<String>,
    rng: Rng,
    /// Set by `new_convert`: the questions of a file are converted as they are (IA-6)
    convert: Option<Conv>,
}

/// The convert policy's state: the regions, the size budget, which slots were given up, and the output sizes asked for.
struct Conv {
    specs: Vec<ConvertSpec>,
    /// Estimated output characters per call
    budget: usize,
    skipped: Vec<bool>,
    /// Slots that missed in a multi-slot call: asked one per call from then on (A-OWNER-66.2)
    solo: Vec<bool>,
    /// A raised answer size in characters per slot after a truncation (0 = none)
    boost: Vec<usize>,
}

/// Failed tries after which a converted slot is given up (A-OWNER-66), with 无限制 on or off.
pub const GIVE_UP_TRIES: u32 = 3;

/// The longest answer a conversion call may ask for, in characters.
pub const CONVERT_WANT_MAX: usize = 32_000;

/// A seed for the tie-breaks of the run, derived from the plan itself (`Run::new` takes no seed): the same plan always
/// runs the same way.
fn plan_seed(plan: &Plan) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for s in &plan.slots {
        for b in s.seed.bytes().chain(s.no.to_le_bytes()) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

impl Run {
    pub fn new(plan: Plan, deduper: Deduper, limits: JobLimits) -> Run {
        let n = plan.slots.len();
        let per_call = plan.per_call.max(1);
        let rng = Rng::new(plan_seed(&plan));
        Run {
            plan,
            filled: vec![None; n],
            asked: vec![false; n],
            in_queue: vec![false; n],
            queue: VecDeque::new(),
            rejects: vec![0; n],
            last_why: vec![None; n],
            calls: 0,
            answers: 0,
            reasons: std::collections::BTreeMap::new(),
            fails_in_row: 0,
            per_call,
            deduper,
            limits,
            hard_cap: usize::MAX,
            unlimited: false,
            last_error: None,
            written: Vec::new(),
            rng,
            convert: None,
        }
    }

    /// Sets the hard ceiling on answered calls of this job (the AI-settings value, the same for everyone); the
    /// truncation rules that raise `limits.max_calls` can never lift it.
    pub fn with_hard_cap(mut self, cap: usize) -> Run {
        self.hard_cap = cap.max(1);
        self
    }

    /// Switches the overall call ceiling off (the `genUnlimited` / `importUnlimited` limits, A-OWNER-45). The per-slot
    /// give-up (A-OWNER-66) stays: a slot that fails 3 times (a generation: its 5 rejections) still ends or is skipped.
    /// Three provider errors in a row still end the job (`max_fails_in_row`), and the runner still honours a cancel.
    pub fn with_unlimited(mut self, on: bool) -> Run {
        self.unlimited = on;
        self
    }

    /// A run that converts the questions of a file as they are (IA-6): no de-duplication, no seed switch, batches cut by
    /// the output size budget, a slot given up after its third rejection. `plan` comes from `import::convert::convert_plan`.
    pub fn new_convert(
        plan: Plan,
        specs: Vec<ConvertSpec>,
        limits: JobLimits,
        budget_chars: usize,
    ) -> Run {
        let n = plan.slots.len();
        let mut run = Run::new(plan, Deduper::new(Vec::new(), Vec::new()), limits);
        run.convert = Some(Conv {
            specs,
            budget: budget_chars.max(1),
            skipped: vec![false; n],
            solo: vec![false; n],
            boost: vec![0; n],
        });
        run
    }

    /// The slots that are neither valid nor given up.
    fn open_slots(&self) -> Vec<usize> {
        (0..self.plan.slots.len())
            .filter(|&i| {
                self.filled[i].is_none() && self.convert.as_ref().is_none_or(|c| !c.skipped[i])
            })
            .collect()
    }

    /// The answer size a conversion batch asks for, in characters: a slot larger than the budget (or one that was cut
    /// before) asks for `min(est x 5 / 4, 32 000)`, raised by a half after each truncation. `None` = the default.
    pub fn want_chars(&self, batch: &[usize]) -> Option<usize> {
        let c = self.convert.as_ref()?;
        batch
            .iter()
            .filter_map(|&i| {
                let est = c.specs[i].est;
                if c.boost[i] > 0 {
                    Some(c.boost[i])
                } else if est > c.budget {
                    Some((est * 5 / 4).min(CONVERT_WANT_MAX))
                } else {
                    None
                }
            })
            .max()
    }

    /// The question numbers (`Slot::no`) that are not valid, in order: given up, or still open at the safety stop.
    pub fn skipped(&self) -> Vec<usize> {
        (0..self.plan.slots.len())
            .filter(|&i| self.filled[i].is_none())
            .map(|i| self.plan.slots[i].no)
            .collect()
    }

    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    /// The slots of a batch, for `prompt::build_prompt`.
    pub fn slots(&self, batch: &[usize]) -> Vec<&Slot> {
        batch
            .iter()
            .filter_map(|&i| self.plan.slots.get(i))
            .collect()
    }

    /// Smaller batches for regenerations: `max(1, ceil(per_call / 2))` (5 -> 3, 3 -> 2, 2 -> 1, 1 -> 1).
    pub fn redo_per_call(&self) -> usize {
        self.per_call.div_ceil(2).max(1)
    }

    fn all_filled(&self) -> bool {
        self.filled.iter().all(|f| f.is_some())
    }

    /// Every slot is valid, or (convert policy) valid or given up.
    fn finished(&self) -> bool {
        match &self.convert {
            Some(_) => self.open_slots().is_empty(),
            None => self.all_filled(),
        }
    }

    fn stopped(&self) -> bool {
        self.finished()
            || (!self.unlimited && self.answers >= self.limits.max_calls.min(self.hard_cap))
            || self.fails_in_row >= self.limits.max_fails_in_row
            || self.slot_budget_spent()
    }

    /// A generation slot that has used up its quality budget ends the job (the bank needs every slot).
    fn slot_budget_spent(&self) -> bool {
        let cap = self.limits.max_slot_rejects as u32;
        cap > 0
            && self.convert.is_none()
            && (0..self.plan.slots.len())
                .any(|i| self.filled[i].is_none() && self.rejects[i] >= cap)
    }

    /// Rejection counts by reason code (`Reject::code`, plus `missing` for a slot an answer did not deliver), sorted.
    pub fn reason_counts(&self) -> String {
        if self.reasons.is_empty() {
            return "none".to_string();
        }
        self.reasons
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The next batch: first the slots never asked, in `no` order, chunked by the current `per_call`; then the redo
    /// queue, chunked by `redo_per_call()`. `Finish` when every slot is valid or the safety stop is reached.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Next {
        if self.stopped() {
            return Next::Finish;
        }
        if self.convert.is_some() {
            return self.next_convert();
        }
        let fresh: Vec<usize> = (0..self.plan.slots.len())
            .filter(|&i| !self.asked[i] && self.filled[i].is_none())
            .take(self.per_call)
            .collect();
        if !fresh.is_empty() {
            for &i in &fresh {
                self.asked[i] = true;
            }
            return Next::Ask(fresh);
        }
        let take = self.redo_per_call();
        let mut batch = Vec::new();
        while batch.len() < take {
            let Some(i) = self.queue.pop_front() else {
                break;
            };
            self.in_queue[i] = false;
            if self.filled[i].is_none() {
                batch.push(i);
            }
        }
        if batch.is_empty() {
            Next::Finish
        } else {
            Next::Ask(batch)
        }
    }

    /// The convert policy's next batch: the slots never asked, cut by the size budget; then the redo queue with half the
    /// budget (at least one slot per call).
    fn next_convert(&mut self) -> Next {
        let Some(conv) = &self.convert else {
            return Next::Finish;
        };
        let est: Vec<usize> = conv.specs.iter().map(|s| s.est).collect();
        let budget = conv.budget;
        let fresh: Vec<usize> = self
            .open_slots()
            .into_iter()
            .filter(|&i| !self.asked[i])
            .collect();
        if !fresh.is_empty() {
            let mut first = convert_batches(&est, &fresh, budget)
                .into_iter()
                .next()
                .unwrap_or_default();
            solo_only(&mut first, conv);
            for &i in &first {
                self.asked[i] = true;
            }
            return Next::Ask(first);
        }
        let queued: Vec<usize> = self
            .queue
            .iter()
            .copied()
            .filter(|&i| self.filled[i].is_none() && !conv_skipped(&self.convert, i))
            .collect();
        let half = (budget / 2).max(1);
        let mut batch = convert_batches(&est, &queued, half)
            .into_iter()
            .next()
            .unwrap_or_default();
        solo_only(&mut batch, conv);
        if batch.is_empty() {
            return Next::Finish;
        }
        self.queue.retain(|i| !batch.contains(i));
        for &i in &batch {
            self.in_queue[i] = false;
        }
        Next::Ask(batch)
    }

    fn requeue(&mut self, i: usize) {
        if self.filled[i].is_none() && !self.in_queue[i] {
            self.in_queue[i] = true;
            self.queue.push_back(i);
        }
    }

    /// After a slot's second (and every further second) rejection its seed is switched to the least-used other seed of
    /// the plan with the same sheet and kind (ties: seeded random); none -> the slot keeps its seed.
    fn switch_seed(&mut self, idx: usize) {
        let (stratum, kind, current) = {
            let s = &self.plan.slots[idx];
            (s.stratum, s.kind, s.seed.clone())
        };
        let mut uses: HashMap<&str, usize> = HashMap::new();
        for (j, s) in self.plan.slots.iter().enumerate() {
            if j != idx {
                *uses.entry(s.seed.as_str()).or_insert(0) += 1;
            }
        }
        let mut pool: Vec<(String, usize)> = Vec::new();
        for s in &self.plan.slots {
            if s.stratum == stratum
                && s.kind == kind
                && s.seed != current
                && !pool.iter().any(|(id, _)| *id == s.seed)
            {
                pool.push((
                    s.seed.clone(),
                    uses.get(s.seed.as_str()).copied().unwrap_or(0),
                ));
            }
        }
        let Some(least) = pool.iter().map(|(_, n)| *n).min() else {
            return;
        };
        let tied: Vec<&(String, usize)> = pool.iter().filter(|(_, n)| *n == least).collect();
        let at = if tied.len() > 1 {
            self.rng.below(tied.len())
        } else {
            0
        };
        let chosen = tied[at].0.clone();
        self.plan.slots[idx].seed = chosen;
    }

    /// Files a failed try of a slot. `counted` tries use up the slot's budget; returns `true` when the slot is given up
    /// (a converted slot after `GIVE_UP_TRIES`, a generation slot at its own cap).
    fn reject(&mut self, idx: usize, counted: bool) -> bool {
        if counted {
            self.rejects[idx] += 1;
            if let Some(c) = &mut self.convert {
                // a slot's third failed try gives it up for good (no seed to switch: a region has no other seed)
                if self.rejects[idx] >= GIVE_UP_TRIES {
                    c.skipped[idx] = true;
                    return true;
                }
            } else {
                // A-OWNER-31: every retry of a slot gets a fresh seed
                self.switch_seed(idx);
                let cap = self.limits.max_slot_rejects as u32;
                if cap > 0 && self.rejects[idx] >= cap {
                    self.requeue(idx);
                    return true;
                }
            }
        }
        self.requeue(idx);
        false
    }

    /// A slot the answer did not deliver (`why`: `missing` or `unreadable`). Only a call of ONE slot counts it as a
    /// failed try (A-OWNER-66.2: a bigger call that returns nothing usable is first repeated one slot per call); a
    /// truncated answer counts only when the slot already asks for the longest answer.
    fn lack(
        &mut self,
        i: usize,
        why: &'static str,
        multi: bool,
        truncated: bool,
        report: &mut BatchReport,
    ) {
        if !truncated {
            *self.reasons.entry(why).or_insert(0) += 1;
            self.last_why[i] = Some(why);
        }
        let maxed = self
            .convert
            .as_ref()
            .is_none_or(|c| c.boost[i] >= CONVERT_WANT_MAX);
        let counted = !multi && (!truncated || maxed);
        if multi {
            if let Some(c) = &mut self.convert {
                c.solo[i] = true;
            }
        }
        let gave_up = self.reject(i, counted);
        report.missing.push(i + 1);
        report.failed.push((i + 1, why));
        if gave_up {
            report.given_up.push(i + 1);
        }
    }

    /// Feeds the model's answer for `batch` (slot indexes from `next()`). `truncated` = the provider cut the answer
    /// (`finish_reason: length` / `stop_reason: max_tokens`). Parses, checks, de-duplicates and places each question;
    /// rejected and missing slots go to the redo queue. A rejection counts toward the seed switch for any GP-4 / GP-5
    /// reason or for a slot missing from a parsed answer; an unparsable answer and truncation do not count.
    pub fn on_answer(&mut self, batch: &[usize], text: &str, truncated: bool) -> BatchReport {
        self.calls += 1;
        self.answers += 1;
        self.fails_in_row = 0;
        if truncated {
            self.shrink(batch);
        }
        let mut report = BatchReport::default();
        let parsed = match parse_answer(text) {
            Ok(list) => list,
            Err(_) => {
                for &i in batch {
                    self.lack(i, "unreadable", batch.len() > 1, truncated, &mut report);
                }
                if !truncated && batch.len() > 1 {
                    // nothing readable from a multi-slot call: most likely cut off without saying so (L-44)
                    self.shrink(batch);
                }
                return report;
            }
        };
        let mut seen: Vec<usize> = Vec::new();
        let mut unreadable: Vec<usize> = Vec::new();
        for element in parsed {
            match element {
                Ok(raw) => self.take(batch, raw, &mut seen, &mut report),
                // An unreadable element: its slot (when known and in the batch) is lacking below
                Err(no) => unreadable.push(no),
            }
        }
        for &i in batch {
            if self.filled[i].is_none()
                && !self.in_queue[i]
                && !report.rejected.iter().any(|(no, _)| *no == i + 1)
            {
                // a slot of a bigger call never uses up a try (L-44): the answer was probably cut, one slot per call
                // follows (A-OWNER-66)
                let why = if unreadable.contains(&self.plan.slots[i].no) {
                    "unreadable"
                } else {
                    "missing"
                };
                self.lack(i, why, batch.len() > 1, truncated, &mut report);
            }
        }
        if !truncated && batch.len() > 1 && !report.missing.is_empty() {
            // fewer slots than asked counts as truncation (L-44)
            self.shrink(batch);
        }
        report
    }

    fn take(
        &mut self,
        batch: &[usize],
        raw: RawQuestion,
        seen: &mut Vec<usize>,
        report: &mut BatchReport,
    ) {
        let Some(&idx) = batch.iter().find(|&&i| self.plan.slots[i].no == raw.slot) else {
            report.rejected.push((raw.slot, Reject::UnknownSlot));
            report.failed.push((raw.slot, Reject::UnknownSlot.code()));
            return;
        };
        if seen.contains(&raw.slot) {
            report.rejected.push((raw.slot, Reject::DuplicateSlot));
            report.failed.push((raw.slot, Reject::DuplicateSlot.code()));
            return;
        }
        seen.push(raw.slot);
        if self.filled[idx].is_some() {
            return;
        }
        let slot = self.plan.slots[idx].clone();
        if let Some(c) = &self.convert {
            let spec = c.specs[idx].clone();
            match check_convert(&raw, &slot, &spec) {
                Ok(()) => {
                    self.filled[idx] = Some(place_convert(&raw, &slot, &spec));
                    report.accepted += 1;
                }
                Err(why) => {
                    report.rejected.push((raw.slot, why));
                    report.failed.push((raw.slot, why.code()));
                    *self.reasons.entry(why.code()).or_insert(0) += 1;
                    self.last_why[idx] = Some(why.code());
                    if self.reject(idx, true) {
                        report.given_up.push(raw.slot);
                    }
                }
            }
            return;
        }
        let verdict = check(&raw, &slot).and_then(|_| {
            let options: Vec<_> = raw.options.iter().map(|o| o.text.clone()).collect();
            self.deduper.check(&raw.stem, &options)
        });
        match verdict {
            Ok(()) => {
                let fin = place(&raw, &slot);
                self.deduper.accept(&fin.stem, &fin.options);
                self.written.push(fin.stem.en.clone());
                self.filled[idx] = Some(fin);
                report.accepted += 1;
            }
            Err(why) => {
                report.rejected.push((raw.slot, why));
                report.failed.push((raw.slot, why.code()));
                *self.reasons.entry(why.code()).or_insert(0) += 1;
                self.last_why[idx] = Some(why.code());
                if self.reject(idx, true) {
                    report.given_up.push(raw.slot);
                }
            }
        }
    }

    /// A provider error (`ai.up.*` code) for `batch`: the slots go back to the redo queue, nothing counts against them.
    pub fn on_provider_error(&mut self, batch: &[usize], code: &str) {
        self.calls += 1;
        self.fails_in_row += 1;
        self.last_error = Some(code.to_string());
        for &i in batch {
            self.requeue(i);
        }
    }

    /// Truncation (a cut-off or short answer for a batch of more than one slot, or an explicit finish reason `length`):
    /// the generation's batch ladder steps 5 -> 3 -> 2 -> 1; a conversion halves its output budget.
    fn shrink(&mut self, batch: &[usize]) {
        if self.convert.is_some() {
            self.truncated_convert(batch);
        } else {
            self.lower_per_call(self.per_call.div_ceil(2));
        }
    }

    /// Questions per call now (lowered by truncation and by `cap_per_call`, never raised).
    pub fn per_call(&self) -> usize {
        self.per_call
    }

    /// A conversion answer was cut: the output budget is halved (one slot per call at the least), the slots of the batch
    /// ask for half as many characters again (at most 32 000), and the safety stop follows the new batch size
    /// (`max_calls = max(max_calls, 3 x the batches the open slots need at the new budget)`).
    fn truncated_convert(&mut self, batch: &[usize]) {
        let open = self.open_slots();
        let Some(c) = &mut self.convert else {
            return;
        };
        c.budget = (c.budget / 2).max(1);
        for &i in batch {
            let now = if c.boost[i] > 0 {
                c.boost[i]
            } else {
                c.specs[i].est * 5 / 4
            };
            c.boost[i] = (now * 3 / 2).min(CONVERT_WANT_MAX);
        }
        let est: Vec<usize> = c.specs.iter().map(|s| s.est).collect();
        let needed = convert_batches(&est, &open, c.budget).len();
        self.limits.max_calls = self.limits.max_calls.max(3 * needed);
    }

    /// Lowers `per_call` (never raises it; minimum 1): a ceiling learned meanwhile re-chunks the planned slots too. The
    /// convert policy sizes its batches by output characters instead and ignores it.
    pub fn cap_per_call(&mut self, n: usize) {
        if self.convert.is_none() {
            self.lower_per_call(n);
        }
    }

    /// Lowers `per_call` to `n` (minimum 1; never raises it). The safety stop follows the batch size: `max_calls`
    /// becomes `5 * ceil(goal / per_call)` when that is more than before (coordinator A-v11-Q5).
    fn lower_per_call(&mut self, n: usize) {
        let n = n.max(1);
        if n < self.per_call {
            self.per_call = n;
            let planned = self.plan.slots.len().div_ceil(n);
            self.limits.max_calls = self.limits.max_calls.max(GEN_CEILING * planned);
        }
    }

    /// Seconds to wait before the next call: after provider errors `min(15 x errors in a row, 60)`, else 0.
    pub fn backoff_secs(&self) -> u64 {
        (15 * self.fails_in_row as u64).min(60)
    }

    /// Answered calls the safety stop and the ceiling still allow (`usize::MAX` for an unlimited job). A driver with
    /// several calls in flight counts them against this before it asks for another batch.
    pub fn room(&self) -> usize {
        if self.unlimited {
            usize::MAX
        } else {
            self.limits
                .max_calls
                .min(self.hard_cap)
                .saturating_sub(self.answers)
        }
    }

    /// Calls sent so far (answers and provider errors).
    pub fn calls(&self) -> usize {
        self.calls
    }

    /// Valid questions so far.
    pub fn made(&self) -> usize {
        self.filled.iter().filter(|f| f.is_some()).count()
    }

    pub fn goal(&self) -> usize {
        self.plan.slots.len()
    }

    /// The retry note of a batch (A-COORD-75): for every slot that already failed a try, its try number and the last
    /// rejection in plain words. It goes AFTER the cached prefix of a request, so a retry never repeats the first
    /// request byte for byte (a gateway's response cache would answer it with the same rejected answer). Empty when
    /// no slot of the batch has failed yet.
    pub fn retry_note(&self, batch: &[usize]) -> String {
        let mut out = String::new();
        for &i in batch {
            let Some(slot) = self.plan.slots.get(i) else {
                continue;
            };
            let Some(why) = self.last_why[i].filter(|_| self.rejects[i] > 0) else {
                continue;
            };
            out.push_str(&format!(
                "slot {}, try {}: the last answer was rejected - {}\n",
                slot.no,
                self.rejects[i] + 1,
                plain_reason(why)
            ));
        }
        if out.is_empty() {
            out
        } else {
            format!("\n\nRETRY\n{out}")
        }
    }

    /// The ALREADY WRITTEN list: the English stems accepted so far, oldest first.
    pub fn written(&self) -> Vec<String> {
        self.written.clone()
    }

    /// Read after `next()` returned `Finish`.
    pub fn outcome(&self) -> Outcome {
        if self.convert.is_some() {
            if self.all_filled() {
                return Outcome::Full;
            }
            if self.fails_in_row >= self.limits.max_fails_in_row {
                return Outcome::Failed(FailCode::Provider(
                    self.last_error.clone().unwrap_or_default(),
                ));
            }
            return if self.made() > 0 {
                Outcome::Partial {
                    skipped: self.skipped(),
                }
            } else {
                Outcome::Failed(FailCode::Quality)
            };
        }
        if self.all_filled() {
            Outcome::Full
        } else if self.fails_in_row >= self.limits.max_fails_in_row {
            Outcome::Failed(FailCode::Provider(
                self.last_error.clone().unwrap_or_default(),
            ))
        } else {
            Outcome::Failed(FailCode::Quality)
        }
    }

    /// The accepted questions in slot order; exactly `goal` when the outcome is `Full`.
    pub fn questions(&self) -> Vec<FinalQuestion> {
        self.filled.iter().flatten().cloned().collect()
    }
}

/// A batch holding a slot that missed in a bigger call shrinks to that one slot (A-OWNER-66.2).
fn solo_only(batch: &mut Vec<usize>, conv: &Conv) {
    if batch.len() > 1 {
        if let Some(&i) = batch.iter().find(|&&i| conv.solo[i]) {
            *batch = vec![i];
        }
    }
}

fn conv_skipped(c: &Option<Conv>, i: usize) -> bool {
    c.as_ref().is_some_and(|c| c.skipped[i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen::plan::{plan, PlanInput};
    use crate::gen::testkit::*;
    use crate::gen::{Composition, Tri};

    fn make_plan(single: usize, multi: usize, per_call: usize, seed: u64) -> Plan {
        plan(&PlanInput {
            items: &items3(),
            strata: &strata3(),
            comp: Composition { single, multi },
            per_call,
            seed,
        })
        .unwrap()
    }

    fn make_run(single: usize, multi: usize, per_call: usize) -> Run {
        let p = make_plan(single, multi, per_call, 17);
        let source: Vec<Tri> = items3().into_iter().map(|i| i.stem).collect();
        let limits = JobLimits::for_plan(&p);
        Run::new(p, Deduper::new(source, vec![]), limits)
    }

    /// The answer of a well-behaved model for the slots of a batch (question number = slot number).
    fn good(run: &Run, batch: &[usize]) -> String {
        let raws: Vec<RawQuestion> = batch
            .iter()
            .map(|&i| raw_for(&run.plan().slots[i], run.plan().slots[i].no))
            .collect();
        answer_json(&raws)
    }

    /// Drives the run with a model that answers every batch correctly.
    fn drive_good(run: &mut Run) -> usize {
        let mut calls = 0;
        while let Next::Ask(b) = run.next() {
            let t = good(run, &b);
            run.on_answer(&b, &t, false);
            calls += 1;
        }
        calls
    }

    #[test]
    fn the_hard_cap_binds_even_when_truncation_raises_the_safety_stop() {
        let mut run = make_run(14, 6, 5).with_hard_cap(3);
        run.cap_per_call(1);
        assert!(run.limits.max_calls > 3);
        let mut calls = 0;
        while let Next::Ask(b) = run.next() {
            let t = good(&run, &b);
            run.on_answer(&b, &t, false);
            calls += 1;
        }
        assert_eq!(calls, 3, "the settings ceiling stops the job");
    }

    #[test]
    fn a_retry_note_names_the_try_and_the_last_reason_and_is_empty_before_any_failure() {
        let mut run = make_run(14, 6, 5);
        let Next::Ask(b) = run.next() else { panic!() };
        assert_eq!(run.retry_note(&b), "");
        let mut raws: Vec<RawQuestion> = b
            .iter()
            .map(|&i| raw_for(&run.plan().slots[i], run.plan().slots[i].no))
            .collect();
        raws[2].stem.zh = "只有中文标点".into(); // too short
        run.on_answer(&b, &answer_json(&raws), false);
        let note = run.retry_note(&b);
        assert!(
            note.starts_with(
                "

RETRY
"
            ),
            "{note}"
        );
        assert!(note.contains("slot 3, try 2:"), "{note}");
        assert!(note.contains("too short"), "{note}");
        assert_eq!(note.matches("slot ").count(), 1, "only the failed slot");
    }

    #[test]
    fn limits_are_five_times_the_planned_calls() {
        let p = make_plan(14, 6, 5, 1);
        assert_eq!(
            JobLimits::for_plan(&p),
            JobLimits {
                max_calls: 20,
                max_fails_in_row: 3,
                max_slot_rejects: 5
            }
        );
        let q = make_plan(14, 6, 3, 1);
        assert_eq!(JobLimits::for_plan(&q).max_calls, 35);
    }

    #[test]
    fn a_good_model_fills_everything_in_the_planned_calls() {
        let mut run = make_run(14, 6, 5);
        assert_eq!(drive_good(&mut run), 4);
        assert_eq!(run.outcome(), Outcome::Full);
        assert_eq!(run.made(), 20);
        assert_eq!(run.questions().len(), 20);
        assert_eq!(run.calls(), 4);
        assert_eq!(run.written().len(), 20);
        assert_eq!(run.next(), Next::Finish);
    }

    #[test]
    fn questions_keep_slot_order_kinds_and_planned_letters() {
        let mut run = make_run(14, 6, 5);
        drive_good(&mut run);
        let qs = run.questions();
        for (q, s) in qs.iter().zip(&run.plan().slots) {
            assert_eq!(q.kind, s.kind);
            assert_eq!(q.options.len(), s.opts as usize);
            let letters: Vec<String> = s
                .correct
                .iter()
                .map(|&p| ((b'A' + p) as char).to_string())
                .collect();
            assert_eq!(q.answer, letters);
        }
    }

    #[test]
    fn first_batches_follow_per_call_then_the_redo_queue_is_smaller() {
        let mut run = make_run(14, 6, 5);
        assert_eq!(run.redo_per_call(), 3);
        let Next::Ask(b1) = run.next() else { panic!() };
        assert_eq!(b1, vec![0, 1, 2, 3, 4]);
        // Only slot 1 of the five is delivered: a short answer counts as truncation (L-44), so per_call goes 5 -> 3.
        let raws = vec![raw_for(&run.plan().slots[0].clone(), 1)];
        run.on_answer(&b1, &answer_json(&raws), false);
        assert_eq!(run.made(), 1);
        assert_eq!((run.per_call(), run.redo_per_call()), (3, 2));
        // The never-asked slots come first, 3 at a time.
        for first in [5usize, 8, 11, 14] {
            let Next::Ask(b) = run.next() else { panic!() };
            assert_eq!(b, vec![first, first + 1, first + 2]);
            let t = good(&run, &b);
            run.on_answer(&b, &t, false);
        }
        let Next::Ask(b) = run.next() else { panic!() };
        assert_eq!(b, vec![17, 18, 19]);
        let t = good(&run, &b);
        run.on_answer(&b, &t, false);
        // Now the four missing slots of the first batch, 2 + 2.
        let Next::Ask(r1) = run.next() else { panic!() };
        assert_eq!(r1, vec![1, 2]);
        let t = good(&run, &r1);
        run.on_answer(&r1, &t, false);
        let Next::Ask(r2) = run.next() else { panic!() };
        assert_eq!(r2, vec![3, 4]);
        let t = good(&run, &r2);
        run.on_answer(&r2, &t, false);
        assert_eq!(run.next(), Next::Finish);
        assert_eq!(run.outcome(), Outcome::Full);
        assert_eq!(run.calls(), 8);
    }

    #[test]
    fn a_bad_question_is_redone_and_the_bank_still_has_every_slot() {
        let mut run = make_run(14, 6, 5);
        let Next::Ask(b) = run.next() else { panic!() };
        let mut raws: Vec<RawQuestion> = b
            .iter()
            .map(|&i| raw_for(&run.plan().slots[i], run.plan().slots[i].no))
            .collect();
        raws[2].stem.zh = "只有中文标点".into(); // too short
        let rep = run.on_answer(&b, &answer_json(&raws), false);
        assert_eq!(rep.accepted, 4);
        assert_eq!(rep.rejected, vec![(3, Reject::TooShort)]);
        assert!(rep.missing.is_empty());
        while let Next::Ask(b) = run.next() {
            let t = good(&run, &b);
            run.on_answer(&b, &t, false);
        }
        assert_eq!(run.outcome(), Outcome::Full);
        assert_eq!(run.made(), 20);
    }

    #[test]
    fn no_slot_is_ever_given_up_until_the_safety_stop() {
        // A model that never produces a valid question: every slot is asked again until the safety stop (5 x 4 calls, or a slot's 5th rejection).
        let mut run = make_run(14, 6, 5);
        let mut asked = 0;
        while let Next::Ask(b) = run.next() {
            run.on_answer(&b, r#"{"questions":[]}"#, false);
            asked += 1;
        }
        assert_eq!(asked, run.calls());
        // missing slots use no retries (L-44), so the batch ladder runs 5 -> 3 -> 2 -> 1 and the stop is 5 x 20 calls
        assert!(asked <= 100);
        assert_eq!(run.made(), 0);
        assert_eq!(run.outcome(), Outcome::Failed(FailCode::Quality));
        assert_eq!(run.next(), Next::Finish);
    }

    #[test]
    fn a_model_that_delivers_one_slot_per_call_stays_within_five_times_the_slot_count() {
        for per_call in [5usize, 3] {
            let mut run = make_run(14, 6, per_call);
            let mut calls = 0;
            while let Next::Ask(b) = run.next() {
                // Valid for the first slot of every batch only; the ladder ends at one slot per call.
                let first = raw_for(&run.plan().slots[b[0]].clone(), run.plan().slots[b[0]].no);
                run.on_answer(&b, &answer_json(&[first]), false);
                calls += 1;
                assert!(calls <= 5 * run.goal());
            }
            assert_eq!(run.calls(), calls);
            assert_eq!(run.outcome(), Outcome::Full);
            assert_eq!(run.made(), 20);
        }
    }

    #[test]
    fn invalid_questions_are_regenerated_in_smaller_batches_until_valid() {
        // A model that gets the first answer of a slot wrong, then right.
        let mut run = make_run(14, 6, 5);
        let mut attempts: HashMap<usize, u32> = HashMap::new();
        let mut sizes: Vec<usize> = Vec::new();
        while let Next::Ask(b) = run.next() {
            sizes.push(b.len());
            let raws: Vec<RawQuestion> = b
                .iter()
                .map(|&i| {
                    let slot = run.plan().slots[i].clone();
                    let mut r = raw_for(&slot, slot.no);
                    let n = attempts.entry(i).or_insert(0);
                    *n += 1;
                    if *n == 1 && i % 3 == 0 {
                        r.options.pop();
                    }
                    r
                })
                .collect();
            run.on_answer(&b, &answer_json(&raws), false);
        }
        assert_eq!(run.outcome(), Outcome::Full);
        assert_eq!(run.questions().len(), 20);
        assert!(sizes.iter().skip(4).all(|&s| s <= 3), "{sizes:?}");
        assert!(run.calls() <= 8);
    }

    #[test]
    fn provider_errors_requeue_the_batch_and_back_off() {
        let mut run = make_run(14, 6, 5);
        let Next::Ask(b) = run.next() else { panic!() };
        assert_eq!(run.backoff_secs(), 0);
        run.on_provider_error(&b, "ai.up.rate");
        assert_eq!(run.backoff_secs(), 15);
        assert_eq!(run.calls(), 1);
        let Next::Ask(b2) = run.next() else { panic!() };
        assert_eq!(b2, vec![5, 6, 7, 8, 9]);
        run.on_provider_error(&b2, "ai.up.rate");
        assert_eq!(run.backoff_secs(), 30);
        // A good answer resets the streak.
        let Next::Ask(b3) = run.next() else { panic!() };
        let t = good(&run, &b3);
        run.on_answer(&b3, &t, false);
        assert_eq!(run.backoff_secs(), 0);
    }

    #[test]
    fn backoff_is_capped_at_sixty() {
        let p = make_plan(14, 6, 5, 3);
        let limits = JobLimits {
            max_calls: 100,
            max_fails_in_row: 100,
            max_slot_rejects: 5,
        };
        let mut run = Run::new(p, Deduper::new(vec![], vec![]), limits);
        for k in 1..=6u64 {
            let Next::Ask(b) = run.next() else { panic!() };
            run.on_provider_error(&b, "ai.up.down");
            assert_eq!(run.backoff_secs(), (15 * k).min(60));
        }
    }

    #[test]
    fn three_provider_errors_in_a_row_fail_with_the_last_code() {
        let mut run = make_run(14, 6, 5);
        for code in ["ai.up.a", "ai.up.b", "ai.up.c"] {
            let Next::Ask(b) = run.next() else { panic!() };
            run.on_provider_error(&b, code);
        }
        assert_eq!(run.next(), Next::Finish);
        assert_eq!(
            run.outcome(),
            Outcome::Failed(FailCode::Provider("ai.up.c".into()))
        );
        assert_eq!(run.calls(), 3);
        assert!(run.questions().is_empty());
    }

    #[test]
    fn errors_that_are_not_in_a_row_do_not_stop_the_job() {
        let mut run = make_run(14, 6, 5);
        let mut flip = false;
        while let Next::Ask(b) = run.next() {
            flip = !flip;
            if flip && run.calls() < 4 {
                run.on_provider_error(&b, "ai.up.rate");
            } else {
                let t = good(&run, &b);
                run.on_answer(&b, &t, false);
            }
        }
        assert_eq!(run.outcome(), Outcome::Full);
    }

    #[test]
    fn truncation_halves_per_call_and_requeues_without_counting() {
        let mut run = make_run(14, 6, 5);
        let Next::Ask(b) = run.next() else { panic!() };
        assert_eq!(b.len(), 5);
        run.on_answer(
            &b,
            "{\"questions\":[{\"slot\":1,\"stem\":{\"en\":\"cut",
            true,
        );
        assert_eq!(run.per_call, 3);
        assert_eq!(run.redo_per_call(), 2);
        // The planned slots are chunked by the new per_call (3).
        let Next::Ask(b2) = run.next() else { panic!() };
        assert_eq!(b2, vec![5, 6, 7]);
        let t = good(&run, &b2);
        run.on_answer(&b2, &t, false);
        // The truncated slots never counted as rejected.
        assert!(run.rejects.iter().all(|&r| r == 0));
    }

    #[test]
    fn one_truncated_answer_never_fails_the_job() {
        // A-v11-Q5: whichever call is cut, a model that answers every other call well delivers the whole bank.
        for at in 0..4 {
            let mut run = make_run(14, 6, 5);
            assert_eq!(run.limits.max_calls, 20);
            let mut k = 0;
            while let Next::Ask(b) = run.next() {
                if k == at {
                    run.on_answer(&b, "{\"questions\":[{\"slot\":", true);
                    assert_eq!(run.limits.max_calls, 35, "5 x ceil(20 / 3)");
                } else {
                    let t = good(&run, &b);
                    run.on_answer(&b, &t, false);
                }
                k += 1;
            }
            assert_eq!(run.outcome(), Outcome::Full, "truncated at call {at}");
            assert_eq!(run.questions().len(), 20);
            assert!(run.calls() <= 35);
        }
    }

    #[test]
    fn the_safety_stop_follows_a_lowered_batch_size_and_never_shrinks() {
        let mut run = make_run(14, 6, 5);
        run.cap_per_call(5);
        assert_eq!(
            run.limits.max_calls, 20,
            "no change without a lower per_call"
        );
        run.cap_per_call(1);
        assert_eq!(run.limits.max_calls, 100);
        run.cap_per_call(3);
        assert_eq!((run.per_call, run.limits.max_calls), (1, 100));
        // A provider that cuts every answer is still stopped: at 5 x 20 answered calls once per_call is 1.
        let mut run = make_run(14, 6, 5);
        let mut calls = 0;
        while let Next::Ask(b) = run.next() {
            run.on_answer(&b, "{", true);
            calls += 1;
            assert!(calls <= 100);
        }
        // a cut answer of ONE slot counts as a failed try (A-OWNER-66), so the job may stop before the ceiling
        assert_eq!(run.per_call, 1);
        assert!(run.calls() <= 100);
        assert_eq!(run.made(), 0);
        assert_eq!(run.outcome(), Outcome::Failed(FailCode::Quality));
    }

    /// A stub provider that silently cuts every answer after `limit` questions (no finish reason): 20 valid questions
    /// within 15 calls (L-44).
    #[test]
    fn a_provider_that_cuts_answers_still_gives_every_question_within_the_call_ceiling() {
        let p = make_plan(14, 6, 5, 17);
        let source: Vec<Tri> = items3().into_iter().map(|i| i.stem).collect();
        let limits = JobLimits::for_plan(&p);
        let mut run = Run::new(p, Deduper::new(source, vec![]), limits).with_hard_cap(15);
        let mut sizes = Vec::new();
        while let Next::Ask(b) = run.next() {
            sizes.push(b.len());
            let kept: Vec<usize> = b.iter().copied().take(3).collect();
            let t = good(&run, &kept);
            run.on_answer(&b, &t, false);
        }
        assert_eq!(run.outcome(), Outcome::Full, "{sizes:?}");
        assert_eq!(run.made(), 20);
        assert!(run.calls() <= 15, "{sizes:?}");
        assert_eq!(&sizes[..2], &[5, 3], "the ladder starts 5 -> 3");
        // missing slots never used up a retry
        assert!(run.rejects.iter().all(|&r| r == 0));
    }

    #[test]
    fn the_ladder_is_five_three_two_one() {
        let mut run = make_run(14, 6, 5);
        let mut seen = vec![run.per_call()];
        for _ in 0..4 {
            let Next::Ask(b) = run.next() else { panic!() };
            run.on_answer(&b, "{\"questions\":[]}", false);
            seen.push(run.per_call());
        }
        assert_eq!(seen, vec![5, 3, 2, 1, 1]);
    }

    #[test]
    fn unlimited_has_no_call_ceiling_but_keeps_the_per_slot_rule_and_stops_after_three_errors() {
        let mut run = make_run(14, 6, 5).with_hard_cap(8).with_unlimited(true);
        let mut asked = 0;
        // one slot is always rejected as too short, the others stay missing
        while let Next::Ask(b) = run.next() {
            asked += 1;
            assert!(asked < 300, "the per-slot rule must end the job");
            let mut raws: Vec<RawQuestion> = b
                .iter()
                .map(|&i| raw_for(&run.plan().slots[i], run.plan().slots[i].no))
                .collect();
            raws[0].stem.zh = "短".into();
            run.on_answer(&b, &answer_json(&raws[..1]), false);
        }
        assert!(asked > 8, "no call ceiling: {asked}");
        assert!(run.made() < 20);
        assert_eq!(run.outcome(), Outcome::Failed(FailCode::Quality));
        let mut run = make_run(14, 6, 5).with_hard_cap(8).with_unlimited(true);
        // three provider errors in a row still end it
        for _ in 0..3 {
            let Next::Ask(b) = run.next() else { panic!() };
            run.on_provider_error(&b, "ai.up.rate");
        }
        assert_eq!(run.next(), Next::Finish);
        assert_eq!(
            run.outcome(),
            Outcome::Failed(FailCode::Provider("ai.up.rate".into()))
        );
        // the same run without the switch stops at its ceiling
        let mut run = make_run(14, 6, 5).with_hard_cap(8);
        let mut n = 0;
        while let Next::Ask(b) = run.next() {
            run.on_answer(&b, "{\"questions\":[]}", false);
            n += 1;
        }
        assert_eq!(n, 8);
    }

    #[test]
    fn truncation_never_goes_below_one() {
        let mut run = make_run(14, 6, 5);
        for _ in 0..4 {
            let Next::Ask(b) = run.next() else { panic!() };
            run.on_answer(&b, "{", true);
        }
        assert_eq!(run.per_call, 1);
        assert_eq!(run.redo_per_call(), 1);
    }

    #[test]
    fn cap_per_call_rechunks_planned_slots_and_never_raises() {
        let mut run = make_run(14, 6, 5);
        run.cap_per_call(2);
        let Next::Ask(b) = run.next() else { panic!() };
        assert_eq!(b, vec![0, 1]);
        run.cap_per_call(9);
        let Next::Ask(b2) = run.next() else { panic!() };
        assert_eq!(b2.len(), 2);
        run.cap_per_call(0);
        let Next::Ask(b3) = run.next() else { panic!() };
        assert_eq!(b3.len(), 1);
    }

    #[test]
    fn the_seed_switches_after_every_rejection_of_a_slot() {
        let mut run = make_run(14, 6, 5);
        // A slot that shares its sheet and kind with another slot (so there is a seed to switch to).
        let idx = (0..20)
            .find(|&i| {
                let s = &run.plan().slots[i];
                run.plan().slots.iter().enumerate().any(|(j, o)| {
                    j != i && o.stratum == s.stratum && o.kind == s.kind && o.seed != s.seed
                })
            })
            .unwrap();
        let (stratum, kind, first_seed) = {
            let s = &run.plan().slots[idx];
            (s.stratum, s.kind, s.seed.clone())
        };
        let mut guard = 0;
        while run.rejects[idx] < 1 && guard < 20 {
            guard += 1;
            let Next::Ask(b) = run.next() else { break };
            let raws: Vec<RawQuestion> = b
                .iter()
                .map(|&i| {
                    let slot = run.plan().slots[i].clone();
                    let mut r = raw_for(&slot, slot.no);
                    if i == idx {
                        r.options.pop();
                    }
                    r
                })
                .collect();
            run.on_answer(&b, &answer_json(&raws), false);
        }
        assert_eq!(run.rejects[idx], 1);
        let s = &run.plan().slots[idx];
        assert_ne!(
            s.seed, first_seed,
            "a rejection switches the seed (A-OWNER-31)"
        );
        assert_eq!((s.stratum, s.kind), (stratum, kind));
        assert!(run
            .plan()
            .slots
            .iter()
            .enumerate()
            .any(|(j, o)| j != idx && o.seed == s.seed));
    }

    #[test]
    fn a_slot_without_another_seed_keeps_its_own() {
        // One item per sheet: nothing to switch to.
        let strata = vec![crate::gen::Stratum {
            bank: 1,
            sheet: "Only".into(),
            n: 10,
            multi: 0,
        }];
        let items = vec![crate::gen::SourceItem {
            id: "only".into(),
            stratum: 0,
            kind: crate::gen::Kind::Single,
            opts: 4,
            right: 1,
            diff: None,
            stem: tri(1, 10),
        }];
        let p = plan(&PlanInput {
            items: &items,
            strata: &strata,
            comp: Composition {
                single: 2,
                multi: 0,
            },
            per_call: 2,
            seed: 1,
        })
        .unwrap();
        let limits = JobLimits {
            max_calls: 50,
            max_fails_in_row: 3,
            max_slot_rejects: 5,
        };
        let mut run = Run::new(p, Deduper::new(vec![], vec![]), limits);
        for _ in 0..4 {
            let Next::Ask(b) = run.next() else { break };
            run.on_answer(&b, r#"{"questions":[{"slot":99}]}"#, false);
        }
        assert!(run.plan().slots.iter().all(|s| s.seed == "only"));
    }

    #[test]
    fn unknown_and_duplicate_slots_are_reported() {
        let mut run = make_run(14, 6, 5);
        let Next::Ask(b) = run.next() else { panic!() };
        let s0 = run.plan().slots[0].clone();
        let mut stray = raw_for(&s0, 1);
        stray.slot = 99;
        let first = raw_for(&s0, 1);
        let second = raw_for(&s0, 2);
        let rep = run.on_answer(&b, &answer_json(&[stray, first, second]), false);
        assert_eq!(rep.accepted, 1);
        assert!(rep.rejected.contains(&(99, Reject::UnknownSlot)));
        assert!(rep.rejected.contains(&(1, Reject::DuplicateSlot)));
        assert_eq!(rep.missing, vec![2, 3, 4, 5]);
    }

    #[test]
    fn dedup_rejects_a_copy_of_a_source_stem_and_a_repeat() {
        let mut run = make_run(14, 6, 5);
        let Next::Ask(b) = run.next() else { panic!() };
        let s0 = run.plan().slots[0].clone();
        let s1 = run.plan().slots[1].clone();
        let mut copy = raw_for(&s0, 1);
        copy.stem = items3()[0].stem.clone();
        let ok = raw_for(&s1, 2);
        let mut repeat = raw_for(&run.plan().slots[2].clone(), 3);
        repeat.stem = ok.stem.clone();
        let rep = run.on_answer(&b, &answer_json(&[copy, ok, repeat]), false);
        assert_eq!(rep.accepted, 1);
        assert_eq!(
            rep.rejected,
            vec![(1, Reject::Copy), (3, Reject::Duplicate)]
        );
    }

    #[test]
    fn written_list_and_deduper_include_every_accepted_question() {
        let mut run = make_run(14, 6, 5);
        let Next::Ask(b) = run.next() else { panic!() };
        let t = good(&run, &b);
        run.on_answer(&b, &t, false);
        let w = run.written();
        assert_eq!(w.len(), 5);
        let qs = run.questions();
        assert_eq!(w[0], qs[0].stem.en);
        // The same questions answered again for other slots are duplicates.
        let Next::Ask(b2) = run.next() else { panic!() };
        let again: Vec<RawQuestion> = b2
            .iter()
            .enumerate()
            .map(|(k, &i)| {
                let mut r = raw_for(&run.plan().slots[i].clone(), 1);
                r.slot = run.plan().slots[i].no;
                if k == 0 {
                    r.stem = raw_for(&run.plan().slots[0].clone(), 1).stem;
                }
                r
            })
            .collect();
        let rep = run.on_answer(&b2, &answer_json(&again), false);
        assert!(rep.rejected.iter().any(|(_, r)| *r == Reject::Duplicate));
    }

    #[test]
    fn an_unparsable_answer_requeues_without_counting() {
        let mut run = make_run(14, 6, 5);
        let Next::Ask(b) = run.next() else { panic!() };
        let rep = run.on_answer(&b, "Sorry, I cannot help.", false);
        assert_eq!(rep.accepted, 0);
        assert_eq!(rep.missing, vec![1, 2, 3, 4, 5]);
        assert!(run.rejects.iter().all(|&r| r == 0));
        assert_eq!(run.calls(), 1);
    }

    #[test]
    fn replays_are_deterministic() {
        let go = || {
            let mut run = make_run(14, 6, 5);
            let mut log = Vec::new();
            while let Next::Ask(b) = run.next() {
                log.push(b.clone());
                let mut raws: Vec<RawQuestion> = b
                    .iter()
                    .map(|&i| raw_for(&run.plan().slots[i].clone(), run.plan().slots[i].no))
                    .collect();
                if log.len() == 1 {
                    raws.truncate(2);
                }
                run.on_answer(&b, &answer_json(&raws), false);
            }
            log
        };
        assert_eq!(go(), go());
    }

    // —— the convert policy (IA-6) ————————————————————————————————————————————————————————————————————————————————————

    use crate::gen::parse::RawOption;
    use crate::import::convert::{convert_plan, make_spec};
    use crate::import::segment::SegType;

    fn region(i: usize) -> String {
        format!(
            "Q{i}. Which kitchen device {} keeps a stock pot at a steady simmer?\nA. {} {}\nB. {} {}\nC. {} {}\nAnswer: B",
            en_text(300 + i, 3),
            en_text(10 + i, 1),
            en_text(20 + i, 1),
            en_text(30 + i, 1),
            en_text(40 + i, 1),
            en_text(50 + i, 1),
            en_text(60 + i, 1)
        )
    }

    fn convert_run(regions: &[String]) -> Run {
        let specs: Vec<_> = regions
            .iter()
            .map(|r| make_spec(r, SegType::Single))
            .collect();
        let plan = convert_plan(&specs, 8000, 5);
        let limits = JobLimits::for_convert(&plan);
        Run::new_convert(plan, specs, limits, 8000)
    }

    /// A faithful answer for the slots of a batch: the region's own words, answer B.
    fn convert_good(run: &Run, regions: &[String], batch: &[usize]) -> String {
        let raws: Vec<RawQuestion> = batch
            .iter()
            .map(|&i| {
                let no = run.plan().slots[i].no;
                let lines: Vec<&str> = regions[i].lines().collect();
                let stem = lines[0].split_once(". ").map_or(lines[0], |x| x.1);
                let opt = |k: usize| {
                    lines[k]
                        .split_once(". ")
                        .map_or(lines[k], |x| x.1)
                        .to_string()
                };
                RawQuestion {
                    slot: no,
                    stem: Tri::new(stem, &han(no, 9), &kana(no, 9)),
                    options: (1..=3)
                        .map(|k| RawOption {
                            text: Tri::new(&opt(k), &han(no * 10 + k, 3), &kana(no * 10 + k, 3)),
                            correct: k == 2,
                        })
                        .collect(),
                    explanation: Tri::new(
                        &en_text(900 + no, 8),
                        &han(900 + no, 10),
                        &kana(900 + no, 10),
                    ),
                }
            })
            .collect();
        answer_json(&raws)
    }

    #[test]
    fn the_convert_policy_fills_every_slot_without_dedup_and_ends_full() {
        // a file may repeat a question: it is imported as it is
        let mut regions: Vec<String> = (1..=5).map(region).collect();
        regions.push(regions[0].clone());
        let mut run = convert_run(&regions);
        let mut calls = 0;
        while let Next::Ask(b) = run.next() {
            let t = convert_good(&run, &regions, &b);
            let rep = run.on_answer(&b, &t, false);
            assert!(rep.rejected.is_empty(), "{:?}", rep.rejected);
            calls += 1;
        }
        assert_eq!(run.outcome(), Outcome::Full);
        assert_eq!(
            (run.made(), run.questions().len(), calls),
            (6, 6, 2),
            "3 per call at 8000 characters"
        );
        assert!(run.written().is_empty());
        assert!(run.skipped().is_empty());
        let qs = run.questions();
        assert_eq!(qs[0].answer, vec!["B"]);
        assert_eq!(qs[0].options.len(), 3);
    }

    #[test]
    fn a_slot_rejected_three_times_is_skipped_and_never_asked_again() {
        let regions: Vec<String> = (1..=4).map(region).collect();
        let mut run = convert_run(&regions);
        let mut asked_slot2 = 0;
        while let Next::Ask(b) = run.next() {
            if b.contains(&1) {
                asked_slot2 += 1;
            }
            // slot 2 comes back reworded every time
            let mut t: serde_json::Value =
                serde_json::from_str(&convert_good(&run, &regions, &b)).unwrap();
            for q in t["questions"].as_array_mut().unwrap() {
                if q["slot"] == 2 {
                    q["stem"]["en"] = serde_json::json!(
                        "What apparatus holds a large cooking vessel at an even low heat setting"
                    );
                }
            }
            run.on_answer(&b, &t.to_string(), false);
        }
        assert_eq!(asked_slot2, 3, "its third rejection ends it");
        assert_eq!(run.made(), 3);
        assert_eq!(run.skipped(), vec![2]);
        assert_eq!(run.outcome(), Outcome::Partial { skipped: vec![2] });
        // no seed was switched: the plan's seeds are the planned ones
        assert!(run
            .plan()
            .slots
            .iter()
            .enumerate()
            .all(|(i, s)| s.seed == format!("C{}", i + 1)));
    }

    #[test]
    fn nothing_converted_is_a_quality_failure_and_three_provider_errors_in_a_row_are_a_provider_failure(
    ) {
        let regions: Vec<String> = (1..=3).map(region).collect();
        let mut run = convert_run(&regions);
        while let Next::Ask(b) = run.next() {
            run.on_answer(&b, "{\"questions\": []}", false);
        }
        assert_eq!(run.outcome(), Outcome::Failed(FailCode::Quality));
        assert_eq!(run.skipped(), vec![1, 2, 3]);
        let mut run = convert_run(&regions);
        for _ in 0..3 {
            let Next::Ask(b) = run.next() else { panic!() };
            run.on_provider_error(&b, "ai.up.rateLimit");
        }
        assert_eq!(run.next(), Next::Finish);
        assert_eq!(
            run.outcome(),
            Outcome::Failed(FailCode::Provider("ai.up.rateLimit".into()))
        );
        assert_eq!(run.backoff_secs(), 45);
        // errors that are not in a row do not stop it, and they never count against a slot
        let mut run = convert_run(&regions);
        run.limits.max_calls = 100; // the safety stop is tested elsewhere
        let Next::Ask(b) = run.next() else { panic!() };
        run.on_provider_error(&b, "ai.up.server");
        let Next::Ask(b2) = run.next() else { panic!() };
        assert_eq!(
            b2[..],
            b[..1],
            "the slots go back, in smaller batches (half the budget)"
        );
        let t = convert_good(&run, &regions, &b2);
        run.on_answer(&b2, &t, false);
        while let Next::Ask(b3) = run.next() {
            let t = convert_good(&run, &regions, &b3);
            run.on_answer(&b3, &t, false);
        }
        assert_eq!(run.outcome(), Outcome::Full);
    }

    #[test]
    fn a_truncated_answer_halves_the_budget_raises_the_wanted_size_and_the_safety_stop_follows() {
        let regions: Vec<String> = (1..=6).map(region).collect();
        let mut run = convert_run(&regions);
        assert_eq!(run.limits.max_calls, 6, "3 x the two planned calls");
        let Next::Ask(b) = run.next() else { panic!() };
        assert_eq!(b.len(), 3);
        run.on_answer(&b, "{\"questions\": [", true);
        // a budget of 4 000 characters fits one slot: six calls for six slots, x 3
        assert!(run.limits.max_calls >= 18, "{}", run.limits.max_calls);
        let Next::Ask(b2) = run.next() else { panic!() };
        assert_eq!(b2.len(), 1, "one slot per call after the cut");
        assert!(
            run.want_chars(&b).is_some_and(|w| w > 0),
            "the cut slots ask for more"
        );
        // nothing was counted against the slots by the cut
        assert!(run.rejects.iter().all(|&r| r == 0));
    }

    #[test]
    fn an_oversized_region_goes_alone_and_asks_for_a_larger_answer() {
        let mut regions: Vec<String> = (1..=3).map(region).collect();
        regions[1] = format!("{}\n{}", regions[1], "x".repeat(2900));
        let mut run = convert_run(&regions);
        let mut batches = Vec::new();
        let mut wants = Vec::new();
        while let Next::Ask(b) = run.next() {
            wants.push(run.want_chars(&b));
            batches.push(b.clone());
            let t = convert_good(&run, &regions, &b);
            run.on_answer(&b, &t, false);
        }
        assert_eq!(batches, vec![vec![0], vec![1], vec![2]]);
        let est = 3 * regions[1].chars().count() + 1800;
        assert_eq!(wants, vec![None, Some((est * 5 / 4).min(32_000)), None]);
    }

    #[test]
    fn the_generation_policy_never_ends_partial() {
        let mut run = make_run(14, 6, 5);
        while let Next::Ask(b) = run.next() {
            run.on_answer(&b, "no questions here", false);
        }
        assert_eq!(run.outcome(), Outcome::Failed(FailCode::Quality));
        assert_eq!(
            run.skipped().len(),
            20,
            "everything is open, nothing is ever given up"
        );
        // only the calls of ONE slot count an unreadable answer (A-OWNER-66.2), and never past the slot's cap
        assert!(run.rejects.iter().any(|&r| r > 0));
        assert!(run.rejects.iter().all(|&r| r as usize <= SLOT_REJECTS));
    }

    /// A-OWNER-31: a small source (10 questions for a goal of 20) that the model keeps close to still gives 20 valid
    /// questions: near matches are no longer rejected, and quality rejections do not use up the provider stop.
    #[test]
    fn quality_rejections_have_their_own_budget_and_a_small_source_relaxes() {
        let mut run = make_run(14, 6, 5);
        let b = match run.next() {
            Next::Ask(b) => b,
            Next::Finish => panic!(),
        };
        // three provider errors in a row would stop a job; interleaved answers reset the count, and errors never
        // use the ceiling of answered calls
        run.on_provider_error(&b, "ai.up.server");
        run.on_provider_error(&b, "ai.up.server");
        assert_eq!(run.answers, 0);
        assert_eq!(run.reason_counts(), "none");
        let t = good(&run, &b);
        run.on_answer(&b, &t, false);
        assert_eq!(run.fails_in_row, 0);
        assert_eq!(run.answers, 1);
    }

    /// A-OWNER-31: a slot may collect 4 retries (5 rejections) before the job stops; below that the job goes on.
    #[test]
    fn a_slot_has_four_retries() {
        let mut run = make_run(14, 6, 5);
        run.limits.max_calls = 1000;
        let mut rounds = 0;
        while let Next::Ask(b) = run.next() {
            let raws: Vec<RawQuestion> = b
                .iter()
                .map(|&i| {
                    let slot = run.plan().slots[i].clone();
                    let mut r = raw_for(&slot, slot.no);
                    r.options.pop();
                    r
                })
                .collect();
            run.on_answer(&b, &answer_json(&raws), false);
            rounds += 1;
            let worst = run.rejects.iter().copied().max().unwrap_or(0);
            assert!(worst <= 5);
            assert!(
                rounds < 100,
                "rounds {rounds} rejects {:?} answers {} fails {}",
                run.rejects,
                run.answers,
                run.fails_in_row
            );
        }
        assert_eq!(run.rejects.iter().copied().max(), Some(5));
        assert_eq!(run.outcome(), Outcome::Failed(FailCode::Quality));
    }
}
