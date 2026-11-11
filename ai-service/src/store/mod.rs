//! The AI database (`<XEXAM_AI_DATA>/xexam-ai.db`, spec 2.2): one `Mutex<Connection>`, every write in
//! `BEGIN IMMEDIATE … COMMIT`, the lock never held across an `.await`.
//!
//! Deleted user content: `secure_delete=ON` zeroes freed pages, and every delete of user content schedules a
//! `PRAGMA wal_checkpoint(TRUNCATE)` (coalesced, at most one run per 10 s, see `app::spawn_checkpointer`) so the old page
//! images do not stay in `xexam-ai.db-wal`. Backups and volume snapshots keep deleted content (documented).
use crate::config::JournalMode;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::Notify;

pub mod data;
pub mod schema;
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

/// Adds the columns of schema v2 that the tables lack (the same step for new and upgraded databases).
fn add_v2_columns(c: &Connection) -> rusqlite::Result<()> {
    for (table, column, def) in schema::V2_COLUMNS {
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

    /// Creates a new database, upgrades a schema-1 database (after a backup copy), or refuses an unknown schema; then
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
                    tx.execute_batch(schema::SCHEMA)?;
                    add_v2_columns(tx)?;
                    tx.execute(
                        "INSERT OR IGNORE INTO settings (id, updated_at) VALUES (1, ?1)",
                        [now],
                    )?;
                    tx.execute(
                        "INSERT OR IGNORE INTO limits (id, updated_at) VALUES (1, ?1)",
                        [now],
                    )?;
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
            Some(v) if v == schema::SCHEMA_VERSION => {
                // A schema-2 file from an earlier round-2 build may lack later columns: add them (idempotent, writes
                // nothing when all are present)
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|e| format!("cannot open the AI database {{path}}: {e}"))?;
                add_v2_columns(&tx)
                    .map_err(|e| format!("cannot upgrade the AI database {{path}}: {e}"))?;
                tx.commit()
                    .map_err(|e| format!("cannot upgrade the AI database {{path}}: {e}"))?;
            }
            Some(v) if v == schema::PREVIOUS_VERSION => {
                let copy = backup_copy(&conn, backups, v).map_err(|e| {
                    format!("cannot back up the AI database before the upgrade ({e}); nothing was changed")
                })?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|e| format!("cannot upgrade the AI database {{path}}: {e}"))?;
                let run = |tx: &Transaction| -> rusqlite::Result<()> {
                    add_v2_columns(tx)?;
                    tx.execute(
                        "UPDATE meta SET value = ?1 WHERE key = 'schema'",
                        [schema::SCHEMA_VERSION],
                    )?;
                    Ok(())
                };
                run(&tx).map_err(|e| format!("cannot upgrade the AI database {{path}}: {e}"))?;
                tx.commit()
                    .map_err(|e| format!("cannot upgrade the AI database {{path}}: {e}"))?;
                tracing::info!(
                    "Upgraded the AI database to schema {}. A copy of the previous version was saved first: {}.",
                    schema::SCHEMA_VERSION,
                    copy.display()
                );
            }
            Some(v) => {
                return Err(format!(
                    "The AI database {{path}} has schema {v}, written by a newer ai-service; this version (schema {}) cannot open it. Run the newer version, or restore a backup.",
                    schema::SCHEMA_VERSION
                ));
            }
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
    fn a_schema_2_file_from_an_earlier_round_2_build_gets_the_missing_columns() {
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
            for (table, column, _) in schema::V2_COLUMNS {
                assert!(has_column(&c, table, column), "{table}.{column}");
            }
            let v: String = c
                .query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
                    r.get(0)
                })
                .expect("schema");
            assert_eq!(v, "2");
        }
        drop(store);
        // reopening the repaired file works too
        Store::open(dir.path(), JournalMode::Delete).expect("reopen");
    }
}
