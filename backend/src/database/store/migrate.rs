//! Schema v1 → v2 (v0.5), run by `Store::open`.
//!
//! - `meta.schema` missing or `"1"`: if the database holds any data, first write a consistent copy with `VACUUM INTO` to
//!   `<dir of the database>/backups/xexam-schema1-<YYYYmmdd-HHMMSS>.db` (never overwriting: `-2`, `-3`, … suffixes). If that fails,
//!   the open fails and nothing is migrated. Then, in ONE `BEGIN IMMEDIATE` transaction: create the v2 tables / columns / indexes,
//!   attach every existing question and import to a new "legacy" bank (question ids unchanged), give every exam record that bank as
//!   its `banks` snapshot, and set `meta.schema='2'`. A brand-new database goes through the same steps (without a backup).
//! - Every open: questions whose `bank_id` is NULL (written by an older binary after a rollback) are attached to the legacy bank,
//!   creating it if needed; so are import rows without a bank, once there is a legacy bank. Nothing is written when nothing needs
//!   fixing, so re-opening a v2 database leaves the file unchanged.
use super::super::pyjson;
use super::super::schema::{MIGRATE_V2_INDEXES, MIGRATE_V2_TABLES, SCHEMA_VERSION, V2_COLUMNS};
use super::super::timefmt::{local_file_stamp, Clock};
use super::{banks, get_meta, run_tx};
use crate::error::DbError;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;
use std::path::{Path, PathBuf};

/// Directory (next to the database file) that receives the pre-migration backups.
pub const BACKUP_DIR: &str = "backups";

/// Brings the database opened at `db_path` up to schema v2. Returns the backup written before migrating, if any.
pub(super) fn upgrade(
    conn: &Connection,
    db_path: &Path,
    clock: &dyn Clock,
) -> Result<Option<PathBuf>, DbError> {
    let mut backup = None;
    if matches!(get_meta(conn, "schema")?.as_deref(), None | Some("1")) {
        if has_any_data(conn)? {
            backup = Some(write_backup(conn, db_path)?);
        }
        let now = clock.now();
        run_tx(conn, |c| migrate_v2(c, &now))?;
    }
    if needs_attach(conn)? {
        let now = clock.now();
        run_tx(conn, |c| attach_unassigned(c, &now).map(|_| ()))?;
    }
    Ok(backup)
}

/// Any row a user could lose: questions, exam records, stats, import history, or meta flags other than the schema version.
fn has_any_data(conn: &Connection) -> Result<bool, DbError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM questions) OR EXISTS(SELECT 1 FROM attempts) OR EXISTS(SELECT 1 FROM stats) \
         OR EXISTS(SELECT 1 FROM imports) OR EXISTS(SELECT 1 FROM meta WHERE key <> 'schema')",
        [],
        |r| r.get::<_, bool>(0),
    )?)
}

/// `VACUUM INTO` a fresh file under `<dir of db_path>/backups/`. The target path is absolute, so it can never be taken for a
/// `file:` URI.
fn write_backup(conn: &Connection, db_path: &Path) -> Result<PathBuf, DbError> {
    let parent = db_path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let dir = std::path::absolute(parent.join(BACKUP_DIR))
        .map_err(|e| DbError::Backup(format!("cannot resolve the backup directory: {e}")))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| DbError::Backup(format!("cannot create {}: {}", dir.display(), e.kind())))?;
    let path = free_backup_path(&dir, &local_file_stamp());
    let target = path
        .to_str()
        .ok_or_else(|| DbError::Backup("the backup path is not valid UTF-8".to_string()))?;
    conn.execute("VACUUM INTO ?1", params![target])
        .map_err(|e| DbError::Backup(format!("VACUUM INTO {}: {e}", path.display())))?;
    Ok(path)
}

/// `xexam-schema1-<stamp>.db`, or `…-2.db`, `…-3.db`, … when that name is taken (by anything, even a dangling link).
pub fn free_backup_path(dir: &Path, stamp: &str) -> PathBuf {
    let mut n = 1u32;
    loop {
        let name = if n == 1 {
            format!("xexam-schema1-{stamp}.db")
        } else {
            format!("xexam-schema1-{stamp}-{n}.db")
        };
        let p = dir.join(name);
        if std::fs::symlink_metadata(&p).is_err() {
            return p;
        }
        n += 1;
    }
}

fn has_column(c: &Connection, table: &str, column: &str) -> Result<bool, DbError> {
    Ok(c.query_row(
        "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name=?2",
        params![table, column],
        |r| r.get::<_, i64>(0),
    )? > 0)
}

