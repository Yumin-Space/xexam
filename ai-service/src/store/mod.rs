//! The AI database (`<XEXAM_AI_DATA>/xexam-ai.db`, spec 2.2): one `Mutex<Connection>`, every write in
//! `BEGIN IMMEDIATE … COMMIT`, the lock never held across an `.await`.
//!
//! Deleted user content: `secure_delete=ON` zeroes freed pages, and every delete of user content schedules a
//! `PRAGMA wal_checkpoint(TRUNCATE)` (coalesced, at most one run per 10 s, see `app::spawn_checkpointer`) so the old page
//! images do not stay in `xexam-ai.db-wal`. Backups and volume snapshots keep deleted content (documented).
use crate::config::JournalMode;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::Notify;

pub mod conn;
pub mod data;
pub mod geo;
pub mod items;
pub mod jobs;
pub mod schema;
pub mod shared;
pub mod stats;
pub mod tables;

/// File name of the AI database.
pub const DB_FILE: &str = "xexam-ai.db";
/// Directory (inside the AI data directory) of the copies made before a schema upgrade.
pub const BACKUP_DIR: &str = "backups";

/// The AI database.
pub struct Store {
    conn: Mutex<Connection>,
    path: PathBuf,
    journal: String,
    checkpoint: Arc<Notify>,
    checkpoint_runs: AtomicU64,
}

fn create_dir_private(dir: &Path) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir)
}

fn apply_journal(conn: &Connection, want: JournalMode) -> String {
    match conn.pragma_update_and_check(None, "journal_mode", want.as_str(), |r| {
        r.get::<_, String>(0)
    }) {
        Ok(mode) => {
            let mode = mode.to_ascii_lowercase();
            if mode != want.as_str() {
                tracing::warn!(
                    "SQLite did not switch the AI database's journal mode to {}; continuing in mode {mode}",
                    want.as_str()
                );
            }
            mode
        }
        Err(e) => {
            tracing::warn!(
                "could not switch the AI database's journal mode to {}: {e}",
                want.as_str()
            );
            conn.pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))
                .map(|m| m.to_ascii_lowercase())
                .unwrap_or_else(|_| "unknown".to_string())
        }
    }
}

/// Rebuilds `model_slots` for schema 9 when it still allows the slot `study` (a CHECK cannot be altered): `study`
/// becomes `hint` and `explain` with the same model, time, author and effort. Then seeds the seven slots. The table is
/// referenced by nothing (it is a child of `models` only), so `foreign_keys` may stay ON during the rebuild.
fn split_study_slot(c: &Connection, now: i64) -> rusqlite::Result<()> {
    let old: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'model_slots' AND sql LIKE '%study%')",
        [],
        |r| r.get(0),
    )?;
    if old {
        let copy = |slot_expr: &str, filter: &str| {
            format!(
                "INSERT INTO model_slots (slot, model_id, updated_at, updated_by, effort)
                 SELECT {slot_expr}, model_id, updated_at, updated_by, effort FROM model_slots_v8 WHERE {filter};"
            )
        };
        c.execute_batch(&format!(
            "ALTER TABLE model_slots RENAME TO model_slots_v8; {}; {} {} {} DROP TABLE model_slots_v8;",
            schema::V9_MODEL_SLOTS,
            copy("slot", "slot <> 'study'"),
            copy("'hint'", "slot = 'study'"),
            copy("'explain'", "slot = 'study'"),
        ))?;
    }
    c.execute(schema::V9_SEED, [now])?;
    Ok(())
}

/// Adds the listed columns that the tables lack (the same step for new and upgraded databases).
fn add_columns(c: &Connection, cols: &[(&str, &str, &str)]) -> rusqlite::Result<()> {
    for (table, column, def) in cols {
        let present: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
            [table, column],
            |r| r.get(0),
        )?;
        if !present {
            c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {def};"))?;
        }
    }
    Ok(())
}

/// The same-version repair of [`add_columns`]: skips the retired columns (the history lists still name some of them, and
/// a schema-10 file must never get them back).
fn add_live_columns(c: &Connection, cols: &[(&str, &str, &str)]) -> rusqlite::Result<()> {
    let live: Vec<(&str, &str, &str)> = cols
        .iter()
        .filter(|(t, col, _)| !schema::is_retired(t, col))
        .copied()
        .collect();
    add_columns(c, &live)
}

