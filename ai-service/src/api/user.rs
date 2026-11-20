//! User routes: status, chats, the streaming answers, Stop, memories. Reading and deleting need only a valid identity
//! (`Signed`), so users whose AI+ service or function, or AI as a whole, was switched off can still read and delete
//! their own data; writing needs `AiUser`.
//!
//! send / regenerate / edit / retry / suggest carry the reply language (`lang`, the Settings language). Free mode is a state of the
//! administrator (`POST /api/ai/free`, `freeMode` in the status): while it is on, every send / retry in any chat uses the
//! language line alone as its system prompt and no memories.
use super::{
    body_id, id_or_all, ok, only_keys, query_id, require_ai_access, require_ai_user, Answer, Ctx,
};
use crate::chat::{self, Frame, Generation, Kind, StopReason};
use crate::codes::ApiError;
use crate::features::Feature;
use crate::identity::Identity;
use crate::prompt::{self, Lang};
use crate::provider::{self, Role, UpCode};
use crate::quota;
use crate::route::{Pick, Slot};
use crate::store::{data, stats};
use crate::util::{char_len, chat_title, collapse_ws, cut_chars, unix_now};
use axum::body::{Body, Bytes};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

pub async fn status(ctx: Ctx) -> Answer {
    let app = &ctx.app.0;
    let who = &ctx.who;
    let snap = app.snapshot()?;
    let effective = app.effective()?;
    // Available: the effective AI+ service, and the main service accepted a report less than 60 s ago
    let eff = effective && app.link_fresh();
    let suspended = app.suspended(who);
    let has = app.store.read(|c| data::has_data(c, who.id))?;
    let running = app
        .running_of(who.id)
        .map_or(Value::Null, |(c, m)| json!({"chatId": c, "messageId": m}));
    let l = app.limits();
    let admin = who.is_admin();
    // The per-user call counts: only for a user who has the AI+ service while AI is on (paused or not)
    let calls = if eff && who.ai_allowed {
        let st = call_states(app, &[who.id], unix_now())?.remove(&who.id);
        st.map(|mut st| {
            // An exempt administrator is never refused, so the status never reports a block for one
            if admin && l.calls_admins_exempt == 1 {
                st.blocked = None;
            }
            calls_json(&st, Some(l.caps()))
        })
    } else {
        None
    };
    let mut answer = json!({
        "enabled": eff,
        "allowed": eff && who.ai_allowed && !suspended,
        "suspended": suspended,
        "hasData": has,
        "running": running,
        "retentionDays": snap.retention_days,
        "memoryLimit": l.memories_per_user,
        "memoryMaxTokens": l.memory_tokens,
        "maxMessageChars": crate::limits::MESSAGE_CHARS,
        "messageChars": crate::limits::MESSAGE_CHARS,
        "memoryTokens": l.memory_tokens,
        "keyProblem": admin && snap.key_problem.is_some(),
        "configured": admin && app.configured(),
        "freeMode": admin && free_mode_of(app, who)?,
    });
    if let (Some(mut c), Some(m)) = (calls, answer.as_object_mut()) {
        // An administrator is exempt from the per-user caps while `callsAdminsExempt` is 1 (counts are still sent)
        if admin && l.calls_admins_exempt == 1 {
            if let Some(o) = c.as_object_mut() {
                o.insert("exempt".into(), Value::Bool(true));
            }
        }
        m.insert("calls".into(), c);
    }
    if let Some(m) = answer.as_object_mut() {
        // `why`: null when usable; `off` when the AI+ service is not effective; `link` when it is effective but the
        // main service has not accepted a report for 60 s
        let why = if eff {
            Value::Null
        } else if !effective {
            Value::from("off")
        } else {
            Value::from("link")
        };
        m.insert("why".into(), why);
        // `features` and `gen` exactly when the user has the AI+ service, whether AI is enabled or not
        if who.ai_allowed {
            // A feature is usable when its slot resolves and the global switch is on (administrators included); the
            // user's own bit applies to everyone except administrators
            let (global, own) = app.store.read(|c| {
                let own = if admin {
                    0
                } else {
                    data::features_off(c, who.id)?
                };
                Ok((crate::store::conn::global_features_off(c)?, own))
            })?;
            let view = app.routes()?;
            let mut feats = Map::new();
            for f in crate::features::ALL {
                let on = crate::features::allowed(global, f)
                    && crate::features::allowed(own, f)
                    && view.exists(f.slot());
                feats.insert(f.key().to_string(), Value::Bool(on));
            }
            m.insert("features".into(), Value::Object(feats));
            if admin {
                let default = view.resolve(Slot::Chat, None).ok().map(|r| {
                    json!({
                        "connectionId": r.connection_id,
                        "connectionName": r.connection_name,
                        "model": r.model,
                    })
                });
                m.insert("chatDefault".into(), default.unwrap_or(Value::Null));
            }
            m.insert(
                "gen".into(),
                crate::gen::runner::gen_status(&ctx.app, who, unix_now())?,
            );
        }
    }
    super::hide::for_viewer(admin, &mut answer);
    ok(answer)
}

/// What the per-user caps say about one user right now: calls counted in each window (running ones included) and the
/// cap that would refuse one more call, if any.
pub(super) struct CallState {
    pub used: quota::Used,
    pub blocked: Option<quota::Hit>,
}

/// The call state of each of `ids` at `now`, read in one `store.read`: the finished calls from the usage rows
/// (`recent_starts` for one user, one `recent_starts_all` pass grouped in Rust for several, never a `GROUP BY`), plus the
/// running calls of the registry, read inside the same closure (lock order store -> registry).
pub(super) fn call_states(
    app: &crate::app::App,
    ids: &[i64],
    now: i64,
) -> rusqlite::Result<std::collections::HashMap<i64, CallState>> {
    let caps = app.limits().caps();
    app.store.read(|c| {
        let running: Vec<Vec<i64>> = ids
            .iter()
            .map(|&u| app.registry.starts_of(u, None))
            .collect();
        let day = crate::util::day_bounds(now);
        let mut finished: std::collections::HashMap<i64, Vec<i64>> =
            std::collections::HashMap::new();
        if let [one] = ids {
            finished.insert(*one, stats::recent_starts(c, *one, now, day.start)?);
        } else if !ids.is_empty() {
            for (user, start) in stats::recent_starts_all(c, now, day.start)? {
                finished.entry(user).or_default().push(start);
            }
        }
        let mut out = std::collections::HashMap::new();
        for (&user, run) in ids.iter().zip(running) {
            let mut starts = finished.remove(&user).unwrap_or_default();
            starts.extend(run);
            out.insert(
                user,
                CallState {
                    used: quota::used(&starts, now, day),
                    blocked: quota::check(&starts, now, day, caps).err(),
                },
            );
        }
        Ok(out)
    })
}

