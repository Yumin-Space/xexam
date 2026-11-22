//! Users, chats, messages and memories. Every query of user content filters by the owner's id.
use crate::identity::Identity;
use crate::study::assess::parse_assess_line;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Most messages `path` walks up (a cycle guard; `messagesPerChat` is at most 5000).
pub const PATH_LIMIT: i64 = 5001;

/// The three counts of a summary (`messages.sample_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StudySample {
    /// Wrong answers sent to the model
    pub used: i64,
    /// Wrong answers whose question was still available
    pub usable: i64,
    /// Wrong answers in the record
    pub wrong: i64,
}

impl StudySample {
    pub fn to_json(&self) -> Value {
        json!({"used": self.used, "usable": self.usable, "wrong": self.wrong})
    }
}

/// A chat as listed (`ChatSummary` without `running`, which comes from the registry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRow {
    pub id: i64,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub message_count: i64,
    /// 'chat' | 'practice' | 'exam' | 'summary'
    pub kind: String,
    pub record_id: Option<i64>,
    /// Last message of the viewed branch (`chats.leaf_id`); None = no message yet
    pub leaf_id: Option<i64>,
    /// The administrator's model pick (connection id, model id); None = the chat default (also for a stored pick whose
    /// connection no longer exists)
    pub pick: Option<(i64, String)>,
    /// v2.1: the administrator's per-chat strength level (`chats.pick_effort`); None = no choice. The family is the
    /// route's at call time; a level that is not one of it is ignored by the caller.
    pub pick_effort: Option<String>,
}

impl ChatRow {
    pub fn to_json(&self, running: bool) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "createdAt": self.created_at,
            "updatedAt": self.updated_at,
            "messageCount": self.message_count,
            "running": running,
            "kind": self.kind,
            "recordId": self.record_id,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRow {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub status: String,
    pub error_code: Option<String>,
    pub truncated: bool,
    pub model: Option<String>,
    pub created_at: i64,
    /// Written in the administrator's free mode
    pub free: bool,
    /// 'chat' | 'hint' | 'explain' | 'summary'
    pub kind: String,
    pub question_id: Option<String>,
    pub question_no: Option<i64>,
    /// 'practice' | 'result'
    pub source: Option<String>,
    /// 'en' | 'zh-CN' | 'ja'
    pub lang: Option<String>,
    /// `chats.record_id` when `source = 'result'` or `kind = 'summary'`, else None
    pub record_id: Option<i64>,
    /// Some only when all three `sample_*` columns are set
    pub sample: Option<StudySample>,
    /// The message this one follows; None = a root
    pub parent_id: Option<i64>,
    /// Name of the connection that wrote an assistant answer; None before v1.3 and for user messages
    pub connection: Option<String>,
    /// v1.3: the model's reasoning of an AI Chat answer (trimmed, capped); None = none / an older answer. Never sent
    /// to the model.
    pub reasoning: Option<String>,
    /// Milliseconds from the first reasoning to the first answer text (or the end); None while it still runs
    pub reasoning_ms: Option<i64>,
    /// `reasoning` was cut at [`MAX_REASONING_BYTES`]
    pub reasoning_truncated: bool,
}

impl MessageRow {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "role": self.role,
            "content": self.content,
            "status": self.status,
            "error": self.error_code,
            "truncated": self.truncated,
            "model": self.model,
            "createdAt": self.created_at,
            "free": self.free,
            "kind": self.kind,
            "questionId": self.question_id,
            "questionNo": self.question_no,
            "source": self.source,
            "lang": self.lang,
            "recordId": self.record_id,
            "sample": self.sample.map(|s| s.to_json()),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRow {
    pub id: i64,
    pub content: String,
    pub source: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl MemoryRow {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "content": self.content,
            "source": self.source,
            "createdAt": self.created_at,
            "updatedAt": self.updated_at,
        })
    }
}

/// (stamp, username) of a known user.
pub fn user_row(c: &Connection, id: i64) -> rusqlite::Result<Option<(String, String)>> {
    c.query_row(
        "SELECT stamp, username FROM users WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

/// Every user id known to the AI service, with its stamp and username.
pub fn all_users(c: &Connection) -> rusqlite::Result<Vec<(i64, String, String)>> {
    let mut st = c.prepare("SELECT id, stamp, username FROM users ORDER BY id")?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    rows.collect()
}

/// Makes sure the user's row exists (and refreshes the username / last seen time). Called inside every transaction that
/// writes user-owned rows, so the foreign keys always hold.
pub fn ensure_user(c: &Connection, who: &Identity, now: i64) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(id) DO UPDATE SET username = excluded.username, last_seen_at = excluded.last_seen_at
         WHERE users.stamp = excluded.stamp",
        params![who.id, who.stamp, who.username, now],
    )?;
    Ok(())
}

pub fn rename_user(c: &Connection, id: i64, username: &str) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE users SET username = ?2 WHERE id = ?1 AND username <> ?2",
        params![id, username],
    )?;
    Ok(())
}

/// Removes a user's row; chats, messages, memories and tasks cascade, usage rows become anonymous.
pub fn delete_user(c: &Connection, id: i64) -> rusqlite::Result<bool> {
    Ok(c.execute("DELETE FROM users WHERE id = ?1", [id])? > 0)
}

/// `false` when an administrator suspended the user's AI+ function (`users.function_enabled = 0`); a user without a
/// row is active.
pub fn function_enabled(c: &Connection, user: i64) -> rusqlite::Result<bool> {
    Ok(c.query_row(
        "SELECT function_enabled FROM users WHERE id = ?1",
        [user],
        |r| r.get::<_, i64>(0),
    )
    .optional()?
    .is_none_or(|v| v != 0))
}