/// A memory note may be as long as the memory length setting allows (up to 250,000 tokens, A-OWNER-43b), but the
/// schema-1 table limits `content` to 2000 characters. A CHECK cannot be altered, so the table is rebuilt once with
/// the wider bound (rows and ids kept). The same step for new and upgraded databases; a no-op when already wide.
fn relax_memory_check(c: &Connection) -> rusqlite::Result<()> {
    let old: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'memories' AND sql LIKE '%AND 2000)%')",
        [],
        |r| r.get(0),
    )?;
    if !old {
        return Ok(());
    }
    c.execute_batch(
        "ALTER TABLE memories RENAME TO memories_old;
         CREATE TABLE memories (
           id          INTEGER PRIMARY KEY AUTOINCREMENT,
           user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
           content     TEXT NOT NULL CHECK (length(content) BETWEEN 1 AND 2000000),
           source      TEXT NOT NULL DEFAULT 'manual' CHECK (source IN ('manual', 'chat')),
           created_at  INTEGER NOT NULL,
           updated_at  INTEGER NOT NULL
         );
         INSERT INTO memories (id, user_id, content, source, created_at, updated_at)
           SELECT id, user_id, content, source, created_at, updated_at FROM memories_old;
         DROP TABLE memories_old;
         CREATE INDEX IF NOT EXISTS idx_memories_user ON memories(user_id, id);",
    )
}

/// Every additive step of schemas 1 - 9 on an empty file, in the order of the upgrade: the tables, columns, indexes and
/// seed rows of a v1.9 file. No drop and no `meta.schema` stamp (the callers add what they need).
fn build_history(tx: &Transaction, now: i64) -> rusqlite::Result<()> {
    tx.execute_batch(schema::SCHEMA)?;
    add_columns(tx, &schema::V2_COLUMNS)?;
    add_columns(tx, &schema::V3_COLUMNS)?;
    add_columns(tx, &schema::V4_COLUMNS)?;
    add_columns(tx, &schema::V5_COLUMNS)?;
    add_columns(tx, &schema::V6_COLUMNS)?;
    add_columns(tx, &schema::V7_COLUMNS)?;
    relax_memory_check(tx)?;
    tx.execute_batch(schema::V3_INDEXES)?;
    tx.execute_batch(schema::V5_TABLES)?;
    add_columns(tx, &schema::LATE_COLUMNS)?;
    tx.execute_batch(schema::V7_TABLES)?;
    add_columns(tx, &schema::V16_COLUMNS)?;
    add_columns(tx, &schema::V9_COLUMNS)?;
    tx.execute_batch(schema::V7_INDEXES)?;
    tx.execute_batch(schema::V8_TABLES)?;
    tx.execute(schema::V7_SEED, [now])?;
    split_study_slot(tx, now)?;
    tx.execute(
        "INSERT OR IGNORE INTO settings (id, updated_at) VALUES (1, ?1)",
        [now],
    )?;
    tx.execute(schema::V7_NEW_LIMITS, [now])?;
    tx.execute_batch(schema::V4_MEMORY_DEFAULT)?;
    Ok(())
}

/// What the upgrade transaction reports for the log after the commit (counts and ids only).
struct UpgradeReport {
    secret_rows: i64,
    dropped: usize,
    orphans: i64,
    families: Vec<(i64, crate::effort::EffortFamily)>,
    cleared: usize,
    pace: Option<PaceChange>,
}

/// Pace values before and after the 10 -> 11 conversion: `((rpm, rpm'), (tpm, tpm'))`.
type PaceChange = ((i64, i64), (i64, i64));

/// The 10 -> 11 pace conversion (v2.1 uses `paceRpm` / `paceTpm` as entered): a value still on the old default
/// (15 / 250000) becomes the new default (13 / 240000); any other value becomes floor(value * 0.9), at least the
/// column minimum, so the actual pace of v2.0 stays the same.
fn convert_pace(c: &Connection) -> rusqlite::Result<Option<PaceChange>> {
    let old: Option<(i64, i64)> = c
        .query_row("SELECT pace_rpm, pace_tpm FROM limits LIMIT 1", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    let Some((rpm, tpm)) = old else {
        return Ok(None);
    };
    let rpm2 = if rpm == 15 { 13 } else { (rpm * 9 / 10).max(1) };
    let tpm2 = if tpm == 250_000 {
        240_000
    } else {
        (tpm * 9 / 10).max(1000)
    };
    c.execute(
        "UPDATE limits SET pace_rpm = ?1, pace_tpm = ?2",
        [rpm2, tpm2],
    )?;
    Ok(Some(((rpm, rpm2), (tpm, tpm2))))
}

/// An error of the schema-10 step that is not a SQLite error.
fn step_error(msg: String) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(msg.into())
}

