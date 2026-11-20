//! Administrator routes of the connections (`/api/ai/admin/connections/*`) and the shared test engine of the model
//! routes: a candidate connection is tested IN MEMORY before anything is saved (a failed test saves nothing), every test
//! runs under the 90-second request deadline, and one test per connection runs at a time (409 `ai.busy`).
//!
//! No key, hint or typed secret ever reaches a log line, an event or an error: events carry names, hosts and counts only.
use super::admin::settings_value;
use super::{body_id, ok, only_keys, Answer, Ctx};
use crate::activity::Event;
use crate::app::AppState;
use crate::codes::ApiError;
use crate::identity::Identity;
use crate::prompt;
use crate::provider::errors::{UpCode, UpError};
use crate::provider::{self, ModelList, Role, Target, Turn};
use crate::route::{Pick, View};
use crate::secret::{self, SecretName, SecretRef};
use crate::settings::{
    infer_format, normalize_url, origin, strip_v1, valid_model, Format, GatewayAuth, KeyChange,
    Provider, DEFAULT_ANTHROPIC, DEFAULT_OPENAI,
};
use crate::store::conn::{self, ConnRow, NewConnSecret, NewPool, PoolRow};
use crate::store::{data, stats};
use crate::util::unix_now;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// Most connections the settings keep.
pub const MAX_CONNECTIONS: usize = 8;

/// A pool model that passed a test less than this long ago is not tested again by a pick.
pub const FRESH_SECS: i64 = 600;

// —— the candidate and the test engine ——————————————————————————————————————————————————————————————————————————————

/// A connection as it would be, with its keys in memory. No `Debug`.
pub struct Cand {
    pub provider: Provider,
    pub format: Format,
    pub base: String,
    pub gateway_auth: GatewayAuth,
    pub api_key: Option<Zeroizing<String>>,
    pub gateway_key: Option<Zeroizing<String>>,
}

impl Cand {
    pub fn target(&self) -> Target {
        Target {
            provider: self.provider,
            format: self.format,
            base: self.base.clone(),
            gateway_auth: self.gateway_auth,
            api_key: self.api_key.clone(),
            gateway_key: self.gateway_key.clone(),
        }
    }
}

/// `host` or `host:port` (the port only when it is not the scheme default), lower case.
pub fn host_of(url: &str) -> String {
    let Ok(u) = reqwest::Url::parse(url) else {
        return String::new();
    };
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    match u.port() {
        Some(p) => format!("{host}:{p}"),
        None => host,
    }
}

/// The saved keys of a connection: (provider key, gateway key).
pub type Keys = (Option<Zeroizing<String>>, Option<Zeroizing<String>>);

/// Decrypts the saved keys of a connection. A key that does not open is `ai.keyProblem`.
pub fn load_keys(app: &AppState, id: i64) -> Result<Keys, ApiError> {
    let rows = app.0.store.read(|c| conn::conn_secrets(c, id))?;
    let mut api = None;
    let mut gw = None;
    for r in &rows {
        let plain = app
            .0
            .master()
            .and_then(|m| m.open_text(&SecretRef::of_row(id, r), r).ok())
            .ok_or_else(ApiError::key_problem)?;
        match r.name {
            SecretName::ApiKey => api = Some(plain),
            SecretName::GatewayKey => gw = Some(plain),
        }
    }
    Ok((api, gw))
}

/// Like [`load_keys`], but a saved key that does not open is reported as lost (`true`) instead of failing: the two
/// flags are (API key lost, gateway key lost).
fn load_keys_lenient(app: &AppState, id: i64) -> Result<(Keys, bool, bool), ApiError> {
    let rows = app.0.store.read(|c| conn::conn_secrets(c, id))?;
    let (mut api, mut gw, mut lost_api, mut lost_gw) = (None, None, false, false);
    for r in &rows {
        let plain = app
            .0
            .master()
            .and_then(|m| m.open_text(&SecretRef::of_row(id, r), r).ok());
        match (r.name, plain) {
            (SecretName::ApiKey, Some(p)) => api = Some(p),
            (SecretName::ApiKey, None) => lost_api = true,
            (SecretName::GatewayKey, Some(p)) => gw = Some(p),
            (SecretName::GatewayKey, None) => lost_gw = true,
        }
    }
    Ok(((api, gw), lost_api, lost_gw))
}

/// The candidate of a saved connection (its own saved keys). 422 `ai.keyMissing` when a required key is not saved.
pub fn cand_of(app: &AppState, row: &ConnRow) -> Result<Cand, ApiError> {
    let provider = Provider::parse(&row.provider).ok_or_else(ApiError::internal)?;
    let format = Format::parse(&row.format).ok_or_else(ApiError::internal)?;
    let gateway_auth = GatewayAuth::parse(&row.gateway_auth).ok_or_else(ApiError::internal)?;
    let (api, gw) = load_keys(app, row.id)?;
    let gw = if provider == Provider::Gateway {
        gw
    } else {
        None
    };
    if let Some(k) = provider.required_key() {
        let have = match k {
            SecretName::ApiKey => api.is_some(),
            SecretName::GatewayKey => gw.is_some(),
        };
        if !have {
            return Err(ApiError::key_missing(k));
        }
    }
    Ok(Cand {
        provider,
        format,
        base: row.base_url.clone(),
        gateway_auth,
        api_key: api,
        gateway_key: gw,
    })
}

/// One test per connection at a time, per service instance (the key is the instance's address and the connection id).
fn busy_set() -> &'static Mutex<HashSet<(usize, i64)>> {
    static SET: OnceLock<Mutex<HashSet<(usize, i64)>>> = OnceLock::new();
    SET.get_or_init(|| Mutex::new(HashSet::new()))
}

pub struct BusyGuard(usize, i64);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        busy_set()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(self.0, self.1));
    }
}

/// Reserves the connection for one admin test; 409 `ai.busy` when another test of it is running.
pub fn try_busy(app: &AppState, id: i64) -> Result<BusyGuard, ApiError> {
    let key = (std::sync::Arc::as_ptr(&app.0) as usize, id);
    let fresh = busy_set()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key);
    if fresh {
        Ok(BusyGuard(key.0, key.1))
    } else {
        Err(ApiError::test_busy())
    }
}