/// `{"used": {...}, "caps": {...}?, "blocked": null | {"cap", "limit", "retryAt"}}` (`caps` only for the status).
pub(super) fn calls_json(st: &CallState, caps: Option<quota::Caps>) -> Value {
    let mut m = Map::new();
    m.insert(
        "used".into(),
        json!({"minute": st.used.minute, "hour": st.used.hour, "day": st.used.day}),
    );
    if let Some(c) = caps {
        m.insert(
            "caps".into(),
            json!({"minute": c.minute, "hour": c.hour, "day": c.day}),
        );
    }
    m.insert(
        "blocked".into(),
        st.blocked.map_or(
            Value::Null,
            |h| json!({"cap": h.cap.key(), "limit": h.limit, "retryAt": h.retry_at}),
        ),
    );
    Value::Object(m)
}

pub async fn chats(ctx: Ctx) -> Answer {
    let app = &ctx.app.0;
    let rows = app.store.read(|c| data::list_chats(c, ctx.who.id))?;
    let list: Vec<Value> = rows
        .iter()
        .map(|r| r.to_json(app.chat_running(r.id)))
        .collect();
    ok(json!({ "chats": list }))
}

/// The chat as the v1.3 read shows it: today's fields, `leafId`, and for administrators `pick` (`null` = the default;
/// a stored pick whose connection is gone is read as none, `data::CHAT_COLS`).
pub(super) fn chat_json(app: &crate::app::App, admin: bool, chat: &data::ChatRow) -> Value {
    let mut v = chat.to_json(app.chat_running(chat.id));
    if let Some(m) = v.as_object_mut() {
        m.insert("leafId".into(), json!(chat.leaf_id));
        if admin {
            m.insert(
                "pick".into(),
                chat.pick.as_ref().map_or(
                    Value::Null,
                    |(c, model)| json!({"connectionId": c, "model": model}),
                ),
            );
            m.insert("pickEffort".into(), json!(chat.pick_effort));
        }
    }
    v
}

/// The messages of a path: today's fields, `parentId`, `versions` (only when there are 2 or more) and, for administrators,
/// `connection` (the name; `null` before v1.3). Non-administrators get neither `model` nor `connection`
/// (`hide::for_viewer` in `chat_view`).
pub(super) fn messages_json(
    msgs: &[data::MessageRow],
    versions: &HashMap<i64, Vec<i64>>,
    admin: bool,
) -> Vec<Value> {
    msgs.iter()
        .map(|m| {
            let mut v = m.to_json();
            if let Some(o) = v.as_object_mut() {
                o.insert("parentId".into(), json!(m.parent_id));
                if let Some(vs) = versions.get(&m.id) {
                    o.insert("versions".into(), json!(vs));
                }
                if admin {
                    o.insert("connection".into(), json!(m.connection));
                }
                put_reasoning(o, m);
            }
            v
        })
        .collect()
}

/// v1.3: the reasoning of a message (null / null / false for user messages and older answers).
pub(super) fn put_reasoning(o: &mut serde_json::Map<String, Value>, m: &data::MessageRow) {
    o.insert("reasoning".into(), json!(m.reasoning));
    o.insert("reasoningMs".into(), json!(m.reasoning_ms));
    o.insert("reasoningTruncated".into(), json!(m.reasoning_truncated));
}

/// `{"chat": …, "messages": […]}` of a path (the chat read and the branch switch).
fn chat_view(
    app: &crate::app::App,
    admin: bool,
    chat: &data::ChatRow,
    path: &[data::MessageRow],
    versions: &HashMap<i64, Vec<i64>>,
) -> Value {
    let mut v = json!({
        "chat": chat_json(app, admin, chat),
        "messages": messages_json(path, versions, admin),
    });
    super::hide::for_viewer(admin, &mut v);
    v
}

/// A chat of the caller with its viewed path and the versions on it, read in one transaction.
type ChatPath = (data::ChatRow, Vec<data::MessageRow>, HashMap<i64, Vec<i64>>);

pub async fn chat(ctx: Ctx) -> Answer {
    let app = &ctx.app.0;
    let id = query_id(ctx.query.as_deref(), "id")?;
    let found: Option<ChatPath> = app.store.read(|c| {
        let Some(chat) = data::chat(c, ctx.who.id, id)? else {
            return Ok(None);
        };
        let path = data::path(c, id, None)?;
        let versions = data::versions(c, id, &path)?;
        Ok(Some((chat, path, versions)))
    })?;
    let Some((chat, path, versions)) = found else {
        return Err(ApiError::not_found());
    };
    ok(chat_view(app, ctx.who.is_admin(), &chat, &path, &versions))
}

/// The SSE body: frames from the generation, a `: keep-alive` comment when nothing was sent for a while.
pub(super) fn sse_response(rx: UnboundedReceiver<Frame>, keep_alive: Duration) -> Response {
    let stream = futures_util::stream::unfold(rx, move |mut rx| async move {
        tokio::select! {
            f = rx.recv() => f.map(|f| {
                let data = serde_json::to_string(&f.data).unwrap_or_else(|_| "{}".to_string());
                (Ok::<Bytes, Infallible>(Bytes::from(format!("event: {}\ndata: {data}\n\n", f.event))), rx)
            }),
            _ = tokio::time::sleep(keep_alive) => Some((Ok(Bytes::from_static(b": keep-alive\n\n")), rx)),
        }
    });
    let mut r = Response::new(Body::from_stream(stream));
    *r.status_mut() = StatusCode::OK;
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    r
}

