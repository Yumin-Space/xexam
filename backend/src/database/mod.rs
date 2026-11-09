//! SQLite 存储。**接口契约（不要改签名）**。表结构、PRAGMA、SQL 的语义与 `app/db.py` 逐字一致：
//! journal_mode=DELETE、synchronous=FULL、foreign_keys=ON、busy_timeout=5000、`BEGIN IMMEDIATE`。
//!
//! `Store` 内部是 `Mutex<rusqlite::Connection>`，所有方法都是**同步**的；API 层用 `spawn_blocking` 调用，
//! 锁不跨 `.await`。依据：`docs/audit/database.md`。
use crate::error::DbError;
use crate::models::{Attempt, ImportRecord, ParsedSheet, Question};
use serde_json::Value;
use std::path::Path;

pub const MASTER_STREAK: i64 = 2;
pub const HISTORY_LIMIT: i64 = 60;

pub struct Store {
    _private: (),
}

impl Store {
    /// 打开（必要时创建）数据库并建表。对已有数据库**不改动内容**。
    pub fn open(_path: &Path) -> Result<Store, DbError> {
        unimplemented!("Store::open")
    }
    pub fn count_active(&self) -> Result<i64, DbError> {
        unimplemented!()
    }
    /// `/api/bank`：键顺序见 tests/baseline/golden/123_after_import_bank.json
    pub fn bank(&self) -> Result<Value, DbError> {
        unimplemented!()
    }
    /// 预览：只读，不写库。返回值的键顺序见黄金响应 #114（去掉 filename / token）。
    pub fn plan_import(&self, _sheets: &[ParsedSheet], _questions: &[Question]) -> Result<Value, DbError> {
        unimplemented!()
    }
    /// 真正写入：一个事务（BEGIN IMMEDIATE … COMMIT，失败 ROLLBACK）。返回的 plan 与 `plan_import` 同形。
    pub fn apply_import(&self, _filename: &str, _sheets: &[ParsedSheet], _questions: &[Question]) -> Result<Value, DbError> {
        unimplemented!()
    }
    pub fn import_history(&self, _limit: i64) -> Result<Vec<ImportRecord>, DbError> {
        unimplemented!()
    }
    /// `/api/progress`：`{"version":1,"questions":{…},"history":[…],"masterStreak":2}`
    pub fn progress(&self) -> Result<Value, DbError> {
        unimplemented!()
    }
    pub fn record_attempt(&self, _attempt: &Attempt) -> Result<Value, DbError> {
        unimplemented!()
    }
    pub fn reset_progress(&self) -> Result<Value, DbError> {
        unimplemented!()
    }
    /// 旧版 progress.json 迁移。返回 (是否迁移了, 说明)。格式不对时返回 (false, 说明)，不崩溃。
    pub fn migrate_legacy_progress(&self, _path: &Path) -> Result<(bool, String), DbError> {
        unimplemented!()
    }
}
