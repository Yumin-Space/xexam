//! Routes of AI bank generation (spec GJ-2, GJ-5, GJ-6, API-U2 … U5, API-A1): start, job, cancel, dismiss and the
//! administrators' list. The runner (`gen::runner`) does the work after `start` answers 202.
use super::{body_id, ok, only_keys, query_id, query_text, require_ai_user, Answer, Ctx};
use crate::codes::ApiError;
use crate::features::Feature;
use crate::gen::plan::{per_call, PlanError};
use crate::gen::rng::Rng;
use crate::gen::runner::{self, gen_status, GenJob, StopWhy};
use crate::gen::whole;
use crate::gen::wire::{self, Reason, WireError};
use crate::gen::{bank_mode_name, check_name, Composition, NameError, SeedFull, Stratum, Tri};
use crate::store::{data, jobs};
use crate::util::{local_second, local_stamp, unix_now};
use serde_json::{json, Map, Value};

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
        // more than the hard total of 20 000 questions: the same refusal as a source above `genMaxChunks`
        WireError::Refused(Reason::TooMany) => ApiError::gen_too_big(20_000 * 400, i64::MAX, 0),
        WireError::Refused(Reason::User) | WireError::Unreadable => ApiError::backend_down(),
    }
}

/// `POST /api/ai/gen/start` (GJ-2): the order of the steps is the order of the refusals.
pub async fn start(ctx: Ctx) -> Answer {
    let app = ctx.app.clone();
    let who = ctx.who.clone();
    // 1. AI usable for this user
    let route = require_ai_user(&app, &who, crate::route::Slot::Generate)?;
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
    // 6. the source questions: every one, in all its languages, page by page (v1.3 corpus)
    let (min, what) = match body.source {
        Source::Wrong => (l.gen_min_wrong, "wrong"),
        _ => (l.gen_min_source, "source"),
    };
    let src = fetch_corpus(&app, &who, &body, min, what).await?;
    // 7. the job key, the chunks and the plan
    let key = crate::util::hex(&runner::random_bytes::<16>());
    let want = l.gen_tokens();
    let ceiling = app.token_ceiling(&route, want);
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
    let sheet_of: Vec<usize> = whole::source_items(&src.corpus, &src.strata)
        .iter()
        .map(|i| i.stratum)
        .collect();
    let context = crate::gen::par::route_context(&route);
    let ranges = whole::chunks(&src.corpus, &sheet_of, context, want);
    if i64::try_from(ranges.len()).unwrap_or(i64::MAX) > l.gen_max_chunks {
        return Err(ApiError::gen_too_big(
            whole::total_tokens(&src.corpus),
            i64::try_from(ranges.len()).unwrap_or(i64::MAX),
            l.gen_max_chunks,
        ));
    }
    let (the_plan, groups) = whole::plan_chunks(
        &src.corpus,
        &src.strata,
        &ranges,
        comp,
        per_call(call_chars, ceiling),
        Rng::seed_from_key(&key),
    )
    .map_err(|e| match e {
        PlanError::NoSource | PlanError::BadComposition => {
            ApiError::gen_too_few(src.total, min, what)
        }
    })?;
    let prefix_tokens: i64 = ranges
        .iter()
        .map(|r| whole::total_tokens(&src.corpus[r.clone()]))
        .sum();
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
        "chunks": ranges.len(),
        "prefixTokens": prefix_tokens,
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
            groups,
            corpus: src.corpus,
            ranges,
            previous: src.previous,
            at: local_second(now),
            client: ctx.client.clone(),
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

/// What `start` needs from the backend: the whole corpus and the facts around it.
struct Source3 {
    total: i64,
    banks: Vec<(i64, String)>,
    strata: Vec<Stratum>,
    corpus: Vec<SeedFull>,
    previous: Vec<Tri>,
}

/// Pages through `/internal/ai/gen/corpus` (200 questions a page) until `next` is null. The total must not change
/// between pages (a bank edited meanwhile): the start is refused, the user tries again.
async fn fetch_corpus(
    app: &crate::app::AppState,
    who: &crate::identity::Identity,
    body: &StartBody,
    min: i64,
    what: &'static str,
) -> Result<Source3, ApiError> {
    let mut out = Source3 {
        total: 0,
        banks: Vec::new(),
        strata: Vec::new(),
        corpus: Vec::new(),
        previous: Vec::new(),
    };
    let mut after = 0i64;
    for page_no in 0..=(CORPUS_MAX_PAGES + 1) {
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
        req.insert("after".into(), json!(after));
        req.insert("limit".into(), json!(CORPUS_PAGE));
        let bytes = app
            .0
            .link
            .data_call("/internal/ai/gen/corpus", Value::Object(req))
            .await
            .map_err(|_| ApiError::backend_down())?;
        let page = wire::parse_corpus(&bytes).map_err(|e| wire_to_error(e, min, what))?;
        if page.user_id != who.id {
            return Err(ApiError::not_allowed());
        }
        if page_no == 0 {
            out.total = page.total;
            out.banks = page.banks;
            out.strata = page.strata;
            out.previous = page.previous;
        } else if page.total != out.total {
            // the source changed while it was being read
            return Err(ApiError::gen_not_found());
        }
        out.corpus.extend(page.items);
        match page.next {
            None => {
                if i64::try_from(out.corpus.len()).ok() != Some(out.total) {
                    return Err(ApiError::backend_down());
                }
                return Ok(out);
            }
            Some(n) if n > after => after = n,
            Some(_) => return Err(ApiError::backend_down()),
        }
    }
    Err(ApiError::backend_down())
}

/// Questions per page of the corpus route, and the most pages one job reads (20 000 questions / 200, with room).
const CORPUS_PAGE: i64 = 200;
const CORPUS_MAX_PAGES: usize = 120;
/// Items per answer of the job route.
const ITEMS_PER_ANSWER: i64 = 20;

/// The language of a job route's item texts.
fn parse_lang(v: Option<&str>) -> Result<&'static str, ApiError> {
    match v {
        None | Some("en") => Ok("en"),
        Some("zh") => Ok("zh"),
        Some("ja") => Ok("ja"),
        Some(_) => Err(ApiError::bad_request()),
    }
}

