//! v1.1 AI-generated banks: what the AI service reads to plan a generation and the one write it makes, through the token
//! routes `POST /internal/ai/gen/source`, `/gen/seeds`, `/gen/create` and `/gen/lookup` (see `api/internal_gen.rs`).
//!
//! The AI service never opens this database and never writes it. It asks, with the caller's live session, and the backend
//! alone creates the bank:
//!
//! - [`Store::gen_source`]: the source questions of one generation (a bank the caller manages, the checked sheets, or the
//!   wrong-answer questions of the checked sheets) as a stratified, capped, stem-only listing with a difficulty label. Reads
//!   one short statement per (bank, sheet), taking the store lock per sheet (one statement over a big library would hold the
//!   backend's only lock for up to a second). Answers `tooFew` before sampling and before the difficulty statement.
//! - [`Store::gen_seeds`]: the full text (capped per language) of the chosen seed questions.
//! - [`Store::create_ai_bank`]: ONE transaction that makes the bank (idempotent by the job key), shares it like any new bank
//!   (see [`AI_BANKS_AUTO_SHARE`]) and writes its questions, one sheet, no `imports` row.
//! - [`Store::ai_bank_of_job`]: the bank of a job key (the AI service's restart repair).
//!
//! Rules: AI banks are never a source (any AI bank in a scope / wrong / bank request is `aiSource`, A-OWNER-52b);
//! only the viewer's visible banks are read; bank mode needs a bank the viewer may manage (the banks Bank management lists).
//! The permission to generate is "administrator, or AI+ on and may manage banks".
use super::banks::{bank_owner, insert_bank, is_ai_bank, unique_name, viewer_of, NewBankShare};
use super::{question_data_json, Store};
use crate::database::MASTER_STREAK;
use crate::error::DbError;
use crate::models::{
    Answer, Explain, ExplainText, Kind, Opt, Question, Viewer, Zh, SIX_COLUMN_LANG,
};
use crate::parser::make_id;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Whether an AI bank is shared like any new bank: the users with "include banks imported later" who can see every source
/// bank get a `user_banks` row (owner decision A-OWNER-13d: `true`). An administrator removes the bank from a user in User
/// management like any other bank's access. `false` = only the author and the administrators see a new AI bank; the switch
/// is kept and tested both ways ([`Store::create_ai_bank_shared`]).
pub const AI_BANKS_AUTO_SHARE: bool = true;

/// The name of the one sheet of an AI bank (the AI service sends it; kept here for the tests and the docs).
pub const AI_SHEET: &str = "AI";

/// Characters of one stem in `gen_source` items and `previous` (the last one is `…` when cut).
const SOURCE_STEM_CHARS: usize = 160;
/// Characters per language of a seed (`gen_seeds`): stem, option, explanation (the AI service's prompt caps).
const SEED_STEM_CHARS: usize = 600;
const SEED_OPTION_CHARS: usize = 150;
const SEED_EXPL_CHARS: usize = 400;
/// Most stems `previous` carries.
const PREVIOUS_MAX: usize = 200;
/// Attempts below this give no difficulty label.
const DIFF_MIN_ATTEMPTS: i64 = 5;

/// One text in the three languages.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tri {
    pub en: String,
    pub zh: String,
    pub ja: String,
}

impl Tri {
    fn to_json(&self) -> Value {
        json!({ "en": self.en, "zh": self.zh, "ja": self.ja })
    }
}

/// Where the source questions come from (`"source"` of the source and create routes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenKind {
    /// Every active question of one bank the caller manages
    Bank,
    /// The listed sheets
    Scope,
    /// The caller's wrong-answer questions of the listed sheets
    Wrong,
    /// An uploaded file (create route only: the source route refuses it)
    File,
}

impl GenKind {
    pub fn parse(s: &str) -> Option<GenKind> {
        match s {
            "bank" => Some(GenKind::Bank),
            "scope" => Some(GenKind::Scope),
            "wrong" => Some(GenKind::Wrong),
            "file" => Some(GenKind::File),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            GenKind::Bank => "bank",
            GenKind::Scope => "scope",
            GenKind::Wrong => "wrong",
            GenKind::File => "file",
        }
    }
}

/// What `POST /internal/ai/gen/source` asks (after validation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenSourceReq {
    pub kind: GenKind,
    /// The bank of `Bank` mode (ignored otherwise)
    pub bank: Option<i64>,
    /// The (bank, sheet) pairs of `Scope` / `Wrong` mode (ignored for `Bank`; duplicates are ignored)
    pub sheets: Vec<(i64, String)>,
    /// Fewer source questions than this: `TooFew` (1 ..= 2000)
    pub min: usize,
    /// Most items returned (1 ..= 2000)
    pub max: usize,
}

/// One question of `GenFound::items`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenItem {
    pub id: String,
    /// Index into `GenFound::strata`
    pub stratum: usize,
    pub multi: bool,
    pub opts: usize,
    pub right: usize,
    /// `"easy"` | `"mid"` | `"hard"`, `None` below five attempts
    pub diff: Option<&'static str>,
    pub stem: Tri,
}

/// One (bank, sheet) of the source with its counts (all its questions, not only the returned ones).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenStratum {
    pub bank: i64,
    pub sheet: String,
    pub n: usize,
    pub multi: usize,
}

/// A successful `gen_source`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenFound {
    pub user_id: i64,
    pub username: String,
    pub role: String,
    /// Every eligible source question (before the `max` sampling)
    pub total: usize,
    /// The source banks (id, current name), by id
    pub banks: Vec<(i64, String)>,
    pub strata: Vec<GenStratum>,
    pub items: Vec<GenItem>,
    /// `(en, zh)` stems of earlier AI banks derived from the same source banks
    pub previous: Vec<(String, String)>,
}

/// The answer of [`Store::gen_source`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenSource {
    /// Not an administrator and not (AI+ on and may manage banks)
    NotAllowed,
    /// No such bank / sheet, not visible, or (bank mode) not manageable by the caller
    NotFound,
    /// Bank mode on an AI bank
    AiSource,
    /// Fewer eligible questions than `min`
    TooFew {
        have: usize,
    },
    Found(Box<GenFound>),
}

