//! v0.9 AI+ study features: what the AI service may read about one question or one study record, through
//! `POST /internal/ai/question` and `POST /internal/ai/record` (see `api/internal_study.rs`).
//!
//! The AI service never opens this database. It asks, with the caller's live session (the route binds the session to the
//! user id, this module only sees the user id), and gets a small, capped projection:
//!
//! - [`Store::study_question`]: one question the caller may see, as a hint-safe [`StudyQuestion`] (no answer key, no
//!   explanation) next to its [`Solution`] (answer key, explanation, memory aid, note, the text of the correct option), in
//!   the language wanted; with a record of the caller's own that contains the question, also what they answered there.
//! - [`Store::study_record`]: the caller's own record: its counts, and up to `max` of the questions they answered WRONG,
//!   chosen uniformly at random without replacement and returned in record order, each with what they answered. All the item
//!   texts together stay within [`TEXT_BUDGET`] characters (`"budgetCut"` says whether the budget cut anything).
//! - [`Store::study_record_info`]: only the identity of the caller's own record (`at`, `kind`, session `key` / `startedAt`).
//!
//! Rules (spec 4.2 - 4.5):
//! - **Not found is one answer.** A missing question, a question of a bank outside the caller's [`BankScope`] and a record of
//!   another user all give `Ok(None)`; the route turns it into the same bytes (`{"ok": false, "reason": "notFound"}`).
//! - **Wrong answers** are record items with `correct = 0` AND an answer (answered wrong, the rule of the records' `answered`
//!   count). Unanswered items are neither counted nor sent.
//! - **Never included**: the raw column-B `translation`, the four-column raw `english`, `tag`, `sheet`, `row`, bank id / name,
//!   owner, any user name, statistics.
//! - **Caps** ([`FULL`] for one question, [`COMPACT`] for the record route): texts are cut to at most `cap` characters, the
//!   last one being `…` (`cut: true` on the question); option / item / choice counts are cut too.
//! - Read-only: nothing is written, nothing is logged.
//!
//! The pure parts (projection, caps, answer clean-up, sampling) are plain functions with unit tests below; the two store
//! methods only load rows and call them. Needs the v6 columns `attempts.session_key` / `attempts.started_at`
//! (`schema::V6_COLUMNS`).
use super::banks::viewer_of;
use super::{load_questions, Store, StoredQuestion, QUESTION_COLUMNS};
use crate::error::DbError;
use crate::models::{
    setting_valid, Answer, AttemptKind, ExplainText, Kind, Question, Zh, SETTING_UI_LANG,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

/// A language of the question text or of the explanation: the values `"en"`, `"zh"` and `"ja"` of the routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StudyLang {
    En,
    Zh,
    Ja,
}

impl StudyLang {
    pub fn parse(s: &str) -> Option<StudyLang> {
        match s {
            "en" => Some(StudyLang::En),
            "zh" => Some(StudyLang::Zh),
            "ja" => Some(StudyLang::Ja),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            StudyLang::En => "en",
            StudyLang::Zh => "zh",
            StudyLang::Ja => "ja",
        }
    }
}

/// The size limits of one projection (spec 4.5). All text caps are in characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub stem: usize,
    /// One option text or one hotspot item text
    pub text: usize,
    pub options: usize,
    pub items: usize,
    pub choice: usize,
    pub choices: usize,
    pub explanation: usize,
    pub mnemonic: usize,
    pub note: usize,
    /// One entry of `keyTexts`; `keyTexts` is only filled with the full caps
    pub key_text: usize,
    pub key_texts: bool,
}

/// The caps of `POST /internal/ai/question`.
pub const FULL: Caps = Caps {
    stem: 6000,
    text: 1500,
    options: 8,
    items: 30,
    choice: 300,
    choices: 10,
    explanation: 6000,
    mnemonic: 1500,
    note: 500,
    key_text: 300,
    key_texts: true,
};

/// The caps of one item of `POST /internal/ai/record`.
pub const COMPACT: Caps = Caps {
    stem: 1200,
    text: 300,
    options: 8,
    items: 20,
    choice: 80,
    choices: 10,
    explanation: 400,
    mnemonic: 0,
    note: 0,
    key_text: 0,
    key_texts: false,
};

/// Most wrong answers one record answer carries (the route's `max` is 1..=this).
pub const MAX_SAMPLE: usize = 70;

/// Most characters of question and answer text (stems, option / item / choice texts, explanations) all the items of one
/// record answer carry together. The model's context, not the transport, is the real limit: beyond this every item is cut
/// to its fair share (see [`apply_budget`]).
pub const TEXT_BUDGET: usize = 120_000;

// ---------------------------------------------------------------- pure parts

/// `s` cut to at most `cap` characters; a cut text ends with `…` (so it has exactly `cap` characters). The flag says
/// whether it was cut.
pub fn cut_text(s: &str, cap: usize) -> (String, bool) {
    if s.chars().count() <= cap {
        return (s.to_string(), false);
    }
    if cap == 0 {
        return (String::new(), !s.is_empty());
    }
    let mut out: String = s.chars().take(cap - 1).collect();
    out.push('…');
    (out, true)
}

/// The language of a question's own text: English for the six-column layout (`lang == "en"`), Japanese for every older one.
pub fn base_lang(q: &Question) -> StudyLang {
    if q.lang == "en" {
        StudyLang::En
    } else {
        StudyLang::Ja
    }
}

/// The translation structure of `lang` (`None` for the base language and for a language the question lacks).
fn translation(q: &Question, lang: StudyLang) -> Option<&Zh> {
    if lang == base_lang(q) {
        return None;
    }
    match lang {
        StudyLang::Zh => q.zh.as_ref(),
        StudyLang::En => q.en.as_ref(),
        StudyLang::Ja => q.ja.as_ref(),
    }
}

/// The language the question is really shown in when `want` is asked: `want` when it is the base language or the question
/// has that translation, else the base language (the rule of the frontend's `effectiveLang`).
pub fn effective_lang(q: &Question, want: StudyLang) -> StudyLang {
    if want == base_lang(q) || translation(q, want).is_some() {
        want
    } else {
        base_lang(q)
    }
}

/// A question's texts in one language (the frontend's `shown()`): a missing piece falls back to the base text, so the
/// content is never misaligned.
struct Shown<'a> {
    q: &'a Question,
    lang: StudyLang,
    other: Option<&'a Zh>,
}

