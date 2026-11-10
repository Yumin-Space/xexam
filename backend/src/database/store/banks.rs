//! Question banks (v0.5): one bank = one imported workbook.
//!
//! - The legacy bank (`legacy_ids=1`) holds the data from before v0.5 and is the target of uploads without `X-Bank-Target`; its
//!   question ids are the parser ids, unchanged, so old stats / wrong notebook / history stay linked.
//! - Every other bank uses bank-scoped ids ([`scoped_id`]), so the same workbook can be imported as two independent banks.
//! - `GET /api/banks` ([`Store::banks`]) and `POST /api/banks/delete` ([`Store::delete_bank`]).
use super::records::snapshot_ids;
use super::{run_tx, Store, IN_CHUNK};
use crate::error::DbError;
use crate::models::{DeletedBank, Question};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

/// Name of a legacy bank whose workbook name is unknown (no import row).
pub const FALLBACK_BANK_NAME: &str = "Question bank";

/// A row of `banks`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BankRow {
    pub id: i64,
    pub name: String,
    pub filename: String,
    pub legacy: bool,
    pub created_at: String,
    pub updated_at: String,
}

const BANK_COLUMNS: &str = "id, name, filename, legacy_ids, created_at, updated_at";

fn bank_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<BankRow> {
    Ok(BankRow {
        id: r.get(0)?,
        name: r.get(1)?,
        filename: r.get(2)?,
        legacy: r.get::<_, i64>(3)? != 0,
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
    })
}

/// Question id inside a non-legacy bank: the first 12 hex digits of `sha1("xexam-bank:{bank_id}\n{parser_id}")`.
pub fn scoped_id(bank_id: i64, parser_id: &str) -> String {
    let digest = Sha1::digest(format!("xexam-bank:{bank_id}\n{parser_id}").as_bytes());
    let mut out = String::with_capacity(12);
    for b in digest.iter().take(6) {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// The questions with the ids they have in `bank`: unchanged for the legacy bank and for a bank that does not exist yet
/// (nothing to compare against), bank-scoped otherwise.
pub(super) fn scoped_questions<'a>(
    bank: Option<&BankRow>,
    questions: &'a [Question],
) -> Cow<'a, [Question]> {
    match bank {
        Some(b) if !b.legacy => Cow::Owned(
            questions
                .iter()
                .map(|q| {
                    let mut q = q.clone();
                    q.id = scoped_id(b.id, &q.id);
                    q
                })
                .collect(),
        ),
        _ => Cow::Borrowed(questions),
    }
}

pub(super) fn legacy_bank(c: &Connection) -> Result<Option<BankRow>, DbError> {
    Ok(c.query_row(
        &format!("SELECT {BANK_COLUMNS} FROM banks WHERE legacy_ids=1 ORDER BY id LIMIT 1"),
        [],
        bank_row,
    )
    .optional()?)
}

pub(super) fn bank_by_id(c: &Connection, id: i64) -> Result<Option<BankRow>, DbError> {
    Ok(c.query_row(
        &format!("SELECT {BANK_COLUMNS} FROM banks WHERE id=?1"),
        params![id],
        bank_row,
    )
    .optional()?)
}

/// The upload file name without a trailing `.xlsx` (case-insensitive); the whole name if that leaves nothing.
pub(super) fn name_from_filename(filename: &str) -> String {
    let n = filename.len();
    let stem = match filename.get(n.saturating_sub(5)..) {
        Some(ext) if n >= 5 && ext.eq_ignore_ascii_case(".xlsx") => filename.get(..n - 5),
        _ => None,
    };
    match stem {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => filename.to_string(),
    }
}

