//! Schema of the AI database `xexam-ai.db` (spec 2.2, plus the `limits` table of 14.1). Schema version 3: the tables of
//! version 1 (`SCHEMA`, unchanged) plus the columns of `V2_COLUMNS` and `V3_COLUMNS` and the indexes of `V3_INDEXES`,
//! added by the same additive steps on every path (new file, upgrade from 1, upgrade from 2), so `sqlite_master` is the
//! same either way.

/// Value of `meta.schema` written by this version.
pub const SCHEMA_VERSION: &str = "3";

/// The schema versions upgraded by this version (after a backup copy).
pub const UPGRADABLE: [&str; 2] = ["1", "2"];

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
