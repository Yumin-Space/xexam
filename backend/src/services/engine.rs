//! `Engine`：API 层看到的「解析器 + 数据库」。生产环境用 `RealEngine`（直接调用 `parser::` 与 `Store`），
//! 测试里换成假实现，所以 API 层不依赖它们是否已经写完。
use crate::database::Store;
use crate::error::{DbError, ImportError};
use crate::models::{Attempt, ImportRecord, ParsedSheet, ParsedWorkbook, Question};
use crate::parser;
use serde_json::Value;
use std::sync::Arc;

pub trait Engine: Send + Sync + 'static {
    /// 对应旧版 `check_xlsx(data)` + `parse_workbook(data)`。
    fn check_and_parse(&self, data: &[u8]) -> Result<ParsedWorkbook, ImportError>;
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

/// 生产实现：只是转发。
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
