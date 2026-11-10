//! Table schema: **verbatim identical** to `SCHEMA` in `app/db.py` (including the Chinese comments; `sqlite_master.sql` stores this exact text).
//! Do not change it, and do not add tables / columns / indexes: the Python and Rust versions must be able to open the same database.

pub const SCHEMA_VERSION: &str = "1";

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
