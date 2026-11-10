//! Exam records (v0.5): `attempts` rows plus their per-question detail in `attempt_items`.
//!
//! `GET /api/records` ([`Store::records`]), `GET /api/record` ([`Store::record`]), `POST /api/records/delete`
//! ([`Store::delete_record`], [`Store::delete_all_records`]); the `banks` snapshot and the retention prune used by `record_attempt`.
use super::super::{pyjson, RECORD_RETENTION};
use super::{load_questions, question_object, run_tx, Store, QUESTION_COLUMNS};
use crate::error::DbError;
use crate::models::AttemptResult;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// The `(id, name)` pairs of an `attempts.banks` snapshot. Lenient: anything that is not an object with an integer `id` is skipped,
/// and text that is not a JSON array gives no banks.
fn parse_snapshot(text: &str) -> Vec<(i64, String)> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|b| {
            let id = b.get("id")?.as_i64()?;
            let name = b.get("name").and_then(Value::as_str).unwrap_or("");
            Some((id, name.to_string()))
        })
        .collect()
}

/// The bank ids of an `attempts.banks` snapshot (see [`parse_snapshot`]).
pub(super) fn snapshot_ids(text: &str) -> Vec<i64> {
    parse_snapshot(text).into_iter().map(|(id, _)| id).collect()
}

/// The `banks` snapshot of a new exam: the banks of its questions, in order of first appearance, with their current names.
/// Questions that do not exist (or have no bank) are ignored.
pub(super) fn bank_snapshot(c: &Connection, results: &[AttemptResult]) -> Result<String, DbError> {
    let mut seen: HashSet<i64> = HashSet::new();
    let mut out = Vec::new();
    let mut bank_of = c.prepare("SELECT bank_id FROM questions WHERE id=?1")?;
    let mut name_of = c.prepare("SELECT name FROM banks WHERE id=?1")?;
    for r in results {
        let bank_id: Option<i64> = bank_of
            .query_row(params![r.id], |row| row.get(0))
            .optional()?
            .flatten();
        let Some(bank_id) = bank_id else { continue };
        if !seen.insert(bank_id) {
            continue;
        }
        if let Some(name) = name_of
            .query_row(params![bank_id], |row| row.get::<_, String>(0))
            .optional()?
        {
            out.push(json!({ "id": bank_id, "name": name }));
        }
    }
    Ok(pyjson::to_string(&Value::Array(out))?)
}

