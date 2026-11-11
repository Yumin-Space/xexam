//! SQLite storage. **Interface contract (do not change signatures).** The v1 table schema and SQL semantics are verbatim identical to
//! `app/db.py`: synchronous=FULL, foreign_keys=ON, busy_timeout=5000, `BEGIN IMMEDIATE`. v0.5 adds schema v2 on top
//! (additive: question banks, per-question exam detail, settings; see `schema` and `docs/en/database.md`). Since v0.7 the
//! journal mode is WAL instead of DELETE, with the opt-out `XEXAM_SQLITE_JOURNAL=delete` (see `journal`).
//!
//! `Store` wraps a `Mutex<rusqlite::Connection>` and all its methods are **synchronous**; the API layer calls them via `spawn_blocking`,
//! so the lock is never held across `.await`. Source: `docs/zh-CN/history/audit/database.md`.
//!
//! File layout:
//! - `journal`          journal mode (WAL by default, the DELETE opt-out) and the checkpoint run at shutdown
//! - `schema`           table schema text (v1 verbatim identical to the old version) and the v2 additions
//! - `store`            the `Store` itself: open (+ schema v2 migration), question banks, import, study records, settings
//! - `legacy_progress`  reading and item-by-item conversion of the old progress.json (Python `int()` / truthiness semantics)
//! - `timefmt`          local time `%Y-%m-%d %H:%M` and a replaceable `Clock`
//! - `pyjson`           output identical to Python `json.dumps(ensure_ascii=False)` (separators, key sorting)

pub mod journal;
pub mod legacy_progress;
pub mod pyjson;
pub mod schema;
pub mod store;
pub mod timefmt;

pub use journal::{Checkpoint, JournalMode};
pub use store::{
    percent_round_half_even, scoped_id, LegacyMigration, RetiredBank, SessionUser, Store,
    UpdatedUser, UserChange, UserRow, UserUpdate, BACKUP_DIR,
};
pub use timefmt::{Clock, FixedClock, FnClock, SystemClock};

pub const MASTER_STREAK: i64 = 2;
/// Number of exam sessions `/api/progress` returns in `history` (unchanged since v0.2).
pub const HISTORY_LIMIT: i64 = 60;
/// Number of exam sessions kept in the database (older ones are pruned with their per-question detail). Raised from 60 in v0.5.
pub const RECORD_RETENTION: i64 = 500;
