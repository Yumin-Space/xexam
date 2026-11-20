//! Administrator routes of the model pool: the merged model list (cached 10 minutes per connection and revision), picking
//! a model for a slot (tested first unless it passed recently), testing and removing a pool model.
use super::admin::settings_value;
use super::admin_conn::{cand_of, list_call, model_ok, record_model, try_busy, FRESH_SECS};
use super::{body_id, ok, only_keys, query_text, Answer, Ctx};
use crate::activity::Event;
use crate::app::{AppState, ModelsCache};
use crate::codes::ApiError;
use crate::effort::Effort;
use crate::identity::Identity;
use crate::provider::errors::UpError;
use crate::provider::ModelList;
use crate::route::Slot;
use crate::store::conn::{self, ConnRow, PoolRow};
use crate::util::unix_now;
use serde_json::{json, Map, Value};
use std::time::{Duration, Instant};

/// How long a connection's model list is kept.
const LIST_TTL: Duration = Duration::from_secs(600);
/// Entries the list cache keeps.
const CACHE_CAP: usize = 64;

fn cached(app: &AppState, row: &ConnRow) -> Option<ModelList> {
    let cache = app.0.models_cache.lock().ok()?;
    cache
        .get(&(row.id, row.rev))
        .filter(|c| c.at.elapsed() < LIST_TTL)
        .map(|c| c.list.clone())
}

fn remember(app: &AppState, row: &ConnRow, list: &ModelList) {
    let Ok(mut cache) = app.0.models_cache.lock() else {
        return;
    };
    // an older revision of the same connection is never read again
    cache.retain(|(id, rev), _| *id != row.id || *rev == row.rev);
    cache.insert(
        (row.id, row.rev),
        ModelsCache {
            at: Instant::now(),
            list: list.clone(),
        },
    );
    while cache.len() > CACHE_CAP {
        let Some(oldest) = cache.iter().min_by_key(|(_, c)| c.at).map(|(k, _)| *k) else {
            break;
        };
        cache.remove(&oldest);
    }
}

/// `GET models?refresh=0|1`: the models of every connection, listed in parallel when not cached.
pub async fn list(ctx: Ctx) -> Answer {
    let refresh = match query_text(ctx.query.as_deref(), "refresh")? {
        None | Some("0") => false,
        Some("1") => true,
        Some(_) => return Err(ApiError::bad_request()),
    };
    let app = &ctx.app;
    let who = &ctx.who;
    let view = app.0.routes()?;
    let jobs = view.conns.iter().map(|row| {
        let view = &view;
        async move {
            // A connection that is switched off is listed without asking the provider (and without the cache)
            if !row.enabled {
                return json!({
                    "id": row.id,
                    "name": row.name,
                    "provider": row.provider,
                    "status": row.status,
                    "enabled": false,
                    "listed": false,
                    "error": null,
                    "models": [],
                });
            }
            // (list, error code)
            let (list, error): (Option<ModelList>, Option<String>) = if row.status == "broken" {
                (None, Some(row.error_code.clone().unwrap_or_else(|| "ai.up.auth".into())))
            } else if !refresh {
                match cached(app, row) {
                    Some(l) => (Some(l), None),
                    None => fetch(app, who, row).await,
                }
            } else {
                fetch(app, who, row).await
            };
            let mut models: Vec<Value> = Vec::new();
            let pool_of = |m: &str| {
                view.pool
                    .iter()
                    .find(|p| p.connection_id == row.id && p.model == m)
            };
            if let Some(l) = &list {
                for id in &l.models {
                    let p = pool_of(id);
                    models.push(json!({
                        "id": id,
                        "context": l.contexts.get(id).copied().or(p.and_then(|p| p.context.and_then(|c| usize::try_from(c).ok()))),
                        "poolId": p.map(|p| p.id),
                        "status": p.map(|p| p.status.clone()),
                    }));
                }
            }
            // pool models the listing does not name (typed by hand, or the service does not list)
            for p in view.pool.iter().filter(|p| p.connection_id == row.id) {
                let named = list.as_ref().is_some_and(|l| l.models.contains(&p.model));
                if !named {
                    models.push(json!({
                        "id": p.model, "context": p.context, "poolId": p.id, "status": p.status,
                    }));
                }
            }
            json!({
                "id": row.id,
                "name": row.name,
                "provider": row.provider,
                "status": row.status,
                "enabled": true,
                "listed": list.as_ref().is_some_and(|l| l.listed),
                "error": error,
                "models": models,
            })
        }
    });
    let connections = futures_util::future::join_all(jobs).await;
    ok(json!({"connections": connections}))
}

