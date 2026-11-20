//! Schema v1 → v2 (v0.5), run by `Store::open` (the whole chain below ends at schema 11).
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
//! the column `all_banks` is dropped. `meta.users_known` is left as the pre-release build wrote it, so a user an older binary
//! created after the last pre-release open (above the mark) still gets every bank from the fix-up below.
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
//!
//! Schema v4 → v5 (v0.8), in the same way once more: `meta.schema` `"4"` (or older) gets ONE backup named after the schema
//! the file had (`backups/xexam-schema4-<YYYYmmdd-HHMMSS>.db` for a v4 database), then every pending step runs in ONE
//! transaction, the pre-release v4 conversion included (so a pre-release v4 database also gets exactly one backup). The v5
//! step adds `users.ai_enabled` (every user starts with AI+ off) and `users.uid` (a random identity stamp of 32 lower-case
//! hex digits for every user), then stamps the schema `"5"`. Every later open checks (read-only) for users with an empty
//! stamp (created by an older binary after a rollback: it does not know the column, so the default `''` stays) and gives
//! them one.
//!
//! Schema v5 → v6 (v0.9), in the same way again (`backups/xexam-schema5-<YYYYmmdd-HHMMSS>.db` for a v5 database; every
//! older database still gets its single backup named after the schema it had): the v6 step adds `attempts.session_key` and
//! `attempts.started_at` (both NULL for every existing record) and stamps the schema `"6"`. A v0.8 binary opens a v6 file
//! unchanged (it treats a newer schema as current and never reads the two columns), so a rollback needs no restore.
//!
//! Schema v6 → v7 (v1.1), in the same way once more (`backups/xexam-schema6-<YYYYmmdd-HHMMSS>.db` for a v6 database; every
//! older database still gets its single backup named after the schema it had): the v7 step adds `attempts.time_limit`,
//! `attempts.ended`, `banks.ai_job` and `banks.ai_meta` (each only when `pragma_table_info` lacks it; NULL for every
//! existing row), then the two indexes `idx_banks_ai_job` and `idx_questions_layout`, and stamps the schema `"7"`. A v1.0
//! binary opens a v7 file unchanged (it never reads the new columns; its own inserts leave them NULL, which the CHECKs
//! accept), so a rollback needs no restore, but an AI bank then looks like an imported one.
//!
//! Schema v7 → v8 (v1.3), in the same way once more (`backups/xexam-schema7-<YYYYmmdd-HHMMSS>.db` for a v7 database): the
//! v8 step adds the session and sign-in columns, `activity_log`, `system_settings` and the attempts index, sets
//! `sessions.started_at` to the upgrade time and `users.last_active_at` to `last_login_at`, stamps the schema `"8"` (the
//! `system.migrate` row is written once per upgrade, by the last step). A v1.2 binary opens a v8 file unchanged.
//!
//! Schema v8 → v9 (v1.4), in the same way (`backups/xexam-schema8-<YYYYmmdd-HHMMSS>.db` for a v8 database): the v9 step adds
//! the five `users.last_active_*` columns (NULL for every existing row), writes the ONE `system.migrate` activity row of the
//! upgrade and stamps the schema `"9"`. A v1.3 binary opens a v9 file unchanged.
//!
//! Schema v9 → v10 (v1.6), in the same way: the v10 step adds `users.can_download_banks` (0 for every existing row)
//! and stamps the schema `"10"`. A v1.5 binary opens a v10 file unchanged.
//!
//! Schema v10 → v11 (v2.0), the database clean-up: the same single backup (`backups/xexam-schema<N>-<time>.db`, written
//! before anything else), then ONE transaction with foreign keys switched off around it (a guard switches them on again on
//! every path and the setting is read back). The v11 step, in this order:
//! 1. Orphans, counted first, fixed by fixed rules: records of a missing user (items first), items of a missing record,
//!    sessions / per-user stats / per-user settings of a missing user, `user_banks` rows of a missing user or bank are
//!    DELETED (the backup holds them); a bank of a missing owner is cleared and given to the first administrator;
//!    questions / imports of a missing bank are detached (kept; the open attaches them to the legacy bank). Allowed NULLs
//!    (`attempts.user_id`, `questions.bank_id`, `imports.bank_id`, `banks.owner_id`) stay.
//! 2. The legacy `stats` / `settings` tables are dropped (their rows were copied in v0.6; counted, and in the backup).
//! 3. `idx_users_uid` (unique, non-empty stamps) is created; when stamps are shared it is skipped with a warning and no
//!    stamp is changed (a new stamp would make the AI service drop that user's AI data). A later open creates it once the
//!    duplicates are gone.
//! 4. The nine tables of `schema::V11_REBUILDS` are rebuilt (12-step procedure: new table, copy by explicit column list,
//!    drop, rename, indexes after the rename): an unknown column stops the upgrade, the row count must be equal, and the
//!    AUTOINCREMENT high-water mark is restored (ids are never reused).
//! 5. `PRAGMA foreign_key_check` must be empty, the one `system.migrate` activity row `{"from": N, "to": 11}` (plus
//!    `orphansRemoved` when rows were deleted; only when the file held data) is written and the schema is stamped `"11"`.
//!
//! Any surprise rolls the whole transaction back: the file is unchanged, the backup stays, `meta.schema` is unchanged and the
//! next start tries again (with a new backup). A v1.9 binary opens a v11 file as it is (it treats a newer schema as
//! current), but it re-creates an empty `stats` on every open; the every-open checks drop it again.
use super::super::pyjson;
use super::super::schema::{
    Rebuild, MIGRATE_V2_INDEXES, MIGRATE_V2_TABLES, MIGRATE_V3_INDEXES, MIGRATE_V3_TABLES,
    MIGRATE_V4_INDEXES, MIGRATE_V4_TABLES, MIGRATE_V7_INDEXES, MIGRATE_V8_INDEXES,
    MIGRATE_V8_TABLES, SCHEMA_VERSION, V10_COLUMNS, V11_REBUILDS, V11_USERS_UID_INDEX, V2_COLUMNS,
    V3_COLUMNS, V4_COLUMNS, V5_COLUMNS, V6_COLUMNS, V7_COLUMNS, V8_COLUMNS, V9_COLUMNS,
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
    /// What the v11 step and the every-open checks fixed, dropped or kept
    pub report: MigrationReport,
}

