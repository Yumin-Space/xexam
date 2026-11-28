//! Table schema.
//!
//! - `SCHEMA` is schema v1, kept as history: it was word for word the `SCHEMA` text of `app/db.py` (including the Chinese
//!   comments). Do not change it. Since schema 11 it is executed only for a new file or a file older than 11 (the upgrade
//!   chain starts from it); an up-to-date file is never touched by it, so the legacy `stats` table is not re-created.
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
//! - Schema v5 (v0.8) adds two columns to `users` and nothing else (`V5_COLUMNS`): the per-user AI+ switch and a random
//!   identity stamp that the AI service keeps next to the user id. AI settings, chats and memories live in the AI
//!   service's own database, never in this one.
//! - Schema v6 (v0.9) adds two nullable columns to `attempts` and nothing else (`V6_COLUMNS`): the key and the start time of
//!   the practice / exam session a record was saved from, so the AI service can find the one chat of a session again from
//!   its record. Older records keep NULL.
//! - Schema v7 (v1.1) adds four nullable columns and two indexes (`V7_COLUMNS`, `MIGRATE_V7_INDEXES`): the time limit of a
//!   timed exam and how an exam ended (`attempts.time_limit`, `attempts.ended`; `ended` set marks a record saved under the
//!   not-answered rule, whose unanswered questions are derived, never stored as a column), and the job key and the JSON
//!   facts of an AI-generated bank (`banks.ai_job`, `banks.ai_meta`). Older rows keep NULL.

//! - Schema v8 (v1.3) is additive: the session facts (`sessions.started_at / remember / ip / ua`), the last activity and
//!   sign-in info of a user (`users.last_active_at / last_login_ip / _country / _region / _city`), the activity log and the
//!   small `system_settings` table, and an index for the idempotent attempt save (`V8_COLUMNS`, `MIGRATE_V8_TABLES`). IP and
//!   location are written only while the single sign-in info switch is on. A v1.2 binary opens a v8 file unchanged.
//! - Schema v9 (v1.4) is additive: the last activity IP, its place (three columns) and the cleaned user agent of the last
//!   activity of a user (`V9_COLUMNS`). Nothing is backfilled, and a v1.3 binary opens a v9 file unchanged (`upgrade` treats
//!   any newer schema as "nothing to migrate"). The IP and place are written only while the sign-in info switch is on.
//! - Schema v10 (v1.6) adds one column to `users` and nothing else (`V10_COLUMNS`): the per-user permission to download the
//!   question banks the user can see (`users.can_download_banks`, default 0; administrators always may). A v1.5 binary
//!   opens a v10 file unchanged, and its own inserts get the default 0.

//! - Schema v11 (v2.0) is the database clean-up: nine tables are rebuilt with explicit foreign keys (`V11_REBUILDS`, all
//!   `ON DELETE NO ACTION`), two indexes and a unique index on `users.uid` are added, and the legacy `stats` / `settings`
//!   tables are dropped. No column, type, default or CHECK changes. See `store::migrate` and docs/en/database.md.
//!   Deliberately NOT linked (records outlive their questions; audit snapshots): `attempt_items.question_id`,
//!   `user_stats.question_id`, `activity_log.user_id`, `system_settings.updated_by`.
//! - Schema v12 (v2.6) adds one nullable column to `users` and one index (`V12_COLUMNS`, `V12_DISPLAY_NAME_INDEX`): the
//!   public display name (`users.display_name`, NULL = not chosen yet; existing users keep NULL and are asked for one at
//!   their next sign-in). Names are unique case-insensitively among the set ones; `users.username` stays the login. A v2.5
//!   binary opens a v12 file unchanged (its inserts leave the column NULL).

//! - Schema v13 (v3.1) is additive: bank categories (`bank_categories`, `banks.category_id`, `banks.sort`), one row per browser per
//!   user (`devices`, `sessions.device_id`), web push (`push_subscriptions`, `push_outbox`), the server-side marker of an unfinished
//!   practice / exam (`saved_sessions`), and the unused `idx_questions_sheet` is dropped. A v3.0 binary opens a v13 file unchanged;
//!   deleting a user who has rows in the new tables then fails until the v3.0 rollback SQL ran (docs/en/database.md section 6.22).

