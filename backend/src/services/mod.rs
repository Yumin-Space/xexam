//! Business flows. **Interface contract** (v0.5 extended it: targets, `apply_import` returns [`AppliedImport`]).
//!
//! Import flow (conceptual functions from the spec; names copied verbatim for easy cross-reference):
//! `parse_upload → validate_import → plan_import → create_pending_import → confirm_pending_import | cancel_pending_import → apply_import`
//!
//! The preview stage **never writes to the database**; only `apply_import` after `confirm_pending_import` writes, in a single transaction.
//!
//! v0.5: every upload has an [`ImportTarget`] (header `X-Bank-Target`; none = the legacy bank). The target travels with the pending
//! import, so the preview and the write are computed against the same bank. The `*_for` functions take the target; the original
//! ones keep their v0.4 meaning (legacy target).
//!
//! v0.6.5: bank managers (plain users with the permission) import too, so every user who uploads gets their own pending slot
//! (keyed by user id): one manager's upload never replaces, confirms or cancels another user's preview. The `*_as` functions
//! take the uploader; the original ones use slot 0 (no signed-in user: the legacy API of the tests).
//!
//! v0.7: only the six-column layout may be imported. [`check_layout`] refuses a workbook from which the parser read a
//! question of the 3- or 4-column layout ([`MSG_OLD_LAYOUT`], 400), before anything is staged or written; the upload route,
//! the import confirm and the `import` command all call it. The parser still recognises the old layouts (that is what tells
//! them apart). [`OldLayouts::AllowForTests`] lets the golden suites replay the reference implementation's 3-column imports:
//! it is a field of the API state that only test code sets, never an environment variable, flag or setting.
use crate::config::{PENDING_TTL_SECS, SKIP_SHOWN};
use crate::error::{AppError, DbError};
use crate::models::{DeletedBank, ImportTarget, ParsedSheet, ParsedWorkbook, Question, Viewer};
use crate::security;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

pub mod engine;
pub use engine::{Engine, RealEngine};

/// The staged upload (only the most recent one is kept).
pub struct PendingImport {
    pub token: String,
    pub filename: String,
    pub sheets: Vec<ParsedSheet>,
    pub questions: Vec<Question>,
    pub expires: Instant,
    /// The bank the preview was computed against and the confirm writes to.
    pub target: ImportTarget,
    /// v0.6.5: the user who uploaded it (0 = no signed-in user); only they can confirm or cancel it.
    pub user: i64,
    /// v0.7: the uploader's username, for the log ("" = no signed-in user)
    pub username: String,
    /// v0.7: the estimated memory it holds (see [`estimated_bytes`]), counted against [`MAX_PENDING_BYTES`]
    pub bytes: usize,
}

/// v0.7: upper limit on the estimated memory of all staged uploads together. Every administrator and bank manager has a slot,
/// and one parsed workbook near the cell limits holds more than 100 MB until it is confirmed, cancelled or expired, so the
/// slots share this budget: staging an upload drops the oldest staged uploads of other users while the total is above it
/// (their confirm then answers that the preview has expired, as after the time limit). The newest upload is always kept, so
/// a single upload is limited only by the parser's own limits.
pub const MAX_PENDING_BYTES: usize = 128 * 1024 * 1024;

/// A rough measure of the memory a staged upload holds: the length of its questions and sheets as JSON (the text is most of
/// it). Computed without building the JSON.
pub fn estimated_bytes(sheets: &[ParsedSheet], questions: &[Question]) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(buf.len());
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut n = Count(0);
    // Serialising plain data into a counter cannot fail; a failure would only make the estimate smaller
    let _ = serde_json::to_writer(&mut n, questions);
    let _ = serde_json::to_writer(&mut n, sheets);
    n.0
}

/// One slot per uploading user (before v0.6.5: one global slot). `take` empties a slot as it removes its content; a wrong or
/// expired token returns None and **does not consume** the slot's content (except expired ones: as in the old version, an
/// expired one cannot be taken out). Expired previews of other users are dropped whenever a new upload is staged, so at most
/// one parsed workbook per active uploader is kept in memory, and all of them together stay within a memory budget
/// ([`MAX_PENDING_BYTES`]).
pub struct PendingStore {
    slots: Mutex<HashMap<i64, PendingImport>>,
    ttl: Duration,
    budget: usize,
}

