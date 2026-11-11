//! User routes: status, chats, the streaming answers, Stop, memories. Reading and deleting need only a valid identity
//! (`Signed`), so users whose AI+ service or function, or AI as a whole, was switched off can still read and delete
//! their own data; writing needs `AiUser`.
//!
//! send / retry / suggest carry the reply language (`lang`, the Settings language). Free mode is a state of the
//! administrator (`POST /api/ai/free`, `freeMode` in the status): while it is on, every send / retry in any chat uses the
//! language line alone as its system prompt and no memories.
use super::{body_id, id_or_all, ok, only_keys, query_id, require_ai_user, Answer, Ctx};
use crate::chat::{self, Frame, Generation, Kind, StopReason};
use crate::codes::ApiError;
use crate::identity::Identity;
use crate::prompt::{self, Lang};
use crate::provider::{self, Role, UpCode};
use crate::store::{data, stats};
use crate::util::{char_len, chat_title, collapse_ws, cut_chars, unix_now};
use axum::body::{Body, Bytes};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;
use serde_json::{json, Map, Value};
use std::convert::Infallible;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

pub async fn status(ctx: Ctx) -> Answer {
    let app = &ctx.app.0;
    let who = &ctx.who;
    let snap = app.snapshot()?;
    // Available: the effective AI+ service, and the main service accepted a report less than 60 s ago
    let eff = app.available(&snap);
    let suspended = app.suspended(who);
    let has = app.store.read(|c| data::has_data(c, who.id))?;
    let running = app
        .running_of(who.id)
        .map_or(Value::Null, |(c, m)| json!({"chatId": c, "messageId": m}));
    let l = app.limits();
    let admin = who.is_admin();
    ok(json!({
        "enabled": eff,
        "allowed": eff && who.ai_allowed && !suspended,
        "suspended": suspended,
        "hasData": has,
        "running": running,
        "retentionDays": snap.settings.retention_days,
        "memoryLimit": l.memories_per_user,
        "memoryMaxChars": l.memory_chars,
        "maxMessageChars": l.message_chars,
        "messageChars": l.message_chars,
        "memoryChars": l.memory_chars,
        "keyProblem": admin && snap.key_problem.is_some(),
        "configured": admin && snap.configured(),
        "freeMode": admin && free_mode_of(app, who)?,
    }))
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

pub async fn chat(ctx: Ctx) -> Answer {
    let app = &ctx.app.0;
    let id = query_id(ctx.query.as_deref(), "id")?;
    let found = app.store.read(|c| {
        let Some(chat) = data::chat(c, ctx.who.id, id)? else {
            return Ok(None);
        };
        Ok(Some((chat, data::messages(c, id)?)))
    })?;
    let Some((chat, msgs)) = found else {
        return Err(ApiError::not_found());
    };
    let messages: Vec<Value> = msgs.iter().map(data::MessageRow::to_json).collect();
    ok(json!({"chat": chat.to_json(app.chat_running(id)), "messages": messages}))
}

/// The SSE body: frames from the generation, a `: keep-alive` comment when nothing was sent for a while.
fn sse_response(rx: UnboundedReceiver<Frame>, keep_alive: Duration) -> Response {
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

/// What the transaction of a send / retry prepared.
struct Prepared {
    chat: i64,
    title: String,
    new_chat: bool,
    /// The administrator's free mode was on when the message was sent (no study-assistant prompt, no memories)
    free: bool,
    user_msg: i64,
    answer: i64,
    history: Vec<provider::Turn>,
    /// The history sent holds messages of the other mode (a mode block goes into the system prompt)
    switched: bool,
    /// Empty in free mode (not even read)
    memories: Vec<String>,
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

/// What a send / retry streams: the prepared answer, the reply language, and whether it is a retry.
struct Stream {
    p: Prepared,
    lang: Lang,
    retry: bool,
}

/// Starts the generation of a prepared answer and returns the SSE response.
fn start_stream(
    ctx: &Ctx,
    snap: &crate::app::Snapshot,
    target: crate::provider::Target,
    slot: chat::Slot,
    permit: chat::GatePermit,
    stream: Stream,
) -> Answer {
    let Stream { p, lang, retry } = stream;
    let app = &ctx.app;
    let s = &snap.settings;
    let system = if p.free {
        prompt::free_prompt(lang, p.switched)
    } else {
        let today = crate::util::local_today().to_string();
        prompt::system_prompt(lang, &today, &p.memories, p.switched)
    };
    let body = provider::chat_body(
        s.wire_format(),
        &s.model,
        &system,
        &p.history,
        s.usage_option,
        s.max_tokens,
    );
    let request = match target.chat_request(&app.0.client, &body) {
        Ok(r) => r,
        Err(e) => {
            let now = unix_now();
            app.0.store.write(|tx| {
                data::finish_answer(tx, p.answer, "", "error", Some(e.code.code()), false, now)
            })?;
            return Err(ApiError::upstream(e.code));
        }
    };
    drop(target);
    slot.set_target(p.chat, p.answer);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let _ = tx.send(Frame {
        event: "start",
        data: json!({
            "chatId": p.chat,
            "chatTitle": p.title,
            "userMessageId": p.user_msg,
            "assistantMessageId": p.answer,
            "model": s.model,
            "newChat": p.new_chat,
            "retry": retry,
            "free": p.free,
        }),
    });
    let g = Generation {
        app: app.clone(),
        slot,
        permit,
        who: ctx.who.clone(),
        chat: p.chat,
        message: p.answer,
        model: s.model.clone(),
        provider: s.provider,
        format: s.wire_format(),
        request,
        frames: tx,
    };
    tokio::spawn(chat::run(g));
    Ok(sse_response(rx, app.0.timeouts.keep_alive))
}

/// Takes the caller's place and a global place, then checks again that AI is still on and the caller's AI+ function is
/// not suspended (a switch-off or a suspension in between would otherwise miss this call: both stop only the calls that
/// already hold a place).
fn take_places(
    ctx: &Ctx,
    kind: Kind,
    chat: Option<i64>,
) -> Result<(chat::Slot, chat::GatePermit), ApiError> {
    let app = &ctx.app.0;
    let l = app.limits();
    let slot = app
        .registry
        .reserve(
            ctx.who.id,
            kind,
            chat,
            crate::limits::Limits::usize_of(l.per_user_running),
        )
        .ok_or_else(ApiError::busy)?;
    let permit = app.gate.try_acquire().ok_or_else(ApiError::server_busy)?;
    if !app.snapshot()?.effective() {
        return Err(ApiError::disabled());
    }
    if app.suspended(&ctx.who) {
        return Err(ApiError::suspended());
    }
    Ok((slot, permit))
}

pub async fn send(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["chatId", "text", "lang", "free"])?;
    let snap = require_ai_user(&ctx.app, &ctx.who)?;
    let chat_id = match ctx.body.get("chatId") {
        None | Some(Value::Null) => None,
        Some(_) => body_id(&ctx.body, "chatId")?,
    };
    let text = match ctx.body.get("text") {
        Some(Value::String(s)) => s.trim().to_string(),
        _ => return Err(ApiError::bad_request()),
    };
    let lang = lang_of(&ctx.body)?;
    // A `free` in the body is ignored: free mode is a state of the user (`POST /api/ai/free`), read below
    let limits = ctx.app.0.limits();
    if text.is_empty() {
        return Err(ApiError::bad_request());
    }
    if char_len(&text) > crate::limits::Limits::usize_of(limits.message_chars) {
        return Err(ApiError::too_long(limits.message_chars));
    }
    let target = ctx.app.0.target(&snap)?;
    let (slot, permit) = take_places(&ctx, Kind::Chat, chat_id)?;
    let who = &ctx.who;
    let model = snap.settings.model.clone();
    let now = unix_now();
    let budget = limits.history_budget();
    let p = ctx.app.0.store.write(|tx| -> Result<Prepared, ApiError> {
        data::ensure_user(tx, who, now)?;
        let free = who.is_admin() && data::free_mode(tx, who.id)?;
        let (chat, title, new_chat) = match chat_id {
            Some(id) => {
                let c = data::chat(tx, who.id, id)?.ok_or_else(ApiError::not_found)?;
                if c.message_count + 2 > limits.messages_per_chat {
                    return Err(ApiError::message_limit(limits.messages_per_chat));
                }
                (id, c.title, false)
            }
            None => {
                if data::count_chats(tx, who.id)? >= limits.chats_per_user {
                    return Err(ApiError::chat_limit(limits.chats_per_user));
                }
                let title = chat_title(&text);
                let id = data::create_chat(tx, who.id, &title, now)?;
                (id, title, true)
            }
        };
        let prev = data::messages(tx, chat)?;
        let user_msg = data::insert_message(
            tx,
            &data::NewMessage {
                chat,
                role: "user",
                content: &text,
                status: "done",
                model: None,
                free,
                now,
            },
        )?;
        let answer = data::insert_message(
            tx,
            &data::NewMessage {
                chat,
                role: "assistant",
                content: "",
                status: "streaming",
                model: Some(&model),
                free,
                now,
            },
        )?;
        data::touch_chat(tx, chat, now)?;
        let h = prev_turns(&prev, &text, budget, free);
        Ok(Prepared {
            chat,
            title,
            new_chat,
            free,
            user_msg,
            answer,
            history: h.turns,
            switched: h.other_mode,
            memories: prompt_memories(tx, who, free)?,
        })
    })?;
    let stream = Stream {
        p,
        lang,
        retry: false,
    };
    start_stream(&ctx, &snap, target, slot, permit, stream)
}

pub async fn retry(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["chatId", "lang"])?;
    let snap = require_ai_user(&ctx.app, &ctx.who)?;
    let chat_id = body_id(&ctx.body, "chatId")?.ok_or_else(ApiError::bad_request)?;
    let lang = lang_of(&ctx.body)?;
    let target = ctx.app.0.target(&snap)?;
    let (slot, permit) = take_places(&ctx, Kind::Chat, Some(chat_id))?;
    let who = &ctx.who;
    let model = snap.settings.model.clone();
    let now = unix_now();
    let budget = ctx.app.0.limits().history_budget();
    let p = ctx.app.0.store.write(|tx| -> Result<Prepared, ApiError> {
        let c = data::chat(tx, who.id, chat_id)?.ok_or_else(ApiError::not_found)?;
        data::ensure_user(tx, who, now)?;
        let free = who.is_admin() && data::free_mode(tx, who.id)?;
        let msgs = data::messages(tx, chat_id)?;
        let n = msgs.len();
        let (Some(last), Some(question)) =
            (msgs.last(), n.checked_sub(2).and_then(|i| msgs.get(i)))
        else {
            return Err(ApiError::not_retryable());
        };
        let retryable = last.role == "assistant"
            && (last.status == "error" || last.status == "stopped")
            && question.role == "user";
        if !retryable {
            return Err(ApiError::not_retryable());
        }
        data::reset_answer(tx, last.id, &model, free)?;
        data::set_message_free(tx, question.id, free)?;
        data::touch_chat(tx, chat_id, now)?;
        let h = prev_turns(
            msgs.get(..n - 2).unwrap_or(&[]),
            &question.content,
            budget,
            free,
        );
        Ok(Prepared {
            chat: chat_id,
            title: c.title,
            new_chat: false,
            free,
            user_msg: question.id,
            answer: last.id,
            history: h.turns,
            switched: h.other_mode,
            memories: prompt_memories(tx, who, free)?,
        })
    })?;
    let stream = Stream {
        p,
        lang,
        retry: true,
    };
    start_stream(&ctx, &snap, target, slot, permit, stream)
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
    ok(json!({"memories": list, "limit": l.memories_per_user, "maxChars": l.memory_chars}))
}

/// Memory text: whitespace collapsed (a note is one line in the prompt), 1 to `memoryChars` characters.
fn memory_text(ctx: &Ctx) -> Result<String, ApiError> {
    let raw = match ctx.body.get("content") {
        Some(Value::String(s)) => s,
        _ => return Err(ApiError::bad_request()),
    };
    let text = collapse_ws(raw);
    let max = ctx.app.0.limits().memory_chars;
    if text.is_empty() {
        return Err(ApiError::bad_request());
    }
    if char_len(&text) > crate::limits::Limits::usize_of(max) {
        return Err(ApiError::memory_too_long(max));
    }
    Ok(text)
}

pub async fn memory_add(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["content", "source"])?;
    require_ai_user(&ctx.app, &ctx.who)?;
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
    require_ai_user(&ctx.app, &ctx.who)?;
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
    let snap = require_ai_user(&ctx.app, &ctx.who)?;
    let msg = body_id(&ctx.body, "messageId")?.ok_or_else(ApiError::bad_request)?;
    let lang = lang_of(&ctx.body)?;
    let app = &ctx.app.0;
    let who = &ctx.who;
    let Some((_, text)) = app.store.read(|c| data::user_message(c, who.id, msg))? else {
        return Err(ApiError::not_found());
    };
    let target = app.target(&snap)?;
    let (slot, permit) = take_places(&ctx, Kind::Suggest, None)?;
    let s = &snap.settings;
    let turns = [provider::Turn {
        role: Role::User,
        content: cut_chars(&text, prompt::SUGGEST_INPUT_CHARS),
    }];
    let body = provider::chat_body(
        s.wire_format(),
        &s.model,
        &prompt::suggest_prompt(lang),
        &turns,
        s.usage_option,
        s.max_tokens,
    );
    let req = target
        .chat_request(&app.client, &body)
        .map_err(|e| ApiError::upstream(e.code))?;
    drop(target);
    let started = std::time::Instant::now();
    let t = app.timeouts;
    let mut cancel = slot.cancel.clone();
    let mut shutdown = app.shutdown.clone();
    let result = tokio::select! {
        r = provider::collect_chat(&app.client, req, s.wire_format(), t, t.short_total) => Ok(r),
        _ = cancel.wait_for(|v| v.is_some()) => Err(slot.stopped().unwrap_or(StopReason::Stop)),
        _ = shutdown.wait_for(|v| *v) => Err(StopReason::Shutdown),
    };
    let now = unix_now();
    let (status, code, tin, tout) = match &result {
        Ok(Ok(c)) => ("ok", None, c.tokens_in, c.tokens_out),
        Ok(Err(e)) => ("error", Some(e.code.short().to_string()), None, None),
        Err(_) => ("stopped", None, None, None),
    };
    let usage = stats::UsageRow {
        user_id: Some(who.id),
        at: now,
        kind: "memory",
        provider: s.provider.as_str(),
        model: s.model.clone(),
        tokens_in: tin,
        tokens_out: tout,
        tokens_cached: None,
        latency_ms: provider::ms(started.elapsed()),
        first_byte_ms: None,
        status,
        error_code: code,
    };
    app.store.write(|tx| {
        data::ensure_user(tx, who, now)?;
        stats::insert_usage(tx, &usage)
    })?;
    drop(permit);
    drop(slot);
    match result {
        Ok(Ok(c)) => {
            let max = crate::limits::Limits::usize_of(app.limits().memory_chars);
            let suggestion = cut_chars(&collapse_ws(c.text.trim()), max);
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
    }
}