impl<'a> Shown<'a> {
    fn new(q: &'a Question, want: StudyLang) -> Shown<'a> {
        let lang = effective_lang(q, want);
        Shown {
            q,
            lang,
            other: translation(q, lang),
        }
    }
    fn stem(&self) -> &str {
        self.other.map_or(self.q.stem.as_str(), |t| t.stem.as_str())
    }
    fn option_text(&self, i: usize) -> &str {
        self.other
            .and_then(|t| t.options.as_ref())
            .and_then(|o| o.get(i))
            .map(String::as_str)
            .or_else(|| self.q.options.get(i).map(|o| o.text.as_str()))
            .unwrap_or("")
    }
    fn item_text(&self, i: usize) -> &str {
        self.other
            .and_then(|t| t.items.as_ref())
            .and_then(|o| o.get(i))
            .map(String::as_str)
            .or_else(|| self.q.items.get(i).map(|o| o.text.as_str()))
            .unwrap_or("")
    }
    /// Choice `j` of item `i`: this language's own choices when it has a non-empty one, else the base choice.
    fn choice_text(&self, i: usize, j: usize) -> &str {
        let own = self
            .other
            .and_then(|t| t.choices.as_ref())
            .and_then(|c| c.get(i))
            .and_then(|c| c.get(j))
            .map(String::as_str)
            .filter(|s| !s.is_empty());
        own.or_else(|| {
            self.q
                .items
                .get(i)
                .and_then(|it| it.choices.get(j))
                .map(String::as_str)
        })
        .unwrap_or("")
    }
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Single => "single",
        Kind::Multi => "multi",
        Kind::Hotspot => "hotspot",
    }
}

/// The hint-safe projection of a question (`StudyQuestion` of spec 4.5): `{"id", "kind", "lang", "stem", "options",
/// "items", "active", "cut"}`. Holds no answer key and no explanation.
pub fn project_question(
    id: &str,
    active: bool,
    q: &Question,
    qlang: StudyLang,
    caps: &Caps,
) -> Value {
    let sh = Shown::new(q, qlang);
    let mut cut = false;
    let mut take = |s: &str, cap: usize| {
        let (t, c) = cut_text(s, cap);
        cut |= c;
        t
    };
    let stem = take(sh.stem(), caps.stem);
    let mut options = Vec::new();
    let mut items = Vec::new();
    let mut dropped = false;
    if q.kind == Kind::Hotspot {
        dropped |= q.items.len() > caps.items;
        for (i, it) in q.items.iter().take(caps.items).enumerate() {
            let text = take(sh.item_text(i), caps.text);
            let shown_choices = it.choices.len();
            dropped |= shown_choices > caps.choices;
            let choices: Vec<String> = (0..shown_choices.min(caps.choices))
                .map(|j| take(sh.choice_text(i, j), caps.choice))
                .collect();
            items.push(json!({ "text": text, "choices": choices }));
        }
    } else {
        dropped |= q.options.len() > caps.options;
        for (i, o) in q.options.iter().take(caps.options).enumerate() {
            let text = take(sh.option_text(i), caps.text);
            options.push(json!({ "key": o.key, "text": text }));
        }
    }
    json!({
        "id": id,
        "kind": kind_name(q.kind),
        "lang": sh.lang.as_str(),
        "stem": stem,
        "options": options,
        "items": items,
        "active": active,
        "cut": cut || dropped,
    })
}

/// The explanation texts to use for `elang` (spec 4.5): `explain[elang]` when it has text, else `explain[qlang]`, else
/// English, Chinese, Japanese in that order; the language it is written in. A question without per-language explanations
/// (older layouts, or none written) uses its compatibility fields, language `None`.
fn pick_explanation(
    q: &Question,
    elang: StudyLang,
    qlang: StudyLang,
) -> (ExplainText, Option<StudyLang>) {
    if let Some(ex) = &q.explain {
        let of = |l: StudyLang| match l {
            StudyLang::En => ex.en.as_ref(),
            StudyLang::Zh => ex.zh.as_ref(),
            StudyLang::Ja => ex.ja.as_ref(),
        };
        for l in [elang, qlang, StudyLang::En, StudyLang::Zh, StudyLang::Ja] {
            if let Some(t) = of(l).filter(|t| !t.is_empty()) {
                return (t.clone(), Some(l));
            }
        }
    }
    (
        ExplainText {
            explanation: q.explanation.clone(),
            mnemonic: q.mnemonic.clone(),
            note: q.note.clone(),
        },
        None,
    )
}

/// The text of the correct option(s) / choice(s) in every language the question has, de-duplicated, empty ones left out.
fn key_texts(q: &Question, cap: usize) -> Vec<String> {
    let base = base_lang(q);
    let mut langs = vec![base];
    for l in [StudyLang::En, StudyLang::Zh, StudyLang::Ja] {
        if l != base && translation(q, l).is_some() {
            langs.push(l);
        }
    }
    let mut out: Vec<String> = Vec::new();
    let mut add = |s: &str| {
        if s.is_empty() {
            return;
        }
        let (t, _) = cut_text(s, cap);
        if !out.contains(&t) {
            out.push(t);
        }
    };
    match &q.answer {
        Answer::Letters(keys) => {
            for key in keys {
                if let Some(i) = q.options.iter().position(|o| &o.key == key) {
                    for l in &langs {
                        add(Shown::new(q, *l).option_text(i));
                    }
                }
            }
        }
        Answer::Indexes(picks) => {
            for (i, pick) in picks.iter().enumerate() {
                if let Ok(j) = usize::try_from(*pick) {
                    for l in &langs {
                        add(Shown::new(q, *l).choice_text(i, j));
                    }
                }
            }
        }
    }
    out
}

/// The answer side of a question (`Solution` of spec 4.5): `{"answer", "explanation", "mnemonic", "note", "explainLang",
/// "keyTexts"}`. `qlang` is the language the question text was asked in, `elang` the preferred explanation language.
pub fn project_solution(q: &Question, qlang: StudyLang, elang: StudyLang, caps: &Caps) -> Value {
    let (text, written) = pick_explanation(q, elang, effective_lang(q, qlang));
    let answer = match &q.answer {
        Answer::Letters(v) => json!(v),
        Answer::Indexes(v) => json!(v),
    };
    let keys = if caps.key_texts {
        key_texts(q, caps.key_text)
    } else {
        Vec::new()
    };
    json!({
        "answer": answer,
        "explanation": cut_text(&text.explanation, caps.explanation).0,
        "mnemonic": cut_text(&text.mnemonic, caps.mnemonic).0,
        "note": cut_text(&text.note, caps.note).0,
        "explainLang": written.map_or("", StudyLang::as_str),
        "keyTexts": keys,
    })
}