impl PendingStore {
    pub fn new() -> Self {
        Self::with_ttl(Duration::from_secs(PENDING_TTL_SECS))
    }
    pub fn with_ttl(ttl: Duration) -> Self {
        Self::with_limits(ttl, MAX_PENDING_BYTES)
    }
    /// A store with its own time limit and memory budget (for tests).
    pub fn with_limits(ttl: Duration, budget: usize) -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
            ttl,
            budget,
        }
    }
    fn slots(&self) -> std::sync::MutexGuard<'_, HashMap<i64, PendingImport>> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// Stages an upload for the legacy bank. See [`PendingStore::put_for`].
    pub fn put(
        &self,
        filename: String,
        sheets: Vec<ParsedSheet>,
        questions: Vec<Question>,
    ) -> String {
        self.put_for(ImportTarget::Legacy, filename, sheets, questions)
    }
    /// Stages an upload for `target` in slot 0 (replacing the previous one) and returns its token.
    pub fn put_for(
        &self,
        target: ImportTarget,
        filename: String,
        sheets: Vec<ParsedSheet>,
        questions: Vec<Question>,
    ) -> String {
        self.put_as(0, target, filename, sheets, questions)
    }
    /// [`PendingStore::put_named`] without a username (the log then says "an upload without a signed-in user").
    pub fn put_as(
        &self,
        user: i64,
        target: ImportTarget,
        filename: String,
        sheets: Vec<ParsedSheet>,
        questions: Vec<Question>,
    ) -> String {
        self.put_named(user, "", target, filename, sheets, questions)
    }
    /// Stages an upload of `user` (whose username is `username`, kept for the log) for `target`, replacing that user's
    /// previous one, and returns its token. Expired uploads of other users are dropped, and then the oldest uploads of other
    /// users while all of them together are above the budget.
    pub fn put_named(
        &self,
        user: i64,
        username: &str,
        target: ImportTarget,
        filename: String,
        sheets: Vec<ParsedSheet>,
        questions: Vec<Question>,
    ) -> String {
        let token = security::new_token();
        let bytes = estimated_bytes(&sheets, &questions);
        let now = Instant::now();
        let item = PendingImport {
            token: token.clone(),
            filename,
            sheets,
            questions,
            expires: now + self.ttl,
            target,
            user,
            username: username.to_string(),
            bytes,
        };
        let mut g = self.slots();
        g.retain(|_, it| it.expires >= now);
        g.insert(user, item);
        let mut total: usize = g.values().map(|it| it.bytes).fold(0, usize::saturating_add);
        while total > self.budget {
            // The oldest upload of another user (all share the same time limit, so the earliest expiry is the oldest)
            let Some((&oldest, it)) = g
                .iter()
                .filter(|(u, _)| **u != user)
                .min_by_key(|(_, it)| it.expires)
            else {
                break;
            };
            total = total.saturating_sub(it.bytes);
            let whose = if it.username.is_empty() {
                "the staged upload without a signed-in user".to_string()
            } else {
                format!("the staged upload of user {:?}", it.username)
            };
            g.remove(&oldest);
            tracing::info!(
                "dropped {whose}: staged uploads would hold more than {} MB",
                self.budget / (1024 * 1024)
            );
        }
        token
    }
    /// Takes the staged upload whose token is `token`, whoever uploaded it (the legacy API; the routes use [`Self::take_as`]).
    pub fn take(&self, token: &str) -> Option<PendingImport> {
        let user = self
            .slots()
            .iter()
            .find(|(_, it)| security::token_eq(&it.token, token))
            .map(|(u, _)| *u)?;
        self.take_as(user, token)
    }
    /// Takes `user`'s staged upload if `token` is its token. Another user's token never matches.
    pub fn take_as(&self, user: i64, token: &str) -> Option<PendingImport> {
        let mut g = self.slots();
        let it = g.get(&user)?;
        if it.expires < Instant::now() {
            g.remove(&user); // clear on expiry: behaviorally indistinguishable from the old version (kept but not retrievable)
            return None;
        }
        if security::token_eq(&it.token, token) {
            g.remove(&user)
        } else {
            None
        }
    }
    /// Drops every staged upload.
    pub fn drop_pending(&self) {
        self.slots().clear();
    }
    /// Drops `user`'s staged upload (cancel).
    pub fn drop_of(&self, user: i64) {
        self.slots().remove(&user);
    }
    /// Drops every staged upload that targets the bank that was just deleted (its preview was computed against that bank):
    /// an upload for that bank id, or an upload without a target when the deleted bank was the legacy bank.
    /// Returns whether something was dropped.
    pub fn drop_for_deleted_bank(&self, deleted: &DeletedBank) -> bool {
        let mut g = self.slots();
        let before = g.len();
        g.retain(|_, it| match it.target {
            ImportTarget::Bank(id) => id != deleted.id,
            ImportTarget::Legacy => !deleted.legacy,
            ImportTarget::New => true,
        });
        g.len() != before
    }
}

