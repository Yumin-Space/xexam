//! The service state shared by the routes and the background tasks.
use crate::activity::Event;
use crate::chat::{Gate, Kind, Registry, StopReason};
use crate::codes::ApiError;
use crate::config::{Config, Timeouts};
use crate::features::Feature;
use crate::identity::{ClientInfo, Identity};
use crate::limits::Limits;
use crate::link::{Link, LogLine};
use crate::logs::{FailureLog, Throttle};
use crate::provider::{ModelList, Target, UpCode};
use crate::quota;
use crate::route::{KeyState, Pick, Route, Slot, View};
use crate::secret::{self, KeySource, LoadError, MasterKey, SecretName, SecretRef};
use crate::settings::Provider;
use crate::store::tables::{self, SecretRow};
use crate::store::{conn, data, stats, Store};
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

/// The global AI state: what an administrator switched, and whether the keys can be used. Which model serves a call is
/// not part of it (see [`App::route`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    /// The administrators' AI+ switch
    pub enabled: bool,
    /// Global feature switches (bits of `features.rs`): a set bit = off for every normal user
    pub features_off: i64,
    pub retention_days: i64,
    pub key_problem: Option<KeyProblem>,
}

/// Models of one connection listed recently (10 minutes per connection id and revision, in memory only).
pub struct ModelsCache {
    pub at: Instant,
    pub list: ModelList,
}

/// The cache key of [`ModelsCache`]: (connection id, connection revision).
pub type ModelsKey = (i64, i64);

/// How many entries the in-memory memos keep (token ladder, questions per call, model lists).
const MEMO_CAP: usize = 64;

/// Consecutive live `ai.up.auth` answers that mark a connection broken.
const AUTH_STRIKES: u32 = 3;

/// The places of one model call: the registry slot and the instance-wide gate permit.
pub struct Places {
    pub slot: crate::chat::Slot,
    pub permit: crate::chat::GatePermit,
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
    pub models_cache: Mutex<std::collections::HashMap<ModelsKey, ModelsCache>>,
    pub shutdown: watch::Receiver<bool>,
    shutdown_tx: watch::Sender<bool>,
    pub identity_log: Throttle,
    /// At most one line a minute: a request refused because its peer is not in `XEXAM_FRONTEND_PEERS`
    pub peer_log: Throttle,
    /// The addresses allowed to reach this service (`XEXAM_FRONTEND_PEERS`; unset = any)
    frontend_peers: crate::peers::PeerList,
    /// Provider pacing of AI generation and import
    pub pacer: crate::pace::Pacer,
    pub failures: FailureLog,
    /// Which chat a saved record belongs to (the study routes; never holds text)
    pub records: RecordCache,
    /// At most one line a minute: a data answer of the backend that could not be read
    pub unreadable_log: Throttle,
    /// Test hook of the study asks (see [`StudyHold`]); `None` in production
    study_hold: Mutex<Option<StudyHold>>,
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
    /// What the provider taught about the token limit per route key (the field it accepts, the ceiling of an Anthropic
    /// model); a changed connection revision starts again, a restart forgets it
    token_memo: Mutex<std::collections::HashMap<String, crate::provider::TokenMemo>>,
    /// Route keys whose OpenAI-compatible server refused cache markers (content parts with `cache_control`); memory only
    no_markers: Mutex<std::collections::HashSet<String>>,
    /// Route keys whose provider refused structured output (`output_config` / `response_format`); memory only
    no_schema: Mutex<std::collections::HashSet<String>>,
    /// Route keys + thinking strength (`<key>|<family>:<level>`) the provider refused; memory only
    no_effort: Mutex<std::collections::HashSet<String>>,
    /// The most questions per call a provider really delivered per route key (L-44: answers were cut below the token
    /// setting); memory only, forgotten with the token memo
    gen_per_call: Mutex<std::collections::HashMap<String, usize>>,
    /// Consecutive live `ai.up.auth` answers per connection id (reset by any success); memory only
    auth_strikes: Mutex<std::collections::HashMap<i64, u32>>,
    /// Wakes the activity delivery task when an event line was written
    outbox_wake: Arc<Notify>,
    /// Whether the last state report said "paused" (enabled but not effective); `None` before the first report
    last_paused: Mutex<Option<bool>>,
    /// Held for a whole outbox delivery, so two never overlap (each would send the same audit lines)
    outbox: tokio::sync::Mutex<()>,
    /// Held while one state report is read and sent, so an older state never arrives after a newer one
    heartbeat: tokio::sync::Mutex<()>,
    /// Wakes the heartbeat task for a report at once
    heartbeat_signal: Arc<Notify>,
    /// The external IP-location lookup (v1.4): HTTP client, saved settings, per-minute count
    pub geo: crate::geo::GeoState,
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
    /// Percent of the provider pacing spacing and waits (100 = as set; tests use 0 = no pacing delay)
    pub pace_scale: std::sync::atomic::AtomicU64,
    /// Location lookups per wall-clock minute (`geo::GEO_PER_MIN` = 30; tests lower it)
    pub geo_per_minute: std::sync::atomic::AtomicU64,
    /// Milliseconds the location worker asks the backend to wait for work (15 000; tests lower it)
    pub geo_wait_ms: std::sync::atomic::AtomicU64,
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
            pace_scale: AtomicU64::new(100),
            geo_per_minute: AtomicU64::new(crate::geo::GEO_PER_MIN),
            geo_wait_ms: AtomicU64::new(15_000),
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
    /// as `max_tokens`, in the field this route is known to accept, plus the fallbacks of the ladder (spec 3.10 C-8).
    /// The limits are read now, when the call starts.
    pub fn chat_plan(
        &self,
        route: &Route,
        target: &Target,
        spec: &crate::provider::ChatSpec<'_>,
    ) -> Result<crate::provider::Plan, crate::provider::UpError> {
        let want = self.0.limits().reply_max_tokens;
        self.token_plan(route, target, spec, want)
            .map(|(plan, _)| plan)
    }