impl GenSource {
    /// The route's answer (HTTP 200 in every case): `{"ok": true, ...}` or `{"ok": false, "reason": ...}`.
    pub fn to_json(&self) -> Value {
        match self {
            GenSource::NotAllowed => json!({"ok": false, "reason": "notAllowed"}),
            GenSource::NotFound => json!({"ok": false, "reason": "notFound"}),
            GenSource::AiSource => json!({"ok": false, "reason": "aiSource"}),
            GenSource::TooFew { have } => json!({"ok": false, "reason": "tooFew", "have": have}),
            GenSource::Found(f) => json!({
                "ok": true,
                "user": {"id": f.user_id, "username": f.username, "role": f.role},
                "total": f.total,
                "banks": f.banks.iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>(),
                "strata": f.strata.iter().map(|s| json!({
                    "bank": s.bank, "sheet": s.sheet, "n": s.n, "multi": s.multi,
                })).collect::<Vec<_>>(),
                "items": f.items.iter().map(|i| json!({
                    "id": i.id,
                    "s": i.stratum,
                    "kind": if i.multi { "multi" } else { "single" },
                    "opts": i.opts,
                    "right": i.right,
                    "diff": i.diff,
                    "stem": i.stem.to_json(),
                })).collect::<Vec<_>>(),
                "previous": f.previous.iter().map(|(en, zh)| json!({"en": en, "zh": zh})).collect::<Vec<_>>(),
            }),
        }
    }
}

/// The answer of [`Store::gen_seeds`].
#[derive(Debug, Clone, PartialEq)]
pub enum GenSeeds {
    NotAllowed,
    /// In request order; ids that are not usable are left out
    Items(Vec<Value>),
}

impl GenSeeds {
    pub fn to_json(&self) -> Value {
        match self {
            GenSeeds::NotAllowed => json!({"ok": false, "reason": "notAllowed"}),
            GenSeeds::Items(items) => json!({"ok": true, "items": items}),
        }
    }
}

/// One generated question of a create request (validated by the route).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiQuestion {
    pub multi: bool,
    pub stem: Tri,
    pub options: Vec<Tri>,
    /// Letters, sorted (`["B"]`, `["A", "C"]`)
    pub answer: Vec<String>,
    pub explanation: Tri,
}

/// What the AI service read from an uploaded file (`source: "file"`): the cleaned name and the type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiFile {
    pub name: String,
    pub ty: String,
}

/// How a file was turned into questions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiMode {
    /// The file held questions: converted as they are (some may be skipped)
    Questions,
    /// The file was study material: new questions written from it
    Material,
}

impl AiMode {
    pub fn as_str(self) -> &'static str {
        match self {
            AiMode::Questions => "questions",
            AiMode::Material => "material",
        }
    }
}

/// What `POST /internal/ai/gen/create` asks (after validation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiBankReq {
    /// The job key (32 lower-case hex digits): one bank per key
    pub job: String,
    pub user: i64,
    /// The user's identity stamp (`users.uid`)
    pub stamp: String,
    pub name: String,
    pub kind: GenKind,
    /// The source banks (ids)
    pub from: Vec<i64>,
    pub goal: usize,
    /// `YYYY-MM-DD HH:MM:SS`
    pub at: String,
    pub sheet: String,
    pub questions: Vec<AiQuestion>,
    /// File source only: the file facts and the mode
    pub file: Option<(AiFile, AiMode)>,
    /// File source, questions mode only: the 1-based numbers of the questions that could not be converted (sorted)
    pub skipped: Vec<i64>,
}

/// The answer of [`Store::create_ai_bank`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiBankOutcome {
    Created {
        bank_id: i64,
        name: String,
        /// The author's username (for the log line)
        username: String,
    },
    /// The job key already has its bank: nothing was written
    Existing { bank_id: i64, name: String },
    /// No user with that id and stamp, or disabled
    NoUser,
    /// Not an administrator and not (AI+ on and may manage banks)
    NotAllowed,
}

/// The bank of a job key (`Store::ai_bank_of_job`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiBankInfo {
    pub id: i64,
    pub name: String,
    pub made: i64,
    pub goal: i64,
}

// ---------------------------------------------------------------- pure parts

/// `s` cut to at most `cap` characters; a cut text ends with `…` (so it has exactly `cap` characters).
fn cut(s: &str, cap: usize) -> String {
    if s.chars().count() <= cap {
        return s.to_string();
    }
    if cap == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(cap - 1).collect();
    out.push('…');
    out
}

/// How many of `max` items each stratum contributes (Hamilton's largest-remainder method): in proportion to the stratum
/// sizes `ns`, at least one per stratum while `max` allows, never more than the stratum has, the sum `min(max, Σ ns)`.
/// Deterministic: ties go to the earlier stratum.
pub fn hamilton(ns: &[usize], max: usize) -> Vec<usize> {
    let total: usize = ns.iter().sum();
    if total == 0 || ns.is_empty() {
        return vec![0; ns.len()];
    }
    if max >= total {
        return ns.to_vec();
    }
    let mut quota: Vec<usize> = ns.iter().map(|n| max * n / total).collect();
    let mut rest = max - quota.iter().sum::<usize>();
    // The largest remainders get one more each
    let mut order: Vec<usize> = (0..ns.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(max * ns[i] % total), i));
    for &i in &order {
        if rest == 0 {
            break;
        }
        if quota[i] < ns[i] {
            quota[i] += 1;
            rest -= 1;
        }
    }
    // At least one per stratum while possible: take it from the largest quota (the latest on a tie) that has more than one
    let nonempty = ns.iter().filter(|n| **n > 0).count();
    if max >= nonempty {
        for i in 0..ns.len() {
            if quota[i] == 0 && ns[i] > 0 {
                let donor = (0..ns.len())
                    .filter(|&j| quota[j] > 1)
                    .max_by_key(|&j| (quota[j], j));
                if let Some(j) = donor {
                    quota[j] -= 1;
                    quota[i] = 1;
                }
            }
        }
    } else {
        // Fewer places than strata: the `max` largest strata (the earlier on a tie) get one each
        quota = vec![0; ns.len()];
        let mut by_size: Vec<usize> = (0..ns.len()).filter(|&i| ns[i] > 0).collect();
        by_size.sort_by_key(|&i| (std::cmp::Reverse(ns[i]), i));
        for &i in by_size.iter().take(max) {
            quota[i] = 1;
        }
    }
    quota
}