/// The request deadline of a test: the contract's 90 seconds, never longer than three test-call budgets (so a service
/// started with short budgets, as the tests do, keeps proportionate deadlines).
pub fn total_budget(app: &AppState) -> Duration {
    Duration::from_secs(provider::TEST_TOTAL_SECS)
        .min(app.0.timeouts.verify_total.saturating_mul(3))
}

/// A model call that passed: the request shape the model accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Passed {
    pub usage_option: bool,
    pub max_tokens: i64,
}

/// The outcome of one model test.
pub struct ModelOutcome {
    pub model: String,
    pub result: Result<Passed, UpError>,
    pub latency_ms: i64,
}

/// What a test run found. `list` is `None` when no list call was made.
pub struct Run {
    pub list: Option<Result<ModelList, UpError>>,
    pub list_ms: i64,
    pub models: Vec<ModelOutcome>,
}

impl Run {
    pub fn passed(&self) -> bool {
        self.list.as_ref().is_none_or(Result::is_ok) && self.models.iter().all(|m| m.result.is_ok())
    }

    /// The first failure: the list call, then the models in order. (error, failed model)
    pub fn failure(&self) -> Option<(UpError, Option<String>)> {
        if let Some(Err(e)) = &self.list {
            return Some((*e, None));
        }
        self.models
            .iter()
            .find_map(|m| m.result.as_ref().err().map(|e| (*e, Some(m.model.clone()))))
    }

    pub fn listed(&self) -> Option<&ModelList> {
        self.list.as_ref().and_then(|r| r.as_ref().ok())
    }

    /// `tests` of the answer.
    pub fn tests_json(&self) -> Vec<Value> {
        self.models
            .iter()
            .map(|m| match &m.result {
                Ok(_) => json!({"model": m.model, "ok": true, "latencyMs": m.latency_ms}),
                Err(e) => json!({"model": m.model, "ok": false, "latencyMs": m.latency_ms, "code": e.code.code()}),
            })
            .collect()
    }
}

/// Writes a usage row for an administrator's call (the administrator's AI user row is ensured first).
pub fn admin_usage(app: &AppState, who: &Identity, u: stats::UsageRow) {
    let now = u.at;
    let r = app.0.store.write(|tx| {
        data::ensure_user(tx, who, now)?;
        stats::insert_usage(tx, &u)
    });
    if let Err(e) = r {
        tracing::warn!("could not record AI usage: {e}");
    }
}