impl Default for PendingStore {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------- import flow

pub const MSG_PARSE_TIMEOUT: &str =
    "解析这个文件用的时间太长，已经停止。请确认选对了文件，或者换一份小一点的再试。";
/// Number of uploads parsed at the same time. One parse takes up to several hundred MB, so the total is limited here (in real use only one runs at a time).
pub const PARSE_CONCURRENCY: usize = 2;
pub const MSG_NO_QUESTIONS: &str = "这份文件里没有读到任何可用的题，没有导入。";
pub const MSG_UNEXPECTED: &str = "读取这个 Excel 时出错：文件内容不符合预期";
pub const MSG_APPLY_FAILED: &str = "写入数据库失败，题库没有任何改动。";
/// v0.7 (400): the workbook is in the old 3- or 4-column layout; only the six-column layout may be imported.
pub const MSG_OLD_LAYOUT: &str = "只能导入六列格式（A–C 列：英文、中文、日文题目；D–F 列：英文、中文、日文答案）。这个文件是旧的三列或四列格式，请改成六列格式后再导入。";
/// The English reason logged when [`MSG_OLD_LAYOUT`] is answered.
pub const LOG_OLD_LAYOUT: &str =
    "the workbook uses the old 3- or 4-column layout; only the six-column layout can be imported";

/// Whether the import entry points accept workbooks of the old 3- / 4-column layouts (v0.7: they do not).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OldLayouts {
    /// Refuse them with [`MSG_OLD_LAYOUT`] (production: the only value `AppState::new` and the `import` command use)
    #[default]
    Refuse,
    /// TEST-ONLY: accept them, so the golden suites can replay the reference implementation's 3-column imports through
    /// the API. Only test code sets it (a field of the API state); no environment variable, flag or setting can.
    AllowForTests,
}

/// v0.7: refuses a workbook from which the parser read a question of the 3- or 4-column layout (see
/// [`ParsedWorkbook::has_old_layout`]) with 400 [`MSG_OLD_LAYOUT`], unless `rule` allows it (tests only). Logs the English
/// reason with the file name.
pub fn check_layout(
    questions: &[Question],
    rule: OldLayouts,
    filename: &str,
) -> Result<(), AppError> {
    if rule == OldLayouts::Refuse && questions.iter().any(|q| !q.is_six_column()) {
        tracing::info!("import of {filename:?} refused: {LOG_OLD_LAYOUT}");
        return Err(AppError::BadRequest(MSG_OLD_LAYOUT.to_string()));
    }
    Ok(())
}