fn table_exists(c: &Connection, name: &str) -> rusqlite::Result<bool> {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [name],
        |r| r.get(0),
    )
}

fn count_rows(c: &Connection, table: &str) -> rusqlite::Result<i64> {
    c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
}

/// The schema-10 step: drops the retired `secrets` table (the keys live in `connection_secrets` since schema 7) and the
/// retired columns of `settings`, `limits` and `chats` (only those present). Returns (rows of `secrets`, columns
/// dropped). Counts only; no value is read. `settings` and `limits` must keep their row count, else the error rolls the
/// transaction back.
fn drop_retired(c: &Connection) -> rusqlite::Result<(i64, usize)> {
    let secret_rows = if table_exists(c, "secrets")? {
        count_rows(c, "secrets")?
    } else {
        0
    };
    let settings0 = count_rows(c, "settings")?;
    let limits0 = count_rows(c, "limits")?;
    let mut dropped = 0usize;
    for (table, column) in schema::RETIRED_COLUMNS {
        let present: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
            [table, column],
            |r| r.get(0),
        )?;
        if present {
            c.execute_batch(&format!("ALTER TABLE {table} DROP COLUMN {column};"))?;
            dropped += 1;
        }
    }
    for table in schema::RETIRED_TABLES {
        c.execute_batch(&format!("DROP TABLE IF EXISTS {table};"))?;
    }
    for (table, before) in [("settings", settings0), ("limits", limits0)] {
        let after = count_rows(c, table)?;
        if after != before {
            return Err(step_error(format!(
                "the row count of {table} changed ({before} -> {after})"
            )));
        }
    }
    Ok((secret_rows, dropped))
}

/// A file shaped like a v1.9 one (schema 9, with the retired items), for tests of the upgrade.
#[doc(hidden)]
pub fn write_schema9_fixture(path: &Path) -> rusqlite::Result<()> {
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    build_history(&tx, crate::util::unix_now())?;
    // what the v1.9 open added after its schema stamp: the two one-time flags
    tx.execute_batch(
        "INSERT INTO meta (key, value) VALUES ('schema', '9');
         INSERT INTO meta (key, value) VALUES ('unlimited_default', '1');
         INSERT INTO meta (key, value) VALUES ('import_max_50', '1');",
    )?;
    tx.commit()
}

/// Copies the whole database (`VACUUM INTO`) to `<backups>/xexam-ai-schema<from>-<YYYYMMDD-HHMMSS>.db` (local time;
/// `-2`, `-3`, ... when the name is taken) and returns the path. The directory is created private (0700).
fn backup_copy(conn: &Connection, backups: &Path, from: &str) -> Result<PathBuf, String> {
    create_dir_private(backups).map_err(|e| format!("{}: {}", backups.display(), e.kind()))?;
    let stamp = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
    let base = format!("xexam-ai-schema{from}-{stamp}");
    let mut n = 1u32;
    let target = loop {
        let name = if n == 1 {
            format!("{base}.db")
        } else {
            format!("{base}-{n}.db")
        };
        let p = backups.join(name);
        if !p.exists() {
            break p;
        }
        n += 1;
        if n > 1000 {
            return Err(format!("{}: no free file name", backups.display()));
        }
    };
    let text = target
        .to_str()
        .ok_or_else(|| format!("{}: not a UTF-8 path", target.display()))?;
    conn.execute("VACUUM INTO ?1", [text])
        .map_err(|e| format!("{}: {e}", target.display()))?;
    Ok(target)
}

