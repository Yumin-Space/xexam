//! `Store`: same semantics as `Store` in `app/db.py`.
//!
//! - One connection lives inside a `std::sync::Mutex` (the old `RLock`); a poisoned lock is recovered with `into_inner`,
//!   and acquiring the lock also rolls back "the uncommitted transaction left behind after the previous call panicked".
//! - Every write is `BEGIN IMMEDIATE … COMMIT`; any failure (including a COMMIT failure) triggers `ROLLBACK`.
//! - The preview plan of `apply_import` is recomputed **inside the same write transaction** (fixes T4 of the old version: another write could slip in between the plan and the actual write).
//! - Schema v2 (v0.5): question banks, per-question exam detail and settings. Every import is scoped to one bank (see `banks`);
//!   the legacy `plan_import` / `apply_import` mean "the legacy bank" (uploads without `X-Bank-Target`).
//!
//! - Schema v3 (v0.6): users and sessions; exam records, per-question stats (`user_stats`) and settings (`user_settings`) belong
//!   to one user, so every method that reads or writes them takes the signed-in user's id. Banks, questions and the import
//!   history stay shared.
//!
//! - Schema v4 (v0.6.5): question banks per user and bank owners. Every read of shared bank data has a `*_as` variant that takes
//!   the request's [`Viewer`] (or its [`BankScope`]): a plain user sees only the banks they may see, in `/api/bank`,
//!   `/api/banks`, the statistics of `/api/progress`, the questions an exam may count and the record detail; a bank manager
//!   imports, updates and deletes only the banks they own (checked inside the write transaction). The methods without `_as`
//!   keep their v0.6 meaning: everything, as an administrator sees it (the `import` command, the golden tests).
//!   Study records also have a kind (`attempts.kind`: `"exam"` or `"practice"`).
//!
//! - v0.7: only the six-column layout can be imported (the import entry points refuse 3- and 4-column workbooks before they
//!   reach the store), and [`Store::retire_old_layout_questions`] retires every active question of those layouts (run by the
//!   server and the command-line program on every start, after the migrations). The store methods themselves accept any
//!   question, so the golden suites can replay the reference implementation's 3-column imports.
//!
//! - Schema v5 (v0.8): two columns on `users`, the AI+ switch and the identity stamp the AI service keeps next to the user
//!   id (see `users`); AI chats, memories and settings live in the AI service's own database, never in this one.
//!
//! Submodules (they extend `impl Store`): `migrate` (v1 → v2 → v3 → v4 → v5, backups, fix-ups), `banks` (bank list, delete,
//! import targets, bank-scoped ids, viewers), `records` (exam records and their per-question detail), `settings`, `users`
//! (accounts, sessions, the users the AI service asks about).
use super::journal::{self, Checkpoint, JournalMode};
use super::legacy_progress::{self, AttemptRow, StatRow};
use super::pyjson;
use super::schema::SCHEMA;
use super::timefmt::{Clock, SystemClock};
use super::{HISTORY_LIMIT, MASTER_STREAK, RECORD_RETENTION};
use crate::error::DbError;
use crate::models::{
    Attempt, AttemptResult, BankScope, ImportRecord, ImportTarget, ParsedSheet, Question,
    SessionTag, SkippedRow, Viewer,
};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::de::Error as _;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

mod banks;
mod migrate;
mod records;
mod settings;
mod study;
mod users;

pub use banks::scoped_id;
pub use migrate::BACKUP_DIR;
pub use study::{QuestionAsk, StudyLang, MAX_SAMPLE, TEXT_BUDGET};
pub use users::{AiUserRow, SessionUser, UpdatedUser, UserChange, UserRow, UserUpdate};

/// One bank whose old-layout questions [`Store::retire_old_layout_questions`] retired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetiredBank {
    /// The bank id (`None`: questions without a bank, which an open normally attaches to the legacy bank first)
    pub id: Option<i64>,
    /// The bank's name ("" for questions without a bank)
    pub name: String,
    /// How many questions were retired
    pub retired: i64,
}

/// SQL condition (over `questions`) that is true for a question that is NOT of the six-column layout: its JSON `data` has no
/// `"lang": "en"` (3- and 4-column questions never have `lang`; data that is not valid JSON is not six-column either).
const OLD_LAYOUT_SQL: &str =
    "(CASE WHEN json_valid(data) THEN json_extract(data, '$.lang') END) IS NOT 'en'";

/// Maximum number of placeholders in one `IN (?,…)` (SQLite's default limit is 32766; this leaves plenty of headroom).
const IN_CHUNK: usize = 500;

pub struct Store {
    conn: Mutex<Connection>,
    clock: Arc<dyn Clock>,
    /// The copy written before migrating this database at open (None: no migration, or nothing to back up).
    migration_backup: Option<PathBuf>,
    /// This open created the default administrator `admin` / `admin`.
    created_default_admin: bool,
    /// This open converted a pre-release v4 database (`users.all_banks`) to the released shape (`users.new_banks`)
    converted_prerelease: bool,
    /// The journal mode SQLite reported after the open (lower case)
    journal_mode: String,
}

fn json_err(msg: &str) -> DbError {
    DbError::Json(serde_json::Error::custom(msg.to_string()))
}

fn invalid_input(msg: &str) -> DbError {
    DbError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
        msg.to_string().into(),
    ))
}

/// `round(100 * good / total)` with Python's "banker's rounding" (round half to even), using integer arithmetic only.
/// Returns 0 when `total <= 0` or `good < 0` (the old version raised ZeroDivisionError when `total == 0`; the API layer rejected that case first).
pub fn percent_round_half_even(good: i64, total: i64) -> i64 {
    if total <= 0 || good < 0 {
        return 0;
    }
    let num = i128::from(good) * 100;
    let tot = i128::from(total);
    let q = num / tot;
    let r = num % tot;
    let twice = r * 2;
    let up = twice > tot || (twice == tot && q % 2 == 1);
    i64::try_from(if up { q + 1 } else { q }).unwrap_or(i64::MAX)
}

/// A transaction that is rolled back on failure. The caller holds the lock on `conn`.
fn run_tx<T>(
    conn: &Connection,
    f: impl FnOnce(&Connection) -> Result<T, DbError>,
) -> Result<T, DbError> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    match f(conn) {
        Ok(v) => match conn.execute_batch("COMMIT") {
            Ok(()) => Ok(v),
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(DbError::Sqlite(e))
            }
        },
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

fn get_meta(conn: &Connection, key: &str) -> Result<Option<String>, DbError> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key=?1", params![key], |r| {
            r.get::<_, String>(0)
        })
        .optional()?)
}

/// Any study record of any user (the old `stats` table is no longer read: its rows were claimed into `user_stats`).
fn has_progress(conn: &Connection) -> Result<bool, DbError> {
    let n: i64 = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM user_stats) + (SELECT COUNT(*) FROM attempts) n",
        [],
        |r| r.get(0),
    )?;
    Ok(n != 0)
}

struct StoredQuestion {
    id: String,
    sheet: String,
    row_no: i64,
    data: String,
    active: bool,
    bank_id: Option<i64>,
}

/// Column list matching [`stored_question`].
const QUESTION_COLUMNS: &str = "id, sheet, row_no, data, active, bank_id";

fn stored_question(r: &rusqlite::Row<'_>) -> rusqlite::Result<StoredQuestion> {
    Ok(StoredQuestion {
        id: r.get(0)?,
        sheet: r.get(1)?,
        row_no: r.get(2)?,
        data: r.get(3)?,
        active: r.get::<_, i64>(4)? != 0,
        bank_id: r.get(5)?,
    })
}