/// Upload content check + parse (old `check_xlsx` + `parse_workbook`). The synchronous parser goes into `spawn_blocking`,
/// with an overall time limit (`XEXAM_PARSE_TIMEOUT_SECS`): on timeout it returns 400 with an explanatory message.
///
/// The timeout is **cooperative**: the parser checks the deadline after each batch of rows / events, stops on its own when it is reached and returns `ImportError::Timeout`,
/// so the background thread does not keep occupying CPU and memory after a timeout. The outer `tokio::time::timeout` is only a safety net (half a second later than the deadline),
/// covering the steps the parser cannot check midway (opening the zip, verifying the whole archive by decompressing it).
/// At most `PARSE_CONCURRENCY` parses run at the same time; the permit is held by the background thread itself until it really finishes.
pub async fn parse_upload(
    engine: Arc<dyn Engine>,
    gate: Arc<Semaphore>,
    data: Vec<u8>,
    timeout: Duration,
) -> Result<ParsedWorkbook, AppError> {
    let deadline = Instant::now() + timeout;
    let fallback = timeout + Duration::from_millis(500);
    let work = async {
        // waiting in the queue for a permit also counts toward the overall time limit
        let permit = gate.acquire_owned().await.map_err(|_| AppError::Internal)?;
        let task = tokio::task::spawn_blocking(move || {
            let r = engine.check_and_parse_until(&data, deadline);
            drop(permit); // the permit is returned only once the thread has really finished
            r
        });
        Ok::<_, AppError>(task.await)
    };
    let joined = match tokio::time::timeout(fallback, work).await {
        Err(_) => {
            tracing::warn!("upload parse did not finish within {:?}", fallback);
            return Err(AppError::BadRequest(MSG_PARSE_TIMEOUT.to_string()));
        }
        Ok(Err(e)) => return Err(e),
        Ok(Ok(j)) => j,
    };
    match joined {
        Err(join) => {
            tracing::error!("upload parser task failed: {join}");
            Err(AppError::BadRequest(MSG_UNEXPECTED.to_string()))
        }
        Ok(Err(e)) => {
            tracing::info!("upload rejected: {}", e.log_reason());
            Err(e.into())
        }
        Ok(Ok(wb)) => Ok(wb),
    }
}

/// Validation on confirm: nothing is imported if there are no usable questions (the token has already been taken at this point, as in the old version).
pub fn validate_import(item: &PendingImport) -> Result<(), AppError> {
    if item.questions.is_empty() {
        return Err(AppError::BadRequest(MSG_NO_QUESTIONS.to_string()));
    }
    Ok(())
}

/// Preview against the legacy bank: read-only, does not write to the database. Returns `(plan, the parse result handed back as is)`.
pub async fn plan_import(
    engine: Arc<dyn Engine>,
    wb: ParsedWorkbook,
) -> Result<(Value, ParsedWorkbook), AppError> {
    let (plan, wb) = plan_import_for(engine, ImportTarget::Legacy, wb).await?;
    Ok((plan.ok_or(AppError::Internal)?, wb))
}

/// Preview against `target`: read-only. The plan is `None` when the target bank does not exist.
pub async fn plan_import_for(
    engine: Arc<dyn Engine>,
    target: ImportTarget,
    wb: ParsedWorkbook,
) -> Result<(Option<Value>, ParsedWorkbook), AppError> {
    let joined = tokio::task::spawn_blocking(move || {
        let r = engine.plan_import_for(target, &wb.sheets, &wb.questions);
        (r, wb)
    })
    .await
    .map_err(|_| DbError::Join)?;
    let (r, wb) = joined;
    Ok((r?, wb))
}

/// Old `preview_payload`: lists at most 30 "unreadable" reasons per sheet and appends `skippedMore`, `skippedCount`; appends `filename`, `token` at the end.
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
                    // Six-column warnings get the same cap and counters, only on sheets that have some
                    if let Some(Value::Array(warned)) = so.get("warnings").cloned() {
                        let total = warned.len();
                        let shown: Vec<Value> = warned.into_iter().take(SKIP_SHOWN).collect();
                        so.insert("warnings".to_string(), Value::Array(shown));
                        so.insert(
                            "warningsMore".to_string(),
                            Value::from(total.saturating_sub(SKIP_SHOWN)),
                        );
                        so.insert("warningsCount".to_string(), Value::from(total));
                    }
                }
            }
        }
        map.insert("filename".to_string(), Value::from(filename));
        map.insert("token".to_string(), Value::from(token));
    }
    plan
}

/// Puts the parse result into the single pending-confirmation slot (replacing the previous one) and returns the preview payload for the frontend.
pub fn create_pending_import(
    pending: &PendingStore,
    filename: String,
    wb: ParsedWorkbook,
    plan: Value,
) -> Value {
    create_pending_import_for(pending, ImportTarget::Legacy, filename, wb, plan)
}

/// [`create_pending_import`] for an upload with an explicit target.
pub fn create_pending_import_for(
    pending: &PendingStore,
    target: ImportTarget,
    filename: String,
    wb: ParsedWorkbook,
    plan: Value,
) -> Value {
    create_pending_import_as(pending, 0, target, filename, wb, plan)
}