/// Waits for a place in the gate until `deadline`.
async fn acquire(app: &AppState, deadline: Instant) -> Option<crate::chat::GatePermit> {
    loop {
        if let Some(p) = app.0.gate.try_acquire() {
            return Some(p);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// The list call of a candidate (one usage row of kind `models`). Does not take a gate place.
pub async fn list_call(
    app: &AppState,
    who: &Identity,
    cand: &Cand,
    cid: Option<i64>,
    cname: &str,
) -> (Result<ModelList, UpError>, i64) {
    let started = Instant::now();
    let t = app.0.timeouts;
    let limit = Duration::from_secs(provider::TEST_LIST_SECS).min(t.short_total);
    let target = cand.target();
    let result =
        match tokio::time::timeout(limit, provider::list_models(&app.0.client, &target, t)).await {
            Ok(r) => r,
            Err(_) => Err(UpError::new(UpCode::Timeout, None)),
        };
    let ms = provider::ms(started.elapsed());
    let (status, code) = match &result {
        Ok(_) => ("ok", None),
        Err(e) => ("error", Some(e.code.short().to_string())),
    };
    admin_usage(
        app,
        who,
        stats::UsageRow {
            user_id: Some(who.id),
            at: unix_now(),
            kind: "models",
            provider: cand.provider.as_str(),
            // listing the models uses no model: the '' row
            model: String::new(),
            tokens_in: None,
            tokens_out: None,
            tokens_cached: None,
            tokens_cache_write: None,
            latency_ms: ms,
            first_byte_ms: None,
            status,
            error_code: code,
            connection_id: cid,
            connection: Some(cname.to_string()),
        },
    );
    (result, ms)
}

/// One test call with the given request shape: `Ok` when the answer has text or token counts.
#[allow(clippy::too_many_arguments)]
async fn attempt(
    app: &AppState,
    who: &Identity,
    cand: &Cand,
    cid: Option<i64>,
    cname: &str,
    model: &str,
    usage_option: bool,
    max_tokens: i64,
) -> (Result<(), UpError>, i64) {
    let started = Instant::now();
    let t = app.0.timeouts;
    let total = Duration::from_secs(provider::TEST_ATTEMPT_SECS).min(t.verify_total);
    let turns = [Turn {
        role: Role::User,
        content: prompt::VERIFY_MESSAGE.to_string(),
    }];
    // the test keeps its v0.9.1 body: no token field for OpenAI-compatible services, the stored value for Anthropic
    let token = match cand.format {
        Format::OpenAi | Format::Responses => provider::TokenLimit::omit(),
        Format::Anthropic => provider::TokenLimit::max_tokens(max_tokens),
    };
    let body = provider::chat_body(
        cand.format,
        model,
        prompt::VERIFY_PROMPT,
        &turns,
        usage_option,
        token,
    );
    let target = cand.target();
    let result = match target.chat_request(&app.0.client, &body, false) {
        Err(e) => Err(e),
        Ok(req) => {
            drop(target);
            provider::collect_chat(
                &app.0.client,
                provider::Plan::single(cand.format, req),
                t,
                total,
            )
            .await
        }
    };
    let ms = provider::ms(started.elapsed());
    let outcome = match &result {
        Ok(c) if !c.text.is_empty() || c.tokens_in.is_some() || c.tokens_out.is_some() => Ok(()),
        Ok(_) => Err(UpError::new(UpCode::Protocol, Some(200))),
        Err(e) => Err(*e),
    };
    let (tin, tout) = match &result {
        Ok(c) => (c.tokens_in, c.tokens_out),
        Err(_) => (None, None),
    };
    admin_usage(
        app,
        who,
        stats::UsageRow {
            user_id: Some(who.id),
            at: unix_now(),
            kind: "verify",
            provider: cand.provider.as_str(),
            model: model.to_string(),
            tokens_in: tin,
            tokens_out: tout,
            tokens_cached: None,
            tokens_cache_write: None,
            latency_ms: ms,
            first_byte_ms: None,
            status: if outcome.is_ok() { "ok" } else { "error" },
            error_code: outcome.as_ref().err().map(|e| e.code.short().to_string()),
            connection_id: cid,
            connection: Some(cname.to_string()),
        },
    );
    (outcome, ms)
}

/// One model test: the existing ladder, at most two attempts (a 400 retries once with the other request shape).
async fn model_call(
    app: &AppState,
    who: &Identity,
    cand: &Cand,
    cid: Option<i64>,
    cname: &str,
    model: &str,
) -> (Result<Passed, UpError>, i64) {
    let mut usage_option = true;
    let mut max_tokens = 8192;
    let mut last = (Err(UpError::new(UpCode::Protocol, None)), 0);
    for n in 0..provider::TEST_ATTEMPTS {
        let (r, ms) = attempt(app, who, cand, cid, cname, model, usage_option, max_tokens).await;
        last = (
            r.map(|()| Passed {
                usage_option,
                max_tokens,
            }),
            ms,
        );
        let again = match &last.0 {
            Err(e) if e.code == UpCode::BadRequest && n + 1 < provider::TEST_ATTEMPTS => {
                // Responses has no `stream_options`: usage always arrives in the terminal event
                if cand.format == Format::OpenAi && usage_option {
                    usage_option = false;
                    true
                } else if cand.format == Format::Anthropic && max_tokens == 8192 {
                    max_tokens = 4096;
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        if !again {
            break;
        }
    }
    last
}

/// Runs `jobs` side by side, each cut off at `deadline`: a job still running then is reported by `late`.
pub async fn with_deadline<T, F>(deadline: Instant, jobs: Vec<F>, late: impl Fn() -> T) -> Vec<T>
where
    F: std::future::Future<Output = T>,
{
    let at = tokio::time::Instant::from_std(deadline);
    futures_util::future::join_all(jobs.into_iter().map(|f| async {
        match tokio::time::timeout_at(at, f).await {
            Ok(v) => v,
            Err(_) => late(),
        }
    }))
    .await
}

/// Runs a test: an optional list call and one call per model, side by side through the gate, under the request deadline.
/// Models still running at the deadline are `ai.up.timeout`.
pub async fn run(
    app: &AppState,
    who: &Identity,
    cid: Option<i64>,
    cname: &str,
    cand: &Cand,
    list: bool,
    models: &[String],
) -> Run {
    let deadline = Instant::now() + total_budget(app);
    let late = UpError::new(UpCode::Timeout, None);
    let timeout_list = || (Err::<ModelList, UpError>(late), 0i64);
    let timeout_model = || (Err::<Passed, UpError>(late), 0i64);
    let list_job = async {
        if !list {
            return None;
        }
        let Some(permit) = acquire(app, deadline).await else {
            return Some(timeout_list());
        };
        let r = list_call(app, who, cand, cid, cname).await;
        drop(permit);
        Some(r)
    };
    let jobs: Vec<_> = models
        .iter()
        .map(|m| async move {
            let Some(permit) = acquire(app, deadline).await else {
                return timeout_model();
            };
            let r = model_call(app, who, cand, cid, cname, m).await;
            drop(permit);
            r
        })
        .collect();
    let at = tokio::time::Instant::from_std(deadline);
    let (list_out, model_out) = tokio::join!(
        async {
            match tokio::time::timeout_at(at, list_job).await {
                Ok(v) => v,
                Err(_) => Some(timeout_list()),
            }
        },
        with_deadline(deadline, jobs, timeout_model)
    );
    let (list, list_ms) = match list_out {
        Some((r, ms)) => (Some(r), ms),
        None => (None, 0),
    };
    Run {
        list,
        list_ms,
        models: models
            .iter()
            .zip(model_out)
            .map(|(m, (result, latency_ms))| ModelOutcome {
                model: m.clone(),
                result,
                latency_ms,
            })
            .collect(),
    }
}

/// Stores the outcome of one model test in the pool: a pass upserts the row as `ok` with the request shape found; a
/// configuration-type failure marks an existing row `failed`; a transient failure (timeout, rate limit) changes nothing.
/// Returns the pool id.
pub fn record_model(
    tx: &rusqlite::Transaction,
    cid: i64,
    o: &ModelOutcome,
    context: Option<i64>,
    manual: Option<bool>,
    now: i64,
) -> rusqlite::Result<Option<i64>> {
    let existing = conn::pool(tx)?
        .into_iter()
        .find(|p| p.connection_id == cid && p.model == o.model);
    match &o.result {
        Ok(p) => Ok(Some(conn::upsert_pool(
            tx,
            &NewPool {
                connection_id: cid,
                model: &o.model,
                manual: manual.unwrap_or_else(|| existing.as_ref().is_some_and(|e| e.manual)),
                status: "ok",
                error_code: None,
                usage_option: p.usage_option,
                max_tokens: p.max_tokens,
                context: context.or_else(|| existing.as_ref().and_then(|e| e.context)),
                latency_ms: Some(o.latency_ms),
                tested_at: Some(now),
            },
        )?)),
        Err(e) => {
            if let (true, Some(row)) = (e.code.is_config(), &existing) {
                conn::set_pool_status(
                    tx,
                    row.id,
                    "failed",
                    Some(e.code.code()),
                    Some(o.latency_ms),
                    Some(now),
                )?;
            }
            Ok(existing.map(|e| e.id))
        }
    }
}

/// The pool models of a connection that a slot uses (at most six), in slot order.
pub fn in_use(view: &View, cid: i64) -> Vec<PoolRow> {
    let mut out: Vec<PoolRow> = Vec::new();
    for id in view.slots.iter().flatten() {
        if let Some(m) = view
            .pool
            .iter()
            .find(|m| m.id == *id && m.connection_id == cid)
        {
            if !out.iter().any(|o| o.id == m.id) {
                out.push(m.clone());
            }
        }
    }
    out
}

/// The slot names that use a pool model of the connection.
pub fn slots_using(view: &View, cid: i64) -> Vec<&'static str> {
    crate::route::Slot::ALL
        .iter()
        .filter(|s| {
            view.slots
                .get(s.index())
                .copied()
                .flatten()
                .and_then(|id| view.pool.iter().find(|m| m.id == id))
                .is_some_and(|m| m.connection_id == cid)
        })
        .map(|s| s.as_str())
        .collect()
}

/// The answer of a failed test (nothing saved).
fn failed_answer(app: &AppState, run: &Run, id: Option<i64>) -> Result<Value, ApiError> {
    let (e, model) = run.failure().ok_or_else(ApiError::internal)?;
    let mut m = Map::new();
    m.insert("ok".into(), Value::Bool(false));
    m.insert("saved".into(), Value::Bool(false));
    if let Some(id) = id {
        m.insert("connectionId".into(), Value::from(id));
    }
    m.insert("code".into(), Value::from(e.code.code()));
    m.insert("httpStatus".into(), e.http.map_or(Value::Null, Value::from));
    m.insert("failedModel".into(), model.map_or(Value::Null, Value::from));
    m.insert("tests".into(), Value::Array(run.tests_json()));
    m.insert("settings".into(), settings_value(app)?);
    Ok(Value::Object(m))
}

// —— save ———————————————————————————————————————————————————————————————————————————————————————————————————————————

struct Req {
    id: Option<i64>,
    rev: Option<i64>,
    name: String,
    provider: Provider,
    base_url: Option<String>,
    format: Option<Format>,
    gateway_auth: Option<GatewayAuth>,
    /// The thinking-strength family: `None` = keep (update) or infer (create)
    effort_family: Option<crate::effort::EffortFamily>,
    api: KeyChange,
    gw: KeyChange,
    reuse_from: Option<i64>,
    confirm: Option<String>,
}

fn key_field(body: &Map<String, Value>, name: SecretName) -> Result<KeyChange, ApiError> {
    match body.get(name.json()) {
        None => Ok(KeyChange::Keep),
        Some(Value::Null) => Ok(KeyChange::Remove),
        Some(Value::String(s)) => {
            if secret::valid_key(s) {
                Ok(KeyChange::Set(Zeroizing::new(s.clone())))
            } else {
                Err(ApiError::bad_key(name))
            }
        }
        Some(_) => Err(ApiError::bad_request()),
    }
}

fn enum_field<T>(
    body: &Map<String, Value>,
    key: &str,
    parse: fn(&str) -> Option<T>,
) -> Result<Option<T>, ApiError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => parse(s).map(Some).ok_or_else(ApiError::bad_request),
        Some(_) => Err(ApiError::bad_request()),
    }
}

/// A connection name: 1..=40 characters after trimming, no control character.
pub fn clean_name(raw: &str) -> Result<String, ApiError> {
    let t = raw.trim();
    let n = t.chars().count();
    if !(1..=40).contains(&n) || t.chars().any(char::is_control) {
        return Err(ApiError::bad_name());
    }
    Ok(t.to_string())
}

fn parse(body: &Map<String, Value>) -> Result<Req, ApiError> {
    let id = body_id(body, "id")?;
    let rev = body_id(body, "rev")?;
    if id.is_some() != rev.is_some() {
        return Err(ApiError::bad_request());
    }
    let reuse_from = body_id(body, "reuseKeysFrom")?;
    if id.is_some() && reuse_from.is_some() {
        return Err(ApiError::bad_request());
    }
    let name = match body.get("name") {
        Some(Value::String(s)) => clean_name(s)?,
        _ => return Err(ApiError::bad_request()),
    };
    let provider =
        enum_field(body, "provider", Provider::parse)?.ok_or_else(ApiError::bad_request)?;
    let base_url = match body.get("baseUrl") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.trim().is_empty() => None,
        Some(Value::String(s)) => Some(normalize_url(s).ok_or_else(ApiError::bad_url)?),
        Some(_) => return Err(ApiError::bad_request()),
    };
    let confirm = match body.get("confirmHost") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.trim().to_string()),
        Some(_) => return Err(ApiError::bad_request()),
    };
    let effort_family = match body.get("effortFamily") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            Some(crate::effort::EffortFamily::parse(s).ok_or_else(ApiError::bad_request)?)
        }
        Some(_) => return Err(ApiError::bad_request()),
    };
    Ok(Req {
        id,
        rev,
        name,
        provider,
        base_url,
        format: enum_field(body, "format", Format::parse)?,
        gateway_auth: enum_field(body, "gatewayAuth", GatewayAuth::parse)?,
        effort_family,
        api: key_field(body, SecretName::ApiKey)?,
        gw: key_field(body, SecretName::GatewayKey)?,
        reuse_from,
        confirm,
    })
}

