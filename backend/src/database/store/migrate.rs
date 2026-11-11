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
//!
//! Schema v2 → v3 (v0.6), in the same way: `meta.schema` `"2"` (or older): if the database holds any data, first write
//! `backups/xexam-schema2-<YYYYmmdd-HHMMSS>.db` (`xexam-schema1-…` when it was a v1 database: one backup of the state before this
//! open; the v1 → v2 → v3 steps then run in ONE transaction). The v3 step creates the user tables and `attempts.user_id`, creates
//! the default administrator `admin` / `admin` (must change the password) and lets it claim the existing data: every attempt
//! without a user, a copy of every `stats` row into `user_stats` and of every `settings` row into `user_settings` (the old tables
//! stay, unused). Every later open checks (read-only) whether there are no users at all, or exam records without a user (written
//! by an older binary after a rollback), and fixes that: a new default administrator, the records claimed by the first one.
//!
//! Schema v3 → v4 (v0.6.5 / v0.7), in the same way again: `meta.schema` `"3"` (or older): if the database holds any data,
//! first write `backups/xexam-schema3-<YYYYmmdd-HHMMSS>.db` (or `xexam-schema1-…` / `xexam-schema2-…`: always ONE backup, named
//! after the schema the file had before this open; every pending step then runs in ONE transaction). The v4 step adds
//! `user_banks`, `users.new_banks`, `users.can_manage_banks`, `banks.owner_id` (every existing bank is given to the first
//! administrator) and `attempts.kind` (every existing record is an exam, the column default). Nobody loses a bank at the
//! upgrade: every user that exists gets a `user_banks` row for every existing bank AND `new_banks = 1` (they saw every bank
//! and would have seen every later one).
//!
//! Pre-release v4 (v0.7 builds before its release): those stored `users.all_banks` (1 = every bank, including later ones)
//! instead of `users.new_banks`. A database stamped `"4"` that still has `users.all_banks` is converted on open, after the
//! same kind of backup (`backups/xexam-schema4-<YYYYmmdd-HHMMSS>.db`), in one transaction: a user with `all_banks = 1` gets a
//! row for every existing bank and `new_banks = 1`; a user with `all_banks = 0` keeps their rows, with `new_banks = 0`; then
//! the column `all_banks` is dropped.
//!
//! Every later open checks (read-only) and fixes, each in its own transaction, what only an older binary can leave behind
//! after a rollback (older versions know neither `user_banks` nor `users.new_banks`, and every user saw every bank there):
//! - banks it created: `meta.banks_known` holds the highest bank id this version knows (written by the migration and
//!   whenever this version creates a bank); a bank above it gets a row for every user with `new_banks = 1`, as if this
//!   version had created it (checked before anything else on open, so a bank this open creates cannot hide them);
//! - users it created: `meta.users_known` holds the highest user id this version knows; a user above it gets a row for
//!   every existing bank and `new_banks = 1`, like the users of the upgrade;
//! - banks without a valid owner, and `user_banks` rows of a user or bank that no longer exists.
//!
//! Without a mark (a database first opened by a build that did not write it) such rows cannot be told apart, so only the
//! mark is written.
use super::super::pyjson;
use super::super::schema::{
    MIGRATE_V2_INDEXES, MIGRATE_V2_TABLES, MIGRATE_V3_INDEXES, MIGRATE_V3_TABLES,
    MIGRATE_V4_INDEXES, MIGRATE_V4_TABLES, SCHEMA_VERSION, V2_COLUMNS, V3_COLUMNS, V4_COLUMNS,
};
use super::super::timefmt::{local_file_stamp, Clock};
use super::{banks, get_meta, run_tx, users};
use crate::error::DbError;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;
use std::path::{Path, PathBuf};

/// Directory (next to the database file) that receives the pre-migration backups.
pub const BACKUP_DIR: &str = "backups";