/// Keeps only the newest `keep` exams; the detail rows of the pruned ones go with them.
pub(super) fn prune(c: &Connection, keep: i64) -> Result<(), DbError> {
    let cutoff: Option<i64> = c
        .query_row(
            "SELECT id FROM attempts ORDER BY id DESC LIMIT 1 OFFSET ?1",
            params![keep - 1],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(cutoff) = cutoff {
        c.execute(
            "DELETE FROM attempt_items WHERE attempt_id < ?1",
            params![cutoff],
        )?;
        c.execute("DELETE FROM attempts WHERE id < ?1", params![cutoff])?;
    }
    Ok(())
}

/// Record summaries, newest first (`only`: just that record).
fn summaries(c: &Connection, only: Option<i64>) -> Result<Vec<Value>, DbError> {
    let names: HashMap<i64, String> = {
        let mut st = c.prepare("SELECT id, name FROM banks")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    // attempt id -> (detail rows, answered rows)
    let mut counts: HashMap<i64, (i64, i64)> = HashMap::new();
    {
        let sql = match only {
            Some(_) => "SELECT attempt_id, COUNT(*), COUNT(answer) FROM attempt_items WHERE attempt_id=?1 GROUP BY attempt_id",
            None => "SELECT attempt_id, COUNT(*), COUNT(answer) FROM attempt_items GROUP BY attempt_id",
        };
        let mut st = c.prepare(sql)?;
        let mut rows = match only {
            Some(id) => st.query(params![id])?,
            None => st.query([])?,
        };
        while let Some(r) = rows.next()? {
            counts.insert(r.get(0)?, (r.get(1)?, r.get(2)?));
        }
    }
    let base = "SELECT id, at, sheets, mode, total, correct, percent, seconds, banks FROM attempts";
    let mut st = match only {
        Some(_) => c.prepare(&format!("{base} WHERE id=?1"))?,
        None => c.prepare(&format!("{base} ORDER BY id DESC"))?,
    };
    let mut rows = match only {
        Some(id) => st.query(params![id])?,
        None => st.query([])?,
    };
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let sheets: Value = serde_json::from_str(&r.get::<_, String>(2)?)?;
        let banks: Vec<Value> = parse_snapshot(&r.get::<_, String>(8)?)
            .into_iter()
            .map(|(bank_id, snapshot_name)| {
                let current = names.get(&bank_id);
                json!({
                    "id": bank_id,
                    "name": current.cloned().unwrap_or(snapshot_name),
                    "deleted": current.is_none(),
                })
            })
            .collect();
        let (items, answered) = match counts.get(&id) {
            Some(&(n, a)) if n > 0 => (n, Value::from(a)),
            _ => (0, Value::Null),
        };
        out.push(json!({
            "id": id,
            "at": r.get::<_, String>(1)?,
            "banks": banks,
            "sheets": sheets,
            "mode": r.get::<_, String>(3)?,
            "total": r.get::<_, i64>(4)?,
            "correct": r.get::<_, i64>(5)?,
            "percent": r.get::<_, i64>(6)?,
            "seconds": r.get::<_, i64>(7)?,
            "answered": answered,
            "detail": items > 0,
        }));
    }
    Ok(out)
}

/// `{"records": [...], "retention": 500}`
fn records_json(c: &Connection) -> Result<Value, DbError> {
    Ok(json!({
        "records": summaries(c, None)?,
        "retention": RECORD_RETENTION,
    }))
}

fn record_exists(c: &Connection, id: i64) -> Result<bool, DbError> {
    Ok(c.query_row(
        "SELECT 1 FROM attempts WHERE id=?1",
        params![id],
        |_| Ok(()),
    )
    .optional()?
    .is_some())
}

impl Store {
    /// `GET /api/records`: every kept exam record, newest first, and the retention limit.
    pub fn records(&self) -> Result<Value, DbError> {
        let g = self.lock();
        records_json(&g)
    }

    /// `GET /api/record?id=`: `{"record": summary, "items": [{"seq", "questionId", "answer", "correct", "question"}]}`, where
    /// `question` is the question as `/api/bank` shows it plus a trailing `active` (retired questions included), or null when the
    /// question no longer exists. `Ok(None)` = no such record.
    pub fn record(&self, id: i64) -> Result<Option<Value>, DbError> {
        let g = self.lock();
        let Some(record) = summaries(&g, Some(id))?.into_iter().next() else {
            return Ok(None);
        };
        let rows: Vec<(i64, String, Option<String>, bool)> = {
            let mut st = g.prepare(
                "SELECT seq, question_id, answer, correct FROM attempt_items WHERE attempt_id=?1 ORDER BY seq",
            )?;
            let rows = st.query_map(params![id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? != 0))
            })?;
            rows.collect::<Result<_, _>>()?
        };
        let question_sql = format!("SELECT {QUESTION_COLUMNS} FROM questions WHERE id=?1");
        let mut items = Vec::with_capacity(rows.len());
        for (seq, question_id, answer, correct) in rows {
            let answer: Value = match answer {
                Some(text) => serde_json::from_str(&text)?,
                None => Value::Null,
            };
            let question = match load_questions(&g, &question_sql, params![question_id])?.first() {
                Some(q) => {
                    let mut obj = question_object(q)?;
                    obj.insert("active".to_string(), Value::Bool(q.active));
                    Value::Object(obj)
                }
                None => Value::Null,
            };
            items.push(json!({
                "seq": seq,
                "questionId": question_id,
                "answer": answer,
                "correct": correct,
                "question": question,
            }));
        }
        Ok(Some(json!({ "record": record, "items": items })))
    }

    /// Deletes one exam record and its detail (the per-question stats are not changed). `false` = no such record (nothing written).
    pub fn delete_record(&self, id: i64) -> Result<bool, DbError> {
        let g = self.lock();
        run_tx(&g, |c| {
            if !record_exists(c, id)? {
                return Ok(false);
            }
            c.execute("DELETE FROM attempt_items WHERE attempt_id=?1", params![id])?;
            c.execute("DELETE FROM attempts WHERE id=?1", params![id])?;
            Ok(true)
        })
    }

    /// Deletes every exam record and its detail; `with_stats` also empties the per-question stats (the wrong notebook).
    pub fn delete_all_records(&self, with_stats: bool) -> Result<(), DbError> {
        let g = self.lock();
        run_tx(&g, |c| {
            c.execute("DELETE FROM attempt_items", [])?;
            c.execute("DELETE FROM attempts", [])?;
            if with_stats {
                c.execute("DELETE FROM stats", [])?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_are_parsed_leniently() {
        assert_eq!(
            parse_snapshot(r#"[{"id": 1, "name": "A"}, {"id": 2}]"#),
            vec![(1, "A".to_string()), (2, String::new())]
        );
        assert_eq!(
            parse_snapshot(r#"[{"id": "1"}, 5, null, {"name": "x"}, {"id": 1.5}]"#),
            vec![]
        );
        assert!(parse_snapshot("").is_empty());
        assert!(parse_snapshot("{}").is_empty());
        assert!(parse_snapshot("[").is_empty());
        assert_eq!(
            snapshot_ids(r#"[{"id": 3, "name": "c"}, {"id": 1, "name": "a"}]"#),
            vec![3, 1]
        );
    }
}