pub const SCHEMA_VERSION: &str = "13";

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

/// Columns added by schema v5 (same rules as [`V2_COLUMNS`]).
pub const V5_COLUMNS: [(&str, &str, &str); 2] = [
    // 1 = AI+ is on for this user: they may use AI Chat while AI is enabled in AI settings (the AI service decides that).
    // Stored for administrators too but ignored for them (administrators always may), like can_manage_banks.
    (
        "users",
        "ai_enabled",
        "ai_enabled INTEGER NOT NULL DEFAULT 0",
    ),
    // Random identity stamp (32 lower-case hex digits), set once when the account is created and never changed. The AI
    // service stores it next to the user id and drops AI data whose stamp differs (an id reused after a restore).
    ("users", "uid", "uid TEXT NOT NULL DEFAULT ''"),
];

/// Columns added by schema v6 (same rules as [`V2_COLUMNS`]).
pub const V6_COLUMNS: [(&str, &str, &str); 2] = [
    // The random key (32 lower-case hex digits) of the practice / exam session the record was saved from; the AI service names
    // the session's chat by it. NULL for records saved before v0.9 and when the browser sent none (or an invalid one).
    ("attempts", "session_key", "session_key TEXT"),
    // When that session started (unix seconds, the browser's clock); NULL like `session_key`.
    ("attempts", "started_at", "started_at INTEGER"),
];

/// Columns added by schema v7 (same rules as [`V2_COLUMNS`]). In this order: `ended`'s CHECK names `time_limit`, and
/// `ai_meta`'s names `ai_job` (`ADD COLUMN` accepts and enforces both). The CHECKs are a safety net only: the cleaning of
/// `POST /api/attempt` and the AI-bank create route never write a value they refuse, and a rolled-back v1.0 binary writes
/// NULL / NULL, which they accept.
pub const V7_COLUMNS: [(&str, &str, &str); 4] = [
    // The time limit of a timed exam (seconds, 60..=36000); NULL = untimed (every untimed exam, every practice, every record
    // before v1.1)
    (
        "attempts",
        "time_limit",
        "time_limit INTEGER CHECK (time_limit IS NULL OR (time_limit BETWEEN 60 AND 36000 AND kind = 'exam'))",
    ),
    // How an exam saved by v1.1 or later ended: 'timeUp' (submitted by the countdown; timed only) or 'submit' (by hand, timed
    // or not); NULL = a practice, or a record written before v1.1 (the v1.0 rule: an empty answer is wrong). Set = saved under
    // the not-answered rule; the unanswered items are derived (items of such a record with answer NULL and correct 0). CASE
    // keeps the CHECK NULL-safe (a NULL result would pass a CHECK).
    (
        "attempts",
        "ended",
        "ended TEXT CHECK (CASE WHEN ended IS NULL THEN time_limit IS NULL \
         ELSE kind = 'exam' AND (ended = 'submit' OR (ended = 'timeUp' AND time_limit IS NOT NULL)) END)",
    ),
    // The ai-service job key of an AI-generated bank (32 lower-case hex digits); NULL = an imported bank
    ("banks", "ai_job", "ai_job TEXT"),
    // JSON facts of an AI-generated bank (source, source banks, author, counts, time); NULL = an imported bank
    (
        "banks",
        "ai_meta",
        "ai_meta TEXT CHECK ((ai_meta IS NULL) = (ai_job IS NULL))",
    ),
];

/// Indexes created after the v7 columns: one bank per job (the create route's idempotency), and the covering index of the
/// bank list (`banks_json`'s aggregate then reads only the index).
pub const MIGRATE_V7_INDEXES: &str = "
CREATE UNIQUE INDEX IF NOT EXISTS idx_banks_ai_job ON banks(ai_job) WHERE ai_job IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_questions_layout ON questions(bank_id, sheet, active, seq);";