/// [`create_pending_import_for`] in the slot of `user` (the signed-in uploader).
pub fn create_pending_import_as(
    pending: &PendingStore,
    user: i64,
    target: ImportTarget,
    filename: String,
    wb: ParsedWorkbook,
    plan: Value,
) -> Value {
    create_pending_import_named(pending, user, "", target, filename, wb, plan)
}

/// [`create_pending_import_as`] with the uploader's username (kept for the log).
pub fn create_pending_import_named(
    pending: &PendingStore,
    user: i64,
    username: &str,
    target: ImportTarget,
    filename: String,
    wb: ParsedWorkbook,
    plan: Value,
) -> Value {
    let ParsedWorkbook { sheets, questions } = wb;
    let token = pending.put_named(user, username, target, filename.clone(), sheets, questions);
    preview_payload(plan, &filename, &token)
}

/// Takes it out (one time) and validates. Wrong / expired / already-handled token → 409; no questions → 400.
pub fn confirm_pending_import(
    pending: &PendingStore,
    token: &str,
) -> Result<PendingImport, AppError> {
    let item = pending.take(token).ok_or(AppError::PendingGone)?;
    validate_import(&item)?;
    Ok(item)
}

/// [`confirm_pending_import`] of `user`'s own staged upload (another user's token answers 409 like a wrong one).
pub fn confirm_pending_import_as(
    pending: &PendingStore,
    user: i64,
    token: &str,
) -> Result<PendingImport, AppError> {
    let item = pending.take_as(user, token).ok_or(AppError::PendingGone)?;
    validate_import(&item)?;
    Ok(item)
}

/// Cancel does not need a token (as in the old version). Drops every staged upload (the legacy API).
pub fn cancel_pending_import(pending: &PendingStore) {
    pending.drop_pending();
}

/// Cancel of the signed-in `user`: drops only their own staged upload.
pub fn cancel_pending_import_as(pending: &PendingStore, user: i64) {
    pending.drop_of(user);
}

/// What a confirmed import wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedImport {
    /// Same shape as the preview plan
    pub result: Value,
    /// `/api/bank` after the write
    pub bank: Value,
    /// The bank that was created / updated
    pub bank_id: i64,
}

/// Actually writes the confirmed upload into its target bank (one transaction), then reads back the full question bank. The synchronous
/// `Store` goes into `spawn_blocking`. `Ok(None)` = the target bank was deleted after the preview (nothing written).
/// (v0.5: returns [`AppliedImport`] instead of `(result, bank)`, to report the bank id and a deleted target.)
pub async fn apply_import(
    engine: Arc<dyn Engine>,
    item: PendingImport,
) -> Result<Option<AppliedImport>, DbError> {
    tokio::task::spawn_blocking(move || {
        let Some((result, bank_id)) =
            engine.apply_import_for(item.target, &item.filename, &item.sheets, &item.questions)?
        else {
            return Ok(None);
        };
        let bank = engine.bank()?;
        Ok(Some(AppliedImport {
            result,
            bank,
            bank_id,
        }))
    })
    .await
    .map_err(|_| DbError::Join)?
}

/// [`apply_import`] as the signed-in `viewer`: a bank it creates is owned by them, an update needs that they may manage the
/// bank (`Err(DbError::NotBankOwner)` otherwise, nothing written), and the returned `/api/bank` is the one they see.
pub async fn apply_import_as(
    engine: Arc<dyn Engine>,
    viewer: Viewer,
    item: PendingImport,
) -> Result<Option<AppliedImport>, DbError> {
    tokio::task::spawn_blocking(move || {
        let Some((result, bank_id)) = engine.apply_import_as(
            &viewer,
            item.target,
            &item.filename,
            &item.sheets,
            &item.questions,
        )?
        else {
            return Ok(None);
        };
        // A new bank of a bank manager is visible to them from now on: read their scope again
        let bank = engine.bank_as(&Viewer {
            scope: refreshed_scope(&viewer, bank_id),
            ..viewer.clone()
        })?;
        Ok(Some(AppliedImport {
            result,
            bank,
            bank_id,
        }))
    })
    .await
    .map_err(|_| DbError::Join)?
}