/// How a key changes (for the event).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Change {
    None,
    Set,
    Replace,
    Remove,
}

impl Change {
    fn of(c: &KeyChange, had: bool) -> Change {
        match (c, had) {
            (KeyChange::Keep, _) => Change::None,
            (KeyChange::Remove, true) => Change::Remove,
            (KeyChange::Remove, false) => Change::None,
            (KeyChange::Set(_), true) => Change::Replace,
            (KeyChange::Set(_), false) => Change::Set,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Change::Set => "set",
            Change::Replace => "replace",
            _ => "remove",
        }
    }
}

enum Level {
    None,
    List,
    ListOne,
    ListAll,
}

pub async fn save(ctx: Ctx) -> Answer {
    only_keys(
        &ctx.body,
        &[
            "id",
            "rev",
            "name",
            "provider",
            "baseUrl",
            "format",
            "gatewayAuth",
            "effortFamily",
            "apiKey",
            "gatewayKey",
            "reuseKeysFrom",
            "confirmHost",
        ],
    )?;
    let req = parse(&ctx.body)?;
    let app = &ctx.app;
    let who = &ctx.who;
    let view = app.0.routes()?;
    let existing: Option<ConnRow> = match req.id {
        Some(id) => {
            let row = view.conn(id).cloned().ok_or_else(ApiError::not_found)?;
            if Some(row.rev) != req.rev {
                return Err(ApiError::conflict());
            }
            Some(row)
        }
        None => None,
    };
    let provider = req.provider;
    // Address: given, kept (same provider), or the provider's default
    let mut url = match (&req.base_url, &existing) {
        (Some(u), _) => u.clone(),
        (None, Some(e)) if e.provider == provider.as_str() => e.base_url.clone(),
        (None, _) => match provider {
            Provider::OpenAi => DEFAULT_OPENAI.to_string(),
            Provider::Anthropic => DEFAULT_ANTHROPIC.to_string(),
            Provider::Gateway => return Err(ApiError::bad_url()),
        },
    };
    let format = match provider {
        Provider::OpenAi => Format::OpenAi,
        Provider::Anthropic => Format::Anthropic,
        Provider::Gateway => req
            .format
            .or_else(|| {
                existing
                    .as_ref()
                    .filter(|e| e.provider == "gateway")
                    .and_then(|e| Format::parse(&e.format))
            })
            .unwrap_or_else(|| infer_format(&url)),
    };
    if format == Format::Anthropic {
        url = strip_v1(&url);
    }
    let gateway_auth = req
        .gateway_auth
        .or_else(|| {
            existing
                .as_ref()
                .and_then(|e| GatewayAuth::parse(&e.gateway_auth))
        })
        .unwrap_or(GatewayAuth::Bearer);

    // Keys: the saved ones (or the source's, for a copy) with the requested changes applied
    // A saved key that no longer opens (the encryption key changed) is only a problem when the request keeps it: a
    // typed or removed key replaces it, which is how an administrator repairs `keyProblem`.
    let keeps_gw = provider == Provider::Gateway && matches!(req.gw, KeyChange::Keep);
    let ((old_api, old_gw), lost_api, lost_gw) = match &existing {
        Some(e) => load_keys_lenient(app, e.id)?,
        None => ((None, None), false, false),
    };
    if (lost_api && matches!(req.api, KeyChange::Keep)) || (lost_gw && keeps_gw) {
        return Err(ApiError::key_problem());
    }
    let mut source: Option<ConnRow> = None;
    let (mut src_api, mut src_gw) = (None, None);
    if let Some(sid) = req.reuse_from {
        let s = view.conn(sid).cloned().ok_or_else(ApiError::bad_request)?;
        let (a, g) = load_keys(app, sid)?;
        source = Some(s);
        src_api = a;
        src_gw = g;
    }
    let had_api = old_api.is_some() || lost_api;
    let had_gw = old_gw.is_some() || lost_gw;
    let final_api: Option<Zeroizing<String>> = match &req.api {
        KeyChange::Set(k) => Some(k.clone()),
        KeyChange::Remove => None,
        KeyChange::Keep => old_api.clone().or(src_api),
    };
    // a gateway key only means something for a gateway; for other providers it is dropped
    let final_gw: Option<Zeroizing<String>> = if provider == Provider::Gateway {
        match &req.gw {
            KeyChange::Set(k) => Some(k.clone()),
            KeyChange::Remove => None,
            KeyChange::Keep => old_gw.clone().or(src_gw),
        }
    } else {
        None
    };
    let api_change = Change::of(&req.api, had_api);
    let gw_change = if provider == Provider::Gateway {
        Change::of(&req.gw, had_gw)
    } else if had_gw {
        Change::Remove
    } else {
        Change::None
    };

    if provider == Provider::Gateway
        && final_api.is_some()
        && matches!(
            (gateway_auth, format),
            (GatewayAuth::Bearer, Format::OpenAi | Format::Responses)
                | (GatewayAuth::XApiKey, Format::Anthropic)
        )
    {
        return Err(ApiError::header_clash());
    }
    if let Some(k) = provider.required_key() {
        let have = match k {
            SecretName::ApiKey => final_api.is_some(),
            SecretName::GatewayKey => final_gw.is_some(),
        };
        if !have {
            return Err(ApiError::key_missing(k));
        }
    }
    // Limit and name
    if existing.is_none() && view.conns.len() >= MAX_CONNECTIONS {
        return Err(ApiError::connection_limit(
            i64::try_from(MAX_CONNECTIONS).unwrap_or(8),
        ));
    }
    if app
        .0
        .store
        .read(|c| conn::name_taken(c, &req.name, existing.as_ref().map(|e| e.id)))?
    {
        return Err(ApiError::name_taken());
    }

    // Key reuse: a saved key would go to another provider or origin (or a copy to a new connection)
    let kept_any = (matches!(req.api, KeyChange::Keep) && final_api.is_some() && had_api)
        || (matches!(req.gw, KeyChange::Keep) && final_gw.is_some() && had_gw);
    let moved = existing
        .as_ref()
        .is_some_and(|e| provider.as_str() != e.provider || origin(&url) != origin(&e.base_url));
    let copied = existing.is_none()
        && source.is_some()
        && ((matches!(req.api, KeyChange::Keep) && final_api.is_some())
            || (matches!(req.gw, KeyChange::Keep) && final_gw.is_some()));
    let reuse = (moved && kept_any) || copied;
    let new_host = host_of(&url);
    let old_host = match (&existing, &source) {
        (Some(e), _) => host_of(&e.base_url),
        (None, Some(s)) => host_of(&s.base_url),
        _ => String::new(),
    };
    let used_by: Vec<&'static str> = existing
        .as_ref()
        .map(|e| slots_using(&view, e.id))
        .unwrap_or_default();
    if reuse {
        let confirmed = req
            .confirm
            .as_deref()
            .is_some_and(|c| c.eq_ignore_ascii_case(&new_host));
        if !confirmed {
            return Err(ApiError::key_reuse_confirm(
                &new_host,
                &old_host,
                json!(used_by),
            ));
        }
    }

    // What changed (existing connection)
    let mut changed: Vec<&'static str> = Vec::new();
    if let Some(e) = &existing {
        if req.name != e.name {
            changed.push("name");
        }
        if provider.as_str() != e.provider {
            changed.push("provider");
        }
        if url != e.base_url {
            changed.push("baseUrl");
        }
        if format.as_str() != e.format {
            changed.push("format");
        }
        if gateway_auth.as_str() != e.gateway_auth {
            changed.push("gatewayAuth");
        }
        if req
            .effort_family
            .is_some_and(|f| f.as_str() != e.effort_family)
        {
            changed.push("effortFamily");
        }
        if api_change != Change::None {
            changed.push("apiKey");
        }
        if gw_change != Change::None {
            changed.push("gatewayKey");
        }
    }
    let fmt_changed = existing.as_ref().is_some_and(|e| {
        provider.as_str() != e.provider
            || url != e.base_url
            || format.as_str() != e.format
            || gateway_auth.as_str() != e.gateway_auth
    });
    let key_changed = api_change != Change::None || gw_change != Change::None;
    let level = if existing.is_none() {
        Level::List
    } else if fmt_changed {
        Level::ListAll
    } else if key_changed {
        Level::ListOne
    } else {
        Level::None
    };

    let _busy = match &existing {
        Some(e) => Some(try_busy(app, e.id)?),
        None => None,
    };
    let cand = Cand {
        provider,
        format,
        base: url.clone(),
        gateway_auth,
        api_key: final_api.clone(),
        gateway_key: final_gw.clone(),
    };
    let models: Vec<String> = match (&level, &existing) {
        (Level::ListAll, Some(e)) => in_use(&view, e.id).into_iter().map(|m| m.model).collect(),
        (Level::ListOne, Some(e)) => in_use(&view, e.id)
            .into_iter()
            .take(1)
            .map(|m| m.model)
            .collect(),
        _ => Vec::new(),
    };
    let tested = !matches!(level, Level::None);
    let run = if tested {
        run(
            app,
            who,
            existing.as_ref().map(|e| e.id),
            &req.name,
            &cand,
            true,
            &models,
        )
        .await
    } else {
        Run {
            list: None,
            list_ms: 0,
            models: Vec::new(),
        }
    };

    // The thinking-strength family: the request's, else the stored one (update), else inferred (create) from the provider,
    // the format and the model names the listing returned
    let family = match (req.effort_family, &existing) {
        (Some(f), _) => f,
        (None, Some(e)) => e.family(),
        (None, None) => {
            let names: Vec<&str> = run
                .listed()
                .map(|l| l.models.iter().map(String::as_str).collect())
                .unwrap_or_default();
            conn::infer_family(provider.as_str(), format.as_str(), &names)
        }
    };

    // The event: one per request
    let ev = |failed: Option<&str>| {
        let (action, detail) = if reuse {
            (
                "ai.settings.keyReuse",
                json!({"oldHost": old_host, "newHost": new_host, "slots": used_by}),
            )
        } else if existing.is_none() {
            (
                "ai.settings.connectionAdd",
                json!({"provider": provider.as_str(), "host": new_host, "family": family.as_str()}),
            )
        } else if changed == ["apiKey"] || changed == ["gatewayKey"] {
            let (name, ch) = if changed == ["apiKey"] {
                ("apiKey", api_change)
            } else {
                ("gatewayKey", gw_change)
            };
            (
                "ai.settings.key",
                json!({"secret": name, "change": ch.word()}),
            )
        } else {
            (
                "ai.settings.connectionUpdate",
                json!({"provider": provider.as_str(), "host": new_host, "family": family.as_str(), "changed": changed}),
            )
        };
        let e = Event::new(Some(who), action, &ctx.client)
            .object(req.name.clone())
            .detail(detail);
        match failed {
            Some(code) => e.failed(code),
            None => e,
        }
    };

    if !run.passed() {
        let answer = failed_answer(app, &run, existing.as_ref().map(|e| e.id))?;
        let code = answer["code"]
            .as_str()
            .unwrap_or("ai.up.protocol")
            .to_string();
        app.0.activity(ev(Some(&code)));
        tracing::info!(
            "user {:?} tried to save the AI connection {:?}: the test failed ({code})",
            who.username,
            req.name
        );
        return ok(answer);
    }

    // Everything passed: save in one transaction
    let now = unix_now();
    let master = app.0.master().ok_or_else(ApiError::key_problem)?;
    let listed = run.listed();
    let lists_models = match (&listed, &existing) {
        (Some(l), _) => l.listed,
        (None, Some(e)) => e.lists_models,
        (None, None) => true,
    };
    let by = who.username.clone();
    let (id, was_broken) = app.0.store.write(|tx| -> Result<(i64, bool), ApiError> {
        if conn::name_taken(tx, &req.name, existing.as_ref().map(|e| e.id))? {
            return Err(ApiError::name_taken());
        }
        let (id, was_broken) = match &existing {
            Some(e) => {
                let change = conn::ConnChange {
                    name: &req.name,
                    provider: provider.as_str(),
                    base_url: &url,
                    format: format.as_str(),
                    gateway_auth: gateway_auth.as_str(),
                    lists_models,
                    effort_family: family.as_str(),
                    by: &by,
                    now,
                };
                if !conn::update_connection(tx, e.id, e.rev, &change)? {
                    return Err(ApiError::conflict());
                }
                (e.id, e.status == "broken")
            }
            None => {
                if conn::connections(tx)?.len() >= MAX_CONNECTIONS {
                    return Err(ApiError::connection_limit(
                        i64::try_from(MAX_CONNECTIONS).unwrap_or(8),
                    ));
                }
                let id = conn::insert_connection(
                    tx,
                    &conn::NewConn {
                        name: &req.name,
                        provider: provider.as_str(),
                        base_url: &url,
                        format: format.as_str(),
                        gateway_auth: gateway_auth.as_str(),
                        lists_models,
                        effort_family: family.as_str(),
                        by: &by,
                        now,
                    },
                )?;
                (id, false)
            }
        };
        if tested {
            conn::set_connection_status(tx, id, "ok", None, Some(now))?;
        } else if was_broken {
            // a name-only save does not test: the status stays as it is
        }
        // Keys: write what the final state holds when it differs from what was stored
        for (name, plain, change) in [
            (SecretName::ApiKey, &final_api, api_change),
            (SecretName::GatewayKey, &final_gw, gw_change),
        ] {
            let copy = existing.is_none() && plain.is_some();
            match plain {
                Some(k) if change != Change::None || copy => {
                    let sealed = master.seal(&SecretRef::new(id, name), k.as_bytes());
                    conn::put_conn_secret(
                        tx,
                        id,
                        &NewConnSecret {
                            name,
                            key_id: master.key_id(),
                            sealed: &sealed,
                            hint: &secret::hint(k),
                            aad_ver: 2,
                            now,
                        },
                    )?;
                }
                None if change == Change::Remove => {
                    conn::delete_conn_secret(tx, id, name)?;
                }
                _ => {}
            }
        }
        for o in &run.models {
            let ctx_tokens = listed
                .and_then(|l| l.contexts.get(&o.model))
                .and_then(|n| i64::try_from(*n).ok());
            record_model(tx, id, o, ctx_tokens, None, now)?;
        }
        Ok((id, was_broken && tested))
    })?;
    app.0.recheck_keys();
    app.0.reset_strikes(id);
    app.0.forget_no_effort();
    if was_broken {
        app.0.connection_changed(&req.name, false, None);
    }
    if let Ok(mut cache) = app.0.models_cache.lock() {
        cache.retain(|(cid, _), _| *cid != id);
    }
    app.0.activity(ev(None));
    tracing::info!(
        "user {:?} saved the AI connection {:?} ({}, {new_host})",
        who.username,
        req.name,
        provider.display()
    );
    app.0.heartbeat_now().await;
    let model_count = listed.map_or(0, |l| l.models.len());
    ok(json!({
        "ok": true,
        "saved": true,
        "connectionId": id,
        "latencyMs": run.list_ms,
        "listed": lists_models,
        "modelCount": model_count,
        "tests": run.tests_json(),
        "settings": settings_value(app)?,
    }))
}

