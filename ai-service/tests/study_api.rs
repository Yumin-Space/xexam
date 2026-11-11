//! The v0.9 study routes (`/api/ai/study/*`, `/api/ai/summary/*`) with the in-process stub provider and the fake
//! backend. All questions, records and texts are made up. The tests run with `TZ=UTC` (set once in `util::env`).
mod common;

#[path = "study_api/access.rs"]
mod access;
#[path = "study_api/admin.rs"]
mod admin;
#[path = "study_api/bounds.rs"]
mod bounds;
#[path = "study_api/identity.rs"]
mod identity;
#[path = "study_api/logs.rs"]
mod logs;
#[path = "study_api/saved.rs"]
mod saved;
#[path = "study_api/streaming.rs"]
mod streaming;
#[path = "study_api/summary.rs"]
mod summary;
#[path = "study_api/util.rs"]
mod util;