/// Brings the database opened at `db_path` up to schema v11.
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
        Some("4") => 4,
        Some("5") => 5,
        Some("6") => 6,
        Some("7") => 7,
        Some("8") => 8,
        Some("9") => 9,
        Some("10") => 10,
        // "11", or a newer schema written by a later version: nothing to migrate
        Some(_) => 11,
    };
    // Read-only check, before anything is written
    let pre = from == 4 && is_prerelease_v4(conn)?;
    if from < 11 || pre {
        // ONE backup of the state before this open, named after the schema the file had
        let had_data = has_any_data(conn)?;
        if had_data {
            out.backup = Some(write_backup(conn, db_path, from)?);
        }
        let now = clock.now();
        // Foreign keys are switched off OUTSIDE the transaction (the pragma is a no-op inside one) and always back on after it
        let (created, report) = with_foreign_keys_off(conn, |conn| {
            run_tx(conn, |c| {
                if from < 2 {
                    migrate_v2(c, &now)?;
                }
                let created = if from < 3 {
                    migrate_v3(c, &now)?
                } else {
                    false
                };
                if from < 4 {
                    migrate_v4(c)?;
                }
                if pre {
                    convert_prerelease_v4(c)?;
                }
                if from < 5 {
                    migrate_v5(c)?;
                }
                if from < 6 {
                    migrate_v6(c)?;
                }
                migrate_v7(c)?;
                if from < 8 {
                    migrate_v8(c)?;
                }
                migrate_v9(c)?;
                migrate_v10(c)?;
                let report = migrate_v11(c, from, had_data).map_err(stopped)?;
                Ok((created, report))
            })
        })?;
        out.created_admin = created;
        out.report = report;
        out.converted_prerelease = pre;
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
    // Last: users an older binary created after a rollback have no identity stamp yet
    if needs_uid_fixup(conn)? {
        run_tx(conn, fill_missing_uids)?;
    }
    post_checks(conn, &mut out.report)?;
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

pub(super) fn has_column(c: &Connection, table: &str, column: &str) -> Result<bool, DbError> {
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
/// needs no backfill: its default `'exam'` is what every older record is. The schema is stamped by [`migrate_v5`], which
/// always runs after it in the same transaction.
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
    give_ownerless_banks(c)
}

/// The v5 DDL and its backfill (inside the caller's transaction): `users.ai_enabled` (0 for everyone: AI+ starts off) and
/// `users.uid` (a random stamp for every user), then the schema version.
fn migrate_v5(c: &Connection) -> Result<(), DbError> {
    for (table, column, definition) in V5_COLUMNS {
        if !has_column(c, table, column)? {
            c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {definition};"))?;
        }
    }
    fill_missing_uids(c)?;
    set_schema(c, "5")
}

/// The v6 DDL (inside the caller's transaction): `attempts.session_key` and `attempts.started_at`, then the schema version.
/// Nothing to backfill: an older record has no session key.
fn migrate_v6(c: &Connection) -> Result<(), DbError> {
    for (table, column, definition) in V6_COLUMNS {
        if !has_column(c, table, column)? {
            c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {definition};"))?;
        }
    }
    set_schema(c, "6")
}

/// The v7 DDL (inside the caller's transaction): the four columns, then the two indexes (the unique one needs
/// `banks.ai_job`), then the schema version. Nothing to backfill: an older exam is untimed and has no `ended` (it keeps the
/// v1.0 rule), an older bank is an imported one.
fn migrate_v7(c: &Connection) -> Result<(), DbError> {
    for (table, column, definition) in V7_COLUMNS {
        if !has_column(c, table, column)? {
            c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {definition};"))?;
        }
    }
    c.execute_batch(MIGRATE_V7_INDEXES)?;
    set_schema(c, "7")
}

/// The v8 DDL and its backfill (inside the caller's transaction): the columns, the activity log and `system_settings`, the
/// attempts index, `sessions.started_at` = now and `users.last_active_at` = `last_login_at` for the rows that exist. No IP
/// or location is written here: the sign-in info switch starts off. The upgrade's activity row and the schema stamp are
/// written by [`migrate_v11`] (one row per upgrade).
fn migrate_v8(c: &Connection) -> Result<(), DbError> {
    for (table, column, definition) in V8_COLUMNS {
        if !has_column(c, table, column)? {
            c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {definition};"))?;
        }
    }
    c.execute_batch(MIGRATE_V8_TABLES)?;
    c.execute_batch(MIGRATE_V8_INDEXES)?;
    let unix = jiff::Timestamp::now().as_second();
    c.execute(
        "UPDATE sessions SET started_at=?1 WHERE started_at IS NULL",
        params![unix],
    )?;
    c.execute(
        "UPDATE users SET last_active_at=CASE WHEN length(last_login_at)=16 THEN last_login_at || ':00' ELSE last_login_at END 
         WHERE last_active_at IS NULL",
        [],
    )?;
    set_schema(c, "8")
}

/// The v9 DDL (inside the caller's transaction): the five `users.last_active_*` columns (each only when missing). Nothing is
/// backfilled. The upgrade's activity row and the final schema stamp are written by [`migrate_v11`].
fn migrate_v9(c: &Connection) -> Result<(), DbError> {
    for (table, column, definition) in V9_COLUMNS {
        if !has_column(c, table, column)? {
            c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {definition};"))?;
        }
    }
    set_schema(c, "9")
}

/// The v10 DDL (inside the caller's transaction): the `users.can_download_banks` column (only when missing), then the schema
/// version `"10"`. The upgrade's one activity row is written by [`migrate_v11`] (it used to be written here up to v1.9).
fn migrate_v10(c: &Connection) -> Result<(), DbError> {
    for (table, column, definition) in V10_COLUMNS {
        if !has_column(c, table, column)? {
            c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {definition};"))?;
        }
    }
    set_schema(c, "10")
}

/// Gives every user without an identity stamp a random one (32 lower-case hex digits; inside the caller's transaction).
fn fill_missing_uids(c: &Connection) -> Result<(), DbError> {
    c.execute(
        "UPDATE users SET uid = lower(hex(randomblob(16))) WHERE uid = ''",
        [],
    )?;
    Ok(())
}

/// Read-only check run on every open of a v5 database: users with an empty identity stamp (created by an older binary
/// after a rollback, which does not know `users.uid`).
fn needs_uid_fixup(c: &Connection) -> Result<bool, DbError> {
    if !table_exists(c, "users")? || !has_column(c, "users", "uid")? {
        // A database of a newer, unknown schema without our column: leave it alone
        return Ok(false);
    }
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE uid = '')",
        [],
        |r| r.get(0),
    )?)
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
    // `users_known` is not moved here: users above the pre-release mark were created by an older binary (with no access
    // rows and `all_banks = 0`); the bank access fix-up that runs next in the same open gives them every bank and
    // `new_banks = 1`, then moves the mark (or writes it when there is none)
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
            let id = banks::insert_bank(
                c,
                &name,
                &filename,
                true,
                &created_at,
                &updated_at,
                banks::NewBankShare::All,
            )?;
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