/// The scope of `viewer` after an import into `bank_id`: a bank manager sees the banks they own, so a bank they just created
/// joins their scope (an administrator, or a user who sees every bank, already sees it).
fn refreshed_scope(viewer: &Viewer, bank_id: i64) -> crate::models::BankScope {
    use crate::models::BankScope;
    match &viewer.scope {
        BankScope::All => BankScope::All,
        BankScope::Only(ids) => {
            let mut ids = ids.clone();
            if viewer.can_manage {
                ids.insert(bank_id);
            }
            BankScope::Only(ids)
        }
    }
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
            english: String::new(),
            en: None,
            lang: String::new(),
            ja: None,
            explanation: String::new(),
            mnemonic: String::new(),
            note: String::new(),
            explain: None,
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
    fn targets_travel_with_the_pending_import() {
        let p = PendingStore::new();
        let t = p.put("a".into(), vec![], vec![q()]);
        assert_eq!(p.take(&t).map(|it| it.target), Some(ImportTarget::Legacy));
        let t = p.put_for(ImportTarget::Bank(7), "b".into(), vec![], vec![q()]);
        assert_eq!(p.take(&t).map(|it| it.target), Some(ImportTarget::Bank(7)));
    }

    #[test]
    fn deleting_a_bank_drops_only_the_upload_that_targets_it() {
        let p = PendingStore::new();
        let gone = |id, legacy| DeletedBank {
            id,
            legacy,
            name: format!("Demo bank {id}"),
        };
        // explicit bank id
        let t = p.put_for(ImportTarget::Bank(7), "b".into(), vec![], vec![q()]);
        assert!(!p.drop_for_deleted_bank(&gone(8, false)));
        assert!(!p.drop_for_deleted_bank(&gone(8, true)));
        assert!(p.drop_for_deleted_bank(&gone(7, false)));
        assert!(p.take(&t).is_none());
        // a new bank is never affected
        let t = p.put_for(ImportTarget::New, "n".into(), vec![], vec![q()]);
        assert!(!p.drop_for_deleted_bank(&gone(1, true)));
        assert!(p.take(&t).is_some());
        // no header = the legacy bank
        let t = p.put("l".into(), vec![], vec![q()]);
        assert!(!p.drop_for_deleted_bank(&gone(1, false)));
        assert!(p.drop_for_deleted_bank(&gone(1, true)));
        assert!(p.take(&t).is_none());
        assert!(!p.drop_for_deleted_bank(&gone(1, true)), "empty slot");
    }

    #[test]
    fn staged_uploads_share_a_memory_budget() {
        let one = estimated_bytes(&[], &[q()]);
        // room for two uploads of one question, not three
        let p = PendingStore::with_limits(Duration::from_secs(60), one * 2 + one / 2);
        let t1 = p.put_as(1, ImportTarget::New, "a".into(), vec![], vec![q()]);
        std::thread::sleep(Duration::from_millis(5));
        let t2 = p.put_as(2, ImportTarget::New, "b".into(), vec![], vec![q()]);
        std::thread::sleep(Duration::from_millis(5));
        let t3 = p.put_as(3, ImportTarget::New, "c".into(), vec![], vec![q()]);
        assert!(p.take_as(1, &t1).is_none(), "the oldest upload was dropped");
        assert!(p.take_as(2, &t2).is_some());
        assert!(p.take_as(3, &t3).is_some());
        // a single upload above the budget is still staged (the parser's own limits bound it)
        let p = PendingStore::with_limits(Duration::from_secs(60), 1);
        let t1 = p.put_as(1, ImportTarget::New, "a".into(), vec![], vec![q()]);
        assert!(p.take_as(1, &t1).is_some());
        // a user's new upload replaces their own one, which no longer counts
        let p = PendingStore::with_limits(Duration::from_secs(60), one * 2 + one / 2);
        let t1 = p.put_as(1, ImportTarget::New, "a".into(), vec![], vec![q()]);
        let t2 = p.put_as(2, ImportTarget::New, "b".into(), vec![], vec![q()]);
        let t2b = p.put_as(2, ImportTarget::New, "b2".into(), vec![], vec![q()]);
        assert!(p.take_as(2, &t2).is_none());
        assert!(p.take_as(1, &t1).is_some(), "two uploads fit");
        assert!(p.take_as(2, &t2b).is_some());
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