    /// The first token limit a call of `want` tokens starts with (from the route and what the provider taught), the
    /// same inputs `token_plan` uses. A generation takes its batch size from it.
    pub fn first_token_limit(&self, route: &Route, want: i64) -> crate::provider::TokenLimit {
        let memo = self.0.token_memo_of(&route.key);
        crate::provider::first_token_limit(route.format, want, route.max_tokens, memo)
    }

    /// The batch ceiling of a generation: `Some(first.value)` when the first step has a token field and a value below
    /// `want` (Anthropic with a learned cap), else `None` (GP-1).
    pub fn token_ceiling(&self, route: &Route, want: i64) -> Option<usize> {
        let first = self.first_token_limit(route, want);
        (first.field != crate::provider::TokenField::Omit && first.value < want)
            .then(|| usize::try_from(first.value).unwrap_or(0))
    }

    /// The requests of one model call of `want` tokens, with the plan and its first token limit (`chat_plan` is this with
    /// the answer length of the limits).
    pub fn token_plan(
        &self,
        route: &Route,
        target: &Target,
        spec: &crate::provider::ChatSpec<'_>,
        want: i64,
    ) -> Result<(crate::provider::Plan, crate::provider::TokenLimit), crate::provider::UpError>
    {
        let format = route.format;
        let stored_max = route.max_tokens;
        let first = self.first_token_limit(route, want);
        let app = self.clone();
        let key = route.key.clone();
        let remember: crate::provider::Remember = Arc::new(move |step| {
            app.0.remember_token_step(&key, format, step);
        });
        let no_markers = self.0.no_markers_of(&route.key);
        let app = self.clone();
        let key = route.key.clone();
        let on_no_markers: Arc<dyn Fn() + Send + Sync> =
            Arc::new(move || app.0.remember_no_markers(&key));
        let no_schema = self.0.no_schema_of(&route.key);
        let app = self.clone();
        let key = route.key.clone();
        let on_no_schema: Arc<dyn Fn() + Send + Sync> =
            Arc::new(move || app.0.remember_no_schema(&key));
        let (no_effort, on_no_effort) = match spec.effort {
            Some(e) => {
                // per connection (id + revision), model and level: the route key already holds the model, so one model
                // (or one level) that is rejected does not stop the parameter for the others
                let key = format!("{}|{}", route.key, e.tag());
                let refused = self.0.no_effort_of(&key);
                let app = self.clone();
                let family = e.family();
                let f: Arc<dyn Fn() + Send + Sync> =
                    Arc::new(move || app.0.remember_no_effort(&key, family));
                (refused, Some(f))
            }
            None => (false, None),
        };
        let plan = crate::provider::build_plan(
            target,
            &self.0.client,
            spec,
            first,
            stored_max,
            Some(remember),
            no_markers,
            Some(on_no_markers),
            no_schema,
            Some(on_no_schema),
            no_effort,
            on_no_effort,
        )?;
        Ok((plan, first))
    }
}

impl App {
    /// The token-limit memo of a route key.
    fn token_memo_of(&self, key: &str) -> Option<crate::provider::TokenMemo> {
        let m = self.token_memo.lock().unwrap_or_else(|e| e.into_inner());
        m.get(key).copied()
    }

    /// `true` when this route's server refused cache markers earlier (the requests go out without them).
    fn no_markers_of(&self, key: &str) -> bool {
        let m = self.no_markers.lock().unwrap_or_else(|e| e.into_inner());
        m.contains(key)
    }

