//! The study routes (v0.9): AI hints and explanations of one question, the AI summary of a record, and the lookups of
//! what was saved. Only users with the AI+ feature reach the ask routes (`require_ai_user`); the saved routes need only
//! a valid identity, so a user whose AI is off can still read their own saved answers.
//!
//! Every answer of an ask is written into the chat of its practice session, exam or record (`chats.kind`
//! `practice` / `exam` / `summary`). The backend decides which chat: the question and the record come from its data
//! routes (`Link::data_call`), never from the browser. Nothing of a question, an answer, a chat title, a session key or
//! a record time is ever logged.
use super::user::{launch, take_places, Launch};
use super::{ok, only_keys, query_text, require_ai_user, Answer, Ctx};
use crate::chat::Kind;
use crate::codes::ApiError;
use crate::link::LinkError;
use crate::prompt::Lang;
use crate::provider::{Role, Turn};
use crate::store::data::{self, StudyError, StudyInsert, StudySample};
use crate::study::guard::HintGuard;
use crate::study::summary::{RecordInfo, Sample};
use crate::study::wire::{self, RecordIdent, Refusal, WireError};
use crate::study::{
    self, at_digits_to_at, clamp_started_at, hint_prompt, lang_code, practice_explain_prompt,
    result_explain_prompt, study_user_message, summary_prompt, summary_ref, summary_user_message,
    MAX_SUMMARY_ITEMS,
};
use crate::util::{is_lower_hex, local_minute, unix_now};
use serde_json::{json, Map, Value};

// —— shared helpers ——————————————————————————————————————————————————————————————————————————————————————————————————

/// `en` / `zh` / `ja` as the backend's routes spell them.
fn wire_lang(l: Lang) -> &'static str {
    match l {
        Lang::En => "en",
        Lang::Zh => "zh",
        Lang::Ja => "ja",
    }
}

fn valid_question_id(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A required or optional string of the body.
fn body_str<'a>(body: &'a Map<String, Value>, name: &str) -> Result<Option<&'a str>, ApiError> {
    match body.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(ApiError::bad_request()),
    }
}

/// An optional integer of the body (`null` = absent), at least `min`.
fn body_int(body: &Map<String, Value>, name: &str, min: i64) -> Result<Option<i64>, ApiError> {
    match body.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => match n.as_i64() {
            Some(v) if v >= min => Ok(Some(v)),
            _ => Err(ApiError::bad_request()),
        },
        Some(_) => Err(ApiError::bad_request()),
    }
}

/// The reply language of the body: `en`, `zh`, `zh-CN`, `ja`; absent / null = the backend's `uiLang` decides.
fn body_lang(body: &Map<String, Value>) -> Result<Option<Lang>, ApiError> {
    match body_str(body, "lang")? {
        None => Ok(None),
        Some(s) => Lang::parse(s).map(Some).ok_or_else(ApiError::bad_request),
    }
}

/// The text of the `running` info of the busy answer: the user's running answer in `chat`, else the most recent one.
fn running_value(ctx: &Ctx, chat: Option<i64>) -> Value {
    let app = &ctx.app.0;
    let user = ctx.who.id;
    match app.registry.running_in(user, chat) {
        Some((c, m)) => app
            .store
            .read(|conn| data::running_info(conn, user, c, m))
            .ok()
            .flatten()
            .unwrap_or(Value::Null),
        None => Value::Null,
    }
}

/// Calls a data route of the backend; every failure becomes the API error of the routes (6.3 of the contracts).
async fn backend(ctx: &Ctx, path: &str, body: Value) -> Result<Vec<u8>, ApiError> {
    // No token, a refused token (404), an unreachable backend and a bad status are one answer; the link state was
    // already updated by the call (a 404 of a data route changes nothing)
    ctx.app
        .0
        .link
        .data_call(path, body)
        .await
        .map_err(|_: LinkError| ApiError::backend_down())
}

/// A refusal or an unreadable answer of a data route as the API error.
fn wire_error(ctx: &Ctx, path: &str, e: WireError) -> ApiError {
    match e {
        WireError::Refused(Refusal::Session | Refusal::NotAllowed) => ApiError::not_allowed(),
        WireError::Refused(Refusal::NotFound) => ApiError::not_found(),
        WireError::Unreadable => {
            // Only this request fails; the link state is not touched. The line names no content.
            if ctx.app.0.unreadable_log.allow() {
                tracing::warn!("AI link: an answer of {path} could not be read");
            }
            ApiError::backend_down()
        }
    }
}

fn kind_str(k: study::RecordKind) -> &'static str {
    k.as_str()
}