/// The positions (ascending) of `q` items chosen from `n` in a stratum, evenly spread (every k-th, centred), so the choice is
/// deterministic and covers the sheet from start to end.
pub fn spread(n: usize, q: usize) -> Vec<usize> {
    if q >= n {
        return (0..n).collect();
    }
    (0..q).map(|j| (2 * j + 1) * n / (2 * q)).collect()
}

/// The difficulty label of a question from its summed statistics: `None` below [`DIFF_MIN_ATTEMPTS`] attempts, `"easy"` when
/// fewer than 20 % were wrong, `"mid"` up to 50 %, else `"hard"` (integer arithmetic, no rounding).
pub fn difficulty(attempts: i64, wrong: i64) -> Option<&'static str> {
    if attempts < DIFF_MIN_ATTEMPTS {
        None
    } else if wrong * 5 < attempts {
        Some("easy")
    } else if wrong * 2 <= attempts {
        Some("mid")
    } else {
        Some("hard")
    }
}

fn letter(i: usize) -> String {
    // 3 ..= 6 options (validated by the route); `A` + i never leaves the alphabet for any realistic count
    char::from(b'A' + u8::try_from(i.min(25)).unwrap_or(25)).to_string()
}

/// The stored form of a generated question (GB-5): six-column, English base text, `zh` / `ja` translations aligned to the
/// English options, the column-B style `translation`, an explanation per language and the Chinese one as the compatibility
/// text. Id, sheet and row are set by the caller.
pub fn build_question(q: &AiQuestion) -> Question {
    let options: Vec<Opt> = q
        .options
        .iter()
        .enumerate()
        .map(|(i, o)| Opt {
            key: letter(i),
            text: o.en.clone(),
        })
        .collect();
    let zh_options: Vec<String> = q.options.iter().map(|o| o.zh.clone()).collect();
    let ja_options: Vec<String> = q.options.iter().map(|o| o.ja.clone()).collect();
    let mut translation = q.stem.zh.clone();
    for (i, o) in zh_options.iter().enumerate() {
        translation.push('\n');
        translation.push_str(&format!("{}. {o}", letter(i)));
    }
    let text = |t: &str| ExplainText {
        explanation: t.to_string(),
        mnemonic: String::new(),
        note: String::new(),
    };
    Question {
        kind: if q.multi { Kind::Multi } else { Kind::Single },
        tag: String::new(),
        stem: q.stem.en.clone(),
        options,
        items: Vec::new(),
        answer: Answer::Letters(q.answer.clone()),
        translation,
        zh: Some(Zh {
            stem: q.stem.zh.clone(),
            options: Some(zh_options),
            items: None,
            choices: None,
        }),
        english: String::new(),
        en: None,
        lang: SIX_COLUMN_LANG.to_string(),
        ja: Some(Zh {
            stem: q.stem.ja.clone(),
            options: Some(ja_options),
            items: None,
            choices: None,
        }),
        explanation: q.explanation.zh.clone(),
        mnemonic: String::new(),
        note: String::new(),
        explain: Some(Explain {
            en: Some(text(&q.explanation.en)),
            zh: Some(text(&q.explanation.zh)),
            ja: Some(text(&q.explanation.ja)),
        }),
        id: String::new(),
        sheet: String::new(),
        row: 0,
    }
}

/// The id of generated question `q` in bank `bank_id` (the parser's id rule, then bank-scoped).
pub fn ai_question_id(bank_id: i64, sheet: &str, q: &AiQuestion) -> String {
    let first = q.options.first().map(|o| o.en.as_str()).unwrap_or("");
    super::banks::scoped_id(bank_id, &make_id(sheet, &q.stem.en, first))
}

/// The id seed of question number `row` (1-based): the sheet name for generated banks; for a file import `AI<row>`, so a file
/// that repeats a question still gets distinct ids.
pub fn id_seed(req_kind: GenKind, sheet: &str, row: usize) -> String {
    if req_kind == GenKind::File {
        format!("AI{row}")
    } else {
        sheet.to_string()
    }
}

/// The id of question number `row` of a request in bank `bank_id`.
fn request_question_id(bank_id: i64, req: &AiBankReq, row: usize, q: &AiQuestion) -> String {
    ai_question_id(bank_id, &id_seed(req.kind, &req.sheet, row), q)
}

/// The `banks.ai_meta` text of a new AI bank (compact JSON; `from` names are the banks' current names, `null` for a bank
/// that is gone).
fn ai_meta_json(
    req: &AiBankReq,
    username: &str,
    names: &[(i64, Option<String>)],
) -> Result<String, DbError> {
    let from: Vec<Value> = names
        .iter()
        .map(|(id, name)| json!({ "id": id, "name": name }))
        .collect();
    let mut meta = serde_json::Map::new();
    meta.insert("v".into(), json!(1));
    meta.insert("source".into(), json!(req.kind.as_str()));
    meta.insert("from".into(), Value::Array(from));
    if let Some((file, mode)) = &req.file {
        meta.insert("file".into(), json!({"name": file.name, "type": file.ty}));
        meta.insert("mode".into(), json!(mode.as_str()));
        if *mode == AiMode::Questions {
            meta.insert("skipped".into(), json!(req.skipped));
        }
    }
    meta.insert("author".into(), json!(username));
    meta.insert("authorId".into(), json!(req.user));
    meta.insert("made".into(), json!(req.questions.len()));
    meta.insert("goal".into(), json!(req.goal));
    meta.insert("at".into(), json!(req.at));
    Ok(serde_json::to_string(&Value::Object(meta))?)
}

/// One stored source question as the generation reads it.
struct Src {
    id: String,
    multi: bool,
    opts: usize,
    right: usize,
    stem: Tri,
}

/// A stored question as a generation source: six-column single / multi only.
fn parse_source(id: String, data: &str) -> Option<(Src, Question)> {
    let q = serde_json::from_str::<Question>(data).ok()?;
    if !q.is_six_column() {
        return None;
    }
    let multi = match q.kind {
        Kind::Single => false,
        Kind::Multi => true,
        Kind::Hotspot => return None,
    };
    let right = match &q.answer {
        Answer::Letters(l) => l.len(),
        Answer::Indexes(_) => return None,
    };
    let zh = q.zh.as_ref().map(|z| z.stem.clone()).unwrap_or_default();
    let ja = q.ja.as_ref().map(|z| z.stem.clone()).unwrap_or_default();
    let src = Src {
        id,
        multi,
        opts: q.options.len(),
        right,
        stem: Tri {
            en: cut(&q.stem, SOURCE_STEM_CHARS),
            zh: cut(&zh, SOURCE_STEM_CHARS),
            ja: cut(&ja, SOURCE_STEM_CHARS),
        },
    };
    Some((src, q))
}

