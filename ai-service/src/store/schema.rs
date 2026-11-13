//! Schema of the AI database `xexam-ai.db` (spec 2.2, plus the `limits` table of 14.1). Schema version 6: the tables of
//! version 1 (`SCHEMA`, unchanged) plus the columns of `V2_COLUMNS` … `V6_COLUMNS`, the indexes of `V3_INDEXES` and the
//! table and indexes of `V5_TABLES`, added by the same additive steps on every path (new file, upgrade from 1 … 5), so
//! `sqlite_master` is the same either way.

/// Value of `meta.schema` written by this version.
pub const SCHEMA_VERSION: &str = "6";

/// The schema versions upgraded by this version (after a backup copy).
pub const UPGRADABLE: [&str; 5] = ["1", "2", "3", "4", "5"];

/// Columns added by schema v2 (each only when `pragma_table_info` lacks it): (table, column, column definition).
pub const V2_COLUMNS: [(&str, &str, &str); 4] = [
    // 0 = the user's AI+ function is suspended by an administrator (AI settings); 1 (default) = active. Never set for
    // administrators. A user without a row is active.
    (
        "users",
        "function_enabled",
        "function_enabled INTEGER NOT NULL DEFAULT 1",
    ),
    // 1 = the administrator's free mode is on (a state of the user, not of a chat): every answer of that user, in any
    // chat, uses no study-assistant prompt and no memories. Only read for administrators.
    ("users", "free_mode", "free_mode INTEGER NOT NULL DEFAULT 0"),
    // 1 = the message was written (and, for an answer, generated) in free mode. The whole (shortened) history is sent in
    // both modes; when it holds messages of the other mode, one line at the end of the system prompt names the current
    // mode (prompt.rs).
    ("messages", "free", "free INTEGER NOT NULL DEFAULT 0"),
    // Input tokens the provider served from its prompt cache (NULL = not reported)
    ("usage", "tokens_cached", "tokens_cached INTEGER"),
];

/// Columns added by AI schema 3 (each only when `pragma_table_info` lacks it).
pub const V3_COLUMNS: [(&str, &str, &str); 12] = [
    // chat kind: 'chat' (AI Chat), 'practice' / 'exam' (one per session), 'summary' (one per record)
    (
        "chats",
        "kind",
        "kind TEXT NOT NULL DEFAULT 'chat' CHECK (kind IN ('chat', 'practice', 'exam', 'summary'))",
    ),
    // practice / exam: the session key (32 lower-case hex) or, for a record saved without one, 'r<recordId>@<at>';
    // summary: 'r<recordId>@<at>'; NULL for 'chat'
    ("chats", "ref", "ref TEXT"),
    // the main-app record this chat belongs to (display only; never a lookup key: ids can be reused after a restore)
    ("chats", "record_id", "record_id INTEGER"),
    // message kind: 'chat', 'hint', 'explain', 'summary' (both messages of a pair carry it)
    (
        "messages",
        "kind",
        "kind TEXT NOT NULL DEFAULT 'chat' CHECK (kind IN ('chat', 'hint', 'explain', 'summary'))",
    ),
    // hint / explain; NULL otherwise
    ("messages", "question_id", "question_id TEXT"),
    // the number shown to the user (1-based)
    ("messages", "question_no", "question_no INTEGER"),
    (
        "messages",
        "source",
        "source TEXT CHECK (source IS NULL OR source IN ('practice', 'result'))",
    ),
    // reply language
    (
        "messages",
        "lang",
        "lang TEXT CHECK (lang IS NULL OR lang IN ('en', 'zh-CN', 'ja'))",
    ),
    // summary: wrong answers sent
    ("messages", "sample_used", "sample_used INTEGER"),
    // summary: wrong answers whose question was available
    ("messages", "sample_usable", "sample_usable INTEGER"),
    // summary: wrong answers in the record
    ("messages", "sample_wrong", "sample_wrong INTEGER"),
    // NULL = an AI Chat call or a non-chat kind; set only by the study generations
    (
        "usage",
        "purpose",
        "purpose TEXT CHECK (purpose IS NULL OR purpose IN ('hint', 'explain', 'summary'))",
    ),
];

