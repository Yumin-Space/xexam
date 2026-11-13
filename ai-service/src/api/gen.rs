//! Routes of AI bank generation (spec GJ-2, GJ-5, GJ-6, API-U2 … U5, API-A1): start, job, cancel, dismiss and the
//! administrators' list. The runner (`gen::runner`) does the work after `start` answers 202.
use super::{body_id, ok, only_keys, query_id, query_text, require_ai_user, Answer, Ctx};
use crate::codes::ApiError;
use crate::features::Feature;
use crate::gen::plan::{per_call, plan, replace_seeds, PlanError, PlanInput};
use crate::gen::rng::Rng;
use crate::gen::runner::{self, gen_status, GenJob, StopWhy};
use crate::gen::wire::{self, Reason, WireError};
use crate::gen::{bank_mode_name, check_name, Composition, NameError, SeedFull};
use crate::store::{data, jobs};
use crate::util::{local_second, local_stamp, unix_now};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

/// The three modes of a start request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Bank,
    Scope,
    Wrong,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Bank => "bank",
            Source::Scope => "scope",
            Source::Wrong => "wrong",
        }
    }
}

/// A parsed start request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartBody {
    pub source: Source,
    pub bank: Option<i64>,
    pub sheets: Vec<(i64, String)>,
    pub name: Option<String>,
}

fn bad() -> ApiError {
    ApiError::bad_request()
}

/// Validates the body of `POST /api/ai/gen/start` (API-U2): exactly the keys of the source, `sheets` 1 … 200 distinct
/// entries, `sheet` 1 … 80 characters, the Home name through `check_name`.
pub fn parse_start(body: &Map<String, Value>) -> Result<StartBody, ApiError> {
    let source = match body.get("source").and_then(Value::as_str) {
        Some("bank") => Source::Bank,
        Some("scope") => Source::Scope,
        Some("wrong") => Source::Wrong,
        _ => return Err(bad()),
    };
    if source == Source::Bank {
        only_keys(body, &["source", "bankId"])?;
        let id = body_id(body, "bankId")?.ok_or_else(bad)?;
        return Ok(StartBody {
            source,
            bank: Some(id),
            sheets: Vec::new(),
            name: None,
        });
    }
    only_keys(body, &["source", "sheets", "name"])?;
    let Some(Value::Array(list)) = body.get("sheets") else {
        return Err(bad());
    };
    if list.is_empty() || list.len() > 200 {
        return Err(bad());
    }
    let mut sheets: Vec<(i64, String)> = Vec::new();
    for e in list {
        let Some(o) = e.as_object() else {
            return Err(bad());
        };
        only_keys(o, &["bank", "sheet"])?;
        let bank = body_id(o, "bank")?.ok_or_else(bad)?;
        let Some(Value::String(sheet)) = o.get("sheet") else {
            return Err(bad());
        };
        let n = sheet.chars().count();
        if !(1..=80).contains(&n) {
            return Err(bad());
        }
        if !sheets.iter().any(|(b, s)| *b == bank && s == sheet) {
            sheets.push((bank, sheet.clone()));
        }
    }
    let Some(Value::String(name)) = body.get("name") else {
        return Err(bad());
    };
    let name = check_name(name).map_err(|e| match e {
        NameError::Empty | NameError::TooLong | NameError::Control => {
            ApiError::gen_bad_name(crate::gen::NAME_MAX_HOME as i64)
        }
    })?;
    Ok(StartBody {
        source,
        bank: None,
        sheets,
        name: Some(name),
    })
}

fn sheets_json(sheets: &[(i64, String)]) -> Value {
    Value::Array(
        sheets
            .iter()
            .map(|(b, s)| json!({"bank": b, "sheet": s}))
            .collect(),
    )
}

fn wire_to_error(e: WireError, min: i64, what: &'static str) -> ApiError {
    match e {
        WireError::Refused(Reason::Session) => ApiError::not_allowed(),
        WireError::Refused(Reason::NotAllowed) => ApiError::gen_forbidden(),
        WireError::Refused(Reason::NotFound) => ApiError::gen_not_found(),
        WireError::Refused(Reason::AiSource) => ApiError::gen_ai_source(),
        WireError::Refused(Reason::TooFew(have)) => ApiError::gen_too_few(have, min, what),
        WireError::Refused(Reason::User) | WireError::Unreadable => ApiError::backend_down(),
    }
}

