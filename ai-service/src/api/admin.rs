//! Administrator routes: settings, models, test, enable, usage, users with AI+ and their data (read only, audited),
//! clear, safety limits, Suspend / Resume of a user's AI+ function (audited).
//!
//! Every handler that changes the effective AI+ service state or a user's AI+ function sends a state report to the
//! main backend before it answers (round-2 contracts 2.6; best effort, the answer does not depend on it).
use super::{body_id, ok, only_keys, query_id, query_text, Answer, Ctx};
use crate::app::AppState;
use crate::chat::StopReason;
use crate::codes::ApiError;
use crate::limits::{LimitError, DEFS};
use crate::link::BackendUser;
use crate::retention;
use crate::store::{data, stats, tables};
use crate::util::unix_now;
use serde_json::{json, Map, Value};

use super::admin_conn::{host_of, slots_using, MAX_CONNECTIONS};
use crate::activity::Event;
use crate::route::Slot;
use crate::store::conn;

/// The state of one secret of a connection as the settings object shows it (never the key).
fn key_json(row: Option<&crate::store::tables::SecretRow>, key_id: &str) -> Value {
    match row {
        Some(r) => json!({"set": true, "hint": r.hint, "unreadable": r.key_id != key_id}),
        None => json!({"set": false, "hint": "", "unreadable": false}),
    }
}

/// The v1.3 settings object (routes 5.1): switch, state, connections, pool, slots and the global feature switches.
pub fn settings_value(app: &AppState) -> Result<Value, ApiError> {
    let snap = app.0.snapshot()?;
    let view = app.0.routes()?;
    let effective = snap.enabled && snap.key_problem.is_none() && view.any();
    let configured = view.slots[Slot::Default.index()].is_some();
    let (state, reason) = if !configured && !snap.enabled {
        ("setup", Value::Null)
    } else if !snap.enabled {
        ("off", Value::Null)
    } else if effective {
        ("on", Value::Null)
    } else if snap.key_problem.is_some() {
        ("paused", Value::from("keyProblem"))
    } else {
        ("paused", Value::from("noRoute"))
    };
    let row = app.0.store.read(tables::load_settings)?;
    let key_id = app
        .0
        .master()
        .map_or(String::new(), |m| m.key_id().to_string());
    let secrets = app.0.store.read(|c| {
        let mut all = Vec::new();
        for k in &view.conns {
            all.push((k.id, conn::conn_secrets(c, k.id)?));
        }
        Ok(all)
    })?;
    let connections: Vec<Value> = view
        .conns
        .iter()
        .map(|c| {
            let rows = secrets
                .iter()
                .find(|(id, _)| *id == c.id)
                .map(|(_, r)| r.as_slice())
                .unwrap_or(&[]);
            let find = |n: crate::secret::SecretName| rows.iter().find(|r| r.name == n);
            json!({
                "id": c.id,
                "rev": c.rev,
                "name": c.name,
                "provider": c.provider,
                "baseUrl": c.base_url,
                "host": host_of(&c.base_url),
                "format": c.format,
                "gatewayAuth": c.gateway_auth,
                "keys": {
                    "apiKey": key_json(find(crate::secret::SecretName::ApiKey), &key_id),
                    "gatewayKey": key_json(find(crate::secret::SecretName::GatewayKey), &key_id),
                },
                "status": c.status,
                "errorCode": c.error_code,
                "testedAt": c.tested_at,
                "listsModels": c.lists_models,
                "insecureHttp": crate::settings::insecure_http(&c.base_url),
                "usedBy": slots_using(&view, c.id),
            })
        })
        .collect();
    let pool: Vec<Value> = view
        .pool
        .iter()
        .map(|m| {
            let used: Vec<&'static str> = Slot::ALL
                .iter()
                .filter(|s| view.slots[s.index()] == Some(m.id))
                .map(|s| s.as_str())
                .collect();
            json!({
                "id": m.id,
                "connectionId": m.connection_id,
                "connectionName": view.conn(m.connection_id).map_or("", |c| c.name.as_str()),
                "model": m.model,
                "manual": m.manual,
                "status": m.status,
                "errorCode": m.error_code,
                "testedAt": m.tested_at,
                "latencyMs": m.latency_ms,
                "context": m.context,
                "usedBy": used,
            })
        })
        .collect();
    let mut slots = Map::new();
    for s in Slot::ALL {
        let using = view
            .resolve(s, None)
            .ok()
            .map(|r| json!({"modelId": r.pool_id, "fallback": r.fallback.as_str()}));
        slots.insert(
            s.as_str().to_string(),
            json!({
                "modelId": view.slots[s.index()],
                "using": using,
                "effort": view.efforts[s.index()].map(crate::effort::Effort::to_json),
            }),
        );
    }
    Ok(json!({
        "enabled": snap.enabled,
        "effective": effective,
        "state": state,
        "pauseReason": reason,
        "keyProblem": snap.key_problem.map(|p| p.as_str()),
        "retentionDays": snap.retention_days,
        "updatedAt": row.updated_at,
        "updatedBy": row.updated_by,
        "defaults": {
            "openai": crate::settings::DEFAULT_OPENAI,
            "anthropic": crate::settings::DEFAULT_ANTHROPIC,
        },
        "maxConnections": MAX_CONNECTIONS,
        "connections": connections,
        "pool": pool,
        "slots": Value::Object(slots),
        "effortLevels": crate::effort::levels_json(),
        "featureSwitches": crate::features::to_json(snap.features_off),
    }))
}

