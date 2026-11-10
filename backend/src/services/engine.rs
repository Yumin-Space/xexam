//! `Engine`: the "parser + database" as seen by the API layer. Production uses `RealEngine` (calls `parser::` and `Store` directly),
//! tests swap in a fake implementation, so the API layer does not depend on whether those are finished.
//!
//! The methods added in v0.5 (bank targets, banks, records, settings, answers) have default implementations so that fake engines
//! written for v0.4 keep compiling: the import ones fall back to the legacy methods for the legacy target, the attempt one drops
//! the answers, everything else returns `DbError::Unsupported`.
//!
//! v0.6: study records, statistics and settings belong to a user, so those methods take `user`, the id of the signed-in user
//! (the API layer passes it from the session; it is never taken from the request body). Accounts and sessions are not part of
//! this trait: the API layer handles them with the real `Store` directly, so no test engine can stand in for them.
use crate::database::Store;
use crate::error::{DbError, ImportError};
use crate::models::{
    Attempt, DeletedBank, ImportRecord, ImportTarget, ParsedSheet, ParsedWorkbook, Question,
};
use crate::parser;
use serde_json::Value;
use std::sync::Arc;

pub trait Engine: Send + Sync + 'static {
    /// Corresponds to the old `check_xlsx(data)` + `parse_workbook(data)`.
    fn check_and_parse(&self, data: &[u8]) -> Result<ParsedWorkbook, ImportError>;
    /// Same as above, but the parser stops on its own when it sees `deadline`. The default implementation ignores the deadline (the fake engine used in tests); the real engine overrides it.
    fn check_and_parse_until(
        &self,
        data: &[u8],
        _deadline: std::time::Instant,
    ) -> Result<ParsedWorkbook, ImportError> {
        self.check_and_parse(data)
    }
    fn count_active(&self) -> Result<i64, DbError>;
    fn bank(&self) -> Result<Value, DbError>;
    fn progress(&self, user: i64) -> Result<Value, DbError>;
    fn import_history(&self, limit: i64) -> Result<Vec<ImportRecord>, DbError>;
    /// Preview against the legacy bank (an upload without `X-Bank-Target`).
    fn plan_import(&self, sheets: &[ParsedSheet], questions: &[Question])
        -> Result<Value, DbError>;
    /// Import into the legacy bank (created if it does not exist).
    fn apply_import(
        &self,
        filename: &str,
        sheets: &[ParsedSheet],
        questions: &[Question],
    ) -> Result<Value, DbError>;
    fn record_attempt(&self, user: i64, attempt: &Attempt) -> Result<Value, DbError>;
    fn reset_progress(&self, user: i64) -> Result<Value, DbError>;

    // ---- v0.5 ----

    /// Preview against `target`. `Ok(None)` = the target bank does not exist.
    fn plan_import_for(
        &self,
        target: ImportTarget,
        sheets: &[ParsedSheet],
        questions: &[Question],
    ) -> Result<Option<Value>, DbError> {
        match target {
            ImportTarget::Legacy => self.plan_import(sheets, questions).map(Some),
            _ => Err(DbError::Unsupported),
        }
    }
    /// Import into `target`. Returns `(result, bank id)`; `Ok(None)` = the target bank no longer exists (nothing written).
    /// The default (legacy target only) reports bank id 0: an engine without banks has no id to give.
    fn apply_import_for(
        &self,
        target: ImportTarget,
        filename: &str,
        sheets: &[ParsedSheet],
        questions: &[Question],
    ) -> Result<Option<(Value, i64)>, DbError> {
        match target {
            ImportTarget::Legacy => self
                .apply_import(filename, sheets, questions)
                .map(|v| Some((v, 0))),
            _ => Err(DbError::Unsupported),
        }
    }
    fn bank_exists(&self, _id: i64) -> Result<bool, DbError> {
        Err(DbError::Unsupported)
    }
    /// `GET /api/banks`
    fn banks(&self) -> Result<Value, DbError> {
        Err(DbError::Unsupported)
    }
    /// `Ok(None)` = no such bank.
    fn delete_bank(&self, _id: i64, _with_records: bool) -> Result<Option<DeletedBank>, DbError> {
        Err(DbError::Unsupported)
    }
    /// `GET /api/records`
    fn records(&self, _user: i64) -> Result<Value, DbError> {
        Err(DbError::Unsupported)
    }
    /// `GET /api/record?id=`; `Ok(None)` = no such record.
    fn record(&self, _user: i64, _id: i64) -> Result<Option<Value>, DbError> {
        Err(DbError::Unsupported)
    }
    /// `false` = no such record.
    fn delete_record(&self, _user: i64, _id: i64) -> Result<bool, DbError> {
        Err(DbError::Unsupported)
    }
    fn delete_all_records(&self, _user: i64, _with_stats: bool) -> Result<(), DbError> {
        Err(DbError::Unsupported)
    }
    /// `GET /api/settings`
    fn settings(&self, _user: i64) -> Result<Value, DbError> {
        Err(DbError::Unsupported)
    }
    /// Every pair is a known key with a valid value (validated by the caller).
    fn update_settings(&self, _user: i64, _changes: &[(String, Value)]) -> Result<Value, DbError> {
        Err(DbError::Unsupported)
    }
    fn reset_settings(&self, _user: i64) -> Result<Value, DbError> {
        Err(DbError::Unsupported)
    }
    /// `record_attempt` plus the per-question answers (`answers[i]` belongs to `attempt.results[i]`). The default drops the answers.
    fn record_attempt_with_answers(
        &self,
        user: i64,
        attempt: &Attempt,
        _answers: &[Option<Value>],
    ) -> Result<Value, DbError> {
        self.record_attempt(user, attempt)
    }
}