/// `sql` must select [`QUESTION_COLUMNS`] in that order.
fn load_questions(
    conn: &Connection,
    sql: &str,
    p: impl rusqlite::Params,
) -> Result<Vec<StoredQuestion>, DbError> {
    let mut st = conn.prepare(sql)?;
    let rows = st.query_map(p, stored_question)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// A question as `/api/bank` shows it: the stored data, then `id`, `sheet`, `row`, `bank`.
fn question_object(r: &StoredQuestion) -> Result<Map<String, Value>, DbError> {
    let mut q = parse_data(&r.data)?;
    q.insert("id".to_string(), Value::String(r.id.clone()));
    q.insert("sheet".to_string(), Value::String(r.sheet.clone()));
    q.insert("row".to_string(), Value::from(r.row_no));
    q.insert("bank".to_string(), Value::from(r.bank_id));
    Ok(q)
}

fn parse_data(data: &str) -> Result<Map<String, Value>, DbError> {
    match serde_json::from_str::<Value>(data)? {
        Value::Object(m) => Ok(m),
        _ => Err(json_err("question data is not a JSON object")),
    }
}

fn drop_key(v: Value, key: &str) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.into_iter().filter(|(k, _)| k != key).collect()),
        other => other,
    }
}

/// The old `_q_hash`: JSON text with `row` removed, keys sorted, and `ensure_ascii=False`.
fn question_hash(q: &Question) -> Result<String, DbError> {
    let v = serde_json::to_value(q)?;
    Ok(pyjson::canonical(&drop_key(v, "row"))?)
}

fn stored_hash(r: &StoredQuestion) -> Result<String, DbError> {
    let mut obj = parse_data(&r.data)?;
    obj.insert("id".to_string(), Value::String(r.id.clone()));
    obj.insert("sheet".to_string(), Value::String(r.sheet.clone()));
    obj.insert("row".to_string(), Value::from(r.row_no));
    Ok(pyjson::canonical(&drop_key(Value::Object(obj), "row"))?)
}

/// The JSON stored in `questions.data`: all fields except id / sheet / row, in the declaration order of `Question`.
fn question_data_json(q: &Question) -> Result<String, DbError> {
    let Value::Object(m) = serde_json::to_value(q)? else {
        return Err(json_err("question did not serialise to an object"));
    };
    let data: Map<String, Value> = m
        .into_iter()
        .filter(|(k, _)| k != "id" && k != "sheet" && k != "row")
        .collect();
    Ok(pyjson::to_string(&Value::Object(data))?)
}

struct SheetAgg {
    name: String,
    total: i64,
    usable: i64,
    skipped: Vec<SkippedRow>,
    /// Rows imported with a warning (six-column layout); shown in the preview only when there are some
    warnings: Vec<SkippedRow>,
    added: i64,
    updated: i64,
    removed: i64,
    restored: i64,
}

impl SheetAgg {
    fn new(name: &str, total: i64, usable: i64, skipped: Vec<SkippedRow>) -> Self {
        SheetAgg {
            name: name.to_string(),
            total,
            usable,
            skipped,
            warnings: Vec::new(),
            added: 0,
            updated: 0,
            removed: 0,
            restored: 0,
        }
    }
    fn to_value(&self) -> Result<Value, DbError> {
        let mut v = json!({
            "name": self.name,
            "total": self.total,
            "usable": self.usable,
            "skipped": serde_json::to_value(&self.skipped)?,
            "added": self.added,
            "updated": self.updated,
            "removed": self.removed,
            "restored": self.restored,
        });
        // Only sheets with warnings get the key, so the plan of a 3 / 4-column file stays byte-identical
        if !self.warnings.is_empty() {
            if let Value::Object(m) = &mut v {
                m.insert(
                    "warnings".to_string(),
                    serde_json::to_value(&self.warnings)?,
                );
            }
        }
        Ok(v)
    }
}

struct Aggregator {
    aggs: Vec<SheetAgg>,
    pos: HashMap<String, usize>,
}

impl Aggregator {
    fn new(sheets: &[ParsedSheet]) -> Self {
        let mut a = Aggregator {
            aggs: Vec::new(),
            pos: HashMap::new(),
        };
        for s in sheets {
            let mut agg = SheetAgg::new(&s.name, s.total, s.usable, s.skipped.clone());
            agg.warnings.clone_from(&s.warnings);
            match a.pos.get(&s.name).copied() {
                Some(i) => {
                    if let Some(slot) = a.aggs.get_mut(i) {
                        *slot = agg;
                    }
                }
                None => {
                    a.pos.insert(s.name.clone(), a.aggs.len());
                    a.aggs.push(agg);
                }
            }
        }
        a
    }
    /// Gets (creating it if needed) the stats of a sheet; for "a sheet that has only retired questions" the old version created one with `setdefault`.
    fn entry(&mut self, name: &str) -> Option<&mut SheetAgg> {
        let i = match self.pos.get(name).copied() {
            Some(i) => i,
            None => {
                let i = self.aggs.len();
                self.aggs.push(SheetAgg::new(name, 0, 0, Vec::new()));
                self.pos.insert(name.to_string(), i);
                i
            }
        };
        self.aggs.get_mut(i)
    }
}

struct PlanOut {
    json: Value,
    added: i64,
    updated: i64,
    restored: i64,
    removed: i64,
    skipped: i64,
}