/// Columns added by schema v8 (same rules as [`V2_COLUMNS`]).
pub const V8_COLUMNS: [(&str, &str, &str); 9] = [
    // 'YYYY-MM-DD HH:MM:SS': the last time the user did something (touched at most once a minute; a migrated row may hold 'YYYY-MM-DD HH:MM:00')
    ("users", "last_active_at", "last_active_at TEXT"),
    // Sign-in info: written and shown only while the `users.signin_info` switch is open (stored values stay when it closes)
    ("users", "last_login_ip", "last_login_ip TEXT"),
    ("users", "last_login_country", "last_login_country TEXT"),
    ("users", "last_login_region", "last_login_region TEXT"),
    ("users", "last_login_city", "last_login_city TEXT"),
    // Unix seconds of the sign-in; NULL = unknown (a session a v1.2 binary created; fixed at the first touch)
    ("sessions", "started_at", "started_at INTEGER"),
    (
        "sessions",
        "remember",
        "remember INTEGER NOT NULL DEFAULT 1",
    ),
    // ONLY while the switch is on
    ("sessions", "ip", "ip TEXT"),
    // User-Agent, cleaned, at most 256 characters
    ("sessions", "ua", "ua TEXT"),
];

/// Tables and indexes added by schema v8 (all `IF NOT EXISTS`). The attempts index is for the idempotent save: it is NOT
/// unique on purpose (rows written before v1.3 are not checked for duplicates).
pub const MIGRATE_V8_TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS activity_log (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  ts        INTEGER NOT NULL,
  last_ts   INTEGER NOT NULL,
  count     INTEGER NOT NULL DEFAULT 1,
  actor     TEXT NOT NULL CHECK (actor IN ('user', 'anon', 'system')),
  user_id   INTEGER,
  username  TEXT,
  role      TEXT,
  action    TEXT NOT NULL,
  object    TEXT,
  detail    TEXT,
  result    TEXT NOT NULL CHECK (result IN ('ok', 'fail')),
  reason    TEXT,
  ip        TEXT,
  country   TEXT,
  region    TEXT,
  city      TEXT,
  ua        TEXT,
  source    TEXT NOT NULL DEFAULT 'backend' CHECK (source IN ('backend', 'ai')),
  src_id    TEXT
);
CREATE INDEX IF NOT EXISTS idx_activity_ts     ON activity_log(ts);
CREATE INDEX IF NOT EXISTS idx_activity_user   ON activity_log(user_id, id);
CREATE INDEX IF NOT EXISTS idx_activity_action ON activity_log(action, id);
-- retention skips the sign-in rows: its age scan and count read only the other rows
CREATE INDEX IF NOT EXISTS idx_activity_prune  ON activity_log(ts) WHERE action <> 'auth.signin';
CREATE UNIQUE INDEX IF NOT EXISTS idx_activity_src ON activity_log(src_id) WHERE src_id IS NOT NULL;
CREATE TABLE IF NOT EXISTS system_settings (
  key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL, updated_at TEXT NOT NULL, updated_by INTEGER
);
"#;

/// Index that needs `attempts.session_key` / `attempts.started_at` (schema v6): the lookup of the idempotent attempt save.
pub const MIGRATE_V8_INDEXES: &str = "
CREATE INDEX IF NOT EXISTS idx_attempts_session ON attempts(user_id, session_key) WHERE session_key IS NOT NULL;";

/// Columns added by schema v9 (same rules as [`V2_COLUMNS`]).
pub const V9_COLUMNS: [(&str, &str, &str); 5] = [
    // IP of the last activity write: written and shown only while the `users.signin_info` switch is open
    ("users", "last_active_ip", "last_active_ip TEXT"),
    // Place of that IP (country code or name); written and shown only while the switch is open
    ("users", "last_active_country", "last_active_country TEXT"),
    ("users", "last_active_region", "last_active_region TEXT"),
    ("users", "last_active_city", "last_active_city TEXT"),
    // Cleaned User-Agent (at most 256 characters) of the last activity write; written also while the switch is closed
    ("users", "last_active_ua", "last_active_ua TEXT"),
];

