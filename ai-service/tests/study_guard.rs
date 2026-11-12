//! The hint guard (`xexam_ai_service::study::guard`): whole-text rules, streaming, persistence rule and costs.
//! All questions are self-made samples.
#[path = "study_guard/claims.rs"]
mod claims;
#[path = "study_guard/hotspot.rs"]
mod hotspot;
#[path = "study_guard/incremental.rs"]
mod incremental;
#[path = "study_guard/limits.rs"]
mod limits;
#[path = "study_guard/lists.rs"]
mod lists;
#[path = "study_guard/persistence.rs"]
mod persistence;
#[path = "study_guard/think.rs"]
mod think;
#[path = "study_guard/util.rs"]
mod util;