/// Sets the AI+ function flag of a user, creating the user's row first when it is missing (`stamp` and `username` as
/// the main backend describes the user). Returns `true` when the stored value changed (a new row counts as a change
/// only when it is created suspended).
pub fn set_function_enabled(
    c: &Connection,
    user: i64,
    stamp: &str,
    username: &str,
    enabled: bool,
    now: i64,
) -> rusqlite::Result<bool> {
    let was = function_enabled(c, user)?;
    c.execute(
        "INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at, function_enabled)
         VALUES (?1, ?2, ?3, ?4, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET function_enabled = excluded.function_enabled, username = excluded.username",
        params![user, stamp, username, now, i64::from(enabled)],
    )?;
    Ok(was != enabled)
}

/// The user's switched-off feature bits (`users.features_off`, see `features`); 0 for a user without a row.
pub fn features_off(c: &Connection, user: i64) -> rusqlite::Result<i64> {
    Ok(c.query_row(
        "SELECT features_off FROM users WHERE id = ?1",
        [user],
        |r| r.get::<_, i64>(0),
    )
    .optional()?
    .unwrap_or(0))
}

/// Sets the feature bits of a user, creating the row first when it is missing (like [`set_function_enabled`]). Returns
/// the value stored before (0 when there was no row).
pub fn set_features_off(
    c: &Connection,
    user: i64,
    stamp: &str,
    username: &str,
    off: i64,
    now: i64,
) -> rusqlite::Result<i64> {
    let was = features_off(c, user)?;
    c.execute(
        "INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at, features_off)
         VALUES (?1, ?2, ?3, ?4, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET features_off = excluded.features_off, username = excluded.username",
        params![user, stamp, username, now, off],
    )?;
    Ok(was)
}

/// Ids of the users whose AI+ function is suspended, lowest first, at most `limit`.
pub fn suspended_ids(c: &Connection, limit: usize) -> rusqlite::Result<Vec<i64>> {
    let mut st =
        c.prepare("SELECT id FROM users WHERE function_enabled = 0 ORDER BY id LIMIT ?1")?;
    let rows = st.query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |r| r.get(0))?;
    rows.collect()
}

/// The pick is read through `connections`: a pick whose connection is gone reads as no pick (the chat default).
///
/// A study assessment is stored as a `summary` chat whose ref starts with `assess@` (the CHECKs of `chats.kind` stay
/// as they are); it is read as the wire kind `assess` ([`CHAT_KIND_SQL`], [`MSG_KIND_SQL`]).
const CHAT_COLS: &str = "c.id, c.title, c.created_at, c.updated_at,
     (SELECT COUNT(*) FROM messages m WHERE m.chat_id = c.id),
     CASE WHEN c.kind = 'summary' AND c.ref LIKE 'assess@%' THEN 'assess' ELSE c.kind END,
     c.record_id, c.leaf_id,
     (SELECT k.id FROM connections k WHERE k.id = c.pick_conn), c.pick_model, c.pick_effort";

fn chat_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChatRow> {
    Ok(ChatRow {
        id: r.get(0)?,
        title: r.get(1)?,
        created_at: r.get(2)?,
        updated_at: r.get(3)?,
        message_count: r.get(4)?,
        kind: r.get(5)?,
        record_id: r.get(6)?,
        leaf_id: r.get(7)?,
        pick: match (r.get::<_, Option<i64>>(8)?, r.get::<_, Option<String>>(9)?) {
            (Some(conn), Some(model)) => Some((conn, model)),
            _ => None,
        },
        pick_effort: r.get(10)?,
    })
}

/// The user's chats, newest `updatedAt` first.
pub fn list_chats(c: &Connection, user: i64) -> rusqlite::Result<Vec<ChatRow>> {
    let sql = format!(
        "SELECT {CHAT_COLS} FROM chats c WHERE c.user_id = ?1 ORDER BY c.updated_at DESC, c.id DESC"
    );
    let mut st = c.prepare(&sql)?;
    let rows = st.query_map([user], chat_from)?;
    rows.collect()
}

/// One chat of the user (`None` for a missing chat and for another user's chat alike).
pub fn chat(c: &Connection, user: i64, id: i64) -> rusqlite::Result<Option<ChatRow>> {
    let sql = format!("SELECT {CHAT_COLS} FROM chats c WHERE c.id = ?1 AND c.user_id = ?2");
    c.query_row(&sql, [id, user], chat_from).optional()
}

/// The sample of a row: Some only when all three counts are set.
fn sample_of(used: Option<i64>, usable: Option<i64>, wrong: Option<i64>) -> Option<StudySample> {
    match (used, usable, wrong) {
        (Some(used), Some(usable), Some(wrong)) => Some(StudySample {
            used,
            usable,
            wrong,
        }),
        _ => None,
    }
}

/// The wire kind of a chat row `c` (see [`CHAT_COLS`]).
const CHAT_KIND_SQL: &str =
    "CASE WHEN c.kind = 'summary' AND c.ref LIKE 'assess@%' THEN 'assess' ELSE c.kind END";
/// The wire kind of a message row `m` of a chat row `c`.
const MSG_KIND_SQL: &str =
    "CASE WHEN m.kind = 'summary' AND c.ref LIKE 'assess@%' THEN 'assess' ELSE m.kind END";

/// Columns of a message as [`MessageRow`] reads them (the query joins `chats c`).
const MSG_COLS: &str =
    "m.id, m.role, m.content, m.status, m.error_code, m.truncated, m.model, m.created_at, m.free,
                CASE WHEN m.kind = 'summary' AND c.ref LIKE 'assess@%' THEN 'assess' ELSE m.kind END,
                m.question_id, m.question_no, m.source, m.lang,
                CASE WHEN m.source = 'result' OR m.kind = 'summary' THEN c.record_id END,
                m.sample_used, m.sample_usable, m.sample_wrong, m.parent_id, m.connection,
                m.reasoning, m.reasoning_ms, m.reasoning_truncated";

fn message_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        id: r.get(0)?,
        role: r.get(1)?,
        content: r.get(2)?,
        status: r.get(3)?,
        error_code: r.get(4)?,
        truncated: r.get::<_, i64>(5)? != 0,
        model: r.get(6)?,
        created_at: r.get(7)?,
        free: r.get::<_, i64>(8)? != 0,
        kind: r.get(9)?,
        question_id: r.get(10)?,
        question_no: r.get(11)?,
        source: r.get(12)?,
        lang: r.get(13)?,
        record_id: r.get(14)?,
        sample: sample_of(r.get(15)?, r.get(16)?, r.get(17)?),
        parent_id: r.get(18)?,
        connection: r.get(19)?,
        reasoning: r.get(20)?,
        reasoning_ms: r.get(21)?,
        reasoning_truncated: r.get::<_, i64>(22)? != 0,
    })
}