/// A stored answer (`attempt_items.answer`, a JSON array) as a list; `None` for NULL, text that is not JSON and anything
/// that is not an array.
fn parse_answer(text: Option<&str>) -> Option<Vec<Value>> {
    match serde_json::from_str::<Value>(text?) {
        Ok(Value::Array(a)) => Some(a),
        _ => None,
    }
}

/// What the user answered, as it fits the question now (a re-import can change options): every entry is kept in place, but
/// one that does not fit becomes `null` - a letter that is not one of the question's option keys, an index that is not a
/// choice of that item, an entry beyond the item count, or an entry of the wrong type for the kind.
pub fn clean_user_answer(entries: &[Value], q: &Question) -> Vec<Value> {
    entries
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if q.kind == Kind::Hotspot {
                let fits = v
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .zip(q.items.get(i))
                    .is_some_and(|(n, it)| n < it.choices.len());
                if fits {
                    v.clone()
                } else {
                    Value::Null
                }
            } else {
                match v.as_str() {
                    Some(s) if q.options.iter().any(|o| o.key == s) => v.clone(),
                    _ => Value::Null,
                }
            }
        })
        .collect()
}

/// `k` positions out of `0..n`, uniformly at random without replacement (a partial Fisher-Yates shuffle), returned in
/// ascending order. `pick(m)` must return a number in `0..m` (a larger one is clamped); it is injectable so tests can fix
/// the draw. `k >= n` returns every position without drawing.
pub fn sample_indices(n: usize, k: usize, pick: &mut dyn FnMut(usize) -> usize) -> Vec<usize> {
    let k = k.min(n);
    if k == n {
        return (0..n).collect();
    }
    let mut idx: Vec<usize> = (0..n).collect();
    for i in 0..k {
        let left = n - i;
        let j = i + pick(left).min(left - 1);
        idx.swap(i, j);
    }
    idx.truncate(k);
    idx.sort_unstable();
    idx
}

/// A uniform random number in `0..bound` from the operating system's randomness (rejection sampling: no modulo bias).
/// `bound` must be at least 1.
pub fn random_below(bound: usize) -> usize {
    let bound = u64::try_from(bound.max(1)).unwrap_or(u64::MAX);
    let limit = u64::MAX - (u64::MAX % bound);
    loop {
        let mut buf = [0u8; 8];
        if getrandom::fill(&mut buf).is_err() {
            // Never fall back to a predictable draw when the system randomness source is unavailable
            panic!("operating system random source is unavailable");
        }
        let v = u64::from_le_bytes(buf);
        if v < limit {
            return usize::try_from(v % bound).unwrap_or(0);
        }
    }
}

// ---------------------------------------------------------------- the text budget of a record answer

fn chars_of(v: &Value) -> usize {
    v.as_str().map_or(0, |s| s.chars().count())
}

/// Characters of text in one item (`question` and `solution` as [`project_question`] / [`project_solution`] made them): the
/// stem, every option / item text and choice, the explanation, memory aid, note and key texts.
fn item_text_len(q: &Value, s: &Value) -> usize {
    let mut n = chars_of(&q["stem"]);
    for o in q["options"].as_array().into_iter().flatten() {
        n += chars_of(&o["text"]);
    }
    for it in q["items"].as_array().into_iter().flatten() {
        n += chars_of(&it["text"]);
        n += it["choices"]
            .as_array()
            .into_iter()
            .flatten()
            .map(chars_of)
            .sum::<usize>();
    }
    for f in ["explanation", "mnemonic", "note"] {
        n += chars_of(&s[f]);
    }
    n + s["keyTexts"]
        .as_array()
        .into_iter()
        .flatten()
        .map(chars_of)
        .sum::<usize>()
}

/// Cuts the string at `v` to `len * num / den` characters (rounded down; ends with `…` when cut). Returns whether it cut.
fn shrink(v: &mut Value, num: usize, den: usize) -> bool {
    let Some(text) = v.as_str() else { return false };
    let len = text.chars().count();
    let cap = usize::try_from(
        u128::try_from(len).unwrap_or(0) * u128::try_from(num).unwrap_or(0)
            / u128::try_from(den.max(1)).unwrap_or(1),
    )
    .unwrap_or(len);
    if cap >= len {
        return false;
    }
    let (cut, _) = cut_text(text, cap);
    *v = Value::String(cut);
    true
}

/// Brings the item texts of a record answer within `budget` characters in total: every item gets a fair share (the smaller
/// items keep everything, what they leave over goes to the larger ones), and an item over its share has every text cut in the
/// same proportion (an option keeps its letter and is never dropped; a text may end up as just `…` or empty) and its
/// question's `cut` set. Returns whether anything was cut. Items within budget are not touched.
pub fn apply_budget(items: &mut [Value], budget: usize) -> bool {
    let sizes: Vec<usize> = items
        .iter()
        .map(|i| item_text_len(&i["question"], &i["solution"]))
        .collect();
    if sizes.iter().sum::<usize>() <= budget {
        return false;
    }
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| sizes[i]);
    let mut remaining = budget;
    let mut allowed = vec![0_usize; items.len()];
    for (pos, &i) in order.iter().enumerate() {
        let share = remaining / (items.len() - pos);
        allowed[i] = sizes[i].min(share);
        remaining -= allowed[i];
    }
    let mut any = false;
    for (i, item) in items.iter_mut().enumerate() {
        if allowed[i] >= sizes[i] {
            continue;
        }
        let (num, den) = (allowed[i], sizes[i]);
        let mut cut = false;
        if let Some(q) = item.get_mut("question") {
            cut |= shrink(&mut q["stem"], num, den);
            for o in q["options"].as_array_mut().into_iter().flatten() {
                cut |= shrink(&mut o["text"], num, den);
            }
            for it in q["items"].as_array_mut().into_iter().flatten() {
                cut |= shrink(&mut it["text"], num, den);
                for c in it["choices"].as_array_mut().into_iter().flatten() {
                    cut |= shrink(c, num, den);
                }
            }
            q["cut"] = json!(true);
        }
        if let Some(sol) = item.get_mut("solution") {
            for f in ["explanation", "mnemonic", "note"] {
                cut |= shrink(&mut sol[f], num, den);
            }
            for k in sol["keyTexts"].as_array_mut().into_iter().flatten() {
                cut |= shrink(k, num, den);
            }
        }
        any |= cut || num < den;
    }
    any
}

// ---------------------------------------------------------------- store