/// Production implementation: just forwards.
pub struct RealEngine {
    pub store: Arc<Store>,
}

impl RealEngine {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

impl Engine for RealEngine {
    fn check_and_parse(&self, data: &[u8]) -> Result<ParsedWorkbook, ImportError> {
        parser::check_xlsx(data)?;
        parser::parse_workbook(data)
    }
    fn check_and_parse_until(
        &self,
        data: &[u8],
        deadline: std::time::Instant,
    ) -> Result<ParsedWorkbook, ImportError> {
        parser::check_xlsx(data)?;
        parser::parse_workbook_until(data, Some(deadline))
    }
    fn count_active(&self) -> Result<i64, DbError> {
        self.store.count_active()
    }
    fn bank(&self) -> Result<Value, DbError> {
        self.store.bank()
    }
    fn progress(&self, user: i64) -> Result<Value, DbError> {
        self.store.progress(user)
    }
    fn import_history(&self, limit: i64) -> Result<Vec<ImportRecord>, DbError> {
        self.store.import_history(limit)
    }
    fn plan_import(
        &self,
        sheets: &[ParsedSheet],
        questions: &[Question],
    ) -> Result<Value, DbError> {
        self.store.plan_import(sheets, questions)
    }
    fn apply_import(
        &self,
        filename: &str,
        sheets: &[ParsedSheet],
        questions: &[Question],
    ) -> Result<Value, DbError> {
        self.store.apply_import(filename, sheets, questions)
    }
    fn record_attempt(&self, user: i64, attempt: &Attempt) -> Result<Value, DbError> {
        self.store.record_attempt(user, attempt)
    }
    fn reset_progress(&self, user: i64) -> Result<Value, DbError> {
        self.store.reset_progress(user)
    }
    fn plan_import_for(
        &self,
        target: ImportTarget,
        sheets: &[ParsedSheet],
        questions: &[Question],
    ) -> Result<Option<Value>, DbError> {
        self.store.plan_import_for(target, sheets, questions)
    }
    fn apply_import_for(
        &self,
        target: ImportTarget,
        filename: &str,
        sheets: &[ParsedSheet],
        questions: &[Question],
    ) -> Result<Option<(Value, i64)>, DbError> {
        self.store
            .apply_import_for(target, filename, sheets, questions)
    }
    fn bank_exists(&self, id: i64) -> Result<bool, DbError> {
        self.store.bank_exists(id)
    }
    fn banks(&self) -> Result<Value, DbError> {
        self.store.banks()
    }
    fn delete_bank(&self, id: i64, with_records: bool) -> Result<Option<DeletedBank>, DbError> {
        self.store.delete_bank(id, with_records)
    }
    fn records(&self, user: i64) -> Result<Value, DbError> {
        self.store.records(user)
    }
    fn record(&self, user: i64, id: i64) -> Result<Option<Value>, DbError> {
        self.store.record(user, id)
    }
    fn delete_record(&self, user: i64, id: i64) -> Result<bool, DbError> {
        self.store.delete_record(user, id)
    }
    fn delete_all_records(&self, user: i64, with_stats: bool) -> Result<(), DbError> {
        self.store.delete_all_records(user, with_stats)
    }
    fn settings(&self, user: i64) -> Result<Value, DbError> {
        self.store.settings(user)
    }
    fn update_settings(&self, user: i64, changes: &[(String, Value)]) -> Result<Value, DbError> {
        self.store.update_settings(user, changes)
    }
    fn reset_settings(&self, user: i64) -> Result<Value, DbError> {
        self.store.reset_settings(user)
    }
    fn record_attempt_with_answers(
        &self,
        user: i64,
        attempt: &Attempt,
        answers: &[Option<Value>],
    ) -> Result<Value, DbError> {
        self.store
            .record_attempt_with_answers(user, attempt, answers)
    }
}