/// `POST /api/ai/gen/start` (GJ-2): the order of the steps is the order of the refusals.
pub async fn start(ctx: Ctx) -> Answer {
    let app = ctx.app.clone();
    let who = ctx.who.clone();
    // 1. AI usable for this user
    let snap = require_ai_user(&app, &who)?;
    // 2. the feature
    if !app.0.feature_allowed(&who, Feature::Generate) {
        return Err(ApiError::feature_off(Feature::Generate));
    }
    // 3. the body
    let body = parse_start(&ctx.body)?;
    let now = unix_now();
    let l = app.0.limits();
    // 4. busy (an early answer; the unique index is the rule)
    if let Some(active) = app.0.store.read(|c| jobs::active_for_user(c, who.id))? {
        return Err(ApiError::gen_busy(active.to_json()));
    }
    // 5. quota
    let q = app
        .0
        .store
        .read(|c| runner::quota_of(c, &app.0, &who, now))?;
    if q.exhausted() {
        return Err(ApiError::gen_quota(q.used, q.limit, q.day.next, now));
    }
    // 6. the source questions (API-I1)
    let (min, what) = match body.source {
        Source::Wrong => (l.gen_min_wrong, "wrong"),
        _ => (l.gen_min_source, "source"),
    };
    let mut req = Map::new();
    req.insert("sid".into(), json!(who.sid));
    req.insert("source".into(), json!(body.source.as_str()));
    match body.source {
        Source::Bank => {
            req.insert("bank".into(), json!(body.bank));
        }
        _ => {
            req.insert("sheets".into(), sheets_json(&body.sheets));
        }
    }
    req.insert("min".into(), json!(min));
    req.insert("max".into(), json!(2000));
    let bytes = app
        .0
        .link
        .data_call("/internal/ai/gen/source", Value::Object(req))
        .await
        .map_err(|_| ApiError::backend_down())?;
    let src = wire::parse_source(&bytes).map_err(|e| wire_to_error(e, min, what))?;
    if src.user.id != who.id {
        return Err(ApiError::not_allowed());
    }
    // 7. the job key and the plan
    let key = crate::util::hex(&runner::random_bytes::<16>());
    let want = l.gen_tokens();
    let ceiling = app.token_ceiling(&snap, want);
    let call_chars = crate::limits::Limits::usize_of(crate::limits::tokens_to_chars(want));
    let comp = match body.source {
        Source::Wrong => Composition {
            single: usize::try_from(l.gen_wrong_single).unwrap_or(0),
            multi: usize::try_from(l.gen_wrong_multi).unwrap_or(0),
        },
        _ => Composition {
            single: usize::try_from(l.gen_bank_single).unwrap_or(0),
            multi: usize::try_from(l.gen_bank_multi).unwrap_or(0),
        },
    };
    let input = PlanInput {
        items: &src.items,
        strata: &src.strata,
        comp,
        per_call: per_call(call_chars, ceiling),
        seed: Rng::seed_from_key(&key),
    };
    let mut the_plan = plan(&input).map_err(|e| match e {
        PlanError::NoSource | PlanError::BadComposition => {
            ApiError::gen_too_few(src.total, min, what)
        }
    })?;
    // 8. the seeds (API-I2); missing ones are replaced, at most 3 rounds, never dropping a slot
    let mut seeds: HashMap<String, SeedFull> = HashMap::new();
    let mut missing: Vec<String> = Vec::new();
    let mut to_fetch: Vec<String> = distinct_seeds(&the_plan);
    for round in 0..=3 {
        if to_fetch.is_empty() {
            break;
        }
        let bytes = app
            .0
            .link
            .data_call(
                "/internal/ai/gen/seeds",
                json!({"sid": who.sid, "ids": to_fetch}),
            )
            .await
            .map_err(|_| ApiError::backend_down())?;
        let got = wire::parse_seeds(&bytes).map_err(|e| wire_to_error(e, min, what))?;
        let got_ids: HashSet<String> = got.iter().map(|s| s.id.clone()).collect();
        for s in got {
            seeds.insert(s.id.clone(), s);
        }
        let gone: Vec<String> = to_fetch
            .iter()
            .filter(|id| !got_ids.contains(*id))
            .cloned()
            .collect();
        if gone.is_empty() {
            break;
        }
        if round == 3 {
            return Err(ApiError::gen_not_found());
        }
        missing.extend(gone);
        to_fetch = replace_seeds(&mut the_plan, &input, &missing)
            .map_err(|_| ApiError::gen_not_found())?
            .into_iter()
            .filter(|id| !seeds.contains_key(id))
            .collect();
    }
    if the_plan.slots.iter().any(|s| !seeds.contains_key(&s.seed)) {
        return Err(ApiError::gen_not_found());
    }
    // the name and the payload of the job
    let from_ids: Vec<i64> = src.banks.iter().map(|(id, _)| *id).collect();
    let name = match (&body.name, body.source) {
        (Some(n), _) => n.clone(),
        (None, _) => {
            let first = src.banks.first().map_or("", |(_, n)| n.as_str());
            bank_mode_name(first, &who.username, &local_stamp(now))
        }
    };
    let from_json: Vec<Value> = src
        .banks
        .iter()
        .map(|(id, n)| json!({"id": id, "name": n}))
        .collect();
    let goal = the_plan.slots.len() as i64;
    let payload = json!({
        "source": body.source.as_str(),
        "name": name,
        "from": from_json,
        "fromCount": src.banks.len(),
        "have": src.total,
        "comp": {"single": comp.single, "multi": comp.multi},
        "perCall": the_plan.per_call,
        "callTokens": l.gen_call_tokens,
    });
    // 9. one write transaction: the user's row, the quota again, the job
    let ins = app
        .0
        .store
        .write(|tx| -> Result<Result<i64, bool>, rusqlite::Error> {
            data::ensure_user(tx, &who, now)?;
            let q = runner::quota_of(tx, &app.0, &who, now)?;
            if q.exhausted() {
                return Ok(Err(true));
            }
            match jobs::insert(
                tx,
                &jobs::NewJob {
                    user_id: who.id,
                    job_key: &key,
                    goal,
                    payload: &payload,
                    charged: true,
                    phase: None,
                    now,
                },
            ) {
                Ok(id) => Ok(Ok(id)),
                Err(jobs::InsertError::Busy) => Ok(Err(false)),
                Err(jobs::InsertError::Db(e)) => Err(e),
            }
        })?;
    let task = match ins {
        Ok(id) => id,
        Err(true) => {
            let q = app
                .0
                .store
                .read(|c| runner::quota_of(c, &app.0, &who, now))?;
            return Err(ApiError::gen_quota(q.used, q.limit, q.day.next, now));
        }
        Err(false) => {
            let active = app.0.store.read(|c| jobs::active_for_user(c, who.id))?;
            return Err(ApiError::gen_busy(
                active.map_or(Value::Null, |j| j.to_json()),
            ));
        }
    };
    let source_stems: Vec<crate::gen::Tri> = src.items.iter().map(|i| i.stem.clone()).collect();
    runner::spawn_generation(
        &app,
        GenJob {
            task,
            key,
            who: who.clone(),
            source: body.source.as_str().to_string(),
            name,
            from_ids,
            plan: the_plan,
            seeds,
            source_stems,
            previous: src.previous,
            at: local_second(now),
        },
    );
    let row = app
        .0
        .store
        .read(|c| jobs::get(c, task))?
        .ok_or_else(ApiError::internal)?;
    let gen = gen_status(&app, &who, now)?;
    Ok(super::json_response(
        axum::http::StatusCode::ACCEPTED,
        &json!({"job": row.to_json(), "gen": gen}),
    ))
}