/// What the schema-11 step and the every-open checks of one open found. Every list holds only non-zero items; all of them
/// are empty for an up-to-date file. `main.rs` turns them into log lines.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    /// Orphaned rows fixed by the v11 step, keyed by relation (`attempts.user_id`, `attempt_items.attempt_id`,
    /// `sessions.user_id`, `user_stats.user_id`, `user_settings.user_id`, `user_banks`: rows deleted; `banks.owner_id`: owner
    /// cleared and given to the first administrator; `questions.bank_id`, `imports.bank_id`: detached from the missing bank,
    /// kept). Every deleted row is in the backup.
    pub orphans: Vec<(&'static str, i64)>,
    /// Legacy tables (`stats`, `settings`) with rows that the v11 step dropped, with their row count (already copied in v0.6;
    /// also in the backup). Empty legacy tables are dropped as well but not listed.
    pub legacy_rows: Vec<(&'static str, i64)>,
    /// Number of non-empty `users.uid` values shared by more than one user; the unique index is then not created (no uid is
    /// ever changed).
    pub uid_duplicates: i64,
    /// A legacy table that an older version re-created and that holds rows: kept (name, rows).
    pub kept_legacy: Vec<(&'static str, i64)>,
}

impl MigrationReport {
    /// The rows the v11 step deleted as orphans (what the `system.migrate` row calls `orphansRemoved`): detached questions /
    /// imports and re-owned banks are not deleted rows.
    pub fn orphans_removed(&self) -> i64 {
        self.orphans
            .iter()
            .filter(|(rel, _)| {
                !matches!(
                    *rel,
                    "banks.owner_id" | "questions.bank_id" | "imports.bank_id"
                )
            })
            .map(|(_, n)| n)
            .sum()
    }
}

/// Sets `PRAGMA foreign_keys=OFF`, runs `f`, and ALWAYS switches it on again (also when `f` fails or panics: the guard's `Drop`
/// does it). `PRAGMA foreign_keys` is a no-op inside a transaction, so this must be called with none open. Both switches are
/// read back: if the pragma did not take effect the upgrade stops instead of running with the wrong setting.
fn with_foreign_keys_off<T>(
    conn: &Connection,
    f: impl FnOnce(&Connection) -> Result<T, DbError>,
) -> Result<T, DbError> {
    struct Guard<'a>(&'a Connection);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            let _ = self.0.pragma_update(None, "foreign_keys", "ON");
        }
    }
    fn setting(c: &Connection) -> Result<i64, DbError> {
        Ok(c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?)
    }
    conn.pragma_update(None, "foreign_keys", "OFF")?;
    let result = {
        let _guard = Guard(conn);
        if setting(conn)? != 0 {
            return Err(DbError::Migration(
                "foreign keys could not be switched off".to_string(),
            ));
        }
        f(conn)
    };
    // The guard has switched them on again; read it back
    if setting(conn)? != 1 {
        return Err(DbError::Migration(
            "foreign keys could not be switched back on".to_string(),
        ));
    }
    result
}