/// ALL messages of a chat, oldest first, every version included (tests and tools; no route uses it since v1.3, they use
/// [`path`]). The caller has checked that the chat is the user's. `record_id` is the chat's, shown only for result-page
/// answers and summaries.
pub fn messages(c: &Connection, chat: i64) -> rusqlite::Result<Vec<MessageRow>> {
    let mut st = c.prepare(&format!(
        "SELECT {MSG_COLS} FROM messages m JOIN chats c ON c.id = m.chat_id WHERE m.chat_id = ?1 ORDER BY m.id"
    ))?;
    let rows = st.query_map([chat], message_from)?;
    rows.collect()
}

/// The viewed line of a chat, oldest first: from `leaf` (default `chats.leaf_id`) up the `parent_id` chain, every step
/// inside the chat, at most [`PATH_LIMIT`] messages (cycle guard). Empty for a chat without messages. The caller has
/// checked that the chat is the user's.
pub fn path(c: &Connection, chat: i64, leaf: Option<i64>) -> rusqlite::Result<Vec<MessageRow>> {
    let leaf = match leaf {
        Some(l) => Some(l),
        None => c
            .query_row("SELECT leaf_id FROM chats WHERE id = ?1", [chat], |r| {
                r.get::<_, Option<i64>>(0)
            })
            .optional()?
            .flatten(),
    };
    let Some(leaf) = leaf else {
        return Ok(Vec::new());
    };
    let mut st = c.prepare(&format!(
        "WITH RECURSIVE line(id, parent, depth) AS (
           SELECT id, parent_id, 1 FROM messages WHERE id = ?2 AND chat_id = ?1
           UNION ALL
           SELECT p.id, p.parent_id, line.depth + 1
           FROM line JOIN messages p ON p.id = line.parent AND p.chat_id = ?1
           WHERE line.depth < ?3)
         SELECT {MSG_COLS} FROM line JOIN messages m ON m.id = line.id JOIN chats c ON c.id = m.chat_id
         ORDER BY line.depth DESC LIMIT ?3"
    ))?;
    let rows = st.query_map(params![chat, leaf, PATH_LIMIT], message_from)?;
    rows.collect()
}

/// The versions of the messages of a path: message id -> the ids of all messages with the same parent and the same role
/// in the chat (the message itself included), ordered by id. Only groups of two or more are returned.
pub fn versions(
    c: &Connection,
    chat: i64,
    path: &[MessageRow],
) -> rusqlite::Result<HashMap<i64, Vec<i64>>> {
    let mut out = HashMap::new();
    if path.is_empty() {
        return Ok(out);
    }
    let mut parents: Vec<i64> = path.iter().filter_map(|m| m.parent_id).collect();
    parents.sort_unstable();
    parents.dedup();
    let mut cond = String::from("parent_id IS NULL");
    if !parents.is_empty() {
        let list: Vec<String> = parents.iter().map(i64::to_string).collect();
        cond = format!("{cond} OR parent_id IN ({})", list.join(","));
    }
    let mut st = c.prepare(&format!(
        "SELECT id, parent_id, role FROM messages WHERE chat_id = ?1 AND ({cond}) ORDER BY id"
    ))?;
    let rows = st.query_map([chat], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut groups: HashMap<(Option<i64>, String), Vec<i64>> = HashMap::new();
    for row in rows {
        let (id, parent, role) = row?;
        groups.entry((parent, role)).or_default().push(id);
    }
    for m in path {
        if let Some(g) = groups.get(&(m.parent_id, m.role.clone())) {
            if g.len() > 1 {
                out.insert(m.id, g.clone());
            }
        }
    }
    Ok(out)
}

/// The end of the line that runs through `from`: repeat "newest child (highest id)" until a message has none.
pub fn line_end(c: &Connection, chat: i64, from: i64) -> rusqlite::Result<i64> {
    let mut cur = from;
    for _ in 0..PATH_LIMIT {
        let next: Option<i64> = c.query_row(
            "SELECT MAX(id) FROM messages WHERE chat_id = ?1 AND parent_id = ?2",
            [chat, cur],
            |r| r.get(0),
        )?;
        match next {
            Some(n) => cur = n,
            None => break,
        }
    }
    Ok(cur)
}

/// Makes `leaf` the viewed end of the chat. `chats.updated_at` is untouched. Fails with `QueryReturnedNoRows` when
/// `leaf` is not a message of this chat (the leaf is always a message of its chat).
pub fn set_leaf(c: &Connection, chat: i64, leaf: i64) -> rusqlite::Result<()> {
    let n = c.execute(
        "UPDATE chats SET leaf_id = ?2
         WHERE id = ?1 AND EXISTS (SELECT 1 FROM messages WHERE id = ?2 AND chat_id = ?1)",
        [chat, leaf],
    )?;
    if n == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

/// Stores (or clears, with None) the administrator's model pick of a chat. `updated_at` is untouched.
pub fn set_chat_pick(c: &Connection, chat: i64, pick: Option<(i64, &str)>) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE chats SET pick_conn = ?2, pick_model = ?3 WHERE id = ?1",
        params![chat, pick.map(|p| p.0), pick.map(|p| p.1)],
    )?;
    Ok(())
}

/// Stores (or clears, with None) the administrator's per-chat strength level. `updated_at` is untouched; the model
/// pick is left alone.
pub fn set_chat_pick_effort(
    c: &Connection,
    chat: i64,
    level: Option<&str>,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE chats SET pick_effort = ?2 WHERE id = ?1",
        params![chat, level],
    )?;
    Ok(())
}

/// `true` when `message` is on the viewed line (the path of `chats.leaf_id`).
pub fn on_path(c: &Connection, chat: i64, message: i64) -> rusqlite::Result<bool> {
    c.query_row(
        "WITH RECURSIVE line(id, parent, depth) AS (
           SELECT m.id, m.parent_id, 1 FROM chats c JOIN messages m ON m.id = c.leaf_id AND m.chat_id = c.id
           WHERE c.id = ?1
           UNION ALL
           SELECT p.id, p.parent_id, line.depth + 1
           FROM line JOIN messages p ON p.id = line.parent AND p.chat_id = ?1
           WHERE line.depth < ?3)
         SELECT EXISTS(SELECT 1 FROM line WHERE id = ?2)",
        params![chat, message, PATH_LIMIT],
        |r| r.get(0),
    )
}