/// Columns added by AI schema 4 (the ranges equal `limits::DEFS`): the per-user caps on AI model calls and the answer
/// length in characters. The retired `limits.message_chars` column (version 1) stays in the table, unused.
pub const V4_COLUMNS: [(&str, &str, &str); 4] = [
    (
        "limits",
        "calls_per_minute",
        "calls_per_minute INTEGER NOT NULL DEFAULT 5 CHECK (calls_per_minute BETWEEN 1 AND 100)",
    ),
    (
        "limits",
        "calls_per_hour",
        "calls_per_hour INTEGER NOT NULL DEFAULT 20 CHECK (calls_per_hour BETWEEN 1 AND 2000)",
    ),
    (
        "limits",
        "calls_per_day",
        "calls_per_day INTEGER NOT NULL DEFAULT 100 CHECK (calls_per_day BETWEEN 1 AND 20000)",
    ),
    (
        "limits",
        "reply_max_chars",
        "reply_max_chars INTEGER NOT NULL DEFAULT 1000 CHECK (reply_max_chars BETWEEN 1 AND 16000)",
    ),
];

/// Columns added by AI schema 5 (the CHECK ranges equal `limits::DEFS`): per-user AI feature switches, the fields of
/// generation / import jobs (kind `bank_gen` rows of `tasks`) and the generation and import settings.
pub const V5_COLUMNS: [(&str, &str, &str); 27] = [
    // AI features an administrator switched off for this user (bits: chat 1, hint 2, explain 4, summary 8, generate
    // 16, import 32; checked in code, so a seventh feature needs no table rebuild). 0 = everything allowed.
    (
        "users",
        "features_off",
        "features_off INTEGER NOT NULL DEFAULT 0 CHECK (features_off >= 0)",
    ),
    ("tasks", "job_key", "job_key TEXT"),
    ("tasks", "goal", "goal INTEGER NOT NULL DEFAULT 0"),
    ("tasks", "made", "made INTEGER NOT NULL DEFAULT 0"),
    // 'reading' | 'confirm' | 'writing' | 'saving' while a job is active, else NULL (checked in code)
    ("tasks", "phase", "phase TEXT"),
    // 1 = counts against the day's quota; a row that does not set it never counts
    (
        "tasks",
        "charged",
        "charged INTEGER NOT NULL DEFAULT 0",
    ),
    ("tasks", "seen", "seen INTEGER NOT NULL DEFAULT 0"),
    ("tasks", "started_at", "started_at INTEGER"),
    ("tasks", "finished_at", "finished_at INTEGER"),
    // The moment the unit was charged (NULL until charged); the quota day is the day a unit was charged (SEC-9)
    ("tasks", "charged_at", "charged_at INTEGER"),
    (
        "limits",
        "gen_per_day",
        "gen_per_day INTEGER NOT NULL DEFAULT 3 CHECK (gen_per_day BETWEEN 0 AND 100)",
    ),
    (
        "limits",
        "gen_admins_exempt",
        "gen_admins_exempt INTEGER NOT NULL DEFAULT 1 CHECK (gen_admins_exempt BETWEEN 0 AND 1)",
    ),
    (
        "limits",
        "gen_min_source",
        "gen_min_source INTEGER NOT NULL DEFAULT 30 CHECK (gen_min_source BETWEEN 5 AND 2000)",
    ),
    (
        "limits",
        "gen_min_wrong",
        "gen_min_wrong INTEGER NOT NULL DEFAULT 10 CHECK (gen_min_wrong BETWEEN 1 AND 500)",
    ),
    (
        "limits",
        "gen_bank_single",
        "gen_bank_single INTEGER NOT NULL DEFAULT 14 CHECK (gen_bank_single BETWEEN 0 AND 100)",
    ),
    (
        "limits",
        "gen_bank_multi",
        "gen_bank_multi INTEGER NOT NULL DEFAULT 6 CHECK (gen_bank_multi BETWEEN 0 AND 100)",
    ),
    (
        "limits",
        "gen_wrong_single",
        "gen_wrong_single INTEGER NOT NULL DEFAULT 14 CHECK (gen_wrong_single BETWEEN 0 AND 100)",
    ),
    (
        "limits",
        "gen_wrong_multi",
        "gen_wrong_multi INTEGER NOT NULL DEFAULT 6 CHECK (gen_wrong_multi BETWEEN 0 AND 100)",
    ),
    (
        "limits",
        "gen_call_chars",
        "gen_call_chars INTEGER NOT NULL DEFAULT 10000 CHECK (gen_call_chars BETWEEN 1000 AND 32000)",
    ),
    (
        "limits",
        "gen_max_calls",
        "gen_max_calls INTEGER NOT NULL DEFAULT 15 CHECK (gen_max_calls BETWEEN 8 AND 40)",
    ),
    (
        "limits",
        "import_max_calls",
        "import_max_calls INTEGER NOT NULL DEFAULT 25 CHECK (import_max_calls BETWEEN 8 AND 40)",
    ),
    (
        "limits",
        "recognize_max_calls",
        "recognize_max_calls INTEGER NOT NULL DEFAULT 20 CHECK (recognize_max_calls BETWEEN 8 AND 40)",
    ),
    (
        "limits",
        "gen_unlimited",
        "gen_unlimited INTEGER NOT NULL DEFAULT 1 CHECK (gen_unlimited BETWEEN 0 AND 1)",
    ),
    (
        "limits",
        "import_unlimited",
        "import_unlimited INTEGER NOT NULL DEFAULT 1 CHECK (import_unlimited BETWEEN 0 AND 1)",
    ),
    (
        "limits",
        "gen_running",
        "gen_running INTEGER NOT NULL DEFAULT 2 CHECK (gen_running BETWEEN 1 AND 8)",
    ),
    (
        "limits",
        "import_max_mb",
        "import_max_mb INTEGER NOT NULL DEFAULT 20 CHECK (import_max_mb BETWEEN 1 AND 20)",
    ),
    (
        "limits",
        "import_max_questions",
        "import_max_questions INTEGER NOT NULL DEFAULT 50 CHECK (import_max_questions BETWEEN 1 AND 50)",
    ),
];