fn distinct_seeds(p: &crate::gen::plan::Plan) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for s in &p.slots {
        if !v.contains(&s.seed) {
            v.push(s.seed.clone());
        }
    }
    v
}

/// `GET /api/ai/gen/job?id=` (own job).
pub async fn job(ctx: Ctx) -> Answer {
    let id = query_id(ctx.query.as_deref(), "id")?;
    let row = ctx
        .app
        .0
        .store
        .read(|c| jobs::get_for_user(c, ctx.who.id, id))?
        .ok_or_else(ApiError::gen_not_found)?;
    ok(json!({"job": row.to_json()}))
}

/// What the cancel transaction found.
enum Cancelled {
    NotFound,
    TooLate(jobs::JobRow),
    Done(jobs::JobRow),
    /// "Keep what is finished": the runner saves the partial bank (phase `saving` is set)
    Kept(jobs::JobRow),
}

/// Cancels a job of `user`: the end state is written here (so the answer shows it), the runner is signalled afterwards
/// and finds nothing left to write. Charging: a generation keeps its unit when a model call was sent; an import keeps it
/// when it was confirmed and is writing (state running), and was never charged before its confirm.
pub fn cancel_job(
    app: &crate::app::AppState,
    user: i64,
    id: i64,
    why: StopWhy,
    keep_done: bool,
) -> rusqlite::Result<(Option<jobs::JobRow>, bool)> {
    let now = unix_now();
    let res = app.0.store.write(|tx| -> rusqlite::Result<Cancelled> {
        let Some(row) = jobs::get_for_user(tx, user, id)? else {
            return Ok(Cancelled::NotFound);
        };
        if !row.active() || row.phase.as_deref() == Some("saving") {
            return Ok(Cancelled::TooLate(row));
        }
        // A-OWNER-66.4: an import that is writing and already holds finished questions may save them as a partial bank
        if keep_done
            && row.is_file()
            && row.payload.get("mode").and_then(Value::as_str) == Some("questions")
            && row.state == "running"
            && row.phase.as_deref() == Some("writing")
            && row.made > 0
            && jobs::set_phase(tx, id, "saving", now)? > 0
        {
            return Ok(Cancelled::Kept(jobs::get(tx, id)?.unwrap_or(row)));
        }
        let keep = row.charged
            && if row.is_file() {
                row.state == "running"
            } else {
                row.attempts > 0 || app.0.jobs.has_sent(id)
            };
        let n = jobs::finish(
            tx,
            id,
            &jobs::End {
                state: "cancelled",
                error: None,
                charged: keep,
                result: None,
                seen: true,
            },
            now,
        )?;
        if n == 0 {
            return Ok(Cancelled::TooLate(row));
        }
        Ok(Cancelled::Done(jobs::get(tx, id)?.unwrap_or(row)))
    })?;
    match res {
        Cancelled::NotFound => Ok((None, false)),
        Cancelled::TooLate(r) => Ok((Some(r), false)),
        Cancelled::Done(r) => {
            app.0.jobs.cancel(id, why);
            Ok((Some(r), true))
        }
        Cancelled::Kept(r) => {
            app.0.jobs.cancel(id, StopWhy::Keep);
            Ok((Some(r), true))
        }
    }
}

