//! Running model calls: the instance-wide gate, the per-user registry, and the generation task (spec 4.7, 5.6, 14.1).
//!
//! A generation runs in its own task, independent of the HTTP response: a closed browser does not stop it (the page
//! re-attaches by polling). It ends on the upstream's end, Stop, server shutdown, the 2-s access re-check, a global
//! switch-off, a suspension of the user's AI+ function or a deletion; the text so far is always kept. Exactly one usage
//! row is written per generation, in the same final transaction as the answer; the slots are released after that
//! transaction. Reasoning of the model is never kept: before the first text it only produces one `thinking` frame.
use crate::app::AppState;
use crate::codes::text_of;
use crate::identity::Identity;
use crate::link::{LinkError, SessionUser};
use crate::provider::{self, ms, Up, UpCode, UpError};
use crate::settings::{Format, Provider};
use crate::store::{data, stats};
use crate::study::guard::{HintGuard, Verdict};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

// —— instance-wide gate ——————————————————————————————————————————————————————————————————————————————————————————————

#[derive(Default)]
struct GateState {
    size: usize,
    /// Permits to forget as running calls finish (after the size was lowered below the number in use)
    debt: usize,
}

/// At most `globalRunning` upstream calls at once; resizable at runtime without cancelling running calls.
pub struct Gate {
    sem: Arc<Semaphore>,
    state: Mutex<GateState>,
}

/// One upstream call's place in the gate.
pub struct GatePermit {
    permit: Option<OwnedSemaphorePermit>,
    gate: Arc<Gate>,
}

impl Drop for GatePermit {
    fn drop(&mut self) {
        let mut st = self.gate.lock();
        if let Some(p) = self.permit.take() {
            if st.debt > 0 {
                st.debt -= 1;
                p.forget();
            } else {
                drop(p);
            }
        }
    }
}

impl Gate {
    pub fn new(size: usize) -> Arc<Gate> {
        Arc::new(Gate {
            sem: Arc::new(Semaphore::new(size)),
            state: Mutex::new(GateState { size, debt: 0 }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, GateState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A place, or `None` when every place is taken (503 `ai.serverBusy`).
    pub fn try_acquire(self: &Arc<Self>) -> Option<GatePermit> {
        let _st = self.lock();
        let p = self.sem.clone().try_acquire_owned().ok()?;
        Some(GatePermit {
            permit: Some(p),
            gate: self.clone(),
        })
    }

    /// Changes the number of places. Raising adds places at once; lowering removes free places at once and the rest
    /// as running calls finish.
    pub fn resize(&self, size: usize) {
        let mut st = self.lock();
        if size > st.size {
            let mut add = size - st.size;
            let paid = add.min(st.debt);
            st.debt -= paid;
            add -= paid;
            if add > 0 {
                self.sem.add_permits(add);
            }
        } else if size < st.size {
            let remove = st.size - size;
            let forgotten = self.sem.forget_permits(remove);
            st.debt += remove - forgotten;
        }
        st.size = size;
    }

    /// Free places right now.
    pub fn available(&self) -> usize {
        self.sem.available_permits()
    }

    pub fn size(&self) -> usize {
        self.lock().size
    }
}

// —— per-user registry ———————————————————————————————————————————————————————————————————————————————————————————————

/// Why a running call is being stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The user pressed Stop
    Stop,
    /// The server is shutting down
    Shutdown,
    /// The access re-check said no (AI+ off, user disabled, signed out, password change)
    NotAllowed,
    /// The access re-check failed three times in a row
    BackendDown,
    /// AI was switched off globally
    Disabled,
    /// The chat was deleted (by the user or an administrator)
    ChatDeleted,
    /// The user's AI data is being removed (deleted user, reused id)
    UserRemoved,
    /// An administrator suspended the user's AI+ function (AI settings)
    Suspended,
}

impl StopReason {
    /// The stored error code, or `None` for a plain stop (status `stopped`).
    pub fn code(self) -> Option<&'static str> {
        match self {
            StopReason::Stop | StopReason::Shutdown => None,
            StopReason::NotAllowed | StopReason::UserRemoved => Some("ai.notAllowed"),
            StopReason::BackendDown => Some("ai.backendDown"),
            StopReason::Disabled => Some("ai.disabled"),
            StopReason::ChatDeleted => Some("ai.notFound"),
            StopReason::Suspended => Some("ai.suspended"),
        }
    }
}

/// What a slot is used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Chat,
    Suggest,
}

struct Entry {
    user: i64,
    kind: Kind,
    chat: Option<i64>,
    message: i64,
    seq: u64,
    cancel: watch::Sender<Option<StopReason>>,
    done: watch::Receiver<bool>,
}

#[derive(Default)]
struct RegInner {
    next: u64,
    entries: HashMap<u64, Entry>,
}

/// Running model calls per user (chat answers and memory suggestions).
#[derive(Default)]
pub struct Registry {
    inner: Mutex<RegInner>,
}

/// A reserved place of a user. Dropping it removes the entry and signals the end to anyone waiting for it.
pub struct Slot {
    reg: Arc<Registry>,
    id: u64,
    pub cancel: watch::Receiver<Option<StopReason>>,
    done: watch::Sender<bool>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.reg.lock().entries.remove(&self.id);
        let _ = self.done.send(true);
    }
}