/// Host part of a base URL, cut to 40 characters; `connection-1` when there is none.
fn connection_name_of(base_url: &str) -> String {
    let rest = base_url.split_once("://").map_or(base_url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = if let Some(inner) = host_port.strip_prefix('[') {
        inner.split(']').next().unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    let name: String = host.trim().chars().take(40).collect();
    if name.is_empty() {
        "connection-1".to_string()
    } else {
        name
    }
}

/// The old `settings` row as far as the migration needs it.
struct OldSettings {
    provider: String,
    base_url: String,
    format: String,
    gateway_auth: String,
    model: String,
    manual: i64,
    usage_option: i64,
    max_tokens: i64,
    verified_at: Option<i64>,
    verified_fp: Option<String>,
    updated_by: String,
}

/// Steps 2 - 5 of the 6 -> 7 migration (inside the upgrade transaction): connection 1 from the old `settings` row, only
/// when it was ever configured; its keys copied as they are (`aad_ver` 1, no master key needed); the model and the
/// `default` slot. `settings.enabled` stays as it is; `usage` / `gen_calls` rows keep a NULL connection.
fn connect_old_settings(tx: &Transaction, now: i64) -> rusqlite::Result<()> {
    let configured: Option<bool> = tx
        .query_row(
            "SELECT updated_by <> '' OR model <> '' OR EXISTS(SELECT 1 FROM secrets) FROM settings WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if configured != Some(true) {
        return Ok(());
    }
    let o = tx.query_row(
        "SELECT provider, base_url, format, gateway_auth, model, manual_model, usage_option, max_tokens,
                verified_at, verified_fp, updated_by FROM settings WHERE id = 1",
        [],
        |r| {
            Ok(OldSettings {
                provider: r.get(0)?,
                base_url: r.get(1)?,
                format: r.get(2)?,
                gateway_auth: r.get(3)?,
                model: r.get(4)?,
                manual: r.get(5)?,
                usage_option: r.get(6)?,
                max_tokens: r.get(7)?,
                verified_at: r.get(8)?,
                verified_fp: r.get(9)?,
                updated_by: r.get(10)?,
            })
        },
    )?;
    tx.execute(
        "INSERT INTO connections (id, name, provider, base_url, format, gateway_auth, status, tested_at, rev, sort,
                                  created_at, updated_at, updated_by)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, 'ok', ?6, 1, 0, ?7, ?7, ?8)",
        params![
            connection_name_of(&o.base_url),
            o.provider,
            o.base_url,
            o.format,
            o.gateway_auth,
            o.verified_at,
            now,
            o.updated_by
        ],
    )?;
    tx.execute(
        "INSERT INTO connection_secrets (connection_id, name, key_id, nonce, ciphertext, hint, rev, aad_ver, updated_at)
         SELECT 1, name, key_id, nonce, ciphertext, hint, rev, 1, updated_at FROM secrets",
        [],
    )?;
    if !o.model.is_empty() {
        tx.execute(
            "INSERT INTO models (connection_id, model, manual, status, usage_option, max_tokens, tested_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                o.model,
                o.manual,
                if o.verified_fp.is_some() { "ok" } else { "untested" },
                o.usage_option,
                o.max_tokens,
                o.verified_at
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "UPDATE model_slots SET model_id = ?1, updated_at = ?2 WHERE slot = 'default'",
            params![id, now],
        )?;
    }
    Ok(())
}

impl Store {
    /// Opens (and creates) the AI database in `dir`. A database written by a newer AI service is refused.
    pub fn open(dir: &Path, journal: JournalMode) -> Result<Store, String> {
        create_dir_private(dir).map_err(|e| {
            format!(
                "cannot create the AI data directory {}: {}",
                dir.display(),
                e.kind()
            )
        })?;
        let path = dir.join(DB_FILE);
        let conn = Connection::open(&path)
            .map_err(|e| format!("cannot open the AI database {}: {e}", path.display()))?;
        let fail =
            |e: rusqlite::Error| format!("cannot open the AI database {}: {e}", path.display());
        conn.busy_timeout(std::time::Duration::from_millis(5000))
            .map_err(fail)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(fail)?;
        conn.pragma_update(None, "secure_delete", "ON")
            .map_err(fail)?;
        conn.pragma_update(None, "synchronous", "FULL")
            .map_err(fail)?;
        let mode = apply_journal(&conn, journal);
        let store = Store {
            conn: Mutex::new(conn),
            path: path.clone(),
            journal: mode,
            checkpoint: Arc::new(Notify::new()),
            checkpoint_runs: AtomicU64::new(0),
        };
        store
            .init(&dir.join(BACKUP_DIR))
            .map_err(|e| e.replace("{path}", &path.display().to_string()))?;
        Ok(store)
    }