/// `POST /api/ai/gen/cancel {"id", "keep"?}`: `keep: true` on an import with finished questions saves them as a partial
/// bank (the answer shows the job in its phase `saving`); in any other case it is an ordinary cancel.
pub async fn cancel(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["id", "keep"])?;
    let id = body_id(&ctx.body, "id")?.ok_or_else(ApiError::bad_request)?;
    let keep = match ctx.body.get("keep") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(ApiError::bad_request()),
    };
    let (row, done) = cancel_job(&ctx.app, ctx.who.id, id, StopWhy::Cancel, keep)?;
    match (row, done) {
        (None, _) => Err(ApiError::gen_not_found()),
        (Some(r), false) => Err(ApiError::gen_too_late(r.to_json())),
        (Some(r), true) => {
            tracing::info!(
                "user {:?} cancelled an AI bank job ({} model calls)",
                ctx.who.username,
                r.attempts
            );
            ok(json!({"job": r.to_json()}))
        }
    }
}

/// `POST /api/ai/gen/dismiss {"id"}`: a finished job is marked seen (idempotent).
pub async fn dismiss(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["id"])?;
    let id = body_id(&ctx.body, "id")?.ok_or_else(ApiError::bad_request)?;
    let row = ctx
        .app
        .0
        .store
        .read(|c| jobs::get_for_user(c, ctx.who.id, id))?
        .ok_or_else(ApiError::gen_not_found)?;
    if row.active() {
        return Err(ApiError::gen_too_late(row.to_json()));
    }
    ctx.app.0.store.write(|tx| jobs::set_seen(tx, id))?;
    ok(json!({"ok": true}))
}

