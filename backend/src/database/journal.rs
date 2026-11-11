//! SQLite journal mode (v0.7).
//!
//! Since v0.7 the database runs in WAL mode (`journal_mode=WAL`, which is persistent: it is stored in the file header) with
//! `synchronous=FULL`, so a commit still waits for one fsync (of the `-wal` file) and nothing that was committed is lost on a
//! power failure. Readers no longer block the writer and the writer no longer blocks readers.
//!
//! The opt-out `XEXAM_SQLITE_JOURNAL=delete` keeps the old rollback-journal mode (`DELETE`) for storage where the shared
//! memory file (`xexam.db-shm`) is unreliable: network filesystems, or a Windows folder bind-mounted into Docker Desktop.
//! Any other value, or no value, means WAL.
//!
//! Switching the mode never makes opening fail: when SQLite cannot switch (another process holds the database in a way that
//! prevents it, or the filesystem does not support it), a warning is logged and the database keeps the mode SQLite reports.
//! An older Xexam version that opens a WAL database switches it back to `DELETE` itself (it still asks for that mode), so a
//! rollback needs no extra step.
use rusqlite::Connection;

/// Environment variable of the opt-out.
pub const JOURNAL_ENV: &str = "XEXAM_SQLITE_JOURNAL";

/// The journal mode asked for when the database is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JournalMode {
    /// Write-ahead log (the default since v0.7)
    #[default]
    Wal,
    /// The rollback journal of v0.6 and older (opt-out `XEXAM_SQLITE_JOURNAL=delete`)
    Delete,
}

impl JournalMode {
    /// Reads the value of `XEXAM_SQLITE_JOURNAL`: `delete` (trimmed, any case) is the opt-out; anything else, or nothing, is WAL.
    pub fn from_env_value(value: Option<&str>) -> JournalMode {
        match value {
            Some(v) if v.trim().eq_ignore_ascii_case("delete") => JournalMode::Delete,
            _ => JournalMode::Wal,
        }
    }

    /// The mode as `PRAGMA journal_mode` reports it (lower case).
    pub fn as_str(self) -> &'static str {
        match self {
            JournalMode::Wal => "wal",
            JournalMode::Delete => "delete",
        }
    }
}

/// The journal mode the connection reports right now (lower case), or `"unknown"` when it cannot be read.
pub fn current(conn: &Connection) -> String {
    conn.pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))
        .map(|m| m.to_ascii_lowercase())
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Asks for `want` and returns the mode in effect afterwards (lower case). Never fails: when the switch is refused or
/// SQLite stays in another mode, a warning is logged and the reported mode is returned.
pub fn apply(conn: &Connection, want: JournalMode) -> String {
    let effective = match conn.pragma_update_and_check(None, "journal_mode", want.as_str(), |r| {
        r.get::<_, String>(0)
    }) {
        Ok(mode) => mode.to_ascii_lowercase(),
        Err(e) => {
            let now = current(conn);
            tracing::warn!(
                "could not switch the SQLite journal mode to {}: {e}; continuing in mode {now}",
                want.as_str()
            );
            return now;
        }
    };
    if effective != want.as_str() {
        // For example a filesystem without shared-memory support: SQLite answers with the mode it kept
        tracing::warn!(
            "SQLite did not switch the journal mode to {}; continuing in mode {effective}",
            want.as_str()
        );
    }
    effective
}

/// What `PRAGMA wal_checkpoint(TRUNCATE)` reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checkpoint {
    /// The checkpoint could not finish because another connection was reading or writing (the `-wal` file was not emptied)
    pub busy: bool,
    /// Frames in the `-wal` file after the checkpoint as SQLite reports them: 0 once a TRUNCATE checkpoint has emptied it
    pub log_frames: i64,
    /// Frames copied into the database file, as SQLite reports them (also 0 after a complete TRUNCATE checkpoint)
    pub checkpointed: i64,
}

/// Runs `PRAGMA wal_checkpoint(TRUNCATE)`: copies every committed frame of the `-wal` file into the database file and
/// truncates the `-wal` file to zero bytes, so the volume holds just `xexam.db` at rest.
pub fn checkpoint_truncate(conn: &Connection) -> rusqlite::Result<Checkpoint> {
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
        Ok(Checkpoint {
            busy: r.get::<_, i64>(0)? != 0,
            log_frames: r.get(1)?,
            checkpointed: r.get(2)?,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_delete_opts_out() {
        assert_eq!(JournalMode::from_env_value(None), JournalMode::Wal);
        assert_eq!(JournalMode::from_env_value(Some("")), JournalMode::Wal);
        assert_eq!(JournalMode::from_env_value(Some("wal")), JournalMode::Wal);
        assert_eq!(
            JournalMode::from_env_value(Some("truncate")),
            JournalMode::Wal
        );
        assert_eq!(
            JournalMode::from_env_value(Some("delete")),
            JournalMode::Delete
        );
        assert_eq!(
            JournalMode::from_env_value(Some(" DELETE \n")),
            JournalMode::Delete
        );
        assert_eq!(JournalMode::default(), JournalMode::Wal);
    }

    #[test]
    fn an_in_memory_database_keeps_its_own_mode() {
        // SQLite refuses WAL for an in-memory database and answers "memory": no error, the reported mode is kept
        let c = Connection::open_in_memory().expect("open");
        assert_eq!(apply(&c, JournalMode::Wal), "memory");
        assert_eq!(current(&c), "memory");
    }
}