    /// Creates a new database, upgrades a schema-1 … 9 database (after a backup copy), or refuses an unknown schema; then
    /// repairs answers left `streaming` by a crash. Reopening a current database writes nothing.
    fn init(&self, backups: &Path) -> Result<(), String> {
        let mut conn = self.lock();
        let existing: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
                r.get(0)
            })
            .optional()
            .unwrap_or(None);
        let now = crate::util::unix_now();
        match existing.as_deref() {
            None => {
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|e| format!("cannot open the AI database {{path}}: {e}"))?;
                let run = |tx: &Transaction| -> rusqlite::Result<()> {
                    build_history(tx, now)?;
                    drop_retired(tx)?;
                    add_columns(tx, &schema::V21_COLUMNS)?;
                    tx.execute(
                        "INSERT OR IGNORE INTO meta (key, value) VALUES ('schema', ?1)",
                        [schema::SCHEMA_VERSION],
                    )?;
                    Ok(())
                };
                run(&tx).map_err(|e| format!("cannot create the AI database {{path}}: {e}"))?;
                tx.commit()
                    .map_err(|e| format!("cannot create the AI database {{path}}: {e}"))?;
            }
            // The current schema: nothing to do, nothing written, except that a file may lack a column added later: add it (idempotent, no version change)
            Some(v) if v == schema::SCHEMA_VERSION => {
                let missing = |c: &Connection| -> rusqlite::Result<bool> {
                    for (table, column, _) in schema::V5_COLUMNS
                        .iter()
                        .chain(schema::V6_COLUMNS.iter())
                        .chain(schema::V7_COLUMNS.iter())
                        .chain(schema::LATE_COLUMNS.iter())
                        .chain(schema::V16_COLUMNS.iter())
                        .chain(schema::V9_COLUMNS.iter())
                        .chain(schema::V21_COLUMNS.iter())
                        .filter(|(t, col, _)| !schema::is_retired(t, col))
                    {
                        let present: bool = c.query_row(
                            "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
                            [table, column],
                            |r| r.get(0),
                        )?;
                        if !present {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                };
                if missing(&conn).unwrap_or(false) {
                    let tx = conn
                        .transaction_with_behavior(TransactionBehavior::Immediate)
                        .map_err(|e| format!("cannot open the AI database {{path}}: {e}"))?;
                    add_live_columns(&tx, &schema::V5_COLUMNS)
                        .and_then(|()| add_live_columns(&tx, &schema::V6_COLUMNS))
                        .and_then(|()| add_live_columns(&tx, &schema::V7_COLUMNS))
                        .and_then(|()| add_live_columns(&tx, &schema::LATE_COLUMNS))
                        .and_then(|()| tx.execute_batch(schema::V7_TABLES))
                        .and_then(|()| add_live_columns(&tx, &schema::V16_COLUMNS))
                        .and_then(|()| add_live_columns(&tx, &schema::V9_COLUMNS))
                        .and_then(|()| add_live_columns(&tx, &schema::V21_COLUMNS))
                        .and_then(|()| tx.execute_batch(schema::V7_INDEXES))
                        .and_then(|()| tx.execute_batch(schema::V8_TABLES))
                        .map_err(|e| format!("cannot upgrade the AI database {{path}}: {e}"))?;
                    tx.commit()
                        .map_err(|e| format!("cannot upgrade the AI database {{path}}: {e}"))?;
                }
                // a file written by a v1.3 build from before the per-user hint + analysis switch became one (the global
                // bits are independent since v1.9 and are never touched)
                let split: bool = conn
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM users WHERE (features_off & 6) IN (2, 4))",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(|e| format!("cannot read the AI database {{path}}: {e}"))?;
                if split {
                    conn.execute_batch(schema::PAIR_USER_STUDY_BITS)
                        .map_err(|e| format!("cannot upgrade the AI database {{path}}: {e}"))?;
                }
                // a schema-7 file written by a v1.3 build that kept the chat picks of a removed connection
                let stale: bool = conn
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM chats WHERE pick_conn IS NOT NULL
                             AND NOT EXISTS (SELECT 1 FROM connections k WHERE k.id = chats.pick_conn))",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(|e| format!("cannot read the AI database {{path}}: {e}"))?;
                if stale {
                    conn.execute_batch(schema::CLEAR_STALE_PICKS)
                        .map_err(|e| format!("cannot upgrade the AI database {{path}}: {e}"))?;
                }
            }
            Some(v) if schema::UPGRADABLE.contains(&v) => {
                let copy = backup_copy(&conn, backups, v).map_err(|e| {
                    format!("cannot back up the AI database before the upgrade ({e}); nothing was changed")
                })?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|e| format!("cannot upgrade the AI database {{path}}: {e}"))?;
                let run = |tx: &Transaction| -> rusqlite::Result<UpgradeReport> {
                    let from: u32 = v.parse().unwrap_or(0);
                    // adds what a '1' … '4' file lacks; a no-op for the rest. A schema-10 file already dropped its retired
                    // columns: they must not come back
                    let add = |tx: &Transaction, cols: &[(&str, &str, &str)]| {
                        if from >= 10 {
                            add_live_columns(tx, cols)
                        } else {
                            add_columns(tx, cols)
                        }
                    };
                    add(tx, &schema::V2_COLUMNS)?;
                    add(tx, &schema::V3_COLUMNS)?;
                    add(tx, &schema::V4_COLUMNS)?;
                    add(tx, &schema::V5_COLUMNS)?;
                    add(tx, &schema::V6_COLUMNS)?;
                    add(tx, &schema::V7_COLUMNS)?;
                    relax_memory_check(tx)?;
                    tx.execute_batch(schema::V3_INDEXES)?;
                    tx.execute_batch(schema::V5_TABLES)?;
                    add(tx, &schema::LATE_COLUMNS)?;
                    tx.execute_batch(schema::V7_TABLES)?;
                    add(tx, &schema::V16_COLUMNS)?;
                    add(tx, &schema::V9_COLUMNS)?;
                    add_columns(tx, &schema::V21_COLUMNS)?;
                    tx.execute_batch(schema::V7_INDEXES)?;
                    tx.execute_batch(schema::V8_TABLES)?;
                    tx.execute(schema::V7_SEED, [now])?;
                    // the character -> token conversion belongs to the 1 .. 5 -> 6 step only (a schema-6 file holds tokens)
                    if from < 6 {
                        tx.execute_batch(schema::V4_MEMORY_DEFAULT)?;
                        tx.execute_batch(schema::V6_MIGRATE)?;
                    }
                    // the 1 .. 6 -> 7 steps; a schema-7 or newer file already has them
                    if from < 7 {
                        tx.execute_batch(schema::V7_MIGRATE)?;
                        tx.execute_batch(schema::PAIR_STUDY_BITS)?;
                        tx.execute_batch(schema::CLEAR_STALE_PICKS)?;
                        connect_old_settings(tx, now)?;
                    } else {
                        tx.execute_batch(schema::PAIR_USER_STUDY_BITS)?;
                        tx.execute_batch(schema::CLEAR_STALE_PICKS)?;
                    }
                    split_study_slot(tx, now)?;
                    // every reader of the retired items ran above; now they go (the backup holds them)
                    let (secret_rows, dropped) = if from < 10 { drop_retired(tx)? } else { (0, 0) };
                    // the 10 -> 11 step (the number-based guard: a schema-11 file never gets here)
                    let (families, cleared) = if from < 11 {
                        conn::infer_families(tx)?
                    } else {
                        (Vec::new(), 0)
                    };
                    // the pace values lose the hidden 90 % margin: a schema-9 or -10 file keeps the same actual pace
                    // (older files just got the new column defaults, so they need no conversion)
                    let pace = if (9..11).contains(&from) {
                        convert_pace(tx)?
                    } else {
                        None
                    };
                    // informational only: the AI foreign keys have been enforced since v0.8, so nothing new appears
                    let orphans = {
                        let mut st = tx.prepare("PRAGMA foreign_key_check")?;
                        let rows = st.query_map([], |_| Ok(()))?;
                        rows.count() as i64
                    };
                    tx.execute(
                        "UPDATE meta SET value = ?1 WHERE key = 'schema'",
                        [schema::SCHEMA_VERSION],
                    )?;
                    Ok(UpgradeReport {
                        secret_rows,
                        dropped,
                        orphans,
                        families,
                        cleared,
                        pace,
                    })
                };
                let UpgradeReport {
                    secret_rows,
                    dropped,
                    orphans,
                    families,
                    cleared,
                    pace,
                } = run(&tx)
                    .map_err(|e| format!("cannot upgrade the AI database {{path}}: {e}"))?;
                tx.commit()
                    .map_err(|e| format!("cannot upgrade the AI database {{path}}: {e}"))?;
                tracing::info!(
                    "Upgraded the AI database to schema {}. A copy of the previous version was saved first: {}.",
                    schema::SCHEMA_VERSION,
                    copy.display()
                );
                // only the step to schema 10 removes legacy items; a schema-10 file has none left to report
                if v.parse::<u32>().unwrap_or(0) < 10 {
                    tracing::info!(
                        "Removed the unused legacy table secrets ({secret_rows} rows) and {dropped} unused columns (copy in the backup)."
                    );
                }
                for (id, family) in &families {
                    tracing::info!(
                        "Connection {id}: thinking-strength family set to {}.",
                        family.as_str()
                    );
                }
                if let Some(((r0, r1), (t0, t1))) = pace {
                    tracing::info!(
                        "Provider pacing is now used as entered: requests per minute {r0} -> {r1}, tokens per minute {t0} -> {t1}."
                    );
                }
                if cleared > 0 {
                    tracing::info!(
                        "{cleared} stored thinking strengths of another family were reset to the model default."
                    );
                }
                if orphans > 0 {
                    tracing::warn!(
                        "The AI database holds {orphans} rows that break a foreign key (kept as they are)."
                    );
                }
                // the freed `secrets` pages were zeroed (secure_delete); let them leave the -wal file too
                if self.journal == "wal" {
                    let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
                }
            }
            Some(v) => {
                return Err(format!(
                    "The AI database {{path}} has schema {v}, written by a newer ai-service; this version (schema {}) cannot open it. Run the newer version, or restore a backup.",
                    schema::SCHEMA_VERSION
                ));
            }
        }
        // A-OWNER-48: the two "unlimited" switches default to on. A schema-6 file written before that keeps its stored 0 until
        // this one-time step (after a backup copy) moves it; every other path already has the new column default.
        let flagged: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM meta WHERE key = ?1)",
                [schema::UNLIMITED_DEFAULT_KEY],
                |r| r.get(0),
            )
            .map_err(|e| format!("cannot read the AI database {{path}}: {e}"))?;
        if !flagged {
            let existing_num: Option<u32> = existing.as_deref().and_then(|v| v.parse().ok());
            if matches!(existing_num, Some(n) if n >= 7) {
                let copy = backup_copy(&conn, backups, schema::SCHEMA_VERSION).map_err(|e| {
                    format!("cannot back up the AI database before the upgrade ({e}); nothing was changed")
                })?;
                conn.execute_batch(schema::UNLIMITED_DEFAULT_SET)
                    .map_err(|e| format!("cannot upgrade the AI database {{path}}: {e}"))?;
                tracing::info!(
                    "The AI call ceilings now default to unlimited. A copy of the previous database was saved first: {}.",
                    copy.display()
                );
            }
            conn.execute(
                "INSERT OR IGNORE INTO meta (key, value) VALUES (?1, '1')",
                [schema::UNLIMITED_DEFAULT_KEY],
            )
            .map_err(|e| format!("cannot upgrade the AI database {{path}}: {e}"))?;
        }
        // v1.9: the Gemma thinking family is gone; its stored choices become the Google ones (same wire request)
        let gemma: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM model_slots WHERE substr(effort, 1, 6) = 'gemma:'",
                [],
                |r| r.get(0),
            )
            .map_err(|e| format!("cannot read the AI database {{path}}: {e}"))?;
        if gemma > 0 {
            conn.execute_batch(schema::FOLD_GEMMA_EFFORT)
                .map_err(|e| format!("cannot upgrade the AI database {{path}}: {e}"))?;
            tracing::info!(
                "moved {gemma} thinking-strength choices from the removed Gemma family to Google (same request)"
            );
        }
        // Answers left 'streaming' by a crash (check first: reopening a clean database writes nothing)
        let left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE status = 'streaming'",
                [],
                |r| r.get(0),
            )
            .map_err(|e| format!("cannot read the AI database {{path}}: {e}"))?;
        if left > 0 {
            conn.execute(
                "UPDATE messages SET status = 'error', error_code = 'ai.interrupted', finished_at = ?1
                 WHERE status = 'streaming'",
                [now],
            )
            .map_err(|e| format!("cannot repair the AI database {{path}}: {e}"))?;
            tracing::info!("marked {left} AI answers cut off by a restart as interrupted");
        }
        Ok(())
    }

    /// Closes the database file even while clones of the service still hold this store (tests on Windows: a
    /// `TempDir` cannot be removed while the file is open). Checkpoints the WAL, then swaps the connection for an
    /// in-memory one; later calls fail with SQLite errors instead of touching the file.
    #[doc(hidden)]
    pub fn release_file(&self) {
        let mut conn = self.lock();
        if self.journal == "wal" {
            let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
        }
        if let Ok(mem) = Connection::open_in_memory() {
            let old = std::mem::replace(&mut *conn, mem);
            drop(old);
        }
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The journal mode in effect (lower case).
    pub fn journal_mode(&self) -> &str {
        &self.journal
    }

    /// Runs a read.
    pub fn read<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> rusqlite::Result<T> {
        f(&self.lock())
    }

    /// Runs `f` in one `BEGIN IMMEDIATE … COMMIT` transaction (rolled back when `f` fails).
    pub fn write<T, E: From<rusqlite::Error>>(
        &self,
        f: impl FnOnce(&Transaction) -> Result<T, E>,
    ) -> Result<T, E> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    /// Asks for a `wal_checkpoint(TRUNCATE)` soon (coalesced by the checkpoint task).
    pub fn schedule_checkpoint(&self) {
        self.checkpoint.notify_one();
    }

    /// The signal the checkpoint task waits for.
    pub fn checkpoint_signal(&self) -> Arc<Notify> {
        self.checkpoint.clone()
    }

    /// Runs `PRAGMA wal_checkpoint(TRUNCATE)` now; `Ok(true)` when it could not finish (busy).
    pub fn checkpoint_now(&self) -> rusqlite::Result<bool> {
        self.checkpoint_runs.fetch_add(1, Ordering::Relaxed);
        let conn = self.lock();
        if self.journal != "wal" {
            return Ok(false);
        }
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            Ok(r.get::<_, i64>(0)? != 0)
        })
    }

    /// How many checkpoints ran (tests).
    pub fn checkpoint_runs(&self) -> u64 {
        self.checkpoint_runs.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_column(c: &Connection, table: &str, column: &str) -> bool {
        c.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
            [table, column],
            |r| r.get(0),
        )
        .expect("pragma")
    }

    #[test]
    fn a_schema_2_file_from_an_earlier_round_2_build_is_backed_up_and_upgraded_to_5() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join(DB_FILE);
        {
            let c = Connection::open(&path).expect("open");
            c.execute_batch(schema::SCHEMA).expect("v1 schema");
            // the earlier build: users.function_enabled and a per-chat chats.free, but no users.free_mode
            c.execute_batch(
                "ALTER TABLE users ADD COLUMN function_enabled INTEGER NOT NULL DEFAULT 1;
                 ALTER TABLE chats ADD COLUMN free INTEGER NOT NULL DEFAULT 0;
                 INSERT INTO meta (key, value) VALUES ('schema', '2');",
            )
            .expect("fixture");
        }
        let store = Store::open(dir.path(), JournalMode::Delete).expect("open store");
        {
            let c = store.lock();
            for (table, column, _) in schema::V2_COLUMNS
                .iter()
                .chain(schema::V3_COLUMNS.iter())
                .chain(schema::V4_COLUMNS.iter())
                .chain(schema::V5_COLUMNS.iter())
                .chain(schema::V6_COLUMNS.iter())
                .chain(schema::LATE_COLUMNS.iter())
                .filter(|(t, c, _)| !schema::is_retired(t, c))
            {
                assert!(has_column(&c, table, column), "{table}.{column}");
            }
            let v: String = c
                .query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
                    r.get(0)
                })
                .expect("schema");
            assert_eq!(v, schema::SCHEMA_VERSION);
        }
        let copies: Vec<_> = std::fs::read_dir(dir.path().join(BACKUP_DIR))
            .expect("backups")
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(copies.len(), 1);
        drop(store);
        // reopening the repaired file works too
        Store::open(dir.path(), JournalMode::Delete).expect("reopen");
    }

    #[test]
    fn a_schema_5_file_without_recognize_max_calls_gets_the_column_on_open() {
        let dir = tempfile::tempdir().expect("dir");
        drop(Store::open(dir.path(), JournalMode::Delete).expect("create"));
        let path = dir.path().join(DB_FILE);
        {
            let c = Connection::open(&path).expect("open");
            c.execute_batch("ALTER TABLE limits DROP COLUMN recognize_max_calls;")
                .expect("drop");
        }
        let store = Store::open(dir.path(), JournalMode::Delete).expect("reopen");
        assert!(has_column(&store.lock(), "limits", "recognize_max_calls"));
    }
}