/// What [`upgrade`] did.
#[derive(Debug, Default)]
pub(super) struct Upgrade {
    /// The backup written before migrating, if any
    pub backup: Option<PathBuf>,
    /// The default administrator `admin` / `admin` was created
    pub created_admin: bool,
    /// A pre-release v4 database (`users.all_banks`) was converted to the released v4 shape (`users.new_banks`)
    pub converted_prerelease: bool,
}

/// Brings the database opened at `db_path` up to schema v4.
pub(super) fn upgrade(
    conn: &Connection,
    db_path: &Path,
    clock: &dyn Clock,
) -> Result<Upgrade, DbError> {
    let mut out = Upgrade::default();
    let from = match get_meta(conn, "schema")?.as_deref() {
        None | Some("1") => 1,
        Some("2") => 2,
        Some("3") => 3,
        // "4", or a newer schema written by a later version: nothing to migrate
        Some(_) => 4,
    };
    if from < 4 {
        if has_any_data(conn)? {
            out.backup = Some(write_backup(conn, db_path, from)?);
        }
        let now = clock.now();
        out.created_admin = run_tx(conn, |c| {
            if from < 2 {
                migrate_v2(c, &now)?;
            }
            let created = if from < 3 {
                migrate_v3(c, &now)?
            } else {
                false
            };
            migrate_v4(c)?;
            Ok(created)
        })?;
    } else if is_prerelease_v4(conn)? {
        if has_any_data(conn)? {
            out.backup = Some(write_backup(conn, db_path, 4)?);
        }
        run_tx(conn, convert_prerelease_v4)?;
        out.converted_prerelease = true;
    }
    // Before anything that may create a bank (the attach below): banks an older binary created are found by their id
    if needs_unknown_bank_grants(conn)? {
        run_tx(conn, grant_unknown_banks)?;
    }
    if needs_attach(conn)? {
        let now = clock.now();
        run_tx(conn, |c| attach_unassigned(c, &now).map(|_| ()))?;
    }
    if needs_owner_fixup(conn)? {
        let now = clock.now();
        out.created_admin |= run_tx(conn, |c| owner_fixup(c, &now))?;
    }
    if needs_bank_access_fixup(conn)? {
        run_tx(conn, bank_access_fixup)?;
    }
    Ok(out)
}

fn table_exists(c: &Connection, table: &str) -> Result<bool, DbError> {
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        params![table],
        |r| r.get::<_, bool>(0),
    )?)
}

/// Any row a user could lose: questions, exam records, stats, import history, banks, settings, accounts, or meta flags other than
/// the schema version (tables a v1 database does not have yet are skipped).
fn has_any_data(conn: &Connection) -> Result<bool, DbError> {
    for table in [
        "questions",
        "attempts",
        "stats",
        "imports",
        "banks",
        "attempt_items",
        "settings",
        "users",
        "user_banks",
    ] {
        if table_exists(conn, table)?
            && conn.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {table})"), [], |r| {
                r.get::<_, bool>(0)
            })?
        {
            return Ok(true);
        }
    }
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key <> 'schema')",
        [],
        |r| r.get::<_, bool>(0),
    )?)
}

/// `VACUUM INTO` a fresh file under `<dir of db_path>/backups/`. The target path is absolute, so it can never be taken for a
/// `file:` URI.
fn write_backup(conn: &Connection, db_path: &Path, schema: u32) -> Result<PathBuf, DbError> {
    let parent = db_path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let dir = std::path::absolute(parent.join(BACKUP_DIR))
        .map_err(|e| DbError::Backup(format!("cannot resolve the backup directory: {e}")))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| DbError::Backup(format!("cannot create {}: {}", dir.display(), e.kind())))?;
    let path = free_backup_path_for(&dir, schema, &local_file_stamp());
    let target = path
        .to_str()
        .ok_or_else(|| DbError::Backup("the backup path is not valid UTF-8".to_string()))?;
    conn.execute("VACUUM INTO ?1", params![target])
        .map_err(|e| DbError::Backup(format!("VACUUM INTO {}: {e}", path.display())))?;
    Ok(path)
}