/// Number of assistant answers (versions) that follow the user message `question`.
pub fn answer_versions(c: &Connection, chat: i64, question: i64) -> rusqlite::Result<i64> {
    c.query_row(
        "SELECT COUNT(*) FROM messages WHERE chat_id = ?1 AND parent_id = ?2 AND role = 'assistant'",
        [chat, question],
        |r| r.get(0),
    )
}

/// The viewed end of a chat (`chats.leaf_id`); None for no message yet.
pub fn leaf_of(c: &Connection, chat: i64) -> rusqlite::Result<Option<i64>> {
    Ok(
        c.query_row("SELECT leaf_id FROM chats WHERE id = ?1", [chat], |r| {
            r.get::<_, Option<i64>>(0)
        })
        .optional()?
        .flatten(),
    )
}

pub fn count_chats(c: &Connection, user: i64) -> rusqlite::Result<i64> {
    c.query_row(
        "SELECT COUNT(*) FROM chats WHERE user_id = ?1",
        [user],
        |r| r.get(0),
    )
}

pub fn count_messages(c: &Connection, chat: i64) -> rusqlite::Result<i64> {
    c.query_row(
        "SELECT COUNT(*) FROM messages WHERE chat_id = ?1",
        [chat],
        |r| r.get(0),
    )
}

pub fn count_memories(c: &Connection, user: i64) -> rusqlite::Result<i64> {
    c.query_row(
        "SELECT COUNT(*) FROM memories WHERE user_id = ?1",
        [user],
        |r| r.get(0),
    )
}

/// `true` when the user has at least one chat or memory.
pub fn has_data(c: &Connection, user: i64) -> rusqlite::Result<bool> {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM chats WHERE user_id = ?1) OR EXISTS(SELECT 1 FROM memories WHERE user_id = ?1)",
        [user],
        |r| r.get(0),
    )
}

/// Creates a chat; returns its id.
pub fn create_chat(c: &Connection, user: i64, title: &str, now: i64) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO chats (user_id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
        params![user, title, now],
    )?;
    Ok(c.last_insert_rowid())
}

/// The administrator's free mode (`users.free_mode`); a user without a row is not in free mode. The caller combines it
/// with "is an administrator".
pub fn free_mode(c: &Connection, user: i64) -> rusqlite::Result<bool> {
    Ok(
        c.query_row("SELECT free_mode FROM users WHERE id = ?1", [user], |r| {
            r.get::<_, i64>(0)
        })
        .optional()?
        .is_some_and(|v| v != 0),
    )
}

/// A-OWNER-21: sets the free mode of an administrator account by id (the row is created first when it is missing, under
/// the account's current stamp; a row of an older account under the same id is purged by the caller first). Returns the
/// previous value.
pub fn set_free_mode_of(
    c: &Connection,
    id: i64,
    stamp: &str,
    username: &str,
    free: bool,
    now: i64,
) -> rusqlite::Result<bool> {
    c.execute(
        "INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(id) DO NOTHING",
        params![id, stamp, username, now],
    )?;
    let was = free_mode(c, id)?;
    c.execute(
        "UPDATE users SET free_mode = ?2 WHERE id = ?1",
        params![id, i64::from(free)],
    )?;
    Ok(was)
}

/// Sets the free mode of the caller (the row is created first when it is missing).
pub fn set_free_mode(c: &Connection, who: &Identity, free: bool, now: i64) -> rusqlite::Result<()> {
    ensure_user(c, who, now)?;
    c.execute(
        "UPDATE users SET free_mode = ?2 WHERE id = ?1",
        params![who.id, i64::from(free)],
    )?;
    Ok(())
}

/// A message to insert (`free` = the mode of the turn it belongs to).
pub struct NewMessage<'a> {
    pub chat: i64,
    pub role: &'a str,
    pub content: &'a str,
    pub status: &'a str,
    pub model: Option<&'a str>,
    pub free: bool,
    pub now: i64,
    /// The message this one follows. `None` = the chat's current leaf (v1.2 behaviour: the line grows at its end), which
    /// is a root while the chat is empty. A `Some` must be a message of the same chat (else `QueryReturnedNoRows`).
    pub parent: Option<i64>,
    /// Connection NAME that writes an assistant answer
    pub connection: Option<&'a str>,
}

/// Inserts a message after `m.parent` (default: the current leaf) and makes it the leaf of the chat; returns its id. A
/// pair insert (question, then answer with `parent: Some(question)` or `None`) therefore ends with the answer as the leaf.
pub fn insert_message(c: &Connection, m: &NewMessage<'_>) -> rusqlite::Result<i64> {
    let parent = match m.parent {
        Some(p) => {
            let ok: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE id = ?1 AND chat_id = ?2)",
                [p, m.chat],
                |r| r.get(0),
            )?;
            if !ok {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            Some(p)
        }
        None => leaf_of(c, m.chat)?,
    };
    c.execute(
        "INSERT INTO messages (chat_id, role, content, status, model, created_at, finished_at, free, parent_id,
                               connection)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, CASE WHEN ?4 = 'streaming' THEN NULL ELSE ?6 END, ?7, ?8, ?9)",
        params![
            m.chat,
            m.role,
            m.content,
            m.status,
            m.model,
            m.now,
            i64::from(m.free),
            parent,
            m.connection
        ],
    )?;
    let id = c.last_insert_rowid();
    set_leaf(c, m.chat, id)?;
    Ok(id)
}

pub fn touch_chat(c: &Connection, chat: i64, now: i64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE chats SET updated_at = ?2 WHERE id = ?1",
        params![chat, now],
    )?;
    Ok(())
}

/// Resets an answer for a retry; the retry's mode (`free`) is stored on the answer and on its question.
pub fn reset_answer(c: &Connection, msg: i64, model: &str, free: bool) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE messages SET content = '', status = 'streaming', error_code = NULL, truncated = 0, model = ?2,
                finished_at = NULL, free = ?3, reasoning = NULL, reasoning_ms = NULL, reasoning_truncated = 0
         WHERE id = ?1",
        params![msg, model, i64::from(free)],
    )?;
    Ok(())
}

/// Stores the mode of a message (the question of a retry follows its answer).
pub fn set_message_free(c: &Connection, msg: i64, free: bool) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE messages SET free = ?2 WHERE id = ?1",
        params![msg, i64::from(free)],
    )?;
    Ok(())
}