/// What the transaction of a send / regenerate / edit prepared.
struct Prepared {
    mode: Mode,
    chat: i64,
    title: String,
    new_chat: bool,
    /// The chat's kind (`chat` for a new one), sent in the `start` frame
    chat_kind: String,
    /// The administrator's free mode was on when the message was sent (no study-assistant prompt, no memories)
    free: bool,
    /// The user message the answer follows (the new one for send / edit, the question for a regenerate)
    user_msg: i64,
    /// Parent of the user message; None = a root
    user_parent: Option<i64>,
    answer: i64,
    /// The answer reset an empty failed answer in place (no new row)
    in_place: bool,
    /// 1 for a first answer; above 1 a regenerate (the gateway cache is bypassed, a note ends the question)
    attempt: u32,
    /// Ids of the answer's versions / of the user message's versions (edit), sent only when there are 2 or more
    versions: Vec<i64>,
    user_versions: Vec<i64>,
    /// A stored model pick no longer resolved and was cleared (an administrator's chat)
    fallback: bool,
    history: Vec<provider::Turn>,
    /// The history sent holds messages of the other mode (a mode block goes into the system prompt)
    switched: bool,
    /// Empty in free mode (not even read)
    memories: Vec<String>,
}

/// Which generating route a request came through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Send,
    Regenerate,
    Edit,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Mode::Send => "send",
            Mode::Regenerate => "regenerate",
            Mode::Edit => "edit",
        }
    }
}

/// What the body of a generating route said.
struct Req {
    mode: Mode,
    chat: Option<i64>,
    /// regenerate / edit: the message; None for send and for the old retry route (which means the leaf)
    target: Option<i64>,
    /// send / edit: the trimmed text
    text: String,
    lang: Lang,
    /// Outer None: no `model` key (use the stored pick); `Some(None)`: `null` (clear it); `Some(Some(p))`: set it
    pick: Option<Option<Pick>>,
    /// Outer None: no `effort` key (use the stored level); `Some(None)`: `null` (clear it); `Some(Some(l))`: set it
    effort: Option<Option<String>>,
}

/// The reply language of a send / retry / suggest body (round 2, 5.1): `"en"`, `"zh"` (alias `"zh-CN"`), `"ja"`;
/// missing → English (the Xexam frontend always sends it); anything else → 400 `ai.badRequest`.
fn lang_of(body: &Map<String, Value>) -> Result<Lang, ApiError> {
    match body.get("lang") {
        None => Ok(Lang::En),
        Some(Value::String(s)) => Lang::parse(s).ok_or_else(ApiError::bad_request),
        Some(_) => Err(ApiError::bad_request()),
    }
}

/// The text of a send / edit: trimmed, 1 to `MESSAGE_CHARS` characters.
fn text_of(body: &Map<String, Value>) -> Result<String, ApiError> {
    let text = match body.get("text") {
        Some(Value::String(s)) => s.trim().to_string(),
        _ => return Err(ApiError::bad_request()),
    };
    if text.is_empty() {
        return Err(ApiError::bad_request());
    }
    if char_len(&text) > crate::limits::MESSAGE_CHARS {
        return Err(ApiError::too_long(crate::limits::MESSAGE_CHARS as i64));
    }
    Ok(text)
}

/// The `model` of a body (routes.md 3.3): a plain user may not name one (403), `null` always means "the default".
fn pick_of(body: &Map<String, Value>, admin: bool) -> Result<Option<Option<Pick>>, ApiError> {
    match body.get("model") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(_) if !admin => Err(ApiError::not_allowed()),
        Some(v) => Ok(Some(Pick::from_json(v)?)),
    }
}

/// The `effort` of a body (v2.1): a strength level of the chat, administrators only; `null` always means "clear".
/// Whether the level is valid is checked against the route's family later.
fn effort_of(body: &Map<String, Value>, admin: bool) -> Result<Option<Option<String>>, ApiError> {
    match body.get("effort") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(_) if !admin => Err(ApiError::not_allowed()),
        Some(Value::String(l)) if !l.is_empty() && l.len() <= 32 => Ok(Some(Some(l.clone()))),
        Some(_) => Err(ApiError::bad_request()),
    }
}

/// The connection of a pick exists and can serve calls, else 400 `ai.badModel`; a connection that is switched off is
/// 409 `ai.connectionOff`. A model outside the pool is allowed.
fn ensure_usable(app: &crate::app::App, p: &Pick) -> Result<(), ApiError> {
    let view = app.routes()?;
    match view.conn(p.connection_id) {
        Some(c) if !c.enabled => Err(ApiError::connection_off()),
        Some(c) if view.conn_usable(c) => Ok(()),
        _ => Err(ApiError::bad_model()),
    }
}

/// Free mode is a state of the administrator, not of a chat: while it is on, every answer in any chat (old or new) uses
/// only the language line and no memories. Never for a plain user, whatever is stored.
fn free_mode_of(app: &crate::app::App, who: &Identity) -> Result<bool, ApiError> {
    Ok(who.is_admin() && app.store.read(|c| data::free_mode(c, who.id))?)
}

/// The memories for the system prompt; none in free mode (not even read).
fn prompt_memories(
    tx: &rusqlite::Connection,
    who: &Identity,
    free: bool,
) -> rusqlite::Result<Vec<String>> {
    if free {
        Ok(Vec::new())
    } else {
        data::memory_texts(tx, who.id)
    }
}

fn prev_turns(
    msgs: &[data::MessageRow],
    new_text: &str,
    budget: usize,
    free: bool,
) -> prompt::History {
    let prev: Vec<prompt::Prev<'_>> = msgs
        .iter()
        .map(|m| prompt::Prev {
            role: if m.role == "assistant" {
                Role::Assistant
            } else {
                Role::User
            },
            content: &m.content,
            status: &m.status,
            free: m.free,
        })
        .collect();
    prompt::history(&prev, new_text, budget, free)
}

/// What a generation needs besides its places: the whole `start` frame, the system prompt, the turns sent, and the
/// study extras.
pub(super) struct Launch {
    pub chat: i64,
    pub answer: i64,
    pub start: Value,
    pub system: String,
    pub history: Vec<provider::Turn>,
    pub guard: Option<crate::study::guard::HintGuard>,
    pub purpose: Option<&'static str>,
    /// 1 for a first ask; above 1 a regenerate: the gateway cache is bypassed and the last user turn sent carries the
    /// regenerate note of that number (never stored)
    pub attempt: u32,
}