/// Columns added by schema v10 (same rules as [`V2_COLUMNS`]).
pub const V10_COLUMNS: [(&str, &str, &str); 1] = [
    // 1 = a plain user may download the banks they can see (administrators always may, whatever is stored)
    (
        "users",
        "can_download_banks",
        "can_download_banks INTEGER NOT NULL DEFAULT 0",
    ),
];

/// One table rebuilt by the schema-v11 step (the 12-step procedure of the SQLite documentation, see `store::migrate`).
pub struct Rebuild {
    pub table: &'static str,
    /// `CREATE TABLE new_<table> (…)`; after the rename SQLite stores it as `CREATE TABLE "<table>" (…)`.
    pub create: &'static str,
    /// The explicit column list, in table order (also the list copied from the old table).
    pub columns: &'static str,
    /// Index statements, run AFTER the rename (never on `new_<table>`, so the stored text is `ON <table>(…)`).
    pub indexes: &'static str,
}

/// The nine tables rebuilt by schema v11, in the fixed order (parents before children). Column names, order, types, DEFAULTs
/// and CHECKs are those of the schema-v10 tables; the only additions are the `REFERENCES … ON DELETE NO ACTION` clauses
/// (NO ACTION is written out on purpose: deleting a parent that still has children is an error, never a silent cascade) and
/// the comments. Not linked on purpose: `attempt_items.question_id` and `user_stats.question_id` (a record outlives a deleted
/// question; legacy banks use unscoped ids), `activity_log.user_id` (audit snapshot), `system_settings.updated_by` (a
/// write-only audit field that must not block deleting an administrator).
pub const V11_REBUILDS: [Rebuild; 9] = [
    Rebuild {
        table: "banks",
        create: "CREATE TABLE new_banks (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  name       TEXT NOT NULL,
  filename   TEXT NOT NULL,
  legacy_ids INTEGER NOT NULL DEFAULT 0,   -- 1 = question ids are the unscoped parser ids (data from before v0.5)
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  owner_id   INTEGER REFERENCES users(id) ON DELETE NO ACTION, -- NULL only from an older binary (given to the first administrator on open)
  ai_job     TEXT,
  ai_meta    TEXT CHECK ((ai_meta IS NULL) = (ai_job IS NULL))
)",
        columns: "id, name, filename, legacy_ids, created_at, updated_at, owner_id, ai_job, ai_meta",
        indexes: "CREATE INDEX IF NOT EXISTS idx_banks_owner ON banks(owner_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_banks_ai_job ON banks(ai_job) WHERE ai_job IS NOT NULL;",
    },
    Rebuild {
        table: "questions",
        create: "CREATE TABLE new_questions (
  id TEXT PRIMARY KEY NOT NULL, sheet TEXT NOT NULL, row_no INTEGER NOT NULL, seq INTEGER NOT NULL, data TEXT NOT NULL,
  active INTEGER NOT NULL DEFAULT 1, added_at TEXT NOT NULL, updated_at TEXT NOT NULL,
  bank_id INTEGER REFERENCES banks(id) ON DELETE NO ACTION  -- NULL only from an older binary (attached to the legacy bank on open)
)",
        columns: "id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id",
        indexes: "CREATE INDEX IF NOT EXISTS idx_questions_sheet ON questions(sheet, active);