/// Most bytes (UTF-8) of stored reasoning per answer; the head is kept, cut at a char boundary.
pub const MAX_REASONING_BYTES: usize = 32 * 1024;

/// The head of `s` of at most `max` bytes (cut at a char boundary) and whether it was cut.
pub fn cap_head(s: &str, max: usize) -> (&str, bool) {
    if s.len() <= max {
        return (s, false);
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    (&s[..end], true)
}

/// Reasoning of an AI Chat answer. `text` is trimmed (both ends) and capped; empty after trimming clears all three
/// columns (NULL, NULL, 0). `truncated` is OR-ed with the cap's own cut. No status guard (written by the running
/// generation's flush and by its final transaction only).
pub fn set_reasoning(
    c: &Connection,
    msg: i64,
    text: &str,
    ms: Option<i64>,
    truncated: bool,
) -> rusqlite::Result<()> {
    let (head, cut) = cap_head(text.trim(), MAX_REASONING_BYTES);
    let (value, ms, truncated) = if head.is_empty() {
        (None, None, false)
    } else {
        (Some(head), ms, truncated || cut)
    };
    c.execute(
        "UPDATE messages SET reasoning = ?2, reasoning_ms = ?3, reasoning_truncated = ?4 WHERE id = ?1",
        params![msg, value, ms, i64::from(truncated)],
    )?;
    Ok(())
}

/// Writes the text of a running answer (a periodic flush).
pub fn flush_answer(c: &Connection, msg: i64, content: &str) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE messages SET content = ?2 WHERE id = ?1 AND status = 'streaming'",
        params![msg, content],
    )?;
    Ok(())
}

/// Final state of an answer.
pub fn finish_answer(
    c: &Connection,
    msg: i64,
    content: &str,
    status: &str,
    error_code: Option<&str>,
    truncated: bool,
    now: i64,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE messages SET content = ?2, status = ?3, error_code = ?4, truncated = ?5, finished_at = ?6
         WHERE id = ?1",
        params![msg, content, status, error_code, i64::from(truncated), now],
    )?;
    Ok(())
}

/// Deletes one chat of the user; returns the number of chats deleted (0 or 1).
pub fn delete_chat(c: &Connection, user: i64, chat: i64) -> rusqlite::Result<usize> {
    c.execute(
        "DELETE FROM chats WHERE id = ?1 AND user_id = ?2",
        [chat, user],
    )
}

pub fn delete_all_chats(c: &Connection, user: i64) -> rusqlite::Result<usize> {
    c.execute("DELETE FROM chats WHERE user_id = ?1", [user])
}

/// The user's memories, newest first.
pub fn list_memories(c: &Connection, user: i64) -> rusqlite::Result<Vec<MemoryRow>> {
    let mut st = c.prepare(
        "SELECT id, content, source, created_at, updated_at FROM memories WHERE user_id = ?1
         ORDER BY id DESC",
    )?;
    let rows = st.query_map([user], |r| {
        Ok(MemoryRow {
            id: r.get(0)?,
            content: r.get(1)?,
            source: r.get(2)?,
            created_at: r.get(3)?,
            updated_at: r.get(4)?,
        })
    })?;
    rows.collect()
}

/// Memory texts for the system prompt, newest-updated first.
pub fn memory_texts(c: &Connection, user: i64) -> rusqlite::Result<Vec<String>> {
    let mut st = c.prepare(
        "SELECT content FROM memories WHERE user_id = ?1 ORDER BY updated_at DESC, id DESC",
    )?;
    let rows = st.query_map([user], |r| r.get(0))?;
    rows.collect()
}

pub fn memory(c: &Connection, user: i64, id: i64) -> rusqlite::Result<Option<MemoryRow>> {
    c.query_row(
        "SELECT id, content, source, created_at, updated_at FROM memories WHERE id = ?1 AND user_id = ?2",
        [id, user],
        |r| {
            Ok(MemoryRow {
                id: r.get(0)?,
                content: r.get(1)?,
                source: r.get(2)?,
                created_at: r.get(3)?,
                updated_at: r.get(4)?,
            })
        },
    )
    .optional()
}

pub fn add_memory(
    c: &Connection,
    user: i64,
    content: &str,
    source: &str,
    now: i64,
) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO memories (user_id, content, source, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
        params![user, content, source, now],
    )?;
    Ok(c.last_insert_rowid())
}

pub fn update_memory(
    c: &Connection,
    user: i64,
    id: i64,
    content: &str,
    now: i64,
) -> rusqlite::Result<bool> {
    Ok(c.execute(
        "UPDATE memories SET content = ?3, updated_at = ?4 WHERE id = ?1 AND user_id = ?2",
        params![id, user, content, now],
    )? > 0)
}

pub fn delete_memory(c: &Connection, user: i64, id: i64) -> rusqlite::Result<usize> {
    c.execute(
        "DELETE FROM memories WHERE id = ?1 AND user_id = ?2",
        [id, user],
    )
}

pub fn delete_all_memories(c: &Connection, user: i64) -> rusqlite::Result<usize> {
    c.execute("DELETE FROM memories WHERE user_id = ?1", [user])
}

