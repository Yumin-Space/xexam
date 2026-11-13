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
    pub tokens_cache_write: Option<i64>,
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
                            status, error_code, tokens_cached, purpose, tokens_cache_write)
         VALUES ((SELECT id FROM users WHERE id = ?1), ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
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
            u.tokens_cache_write,
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
    pub tokens_cache_write: i64,
}

/// Every model call of a statistics period: the `usage` rows and the `gen_calls` rows of generation and import jobs
/// (`UNION ALL`; the per-user call caps use `usage` only: import and generation calls are outside them, A-OWNER-56).
const ALL_CALLS: &str =
    "(SELECT user_id, at, day, model, tokens_in, tokens_out, tokens_cached, status, tokens_cache_write FROM usage
     UNION ALL
     SELECT user_id, at, day, model, tokens_in, tokens_out, tokens_cached, status, tokens_cache_write FROM gen_calls)";

const SUMS: &str = "COUNT(*), COALESCE(SUM(tokens_in), 0), COALESCE(SUM(tokens_out), 0),
     COALESCE(SUM(CASE WHEN status = 'error' THEN 1 ELSE 0 END), 0), COALESCE(SUM(tokens_cached), 0),
     COALESCE(SUM(tokens_cache_write), 0)";

fn sums_at(r: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<Sums> {
    Ok(Sums {
        calls: r.get(i)?,
        tokens_in: r.get(i + 1)?,
        tokens_out: r.get(i + 2)?,
        errors: r.get(i + 3)?,
        tokens_cached: r.get(i + 4)?,
        tokens_cache_write: r.get(i + 5)?,
    })
}

pub fn totals(c: &Connection, from: &str, to: &str) -> rusqlite::Result<Sums> {
    c.query_row(
        &format!("SELECT {SUMS} FROM {ALL_CALLS} WHERE day BETWEEN ?1 AND ?2"),
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
         FROM {ALL_CALLS} u WHERE u.day BETWEEN ?1 AND ?2 GROUP BY u.user_id
         ORDER BY COUNT(*) DESC, u.user_id IS NULL, u.user_id"
    ))?;
    let rows = st.query_map([from, to], |r| {
        Ok((r.get(0)?, r.get(1)?, sums_at(r, 2)?, r.get(8)?))
    })?;
    rows.collect()
}

/// Calls of the period per model. The calls that use no chosen model for a feature, the administrators' model list and
/// connection tests (`usage` kinds `models` and `verify`), are one row with the empty model name (the "other" row), so
/// the rows add up to the total calls.
pub fn by_model(c: &Connection, from: &str, to: &str) -> rusqlite::Result<Vec<(String, Sums)>> {
    let mut st = c.prepare(&format!(
        "SELECT m, {SUMS} FROM (
           SELECT CASE WHEN kind IN ('verify', 'models') THEN '' ELSE model END AS m,
                  day, tokens_in, tokens_out, tokens_cached, status, tokens_cache_write FROM usage
           UNION ALL
           SELECT model AS m, day, tokens_in, tokens_out, tokens_cached, status, tokens_cache_write FROM gen_calls)
         WHERE day BETWEEN ?1 AND ?2 GROUP BY m ORDER BY COUNT(*) DESC, m"
    ))?;
    let rows = st.query_map([from, to], |r| Ok((r.get(0)?, sums_at(r, 1)?)))?;
    rows.collect()
}

/// The features, in the order `by_purpose` returns them. `generate` = AI bank generation calls; `import` = the AI file
/// import's calls (`gen_calls` purposes `import` and `recognise`); `other` = the calls that belong to no feature (the
/// administrators' model list and connection tests).
pub const PURPOSES: [&str; 7] = [
    "chat", "hint", "explain", "summary", "generate", "import", "other",
];