pub async fn settings_get(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &[])?;
    ok(settings_value(&ctx.app)?)
}

/// `POST /api/ai/admin/settings` accepts only `{"retentionDays": 1..3650}` in v1.3.
pub async fn settings_post(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["retentionDays"])?;
    let days = match ctx.body.get("retentionDays") {
        Some(Value::Number(n)) => n
            .as_i64()
            .filter(|d| (1..=3650).contains(d))
            .ok_or_else(ApiError::bad_retention)?,
        Some(_) => return Err(ApiError::bad_request()),
        None => return ok(settings_value(&ctx.app)?),
    };
    let now = unix_now();
    let by = ctx.who.username.clone();
    let changed = ctx.app.0.store.write(|tx| -> rusqlite::Result<bool> {
        let mut cur = tables::load_settings(tx)?;
        if cur.retention_days == days {
            return Ok(false);
        }
        cur.retention_days = days;
        cur.updated_at = now;
        cur.updated_by = by.clone();
        tables::save_settings(tx, &cur)?;
        Ok(true)
    })?;
    if changed {
        tracing::info!(
            "user {:?} set the AI data retention to {days} days",
            ctx.who.username
        );
    }
    ctx.app.0.activity(
        Event::new(Some(&ctx.who), "ai.settings.retention", &ctx.client)
            .detail(json!({"keys": ["retentionDays"]})),
    );
    ok(settings_value(&ctx.app)?)
}