/// The line appended to the last user turn of a regenerate (attempt 2 and up). Only the provider request carries it, so
/// the request differs from every earlier one even for a gateway that caches by content and ignores cache headers.
fn regenerate_note(attempt: u32) -> String {
    format!(
        "(Regenerate request #{attempt}: write a new answer; do not repeat an earlier answer word for word.)"
    )
}

/// The turns sent upstream: for a regenerate the note ends the last user turn (the stored message is untouched).
fn with_regenerate_note(history: &[provider::Turn], attempt: u32) -> Vec<provider::Turn> {
    let mut turns = history.to_vec();
    if attempt > 1 {
        if let Some(t) = turns.last_mut().filter(|t| t.role == Role::User) {
            t.content.push_str("\n\n");
            t.content.push_str(&regenerate_note(attempt));
        }
    }
    turns
}

/// Starts the generation of a prepared answer and returns the SSE response.
pub(super) fn launch(
    ctx: &Ctx,
    route: &crate::route::Route,
    target: crate::provider::Target,
    slot: chat::Slot,
    permit: chat::GatePermit,
    l: Launch,
) -> Answer {
    let app = &ctx.app;
    let history = with_regenerate_note(&l.history, l.attempt);
    let spec = provider::ChatSpec {
        model: &route.model,
        system: &l.system,
        turns: &history,
        usage_option: route.usage_option != 0,
        skip_cache: l.attempt > 1,
        prefix: None,
        cache: true,
        schema: None,
        effort: route.effort,
    };
    let plan = match app.chat_plan(route, &target, &spec) {
        Ok(r) => r,
        Err(e) => {
            let now = unix_now();
            app.0.store.write(|tx| {
                data::finish_answer(tx, l.answer, "", "error", Some(e.code.code()), false, now)
            })?;
            return Err(ApiError::upstream(e.code));
        }
    };
    drop(target);
    slot.set_target(l.chat, l.answer);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut start = l.start;
    super::hide::for_viewer(ctx.who.is_admin(), &mut start);
    let _ = tx.send(Frame {
        event: "start",
        data: start,
    });
    let g = Generation {
        app: app.clone(),
        slot,
        permit,
        who: ctx.who.clone(),
        chat: l.chat,
        message: l.answer,
        route: route.clone(),
        plan,
        frames: tx,
        guard: l.guard,
        purpose: l.purpose,
    };
    tokio::spawn(chat::run(g));
    Ok(sse_response(rx, app.0.timeouts.keep_alive))
}

/// Starts the generation of a prepared answer.
fn start_stream(
    ctx: &Ctx,
    route: &crate::route::Route,
    target: crate::provider::Target,
    slot: chat::Slot,
    permit: chat::GatePermit,
    p: Prepared,
    lang: Lang,
) -> Answer {
    let system = if p.free {
        prompt::free_prompt(lang, p.switched)
    } else {
        let today = crate::util::local_today().to_string();
        prompt::system_prompt(lang, &today, &p.memories, p.switched)
    };
    let mut start = json!({
        "chatId": p.chat,
        "chatTitle": p.title,
        "userMessageId": p.user_msg,
        "assistantMessageId": p.answer,
        "model": route.model,
        "newChat": p.new_chat,
        "retry": p.in_place,
        "free": p.free,
        "chatKind": p.chat_kind,
        "mode": p.mode.as_str(),
        "parentId": p.user_msg,
        "userParentId": p.user_parent,
    });
    if let Some(m) = start.as_object_mut() {
        if p.versions.len() > 1 {
            m.insert("versions".into(), json!(p.versions));
        }
        if p.user_versions.len() > 1 {
            m.insert("userVersions".into(), json!(p.user_versions));
        }
        if ctx.who.is_admin() {
            m.insert("connection".into(), json!(route.connection_name));
        }
        if p.fallback {
            m.insert("modelFallback".into(), json!(true));
        }
    }
    let l = Launch {
        chat: p.chat,
        answer: p.answer,
        start,
        system,
        history: p.history,
        guard: None,
        purpose: None,
        attempt: p.attempt,
    };
    launch(ctx, route, target, slot, permit, l)
}

/// Takes the caller's places (`App::take_places`) as the registry slot and the gate permit.
pub(super) fn take_places(
    ctx: &Ctx,
    kind: Kind,
    chat: Option<i64>,
    feature: crate::features::Feature,
    pick: Option<&Pick>,
) -> Result<(chat::Slot, chat::GatePermit), ApiError> {
    let p = ctx
        .app
        .0
        .take_places_pick(&ctx.who, kind, chat, feature, pick)?;
    Ok((p.slot, p.permit))
}

/// `true` when `message` is a message of `chat` (on the viewed path or not).
fn message_in_chat(tx: &rusqlite::Connection, chat: i64, message: i64) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE id = ?1 AND chat_id = ?2)",
        [message, chat],
        |r| r.get(0),
    )
}

/// The viewed path and the position of the target in it. A target outside the chat is 404 `ai.notFound`, one off the
/// viewed path 409 `ai.branchMoved`. No target (the old retry route) means the leaf.
fn locate(
    tx: &rusqlite::Connection,
    chat: i64,
    target: Option<i64>,
) -> Result<(Vec<data::MessageRow>, usize), ApiError> {
    if let Some(id) = target {
        if !message_in_chat(tx, chat, id)? {
            return Err(ApiError::not_found());
        }
        if !data::on_path(tx, chat, id)? {
            return Err(ApiError::branch_moved());
        }
    }
    let path = data::path(tx, chat, None)?;
    let idx = match target {
        Some(id) => path.iter().position(|m| m.id == id),
        None => path.len().checked_sub(1),
    };
    match idx {
        Some(i) => Ok((path, i)),
        None if target.is_some() => Err(ApiError::branch_moved()),
        None => Err(ApiError::not_retryable()),
    }
}

/// The state a send / regenerate / edit hands to the stream, besides the places.
struct Core {
    user_msg: i64,
    user_parent: Option<i64>,
    answer: i64,
    in_place: bool,
    attempt: u32,
    history: Vec<provider::Turn>,
    switched: bool,
}