fn count(c: &Connection, sql: &str) -> Result<i64, DbError> {
    Ok(c.query_row(sql, [], |r| r.get(0))?)
}

/// Runs a DELETE / UPDATE that must touch exactly `expected` rows (counted just before); anything else is a surprise and
/// stops the upgrade (the whole transaction rolls back).
fn exec_expect(c: &Connection, sql: &str, expected: i64, what: &str) -> Result<(), DbError> {
    let n = c.execute(sql, [])?;
    if i64::try_from(n).ok() != Some(expected) {
        return Err(DbError::Migration(format!(
            "{what}: expected {expected} rows, changed {n}"
        )));
    }
    Ok(())
}

const ORPHAN_ATTEMPTS: &str =
    "SELECT id FROM attempts WHERE user_id IS NOT NULL AND user_id NOT IN (SELECT id FROM users)";

/// Step a of the v11 step: fixes every orphaned row by the fixed rules (see the module documentation), counts first and
/// checks the number of changed rows. Allowed NULLs stay.
fn fix_orphans(c: &Connection) -> Result<Vec<(&'static str, i64)>, DbError> {
    let mut out: Vec<(&'static str, i64)> = Vec::new();
    let mut note = |rel: &'static str, n: i64| {
        if n > 0 {
            out.push((rel, n));
        }
    };
    // Records of a missing user: the items first, then the records; then items of a missing record
    let items_of_orphans = count(
        c,
        &format!("SELECT COUNT(*) FROM attempt_items WHERE attempt_id IN ({ORPHAN_ATTEMPTS})"),
    )?;
    let attempts = count(c, &format!("SELECT COUNT(*) FROM ({ORPHAN_ATTEMPTS})"))?;
    exec_expect(
        c,
        &format!("DELETE FROM attempt_items WHERE attempt_id IN ({ORPHAN_ATTEMPTS})"),
        items_of_orphans,
        "items of orphaned attempts",
    )?;
    exec_expect(
        c,
        &format!("DELETE FROM attempts WHERE id IN ({ORPHAN_ATTEMPTS})"),
        attempts,
        "orphaned attempts",
    )?;
    note("attempts.user_id", attempts);
    let items = count(
        c,
        "SELECT COUNT(*) FROM attempt_items WHERE attempt_id NOT IN (SELECT id FROM attempts)",
    )?;
    exec_expect(
        c,
        "DELETE FROM attempt_items WHERE attempt_id NOT IN (SELECT id FROM attempts)",
        items,
        "orphaned attempt items",
    )?;
    note("attempt_items.attempt_id", items_of_orphans + items);
    for (table, rel) in [
        ("sessions", "sessions.user_id"),
        ("user_stats", "user_stats.user_id"),
        ("user_settings", "user_settings.user_id"),
    ] {
        let cond = "user_id NOT IN (SELECT id FROM users)";
        let n = count(c, &format!("SELECT COUNT(*) FROM {table} WHERE {cond}"))?;
        exec_expect(c, &format!("DELETE FROM {table} WHERE {cond}"), n, rel)?;
        note(rel, n);
    }
    let cond = "user_id NOT IN (SELECT id FROM users) OR bank_id NOT IN (SELECT id FROM banks)";
    let n = count(c, &format!("SELECT COUNT(*) FROM user_banks WHERE {cond}"))?;
    exec_expect(
        c,
        &format!("DELETE FROM user_banks WHERE {cond}"),
        n,
        "user_banks",
    )?;
    note("user_banks", n);
    // Banks of a missing owner: cleared, then given to the first administrator (stays NULL when there is none; the every-open
    // owner fix-up handles that)
    let cond = "owner_id IS NOT NULL AND owner_id NOT IN (SELECT id FROM users)";
    let n = count(c, &format!("SELECT COUNT(*) FROM banks WHERE {cond}"))?;
    exec_expect(
        c,
        &format!("UPDATE banks SET owner_id=NULL WHERE {cond}"),
        n,
        "banks.owner_id",
    )?;
    give_ownerless_banks(c)?;
    note("banks.owner_id", n);
    // Content of a missing bank is kept, detached; the every-open `attach_unassigned` attaches it to the legacy bank
    for (table, rel) in [
        ("questions", "questions.bank_id"),
        ("imports", "imports.bank_id"),
    ] {
        let cond = "bank_id IS NOT NULL AND bank_id NOT IN (SELECT id FROM banks)";
        let n = count(c, &format!("SELECT COUNT(*) FROM {table} WHERE {cond}"))?;
        exec_expect(
            c,
            &format!("UPDATE {table} SET bank_id=NULL WHERE {cond}"),
            n,
            rel,
        )?;
        note(rel, n);
    }
    Ok(out)
}