// ---------------------------------------------------------------- SQL helpers

/// Why the caller may not generate (`None` = may), with their username and role.
struct Who {
    username: String,
    role: String,
    allowed: bool,
}

fn account(c: &Connection, user: i64) -> Result<Option<Who>, DbError> {
    Ok(c.query_row(
        "SELECT username, role, ai_enabled, can_manage_banks, disabled FROM users WHERE id=?1",
        params![user],
        |r| {
            let role: String = r.get(1)?;
            let ai = r.get::<_, i64>(2)? != 0;
            let manage = r.get::<_, i64>(3)? != 0;
            let disabled = r.get::<_, i64>(4)? != 0;
            Ok(Who {
                username: r.get(0)?,
                allowed: !disabled && (role == "admin" || (ai && manage)),
                role,
            })
        },
    )
    .optional()?)
}

fn bank_name(c: &Connection, id: i64) -> Result<Option<String>, DbError> {
    Ok(
        c.query_row("SELECT name FROM banks WHERE id=?1", params![id], |r| {
            r.get(0)
        })
        .optional()?,
    )
}

/// The sheets of `bank` that have active questions, in order of their first `seq`.
fn bank_sheets(c: &Connection, bank: i64) -> Result<Vec<String>, DbError> {
    let mut st = c.prepare(
        "SELECT sheet FROM questions WHERE bank_id=?1 AND active=1 GROUP BY sheet ORDER BY MIN(seq)",
    )?;
    let rows = st.query_map(params![bank], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn sheet_exists(c: &Connection, bank: i64, sheet: &str) -> Result<bool, DbError> {
    Ok(c.query_row(
        "SELECT 1 FROM questions WHERE bank_id=?1 AND sheet=?2 LIMIT 1",
        params![bank, sheet],
        |_| Ok(()),
    )
    .optional()?
    .is_some())
}

/// The active questions of one sheet, in `seq` order, as `(id, seq, data)`; `wrong_of`: only those in that user's
/// wrong-answer book (`wrong > 0` and a streak below [`MASTER_STREAK`], the rule of `/api/progress`).
fn sheet_rows(
    c: &Connection,
    bank: i64,
    sheet: &str,
    wrong_of: Option<i64>,
) -> Result<Vec<(String, i64, String)>, DbError> {
    let map = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?));
    match wrong_of {
        None => {
            let mut st = c.prepare(
                "SELECT id, seq, data FROM questions WHERE bank_id=?1 AND sheet=?2 AND active=1 ORDER BY seq",
            )?;
            let rows = st.query_map(params![bank, sheet], map)?;
            Ok(rows.collect::<Result<_, _>>()?)
        }
        Some(user) => {
            let mut st = c.prepare(
                "SELECT id, seq, data FROM questions WHERE bank_id=?1 AND sheet=?2 AND active=1 \
                 AND id IN (SELECT question_id FROM user_stats WHERE user_id=?3 AND wrong > 0 AND streak < ?4) \
                 ORDER BY seq",
            )?;
            let rows = st.query_map(params![bank, sheet, user, MASTER_STREAK], map)?;
            Ok(rows.collect::<Result<_, _>>()?)
        }
    }
}

/// The difficulty labels of `ids` from ONE statement over `user_stats` (all users summed), never one query per item.
fn difficulties(c: &Connection, ids: &[&str]) -> Result<HashMap<String, &'static str>, DbError> {
    let mut out = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let mut st = c.prepare(
        "SELECT question_id, SUM(attempts), SUM(wrong) FROM user_stats \
         WHERE question_id IN (SELECT value FROM json_each(?1)) GROUP BY question_id",
    )?;
    let mut rows = st.query(params![serde_json::to_string(ids)?])?;
    while let Some(r) = rows.next()? {
        let id: String = r.get(0)?;
        let attempts: i64 = r.get(1)?;
        let wrong: i64 = r.get(2)?;
        if let Some(d) = difficulty(attempts, wrong) {
            out.insert(id, d);
        }
    }
    Ok(out)
}

