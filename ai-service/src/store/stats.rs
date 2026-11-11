//! Usage rows and statistics, the audit outbox, retention.
use rusqlite::{params, Connection};

/// One model call (never content, keys or URLs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageRow {
    pub user_id: Option<i64>,
    pub at: i64,
    pub kind: &'static str,
    pub provider: &'static str,
    pub model: String,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub tokens_cached: Option<i64>,
    pub latency_ms: i64,
    pub first_byte_ms: Option<i64>,
    pub status: &'static str,
    /// `ai.up.*` code without the prefix
    pub error_code: Option<String>,
}

/// Inserts a usage row. A user id that no longer exists (deleted meanwhile) is stored as NULL.
pub fn insert_usage(c: &Connection, u: &UsageRow) -> rusqlite::Result<()> {
    insert_usage_purpose(c, u, None)
}

/// `insert_usage` with the feature of a study generation (`hint` | `explain` | `summary`); None for AI Chat and every
/// non-chat call.
pub fn insert_usage_purpose(
    c: &Connection,
    u: &UsageRow,
    purpose: Option<&str>,
) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO usage (user_id, at, day, kind, provider, model, tokens_in, tokens_out, latency_ms, first_byte_ms,
                            status, error_code, tokens_cached, purpose)
         VALUES ((SELECT id FROM users WHERE id = ?1), ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            u.user_id,
            u.at,
            crate::util::local_date(u.at),
            u.kind,
            u.provider,
            u.model,
            u.tokens_in,
            u.tokens_out,
            u.latency_ms,
            u.first_byte_ms,
            u.status,
            u.error_code,
            u.tokens_cached,
            purpose,
        ],
    )?;
    Ok(())
}

/// Sums over a set of usage rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sums {
    pub calls: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub errors: i64,
    pub tokens_cached: i64,
}

const SUMS: &str = "COUNT(*), COALESCE(SUM(tokens_in), 0), COALESCE(SUM(tokens_out), 0),
     COALESCE(SUM(CASE WHEN status = 'error' THEN 1 ELSE 0 END), 0), COALESCE(SUM(tokens_cached), 0)";

fn sums_at(r: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<Sums> {
    Ok(Sums {
        calls: r.get(i)?,
        tokens_in: r.get(i + 1)?,
        tokens_out: r.get(i + 2)?,
        errors: r.get(i + 3)?,
        tokens_cached: r.get(i + 4)?,
    })
}

pub fn totals(c: &Connection, from: &str, to: &str) -> rusqlite::Result<Sums> {
    c.query_row(
        &format!("SELECT {SUMS} FROM usage WHERE day BETWEEN ?1 AND ?2"),
        [from, to],
        |r| sums_at(r, 0),
    )
}

/// One row of the per-user statistics: user id (None = deleted users), username, sums, time of the last call.
pub type UserUsage = (Option<i64>, Option<String>, Sums, i64);

/// Per user, sorted by calls, most first.
pub fn by_user(c: &Connection, from: &str, to: &str) -> rusqlite::Result<Vec<UserUsage>> {
    let mut st = c.prepare(&format!(
        "SELECT u.user_id, (SELECT username FROM users WHERE id = u.user_id), {SUMS}, MAX(u.at)
         FROM usage u WHERE u.day BETWEEN ?1 AND ?2 GROUP BY u.user_id
         ORDER BY COUNT(*) DESC, u.user_id IS NULL, u.user_id"
    ))?;
    let rows = st.query_map([from, to], |r| {
        Ok((r.get(0)?, r.get(1)?, sums_at(r, 2)?, r.get(7)?))
    })?;
    rows.collect()
}

pub fn by_model(c: &Connection, from: &str, to: &str) -> rusqlite::Result<Vec<(String, Sums)>> {
    let mut st = c.prepare(&format!(
        "SELECT model, {SUMS} FROM usage WHERE day BETWEEN ?1 AND ?2 GROUP BY model ORDER BY COUNT(*) DESC, model"
    ))?;
    let rows = st.query_map([from, to], |r| Ok((r.get(0)?, sums_at(r, 1)?)))?;
    rows.collect()
}

/// The four features, in the order `by_purpose` returns them.
pub const PURPOSES: [&str; 4] = ["chat", "hint", "explain", "summary"];

/// Chat calls (`kind = 'chat'`) of the period per feature (`COALESCE(purpose, 'chat')`); always the four rows in the
/// order chat, hint, explain, summary, zeros kept.
pub fn by_purpose(
    c: &Connection,
    from: &str,
    to: &str,
) -> rusqlite::Result<Vec<(&'static str, Sums)>> {
    let mut st = c.prepare(&format!(
        "SELECT COALESCE(purpose, 'chat') AS p, {SUMS} FROM usage
         WHERE kind = 'chat' AND day BETWEEN ?1 AND ?2 GROUP BY p"
    ))?;
    let rows = st.query_map([from, to], |r| Ok((r.get::<_, String>(0)?, sums_at(r, 1)?)))?;
    let found: Vec<(String, Sums)> = rows.collect::<rusqlite::Result<_>>()?;
    Ok(PURPOSES
        .iter()
        .map(|p| {
            let s = found
                .iter()
                .find(|(k, _)| k == p)
                .map(|(_, s)| s.clone())
                .unwrap_or_default();
            (*p, s)
        })
        .collect())
}

/// Per period key: the first `len` characters of `day` (10 = day, 7 = month).
pub fn by_period(
    c: &Connection,
    from: &str,
    to: &str,
    len: i64,
) -> rusqlite::Result<Vec<(String, Sums)>> {
    let mut st = c.prepare(&format!(
        "SELECT substr(day, 1, ?3) AS p, {SUMS} FROM usage WHERE day BETWEEN ?1 AND ?2 GROUP BY p ORDER BY p"
    ))?;
    let rows = st.query_map(params![from, to, len], |r| Ok((r.get(0)?, sums_at(r, 1)?)))?;
    rows.collect()
}

/// Per-user counts for the administrators' list: chats, messages, memories, last usage time, all-time tokens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserCounts {
    pub chats: i64,
    pub messages: i64,
    pub memories: i64,
    pub last_used_at: Option<i64>,
    pub tokens_in: i64,
    pub tokens_out: i64,
}

