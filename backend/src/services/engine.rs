//! `Engine`: the "parser + database" as seen by the API layer. Production uses `RealEngine` (calls `parser::` and `Store` directly),
//! tests swap in a fake implementation, so the API layer does not depend on whether those are finished.
use crate::database::Store;
use crate::error::{DbError, ImportError};
use crate::models::{Attempt, ImportRecord, ParsedSheet, ParsedWorkbook, Question};
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
    fn progress(&self) -> Result<Value, DbError>;
    fn import_history(&self, limit: i64) -> Result<Vec<ImportRecord>, DbError>;
    fn plan_import(&self, sheets: &[ParsedSheet], questions: &[Question])
        -> Result<Value, DbError>;
    fn apply_import(
        &self,
        filename: &str,
        sheets: &[ParsedSheet],
        questions: &[Question],
    ) -> Result<Value, DbError>;
    fn record_attempt(&self, attempt: &Attempt) -> Result<Value, DbError>;
    fn reset_progress(&self) -> Result<Value, DbError>;
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
    fn progress(&self) -> Result<Value, DbError> {
        self.store.progress()
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
    fn record_attempt(&self, attempt: &Attempt) -> Result<Value, DbError> {
        self.store.record_attempt(attempt)
    }
    fn reset_progress(&self) -> Result<Value, DbError> {
        self.store.reset_progress()
    }
}
