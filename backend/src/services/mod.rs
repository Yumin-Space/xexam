//! 业务流程。**接口契约（不要改签名）**。
//!
//! 导入流程（对应规格的概念函数，名字照搬便于对照）：
//! `parse_upload → validate_import → plan_import → create_pending_import → confirm_pending_import | cancel_pending_import → apply_import`
//!
//! 预览阶段**绝不写库**；只有 `confirm_pending_import` 之后的 `apply_import` 写，且是一个事务。
use crate::config::PENDING_TTL_SECS;
use crate::models::{ParsedSheet, Question};
use crate::security;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 暂存的一次上传（只保留最近一次）。
pub struct PendingImport {
    pub token: String,
    pub filename: String,
    pub sheets: Vec<ParsedSheet>,
    pub questions: Vec<Question>,
    pub expires: Instant,
}

/// 全局单槽位。`take` 取出即清空；令牌不对或过期返回 None，且**不消耗**槽位里的内容（过期的除外：与旧版一致，过期的取不出来但留着也无妨）。
pub struct PendingStore {
    slot: Mutex<Option<PendingImport>>,
    ttl: Duration,
}

impl PendingStore {
    pub fn new() -> Self {
        Self { slot: Mutex::new(None), ttl: Duration::from_secs(PENDING_TTL_SECS) }
    }
    pub fn with_ttl(ttl: Duration) -> Self {
        Self { slot: Mutex::new(None), ttl }
    }
    pub fn put(&self, filename: String, sheets: Vec<ParsedSheet>, questions: Vec<Question>) -> String {
        let token = security::new_token();
        let item = PendingImport { token: token.clone(), filename, sheets, questions, expires: Instant::now() + self.ttl };
        *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(item);
        token
    }
    pub fn take(&self, token: &str) -> Option<PendingImport> {
        let mut g = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        let ok = matches!(g.as_ref(), Some(it) if security::token_eq(&it.token, token) && it.expires >= Instant::now());
        if ok { g.take() } else { None }
    }
    pub fn drop_pending(&self) {
        *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

impl Default for PendingStore {
    fn default() -> Self {
        Self::new()
    }
}
