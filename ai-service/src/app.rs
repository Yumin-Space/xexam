//! The service state shared by the routes and the background tasks.
use crate::chat::{Gate, Kind, Registry, StopReason};
use crate::codes::ApiError;
use crate::config::{Config, Timeouts};
use crate::identity::Identity;
use crate::limits::Limits;
use crate::link::{Link, LogLine};
use crate::logs::{FailureLog, Throttle};
use crate::provider::{ModelList, Target};
use crate::secret::{self, KeySource, LoadError, MasterKey, SecretName};
use crate::settings::{fingerprint, KeyRevs, Provider, Settings};
use crate::store::tables::{self, SecretRow};
use crate::store::{data, stats, Store};
use crate::study::cache::RecordCache;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::{watch, Notify};
use zeroize::Zeroizing;

/// Time limit of the last state report, sent while shutting down.
const STOPPING_REPORT: Duration = Duration::from_secs(2);

/// Why the saved keys cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyProblem {
    /// A saved key was sealed with another encryption key, or does not open
    Mismatch,
    /// `XEXAM_AI_SECRET_KEY` is not a valid key
    InvalidEnv,
    /// The key file is not valid (or could not be created)
    InvalidFile,
}

impl KeyProblem {
    pub fn as_str(self) -> &'static str {
        match self {
            KeyProblem::Mismatch => "mismatch",
            KeyProblem::InvalidEnv => "invalidEnv",
            KeyProblem::InvalidFile => "invalidFile",
        }
    }
}

/// The settings with what is known about the keys, and the fingerprint.
pub struct Snapshot {
    pub settings: Settings,
    pub secrets: Vec<SecretRow>,
    pub revs: KeyRevs,
    pub fp: String,
    pub key_problem: Option<KeyProblem>,
}

impl Snapshot {
    /// Effective AI state (2.3 rule 4).
    pub fn effective(&self) -> bool {
        self.settings.enabled
            && self.settings.verified_fp.as_deref() == Some(self.fp.as_str())
            && self.key_problem.is_none()
    }

    pub fn verified(&self) -> bool {
        self.settings.verified_at.is_some()
            && self.settings.verified_fp.as_deref() == Some(self.fp.as_str())
    }

    /// A model and the keys the provider needs are saved.
    pub fn configured(&self) -> bool {
        !self.settings.model.is_empty() && self.missing_key().is_none()
    }

    /// The first required key that is not saved.
    pub fn missing_key(&self) -> Option<SecretName> {
        self.settings
            .provider
            .required_key()
            .filter(|k| self.revs.get(*k).is_none())
    }
}

/// Models listed recently (10 minutes per fingerprint, in memory only).
pub struct ModelsCache {
    pub fp: String,
    pub at: Instant,
    pub list: ModelList,
}