/// Lists one connection (cached afterwards). The error is the code of the failure.
async fn fetch(
    app: &AppState,
    who: &Identity,
    row: &ConnRow,
) -> (Option<ModelList>, Option<String>) {
    let cand = match cand_of(app, row) {
        Ok(c) => c,
        Err(e) => return (None, Some(e.code.to_string())),
    };
    let permit = app.0.gate.try_acquire();
    let (r, _) = list_call(app, who, &cand, Some(row.id), &row.name).await;
    drop(permit);
    match r {
        Ok(l) => {
            remember(app, row, &l);
            (Some(l), None)
        }
        Err(e) => (None, Some(e.code.code().to_string())),
    }
}

/// What a pick or an enable found.
pub struct Picked {
    pub ok: bool,
    pub error: Option<UpError>,
    pub latency_ms: Option<i64>,
    pub pool_id: Option<i64>,
}

/// Tests a model (unless its pool row passed less than 10 minutes ago), stores the result and, on a pass, assigns it to
/// `slot`. A failing test changes no slot. `AI+` stays as it is.
pub async fn pick_into(
    app: &AppState,
    who: &Identity,
    slot: Slot,
    connection_id: i64,
    model: &str,
    manual: bool,
) -> Result<Picked, ApiError> {
    if !model_ok(model) {
        return Err(ApiError::bad_model());
    }
    let view = app.0.routes()?;
    let row = view
        .conn(connection_id)
        .cloned()
        .filter(|c| c.status != "broken")
        .ok_or_else(ApiError::bad_model)?;
    if !row.enabled {
        return Err(ApiError::connection_off());
    }
    let pool: Option<PoolRow> = view
        .pool
        .iter()
        .find(|p| p.connection_id == row.id && p.model == model)
        .cloned();
    let now = unix_now();
    let fresh = pool
        .as_ref()
        .is_some_and(|p| p.status == "ok" && p.tested_at.is_some_and(|t| now - t < FRESH_SECS));
    let by = who.username.clone();
    if let (true, Some(p)) = (fresh, &pool) {
        app.0
            .store
            .write(|tx| conn::set_slot(tx, slot.as_str(), Some(p.id), &by))?;
        return Ok(Picked {
            ok: true,
            error: None,
            latency_ms: p.latency_ms,
            pool_id: Some(p.id),
        });
    }
    let _busy = try_busy(app, row.id)?;
    let cand = cand_of(app, &row)?;
    let run = super::admin_conn::run(
        app,
        who,
        Some(row.id),
        &row.name,
        &cand,
        false,
        &[model.to_string()],
    )
    .await;
    let o = run.models.first().ok_or_else(ApiError::internal)?;
    let context = cached(app, &row)
        .and_then(|l| l.contexts.get(model).copied())
        .and_then(|n| i64::try_from(n).ok());
    let pool_id = app.0.store.write(|tx| -> rusqlite::Result<Option<i64>> {
        let id = record_model(tx, row.id, o, context, Some(manual), now)?;
        if o.result.is_ok() {
            if let Some(id) = id {
                conn::set_slot(tx, slot.as_str(), Some(id), &by)?;
            }
        }
        Ok(id)
    })?;
    Ok(Picked {
        ok: o.result.is_ok(),
        error: o.result.as_ref().err().copied(),
        latency_ms: Some(o.latency_ms),
        pool_id,
    })
}