/// `base`, or `base (2)`, `base (3)`, … — the first one no other bank uses.
fn unique_name(c: &Connection, base: &str) -> Result<String, DbError> {
    let taken: HashSet<String> = {
        let mut st = c.prepare("SELECT name FROM banks")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut name = base.to_string();
    let mut n = 2u64;
    while taken.contains(&name) {
        name = format!("{base} ({n})");
        n += 1;
    }
    Ok(name)
}

pub(super) fn insert_bank(
    c: &Connection,
    name: &str,
    filename: &str,
    legacy: bool,
    created_at: &str,
    updated_at: &str,
) -> Result<i64, DbError> {
    c.execute(
        "INSERT INTO banks(name, filename, legacy_ids, created_at, updated_at) VALUES(?1,?2,?3,?4,?5)",
        params![name, filename, i64::from(legacy), created_at, updated_at],
    )?;
    Ok(c.last_insert_rowid())
}

/// Creates a bank for an upload named `filename` (inside the caller's transaction), named after the file.
pub(super) fn create_bank(
    c: &Connection,
    filename: &str,
    legacy: bool,
    now: &str,
) -> Result<BankRow, DbError> {
    let name = unique_name(c, &name_from_filename(filename))?;
    let id = insert_bank(c, &name, filename, legacy, now, now)?;
    Ok(BankRow {
        id,
        name,
        filename: filename.to_string(),
        legacy,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    })
}

#[derive(Default)]
struct BankAgg {
    total: i64,
    inactive: i64,
    /// (sheet, usable, inactive) in order of first appearance
    sheets: Vec<(String, i64, i64)>,
}

/// `{"banks": [...]}` for `GET /api/banks`, ordered by id.
fn banks_json(c: &Connection) -> Result<Value, DbError> {
    let banks: Vec<BankRow> = {
        let mut st = c.prepare(&format!("SELECT {BANK_COLUMNS} FROM banks ORDER BY id"))?;
        let rows = st.query_map([], bank_row)?;
        rows.collect::<Result<_, _>>()?
    };
    // Same query shape and row order as `/api/bank` (a full scan sorted by `seq`), so the sheet order matches it
    let mut aggs: HashMap<i64, BankAgg> = HashMap::new();
    {
        let mut st = c.prepare("SELECT bank_id, sheet, active FROM questions ORDER BY seq")?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let Some(bank_id) = r.get::<_, Option<i64>>(0)? else {
                continue;
            };
            let sheet: String = r.get(1)?;
            let active = r.get::<_, i64>(2)? != 0;
            let agg = aggs.entry(bank_id).or_default();
            let i = match agg.sheets.iter().position(|(name, _, _)| *name == sheet) {
                Some(i) => i,
                None => {
                    agg.sheets.push((sheet, 0, 0));
                    agg.sheets.len() - 1
                }
            };
            if let Some(entry) = agg.sheets.get_mut(i) {
                if active {
                    entry.1 += 1;
                } else {
                    entry.2 += 1;
                }
            }
            if active {
                agg.total += 1;
            } else {
                agg.inactive += 1;
            }
        }
    }
    let studied: HashMap<i64, i64> = {
        // Shared numbers: a question counts once if any user has statistics for it
        let mut st = c.prepare(
            "SELECT q.bank_id, COUNT(*) FROM (SELECT DISTINCT question_id FROM user_stats) s JOIN questions q ON q.id = s.question_id \
             WHERE q.bank_id IS NOT NULL GROUP BY q.bank_id",
        )?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut records: HashMap<i64, i64> = HashMap::new();
    {
        let mut st = c.prepare("SELECT banks FROM attempts")?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let ids: HashSet<i64> = snapshot_ids(&r.get::<_, String>(0)?).into_iter().collect();
            for id in ids {
                *records.entry(id).or_default() += 1;
            }
        }
    }
    let list: Vec<Value> = banks
        .iter()
        .map(|b| {
            let agg = aggs.remove(&b.id).unwrap_or_default();
            let sheets: Vec<Value> = agg
                .sheets
                .iter()
                .map(|(name, usable, inactive)| json!({"name": name, "usable": usable, "inactive": inactive}))
                .collect();
            json!({
                "id": b.id,
                "name": b.name,
                "filename": b.filename,
                "legacy": b.legacy,
                "createdAt": b.created_at,
                "updatedAt": b.updated_at,
                "total": agg.total,
                "inactive": agg.inactive,
                "sheets": sheets,
                "studied": studied.get(&b.id).copied().unwrap_or(0),
                "records": records.get(&b.id).copied().unwrap_or(0),
            })
        })
        .collect();
    Ok(json!({ "banks": list }))
}

/// Runs `sql` (with one `{marks}` placeholder for `?,?,…`) for every chunk of `ids`.
fn exec_in_chunks(c: &Connection, sql: &str, ids: &[String]) -> Result<(), DbError> {
    for chunk in ids.chunks(IN_CHUNK) {
        let marks = vec!["?"; chunk.len()].join(",");
        c.execute(
            &sql.replace("{marks}", &marks),
            rusqlite::params_from_iter(chunk.iter()),
        )?;
    }
    Ok(())
}

impl Store {
    pub fn bank_exists(&self, id: i64) -> Result<bool, DbError> {
        let g = self.lock();
        Ok(bank_by_id(&g, id)?.is_some())
    }

    /// `GET /api/banks`: `{"banks": [{"id", "name", "filename", "legacy", "createdAt", "updatedAt", "total", "inactive",
    /// "sheets": [{"name", "usable", "inactive"}], "studied", "records"}]}`.
    pub fn banks(&self) -> Result<Value, DbError> {
        let g = self.lock();
        banks_json(&g)
    }