fn len_i64(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// How many of `ids` have study statistics (of any user: an import affects every user's notebook).
fn count_stats_for(conn: &Connection, ids: &[&str]) -> Result<i64, DbError> {
    let mut total = 0i64;
    for chunk in ids.chunks(IN_CHUNK) {
        let marks = vec!["?"; chunk.len()].join(",");
        let sql = format!(
            "SELECT COUNT(DISTINCT question_id) c FROM user_stats WHERE question_id IN ({marks})"
        );
        let n: i64 =
            conn.query_row(&sql, rusqlite::params_from_iter(chunk.iter()), |r| r.get(0))?;
        total += n;
    }
    Ok(total)
}

/// The old `plan_import`: read-only. Compares against the questions of bank `bank_id` only (`None`: a bank that does not exist yet,
/// so every question is `added`). `questions` must already carry the ids used in that bank (see `banks::scoped_questions`).
/// Rows are read in rowid order, the same order the old full-table scan produced.
fn compute_plan(
    conn: &Connection,
    bank_id: Option<i64>,
    sheets: &[ParsedSheet],
    questions: &[Question],
) -> Result<PlanOut, DbError> {
    let rows = match bank_id {
        Some(b) => load_questions(
            conn,
            &format!("SELECT {QUESTION_COLUMNS} FROM questions WHERE bank_id=?1 ORDER BY rowid"),
            params![b],
        )?,
        None => Vec::new(),
    };
    let index: HashMap<&str, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (r.id.as_str(), i))
        .collect();
    let new_ids: HashSet<&str> = questions.iter().map(|q| q.id.as_str()).collect();

    let (mut added, mut updated, mut restored): (Vec<&Question>, Vec<&Question>, Vec<&Question>) =
        (Vec::new(), Vec::new(), Vec::new());
    let mut unchanged = 0i64;
    for q in questions {
        match index.get(q.id.as_str()).and_then(|&i| rows.get(i)) {
            None => added.push(q),
            Some(r) if !r.active => restored.push(q),
            Some(r) => {
                if stored_hash(r)? != question_hash(q)? {
                    updated.push(q);
                } else {
                    unchanged += 1;
                }
            }
        }
    }
    let removed: Vec<&StoredQuestion> = rows
        .iter()
        .filter(|r| r.active && !new_ids.contains(r.id.as_str()))
        .collect();

    let mut agg = Aggregator::new(sheets);
    for q in &added {
        if let Some(a) = agg.entry(&q.sheet) {
            a.added += 1;
        }
    }
    for q in &updated {
        if let Some(a) = agg.entry(&q.sheet) {
            a.updated += 1;
        }
    }
    for q in &restored {
        if let Some(a) = agg.entry(&q.sheet) {
            a.restored += 1;
        }
    }
    for r in &removed {
        if let Some(a) = agg.entry(&r.sheet) {
            a.removed += 1;
        }
    }

    let mut samples = Vec::new();
    for r in removed.iter().take(5) {
        let stem = parse_data(&r.data)?
            .get("stem")
            .and_then(Value::as_str)
            .map(|s| s.chars().take(40).collect::<String>())
            .unwrap_or_default();
        samples.push(Value::String(stem));
    }
    let removed_ids: Vec<&str> = removed.iter().map(|r| r.id.as_str()).collect();
    let removed_stats = count_stats_for(conn, &removed_ids)?;

    let sheet_values = agg
        .aggs
        .iter()
        .map(SheetAgg::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    let skipped: i64 = sheets.iter().map(|s| len_i64(s.skipped.len())).sum();
    let warnings: i64 = sheets.iter().map(|s| len_i64(s.warnings.len())).sum();
    let out = PlanOut {
        json: Value::Null,
        added: len_i64(added.len()),
        updated: len_i64(updated.len()),
        restored: len_i64(restored.len()),
        removed: len_i64(removed.len()),
        skipped,
    };
    let json = json!({
        "sheets": sheet_values,
        "added": out.added,
        "updated": out.updated,
        "unchanged": unchanged,
        "restored": out.restored,
        "removed": out.removed,
        "skipped": out.skipped,
        "usable": len_i64(questions.len()),
        "removed_samples": samples,
        "removed_stats": removed_stats,
    });
    // Six-column files: how many imported rows have a warning (left out when there is none, as in older versions)
    let mut json = json;
    if warnings > 0 {
        if let Value::Object(m) = &mut json {
            m.insert("warnings".to_string(), Value::from(warnings));
        }
    }
    Ok(PlanOut { json, ..out })
}

/// The write part of an import into bank `bank_id` (inside the caller's transaction). `questions` carry the bank's ids.
fn write_import(
    c: &Connection,
    bank_id: i64,
    filename: &str,
    sheets: &[ParsedSheet],
    questions: &[Question],
    t: &str,
) -> Result<Value, DbError> {
    let plan = compute_plan(c, Some(bank_id), sheets, questions)?;
    let mut new_ids: HashSet<&str> = HashSet::with_capacity(questions.len());
    {
        let mut up = c.prepare(
            "INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id) VALUES(?1,?2,?3,?4,?5,1,?6,?7,?8) \
             ON CONFLICT(id) DO UPDATE SET sheet=excluded.sheet, row_no=excluded.row_no, seq=excluded.seq, \
             data=excluded.data, active=1, updated_at=excluded.updated_at, bank_id=excluded.bank_id",
        )?;
        for (seq, q) in questions.iter().enumerate() {
            new_ids.insert(q.id.as_str());
            let data = question_data_json(q)?;
            up.execute(params![
                q.id,
                q.sheet,
                q.row,
                len_i64(seq),
                data,
                t,
                t,
                bank_id
            ])?;
        }
    }
    // Questions of this bank missing from the uploaded file -> retired (study records are kept); other banks are untouched.
    // Rowid order = the order of the old full-table scan, which decides the retired questions' `seq`.
    let base = questions.len();
    let active_ids: Vec<String> = {
        let mut st =
            c.prepare("SELECT id FROM questions WHERE active=1 AND bank_id=?1 ORDER BY rowid")?;
        let rows = st.query_map(params![bank_id], |r| r.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let gone: Vec<&String> = active_ids
        .iter()
        .filter(|id| !new_ids.contains(id.as_str()))
        .collect();
    {
        let mut down =
            c.prepare("UPDATE questions SET active=0, seq=?1, updated_at=?2 WHERE id=?3")?;
        for (i, qid) in gone.iter().enumerate() {
            down.execute(params![len_i64(base + i), t, qid])?;
        }
    }
    c.execute(
        "INSERT INTO imports(at, filename, added, updated, removed, restored, skipped, total, bank_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![t, filename, plan.added, plan.updated, plan.removed, plan.restored, plan.skipped, len_i64(questions.len()), bank_id],
    )?;
    Ok(plan.json)
}

fn last_import(conn: &Connection, scope: &BankScope) -> Result<Option<ImportRecord>, DbError> {
    Ok(import_rows(conn, 1, scope)?.into_iter().next())
}

/// The newest `limit` import rows of the banks in `scope`, newest first.
fn import_rows(
    conn: &Connection,
    limit: i64,
    scope: &BankScope,
) -> Result<Vec<ImportRecord>, DbError> {
    // A limited scope reads every row (the history is short: one row per upload) and keeps the visible ones
    let sql_limit = if scope.is_all() { limit } else { -1 };
    let mut st = conn.prepare(
        "SELECT id, at, filename, added, updated, removed, restored, skipped, total, bank_id FROM imports ORDER BY id DESC LIMIT ?1",
    )?;
    let mut rows = st.query(params![sql_limit])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        if i64::try_from(out.len()).unwrap_or(i64::MAX) >= limit {
            break;
        }
        if !scope.allows(r.get::<_, Option<i64>>(9)?) {
            continue;
        }
        out.push(ImportRecord {
            id: r.get(0)?,
            at: r.get(1)?,
            filename: r.get(2)?,
            added: r.get(3)?,
            updated: r.get(4)?,
            removed: r.get(5)?,
            restored: r.get(6)?,
            skipped: r.get(7)?,
            total: r.get(8)?,
        });
    }
    Ok(out)
}

/// The ids of the questions (active or retired) of the banks in a limited scope; `None` for [`BankScope::All`].
fn visible_question_ids(
    conn: &Connection,
    scope: &BankScope,
) -> Result<Option<HashSet<String>>, DbError> {
    if scope.is_all() {
        return Ok(None);
    }
    let mut st = conn.prepare("SELECT id, bank_id FROM questions")?;
    let rows = st.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))
    })?;
    let mut out = HashSet::new();
    for row in rows {
        let (id, bank) = row?;
        if scope.allows(bank) {
            out.insert(id);
        }
    }
    Ok(Some(out))
}

/// `/api/progress` of `user`. Statistics are read in the order they were first written (rowid), like the old `stats` scan.
/// With a limited `scope`, `questions` holds only the statistics of questions of visible banks (the wrong-answer notebook and
/// the review follow it); `history` is unchanged, records stay visible as history. Every history entry ends with `kind`.
fn progress_from(conn: &Connection, user: i64, scope: &BankScope) -> Result<Value, DbError> {
    let visible = visible_question_ids(conn, scope)?;
    let mut questions = Map::new();
    {
        let mut st = conn.prepare(
            "SELECT question_id, attempts, correct, wrong, streak, last, at FROM user_stats WHERE user_id=?1 ORDER BY rowid",
        )?;
        let mut rows = st.query(params![user])?;
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            if visible.as_ref().is_some_and(|v| !v.contains(&id)) {
                continue;
            }
            let mut rec = Map::new();
            rec.insert("attempts".to_string(), Value::from(r.get::<_, i64>(1)?));
            rec.insert("correct".to_string(), Value::from(r.get::<_, i64>(2)?));
            rec.insert("wrong".to_string(), Value::from(r.get::<_, i64>(3)?));
            rec.insert("streak".to_string(), Value::from(r.get::<_, i64>(4)?));
            if let Some(last) = r.get::<_, Option<i64>>(5)? {
                rec.insert("last".to_string(), Value::Bool(last != 0));
            }
            if let Some(at) = r.get::<_, Option<String>>(6)?.filter(|a| !a.is_empty()) {
                rec.insert("at".to_string(), Value::String(at));
            }
            questions.insert(id, Value::Object(rec));
        }
    }
    let mut history = Vec::new();
    {
        let mut st =
            conn.prepare("SELECT at, sheets, mode, total, correct, percent, seconds, kind FROM attempts WHERE user_id=?1 ORDER BY id DESC LIMIT ?2")?;
        let mut rows = st.query(params![user, HISTORY_LIMIT])?;
        while let Some(r) = rows.next()? {
            let sheets: Value = serde_json::from_str(&r.get::<_, String>(1)?)?;
            history.push(json!({
                "at": r.get::<_, String>(0)?,
                "sheets": sheets,
                "mode": r.get::<_, String>(2)?,
                "total": r.get::<_, i64>(3)?,
                "correct": r.get::<_, i64>(4)?,
                "percent": r.get::<_, i64>(5)?,
                "seconds": r.get::<_, i64>(6)?,
                "kind": r.get::<_, String>(7)?,
            }));
        }
    }
    Ok(json!({
        "version": 1,
        "questions": Value::Object(questions),
        "history": history,
        "masterStreak": MASTER_STREAK,
    }))
}

