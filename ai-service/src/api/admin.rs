//! Administrator routes: settings, models, test, enable, usage, users with AI+ and their data (read only, audited),
//! clear, safety limits, Suspend / Resume of a user's AI+ function (audited).
//!
//! Every handler that changes the effective AI+ service state or a user's AI+ function sends a state report to the
//! main backend before it answers (round-2 contracts 2.6; best effort, the answer does not depend on it).
use super::{body_id, ok, only_keys, query_id, query_text, Answer, Ctx};
use crate::app::{AppState, ModelsCache, Snapshot};
use crate::chat::StopReason;
use crate::codes::ApiError;
use crate::limits::{LimitError, DEFS};
use crate::link::BackendUser;
use crate::prompt;
use crate::provider::{self, Role, Turn, UpCode, UpError};
use crate::retention;
use crate::secret::{self, SecretName};
use crate::settings::{
    fingerprint, plan_update, Format, KeyChange, UpdateError, DEFAULT_ANTHROPIC, DEFAULT_OPENAI,
};
use crate::store::{data, stats, tables};
use crate::util::unix_now;
use serde_json::{json, Map, Value};
use std::time::{Duration, Instant};

const MODELS_TTL: Duration = Duration::from_secs(600);

fn key_info(app: &AppState, snap: &Snapshot, name: SecretName) -> Value {
    let row = snap.secrets.iter().find(|r| r.name == name);
    match row {
        None => json!({"set": false, "hint": "", "unreadable": false}),
        Some(r) => {
            let readable = match app.0.master() {
                Some(m) => {
                    r.key_id == m.key_id() && m.open(r.name, &r.nonce, &r.ciphertext).is_ok()
                }
                None => false,
            };
            json!({"set": true, "hint": r.hint, "unreadable": !readable})
        }
    }
}