/// Everything one write transaction of send / regenerate / edit does (rules of routes.md 3.2).
fn prepare(
    tx: &rusqlite::Connection,
    who: &Identity,
    req: &Req,
    route: &crate::route::Route,
    limits: &crate::limits::Limits,
    now: i64,
) -> Result<Prepared, ApiError> {
    let budget = limits.history_budget();
    data::ensure_user(tx, who, now)?;
    let free = who.is_admin() && data::free_mode(tx, who.id)?;
    let model = route.model.as_str();
    let connection = route.connection_name.as_str();
    let (chat, title, new_chat, chat_kind, stored) = match req.chat {
        Some(id) => {
            let c = data::chat(tx, who.id, id)?.ok_or_else(ApiError::not_found)?;
            (id, c.title, false, c.kind, c.message_count)
        }
        None if req.mode == Mode::Send => {
            if data::count_chats(tx, who.id)? >= limits.chats_per_user {
                return Err(ApiError::chat_limit(limits.chats_per_user));
            }
            let title = chat_title(&req.text);
            let id = data::create_chat(tx, who.id, &title, now)?;
            (id, title, true, "chat".to_string(), 0)
        }
        None => return Err(ApiError::bad_request()),
    };
    // The administrator's pick of this chat: a body value is stored (null clears it); a stored pick that did not resolve
    // is cleared and the chat default answers
    let mut fallback = false;
    if who.is_admin() {
        if route.pick_dropped {
            data::set_chat_pick(tx, chat, None)?;
            fallback = true;
        } else if let Some(p) = &req.pick {
            let value = p.as_ref().map(|p| (p.connection_id, p.model.as_str()));
            data::set_chat_pick(tx, chat, value)?;
        }
        // The strength level of the chat is stored on its own: clearing a pick keeps it unless `effort: null` is sent
        if let Some(e) = &req.effort {
            data::set_chat_pick_effort(tx, chat, e.as_deref())?;
        }
    }
    let answer_row = |parent: i64| data::NewMessage {
        parent: Some(parent),
        connection: Some(connection),
        chat,
        role: "assistant",
        content: "",
        status: "streaming",
        model: Some(model),
        free,
        now,
    };
    let user_row = |parent: Option<i64>| data::NewMessage {
        parent,
        connection: None,
        chat,
        role: "user",
        content: &req.text,
        status: "done",
        model: None,
        free,
        now,
    };
    let core = match req.mode {
        Mode::Send => {
            if stored + 2 > limits.messages_per_chat {
                return Err(ApiError::message_limit(limits.messages_per_chat));
            }
            let prev = data::path(tx, chat, None)?;
            let user_msg = data::insert_message(tx, &user_row(None))?;
            let answer = data::insert_message(tx, &answer_row(user_msg))?;
            let h = prev_turns(&prev, &req.text, budget, free);
            Core {
                user_msg,
                user_parent: prev.last().map(|m| m.id),
                answer,
                in_place: false,
                attempt: 1,
                history: h.turns,
                switched: h.other_mode,
            }
        }
        Mode::Edit => {
            if stored + 2 > limits.messages_per_chat {
                return Err(ApiError::message_limit(limits.messages_per_chat));
            }
            let (path, idx) = locate(tx, chat, req.target)?;
            let t = &path[idx];
            if t.role != "user" || t.kind != "chat" {
                return Err(ApiError::not_retryable());
            }
            let user_msg = data::insert_message(tx, &user_row(t.parent_id))?;
            if t.parent_id.is_none() {
                // `parent: None` means "after the leaf" for the store; an edited first message is a new root
                tx.execute(
                    "UPDATE messages SET parent_id = NULL WHERE id = ?1",
                    [user_msg],
                )?;
            }
            let answer = data::insert_message(tx, &answer_row(user_msg))?;
            let h = prev_turns(path.get(..idx).unwrap_or(&[]), &req.text, budget, free);
            Core {
                user_msg,
                user_parent: t.parent_id,
                answer,
                in_place: false,
                attempt: 1,
                history: h.turns,
                switched: h.other_mode,
            }
        }
        Mode::Regenerate => {
            let (path, idx) = locate(tx, chat, req.target)?;
            let t = &path[idx];
            let question = idx.checked_sub(1).and_then(|i| path.get(i));
            let Some(q) = question.filter(|q| {
                t.role == "assistant" && t.kind == "chat" && q.role == "user" && q.kind == "chat"
            }) else {
                return Err(ApiError::not_retryable());
            };
            if t.status == "streaming" {
                return Err(ApiError::busy());
            }
            let in_place = t.status == "error" && t.content.is_empty();
            let answer = if in_place {
                data::reset_answer(tx, t.id, model, free)?;
                tx.execute(
                    "UPDATE messages SET connection = ?2 WHERE id = ?1",
                    rusqlite::params![t.id, connection],
                )?;
                t.id
            } else {
                if stored + 1 > limits.messages_per_chat {
                    return Err(ApiError::message_limit(limits.messages_per_chat));
                }
                data::insert_message(tx, &answer_row(q.id))?
            };
            data::set_message_free(tx, q.id, free)?;
            let n = data::answer_versions(tx, chat, q.id)?;
            let h = prev_turns(path.get(..idx - 1).unwrap_or(&[]), &q.content, budget, free);
            Core {
                user_msg: q.id,
                user_parent: q.parent_id,
                answer,
                in_place,
                // A new version is already counted in `n`; an in-place reset of a failed answer is one more try on top of
                // the existing versions, so it too carries the regenerate note and skips the gateway cache
                attempt: u32::try_from(n)
                    .unwrap_or(u32::MAX)
                    .saturating_add(u32::from(in_place))
                    .max(1),
                history: h.turns,
                switched: h.other_mode,
            }
        }
    };
    data::touch_chat(tx, chat, now)?;
    let (mut versions, mut user_versions) = (Vec::new(), Vec::new());
    if req.mode != Mode::Send {
        let path = data::path(tx, chat, None)?;
        let all = data::versions(tx, chat, &path)?;
        versions = all.get(&core.answer).cloned().unwrap_or_default();
        if req.mode == Mode::Edit {
            user_versions = all.get(&core.user_msg).cloned().unwrap_or_default();
        }
    }
    Ok(Prepared {
        mode: req.mode,
        chat,
        title,
        new_chat,
        chat_kind,
        free,
        user_msg: core.user_msg,
        user_parent: core.user_parent,
        answer: core.answer,
        in_place: core.in_place,
        attempt: core.attempt,
        versions,
        user_versions,
        fallback,
        history: core.history,
        switched: core.switched,
        memories: prompt_memories(tx, who, free)?,
    })
}