/// The `uiLang` setting of `user` as `"en"`, `"zh"` or `"ja"` (the default `"en"`; an invalid stored value is ignored, like
/// the settings API does).
fn ui_lang(c: &Connection, user: i64) -> Result<&'static str, DbError> {
    let text: Option<String> = c
        .query_row(
            "SELECT value FROM user_settings WHERE user_id=?1 AND key=?2",
            params![user, SETTING_UI_LANG],
            |r| r.get(0),
        )
        .optional()?;
    let value = text.and_then(|t| serde_json::from_str::<Value>(&t).ok());
    Ok(match value {
        Some(v) if setting_valid(SETTING_UI_LANG, &v) => match v.as_str() {
            Some("zh") => "zh",
            Some("ja") => "ja",
            _ => "en",
        },
        _ => "en",
    })
}

/// One record of `user` as the study routes see it.
struct RecordRow {
    id: i64,
    at: String,
    kind: String,
    mode: String,
    total: i64,
    correct: i64,
    /// Detail rows / rows with an answer
    items: i64,
    answered: i64,
    key: Option<String>,
    started_at: Option<i64>,
}

fn load_record(c: &Connection, user: i64, id: i64) -> Result<Option<RecordRow>, DbError> {
    Ok(c.query_row(
        "SELECT a.id, a.at, a.kind, a.mode, a.total, a.correct, a.session_key, a.started_at, \
                (SELECT COUNT(*) FROM attempt_items i WHERE i.attempt_id = a.id), \
                (SELECT COUNT(i.answer) FROM attempt_items i WHERE i.attempt_id = a.id) \
         FROM attempts a WHERE a.id = ?1 AND a.user_id = ?2",
        params![id, user],
        |r| {
            Ok(RecordRow {
                id: r.get(0)?,
                at: r.get(1)?,
                kind: r.get(2)?,
                mode: r.get(3)?,
                total: r.get(4)?,
                correct: r.get(5)?,
                key: r.get(6)?,
                started_at: r.get(7)?,
                items: r.get(8)?,
                answered: r.get(9)?,
            })
        },
    )
    .optional()?)
}

/// A stored question as a [`Question`]; `None` when its data cannot be read (never for rows this program wrote).
fn parse_question(row: &StoredQuestion) -> Option<Question> {
    serde_json::from_str::<Question>(&row.data).ok()
}

/// What `POST /internal/ai/question` asks (after validation).
#[derive(Debug, Clone)]
pub struct QuestionAsk {
    pub question_id: String,
    /// The language of the question text wanted (the card's language)
    pub qlang: StudyLang,
    /// The preferred language of the stored explanation (the reply language)
    pub elang: StudyLang,
    /// A record of the caller's own that must contain the question: `(record id, its kind)`
    pub record: Option<(i64, AttemptKind)>,
}

