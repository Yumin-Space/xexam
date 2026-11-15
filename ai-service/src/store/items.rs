//! Accepted questions of a running generation job (`gen_items`, schema 7). Rows are deleted when the job ends (cascade
//! with the task, or `retention::sweep_once` for a job that did not run to its end).
use rusqlite::{params, Connection};

/// One accepted question of a job: `no` is the arrival order (never reused), `slot` the planned position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRow {
    pub no: i64,
    pub slot: i64,
    /// JSON: `{"kind","stem":{en,zh,ja},"options":[{en,zh,ja,"correct"}],"explanation":{en,zh,ja}}`
    pub data: String,
    pub at: i64,
}

/// Adds an item; returns its `no` (1 for the first item of the task, then highest + 1; a removed item stays as a tombstone, so a `no` is never reused).
pub fn items_add(
    c: &Connection,
    task: i64,
    slot: i64,
    data: &str,
    now: i64,
) -> rusqlite::Result<i64> {
    let no: i64 = c.query_row(
        "SELECT COALESCE(MAX(no), 0) + 1 FROM gen_items WHERE task_id = ?1",
        [task],
        |r| r.get(0),
    )?;
    c.execute(
        "INSERT INTO gen_items (task_id, no, slot, data, at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![task, no, slot, data, now],
    )?;
    Ok(no)
}

/// The items with `no > after`, in arrival order, at most `limit`.
pub fn items_after(
    c: &Connection,
    task: i64,
    after: i64,
    limit: i64,
) -> rusqlite::Result<Vec<ItemRow>> {
    let mut st = c.prepare(
        "SELECT no, slot, data, at FROM gen_items WHERE task_id = ?1 AND no > ?2 AND data <> '' ORDER BY no LIMIT ?3",
    )?;
    let rows = st.query_map(params![task, after, limit], |r| {
        Ok(ItemRow {
            no: r.get(0)?,
            slot: r.get(1)?,
            data: r.get(2)?,
            at: r.get(3)?,
        })
    })?;
    rows.collect()
}

/// Removes one item (a question later dropped as a cross-batch duplicate). The row stays as a tombstone (`data = ''`,
/// skipped by [`items_after`]) so its `no` is never reused, even when it was the newest.
pub fn items_remove(c: &Connection, task: i64, no: i64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE gen_items SET data = '' WHERE task_id = ?1 AND no = ?2",
        [task, no],
    )?;
    Ok(())
}

/// Removes every item of a task.
pub fn items_clear(c: &Connection, task: i64) -> rusqlite::Result<()> {
    c.execute("DELETE FROM gen_items WHERE task_id = ?1", [task])?;
    Ok(())
}

/// Removes the items of tasks that are no longer running (retention); returns how many rows went.
pub fn items_sweep(c: &Connection) -> rusqlite::Result<usize> {
    c.execute(
        "DELETE FROM gen_items WHERE task_id NOT IN (SELECT id FROM tasks WHERE state IN ('queued', 'running'))",
        [],
    )
}