fn chat_busy(ctx: &Ctx, chat: Option<i64>) -> ApiError {
    ApiError::busy_with(running_value(ctx, chat))
}

/// Takes the places of an ask; a busy refusal names the running answer.
fn places(
    ctx: &Ctx,
    chat: Option<i64>,
) -> Result<(crate::chat::Slot, crate::chat::GatePermit), ApiError> {
    take_places(ctx, Kind::Chat, chat).map_err(|e| {
        if e.code == "ai.busy" {
            chat_busy(ctx, chat)
        } else {
            e
        }
    })
}

/// Maps the errors of the ask transaction.
fn study_error(ctx: &Ctx, chat: Option<i64>, e: StudyError) -> ApiError {
    match e {
        StudyError::ChatLimit(n) => ApiError::chat_limit(n),
        StudyError::MessageLimit(n) => ApiError::message_limit(n),
        StudyError::Busy => chat_busy(ctx, chat),
        StudyError::Db(e) => ApiError::from(e),
    }
}

// —— POST /api/ai/study/ask ——————————————————————————————————————————————————————————————————————————————————————————

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AskKind {
    Hint,
    Explain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Practice,
    Exam,
}

impl Mode {
    fn parse(s: &str) -> Option<Mode> {
        match s {
            "practice" => Some(Mode::Practice),
            "exam" => Some(Mode::Exam),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Mode::Practice => "practice",
            Mode::Exam => "exam",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Practice,
    Result,
}

impl Source {
    fn parse(s: &str) -> Option<Source> {
        match s {
            "practice" => Some(Source::Practice),
            "result" => Some(Source::Result),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Source::Practice => "practice",
            Source::Result => "result",
        }
    }
}

/// A validated study ask body.
struct Ask {
    kind: AskKind,
    mode: Mode,
    key: Option<String>,
    started_at: Option<i64>,
    question_id: String,
    no: i64,
    qlang: &'static str,
    source: Source,
    record_id: Option<i64>,
    lang: Option<Lang>,
}

fn parse_ask(body: &Map<String, Value>) -> Result<Ask, ApiError> {
    only_keys(
        body,
        &[
            "kind",
            "mode",
            "key",
            "startedAt",
            "questionId",
            "no",
            "qlang",
            "lang",
            "source",
            "recordId",
        ],
    )?;
    let kind = match body_str(body, "kind")? {
        Some("hint") => AskKind::Hint,
        Some("explain") => AskKind::Explain,
        _ => return Err(ApiError::bad_request()),
    };
    let mode = body_str(body, "mode")?
        .and_then(Mode::parse)
        .ok_or_else(ApiError::bad_request)?;
    let source = body_str(body, "source")?
        .and_then(Source::parse)
        .ok_or_else(ApiError::bad_request)?;
    let key = match body_str(body, "key")? {
        None => None,
        Some(k) if is_lower_hex(k, 32) => Some(k.to_string()),
        Some(_) => return Err(ApiError::bad_request()),
    };
    let started_at = body_int(body, "startedAt", 1)?;
    let question_id = match body_str(body, "questionId")? {
        Some(q) if valid_question_id(q) => q.to_string(),
        _ => return Err(ApiError::bad_request()),
    };
    let no = match body_int(body, "no", 1)? {
        Some(n) if n <= 100_000 => n,
        _ => return Err(ApiError::bad_request()),
    };
    let qlang = match body_str(body, "qlang")? {
        Some("en") => "en",
        Some("zh") => "zh",
        Some("ja") => "ja",
        _ => return Err(ApiError::bad_request()),
    };
    let record_id = body_int(body, "recordId", 1)?;
    let lang = body_lang(body)?;
    // Combinations
    let hint_ok = kind != AskKind::Hint || (mode == Mode::Practice && source == Source::Practice);
    let practice_ok = source != Source::Practice || (mode == Mode::Practice && record_id.is_none());
    let exam_ok = mode != Mode::Exam || (source == Source::Result && record_id.is_some());
    let ident_ok = record_id.is_some() || (key.is_some() && started_at.is_some());
    if !(hint_ok && practice_ok && exam_ok && ident_ok) {
        return Err(ApiError::bad_request());
    }
    Ok(Ask {
        kind,
        mode,
        key,
        started_at,
        question_id,
        no,
        qlang,
        source,
        record_id,
        lang,
    })
}

/// Which chat an ask belongs to (decided from the backend's record when there is one).
struct ChatIdent {
    kind: &'static str,
    chat_ref: String,
    title: String,
    record_id: Option<i64>,
}

fn identity_of(ask: &Ask, record: Option<&RecordIdent>, now: i64) -> Result<ChatIdent, ApiError> {
    match record {
        Some(r) => {
            if let Some(k) = &ask.key {
                if r.key.as_deref() != Some(k.as_str()) {
                    return Err(ApiError::not_found());
                }
            }
            match &r.key {
                Some(k) => {
                    let title = match r.started_at.or(ask.started_at) {
                        Some(t) => local_minute(clamp_started_at(t, now)),
                        None => r.at.clone(),
                    };
                    Ok(ChatIdent {
                        kind: ask.mode.as_str(),
                        chat_ref: k.clone(),
                        title,
                        record_id: Some(r.id),
                    })
                }
                // A record saved before v0.9 has no session key: its own chat, titled with the record's time
                None => Ok(ChatIdent {
                    kind: kind_str(r.kind),
                    chat_ref: summary_ref(r.id, &r.at),
                    title: r.at.clone(),
                    record_id: Some(r.id),
                }),
            }
        }
        None => {
            let (Some(k), Some(t)) = (&ask.key, ask.started_at) else {
                return Err(ApiError::bad_request());
            };
            Ok(ChatIdent {
                kind: ask.mode.as_str(),
                chat_ref: k.clone(),
                title: local_minute(clamp_started_at(t, now)),
                record_id: None,
            })
        }
    }
}

pub async fn ask(ctx: Ctx) -> Answer {
    let a = parse_ask(&ctx.body)?;
    let snap = require_ai_user(&ctx.app, &ctx.who)?;
    let target = ctx.app.0.target(&snap)?;
    let app = &ctx.app.0;
    let who = &ctx.who;

    // The question (and the saved item of the record) from the backend
    let elang = a.lang.map_or("en", wire_lang);
    let mut req = json!({
        "sid": who.sid,
        "id": who.id,
        "questionId": a.question_id,
        "qlang": a.qlang,
        "elang": elang,
    });
    if let Some(rid) = a.record_id {
        req["record"] = json!({"id": rid, "kind": a.mode.as_str()});
    }
    const PATH: &str = "/internal/ai/question";
    let bytes = backend(&ctx, PATH, req).await?;
    let reply = wire::parse_question(&bytes, a.record_id.is_some())
        .map_err(|e| wire_error(&ctx, PATH, e))?;
    let lang = a.lang.unwrap_or(reply.ui_lang);

    // Which chat
    let now = unix_now();
    let ident = identity_of(&a, reply.record.as_ref(), now)?;
    if let Some(r) = &reply.record {
        app.records.put(who.id, &who.stamp, r);
    }

    // The prompt (a hint never receives the solution; only the guard does)
    let q = &reply.question;
    let sol = &reply.solution;
    let no = u32::try_from(a.no).map_err(|_| ApiError::bad_request())?;
    let (system, user, guard, purpose, kind) = match a.kind {
        AskKind::Hint => {
            let (s, u) = hint_prompt(q, no, lang);
            (s, u, Some(HintGuard::new(q, sol)), "hint", "hint")
        }
        AskKind::Explain => {
            let (s, u) = match (&a.source, &reply.item) {
                (Source::Result, Some(item)) => {
                    result_explain_prompt(q, sol, item.answer.as_deref(), no, lang)
                }
                _ => practice_explain_prompt(q, sol, no, lang),
            };
            (s, u, None, "explain", "explain")
        }
    };
    let user_content = study_user_message(q);

    // The places, then the write transaction (which binds the chat to the place before anything is written)
    let chat_opt = app
        .store
        .read(|c| data::chat_by_ref(c, who.id, ident.kind, &ident.chat_ref))?;
    let (slot, permit) = places(&ctx, chat_opt)?;
    app.study_hold_point().await;
    let limits = app.limits();
    let model = snap.settings.model.clone();
    let insert = StudyInsert {
        chat_kind: ident.kind,
        chat_ref: &ident.chat_ref,
        title: &ident.title,
        record_id: ident.record_id,
        kind,
        question_id: Some(&a.question_id),
        question_no: Some(a.no),
        source: Some(a.source.as_str()),
        lang: lang_code(lang),
        sample: None,
        user_content: &user_content,
        model: &model,
        now,
        chats_per_user: limits.chats_per_user,
        messages_per_chat: limits.messages_per_chat,
    };
    let ids = app
        .store
        .write(|tx| data::prepare_study(tx, who, &insert, &mut |c| app.registry.bind(&slot, c)))
        .map_err(|e| study_error(&ctx, chat_opt, e))?;

    let start = json!({
        "chatId": ids.chat,
        "chatTitle": ids.title,
        "userMessageId": ids.user_msg,
        "assistantMessageId": ids.answer,
        "model": model,
        "newChat": ids.new_chat,
        "retry": false,
        "free": false,
        "chatKind": ident.kind,
        "kind": kind,
        "questionId": a.question_id,
        "questionNo": a.no,
        "source": a.source.as_str(),
        "recordId": if a.source == Source::Result { json!(ids.record_id) } else { Value::Null },
        "sample": Value::Null,
        "lang": lang_code(lang),
    });
    let l = Launch {
        chat: ids.chat,
        answer: ids.answer,
        start,
        system,
        history: vec![Turn {
            role: Role::User,
            content: user,
        }],
        guard,
        purpose: Some(purpose),
        attempt: ids.attempt,
    };
    launch(&ctx, &snap, target, slot, permit, l)
}

// —— GET /api/ai/study/saved —————————————————————————————————————————————————————————————————————————————————————————

fn positive(text: &str) -> Option<i64> {
    if (1..=18).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit()) {
        text.parse::<i64>().ok().filter(|n| *n > 0)
    } else {
        None
    }
}

fn empty_saved() -> Value {
    json!({"chatId": Value::Null, "latest": Value::Null, "lastGood": Value::Null})
}

pub async fn saved(ctx: Ctx) -> Answer {
    let q = ctx.query.as_deref();
    let mode = query_text(q, "mode")?
        .and_then(Mode::parse)
        .ok_or_else(ApiError::bad_request)?;
    let source = query_text(q, "source")?
        .and_then(Source::parse)
        .ok_or_else(ApiError::bad_request)?;
    let question = match query_text(q, "question")? {
        Some(s) if valid_question_id(s) => s,
        _ => return Err(ApiError::bad_request()),
    };
    let kind = match query_text(q, "kind")? {
        Some("hint") => AskKind::Hint,
        Some("explain") => AskKind::Explain,
        _ => return Err(ApiError::bad_request()),
    };
    let key = query_text(q, "key")?;
    let record = query_text(q, "record")?;
    // Exactly one of key / record
    let (key, record) = match (key, record) {
        (Some(k), None) if is_lower_hex(k, 32) => (Some(k), None),
        (None, Some(r)) => (None, Some(positive(r).ok_or_else(ApiError::bad_request)?)),
        _ => return Err(ApiError::bad_request()),
    };
    let hint_ok = kind != AskKind::Hint || (mode == Mode::Practice && source == Source::Practice);
    let practice_ok = source != Source::Practice || (mode == Mode::Practice && key.is_some());
    let exam_ok = mode != Mode::Exam || source == Source::Result;
    let record_ok = record.is_none() || source == Source::Result;
    if !(hint_ok && practice_ok && exam_ok && record_ok) {
        return Err(ApiError::bad_request());
    }
    let kind_text = match kind {
        AskKind::Hint => "hint",
        AskKind::Explain => "explain",
    };
    let app = &ctx.app.0;
    let who = &ctx.who;
    let (chat_kind, chat_ref) = match (key, record) {
        (Some(k), _) => (mode.as_str(), k.to_string()),
        (None, Some(rid)) => {
            let rec = match app.records.get(who.id, &who.stamp, rid) {
                Some(r) => r,
                None => {
                    const PATH: &str = "/internal/ai/record-info";
                    let req = json!({"sid": who.sid, "id": who.id, "recordId": rid});
                    let bytes = backend(&ctx, PATH, req).await?;
                    match wire::parse_info(&bytes) {
                        Ok(info) => {
                            app.records.put(who.id, &who.stamp, &info.record);
                            info.record
                        }
                        // Nothing to find (not cached)
                        Err(WireError::Refused(_)) => return ok(empty_saved()),
                        Err(e) => return Err(wire_error(&ctx, PATH, e)),
                    }
                }
            };
            if kind_str(rec.kind) != mode.as_str() {
                return ok(empty_saved());
            }
            match &rec.key {
                Some(k) => (mode.as_str(), k.clone()),
                None => (kind_str(rec.kind), summary_ref(rec.id, &rec.at)),
            }
        }
        (None, None) => return Err(ApiError::bad_request()),
    };
    let found = app.store.read(|c| {
        data::study_saved(
            c,
            who.id,
            chat_kind,
            &chat_ref,
            kind_text,
            question,
            source.as_str(),
        )
    })?;
    ok(found.to_json(false))
}

// —— POST /api/ai/summary/ask ————————————————————————————————————————————————————————————————————————————————————————

pub async fn summary_ask(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["recordId", "lang"])?;
    let record_id = body_int(&ctx.body, "recordId", 1)?.ok_or_else(ApiError::bad_request)?;
    let lang_opt = body_lang(&ctx.body)?;
    let snap = require_ai_user(&ctx.app, &ctx.who)?;
    let target = ctx.app.0.target(&snap)?;
    let app = &ctx.app.0;
    let who = &ctx.who;