/// Send, regenerate and edit: one place, one cap count, one usage row each (routes.md 3.2).
async fn generate(ctx: Ctx, req: Req) -> Answer {
    let app = &ctx.app.0;
    let who = &ctx.who;
    require_ai_access(&ctx.app, who)?;
    // The chat must be the caller's (404 before anything is taken); an administrator's stored pick comes from it
    let (stored, stored_effort) = match req.chat {
        Some(id) => {
            let c = app
                .store
                .read(|c| data::chat(c, who.id, id))?
                .ok_or_else(ApiError::not_found)?;
            (c.pick, c.pick_effort)
        }
        None => (None, None),
    };
    // The model menu exists in free mode only: outside it a named model is refused and a stored pick is not used
    let free = free_mode_of(app, who)?;
    if matches!(&req.pick, Some(Some(_))) && !free {
        return Err(ApiError::bad_model());
    }
    let wanted: Option<Pick> = match &req.pick {
        Some(p) => p.clone(),
        None if free => stored.map(|(connection_id, model)| Pick {
            connection_id,
            model,
        }),
        None => None,
    };
    if let Some(Some(p)) = &req.pick {
        ensure_usable(app, p)?;
    }
    // The strength level follows the same gate as the model menu; a stored level that does not fit the route's family
    // is ignored by the resolver
    if matches!(&req.effort, Some(Some(_))) && !free {
        return Err(ApiError::bad_request());
    }
    let level: Option<String> = match &req.effort {
        Some(e) => e.clone(),
        None if free => stored_effort,
        None => None,
    };
    let route = app.route_with(Slot::Chat, wanted.as_ref(), level.as_deref())?;
    if let Some(Some(l)) = &req.effort {
        if crate::effort::Effort::new(route.family, l).is_none() {
            return Err(ApiError::bad_request());
        }
    }
    let target = app.target(&route)?;
    let (slot, permit) = take_places(&ctx, Kind::Chat, req.chat, Feature::Chat, wanted.as_ref())?;
    let limits = app.limits();
    let now = unix_now();
    let p = app
        .store
        .write(|tx| prepare(tx, who, &req, &route, &limits, now))?;
    start_stream(&ctx, &route, target, slot, permit, p, req.lang)
}

pub async fn send(ctx: Ctx) -> Answer {
    only_keys(
        &ctx.body,
        &["chatId", "text", "lang", "free", "model", "effort"],
    )?;
    let chat = match ctx.body.get("chatId") {
        None | Some(Value::Null) => None,
        Some(_) => body_id(&ctx.body, "chatId")?,
    };
    // A `free` in the body is ignored: free mode is a state of the user (`POST /api/ai/free`), read in `prepare`
    let req = Req {
        mode: Mode::Send,
        chat,
        target: None,
        text: text_of(&ctx.body)?,
        lang: lang_of(&ctx.body)?,
        pick: pick_of(&ctx.body, ctx.who.is_admin())?,
        effort: effort_of(&ctx.body, ctx.who.is_admin())?,
    };
    generate(ctx, req).await
}

pub async fn regenerate(ctx: Ctx) -> Answer {
    only_keys(
        &ctx.body,
        &["chatId", "messageId", "lang", "model", "effort"],
    )?;
    let req = Req {
        mode: Mode::Regenerate,
        chat: Some(body_id(&ctx.body, "chatId")?.ok_or_else(ApiError::bad_request)?),
        target: Some(body_id(&ctx.body, "messageId")?.ok_or_else(ApiError::bad_request)?),
        text: String::new(),
        lang: lang_of(&ctx.body)?,
        pick: pick_of(&ctx.body, ctx.who.is_admin())?,
        effort: effort_of(&ctx.body, ctx.who.is_admin())?,
    };
    generate(ctx, req).await
}

pub async fn edit(ctx: Ctx) -> Answer {
    only_keys(
        &ctx.body,
        &["chatId", "messageId", "text", "lang", "model", "effort"],
    )?;
    let req = Req {
        mode: Mode::Edit,
        chat: Some(body_id(&ctx.body, "chatId")?.ok_or_else(ApiError::bad_request)?),
        target: Some(body_id(&ctx.body, "messageId")?.ok_or_else(ApiError::bad_request)?),
        text: text_of(&ctx.body)?,
        lang: lang_of(&ctx.body)?,
        pick: pick_of(&ctx.body, ctx.who.is_admin())?,
        effort: effort_of(&ctx.body, ctx.who.is_admin())?,
    };
    generate(ctx, req).await
}

/// The old retry route, kept one release for open tabs: a regenerate of the leaf.
pub async fn retry(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["chatId", "lang"])?;
    let req = Req {
        mode: Mode::Regenerate,
        chat: Some(body_id(&ctx.body, "chatId")?.ok_or_else(ApiError::bad_request)?),
        target: None,
        text: String::new(),
        lang: lang_of(&ctx.body)?,
        pick: None,
        effort: None,
    };
    generate(ctx, req).await
}

/// Switches to another version: the viewed end becomes the end of that version's line. No model call, no cap, no usage,
/// `updated_at` untouched; allowed in read-only mode; refused while an answer is being written in the chat.
pub async fn branch(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["chatId", "messageId"])?;
    let chat_id = body_id(&ctx.body, "chatId")?.ok_or_else(ApiError::bad_request)?;
    let msg = body_id(&ctx.body, "messageId")?.ok_or_else(ApiError::bad_request)?;
    let app = &ctx.app.0;
    let user = ctx.who.id;
    let (chat, path, versions) = app.store.write(|tx| -> Result<ChatPath, ApiError> {
        data::chat(tx, user, chat_id)?.ok_or_else(ApiError::not_found)?;
        if !message_in_chat(tx, chat_id, msg)? {
            return Err(ApiError::not_found());
        }
        if app.chat_running(chat_id) {
            return Err(ApiError::busy());
        }
        let end = data::line_end(tx, chat_id, msg)?;
        data::set_leaf(tx, chat_id, end)?;
        let chat = data::chat(tx, user, chat_id)?.ok_or_else(ApiError::not_found)?;
        let path = data::path(tx, chat_id, None)?;
        let versions = data::versions(tx, chat_id, &path)?;
        Ok((chat, path, versions))
    })?;
    ok(chat_view(app, ctx.who.is_admin(), &chat, &path, &versions))
}