// —— test, delete, order ————————————————————————————————————————————————————————————————————————————————————————————

/// `POST connections/test` `{"id"}`: a list call and one call per in-use model of the connection. Sets the connection's
/// status (`ok`, or `broken` on a configuration-type code) and the status of the models tested.
pub async fn test(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["id"])?;
    let id = body_id(&ctx.body, "id")?.ok_or_else(ApiError::bad_request)?;
    let app = &ctx.app;
    let who = &ctx.who;
    let view = app.0.routes()?;
    let row = view.conn(id).cloned().ok_or_else(ApiError::not_found)?;
    let _busy = try_busy(app, id)?;
    let cand = cand_of(app, &row)?;
    let models: Vec<String> = in_use(&view, id).into_iter().map(|m| m.model).collect();
    let run = run(app, who, Some(id), &row.name, &cand, true, &models).await;
    let now = unix_now();
    let failure = run.failure();
    let broke = failure
        .as_ref()
        .filter(|(e, _)| e.code.is_config())
        .map(|(e, _)| e.code.code());
    let listed = run.listed();
    let was_broken = row.status == "broken";
    app.0.store.write(|tx| -> rusqlite::Result<()> {
        for o in &run.models {
            record_model(tx, id, o, None, None, now)?;
        }
        if let Some(code) = broke {
            conn::set_connection_status(tx, id, "broken", Some(code), Some(now))?;
        } else if run.passed() {
            conn::set_connection_status(tx, id, "ok", None, Some(now))?;
        }
        if let Some(l) = listed {
            tx.execute(
                "UPDATE connections SET lists_models = ?2 WHERE id = ?1",
                rusqlite::params![id, i64::from(l.listed)],
            )?;
        }
        Ok(())
    })?;
    if run.passed() {
        app.0.reset_strikes(id);
        if was_broken {
            app.0.connection_changed(&row.name, false, None);
        }
    } else if let (Some(code), false) = (broke, was_broken) {
        app.0.connection_changed(&row.name, true, Some(code));
    }
    let mut ev = Event::new(Some(who), "ai.settings.connectionTest", &ctx.client)
        .object(row.name.clone())
        .detail(json!({"models": models.len()}));
    if let Some((e, _)) = &failure {
        ev = ev.failed(e.code.code());
    }
    app.0.activity(ev);
    tracing::info!(
        "user {:?} tested the AI connection {:?}: {}",
        who.username,
        row.name,
        failure.as_ref().map_or("OK", |(e, _)| e.code.code())
    );
    app.0.heartbeat_now().await;
    if !run.passed() {
        return ok(failed_answer(app, &run, Some(id))?);
    }
    ok(json!({
        "ok": true,
        "saved": false,
        "connectionId": id,
        "latencyMs": run.list_ms,
        "listed": listed.is_some_and(|l| l.listed),
        "modelCount": listed.map_or(0, |l| l.models.len()),
        "tests": run.tests_json(),
        "settings": settings_value(app)?,
    }))
}