/// The `Settings` answer (5.3).
fn settings_json(app: &AppState, snap: &Snapshot) -> Map<String, Value> {
    let s = &snap.settings;
    let v = json!({
        "provider": s.provider.as_str(),
        "baseUrl": s.base_url,
        "format": s.wire_format().as_str(),
        "gatewayAuth": s.gateway_auth.as_str(),
        "model": s.model,
        "manualModel": s.manual_model,
        "enabled": snap.effective(),
        "verified": snap.verified(),
        "verifiedAt": s.verified_at,
        "retentionDays": s.retention_days,
        "keys": {
            "apiKey": key_info(app, snap, SecretName::ApiKey),
            "gatewayKey": key_info(app, snap, SecretName::GatewayKey),
        },
        "keyProblem": snap.key_problem.map(|p| p.as_str()),
        "updatedAt": s.updated_at,
        "updatedBy": s.updated_by,
        "defaults": {"openai": DEFAULT_OPENAI, "anthropic": DEFAULT_ANTHROPIC},
    });
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

pub async fn settings_get(ctx: Ctx) -> Answer {
    let snap = ctx.app.0.snapshot()?;
    ok(Value::Object(settings_json(&ctx.app, &snap)))
}

fn update_error(e: UpdateError) -> ApiError {
    match e {
        UpdateError::BadRequest => ApiError::bad_request(),
        UpdateError::BadUrl => ApiError::bad_url(),
        UpdateError::BadKey(n) => ApiError::bad_key(n),
        UpdateError::KeyReenter(n) => ApiError::key_reenter(n),
        UpdateError::HeaderClash => ApiError::header_clash(),
        UpdateError::BadRetention => ApiError::bad_retention(),
    }
}

/// What a settings save changed (for the log line).
struct Saved {
    switched_off: bool,
    fp_changed: bool,
    keys_changed: bool,
    connection: bool,
    retention: Option<i64>,
    parts: Vec<&'static str>,
}

pub async fn settings_post(ctx: Ctx) -> Answer {
    let app = &ctx.app;
    let who = &ctx.who;
    let now = unix_now();
    let key_id = app.0.key_id().to_string();
    let saved = app.0.store.write(|tx| -> Result<Saved, ApiError> {
        let cur = tables::load_settings(tx)?;
        let secrets = tables::load_secrets(tx)?;
        let revs = tables::key_revs(&secrets);
        let plan = plan_update(&cur, revs, &ctx.body).map_err(update_error)?;
        let mut parts: Vec<&'static str> = Vec::new();
        for (name, change) in [
            (SecretName::ApiKey, &plan.api_key),
            (SecretName::GatewayKey, &plan.gateway_key),
        ] {
            match change {
                KeyChange::Keep => {}
                KeyChange::Remove => {
                    if tables::delete_secret(tx, name)? {
                        parts.push(match name {
                            SecretName::ApiKey => "the API key was removed",
                            SecretName::GatewayKey => "the gateway key was removed",
                        });
                    }
                }
                KeyChange::Set(k) => {
                    let master = app.0.master().ok_or_else(ApiError::key_problem)?;
                    let sealed = master
                        .seal(name, k.as_bytes())
                        .ok_or_else(ApiError::internal)?;
                    tables::put_secret(tx, name, &key_id, &sealed, &secret::hint(k), now)?;
                    parts.push(match name {
                        SecretName::ApiKey => "the API key was replaced",
                        SecretName::GatewayKey => "the gateway key was replaced",
                    });
                }
            }
        }
        if plan.url_changed {
            parts.push("the base URL was changed");
        }
        let new_revs = tables::key_revs(&tables::load_secrets(tx)?);
        let mut next = plan.next;
        let fp_changed = fingerprint(&cur, revs, &key_id) != fingerprint(&next, new_revs, &key_id);
        let mut switched_off = false;
        if fp_changed {
            switched_off = next.enabled;
            next.enabled = false;
            next.verified_at = None;
            next.verified_fp = None;
            next.usage_option = true;
            next.max_tokens = 8192;
        }
        let retention = (plan.retention_sent && next.retention_days != cur.retention_days)
            .then_some(next.retention_days);
        next.updated_at = now;
        next.updated_by = who.username.clone();
        tables::save_settings(tx, &next)?;
        Ok(Saved {
            switched_off,
            fp_changed,
            keys_changed: !(plan.api_key.is_keep() && plan.gateway_key.is_keep()),
            connection: plan.connection_fields,
            retention,
            parts,
        })
    })?;
    let snap = app.0.snapshot()?;
    if saved.keys_changed {
        app.0.recheck_keys(&snap.secrets);
    }
    if saved.fp_changed {
        *app.0.models_cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
    if saved.connection {
        let model = if snap.settings.model.is_empty() {
            "none"
        } else {
            snap.settings.model.as_str()
        };
        let mut extra = String::new();
        for p in &saved.parts {
            extra.push_str("; ");
            extra.push_str(p);
        }
        tracing::info!(
            "user {:?} saved the AI settings (provider: {}, model: {model}{extra})",
            who.username,
            snap.settings.provider.display()
        );
    }
    if let Some(n) = saved.retention {
        tracing::info!(
            "user {:?} set the AI chat retention to {n} days",
            who.username
        );
    }
    if saved.switched_off {
        tracing::info!(
            "AI was switched off: its settings changed and need a new test (by user {:?})",
            who.username
        );
        app.0.cancel_everything(StopReason::Disabled);
        app.0.heartbeat_now().await;
    }
    let snap = app.0.snapshot()?;
    let mut m = settings_json(app, &snap);
    m.insert("switchedOff".into(), Value::Bool(saved.switched_off));
    ok(Value::Object(m))
}

/// Writes a usage row for an administrator's call (the administrator's AI user row is ensured).
fn admin_usage(app: &AppState, who: &crate::identity::Identity, u: stats::UsageRow) {
    let now = u.at;
    let r = app.0.store.write(|tx| {
        data::ensure_user(tx, who, now)?;
        stats::insert_usage(tx, &u)
    });
    if let Err(e) = r {
        tracing::warn!("could not record AI usage: {e}");
    }
}

pub async fn models(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &[])?;
    let app = &ctx.app;
    let who = &ctx.who;
    let snap = app.0.snapshot()?;
    if let Some(k) = snap.missing_key() {
        return Err(ApiError::key_missing(k));
    }
    {
        let cache = app.0.models_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = cache.as_ref() {
            if c.fp == snap.fp && c.at.elapsed() < MODELS_TTL {
                return ok(json!({"models": c.list.models, "listed": c.list.listed}));
            }
        }
    }
    let target = app.0.target(&snap)?;
    let permit = app.0.gate.try_acquire().ok_or_else(ApiError::server_busy)?;
    let started = Instant::now();
    let result = provider::list_models(&app.0.client, &target, app.0.timeouts).await;
    drop(target);
    drop(permit);
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
            provider: snap.settings.provider.as_str(),
            // Listing the models uses no model: always the '' row (the "Model list" row of AI settings › Usage)
            model: String::new(),
            tokens_in: None,
            tokens_out: None,
            tokens_cached: None,
            latency_ms: provider::ms(started.elapsed()),
            first_byte_ms: None,
            status,
            error_code: code,
        },
    );
    let list = result.map_err(|e| ApiError::upstream(e.code))?;
    if list.listed {
        tracing::info!(
            "user {:?} listed the AI models: {} found",
            who.username,
            list.models.len()
        );
    } else {
        tracing::info!(
            "user {:?} listed the AI models: the service does not list models",
            who.username
        );
    }
    *app.0.models_cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(ModelsCache {
        fp: snap.fp.clone(),
        at: Instant::now(),
        list: list.clone(),
    });
    ok(json!({"models": list.models, "listed": list.listed}))
}