/// `POST /api/ai/admin/enable` `{"enabled": bool, "default": ModelRef?}`. Turning on needs usable keys, a default model
/// (`default` is required while the default slot is empty and is tested like a pick: a failing test leaves AI+ off) and a
/// slot that resolves. Turning off stops every running call.
pub async fn enable(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["enabled", "default"])?;
    let on = match ctx.body.get("enabled") {
        Some(Value::Bool(b)) => *b,
        _ => return Err(ApiError::bad_request()),
    };
    let app = &ctx.app;
    let who = &ctx.who;
    let mut latency: Option<i64> = None;
    if on {
        if app.0.key_problem().is_some() {
            return Err(ApiError::key_problem());
        }
        let default = match ctx.body.get("default") {
            None | Some(Value::Null) => None,
            Some(v) => Some(crate::route::Pick::from_json(v)?.ok_or_else(ApiError::bad_model)?),
        };
        let view = app.0.routes()?;
        if default.is_none() && view.slots[Slot::Default.index()].is_none() {
            return Err(ApiError::model_missing());
        }
        if let Some(p) = &default {
            let picked = super::admin_models::pick_into(
                app,
                who,
                Slot::Default,
                p.connection_id,
                &p.model,
                false,
            )
            .await?;
            latency = picked.latency_ms;
            if !picked.ok {
                let code = picked.error.map(|e| e.code.code());
                app.0.activity(
                    Event::new(Some(who), "ai.settings.enable", &ctx.client)
                        .detail(json!({"model": p.model}))
                        .failed(code.unwrap_or("ai.up.protocol")),
                );
                return ok(json!({
                    "ok": false,
                    "code": code,
                    "httpStatus": picked.error.and_then(|e| e.http),
                    "latencyMs": picked.latency_ms,
                    "enabled": false,
                    "settings": settings_value(app)?,
                }));
            }
        } else if !view.any() {
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
        tracing::info!(
            "user {:?} {} AI",
            who.username,
            if on { "enabled" } else { "disabled" }
        );
    }
    if !on {
        app.0.cancel_everything(StopReason::Disabled);
    }
    // the connection and model of the default slot, for the event (names only)
    let view = app.0.routes()?;
    let route = view.resolve(Slot::Default, None).ok();
    let detail = json!({
        "connection": route.as_ref().map(|r| r.connection_name.clone()),
        "model": route.as_ref().map(|r| r.model.clone()),
    });
    app.0.activity(
        Event::new(
            Some(who),
            if on {
                "ai.settings.enable"
            } else {
                "ai.settings.disable"
            },
            &ctx.client,
        )
        .detail(detail),
    );
    app.0.heartbeat_now().await;
    let settings = settings_value(app)?;
    ok(json!({
        "ok": true,
        "code": null,
        "httpStatus": null,
        "latencyMs": latency,
        "enabled": settings["effective"],
        "settings": settings,
    }))
}

/// `POST /api/ai/admin/feature-switches` (any subset of the six keys, booleans: `true` = on): the global switches. They
/// apply to normal users only; administrators keep every feature. Running calls of normal users stop at their next
/// access re-check.
pub async fn feature_switches(ctx: Ctx) -> Answer {
    let mut set: Vec<(crate::features::Feature, bool)> = Vec::new();
    for (k, v) in &ctx.body {
        let f = crate::features::Feature::parse(k).ok_or_else(ApiError::bad_request)?;
        let Value::Bool(b) = v else {
            return Err(ApiError::bad_request());
        };
        set.push((f, *b));
    }
    let app = &ctx.app;
    let before = app.0.store.read(conn::global_features_off)?;
    let mut after = before;
    for (f, on) in &set {
        if *on {
            after &= !f.bit();
        } else {
            after |= f.bit();
        }
    }
    // AI hint and AI analysis are one switch: the two bits change together
    let after = crate::features::paired(before, after);
    if after != before {
        app.0.store.write(|tx| conn::set_features_off(tx, after))?;
        // Normal users' running calls of a feature that was just switched off stop; administrators' calls go on.
        let newly_off = after & !before;
        if newly_off != 0 {
            app.0.registry.cancel_features(newly_off);
        }
        tracing::info!(
            "user {:?} changed the global AI features: {}",
            ctx.who.username,
            crate::features::describe_change(before, after).unwrap_or_default()
        );
    }
    let list = |want_on: bool| -> Vec<&'static str> {
        crate::features::ALL
            .iter()
            .filter(|f| {
                crate::features::allowed(before, **f) != crate::features::allowed(after, **f)
                    && crate::features::allowed(after, **f) == want_on
            })
            .map(|f| f.key())
            .collect()
    };
    app.0.activity(
        Event::new(Some(&ctx.who), "ai.settings.features", &ctx.client)
            .detail(json!({"on": list(true), "off": list(false)})),
    );
    app.0.heartbeat_now().await;
    ok(settings_value(app)?)
}

fn sums_json(s: &stats::Sums) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("calls".into(), Value::from(s.calls));
    m.insert("tokensIn".into(), Value::from(s.tokens_in));
    m.insert("tokensOut".into(), Value::from(s.tokens_out));
    m.insert("errors".into(), Value::from(s.errors));
    m
}