/// A user message of one of the user's chats (AI Chat messages only; the user messages of hints, explanations
/// and summaries are refused): (chat id, text).
pub fn user_message(
    c: &Connection,
    user: i64,
    msg: i64,
) -> rusqlite::Result<Option<(i64, String)>> {
    c.query_row(
        "SELECT m.chat_id, m.content FROM messages m JOIN chats c ON c.id = m.chat_id
         WHERE m.id = ?1 AND c.user_id = ?2 AND m.role = 'user' AND m.kind = 'chat'",
        [msg, user],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

// ---------------------------------------------------------------------------------------------------------------------
// Study chats (v0.9): one chat per practice session, exam and record summary; hints, explanations and summaries are
// messages of those chats.

/// Chat id of a user's chat with this kind and ref (`idx_chats_ref`, covering).
pub const SQL_CHAT_BY_REF: &str =
    "SELECT id FROM chats WHERE user_id = ?1 AND kind = ?2 AND ref = ?3";
/// Newest assistant message of a hint / explanation key, whatever its status (`idx_messages_study`, no sort).
pub const SQL_STUDY_LATEST: &str =
    "SELECT id FROM messages WHERE chat_id = ?1 AND question_id = ?2 \
     AND kind = ?3 AND source = ?4 AND role = 'assistant' ORDER BY id DESC LIMIT 1";
/// Newest finished, non-empty assistant message of a hint / explanation key.
pub const SQL_STUDY_LAST_GOOD: &str =
    "SELECT id FROM messages WHERE chat_id = ?1 AND question_id = ?2 \
     AND kind = ?3 AND source = ?4 AND role = 'assistant' AND status = 'done' AND content <> '' \
     ORDER BY id DESC LIMIT 1";
/// Newest assistant summary of a chat (`idx_messages_chat`, backwards).
pub const SQL_SUMMARY_LATEST: &str = "SELECT id FROM messages WHERE chat_id = ?1 \
     AND role = 'assistant' AND kind = 'summary' ORDER BY id DESC LIMIT 1";
/// Newest finished, non-empty assistant summary of a chat.
pub const SQL_SUMMARY_LAST_GOOD: &str = "SELECT id FROM messages WHERE chat_id = ?1 \
     AND role = 'assistant' AND kind = 'summary' AND status = 'done' AND content <> '' \
     ORDER BY id DESC LIMIT 1";

/// The id of the user's chat with this kind and ref, or None.
pub fn chat_by_ref(
    c: &Connection,
    user: i64,
    kind: &str,
    chat_ref: &str,
) -> rusqlite::Result<Option<i64>> {
    c.query_row(SQL_CHAT_BY_REF, params![user, kind, chat_ref], |r| r.get(0))
        .optional()
}

/// Everything the write transaction of a hint / explanation / summary ask needs.
pub struct StudyInsert<'a> {
    /// "practice" | "exam" | "summary"
    pub chat_kind: &'a str,
    /// Session key, or "r<recordId>@<at>"
    pub chat_ref: &'a str,
    /// Stored only when the chat is created
    pub title: &'a str,
    /// Written to `chats.record_id` when it is NULL
    pub record_id: Option<i64>,
    /// Message kind: "hint" | "explain" | "summary"
    pub kind: &'a str,
    pub question_id: Option<&'a str>,
    pub question_no: Option<i64>,
    /// "practice" | "result" | None (summary)
    pub source: Option<&'a str>,
    /// "en" | "zh-CN" | "ja"
    pub lang: &'a str,
    /// Summary only
    pub sample: Option<StudySample>,
    /// The stored user message
    pub user_content: &'a str,
    pub model: &'a str,
    pub now: i64,
    pub chats_per_user: i64,
    pub messages_per_chat: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudyIds {
    pub chat: i64,
    pub title: String,
    pub new_chat: bool,
    /// `chats.record_id` after the update
    pub record_id: Option<i64>,
    pub user_msg: i64,
    pub answer: i64,
    /// 1 + the assistant answers the chat already held for the same key (question / kind / source, or the summary),
    /// whatever their status: above 1 this ask regenerates it (a stopped, failed or withdrawn answer may still sit in the
    /// gateway cache)
    pub attempt: u32,
}

#[derive(Debug)]
pub enum StudyError {
    /// The user already has this many chats (and the chat would be new)
    ChatLimit(i64),
    /// The chat already holds this many messages
    MessageLimit(i64),
    /// Another answer of the user already runs in this chat
    Busy,
    Db(rusqlite::Error),
}

impl From<rusqlite::Error> for StudyError {
    fn from(e: rusqlite::Error) -> Self {
        StudyError::Db(e)
    }
}

fn insert_study_message(
    c: &Connection,
    chat: i64,
    role: &str,
    content: &str,
    status: &str,
    model: Option<&str>,
    s: &StudyInsert<'_>,
) -> rusqlite::Result<i64> {
    // appended at the viewed end of the chat, which then moves to the new message
    let parent = leaf_of(c, chat)?;
    c.execute(
        "INSERT INTO messages (chat_id, role, content, status, model, created_at, finished_at, free, kind,
                               question_id, question_no, source, lang, sample_used, sample_usable, sample_wrong,
                               parent_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, CASE WHEN ?4 = 'streaming' THEN NULL ELSE ?6 END, 0, ?7,
                 ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            chat,
            role,
            content,
            status,
            model,
            s.now,
            s.kind,
            s.question_id,
            s.question_no,
            s.source,
            s.lang,
            s.sample.map(|x| x.used),
            s.sample.map(|x| x.usable),
            s.sample.map(|x| x.wrong),
            parent,
        ],
    )?;
    let id = c.last_insert_rowid();
    set_leaf(c, chat, id)?;
    Ok(id)
}

/// The write transaction of a hint / explanation / summary ask (run it inside `Store::write`; an `Err` rolls
/// everything back): finds or creates the chat, binds it to the running answer through `bind` (in both branches, before
/// any message is written), checks the message limit, sets `chats.record_id` once, writes the question / answer pair.
pub fn prepare_study(
    c: &Connection,
    who: &Identity,
    s: &StudyInsert<'_>,
    bind: &mut dyn FnMut(i64) -> bool,
) -> Result<StudyIds, StudyError> {
    ensure_user(c, who, s.now)?;
    let (chat, new_chat) = match chat_by_ref(c, who.id, s.chat_kind, s.chat_ref)? {
        Some(id) => (id, false),
        None => {
            if count_chats(c, who.id)? >= s.chats_per_user {
                return Err(StudyError::ChatLimit(s.chats_per_user));
            }
            c.execute(
                "INSERT INTO chats (user_id, title, created_at, updated_at, kind, ref, record_id)
                 VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6)
                 ON CONFLICT(user_id, kind, ref) WHERE ref IS NOT NULL DO NOTHING",
                params![who.id, s.title, s.now, s.chat_kind, s.chat_ref, s.record_id],
            )?;
            match chat_by_ref(c, who.id, s.chat_kind, s.chat_ref)? {
                Some(id) => (id, true),
                None => return Err(StudyError::Db(rusqlite::Error::QueryReturnedNoRows)),
            }
        }
    };
    if !bind(chat) {
        return Err(StudyError::Busy);
    }
    if count_messages(c, chat)? + 2 > s.messages_per_chat {
        return Err(StudyError::MessageLimit(s.messages_per_chat));
    }
    if let Some(record) = s.record_id {
        c.execute(
            "UPDATE chats SET record_id = ?2 WHERE id = ?1 AND record_id IS NULL",
            params![chat, record],
        )?;
    }
    let held: i64 = if s.kind == "summary" {
        c.query_row(
            "SELECT COUNT(*) FROM messages WHERE chat_id = ?1 AND role = 'assistant' AND kind = 'summary'",
            [chat],
            |r| r.get(0),
        )?
    } else {
        c.query_row(
            "SELECT COUNT(*) FROM messages
             WHERE chat_id = ?1 AND question_id = ?2 AND kind = ?3 AND source = ?4 AND role = 'assistant'",
            params![chat, s.question_id, s.kind, s.source],
            |r| r.get(0),
        )?
    };
    let attempt = u32::try_from(held).unwrap_or(u32::MAX).saturating_add(1);
    let user_msg = insert_study_message(c, chat, "user", s.user_content, "done", None, s)?;
    let answer = insert_study_message(c, chat, "assistant", "", "streaming", Some(s.model), s)?;
    touch_chat(c, chat, s.now)?;
    let (title, record_id): (String, Option<i64>) = c.query_row(
        "SELECT title, record_id FROM chats WHERE id = ?1",
        [chat],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(StudyIds {
        chat,
        title,
        new_chat,
        record_id,
        user_msg,
        answer,
        attempt,
    })
}

/// A saved hint / explanation / summary as the saved routes return it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerRow {
    pub message_id: i64,
    pub kind: String,
    pub question_id: Option<String>,
    pub question_no: Option<i64>,
    pub source: Option<String>,
    pub status: String,
    pub error_code: Option<String>,
    pub truncated: bool,
    pub content: String,
    pub lang: Option<String>,
    pub model: Option<String>,
    pub created_at: i64,
    pub finished_at: Option<i64>,
    pub sample: Option<StudySample>,
    /// The chat's `record_id`
    pub record_id: Option<i64>,
}

