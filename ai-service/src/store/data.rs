//! Users, chats, messages and memories. Every query of user content filters by the owner's id.
use crate::identity::Identity;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

/// A chat as listed (`ChatSummary` without `running`, which comes from the registry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRow {
    pub id: i64,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub message_count: i64,
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

/// Ids of the users whose AI+ function is suspended, lowest first, at most `limit`.
pub fn suspended_ids(c: &Connection, limit: usize) -> rusqlite::Result<Vec<i64>> {
    let mut st =
        c.prepare("SELECT id FROM users WHERE function_enabled = 0 ORDER BY id LIMIT ?1")?;
    let rows = st.query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |r| r.get(0))?;
    rows.collect()
}

const CHAT_COLS: &str = "c.id, c.title, c.created_at, c.updated_at,
     (SELECT COUNT(*) FROM messages m WHERE m.chat_id = c.id)";

fn chat_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChatRow> {
    Ok(ChatRow {
        id: r.get(0)?,
        title: r.get(1)?,
        created_at: r.get(2)?,
        updated_at: r.get(3)?,
        message_count: r.get(4)?,
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

/// The messages of a chat, oldest first. The caller has checked that the chat is the user's.
pub fn messages(c: &Connection, chat: i64) -> rusqlite::Result<Vec<MessageRow>> {
    let mut st = c.prepare(
        "SELECT id, role, content, status, error_code, truncated, model, created_at, free
         FROM messages WHERE chat_id = ?1 ORDER BY id",
    )?;
    let rows = st.query_map([chat], |r| {
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
        })
    })?;
    rows.collect()
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
}

/// Inserts a message; returns its id.
pub fn insert_message(c: &Connection, m: &NewMessage<'_>) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO messages (chat_id, role, content, status, model, created_at, finished_at, free)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, CASE WHEN ?4 = 'streaming' THEN NULL ELSE ?6 END, ?7)",
        params![
            m.chat,
            m.role,
            m.content,
            m.status,
            m.model,
            m.now,
            i64::from(m.free)
        ],
    )?;
    Ok(c.last_insert_rowid())
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
                finished_at = NULL, free = ?3
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

/// A user message of one of the user's chats: (chat id, text).
pub fn user_message(
    c: &Connection,
    user: i64,
    msg: i64,
) -> rusqlite::Result<Option<(i64, String)>> {
    c.query_row(
        "SELECT m.chat_id, m.content FROM messages m JOIN chats c ON c.id = m.chat_id
         WHERE m.id = ?1 AND c.user_id = ?2 AND m.role = 'user'",
        [msg, user],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}