impl Store {
    /// Opens (creating it if needed) the database and creates the tables. Timestamps use the system local time (honoring `TZ`).
    ///
    /// A schema-v1 database (v0.4 and older) is migrated to schema v2 once: if it holds any data, a consistent copy is first written
    /// to `<dir of the database>/backups/xexam-schema1-<YYYYmmdd-HHMMSS>.db`; if that copy cannot be made, opening fails and nothing
    /// is migrated. Opening an up-to-date database whose rows need no fix-up **writes nothing** (no transaction is committed;
    /// the only change a first v0.7 open makes to an older file is the switch to WAL in its header).
    ///
    /// The journal mode is WAL (see [`super::journal`]); [`Store::open_with`] takes the mode from the caller.
    pub fn open(path: &Path) -> Result<Store, DbError> {
        Self::open_with(path, Arc::new(SystemClock), JournalMode::Wal)
    }

    /// Same as [`Store::open`], but the time is supplied by the caller (for tests).
    pub fn open_with_clock(path: &Path, clock: Arc<dyn Clock>) -> Result<Store, DbError> {
        Self::open_with(path, clock, JournalMode::Wal)
    }

    /// Same as [`Store::open`], with the clock and the journal mode supplied by the caller (`main.rs` passes the mode chosen
    /// by `XEXAM_SQLITE_JOURNAL`). A mode SQLite refuses is not an error: a warning is logged and the reported mode is kept
    /// ([`Store::journal_mode`]).
    pub fn open_with(
        path: &Path,
        clock: Arc<dyn Clock>,
        journal: JournalMode,
    ) -> Result<Store, DbError> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| {
                rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
                    Some(format!("cannot create data directory: {}", e.kind())),
                )
            })?;
        }
        // Do not parse the path as a URI; treat it literally
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags)?;
        conn.busy_timeout(Duration::from_millis(5000))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.execute_batch(SCHEMA)?;
        // Check first, then write: the content of an up-to-date database stays unchanged
        let upgrade = migrate::upgrade(&conn, path, clock.as_ref())?;
        // WAL by default (v0.7): one fsync per commit with synchronous=FULL, readers and the writer do not block each other.
        // Switched only after a successful upgrade, so an open that fails (for example because the pre-migration backup
        // cannot be written) leaves an older database file exactly as it was. The default auto-checkpoint (1000 pages)
        // stays; `checkpoint` empties the -wal file at shutdown.
        let journal_mode = journal::apply(&conn, journal);
        Ok(Store {
            conn: Mutex::new(conn),
            clock,
            migration_backup: upgrade.backup,
            created_default_admin: upgrade.created_admin,
            converted_prerelease: upgrade.converted_prerelease,
            journal_mode,
        })
    }

    /// The journal mode in effect, as SQLite reported it when the database was opened (lower case: `wal`, `delete`, …).
    pub fn journal_mode(&self) -> &str {
        &self.journal_mode
    }

    /// `PRAGMA wal_checkpoint(TRUNCATE)` (run at a graceful shutdown): copies the `-wal` file into the database file and
    /// empties it. `Ok(None)` when the database is not in WAL mode (nothing to do).
    pub fn checkpoint(&self) -> Result<Option<Checkpoint>, DbError> {
        if self.journal_mode != JournalMode::Wal.as_str() {
            return Ok(None);
        }
        let g = self.lock();
        Ok(Some(journal::checkpoint_truncate(&g)?))
    }

    /// The backup written before this open migrated the database to the current schema, if any.
    pub fn migration_backup(&self) -> Option<&Path> {
        self.migration_backup.as_deref()
    }

    /// Whether this open converted a database of a v0.7 pre-release build (schema 4 with `users.all_banks`) to the released
    /// shape (`users.new_banks`, see `migrate.rs`); [`Store::migration_backup`] is then the copy written before.
    pub fn converted_prerelease(&self) -> bool {
        self.converted_prerelease
    }

    /// v0.7: retires every active question that is not of the six-column layout (imported from the 3- or 4-column layouts,
    /// by this version before v0.7 or by an older binary after a rollback), exactly as an import retires a question that left
    /// its workbook: `active = 0`, `updated_at = now` and a `seq` after the bank's other questions (their relative order
    /// kept). Study records, statistics, the wrong-answer notebook, banks and import history stay; exams, practice and the
    /// bank's question list no longer offer the question, and the bank page counts it as retired. A bank left without active
    /// questions stays, so it can be updated with a six-column file or deleted. Nothing is deleted.
    ///
    /// Idempotent and in ONE transaction; when there is nothing to retire nothing is written (the check is read-only).
    /// Returns what was retired, per bank in id order (empty when nothing was).
    pub fn retire_old_layout_questions(&self) -> Result<Vec<RetiredBank>, DbError> {
        let t = self.clock.now();
        let g = self.lock();
        let any: bool = g.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM questions WHERE active=1 AND {OLD_LAYOUT_SQL})"),
            [],
            |r| r.get(0),
        )?;
        if !any {
            return Ok(Vec::new());
        }
        run_tx(&g, |c| {
            let old: Vec<(String, Option<i64>)> = {
                let mut st = c.prepare(&format!(
                    "SELECT id, bank_id FROM questions WHERE active=1 AND {OLD_LAYOUT_SQL} ORDER BY bank_id, seq, rowid"
                ))?;
                let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect::<Result<_, _>>()?
            };
            let mut out: Vec<RetiredBank> = Vec::new();
            let mut next_seq: HashMap<Option<i64>, i64> = HashMap::new();
            let mut first_seq =
                c.prepare("SELECT COALESCE(MAX(seq), -1) + 1 FROM questions WHERE bank_id IS ?1")?;
            let mut down =
                c.prepare("UPDATE questions SET active=0, seq=?1, updated_at=?2 WHERE id=?3")?;
            for (id, bank) in &old {
                if !next_seq.contains_key(bank) {
                    let n: i64 = first_seq.query_row(params![bank], |r| r.get(0))?;
                    next_seq.insert(*bank, n);
                }
                let seq = next_seq.entry(*bank).or_insert(0);
                down.execute(params![*seq, t, id])?;
                *seq += 1;
                match out.last_mut() {
                    Some(last) if last.id == *bank => last.retired += 1,
                    _ => out.push(RetiredBank {
                        id: *bank,
                        name: String::new(),
                        retired: 1,
                    }),
                }
            }
            let mut name_of = c.prepare("SELECT name FROM banks WHERE id=?1")?;
            for b in &mut out {
                if let Some(id) = b.id {
                    b.name = name_of
                        .query_row(params![id], |r| r.get::<_, String>(0))
                        .optional()?
                        .unwrap_or_default();
                }
            }
            Ok(out)
        })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        let g = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        if !g.is_autocommit() {
            // The previous lock holder panicked and left an unfinished transaction
            let _ = g.execute_batch("ROLLBACK");
        }
        g
    }

    fn tx<T>(&self, f: impl FnOnce(&Connection) -> Result<T, DbError>) -> Result<T, DbError> {
        let g = self.lock();
        run_tx(&g, f)
    }

    pub fn count_active(&self) -> Result<i64, DbError> {
        let g = self.lock();
        Ok(
            g.query_row("SELECT COUNT(*) c FROM questions WHERE active=1", [], |r| {
                r.get(0)
            })?,
        )
    }

    /// `/api/bank`: key order file, mtime, error, questions, sheets (see 123_after_import_bank.json). Questions carry a trailing
    /// `bank` (their bank id). All banks are listed: grouped by bank id (ascending), each bank in `seq` order.
    pub fn bank(&self) -> Result<Value, DbError> {
        self.bank_as(&BankScope::All)
    }

    /// `/api/bank` limited to the banks in `scope` (see [`Store::bank`]): their questions and sheets only, and `file` /
    /// `mtime` of the latest import of one of those banks.
    pub fn bank_as(&self, scope: &BankScope) -> Result<Value, DbError> {
        let (mut rows, last) = {
            let g = self.lock();
            (
                load_questions(
                    &g,
                    &format!("SELECT {QUESTION_COLUMNS} FROM questions ORDER BY seq"),
                    [],
                )?,
                last_import(&g, scope)?,
            )
        };
        if !scope.is_all() {
            rows.retain(|r| scope.allows(r.bank_id));
        }
        // Stable: with a single bank this is exactly the old `ORDER BY seq` order (ties included)
        rows.sort_by_key(|r| r.bank_id);
        let mut order: Vec<(String, i64, i64)> = Vec::new();
        let mut pos: HashMap<String, usize> = HashMap::new();
        let mut questions = Vec::new();
        for r in &rows {
            let i = match pos.get(&r.sheet).copied() {
                Some(i) => i,
                None => {
                    order.push((r.sheet.clone(), 0, 0));
                    pos.insert(r.sheet.clone(), order.len() - 1);
                    order.len() - 1
                }
            };
            if let Some(entry) = order.get_mut(i) {
                if r.active {
                    entry.1 += 1;
                } else {
                    entry.2 += 1;
                }
            }
            if r.active {
                questions.push(Value::Object(question_object(r)?));
            }
        }
        let sheets: Vec<Value> = order
            .iter()
            .filter(|(_, usable, inactive)| *usable != 0 || *inactive != 0)
            .map(|(name, usable, inactive)| json!({"name": name, "usable": usable, "inactive": inactive, "total": usable, "skipped": []}))
            .collect();
        Ok(json!({
            "file": last.as_ref().map(|l| l.filename.as_str()).unwrap_or(""),
            "mtime": last.as_ref().map(|l| l.at.as_str()).unwrap_or(""),
            "error": "",
            "questions": questions,
            "sheets": sheets,
        }))
    }

    /// Preview against the legacy bank (uploads without `X-Bank-Target`): read-only, does not write to the database.
    pub fn plan_import(
        &self,
        sheets: &[ParsedSheet],
        questions: &[Question],
    ) -> Result<Value, DbError> {
        self.plan_import_for(ImportTarget::Legacy, sheets, questions)?
            .ok_or_else(|| json_err("the legacy target always exists"))
    }

    /// Preview against `target`: read-only. `Ok(None)` = the target bank does not exist.
    pub fn plan_import_for(
        &self,
        target: ImportTarget,
        sheets: &[ParsedSheet],
        questions: &[Question],
    ) -> Result<Option<Value>, DbError> {
        let g = self.lock();
        let bank = match target {
            ImportTarget::Legacy => banks::legacy_bank(&g)?,
            ImportTarget::New => None,
            ImportTarget::Bank(id) => match banks::bank_by_id(&g, id)? {
                Some(b) => Some(b),
                None => return Ok(None),
            },
        };
        let qs = banks::scoped_questions(bank.as_ref(), questions);
        Ok(Some(
            compute_plan(&g, bank.map(|b| b.id), sheets, &qs)?.json,
        ))
    }

    /// The real write against the legacy bank (created if it does not exist). See [`Store::apply_import_for`].
    pub fn apply_import(
        &self,
        filename: &str,
        sheets: &[ParsedSheet],
        questions: &[Question],
    ) -> Result<Value, DbError> {
        self.apply_import_for(ImportTarget::Legacy, filename, sheets, questions)?
            .map(|(plan, _)| plan)
            .ok_or_else(|| json_err("the legacy target always exists"))
    }

    /// The real write: one transaction (BEGIN IMMEDIATE … COMMIT, ROLLBACK on failure). Creates the bank for `New` (and the legacy
    /// bank for `Legacy` when it does not exist yet), recomputes the plan inside the transaction, upserts the questions, retires the
    /// bank's questions missing from the file (only within that bank), records the import and updates the bank's `filename` /
    /// `updated_at`. Returns `(plan, bank id)`; `Ok(None)` = the target bank no longer exists (nothing written).
    pub fn apply_import_for(
        &self,
        target: ImportTarget,
        filename: &str,
        sheets: &[ParsedSheet],
        questions: &[Question],
    ) -> Result<Option<(Value, i64)>, DbError> {
        self.apply_import_inner(None, target, filename, sheets, questions)
    }

    /// [`Store::apply_import_for`] for the signed-in `viewer`: a bank it creates is owned by them, and updating an existing
    /// bank (the target id, or the legacy bank) requires that they may manage it, checked inside the write transaction
    /// ([`DbError::NotBankOwner`] = nothing written).
    pub fn apply_import_as(
        &self,
        viewer: &Viewer,
        target: ImportTarget,
        filename: &str,
        sheets: &[ParsedSheet],
        questions: &[Question],
    ) -> Result<Option<(Value, i64)>, DbError> {
        self.apply_import_inner(Some(viewer), target, filename, sheets, questions)
    }

    /// `actor`: the signed-in user (owner of a created bank; must manage an updated one), or `None` for an import without a
    /// user (the `import` command, the legacy API): no check, created banks belong to the first administrator.
    fn apply_import_inner(
        &self,
        actor: Option<&Viewer>,
        target: ImportTarget,
        filename: &str,
        sheets: &[ParsedSheet],
        questions: &[Question],
    ) -> Result<Option<(Value, i64)>, DbError> {
        let t = self.clock.now();
        let owner = actor.map(|v| v.user);
        self.tx(|c| {
            let bank = match target {
                ImportTarget::Legacy => match banks::legacy_bank(c)? {
                    Some(b) => b,
                    None => banks::create_bank(c, filename, true, &t, owner)?,
                },
                ImportTarget::New => banks::create_bank(c, filename, false, &t, owner)?,
                ImportTarget::Bank(id) => match banks::bank_by_id(c, id)? {
                    Some(b) => b,
                    None => return Ok(None),
                },
            };
            if let Some(v) = actor {
                if !v.manages(banks::bank_owner(c, bank.id)?) {
                    return Err(DbError::NotBankOwner);
                }
            }
            let qs = banks::scoped_questions(Some(&bank), questions);
            let plan = write_import(c, bank.id, filename, sheets, &qs, &t)?;
            c.execute(
                "UPDATE banks SET filename=?1, updated_at=?2 WHERE id=?3",
                params![filename, t, bank.id],
            )?;
            Ok(Some((plan, bank.id)))
        })
    }

    pub fn import_history(&self, limit: i64) -> Result<Vec<ImportRecord>, DbError> {
        let g = self.lock();
        import_rows(&g, limit, &BankScope::All)
    }

    /// `/api/imports` as `viewer` sees it: everything for an administrator, the history of the banks they own for a bank
    /// manager (nothing for anyone else).
    pub fn import_history_as(
        &self,
        viewer: &Viewer,
        limit: i64,
    ) -> Result<Vec<ImportRecord>, DbError> {
        let g = self.lock();
        let scope = if viewer.admin {
            BankScope::All
        } else if viewer.can_manage {
            BankScope::Only(banks::owned_banks(&g, viewer.user)?)
        } else {
            BankScope::Only(Default::default())
        };
        import_rows(&g, limit, &scope)
    }

    /// `/api/progress` of `user`: `{"version":1,"questions":{…},"history":[…],"masterStreak":2}`
    pub fn progress(&self, user: i64) -> Result<Value, DbError> {
        self.progress_as(user, &BankScope::All)
    }

    /// `/api/progress` of `user`, with the statistics of the questions in `scope` only (see [`progress_from`]).
    pub fn progress_as(&self, user: i64, scope: &BankScope) -> Result<Value, DbError> {
        let g = self.lock();
        progress_from(&g, user, scope)
    }

    /// Records an exam without per-question answers (every detail row is stored as unanswered). See [`Store::record_attempt_with_answers`].
    pub fn record_attempt(&self, user: i64, attempt: &Attempt) -> Result<Value, DbError> {
        self.record_attempt_with_answers(user, attempt, &[])
    }

    /// Records an exam in one transaction: updates `stats`, inserts the `attempts` row (with the `banks` snapshot of the questions'
    /// banks, in order of first appearance) and one `attempt_items` row per result (`answers[i]` belongs to `results[i]`; missing
    /// entries are stored as NULL), then keeps only the newest `RECORD_RETENTION` exams of `user` (their detail rows go with
    /// them). Everything belongs to `user`. Returns their `/api/progress`.
    ///
    /// Only questions that still exist are counted (v0.6.5): an exam started before its question bank was deleted, or a result
    /// with a made-up id, must not write statistics or detail rows for a question that is gone, nor an exam record that no later
    /// bank deletion could ever find. Such results are left out of the totals and the record; their ids come back in
    /// `"notCounted"` (only when there are any) so the page can say so. Retired questions (`active = 0`, left over from a
    /// re-import) still exist and are counted. When no result is left, nothing is written: [`DbError::QuestionsGone`].
    pub fn record_attempt_with_answers(
        &self,
        user: i64,
        attempt: &Attempt,
        answers: &[Option<Value>],
    ) -> Result<Value, DbError> {
        self.record_attempt_as(user, &BankScope::All, attempt, answers)
    }

    /// [`Store::record_attempt_with_answers`] for a user who may see only the banks in `scope` (v0.6.5): a question of a bank
    /// outside it is treated exactly like a question that no longer exists (left out, listed in `"notCounted"`), so a made-up
    /// or stale request can never write statistics for a bank the user cannot see. The record keeps `attempt.kind`; a practice
    /// session updates the statistics (the wrong-answer notebook) exactly like an exam. Returns `/api/progress` limited to
    /// `scope`.
    pub fn record_attempt_as(
        &self,
        user: i64,
        scope: &BankScope,
        attempt: &Attempt,
        answers: &[Option<Value>],
    ) -> Result<Value, DbError> {
        self.record_attempt_tagged(user, scope, attempt, answers, &SessionTag::default())
    }

    /// [`Store::record_attempt_as`] that also stores the session key and start time of `tag` in the record (v0.9; NULL for
    /// what is `None`). The answer is `/api/progress` plus, when there are any, `"notCounted"`, plus the new record's
    /// `"recordId"` (the `attempts.id`), always last.
    pub fn record_attempt_tagged(
        &self,
        user: i64,
        scope: &BankScope,
        attempt: &Attempt,
        answers: &[Option<Value>],
        tag: &SessionTag,
    ) -> Result<Value, DbError> {
        if attempt.results.is_empty() {
            return Err(invalid_input("an attempt needs at least one result"));
        }
        let answer_texts = (0..attempt.results.len())
            .map(|i| {
                answers
                    .get(i)
                    .and_then(Option::as_ref)
                    .map(pyjson::to_string)
                    .transpose()
            })
            .collect::<Result<Vec<Option<String>>, _>>()?;
        let t = self.clock.now();
        let g = self.lock();
        let (not_counted, record_id) = run_tx(&g, |c| {
            users::ensure_active(c, user)?;
            // Split the results into the questions that still exist and those that are gone (checked inside the transaction,
            // so a bank deleted at the same moment is either fully counted before or fully left out after)
            let mut kept = Vec::new();
            let mut gone = Vec::new();
            {
                // Some(bank) = the question exists (bank None: a row without a bank, from an older binary)
                let mut bank_of = c.prepare("SELECT bank_id FROM questions WHERE id=?1")?;
                for (r, answer) in attempt.results.iter().zip(&answer_texts) {
                    let bank: Option<Option<i64>> = bank_of
                        .query_row(params![r.id], |row| row.get(0))
                        .optional()?;
                    if bank.is_some_and(|b| scope.allows(b)) {
                        kept.push((r, answer));
                    } else {
                        gone.push(r.id.clone());
                    }
                }
            }
            if kept.is_empty() {
                return Err(DbError::QuestionsGone);
            }
            let total = len_i64(kept.len());
            let good = len_i64(kept.iter().filter(|(r, _)| r.correct).count());
            let percent = percent_round_half_even(good, total);
            {
                let mut ins = c.prepare(
                    "INSERT OR IGNORE INTO user_stats(user_id, question_id) VALUES(?1, ?2)",
                )?;
                let mut ok = c.prepare(
                    "UPDATE user_stats SET attempts=attempts+1, correct=correct+1, streak=streak+1, last=1, at=?1 WHERE user_id=?2 AND question_id=?3",
                )?;
                let mut bad = c.prepare(
                    "UPDATE user_stats SET attempts=attempts+1, wrong=wrong+1, streak=0, last=0, at=?1 WHERE user_id=?2 AND question_id=?3",
                )?;
                for (r, _) in &kept {
                    ins.execute(params![user, r.id])?;
                    if r.correct {
                        ok.execute(params![t, user, r.id])?;
                    } else {
                        bad.execute(params![t, user, r.id])?;
                    }
                }
            }
            let counted: Vec<_> = kept.iter().map(|(r, _)| (*r).clone()).collect();
            let banks_json = records::bank_snapshot(c, &counted)?;
            // The record names the sheets the client sent; when questions were left out, only those sheets that a counted
            // question belongs to (a deleted bank's sheets must not show on a record none of whose questions came from them)
            let sheets_json = if gone.is_empty() {
                pyjson::to_string(&attempt.sheets)?
            } else {
                pyjson::to_string(&counted_sheets(c, &attempt.sheets, &counted)?)?
            };
            c.execute(
                "INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, banks, user_id, kind, session_key, started_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![t, sheets_json, attempt.mode, total, good, percent, attempt.seconds, banks_json, user, attempt.kind.as_str(), tag.key, tag.started_at],
            )?;
            let attempt_id = c.last_insert_rowid();
            {
                let mut item = c.prepare(
                    "INSERT INTO attempt_items(attempt_id, seq, question_id, answer, correct) VALUES(?1,?2,?3,?4,?5)",
                )?;
                for (seq, (r, answer)) in kept.iter().enumerate() {
                    item.execute(params![attempt_id, len_i64(seq), r.id, answer, r.correct])?;
                }
            }
            // Keep only the most recent RECORD_RETENTION sessions (and their detail); `/api/progress` still shows HISTORY_LIMIT
            records::prune(c, user, RECORD_RETENTION)?;
            Ok((gone, attempt_id))
        })?;
        let mut progress = progress_from(&g, user, scope)?;
        if !not_counted.is_empty() {
            if let Some(obj) = progress.as_object_mut() {
                obj.insert("notCounted".to_string(), json!(not_counted));
            }
        }
        if let Some(obj) = progress.as_object_mut() {
            obj.insert("recordId".to_string(), json!(record_id));
        }
        Ok(progress)
    }

    /// `/api/reset`: empties `user`'s per-question stats and every exam record of theirs (with its detail).
    pub fn reset_progress(&self, user: i64) -> Result<Value, DbError> {
        let g = self.lock();
        run_tx(&g, |c| records::delete_user_records(c, user, true))?;
        progress_from(&g, user, &BankScope::All)
    }

    /// Migration of the old progress.json into the study records of the first administrator (the account that claimed the data
    /// of earlier versions). Returns (whether anything was migrated, message): the Chinese message of the reference
    /// implementation (see [`LegacyMigration::message`]). On a malformed file returns (false, message) without crashing.
    pub fn migrate_legacy_progress(&self, path: &Path) -> Result<(bool, String), DbError> {
        let outcome = self.migrate_legacy_progress_outcome(path)?;
        Ok((outcome.migrated(), outcome.message()))
    }

    /// [`Store::migrate_legacy_progress`], returning what happened (the command-line program prints it in English).
    pub fn migrate_legacy_progress_outcome(&self, path: &Path) -> Result<LegacyMigration, DbError> {
        let g = self.lock();
        if get_meta(&g, "legacy_migrated")?.is_some_and(|v| !v.is_empty()) {
            return Ok(LegacyMigration::AlreadyMigrated);
        }
        if !path.is_file() {
            return Ok(LegacyMigration::NotFound);
        }
        let (qs, hist) = match legacy_progress::load(path) {
            Ok(x) => x,
            Err(e) => return Ok(LegacyMigration::Unreadable(e)),
        };
        if has_progress(&g)? {
            return Ok(LegacyMigration::HasRecords);
        }
        let (stats, attempts) = match legacy_progress::convert(&qs, &hist) {
            Ok(x) => x,
            Err(e) => return Ok(LegacyMigration::Malformed(e)),
        };
        let Some(owner) = users::owner_admin(&g)? else {
            return Ok(LegacyMigration::NoAdmin);
        };
        let t = self.clock.now();
        let res = run_tx(&g, |c| {
            if has_progress(c)? {
                return Ok(false);
            }
            write_legacy_rows(c, owner, &stats, &attempts, &t)?;
            Ok(true)
        });
        match res {
            Ok(true) => Ok(LegacyMigration::Migrated {
                sessions: hist.len(),
                questions: qs.len(),
            }),
            Ok(false) => Ok(LegacyMigration::HasRecords),
            Err(DbError::Sqlite(rusqlite::Error::SqliteFailure(e, _)))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Ok(LegacyMigration::Constraint)
            }
            Err(e) => Err(e),
        }
    }
}

