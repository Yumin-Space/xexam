//! Shared study answers (`study_shared`, schema 7): one hint / explanation per question content, language and kind,
//! with no user id. Reads and writes take the caller's connection or transaction.
use rusqlite::{params, Connection, OptionalExtension};

/// The natural key of a shared answer. `content_hash` is 32 lower-case hex (computed by the study code).
#[derive(Debug, Clone, Copy)]
pub struct SharedKey<'a> {
    pub question_id: &'a str,
    pub content_hash: &'a str,
    /// 'hint' | 'explain'
    pub kind: &'a str,
    /// 'en' | 'zh' | 'ja'
    pub lang: &'a str,
}

/// A stored shared answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedRow {
    pub id: i64,
    pub content: String,
    pub model: String,
    /// Connection NAME at call time
    pub connection: String,
    pub gen: i64,
    pub hits: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_hit_at: Option<i64>,
}

/// An answer to store.
#[derive(Debug, Clone)]
pub struct NewShared<'a> {
    pub content: &'a str,
    pub model: &'a str,
    pub connection: &'a str,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub now: i64,
}

/// The shared answer of a key, or None.
pub fn shared_get(c: &Connection, k: &SharedKey<'_>) -> rusqlite::Result<Option<SharedRow>> {
    c.query_row(
        "SELECT id, content, model, connection, gen, hits, created_at, updated_at, last_hit_at FROM study_shared
         WHERE question_id = ?1 AND content_hash = ?2 AND kind = ?3 AND lang = ?4",
        params![k.question_id, k.content_hash, k.kind, k.lang],
        |r| {
            Ok(SharedRow {
                id: r.get(0)?,
                content: r.get(1)?,
                model: r.get(2)?,
                connection: r.get(3)?,
                gen: r.get(4)?,
                hits: r.get(5)?,
                created_at: r.get(6)?,
                updated_at: r.get(7)?,
                last_hit_at: r.get(8)?,
            })
        },
    )
    .optional()
}

/// Counts a hit (`hits + 1`, `last_hit_at = now`). Best effort: the caller ignores a failure.
pub fn shared_hit(c: &Connection, id: i64, now: i64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE study_shared SET hits = hits + 1, last_hit_at = ?2 WHERE id = ?1",
        params![id, now],
    )?;
    Ok(())
}

/// Stores the first answer of a key. `false` = another answer won the race (nothing written).
pub fn shared_put_first(
    c: &Connection,
    k: &SharedKey<'_>,
    n: &NewShared<'_>,
) -> rusqlite::Result<bool> {
    Ok(c.execute(
        "INSERT INTO study_shared (question_id, content_hash, kind, lang, content, model, connection, tokens_in,
                                   tokens_out, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
         ON CONFLICT(question_id, content_hash, kind, lang) DO NOTHING",
        params![
            k.question_id,
            k.content_hash,
            k.kind,
            k.lang,
            n.content,
            n.model,
            n.connection,
            n.tokens_in,
            n.tokens_out,
            n.now
        ],
    )? > 0)
}

/// Replaces the answer of a key (a regeneration), or stores it as the first one; `gen` counts the replacements and
/// `hits` is kept.
pub fn shared_replace(
    c: &Connection,
    k: &SharedKey<'_>,
    n: &NewShared<'_>,
) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO study_shared (question_id, content_hash, kind, lang, content, model, connection, tokens_in,
                                   tokens_out, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
         ON CONFLICT(question_id, content_hash, kind, lang) DO UPDATE SET content = excluded.content,
             model = excluded.model, connection = excluded.connection, tokens_in = excluded.tokens_in,
             tokens_out = excluded.tokens_out, gen = gen + 1, updated_at = excluded.updated_at",
        params![
            k.question_id,
            k.content_hash,
            k.kind,
            k.lang,
            n.content,
            n.model,
            n.connection,
            n.tokens_in,
            n.tokens_out,
            n.now
        ],
    )?;
    Ok(())
}

/// Deletes every shared answer; returns how many.
pub fn shared_clear(c: &Connection) -> rusqlite::Result<i64> {
    Ok(i64::try_from(c.execute("DELETE FROM study_shared", [])?).unwrap_or(i64::MAX))
}

/// `(rows, hits, rows hit today)`: the number of stored answers, the sum of their hits, and how many answers were hit
/// at or after `today_start` (an answer counts once however often it was hit).
pub fn shared_stats(c: &Connection, today_start: i64) -> rusqlite::Result<(i64, i64, i64)> {
    c.query_row(
        "SELECT COUNT(*), COALESCE(SUM(hits), 0),
                COALESCE(SUM(CASE WHEN last_hit_at >= ?1 THEN 1 ELSE 0 END), 0)
         FROM study_shared",
        [today_start],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
}

/// Retention: deletes the answers last used before `older_than` (`last_hit_at`, else `updated_at`), then the
/// least-recently used ones beyond `cap` rows; returns how many were deleted.
pub fn shared_prune(c: &Connection, older_than: i64, cap: i64) -> rusqlite::Result<i64> {
    let old = c.execute(
        "DELETE FROM study_shared WHERE COALESCE(last_hit_at, updated_at) < ?1",
        [older_than],
    )?;
    let over = c.execute(
        "DELETE FROM study_shared WHERE id IN (
           SELECT id FROM study_shared ORDER BY COALESCE(last_hit_at, updated_at), id
           LIMIT MAX(0, (SELECT COUNT(*) FROM study_shared) - ?1))",
        [cap.max(0)],
    )?;
    Ok(i64::try_from(old + over).unwrap_or(i64::MAX))
}