/// Calls of the period per feature, always the seven rows in the order chat, hint, explain, summary, generate, import,
/// other, zeros kept: the `usage` rows by `COALESCE(purpose, 'chat')` (memory suggestions are AI Chat's; the model list and
/// connection tests are `other`), then the `gen_calls` rows. The rows add up to the total calls.
pub fn by_purpose(
    c: &Connection,
    from: &str,
    to: &str,
) -> rusqlite::Result<Vec<(&'static str, Sums)>> {
    let mut st = c.prepare(&format!(
        "SELECT CASE WHEN kind IN ('verify', 'models') THEN 'other' ELSE COALESCE(purpose, 'chat') END AS p, {SUMS}
         FROM usage WHERE day BETWEEN ?1 AND ?2 GROUP BY p"
    ))?;
    let rows = st.query_map([from, to], |r| Ok((r.get::<_, String>(0)?, sums_at(r, 1)?)))?;
    let mut found: Vec<(String, Sums)> = rows.collect::<rusqlite::Result<_>>()?;
    let mut st = c.prepare(&format!(
        "SELECT CASE WHEN purpose = 'generate' THEN 'generate' ELSE 'import' END AS p, {SUMS} FROM gen_calls
         WHERE day BETWEEN ?1 AND ?2 GROUP BY p"
    ))?;
    let rows = st.query_map([from, to], |r| Ok((r.get::<_, String>(0)?, sums_at(r, 1)?)))?;
    found.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
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
        "SELECT substr(day, 1, ?3) AS p, {SUMS} FROM {ALL_CALLS} WHERE day BETWEEN ?1 AND ?2 GROUP BY p ORDER BY p"
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
                u.last_at, u.tin, u.tout
         FROM (SELECT MAX(at) AS last_at, COALESCE(SUM(tokens_in), 0) AS tin, COALESCE(SUM(tokens_out), 0) AS tout
               FROM usage WHERE user_id = ?1) u",
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

/// The usage kinds that count as an AI model call for the per-user caps (as an SQL list); the administrator's connection
/// test (`verify`) and model detection (`models`) are not counted. One constant for every counting query.
pub const COUNTED_KINDS: &str = "('chat', 'memory')";

/// The bounds of a counting read: `(earliest start any window still needs, first calendar day to scan)`. The earliest
/// start is the hour window's edge (`now - 3599`) or today's 0:00, whichever is earlier; the day scan begins one calendar
/// day before it, so rows written under another time zone (a date up to one day off) are still found. The date only
/// narrows the index range; the exact filter is the start time.
fn count_bounds(now: i64, day_start: i64) -> (i64, String) {
    let earliest = (now - 3599).min(day_start);
    (earliest, crate::util::local_date(earliest - 86400))
}

/// Start times (unix s) of one user's counted calls (kinds in [`COUNTED_KINDS`], whatever their status) that any cap
/// window can still need: `at - latency_ms / 1000 >= min(now - 3599, day_start)`. Callers sort them into the windows with
/// `quota::check`. Reads an index range of `idx_usage_user` (about one to two days of that user's rows).
pub fn recent_starts(
    c: &Connection,
    user: i64,
    now: i64,
    day_start: i64,
) -> rusqlite::Result<Vec<i64>> {
    let (earliest, from_day) = count_bounds(now, day_start);
    let mut st = c.prepare(&format!(
        "SELECT at - latency_ms / 1000 FROM usage
         WHERE user_id = ?1 AND day >= ?2 AND kind IN {COUNTED_KINDS}
           AND at - latency_ms / 1000 >= ?3"
    ))?;
    let rows = st.query_map(params![user, from_day, earliest], |r| r.get(0))?;
    rows.collect()
}

/// [`recent_starts`] for every user at once: `(user id, start)` pairs, rows without a user excluded. The unary `+` on
/// `user_id` keeps the planner on `idx_usage_day`; never a `GROUP BY user_id` over `usage` (it walks the whole index).
pub fn recent_starts_all(
    c: &Connection,
    now: i64,
    day_start: i64,
) -> rusqlite::Result<Vec<(i64, i64)>> {
    let (earliest, from_day) = count_bounds(now, day_start);
    let mut st = c.prepare(&format!(
        "SELECT user_id, at - latency_ms / 1000 FROM usage
         WHERE day >= ?1 AND kind IN {COUNTED_KINDS} AND +user_id IS NOT NULL
           AND at - latency_ms / 1000 >= ?2"
    ))?;
    let rows = st.query_map(params![from_day, earliest], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
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

/// One model call of a generation or import job (never content).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenCallRow {
    pub task_id: i64,
    pub user_id: i64,
    /// `generate`, `recognise` or `import`
    pub purpose: &'static str,
    pub at: i64,
    pub provider: &'static str,
    pub model: String,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub tokens_cached: Option<i64>,
    pub tokens_cache_write: Option<i64>,
    pub latency_ms: i64,
    pub first_byte_ms: Option<i64>,
    /// `ok`, `error` or `stopped`
    pub status: &'static str,
    pub error_code: Option<String>,
    pub asked: i64,
    pub accepted: i64,
    pub truncated: bool,
}

/// Inserts a `gen_calls` row (FK-safe: a task or user purged meanwhile is stored as NULL).
pub fn insert_gen_call(c: &Connection, g: &GenCallRow) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO gen_calls (task_id, user_id, purpose, at, day, provider, model, tokens_in, tokens_out,
                                tokens_cached, latency_ms, first_byte_ms, status, error_code, asked, accepted, truncated,
                                tokens_cache_write)
         VALUES ((SELECT id FROM tasks WHERE id = ?1), (SELECT id FROM users WHERE id = ?2), ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                 ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            g.task_id,
            g.user_id,
            g.purpose,
            g.at,
            crate::util::local_date(g.at),
            g.provider,
            g.model,
            g.tokens_in,
            g.tokens_out,
            g.tokens_cached,
            g.latency_ms,
            g.first_byte_ms,
            g.status,
            g.error_code,
            g.asked,
            g.accepted,
            i64::from(g.truncated),
            g.tokens_cache_write,
        ],
    )?;
    Ok(())
}