/// What a migration of the old progress.json did (see [`Store::migrate_legacy_progress_outcome`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyMigration {
    /// Migrated: this many exam sessions and the statistics of this many questions
    Migrated { sessions: usize, questions: usize },
    /// The database says it was migrated before (the normal case on every later start)
    AlreadyMigrated,
    /// There is no progress.json at that path
    NotFound,
    /// The file cannot be read or is not the expected JSON (the reason, in English)
    Unreadable(String),
    /// The database already has study records: nothing is overwritten
    HasRecords,
    /// The file has malformed content (the reason, in English); nothing was migrated
    Malformed(String),
    /// The rows do not satisfy the table constraints; nothing was migrated
    Constraint,
    /// The database has no administrator to give the records to
    NoAdmin,
}

impl LegacyMigration {
    /// Whether anything was migrated.
    pub fn migrated(&self) -> bool {
        matches!(self, LegacyMigration::Migrated { .. })
    }

    /// The Chinese message of the reference implementation (pinned by the golden tests).
    pub fn message(&self) -> String {
        match self {
            LegacyMigration::Migrated {
                sessions,
                questions,
            } => format!("迁移了 {} 场成绩、{} 道题的答题记录", *sessions, *questions),
            LegacyMigration::AlreadyMigrated => "之前已经迁移过".to_string(),
            LegacyMigration::NotFound => "没有找到旧的 progress.json".to_string(),
            LegacyMigration::Unreadable(e) => format!("旧的 progress.json 读不了：{e}"),
            LegacyMigration::HasRecords => "数据库里已经有学习记录，没有覆盖".to_string(),
            LegacyMigration::Malformed(e) => {
                format!("旧的 progress.json 里有格式不对的内容，没有迁移：{e}")
            }
            LegacyMigration::Constraint => {
                "旧的 progress.json 里有格式不对的内容，没有迁移：数据不满足表的约束".to_string()
            }
            LegacyMigration::NoAdmin => "数据库里没有管理员账户，没有迁移".to_string(),
        }
    }