pub fn user_counts(c: &Connection, user: i64) -> rusqlite::Result<UserCounts> {
    c.query_row(
        "SELECT (SELECT COUNT(*) FROM chats WHERE user_id = ?1),
                (SELECT COUNT(*) FROM messages m JOIN chats c ON c.id = m.chat_id WHERE c.user_id = ?1),
                (SELECT COUNT(*) FROM memories WHERE user_id = ?1),
                (SELECT MAX(at) FROM usage WHERE user_id = ?1),
                (SELECT COALESCE(SUM(tokens_in), 0) FROM usage WHERE user_id = ?1),
                (SELECT COALESCE(SUM(tokens_out), 0) FROM usage WHERE user_id = ?1)",
        [user],
        |r| {
            Ok(UserCounts {
                chats: r.get(0)?,
                messages: r.get(1)?,
                memories: r.get(2)?,
                last_used_at: r.get(3)?,
                tokens_in: r.get(4)?,
                tokens_out: r.get(5)?,
            })
        },
    )
}

/// Inserts an audit line into the outbox; returns its id.
pub fn outbox_add(c: &Connection, at: i64, message: &str) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO audit_outbox (at, message) VALUES (?1, ?2)",
        params![at, message],
    )?;
    Ok(c.last_insert_rowid())
}

/// The oldest undelivered audit lines.
pub fn outbox_pending(c: &Connection, limit: i64) -> rusqlite::Result<Vec<(i64, String)>> {
    let mut st = c.prepare("SELECT id, message FROM audit_outbox ORDER BY id LIMIT ?1")?;
    let rows = st.query_map([limit], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

pub fn outbox_remove(c: &Connection, ids: &[i64]) -> rusqlite::Result<()> {
    for id in ids {
        c.execute("DELETE FROM audit_outbox WHERE id = ?1", [id])?;
    }
    Ok(())
}

/// Retention: deletes up to `limit` chats last updated before `cutoff` (oldest first), except the running ones; returns
/// how many.
pub fn sweep_chats(
    c: &Connection,
    cutoff: i64,
    running: &[i64],
    limit: i64,
) -> rusqlite::Result<usize> {
    let keep: Vec<String> = running.iter().map(i64::to_string).collect();
    let sql = if keep.is_empty() {
        "DELETE FROM chats WHERE id IN
           (SELECT id FROM chats WHERE updated_at < ?1 ORDER BY updated_at LIMIT ?2)"
            .to_string()
    } else {
        format!(
            "DELETE FROM chats WHERE id IN
               (SELECT id FROM chats WHERE updated_at < ?1 AND id NOT IN ({}) ORDER BY updated_at LIMIT ?2)",
            keep.join(",")
        )
    };
    c.execute(&sql, params![cutoff, limit])
}

/// Finished tasks older than `cutoff`.
pub fn sweep_tasks(c: &Connection, cutoff: i64) -> rusqlite::Result<usize> {
    c.execute(
        "DELETE FROM tasks WHERE state IN ('done', 'failed', 'cancelled') AND updated_at < ?1",
        [cutoff],
    )
}