impl Store {
    /// `POST /internal/ai/question` for `user` (the route has bound the session to this id already): `Ok(None)` = not found
    /// (no such question, a bank `user` cannot see, or - with a record - a record that is not theirs, of another kind, or
    /// does not contain the question). Otherwise `{"ok": true, "uiLang", "question", "solution", "item", "record"}`; `item` and
    /// `record` are null without a record (`item`: `{"seq", "answer", "correct"}`; `record`: `{"id", "at", "kind", "key",
    /// "startedAt"}`). Retired questions are found too (`"active": false` in the question). Reads only.
    pub fn study_question(&self, user: i64, ask: &QuestionAsk) -> Result<Option<Value>, DbError> {
        let g = self.lock();
        let viewer = viewer_of(&g, user)?;
        let sql = format!("SELECT {QUESTION_COLUMNS} FROM questions WHERE id=?1");
        let rows = load_questions(&g, &sql, params![ask.question_id])?;
        let Some(row) = rows.first().filter(|r| viewer.scope.allows(r.bank_id)) else {
            return Ok(None);
        };
        let Some(q) = parse_question(row) else {
            return Ok(None);
        };
        let (item, record) = match ask.record {
            None => (Value::Null, Value::Null),
            Some((record_id, kind)) => {
                let Some(rec) =
                    load_record(&g, user, record_id)?.filter(|r| r.kind == kind.as_str())
                else {
                    return Ok(None);
                };
                // The lowest seq when the question appears twice in one record
                let found: Option<(i64, Option<String>, i64)> = g
                    .query_row(
                        "SELECT seq, answer, correct FROM attempt_items WHERE attempt_id=?1 AND question_id=?2 \
                         ORDER BY seq LIMIT 1",
                        params![rec.id, ask.question_id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                let Some((seq, answer, correct)) = found else {
                    return Ok(None);
                };
                let answer = match parse_answer(answer.as_deref()) {
                    Some(a) => Value::Array(clean_user_answer(&a, &q)),
                    None => Value::Null,
                };
                (
                    json!({ "seq": seq, "answer": answer, "correct": correct != 0 }),
                    json!({
                        "id": rec.id,
                        "at": rec.at,
                        "kind": rec.kind,
                        "key": rec.key,
                        "startedAt": rec.started_at,
                    }),
                )
            }
        };
        Ok(Some(json!({
            "ok": true,
            "uiLang": ui_lang(&g, user)?,
            "question": project_question(&row.id, row.active, &q, ask.qlang, &FULL),
            "solution": project_solution(&q, ask.qlang, ask.elang, &FULL),
            "item": item,
            "record": record,
        })))
    }

    /// `POST /internal/ai/record` for `user`: `Ok(None)` = no such record of theirs. Otherwise `{"ok": true, "uiLang",
    /// "record", "wrong", "usable", "items"}` (see the module documentation): at most `max` of the answered-wrong items
    /// whose question still exists and is visible to `user`, chosen at random. Reads only.
    pub fn study_record(
        &self,
        user: i64,
        record_id: i64,
        max: usize,
        elang: StudyLang,
    ) -> Result<Option<Value>, DbError> {
        self.study_record_with(user, record_id, max, elang, &mut random_below)
    }

    /// [`Store::study_record`] with the random draw injected (tests fix it).
    pub fn study_record_with(
        &self,
        user: i64,
        record_id: i64,
        max: usize,
        elang: StudyLang,
        pick: &mut dyn FnMut(usize) -> usize,
    ) -> Result<Option<Value>, DbError> {
        let g = self.lock();
        let viewer = viewer_of(&g, user)?;
        let Some(rec) = load_record(&g, user, record_id)? else {
            return Ok(None);
        };
        // Every answered-wrong item; `usable`: its question exists (the join: the id test is what leaves out deleted
        // questions for an administrator, whose scope allows "no bank") and the viewer may see its bank
        let mut wrong_rows: Vec<(i64, String, Option<String>)> = Vec::new();
        let mut wrong = 0_i64;
        {
            let mut st = g.prepare(
                "SELECT i.seq, i.question_id, i.answer, q.id, q.bank_id FROM attempt_items i \
                 LEFT JOIN questions q ON q.id = i.question_id \
                 WHERE i.attempt_id = ?1 AND i.correct = 0 AND i.answer IS NOT NULL ORDER BY i.seq",
            )?;
            let mut rows = st.query(params![rec.id])?;
            while let Some(r) = rows.next()? {
                wrong += 1;
                let exists: Option<String> = r.get(3)?;
                let bank: Option<i64> = r.get(4)?;
                if exists.is_some() && viewer.scope.allows(bank) {
                    wrong_rows.push((r.get(0)?, r.get(1)?, r.get(2)?));
                }
            }
        }
        let usable = wrong_rows.len();
        let chosen = sample_indices(usable, max.min(MAX_SAMPLE), pick);
        let sql = format!("SELECT {QUESTION_COLUMNS} FROM questions WHERE id=?1");
        let mut items = Vec::with_capacity(chosen.len());
        for i in chosen {
            let (seq, question_id, answer) = &wrong_rows[i];
            let rows = load_questions(&g, &sql, params![question_id])?;
            let Some(row) = rows.first() else { continue };
            let Some(q) = parse_question(row) else {
                continue;
            };
            let answer = parse_answer(answer.as_deref()).unwrap_or_default();
            items.push(json!({
                "seq": seq,
                "answer": clean_user_answer(&answer, &q),
                "question": project_question(&row.id, row.active, &q, elang, &COMPACT),
                "solution": project_solution(&q, elang, elang, &COMPACT),
            }));
        }
        let budget_cut = apply_budget(&mut items, TEXT_BUDGET);
        let (detail_items, answered) = if rec.items > 0 {
            (rec.items, Value::from(rec.answered))
        } else {
            (0, Value::Null)
        };
        Ok(Some(json!({
            "ok": true,
            "uiLang": ui_lang(&g, user)?,
            "record": {
                "id": rec.id,
                "at": rec.at,
                "kind": rec.kind,
                "mode": rec.mode,
                "total": rec.total,
                "correct": rec.correct,
                "answered": answered,
                "detail": detail_items > 0,
                "key": rec.key,
                "startedAt": rec.started_at,
            },
            "wrong": wrong,
            "usable": usable,
            "items": items,
            "budgetCut": budget_cut,
        })))
    }

    /// `POST /internal/ai/record-info` for `user`: just the identity of their own record, `{"ok": true, "uiLang", "record":
    /// {"id", "at", "kind", "key", "startedAt"}}`; `Ok(None)` = no such record of theirs. Cheap (one row): the AI service uses
    /// it to find the chat of a record without sampling anything. Reads only.
    pub fn study_record_info(&self, user: i64, record_id: i64) -> Result<Option<Value>, DbError> {
        let g = self.lock();
        let Some(rec) = load_record(&g, user, record_id)? else {
            return Ok(None);
        };
        Ok(Some(json!({
            "ok": true,
            "uiLang": ui_lang(&g, user)?,
            "record": {
                "id": rec.id,
                "at": rec.at,
                "kind": rec.kind,
                "key": rec.key,
                "startedAt": rec.started_at,
            },
        })))
    }
}

/// The keys of a JSON object in order (tests that compare key lists).
#[cfg(test)]
fn keys(v: &Value) -> Vec<String> {
    v.as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Explain, Item, Opt};

    fn opt(key: &str, text: &str) -> Opt {
        Opt {
            key: key.to_string(),
            text: text.to_string(),
        }
    }

    fn zh(stem: &str, options: &[&str]) -> Zh {
        Zh {
            stem: stem.to_string(),
            options: Some(options.iter().map(|s| s.to_string()).collect()),
            items: None,
            choices: None,
        }
    }

    /// A made-up six-column single-choice question (English base, Chinese and Japanese translations, per-language
    /// explanations).
    fn six_column() -> Question {
        Question {
            kind: Kind::Single,
            tag: "tag".into(),
            stem: "Which fruit is red?".into(),
            options: vec![opt("A", "Apple"), opt("B", "Banana"), opt("C", "Cherry")],
            items: vec![],
            answer: Answer::Letters(vec!["A".into()]),
            translation: "RAW-TRANSLATION".into(),
            zh: Some(zh("哪种水果是红色的？", &["苹果", "香蕉", "樱桃"])),
            english: String::new(),
            en: None,
            lang: "en".into(),
            ja: Some(zh("赤い果物はどれ？", &["りんご", "バナナ", "さくらんぼ"])),
            explanation: "COMPAT-ZH".into(),
            mnemonic: String::new(),
            note: String::new(),
            explain: Some(Explain {
                en: Some(ExplainText {
                    explanation: "Apples are red.".into(),
                    mnemonic: "A for apple".into(),
                    note: "".into(),
                }),
                zh: Some(ExplainText {
                    explanation: "苹果是红色的。".into(),
                    mnemonic: "".into(),
                    note: "".into(),
                }),
                ja: None,
            }),
            id: String::new(),
            sheet: "SHEET".into(),
            row: 7,
        }
    }

    fn hotspot() -> Question {
        let mut q = six_column();
        q.kind = Kind::Hotspot;
        q.stem = "Judge each statement.".into();
        q.options = vec![];
        q.items = vec![
            Item {
                text: "Water is wet.".into(),
                choices: vec!["Yes".into(), "No".into()],
            },
            Item {
                text: "Fire is cold.".into(),
                choices: vec!["Yes".into(), "No".into()],
            },
        ];
        q.answer = Answer::Indexes(vec![0, 1]);
        q.zh = Some(Zh {
            stem: "判断每句话。".into(),
            options: None,
            items: Some(vec!["水是湿的。".into(), "火是冷的。".into()]),
            choices: Some(vec![
                vec!["是".into(), "否".into()],
                vec!["是".into(), "否".into()],
            ]),
        });
        q.ja = None;
        q
    }

    fn old_layout() -> Question {
        // 3-column layout: Japanese base text (no lang), Chinese translation, one compatibility explanation
        let mut q = six_column();
        q.lang = String::new();
        q.stem = "どの果物が赤い？".into();
        q.options = vec![opt("A", "りんご"), opt("B", "バナナ")];
        q.zh = Some(zh("哪种水果是红色的？", &["苹果", "香蕉"]));
        q.ja = None;
        q.explain = None;
        q.explanation = "りんごは赤い。".into();
        q.mnemonic = "覚え方".into();
        q.note = "確信度: 高".into();
        q
    }

    #[test]
    fn cut_text_keeps_at_most_cap_characters_and_ends_with_an_ellipsis() {
        assert_eq!(cut_text("abc", 3), ("abc".to_string(), false));
        assert_eq!(cut_text("abcd", 3), ("ab…".to_string(), true));
        assert_eq!(
            cut_text("日本語のテキスト", 4),
            ("日本語…".to_string(), true)
        );
        assert_eq!(cut_text("", 0), (String::new(), false));
        assert_eq!(cut_text("x", 0), (String::new(), true));
        let (t, cut) = cut_text(&"a".repeat(7000), 6000);
        assert!(cut);
        assert_eq!(t.chars().count(), 6000);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn the_question_projection_has_exactly_the_hint_safe_keys() {
        let q = six_column();
        let v = project_question("abc123", true, &q, StudyLang::En, &FULL);
        assert_eq!(
            keys(&v),
            ["id", "kind", "lang", "stem", "options", "items", "active", "cut"]
        );
        let text = v.to_string();
        for forbidden in ["RAW-TRANSLATION", "SHEET", "tag", "answer", "explanation"] {
            assert!(!text.contains(forbidden), "{forbidden} leaked: {text}");
        }
        assert_eq!(v["options"][0], json!({"key": "A", "text": "Apple"}));
        assert_eq!(v["items"], json!([]));
        assert_eq!(v["cut"], json!(false));
        assert_eq!(v["kind"], json!("single"));
    }

    #[test]
    fn the_question_language_follows_the_translations_and_falls_back_to_the_base() {
        let q = six_column();
        let zh_v = project_question("i", true, &q, StudyLang::Zh, &FULL);
        assert_eq!(zh_v["lang"], json!("zh"));
        assert_eq!(zh_v["stem"], json!("哪种水果是红色的？"));
        assert_eq!(zh_v["options"][1], json!({"key": "B", "text": "香蕉"}));
        let ja_v = project_question("i", true, &q, StudyLang::Ja, &FULL);
        assert_eq!(ja_v["lang"], json!("ja"));
        assert_eq!(ja_v["options"][2]["text"], json!("さくらんぼ"));
        // A six-column question has no `en` structure: English is its base text
        let en_v = project_question("i", true, &q, StudyLang::En, &FULL);
        assert_eq!(en_v["lang"], json!("en"));
        // Without the Japanese translation the base language comes back and says so
        let mut no_ja = six_column();
        no_ja.ja = None;
        let v = project_question("i", true, &no_ja, StudyLang::Ja, &FULL);
        assert_eq!(v["lang"], json!("en"));
        assert_eq!(v["stem"], json!("Which fruit is red?"));
        // An option the translation does not list keeps the base text
        let mut short = six_column();
        short.zh = Some(zh("短", &["苹果"]));
        let v = project_question("i", true, &short, StudyLang::Zh, &FULL);
        assert_eq!(v["options"][0]["text"], json!("苹果"));
        assert_eq!(v["options"][2]["text"], json!("Cherry"));
    }

    #[test]
    fn an_old_layout_question_has_a_japanese_base_and_compatibility_text() {
        let q = old_layout();
        assert_eq!(base_lang(&q), StudyLang::Ja);
        let v = project_question("i", false, &q, StudyLang::Ja, &FULL);
        assert_eq!(v["lang"], json!("ja"));
        assert_eq!(v["active"], json!(false));
        // English is asked but there is no `en` structure: base language
        let v = project_question("i", true, &q, StudyLang::En, &FULL);
        assert_eq!(v["lang"], json!("ja"));
        let s = project_solution(&q, StudyLang::Ja, StudyLang::En, &FULL);
        assert_eq!(s["explainLang"], json!(""));
        assert_eq!(s["explanation"], json!("りんごは赤い。"));
        assert_eq!(s["mnemonic"], json!("覚え方"));
        assert_eq!(s["note"], json!("確信度: 高"));
        // The four-column layout keeps an English structure in `en`
        let mut four = old_layout();
        four.en = Some(zh("Which fruit is red?", &["Apple", "Banana"]));
        let v = project_question("i", true, &four, StudyLang::En, &FULL);
        assert_eq!(v["lang"], json!("en"));
        assert_eq!(v["options"][0]["text"], json!("Apple"));
    }

    #[test]
    fn the_explanation_follows_the_preference_chain() {
        let q = six_column();
        let pick = |e, ql| project_solution(&q, ql, e, &FULL);
        // elang with text wins
        let s = pick(StudyLang::Zh, StudyLang::En);
        assert_eq!(
            (s["explainLang"].as_str(), s["explanation"].as_str()),
            (Some("zh"), Some("苹果是红色的。"))
        );
        // elang without text (ja is missing) -> the question's language
        let s = pick(StudyLang::Ja, StudyLang::Zh);
        assert_eq!(s["explainLang"], json!("zh"));
        // neither -> English, then Chinese, then Japanese
        let s = pick(StudyLang::Ja, StudyLang::Ja);
        assert_eq!(s["explainLang"], json!("en"));
        let mut only_ja = six_column();
        only_ja.explain = Some(Explain {
            en: None,
            zh: None,
            ja: Some(ExplainText {
                explanation: "りんごは赤い。".into(),
                mnemonic: String::new(),
                note: String::new(),
            }),
        });
        let s = project_solution(&only_ja, StudyLang::En, StudyLang::En, &FULL);
        assert_eq!(s["explainLang"], json!("ja"));
        // An empty per-language text does not count
        let mut empty = six_column();
        empty.explain = Some(Explain {
            en: Some(ExplainText::default()),
            zh: None,
            ja: None,
        });
        let s = project_solution(&empty, StudyLang::En, StudyLang::En, &FULL);
        assert_eq!(s["explainLang"], json!(""));
        assert_eq!(s["explanation"], json!("COMPAT-ZH"));
    }

    #[test]
    fn the_solution_has_exactly_its_keys_and_the_key_texts_in_every_language() {
        let q = six_column();
        let s = project_solution(&q, StudyLang::En, StudyLang::En, &FULL);
        assert_eq!(
            keys(&s),
            [
                "answer",
                "explanation",
                "mnemonic",
                "note",
                "explainLang",
                "keyTexts"
            ]
        );
        assert_eq!(s["answer"], json!(["A"]));
        assert_eq!(s["keyTexts"], json!(["Apple", "苹果", "りんご"]));
        // Compact: no key texts, no mnemonic / note, explanation cut to 400
        let mut long = six_column();
        if let Some(ex) = long.explain.as_mut() {
            if let Some(en) = ex.en.as_mut() {
                en.explanation = "e".repeat(900);
            }
        }
        let c = project_solution(&long, StudyLang::En, StudyLang::En, &COMPACT);
        assert_eq!(c["keyTexts"], json!([]));
        assert_eq!(c["mnemonic"], json!(""));
        assert_eq!(c["note"], json!(""));
        assert_eq!(
            c["explanation"].as_str().map(|s| s.chars().count()),
            Some(400)
        );
        // Multi-choice: several correct options, de-duplicated when the texts coincide across languages
        let mut multi = six_column();
        multi.kind = Kind::Multi;
        multi.answer = Answer::Letters(vec!["A".into(), "C".into()]);
        multi.zh = Some(zh("多选", &["Apple", "香蕉", "樱桃"]));
        let s = project_solution(&multi, StudyLang::En, StudyLang::En, &FULL);
        assert_eq!(
            s["keyTexts"],
            json!(["Apple", "りんご", "Cherry", "樱桃", "さくらんぼ"])
        );
    }

    #[test]
    fn hotspot_questions_project_items_choices_and_key_texts() {
        let q = hotspot();
        let v = project_question("h", true, &q, StudyLang::Zh, &FULL);
        assert_eq!(v["options"], json!([]));
        assert_eq!(
            v["items"][0],
            json!({"text": "水是湿的。", "choices": ["是", "否"]})
        );
        // Japanese is missing: the base language, with the base choices
        let v = project_question("h", true, &q, StudyLang::Ja, &FULL);
        assert_eq!(v["lang"], json!("en"));
        assert_eq!(
            v["items"][1],
            json!({"text": "Fire is cold.", "choices": ["Yes", "No"]})
        );
        let s = project_solution(&q, StudyLang::En, StudyLang::En, &FULL);
        assert_eq!(s["answer"], json!([0, 1]));
        assert_eq!(s["keyTexts"], json!(["Yes", "是", "No", "否"]));
        // This language writes no choices: the base ones are shown
        let mut plain = hotspot();
        if let Some(z) = plain.zh.as_mut() {
            z.choices = None;
        }
        let v = project_question("h", true, &plain, StudyLang::Zh, &FULL);
        assert_eq!(v["items"][0]["choices"], json!(["Yes", "No"]));
    }

    #[test]
    fn caps_cut_texts_and_counts_and_say_so() {
        let mut q = six_column();
        q.stem = "s".repeat(1300);
        q.options = (0..10)
            .map(|i| opt(&((b'A' + i) as char).to_string(), &"o".repeat(400)))
            .collect();
        q.zh = None;
        q.ja = None;
        let v = project_question("c", true, &q, StudyLang::En, &COMPACT);
        assert_eq!(v["cut"], json!(true));
        assert_eq!(v["stem"].as_str().map(|s| s.chars().count()), Some(1200));
        assert_eq!(v["options"].as_array().map(Vec::len), Some(8));
        assert_eq!(
            v["options"][0]["text"].as_str().map(|s| s.chars().count()),
            Some(300)
        );
        // The same question with the full caps: the stem fits, 8 options, each 400 < 1500 chars
        let v = project_question("c", true, &q, StudyLang::En, &FULL);
        assert_eq!(v["cut"], json!(true), "only the option count is cut");
        assert_eq!(v["stem"].as_str().map(|s| s.chars().count()), Some(1300));
        // Within every cap: not cut
        let small = project_question("c", true, &six_column(), StudyLang::En, &COMPACT);
        assert_eq!(small["cut"], json!(false));
        // Hotspot counts: 25 items with 12 choices
        let mut h = hotspot();
        h.items = (0..25)
            .map(|i| Item {
                text: format!("t{i}"),
                choices: (0..12).map(|j| format!("c{j}")).collect(),
            })
            .collect();
        let v = project_question("h", true, &h, StudyLang::En, &COMPACT);
        assert_eq!(v["items"].as_array().map(Vec::len), Some(20));
        assert_eq!(v["items"][0]["choices"].as_array().map(Vec::len), Some(10));
        assert_eq!(v["cut"], json!(true));
    }

    #[test]
    fn user_answers_pass_through_and_misfits_become_null() {
        let q = six_column();
        let ans = |s: &str| serde_json::from_str::<Vec<Value>>(s).expect("json");
        assert_eq!(
            clean_user_answer(&ans(r#"["A","C"]"#), &q),
            ans(r#"["A","C"]"#)
        );
        assert_eq!(clean_user_answer(&[], &q), Vec::<Value>::new());
        // a letter that is not an option key, a number and a null
        assert_eq!(
            clean_user_answer(&ans(r#"["Z", 1, null, "B"]"#), &q),
            ans(r#"[null, null, null, "B"]"#)
        );
        let h = hotspot();
        assert_eq!(
            clean_user_answer(&ans("[0, null, 1]"), &h),
            ans("[0, null, null]")
        );
        // 0 and 1 fit item 0 and item 1; item index 2 does not exist
        assert_eq!(
            clean_user_answer(&ans("[1, 1, 0]"), &h),
            ans("[1, 1, null]")
        );
        // index past the choices, a string, a negative number, a float
        assert_eq!(
            clean_user_answer(&ans(r#"[2, "A", -1]"#), &h),
            ans("[null, null, null]")
        );
        assert_eq!(
            clean_user_answer(&ans("[0.0, 1.5]"), &h),
            ans("[null, null]")
        );
    }

    #[test]
    fn sampling_is_without_replacement_sorted_and_uses_the_injected_draw() {
        // Always drawing 0 takes the first k of the shuffled order: positions 0..k
        let mut zero = |_m: usize| 0;
        assert_eq!(sample_indices(5, 3, &mut zero), vec![0, 1, 2]);
        // Always drawing the last remaining position
        let mut last = |m: usize| m - 1;
        // (the shuffle swaps position 5 into slot 0, then the new last into slot 1 ...: the picked set is {0, 1, 5})
        assert_eq!(sample_indices(6, 3, &mut last), vec![0, 1, 5]);
        // k >= n: everything, no draw at all
        let mut never = |_m: usize| -> usize { panic!("no draw expected") };
        assert_eq!(sample_indices(3, 3, &mut never), vec![0, 1, 2]);
        assert_eq!(sample_indices(3, 70, &mut never), vec![0, 1, 2]);
        assert_eq!(sample_indices(0, 5, &mut never), Vec::<usize>::new());
        assert_eq!(sample_indices(4, 0, &mut zero), Vec::<usize>::new());
        // An out-of-range draw is clamped
        let mut wild = |_m: usize| 1_000_000;
        assert_eq!(sample_indices(4, 2, &mut wild), vec![0, 3]);
        // The bound handed to the draw shrinks by one each step
        let mut bounds = Vec::new();
        let mut spy = |m: usize| {
            bounds.push(m);
            0
        };
        sample_indices(10, 3, &mut spy);
        assert_eq!(bounds, vec![10, 9, 8]);
    }

    #[test]
    fn a_real_draw_gives_distinct_sorted_positions_in_range() {
        for _ in 0..50 {
            let s = sample_indices(100, 70, &mut random_below);
            assert_eq!(s.len(), 70);
            assert!(s.windows(2).all(|w| w[0] < w[1]), "sorted and distinct");
            assert!(s.iter().all(|&i| i < 100));
        }
        for bound in [1_usize, 2, 3, 7, 100] {
            for _ in 0..200 {
                assert!(random_below(bound) < bound);
            }
        }
        // Every position is reachable (a sanity check on bias: 4000 draws of 0..4 hit each value)
        let mut seen = [0_u32; 4];
        for _ in 0..4000 {
            seen[random_below(4)] += 1;
        }
        assert!(seen.iter().all(|&n| n > 700), "{seen:?}");
    }

    /// An item as the record route builds it: a question with `n_opts` options of `len` characters each, and an explanation.
    fn item_of(n_opts: usize, len: usize, expl: usize) -> Value {
        let mut q = six_column();
        q.stem = "s".repeat(len);
        q.options = (0..n_opts)
            .map(|i| opt(&((b'A' + i as u8) as char).to_string(), &"o".repeat(len)))
            .collect();
        q.zh = None;
        q.ja = None;
        q.explain = Some(Explain {
            en: Some(ExplainText {
                explanation: "e".repeat(expl),
                mnemonic: String::new(),
                note: String::new(),
            }),
            zh: None,
            ja: None,
        });
        json!({
            "seq": 0,
            "answer": [],
            "question": project_question("x", true, &q, StudyLang::En, &COMPACT),
            "solution": project_solution(&q, StudyLang::En, StudyLang::En, &COMPACT),
        })
    }

    fn total(items: &[Value]) -> usize {
        items
            .iter()
            .map(|i| item_text_len(&i["question"], &i["solution"]))
            .sum()
    }

    #[test]
    fn a_record_within_the_text_budget_is_not_touched() {
        let mut items: Vec<Value> = (0..5).map(|_| item_of(4, 100, 100)).collect();
        let before = items.clone();
        assert!(!apply_budget(&mut items, TEXT_BUDGET));
        assert_eq!(items, before);
        // exactly at the budget is within it
        let size = total(&items);
        assert!(!apply_budget(&mut items, size));
        assert!(apply_budget(&mut items, size - 1));
    }

    #[test]
    fn an_over_budget_record_is_cut_to_fair_shares_that_keep_the_letters() {
        // one small item (stays whole) and three large ones
        let mut items = vec![
            item_of(2, 10, 10),
            item_of(8, 300, 400),
            item_of(8, 300, 400),
            item_of(8, 300, 400),
        ];
        let small = items[0].clone();
        let large_len = item_text_len(&items[1]["question"], &items[1]["solution"]);
        let budget = 3000;
        assert!(total(&items) > budget);
        assert!(apply_budget(&mut items, budget));
        assert!(total(&items) <= budget, "{} > {budget}", total(&items));
        assert_eq!(items[0], small, "an item below its share keeps everything");
        assert_eq!(items[0]["question"]["cut"], json!(false));
        let small_len = item_text_len(&small["question"], &small["solution"]);
        // the three large items share what the small one left over, equally (to within rounding)
        for item in &items[1..4] {
            let n = item_text_len(&item["question"], &item["solution"]);
            let share = (budget - small_len) / 3;
            assert!(
                n <= share && n + 30 >= share.min(large_len),
                "{n} vs {share}"
            );
            assert_eq!(item["question"]["cut"], json!(true));
            // every option is still there with its letter
            let opts = item["question"]["options"].as_array().expect("options");
            assert_eq!(opts.len(), 8);
            assert_eq!(opts[7]["key"], json!("H"));
        }
        // Equal items are cut equally
        assert_eq!(items[1], items[2]);
        assert_eq!(items[2], items[3]);
    }

    #[test]
    fn seventy_maximal_hotspot_items_fit_the_budget() {
        let mut h = hotspot();
        h.stem = "s".repeat(1200);
        h.items = (0..20)
            .map(|_| Item {
                text: "t".repeat(300),
                choices: (0..10).map(|_| "c".repeat(80)).collect(),
            })
            .collect();
        h.answer = Answer::Indexes(vec![0; 20]);
        h.zh = None;
        let one = json!({
            "question": project_question("h", true, &h, StudyLang::En, &COMPACT),
            "solution": project_solution(&h, StudyLang::En, StudyLang::En, &COMPACT),
        });
        let mut items: Vec<Value> = (0..70).map(|_| one.clone()).collect();
        assert!(
            total(&items) > TEXT_BUDGET * 10,
            "the caps alone would allow far more"
        );
        assert!(apply_budget(&mut items, TEXT_BUDGET));
        assert!(total(&items) <= TEXT_BUDGET, "{}", total(&items));
        assert!(items.iter().all(|i| i["question"]["cut"] == json!(true)));
        assert!(items
            .iter()
            .all(|i| i["question"]["items"].as_array().map(Vec::len) == Some(20)));
        // The item structure survives: 20 statements with 10 choices each
        assert!(items.iter().all(|i| i["question"]["items"][0]["choices"]
            .as_array()
            .map(Vec::len)
            == Some(10)));
    }

    #[test]
    fn an_absurdly_small_budget_still_keeps_the_structure() {
        let mut items = vec![item_of(3, 50, 50), item_of(3, 50, 50)];
        assert!(apply_budget(&mut items, 10));
        assert!(total(&items) <= 10);
        assert_eq!(items[0]["question"]["options"][2]["key"], json!("C"));
        assert!(!apply_budget(&mut [], 0));
        assert!(apply_budget(&mut [item_of(1, 5, 5)], 0));
    }

    #[test]
    fn languages_parse_only_the_three_codes() {
        for (s, l) in [
            ("en", StudyLang::En),
            ("zh", StudyLang::Zh),
            ("ja", StudyLang::Ja),
        ] {
            assert_eq!(StudyLang::parse(s), Some(l));
            assert_eq!(l.as_str(), s);
        }
        for bad in ["", "zh-CN", "EN", "fr", "ja "] {
            assert_eq!(StudyLang::parse(bad), None, "{bad:?}");
        }
    }
}