/// Number of non-empty `users.uid` values that more than one user holds.
fn uid_duplicates(c: &Connection) -> Result<i64, DbError> {
    count(
        c,
        "SELECT COUNT(*) FROM (SELECT uid FROM users WHERE uid <> '' GROUP BY uid HAVING COUNT(*) > 1)",
    )
}

/// Rebuilds one table by the 12-step procedure (inside the caller's transaction, foreign keys off): refuses an unknown
/// column, copies by explicit column list, checks the row count, restores the AUTOINCREMENT high-water mark.
fn rebuild_table(c: &Connection, r: &Rebuild) -> Result<(), DbError> {
    let t = r.table;
    let known: Vec<&str> = r.columns.split(',').map(str::trim).collect();
    let mut st = c.prepare("SELECT name FROM pragma_table_info(?1)")?;
    let old: Vec<String> = st
        .query_map(params![t], |row| row.get::<_, String>(0))?
        .collect::<Result<_, _>>()?;
    for col in &old {
        if !known.contains(&col.as_str()) {
            return Err(DbError::Migration(format!("unexpected column {t}.{col}")));
        }
    }
    let n0 = count(c, &format!("SELECT COUNT(*) FROM {t}"))?;
    let s0: Option<i64> = c
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name=?1",
            params![t],
            |row| row.get(0),
        )
        .optional()?;
    let copied = c.execute_batch(r.create).and_then(|()| {
        c.execute_batch(&format!(
            "INSERT INTO new_{t}({cols}) SELECT {cols} FROM {t};
             DROP TABLE {t};
             ALTER TABLE new_{t} RENAME TO {t};",
            cols = r.columns
        ))
    });
    copied.map_err(|e| DbError::Migration(format!("rebuilding {t}: {e}")))?;
    if let Some(s0) = s0 {
        // The AUTOINCREMENT high-water mark: ids are never reused (`meta.banks_known` and the record ids rely on it)
        let changed = c.execute(
            "UPDATE sqlite_sequence SET seq=MAX(seq, ?2) WHERE name=?1",
            params![t, s0],
        )?;
        if changed == 0 {
            c.execute(
                "INSERT INTO sqlite_sequence(name, seq) VALUES(?1, ?2)",
                params![t, s0],
            )?;
        }
    }
    if !r.indexes.is_empty() {
        c.execute_batch(r.indexes)?;
    }
    if count(c, &format!("SELECT COUNT(*) FROM {t}"))? != n0 {
        return Err(DbError::Migration(format!("row count of {t} changed")));
    }
    Ok(())
}