/// `{chatId, model: {connectionId, model} | null, effort?: string | null}`: an administrator's model and strength pick of
/// one chat. `effort` absent keeps the stored level, `null` clears it, a level is checked against the family of the
/// connection the pick resolves to (the pick's, else the chat slot's) and stored in `chats.pick_effort`. No cap, no usage.
pub async fn chat_model(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["chatId", "model", "effort"])?;
    if !ctx.who.is_admin() {
        return Err(ApiError::not_allowed());
    }
    let chat_id = body_id(&ctx.body, "chatId")?.ok_or_else(ApiError::bad_request)?;
    let pick = Pick::from_json(ctx.body.get("model").ok_or_else(ApiError::bad_request)?)?;
    let effort = effort_of(&ctx.body, true)?;
    let app = &ctx.app.0;
    let free = free_mode_of(app, &ctx.who)?;
    if let Some(p) = &pick {
        if !free {
            return Err(ApiError::bad_model());
        }
        ensure_usable(app, p)?;
    }
    if let Some(Some(level)) = &effort {
        if !free {
            return Err(ApiError::bad_request());
        }
        let route = app.route(Slot::Chat, pick.as_ref())?;
        if crate::effort::Effort::new(route.family, level).is_none() {
            return Err(ApiError::bad_request());
        }
    }
    let user = ctx.who.id;
    let chat = app.store.write(|tx| -> Result<data::ChatRow, ApiError> {
        data::chat(tx, user, chat_id)?.ok_or_else(ApiError::not_found)?;
        let value = pick.as_ref().map(|p| (p.connection_id, p.model.as_str()));
        data::set_chat_pick(tx, chat_id, value)?;
        if let Some(e) = &effort {
            data::set_chat_pick_effort(tx, chat_id, e.as_deref())?;
        }
        data::chat(tx, user, chat_id)?.ok_or_else(ApiError::not_found)
    })?;
    ok(json!({"chat": chat_json(app, true, &chat)}))
}

pub async fn stop(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["chatId"])?;
    let chat = body_id(&ctx.body, "chatId")?;
    let user = ctx.who.id;
    let waits = ctx
        .app
        .0
        .registry
        .cancel_where(StopReason::Stop, |u, k, c| {
            u == user && k == Kind::Chat && (chat.is_none() || c == chat)
        });
    let stopped = !waits.is_empty();
    for mut w in waits {
        let _ = tokio::time::timeout(Duration::from_secs(5), w.wait_for(|d| *d)).await;
    }
    ok(json!({"ok": true, "stopped": stopped}))
}

/// `{"free": bool}`: the administrator's free mode (administrators only, checked by the route table). It applies to every
/// message from now on, in any chat. Answers `{"free": bool}`.
pub async fn set_free_mode(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["free"])?;
    let Some(Value::Bool(free)) = ctx.body.get("free") else {
        return Err(ApiError::bad_request());
    };
    let free = *free;
    let now = unix_now();
    ctx.app
        .0
        .store
        .write(|tx| data::set_free_mode(tx, &ctx.who, free, now))?;
    ok(json!({ "free": free }))
}

pub async fn delete_chats(ctx: Ctx) -> Answer {
    let target = id_or_all(&ctx.body)?;
    let app = &ctx.app.0;
    let user = ctx.who.id;
    if let Some(id) = target {
        if app.store.read(|c| data::chat(c, user, id))?.is_none() {
            return Err(ApiError::not_found());
        }
    }
    app.stop_chats(user, target).await;
    let n = app.store.write(|tx| match target {
        Some(id) => data::delete_chat(tx, user, id),
        None => data::delete_all_chats(tx, user),
    })?;
    if target.is_some() && n == 0 {
        return Err(ApiError::not_found());
    }
    if n > 0 {
        app.store.schedule_checkpoint();
    }
    ok(json!({ "deleted": n }))
}

pub async fn memories(ctx: Ctx) -> Answer {
    let app = &ctx.app.0;
    let rows = app.store.read(|c| data::list_memories(c, ctx.who.id))?;
    let l = app.limits();
    let list: Vec<Value> = rows.iter().map(data::MemoryRow::to_json).collect();
    ok(json!({"memories": list, "limit": l.memories_per_user, "maxTokens": l.memory_tokens}))
}

/// Memory text: whitespace collapsed (a note is one line in the prompt), 1 to `memoryTokens` estimated tokens.
fn memory_text(ctx: &Ctx) -> Result<String, ApiError> {
    let raw = match ctx.body.get("content") {
        Some(Value::String(s)) => s,
        _ => return Err(ApiError::bad_request()),
    };
    let text = collapse_ws(raw);
    let max = ctx.app.0.limits().memory_tokens;
    if text.is_empty() {
        return Err(ApiError::bad_request());
    }
    if crate::limits::estimate_tokens(&text) > max {
        return Err(ApiError::memory_too_long(max));
    }
    Ok(text)
}

pub async fn memory_add(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["content", "source"])?;
    require_ai_user(&ctx.app, &ctx.who, Slot::Chat)?;
    let source = match ctx.body.get("source") {
        None => "manual",
        Some(Value::String(s)) if s == "manual" => "manual",
        Some(Value::String(s)) if s == "chat" => "chat",
        Some(_) => return Err(ApiError::bad_request()),
    };
    let text = memory_text(&ctx)?;
    let app = &ctx.app.0;
    let max = app.limits().memories_per_user;
    let now = unix_now();
    let who = &ctx.who;
    let row = app.store.write(|tx| -> Result<data::MemoryRow, ApiError> {
        data::ensure_user(tx, who, now)?;
        if data::count_memories(tx, who.id)? >= max {
            return Err(ApiError::memory_limit(max));
        }
        let id = data::add_memory(tx, who.id, &text, source, now)?;
        data::memory(tx, who.id, id)?.ok_or_else(ApiError::internal)
    })?;
    ok(json!({ "memory": row.to_json() }))
}

