//! SQLite storage. **Interface contract (do not change signatures).** Table schema, PRAGMAs and SQL semantics are verbatim identical to `app/db.py`:
//! journal_mode=DELETE, synchronous=FULL, foreign_keys=ON, busy_timeout=5000, `BEGIN IMMEDIATE`.
//!
//! `Store` wraps a `Mutex<rusqlite::Connection>` and all its methods are **synchronous**; the API layer calls them via `spawn_blocking`,
//! so the lock is never held across `.await`. Source: `docs/audit/database.md`.
//!
//! File layout:
//! - `schema`           table schema text (verbatim identical to the old version)
//! - `store`            the `Store` itself: open, question bank, import, study records
//! - `legacy_progress`  reading and item-by-item conversion of the old progress.json (Python `int()` / truthiness semantics)
//! - `timefmt`          local time `%Y-%m-%d %H:%M` and a replaceable `Clock`
//! - `pyjson`           output identical to Python `json.dumps(ensure_ascii=False)` (separators, key sorting)

pub mod legacy_progress;
pub mod pyjson;
pub mod schema;
pub mod store;
pub mod timefmt;

pub use store::{percent_round_half_even, Store};
pub use timefmt::{Clock, FixedClock, FnClock, SystemClock};

pub const MASTER_STREAK: i64 = 2;
pub const HISTORY_LIMIT: i64 = 60;