/// Turns a SQLite error of the v11 step into [`DbError::Migration`] (so the start-up message names the stopped upgrade and
/// says that nothing was changed); other errors pass through.
fn stopped(e: DbError) -> DbError {
    match e {
        DbError::Sqlite(e) => DbError::Migration(e.to_string()),
        other => other,
    }
}

/// The v11 step (inside the caller's transaction, foreign keys off); see the module documentation. Returns what it fixed.
fn migrate_v11(c: &Connection, from: u32, had_data: bool) -> Result<MigrationReport, DbError> {
    let mut report = MigrationReport::default();
    // A trigger on a rebuilt table would be dropped silently with the old table: stop instead
    if let Some(name) = c
        .query_row(
            "SELECT name FROM sqlite_master WHERE type='trigger' LIMIT 1",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        return Err(DbError::Migration(format!("unexpected trigger {name}")));
    }
    // a. orphans
    report.orphans = fix_orphans(c)?;
    // b. legacy tables (their rows were copied in v0.6 / by `migrate_v3`; the backup holds them as well)
    for table in ["stats", "settings"] {
        if table_exists(c, table)? {
            let n = count(c, &format!("SELECT COUNT(*) FROM {table}"))?;
            // an empty table (every fresh file: the v1 text creates both) is dropped without a report line
            if n > 0 {
                report.legacy_rows.push((table, n));
            }
            c.execute_batch(&format!("DROP TABLE IF EXISTS {table};"))?;
        }
    }
    // c. the unique uid index, before the rebuilds (fixed `sqlite_master` order); duplicates: skipped, nothing changed
    report.uid_duplicates = uid_duplicates(c)?;
    if report.uid_duplicates == 0 {
        c.execute_batch(V11_USERS_UID_INDEX)?;
    }
    // d. the nine rebuilds
    for r in &V11_REBUILDS {
        rebuild_table(c, r)?;
    }
    // e. nothing may be left over
    let mut st = c.prepare("PRAGMA foreign_key_check")?;
    let bad: Vec<String> = st
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<_, _>>()?;
    if let Some(first) = bad.first() {
        let n = bad.iter().filter(|t| *t == first).count();
        return Err(DbError::Migration(format!(
            "foreign key check: {first} {n} rows"
        )));
    }
    drop(st);
    // f. the one activity row of the upgrade
    if had_data {
        let unix = jiff::Timestamp::now().as_second();
        let removed = report.orphans_removed();
        let detail = if removed > 0 {
            json!({ "from": from, "to": 11, "orphansRemoved": removed })
        } else {
            json!({ "from": from, "to": 11 })
        };
        c.execute(
            "INSERT INTO activity_log(ts, last_ts, actor, action, detail, result)              VALUES(?1, ?1, 'system', 'system.migrate', ?2, 'ok')",
            params![unix, detail.to_string()],
        )?;
    }
    // g.
    set_schema(c, SCHEMA_VERSION)?;
    Ok(report)
}