/// `previous`: the `(en, zh)` stems of the questions of the AI banks `viewer` can see whose source banks share one with
/// `sources`, newest banks first, at most [`PREVIOUS_MAX`].
fn previous_stems(
    c: &Connection,
    viewer: &Viewer,
    sources: &BTreeSet<i64>,
) -> Result<Vec<(String, String)>, DbError> {
    let metas: Vec<(i64, String)> = {
        let mut st = c.prepare(
            "SELECT id, ai_meta FROM banks WHERE ai_job IS NOT NULL AND ai_meta IS NOT NULL ORDER BY id DESC",
        )?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut out = Vec::new();
    for (bank, meta) in metas {
        if out.len() >= PREVIOUS_MAX {
            break;
        }
        if !viewer.scope.allows(Some(bank)) {
            continue;
        }
        let Ok(m) = serde_json::from_str::<Value>(&meta) else {
            continue;
        };
        let shares = m.get("from").and_then(Value::as_array).is_some_and(|a| {
            a.iter()
                .filter_map(|b| b.get("id").and_then(Value::as_i64))
                .any(|id| sources.contains(&id))
        });
        if !shares {
            continue;
        }
        let mut st = c.prepare(
            "SELECT data FROM questions WHERE bank_id=?1 AND active=1 ORDER BY seq LIMIT ?2",
        )?;
        let limit = i64::try_from(PREVIOUS_MAX - out.len()).unwrap_or(i64::MAX);
        let rows = st.query_map(params![bank, limit], |r| r.get::<_, String>(0))?;
        for data in rows {
            let Ok(q) = serde_json::from_str::<Question>(&data?) else {
                continue;
            };
            let zh = q.zh.as_ref().map(|z| z.stem.as_str()).unwrap_or("");
            out.push((cut(&q.stem, SOURCE_STEM_CHARS), cut(zh, SOURCE_STEM_CHARS)));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- the store methods

impl Store {
    /// `POST /internal/ai/gen/source` for `user` (the route has bound the session to this id): see the module documentation.
    /// Never writes.
    pub fn gen_source(&self, user: i64, req: &GenSourceReq) -> Result<GenSource, DbError> {
        // Step 1, under one lock: rights, the visible (bank, sheet) pairs that count, the bank names
        let (viewer, who, targets, bank_names) = {
            let g = self.lock();
            let Some(who) = account(&g, user)? else {
                return Ok(GenSource::NotAllowed);
            };
            if !who.allowed {
                return Ok(GenSource::NotAllowed);
            }
            let viewer = viewer_of(&g, user)?;
            let mut targets: Vec<(i64, String)> = Vec::new();
            match req.kind {
                GenKind::Bank => {
                    let Some(bank) = req.bank else {
                        return Ok(GenSource::NotFound);
                    };
                    let visible = viewer.scope.allows(Some(bank));
                    let exists = bank_name(&g, bank)?.is_some();
                    if !exists || !visible || !viewer.manages(bank_owner(&g, bank)?) {
                        return Ok(GenSource::NotFound);
                    }
                    if is_ai_bank(&g, bank)? {
                        return Ok(GenSource::AiSource);
                    }
                    for sheet in bank_sheets(&g, bank)? {
                        targets.push((bank, sheet));
                    }
                }
                // The route never lets a file reach the source step
                GenKind::File => return Ok(GenSource::NotFound),
                GenKind::Scope | GenKind::Wrong => {
                    let mut seen: HashSet<(i64, &str)> = HashSet::new();
                    let mut ai: HashMap<i64, bool> = HashMap::new();
                    let mut has_ai = false;
                    for (bank, sheet) in &req.sheets {
                        if !seen.insert((*bank, sheet.as_str())) {
                            continue;
                        }
                        if !viewer.scope.allows(Some(*bank)) || !sheet_exists(&g, *bank, sheet)? {
                            return Ok(GenSource::NotFound);
                        }
                        let is_ai = match ai.get(bank) {
                            Some(v) => *v,
                            None => {
                                let v = is_ai_bank(&g, *bank)?;
                                ai.insert(*bank, v);
                                v
                            }
                        };
                        // A scope holding any AI bank is refused (A-OWNER-52b), after every sheet was checked visible
                        has_ai |= is_ai;
                        targets.push((*bank, sheet.clone()));
                    }
                    if has_ai {
                        return Ok(GenSource::AiSource);
                    }
                }
            }
            let mut names = BTreeMap::new();
            for (bank, _) in &targets {
                if let std::collections::btree_map::Entry::Vacant(e) = names.entry(*bank) {
                    if let Some(n) = bank_name(&g, *bank)? {
                        e.insert(n);
                    }
                }
            }
            (viewer, who, targets, names)
        };
        // Step 2: one short read per sheet, the lock taken per sheet
        let wrong_of = (req.kind == GenKind::Wrong).then_some(user);
        // (bank, first seq, sheet, questions)
        let mut read: Vec<(i64, i64, String, Vec<Src>)> = Vec::new();
        for (bank, sheet) in &targets {
            let rows = {
                let g = self.lock();
                sheet_rows(&g, *bank, sheet, wrong_of)?
            };
            let mut first = i64::MAX;
            let mut srcs = Vec::new();
            for (id, seq, data) in rows {
                if let Some((src, _)) = parse_source(id, &data) {
                    first = first.min(seq);
                    srcs.push(src);
                }
            }
            // A sheet gone between the steps, or without a usable question, is skipped
            if !srcs.is_empty() {
                read.push((*bank, first, sheet.clone(), srcs));
            }
        }
        read.sort_by_key(|(bank, first, _, _)| (*bank, *first));
        let total: usize = read.iter().map(|(_, _, _, s)| s.len()).sum();
        // `tooFew` at once: before the sampling and before the difficulty statement
        if total < req.min {
            return Ok(GenSource::TooFew { have: total });
        }
        let strata: Vec<GenStratum> = read
            .iter()
            .map(|(bank, _, sheet, s)| GenStratum {
                bank: *bank,
                sheet: sheet.clone(),
                n: s.len(),
                multi: s.iter().filter(|x| x.multi).count(),
            })
            .collect();
        let ns: Vec<usize> = strata.iter().map(|s| s.n).collect();
        let quota = hamilton(&ns, req.max);
        let mut items: Vec<GenItem> = Vec::new();
        for (si, ((_, _, _, srcs), q)) in read.iter().zip(&quota).enumerate() {
            for pos in spread(srcs.len(), *q) {
                if let Some(s) = srcs.get(pos) {
                    items.push(GenItem {
                        id: s.id.clone(),
                        stratum: si,
                        multi: s.multi,
                        opts: s.opts,
                        right: s.right,
                        diff: None,
                        stem: s.stem.clone(),
                    });
                }
            }
        }
        // The difficulty: ONE statement over the returned ids
        let labels = {
            let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
            let g = self.lock();
            difficulties(&g, &ids)?
        };
        for item in &mut items {
            item.diff = labels.get(&item.id).copied();
        }
        let source_banks: BTreeSet<i64> = strata.iter().map(|s| s.bank).collect();
        let previous = {
            let g = self.lock();
            previous_stems(&g, &viewer, &source_banks)?
        };
        let banks: Vec<(i64, String)> = source_banks
            .iter()
            .filter_map(|b| bank_names_get(&bank_names, *b))
            .collect();
        Ok(GenSource::Found(Box::new(GenFound {
            user_id: user,
            username: who.username,
            role: who.role,
            total,
            banks,
            strata,
            items,
            previous,
        })))
    }

    /// `POST /internal/ai/gen/seeds` for `user`: the full texts (capped per language) of the questions `ids` the caller may
    /// see, in request order; an id that is not active, not single / multi, in an AI bank or not visible is left out. Never
    /// writes.
    pub fn gen_seeds(&self, user: i64, ids: &[String]) -> Result<GenSeeds, DbError> {
        let g = self.lock();
        let Some(w) = account(&g, user)? else {
            return Ok(GenSeeds::NotAllowed);
        };
        if !w.allowed {
            return Ok(GenSeeds::NotAllowed);
        }
        let viewer = viewer_of(&g, user)?;
        let mut ai: HashMap<i64, bool> = HashMap::new();
        let mut kept: Vec<(String, String, Question)> = Vec::new();
        for id in ids {
            let row: Option<(String, String, i64, Option<i64>)> = g
                .query_row(
                    "SELECT sheet, data, active, bank_id FROM questions WHERE id=?1",
                    params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            let Some((sheet, data, active, bank)) = row else {
                continue;
            };
            if active == 0 || !viewer.scope.allows(bank) {
                continue;
            }
            let Some(bank) = bank else { continue };
            let is_ai = match ai.get(&bank) {
                Some(v) => *v,
                None => {
                    let v = is_ai_bank(&g, bank)?;
                    ai.insert(bank, v);
                    v
                }
            };
            if is_ai {
                continue;
            }
            if let Some((_, q)) = parse_source(id.clone(), &data) {
                kept.push((id.clone(), sheet, q));
            }
        }
        let labels = {
            let ids: Vec<&str> = kept.iter().map(|(id, _, _)| id.as_str()).collect();
            difficulties(&g, &ids)?
        };
        let items = kept
            .iter()
            .map(|(id, sheet, q)| seed_json(id, sheet, q, labels.get(id).copied()))
            .collect();
        Ok(GenSeeds::Items(items))
    }

    /// `POST /internal/ai/gen/create`: makes the AI bank of `req` (see [`Store::create_ai_bank_shared`]) with
    /// [`AI_BANKS_AUTO_SHARE`].
    pub fn create_ai_bank(&self, req: &AiBankReq) -> Result<AiBankOutcome, DbError> {
        self.create_ai_bank_shared(req, AI_BANKS_AUTO_SHARE)
    }

    /// [`Store::create_ai_bank`] with the sharing switch as a parameter (the tests run it both ways). ONE transaction:
    /// (1) a bank with this job key exists: answer it, nothing written; (2) the user (id AND stamp, enabled) may generate;
    /// (3) the name is made unique (`name`, `name (2)`, ...); (4) the bank is inserted and shared (`share`: the "include
    /// later" users who can see every source bank, or nobody; the known-bank mark is written either way); then its owner,
    /// job key and facts; (5) the questions, `seq` 0 ..., `row` 1 ..., one sheet, no `imports` row.
    pub fn create_ai_bank_shared(
        &self,
        req: &AiBankReq,
        auto_share: bool,
    ) -> Result<AiBankOutcome, DbError> {
        if req.questions.is_empty() || req.options_invalid() {
            return Err(super::invalid_input("an AI bank needs valid questions"));
        }
        let now = self.clock.now();
        self.tx(|c| {
            let existing: Option<(i64, String)> = c
                .query_row(
                    "SELECT id, name FROM banks WHERE ai_job=?1",
                    params![req.job],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((bank_id, name)) = existing {
                return Ok(AiBankOutcome::Existing { bank_id, name });
            }
            // The account is re-checked here: the token route trusts the AI service's user / stamp only this far
            let found: Option<(String, String, i64, i64, i64)> = c
                .query_row(
                    "SELECT username, role, ai_enabled, can_manage_banks, disabled FROM users WHERE id=?1 AND uid=?2",
                    params![req.user, req.stamp],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()?;
            let Some((username, role, ai, manage, disabled)) = found else {
                return Ok(AiBankOutcome::NoUser);
            };
            if disabled != 0 {
                return Ok(AiBankOutcome::NoUser);
            }
            if !(role == "admin" || (ai != 0 && manage != 0)) {
                return Ok(AiBankOutcome::NotAllowed);
            }
            let name = unique_name(c, req.name.trim())?;
            let share = if auto_share && req.kind == GenKind::File {
                // A file has no source banks to guard: shared like an Excel import (A-OWNER-13d, API-I3a)
                NewBankShare::All
            } else if auto_share {
                NewBankShare::Visible(&req.from)
            } else {
                NewBankShare::Nobody
            };
            let bank_id = insert_bank(c, &name, "", false, &now, &now, share)?;
            let names: Vec<(i64, Option<String>)> = req
                .from
                .iter()
                .map(|id| Ok((*id, bank_name(c, *id)?)))
                .collect::<Result<_, DbError>>()?;
            c.execute(
                "UPDATE banks SET owner_id=?1, ai_job=?2, ai_meta=?3 WHERE id=?4",
                params![req.user, req.job, ai_meta_json(req, &username, &names)?, bank_id],
            )?;
            {
                let mut ins = c.prepare(
                    "INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id) \
                     VALUES(?1,?2,?3,?4,?5,1,?6,?6,?7)",
                )?;
                for (i, q) in req.questions.iter().enumerate() {
                    let mut stored = build_question(q);
                    stored.sheet.clone_from(&req.sheet);
                    stored.row = i64::try_from(i + 1).unwrap_or(i64::MAX);
                    let id = request_question_id(bank_id, req, i + 1, q);
                    let data = question_data_json(&stored)?;
                    ins.execute(params![
                        id,
                        req.sheet,
                        stored.row,
                        i64::try_from(i).unwrap_or(i64::MAX),
                        data,
                        now,
                        bank_id
                    ])?;
                }
            }
            Ok(AiBankOutcome::Created {
                bank_id,
                name,
                username,
            })
        })
    }

    /// `POST /internal/ai/import/check` (API-I5): the account `user` (a valid session already bound to this id) as
    /// `(username, role)` when it may make AI banks (an administrator, or AI+ on and may manage banks), else `None`.
    pub fn ai_bank_maker(&self, user: i64) -> Result<Option<(String, String)>, DbError> {
        let g = self.lock();
        Ok(account(&g, user)?
            .filter(|w| w.allowed)
            .map(|w| (w.username, w.role)))
    }

    /// Whether a bank with exactly this name exists (the confirm step of an AI file import refuses a clash). Read-only.
    pub fn bank_name_taken(&self, name: &str) -> Result<bool, DbError> {
        let g = self.lock();
        Ok(g.query_row(
            "SELECT EXISTS(SELECT 1 FROM banks WHERE name=?1)",
            params![name],
            |r| r.get::<_, i64>(0),
        )? != 0)
    }

    /// `POST /internal/ai/gen/lookup`: the AI bank of job key `job`, if any (`made` / `goal` from its facts). Never writes.
    pub fn ai_bank_of_job(&self, job: &str) -> Result<Option<AiBankInfo>, DbError> {
        let g = self.lock();
        let row: Option<(i64, String, Option<String>)> = g
            .query_row(
                "SELECT id, name, ai_meta FROM banks WHERE ai_job=?1",
                params![job],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(row.map(|(id, name, meta)| {
            let m: Value = meta
                .as_deref()
                .and_then(|t| serde_json::from_str(t).ok())
                .unwrap_or(Value::Null);
            AiBankInfo {
                id,
                name,
                made: m.get("made").and_then(Value::as_i64).unwrap_or(0),
                goal: m.get("goal").and_then(Value::as_i64).unwrap_or(0),
            }
        }))
    }
}

impl AiBankReq {
    /// Structural problems the store itself will not write: fewer than two options, an answer letter outside the options, an
    /// empty answer. (The route validates far more; this keeps a direct caller from storing a question that cannot be shown.)
    fn options_invalid(&self) -> bool {
        self.questions.iter().any(|q| {
            q.options.len() < 2
                || q.options.len() > 26
                || q.answer.is_empty()
                || q.answer
                    .iter()
                    .any(|a| !(0..q.options.len()).any(|i| letter(i) == *a))
        })
    }
}

fn bank_names_get(names: &BTreeMap<i64, String>, id: i64) -> Option<(i64, String)> {
    names.get(&id).map(|n| (id, n.clone()))
}

/// One seed of `gen_seeds`: the question with every text cut to the prompt caps, per language.
fn seed_json(id: &str, sheet: &str, q: &Question, diff: Option<&'static str>) -> Value {
    let zh_stem = q.zh.as_ref().map(|z| z.stem.as_str()).unwrap_or("");
    let ja_stem = q.ja.as_ref().map(|z| z.stem.as_str()).unwrap_or("");
    let opt_lang = |z: &Option<Zh>, i: usize| -> String {
        z.as_ref()
            .and_then(|z| z.options.as_ref())
            .and_then(|o| o.get(i))
            .cloned()
            .unwrap_or_default()
    };
    let options: Vec<Value> = q
        .options
        .iter()
        .enumerate()
        .map(|(i, o)| {
            json!({
                "key": o.key,
                "en": cut(&o.text, SEED_OPTION_CHARS),
                "zh": cut(&opt_lang(&q.zh, i), SEED_OPTION_CHARS),
                "ja": cut(&opt_lang(&q.ja, i), SEED_OPTION_CHARS),
            })
        })
        .collect();
    let expl = |pick: fn(&Explain) -> &Option<ExplainText>| -> String {
        q.explain
            .as_ref()
            .and_then(|e| pick(e).as_ref())
            .map(|t| cut(&t.explanation, SEED_EXPL_CHARS))
            .unwrap_or_default()
    };
    let answer: Vec<String> = match &q.answer {
        Answer::Letters(l) => l.clone(),
        Answer::Indexes(_) => Vec::new(),
    };
    json!({
        "id": id,
        "kind": if q.kind == Kind::Multi { "multi" } else { "single" },
        "sheet": sheet,
        "diff": diff,
        "stem": {
            "en": cut(&q.stem, SEED_STEM_CHARS),
            "zh": cut(zh_stem, SEED_STEM_CHARS),
            "ja": cut(ja_stem, SEED_STEM_CHARS),
        },
        "options": options,
        "answer": answer,
        "explanation": {
            "en": expl(|e| &e.en),
            "zh": expl(|e| &e.zh),
            "ja": expl(|e| &e.ja),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri(s: &str) -> Tri {
        Tri {
            en: s.to_string(),
            zh: format!("{s}-zh"),
            ja: format!("{s}-ja"),
        }
    }

    fn aq(stem: &str, n: usize, answer: &[&str], multi: bool) -> AiQuestion {
        AiQuestion {
            multi,
            stem: tri(stem),
            options: (0..n).map(|i| tri(&format!("opt {i}"))).collect(),
            answer: answer.iter().map(|s| (*s).to_string()).collect(),
            explanation: tri("why"),
        }
    }

    #[test]
    fn hamilton_is_proportional_capped_and_keeps_every_stratum() {
        // proportional, the remainder to the largest fraction
        assert_eq!(hamilton(&[30, 27], 20), vec![11, 9]);
        assert_eq!(hamilton(&[50, 30, 20], 10), vec![5, 3, 2]);
        // everything fits: all of it
        assert_eq!(hamilton(&[3, 4], 10), vec![3, 4]);
        assert_eq!(hamilton(&[3, 4], 7), vec![3, 4]);
        // at least one per stratum while max allows, taken from the largest quota
        assert_eq!(hamilton(&[1000, 1, 1], 10), vec![8, 1, 1]);
        assert_eq!(hamilton(&[1000, 1, 1], 3), vec![1, 1, 1]);
        // fewer places than strata: the largest strata, the earlier on a tie
        assert_eq!(hamilton(&[5, 9, 9, 1], 2), vec![0, 1, 1, 0]);
        // empty strata and empty input
        assert_eq!(hamilton(&[], 5), Vec::<usize>::new());
        assert_eq!(hamilton(&[0, 0], 5), vec![0, 0]);
        assert_eq!(hamilton(&[0, 10], 4), vec![0, 4]);
        // the sum is min(max, total), never above a stratum
        for ns in [vec![7, 13, 2, 40], vec![1, 1, 1, 1], vec![100], vec![2, 98]] {
            for max in 1..=60 {
                let q = hamilton(&ns, max);
                let total: usize = ns.iter().sum();
                assert_eq!(q.iter().sum::<usize>(), max.min(total), "{ns:?} {max}");
                assert!(q.iter().zip(&ns).all(|(a, n)| a <= n), "{ns:?} {max} {q:?}");
                if max >= ns.len() {
                    assert!(q.iter().all(|a| *a >= 1), "{ns:?} {max} {q:?}");
                }
            }
        }
    }

    #[test]
    fn spread_is_even_and_deterministic() {
        assert_eq!(spread(5, 5), vec![0, 1, 2, 3, 4]);
        assert_eq!(spread(5, 9), vec![0, 1, 2, 3, 4]);
        assert_eq!(spread(10, 5), vec![1, 3, 5, 7, 9]);
        assert_eq!(spread(10, 1), vec![5]);
        assert_eq!(spread(0, 3), Vec::<usize>::new());
        for (n, q) in [(30, 7), (57, 20), (100, 3)] {
            let p = spread(n, q);
            assert_eq!(p.len(), q);
            assert!(p.windows(2).all(|w| w[0] < w[1]), "{p:?}");
            assert!(p.iter().all(|i| *i < n));
        }
    }

    #[test]
    fn difficulty_labels() {
        assert_eq!(difficulty(0, 0), None);
        assert_eq!(difficulty(4, 4), None, "fewer than 5 attempts");
        assert_eq!(difficulty(5, 0), Some("easy"));
        assert_eq!(difficulty(10, 1), Some("easy"), "10 % wrong");
        assert_eq!(difficulty(10, 2), Some("mid"), "exactly 20 % is not easy");
        assert_eq!(difficulty(10, 5), Some("mid"), "exactly 50 % is still mid");
        assert_eq!(difficulty(10, 6), Some("hard"));
        assert_eq!(difficulty(5, 5), Some("hard"));
    }

    #[test]
    fn cut_keeps_the_cap_and_ends_with_an_ellipsis() {
        assert_eq!(cut("abc", 3), "abc");
        assert_eq!(cut("abcd", 3), "ab…");
        assert_eq!(cut("令和令和令", 4), "令和令…");
        assert_eq!(cut("", 5), "");
        assert_eq!(cut("abc", 0), "");
    }

    #[test]
    fn a_generated_question_is_stored_like_a_six_column_one() {
        let q = aq("Which tool slices bread?", 3, &["B"], false);
        let s = build_question(&q);
        assert_eq!(s.kind, Kind::Single);
        assert_eq!(s.lang, "en");
        assert!(s.is_six_column());
        assert_eq!(s.stem, "Which tool slices bread?");
        assert_eq!(
            s.options
                .iter()
                .map(|o| (o.key.as_str(), o.text.as_str()))
                .collect::<Vec<_>>(),
            vec![("A", "opt 0"), ("B", "opt 1"), ("C", "opt 2")]
        );
        assert_eq!(s.answer, Answer::Letters(vec!["B".to_string()]));
        assert_eq!(
            s.translation,
            "Which tool slices bread?-zh\nA. opt 0-zh\nB. opt 1-zh\nC. opt 2-zh"
        );
        let zh = s.zh.as_ref().expect("zh");
        assert_eq!(zh.stem, "Which tool slices bread?-zh");
        assert_eq!(zh.options.as_deref().map(<[String]>::len), Some(3));
        let ja = s.ja.as_ref().expect("ja");
        assert_eq!(ja.options.as_ref().map(|o| o[2].as_str()), Some("opt 2-ja"));
        // the Chinese explanation is the compatibility text; the three languages are in `explain`
        assert_eq!(s.explanation, "why-zh");
        let e = s.explain.as_ref().expect("explain");
        assert_eq!(e.en.as_ref().map(|t| t.explanation.as_str()), Some("why"));
        assert_eq!(
            e.ja.as_ref().map(|t| t.explanation.as_str()),
            Some("why-ja")
        );
        assert!(
            s.tag.is_empty() && s.mnemonic.is_empty() && s.note.is_empty() && s.english.is_empty()
        );
        assert!(s.items.is_empty() && s.en.is_none());
        // a multiple-choice question keeps its sorted letters
        let m = build_question(&aq("Pick two.", 4, &["A", "C"], true));
        assert_eq!(m.kind, Kind::Multi);
        assert_eq!(
            m.answer,
            Answer::Letters(vec!["A".to_string(), "C".to_string()])
        );
    }

    #[test]
    fn question_ids_follow_the_parser_rule_then_the_bank_scope() {
        let q = aq("Same stem", 3, &["A"], false);
        let a = ai_question_id(7, "AI", &q);
        assert_eq!(
            a,
            super::super::banks::scoped_id(7, &make_id("AI", "Same stem", "opt 0"))
        );
        assert_eq!(a.len(), 12);
        assert_ne!(a, ai_question_id(8, "AI", &q));
        let other = aq("Same stem", 3, &["A"], false);
        assert_eq!(a, ai_question_id(7, "AI", &other));
    }

    #[test]
    fn the_request_structure_check_refuses_what_cannot_be_shown() {
        let base = |q: AiQuestion| AiBankReq {
            job: "0".repeat(32),
            user: 1,
            stamp: "0".repeat(32),
            name: "n".into(),
            kind: GenKind::Bank,
            from: vec![1],
            goal: 1,
            at: "2026-11-12 09:30:00".into(),
            sheet: "AI".into(),
            questions: vec![q],
            file: None,
            skipped: Vec::new(),
        };
        assert!(!base(aq("s", 3, &["B"], false)).options_invalid());
        assert!(base(aq("s", 1, &["A"], false)).options_invalid());
        assert!(base(aq("s", 3, &["D"], false)).options_invalid());
        assert!(base(aq("s", 3, &[], false)).options_invalid());
    }

    #[test]
    fn the_ai_summary_hides_banks_the_viewer_cannot_see() {
        use crate::models::BankScope;
        let meta = r#"{"v":1,"source":"scope","from":[{"id":3,"name":"Kitchen"},{"id":4,"name":"Secret"},{"id":5,"name":null}],"author":"alexandra_k","authorId":12,"made":20,"goal":20,"at":"2026-11-12 09:30:00"}"#;
        let admin = Viewer::admin(1);
        let v = super::super::banks::ai_summary(meta, &admin);
        assert_eq!(
            v["from"],
            json!(["Kitchen", "Secret"]),
            "nulls dropped, admins see all"
        );
        assert_eq!(v["author"], json!("alexandra_k"));
        assert!(v.get("authorId").is_none());
        let user = Viewer {
            user: 2,
            admin: false,
            can_manage: false,
            scope: BankScope::Only([3].into_iter().collect()),
        };
        let v = super::super::banks::ai_summary(meta, &user);
        assert_eq!(v["from"], json!(["Kitchen"]));
        assert_eq!(v["source"], json!("scope"));
        assert_eq!(
            (v["made"].as_i64(), v["goal"].as_i64()),
            (Some(20), Some(20))
        );
        // unreadable facts give safe defaults
        let v = super::super::banks::ai_summary("not json", &admin);
        assert_eq!(
            v,
            json!({"source": "bank", "from": [], "author": null, "made": 0, "goal": 0, "at": ""})
        );
    }
}