/// `xexam-schema<schema>-<stamp>.db` (the schema the database had before the migration), or `…-2.db`, `…-3.db`, … when that
/// name is taken (by anything, even a dangling link).
pub fn free_backup_path_for(dir: &Path, schema: u32, stamp: &str) -> PathBuf {
    let mut n = 1u32;
    loop {
        let name = if n == 1 {
            format!("xexam-schema{schema}-{stamp}.db")
        } else {
            format!("xexam-schema{schema}-{stamp}-{n}.db")
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
    set_schema(c, "2")
}

fn set_schema(c: &Connection, version: &str) -> Result<(), DbError> {
    c.execute(
        "INSERT INTO meta(key,value) VALUES('schema', ?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![version],
    )?;
    Ok(())
}

/// The v3 DDL, the default administrator and the claim of the existing data (inside the caller's transaction). Returns whether
/// the default administrator was created (always, unless the database somehow already has users).
fn migrate_v3(c: &Connection, now: &str) -> Result<bool, DbError> {
    c.execute_batch(MIGRATE_V3_TABLES)?;
    for (table, column, definition) in V3_COLUMNS {
        if !has_column(c, table, column)? {
            c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {definition};"))?;
        }
    }
    c.execute_batch(MIGRATE_V3_INDEXES)?;
    let created = if users::has_users(c)? {
        false
    } else {
        let admin = users::insert_default_admin(c, now)?;
        c.execute(
            "INSERT OR IGNORE INTO user_stats(user_id, question_id, attempts, correct, wrong, streak, last, at) \
             SELECT ?1, question_id, attempts, correct, wrong, streak, last, at FROM stats ORDER BY rowid",
            params![admin],
        )?;
        c.execute(
            "INSERT OR IGNORE INTO user_settings(user_id, key, value) SELECT ?1, key, value FROM settings ORDER BY rowid",
            params![admin],
        )?;
        true
    };
    claim_orphan_attempts(c)?;
    set_schema(c, "3")?;
    Ok(created)
}

/// Gives every user a `user_banks` row for every existing bank (inside the caller's transaction); `only_above`: only the users
/// with a higher id.
fn every_bank_for_users(c: &Connection, only_above: Option<i64>) -> Result<(), DbError> {
    c.execute(
        "INSERT OR IGNORE INTO user_banks(user_id, bank_id) SELECT u.id, b.id FROM users u CROSS JOIN banks b \
         WHERE ?1 IS NULL OR u.id > ?1",
        params![only_above],
    )?;
    Ok(())
}

/// The v4 DDL and its backfill (inside the caller's transaction): every user that exists keeps seeing every bank (a row for
/// each, and `new_banks = 1` for the banks created later), every bank is owned by the first administrator. `attempts.kind`
/// needs no backfill: its default `'exam'` is what every older record is.
fn migrate_v4(c: &Connection) -> Result<(), DbError> {
    c.execute_batch(MIGRATE_V4_TABLES)?;
    for (table, column, definition) in V4_COLUMNS {
        if !has_column(c, table, column)? {
            c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {definition};"))?;
        }
    }
    c.execute_batch(MIGRATE_V4_INDEXES)?;
    every_bank_for_users(c, None)?;
    c.execute("UPDATE users SET new_banks=1", [])?;
    users::note_known_users(c)?;
    banks::note_known_banks(c)?;
    give_ownerless_banks(c)?;
    set_schema(c, SCHEMA_VERSION)
}

/// A v4 database written by a pre-release build of v0.7: it still has `users.all_banks` (read-only check).
fn is_prerelease_v4(c: &Connection) -> Result<bool, DbError> {
    Ok(table_exists(c, "users")? && has_column(c, "users", "all_banks")?)
}

/// Converts a pre-release v4 database (inside the caller's transaction, see the module documentation): `all_banks = 1` →
/// a row for every existing bank and `new_banks = 1`; `all_banks = 0` → `new_banks = 0`, rows kept; `all_banks` dropped.
fn convert_prerelease_v4(c: &Connection) -> Result<(), DbError> {
    c.execute_batch(MIGRATE_V4_TABLES)?;
    for (table, column, definition) in V4_COLUMNS {
        if !has_column(c, table, column)? {
            c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {definition};"))?;
        }
    }
    c.execute_batch(MIGRATE_V4_INDEXES)?;
    c.execute(
        "INSERT OR IGNORE INTO user_banks(user_id, bank_id) SELECT u.id, b.id FROM users u CROSS JOIN banks b \
         WHERE u.all_banks <> 0",
        [],
    )?;
    c.execute(
        "UPDATE users SET new_banks = CASE WHEN all_banks <> 0 THEN 1 ELSE 0 END",
        [],
    )?;
    c.execute_batch("ALTER TABLE users DROP COLUMN all_banks;")?;
    users::note_known_users(c)?;
    banks::note_known_banks(c)?;
    Ok(())
}

/// `meta.banks_known` (see [`banks::KNOWN_BANKS_KEY`]); `None` when it is missing or not a number.
fn known_banks(c: &Connection) -> Result<Option<i64>, DbError> {
    Ok(get_meta(c, banks::KNOWN_BANKS_KEY)?.and_then(|v| v.parse().ok()))
}

/// Read-only check run on every open of a v4 database: banks above `meta.banks_known` (created by an older binary after a
/// rollback), or no such mark yet.
fn needs_unknown_bank_grants(c: &Connection) -> Result<bool, DbError> {
    if !table_exists(c, "user_banks")?
        || !table_exists(c, "users")?
        || !has_column(c, "users", "new_banks")?
    {
        // A database of a newer, unknown schema without our tables: leave it alone
        return Ok(false);
    }
    match known_banks(c)? {
        None => Ok(true),
        Some(known) => Ok(c.query_row(
            "SELECT EXISTS(SELECT 1 FROM banks WHERE id > ?1)",
            params![known],
            |r| r.get(0),
        )?),
    }
}

/// The fix of [`needs_unknown_bank_grants`] (inside the caller's transaction): every user with `new_banks = 1` gets a row for
/// each bank above the mark, as if this version had created it; then the mark moves. Without a mark only the mark is written.
fn grant_unknown_banks(c: &Connection) -> Result<(), DbError> {
    if let Some(known) = known_banks(c)? {
        c.execute(
            "INSERT OR IGNORE INTO user_banks(user_id, bank_id) SELECT u.id, b.id FROM users u CROSS JOIN banks b \
             WHERE u.new_banks <> 0 AND b.id > ?1",
            params![known],
        )?;
    }
    banks::note_known_banks(c)
}

/// Gives every bank without a valid owner (NULL, or a user that no longer exists) to the first administrator (inside the
/// caller's transaction). Nothing happens when there is no administrator yet.
fn give_ownerless_banks(c: &Connection) -> Result<(), DbError> {
    if let Some(owner) = users::owner_admin(c)? {
        c.execute(
            "UPDATE banks SET owner_id=?1 WHERE owner_id IS NULL OR owner_id NOT IN (SELECT id FROM users)",
            params![owner],
        )?;
    }
    Ok(())
}

/// Read-only check run on every open of a v4 database: banks without a valid owner (while there is an administrator to give
/// them to), `user_banks` rows whose user or bank no longer exists, or users created by an older binary (above
/// `meta.users_known`, or no such mark yet).
fn needs_bank_access_fixup(c: &Connection) -> Result<bool, DbError> {
    if !table_exists(c, "user_banks")?
        || !has_column(c, "banks", "owner_id")?
        || !table_exists(c, "users")?
    {
        // A database of a newer, unknown schema without our tables: leave it alone
        return Ok(false);
    }
    let ownerless: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM banks WHERE owner_id IS NULL OR owner_id NOT IN (SELECT id FROM users))",
        [],
        |r| r.get(0),
    )?;
    if ownerless && users::owner_admin(c)?.is_some() {
        return Ok(true);
    }
    let newer_users: bool = match known_users(c)? {
        None => true,
        Some(known) => c.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id > ?1)",
            params![known],
            |r| r.get(0),
        )?,
    };
    if newer_users {
        return Ok(true);
    }
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM user_banks WHERE user_id NOT IN (SELECT id FROM users) \
         OR bank_id NOT IN (SELECT id FROM banks))",
        [],
        |r| r.get(0),
    )?)
}