impl AnswerRow {
    /// `StudyAnswer`
    pub fn study_json(&self) -> Value {
        json!({
            "messageId": self.message_id,
            "kind": self.kind,
            "questionId": self.question_id,
            "questionNo": self.question_no,
            "source": self.source,
            "status": self.status,
            "error": self.error_code,
            "truncated": self.truncated,
            "content": self.content,
            "lang": self.lang,
            "model": self.model,
            "createdAt": self.created_at,
            "finishedAt": self.finished_at,
        })
    }

    /// `SummaryAnswer`
    pub fn summary_json(&self) -> Value {
        json!({
            "messageId": self.message_id,
            "recordId": self.record_id,
            "status": self.status,
            "error": self.error_code,
            "truncated": self.truncated,
            "content": self.content,
            "lang": self.lang,
            "model": self.model,
            "createdAt": self.created_at,
            "finishedAt": self.finished_at,
            "sample": self.sample.map(|s| s.to_json()),
        })
    }
}

/// The result of a lookup: the chat (if any), the newest answer and, when that one is not finished, the newest good one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub chat_id: Option<i64>,
    /// The chat's real title (the browser shows it in the "saved in AI Chat" tip)
    pub chat_title: Option<String>,
    pub latest: Option<AnswerRow>,
    pub last_good: Option<AnswerRow>,
}

impl Saved {
    /// `{"chatId", "chatTitle", "latest", "lastGood"}`; `summary` picks the `SummaryAnswer` shape.
    pub fn to_json(&self, summary: bool) -> Value {
        let one = |a: &Option<AnswerRow>| match a {
            None => Value::Null,
            Some(a) if summary => a.summary_json(),
            Some(a) => a.study_json(),
        };
        json!({
            "chatId": self.chat_id,
            "chatTitle": self.chat_title,
            "latest": one(&self.latest),
            "lastGood": one(&self.last_good),
        })
    }
}

fn answer_by_id(c: &Connection, id: i64) -> rusqlite::Result<AnswerRow> {
    c.query_row(
        "SELECT m.id, m.kind, m.question_id, m.question_no, m.source, m.status, m.error_code, m.truncated, m.content,
                m.lang, m.model, m.created_at, m.finished_at, m.sample_used, m.sample_usable, m.sample_wrong,
                c.record_id
         FROM messages m JOIN chats c ON c.id = m.chat_id WHERE m.id = ?1",
        [id],
        |r| {
            Ok(AnswerRow {
                message_id: r.get(0)?,
                kind: r.get(1)?,
                question_id: r.get(2)?,
                question_no: r.get(3)?,
                source: r.get(4)?,
                status: r.get(5)?,
                error_code: r.get(6)?,
                truncated: r.get::<_, i64>(7)? != 0,
                content: r.get(8)?,
                lang: r.get(9)?,
                model: r.get(10)?,
                created_at: r.get(11)?,
                finished_at: r.get(12)?,
                sample: sample_of(r.get(13)?, r.get(14)?, r.get(15)?),
                record_id: r.get(16)?,
            })
        },
    )
}

/// Latest answer and, only when it is not `done`, the newest good one. `extra` are the parameters after the chat id.
fn saved_with(
    c: &Connection,
    chat: i64,
    latest_sql: &str,
    good_sql: &str,
    extra: &[&str],
) -> rusqlite::Result<Saved> {
    let run = |sql: &str| -> rusqlite::Result<Option<i64>> {
        let mut args: Vec<&dyn rusqlite::ToSql> = vec![&chat];
        for e in extra {
            args.push(e);
        }
        c.query_row(sql, args.as_slice(), |r| r.get(0)).optional()
    };
    let latest = match run(latest_sql)? {
        Some(id) => Some(answer_by_id(c, id)?),
        None => None,
    };
    let last_good = match &latest {
        Some(l) if l.status != "done" => match run(good_sql)? {
            Some(id) => Some(answer_by_id(c, id)?),
            None => None,
        },
        _ => None,
    };
    let chat_title = c
        .query_row("SELECT title FROM chats WHERE id = ?1", [chat], |r| {
            r.get::<_, String>(0)
        })
        .optional()?;
    Ok(Saved {
        chat_id: Some(chat),
        chat_title,
        latest,
        last_good,
    })
}

const NO_SAVED: Saved = Saved {
    chat_id: None,
    chat_title: None,
    latest: None,
    last_good: None,
};