pub async fn memory_update(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["id", "content"])?;
    require_ai_user(&ctx.app, &ctx.who, Slot::Chat)?;
    let id = body_id(&ctx.body, "id")?.ok_or_else(ApiError::bad_request)?;
    let text = memory_text(&ctx)?;
    let app = &ctx.app.0;
    let now = unix_now();
    let who = &ctx.who;
    let row = app.store.write(|tx| -> Result<data::MemoryRow, ApiError> {
        if !data::update_memory(tx, who.id, id, &text, now)? {
            return Err(ApiError::not_found());
        }
        data::ensure_user(tx, who, now)?;
        data::memory(tx, who.id, id)?.ok_or_else(ApiError::not_found)
    })?;
    ok(json!({ "memory": row.to_json() }))
}

pub async fn memory_delete(ctx: Ctx) -> Answer {
    let target = id_or_all(&ctx.body)?;
    let app = &ctx.app.0;
    let user = ctx.who.id;
    let n = app.store.write(|tx| match target {
        Some(id) => data::delete_memory(tx, user, id),
        None => data::delete_all_memories(tx, user),
    })?;
    if target.is_some() && n == 0 {
        return Err(ApiError::not_found());
    }
    if n > 0 {
        app.store.schedule_checkpoint();
    }
    ok(json!({ "deleted": n }))
}

pub async fn memory_suggest(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["messageId", "lang"])?;
    let route = require_ai_user(&ctx.app, &ctx.who, Slot::Chat)?;
    let msg = body_id(&ctx.body, "messageId")?.ok_or_else(ApiError::bad_request)?;
    let lang = lang_of(&ctx.body)?;
    let app = &ctx.app.0;
    let who = &ctx.who;
    let Some((_, text)) = app.store.read(|c| data::user_message(c, who.id, msg))? else {
        return Err(ApiError::not_found());
    };
    let target = app.target(&route)?;
    let (slot, permit) = take_places(&ctx, Kind::Suggest, None, Feature::Chat, None)?;
    let turns = [provider::Turn {
        role: Role::User,
        content: cut_chars(&text, prompt::SUGGEST_INPUT_CHARS),
    }];
    let system = prompt::suggest_prompt(lang);
    let spec = provider::ChatSpec {
        model: &route.model,
        system: &system,
        turns: &turns,
        usage_option: route.usage_option != 0,
        skip_cache: false,
        prefix: None,
        cache: true,
        schema: None,
        effort: route.effort,
    };
    let plan = ctx
        .app
        .chat_plan(&route, &target, &spec)
        .map_err(|e| ApiError::upstream(e.code))?;
    drop(target);
    // The provider call and its usage row run in one spawned task that owns the slot and the permit: a browser that
    // goes away (the handler future is dropped) neither cancels the call nor loses its row, so the cap still counts it.
    let task = {
        let app = ctx.app.clone();
        let who = who.clone();
        let route = route.clone();
        tokio::spawn(async move {
            let started = std::time::Instant::now();
            let t = app.0.timeouts;
            let mut cancel = slot.cancel.clone();
            let mut shutdown = app.0.shutdown.clone();
            let result = tokio::select! {
                r = provider::collect_chat(&app.0.client, plan, t, t.short_total) => Ok(r),
                _ = cancel.wait_for(|v| v.is_some()) => Err(slot.stopped().unwrap_or(StopReason::Stop)),
                _ = shutdown.wait_for(|v| *v) => Err(StopReason::Shutdown),
            };
            let now = unix_now();
            let (status, code, tin, tout, tcached, tcw) = match &result {
                Ok(Ok(c)) => (
                    "ok",
                    None,
                    c.tokens_in,
                    c.tokens_out,
                    c.tokens_cached,
                    c.tokens_cache_write,
                ),
                Ok(Err(e)) => (
                    "error",
                    Some(e.code.short().to_string()),
                    None,
                    None,
                    None,
                    None,
                ),
                Err(_) => ("stopped", None, None, None, None, None),
            };
            let usage = stats::UsageRow {
                connection_id: Some(route.connection_id),
                connection: Some(route.connection_name.clone()),
                user_id: Some(who.id),
                at: now,
                kind: "memory",
                provider: route.provider.as_str(),
                model: route.model.clone(),
                tokens_in: tin,
                tokens_out: tout,
                tokens_cached: tcached,
                tokens_cache_write: tcw,
                latency_ms: provider::ms(started.elapsed()),
                first_byte_ms: None,
                status,
                error_code: code,
            };
            let written = app.0.store.write(|tx| {
                data::ensure_user(tx, &who, now)?;
                stats::insert_usage(tx, &usage)
            });
            // The row is committed before the places are given back (the caps count both the registry and the rows)
            drop(permit);
            drop(slot);
            match &result {
                Ok(Ok(_)) => app.0.note_outcome(&route, None),
                Ok(Err(e)) => app.0.note_outcome(&route, Some(e.code)),
                Err(_) => {}
            }
            written.map(|()| result)
        })
    };
    let result = match task.await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => return Err(ApiError::internal()),
    };
    match result {
        Ok(Ok(c)) => {
            let max = app.limits().memory_tokens;
            let suggestion = crate::limits::cut_to_tokens(&collapse_ws(c.text.trim()), max);
            if suggestion.is_empty() {
                return Err(ApiError::upstream(UpCode::Protocol));
            }
            ok(json!({ "suggestion": suggestion }))
        }
        Ok(Err(e)) => {
            app.failures.note(&who.username, e.code.code(), e.http);
            Err(ApiError::upstream(e.code))
        }
        Err(StopReason::Disabled) => Err(ApiError::disabled()),
        Err(StopReason::Shutdown | StopReason::Stop) => Err(ApiError::server_busy()),
        Err(StopReason::BackendDown) => Err(ApiError::backend_down()),
        Err(StopReason::ChatDeleted) => Err(ApiError::not_found()),
        Err(StopReason::NotAllowed | StopReason::UserRemoved) => Err(ApiError::not_allowed()),
        Err(StopReason::Suspended) => Err(ApiError::suspended()),
        Err(StopReason::FeatureOff) => Err(ApiError::feature_off(Feature::Chat)),
    }
}