impl Slot {
    /// Names the chat and the answer once they exist.
    pub fn set_target(&self, chat: i64, message: i64) {
        if let Some(e) = self.reg.lock().entries.get_mut(&self.id) {
            e.chat = Some(chat);
            e.message = message;
        }
    }

    /// The stop reason, if one was given.
    pub fn stopped(&self) -> Option<StopReason> {
        *self.cancel.borrow()
    }
}

impl Registry {
    fn lock(&self) -> std::sync::MutexGuard<'_, RegInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Reserves a place for `user` (at most `per_user` at once; never two in the same chat). `None` → 409 `ai.busy`.
    pub fn reserve(
        self: &Arc<Self>,
        user: i64,
        kind: Kind,
        chat: Option<i64>,
        per_user: usize,
    ) -> Option<Slot> {
        let mut inner = self.lock();
        let mine = inner.entries.values().filter(|e| e.user == user).count();
        let same_chat = chat.is_some()
            && inner
                .entries
                .values()
                .any(|e| e.user == user && e.chat == chat);
        if mine >= per_user.max(1) || same_chat {
            return None;
        }
        inner.next += 1;
        let id = inner.next;
        let (cancel_tx, cancel_rx) = watch::channel(None);
        let (done_tx, done_rx) = watch::channel(false);
        inner.entries.insert(
            id,
            Entry {
                user,
                kind,
                chat,
                message: 0,
                seq: id,
                cancel: cancel_tx,
                done: done_rx,
            },
        );
        Some(Slot {
            reg: self.clone(),
            id,
            cancel: cancel_rx,
            done: done_tx,
        })
    }

    /// Names the chat of a reserved place before anything is written (the study asks bind it inside their write
    /// transaction): under the registry lock, `false` when another entry of the same user already has that chat, else
    /// the place's entry gets the chat and `true` is returned. A first ask of a session and a second one of the same
    /// session therefore never run in one chat.
    pub fn bind(&self, slot: &Slot, chat: i64) -> bool {
        let mut inner = self.lock();
        let Some(user) = inner.entries.get(&slot.id).map(|e| e.user) else {
            return false;
        };
        if inner
            .entries
            .iter()
            .any(|(id, e)| *id != slot.id && e.user == user && e.chat == Some(chat))
        {
            return false;
        }
        if let Some(e) = inner.entries.get_mut(&slot.id) {
            e.chat = Some(chat);
        }
        true
    }

    /// The user's running answer in `chat` (when given and it has one), else the user's most recent running chat
    /// answer: (chat id, message id).
    pub fn running_in(&self, user: i64, chat: Option<i64>) -> Option<(i64, i64)> {
        if let Some(c) = chat {
            let found = self
                .lock()
                .entries
                .values()
                .find(|e| e.user == user && e.chat == Some(c) && e.message > 0)
                .map(|e| (c, e.message));
            if found.is_some() {
                return found;
            }
        }
        self.running_of(user)
    }

    /// The user's most recent running chat answer: (chat id, message id).
    pub fn running_of(&self, user: i64) -> Option<(i64, i64)> {
        self.lock()
            .entries
            .values()
            .filter(|e| e.user == user && e.kind == Kind::Chat && e.message > 0)
            .max_by_key(|e| e.seq)
            .and_then(|e| e.chat.map(|c| (c, e.message)))
    }

    /// Chats with a running answer.
    pub fn running_chats(&self) -> Vec<i64> {
        self.lock()
            .entries
            .values()
            .filter_map(|e| e.chat)
            .collect()
    }

    pub fn chat_running(&self, chat: i64) -> bool {
        self.lock().entries.values().any(|e| e.chat == Some(chat))
    }

    /// Number of running chat answers.
    pub fn chat_count(&self) -> usize {
        self.lock()
            .entries
            .values()
            .filter(|e| e.kind == Kind::Chat)
            .count()
    }