/// One test call: `Ok(latency)` or the upstream error.
async fn verify_attempt(
    ctx: &Ctx,
    snap: &Snapshot,
    usage_option: bool,
    max_tokens: i64,
) -> (Result<(), UpError>, i64) {
    let app = &ctx.app;
    let s = &snap.settings;
    let started = Instant::now();
    let turns = [Turn {
        role: Role::User,
        content: prompt::VERIFY_MESSAGE.to_string(),
    }];
    // The test keeps its v0.9.1 body: no token field for OpenAI-compatible services, the stored value for Anthropic
    let token = match s.wire_format() {
        Format::OpenAi => provider::TokenLimit::omit(),
        Format::Anthropic => provider::TokenLimit::max_tokens(max_tokens),
    };
    let body = provider::chat_body(
        s.wire_format(),
        &s.model,
        prompt::VERIFY_PROMPT,
        &turns,
        usage_option,
        token,
    );
    let result = match app.0.target(snap) {
        Err(_) => Err(UpError::new(UpCode::BadRequest, None)),
        Ok(target) => match target.chat_request(&app.0.client, &body, false) {
            Err(e) => Err(e),
            Ok(req) => {
                drop(target);
                let t = app.0.timeouts;
                provider::collect_chat(
                    &app.0.client,
                    provider::Plan::single(s.wire_format(), req),
                    t,
                    t.verify_total,
                )
                .await
            }
        },
    };
    let latency = provider::ms(started.elapsed());
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
        &ctx.who,
        stats::UsageRow {
            user_id: Some(ctx.who.id),
            at: unix_now(),
            kind: "verify",
            provider: s.provider.as_str(),
            model: s.model.clone(),
            tokens_in: tin,
            tokens_out: tout,
            tokens_cached: None,
            latency_ms: latency,
            // time to the first text is recorded for chat answers only
            first_byte_ms: None,
            status: if outcome.is_ok() { "ok" } else { "error" },
            error_code: outcome.as_ref().err().map(|e| e.code.short().to_string()),
        },
    );
    (outcome, latency)
}