/// `POST connections/delete` `{"id", "force", "newDefault"}`.
pub async fn delete(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["id", "force", "newDefault"])?;
    let id = body_id(&ctx.body, "id")?.ok_or_else(ApiError::bad_request)?;
    let force = match ctx.body.get("force") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(ApiError::bad_request()),
    };
    let app = &ctx.app;
    let view = app.0.routes()?;
    let row = view.conn(id).cloned().ok_or_else(ApiError::not_found)?;
    let used_by = slots_using(&view, id);
    if !used_by.is_empty() && !force {
        return Err(ApiError::connection_in_use(json!(used_by)));
    }
    let default_used = used_by.contains(&"default");
    let others = view.conns.iter().any(|c| c.id != id);
    let new_default: Option<PoolRow> = match ctx.body.get("newDefault") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let p: Pick = Pick::from_json(v)?.ok_or_else(ApiError::bad_model)?;
            let m = view
                .pool
                .iter()
                .find(|m| m.connection_id == p.connection_id && m.model == p.model)
                .filter(|m| m.connection_id != id && view.model_usable(m))
                .ok_or_else(ApiError::bad_model)?;
            Some(m.clone())
        }
    };
    if default_used && new_default.is_none() && others {
        return Err(ApiError::default_needed());
    }
    let by = ctx.who.username.clone();
    let turned_off = app.0.store.write(|tx| -> rusqlite::Result<bool> {
        if let Some(m) = &new_default {
            conn::set_slot(tx, "default", Some(m.id), &by)?;
        }
        conn::delete_connection(tx, id)?;
        // no connection left: AI+ cannot work any more, so the switch goes off as well
        let left = conn::connections(tx)?.len();
        if left == 0 {
            tx.execute("UPDATE settings SET enabled = 0 WHERE id = 1", [])?;
            return Ok(true);
        }
        Ok(false)
    })?;
    if let Ok(mut cache) = app.0.models_cache.lock() {
        cache.retain(|(cid, _), _| *cid != id);
    }
    app.0.reset_strikes(id);
    app.0.recheck_keys();
    if turned_off {
        app.0.cancel_everything(crate::chat::StopReason::Disabled);
    }
    app.0.activity(
        Event::new(Some(&ctx.who), "ai.settings.connectionRemove", &ctx.client)
            .object(row.name.clone())
            .detail(json!({"provider": row.provider, "host": host_of(&row.base_url)})),
    );
    tracing::info!(
        "user {:?} removed the AI connection {:?}",
        ctx.who.username,
        row.name
    );
    app.0.heartbeat_now().await;
    ok(json!({"settings": settings_value(app)?}))
}

