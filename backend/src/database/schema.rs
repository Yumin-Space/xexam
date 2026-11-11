//! Table schema.
//!
//! - `SCHEMA` is schema v1: **verbatim identical** to `SCHEMA` in `app/db.py` (including the Chinese comments; `sqlite_master.sql`
//!   stores this exact text). Do not change it; it is still executed first on every open.
//! - Schema v2 (v0.5) is purely additive: `MIGRATE_V2_TABLES` plus the columns in `V2_COLUMNS`, applied by `store::migrate` in one
//!   transaction (a brand-new database goes through the same migration, so fresh and migrated databases have the same structure).
//!   Older binaries can still open a v2 file (the new columns are nullable or have defaults), but must not be used once there is
//!   more than one question bank; rolling back = restoring the backup written before the migration (see docs/en/database.md).
//! - Schema v3 (v0.6) is purely additive too: users, sessions, per-user stats and settings (`MIGRATE_V3_TABLES`) and
//!   `attempts.user_id` (`V3_COLUMNS`). The old `stats` and `settings` tables are kept, unused after the first administrator has
//!   claimed their rows, so a rollback is still "restore the backup".
//! - Schema v4 (v0.6.5 / v0.7) is purely additive as well: which banks each plain user may see (the table `user_banks`, one
//!   row per bank) and whether banks created later are added for them (`users.new_banks`), the bank management permission
//!   (`users.can_manage_banks`), bank owners (`banks.owner_id`) and the kind of a study record (`attempts.kind`: exam or
//!   practice). The six-column workbook layout needs no column: its per-language texts live in the question's JSON
//!   (`questions.data`, with `"lang": "en"`) like every other question field.
//!   Before v0.7 was released, its pre-release builds stored `users.all_banks` instead of `users.new_banks` ("every bank,
//!   including later ones"); `store::migrate` converts such a database on open (see there).