pub async fn verify(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &[])?;
    let app = &ctx.app;
    let who = &ctx.who;
    let snap = app.0.snapshot()?;
    if snap.settings.model.is_empty() {
        return Err(ApiError::model_missing());
    }
    if let Some(k) = snap.missing_key() {
        return Err(ApiError::key_missing(k));
    }
    // Decrypts once to find a key problem before anything is sent
    drop(app.0.target(&snap)?);
    let permit = app.0.gate.try_acquire().ok_or_else(ApiError::server_busy)?;
    let s = &snap.settings;
    let format = s.wire_format();
    let mut usage_option = s.usage_option;
    let mut max_tokens = s.max_tokens;
    let (mut result, mut latency) = verify_attempt(&ctx, &snap, usage_option, max_tokens).await;
    if let Err(e) = result {
        if e.code == UpCode::BadRequest {
            if format == Format::OpenAi && usage_option {
                usage_option = false;
                (result, latency) = verify_attempt(&ctx, &snap, usage_option, max_tokens).await;
            } else if format == Format::Anthropic && max_tokens == 8192 {
                max_tokens = 4096;
                (result, latency) = verify_attempt(&ctx, &snap, usage_option, max_tokens).await;
            }
        }
    }
    drop(permit);
    let now = unix_now();
    let fp0 = snap.fp.clone();
    let key_id = app.0.key_id().to_string();
    let mut switched_off = false;
    match result {
        Ok(()) => {
            app.0.store.write(|tx| -> rusqlite::Result<()> {
                let mut cur = tables::load_settings(tx)?;
                let revs = tables::key_revs(&tables::load_secrets(tx)?);
                if fingerprint(&cur, revs, &key_id) == fp0 {
                    cur.verified_at = Some(now);
                    cur.verified_fp = Some(fp0.clone());
                    cur.usage_option = usage_option;
                    cur.max_tokens = max_tokens;
                    tables::save_settings(tx, &cur)?;
                }
                Ok(())
            })?;
            tracing::info!(
                "user {:?} tested the AI settings: OK (model {}, {latency} ms)",
                who.username,
                s.model
            );
        }
        Err(e) => {
            if e.code.is_config() {
                switched_off = app.0.store.write(|tx| -> rusqlite::Result<bool> {
                    let mut cur = tables::load_settings(tx)?;
                    let revs = tables::key_revs(&tables::load_secrets(tx)?);
                    if fingerprint(&cur, revs, &key_id) != fp0 {
                        return Ok(false);
                    }
                    let was = cur.enabled;
                    cur.enabled = false;
                    cur.verified_at = None;
                    cur.verified_fp = None;
                    tables::save_settings(tx, &cur)?;
                    Ok(was)
                })?;
            }
            let status = e.http.map_or_else(|| "-".to_string(), |s| s.to_string());
            tracing::info!(
                "user {:?} tested the AI settings: failed ({}, HTTP {status})",
                who.username,
                e.code.code()
            );
            if switched_off {
                tracing::info!(
                    "AI was switched off: the test failed ({}) (by user {:?})",
                    e.code.code(),
                    who.username
                );
                app.0.cancel_everything(StopReason::Disabled);
                app.0.heartbeat_now().await;
            }
        }
    }
    let after = app.0.snapshot()?;
    let (ok_flag, code, http) = match result {
        Ok(()) => (true, Value::Null, Value::Null),
        Err(e) => (
            false,
            Value::from(e.code.code()),
            e.http.map_or(Value::Null, Value::from),
        ),
    };
    ok(json!({
        "ok": ok_flag,
        "code": code,
        "httpStatus": http,
        "latencyMs": latency,
        "model": s.model,
        "verifiedAt": if after.verified() { after.settings.verified_at } else { None },
        "enabled": after.effective(),
        "switchedOff": switched_off,
    }))
}

pub async fn enable(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["enabled"])?;
    let on = match ctx.body.get("enabled") {
        Some(Value::Bool(b)) => *b,
        _ => return Err(ApiError::bad_request()),
    };
    let app = &ctx.app;
    let who = &ctx.who;
    let snap = app.0.snapshot()?;
    if on {
        if snap.key_problem.is_some() {
            return Err(ApiError::key_problem());
        }
        if !snap.verified() {
            return Err(ApiError::not_verified());
        }
    }
    let changed = app.0.store.write(|tx| -> rusqlite::Result<bool> {
        let mut cur = tables::load_settings(tx)?;
        if cur.enabled == on {
            return Ok(false);
        }
        cur.enabled = on;
        tables::save_settings(tx, &cur)?;
        Ok(true)
    })?;
    if changed {
        if on {
            tracing::info!("user {:?} enabled AI", who.username);
        } else {
            tracing::info!("user {:?} disabled AI", who.username);
        }
    }
    if !on {
        app.0.cancel_everything(StopReason::Disabled);
    }
    app.0.heartbeat_now().await;
    let snap = app.0.snapshot()?;
    ok(Value::Object(settings_json(app, &snap)))
}