/// Columns added by AI schema 6 (A-OWNER-43): the answer-length settings in tokens. They replace `reply_max_chars`,
/// `gen_call_chars` and `memory_chars`, which stay in the table, unused (a column's CHECK range cannot be altered).
pub const V6_COLUMNS: [(&str, &str, &str); 4] = [
    // Input tokens the provider wrote to its prompt cache (NULL = not reported)
    ("usage", "tokens_cache_write", "tokens_cache_write INTEGER"),
    (
        "limits",
        "reply_max_tokens",
        "reply_max_tokens INTEGER NOT NULL DEFAULT 3000 CHECK (reply_max_tokens BETWEEN 1 AND 250000)",
    ),
    (
        "limits",
        "gen_call_tokens",
        "gen_call_tokens INTEGER NOT NULL DEFAULT 20000 CHECK (gen_call_tokens BETWEEN 1000 AND 250000)",
    ),
    (
        "limits",
        "memory_tokens",
        "memory_tokens INTEGER NOT NULL DEFAULT 1000 CHECK (memory_tokens BETWEEN 50 AND 250000)",
    ),
];

/// Column of a table that `V5_TABLES` creates, so it is added after them (v1.1): cache-write tokens of a model call.
pub const LATE_COLUMNS: [(&str, &str, &str); 1] = [(
    "gen_calls",
    "tokens_cache_write",
    "tokens_cache_write INTEGER",
)];

/// `meta` key of the one-time move of the two "unlimited" switches to their v1.1 default (on, A-OWNER-48).
pub const UNLIMITED_DEFAULT_KEY: &str = "unlimited_default";

/// `meta` key of the one-time lowering of `import_max_questions` to the v1.1 ceiling of 50 (A-OWNER-58b).
pub const IMPORT_MAX_KEY: &str = "import_max_50";

/// Run once on a file whose column was created with the earlier range 1 ..= 100 (a column's CHECK cannot be altered, so
/// the stored value is lowered; the new range is enforced by the limits code).
pub const IMPORT_MAX_SET: &str =
    "UPDATE limits SET import_max_questions = 50 WHERE id = 1 AND import_max_questions > 50";

/// Run once (after a backup copy) on a schema-6 database written before the switches defaulted to on: both go on.
pub const UNLIMITED_DEFAULT_SET: &str =
    "UPDATE limits SET gen_unlimited = 1, import_unlimited = 1 WHERE id = 1";

/// Run once in the upgrade transaction to schema 6 (after the backup copy; never on a later start): the stored
/// character values become tokens with the old Chinese / Japanese factor 1.5. A value still at the old default
/// (1000 / 10000 / 200) moves to the new default (3000 / 20000 / 1000); every result is clamped to the new range.
pub const V6_MIGRATE: &str = "UPDATE limits SET
  reply_max_tokens = CASE WHEN reply_max_chars = 1000 THEN 3000
                          ELSE MIN(250000, MAX(1, reply_max_chars * 3 / 2)) END,
  gen_call_tokens  = CASE WHEN gen_call_chars = 10000 THEN 20000
                          ELSE MIN(250000, MAX(1000, gen_call_chars * 3 / 2)) END,
  memory_tokens    = CASE WHEN memory_chars = 200 THEN 1000
                          ELSE MIN(250000, MAX(50, memory_chars * 3 / 2)) END
  WHERE id = 1";