fn picked_answer(app: &AppState, p: &Picked) -> Result<Value, ApiError> {
    Ok(json!({
        "ok": p.ok,
        "code": p.error.map(|e| e.code.code()),
        "httpStatus": p.error.and_then(|e| e.http),
        "latencyMs": p.latency_ms,
        "settings": settings_value(app)?,
    }))
}

/// `POST models/pick`.
pub async fn pick(ctx: Ctx) -> Answer {
    only_keys(
        &ctx.body,
        &["slot", "connectionId", "model", "manual", "clear"],
    )?;
    let slot = match ctx.body.get("slot") {
        Some(Value::String(s)) => Slot::parse(s).ok_or_else(ApiError::bad_request)?,
        _ => return Err(ApiError::bad_request()),
    };
    let app = &ctx.app;
    let who = &ctx.who;
    let by = who.username.clone();
    if let Some(c) = ctx.body.get("clear") {
        if c != &Value::Bool(true) || slot == Slot::Default || ctx.body.len() != 2 {
            return Err(ApiError::bad_request());
        }
        app.0
            .store
            .write(|tx| conn::set_slot(tx, slot.as_str(), None, &by))?;
        app.0.activity(
            Event::new(Some(who), "ai.settings.model", &ctx.client)
                .object(slot.as_str())
                .detail(json!({"cleared": true})),
        );
        app.0.heartbeat_now().await;
        return ok(json!({
            "ok": true, "code": null, "httpStatus": null, "latencyMs": null,
            "settings": settings_value(app)?,
        }));
    }
    let cid = body_id(&ctx.body, "connectionId")?.ok_or_else(ApiError::bad_request)?;
    let model = match ctx.body.get("model") {
        Some(Value::String(s)) => s.clone(),
        _ => return Err(ApiError::bad_request()),
    };
    let manual = match ctx.body.get("manual") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(ApiError::bad_request()),
    };
    let picked = pick_into(app, who, slot, cid, &model, manual).await?;
    let cname = app
        .0
        .store
        .read(|c| conn::connection(c, cid))?
        .map(|c| c.name)
        .unwrap_or_default();
    let mut ev = Event::new(Some(who), "ai.settings.model", &ctx.client)
        .object(slot.as_str())
        .detail(json!({"connection": cname, "model": model}));
    if let Some(e) = picked.error {
        ev = ev.failed(e.code.code());
    }
    app.0.activity(ev);
    app.0.heartbeat_now().await;
    ok(picked_answer(app, &picked)?)
}

/// `POST models/effort` `{"slot", "level": string | null}`: sets the thinking strength of a slot. The family is the one
/// of the connection the slot effectively uses (its own model's connection, else the default slot's); 400 when the slot
/// resolves to none or the level is not one of that family's levels. `null` is back to the model default (stored NULL).
pub async fn effort(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["slot", "level"])?;
    let slot = match ctx.body.get("slot") {
        Some(Value::String(s)) => Slot::parse(s).ok_or_else(ApiError::bad_request)?,
        _ => return Err(ApiError::bad_request()),
    };
    let level = match ctx.body.get("level") {
        Some(Value::Null) => None,
        Some(Value::String(l)) => Some(l.as_str()),
        _ => return Err(ApiError::bad_request()),
    };
    // clearing needs no family (a stale tag of a slot that resolves to nothing must stay clearable)
    let choice = match level {
        None => None,
        Some(l) => {
            let family = ctx
                .app
                .0
                .routes()?
                .slot_effort(slot)
                .map(|e| e.family())
                .ok_or_else(ApiError::bad_request)?;
            Some(Effort::new(family, l).ok_or_else(ApiError::bad_request)?)
        }
    };
    let app = &ctx.app;
    let who = &ctx.who;
    let by = who.username.clone();
    app.0
        .store
        .write(|tx| conn::set_effort(tx, slot.as_str(), choice, &by))?;
    app.0.forget_no_effort();
    app.0.activity(
        Event::new(Some(who), "ai.settings.effort", &ctx.client)
            .object(slot.as_str())
            .detail(json!({
                "family": choice.map_or("default", |e| e.family().as_str()),
                "level": choice.map(Effort::level),
            })),
    );
    ok(json!({"settings": settings_value(app)?}))
}