fn sums_json(s: &stats::Sums) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("calls".into(), Value::from(s.calls));
    m.insert("tokensIn".into(), Value::from(s.tokens_in));
    m.insert("tokensOut".into(), Value::from(s.tokens_out));
    m.insert("errors".into(), Value::from(s.errors));
    m
}

pub async fn usage(ctx: Ctx) -> Answer {
    let days: i64 = match query_text(ctx.query.as_deref(), "days")? {
        None => 7,
        Some("7") => 7,
        Some("30") => 30,
        Some("90") => 90,
        Some("365") => 365,
        Some(_) => return Err(ApiError::bad_request()),
    };
    let to = crate::util::local_today();
    let from = to
        .checked_sub(jiff::Span::new().days(days - 1))
        .map_err(|_| ApiError::internal())?;
    let monthly = days >= 90;
    let (from_s, to_s) = (from.to_string(), to.to_string());
    let app = &ctx.app.0;
    let (totals, users, models, periods, purposes) = app.store.read(|c| {
        Ok((
            stats::totals(c, &from_s, &to_s)?,
            stats::by_user(c, &from_s, &to_s)?,
            stats::by_model(c, &from_s, &to_s)?,
            stats::by_period(c, &from_s, &to_s, if monthly { 7 } else { 10 })?,
            stats::by_purpose(c, &from_s, &to_s)?,
        ))
    })?;
    // Each listed user's counted calls since today's 0:00 (the number the day cap compares), independent of the period
    let ids: Vec<i64> = users.iter().filter_map(|(id, ..)| *id).collect();
    let today = super::user::call_states(app, &ids, unix_now())?;
    let by_user: Vec<Value> = users
        .iter()
        .map(|(id, name, s, last)| {
            let mut m = Map::new();
            m.insert("userId".into(), id.map_or(Value::Null, Value::from));
            m.insert(
                "username".into(),
                match (id, name) {
                    (Some(_), Some(n)) => Value::from(n.clone()),
                    _ => Value::Null,
                },
            );
            m.extend(sums_json(s));
            m.insert("lastAt".into(), Value::from(*last));
            if let Some(st) = id.and_then(|i| today.get(&i)) {
                m.insert("today".into(), Value::from(st.used.day));
            }
            m.insert("tokensCached".into(), Value::from(s.tokens_cached));
            Value::Object(m)
        })
        .collect();
    let by_model: Vec<Value> = models
        .iter()
        .map(|(model, s)| {
            let mut m = Map::new();
            m.insert("model".into(), Value::from(model.clone()));
            m.extend(sums_json(s));
            m.insert("tokensCached".into(), Value::from(s.tokens_cached));
            Value::Object(m)
        })
        .collect();
    // Every period of the range, zeros included, oldest first
    let mut keys: Vec<String> = Vec::new();
    if monthly {
        let mut d = from.first_of_month();
        let end = to.first_of_month();
        while d <= end {
            keys.push(format!("{:04}-{:02}", d.year(), d.month()));
            d = match d.checked_add(jiff::Span::new().months(1)) {
                Ok(n) => n,
                Err(_) => break,
            };
        }
    } else {
        let mut d = from;
        while d <= to {
            keys.push(d.to_string());
            d = match d.tomorrow() {
                Ok(n) => n,
                Err(_) => break,
            };
        }
    }
    let by_period: Vec<Value> = keys
        .iter()
        .map(|k| {
            let s = periods
                .iter()
                .find(|(p, _)| p == k)
                .map(|(_, s)| s.clone())
                .unwrap_or_default();
            let mut m = Map::new();
            m.insert("period".into(), Value::from(k.clone()));
            m.extend(sums_json(&s));
            m.insert("tokensCached".into(), Value::from(s.tokens_cached));
            Value::Object(m)
        })
        .collect();
    let by_purpose: Vec<Value> = purposes
        .iter()
        .map(|(purpose, s)| {
            let mut m = Map::new();
            m.insert("purpose".into(), Value::from(*purpose));
            m.extend(sums_json(s));
            m.insert("tokensCached".into(), Value::from(s.tokens_cached));
            Value::Object(m)
        })
        .collect();
    let mut totals_json = sums_json(&totals);
    totals_json.insert("tokensCached".into(), Value::from(totals.tokens_cached));
    ok(json!({
        "days": days,
        "from": from_s,
        "to": to_s,
        "unit": if monthly { "month" } else { "day" },
        "totals": Value::Object(totals_json),
        "byUser": by_user,
        "byModel": by_model,
        "byPeriod": by_period,
        "byPurpose": by_purpose,
    }))
}

