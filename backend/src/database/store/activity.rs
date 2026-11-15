//! The activity log (schema v8): `activity_log`, written in batches by the activity writer task of the backend and read by the
//! administrators' log page.
//!
//! - Insert ([`Store::activity_insert`]): one transaction per batch. A row that carries a `src_id` (an AI service event) is
//!   `INSERT OR IGNORE`d, so a re-delivered batch never duplicates. A row with a [`Merge`] rule first tries to fold into the
//!   latest matching row (`count + 1`, `last_ts`), and is inserted only when nothing matched: page visits merge within 30
//!   minutes and stop merging 24 hours after the row's own `ts`; refused sign-ins merge within 60 seconds.
//! - `ip` and location are stored only while the sign-in info switch is open (the writer decides, see `activity.rs`). The
//!   switch is also applied when READING: while it is closed, [`Store::activity_page`] answers them as `None`; opening it
//!   shows the stored ones again (rows written while it was closed simply have none).
//! - Read ([`Store::activity_page`]): keyset by `id`, newest first, with the filters of `GET /api/activity`.
//! - Sign-in rows (`auth.signin`, successful and refused) never expire: retention (age and row cap) skips them. Only the
//!   explicit clearing blanks their IP / location (the rows stay), and deleting a user leaves them as they are.
//! - Retention ([`Store::activity_retention`]) and the clearing of IP / location ([`Store::clear_activity_ip`]) run in batches
//!   so the single connection is never held for long.
use super::system_settings::{flag, get, GEO_SOURCE, SIGNIN_INFO};
use super::users::Place;
use super::users::{format_unix_second, location_json};
use super::{run_tx, Store};
use crate::error::DbError;
use rusqlite::types::Value as Sql;
use rusqlite::{params, params_from_iter, Connection};
use serde_json::{json, Value};

/// Page visits merge while the last visit of the row is younger than this (seconds).
pub const PAGE_MERGE_SECS: i64 = 1800;
/// A page-visit row stops merging this long after its own first event (seconds).
pub const PAGE_MERGE_MAX_SECS: i64 = 86_400;
/// Refused sign-ins merge within this window (seconds).
pub const REFUSED_MERGE_SECS: i64 = 60;
/// Rows deleted per retention / clearing statement.
pub const BATCH: i64 = 5000;
const OBJECT_MAX: usize = 200;
const DETAIL_MAX: usize = 2000;
const UA_MAX: usize = 256;
const DEFAULT_LIMIT: i64 = 100;
const MAX_LIMIT: i64 = 200;

/// Who wrote a row: this backend or the AI service (through `/internal/ai/activity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivitySource {
    Backend,
    Ai,
}

impl ActivitySource {
    pub fn as_str(self) -> &'static str {
        match self {
            ActivitySource::Backend => "backend",
            ActivitySource::Ai => "ai",
        }
    }
}

/// How a new row may fold into the latest matching row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Merge {
    /// Always a new row
    None,
    /// `page.view`: same action, user and object (the page); [`PAGE_MERGE_SECS`] / [`PAGE_MERGE_MAX_SECS`]
    PageView,
    /// A refused sign-in: same action, object and ip within [`REFUSED_MERGE_SECS`]
    Refused,
}

/// One row to insert. Strings are cut to their column limits by [`Store::activity_insert`]; `detail` is a JSON text (a
/// `detail` longer than 2000 characters is dropped, never cut into invalid JSON).
#[derive(Debug, Clone)]
pub struct ActivityRow {
    pub ts: i64,
    /// `user`, `anon` or `system`
    pub actor: String,
    pub user_id: Option<i64>,
    pub username: Option<String>,
    pub role: Option<String>,
    pub action: String,
    pub object: Option<String>,
    pub detail: Option<String>,
    pub ok: bool,
    pub reason: Option<String>,
    pub ip: Option<String>,
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub ua: Option<String>,
    pub source: ActivitySource,
    pub src_id: Option<String>,
    pub merge: Merge,
}

/// The user filter of `GET /api/activity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserFilter {
    Id(i64),
    /// `actor = 'system'`
    System,
    /// `actor = 'anon'`
    Anon,
}