/// The period of the usage answer: `days=7|30|90|365` (calendar days, today included), or `period=12h` (the last 12
/// hours) / `period=today` (since today's 0:00, server time zone). Both together, or any other value, is a 400; neither
/// means 7 days.
enum UsagePeriod {
    Days(i64),
    Last12Hours,
    Today,
}

fn usage_period(query: Option<&str>) -> Result<UsagePeriod, ApiError> {
    let days = query_text(query, "days")?;
    let period = query_text(query, "period")?;
    match (days, period) {
        (Some(_), Some(_)) => Err(ApiError::bad_request()),
        (_, Some("12h")) => Ok(UsagePeriod::Last12Hours),
        (_, Some("today")) => Ok(UsagePeriod::Today),
        (_, Some(_)) => Err(ApiError::bad_request()),
        (None | Some("7"), None) => Ok(UsagePeriod::Days(7)),
        (Some("30"), None) => Ok(UsagePeriod::Days(30)),
        (Some("90"), None) => Ok(UsagePeriod::Days(90)),
        (Some("365"), None) => Ok(UsagePeriod::Days(365)),
        (Some(_), None) => Err(ApiError::bad_request()),
    }
}

pub async fn usage(ctx: Ctx) -> Answer {
    let period = usage_period(ctx.query.as_deref())?;
    let to = crate::util::local_today();
    let now = unix_now();
    let day_start = crate::util::day_bounds(now).start;
    let (window, hourly_from, days) = match period {
        UsagePeriod::Days(d) => {
            let from = to
                .checked_sub(jiff::Span::new().days(d - 1))
                .map_err(|_| ApiError::internal())?;
            (
                stats::Window::days(&from.to_string(), &to.to_string()),
                None,
                Some((from, d)),
            )
        }
        UsagePeriod::Last12Hours => (
            stats::Window::Since(now - 12 * 3600),
            Some(now - 12 * 3600),
            None,
        ),
        UsagePeriod::Today => (stats::Window::Since(day_start), Some(day_start), None),
    };
    // `from` / `to`: the first and last calendar day (the short windows: today's date twice)
    let from_s = days.map_or_else(|| to.to_string(), |(f, _)| f.to_string());
    let to_s = to.to_string();
    let period_name = match period {
        UsagePeriod::Days(d) => format!("{d}d"),
        UsagePeriod::Last12Hours => "12h".to_string(),
        UsagePeriod::Today => "today".to_string(),
    };
    let monthly = days.is_some_and(|(_, d)| d >= 90);
    let app = &ctx.app.0;
    let today_s = to_s.clone();
    let (totals, users, models, periods, purposes, conns, shared, used_today) =
        app.store.read(|c| {
            Ok((
                stats::totals(c, &window)?,
                stats::by_user(c, &window)?,
                stats::by_model_connection(c, &window)?,
                if hourly_from.is_some() {
                    stats::by_hour(c, &window)?
                } else {
                    stats::by_period(c, &window, if monthly { 7 } else { 10 })?
                },
                stats::by_purpose(c, &window)?,
                stats::by_connection(c, &window)?,
                crate::store::shared::shared_stats(c, day_start)?,
                stats::tokens_today(c, &today_s)?,
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
            m.insert("tokensCacheWrite".into(), Value::from(s.tokens_cache_write));
            Value::Object(m)
        })
        .collect();
    let by_model: Vec<Value> = models
        .iter()
        .map(|u| {
            let (model, s) = (&u.model, &u.sums);
            let mut m = Map::new();
            m.insert("model".into(), Value::from(model.clone()));
            m.insert("connectionId".into(), json!(u.connection_id));
            m.insert("connection".into(), json!(u.connection));
            // the empty name is the "other" row: the model list and the connection tests, which use no feature
            m.insert("other".into(), Value::Bool(model.is_empty()));
            m.extend(sums_json(s));
            m.insert("tokensCached".into(), Value::from(s.tokens_cached));
            m.insert("tokensCacheWrite".into(), Value::from(s.tokens_cache_write));
            Value::Object(m)
        })
        .collect();
    // Every period of the range, zeros included, oldest first
    let mut keys: Vec<String> = Vec::new();
    if let Some(start) = hourly_from {
        // one key per local hour from the window's start to now
        let mut t = start;
        while t <= now {
            let k = stats::hour_label(t);
            if keys.last() != Some(&k) {
                keys.push(k);
            }
            t += 1800;
        }
        let last = stats::hour_label(now);
        if keys.last() != Some(&last) {
            keys.push(last);
        }
    } else if let Some((from, _)) = days {
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
            m.insert("tokensCacheWrite".into(), Value::from(s.tokens_cache_write));
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
            m.insert("tokensCacheWrite".into(), Value::from(s.tokens_cache_write));
            Value::Object(m)
        })
        .collect();
    let mut totals_json = sums_json(&totals);
    totals_json.insert("tokensCached".into(), Value::from(totals.tokens_cached));
    totals_json.insert(
        "tokensCacheWrite".into(),
        Value::from(totals.tokens_cache_write),
    );
    let by_connection: Vec<Value> = conns
        .iter()
        .map(|u| {
            let mut m = Map::new();
            m.insert("connectionId".into(), json!(u.connection_id));
            m.insert("connection".into(), json!(u.connection));
            m.extend(sums_json(&u.sums));
            m.insert("tokensCached".into(), Value::from(u.sums.tokens_cached));
            m.insert(
                "tokensCacheWrite".into(),
                Value::from(u.sums.tokens_cache_write),
            );
            Value::Object(m)
        })
        .collect();
    ok(json!({
        "days": days.map(|(_, d)| d),
        "period": period_name,
        "from": from_s,
        "to": to_s,
        "unit": if hourly_from.is_some() { "hour" } else if monthly { "month" } else { "day" },
        "totals": Value::Object(totals_json),
        "byUser": by_user,
        "byModel": by_model,
        "byPeriod": by_period,
        "byPurpose": by_purpose,
        "byConnection": by_connection,
        "shared": {"rows": shared.0, "hits": shared.1, "hitsToday": shared.2},
        "budget": {"limit": app.limits().daily_token_budget, "usedToday": used_today},
    }))
}

/// An administrator who is exempt from the caps is never refused, so no block is listed for one.
fn clear_exempt_blocks(
    app: &AppState,
    users: &[&BackendUser],
    calls: &mut std::collections::HashMap<i64, super::user::CallState>,
) {
    if app.0.limits().calls_admins_exempt != 1 {
        return;
    }
    for u in users.iter().filter(|u| u.role == "admin") {
        if let Some(st) = calls.get_mut(&u.id) {
            st.blocked = None;
        }
    }
}

/// One row of "Users with AI+" (`AdminUserRow`, round-2 contracts 3.5). `exists` is always true (kept for the shape).
fn admin_row(
    u: &BackendUser,
    c: &stats::UserCounts,
    suspended: bool,
    features_off: i64,
    free: bool,
    calls: Option<&super::user::CallState>,
) -> Value {
    let features = if u.role == "admin" {
        crate::features::all_on_json()
    } else {
        crate::features::to_json(features_off)
    };
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
        "features": features,
    });
    // A-OWNER-21: an administrator row carries its free mode (the switch in AI settings > AI+ users)
    if let (true, Some(m)) = (u.role == "admin", row.as_object_mut()) {
        m.insert("freeMode".into(), Value::Bool(free));
    }
    if let (Some(st), Some(m)) = (calls, row.as_object_mut()) {
        m.insert("calls".into(), super::user::calls_json(st, None));
    }
    row
}