/// One row of "Users with AI+" (`AdminUserRow`, round-2 contracts 3.5). `exists` is always true (kept for the shape).
fn admin_row(
    u: &BackendUser,
    c: &stats::UserCounts,
    suspended: bool,
    calls: Option<&super::user::CallState>,
) -> Value {
    let mut row = json!({
        "id": u.id,
        "username": u.username,
        "role": u.role,
        "ai": u.ai,
        "aiAllowed": u.ai_allowed,
        "disabled": u.disabled,
        "exists": true,
        "suspended": suspended,
        "chats": c.chats,
        "messages": c.messages,
        "memories": c.memories,
        "lastUsedAt": c.last_used_at,
        "tokensIn": c.tokens_in,
        "tokensOut": c.tokens_out,
    });
    if let (Some(st), Some(m)) = (calls, row.as_object_mut()) {
        m.insert("calls".into(), super::user::calls_json(st, None));
    }
    row
}

/// The usage counts of a user and whether the user's AI+ function is suspended (never for administrators).
fn counts_and_suspended(
    app: &AppState,
    u: &BackendUser,
) -> rusqlite::Result<(stats::UserCounts, bool)> {
    app.0.store.read(|c| {
        let counts = stats::user_counts(c, u.id)?;
        let on = u.role == "admin" || data::function_enabled(c, u.id)?;
        Ok((counts, !on))
    })
}

