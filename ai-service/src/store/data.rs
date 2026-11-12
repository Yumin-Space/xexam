//! Users, chats, messages and memories. Every query of user content filters by the owner's id.
use crate::identity::Identity;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

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

/// Ids of the users whose AI+ function is suspended, lowest first, at most `limit`.
pub fn suspended_ids(c: &Connection, limit: usize) -> rusqlite::Result<Vec<i64>> {
    let mut st =
        c.prepare("SELECT id FROM users WHERE function_enabled = 0 ORDER BY id LIMIT ?1")?;
    let rows = st.query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |r| r.get(0))?;
    rows.collect()
}

const CHAT_COLS: &str = "c.id, c.title, c.created_at, c.updated_at,
     (SELECT COUNT(*) FROM messages m WHERE m.chat_id = c.id), c.kind, c.record_id";

fn chat_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChatRow> {
    Ok(ChatRow {
        id: r.get(0)?,
        title: r.get(1)?,
        created_at: r.get(2)?,
        updated_at: r.get(3)?,
        message_count: r.get(4)?,
        kind: r.get(5)?,
        record_id: r.get(6)?,
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

/// The messages of a chat, oldest first. The caller has checked that the chat is the user's. `record_id` is the
/// chat's, shown only for result-page answers and summaries.
pub fn messages(c: &Connection, chat: i64) -> rusqlite::Result<Vec<MessageRow>> {
    let mut st = c.prepare(
        "SELECT m.id, m.role, m.content, m.status, m.error_code, m.truncated, m.model, m.created_at, m.free,
                m.kind, m.question_id, m.question_no, m.source, m.lang,
                CASE WHEN m.source = 'result' OR m.kind = 'summary' THEN c.record_id END,
                m.sample_used, m.sample_usable, m.sample_wrong
         FROM messages m JOIN chats c ON c.id = m.chat_id WHERE m.chat_id = ?1 ORDER BY m.id",
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
            kind: r.get(9)?,
            question_id: r.get(10)?,
            question_no: r.get(11)?,
            source: r.get(12)?,
            lang: r.get(13)?,
            record_id: r.get(14)?,
            sample: sample_of(r.get(15)?, r.get(16)?, r.get(17)?),
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
    c.execute(
        "INSERT INTO messages (chat_id, role, content, status, model, created_at, finished_at, free, kind,
                               question_id, question_no, source, lang, sample_used, sample_usable, sample_wrong)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, CASE WHEN ?4 = 'streaming' THEN NULL ELSE ?6 END, 0, ?7,
                 ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
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
        ],
    )?;
    Ok(c.last_insert_rowid())
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
    pub latest: Option<AnswerRow>,
    pub last_good: Option<AnswerRow>,
}

impl Saved {
    /// `{"chatId", "latest", "lastGood"}`; `summary` picks the `SummaryAnswer` shape.
    pub fn to_json(&self, summary: bool) -> Value {
        let one = |a: &Option<AnswerRow>| match a {
            None => Value::Null,
            Some(a) if summary => a.summary_json(),
            Some(a) => a.study_json(),
        };
        json!({
            "chatId": self.chat_id,
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
    Ok(Saved {
        chat_id: Some(chat),
        latest,
        last_good,
    })
}

const NO_SAVED: Saved = Saved {
    chat_id: None,
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

/// `RunningInfo` of an answer of the user, or None.
pub fn running_info(
    c: &Connection,
    user: i64,
    chat: i64,
    message: i64,
) -> rusqlite::Result<Option<Value>> {
    c.query_row(
        "SELECT c.id, m.id, c.kind, m.kind, m.question_id, m.question_no, m.source,
                CASE WHEN m.source = 'result' OR m.kind = 'summary' THEN c.record_id END
         FROM messages m JOIN chats c ON c.id = m.chat_id
         WHERE m.id = ?1 AND c.id = ?2 AND c.user_id = ?3",
        params![message, chat, user],
        |r| {
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
        },
    )
    .optional()
}