/// `meta.users_known` (see [`users::KNOWN_USERS_KEY`]); `None` when it is missing or not a number.
fn known_users(c: &Connection) -> Result<Option<i64>, DbError> {
    Ok(get_meta(c, users::KNOWN_USERS_KEY)?.and_then(|v| v.parse().ok()))
}

/// The fix of [`needs_bank_access_fixup`] (inside the caller's transaction). Users created by an older binary after a
/// rollback get a row for every existing bank and `new_banks = 1`, like the users of the upgrade (every user saw every bank
/// there); without a mark (a database first opened by a build without it) they cannot be told apart, so only the mark is
/// written.
fn bank_access_fixup(c: &Connection) -> Result<(), DbError> {
    if let Some(known) = known_users(c)? {
        every_bank_for_users(c, Some(known))?;
        c.execute("UPDATE users SET new_banks=1 WHERE id > ?1", params![known])?;
    }
    users::note_known_users(c)?;
    give_ownerless_banks(c)?;
    c.execute(
        "DELETE FROM user_banks WHERE user_id NOT IN (SELECT id FROM users) OR bank_id NOT IN (SELECT id FROM banks)",
        [],
    )?;
    Ok(())
}

/// Gives every exam record without a user to the first administrator (inside the caller's transaction).
fn claim_orphan_attempts(c: &Connection) -> Result<(), DbError> {
    if let Some(owner) = users::owner_admin(c)? {
        c.execute(
            "UPDATE attempts SET user_id=?1 WHERE user_id IS NULL",
            params![owner],
        )?;
    }
    Ok(())
}