    /// Asks every matching call to stop; returns how many were asked and the signals of their ends.
    pub fn cancel_where(
        &self,
        reason: StopReason,
        pred: impl Fn(i64, Kind, Option<i64>) -> bool,
    ) -> Vec<watch::Receiver<bool>> {
        let inner = self.lock();
        let mut waits = Vec::new();
        for e in inner.entries.values() {
            if pred(e.user, e.kind, e.chat) {
                e.cancel.send_if_modified(|v| {
                    if v.is_none() {
                        *v = Some(reason);
                        true
                    } else {
                        false
                    }
                });
                waits.push(e.done.clone());
            }
        }
        waits
    }

    /// Stops the matching calls and waits until each has written its final state and released its place.
    pub async fn cancel_and_wait(
        &self,
        reason: StopReason,
        pred: impl Fn(i64, Kind, Option<i64>) -> bool,
    ) -> usize {
        let waits = self.cancel_where(reason, pred);
        let n = waits.len();
        for mut w in waits {
            let _ = w.wait_for(|d| *d).await;
        }
        n
    }
}

// —— generation ——————————————————————————————————————————————————————————————————————————————————————————————————————

/// One frame of the SSE body.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub event: &'static str,
    pub data: Value,
}

/// Everything a generation needs. No `Debug` (the request carries keys).
pub struct Generation {
    pub app: AppState,
    pub slot: Slot,
    pub permit: GatePermit,
    pub who: Identity,
    pub chat: i64,
    pub message: i64,
    pub model: String,
    pub provider: Provider,
    pub format: Format,
    pub request: reqwest::Request,
    pub frames: UnboundedSender<Frame>,
    /// Hints only: the guard that decides what part of the text may be shown and stored
    pub guard: Option<HintGuard>,
    /// `Some("hint" | "explain" | "summary")` for the study answers, `None` for AI Chat
    pub purpose: Option<&'static str>,
}

enum Outcome {
    /// A hint gave the answer away: nothing of it is kept
    Leaked,
    Done,
    Failed(UpError),
    Cancelled(StopReason),
}

async fn join_opt<T>(h: &mut Option<tokio::task::JoinHandle<T>>) -> Option<T> {
    match h {
        Some(handle) => handle.await.ok(),
        None => std::future::pending().await,
    }
}

fn access_ok(who: &Identity, u: &SessionUser) -> bool {
    u.id == who.id && u.stamp == who.stamp && u.ai_allowed
}

/// `true` when the user's AI+ function is suspended now (a database error counts as not suspended: the other checks
/// still run).
fn suspended_now(app: &AppState, user: i64) -> bool {
    matches!(
        app.0.store.read(|c| data::function_enabled(c, user)),
        Ok(false)
    )
}