/// Table and indexes added by AI schema 5 (all `IF NOT EXISTS`; run after `V5_COLUMNS`). `gen_calls` holds the model
/// calls of generation and import jobs and never a `usage` row, so the per-user call caps (which read `usage`) cannot
/// see them (import and generation are outside the caps, A-OWNER-56).
pub const V5_TABLES: &str = "
CREATE TABLE IF NOT EXISTS gen_calls (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id       INTEGER REFERENCES tasks(id) ON DELETE SET NULL,
  user_id       INTEGER REFERENCES users(id) ON DELETE SET NULL,
  purpose       TEXT NOT NULL DEFAULT 'generate',
  at            INTEGER NOT NULL,
  day           TEXT NOT NULL,
  provider      TEXT NOT NULL,
  model         TEXT NOT NULL DEFAULT '',
  tokens_in     INTEGER,
  tokens_out    INTEGER,
  tokens_cached INTEGER,
  latency_ms    INTEGER NOT NULL,
  first_byte_ms INTEGER,
  status        TEXT NOT NULL CHECK (status IN ('ok', 'stopped', 'error')),
  error_code    TEXT,
  asked         INTEGER NOT NULL DEFAULT 0,
  accepted      INTEGER NOT NULL DEFAULT 0,
  truncated     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_gen_calls_day ON gen_calls(day);
CREATE INDEX IF NOT EXISTS idx_gen_calls_task ON gen_calls(task_id);
CREATE INDEX IF NOT EXISTS idx_gen_calls_user ON gen_calls(user_id, day);
CREATE INDEX IF NOT EXISTS idx_tasks_user ON tasks(user_id, kind, created_at);
CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_job ON tasks(job_key) WHERE job_key IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_active ON tasks(user_id, kind) WHERE state IN ('queued', 'running');
CREATE INDEX IF NOT EXISTS idx_tasks_charged ON tasks(user_id, charged_at) WHERE charged = 1;";

/// Run once in the transaction that creates schema 4 or upgrades to it (never on a later start): the memory length
/// default moved from 300 (v0.8 / v0.9) to 200. Only the old default moves; any other value an administrator chose
/// stays. The version-1 `SCHEMA` text keeps its `DEFAULT 300` (it is never edited).
pub const V4_MEMORY_DEFAULT: &str =
    "UPDATE limits SET memory_chars = 200 WHERE id = 1 AND memory_chars = 300";

/// Indexes added by AI schema 3 (all `IF NOT EXISTS`; they need the columns of `V3_COLUMNS`).
pub const V3_INDEXES: &str = "
CREATE UNIQUE INDEX IF NOT EXISTS idx_chats_ref ON chats(user_id, kind, ref) WHERE ref IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_messages_study ON messages(chat_id, question_id, kind, source, role, id) WHERE question_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_messages_streaming ON messages(status) WHERE status = 'streaming';";

/// Every table and index of schema version 1 (all `IF NOT EXISTS`).
pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY NOT NULL,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
  id              INTEGER PRIMARY KEY CHECK (id = 1),
  provider        TEXT NOT NULL DEFAULT 'openai' CHECK (provider IN ('openai', 'anthropic', 'gateway')),
  base_url        TEXT NOT NULL DEFAULT 'https://api.openai.com/v1',
  format          TEXT NOT NULL DEFAULT 'openai' CHECK (format IN ('openai', 'anthropic')),
  gateway_auth    TEXT NOT NULL DEFAULT 'bearer'
                  CHECK (gateway_auth IN ('bearer', 'x-api-key', 'cf-aig-authorization')),
  model           TEXT NOT NULL DEFAULT '',
  manual_model    INTEGER NOT NULL DEFAULT 0,
  usage_option    INTEGER NOT NULL DEFAULT 1,
  max_tokens      INTEGER NOT NULL DEFAULT 8192 CHECK (max_tokens IN (4096, 8192)),
  enabled         INTEGER NOT NULL DEFAULT 0,
  verified_at     INTEGER,
  verified_fp     TEXT,
  retention_days  INTEGER NOT NULL DEFAULT 90 CHECK (retention_days BETWEEN 1 AND 3650),
  updated_at      INTEGER NOT NULL,
  updated_by      TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS secrets (
  name        TEXT PRIMARY KEY NOT NULL CHECK (name IN ('api_key', 'gateway_key')),
  key_id      TEXT NOT NULL,
  nonce       BLOB NOT NULL CHECK (length(nonce) = 24),
  ciphertext  BLOB NOT NULL,
  hint        TEXT NOT NULL DEFAULT '',
  rev         INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS limits (
  id                 INTEGER PRIMARY KEY CHECK (id = 1),
  per_user_running   INTEGER NOT NULL DEFAULT 1     CHECK (per_user_running BETWEEN 1 AND 4),
  global_running     INTEGER NOT NULL DEFAULT 16    CHECK (global_running BETWEEN 1 AND 64),
  chats_per_user     INTEGER NOT NULL DEFAULT 1000  CHECK (chats_per_user BETWEEN 10 AND 100000),
  messages_per_chat  INTEGER NOT NULL DEFAULT 400   CHECK (messages_per_chat BETWEEN 20 AND 5000),
  memories_per_user  INTEGER NOT NULL DEFAULT 50    CHECK (memories_per_user BETWEEN 1 AND 1000),
  memory_chars       INTEGER NOT NULL DEFAULT 300   CHECK (memory_chars BETWEEN 50 AND 2000),
  message_chars      INTEGER NOT NULL DEFAULT 16000 CHECK (message_chars BETWEEN 1000 AND 32000),
  updated_at         INTEGER NOT NULL,
  updated_by         TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS users (
  id            INTEGER PRIMARY KEY NOT NULL,
  stamp         TEXT NOT NULL,
  username      TEXT NOT NULL,
  first_seen_at INTEGER NOT NULL,
  last_seen_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS chats (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  title       TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_chats_user ON chats(user_id, updated_at);
CREATE INDEX IF NOT EXISTS idx_chats_updated ON chats(updated_at);

CREATE TABLE IF NOT EXISTS messages (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  chat_id     INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
  role        TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
  content     TEXT NOT NULL,
  status      TEXT NOT NULL DEFAULT 'done' CHECK (status IN ('streaming', 'done', 'stopped', 'error')),
  error_code  TEXT,
  truncated   INTEGER NOT NULL DEFAULT 0,
  model       TEXT,
  created_at  INTEGER NOT NULL,
  finished_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_messages_chat ON messages(chat_id, id);

CREATE TABLE IF NOT EXISTS memories (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  content     TEXT NOT NULL CHECK (length(content) BETWEEN 1 AND 2000),
  source      TEXT NOT NULL DEFAULT 'manual' CHECK (source IN ('manual', 'chat')),
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_memories_user ON memories(user_id, id);

CREATE TABLE IF NOT EXISTS tasks (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id     INTEGER REFERENCES users(id) ON DELETE CASCADE,
  kind        TEXT NOT NULL,
  state       TEXT NOT NULL DEFAULT 'queued' CHECK (state IN ('queued', 'running', 'done', 'failed', 'cancelled')),
  payload     TEXT NOT NULL DEFAULT '{}',
  result      TEXT,
  error_code  TEXT,
  attempts    INTEGER NOT NULL DEFAULT 0,
  run_after   INTEGER NOT NULL,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tasks_state ON tasks(state, run_after);

CREATE TABLE IF NOT EXISTS usage (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id       INTEGER REFERENCES users(id) ON DELETE SET NULL,
  at            INTEGER NOT NULL,
  day           TEXT NOT NULL,
  kind          TEXT NOT NULL CHECK (kind IN ('chat', 'verify', 'models', 'memory')),
  provider      TEXT NOT NULL,
  model         TEXT NOT NULL DEFAULT '',
  tokens_in     INTEGER,
  tokens_out    INTEGER,
  latency_ms    INTEGER NOT NULL,
  first_byte_ms INTEGER,
  status        TEXT NOT NULL CHECK (status IN ('ok', 'stopped', 'error')),
  error_code    TEXT
);
CREATE INDEX IF NOT EXISTS idx_usage_day ON usage(day);
CREATE INDEX IF NOT EXISTS idx_usage_user ON usage(user_id, day);
CREATE INDEX IF NOT EXISTS idx_usage_model ON usage(model, day);

CREATE TABLE IF NOT EXISTS audit_outbox (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  at       INTEGER NOT NULL,
  message  TEXT NOT NULL
);
";
