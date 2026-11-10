//! Xexam backend (library). The binary is in `main.rs`.
//!
//! Module responsibilities:
//! - `config`    environment variables and startup arguments (XEXAM_*)
//! - `models`    data models (field order = key order of the old JSON)
//! - `security`  Host / Origin guards, tokens, request-body limits, file-name sanitizing
//! - `parser`    XLSX parsing (zip + quick-xml), question ids, translation splitting
//! - `database`  SQLite storage (schema is identical to the old version, character for character)
//! - `services`  import flow (parse_upload → … → apply_import), pending-confirmation slot, result recording
//! - `api`       HTTP routes and JSON output

pub mod api;
pub mod config;
pub mod database;
pub mod error;
pub mod models;
pub mod parser;
pub mod security;
pub mod services;