/// `POST models/test` `{"connectionId", "model"}`: always tests, and stores the result in the pool.
pub async fn test(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["connectionId", "model"])?;
    let cid = body_id(&ctx.body, "connectionId")?.ok_or_else(ApiError::bad_request)?;
    let model = match ctx.body.get("model") {
        Some(Value::String(s)) if model_ok(s) => s.clone(),
        _ => return Err(ApiError::bad_request()),
    };
    let app = &ctx.app;
    let who = &ctx.who;
    let view = app.0.routes()?;
    let row = view.conn(cid).cloned().ok_or_else(ApiError::not_found)?;
    let in_pool = view
        .pool
        .iter()
        .any(|p| p.connection_id == cid && p.model == model);
    let _busy = try_busy(app, cid)?;
    let cand = cand_of(app, &row)?;
    let run = super::admin_conn::run(
        app,
        who,
        Some(cid),
        &row.name,
        &cand,
        false,
        std::slice::from_ref(&model),
    )
    .await;
    let o = run.models.first().ok_or_else(ApiError::internal)?;
    let now = unix_now();
    // a model outside the pool is only tested, never added by a test
    if in_pool {
        app.0
            .store
            .write(|tx| record_model(tx, cid, o, None, None, now).map(|_| ()))?;
    }
    let mut ev = Event::new(Some(who), "ai.settings.modelTest", &ctx.client)
        .object(model.clone())
        .detail(json!({"connection": row.name}));
    if let Err(e) = &o.result {
        ev = ev.failed(e.code.code());
    }
    app.0.activity(ev);
    app.0.heartbeat_now().await;
    let mut m = Map::new();
    m.insert("ok".into(), Value::Bool(o.result.is_ok()));
    m.insert("saved".into(), Value::Bool(false));
    m.insert("connectionId".into(), Value::from(cid));
    m.insert("latencyMs".into(), Value::from(o.latency_ms));
    m.insert("tests".into(), Value::Array(run.tests_json()));
    match &o.result {
        Ok(_) => {
            m.insert("code".into(), Value::Null);
            m.insert("httpStatus".into(), Value::Null);
            m.insert("failedModel".into(), Value::Null);
        }
        Err(e) => {
            m.insert("code".into(), Value::from(e.code.code()));
            m.insert("httpStatus".into(), e.http.map_or(Value::Null, Value::from));
            m.insert("failedModel".into(), Value::from(model));
        }
    }
    m.insert("settings".into(), settings_value(app)?);
    ok(Value::Object(m))
}

/// `POST models/remove` `{"poolId"}`: refused while a slot uses the model.
pub async fn remove(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["poolId"])?;
    let id = body_id(&ctx.body, "poolId")?.ok_or_else(ApiError::bad_request)?;
    let app = &ctx.app;
    let view = app.0.routes()?;
    let row = view
        .pool
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .ok_or_else(ApiError::not_found)?;
    let used_by: Vec<&'static str> = Slot::ALL
        .iter()
        .filter(|s| view.slots.get(s.index()).copied().flatten() == Some(id))
        .map(|s| s.as_str())
        .collect();
    if !used_by.is_empty() {
        return Err(ApiError::model_in_use(json!(used_by)));
    }
    app.0.store.write(|tx| conn::remove_pool(tx, id))?;
    let cname = view
        .conn(row.connection_id)
        .map(|c| c.name.clone())
        .unwrap_or_default();
    app.0.activity(
        Event::new(Some(&ctx.who), "ai.settings.modelRemove", &ctx.client)
            .object(row.model.clone())
            .detail(json!({"connection": cname})),
    );
    ok(json!({"settings": settings_value(app)?}))
}