/// The filters of `GET /api/activity`; `group` is one of `auth users banks study settings visit ai system` (an unknown
/// group matches nothing).
#[derive(Debug, Clone, Default)]
pub struct ActivityFilter {
    pub before: Option<i64>,
    pub after: Option<i64>,
    /// 1..=200, default 100
    pub limit: Option<i64>,
    pub user: Option<UserFilter>,
    pub group: Option<String>,
    pub action: Option<String>,
    /// `Some(true)` = ok only, `Some(false)` = fail only
    pub ok: Option<bool>,
    pub facets: bool,
}

/// One row as the page shows it (`ip` / `location` are already masked while the sign-in info switch is off).
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityEntry {
    pub id: i64,
    pub ts: i64,
    pub last_ts: i64,
    pub count: i64,
    pub actor: String,
    pub user_id: Option<i64>,
    pub username: Option<String>,
    pub role: Option<String>,
    /// The account no longer exists (the entry keeps its snapshot name)
    pub deleted: bool,
    pub action: String,
    pub object: Option<String>,
    pub detail: Option<Value>,
    pub ok: bool,
    pub reason: Option<String>,
    pub ip: Option<String>,
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub ua: Option<String>,
    pub source: ActivitySource,
}

impl ActivityEntry {
    /// The entry of `GET /api/activity`.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "time": format_unix_second(self.ts),
            "lastTime": format_unix_second(self.last_ts),
            "count": self.count,
            "actor": self.actor,
            "userId": self.user_id,
            "username": self.username,
            "role": self.role,
            "deleted": self.deleted,
            "action": self.action,
            "object": self.object,
            "detail": self.detail,
            "result": if self.ok { "ok" } else { "fail" },
            "reason": self.reason,
            "ip": self.ip,
            "location": location_json(self.country.clone(), self.region.clone(), self.city.clone()),
            "ua": self.ua,
            "source": self.source.as_str(),
        })
    }
}

/// A user the log has entries of (the filter list of the page).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetUser {
    pub id: i64,
    /// The current name, or the snapshot of the latest entry when the account is gone
    pub username: String,
    pub deleted: bool,
}

/// One page of `GET /api/activity`, without the `settings` object (the caller adds it).
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityPage {
    pub entries: Vec<ActivityEntry>,
    /// Pass as `before` for the next (older) page; `None` = no more
    pub next: Option<i64>,
    /// Pass as `after` to read newer entries (the newest id seen, the sent `after` when nothing new)
    pub newest: Option<i64>,
    /// Only with `facets = true`
    pub facets: Option<Vec<FacetUser>>,
    /// The sign-in info switch at the time of the read
    pub signin_info: bool,
}

impl ActivityPage {
    /// `{"entries", "next", "newest"}` and, when asked for, `"facets"`; the caller adds `"settings"`.
    pub fn to_json(&self) -> Value {
        let mut out = json!({
            "entries": self.entries.iter().map(ActivityEntry::to_json).collect::<Vec<_>>(),
            "next": self.next,
            "newest": self.newest,
        });
        if let (Some(f), Some(obj)) = (&self.facets, out.as_object_mut()) {
            let users: Vec<Value> = f
                .iter()
                .map(|u| json!({ "id": u.id, "username": u.username, "deleted": u.deleted }))
                .collect();
            obj.insert("facets".to_string(), json!({ "users": users }));
        }
        out
    }
}

fn cut(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn opt_cut(s: &Option<String>, max: usize) -> Option<String> {
    s.as_deref().map(|v| cut(v, max))
}

/// The `WHERE` condition (over `activity_log`) of a group; `None` = unknown group.
fn group_sql(group: &str) -> Option<&'static str> {
    Some(match group {
        "auth" => "action LIKE 'auth.%'",
        "users" => "action LIKE 'users.%'",
        "banks" => "action LIKE 'bank.%'",
        "study" => "(action LIKE 'study.%' OR action LIKE 'records.%' OR action LIKE 'progress.%')",
        "settings" => {
            "(action LIKE 'settings.%' OR action LIKE 'logs.%' OR action = 'system.shutdown')"
        }
        "visit" => "action = 'page.view'",
        "ai" => "action LIKE 'ai.%'",
        "system" => "(action LIKE 'system.%' AND action <> 'system.shutdown')",
        _ => return None,
    })
}

