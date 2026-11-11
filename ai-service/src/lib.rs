//! Xexam AI service (`ai-service`): AI Chat, AI Memory and the administrators' AI settings, in its own container with its
//! own SQLite database.
//!
//! The service never sees a session cookie: every request carries only `X-Xexam-Identity`, a short-lived statement
//! signed by the main backend (`identity`). It reaches main-app data only through the backend's `/internal/ai/*`
//! routes (`link`); the backend never opens this service's database.
pub mod api;
pub mod app;
pub mod chat;
pub mod codes;
pub mod config;
pub mod identity;
pub mod limits;
pub mod link;
pub mod logs;
pub mod prompt;
pub mod provider;
pub mod retention;
pub mod secret;
pub mod settings;
pub mod store;
pub mod study;
pub mod util;
