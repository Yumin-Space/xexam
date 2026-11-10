//! Table schema.
//!
//! - `SCHEMA` is schema v1: **verbatim identical** to `SCHEMA` in `app/db.py` (including the Chinese comments; `sqlite_master.sql`
//!   stores this exact text). Do not change it; it is still executed first on every open.
//! - Schema v2 (v0.5) is purely additive: `MIGRATE_V2_TABLES` plus the columns in `V2_COLUMNS`, applied by `store::migrate` in one
//!   transaction (a brand-new database goes through the same migration, so fresh and migrated databases have the same structure).
//!   Older binaries can still open a v2 file (the new columns are nullable or have defaults), but must not be used once there is
//!   more than one question bank; rolling back = restoring the backup written before the migration (see docs/en/database.md).

pub const SCHEMA_VERSION: &str = "2";

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