/// The service. No `Debug`.
pub struct App {
    pub version: String,
    pub timeouts: Timeouts,
    pub data_dir: PathBuf,
    pub store: Store,
    pub link: Arc<Link>,
    master: Option<MasterKey>,
    load_problem: Option<KeyProblem>,
    mismatch: Mutex<bool>,
    pub client: reqwest::Client,
    pub registry: Arc<Registry>,
    pub gate: Arc<Gate>,
    limits: RwLock<Limits>,
    pub models_cache: Mutex<Option<ModelsCache>>,
    pub shutdown: watch::Receiver<bool>,
    shutdown_tx: watch::Sender<bool>,
    pub identity_log: Throttle,
    pub failures: FailureLog,
    /// Which chat a saved record belongs to (the study routes; never holds text)
    pub records: RecordCache,
    /// At most one line a minute: a data answer of the backend that could not be read
    pub unreadable_log: Throttle,
    /// Test hook of the study asks (see [`StudyHold`]); `None` in production
    study_hold: Mutex<Option<StudyHold>>,
    /// AI Chat retries per answer message: a retry rewrites the failed answer in place, so the database holds no count
    /// of the attempts (see [`App::next_retry_attempt`])
    retries: Mutex<std::collections::HashMap<i64, u32>>,
    /// Per-user call caps that were reached and have not freed yet: (user id, cap key) -> when it frees (unix s). One log
    /// line per entry (see [`App::note_call_cap`]); expired entries are pruned on every use.
    cap_episodes: Mutex<std::collections::HashMap<(i64, &'static str), i64>>,
    /// The job places of AI bank generation and import (`genRunning`); a job holds one for its whole writing phase
    pub gen_gate: Arc<Gate>,
    /// Running and queued generation / import jobs of this process (cancel signals)
    pub jobs: crate::gen::runner::Jobs,
    /// One extraction at a time, server-wide (FIFO)
    pub extract_gate: Arc<tokio::sync::Semaphore>,
    /// Uploads held in memory while they wait for or are in extraction (`IMPORT_PENDING_MAX`)
    pub import_pending: Arc<tokio::sync::Semaphore>,
    /// Users with an upload being taken right now (SEC-7)
    pub uploading: Mutex<std::collections::HashSet<i64>>,
    /// The binary the extraction child is started from (`App::import_exe`)
    exe: PathBuf,
    /// Timers of the jobs (tests lower them)
    pub knobs: Knobs,
    /// What the provider taught about the token limit for one settings fingerprint (the field it accepts, the ceiling
    /// of an Anthropic model); a changed fingerprint starts again, a restart forgets it
    token_memo: Mutex<Option<(String, crate::provider::TokenMemo)>>,
    /// The most questions per call a provider really delivered for one settings fingerprint (L-44: answers were cut
    /// below the token setting); memory only, forgotten with the token memo
    gen_per_call: Mutex<Option<(String, usize)>>,
    /// Held for a whole outbox delivery, so two never overlap (each would send the same audit lines)
    outbox: tokio::sync::Mutex<()>,
    /// Held while one state report is read and sent, so an older state never arrives after a newer one
    heartbeat: tokio::sync::Mutex<()>,
    /// Wakes the heartbeat task for a report at once
    heartbeat_signal: Arc<Notify>,
}

/// Uploads kept in memory at once (IX-1).
pub const IMPORT_PENDING_MAX: usize = 4;

/// The timers of generation and import jobs (seconds). The defaults are the binding numbers; tests lower them.
pub struct Knobs {
    /// Wait for the user's confirm of a recognised import (IJ-3: 30 minutes)
    pub confirm_secs: std::sync::atomic::AtomicU64,
    /// From the upload to the end of recognising (20 minutes)
    pub reading_secs: std::sync::atomic::AtomicU64,
    /// A job's writing time, from the start / the confirm (30 minutes)
    pub writing_secs: std::sync::atomic::AtomicU64,
    /// The extraction child is killed after this many seconds (30)
    pub kill_secs: std::sync::atomic::AtomicU64,
    /// Percent of the binding backoff after provider errors (100 = 15 s x errors in a row, at most 60 s)
    pub backoff_scale: std::sync::atomic::AtomicU64,
    /// Milliseconds between two tries for a spare model-call place (2000)
    pub place_poll_ms: std::sync::atomic::AtomicU64,
    /// Test hook: the model context (tokens) the whole-file import fits its prefix into; 0 = the model listing's figure,
    /// else 128 000 (A-OWNER-71)
    pub import_context_tokens: std::sync::atomic::AtomicU64,
    /// Test hook: the extraction child hangs (debug builds only)
    pub hang_child: std::sync::atomic::AtomicBool,
    /// Test hook: the binary the extraction child is started from (integration tests pass the service's own binary)
    pub exe_override: Mutex<Option<PathBuf>>,
}

impl Default for Knobs {
    fn default() -> Knobs {
        use std::sync::atomic::{AtomicBool, AtomicU64};
        Knobs {
            confirm_secs: AtomicU64::new(1800),
            reading_secs: AtomicU64::new(1200),
            writing_secs: AtomicU64::new(1800),
            kill_secs: AtomicU64::new(crate::extract::child::EXTRACT_KILL_SECS),
            backoff_scale: AtomicU64::new(100),
            place_poll_ms: AtomicU64::new(2000),
            import_context_tokens: AtomicU64::new(0),
            hang_child: AtomicBool::new(false),
            exe_override: Mutex::new(None),
        }
    }
}

impl Knobs {
    pub fn get(a: &std::sync::atomic::AtomicU64) -> u64 {
        a.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Test hook: while set, every study ask waits after it took its places and before its write transaction, until the
/// test adds a permit to `release` (each permit lets one ask go on). Each waiting ask first adds a permit to `arrived`.
/// Used to hold the second of two concurrent first asks of one session.
#[doc(hidden)]
#[derive(Clone)]
pub struct StudyHold {
    pub arrived: Arc<tokio::sync::Semaphore>,
    pub release: Arc<tokio::sync::Semaphore>,
}

/// Cheap handle to the service.
#[derive(Clone)]
pub struct AppState(pub Arc<App>);

impl AppState {
    /// The requests of one counted model call (chat, study ask, memory suggestion): the answer length of the limits
    /// as `max_tokens`, in the field this settings fingerprint is known to accept,
    /// plus the fallbacks of the ladder (spec 3.10 C-8). The limits are read now, when the call starts.
    pub fn chat_plan(
        &self,
        snap: &Snapshot,
        target: &Target,
        spec: &crate::provider::ChatSpec<'_>,
    ) -> Result<crate::provider::Plan, crate::provider::UpError> {
        let want = self.0.limits().reply_max_tokens;
        self.token_plan(snap, target, spec, want)
            .map(|(plan, _)| plan)
    }

    /// The first token limit a call of `want` tokens starts with (from the settings and what the provider taught), the
    /// same inputs `token_plan` uses. A generation takes its batch size from it.
    pub fn first_token_limit(&self, snap: &Snapshot, want: i64) -> crate::provider::TokenLimit {
        let format = snap.settings.wire_format();
        let stored_max = snap.settings.max_tokens;
        let memo = {
            let m = self.0.token_memo.lock().unwrap_or_else(|e| e.into_inner());
            m.as_ref().filter(|(fp, _)| *fp == snap.fp).map(|(_, m)| *m)
        };
        crate::provider::first_token_limit(format, want, stored_max, memo)
    }

    /// The batch ceiling of a generation: `Some(first.value)` when the first step has a token field and a value below
    /// `want` (Anthropic with a learned cap), else `None` (GP-1).
    pub fn token_ceiling(&self, snap: &Snapshot, want: i64) -> Option<usize> {
        let first = self.first_token_limit(snap, want);
        (first.field != crate::provider::TokenField::Omit && first.value < want)
            .then(|| usize::try_from(first.value).unwrap_or(0))
    }

    /// The requests of one model call of `want` tokens, with the plan and its first token limit (`chat_plan` is this with
    /// the answer length of the limits).
    pub fn token_plan(
        &self,
        snap: &Snapshot,
        target: &Target,
        spec: &crate::provider::ChatSpec<'_>,
        want: i64,
    ) -> Result<(crate::provider::Plan, crate::provider::TokenLimit), crate::provider::UpError>
    {
        let format = snap.settings.wire_format();
        let stored_max = snap.settings.max_tokens;
        let memo = {
            let m = self.0.token_memo.lock().unwrap_or_else(|e| e.into_inner());
            m.as_ref().filter(|(fp, _)| *fp == snap.fp).map(|(_, m)| *m)
        };
        let first = crate::provider::first_token_limit(format, want, stored_max, memo);
        let app = self.clone();
        let fp = snap.fp.clone();
        let remember: crate::provider::Remember = Arc::new(move |step| {
            app.0.remember_token_step(&fp, format, step);
        });
        let plan = crate::provider::build_plan(
            target,
            &self.0.client,
            spec,
            first,
            stored_max,
            Some(remember),
        )?;
        Ok((plan, first))
    }
}

impl App {
    /// Remembers the token-limit step a provider accepted after refusing the earlier one, once per fingerprint and step
    /// (one INFO line, English, no key, URL or content).
    fn remember_token_step(
        &self,
        fp: &str,
        format: crate::settings::Format,
        step: &crate::provider::TokenLimit,
    ) {
        use crate::provider::{TokenField, TokenMemo};
        let (memo, line) = match (format, step.field) {
            (crate::settings::Format::Anthropic, _) => (
                TokenMemo {
                    field: TokenField::MaxTokens,
                    cap: Some(step.value),
                },
                format!(
                    "the AI provider refused max_tokens; using max_tokens {}",
                    step.value
                ),
            ),
            (_, TokenField::MaxCompletionTokens) => (
                TokenMemo {
                    field: TokenField::MaxCompletionTokens,
                    cap: None,
                },
                "the AI provider refused max_tokens; using max_completion_tokens".to_string(),
            ),
            (_, TokenField::Omit) => (
                TokenMemo {
                    field: TokenField::Omit,
                    cap: None,
                },
                "the AI provider refused max_completion_tokens; sending no token limit".to_string(),
            ),
            (_, TokenField::MaxTokens) => return,
        };
        let mut m = self.token_memo.lock().unwrap_or_else(|e| e.into_inner());
        let same = m.as_ref().is_some_and(|(f, old)| f == fp && *old == memo);
        if !same {
            *m = Some((fp.to_string(), memo));
            tracing::info!("{line}");
        }
    }

    /// Forgets what a settings fingerprint learned about the token limit, so the next call starts again from the first step
    /// (used when the admin changes the answer length: the earlier refusal may have been about the old value).
    pub fn forget_token_memo(&self) {
        *self.token_memo.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.gen_per_call.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Remembers that this settings fingerprint delivers at most `n` questions per call (keeps the smaller value).
    pub fn note_gen_per_call(&self, fp: &str, n: usize) {
        let mut m = self.gen_per_call.lock().unwrap_or_else(|e| e.into_inner());
        let n = n.max(1);
        match m.as_mut() {
            Some((f, old)) if f == fp => {
                if n < *old {
                    *old = n;
                    tracing::info!(
                        "the AI provider cuts long answers; asking {n} questions per call"
                    );
                }
            }
            _ => {
                *m = Some((fp.to_string(), n));
                tracing::info!("the AI provider cuts long answers; asking {n} questions per call");
            }
        }
    }

    /// The remembered questions per call of this fingerprint, if any.
    pub fn gen_per_call_cap(&self, fp: &str) -> Option<usize> {
        let m = self.gen_per_call.lock().unwrap_or_else(|e| e.into_inner());
        m.as_ref().filter(|(f, _)| f == fp).map(|(_, n)| *n)
    }

    /// The attempt number of the next retry of this answer: 2 for the first retry, 3 for the second, and so on (kept in
    /// memory only; a restart starts counting again, and the table is cleared when it grows large).
    pub fn next_retry_attempt(&self, answer: i64) -> u32 {
        let mut m = self.retries.lock().unwrap_or_else(|e| e.into_inner());
        if m.len() >= 10_000 && !m.contains_key(&answer) {
            m.clear();
        }
        let n = m.entry(answer).or_insert(1);
        *n = n.saturating_add(1);
        *n
    }

    /// Opens the service: key, database, link, limits, key state. Logs the startup lines of section 6.3.
    pub fn open(mut cfg: Config) -> Result<AppState, String> {
        let env_key = cfg.secret_key.take();
        let (master, load_problem) = match secret::load_master(
            env_key.as_deref().map(|s| s.as_str()),
            &cfg.data_dir,
        ) {
            Ok((k, src)) => {
                match src {
                    KeySource::Env => tracing::info!("AI encryption key: from XEXAM_AI_SECRET_KEY"),
                    KeySource::File(p) => tracing::info!("AI encryption key: {}", p.display()),
                    KeySource::Created(p) => {
                        tracing::info!("Created a new AI key file {}", p.display())
                    }
                }
                (Some(k), None)
            }
            Err(LoadError::InvalidEnv) => {
                tracing::error!("XEXAM_AI_SECRET_KEY is not a valid key (expected 32 bytes as 64 hex digits or base64); AI is off");
                (None, Some(KeyProblem::InvalidEnv))
            }
            Err(LoadError::InvalidFile(p)) => {
                tracing::error!("The AI key file {} is not valid; AI is off. Move it out of the AI data volume and restart the ai-service container to create a new one, then enter the AI keys again", p.display());
                (None, Some(KeyProblem::InvalidFile))
            }
            Err(LoadError::CannotCreate(p, kind)) => {
                tracing::error!(
                    "The AI key file {} could not be created ({kind}); AI is off",
                    p.display()
                );
                (None, Some(KeyProblem::InvalidFile))
            }
        };
        drop(env_key);
        let store = Store::open(&cfg.data_dir, cfg.journal)?;
        let link = Arc::new(Link::new(&mut cfg)?);
        let limits = store
            .read(tables::load_limits)
            .map_err(|e| format!("cannot read the AI limits: {e}"))?;
        let client = crate::provider::provider_client(&cfg.version, &cfg.timeouts)?;
        let (shutdown_tx, shutdown) = watch::channel(false);
        let app = App {
            version: cfg.version.clone(),
            timeouts: cfg.timeouts,
            data_dir: cfg.data_dir.clone(),
            store,
            link,
            master,
            load_problem,
            mismatch: Mutex::new(false),
            client,
            registry: Arc::new(Registry::default()),
            gate: Gate::new(Limits::usize_of(limits.global_running)),
            gen_gate: Gate::new(Limits::usize_of(limits.gen_running)),
            jobs: crate::gen::runner::Jobs::default(),
            extract_gate: Arc::new(tokio::sync::Semaphore::new(1)),
            import_pending: Arc::new(tokio::sync::Semaphore::new(IMPORT_PENDING_MAX)),
            uploading: Mutex::new(std::collections::HashSet::new()),
            exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("xexam-ai-service")),
            knobs: Knobs::default(),
            limits: RwLock::new(limits),
            models_cache: Mutex::new(None),
            shutdown,
            shutdown_tx,
            identity_log: Throttle::new(Duration::from_secs(60)),
            failures: FailureLog::default(),
            records: RecordCache::default(),
            unreadable_log: Throttle::new(Duration::from_secs(60)),
            study_hold: Mutex::new(None),
            retries: Mutex::new(std::collections::HashMap::new()),
            cap_episodes: Mutex::new(std::collections::HashMap::new()),
            token_memo: Mutex::new(None),
            gen_per_call: Mutex::new(None),
            outbox: tokio::sync::Mutex::new(()),
            heartbeat: tokio::sync::Mutex::new(()),
            heartbeat_signal: Arc::new(Notify::new()),
        };
        app.check_keys_at_start()?;
        let app = AppState(Arc::new(app));
        // Jobs cut off by a restart (before anything can start a new one)
        crate::gen::runner::repair_at_start(&app);
        Ok(app)
    }

    /// Key state at start (3.4) and rule 4 of 2.3: a stored `enabled = 1` that is not effective is written as 0 once.
    fn check_keys_at_start(&self) -> Result<(), String> {
        let rows = self
            .store
            .read(tables::load_secrets)
            .map_err(|e| format!("cannot read the AI database: {e}"))?;
        if self.master.is_some() && !self.rows_open(&rows) {
            *self.lock_mismatch() = true;
            self.write_mismatch_off();
        }
        let snap = self
            .snapshot()
            .map_err(|e| format!("cannot read the AI database: {e}"))?;
        if snap.settings.enabled && !snap.effective() {
            let r = self.store.write(|tx| {
                let mut s = tables::load_settings(tx)?;
                s.enabled = false;
                tables::save_settings(tx, &s)
            });
            if let Err(e) = r {
                tracing::warn!("could not switch AI off at start: {e}");
            }
        }
        Ok(())
    }

    fn lock_mismatch(&self) -> std::sync::MutexGuard<'_, bool> {
        self.mismatch.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn rows_open(&self, rows: &[SecretRow]) -> bool {
        let Some(m) = &self.master else {
            return rows.is_empty();
        };
        rows.iter()
            .all(|r| r.key_id == m.key_id() && m.open(r.name, &r.nonce, &r.ciphertext).is_ok())
    }

    /// Writes `enabled = 0` and clears the verified state, logs the mismatch line once.
    fn write_mismatch_off(&self) {
        let r = self.store.write(|tx| {
            let mut s = tables::load_settings(tx)?;
            let was = s.enabled;
            s.enabled = false;
            s.verified_at = None;
            s.verified_fp = None;
            tables::save_settings(tx, &s)?;
            Ok::<bool, rusqlite::Error>(was)
        });
        tracing::error!("The saved AI keys cannot be decrypted: the AI encryption key was changed or lost. AI is off until an administrator enters the keys again");
        match r {
            Ok(true) => self.cancel_everything(StopReason::Disabled),
            Ok(false) => {}
            Err(e) => tracing::warn!("could not switch AI off: {e}"),
        }
        // The effective state changed (a key problem): report it soon, without waiting here
        self.notify_heartbeat();
    }

    /// The key state now.
    pub fn key_problem(&self) -> Option<KeyProblem> {
        if let Some(p) = self.load_problem {
            return Some(p);
        }
        (*self.lock_mismatch()).then_some(KeyProblem::Mismatch)
    }

    /// Recomputes the key state after a secret write.
    pub fn recheck_keys(&self, rows: &[SecretRow]) {
        if self.master.is_some() {
            *self.lock_mismatch() = !self.rows_open(rows);
        }
    }

    /// The current encryption key id ("" without a key).
    pub fn key_id(&self) -> &str {
        self.master.as_ref().map_or("", |m| m.key_id())
    }

    pub fn master(&self) -> Option<&MasterKey> {
        self.master.as_ref()
    }

    /// Settings, keys and fingerprint, read now.
    pub fn snapshot(&self) -> rusqlite::Result<Snapshot> {
        let (settings, secrets) = self
            .store
            .read(|c| Ok((tables::load_settings(c)?, tables::load_secrets(c)?)))?;
        let revs = tables::key_revs(&secrets);
        let fp = fingerprint(&settings, revs, self.key_id());
        Ok(Snapshot {
            settings,
            secrets,
            revs,
            fp,
            key_problem: self.key_problem(),
        })
    }

    /// The current limits.
    pub fn limits(&self) -> Limits {
        *self.limits.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Replaces the limits in memory and resizes the gate (the database row is written by the caller).
    pub fn set_limits(&self, l: Limits) {
        *self.limits.write().unwrap_or_else(|e| e.into_inner()) = l;
        self.gate.resize(Limits::usize_of(l.global_running));
        self.gen_gate.resize(Limits::usize_of(l.gen_running));
    }

    /// Decrypts the saved keys into a request target. A key that does not open switches AI off (`ai.keyProblem`).
    pub fn target(&self, snap: &Snapshot) -> Result<Target, ApiError> {
        if snap.key_problem.is_some() {
            return Err(ApiError::key_problem());
        }
        if let Some(k) = snap.missing_key() {
            return Err(ApiError::key_missing(k));
        }
        let mut api_key: Option<Zeroizing<String>> = None;
        let mut gateway_key: Option<Zeroizing<String>> = None;
        let uses_gateway = snap.settings.provider == Provider::Gateway;
        for row in &snap.secrets {
            if row.name == SecretName::GatewayKey && !uses_gateway {
                continue;
            }
            let opened = match &self.master {
                Some(m) if row.key_id == m.key_id() => {
                    m.open(row.name, &row.nonce, &row.ciphertext).ok()
                }
                _ => None,
            };
            let Some(plain) = opened else {
                let was_ok = !std::mem::replace(&mut *self.lock_mismatch(), true);
                if was_ok {
                    self.write_mismatch_off();
                }
                return Err(ApiError::key_problem());
            };
            match row.name {
                SecretName::ApiKey => api_key = Some(plain),
                SecretName::GatewayKey => gateway_key = Some(plain),
            }
        }
        Ok(Target {
            provider: snap.settings.provider,
            format: snap.settings.wire_format(),
            base: snap.settings.base_url.clone(),
            gateway_auth: snap.settings.gateway_auth,
            api_key,
            gateway_key,
        })
    }

    /// Stops every running call at once (global switch-off; 2.3 rule 6). Logs the count when there were any.
    pub fn cancel_everything(&self, reason: StopReason) {
        let n = self.registry.cancel_where(reason, |_, _, _| true).len();
        if n > 0 && reason == StopReason::Disabled {
            tracing::info!("AI was switched off: {n} running answers were stopped");
        }
    }

    /// Removes all AI data of a user (deleted, or an id now used by another account): stops and awaits the user's
    /// running calls first, then deletes the row (cascades; usage rows become anonymous).
    pub async fn purge_user(&self, id: i64) -> bool {
        // The user's generation or import job stops first and writes nothing more (the tasks rows cascade below)
        self.jobs
            .cancel_user(id, crate::gen::runner::StopWhy::Purged)
            .await;
        self.registry
            .cancel_and_wait(StopReason::UserRemoved, |u, _, _| u == id)
            .await;
        let r = self.store.write(|tx| data::delete_user(tx, id));
        match r {
            Ok(removed) => {
                if removed {
                    self.store.schedule_checkpoint();
                    // The user's AI+ function flag went with the row: the next report must not name the id
                    self.notify_heartbeat();
                }
                removed
            }
            Err(e) => {
                tracing::warn!("could not remove AI data of a user: {e}");
                false
            }
        }
    }

    /// Checks the verified identity against the AI `users` table (5.1, 7.4): another stamp → the old data is removed
    /// before the request is served; a new username is stored.
    pub async fn reconcile_identity(&self, who: &Identity) -> Result<(), ApiError> {
        let row = self
            .store
            .read(|c| data::user_row(c, who.id))
            .map_err(|_| ApiError::internal())?;
        match row {
            Some((stamp, _)) if stamp != who.stamp => {
                self.purge_user(who.id).await;
                tracing::info!(
                    "Removed the AI data of a user id that now belongs to another account (id {})",
                    who.id
                );
            }
            Some((_, name)) if name != who.username => {
                let _ = self
                    .store
                    .write(|tx| data::rename_user(tx, who.id, &who.username));
            }
            _ => {}
        }
        Ok(())
    }

    /// The binary the extraction child is started from: this process's own executable (a test may point elsewhere).
    pub fn import_exe(&self) -> PathBuf {
        self.knobs
            .exe_override
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| self.exe.clone())
    }

    /// `true` while the main backend accepted a state report less than 60 s ago (contracts 2.6).
    pub fn link_fresh(&self) -> bool {
        self.link.report_fresh()
    }

    /// AI+ is available: the AI+ service is effectively on and the main backend accepted a report less than 60 s ago.
    pub fn available(&self, snap: &Snapshot) -> bool {
        snap.effective() && self.link_fresh()
    }

    /// `true` when an administrator suspended the caller's AI+ function (never for administrators). A database error
    /// counts as suspended (no AI use without a known state).
    pub fn suspended(&self, who: &Identity) -> bool {
        if who.is_admin() {
            return false;
        }
        match self.store.read(|c| data::function_enabled(c, who.id)) {
            Ok(on) => !on,
            Err(e) => {
                tracing::warn!("could not read the AI+ function of a user: {e}");
                true
            }
        }
    }

    /// `false` when the feature is switched off for the user (administrators always have every feature). A database error
    /// counts as off (no AI use without a known state), like [`App::suspended`].
    pub fn feature_allowed(&self, who: &Identity, f: crate::features::Feature) -> bool {
        if who.is_admin() {
            return true;
        }
        match self.store.read(|c| data::features_off(c, who.id)) {
            Ok(off) => crate::features::allowed(off, f),
            Err(e) => {
                tracing::warn!("could not read the AI features of a user: {e}");
                false
            }
        }
    }

    /// The state report for the main backend (`POST /internal/ai/heartbeat`, contracts 2.2): keys in this order. When
    /// the database cannot be read, the report says the AI+ service is off.
    pub fn report(&self, stopping: bool) -> Value {
        let state = self.snapshot().and_then(|s| {
            let ids = self
                .store
                .read(|c| data::suspended_ids(c, crate::link::MAX_IDS))?;
            Ok((s.effective(), ids))
        });
        let (enabled, suspended) = state.unwrap_or((false, Vec::new()));
        json!({"version": self.version, "enabled": enabled, "suspended": suspended, "stopping": stopping})
    }

    /// Sends one state report now and returns whether the backend accepted it. One report at a time: the state is
    /// read and sent under one lock. Bounded by `Timeouts.backend_total`. Nothing is sent once shutting down (the
    /// stopping report is the last one).
    pub async fn heartbeat_now(&self) -> bool {
        let _one = self.heartbeat.lock().await;
        if *self.shutdown.borrow() {
            return false;
        }
        let report = self.report(false);
        // Shutting down while the report is in flight drops it, so the stopping report is not stuck behind the lock.
        let mut down = self.shutdown.clone();
        let send = tokio::time::timeout(self.timeouts.backend_total, self.link.heartbeat(report));
        tokio::select! {
            r = send => matches!(r, Ok(Ok(()))),
            _ = down.wait_for(|d| *d) => false,
        }
    }

    /// The last state report, sent while shutting down (`"stopping": true`; at most 2 s, failures ignored).
    pub async fn heartbeat_stopping(&self) {
        let send = async {
            let _one = self.heartbeat.lock().await;
            let report = self.report(true);
            let _ = self.link.heartbeat(report).await;
        };
        let _ = tokio::time::timeout(STOPPING_REPORT, send).await;
    }

    /// Asks the heartbeat task for a report at once (does not wait).
    pub fn notify_heartbeat(&self) {
        self.heartbeat_signal.notify_one();
    }

    /// The signal the heartbeat task waits for.
    pub fn heartbeat_signal(&self) -> Arc<Notify> {
        self.heartbeat_signal.clone()
    }

    /// Starts shutting down: every running answer ends with `stopped` and is saved.
    pub fn begin_shutdown(&self) -> usize {
        let n = self.registry.chat_count();
        let _ = self.shutdown_tx.send(true);
        n
    }

    /// Sends one audit line synchronously (admin views); `false` when the backend did not accept it.
    pub async fn audit_now(&self, line: &str) -> bool {
        tracing::info!(target: "audit", "{line}");
        self.link
            .log(&[LogLine {
                level: "INFO",
                message: line.to_string(),
            }])
            .await
            .is_ok()
    }

    /// Delivers undelivered audit lines (oldest first) until the outbox is empty or the backend refuses. One delivery
    /// at a time (the admin routes and the background loop both call this): a second caller waits, then finds only the
    /// lines that are still undelivered.
    pub async fn deliver_outbox(&self) -> usize {
        let _one = self.outbox.lock().await;
        let mut sent = 0;
        loop {
            let Ok(rows) = self.store.read(|c| stats::outbox_pending(c, 50)) else {
                return sent;
            };
            if rows.is_empty() {
                return sent;
            }
            let lines: Vec<LogLine> = rows
                .iter()
                .map(|(_, m)| LogLine {
                    level: "INFO",
                    message: m.clone(),
                })
                .collect();
            if self.link.log(&lines).await.is_err() {
                return sent;
            }
            let ids: Vec<i64> = rows.iter().map(|(id, _)| *id).collect();
            if self
                .store
                .write(|tx| stats::outbox_remove(tx, &ids))
                .is_err()
            {
                return sent;
            }
            sent += ids.len();
        }
    }

    /// Tests: holds the study asks at the point described at [`StudyHold`] and returns the two semaphores.
    #[doc(hidden)]
    pub fn hold_study_asks(&self) -> StudyHold {
        let hold = StudyHold {
            arrived: Arc::new(tokio::sync::Semaphore::new(0)),
            release: Arc::new(tokio::sync::Semaphore::new(0)),
        };
        *self.study_hold.lock().unwrap_or_else(|e| e.into_inner()) = Some(hold.clone());
        hold
    }

    /// Tests: stops holding the study asks.
    #[doc(hidden)]
    pub fn release_study_asks(&self) {
        *self.study_hold.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// The point where a study ask waits when a test holds the asks (no effect otherwise).
    pub async fn study_hold_point(&self) {
        let hold = self
            .study_hold
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(h) = hold {
            h.arrived.add_permits(1);
            if let Ok(p) = h.release.acquire().await {
                p.forget();
            }
        }
    }

    /// Writes the one log line of a cap episode (English, username only): the first refusal of a user by a cap, until that
    /// cap frees again. Further refusals in the same episode write nothing.
    pub fn note_call_cap(&self, username: &str, user: i64, hit: &crate::quota::Hit, now: i64) {
        let first = {
            let mut m = self.cap_episodes.lock().unwrap_or_else(|e| e.into_inner());
            m.retain(|_, until| *until > now);
            m.insert((user, hit.cap.key()), hit.retry_at).is_none()
        };
        if first {
            let per = match hit.cap {
                crate::quota::Cap::Minute => "minute",
                crate::quota::Cap::Hour => "hour",
                crate::quota::Cap::Day => "day",
            };
            tracing::info!(
                "user {username:?} reached the AI request cap ({} per {per})",
                hit.limit
            );
        }
    }

    /// The user's running chat answer, if any.
    pub fn running_of(&self, user: i64) -> Option<(i64, i64)> {
        self.registry.running_of(user)
    }

    /// `true` when a chat has a running answer.
    pub fn chat_running(&self, chat: i64) -> bool {
        self.registry.chat_running(chat)
    }

    /// Stops and awaits the answers running in the user's chats matching `chat` (`None` = every chat).
    pub async fn stop_chats(&self, user: i64, chat: Option<i64>) {
        self.registry
            .cancel_and_wait(StopReason::ChatDeleted, |u, k, c| {
                u == user && k == Kind::Chat && (chat.is_none() || c == chat)
            })
            .await;
    }
}
