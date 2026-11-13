//! v1.1: the four token routes the AI service calls to generate a question bank, `POST /internal/ai/gen/source`,
//! `/gen/seeds`, `/gen/create` and `/gen/lookup` (the reads and the one write themselves are in
//! `database/store/ai_gen.rs`).
//!
//! [`super::internal::dispatch`] has already checked the internal token, the method, the content type and the body size (at
//! most 64 KiB, `MAX_GEN_BODY` = 6 MiB for the create route) and parsed the body as a JSON object; this module validates the
//! keys and values, binds the call to the caller and answers.
//!
//! - Validation: exactly the listed keys (an unknown or missing key, a wrong type or an out-of-range value is 400
//!   `MSG_BAD_REQUEST`). The create route validates with the generator's own limits (stem ≤ 800, option ≤ 300, explanation
//!   ≤ 1 200 characters per language; 3 … 6 options; `answer` sorted letters among the options; exactly `goal` questions, no
//!   partial banks), so a payload the AI service's checks accepted is never refused here.
//! - Session binding (source, seeds): `store.session_user(sid, now, false)` like `internal_study.rs` (no refresh): no valid
//!   session or a pending password change answers `{"ok": false, "reason": "session"}`. The rights (administrator, or AI+ on
//!   and may manage banks) are checked by the store: `{"ok": false, "reason": "notAllowed"}`.
//! - The create route has no session (the job may outlive it): the AI service names the user and the identity stamp, the
//!   backend re-checks both and the account's rights itself (`user` / `notAllowed`), and a user suspended in the last
//!   heartbeat is `notAllowed` too. Token-guarded like the heartbeat; a browser can never reach `/internal/*`.
//! - Every refusal is HTTP 200 (the AI client reads an HTTP 404 from `/internal/*` as "token refused").
//! - source / seeds / lookup write nothing and log nothing; create logs one INFO line (English, username, no question text).
use super::internal_study::{bad_request, check_keys, is_lower_hex, positive, refused};
use super::{blocking, AppState};
use crate::database::store::{
    AiBankOutcome, AiBankReq, AiFile, AiMode, AiQuestion, GenKind, GenSeeds, GenSource,
    GenSourceReq, Tri,
};
use crate::error::AppError;
use crate::parser::make_id;
use serde_json::{json, Map, Value};
use std::collections::HashSet;

/// Most (bank, sheet) pairs of one source request, and the longest sheet name (characters; API-U2).
const MAX_SHEETS: usize = 200;
const MAX_SHEET_CHARS: usize = 80;
/// `min` / `max` of a source request.
const MAX_ITEMS: i64 = 2000;
/// Most ids of one seeds request.
const MAX_SEED_IDS: usize = 100;
/// Longest bank name and sheet name of the create route (characters).
const MAX_NAME_CHARS: usize = 120;
const MAX_AI_SHEET_CHARS: usize = 60;
/// The most questions of one bank and source banks of one create request.
const MAX_GOAL: i64 = 100;
const MAX_FROM: usize = 200;
/// The generator's text limits (characters per language; GP-4).
const MAX_STEM_CHARS: usize = 800;
const MAX_OPTION_CHARS: usize = 300;
const MAX_EXPL_CHARS: usize = 1200;
const MIN_OPTIONS: usize = 3;
const MAX_OPTIONS: usize = 6;
/// The limits of a file import in questions mode (API-I3a): the questions are converted as they are, so wider than the
/// generator's.
const IMPORT_STEM_CHARS: usize = 2000;
const IMPORT_OPTION_CHARS: usize = 400;
const IMPORT_EXPL_CHARS: usize = 1500;
const IMPORT_MIN_OPTIONS: usize = 2;
const IMPORT_MAX_OPTIONS: usize = 8;
/// The file types an import accepts (`file.type`), and the longest file name (characters).
const FILE_TYPES: [&str; 7] = ["pdf", "docx", "xlsx", "pptx", "md", "txt", "csv"];
const MAX_FILE_NAME_CHARS: usize = 200;

/// The text and option limits a question is validated with.
#[derive(Clone, Copy)]
struct Limits {
    stem: usize,
    option: usize,
    expl: usize,
    min_options: usize,
    max_options: usize,
    /// A multi-answer question may have every option right (converted questions only)
    multi_all: bool,
}