fn insert_row(c: &Connection, r: &ActivityRow) -> Result<(), DbError> {
    let object = opt_cut(&r.object, OBJECT_MAX);
    let ip = r.ip.as_deref();
    let merged = match r.merge {
        Merge::None => 0,
        Merge::PageView => c.execute(
            "UPDATE activity_log SET count=count+1, last_ts=?1 WHERE id=(SELECT id FROM activity_log \
             WHERE action=?2 AND user_id IS ?3 AND object IS ?4 AND ?1 - last_ts < ?5 AND ts >= ?1 - 86400 AND ?1 - ts < ?6 \
             ORDER BY id DESC LIMIT 1)",
            params![
                r.ts,
                r.action,
                r.user_id,
                object,
                PAGE_MERGE_SECS,
                PAGE_MERGE_MAX_SECS
            ],
        )?,
        Merge::Refused => c.execute(
            "UPDATE activity_log SET count=count+1, last_ts=?1 WHERE id=(SELECT id FROM activity_log \
             WHERE action=?2 AND result='fail' AND object IS ?3 AND ip IS ?4 AND user_id IS ?6 AND username IS ?7 AND reason IS ?8 AND actor=?9 \n             AND ts >= ?1 - 86400 AND ?1 - last_ts < ?5 \
             ORDER BY id DESC LIMIT 1)",
            params![
                r.ts,
                r.action,
                object,
                ip,
                REFUSED_MERGE_SECS,
                r.user_id,
                opt_cut(&r.username, 64),
                opt_cut(&r.reason, 80),
                r.actor
            ],
        )?,
    };
    if merged > 0 {
        return Ok(());
    }
    let detail = r
        .detail
        .as_ref()
        .filter(|d| d.chars().count() <= DETAIL_MAX);
    c.execute(
        "INSERT OR IGNORE INTO activity_log(ts, last_ts, count, actor, user_id, username, role, action, object, detail, \
         result, reason, ip, country, region, city, ua, source, src_id) \
         VALUES(?1, ?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            r.ts,
            r.actor,
            r.user_id,
            opt_cut(&r.username, 64),
            opt_cut(&r.role, 16),
            cut(&r.action, 80),
            object,
            detail,
            if r.ok { "ok" } else { "fail" },
            opt_cut(&r.reason, 80),
            ip,
            opt_cut(&r.country, 2),
            opt_cut(&r.region, 64),
            opt_cut(&r.city, 64),
            opt_cut(&r.ua, UA_MAX),
            r.source.as_str(),
            r.src_id,
        ],
    )?;
    Ok(())
}

impl Store {
    /// Inserts a batch in one transaction (see the module documentation for merging and idempotence).
    pub fn activity_insert(&self, rows: &[ActivityRow]) -> Result<(), DbError> {
        if rows.is_empty() {
            return Ok(());
        }
        let g = self.lock();
        run_tx(&g, |c| {
            for r in rows {
                insert_row(c, r)?;
            }
            Ok(())
        })
    }