/// Users with AI+: every administrator and every user with the AI+ service (`ai = true` in the main backend),
/// suspended ones included, sorted by username. Users without the service are not listed, also when they still have
/// AI data (they can read and delete it themselves; retention applies).
pub async fn users(ctx: Ctx) -> Answer {
    let app = &ctx.app;
    let answer = retention::reconcile_once(app, true, &[])
        .await
        .map_err(|_| ApiError::backend_down())?;
    let listed: Vec<&BackendUser> = answer
        .iter()
        .filter(|u| u.role == "admin" || u.ai)
        .collect();
    // One read for every listed user's call counts (finished calls grouped in Rust, plus the running ones)
    let ids: Vec<i64> = listed.iter().map(|u| u.id).collect();
    let calls = super::user::call_states(&app.0, &ids, unix_now())?;
    let mut rows: Vec<(String, Value)> = Vec::new();
    for u in listed {
        let (c, suspended) = counts_and_suspended(app, u)?;
        rows.push((
            u.username.clone(),
            admin_row(u, &c, suspended, calls.get(&u.id)),
        ));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let list: Vec<Value> = rows.into_iter().map(|(_, v)| v).collect();
    ok(json!({ "users": list }))
}

pub async fn user(ctx: Ctx) -> Answer {
    let app = &ctx.app;
    let id = query_id(ctx.query.as_deref(), "id")?;
    let answer = retention::reconcile_once(app, true, &[id])
        .await
        .map_err(|_| ApiError::backend_down())?;
    let Some(u) = answer.iter().find(|u| u.id == id) else {
        return Err(ApiError::not_found());
    };
    let (counts, suspended) = counts_and_suspended(app, u)?;
    let calls = super::user::call_states(&app.0, &[id], unix_now())?;
    let (chats, memories) = app
        .0
        .store
        .read(|c| Ok((data::list_chats(c, id)?, data::list_memories(c, id)?)))?;
    let chats: Vec<Value> = chats
        .iter()
        .map(|c| c.to_json(app.0.chat_running(c.id)))
        .collect();
    let memories: Vec<Value> = memories.iter().map(data::MemoryRow::to_json).collect();
    let line = format!(
        "user {:?} viewed the AI chats and memories of user {:?}",
        ctx.who.username, u.username
    );
    if !app.0.audit_now(&line).await {
        return Err(ApiError::backend_down());
    }
    ok(json!({
        "user": admin_row(u, &counts, suspended, calls.get(&id)),
        "chats": chats,
        "memories": memories,
    }))
}

/// `POST /api/ai/admin/suspend` (`enabled = false`) and `POST /api/ai/admin/resume` (`enabled = true`), body
/// `{"userId": N}` (round-2 contracts 3.2): sets the AI+ function of one plain user. The flag and its audit line are
/// written in one transaction (only when the value changed); suspending stops the user's running calls at once.
pub async fn set_function(ctx: Ctx, enabled: bool) -> Answer {
    only_keys(&ctx.body, &["userId"])?;
    let id = body_id(&ctx.body, "userId")?.ok_or_else(ApiError::bad_request)?;
    let app = &ctx.app;
    let found = app
        .0
        .link
        .users(&[id], false)
        .await
        .map_err(|_| ApiError::backend_down())?;
    let Some(u) = found.into_iter().find(|u| u.id == id) else {
        return Err(ApiError::not_found());
    };
    if u.role == "admin" {
        return Err(ApiError::admin_always());
    }
    // An AI row of another account with the same id is removed first (7.4)
    if let Some((stamp, _)) = app.0.store.read(|c| data::user_row(c, id))? {
        if stamp != u.stamp {
            app.0.purge_user(id).await;
            tracing::info!(
                "Removed the AI data of a user id that now belongs to another account (id {id})"
            );
        }
    }
    let admin = &ctx.who.username;
    let line = if enabled {
        format!(
            "AI function turned on for user {:?} by admin {admin:?}",
            u.username
        )
    } else {
        format!(
            "AI function turned off for user {:?} by admin {admin:?}",
            u.username
        )
    };
    let now = unix_now();
    let changed = app.0.store.write(|tx| -> rusqlite::Result<bool> {
        let changed = data::set_function_enabled(tx, id, &u.stamp, &u.username, enabled, now)?;
        if changed {
            stats::outbox_add(tx, now, &line)?;
        }
        Ok(changed)
    })?;
    if !enabled {
        app.0
            .registry
            .cancel_where(StopReason::Suspended, |user, _, _| user == id);
    }
    if changed {
        tracing::info!(target: "audit", "{line}");
        app.0.deliver_outbox().await;
    }
    app.0.heartbeat_now().await;
    let (counts, suspended) = counts_and_suspended(app, &u)?;
    // The Suspend / Resume answer keeps the row without `calls` (the spec adds them to the two list routes only)
    ok(json!({ "user": admin_row(&u, &counts, suspended, None) }))
}

pub async fn chat(ctx: Ctx) -> Answer {
    let app = &ctx.app;
    let user = query_id(ctx.query.as_deref(), "user")?;
    let id = query_id(ctx.query.as_deref(), "id")?;
    let found = app.0.store.read(|c| {
        let Some(chat) = data::chat(c, user, id)? else {
            return Ok(None);
        };
        let name = data::user_row(c, user)?.map(|(_, n)| n).unwrap_or_default();
        Ok(Some((chat, data::messages(c, id)?, name)))
    })?;
    let Some((chat, msgs, owner)) = found else {
        return Err(ApiError::not_found());
    };
    let line = format!(
        "user {:?} viewed an AI chat of user {owner:?} ({} messages)",
        ctx.who.username,
        msgs.len()
    );
    if !app.0.audit_now(&line).await {
        return Err(ApiError::backend_down());
    }
    let messages: Vec<Value> = msgs.iter().map(data::MessageRow::to_json).collect();
    ok(json!({"chat": chat.to_json(app.0.chat_running(id)), "messages": messages}))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClearWhat {
    Chat(i64),
    Chats,
    Memories,
}

pub async fn clear(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["userId", "what", "chatId"])?;
    let user = body_id(&ctx.body, "userId")?.ok_or_else(ApiError::bad_request)?;
    let chat_id = body_id(&ctx.body, "chatId")?;
    let what = match (ctx.body.get("what").and_then(Value::as_str), chat_id) {
        (Some("chat"), Some(c)) => ClearWhat::Chat(c),
        (Some("chats"), None) => ClearWhat::Chats,
        (Some("memories"), None) => ClearWhat::Memories,
        _ => return Err(ApiError::bad_request()),
    };
    let app = &ctx.app;
    let Some((_, owner)) = app.0.store.read(|c| data::user_row(c, user))? else {
        return Err(ApiError::not_found());
    };
    match what {
        ClearWhat::Chat(c) => {
            if app.0.store.read(|x| data::chat(x, user, c))?.is_none() {
                return Err(ApiError::not_found());
            }
            app.0.stop_chats(user, Some(c)).await;
        }
        ClearWhat::Chats => app.0.stop_chats(user, None).await,
        ClearWhat::Memories => {}
    }
    let admin = ctx.who.username.clone();
    let now = unix_now();
    let (n, line) = app
        .0
        .store
        .write(|tx| -> Result<(usize, String), ApiError> {
            let (n, line) = match what {
                ClearWhat::Chat(c) => {
                    let n = data::delete_chat(tx, user, c)?;
                    if n == 0 {
                        return Err(ApiError::not_found());
                    }
                    (
                        n,
                        format!("user {admin:?} cleared one AI chat of user {owner:?}"),
                    )
                }
                ClearWhat::Chats => {
                    let n = data::delete_all_chats(tx, user)?;
                    (
                        n,
                        format!("user {admin:?} cleared all {n} AI chats of user {owner:?}"),
                    )
                }
                ClearWhat::Memories => {
                    let n = data::delete_all_memories(tx, user)?;
                    (
                        n,
                        format!("user {admin:?} cleared all {n} AI memories of user {owner:?}"),
                    )
                }
            };
            stats::outbox_add(tx, now, &line)?;
            Ok((n, line))
        })?;
    app.0.store.schedule_checkpoint();
    tracing::info!(target: "audit", "{line}");
    app.0.deliver_outbox().await;
    ok(json!({ "deleted": n }))
}

pub async fn limits_get(ctx: Ctx) -> Answer {
    ok(ctx.app.0.limits().answer())
}

pub async fn limits_post(ctx: Ctx) -> Answer {
    let app = &ctx.app;
    let cur = app.0.limits();
    let next = match cur.apply_update(&ctx.body) {
        Ok(n) => n,
        Err(LimitError::BadRequest) => return Err(ApiError::bad_request()),
        Err(LimitError::Range(k)) => {
            let (min, max) = DEFS
                .iter()
                .find(|d| d.key == k)
                .map_or((0, 0), |d| (d.min, d.max));
            return Err(ApiError::limit_range(k, min, max));
        }
    };
    // The call caps must stay in order (minute <= hour <= day); refused as a whole
    if let Some(field) = next.order_violation() {
        return Err(ApiError::limit_order(field));
    }
    let now = unix_now();
    let by = ctx.who.username.clone();
    app.0
        .store
        .write(|tx| tables::save_limits(tx, &next, &by, now))?;
    app.0.set_limits(next);
    if cur.reply_max_chars != next.reply_max_chars {
        app.0.forget_token_memo();
    }
    if let Some(change) = cur.describe_change(&next) {
        tracing::info!("AI limits changed by admin {by:?}: {change}");
    }
    ok(next.answer())
}