    /// Remembers that the server of this route key accepted a request only without cache markers (one INFO line,
    /// English, no key, URL or content). At most [`MEMO_CAP`] keys are kept.
    fn remember_no_markers(&self, key: &str) {
        let mut m = self.no_markers.lock().unwrap_or_else(|e| e.into_inner());
        if m.len() >= MEMO_CAP && !m.contains(key) {
            m.clear();
        }
        if m.insert(key.to_string()) {
            tracing::info!(
                "the AI provider refused cache markers; sending none to this connection"
            );
        }
    }

    /// `true` when this route key + thinking strength was refused earlier (the requests go out without it).
    fn no_effort_of(&self, key: &str) -> bool {
        let m = self.no_effort.lock().unwrap_or_else(|e| e.into_inner());
        m.contains(key)
    }

    /// Remembers that the provider accepted a request only without the thinking strength (one INFO line per key,
    /// English, a static parameter name only: no key, URL, model or content). At most [`MEMO_CAP`] keys are kept.
    fn remember_no_effort(&self, key: &str, family: crate::effort::EffortFamily) {
        let mut m = self.no_effort.lock().unwrap_or_else(|e| e.into_inner());
        if m.len() >= MEMO_CAP && !m.contains(key) {
            m.clear();
        }
        if m.insert(key.to_string()) {
            tracing::info!(
                "the AI provider refused {}; sending no thinking strength to this model",
                family.param()
            );
        }
    }

    /// Forgets the refused thinking strengths (a new choice is tried again).
    pub fn forget_no_effort(&self) {
        self.no_effort
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// `true` when this route's provider refused structured output earlier (the requests go out without it).
    fn no_schema_of(&self, key: &str) -> bool {
        let m = self.no_schema.lock().unwrap_or_else(|e| e.into_inner());
        m.contains(key)
    }

    /// Remembers that the provider of this route key accepted a request only without structured output (one INFO line,
    /// English, no key, URL or content). At most [`MEMO_CAP`] keys are kept.
    fn remember_no_schema(&self, key: &str) {
        let mut m = self.no_schema.lock().unwrap_or_else(|e| e.into_inner());
        if m.len() >= MEMO_CAP && !m.contains(key) {
            m.clear();
        }
        if m.insert(key.to_string()) {
            tracing::info!(
                "the AI provider refused structured output; sending none to this connection"
            );
        }
    }

    /// Remembers the token-limit step a provider accepted after refusing the earlier one, once per route key and step
    /// (one INFO line, English, no key, URL or content). At most [`MEMO_CAP`] keys are kept.
    fn remember_token_step(
        &self,
        key: &str,
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
            (crate::settings::Format::Responses, TokenField::Omit) => (
                TokenMemo {
                    field: TokenField::Omit,
                    cap: None,
                },
                "the AI provider refused max_output_tokens; sending no token limit".to_string(),
            ),
            (_, TokenField::Omit) => (
                TokenMemo {
                    field: TokenField::Omit,
                    cap: None,
                },
                "the AI provider refused max_completion_tokens; sending no token limit".to_string(),
            ),
            (_, TokenField::MaxTokens | TokenField::MaxOutputTokens) => return,
        };
        let mut m = self.token_memo.lock().unwrap_or_else(|e| e.into_inner());
        if m.get(key) != Some(&memo) {
            if m.len() >= MEMO_CAP && !m.contains_key(key) {
                m.clear();
            }
            m.insert(key.to_string(), memo);
            tracing::info!("{line}");
        }
    }

    /// Forgets what the routes learned about the token limit, so the next call starts again from the first step (used
    /// when the admin changes the answer length: the earlier refusal may have been about the old value).
    pub fn forget_token_memo(&self) {
        self.token_memo
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.gen_per_call
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.no_schema
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.forget_no_effort();
    }

    /// Remembers that this route delivers at most `n` questions per call (keeps the smaller value).
    pub fn note_gen_per_call(&self, key: &str, n: usize) {
        let mut m = self.gen_per_call.lock().unwrap_or_else(|e| e.into_inner());
        let n = n.max(1);
        match m.get_mut(key) {
            Some(old) => {
                if n < *old {
                    *old = n;
                    tracing::info!(
                        "the AI provider cuts long answers; asking {n} questions per call"
                    );
                }
            }
            None => {
                if m.len() >= MEMO_CAP {
                    m.clear();
                }
                m.insert(key.to_string(), n);
                tracing::info!("the AI provider cuts long answers; asking {n} questions per call");
            }
        }
    }