fn text_in(v: &Value, lang: &str) -> String {
    let get = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("");
    let t = get(lang);
    if t.is_empty() { get("en") } else { t }.to_string()
}

/// One stored item as the job route shows it: texts in `lang` (English when the language is missing).
pub fn item_view(no: i64, data: &str, lang: &str) -> Option<Value> {
    let v: Value = serde_json::from_str(data).ok()?;
    let options: Vec<Value> = v
        .get("options")?
        .as_array()?
        .iter()
        .map(|o| {
            json!({"text": text_in(o, lang), "correct": o.get("correct").and_then(Value::as_bool).unwrap_or(false)})
        })
        .collect();
    Some(json!({
        "no": no,
        "kind": v.get("kind").and_then(Value::as_str).unwrap_or("single"),
        "stem": text_in(v.get("stem")?, lang),
        "options": options,
        "explanation": v.get("explanation").map_or(String::new(), |e| text_in(e, lang)),
    }))
}

/// `GET /api/ai/gen/job?id=` (own job). With `after=<no>` (and `lang`) the answer also carries the questions written so
/// far, at most 20 after `no`, the highest number sent as `last`, and the numbers that were dropped as `gone`.
pub async fn job(ctx: Ctx) -> Answer {
    let id = query_id(ctx.query.as_deref(), "id")?;
    let after = match query_text(ctx.query.as_deref(), "after")? {
        None => None,
        Some(t) => {
            if t.is_empty() || t.len() > 18 || !t.bytes().all(|b| b.is_ascii_digit()) {
                return Err(ApiError::bad_request());
            }
            Some(t.parse::<i64>().map_err(|_| ApiError::bad_request())?)
        }
    };
    let lang = parse_lang(query_text(ctx.query.as_deref(), "lang")?)?;
    let row = ctx
        .app
        .0
        .store
        .read(|c| jobs::get_for_user(c, ctx.who.id, id))?
        .ok_or_else(ApiError::gen_not_found)?;
    let Some(after) = after else {
        return ok(json!({"job": row.to_json()}));
    };
    let (items, gone) = ctx.app.0.store.read(|c| {
        let items = crate::store::items::items_after(c, id, after, ITEMS_PER_ANSWER)?;
        let mut st =
            c.prepare("SELECT no FROM gen_items WHERE task_id = ?1 AND data = '' ORDER BY no")?;
        let gone: Vec<i64> = st
            .query_map([id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        Ok((items, gone))
    })?;
    let last = items.last().map_or(after, |i| i.no);
    let shown: Vec<Value> = items
        .iter()
        .filter_map(|i| item_view(i.no, &i.data, lang))
        .collect();
    let mut job = row.to_json();
    if let Some(m) = job.as_object_mut() {
        m.insert(
            "chunks".into(),
            row.payload.get("chunks").cloned().unwrap_or(json!(1)),
        );
        m.insert(
            "prefixTokens".into(),
            row.payload.get("prefixTokens").cloned().unwrap_or(json!(0)),
        );
    }
    ok(json!({"job": job, "items": shown, "last": last, "gone": gone}))
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