/// GP-4: what the generator writes (bank sources and file material).
const GENERATED: Limits = Limits {
    stem: MAX_STEM_CHARS,
    option: MAX_OPTION_CHARS,
    expl: MAX_EXPL_CHARS,
    min_options: MIN_OPTIONS,
    max_options: MAX_OPTIONS,
    multi_all: false,
};
/// API-I3a: questions converted from a file.
const CONVERTED: Limits = Limits {
    stem: IMPORT_STEM_CHARS,
    option: IMPORT_OPTION_CHARS,
    expl: IMPORT_EXPL_CHARS,
    min_options: IMPORT_MIN_OPTIONS,
    max_options: IMPORT_MAX_OPTIONS,
    multi_all: true,
};

/// Invisible format characters a name must not hold (GP-9): bidi marks, overrides and isolates, zero-width space, word
/// joiner, BOM. U+200C / U+200D stay (emoji sequences and some scripts need them).
pub fn is_hidden_format(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' | '\u{200B}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}'
    )
}

/// Whether `s` has a control (Unicode Cc) or invisible format character.
fn has_control_or_format(s: &str) -> bool {
    s.chars().any(|c| c.is_control() || is_hidden_format(c))
}

/// `^[0-9A-Za-z_-]{1,64}$`
fn is_question_id(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn sid(data: &Map<String, Value>) -> Result<String, AppError> {
    match data.get("sid") {
        Some(Value::String(s)) if is_lower_hex(s, 64) => Ok(s.clone()),
        _ => Err(bad_request()),
    }
}

fn int_in(data: &Map<String, Value>, key: &str, lo: i64, hi: i64) -> Result<i64, AppError> {
    data.get(key)
        .and_then(Value::as_i64)
        .filter(|i| (lo..=hi).contains(i))
        .ok_or_else(bad_request)
}

/// The user of the session `sid` (no refresh), or the refusal to send as it is.
async fn session_user(st: &AppState, sid: String) -> Result<Result<i64, Value>, AppError> {
    let store = st.0.store.clone();
    let now = crate::auth::unix_now();
    let found = blocking(move || store.session_user(&sid, now, false)).await?;
    Ok(match found {
        Some((u, _)) if !u.must_change_password => Ok(u.id),
        _ => Err(refused("session")),
    })
}

// ---------------------------------------------------------------- source

fn source_request(data: &Map<String, Value>) -> Result<GenSourceReq, AppError> {
    let kind = data
        .get("source")
        .and_then(Value::as_str)
        .and_then(GenKind::parse)
        // a file is a create-route source only
        .filter(|k| *k != GenKind::File)
        .ok_or_else(bad_request)?;
    let min = int_in(data, "min", 1, MAX_ITEMS)?;
    let max = int_in(data, "max", 1, MAX_ITEMS)?;
    let (min, max) = (
        usize::try_from(min).map_err(|_| bad_request())?,
        usize::try_from(max).map_err(|_| bad_request())?,
    );
    match kind {
        GenKind::File => Err(bad_request()),
        GenKind::Bank => {
            check_keys(data, &["sid", "source", "bank", "min", "max"], &[])?;
            Ok(GenSourceReq {
                kind,
                bank: Some(positive(data, "bank")?),
                sheets: Vec::new(),
                min,
                max,
            })
        }
        GenKind::Scope | GenKind::Wrong => {
            check_keys(data, &["sid", "source", "sheets", "min", "max"], &[])?;
            let Some(Value::Array(items)) = data.get("sheets") else {
                return Err(bad_request());
            };
            if items.is_empty() || items.len() > MAX_SHEETS {
                return Err(bad_request());
            }
            let mut sheets = Vec::with_capacity(items.len());
            for item in items {
                let Value::Object(m) = item else {
                    return Err(bad_request());
                };
                check_keys(m, &["bank", "sheet"], &[])?;
                let sheet = match m.get("sheet") {
                    Some(Value::String(s))
                        if (1..=MAX_SHEET_CHARS).contains(&s.chars().count()) =>
                    {
                        s.clone()
                    }
                    _ => return Err(bad_request()),
                };
                sheets.push((positive(m, "bank")?, sheet));
            }
            Ok(GenSourceReq {
                kind,
                bank: None,
                sheets,
                min,
                max,
            })
        }
    }
}

/// `POST /internal/ai/gen/source` (API-I1).
pub(super) async fn source(st: &AppState, data: &Map<String, Value>) -> Result<Value, AppError> {
    let sid = sid(data)?;
    let req = source_request(data)?;
    let user = match session_user(st, sid).await? {
        Ok(user) => user,
        Err(answer) => return Ok(answer),
    };
    let store = st.0.store.clone();
    let found: GenSource = blocking(move || store.gen_source(user, &req)).await?;
    Ok(found.to_json())
}

// ---------------------------------------------------------------- seeds

/// `POST /internal/ai/gen/seeds` (API-I2).
pub(super) async fn seeds(st: &AppState, data: &Map<String, Value>) -> Result<Value, AppError> {
    check_keys(data, &["sid", "ids"], &[])?;
    let sid = sid(data)?;
    let Some(Value::Array(raw)) = data.get("ids") else {
        return Err(bad_request());
    };
    if raw.is_empty() || raw.len() > MAX_SEED_IDS {
        return Err(bad_request());
    }
    let mut ids = Vec::with_capacity(raw.len());
    for v in raw {
        match v {
            Value::String(s) if is_question_id(s) => ids.push(s.clone()),
            _ => return Err(bad_request()),
        }
    }
    let user = match session_user(st, sid).await? {
        Ok(user) => user,
        Err(answer) => return Ok(answer),
    };
    let store = st.0.store.clone();
    let found: GenSeeds = blocking(move || store.gen_seeds(user, &ids)).await?;
    Ok(found.to_json())
}

// ---------------------------------------------------------------- create

/// A non-empty (after trimming) string of at most `max` characters.
fn text(v: Option<&Value>, max: usize) -> Result<String, AppError> {
    match v {
        Some(Value::String(s)) if !s.trim().is_empty() && s.chars().count() <= max => Ok(s.clone()),
        _ => Err(bad_request()),
    }
}

/// `{"en": "…", "zh": "…", "ja": "…"}`: exactly these keys, each text valid.
fn tri(v: Option<&Value>, max: usize) -> Result<Tri, AppError> {
    let Some(Value::Object(m)) = v else {
        return Err(bad_request());
    };
    check_keys(m, &["en", "zh", "ja"], &[])?;
    Ok(Tri {
        en: text(m.get("en"), max)?,
        zh: text(m.get("zh"), max)?,
        ja: text(m.get("ja"), max)?,
    })
}

/// `n` letters of the options as the answer expects them: `A`, `B`, …
fn option_letter(i: usize) -> String {
    char::from(b'A' + u8::try_from(i).unwrap_or(0)).to_string()
}

fn question(v: &Value, lim: Limits) -> Result<AiQuestion, AppError> {
    let Value::Object(m) = v else {
        return Err(bad_request());
    };
    check_keys(
        m,
        &["kind", "stem", "options", "answer", "explanation"],
        &[],
    )?;
    let multi = match m.get("kind").and_then(Value::as_str) {
        Some("single") => false,
        Some("multi") => true,
        _ => return Err(bad_request()),
    };
    let stem = tri(m.get("stem"), lim.stem)?;
    let explanation = tri(m.get("explanation"), lim.expl)?;
    let Some(Value::Array(raw_options)) = m.get("options") else {
        return Err(bad_request());
    };
    if !(lim.min_options..=lim.max_options).contains(&raw_options.len()) {
        return Err(bad_request());
    }
    let options = raw_options
        .iter()
        .map(|o| tri(Some(o), lim.option))
        .collect::<Result<Vec<Tri>, AppError>>()?;
    // The answer: distinct letters among the option keys (A … in order), sorted; single exactly one, multi 2 … options - 1
    let Some(Value::Array(raw_answer)) = m.get("answer") else {
        return Err(bad_request());
    };
    let valid: Vec<String> = (0..options.len()).map(option_letter).collect();
    let mut answer: Vec<String> = Vec::with_capacity(raw_answer.len());
    for a in raw_answer {
        match a {
            Value::String(s) if valid.contains(s) => answer.push(s.clone()),
            _ => return Err(bad_request()),
        }
    }
    let mut sorted = answer.clone();
    sorted.sort();
    sorted.dedup();
    if sorted != answer {
        return Err(bad_request());
    }
    let count_ok = if multi {
        let top = if lim.multi_all {
            options.len() + 1
        } else {
            options.len()
        };
        (2..top).contains(&answer.len())
    } else {
        answer.len() == 1
    };
    if !count_ok {
        return Err(bad_request());
    }
    Ok(AiQuestion {
        multi,
        stem,
        options,
        answer,
        explanation,
    })
}

/// `YYYY-MM-DD HH:MM:SS` (digits and plausible ranges; no calendar check).
fn is_timestamp(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 19 {
        return false;
    }
    let digit = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    let pair = |i: usize| -> u32 {
        s.get(i..i + 2)
            .and_then(|t| t.parse::<u32>().ok())
            .unwrap_or(99)
    };
    [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18]
        .iter()
        .all(|i| digit(*i))
        && b.get(4) == Some(&b'-')
        && b.get(7) == Some(&b'-')
        && b.get(10) == Some(&b' ')
        && b.get(13) == Some(&b':')
        && b.get(16) == Some(&b':')
        && (1..=12).contains(&pair(5))
        && (1..=31).contains(&pair(8))
        && pair(11) <= 23
        && pair(14) <= 59
        && pair(17) <= 59
}

/// `file` of a file-import create request: `{"name": "...", "type": "pdf"}` (name trimmed, 1 ... 200 characters, no control or
/// hidden format characters; type one of [`FILE_TYPES`]).
fn file_facts(v: Option<&Value>) -> Result<AiFile, AppError> {
    let Some(Value::Object(m)) = v else {
        return Err(bad_request());
    };
    check_keys(m, &["name", "type"], &[])?;
    let name = match m.get("name") {
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() || t.chars().count() > MAX_FILE_NAME_CHARS || has_control_or_format(t) {
                return Err(bad_request());
            }
            t.to_string()
        }
        _ => return Err(bad_request()),
    };
    let ty = match m.get("type") {
        Some(Value::String(s)) if FILE_TYPES.contains(&s.as_str()) => s.clone(),
        _ => return Err(bad_request()),
    };
    Ok(AiFile { name, ty })
}

