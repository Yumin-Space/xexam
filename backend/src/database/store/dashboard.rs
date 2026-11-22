//! The Home dashboard, `GET /api/dashboard` (read only, writes and logs nothing).
//!
//! - **Ranking**: all time, from `user_stats` (it survives the pruning of old study records), enabled users with at least one
//!   answer, administrators included, every bank. Order: correct answers descending, then fewer answers (better accuracy),
//!   then user id. The first [`TOP`] are listed; the viewer's own position is always reported in `me`.
//! - **Names** (v2.6): a user's public display name (`users.display_name`) is sent as it is, for every viewer. A user who has
//!   not chosen one yet (NULL) shows as [`mask_name`] of the login for others (an e-mail address only as a masked local part,
//!   never a domain) and as the full login for the viewer's own row. No user id leaves.
//! - **Most missed** (`wrongTop`): the viewer's own questions with at least one wrong answer, from the viewer's own
//!   `user_stats` only (unanswered exam items never count there; other users' statistics never leak). Active questions in the
//!   viewer's [`BankScope`](crate::models::BankScope) only. Order: wrong descending, correct ascending, attempts descending,
//!   bank, position. At most [`WRONG_TOP`] items; only ids and counts leave, the page resolves the text itself.
use super::Store;
use crate::error::DbError;
use crate::models::Viewer;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

/// How many entries the leaderboard shows.
pub const TOP: usize = 3;
/// How many questions the "most missed" podium shows.
pub const WRONG_TOP: usize = 5;

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
        "SELECT u.id, u.username, u.display_name, SUM(s.attempts) AS answered, SUM(s.correct) AS good \
         FROM user_stats s JOIN users u ON u.id = s.user_id \
         WHERE u.disabled = 0 \
         GROUP BY u.id HAVING SUM(s.attempts) > 0 \
         ORDER BY good DESC, answered ASC, u.id ASC",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
        ))
    })?;
    let mut top = Vec::new();
    let mut me = json!({"rank": null, "answered": 0, "correct": 0});
    let mut users = 0_i64;
    for row in rows {
        let (id, login, display, answered, good) = row?;
        users += 1;
        let rank = users;
        let own = id == user;
        let name = match display {
            Some(d) => d,
            None if own => login,
            None => mask_name(&login),
        };
        if usize::try_from(rank).is_ok_and(|n| n <= TOP) {
            top.push(json!({
                "rank": rank,
                "name": name,
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

/// The "most missed" query (`?1` = the viewer). `all_banks` adds `LIMIT` (no scope filter runs afterwards).
fn wrong_top_sql(all_banks: bool) -> String {
    let mut sql = String::from(
        "SELECT s.question_id, q.bank_id, s.wrong, s.attempts          FROM user_stats s JOIN questions q ON q.id = s.question_id          WHERE s.user_id = ?1 AND s.wrong > 0 AND q.active = 1 AND q.bank_id IS NOT NULL          ORDER BY s.wrong DESC, s.correct ASC, s.attempts DESC, q.bank_id ASC, q.seq ASC, q.id ASC",
    );
    if all_banks {
        sql.push_str(&format!(" LIMIT {WRONG_TOP}"));
    }
    sql
}

fn wrong_top(c: &Connection, v: &Viewer) -> Result<Value, DbError> {
    let mut st = c.prepare(&wrong_top_sql(v.scope.is_all()))?;
    let rows = st.query_map(params![v.user], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    let mut items = Vec::new();
    for row in rows {
        let (qid, bank, wrong, attempts) = row?;
        if !v.scope.allows(Some(bank)) {
            continue;
        }
        items
            .push(json!({"questionId": qid, "bankId": bank, "wrong": wrong, "attempts": attempts}));
        if items.len() >= WRONG_TOP {
            break;
        }
    }
    Ok(json!({"items": items}))
}

impl Store {
    /// `GET /api/dashboard` as `v` sees it: `{"ranking": {...}, "wrongTop": {"items": [...]}}` (see the module docs).
    pub fn dashboard_as(&self, v: &Viewer) -> Result<Value, DbError> {
        let g = self.lock();
        Ok(json!({
            "ranking": ranking(&g, v.user)?,
            "wrongTop": wrong_top(&g, v)?,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::mask_name;

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