/// `POST connections/order` `{"ids": [2, 1]}`.
pub async fn order(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["ids"])?;
    let Some(Value::Array(list)) = ctx.body.get("ids") else {
        return Err(ApiError::bad_request());
    };
    if list.len() > MAX_CONNECTIONS {
        return Err(ApiError::bad_request());
    }
    let mut ids: Vec<i64> = Vec::new();
    for v in list {
        let n = v
            .as_i64()
            .filter(|n| *n > 0)
            .ok_or_else(ApiError::bad_request)?;
        if ids.contains(&n) {
            return Err(ApiError::bad_request());
        }
        ids.push(n);
    }
    let app = &ctx.app;
    app.0.store.write(|tx| conn::order_connections(tx, &ids))?;
    app.0.activity(
        Event::new(Some(&ctx.who), "ai.settings.connectionOrder", &ctx.client)
            .detail(json!({"count": ids.len()})),
    );
    ok(json!({"settings": settings_value(app)?}))
}

/// `POST connections/enabled` `{"id": N, "enabled": bool}`: turns a connection on or off. Off keeps every setting, key
/// and pool model; the connection is left out of the pickers and of routing, and the choices that use it fail with
/// `ai.connectionOff` until it is on again. Running calls go on and `rev` is not bumped. Also answers when nothing
/// changed.
pub async fn enabled(ctx: Ctx) -> Answer {
    if ctx.body.len() != 2 {
        return Err(ApiError::bad_request());
    }
    only_keys(&ctx.body, &["id", "enabled"])?;
    let id = body_id(&ctx.body, "id")?.ok_or_else(ApiError::bad_request)?;
    let Some(Value::Bool(on)) = ctx.body.get("enabled") else {
        return Err(ApiError::bad_request());
    };
    let on = *on;
    let app = &ctx.app;
    let now = unix_now();
    let by = ctx.who.username.clone();
    let row = app
        .0
        .store
        .write(|tx| -> rusqlite::Result<Option<ConnRow>> {
            let Some(row) = conn::connection(tx, id)? else {
                return Ok(None);
            };
            if !conn::set_connection_enabled(tx, id, on, &by, now)? {
                return Ok(None);
            }
            Ok(Some(row))
        })?
        .ok_or_else(ApiError::not_found)?;
    let (event, word) = if on {
        ("ai.settings.connectionOn", "on")
    } else {
        ("ai.settings.connectionOff", "off")
    };
    app.0.activity(
        Event::new(Some(&ctx.who), event, &ctx.client)
            .object(row.name.clone())
            .detail(json!({"provider": row.provider, "host": host_of(&row.base_url)})),
    );
    tracing::info!(
        "user {:?} switched the AI connection {:?} {word}",
        ctx.who.username,
        row.name
    );
    app.0.heartbeat_now().await;
    ok(json!({"settings": settings_value(app)?}))
}