    /// An opaque stamp of the shared question library (`"library"` of `GET /api/auth/state`): it changes whenever a bank is
    /// imported, replaced or deleted, also by another process (the `import` command). Cheap: a few aggregates, no question text.
    pub fn library_stamp(&self) -> Result<String, DbError> {
        let g = self.lock();
        let (banks, last_bank, updated): (i64, i64, String) = g.query_row(
            "SELECT COUNT(*), COALESCE(MAX(id), 0), COALESCE(MAX(updated_at), '') FROM banks",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let last_import: i64 =
            g.query_row("SELECT COALESCE(MAX(id), 0) FROM imports", [], |r| r.get(0))?;
        let active: i64 =
            g.query_row("SELECT COUNT(*) FROM questions WHERE active=1", [], |r| {
                r.get(0)
            })?;
        Ok(format!(
            "{banks}.{last_bank}.{last_import}.{active}.{}",
            updated.replace(['-', ':', ' '], "")
        ))
    }

    /// Deletes bank `id` in one transaction: its questions (active and retired), its import history and the bank row. With
    /// `with_records` also its study data, of every user: the `user_stats` and `attempt_items` rows of those questions, and every
    /// exam record whose `banks` snapshot names only this bank (with the rest of its detail). `Ok(None)` = no such bank (nothing
    /// written).
    pub fn delete_bank(&self, id: i64, with_records: bool) -> Result<Option<DeletedBank>, DbError> {
        let g = self.lock();
        run_tx(&g, |c| {
            let Some(bank) = bank_by_id(c, id)? else {
                return Ok(None);
            };
            let question_ids: Vec<String> = {
                let mut st = c.prepare("SELECT id FROM questions WHERE bank_id=?1")?;
                let rows = st.query_map(params![id], |r| r.get::<_, String>(0))?;
                rows.collect::<Result<_, _>>()?
            };
            c.execute("DELETE FROM questions WHERE bank_id=?1", params![id])?;
            c.execute("DELETE FROM imports WHERE bank_id=?1", params![id])?;
            c.execute("DELETE FROM banks WHERE id=?1", params![id])?;
            if with_records {
                exec_in_chunks(
                    c,
                    "DELETE FROM user_stats WHERE question_id IN ({marks})",
                    &question_ids,
                )?;
                exec_in_chunks(
                    c,
                    "DELETE FROM attempt_items WHERE question_id IN ({marks})",
                    &question_ids,
                )?;
                // Exams that drew from this bank and from no bank that still exists (an exam mixing this bank with one
                // deleted earlier would otherwise be left behind as an empty record)
                let remaining: HashSet<i64> = {
                    let mut st = c.prepare("SELECT id FROM banks")?;
                    let rows = st.query_map([], |r| r.get::<_, i64>(0))?;
                    rows.collect::<Result<_, _>>()?
                };
                let only_this: Vec<i64> = {
                    let mut st = c.prepare("SELECT id, banks FROM attempts")?;
                    let rows =
                        st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
                    let mut out = Vec::new();
                    for row in rows {
                        let (attempt_id, banks) = row?;
                        let ids = snapshot_ids(&banks);
                        if ids.contains(&id) && ids.iter().all(|b| !remaining.contains(b)) {
                            out.push(attempt_id);
                        }
                    }
                    out
                };
                for attempt_id in only_this {
                    c.execute(
                        "DELETE FROM attempt_items WHERE attempt_id=?1",
                        params![attempt_id],
                    )?;
                    c.execute("DELETE FROM attempts WHERE id=?1", params![attempt_id])?;
                }
            }
            Ok(Some(DeletedBank {
                id,
                legacy: bank.legacy,
            }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_ids_are_twelve_hex_and_depend_on_the_bank() {
        let a = scoped_id(2, "94e4bfe00e8d");
        assert_eq!(a.len(), 12);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, scoped_id(3, "94e4bfe00e8d"));
        assert_ne!(a, scoped_id(2, "94e4bfe00e8e"));
        assert_eq!(a, scoped_id(2, "94e4bfe00e8d"));
        // the documented formula, computed independently
        let digest = Sha1::digest(b"xexam-bank:2\n94e4bfe00e8d");
        let want: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
        assert_eq!(a, want);
    }

    #[test]
    fn names_from_file_names() {
        assert_eq!(name_from_filename("Kitchen basics.xlsx"), "Kitchen basics");
        assert_eq!(name_from_filename("Kitchen basics.XLSX"), "Kitchen basics");
        assert_eq!(name_from_filename("a.b.Xlsx"), "a.b");
        assert_eq!(name_from_filename(".xlsx"), ".xlsx");
        assert_eq!(name_from_filename("notes.xls"), "notes.xls");
        assert_eq!(name_from_filename("日本語.xlsx"), "日本語");
        assert_eq!(name_from_filename("日.xlsx"), "日");
        assert_eq!(name_from_filename("xlsx"), "xlsx");
        assert_eq!(name_from_filename(""), "");
    }
}
