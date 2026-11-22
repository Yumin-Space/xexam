//! v2.5: the Home dashboard, `GET /api/dashboard` (read only, writes and logs nothing).
//!
//! - **Ranking**: all time, from `user_stats` (it survives the pruning of old study records), enabled users with at least one
//!   answer, administrators included, every bank. Order: correct answers descending, then fewer answers (better accuracy),
//!   then user id. The first [`TOP`] are listed; the viewer's own position is always reported in `me`.
//! - **Names** are decided here, for every viewer: the viewer's own row shows the full user name; another user's name is shown
//!   in full unless it is an e-mail address, and then only a masked local part is sent ([`mask_name`]). No user id, no domain.
//! - **Most practised**: the last [`POPULAR_DAYS`] days, answered items of all users, active questions in the viewer's
//!   [`BankScope`](crate::models::BankScope) only. Only question ids and counts leave; the page resolves the text itself.
use super::super::timefmt::minus_days;
use super::Store;
use crate::error::DbError;
use crate::models::Viewer;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

/// The window of the "most practised" list, in days.
pub const POPULAR_DAYS: i64 = 30;
/// How many entries each list shows.
pub const TOP: usize = 3;

/// The name shown for another user: a plain user name unchanged; an e-mail address as its masked local part only
/// (`alice@x.y` -> `a***e`, a local part of one or two characters -> first character + `***`, an empty one -> `***`).
pub(crate) fn mask_name(username: &str) -> String {
    let Some((local, _domain)) = username.split_once('@') else {
        return username.to_string();
    };
    let chars: Vec<char> = local.chars().collect();
    match chars.as_slice() {
        [] => "***".to_string(),
        [a] | [a, _] => format!("{a}***"),
        [first, .., last] => format!("{first}***{last}"),
    }
}

