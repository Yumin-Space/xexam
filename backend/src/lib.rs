//! Xexam backend (library). The binary is in `main.rs`.
//!
//! Module responsibilities:
//! - `config`    environment variables and startup arguments (XEXAM_*)
//! - `models`    data models (field order = key order of the old JSON)
//! - `security`  Host / Origin guards, tokens, request-body limits, file-name sanitizing
//! - `parser`    XLSX parsing (zip + quick-xml), question ids, translation splitting
//! - `database`  SQLite storage (the v1 schema text is identical to the old version, character for character; v0.5 migrates to schema v2)
//! - `services`  import flow (parse_upload → … → apply_import), pending-confirmation slot, result recording
//! - `api`       HTTP routes and JSON output; sign-in is enforced in its single dispatch function; `api::internal` (v0.8):
//!   the link with the AI service, ai-service (the auth step of `/api/ai/*`, the token routes under `/internal/` and its
//!   heartbeat, which says whether AI+ is available)
//! - `auth`      accounts (v0.6): password hashing, session tokens and cookie, login throttle, input rules
//! - `logs`      system log (v0.7): a `tracing` layer keeping the most recent log lines in memory for `GET /api/logs`

pub mod api;
pub mod auth;
pub mod config;
pub mod database;
pub mod error;
pub mod logs;
pub mod models;
pub mod parser;
pub mod security;
pub mod services;