CREATE INDEX IF NOT EXISTS idx_questions_bank ON questions(bank_id, active);
CREATE INDEX IF NOT EXISTS idx_questions_layout ON questions(bank_id, sheet, active, seq);",
    },
    Rebuild {
        table: "imports",
        create: "CREATE TABLE new_imports (
  id INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT NOT NULL, filename TEXT NOT NULL, added INTEGER NOT NULL, updated INTEGER NOT NULL,
  removed INTEGER NOT NULL, restored INTEGER NOT NULL, skipped INTEGER NOT NULL, total INTEGER NOT NULL,
  bank_id INTEGER REFERENCES banks(id) ON DELETE NO ACTION
)",
        columns: "id, at, filename, added, updated, removed, restored, skipped, total, bank_id",
        indexes: "CREATE INDEX IF NOT EXISTS idx_imports_bank ON imports(bank_id);",
    },
    Rebuild {
        table: "attempts",
        create: "CREATE TABLE new_attempts (
  id INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT NOT NULL, sheets TEXT NOT NULL, mode TEXT NOT NULL, total INTEGER NOT NULL,
  correct INTEGER NOT NULL, percent INTEGER NOT NULL, seconds INTEGER NOT NULL,
  banks TEXT NOT NULL DEFAULT '[]',
  user_id INTEGER REFERENCES users(id) ON DELETE NO ACTION,  -- NULL = unclaimed old record (claimed by the first administrator on open)
  kind TEXT NOT NULL DEFAULT 'exam' CHECK (kind IN ('exam', 'practice')),
  session_key TEXT, started_at INTEGER,
  time_limit INTEGER CHECK (time_limit IS NULL OR (time_limit BETWEEN 60 AND 36000 AND kind = 'exam')),
  ended TEXT CHECK (CASE WHEN ended IS NULL THEN time_limit IS NULL ELSE kind = 'exam' AND (ended = 'submit' OR (ended = 'timeUp' AND time_limit IS NOT NULL)) END)
)",
        columns: "id, at, sheets, mode, total, correct, percent, seconds, banks, user_id, kind, session_key, started_at, time_limit, ended",
        indexes: "CREATE INDEX IF NOT EXISTS idx_attempts_user ON attempts(user_id);
CREATE INDEX IF NOT EXISTS idx_attempts_session ON attempts(user_id, session_key) WHERE session_key IS NOT NULL;",
    },
    Rebuild {
        table: "attempt_items",
        create: "CREATE TABLE new_attempt_items (
  attempt_id INTEGER NOT NULL REFERENCES attempts(id) ON DELETE NO ACTION,
  seq INTEGER NOT NULL,
  question_id TEXT NOT NULL,   -- no foreign key: a record outlives a deleted question; legacy banks use unscoped ids
  answer TEXT, correct INTEGER NOT NULL,
  PRIMARY KEY (attempt_id, seq)
)",
        columns: "attempt_id, seq, question_id, answer, correct",
        indexes: "CREATE INDEX IF NOT EXISTS idx_attempt_items_question ON attempt_items(question_id);",
    },
    Rebuild {
        table: "sessions",
        create: "CREATE TABLE new_sessions (
  token_hash TEXT PRIMARY KEY NOT NULL,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE NO ACTION,
  created_at TEXT NOT NULL, last_seen_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
  started_at INTEGER, remember INTEGER NOT NULL DEFAULT 1, ip TEXT, ua TEXT
)",
        columns: "token_hash, user_id, created_at, last_seen_at, expires_at, started_at, remember, ip, ua",
        indexes: "CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);",
    },
    Rebuild {
        table: "user_stats",
        create: "CREATE TABLE new_user_stats (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE NO ACTION,
  question_id TEXT NOT NULL,   -- no foreign key (see attempt_items)
  attempts INTEGER NOT NULL DEFAULT 0, correct INTEGER NOT NULL DEFAULT 0, wrong INTEGER NOT NULL DEFAULT 0,
  streak INTEGER NOT NULL DEFAULT 0, last INTEGER, at TEXT,
  PRIMARY KEY (user_id, question_id)
)",
        columns: "user_id, question_id, attempts, correct, wrong, streak, last, at",
        indexes: "CREATE INDEX IF NOT EXISTS idx_user_stats_question ON user_stats(question_id);",
    },
    Rebuild {
        table: "user_settings",
        create: "CREATE TABLE new_user_settings (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE NO ACTION, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY (user_id, key)
)",
        columns: "user_id, key, value",
        indexes: "",
    },
    Rebuild {
        table: "user_banks",
        create: "CREATE TABLE new_user_banks (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE NO ACTION,
  bank_id INTEGER NOT NULL REFERENCES banks(id) ON DELETE NO ACTION,
  PRIMARY KEY (user_id, bank_id)
)",
        columns: "user_id, bank_id",
        indexes: "CREATE INDEX IF NOT EXISTS idx_user_banks_bank ON user_banks(bank_id);",
    },
];