    /// One page of the log, newest first (see [`ActivityFilter`]). `page.view` rows appear only with `group = visit` or
    /// `action = page.view`.
    pub fn activity_page(&self, f: &ActivityFilter) -> Result<ActivityPage, DbError> {
        let limit = f.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let g = self.lock();
        let signin_info = flag(&g, SIGNIN_INFO)?;
        let mut cond: Vec<String> = Vec::new();
        let mut args: Vec<Sql> = Vec::new();
        if let Some(b) = f.before {
            cond.push("id < ?".to_string());
            args.push(Sql::Integer(b));
        }
        if let Some(a) = f.after {
            cond.push("id > ?".to_string());
            args.push(Sql::Integer(a));
        }
        match f.user {
            Some(UserFilter::Id(id)) => {
                cond.push("user_id = ?".to_string());
                args.push(Sql::Integer(id));
            }
            Some(UserFilter::System) => cond.push("actor = 'system'".to_string()),
            Some(UserFilter::Anon) => cond.push("actor = 'anon'".to_string()),
            None => {}
        }
        if let Some(group) = f.group.as_deref().filter(|s| !s.is_empty()) {
            cond.push(group_sql(group).unwrap_or("0").to_string());
        }
        if let Some(action) = f.action.as_deref().filter(|s| !s.is_empty()) {
            cond.push("action = ?".to_string());
            args.push(Sql::Text(action.to_string()));
        }
        if let Some(ok) = f.ok {
            cond.push(format!("result = '{}'", if ok { "ok" } else { "fail" }));
        }
        let wants_visits =
            f.group.as_deref() == Some("visit") || f.action.as_deref() == Some("page.view");
        if !wants_visits {
            cond.push("action <> 'page.view'".to_string());
        }
        let where_sql = if cond.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", cond.join(" AND "))
        };
        let sql = format!(
            "SELECT id, ts, last_ts, count, actor, user_id, username, role, \
             (user_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM users u WHERE u.id = activity_log.user_id)), \
             action, object, detail, result, reason, ip, country, region, city, ua, source \
             FROM activity_log {where_sql} ORDER BY id DESC LIMIT {}",
            limit + 1
        );
        let mut st = g.prepare(&sql)?;
        let mut entries = Vec::new();
        let mut rows = st.query(params_from_iter(args.iter()))?;
        while let Some(r) = rows.next()? {
            let detail: Option<String> = r.get(11)?;
            let (ip, country, region, city) = if signin_info {
                (r.get(14)?, r.get(15)?, r.get(16)?, r.get(17)?)
            } else {
                (None, None, None, None)
            };
            entries.push(ActivityEntry {
                id: r.get(0)?,
                ts: r.get(1)?,
                last_ts: r.get(2)?,
                count: r.get(3)?,
                actor: r.get(4)?,
                user_id: r.get(5)?,
                username: r.get(6)?,
                role: r.get(7)?,
                deleted: r.get::<_, i64>(8)? != 0,
                action: r.get(9)?,
                object: r.get(10)?,
                detail: detail.and_then(|d| serde_json::from_str(&d).ok()),
                ok: r.get::<_, String>(12)? == "ok",
                reason: r.get(13)?,
                ip,
                country,
                region,
                city,
                ua: r.get(18)?,
                source: if r.get::<_, String>(19)? == "ai" {
                    ActivitySource::Ai
                } else {
                    ActivitySource::Backend
                },
            });
        }
        let more = i64::try_from(entries.len()).unwrap_or(i64::MAX) > limit;
        if more {
            entries.pop();
        }
        let next = if more {
            entries.last().map(|e| e.id)
        } else {
            None
        };
        let newest = entries.first().map(|e| e.id).or(f.after);
        let facets = if f.facets { Some(facets_of(&g)?) } else { None };
        Ok(ActivityPage {
            entries,
            next,
            newest,
            facets,
            signin_info,
        })
    }

    /// The users the log has entries of: the current name, or the snapshot of the latest entry when the account is gone.
    pub fn activity_facets(&self) -> Result<Vec<FacetUser>, DbError> {
        let g = self.lock();
        facets_of(&g)
    }

    /// Deletes the rows older than `days` days in batches of [`BATCH`], then the oldest rows beyond `cap`. Sign-in rows
    /// (`auth.signin`) are exempt from both rules and are not counted against the cap, so the cap loop always ends.
    /// Returns how many rows were deleted. The lock is released between batches.
    pub fn activity_retention(&self, now: i64, days: i64, cap: i64) -> Result<i64, DbError> {
        let cutoff = now.saturating_sub(days.saturating_mul(86_400));
        let mut total = 0;
        loop {
            let n = {
                let g = self.lock();
                run_tx(&g, |c| {
                    Ok(c.execute(
                        "DELETE FROM activity_log WHERE id IN (SELECT id FROM activity_log WHERE ts < ?1 AND action <> 'auth.signin' LIMIT ?2)",
                        params![cutoff, BATCH],
                    )?)
                })?
            };
            total += i64::try_from(n).unwrap_or(0);
            if n == 0 {
                break;
            }
        }
        loop {
            let n = {
                let g = self.lock();
                run_tx(&g, |c| {
                    let count: i64 = c.query_row(
                        "SELECT COUNT(*) FROM activity_log WHERE action <> 'auth.signin'",
                        [],
                        |r| r.get(0),
                    )?;
                    let excess = count.saturating_sub(cap).clamp(0, BATCH);
                    if excess == 0 {
                        return Ok(0);
                    }
                    Ok(c.execute(
                        "DELETE FROM activity_log WHERE id IN (SELECT id FROM activity_log WHERE action <> 'auth.signin' \n                         ORDER BY id LIMIT ?1)",
                        params![excess],
                    )?)
                })?
            };
            total += i64::try_from(n).unwrap_or(0);
            if n == 0 {
                break;
            }
        }
        Ok(total)
    }

    /// v1.4: puts a looked-up place on what was recorded without one, in ONE transaction: `last_login_*` of every user in
    /// `signin_users` (only while their `last_login_ip` is still `ip` and the three parts are still NULL), `last_active_*`
    /// of every user in `active_users` (same rule), and the activity rows of `ip` written since `since` (unix seconds)
    /// that have no place. Nothing is written when `place` is empty, when the eye is closed or when the place source is
    /// not `api` (both read inside the transaction, so a late result can never record something the settings forbid).
    /// Returns whether anything was written.
    pub fn geo_apply(
        &self,
        ip: &str,
        place: &Place,
        since: i64,
        signin_users: &[i64],
        active_users: &[i64],
    ) -> Result<bool, DbError> {
        if place.is_empty() {
            return Ok(false);
        }
        let g = self.lock();
        run_tx(&g, |c| {
            if !flag(c, SIGNIN_INFO)? || get(c, GEO_SOURCE)?.as_deref() != Some("api") {
                return Ok(false);
            }
            let mut changed = 0usize;
            for id in signin_users {
                changed += c.execute(
                    "UPDATE users SET last_login_country=?1, last_login_region=?2, last_login_city=?3 \
                     WHERE id=?4 AND last_login_ip=?5 AND last_login_country IS NULL AND last_login_region IS NULL \
                     AND last_login_city IS NULL",
                    params![place.country, place.region, place.city, id, ip],
                )?;
            }
            for id in active_users {
                changed += c.execute(
                    "UPDATE users SET last_active_country=?1, last_active_region=?2, last_active_city=?3 \
                     WHERE id=?4 AND last_active_ip=?5 AND last_active_country IS NULL AND last_active_region IS NULL \
                     AND last_active_city IS NULL",
                    params![place.country, place.region, place.city, id, ip],
                )?;
            }
            changed += c.execute(
                "UPDATE activity_log SET country=?1, region=?2, city=?3 WHERE ip=?4 AND ts>=?5 AND country IS NULL \
                 AND region IS NULL AND city IS NULL",
                params![place.country, place.region, place.city, ip, since],
            )?;
            Ok(changed > 0)
        })
    }

    /// Sets `ip`, `country`, `region` and `city` to NULL on up to `batch` rows that have any of them; returns the rows
    /// changed. `POST /api/users/signin-info/clear` calls it until it returns 0 (the lock is released between batches).
    pub fn clear_activity_ip(&self, batch: i64) -> Result<i64, DbError> {
        let g = self.lock();
        run_tx(&g, |c| {
            let n = c.execute(
                "UPDATE activity_log SET ip=NULL, country=NULL, region=NULL, city=NULL WHERE id IN (SELECT id \
                 FROM activity_log WHERE ip IS NOT NULL OR country IS NOT NULL OR region IS NOT NULL \
                 OR city IS NOT NULL LIMIT ?1)",
                params![batch.max(1)],
            )?;
            Ok(i64::try_from(n).unwrap_or(0))
        })
    }
}

fn facets_of(c: &Connection) -> Result<Vec<FacetUser>, DbError> {
    let mut st = c.prepare(
        "SELECT l.user_id, COALESCE(u.username, l.username, ''), u.id IS NULL \
         FROM activity_log l LEFT JOIN users u ON u.id = l.user_id \
         WHERE l.id IN (SELECT MAX(id) FROM activity_log WHERE user_id IS NOT NULL GROUP BY user_id) \
         ORDER BY COALESCE(u.username, l.username, '') COLLATE NOCASE, l.user_id",
    )?;
    let rows = st.query_map([], |r| {
        Ok(FacetUser {
            id: r.get(0)?,
            username: r.get(1)?,
            deleted: r.get::<_, i64>(2)? != 0,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}