/// `skipped` of a questions-mode import: distinct integers 1 ... `goal`, sorted.
fn skipped_numbers(v: Option<&Value>, goal: usize) -> Result<Vec<i64>, AppError> {
    let Some(Value::Array(a)) = v else {
        return Err(bad_request());
    };
    let top = i64::try_from(goal).map_err(|_| bad_request())?;
    let mut out: Vec<i64> = Vec::with_capacity(a.len());
    for n in a {
        match n.as_i64() {
            Some(i) if (1..=top).contains(&i) && out.last().is_none_or(|last| *last < i) => {
                out.push(i);
            }
            _ => return Err(bad_request()),
        }
    }
    Ok(out)
}

/// The validated create request (API-I3, and API-I3a for `source: "file"`).
fn create_request(data: &Map<String, Value>) -> Result<AiBankReq, AppError> {
    let kind = data
        .get("source")
        .and_then(Value::as_str)
        .and_then(GenKind::parse)
        .ok_or_else(bad_request)?;
    // The key set depends on the source: `file` and `mode` exactly for a file, `skipped` exactly for its questions mode
    let mode = match (kind, data.get("mode").and_then(Value::as_str)) {
        (GenKind::File, Some("questions")) => Some(AiMode::Questions),
        (GenKind::File, Some("material")) => Some(AiMode::Material),
        (GenKind::File, _) => return Err(bad_request()),
        _ => None,
    };
    let mut keys = vec![
        "job",
        "user",
        "stamp",
        "name",
        "source",
        "from",
        "goal",
        "at",
        "sheet",
        "questions",
    ];
    if let Some(m) = mode {
        keys.extend(["file", "mode"]);
        if m == AiMode::Questions {
            keys.push("skipped");
        }
    }
    check_keys(data, &keys, &[])?;
    let hex32 = |key: &str| -> Result<String, AppError> {
        match data.get(key) {
            Some(Value::String(s)) if is_lower_hex(s, 32) => Ok(s.clone()),
            _ => Err(bad_request()),
        }
    };
    let job = hex32("job")?;
    let stamp = hex32("stamp")?;
    let user = positive(data, "user")?;
    let name = match data.get("name") {
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() || t.chars().count() > MAX_NAME_CHARS || has_control_or_format(t) {
                return Err(bad_request());
            }
            t.to_string()
        }
        _ => return Err(bad_request()),
    };
    let from: Vec<i64> = match data.get("from") {
        Some(Value::Array(a)) if a.len() <= MAX_FROM => a
            .iter()
            .map(|v| v.as_i64().filter(|i| *i > 0).ok_or_else(bad_request))
            .collect::<Result<_, _>>()?,
        _ => return Err(bad_request()),
    };
    if from.iter().collect::<HashSet<_>>().len() != from.len() {
        return Err(bad_request());
    }
    // A file has no source banks
    if kind == GenKind::File && !from.is_empty() {
        return Err(bad_request());
    }
    let goal = usize::try_from(int_in(data, "goal", 1, MAX_GOAL)?).map_err(|_| bad_request())?;
    let at = match data.get("at") {
        Some(Value::String(s)) if is_timestamp(s) => s.clone(),
        _ => return Err(bad_request()),
    };
    let sheet = match data.get("sheet") {
        Some(Value::String(s))
            if (1..=MAX_AI_SHEET_CHARS).contains(&s.chars().count())
                && !has_control_or_format(s) =>
        {
            s.clone()
        }
        _ => return Err(bad_request()),
    };
    let file = match mode {
        Some(m) => Some((file_facts(data.get("file"))?, m)),
        None => None,
    };
    let skipped = if mode == Some(AiMode::Questions) {
        skipped_numbers(data.get("skipped"), goal)?
    } else {
        Vec::new()
    };
    let Some(Value::Array(raw)) = data.get("questions") else {
        return Err(bad_request());
    };
    // Generated banks and file material: exactly `goal` questions, there are no partial banks. Converted questions: the ones
    // that could be converted plus the skipped ones are `goal` (D10), and at least one was converted.
    let count_ok = if mode == Some(AiMode::Questions) {
        !raw.is_empty() && raw.len() + skipped.len() == goal
    } else {
        raw.len() == goal
    };
    if !count_ok {
        return Err(bad_request());
    }
    let lim = if mode == Some(AiMode::Questions) {
        CONVERTED
    } else {
        GENERATED
    };
    let questions = raw
        .iter()
        .map(|q| question(q, lim))
        .collect::<Result<Vec<AiQuestion>, AppError>>()?;
    // The question ids (the parser's id rule over the id seed, the stem and the first option) must be distinct
    let mut ids = HashSet::with_capacity(questions.len());
    for (i, q) in questions.iter().enumerate() {
        let first = q.options.first().map(|o| o.en.as_str()).unwrap_or("");
        let seed = crate::database::store::id_seed(kind, &sheet, i + 1);
        if !ids.insert(make_id(&seed, &q.stem.en, first)) {
            return Err(bad_request());
        }
    }
    Ok(AiBankReq {
        job,
        user,
        stamp,
        name,
        kind,
        from,
        goal,
        at,
        sheet,
        questions,
        file,
        skipped,
    })
}