    const PATH: &str = "/internal/ai/record";
    let req = json!({
        "sid": who.sid,
        "id": who.id,
        "recordId": record_id,
        "max": MAX_SUMMARY_ITEMS,
        "elang": lang_opt.map_or("en", wire_lang),
    });
    let bytes = backend(&ctx, PATH, req).await?;
    let reply =
        wire::parse_record(&bytes, MAX_SUMMARY_ITEMS).map_err(|e| wire_error(&ctx, PATH, e))?;
    app.records.put(who.id, &who.stamp, &reply.record);
    if !reply.detail {
        return Err(ApiError::no_detail());
    }
    if reply.wrong == 0 {
        return Err(ApiError::no_wrong());
    }
    if reply.usable == 0 || reply.items.is_empty() {
        return Err(ApiError::no_usable());
    }
    let lang = lang_opt.unwrap_or(reply.ui_lang);
    let used = i64::try_from(reply.items.len()).map_err(|_| ApiError::internal())?;
    let sample = StudySample {
        used,
        usable: reply.usable,
        wrong: reply.wrong,
    };
    let info = RecordInfo {
        at: reply.record.at.clone(),
        kind: reply.record.kind,
    };
    let psample = Sample::new(
        reply.items.len(),
        usize::try_from(reply.usable).unwrap_or(usize::MAX),
        usize::try_from(reply.wrong).unwrap_or(usize::MAX),
    );
    let (system, user) = summary_prompt(&info, &reply.items, psample, lang);
    let user_content = summary_user_message(&info, psample);
    let chat_ref = summary_ref(reply.record.id, &reply.record.at);