/// Read-only check run on every open of a v3 database: no users at all, or exam records without a user while there is an
/// administrator to give them to.
fn needs_owner_fixup(c: &Connection) -> Result<bool, DbError> {
    if !table_exists(c, "users")? || !has_column(c, "attempts", "user_id")? {
        // A database of a newer, unknown schema without our tables: leave it alone
        return Ok(false);
    }
    if !users::has_users(c)? {
        return Ok(true);
    }
    let orphans: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM attempts WHERE user_id IS NULL)",
        [],
        |r| r.get(0),
    )?;
    Ok(orphans && users::owner_admin(c)?.is_some())
}

/// Creates the default administrator when there are no users (the old `stats` / `settings` are NOT copied again: the migration
/// claimed them once), then gives exam records without a user to the first administrator. Returns whether the default
/// administrator was created.
fn owner_fixup(c: &Connection, now: &str) -> Result<bool, DbError> {
    let created = if users::has_users(c)? {
        false
    } else {
        users::insert_default_admin(c, now)?;
        true
    };
    claim_orphan_attempts(c)?;
    Ok(created)
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
        let a = free_backup_path_for(dir.path(), 1, "20260102-030405");
        assert_eq!(
            a.file_name().and_then(|n| n.to_str()),
            Some("xexam-schema1-20260102-030405.db")
        );
        std::fs::write(&a, b"x").expect("write");
        let b = free_backup_path_for(dir.path(), 1, "20260102-030405");
        assert_eq!(
            b.file_name().and_then(|n| n.to_str()),
            Some("xexam-schema1-20260102-030405-2.db")
        );
        std::fs::write(&b, b"").expect("write");
        let c = free_backup_path_for(dir.path(), 1, "20260102-030405");
        assert_eq!(
            c.file_name().and_then(|n| n.to_str()),
            Some("xexam-schema1-20260102-030405-3.db"),
            "an existing empty file is taken too"
        );
        let d = free_backup_path_for(dir.path(), 2, "20260102-030405");
        assert_eq!(
            d.file_name().and_then(|n| n.to_str()),
            Some("xexam-schema2-20260102-030405.db")
        );
    }
}