/// The unique index on `users.uid` (non-empty stamps only: an empty stamp is a user an older binary created, which the
/// open fixes). Created by the v11 step BEFORE the rebuilds when no non-empty stamp is shared, otherwise skipped with a
/// warning (see `store::migrate`); the every-open check creates it later once the duplicates are gone.
pub const V11_USERS_UID_INDEX: &str =
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_users_uid ON users(uid) WHERE uid <> ''";

/// Columns added by schema v12 (same rules as [`V2_COLUMNS`]).
pub const V12_COLUMNS: [(&str, &str, &str); 1] = [
    // The public display name (leaderboard, greeting, menu). NULL = not chosen yet; an empty string is never stored.
    ("users", "display_name", "display_name TEXT"),
];

/// Unique, case-insensitive display names (set ones only). Created after the column, idempotent.
pub const V12_DISPLAY_NAME_INDEX: &str = "CREATE UNIQUE INDEX IF NOT EXISTS idx_users_display_name ON users(display_name COLLATE NOCASE) WHERE display_name IS NOT NULL";

/// v13 (v3.1): bank categories, one level, edited by administrators only.
pub const MIGRATE_V13_CATEGORIES: &str = "
CREATE TABLE IF NOT EXISTS bank_categories (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  name       TEXT NOT NULL UNIQUE COLLATE NOCASE CHECK (name <> '' AND length(name) <= 60),
  sort       INTEGER NOT NULL DEFAULT 0 CHECK (sort >= 0),
  created_at INTEGER NOT NULL,   -- unix seconds
  updated_at INTEGER NOT NULL    -- unix seconds
);";

/// v13 columns of `banks` (same rules as `V2_COLUMNS`), in this order.
pub const V13_BANK_COLUMNS: [(&str, &str, &str); 2] = [
    // NULL = uncategorised. Moving a bank never changes `banks.updated_at` (content time, part of the library stamp)
    (
        "banks",
        "category_id",
        "category_id INTEGER REFERENCES bank_categories(id) ON DELETE NO ACTION",
    ),
    // Position inside its category (or among the uncategorised banks), 1..n once placed; 0 = not placed yet (new banks).
    // Groups are ordered by ((sort = 0), sort, id), so unplaced banks follow the placed ones in id order
    (
        "banks",
        "sort",
        "sort INTEGER NOT NULL DEFAULT 0 CHECK (sort >= 0)",
    ),
];

pub const V13_BANK_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_banks_category ON banks(category_id, sort);";

/// v13: one row per browser per user (the random `xexam_did` cookie, stored as its SHA-256), with that browser's sign-in
/// and last-activity facts. IP and place columns follow the sign-in info switch exactly like `users.last_*`.
pub const MIGRATE_V13_DEVICES: &str = "
CREATE TABLE IF NOT EXISTS devices (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id        INTEGER NOT NULL REFERENCES users(id) ON DELETE NO ACTION,
  cookie_hash    TEXT NOT NULL CHECK (length(cookie_hash) = 64 AND cookie_hash NOT GLOB '*[^0-9a-f]*'),
  created_at     INTEGER NOT NULL,
  active_at      INTEGER NOT NULL,
  active_ip      TEXT,
  active_country TEXT,
  active_region  TEXT,
  active_city    TEXT,
  signin_at      INTEGER,
  signin_ip      TEXT,
  signin_country TEXT,
  signin_region  TEXT,
  signin_city    TEXT,
  ua             TEXT CHECK (ua IS NULL OR length(ua) <= 256),
  app            INTEGER NOT NULL DEFAULT 0 CHECK (app IN (0, 1)),
  tz             TEXT CHECK (tz IS NULL OR length(tz) <= 64),
  signed_out_at  INTEGER,
  UNIQUE (user_id, cookie_hash)
);";

/// v13 column of `sessions`: the device the session belongs to (NULL = a session made before v3.1 or by an older binary;
/// linked at its next request).
pub const V13_SESSION_COLUMNS: [(&str, &str, &str); 1] = [(
    "sessions",
    "device_id",
    "device_id INTEGER REFERENCES devices(id) ON DELETE NO ACTION",
)];

pub const V13_SESSION_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_sessions_device ON sessions(device_id);";