    /// The same in English, for the console and the log.
    pub fn english(&self) -> String {
        match self {
            LegacyMigration::Migrated {
                sessions,
                questions,
            } => format!(
                "migrated {sessions} exam session(s) and the answer statistics of {questions} question(s)"
            ),
            LegacyMigration::AlreadyMigrated => "it was migrated before".to_string(),
            LegacyMigration::NotFound => "there is no old progress.json".to_string(),
            LegacyMigration::Unreadable(e) => format!("the old progress.json cannot be read: {e}"),
            LegacyMigration::HasRecords => {
                "the database already has study records; nothing was overwritten".to_string()
            }
            LegacyMigration::Malformed(e) => {
                format!("the old progress.json has malformed content, nothing was migrated: {e}")
            }
            LegacyMigration::Constraint => {
                "the old progress.json has malformed content, nothing was migrated: the data does not satisfy the table constraints".to_string()
            }
            LegacyMigration::NoAdmin => {
                "the database has no administrator account; nothing was migrated".to_string()
            }
        }
    }
}

/// The sheets of `sent` (in the client's order) that at least one of the `counted` questions belongs to. When none of them
/// matches (a client that sent other labels), the sheets of the counted questions in order of first appearance instead.
fn counted_sheets(
    c: &Connection,
    sent: &[String],
    counted: &[AttemptResult],
) -> Result<Vec<String>, DbError> {
    let mut of_counted: Vec<String> = Vec::new();
    {
        let mut sheet = c.prepare("SELECT sheet FROM questions WHERE id=?1")?;
        for r in counted {
            let s: String = sheet.query_row(params![r.id], |row| row.get(0))?;
            if !of_counted.contains(&s) {
                of_counted.push(s);
            }
        }
    }
    let mut kept: Vec<String> = Vec::new();
    for s in sent {
        if of_counted.contains(s) && !kept.contains(s) {
            kept.push(s.clone());
        }
    }
    Ok(if kept.is_empty() { of_counted } else { kept })
}