/// Finished tasks older than `cutoff`.
pub fn sweep_tasks(c: &Connection, cutoff: i64) -> rusqlite::Result<usize> {
    c.execute(
        "DELETE FROM tasks WHERE state IN ('done', 'failed', 'cancelled') AND updated_at < ?1",
        [cutoff],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::JournalMode;
    use crate::store::Store;

    const NOW: i64 = 1_700_000_000;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("dir");
        let store = Store::open(dir.path(), JournalMode::Delete).expect("store");
        (dir, store)
    }

    fn add_user(c: &Connection, id: i64) {
        c.execute(
            "INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at) VALUES (?1, ?2, ?3, 1, 1)",
            params![id, format!("{id:032x}"), format!("user{id}")],
        )
        .expect("user");
    }

    /// Inserts a usage row that started at `start` and took `latency_s` seconds (so `at = start + latency_s`; the
    /// millisecond part is 999 to prove the integer division).
    fn add_call(c: &Connection, user: Option<i64>, kind: &'static str, start: i64, latency_s: i64) {
        add_call_day(c, user, kind, start, latency_s, None);
    }

    fn add_call_day(
        c: &Connection,
        user: Option<i64>,
        kind: &'static str,
        start: i64,
        latency_s: i64,
        day: Option<&str>,
    ) {
        insert_usage(
            c,
            &UsageRow {
                user_id: user,
                at: start + latency_s,
                kind,
                provider: "openai",
                model: "demo-model".into(),
                tokens_in: None,
                tokens_out: None,
                tokens_cached: None,
                tokens_cache_write: None,
                latency_ms: latency_s * 1000 + 999,
                first_byte_ms: None,
                status: "ok",
                error_code: None,
            },
        )
        .expect("usage");
        if let Some(d) = day {
            c.execute(
                "UPDATE usage SET day = ?1 WHERE id = last_insert_rowid()",
                [d],
            )
            .expect("day");
        }
    }

    fn sorted(mut v: Vec<i64>) -> Vec<i64> {
        v.sort_unstable();
        v
    }

    #[test]
    fn recent_starts_bound_is_the_earlier_of_the_hour_edge_and_today() {
        let (_d, store) = store();
        store
            .write(|tx| {
                add_user(tx, 1);
                add_call(tx, Some(1), "chat", NOW - 3599, 5);
                add_call(tx, Some(1), "chat", NOW - 3600, 5);
                Ok::<_, rusqlite::Error>(())
            })
            .expect("rows");
        // just after 0:00 (day_start = now - 600): the hour edge is the earlier bound
        let got = store
            .read(|c| recent_starts(c, 1, NOW, NOW - 600))
            .expect("read");
        assert_eq!(got, vec![NOW - 3599]);

        // two hours later in the day: today's 0:00 is the earlier bound
        let day_start = NOW - 600;
        let later = day_start + 7200;
        store
            .write(|tx| {
                add_call(tx, Some(1), "memory", day_start, 2);
                add_call(tx, Some(1), "chat", day_start - 1, 2);
                Ok::<_, rusqlite::Error>(())
            })
            .expect("rows");
        let got = store
            .read(|c| recent_starts(c, 1, later, day_start))
            .expect("read");
        assert_eq!(got, vec![day_start]);
    }

    #[test]
    fn recent_starts_use_the_start_and_find_rows_dated_one_day_off() {
        let (_d, store) = store();
        let day_start = NOW - 20_000;
        let far_date = crate::util::local_date(NOW - 5 * 86400);
        store
            .write(|tx| {
                add_user(tx, 1);
                // ended after the bound but started before it: not returned
                add_call(tx, Some(1), "chat", day_start - 30, 100);
                // started at the bound, `day` written under another zone (one calendar day early): returned
                add_call_day(
                    tx,
                    Some(1),
                    "chat",
                    day_start,
                    1,
                    Some(&crate::util::local_date(day_start - 86400)),
                );
                // a `day` further back than the slack is outside the index range
                add_call_day(tx, Some(1), "chat", day_start + 10, 1, Some(&far_date));
                Ok::<_, rusqlite::Error>(())
            })
            .expect("rows");
        let got = store
            .read(|c| recent_starts(c, 1, NOW, day_start))
            .expect("read");
        assert_eq!(got, vec![day_start]);
    }

    #[test]
    fn only_counted_kinds_of_the_user_come_back() {
        let (_d, store) = store();
        store
            .write(|tx| {
                add_user(tx, 1);
                add_user(tx, 2);
                add_call(tx, Some(1), "chat", NOW - 10, 1);
                add_call(tx, Some(1), "memory", NOW - 20, 1);
                add_call(tx, Some(1), "verify", NOW - 30, 1);
                add_call(tx, Some(1), "models", NOW - 40, 1);
                add_call(tx, Some(2), "chat", NOW - 50, 1);
                add_call(tx, None, "chat", NOW - 60, 1);
                Ok::<_, rusqlite::Error>(())
            })
            .expect("rows");
        let one = store
            .read(|c| recent_starts(c, 1, NOW, NOW - 40_000))
            .expect("read");
        assert_eq!(sorted(one), vec![NOW - 20, NOW - 10]);
        assert_eq!(COUNTED_KINDS, "('chat', 'memory')");
    }

    #[test]
    fn recent_starts_all_matches_recent_starts_per_user() {
        let (_d, store) = store();
        let day_start = NOW - 5000;
        store
            .write(|tx| {
                add_user(tx, 1);
                add_user(tx, 2);
                for (u, start) in [
                    (1, NOW - 3599),
                    (1, NOW - 3600),
                    (1, day_start),
                    (1, day_start - 1),
                    (2, NOW - 100),
                    (2, NOW - 4000),
                    (2, NOW - 3000),
                ] {
                    add_call(tx, Some(u), "chat", start, 3);
                }
                add_call(tx, Some(2), "memory", NOW - 7, 3);
                add_call(tx, Some(1), "verify", NOW - 8, 3);
                add_call(tx, Some(2), "models", NOW - 9, 3);
                add_call(tx, None, "chat", NOW - 10, 3);
                Ok::<_, rusqlite::Error>(())
            })
            .expect("rows");
        for (now, ds) in [(NOW, day_start), (NOW - 4400, NOW - 6000), (NOW, NOW - 600)] {
            let all = store.read(|c| recent_starts_all(c, now, ds)).expect("all");
            assert!(all.iter().all(|(u, _)| *u == 1 || *u == 2), "no NULL user");
            for u in [1_i64, 2] {
                let mine: Vec<i64> = all
                    .iter()
                    .filter(|(x, _)| *x == u)
                    .map(|(_, s)| *s)
                    .collect();
                let one = store.read(|c| recent_starts(c, u, now, ds)).expect("one");
                assert_eq!(sorted(mine), sorted(one), "user {u}, now {now}");
            }
        }
    }

    fn plan(store: &Store, sql: &str, args: &[&dyn rusqlite::ToSql]) -> String {
        store
            .read(|c| {
                let mut st = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
                let rows: Vec<String> = st
                    .query_map(args, |r| r.get::<_, String>(3))?
                    .collect::<rusqlite::Result<_>>()?;
                Ok(rows.join(" | "))
            })
            .expect("plan")
    }

    #[test]
    fn query_plans_use_the_usage_indexes() {
        let (_d, store) = store();
        let one = plan(
            &store,
            &format!(
                "SELECT at - latency_ms / 1000 FROM usage
                 WHERE user_id = ?1 AND day >= ?2 AND kind IN {COUNTED_KINDS}
                   AND at - latency_ms / 1000 >= ?3"
            ),
            &[&1_i64, &"2026-01-01", &0_i64],
        );
        assert!(
            one.contains("USING INDEX idx_usage_user (user_id=? AND day>?)"),
            "{one}"
        );
        assert!(!one.contains("SCAN usage"), "{one}");
        let all = plan(
            &store,
            &format!(
                "SELECT user_id, at - latency_ms / 1000 FROM usage
                 WHERE day >= ?1 AND kind IN {COUNTED_KINDS} AND +user_id IS NOT NULL
                   AND at - latency_ms / 1000 >= ?2"
            ),
            &[&"2026-01-01", &0_i64],
        );
        assert!(all.contains("USING INDEX idx_usage_day (day>?)"), "{all}");
        assert!(!all.contains("SCAN usage"), "{all}");
    }

    #[test]
    fn user_counts_read_the_usage_numbers_in_one_pass() {
        let (_d, store) = store();
        store
            .write(|tx| {
                add_user(tx, 1);
                tx.execute(
                    "INSERT INTO usage (user_id, at, day, kind, provider, latency_ms, status, tokens_in, tokens_out)
                     VALUES (1, 100, '2026-01-01', 'chat', 'openai', 5, 'ok', 10, 20),
                            (1, 300, '2026-01-02', 'chat', 'openai', 5, 'ok', 1, 2)",
                    [],
                )?;
                Ok::<_, rusqlite::Error>(())
            })
            .expect("rows");
        let n = store.read(|c| user_counts(c, 1)).expect("counts");
        assert_eq!(
            (n.last_used_at, n.tokens_in, n.tokens_out),
            (Some(300), 11, 22)
        );
        let none = store.read(|c| user_counts(c, 99)).expect("counts");
        assert_eq!(
            (none.last_used_at, none.tokens_in, none.tokens_out),
            (None, 0, 0)
        );
    }
}