/// `POST /internal/ai/gen/create` (API-I3).
pub(super) async fn create(st: &AppState, data: &Map<String, Value>) -> Result<Value, AppError> {
    let req = create_request(data)?;
    // A user whose AI+ function is suspended in AI settings (the last heartbeat) may not generate; checked before anything is
    // written, but after the job-key lookup (API-I3 step 1): a retry of a job whose bank already exists still gets that bank
    if st.0.ai_link.suspended(req.user) {
        let store = st.0.store.clone();
        let job = req.job.clone();
        return Ok(match blocking(move || store.ai_bank_of_job(&job)).await? {
            Some(b) => json!({"ok": true, "bankId": b.id, "name": b.name, "created": false}),
            None => refused("notAllowed"),
        });
    }
    let store = st.0.store.clone();
    let questions = req.questions.len();
    let from_file = req.kind == GenKind::File;
    let outcome = blocking(move || store.create_ai_bank(&req)).await?;
    Ok(match outcome {
        AiBankOutcome::Created {
            bank_id,
            name,
            username,
        } => {
            if from_file {
                // SEC-10: the bank name holds the file stem, so a file import logs the id only
                tracing::info!(
                    r#"AI bank #{bank_id} created for user "{username}" ({questions} questions, file import)"#
                );
            } else {
                tracing::info!(
                    r#"AI bank "{name}" created for user "{username}" ({questions} questions)"#
                );
            }
            json!({"ok": true, "bankId": bank_id, "name": name, "created": true})
        }
        AiBankOutcome::Existing { bank_id, name } => {
            json!({"ok": true, "bankId": bank_id, "name": name, "created": false})
        }
        AiBankOutcome::NoUser => refused("user"),
        AiBankOutcome::NotAllowed => refused("notAllowed"),
    })
}