/// The usage counts of a user and whether the user's AI+ function is suspended (never for administrators).
fn counts_and_suspended(
    app: &AppState,
    u: &BackendUser,
) -> rusqlite::Result<(stats::UserCounts, bool, i64, bool)> {
    app.0.store.read(|c| {
        let counts = stats::user_counts(c, u.id)?;
        let on = u.role == "admin" || data::function_enabled(c, u.id)?;
        let off = data::features_off(c, u.id)?;
        let free = u.role == "admin" && data::free_mode(c, u.id)?;
        Ok((counts, !on, off, free))
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
    let mut calls = super::user::call_states(&app.0, &ids, unix_now())?;
    clear_exempt_blocks(app, &listed, &mut calls);
    let mut rows: Vec<(String, Value)> = Vec::new();
    for u in listed {
        let (c, suspended, off, free) = counts_and_suspended(app, u)?;
        rows.push((
            u.username.clone(),
            admin_row(u, &c, suspended, off, free, calls.get(&u.id)),
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
    let (counts, suspended, off, free) = counts_and_suspended(app, u)?;
    let mut calls = super::user::call_states(&app.0, &[id], unix_now())?;
    clear_exempt_blocks(app, &[u], &mut calls);
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
        "user": admin_row(u, &counts, suspended, off, free, calls.get(&id)),
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
    }
    app.0.activity(
        Event::new(Some(&ctx.who), "ai.users.features", &ctx.client)
            .object(u.username.clone())
            .detail(json!({"suspended": !enabled})),
    );
    app.0.deliver_outbox().await;
    app.0.heartbeat_now().await;
    let (counts, suspended, off, free) = counts_and_suspended(app, &u)?;
    // The Suspend / Resume answer keeps the row without `calls` (the spec adds them to the two list routes only)
    ok(json!({ "user": admin_row(&u, &counts, suspended, off, free, None) }))
}

/// `POST /api/ai/admin/features` (API-A2), body `{"userId": N, "features": {six booleans}}`: sets the AI features of one
/// plain user. Works like [`set_function`]: the user is looked up through the backend, an AI row of another account is
/// purged first, the row is created when missing, and the audit line goes through the outbox when something changed.
pub async fn set_features(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["userId", "features"])?;
    let id = body_id(&ctx.body, "userId")?.ok_or_else(ApiError::bad_request)?;
    let asked = match ctx.body.get("features") {
        Some(Value::Object(m)) => {
            crate::features::from_json(m).map_err(|_| ApiError::bad_request())?
        }
        _ => return Err(ApiError::bad_request()),
    };
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
    if let Some((stamp, _)) = app.0.store.read(|c| data::user_row(c, id))? {
        if stamp != u.stamp {
            app.0.purge_user(id).await;
            tracing::info!(
                "Removed the AI data of a user id that now belongs to another account (id {id})"
            );
        }
    }
    let admin = &ctx.who.username;
    let now = unix_now();
    let mut was_bits = asked;
    let mut off = asked;
    let line = app
        .0
        .store
        .write(|tx| -> rusqlite::Result<Option<String>> {
            // AI hint and AI analysis are one switch: the two bits change together
            off = crate::features::paired(data::features_off(tx, id)?, asked);
            let was = data::set_features_off(tx, id, &u.stamp, &u.username, off, now)?;
            was_bits = was;
            let Some(change) = crate::features::describe_change(was, off) else {
                return Ok(None);
            };
            let line = format!(
                "admin {admin:?} changed the AI features of {:?}: {change}",
                u.username
            );
            stats::outbox_add(tx, now, &line)?;
            Ok(Some(line))
        })?;
    if let Some(line) = line {
        tracing::info!(target: "audit", "{line}");
    }
    let flips = |want_on: bool| -> Vec<&'static str> {
        crate::features::ALL
            .iter()
            .filter(|f| {
                crate::features::allowed(was_bits, **f) != crate::features::allowed(off, **f)
                    && crate::features::allowed(off, **f) == want_on
            })
            .map(|f| f.key())
            .collect()
    };
    app.0.activity(
        Event::new(Some(&ctx.who), "ai.users.features", &ctx.client)
            .object(u.username.clone())
            .detail(json!({"on": flips(true), "off": flips(false)})),
    );
    app.0.deliver_outbox().await;
    let (counts, suspended, off, free) = counts_and_suspended(app, &u)?;
    ok(json!({ "user": admin_row(&u, &counts, suspended, off, free, None) }))
}

/// `POST /api/ai/admin/free`, body `{"userId": N, "free": bool}` (A-OWNER-21): an administrator switches the AI Chat free
/// mode of any administrator account (its own included); refused for an ordinary account (409 `ai.freeAdminOnly`, free
/// mode stays an administrator state). Written with its audit line only when the value changed.
pub async fn set_free(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["userId", "free"])?;
    let id = body_id(&ctx.body, "userId")?.ok_or_else(ApiError::bad_request)?;
    let Some(Value::Bool(free)) = ctx.body.get("free") else {
        return Err(ApiError::bad_request());
    };
    let free = *free;
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
    if u.role != "admin" {
        return Err(ApiError::free_admin_only());
    }
    if let Some((stamp, _)) = app.0.store.read(|c| data::user_row(c, id))? {
        if stamp != u.stamp {
            app.0.purge_user(id).await;
            tracing::info!(
                "Removed the AI data of a user id that now belongs to another account (id {id})"
            );
        }
    }
    let admin = &ctx.who.username;
    let now = unix_now();
    let line = app
        .0
        .store
        .write(|tx| -> rusqlite::Result<Option<String>> {
            let was = data::set_free_mode_of(tx, id, &u.stamp, &u.username, free, now)?;
            if was == free {
                return Ok(None);
            }
            let state = if free { "on" } else { "off" };
            let line = format!(
                "admin {admin:?} switched the AI Chat free mode of {:?} {state}",
                u.username
            );
            stats::outbox_add(tx, now, &line)?;
            Ok(Some(line))
        })?;
    if let Some(line) = line {
        tracing::info!(target: "audit", "{line}");
    }
    app.0.activity(
        Event::new(Some(&ctx.who), "ai.users.features", &ctx.client)
            .object(u.username.clone())
            .detail(json!({"free": free})),
    );
    app.0.deliver_outbox().await;
    let (counts, suspended, off, free) = counts_and_suspended(app, &u)?;
    ok(json!({ "user": admin_row(&u, &counts, suspended, off, free, None) }))
}

/// `GET /api/ai/admin/chat?user=&id=[&at=]`: the user's current branch, or (`at`) the read-only branch that runs through
/// message `at` down to the newest answer. Never writes the leaf. The reading is an activity event that must be delivered
/// (and the old audit line is kept), or the view is refused.
pub async fn chat(ctx: Ctx) -> Answer {
    let app = &ctx.app;
    let user = query_id(ctx.query.as_deref(), "user")?;
    let id = query_id(ctx.query.as_deref(), "id")?;
    let at = match query_text(ctx.query.as_deref(), "at")? {
        None => None,
        Some(t) => Some(query_id(Some(&format!("at={t}")), "at")?),
    };
    type Found = (
        data::ChatRow,
        Vec<data::MessageRow>,
        std::collections::HashMap<i64, Vec<i64>>,
        String,
    );
    let found: Option<Found> = app.0.store.read(|c| {
        let Some(chat) = data::chat(c, user, id)? else {
            return Ok(None);
        };
        let leaf = match at {
            Some(m) => {
                if !data::messages(c, id)?.iter().any(|x| x.id == m) {
                    return Ok(None);
                }
                Some(data::line_end(c, id, m)?)
            }
            None => None,
        };
        let path = data::path(c, id, leaf)?;
        let versions = data::versions(c, id, &path)?;
        let name = data::user_row(c, user)?.map(|(_, n)| n).unwrap_or_default();
        Ok(Some((chat, path, versions, name)))
    })?;
    let Some((chat, path, versions, owner)) = found else {
        return Err(ApiError::not_found());
    };
    let line = format!(
        "user {:?} viewed an AI chat of user {owner:?} ({} messages)",
        ctx.who.username,
        path.len()
    );
    let event = Event::new(Some(&ctx.who), "ai.admin.viewChats", &ctx.client)
        .object(owner.clone())
        .detail(json!({"messages": path.len(), "stored": chat.message_count}));
    if !app.0.activity_now(event).await || !app.0.audit_now(&line).await {
        return Err(ApiError::backend_down());
    }
    let mut chat_v = chat.to_json(app.0.chat_running(id));
    if let Some(m) = chat_v.as_object_mut() {
        m.insert("leafId".into(), json!(path.last().map(|x| x.id)));
        m.insert(
            "pick".into(),
            chat.pick.as_ref().map_or(
                Value::Null,
                |(c, model)| json!({"connectionId": c, "model": model}),
            ),
        );
    }
    let messages: Vec<Value> = path
        .iter()
        .map(|m| {
            let mut v = m.to_json();
            if let Some(o) = v.as_object_mut() {
                o.insert("parentId".into(), json!(m.parent_id));
                if let Some(vs) = versions.get(&m.id) {
                    o.insert("versions".into(), json!(vs));
                }
                o.insert("connection".into(), json!(m.connection));
                super::user::put_reasoning(o, m);
            }
            v
        })
        .collect();
    ok(json!({"chat": chat_v, "messages": messages}))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClearWhat {
    Chat(i64),
    Chats,
    Memories,
}

pub async fn clear(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["userId", "what", "chatId"])?;
    let chat_id = body_id(&ctx.body, "chatId")?;
    if ctx.body.get("what").and_then(Value::as_str) == Some("shared") {
        // the shared hints / explanations belong to no user: `{"what": "shared"}` alone
        if ctx.body.len() != 1 {
            return Err(ApiError::bad_request());
        }
        let n = ctx
            .app
            .0
            .store
            .write(|tx| -> rusqlite::Result<i64> { crate::store::shared::shared_clear(tx) })?;
        ctx.app.0.store.schedule_checkpoint();
        tracing::info!(
            "user {:?} cleared the {n} shared AI study answers",
            ctx.who.username
        );
        ctx.app.0.activity(
            Event::new(Some(&ctx.who), "ai.admin.clear", &ctx.client)
                .object("shared")
                .detail(json!({"count": n})),
        );
        return ok(json!({ "deleted": n }));
    }
    let user = body_id(&ctx.body, "userId")?.ok_or_else(ApiError::bad_request)?;
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
    let scope = match what {
        ClearWhat::Chat(_) => "chat",
        ClearWhat::Chats => "chats",
        ClearWhat::Memories => "memories",
    };
    app.0.activity(
        Event::new(Some(&ctx.who), "ai.admin.clear", &ctx.client)
            .object(scope)
            .detail(json!({"count": n})),
    );
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
    // The number of questions of each generation mode (single + multiple) must stay 1 ..= 100
    if let Some(field) = next.gen_count_violation() {
        return Err(ApiError::gen_count(field));
    }
    let now = unix_now();
    let by = ctx.who.username.clone();
    app.0
        .store
        .write(|tx| tables::save_limits(tx, &next, &by, now))?;
    app.0.set_limits(next);
    if cur.reply_max_tokens != next.reply_max_tokens {
        app.0.forget_token_memo();
    }
    if let Some(change) = cur.describe_change(&next) {
        tracing::info!("AI limits changed by admin {by:?}: {change}");
    }
    let keys: Vec<&str> = DEFS
        .iter()
        .filter(|d| cur.get(d.key) != next.get(d.key))
        .map(|d| d.key)
        .collect();
    app.0.activity(
        Event::new(Some(&ctx.who), "ai.settings.limits", &ctx.client).detail(json!({"keys": keys})),
    );
    ok(next.answer())
}

#[cfg(test)]
mod usage_period_tests {
    use super::*;

    fn kind(q: &str) -> Result<String, ()> {
        usage_period(Some(q)).map_err(|_| ()).map(|p| match p {
            UsagePeriod::Days(d) => format!("{d}d"),
            UsagePeriod::Last12Hours => "12h".into(),
            UsagePeriod::Today => "today".into(),
        })
    }

    #[test]
    fn the_period_parameter_adds_two_windows_and_days_still_works() {
        assert_eq!(kind("").as_deref(), Ok("7d"));
        assert_eq!(kind("days=30").as_deref(), Ok("30d"));
        assert_eq!(kind("days=365").as_deref(), Ok("365d"));
        assert_eq!(kind("period=12h").as_deref(), Ok("12h"));
        assert_eq!(kind("period=today").as_deref(), Ok("today"));
        assert!(kind("period=3h").is_err());
        assert!(kind("days=8").is_err());
        assert!(kind("days=7&period=today").is_err());
    }
}