/// The v2 DDL plus the legacy-bank backfill (inside the caller's transaction).
fn migrate_v2(c: &Connection, now: &str) -> Result<(), DbError> {
    c.execute_batch(MIGRATE_V2_TABLES)?;
    for (table, column, definition) in V2_COLUMNS {
        if !has_column(c, table, column)? {
            c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {definition};"))?;
        }
    }
    c.execute_batch(MIGRATE_V2_INDEXES)?;
    if let Some((id, name)) = attach_unassigned(c, now)? {
        let snapshot = pyjson::to_string(&json!([{ "id": id, "name": name }]))?;
        c.execute("UPDATE attempts SET banks=?1", params![snapshot])?;
    }
    c.execute(
        "INSERT INTO meta(key,value) VALUES('schema', ?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![SCHEMA_VERSION],
    )?;
    Ok(())
}

/// Rows of `table` (`questions` or `imports`) without a bank.
fn count_unassigned(c: &Connection, table: &str) -> Result<i64, DbError> {
    Ok(c.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE bank_id IS NULL"),
        [],
        |r| r.get(0),
    )?)
}

/// Read-only check run on every open: questions without a bank (always fixed), or import rows without a bank while there is a
/// legacy bank to give them to (an older binary that re-imported an unchanged workbook writes only such an import row).
fn needs_attach(c: &Connection) -> Result<bool, DbError> {
    if count_unassigned(c, "questions")? > 0 {
        return Ok(true);
    }
    Ok(count_unassigned(c, "imports")? > 0 && banks::legacy_bank(c)?.is_some())
}

/// Attaches questions and import rows that have no bank to the legacy bank, creating it when there is none and there are
/// questions to attach. A created legacy bank is described by the unattached rows: `name` / `filename` from the latest
/// unattached import (or `"Question bank"` / `""`), `created_at` = earliest `added_at`, `updated_at` = that import's `at`
/// (or the latest `updated_at`). Returns the legacy bank's `(id, name)` when rows were attached to it.
fn attach_unassigned(c: &Connection, now: &str) -> Result<Option<(i64, String)>, DbError> {
    let unassigned = count_unassigned(c, "questions")?;
    let (id, name) = match banks::legacy_bank(c)? {
        Some(b) => (b.id, b.name),
        None if unassigned == 0 => return Ok(None),
        None => {
            let last: Option<(String, String)> = c
                .query_row(
                    "SELECT filename, at FROM imports WHERE bank_id IS NULL ORDER BY id DESC LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let (first_added, last_updated): (Option<String>, Option<String>) = c.query_row(
                "SELECT MIN(added_at), MAX(updated_at) FROM questions WHERE bank_id IS NULL",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let name = match &last {
                Some((filename, _)) => banks::name_from_filename(filename),
                None => String::new(),
            };
            let name = if name.is_empty() {
                banks::FALLBACK_BANK_NAME.to_string()
            } else {
                name
            };
            let filename = last.as_ref().map(|(f, _)| f.clone()).unwrap_or_default();
            let created_at = first_added.unwrap_or_else(|| now.to_string());
            let updated_at = last
                .map(|(_, at)| at)
                .or(last_updated)
                .unwrap_or_else(|| now.to_string());
            let id = banks::insert_bank(c, &name, &filename, true, &created_at, &updated_at)?;
            (id, name)
        }
    };
    c.execute(
        "UPDATE questions SET bank_id=?1 WHERE bank_id IS NULL",
        params![id],
    )?;
    c.execute(
        "UPDATE imports SET bank_id=?1 WHERE bank_id IS NULL",
        params![id],
    )?;
    Ok(Some((id, name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_names_never_collide() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = free_backup_path(dir.path(), "20260102-030405");
        assert_eq!(
            a.file_name().and_then(|n| n.to_str()),
            Some("xexam-schema1-20260102-030405.db")
        );
        std::fs::write(&a, b"x").expect("write");
        let b = free_backup_path(dir.path(), "20260102-030405");
        assert_eq!(
            b.file_name().and_then(|n| n.to_str()),
            Some("xexam-schema1-20260102-030405-2.db")
        );
        std::fs::write(&b, b"").expect("write");
        let c = free_backup_path(dir.path(), "20260102-030405");
        assert_eq!(
            c.file_name().and_then(|n| n.to_str()),
            Some("xexam-schema1-20260102-030405-3.db"),
            "an existing empty file is taken too"
        );
    }
}