/// v13: web push. One subscription per device; the outbox is queue and 7-day delivery log in one table; the saved-session
/// marker drives the practice / exam reminders.
pub const MIGRATE_V13_PUSH: &str = "
CREATE TABLE IF NOT EXISTS push_subscriptions (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  device_id    INTEGER NOT NULL UNIQUE REFERENCES devices(id) ON DELETE NO ACTION,
  endpoint     TEXT NOT NULL UNIQUE CHECK (substr(endpoint, 1, 8) = 'https://' AND length(endpoint) <= 2048),
  p256dh       TEXT NOT NULL CHECK (length(p256dh) = 87),
  auth         TEXT NOT NULL CHECK (length(auth) = 22),
  key_id       TEXT NOT NULL CHECK (length(key_id) = 16 AND key_id NOT GLOB '*[^0-9a-f]*'),
  created_at   INTEGER NOT NULL,
  confirmed_at INTEGER NOT NULL,
  expires_at   INTEGER,
  last_ok_at   INTEGER,
  last_fail_at INTEGER,
  last_status  INTEGER,
  failures     INTEGER NOT NULL DEFAULT 0 CHECK (failures >= 0)
);
CREATE TABLE IF NOT EXISTS saved_sessions (
  user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE NO ACTION,
  session_key TEXT NOT NULL CHECK (length(session_key) = 32 AND session_key NOT GLOB '*[^0-9a-f]*'),
  device_id   INTEGER REFERENCES devices(id) ON DELETE NO ACTION,
  kind        TEXT NOT NULL CHECK (kind IN ('exam', 'practice')),
  answered    INTEGER NOT NULL CHECK (answered >= 0),
  total       INTEGER NOT NULL CHECK (total BETWEEN 1 AND 100000),
  started_at  INTEGER NOT NULL,
  time_left   INTEGER,
  saved_at    INTEGER NOT NULL,
  remind_at   INTEGER,
  reminders   INTEGER NOT NULL DEFAULT 0 CHECK (reminders BETWEEN 0 AND 3),
  PRIMARY KEY (user_id, session_key),
  CHECK (answered <= total),
  CHECK (time_left IS NULL OR (kind = 'exam' AND time_left BETWEEN 0 AND 36000))
);
CREATE INDEX IF NOT EXISTS idx_saved_sessions_device ON saved_sessions(device_id);
CREATE TABLE IF NOT EXISTS push_outbox (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  subscription_id INTEGER NOT NULL REFERENCES push_subscriptions(id) ON DELETE NO ACTION,
  kind            TEXT NOT NULL CHECK (kind IN ('remind', 'bank', 'version', 'test')),
  topic           TEXT CHECK (topic IS NULL OR (length(topic) BETWEEN 1 AND 32 AND topic NOT GLOB '*[^A-Za-z0-9_-]*')),
  payload         TEXT NOT NULL CHECK (json_valid(payload) AND length(CAST(payload AS BLOB)) <= 1024),
  urgency         TEXT NOT NULL DEFAULT 'normal' CHECK (urgency IN ('very-low', 'low', 'normal', 'high')),
  created_at      INTEGER NOT NULL,
  expires_at      INTEGER NOT NULL,
  state           TEXT NOT NULL DEFAULT 'queued' CHECK (state IN ('queued', 'leased', 'sent', 'failed', 'expired')),
  next_at         INTEGER NOT NULL,
  tries           INTEGER NOT NULL DEFAULT 0 CHECK (tries >= 0),
  status          INTEGER,
  done_at         INTEGER,
  CHECK ((state IN ('queued', 'leased')) = (done_at IS NULL)),
  CHECK (expires_at > created_at)
);
CREATE INDEX IF NOT EXISTS idx_push_outbox_sub ON push_outbox(subscription_id);
CREATE INDEX IF NOT EXISTS idx_push_outbox_due ON push_outbox(next_at) WHERE state IN ('queued', 'leased');";

/// v13: indexes dropped (no query filters on `sheet` without `bank_id`; `idx_questions_layout` serves every sheet filter).
pub const V13_DROP_INDEXES: &str = "DROP INDEX IF EXISTS idx_questions_sheet;";