pub const SCHEMA_VERSION: &str = "4";

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS questions (
  id        TEXT PRIMARY KEY NOT NULL,   -- SQLite 里 TEXT 主键默认允许 NULL，必须显式 NOT NULL
  sheet     TEXT NOT NULL,
  row_no    INTEGER NOT NULL,
  seq       INTEGER NOT NULL,          -- 题库里的顺序（先按工作表出现的顺序，再按行号）
  data      TEXT NOT NULL,             -- 整道题的 JSON（题干、选项、答案、解析……）
  active    INTEGER NOT NULL DEFAULT 1,
  added_at  TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_questions_sheet ON questions(sheet, active);
CREATE TABLE IF NOT EXISTS attempts (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  at       TEXT NOT NULL,
  sheets   TEXT NOT NULL,              -- JSON 数组
  mode     TEXT NOT NULL,
  total    INTEGER NOT NULL,
  correct  INTEGER NOT NULL,
  percent  INTEGER NOT NULL,
  seconds  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS stats (
  question_id TEXT PRIMARY KEY NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  correct  INTEGER NOT NULL DEFAULT 0,
  wrong    INTEGER NOT NULL DEFAULT 0,
  streak   INTEGER NOT NULL DEFAULT 0,
  last     INTEGER,
  at       TEXT
);
CREATE TABLE IF NOT EXISTS imports (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  at        TEXT NOT NULL,
  filename  TEXT NOT NULL,
  added     INTEGER NOT NULL,
  updated   INTEGER NOT NULL,
  removed   INTEGER NOT NULL,
  restored  INTEGER NOT NULL,
  skipped   INTEGER NOT NULL,
  total     INTEGER NOT NULL           -- 导入后上架的题数
);
"#;

/// Tables and indexes added by schema v2 (all `IF NOT EXISTS`, so re-running is harmless).
pub const MIGRATE_V2_TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS banks (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  name       TEXT NOT NULL,
  filename   TEXT NOT NULL,
  legacy_ids INTEGER NOT NULL DEFAULT 0,   -- 1 = question ids are the unscoped parser ids (data from before v0.5)
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS attempt_items (
  attempt_id  INTEGER NOT NULL,
  seq         INTEGER NOT NULL,
  question_id TEXT NOT NULL,
  answer      TEXT,                 -- JSON array as submitted (cleaned); NULL = unanswered
  correct     INTEGER NOT NULL,
  PRIMARY KEY (attempt_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_attempt_items_question ON attempt_items(question_id);
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
"#;

/// Columns added by schema v2: (table, column, column definition). `ALTER TABLE … ADD COLUMN` is not idempotent, so the migration
/// adds a column only when `PRAGMA table_info` does not list it yet.
pub const V2_COLUMNS: [(&str, &str, &str); 3] = [
    // NULL only for rows written by an older binary (attached to the legacy bank on the next open)
    ("questions", "bank_id", "bank_id INTEGER"),
    // JSON snapshot of the banks the exam drew from: [{"id": 1, "name": "…"}]
    ("attempts", "banks", "banks TEXT NOT NULL DEFAULT '[]'"),
    ("imports", "bank_id", "bank_id INTEGER"),
];

/// Index that needs `questions.bank_id`, created after the columns.
pub const MIGRATE_V2_INDEXES: &str =
    "CREATE INDEX IF NOT EXISTS idx_questions_bank ON questions(bank_id, active);";

/// Tables and indexes added by schema v3 (all `IF NOT EXISTS`).
pub const MIGRATE_V3_TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS users (
  id                  INTEGER PRIMARY KEY AUTOINCREMENT,
  username            TEXT NOT NULL UNIQUE COLLATE NOCASE,
  password_hash       TEXT NOT NULL,
  role                TEXT NOT NULL CHECK (role IN ('admin', 'user')),
  disabled            INTEGER NOT NULL DEFAULT 0,
  created_at          TEXT NOT NULL,
  last_login_at       TEXT,
  password_changed_at TEXT NOT NULL,
  must_change_password INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS sessions (
  token_hash   TEXT PRIMARY KEY NOT NULL,
  user_id      INTEGER NOT NULL,
  created_at   TEXT NOT NULL,
  last_seen_at INTEGER NOT NULL,   -- unix seconds
  expires_at   INTEGER NOT NULL    -- unix seconds
);
CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
CREATE TABLE IF NOT EXISTS user_stats (
  user_id     INTEGER NOT NULL,
  question_id TEXT NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0, correct INTEGER NOT NULL DEFAULT 0, wrong INTEGER NOT NULL DEFAULT 0,
  streak INTEGER NOT NULL DEFAULT 0, last INTEGER, at TEXT,
  PRIMARY KEY (user_id, question_id)
);
CREATE TABLE IF NOT EXISTS user_settings (user_id INTEGER NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY (user_id, key));
"#;

/// Columns added by schema v3 (same rules as [`V2_COLUMNS`]).
pub const V3_COLUMNS: [(&str, &str, &str); 1] = [
    // NULL until the first administrator claims the old data (and for rows written by an older binary after a rollback)
    ("attempts", "user_id", "user_id INTEGER"),
];

/// Index that needs `attempts.user_id`, created after the column.
pub const MIGRATE_V3_INDEXES: &str =
    "CREATE INDEX IF NOT EXISTS idx_attempts_user ON attempts(user_id);";

/// Tables and indexes added by schema v4 (all `IF NOT EXISTS`). A row of `user_banks` lets a plain user see one bank in exams,
/// practice, records and the wrong-answer notebook; it is the only thing that does (besides owning the bank as a bank
/// manager). Deleting a bank or a user deletes its rows.
pub const MIGRATE_V4_TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS user_banks (
  user_id INTEGER NOT NULL,
  bank_id INTEGER NOT NULL,
  PRIMARY KEY (user_id, bank_id)
);
CREATE INDEX IF NOT EXISTS idx_user_banks_bank ON user_banks(bank_id);
"#;

/// Columns added by schema v4 (same rules as [`V2_COLUMNS`]).
pub const V4_COLUMNS: [(&str, &str, &str); 4] = [
    // 1 = "include banks imported later": every bank created later (any import as a new bank, the import command) gets a
    // `user_banks` row for this user in the same transaction. Existing banks are never added by it. Set, with a row for every
    // existing bank, for every user that exists at the migration (they saw every bank and would have seen new ones).
    ("users", "new_banks", "new_banks INTEGER NOT NULL DEFAULT 0"),
    // 1 = a plain user may import new banks and update / delete the banks they own (administrators always may)
    (
        "users",
        "can_manage_banks",
        "can_manage_banks INTEGER NOT NULL DEFAULT 0",
    ),
    // The user who created the bank (NULL only for rows written by an older binary; given to the first administrator on open)
    ("banks", "owner_id", "owner_id INTEGER"),
    // An exam or a practice session; every record from before v0.6.5 is an exam
    (
        "attempts",
        "kind",
        "kind TEXT NOT NULL DEFAULT 'exam' CHECK (kind IN ('exam', 'practice'))",
    ),
];

/// Index that needs `banks.owner_id`, created after the columns.
pub const MIGRATE_V4_INDEXES: &str =
    "CREATE INDEX IF NOT EXISTS idx_banks_owner ON banks(owner_id);";