/// A model id the pool and the picks accept.
pub fn model_ok(m: &str) -> bool {
    crate::route::model_id_ok(m) && valid_model(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_carry_the_port_only_when_it_is_not_the_default() {
        assert_eq!(host_of("https://API.example.com/v1"), "api.example.com");
        assert_eq!(host_of("https://api.example.com:443/v1"), "api.example.com");
        assert_eq!(host_of("http://ai.example.com:9000"), "ai.example.com:9000");
        assert_eq!(host_of("http://ai.example.com:80"), "ai.example.com");
        assert_eq!(host_of("not a url"), "");
    }

    #[test]
    fn names_are_one_to_forty_printable_characters() {
        assert_eq!(clean_name("  main ").as_deref().ok(), Some("main"));
        assert!(clean_name("").is_err());
        assert!(clean_name("   ").is_err());
        assert!(clean_name(&"a".repeat(41)).is_err());
        assert!(clean_name(&"字".repeat(40)).is_ok());
        assert!(clean_name("a\u{7}b").is_err());
    }

    #[test]
    fn a_key_change_is_named_for_the_event() {
        let set = KeyChange::Set(Zeroizing::new("k".into()));
        assert!(Change::of(&set, false) == Change::Set);
        assert!(Change::of(&set, true) == Change::Replace);
        assert!(Change::of(&KeyChange::Remove, true) == Change::Remove);
        assert!(Change::of(&KeyChange::Remove, false) == Change::None);
        assert!(Change::of(&KeyChange::Keep, true) == Change::None);
        assert_eq!(Change::Replace.word(), "replace");
    }

    #[tokio::test]
    async fn a_job_past_the_deadline_is_reported_late_and_the_others_finish() {
        let deadline = Instant::now() + Duration::from_millis(150);
        let jobs: Vec<std::pin::Pin<Box<dyn std::future::Future<Output = i32>>>> = vec![
            Box::pin(async { 1 }),
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                2
            }),
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                3
            }),
        ];
        let out = with_deadline(deadline, jobs, || -1).await;
        assert_eq!(out, vec![1, -1, 3]);
    }

    #[tokio::test]
    async fn the_busy_guard_admits_one_test_per_connection_and_releases_on_drop() {
        let key = (0xDEAD_usize, 7_i64);
        let set = busy_set();
        assert!(set.lock().unwrap().insert(key));
        let guard = BusyGuard(key.0, key.1);
        assert!(!set.lock().unwrap().insert(key));
        drop(guard);
        assert!(set.lock().unwrap().insert(key));
        set.lock().unwrap().remove(&key);
    }
}