/// Whether `Store::open` must run the v1 `SCHEMA` text: the file is new, or `meta.schema` is missing or below 11. Read-only.
/// Running it on an up-to-date file would re-create the dropped `stats` table.
pub(super) fn needs_v1_schema(c: &Connection) -> Result<bool, DbError> {
    if !table_exists(c, "meta")? {
        return Ok(true);
    }
    Ok(!matches!(
        get_meta(c, "schema")?.and_then(|v| v.parse::<u32>().ok()),
        Some(n) if n >= 11
    ))
}

/// Every-open checks of a schema-11 file (read-only first; a write only when something needs fixing, so an up-to-date file
/// stays byte-identical):
/// - `stats` / `settings` re-created by an older version after a rollback (v1.9 runs the v1 `SCHEMA` on every open): dropped
///   when empty, kept (and reported) when they hold rows;
/// - `idx_users_uid` missing: created once no non-empty stamp is shared; while some are, the count is reported.
///
/// No full `foreign_key_check` here: its cost grows with `attempt_items`, and enforcement keeps new orphans out.
fn post_checks(c: &Connection, report: &mut MigrationReport) -> Result<(), DbError> {
    if get_meta(c, "schema")?.as_deref() != Some("11") {
        return Ok(());
    }
    for table in ["stats", "settings"] {
        if !table_exists(c, table)? {
            continue;
        }
        let n = count(c, &format!("SELECT COUNT(*) FROM {table}"))?;
        if n == 0 {
            run_tx(c, |c| {
                c.execute_batch(&format!("DROP TABLE IF EXISTS {table};"))?;
                Ok(())
            })?;
        } else {
            report.kept_legacy.push((table, n));
        }
    }
    if table_exists(c, "users")? && has_column(c, "users", "uid")? {
        let has_index: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name='idx_users_uid')",
            [],
            |r| r.get(0),
        )?;
        if !has_index {
            let dups = uid_duplicates(c)?;
            if dups == 0 {
                run_tx(c, |c| Ok(c.execute_batch(V11_USERS_UID_INDEX)?))?;
            } else {
                report.uid_duplicates = dups;
            }
        }
    }
    Ok(())
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