/// `GET /api/ai/admin/gen?limit=50` (administrators): every user's jobs, newest first.
pub async fn admin_list(ctx: Ctx) -> Answer {
    let limit = match query_text(ctx.query.as_deref(), "limit")? {
        None => 50,
        Some(t) => match t.parse::<i64>() {
            Ok(n) if (1..=200).contains(&n) && t.bytes().all(|b| b.is_ascii_digit()) => n,
            _ => return Err(ApiError::bad_request()),
        },
    };
    let rows = ctx.app.0.store.read(|c| jobs::list_admin(c, limit))?;
    let list: Vec<Value> = rows
        .iter()
        .map(|r| {
            let mut v = r.job.to_json();
            if let Some(m) = v.as_object_mut() {
                m.insert("userId".into(), json!(r.job.user_id));
                m.insert("username".into(), json!(r.username));
                m.insert("tokensIn".into(), json!(r.job.tokens_in));
                m.insert("tokensOut".into(), json!(r.job.tokens_out));
            }
            v
        })
        .collect();
    ok(json!({ "jobs": list }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn the_three_bodies() {
        let b = parse_start(&obj(json!({"source": "bank", "bankId": 3}))).expect("bank");
        assert_eq!((b.source, b.bank), (Source::Bank, Some(3)));
        let b = parse_start(&obj(json!({
            "source": "scope",
            "sheets": [{"bank": 3, "sheet": "Ovens"}, {"bank": 3, "sheet": "Ovens"}, {"bank": 5, "sheet": "Knives"}],
            "name": "  AI practice  "
        })))
        .expect("scope");
        assert_eq!(b.sheets.len(), 2, "duplicates ignored");
        assert_eq!(b.name.as_deref(), Some("AI practice"));
        assert!(parse_start(&obj(
            json!({"source": "wrong", "sheets": [{"bank": 1, "sheet": "A"}], "name": "n"})
        ))
        .is_ok());
    }

    #[test]
    fn refusals_of_the_body() {
        for bad in [
            json!({}),
            json!({"source": "file"}),
            json!({"source": "bank"}),
            json!({"source": "bank", "bankId": 0}),
            json!({"source": "bank", "bankId": 3, "name": "x"}),
            json!({"source": "scope", "sheets": [], "name": "n"}),
            json!({"source": "scope", "sheets": [{"bank": 1, "sheet": ""}], "name": "n"}),
            json!({"source": "scope", "sheets": [{"bank": 1, "sheet": "A", "x": 1}], "name": "n"}),
            json!({"source": "scope", "sheets": [{"bank": 1, "sheet": "A"}]}),
            json!({"source": "scope", "sheets": [{"bank": -1, "sheet": "A"}], "name": "n"}),
        ] {
            let e = parse_start(&obj(bad.clone())).expect_err(&bad.to_string());
            assert_eq!(e.code, "ai.badRequest", "{bad}");
        }
        let long = "x".repeat(81);
        assert!(parse_start(&obj(
            json!({"source": "scope", "sheets": [{"bank": 1, "sheet": long}], "name": "n"})
        ))
        .is_err());
        let too_many: Vec<Value> = (1..=201)
            .map(|i| json!({"bank": i, "sheet": "A"}))
            .collect();
        assert!(parse_start(&obj(
            json!({"source": "scope", "sheets": too_many, "name": "n"})
        ))
        .is_err());
    }

    #[test]
    fn names_are_checked() {
        for name in ["", "   ", &"y".repeat(61), "bad\u{202E}name", "tab\tname"] {
            let e = parse_start(&obj(
                json!({"source": "scope", "sheets": [{"bank": 1, "sheet": "A"}], "name": name}),
            ))
            .expect_err(name);
            assert_eq!(e.code, "ai.gen.badName");
            assert_eq!(e.to_json()["max"], 60);
        }
        let sixty = "y".repeat(60);
        assert!(parse_start(&obj(
            json!({"source": "scope", "sheets": [{"bank": 1, "sheet": "A"}], "name": sixty})
        ))
        .is_ok());
    }
}