// ---------------------------------------------------------------- import check

/// `POST /internal/ai/import/check` (API-I5): may the signed-in user import a file as an AI bank? `{"sid"}` (and
/// optionally `"name"`, the bank name typed at the confirm step) → `{"ok": true, "user": {"id", "username", "role"}}`
/// (plus `"nameTaken": bool` when a name was sent), or `{"ok": false, "reason": "session" | "notAllowed"}`. Read-only.
pub(super) async fn import_check(
    st: &AppState,
    data: &Map<String, Value>,
) -> Result<Value, AppError> {
    check_keys(data, &["sid"], &["name"])?;
    let sid = sid(data)?;
    let name = match data.get("name") {
        None => None,
        Some(Value::String(s)) => Some(s.trim().to_string()),
        Some(_) => return Err(bad_request()),
    };
    let user = match session_user(st, sid).await? {
        Ok(u) => u,
        Err(refusal) => return Ok(refusal),
    };
    let store = st.0.store.clone();
    Ok(
        match blocking(move || {
            let maker = store.ai_bank_maker(user)?;
            let taken = match (&maker, &name) {
                (Some(_), Some(n)) => Some(store.bank_name_taken(n)?),
                _ => None,
            };
            Ok((user, maker, taken))
        })
        .await?
        {
            (id, Some((username, role)), taken) => {
                let mut v =
                    json!({"ok": true, "user": {"id": id, "username": username, "role": role}});
                if let (Some(t), Some(o)) = (taken, v.as_object_mut()) {
                    o.insert("nameTaken".into(), Value::Bool(t));
                }
                v
            }
            (_, None, _) => refused("notAllowed"),
        },
    )
}