/// Hint / explanation of one question and place in a study chat (the chat is looked up by `user_id`).
pub fn study_saved(
    c: &Connection,
    user: i64,
    chat_kind: &str,
    chat_ref: &str,
    kind: &str,
    question_id: &str,
    source: &str,
) -> rusqlite::Result<Saved> {
    let Some(chat) = chat_by_ref(c, user, chat_kind, chat_ref)? else {
        return Ok(NO_SAVED);
    };
    saved_with(
        c,
        chat,
        SQL_STUDY_LATEST,
        SQL_STUDY_LAST_GOOD,
        &[question_id, kind, source],
    )
}

/// The summary of a record (chat kind 'summary').
pub fn summary_saved(c: &Connection, user: i64, chat_ref: &str) -> rusqlite::Result<Saved> {
    let Some(chat) = chat_by_ref(c, user, "summary", chat_ref)? else {
        return Ok(NO_SAVED);
    };
    saved_with(c, chat, SQL_SUMMARY_LATEST, SQL_SUMMARY_LAST_GOOD, &[])
}

/// The latest assessment of a user (`assess_latest`, one row per user).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssessLatest {
    pub content: String,
    pub lang: String,
    pub model: Option<String>,
    /// `YYYY-MM-DD HH:MM`, the newest record the assessment was made for
    pub latest_at: String,
    pub records: i64,
    pub answered: i64,
    /// `YYYY-MM-DD`, the ai-service local day it was written
    pub day: String,
    pub created_at: i64,
    /// Successful runs on `day` (the per-day limit counts them)
    pub runs: i64,
}

impl AssessLatest {
    /// The JSON the capsule reads. `model` is stripped for ordinary users by `hide::for_viewer`.
    pub fn to_json(&self) -> Value {
        json!({
            "text": self.content,
            "day": self.day,
            "createdAt": self.created_at,
            "latestAt": self.latest_at,
            "records": self.records,
            "answered": self.answered,
            "model": self.model,
        })
    }
}

/// The user's latest assessment, or None.
pub fn assess_latest(c: &Connection, user: i64) -> rusqlite::Result<Option<AssessLatest>> {
    c.query_row(
        "SELECT content, lang, model, latest_at, records, answered, day, created_at, runs
         FROM assess_latest WHERE user_id = ?1",
        [user],
        |r| {
            Ok(AssessLatest {
                content: r.get(0)?,
                lang: r.get(1)?,
                model: r.get(2)?,
                latest_at: r.get(3)?,
                records: r.get(4)?,
                answered: r.get(5)?,
                day: r.get(6)?,
                created_at: r.get(7)?,
                runs: r.get(8)?,
            })
        },
    )
    .optional()
}

/// Called when an answer finishes: when it is a non-empty `done` answer of an assessment chat (a `summary` chat whose ref
/// starts with `assess@`), it replaces the user's latest assessment. Failed, stopped and empty answers write nothing, so
/// they neither replace the old text nor use up the day. The numbers come from the stored user line the answer replies to.
/// Returns whether a row was written.
pub fn save_assess_latest(
    c: &Connection,
    msg: i64,
    status: &str,
    content: &str,
    model: &str,
    now: i64,
) -> rusqlite::Result<bool> {
    if status != "done" || content.trim().is_empty() {
        return Ok(false);
    }
    // the day is the one the request was accepted on (the chat ref `assess@<day>`), not the day the answer ends
    let row: Option<(i64, Option<String>, String, String)> = c
        .query_row(
            "SELECT ch.user_id,
                    COALESCE(
                      (SELECT u.content FROM messages u WHERE u.id = a.parent_id AND u.chat_id = a.chat_id AND u.role = 'user'),
                      (SELECT u.content FROM messages u WHERE u.chat_id = a.chat_id AND u.role = 'user' AND u.id < a.id
                       ORDER BY u.id DESC LIMIT 1)),
                    COALESCE(a.lang, ''),
                    substr(ch.ref, 8)
             FROM messages a JOIN chats ch ON ch.id = a.chat_id
             WHERE a.id = ?1 AND a.role = 'assistant' AND a.kind = 'summary'
               AND ch.kind = 'summary' AND ch.ref LIKE 'assess@%'",
            [msg],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((user, stored, lang, day)) = row else {
        return Ok(false);
    };
    let Some(line) = stored.as_deref().and_then(parse_assess_line) else {
        return Ok(false);
    };
    c.execute(
        "INSERT INTO assess_latest (user_id, content, lang, model, latest_at, records, answered, day, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(user_id) DO UPDATE SET content = excluded.content, lang = excluded.lang, model = excluded.model,
           latest_at = excluded.latest_at, records = excluded.records, answered = excluded.answered,
           runs = CASE WHEN assess_latest.day = excluded.day THEN assess_latest.runs + 1 ELSE 1 END,
           day = excluded.day, created_at = excluded.created_at",
        params![
            user,
            content,
            lang,
            model,
            line.latest_at,
            line.records,
            line.answered,
            day,
            now
        ],
    )?;
    Ok(true)
}

/// `RunningInfo` of an answer of the user, or None.
pub fn running_info(
    c: &Connection,
    user: i64,
    chat: i64,
    message: i64,
) -> rusqlite::Result<Option<Value>> {
    let sql = format!(
        "SELECT c.id, m.id, {CHAT_KIND_SQL}, {MSG_KIND_SQL}, m.question_id, m.question_no, m.source,
                CASE WHEN m.source = 'result' OR m.kind = 'summary' THEN c.record_id END
         FROM messages m JOIN chats c ON c.id = m.chat_id
         WHERE m.id = ?1 AND c.id = ?2 AND c.user_id = ?3"
    );
    c.query_row(&sql, params![message, chat, user], |r| {
        Ok(json!({
            "chatId": r.get::<_, i64>(0)?,
            "messageId": r.get::<_, i64>(1)?,
            "chatKind": r.get::<_, String>(2)?,
            "kind": r.get::<_, String>(3)?,
            "questionId": r.get::<_, Option<String>>(4)?,
            "questionNo": r.get::<_, Option<i64>>(5)?,
            "source": r.get::<_, Option<String>>(6)?,
            "recordId": r.get::<_, Option<i64>>(7)?,
        }))
    })
    .optional()
}
