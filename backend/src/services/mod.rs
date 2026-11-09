//! 业务流程。**接口契约（不要改签名）**。
//!
//! 导入流程（对应规格的概念函数，名字照搬便于对照）：
//! `parse_upload → validate_import → plan_import → create_pending_import → confirm_pending_import | cancel_pending_import → apply_import`
//!
//! 预览阶段**绝不写库**；只有 `confirm_pending_import` 之后的 `apply_import` 写，且是一个事务。
use crate::config::{PENDING_TTL_SECS, SKIP_SHOWN};
use crate::error::{AppError, DbError};
use crate::models::{ParsedSheet, ParsedWorkbook, Question};
use crate::security;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub mod engine;
pub use engine::{Engine, RealEngine};

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
        Self {
            slot: Mutex::new(None),
            ttl: Duration::from_secs(PENDING_TTL_SECS),
        }
    }
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            slot: Mutex::new(None),
            ttl,
        }
    }
    pub fn put(
        &self,
        filename: String,
        sheets: Vec<ParsedSheet>,
        questions: Vec<Question>,
    ) -> String {
        let token = security::new_token();
        let item = PendingImport {
            token: token.clone(),
            filename,
            sheets,
            questions,
            expires: Instant::now() + self.ttl,
        };
        *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(item);
        token
    }
    pub fn take(&self, token: &str) -> Option<PendingImport> {
        let mut g = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(g.as_ref(), Some(it) if it.expires < Instant::now()) {
            *g = None; // 过期即清：行为上与旧版（留着但取不出来）看不出差别
            return None;
        }
        let ok = matches!(g.as_ref(), Some(it) if security::token_eq(&it.token, token));
        if ok {
            g.take()
        } else {
            None
        }
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

// ---------------------------------------------------------------- 导入流程

pub const MSG_PARSE_TIMEOUT: &str =
    "解析这个文件用的时间太长，已经停止。请确认选对了文件，或者换一份小一点的再试。";
pub const MSG_NO_QUESTIONS: &str = "这份文件里没有读到任何可用的题，没有导入。";
pub const MSG_UNEXPECTED: &str = "读取这个 Excel 时出错：文件内容不符合预期";
pub const MSG_APPLY_FAILED: &str = "写入数据库失败，题库没有任何改动。";

/// 上传内容检查 + 解析（旧版 `check_xlsx` + `parse_workbook`）。同步的解析器放进 `spawn_blocking`，
/// 并有总时限（`XEXAM_PARSE_TIMEOUT_SECS`）：超时回 400 中文说明。
pub async fn parse_upload(
    engine: Arc<dyn Engine>,
    data: Vec<u8>,
    timeout: Duration,
) -> Result<ParsedWorkbook, AppError> {
    let task = tokio::task::spawn_blocking(move || engine.check_and_parse(&data));
    match tokio::time::timeout(timeout, task).await {
        Err(_) => {
            tracing::warn!("upload parse timed out after {:?}", timeout);
            Err(AppError::BadRequest(MSG_PARSE_TIMEOUT.to_string()))
        }
        Ok(Err(join)) => {
            tracing::error!("upload parser task failed: {join}");
            Err(AppError::BadRequest(MSG_UNEXPECTED.to_string()))
        }
        Ok(Ok(Err(e))) => {
            tracing::info!("upload rejected: {e}");
            Err(e.into())
        }
        Ok(Ok(Ok(wb))) => Ok(wb),
    }
}

/// 确认时的校验：没有任何可用的题就不导入（令牌此时已被取走，与旧版一致）。
pub fn validate_import(item: &PendingImport) -> Result<(), AppError> {
    if item.questions.is_empty() {
        return Err(AppError::BadRequest(MSG_NO_QUESTIONS.to_string()));
    }
    Ok(())
}

/// 预览：只读，不写库。返回 `(plan, 原样还回来的解析结果)`。
pub async fn plan_import(
    engine: Arc<dyn Engine>,
    wb: ParsedWorkbook,
) -> Result<(Value, ParsedWorkbook), AppError> {
    let joined = tokio::task::spawn_blocking(move || {
        let r = engine.plan_import(&wb.sheets, &wb.questions);
        (r, wb)
    })
    .await
    .map_err(|_| DbError::Join)?;
    let (r, wb) = joined;
    Ok((r?, wb))
}

/// 旧版 `preview_payload`：每个工作表最多列 30 条「读不出来」的原因，并追加 `skippedMore`、`skippedCount`；末尾追加 `filename`、`token`。
pub fn preview_payload(mut plan: Value, filename: &str, token: &str) -> Value {
    if let Value::Object(map) = &mut plan {
        if let Some(Value::Array(sheets)) = map.get_mut("sheets") {
            for s in sheets.iter_mut() {
                if let Value::Object(so) = s {
                    let all: Vec<Value> = so
                        .get("skipped")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    let total = all.len();
                    let shown: Vec<Value> = all.into_iter().take(SKIP_SHOWN).collect();
                    so.insert("skipped".to_string(), Value::Array(shown));
                    so.insert(
                        "skippedMore".to_string(),
                        Value::from(total.saturating_sub(SKIP_SHOWN)),
                    );
                    so.insert("skippedCount".to_string(), Value::from(total));
                }
            }
        }
        map.insert("filename".to_string(), Value::from(filename));
        map.insert("token".to_string(), Value::from(token));
    }
    plan
}

/// 把解析结果放进唯一的待确认槽位（顶掉上一份），返回给前端的预览负载。
pub fn create_pending_import(
    pending: &PendingStore,
    filename: String,
    wb: ParsedWorkbook,
    plan: Value,
) -> Value {
    let ParsedWorkbook { sheets, questions } = wb;
    let token = pending.put(filename.clone(), sheets, questions);
    preview_payload(plan, &filename, &token)
}

/// 取出（一次性）并校验。令牌错 / 过期 / 已被处理 → 409；没有题 → 400。
pub fn confirm_pending_import(
    pending: &PendingStore,
    token: &str,
) -> Result<PendingImport, AppError> {
    let item = pending.take(token).ok_or(AppError::PendingGone)?;
    validate_import(&item)?;
    Ok(item)
}

/// 取消不需要令牌（旧版如此）。
pub fn cancel_pending_import(pending: &PendingStore) {
    pending.drop_pending();
}

/// 真正写库（一个事务），随后读出完整题库。返回 `(result, bank)`。同步的 `Store` 放进 `spawn_blocking`。
pub async fn apply_import(
    engine: Arc<dyn Engine>,
    item: PendingImport,
) -> Result<(Value, Value), DbError> {
    tokio::task::spawn_blocking(move || {
        let result = engine.apply_import(&item.filename, &item.sheets, &item.questions)?;
        let bank = engine.bank()?;
        Ok((result, bank))
    })
    .await
    .map_err(|_| DbError::Join)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Answer, Kind};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn q() -> Question {
        Question {
            kind: Kind::Single,
            tag: String::new(),
            stem: "s".into(),
            options: vec![],
            items: vec![],
            answer: Answer::Letters(vec!["A".into()]),
            translation: String::new(),
            zh: None,
            explanation: String::new(),
            mnemonic: String::new(),
            note: String::new(),
            id: "i".into(),
            sheet: "x".into(),
            row: 1,
        }
    }

    #[test]
    fn take_is_once_and_wrong_token_keeps_slot() {
        let p = PendingStore::new();
        let t = p.put("f.xlsx".into(), vec![], vec![q()]);
        assert_eq!(t.len(), 22);
        assert!(p.take("nope").is_none());
        assert!(p.take("").is_none());
        assert!(p.take("令牌").is_none());
        let it = p
            .take(&t)
            .expect("right token still works after wrong ones");
        assert_eq!(it.filename, "f.xlsx");
        assert!(p.take(&t).is_none());
    }

    #[test]
    fn second_put_replaces_first_and_drop_clears() {
        let p = PendingStore::new();
        let t1 = p.put("a".into(), vec![], vec![q()]);
        let t2 = p.put("b".into(), vec![], vec![q()]);
        assert!(p.take(&t1).is_none());
        assert!(p.take(&t2).is_some());
        let t3 = p.put("c".into(), vec![], vec![q()]);
        p.drop_pending();
        assert!(p.take(&t3).is_none());
    }

    #[test]
    fn expired_items_cannot_be_taken() {
        let p = PendingStore::with_ttl(Duration::from_millis(20));
        let t = p.put("a".into(), vec![], vec![q()]);
        std::thread::sleep(Duration::from_millis(60));
        assert!(p.take(&t).is_none());
    }

    #[test]
    fn fifty_threads_one_winner() {
        let p = Arc::new(PendingStore::new());
        let t = p.put("a".into(), vec![], vec![q()]);
        let wins = Arc::new(AtomicUsize::new(0));
        let hs: Vec<_> = (0..50)
            .map(|_| {
                let (p, t, w) = (p.clone(), t.clone(), wins.clone());
                std::thread::spawn(move || {
                    if p.take(&t).is_some() {
                        w.fetch_add(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().expect("thread");
        }
        assert_eq!(wins.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn preview_payload_appends_and_caps() {
        let skipped: Vec<Value> = (0..31)
            .map(|i| serde_json::json!({"row": i, "reason": "r"}))
            .collect();
        let plan = serde_json::json!({"sheets": [{"name": "s", "skipped": skipped}], "added": 1});
        let out = preview_payload(plan, "f.xlsx", "tok");
        let s = &out["sheets"][0];
        assert_eq!(s["skipped"].as_array().map(|a| a.len()), Some(30));
        assert_eq!(
            (s["skippedMore"].as_u64(), s["skippedCount"].as_u64()),
            (Some(1), Some(31))
        );
        let keys: Vec<&String> = out.as_object().expect("obj").keys().collect();
        assert_eq!(keys, ["sheets", "added", "filename", "token"]);
    }
}