// ---------------------------------------------------------------- lookup

/// `POST /internal/ai/gen/lookup` (API-I4, P2): the bank of a job key.
pub(super) async fn lookup(st: &AppState, data: &Map<String, Value>) -> Result<Value, AppError> {
    check_keys(data, &["job"], &[])?;
    let job = match data.get("job") {
        Some(Value::String(s)) if is_lower_hex(s, 32) => s.clone(),
        _ => return Err(bad_request()),
    };
    let store = st.0.store.clone();
    let found = blocking(move || store.ai_bank_of_job(&job)).await?;
    Ok(match found {
        Some(b) => {
            json!({"ok": true, "bank": {"id": b.id, "name": b.name, "made": b.made, "goal": b.goal}})
        }
        None => json!({"ok": true, "bank": null}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().expect("object")
    }

    fn t(s: &str) -> Value {
        json!({"en": s, "zh": format!("{s} zh"), "ja": format!("{s} ja")})
    }

    fn q(stem: &str, answer: Value, kind: &str, options: usize) -> Value {
        json!({
            "kind": kind,
            "stem": t(stem),
            "options": (0..options).map(|i| t(&format!("{stem} option {i}"))).collect::<Vec<_>>(),
            "answer": answer,
            "explanation": t("because"),
        })
    }

    fn body(questions: Vec<Value>) -> Value {
        json!({
            "job": "0123456789abcdef0123456789abcdef",
            "user": 12,
            "stamp": "fedcba9876543210fedcba9876543210",
            "name": "Kitchen basics_alexandr_20261112093000",
            "source": "bank",
            "from": [3],
            "goal": questions.len(),
            "at": "2026-11-12 09:30:00",
            "sheet": "AI",
            "questions": questions,
        })
    }

    #[test]
    fn hidden_format_characters_are_the_gp9_list() {
        for c in [
            '\u{061C}', '\u{200B}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202E}', '\u{2060}',
            '\u{2069}', '\u{FEFF}',
        ] {
            assert!(is_hidden_format(c), "{c:?}");
        }
        for c in ['a', '令', '\u{200C}', '\u{200D}', '\u{2070}', ' '] {
            assert!(!is_hidden_format(c), "{c:?}");
        }
        assert!(has_control_or_format("a\u{0007}b"));
        assert!(has_control_or_format("a\nb"));
        assert!(has_control_or_format("evil\u{202E}name"));
        assert!(!has_control_or_format("😀\u{200D}😀 ok"));
    }

    #[test]
    fn a_valid_create_request_is_accepted() {
        let b = body(vec![
            q("One", json!(["B"]), "single", 4),
            q("Two", json!(["A", "C"]), "multi", 4),
        ]);
        let r = create_request(&obj(b)).expect("valid");
        assert_eq!((r.goal, r.questions.len(), r.from.clone()), (2, 2, vec![3]));
        assert!(r.questions[1].multi);
        assert_eq!(r.questions[1].answer, vec!["A", "C"]);
        // no source banks is fine; the name is trimmed
        let mut m = obj(body(vec![q("One", json!(["A"]), "single", 3)]));
        m.insert("from".into(), json!([]));
        m.insert("name".into(), json!("  padded  "));
        let r = create_request(&m).expect("valid");
        assert_eq!((r.from.len(), r.name.as_str()), (0, "padded"));
    }

    #[test]
    fn the_create_request_is_strict() {
        let good = || obj(body(vec![q("One", json!(["B"]), "single", 4)]));
        assert!(create_request(&good()).is_ok());
        let with = |k: &str, v: Value| {
            let mut m = good();
            m.insert(k.to_string(), v);
            create_request(&m).is_err()
        };
        let without = |k: &str| {
            let mut m = good();
            m.remove(k);
            create_request(&m).is_err()
        };
        for k in [
            "job",
            "user",
            "stamp",
            "name",
            "source",
            "from",
            "goal",
            "at",
            "sheet",
            "questions",
        ] {
            assert!(without(k), "missing {k}");
        }
        assert!(with("extra", json!(1)));
        assert!(with("job", json!("ABCDEF0123456789abcdef0123456789")));
        assert!(with("job", json!("abc")));
        assert!(with("stamp", json!("g".repeat(32))));
        assert!(with("user", json!(0)));
        assert!(with("user", json!("12")));
        assert!(with("name", json!("")));
        assert!(with("name", json!("   ")));
        assert!(with("name", json!("x".repeat(121))));
        assert!(!with("name", json!("x".repeat(120))));
        assert!(with("name", json!("evil\u{202E}name")), "bidi override");
        assert!(with("name", json!("zero\u{200B}width")), "zero-width space");
        assert!(with("name", json!("bell\u{0007}")));
        assert!(with("source", json!("all")));
        assert!(with("from", json!([3, 3])));
        assert!(with("from", json!([0])));
        assert!(with("from", json!(["3"])));
        assert!(with("from", json!((1..=201).collect::<Vec<i64>>())));
        assert!(!with("from", json!((1..=200).collect::<Vec<i64>>())));
        assert!(with("goal", json!(0)));
        assert!(with("goal", json!(101)));
        assert!(
            with("goal", json!(2)),
            "goal 2 with one question: no partial banks"
        );
        assert!(with("at", json!("2026-11-12T09:30:00")));
        assert!(with("at", json!("2026-13-12 09:30:00")));
        assert!(with("at", json!("2026-11-12 24:30:00")));
        assert!(with("at", json!("2026-11-12 09:30")));
        assert!(with("sheet", json!("")));
        assert!(with("sheet", json!("s".repeat(61))));
        assert!(with("sheet", json!("a\u{202E}b")));
        assert!(with("questions", json!([])));
        assert!(with("questions", json!("x")));
    }

    #[test]
    fn a_question_follows_the_generators_limits() {
        let ok = |v: Value| question(&v, GENERATED).is_ok();
        assert!(ok(q("S", json!(["A"]), "single", 3)));
        assert!(ok(q("S", json!(["F"]), "single", 6)));
        assert!(ok(q("S", json!(["A", "B"]), "multi", 3)), "multi 2 of 3");
        assert!(ok(q("S", json!(["A", "E"]), "multi", 6)));
        // option counts 3 ..= 6
        assert!(!ok(q("S", json!(["A"]), "single", 2)));
        assert!(!ok(q("S", json!(["A"]), "single", 7)));
        // answer: letters among the options, sorted, distinct, the right number
        assert!(
            !ok(q("S", json!(["D"]), "single", 3)),
            "letter beyond the options"
        );
        assert!(!ok(q("S", json!(["a"]), "single", 3)), "lower case");
        assert!(
            !ok(q("S", json!(["A", "B"]), "single", 3)),
            "single exactly one"
        );
        assert!(!ok(q("S", json!([]), "single", 3)));
        assert!(
            !ok(q("S", json!(["A"]), "multi", 4)),
            "multi needs 2 or more"
        );
        assert!(
            !ok(q("S", json!(["A", "B", "C"]), "multi", 3)),
            "multi at most options - 1"
        );
        assert!(!ok(q("S", json!(["C", "A"]), "multi", 4)), "sorted");
        assert!(!ok(q("S", json!(["A", "A"]), "multi", 4)), "distinct");
        assert!(!ok(q("S", json!([1]), "single", 3)));
        assert!(!ok(q("S", json!("A"), "single", 3)));
        assert!(!ok(q("S", json!(["A"]), "hotspot", 3)));
        // texts: three languages, non-empty, within the limits (characters, not bytes)
        let mut v = q("S", json!(["A"]), "single", 3);
        v["stem"]["ja"] = json!("");
        assert!(!ok(v));
        let mut v = q("S", json!(["A"]), "single", 3);
        v["stem"]["zh"] = json!("令".repeat(800));
        assert!(ok(v.clone()), "800 characters, 2 400 bytes");
        v["stem"]["zh"] = json!("令".repeat(801));
        assert!(!ok(v), "801 characters");
        let mut v = q("S", json!(["A"]), "single", 3);
        v["explanation"]["en"] = json!("e".repeat(1200));
        assert!(ok(v.clone()));
        v["explanation"]["en"] = json!("e".repeat(1201));
        assert!(!ok(v));
        let mut v = q("S", json!(["A"]), "single", 3);
        v["options"][1]["ja"] = json!("o".repeat(300));
        assert!(ok(v.clone()));
        v["options"][1]["ja"] = json!("o".repeat(301));
        assert!(!ok(v));
        let mut v = q("S", json!(["A"]), "single", 3);
        v["stem"].as_object_mut().expect("o").remove("zh");
        assert!(!ok(v), "a missing language");
        let mut v = q("S", json!(["A"]), "single", 3);
        v["extra"] = json!(1);
        assert!(!ok(v));
    }

    #[test]
    fn the_question_ids_must_be_distinct() {
        let a = q("Same stem", json!(["A"]), "single", 3);
        let b = q("Same stem", json!(["B"]), "single", 3);
        let r = create_request(&obj(body(vec![a.clone(), b])));
        assert!(r.is_err(), "same stem and first option");
        let c = q("Other stem", json!(["B"]), "single", 3);
        assert!(create_request(&obj(body(vec![a, c]))).is_ok());
    }

    #[test]
    fn source_requests_have_exactly_their_keys() {
        let sid = "a".repeat(64);
        let ok = |v: Value| source_request(&obj(v)).is_ok();
        assert!(ok(
            json!({"sid": sid, "source": "bank", "bank": 3, "min": 30, "max": 2000})
        ));
        assert!(ok(
            json!({"sid": sid, "source": "scope", "sheets": [{"bank": 3, "sheet": "Ovens"}], "min": 1, "max": 1})
        ));
        assert!(ok(
            json!({"sid": sid, "source": "wrong", "sheets": [{"bank": 3, "sheet": "Ovens"}], "min": 10, "max": 2000})
        ));
        // the key set depends on the source
        assert!(!ok(
            json!({"sid": sid, "source": "bank", "sheets": [{"bank": 3, "sheet": "x"}], "min": 30, "max": 2000})
        ));
        assert!(!ok(
            json!({"sid": sid, "source": "scope", "bank": 3, "min": 30, "max": 2000})
        ));
        assert!(!ok(
            json!({"sid": sid, "source": "bank", "bank": 3, "min": 30, "max": 2000, "x": 1})
        ));
        assert!(!ok(
            json!({"sid": sid, "source": "bank", "bank": 3, "min": 30})
        ));
        // ranges
        assert!(!ok(
            json!({"sid": sid, "source": "bank", "bank": 0, "min": 30, "max": 2000})
        ));
        assert!(!ok(
            json!({"sid": sid, "source": "bank", "bank": 3, "min": 0, "max": 2000})
        ));
        assert!(!ok(
            json!({"sid": sid, "source": "bank", "bank": 3, "min": 30, "max": 2001})
        ));
        assert!(!ok(
            json!({"sid": sid, "source": "bank", "bank": 3, "min": "30", "max": 2000})
        ));
        // sheets: 1 ..= 200 entries of exactly {bank, sheet}, a sheet of 1 ..= 80 characters
        let many = |n: usize| {
            json!({"sid": sid, "source": "scope", "min": 1, "max": 10,
            "sheets": (0..n).map(|i| json!({"bank": 1, "sheet": format!("s{i}")})).collect::<Vec<_>>()})
        };
        assert!(ok(many(200)));
        assert!(!ok(many(201)));
        assert!(!ok(many(0)));
        for bad in [
            json!({"bank": 1, "sheet": ""}),
            json!({"bank": 1, "sheet": "s".repeat(81)}),
            json!({"bank": 0, "sheet": "s"}),
            json!({"bank": 1}),
            json!({"bank": 1, "sheet": "s", "x": 1}),
            json!("s"),
        ] {
            assert!(!ok(
                json!({"sid": sid, "source": "wrong", "sheets": [bad], "min": 1, "max": 10})
            ));
        }
        assert!(ok(
            json!({"sid": sid, "source": "scope", "sheets": [{"bank": 1, "sheet": "令".repeat(80)}], "min": 1, "max": 10})
        ));
    }

    #[test]
    fn timestamps_are_checked() {
        for ok in [
            "2026-11-12 09:30:00",
            "1999-01-01 00:00:00",
            "2026-12-31 23:59:59",
        ] {
            assert!(is_timestamp(ok), "{ok}");
        }
        for bad in [
            "",
            "2026-11-12 9:30:00",
            "2026-00-12 09:30:00",
            "2026-11-32 09:30:00",
            "2026-11-12 09:60:00",
            "2026-11-12 09:30:60",
            "2026-11-12  09:30:0",
            "２０２６-11-12 09:30:00",
        ] {
            assert!(!is_timestamp(bad), "{bad}");
        }
    }

    #[test]
    fn seed_ids_are_url_safe() {
        for ok in ["a", "1f2e3d4c5b6a", "A_b-9", &"x".repeat(64)] {
            assert!(is_question_id(ok), "{ok}");
        }
        for bad in ["", &"x".repeat(65), "a b", "a/b", "é", "a.b"] {
            assert!(!is_question_id(bad), "{bad:?}");
        }
    }
}