    let chat_opt = app
        .store
        .read(|c| data::chat_by_ref(c, who.id, "summary", &chat_ref))?;
    let (slot, permit) = places(&ctx, chat_opt)?;
    app.study_hold_point().await;
    let limits = app.limits();
    let model = snap.settings.model.clone();
    let insert = StudyInsert {
        chat_kind: "summary",
        chat_ref: &chat_ref,
        title: &reply.record.at,
        record_id: Some(reply.record.id),
        kind: "summary",
        question_id: None,
        question_no: None,
        source: None,
        lang: lang_code(lang),
        sample: Some(sample),
        user_content: &user_content,
        model: &model,
        now: unix_now(),
        chats_per_user: limits.chats_per_user,
        messages_per_chat: limits.messages_per_chat,
    };
    let ids = app
        .store
        .write(|tx| data::prepare_study(tx, who, &insert, &mut |c| app.registry.bind(&slot, c)))
        .map_err(|e| study_error(&ctx, chat_opt, e))?;

    let start = json!({
        "chatId": ids.chat,
        "chatTitle": ids.title,
        "userMessageId": ids.user_msg,
        "assistantMessageId": ids.answer,
        "model": model,
        "newChat": ids.new_chat,
        "retry": false,
        "free": false,
        "chatKind": "summary",
        "kind": "summary",
        "questionId": Value::Null,
        "questionNo": Value::Null,
        "source": Value::Null,
        "recordId": reply.record.id,
        "sample": sample.to_json(),
        "lang": lang_code(lang),
    });
    let l = Launch {
        chat: ids.chat,
        answer: ids.answer,
        start,
        system,
        history: vec![Turn {
            role: Role::User,
            content: user,
        }],
        guard: None,
        purpose: Some("summary"),
        attempt: ids.attempt,
    };
    launch(&ctx, &snap, target, slot, permit, l)
}

// —— GET /api/ai/summary/saved ———————————————————————————————————————————————————————————————————————————————————————

pub async fn summary_saved(ctx: Ctx) -> Answer {
    let q = ctx.query.as_deref();
    let record = query_text(q, "record")?
        .and_then(positive)
        .ok_or_else(ApiError::bad_request)?;
    let at = query_text(q, "at")?
        .and_then(at_digits_to_at)
        .ok_or_else(ApiError::bad_request)?;
    let chat_ref = summary_ref(record, &at);
    let who = &ctx.who;
    let found = ctx
        .app
        .0
        .store
        .read(|c| data::summary_saved(c, who.id, &chat_ref))?;
    ok(found.to_json(true))
}