/// Runs a generation to its end and persists it.
pub async fn run(g: Generation) {
    let Generation {
        app,
        slot,
        permit,
        who,
        chat,
        message,
        model,
        provider,
        format,
        request,
        frames,
        mut guard,
        purpose,
    } = g;
    let t = app.0.timeouts;
    let started = Instant::now();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let client = app.0.client.clone();
    let reader = tokio::spawn(async move {
        provider::stream_chat(&client, request, format, t, tx).await;
    });
    let mut text = String::new();
    let mut flushed = 0usize;
    let mut tokens_in = None;
    let mut tokens_out = None;
    let mut tokens_cached = None;
    let mut truncated = false;
    let mut first_byte_ms = None;
    let mut thinking = false;
    let mut cancel = slot.cancel.clone();
    let mut shutdown = app.0.shutdown.clone();
    let deadline = tokio::time::sleep(t.total);
    tokio::pin!(deadline);
    let start = tokio::time::Instant::now();
    let mut access = tokio::time::interval_at(start + t.access_interval, t.access_interval);
    access.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut flush = tokio::time::interval_at(start + t.flush_interval, t.flush_interval);
    flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut check: Option<tokio::task::JoinHandle<Result<Option<SessionUser>, LinkError>>> = None;
    let mut failures = 0u32;

    let outcome = loop {
        if let Some(r) = *cancel.borrow_and_update() {
            break Outcome::Cancelled(r);
        }
        if *shutdown.borrow_and_update() {
            break Outcome::Cancelled(StopReason::Shutdown);
        }
        tokio::select! {
            biased;
            r = cancel.changed() => {
                if r.is_err() {
                    break Outcome::Cancelled(StopReason::Shutdown);
                }
            }
            r = shutdown.changed() => {
                if r.is_err() {
                    break Outcome::Cancelled(StopReason::Shutdown);
                }
            }
            _ = &mut deadline => break Outcome::Failed(UpError::new(UpCode::Timeout, None)),
            item = rx.recv() => match item {
                Some(Up::Text(s)) => {
                    if first_byte_ms.is_none() {
                        first_byte_ms = Some(ms(started.elapsed()));
                    }
                    match guard.as_mut() {
                        None => {
                            text.push_str(&s);
                            let _ = frames.send(Frame { event: "delta", data: json!({ "text": s }) });
                        }
                        // A guarded hint: `text` (the flush, the stored answer) only ever holds released text
                        Some(gd) => match gd.push(&s) {
                            Verdict::Leak => break Outcome::Leaked,
                            Verdict::Release(r) => {
                                if !r.is_empty() {
                                    text.push_str(&r);
                                    let _ = frames.send(Frame { event: "delta", data: json!({ "text": r }) });
                                }
                                if gd.is_truncated() {
                                    truncated = true;
                                    break Outcome::Done;
                                }
                            }
                        },
                    }
                }
                // Reasoning before the first text: one `thinking` frame, so the page can say "Thinking…"
                Some(Up::Thinking) => {
                    if !thinking && text.is_empty() {
                        thinking = true;
                        let _ = frames.send(Frame { event: "thinking", data: json!({}) });
                    }
                }
                Some(Up::Usage { input, output }) => {
                    if input.is_some() { tokens_in = input; }
                    if output.is_some() { tokens_out = output; }
                }
                Some(Up::Cached(n)) => tokens_cached = Some(n),
                Some(Up::Truncated) => truncated = true,
                Some(Up::Done) => break Outcome::Done,
                Some(Up::Fail(e)) => break Outcome::Failed(e),
                None => break Outcome::Failed(UpError::new(UpCode::Protocol, None)),
            },
            res = join_opt(&mut check), if check.is_some() => {
                check = None;
                match res {
                    Some(Ok(Some(u))) if access_ok(&who, &u) => {
                        failures = 0;
                        // A suspension stops the answer at once (the suspend route cancels it); this catches a
                        // suspension written in a gap of that
                        if !who.is_admin() && suspended_now(&app, who.id) {
                            break Outcome::Cancelled(StopReason::Suspended);
                        }
                    }
                    Some(Ok(_)) => break Outcome::Cancelled(StopReason::NotAllowed),
                    _ => {
                        failures += 1;
                        if failures >= 3 {
                            break Outcome::Cancelled(StopReason::BackendDown);
                        }
                    }
                }
            }
            _ = access.tick() => {
                if check.is_none() {
                    let link = app.0.link.clone();
                    let sid = who.sid.clone();
                    let id = who.id;
                    check = Some(tokio::spawn(async move { link.introspect(&sid, id).await }));
                }
            }
            _ = flush.tick() => {
                if text.len() != flushed {
                    let r = app.0.store.write(|tx| data::flush_answer(tx, message, &text));
                    if let Err(e) = r {
                        tracing::warn!("could not save a running AI answer: {e}");
                    } else {
                        flushed = text.len();
                    }
                }
            }
        }
    };
    reader.abort();
    if let Some(h) = check.take() {
        h.abort();
    }
    let mut outcome = outcome;
    // The final check of a guarded hint runs on every outcome, before anything is stored
    if let Some(gd) = guard.as_mut() {
        if !matches!(outcome, Outcome::Leaked) {
            match gd.finish() {
                Verdict::Release(tail) => {
                    if !tail.is_empty() {
                        text.push_str(&tail);
                        let _ = frames.send(Frame {
                            event: "delta",
                            data: json!({ "text": tail }),
                        });
                    }
                }
                Verdict::Leak => outcome = Outcome::Leaked,
            }
        }
    }
    if matches!(outcome, Outcome::Leaked) {
        text.clear();
        truncated = false;
    }

    let (status, code, usage_status, usage_code): (&str, Option<&str>, &str, Option<String>) =
        match &outcome {
            Outcome::Leaked => (
                "error",
                Some("ai.hintLeak"),
                "error",
                Some("hintLeak".to_string()),
            ),
            Outcome::Done => ("done", None, "ok", None),
            Outcome::Failed(e) => (
                "error",
                Some(e.code.code()),
                "error",
                Some(e.code.short().to_string()),
            ),
            Outcome::Cancelled(r) => match r.code() {
                None => ("stopped", None, "stopped", None),
                Some(c) => ("error", Some(c), "stopped", None),
            },
        };
    let now = crate::util::unix_now();
    let usage = stats::UsageRow {
        user_id: Some(who.id),
        at: now,
        kind: "chat",
        provider: provider.as_str(),
        model: model.clone(),
        tokens_in,
        tokens_out,
        tokens_cached,
        latency_ms: ms(started.elapsed()),
        first_byte_ms,
        status: usage_status,
        error_code: usage_code,
    };
    let saved = app.0.store.write(|tx| {
        data::finish_answer(tx, message, &text, status, code, truncated, now)?;
        data::touch_chat(tx, chat, now)?;
        stats::insert_usage_purpose(tx, &usage, purpose)
    });
    if let Err(e) = saved {
        tracing::error!("could not save the end of an AI answer: {e}");
    }
    match &outcome {
        Outcome::Failed(e) => app.0.failures.note(&who.username, e.code.code(), e.http),
        Outcome::Leaked => app.0.failures.note_leak(&who.username),
        _ => {}
    }
    let frame = match (&outcome, code) {
        (Outcome::Leaked, _) => Frame {
            event: "error",
            data: json!({"status": "error", "code": "ai.hintLeak", "error": text_of("ai.hintLeak"), "withdraw": true}),
        },
        (Outcome::Done, _) => Frame {
            event: "done",
            data: json!({"status": "done", "tokensIn": tokens_in, "tokensOut": tokens_out, "truncated": truncated}),
        },
        (_, None) => Frame {
            event: "stopped",
            data: json!({"status": "stopped"}),
        },
        (_, Some(c)) => Frame {
            event: "error",
            data: json!({"status": "error", "code": c, "error": text_of(c)}),
        },
    };
    let _ = frames.send(frame);
    drop(frames);
    // Release the places only after the final transaction (deletions wait for this)
    drop(permit);
    drop(slot);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_resizes_without_cancelling() {
        let g = Gate::new(3);
        let a = g.try_acquire().expect("a");
        let b = g.try_acquire().expect("b");
        let c = g.try_acquire().expect("c");
        assert!(g.try_acquire().is_none());
        // lower to 1 while three run: nothing is cancelled, no new place until two have finished
        g.resize(1);
        assert_eq!(g.size(), 1);
        drop(a);
        assert!(g.try_acquire().is_none());
        drop(b);
        assert!(g.try_acquire().is_none());
        drop(c);
        let d = g.try_acquire().expect("one place");
        assert!(g.try_acquire().is_none());
        // raise to 4: three more places at once
        g.resize(4);
        let more: Vec<_> = (0..3).map(|_| g.try_acquire().expect("place")).collect();
        assert!(g.try_acquire().is_none());
        drop(more);
        drop(d);
        assert_eq!(g.available(), 4);
    }

    #[test]
    fn gate_lowered_then_raised_before_calls_finish() {
        let g = Gate::new(2);
        let a = g.try_acquire().expect("a");
        let b = g.try_acquire().expect("b");
        g.resize(1);
        g.resize(3);
        // the debt was cancelled by the raise: one place is free at once
        let c = g.try_acquire().expect("c");
        assert!(g.try_acquire().is_none());
        drop((a, b, c));
        assert_eq!(g.available(), 3);
    }

    #[tokio::test]
    async fn registry_reserve_cancel_and_wait() {
        let reg = Arc::new(Registry::default());
        let s1 = reg.reserve(1, Kind::Chat, Some(10), 1).expect("first");
        assert!(
            reg.reserve(1, Kind::Chat, None, 1).is_none(),
            "one per user"
        );
        assert!(
            reg.reserve(1, Kind::Suggest, None, 2).is_some(),
            "two allowed"
        );
        assert!(
            reg.reserve(1, Kind::Chat, Some(10), 4).is_none(),
            "never two in one chat"
        );
        s1.set_target(10, 99);
        assert_eq!(reg.running_of(1), Some((10, 99)));
        assert!(reg.chat_running(10));
        let mut cancel = s1.cancel.clone();
        let waiter = {
            let reg = reg.clone();
            tokio::spawn(async move {
                reg.cancel_and_wait(StopReason::ChatDeleted, |u, _, c| u == 1 && c == Some(10))
                    .await
            })
        };
        cancel.wait_for(|v| v.is_some()).await.expect("cancelled");
        assert_eq!(s1.stopped(), Some(StopReason::ChatDeleted));
        assert!(!waiter.is_finished());
        drop(s1);
        assert_eq!(waiter.await.expect("join"), 1);
        assert!(!reg.chat_running(10));
        assert_eq!(StopReason::Stop.code(), None);
        assert_eq!(StopReason::Disabled.code(), Some("ai.disabled"));
    }
}