fn write_legacy_rows(
    c: &Connection,
    user: i64,
    stats: &[StatRow],
    attempts: &[AttemptRow],
    t: &str,
) -> Result<(), DbError> {
    {
        let mut st = c.prepare(
            "INSERT OR REPLACE INTO user_stats(user_id, question_id, attempts, correct, wrong, streak, last, at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        )?;
        for s in stats {
            st.execute(params![
                user, s.id, s.attempts, s.correct, s.wrong, s.streak, s.last, s.at
            ])?;
        }
    }
    {
        let mut at = c.prepare("INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, user_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")?;
        for a in attempts {
            at.execute(params![
                a.at, a.sheets, a.mode, a.total, a.correct, a.percent, a.seconds, user
            ])?;
        }
    }
    c.execute(
        "INSERT INTO meta(key,value) VALUES('legacy_migrated', ?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![t],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::timefmt::FixedClock;
    use crate::models::{Answer, AttemptResult, Kind};

    fn question(sheet: &str, id: &str, stem: &str, row: i64) -> Question {
        Question {
            kind: Kind::Single,
            tag: String::new(),
            stem: stem.to_string(),
            options: vec![],
            items: vec![],
            answer: Answer::Letters(vec!["A".to_string()]),
            translation: String::new(),
            zh: None,
            english: String::new(),
            en: None,
            lang: String::new(),
            ja: None,
            explanation: String::new(),
            mnemonic: String::new(),
            note: String::new(),
            explain: None,
            id: id.to_string(),
            sheet: sheet.to_string(),
            row,
        }
    }

    fn sheet(name: &str, usable: i64) -> ParsedSheet {
        ParsedSheet {
            name: name.to_string(),
            total: usable,
            usable,
            skipped: vec![],
            warnings: vec![],
        }
    }

    fn open_tmp() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::open_with_clock(
            &dir.path().join("x.db"),
            Arc::new(FixedClock("2026-01-02 03:04".to_string())),
        )
        .expect("open");
        (dir, store)
    }

    #[test]
    fn store_is_send_and_sync() {
        fn assert_ss<T: Send + Sync>() {}
        assert_ss::<Store>();
    }

    #[test]
    fn percent_ties_go_to_even() {
        assert_eq!(percent_round_half_even(1, 8), 12);
        assert_eq!(percent_round_half_even(3, 8), 38);
        assert_eq!(percent_round_half_even(1, 40), 2);
        assert_eq!(percent_round_half_even(1, 200), 0);
        assert_eq!(percent_round_half_even(0, 5), 0);
        assert_eq!(percent_round_half_even(5, 5), 100);
        assert_eq!(percent_round_half_even(1, 3), 33);
        assert_eq!(percent_round_half_even(2, 3), 67);
        assert_eq!(percent_round_half_even(1, 0), 0);
    }

    #[test]
    fn percent_matches_float_reference() {
        for total in 1..=400i64 {
            for good in 0..=total {
                let want = (100.0 * good as f64 / total as f64).round_ties_even() as i64;
                assert_eq!(percent_round_half_even(good, total), want, "{good}/{total}");
            }
        }
    }

    #[test]
    fn import_flow_added_updated_restored_removed() {
        let (_d, s) = open_tmp();
        let sh = vec![sheet("S", 3)];
        let qs = vec![
            question("S", "a", "A?", 1),
            question("S", "b", "B?", 2),
            question("S", "c", "C?", 3),
        ];
        let plan = s.plan_import(&sh, &qs).expect("plan");
        assert_eq!(plan["added"], 3);
        assert_eq!(
            s.count_active().expect("count"),
            0,
            "preview must not write"
        );
        let r = s.apply_import("one.xlsx", &sh, &qs).expect("apply");
        assert_eq!(r, plan);
        assert_eq!(s.count_active().expect("count"), 3);

        // b changes its stem, c disappears, d is added
        let qs2 = vec![
            question("S", "a", "A?", 5),
            question("S", "b", "B2?", 2),
            question("S", "d", "D?", 3),
        ];
        let r2 = s
            .apply_import("two.xlsx", &[sheet("S", 3)], &qs2)
            .expect("apply2");
        assert_eq!(
            (
                r2["added"].as_i64(),
                r2["updated"].as_i64(),
                r2["unchanged"].as_i64(),
                r2["removed"].as_i64()
            ),
            (Some(1), Some(1), Some(1), Some(1))
        );
        assert_eq!(r2["removed_samples"], json!(["C?"]));
        let bank = s.bank().expect("bank");
        assert_eq!(bank["file"], "two.xlsx");
        assert_eq!(bank["mtime"], "2026-01-02 03:04");
        assert_eq!(bank["sheets"][0]["inactive"], 1);
        assert_eq!(bank["questions"].as_array().map(Vec::len), Some(3));

        // Back to the first version: c is restored
        let r3 = s.apply_import("three.xlsx", &sh, &qs).expect("apply3");
        assert_eq!(r3["restored"], 1);
        assert_eq!(s.import_history(10).expect("hist").len(), 3);
    }

    #[test]
    fn removed_sheet_only_in_database_is_appended() {
        let (_d, s) = open_tmp();
        s.apply_import(
            "a.xlsx",
            &[sheet("Old", 1)],
            &[question("Old", "o", "O?", 1)],
        )
        .expect("a");
        let plan = s
            .plan_import(&[sheet("New", 1)], &[question("New", "n", "N?", 1)])
            .expect("plan");
        let names: Vec<&str> = plan["sheets"]
            .as_array()
            .expect("arr")
            .iter()
            .filter_map(|x| x["name"].as_str())
            .collect();
        assert_eq!(names, vec!["New", "Old"]);
        assert_eq!(plan["sheets"][1]["removed"], 1);
        assert_eq!(plan["sheets"][1]["total"], 0);
    }

    #[test]
    fn attempt_updates_stats_and_prunes() {
        let (_d, s) = open_tmp();
        // Only questions that exist are counted
        s.apply_import(
            "a.xlsx",
            &[sheet("日本語", 2)],
            &[
                question("日本語", "q1", "Q1?", 1),
                question("日本語", "q2", "Q2?", 2),
            ],
        )
        .expect("import");
        let mk = |ok: bool| Attempt {
            results: vec![
                AttemptResult {
                    id: "q1".to_string(),
                    correct: ok,
                },
                AttemptResult {
                    id: "q2".to_string(),
                    correct: true,
                },
            ],
            sheets: vec!["日本語".to_string()],
            mode: "all".to_string(),
            seconds: 7,
            kind: Default::default(),
        };
        let p = s.record_attempt(1, &mk(false)).expect("rec");
        assert_eq!(p["questions"]["q1"]["wrong"], 1);
        assert_eq!(p["questions"]["q1"]["last"], false);
        assert_eq!(p["history"][0]["percent"], 50);
        for _ in 0..70 {
            s.record_attempt(1, &mk(true)).expect("rec");
        }
        let p = s.progress(1).expect("progress");
        assert_eq!(p["history"].as_array().map(Vec::len), Some(60));
        assert_eq!(p["questions"]["q1"]["attempts"], 71);
        let p = s.reset_progress(1).expect("reset");
        assert_eq!(p["questions"], json!({}));
        assert_eq!(p["history"], json!([]));
        assert!(s
            .record_attempt(
                1,
                &Attempt {
                    results: vec![],
                    sheets: vec![],
                    mode: "all".into(),
                    seconds: 0,
                    kind: Default::default(),
                }
            )
            .is_err());
    }

    #[test]
    fn leftover_transaction_is_rolled_back_when_lock_recovers() {
        let (_d, s) = open_tmp();
        {
            let g = s.lock();
            g.execute_batch("BEGIN IMMEDIATE").expect("begin");
            g.execute("INSERT INTO meta(key,value) VALUES('zzz','1')", [])
                .expect("insert");
        }
        // Simulate a previous call panicking mid-transaction: the next lock acquisition must recover
        assert_eq!(get_meta(&s.lock(), "zzz").expect("meta"), None);
        s.reset_progress(1).expect("reset after recovery");
    }
}