fn ranking(c: &Connection, user: i64) -> Result<Value, DbError> {
    let mut st = c.prepare(
        "SELECT u.id, u.username, SUM(s.attempts) AS answered, SUM(s.correct) AS good \
         FROM user_stats s JOIN users u ON u.id = s.user_id \
         WHERE u.disabled = 0 \
         GROUP BY u.id HAVING SUM(s.attempts) > 0 \
         ORDER BY good DESC, answered ASC, u.id ASC",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    let mut top = Vec::new();
    let mut me = json!({"rank": null, "answered": 0, "correct": 0});
    let mut users = 0_i64;
    for row in rows {
        let (id, name, answered, good) = row?;
        users += 1;
        let rank = users;
        let own = id == user;
        if usize::try_from(rank).is_ok_and(|n| n <= TOP) {
            top.push(json!({
                "rank": rank,
                "name": if own { name } else { mask_name(&name) },
                "answered": answered,
                "correct": good,
                "self": own,
            }));
        }
        if own {
            me = json!({"rank": rank, "answered": answered, "correct": good});
        }
    }
    Ok(json!({"users": users, "top": top, "me": me}))
}

/// The "most practised" query (`?1` = the window start). `CROSS JOIN` pins `attempts` as the outer loop: SQLite reads the
/// attempts table (at most `RECORD_RETENTION` rows per user), skips those outside the window, and reads only their items
/// through the `attempt_items` primary key `(attempt_id, seq)`. With a plain `JOIN` the planner walks
/// `idx_attempt_items_question` instead (it serves `GROUP BY question_id`) and so reads every item ever saved, with two
/// random lookups each, while holding the connection lock. `all_banks` adds `LIMIT 3` (no scope filter runs afterwards).
fn popular_sql(all_banks: bool) -> String {
    let mut sql = String::from(
        "SELECT ai.question_id, q.bank_id, COUNT(*) AS times, SUM(ai.correct) AS good \
         FROM attempts a CROSS JOIN attempt_items ai ON ai.attempt_id = a.id \
         JOIN questions q ON q.id = ai.question_id \
         WHERE a.at >= ?1 AND q.active = 1 AND q.bank_id IS NOT NULL \
           AND NOT (ai.answer IS NULL AND ai.correct = 0) \
         GROUP BY ai.question_id \
         ORDER BY times DESC, good DESC, q.bank_id ASC, q.seq ASC",
    );
    if all_banks {
        sql.push_str(" LIMIT 3");
    }
    sql
}

fn popular(c: &Connection, v: &Viewer, now: &str) -> Result<Value, DbError> {
    let Some(since) = minus_days(now, POPULAR_DAYS) else {
        return Ok(json!({"days": POPULAR_DAYS, "since": now, "items": []}));
    };
    let mut st = c.prepare(&popular_sql(v.scope.is_all()))?;
    let rows = st.query_map(params![since], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    let mut items = Vec::new();
    for row in rows {
        let (qid, bank, times, good) = row?;
        if !v.scope.allows(Some(bank)) {
            continue;
        }
        items.push(json!({"questionId": qid, "bankId": bank, "times": times, "correct": good}));
        if items.len() >= TOP {
            break;
        }
    }
    Ok(json!({"days": POPULAR_DAYS, "since": since, "items": items}))
}

impl Store {
    /// `GET /api/dashboard` as `v` sees it: `{"ranking": {...}, "popular": {...}}` (see the module docs).
    pub fn dashboard_as(&self, v: &Viewer) -> Result<Value, DbError> {
        let now = self.clock.now();
        let g = self.lock();
        Ok(json!({
            "ranking": ranking(&g, v.user)?,
            "popular": popular(&g, v, &now)?,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::{mask_name, popular_sql};
    use crate::database::{FixedClock, Store};
    use std::sync::Arc;

    fn plan(c: &rusqlite::Connection, sql: &str) -> Vec<String> {
        let mut st = c
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .expect("explain");
        let rows = st
            .query_map(["2026-10-23 00:00"], |r| r.get::<_, String>(3))
            .expect("query");
        rows.map(|r| r.expect("row")).collect()
    }

    /// The window drives the query on the real schema (no `ANALYZE`): attempts first, then only their items by primary
    /// key; never a walk of the question index over every item ever saved.
    #[test]
    fn most_practised_reads_only_the_items_of_the_window() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::open_with_clock(
            &dir.path().join("x.db"),
            Arc::new(FixedClock("2026-11-22 12:00".to_string())),
        )
        .expect("open");
        let g = store.lock();
        for all_banks in [true, false] {
            let steps = plan(&g, &popular_sql(all_banks));
            let joined = steps.join(" | ");
            assert!(!joined.contains("idx_attempt_items_question"), "{joined}");
            assert!(!joined.contains("SCAN ai"), "{joined}");
            let outer = steps
                .iter()
                .position(|s| s == "SCAN a" || s.starts_with("SEARCH a "))
                .unwrap_or_else(|| panic!("attempts step: {joined}"));
            let items = steps
                .iter()
                .position(|s| s.starts_with("SEARCH ai ") && s.contains("attempt_id=?"))
                .unwrap_or_else(|| panic!("items by primary key: {joined}"));
            assert!(outer < items, "{joined}");
        }
    }

    #[test]
    fn masks_only_email_shaped_names() {
        assert_eq!(mask_name("alice"), "alice");
        assert_eq!(mask_name("alice@x.y"), "a***e");
        assert_eq!(mask_name("ab@x.y"), "a***");
        assert_eq!(mask_name("a@x.y"), "a***");
        assert_eq!(mask_name("@x.y"), "***");
        assert_eq!(mask_name(""), "");
        assert_eq!(mask_name("小明同学@example.invalid"), "小***学");
        assert_eq!(mask_name("小明@example.invalid"), "小***");
        assert_eq!(mask_name("a@b@c"), "a***");
        assert!(!mask_name("longname@example.invalid").contains("example"));
    }
}