    /// The remembered questions per call of this route key, if any.
    pub fn gen_per_call_cap(&self, key: &str) -> Option<usize> {
        let m = self.gen_per_call.lock().unwrap_or_else(|e| e.into_inner());
        m.get(key).copied()
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
        if cfg.ai_user_agent_invalid {
            tracing::warn!("XEXAM_AI_USER_AGENT_NAME is not printable ASCII of at most {} characters (or none); the default User-Agent xexam-ai-service/<version> is sent to AI providers", crate::config::MAX_USER_AGENT);
        }
        let ua = cfg.provider_user_agent();
        let client = crate::provider::provider_client(ua.as_deref(), &cfg.timeouts)?;
        let geo = crate::geo::GeoState::new(&cfg.version, &cfg.timeouts)?;
        geo.set_config(
            store
                .read(crate::store::geo::load)
                .map_err(|e| format!("cannot read the location lookup settings: {e}"))?,
        );
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
            models_cache: Mutex::new(std::collections::HashMap::new()),
            shutdown,
            shutdown_tx,
            identity_log: Throttle::new(Duration::from_secs(60)),
            peer_log: Throttle::new(Duration::from_secs(60)),
            frontend_peers: cfg.frontend_peers.clone(),
            pacer: crate::pace::Pacer::default(),
            failures: FailureLog::default(),
            records: RecordCache::default(),
            unreadable_log: Throttle::new(Duration::from_secs(60)),
            study_hold: Mutex::new(None),
            cap_episodes: Mutex::new(std::collections::HashMap::new()),
            token_memo: Mutex::new(std::collections::HashMap::new()),
            no_markers: Mutex::new(std::collections::HashSet::new()),
            no_schema: Mutex::new(std::collections::HashSet::new()),
            no_effort: Mutex::new(std::collections::HashSet::new()),
            gen_per_call: Mutex::new(std::collections::HashMap::new()),
            auth_strikes: Mutex::new(std::collections::HashMap::new()),
            outbox_wake: Arc::new(Notify::new()),
            last_paused: Mutex::new(None),
            outbox: tokio::sync::Mutex::new(()),
            heartbeat: tokio::sync::Mutex::new(()),
            heartbeat_signal: Arc::new(Notify::new()),
            geo,
        };
        app.check_keys_at_start()?;
        let app = AppState(Arc::new(app));
        // Jobs cut off by a restart (before anything can start a new one)
        crate::gen::runner::repair_at_start(&app);
        Ok(app)
    }

    /// Key state at start (3.4): every saved key must open with the current encryption key, else AI stays off with a key
    /// problem (nothing is rewritten: the switch the administrators set stays as it is).
    fn check_keys_at_start(&self) -> Result<(), String> {
        if self.master.is_some()
            && !self
                .secrets_open()
                .map_err(|e| format!("cannot read the AI database: {e}"))?
        {
            *self.lock_mismatch() = true;
            self.log_mismatch();
        }
        self.snapshot()
            .map_err(|e| format!("cannot read the AI database: {e}"))?;
        Ok(())
    }

    fn lock_mismatch(&self) -> std::sync::MutexGuard<'_, bool> {
        self.mismatch.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Every sealed key of every connection opens with the current encryption key (`true` without a key row).
    fn secrets_open(&self) -> rusqlite::Result<bool> {
        let rows = self.store.read(all_secrets)?;
        let Some(m) = &self.master else {
            return Ok(rows.is_empty());
        };
        Ok(rows
            .iter()
            .all(|(id, r)| m.open(&SecretRef::of_row(*id, r), r).is_ok()))
    }

    fn log_mismatch(&self) {
        tracing::error!("The saved AI keys cannot be decrypted: the AI encryption key was changed or lost. AI is off until an administrator enters the keys again");
    }

    /// A key that does not open was found while serving a call: the key state turns to a mismatch once, the running calls
    /// stop, and the state is reported soon.
    fn note_unreadable(&self) {
        let first = !std::mem::replace(&mut *self.lock_mismatch(), true);
        if first {
            self.log_mismatch();
            self.cancel_everything(StopReason::Disabled);
            self.notify_heartbeat();
        }
    }

    /// The key state now.
    pub fn key_problem(&self) -> Option<KeyProblem> {
        if let Some(p) = self.load_problem {
            return Some(p);
        }
        (*self.lock_mismatch()).then_some(KeyProblem::Mismatch)
    }

    /// Recomputes the key state after a secret write (the administrators' connection routes call it).
    pub fn recheck_keys(&self) {
        if self.master.is_some() {
            match self.secrets_open() {
                Ok(open) => *self.lock_mismatch() = !open,
                Err(e) => tracing::warn!("could not check the saved AI keys: {e}"),
            }
        }
    }

    /// The current encryption key id ("" without a key).
    pub fn key_id(&self) -> &str {
        self.master.as_ref().map_or("", |m| m.key_id())
    }

    pub fn master(&self) -> Option<&MasterKey> {
        self.master.as_ref()
    }

    /// The global switches, read now.
    pub fn snapshot(&self) -> rusqlite::Result<Snapshot> {
        let (settings, off) = self
            .store
            .read(|c| Ok((tables::load_settings(c)?, conn::global_features_off(c)?)))?;
        Ok(Snapshot {
            enabled: settings.enabled,
            features_off: off,
            retention_days: settings.retention_days,
            key_problem: self.key_problem(),
        })
    }

    /// The peers allowed to reach this service (`XEXAM_FRONTEND_PEERS`).
    pub fn frontend_peers(&self) -> &crate::peers::PeerList {
        &self.frontend_peers
    }

    /// The current limits.
    pub fn limits(&self) -> Limits {
        *self.limits.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Reloads the saved location lookup settings from the database into memory (after a save or a delete).
    pub fn reload_geo(&self) -> rusqlite::Result<()> {
        self.geo
            .set_config(self.store.read(crate::store::geo::load)?);
        Ok(())
    }

    /// Replaces the limits in memory and resizes the gate (the database row is written by the caller).
    pub fn set_limits(&self, l: Limits) {
        *self.limits.write().unwrap_or_else(|e| e.into_inner()) = l;
        self.gate.resize(Limits::usize_of(l.global_running));
        self.gen_gate.resize(Limits::usize_of(l.gen_running));
    }

    // —— model routing ————————————————————————————————————————————————————————————————————————————————————————————

    /// The routing tables as they are now: connections, pool, slots and what is known about each connection's keys.
    pub fn routes(&self) -> rusqlite::Result<View> {
        let key_id = self.master.as_ref().map(|m| m.key_id().to_string());
        self.store.read(|c| {
            let conns = conn::connections(c)?;
            let pool = conn::pool(c)?;
            let slots = conn::slots(c)?;
            let efforts = conn::efforts(c)?;
            let mut keys = std::collections::HashMap::new();
            for k in &conns {
                let rows = conn::conn_secrets(c, k.id)?;
                keys.insert(
                    k.id,
                    KeyState {
                        api: rows.iter().any(|r| r.name == SecretName::ApiKey),
                        gateway: rows.iter().any(|r| r.name == SecretName::GatewayKey),
                        readable: key_id
                            .as_deref()
                            .is_some_and(|id| rows.iter().all(|r| r.key_id == id)),
                    },
                );
            }
            Ok(View {
                conns,
                pool,
                slots,
                efforts,
                keys,
            })
        })
    }

    /// pick (an administrator's chat menu) -> the slot's model -> the default slot's model -> `ai.disabled`. Never any
    /// other pool model (owner: a warning only, no automatic switch).
    pub fn route(&self, slot: Slot, pick: Option<&Pick>) -> Result<Route, ApiError> {
        self.routes()?.resolve(slot, pick)
    }

    /// `route` with an administrator's own strength level of the chat (ignored when not valid for the route's family).
    pub fn route_with(
        &self,
        slot: Slot,
        pick: Option<&Pick>,
        level: Option<&str>,
    ) -> Result<Route, ApiError> {
        self.routes()?.resolve_with(slot, pick, level)
    }

    /// `route(slot, None)` succeeds.
    pub fn route_exists(&self, slot: Slot) -> bool {
        match self.routes() {
            Ok(v) => v.exists(slot),
            Err(e) => {
                tracing::warn!("could not read the AI routes: {e}");
                false
            }
        }
    }

    /// The default slot has a model (the admin's "configured").
    pub fn configured(&self) -> bool {
        self.store
            .read(conn::slots)
            .is_ok_and(|s| s[Slot::Default.index()].is_some())
    }

    /// AI+ is on: the switch is on, the keys are usable and at least one of the seven slots resolves.
    pub fn effective(&self) -> rusqlite::Result<bool> {
        let snap = self.snapshot()?;
        if !snap.enabled || snap.key_problem.is_some() {
            return Ok(false);
        }
        Ok(self.routes()?.any())
    }

    /// Decrypts the keys of the route's connection into a request target. A key that does not open turns the key state to
    /// a mismatch (`ai.keyProblem`).
    pub fn target(&self, route: &Route) -> Result<Target, ApiError> {
        if self.key_problem().is_some() {
            return Err(ApiError::key_problem());
        }
        let (row, secrets) = self
            .store
            .read(|c| {
                let Some(row) = conn::connection(c, route.connection_id)? else {
                    return Ok(None);
                };
                let secrets = conn::conn_secrets(c, row.id)?;
                Ok(Some((row, secrets)))
            })?
            .ok_or_else(ApiError::disabled)?;
        let provider = Provider::parse(&row.provider).ok_or_else(ApiError::internal)?;
        let gateway_auth = crate::settings::GatewayAuth::parse(&row.gateway_auth)
            .ok_or_else(ApiError::internal)?;
        let mut api_key: Option<Zeroizing<String>> = None;
        let mut gateway_key: Option<Zeroizing<String>> = None;
        for r in &secrets {
            if r.name == SecretName::GatewayKey && provider != Provider::Gateway {
                continue;
            }
            let opened = self
                .master
                .as_ref()
                .and_then(|m| m.open_text(&SecretRef::of_row(row.id, r), r).ok());
            let Some(plain) = opened else {
                self.note_unreadable();
                return Err(ApiError::key_problem());
            };
            match r.name {
                SecretName::ApiKey => api_key = Some(plain),
                SecretName::GatewayKey => gateway_key = Some(plain),
            }
        }
        if let Some(k) = provider.required_key() {
            let have = match k {
                SecretName::ApiKey => api_key.is_some(),
                SecretName::GatewayKey => gateway_key.is_some(),
            };
            if !have {
                return Err(ApiError::key_missing(k));
            }
        }
        Ok(Target {
            provider,
            format: route.format,
            base: row.base_url,
            gateway_auth,
            api_key,
            gateway_key,
        })
    }

    /// What a finished live call tells about its connection: three `ai.up.auth` answers in a row mark it broken, any
    /// success resets the count, other errors change nothing (a rate limit or a timeout never marks anything).
    pub fn note_outcome(&self, route: &Route, code: Option<UpCode>) {
        let broke = {
            let mut m = self.auth_strikes.lock().unwrap_or_else(|e| e.into_inner());
            match code {
                None => {
                    m.remove(&route.connection_id);
                    false
                }
                Some(UpCode::Auth) => {
                    let n = m.entry(route.connection_id).or_insert(0);
                    *n += 1;
                    if *n >= AUTH_STRIKES {
                        m.remove(&route.connection_id);
                        true
                    } else {
                        false
                    }
                }
                Some(_) => false,
            }
        };
        if broke {
            self.mark_broken(
                route.connection_id,
                &route.connection_name,
                UpCode::Auth.code(),
            );
        }
    }

    /// Marks a connection broken (the status column) and tells everyone: one log line, one `ai.system.connection` event,
    /// a state report soon. Back to `ok` only through a passing test (the admin routes).
    pub fn mark_broken(&self, id: i64, name: &str, code: &str) {
        let now = crate::util::unix_now();
        let was = self.store.write(|tx| {
            let was = conn::connection(tx, id)?.map(|c| c.status);
            conn::set_connection_status(tx, id, "broken", Some(code), Some(now))?;
            Ok::<_, rusqlite::Error>(was)
        });
        match was {
            Ok(Some(w)) if w == "broken" => {}
            Ok(Some(_)) => self.connection_changed(name, true, Some(code)),
            Ok(None) => {}
            Err(e) => tracing::warn!("could not mark an AI connection as broken: {e}"),
        }
    }

    /// A connection turned `broken` or back to `ok`: forget its auth strikes, write the log line, emit the
    /// `ai.system.connection` event (`broken` as a failed event with the code, `restored` as an ok one) and wake the
    /// heartbeat. The caller has already written the status.
    pub fn connection_changed(&self, name: &str, broken: bool, code: Option<&str>) {
        if broken {
            tracing::warn!(
                "AI connection {name:?} is marked broken ({})",
                code.unwrap_or("error")
            );
        } else {
            tracing::info!("AI connection {name:?} works again");
        }
        let mut ev = Event::new(None, "ai.system.connection", &ClientInfo::default()).object(name);
        if broken {
            ev = ev
                .failed(code.unwrap_or("ai.up.auth"))
                .detail(json!({"code": code.unwrap_or("ai.up.auth")}));
        }
        self.activity(ev);
        self.notify_heartbeat();
    }

    /// Clears the auth strikes of a connection (an admin test passed).
    pub fn reset_strikes(&self, connection_id: i64) {
        self.auth_strikes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&connection_id);
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
    pub fn available(&self) -> bool {
        self.effective().unwrap_or(false) && self.link_fresh()
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

    /// `false` when the feature is switched off. The global switch applies to everyone, administrators included; the
    /// user's own bit applies to everyone except administrators. A database error counts as off (no AI use without a
    /// known state), like [`App::suspended`].
    pub fn feature_allowed(&self, who: &Identity, f: Feature) -> bool {
        let admin = who.is_admin();
        let read = self.store.read(|c| {
            let global = conn::global_features_off(c)?;
            let user = if admin {
                0
            } else {
                data::features_off(c, who.id)?
            };
            Ok((global, user))
        });
        match read {
            Ok((global, user)) => {
                crate::features::allowed(global, f) && crate::features::allowed(user, f)
            }
            Err(e) => {
                tracing::warn!("could not read the AI features: {e}");
                false
            }
        }
    }

    /// AI effective, the user not suspended, the feature allowed and its slot resolves (a slot on a connection that is
    /// switched off fails with `ai.connectionOff`). No place, no quota.
    pub fn admit(&self, who: &Identity, feature: Feature) -> Result<(), ApiError> {
        self.admit_pick(who, feature, None)
    }

    /// `admit` for a call that routes through `pick` (an administrator's chat menu): the route actually used is checked,
    /// not the slot's default.
    pub fn admit_pick(
        &self,
        who: &Identity,
        feature: Feature,
        pick: Option<&Pick>,
    ) -> Result<(), ApiError> {
        if !self.effective()? {
            return Err(ApiError::disabled());
        }
        if self.suspended(who) {
            return Err(ApiError::suspended());
        }
        if !self.feature_allowed(who, feature) {
            return Err(ApiError::feature_off(feature));
        }
        self.route(feature.slot(), pick)?;
        Ok(())
    }

    /// Takes the caller's place and a global place, then checks again that AI is still on, the caller is not suspended
    /// and the feature is allowed (a switch-off or a suspension in between would otherwise miss this call: both stop only
    /// the calls that already hold a place), then the per-user caps (not for an administrator while `callsAdminsExempt`
    /// is 1) and the site-wide daily token budget (not for administrators). Usage rows are written for everyone.
    pub fn take_places(
        &self,
        who: &Identity,
        kind: Kind,
        chat: Option<i64>,
        feature: Feature,
    ) -> Result<Places, ApiError> {
        self.take_places_pick(who, kind, chat, feature, None)
    }

    /// `take_places` for a call that routes through `pick`.
    pub fn take_places_pick(
        &self,
        who: &Identity,
        kind: Kind,
        chat: Option<i64>,
        feature: Feature,
        pick: Option<&Pick>,
    ) -> Result<Places, ApiError> {
        let l = self.limits();
        let slot = self
            .registry
            .reserve(
                who.id,
                kind,
                chat,
                crate::limits::Limits::usize_of(l.per_user_running),
            )
            .ok_or_else(ApiError::busy)?;
        slot.set_feature(feature.bit());
        let permit = self.gate.try_acquire().ok_or_else(ApiError::server_busy)?;
        self.admit_pick(who, feature, pick)?;
        let now = crate::util::unix_now();
        // The per-user caps on model calls. The registry is read inside the same store read as the usage rows (lock order
        // store -> registry), and only the entries reserved before this one count.
        if !(who.is_admin() && l.calls_admins_exempt == 1) {
            let hit = self.store.read(|c| {
                let running = self.registry.starts_of(who.id, Some(&slot));
                let day = crate::util::day_bounds(now);
                let mut starts = stats::recent_starts(c, who.id, now, day.start)?;
                starts.extend(running);
                Ok(quota::check(&starts, now, day, l.caps()).err())
            })?;
            if let Some(hit) = hit {
                self.note_call_cap(&who.username, who.id, &hit, now);
                return Err(ApiError::call_limit(&hit, now));
            }
        }
        if !who.is_admin() && l.daily_token_budget > 0 {
            let day = crate::util::local_today().to_string();
            let used = self.store.read(|c| stats::tokens_today(c, &day))?;
            if used >= l.daily_token_budget {
                return Err(ApiError::budget());
            }
        }
        Ok(Places { slot, permit })
    }

    /// The state report for the main backend (`POST /internal/ai/heartbeat`, contracts 2.2): keys in this order. When
    /// the database cannot be read, the report says the AI+ service is off.
    pub fn report(&self, stopping: bool) -> Value {
        let state = self.snapshot().and_then(|snap| {
            let eff = self.effective()?;
            let ids = self
                .store
                .read(|c| data::suspended_ids(c, crate::link::MAX_IDS))?;
            Ok((snap.enabled, eff, ids))
        });
        let (enabled, eff, suspended) = state.unwrap_or((false, false, Vec::new()));
        if !stopping {
            self.note_paused(enabled && !eff);
        }
        json!({"version": self.version, "enabled": eff, "suspended": suspended, "stopping": stopping})
    }

    /// One `ai.system.paused` / `ai.system.resumed` event per change of the paused state (switched on, but no route or no
    /// usable key); the first report after the start only records the state.
    fn note_paused(&self, paused: bool) {
        let prev = self
            .last_paused
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(paused);
        if prev.is_some_and(|p| p != paused) {
            let action = if paused {
                "ai.system.paused"
            } else {
                "ai.system.resumed"
            };
            tracing::info!(
                "AI+ {}",
                if paused {
                    "is paused: no usable model or key"
                } else {
                    "resumed"
                }
            );
            self.activity(Event::new(None, action, &ClientInfo::default()));
        }
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

    /// The signal the activity delivery task waits for (set whenever an event line was written).
    pub fn outbox_signal(&self) -> Arc<Notify> {
        self.outbox_wake.clone()
    }

    /// Writes one activity event into the outbox and wakes the delivery. Never waits for the backend; a database error
    /// drops the event with one warning (an event that cannot be written must not break the action).
    pub fn activity(&self, ev: Event) {
        if !crate::activity::valid_action(ev.action) {
            tracing::warn!("an AI activity event has an invalid action name and was dropped");
            return;
        }
        let at = crate::util::unix_now();
        let line = crate::activity::line_of(&ev, at);
        match self.store.write(|tx| stats::outbox_add(tx, at, &line)) {
            Ok(_) => self.outbox_wake.notify_one(),
            Err(e) => tracing::warn!("could not queue an AI activity event: {e}"),
        }
    }

    /// Sends one event now (an administrator reading a user's chats): `true` when the backend accepted it. Nothing is
    /// queued: a refused event means the action must not happen.
    pub async fn activity_now(&self, ev: Event) -> bool {
        if !crate::activity::valid_action(ev.action) {
            return false;
        }
        let payload =
            crate::activity::payload(&ev, &crate::activity::new_id(), crate::util::unix_now());
        let send = self.link.activity(std::slice::from_ref(&payload));
        matches!(
            tokio::time::timeout(self.timeouts.backend_total, send).await,
            Ok(Ok(()))
        )
    }

    /// Delivers undelivered outbox lines (oldest first) until the outbox is empty or the backend refuses. One delivery
    /// at a time (the admin routes and the background loop both call this): a second caller waits, then finds only the
    /// lines that are still undelivered. Event lines go to `/internal/ai/activity` (at most 50 per call), plain audit
    /// lines to `/internal/ai/log`.
    pub async fn deliver_outbox(&self) -> usize {
        let _one = self.outbox.lock().await;
        let mut sent = 0;
        loop {
            let Ok(rows) = self
                .store
                .read(|c| stats::outbox_pending(c, crate::activity::BATCH as i64))
            else {
                return sent;
            };
            if rows.is_empty() {
                return sent;
            }
            let mut plain: Vec<(i64, String)> = Vec::new();
            let mut events: Vec<(i64, Value)> = Vec::new();
            for (id, m) in rows {
                match crate::activity::parse_line(&m) {
                    Some(v) => events.push((id, v)),
                    None => plain.push((id, m)),
                }
            }
            let mut done: Vec<i64> = Vec::new();
            if !plain.is_empty() {
                let lines: Vec<LogLine> = plain
                    .iter()
                    .map(|(_, m)| LogLine {
                        level: "INFO",
                        message: m.clone(),
                    })
                    .collect();
                if self.link.log(&lines).await.is_err() {
                    return sent;
                }
                done.extend(plain.iter().map(|(id, _)| *id));
            }
            if !events.is_empty() {
                let now = crate::util::unix_now();
                // An event the backend would refuse as too old is dropped here, so it cannot block the ones behind it
                let (live, stale): (Vec<_>, Vec<_>) = events.into_iter().partition(|(_, v)| {
                    v.get("at")
                        .and_then(Value::as_i64)
                        .is_some_and(|at| crate::activity::fresh_enough(at, now))
                });
                if !stale.is_empty() {
                    tracing::warn!(
                        "{} AI activity events were too old to deliver and were dropped",
                        stale.len()
                    );
                    done.extend(stale.iter().map(|(id, _)| *id));
                }
                if !live.is_empty() {
                    let values: Vec<Value> = live.iter().map(|(_, v)| v.clone()).collect();
                    match self.link.activity(&values).await {
                        Ok(()) => done.extend(live.iter().map(|(id, _)| *id)),
                        Err(crate::link::LinkError::Refused) => {
                            // One bad event refuses the whole batch: send them one by one, drop the refused one
                            for (id, v) in &live {
                                match self.link.activity(std::slice::from_ref(v)).await {
                                    Ok(()) => done.push(*id),
                                    Err(crate::link::LinkError::Refused) => {
                                        let action = v
                                            .get("action")
                                            .and_then(serde_json::Value::as_str)
                                            .unwrap_or("?");
                                        tracing::warn!(
                                            "the main backend refused an AI activity event ({action}); it was dropped"
                                        );
                                        done.push(*id);
                                    }
                                    Err(_) => break,
                                }
                            }
                            if done.is_empty() {
                                return sent;
                            }
                        }
                        Err(_) => {
                            if done.is_empty() {
                                return sent;
                            }
                        }
                    }
                }
            }
            if done.is_empty() {
                return sent;
            }
            if self
                .store
                .write(|tx| stats::outbox_remove(tx, &done))
                .is_err()
            {
                return sent;
            }
            sent += done.len();
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

/// Every sealed key with the id of its connection.
fn all_secrets(c: &rusqlite::Connection) -> rusqlite::Result<Vec<(i64, SecretRow)>> {
    let mut out = Vec::new();
    for k in conn::connections(c)? {
        for r in conn::conn_secrets(c, k.id)? {
            out.push((k.id, r));
        }
    }
    Ok(out)
}
