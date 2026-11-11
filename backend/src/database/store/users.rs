//! Users and sessions (schema v3, v0.6).
//!
//! - `users`: one row per account; `username` is unique case-insensitively (`COLLATE NOCASE`), `password_hash` is an Argon2id
//!   PHC string (hashing happens outside the database lock, in the API layer or the CLI).
//! - `sessions`: one row per signed-in browser, keyed by the SHA-256 of the cookie token; `last_seen_at` / `expires_at` are unix
//!   seconds. Expiry slides: a session is refreshed at most once per hour.
//! - Every check that protects the accounts (username taken, "not yourself", "the last enabled administrator") runs inside the
//!   same `BEGIN IMMEDIATE` transaction as the write it guards, so two requests can never both pass it.
//! - Schema v4 (v0.6.5 / v0.7): which banks a plain user may see (their `user_banks` rows, see [`BankAccess`]), whether banks
//!   created later are added for them ("include banks imported later", `new_banks`, see `banks::insert_bank`) and whether
//!   they may manage question banks (`can_manage_banks`). The two bank settings are independent. All three are stored for
//!   administrators too (and kept across role changes), but only matter for plain users: administrators see and manage
//!   every bank.
//! - Schema v5 (v0.8): the AI+ switch (`ai_enabled`, stored for administrators too but ignored for them: they always may
//!   use AI Chat while AI is enabled) and the identity stamp `uid` (32 random lower-case hex digits, set by SQLite's
//!   `randomblob` when the account is created and never changed; never sent to the browser). The AI service keeps the
//!   stamp next to the user id, so an id that belongs to another account after a restore is never taken for the old one.
use super::{run_tx, Store};
use crate::auth::{self, Role, MAX_SESSIONS_PER_USER, SESSION_REFRESH_SECS, SESSION_TTL_SECS};
use crate::error::DbError;
use crate::models::BankAccess;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

/// A row of `users`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub role: Role,
    pub disabled: bool,
    pub created_at: String,
    pub last_login_at: Option<String>,
    pub must_change_password: bool,
    /// "Include banks imported later" (`users.new_banks`): every bank created later is added to their `user_banks` rows
    pub new_banks: bool,
    /// The stored "can manage question banks" permission (`users.can_manage_banks`)
    pub can_manage_banks: bool,
    /// v0.8: the stored AI+ switch (`users.ai_enabled`)
    pub ai_enabled: bool,
    /// v0.8: the identity stamp (`users.uid`, 32 lower-case hex digits; `""` only for a user an older binary created and
    /// the next open has not stamped yet)
    pub uid: String,
}

/// The signed-in user of a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionUser {
    pub id: i64,
    pub username: String,
    pub role: Role,
    pub must_change_password: bool,
    /// May import banks and update / delete their own: always true for administrators (whatever is stored), else the stored
    /// permission. Read from the database on every request, so a change applies at once.
    pub can_manage_banks: bool,
    /// v0.8: may use AI Chat while AI is enabled in AI settings: always true for administrators (whatever is stored), else
    /// the stored AI+ switch. Read from the database on every request, like `can_manage_banks`.
    pub ai_allowed: bool,
    /// v0.8: the identity stamp (`users.uid`); goes into the identity statement for the AI service, never to the browser
    pub uid: String,
}

impl SessionUser {
    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }
    /// `{"id", "username", "role", "mustChangePassword", "canManageBanks", "aiAllowed"}` (`GET /api/auth/state` and
    /// friends). The identity stamp is not part of it.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "username": self.username,
            "role": self.role.as_str(),
            "mustChangePassword": self.must_change_password,
            "canManageBanks": self.can_manage_banks,
            "aiAllowed": self.ai_allowed,
        })
    }
}

impl From<&UserRow> for SessionUser {
    fn from(u: &UserRow) -> Self {
        SessionUser {
            id: u.id,
            username: u.username.clone(),
            role: u.role,
            must_change_password: u.must_change_password,
            can_manage_banks: u.role == Role::Admin || u.can_manage_banks,
            ai_allowed: u.role == Role::Admin || u.ai_enabled,
            uid: u.uid.clone(),
        }
    }
}

/// One user as `POST /internal/ai/users` reports it to the AI service (v0.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiUserRow {
    pub id: i64,
    /// The identity stamp (`users.uid`)
    pub stamp: String,
    pub username: String,
    pub role: Role,
    pub disabled: bool,
    /// The stored AI+ switch
    pub ai: bool,
    /// Administrator, or the AI+ switch is on
    pub ai_allowed: bool,
}

impl AiUserRow {
    /// `{"id", "stamp", "username", "role", "disabled", "ai", "aiAllowed"}`
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "stamp": self.stamp,
            "username": self.username,
            "role": self.role.as_str(),
            "disabled": self.disabled,
            "ai": self.ai,
            "aiAllowed": self.ai_allowed,
        })
    }
}

/// Outcome of a change to the accounts that can be refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserChange<T> {
    Done(T),
    /// No user with that id (404)
    NotFound,
    /// The username is used by another account (409)
    Taken,
    /// Deleting, disabling or demoting yourself, or resetting your own password through the admin route (409)
    NotYourself,
    /// The change would leave no enabled administrator (409)
    LastAdmin,
    /// A bank id in `"banks"` does not exist (400)
    UnknownBanks,
}

/// Fields of `POST /api/users/update` (absent = unchanged).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserUpdate {
    pub username: Option<String>,
    pub role: Option<Role>,
    pub disabled: Option<bool>,
    /// v0.6.5: the existing banks the user may see (replaces their `user_banks` rows)
    pub banks: Option<BankAccess>,
    /// v0.7: "include banks imported later"
    pub new_banks: Option<bool>,
    /// v0.6.5: the "can manage question banks" permission
    pub can_manage_banks: Option<bool>,
    /// v0.8: the AI+ switch
    pub ai: Option<bool>,
}

/// The names of a user an update changed (for the log): the username now, and the one before when it was renamed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatedUser {
    pub username: String,
    /// `Some(old name)` when the update renamed the user
    pub renamed_from: Option<String>,
    /// v0.8: `Some(new value)` when the update switched AI+ on or off (`None`: unchanged)
    pub ai_changed: Option<bool>,
}

/// Number of columns in [`USER_COLUMNS`] (the next column of a query that appends one is at this index).
const USER_COLUMN_COUNT: usize = 12;

const USER_COLUMNS: &str = "id, username, password_hash, role, disabled, created_at, last_login_at, must_change_password, \
     new_banks, can_manage_banks, ai_enabled, uid";

fn user_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<UserRow> {
    let role: String = r.get(3)?;
    Ok(UserRow {
        id: r.get(0)?,
        username: r.get(1)?,
        password_hash: r.get(2)?,
        // The CHECK constraint allows only these two; anything else is treated as the least privileged role
        role: Role::parse(&role).unwrap_or(Role::User),
        disabled: r.get::<_, i64>(4)? != 0,
        created_at: r.get(5)?,
        last_login_at: r.get(6)?,
        must_change_password: r.get::<_, i64>(7)? != 0,
        new_banks: r.get::<_, i64>(8)? != 0,
        can_manage_banks: r.get::<_, i64>(9)? != 0,
        ai_enabled: r.get::<_, i64>(10)? != 0,
        uid: r.get(11)?,
    })
}

/// Stores the existing banks `user` may see (inside the caller's transaction): replaces their `user_banks` rows with one row
/// per bank of `access` (`All`: every bank that exists now). Does not touch "include banks imported later". Every id must be
/// an existing bank, else nothing is written and the result is `false`.
fn set_bank_access(c: &Connection, user: i64, access: &BankAccess) -> Result<bool, DbError> {
    if let BankAccess::Only(ids) = access {
        let mut exists = c.prepare("SELECT EXISTS(SELECT 1 FROM banks WHERE id=?1)")?;
        for id in ids {
            if !exists.query_row(params![id], |r| r.get::<_, bool>(0))? {
                return Ok(false);
            }
        }
    }
    c.execute("DELETE FROM user_banks WHERE user_id=?1", params![user])?;
    match access {
        BankAccess::All => {
            c.execute(
                "INSERT OR IGNORE INTO user_banks(user_id, bank_id) SELECT ?1, id FROM banks",
                params![user],
            )?;
        }
        BankAccess::Only(ids) => {
            let mut ins =
                c.prepare("INSERT OR IGNORE INTO user_banks(user_id, bank_id) VALUES(?1, ?2)")?;
            for id in ids {
                ins.execute(params![user, id])?;
            }
        }
    }
    Ok(true)
}

pub(super) fn user_by_id(c: &Connection, id: i64) -> Result<Option<UserRow>, DbError> {
    Ok(c.query_row(
        &format!("SELECT {USER_COLUMNS} FROM users WHERE id=?1"),
        params![id],
        user_row,
    )
    .optional()?)
}

fn user_by_name(c: &Connection, name: &str) -> Result<Option<UserRow>, DbError> {
    // `username` is declared COLLATE NOCASE, so `=` compares case-insensitively
    Ok(c.query_row(
        &format!("SELECT {USER_COLUMNS} FROM users WHERE username=?1"),
        params![name],
        user_row,
    )
    .optional()?)
}

fn name_taken(c: &Connection, name: &str, except: i64) -> Result<bool, DbError> {
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE username=?1 AND id<>?2)",
        params![name, except],
        |r| r.get::<_, bool>(0),
    )?)
}

/// Enabled administrators other than `except`.
fn other_enabled_admins(c: &Connection, except: i64) -> Result<i64, DbError> {
    Ok(c.query_row(
        "SELECT COUNT(*) FROM users WHERE role='admin' AND disabled=0 AND id<>?1",
        params![except],
        |r| r.get(0),
    )?)
}

/// Stores a new session (`token_hash` = SHA-256 of the cookie token) that expires [`SESSION_TTL_SECS`] after `now`.
fn insert_session(
    c: &Connection,
    token_hash: &str,
    user_id: i64,
    created_at: &str,
    now: i64,
) -> Result<(), DbError> {
    c.execute(
        "INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at) VALUES(?1,?2,?3,?4,?5)",
        params![token_hash, user_id, created_at, now, now.saturating_add(SESSION_TTL_SECS)],
    )?;
    Ok(())
}

fn delete_sessions_of(c: &Connection, user_id: i64) -> Result<(), DbError> {
    c.execute("DELETE FROM sessions WHERE user_id=?1", params![user_id])?;
    Ok(())
}

/// Inside a per-user write transaction: the user must still exist and be enabled. The session was checked earlier, in its own
/// short lock, so a user deleted or disabled while the request was running would otherwise get rows written under an id that
/// no longer has an account. [`DbError::NoUser`] = write nothing (the API answers 401).
pub(super) fn ensure_active(c: &Connection, user_id: i64) -> Result<(), DbError> {
    let active: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND disabled=0)",
        params![user_id],
        |r| r.get(0),
    )?;
    if active {
        Ok(())
    } else {
        Err(DbError::NoUser)
    }
}

pub(super) fn has_users(c: &Connection) -> Result<bool, DbError> {
    Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM users)", [], |r| {
        r.get::<_, bool>(0)
    })?)
}

/// The administrator that owns data without an owner (the claim of the v3 migration, the old progress.json): the enabled
/// administrator with the lowest id, else any administrator.
pub(super) fn owner_admin(c: &Connection) -> Result<Option<i64>, DbError> {
    Ok(c.query_row(
        "SELECT id FROM users WHERE role='admin' ORDER BY disabled, id LIMIT 1",
        [],
        |r| r.get(0),
    )
    .optional()?)
}

/// Creates the default administrator `admin` / `admin` (must change the password) inside the caller's transaction, with
/// an identity stamp when the table has the column (schema v5). The v3 step of a migration runs before the v5 step has
/// added it; the v5 step then stamps every user, this one included.
pub(super) fn insert_default_admin(c: &Connection, now: &str) -> Result<i64, DbError> {
    let hash = auth::hash_password(auth::DEFAULT_ADMIN_PASSWORD).map_err(DbError::Password)?;
    let sql = if super::migrate::has_column(c, "users", "uid")? {
        "INSERT INTO users(username, password_hash, role, disabled, created_at, password_changed_at, must_change_password, \
         uid) VALUES(?1, ?2, 'admin', 0, ?3, ?3, 1, lower(hex(randomblob(16))))"
    } else {
        "INSERT INTO users(username, password_hash, role, disabled, created_at, password_changed_at, must_change_password) \
         VALUES(?1, ?2, 'admin', 0, ?3, ?3, 1)"
    };
    c.execute(sql, params![auth::DEFAULT_ADMIN_USERNAME, hash, now])?;
    let id = c.last_insert_rowid();
    note_known_users(c)?;
    Ok(id)
}

/// The `meta` key holding the highest user id that this version (v0.7 or later) knows about. A user with a higher id was
/// created by an older binary after a rollback: it was created without `user_banks` rows or `new_banks` (so with no banks),
/// while in that version every user saw every bank. The fix-up on open gives such users every bank and "include banks
/// imported later", and moves the mark (see `migrate.rs`).
pub(super) const KNOWN_USERS_KEY: &str = "users_known";

/// Records the highest user id as known to this version (inside the caller's transaction). User ids are never reused
/// (`AUTOINCREMENT`), so every user an older binary creates later has a higher id.
pub(super) fn note_known_users(c: &Connection) -> Result<(), DbError> {
    c.execute(
        "INSERT INTO meta(key, value) SELECT ?1, CAST(COALESCE(MAX(id), 0) AS TEXT) FROM users \
         WHERE 1 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![KNOWN_USERS_KEY],
    )?;
    Ok(())
}

/// `{"users": [...]}` for `GET /api/users`, ordered by id.
fn users_json(c: &Connection) -> Result<Value, DbError> {
    // user id -> its assigned banks / the banks it owns (ascending ids)
    let mut assigned: std::collections::HashMap<i64, Vec<i64>> = Default::default();
    {
        let mut st = c.prepare(
            "SELECT user_id, bank_id FROM user_banks WHERE bank_id IN (SELECT id FROM banks) ORDER BY user_id, bank_id",
        )?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (user, bank) = row?;
            assigned.entry(user).or_default().push(bank);
        }
    }
    let mut owned: std::collections::HashMap<i64, Vec<i64>> = Default::default();
    {
        let mut st = c.prepare(
            "SELECT owner_id, id FROM banks WHERE owner_id IS NOT NULL ORDER BY owner_id, id",
        )?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (user, bank) = row?;
            owned.entry(user).or_default().push(bank);
        }
    }
    let mut st = c.prepare(&format!(
        "SELECT {USER_COLUMNS}, (SELECT COUNT(*) FROM attempts a WHERE a.user_id = users.id) FROM users ORDER BY id"
    ))?;
    let rows = st.query_map([], |r| {
        Ok((user_row(r)?, r.get::<_, i64>(USER_COLUMN_COUNT)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (u, records) = row?;
        out.push(json!({
            "id": u.id,
            "username": u.username,
            "role": u.role.as_str(),
            "disabled": u.disabled,
            "createdAt": u.created_at,
            "lastLoginAt": u.last_login_at,
            "mustChangePassword": u.must_change_password,
            "records": records,
            "banks": assigned.remove(&u.id).unwrap_or_default(),
            "newBanks": u.new_banks,
            "canManageBanks": u.can_manage_banks,
            "ai": u.ai_enabled,
            "ownedBanks": owned.remove(&u.id).unwrap_or_default(),
        }));
    }
    Ok(json!({ "users": out }))
}

/// Deletes a user's own rows: sessions, exam records (with their detail), wrong-answer statistics, settings, which banks they
/// may see, then the user. The banks they own move to `heir` (the administrator deleting them), so no bank is left without an
/// owner.
fn delete_user_rows(c: &Connection, id: i64, heir: i64) -> Result<(), DbError> {
    c.execute(
        "UPDATE banks SET owner_id=?1 WHERE owner_id=?2",
        params![heir, id],
    )?;
    c.execute("DELETE FROM user_banks WHERE user_id=?1", params![id])?;
    delete_sessions_of(c, id)?;
    c.execute(
        "DELETE FROM attempt_items WHERE attempt_id IN (SELECT id FROM attempts WHERE user_id=?1)",
        params![id],
    )?;
    c.execute("DELETE FROM attempts WHERE user_id=?1", params![id])?;
    c.execute("DELETE FROM user_stats WHERE user_id=?1", params![id])?;
    c.execute("DELETE FROM user_settings WHERE user_id=?1", params![id])?;
    c.execute("DELETE FROM users WHERE id=?1", params![id])?;
    Ok(())
}

impl Store {
    /// Whether this open created the default administrator (`admin` / `admin`), so the startup log can say so once.
    pub fn created_default_admin(&self) -> bool {
        self.created_default_admin
    }

    /// The user with this name (case-insensitive), enabled or not.
    pub fn user_by_name(&self, name: &str) -> Result<Option<UserRow>, DbError> {
        let g = self.lock();
        user_by_name(&g, name)
    }

    pub fn user_by_id(&self, id: i64) -> Result<Option<UserRow>, DbError> {
        let g = self.lock();
        user_by_id(&g, id)
    }

    /// The password hashes of every user who must still choose a new password (the only accounts that can have the default
    /// password; the caller verifies them, outside the database lock).
    pub fn password_hashes_to_change(&self) -> Result<Vec<String>, DbError> {
        let g = self.lock();
        let mut st =
            g.prepare("SELECT password_hash FROM users WHERE must_change_password=1 ORDER BY id")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The username of an enabled administrator that still has one of `hashes` (the hashes found to verify the default password
    /// at startup), the lowest id first; `None` when there is none. The name is read now, so a renamed default administrator
    /// is named correctly. A changed or reset password, a disabled, demoted or deleted account all end it; no hashing is needed.
    pub fn default_admin_pending(&self, hashes: &[String]) -> Result<Option<String>, DbError> {
        let g = self.lock();
        let mut st = g.prepare(
            "SELECT id, username FROM users WHERE role='admin' AND disabled=0 AND password_hash=?1 ORDER BY id LIMIT 1",
        )?;
        let mut found: Vec<(i64, String)> = Vec::new();
        for h in hashes {
            if let Some(row) = st
                .query_row(params![h], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })
                .optional()?
            {
                found.push(row);
            }
        }
        Ok(found.into_iter().min().map(|(_, name)| name))
    }

    /// A successful sign-in of `user_id`: the caller verified the password against `verified_hash` (the PHC string it read).
    /// Stores the new session under `token_hash` and sets `last_login_at`. Re-checks in the transaction that the user still
    /// exists, is enabled and still has exactly that password hash: a password changed or reset while the (slow) verify ran
    /// must not leave a session signed in with the old password. `Ok(None)` otherwise. Keeps at most
    /// [`MAX_SESSIONS_PER_USER`] sessions per user (the ones that expire first are deleted).
    pub fn start_session(
        &self,
        user_id: i64,
        token_hash: &str,
        verified_hash: &str,
        now: i64,
    ) -> Result<Option<SessionUser>, DbError> {
        let at = self.clock.now();
        let g = self.lock();
        run_tx(&g, |c| {
            let Some(u) =
                user_by_id(c, user_id)?.filter(|u| !u.disabled && u.password_hash == verified_hash)
            else {
                return Ok(None);
            };
            insert_session(c, token_hash, user_id, &at, now)?;
            c.execute(
                "DELETE FROM sessions WHERE user_id=?1 AND token_hash NOT IN \
                 (SELECT token_hash FROM sessions WHERE user_id=?1 ORDER BY expires_at DESC, rowid DESC LIMIT ?2)",
                params![user_id, MAX_SESSIONS_PER_USER],
            )?;
            c.execute(
                "UPDATE users SET last_login_at=?1 WHERE id=?2",
                params![at, user_id],
            )?;
            Ok(Some(SessionUser::from(&u)))
        })
    }

    /// The user signed in with the session `token_hash`, if that session exists, has not expired and its user is enabled.
    /// With `may_refresh`, a session last refreshed at least an hour ago gets `last_seen_at = now` and a new 30-day expiry
    /// (the second value says whether that happened). Reading writes nothing otherwise (an expired session is deleted).
    pub fn session_user(
        &self,
        token_hash: &str,
        now: i64,
        may_refresh: bool,
    ) -> Result<Option<(SessionUser, bool)>, DbError> {
        let g = self.lock();
        let found: Option<(i64, i64, UserRow)> = g
            .query_row(
                "SELECT s.last_seen_at, s.expires_at, u.id, u.username, u.password_hash, u.role, u.disabled, u.created_at, \
                 u.last_login_at, u.must_change_password, u.new_banks, u.can_manage_banks, u.ai_enabled, u.uid \
                 FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token_hash=?1",
                params![token_hash],
                |r| {
                    let user = UserRow {
                        id: r.get(2)?,
                        username: r.get(3)?,
                        password_hash: r.get(4)?,
                        role: Role::parse(&r.get::<_, String>(5)?).unwrap_or(Role::User),
                        disabled: r.get::<_, i64>(6)? != 0,
                        created_at: r.get(7)?,
                        last_login_at: r.get(8)?,
                        must_change_password: r.get::<_, i64>(9)? != 0,
                        new_banks: r.get::<_, i64>(10)? != 0,
                        can_manage_banks: r.get::<_, i64>(11)? != 0,
                        ai_enabled: r.get::<_, i64>(12)? != 0,
                        uid: r.get(13)?,
                    };
                    Ok((r.get(0)?, r.get(1)?, user))
                },
            )
            .optional()?;
        let Some((last_seen, expires, user)) = found else {
            return Ok(None);
        };
        if expires <= now {
            g.execute(
                "DELETE FROM sessions WHERE token_hash=?1",
                params![token_hash],
            )?;
            return Ok(None);
        }
        if user.disabled {
            return Ok(None);
        }
        let refresh = may_refresh && now.saturating_sub(last_seen) >= SESSION_REFRESH_SECS;
        if refresh {
            g.execute(
                "UPDATE sessions SET last_seen_at=?1, expires_at=?2 WHERE token_hash=?3",
                params![now, now.saturating_add(SESSION_TTL_SECS), token_hash],
            )?;
        }
        Ok(Some((SessionUser::from(&user), refresh)))
    }

    /// Sign-out: deletes that one session.
    pub fn end_session(&self, token_hash: &str) -> Result<(), DbError> {
        let g = self.lock();
        g.execute(
            "DELETE FROM sessions WHERE token_hash=?1",
            params![token_hash],
        )?;
        Ok(())
    }

    /// Deletes every expired session; returns how many.
    pub fn purge_expired_sessions(&self, now: i64) -> Result<usize, DbError> {
        let g = self.lock();
        Ok(g.execute("DELETE FROM sessions WHERE expires_at<=?1", params![now])?)
    }

    /// `POST /api/auth/password`: the signed-in user's new password (already hashed). Clears `must_change_password` and
    /// rotates the session: every session of the user is deleted, the one that made the request included, and a new one is
    /// stored under `new_token_hash` (the caller sends its cookie). A copy of the old cookie therefore stops working too.
    /// `expected_hash` is the password hash the caller read (and verified the current password against): when it changed in
    /// the meantime (an administrator's reset, a change from another session) nothing is written. `Ok(None)` = the user no
    /// longer exists, is disabled or has another password now.
    pub fn change_own_password(
        &self,
        user_id: i64,
        expected_hash: &str,
        new_hash: &str,
        new_token_hash: &str,
        now: i64,
    ) -> Result<Option<SessionUser>, DbError> {
        let at = self.clock.now();
        let g = self.lock();
        run_tx(&g, |c| {
            let n = c.execute(
                "UPDATE users SET password_hash=?1, password_changed_at=?2, must_change_password=0 \
                 WHERE id=?3 AND password_hash=?4 AND disabled=0",
                params![new_hash, at, user_id, expected_hash],
            )?;
            if n == 0 {
                return Ok(None);
            }
            delete_sessions_of(c, user_id)?;
            insert_session(c, new_token_hash, user_id, &at, now)?;
            Ok(user_by_id(c, user_id)?.as_ref().map(SessionUser::from))
        })
    }

    /// `GET /api/users`
    pub fn users(&self) -> Result<Value, DbError> {
        let g = self.lock();
        users_json(&g)
    }

    /// `POST /internal/ai/users` (v0.8): the users among `ids` that exist, plus, with `holders`, every user who has AI+
    /// (every administrator, and every plain user with the switch on); each user once, ordered by id. Read-only.
    pub fn ai_users(&self, ids: &[i64], holders: bool) -> Result<Vec<AiUserRow>, DbError> {
        let list = serde_json::to_string(ids)?;
        let g = self.lock();
        let mut st = g.prepare(
            "SELECT id, uid, username, role, disabled, ai_enabled FROM users \
             WHERE id IN (SELECT value FROM json_each(?1)) OR (?2 AND (role = 'admin' OR ai_enabled <> 0)) \
             ORDER BY id",
        )?;
        let rows = st.query_map(params![list, holders], |r| {
            let role = Role::parse(&r.get::<_, String>(3)?).unwrap_or(Role::User);
            let ai = r.get::<_, i64>(5)? != 0;
            Ok(AiUserRow {
                id: r.get(0)?,
                stamp: r.get(1)?,
                username: r.get(2)?,
                role,
                disabled: r.get::<_, i64>(4)? != 0,
                ai,
                ai_allowed: role == Role::Admin || ai,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// [`Store::create_user_with`] with the defaults of `POST /api/users/create`: no bank, banks imported later not included,
    /// no bank management permission.
    pub fn create_user(
        &self,
        username: &str,
        hash: &str,
        role: Role,
    ) -> Result<UserChange<i64>, DbError> {
        self.create_user_with(
            username,
            hash,
            role,
            &BankAccess::Only(Vec::new()),
            false,
            false,
        )
    }

    /// [`Store::create_user_opts`] with AI+ off (the default of `POST /api/users/create`).
    pub fn create_user_with(
        &self,
        username: &str,
        hash: &str,
        role: Role,
        banks: &BankAccess,
        new_banks: bool,
        can_manage_banks: bool,
    ) -> Result<UserChange<i64>, DbError> {
        self.create_user_opts(
            username,
            hash,
            role,
            banks,
            new_banks,
            can_manage_banks,
            false,
        )
    }

    /// `POST /api/users/create`: a new enabled account that must choose a new password at its first sign-in, seeing the
    /// existing banks `banks`, with "include banks imported later" `new_banks`, the bank management permission
    /// `can_manage_banks` and the AI+ switch `ai` (v0.8), and a new random identity stamp. `username` is already cleaned
    /// ([`auth::clean_username`]) and `hash` computed. Returns the new id; [`UserChange::UnknownBanks`] when a bank id does
    /// not exist (nothing written).
    #[allow(clippy::too_many_arguments)]
    pub fn create_user_opts(
        &self,
        username: &str,
        hash: &str,
        role: Role,
        banks: &BankAccess,
        new_banks: bool,
        can_manage_banks: bool,
        ai: bool,
    ) -> Result<UserChange<i64>, DbError> {
        let at = self.clock.now();
        let g = self.lock();
        run_tx(&g, |c| {
            if name_taken(c, username, 0)? {
                return Ok(UserChange::Taken);
            }
            c.execute(
                "INSERT INTO users(username, password_hash, role, disabled, created_at, password_changed_at, must_change_password, \
                 new_banks, can_manage_banks, ai_enabled, uid) \
                 VALUES(?1, ?2, ?3, 0, ?4, ?4, 1, ?5, ?6, ?7, lower(hex(randomblob(16))))",
                params![
                    username,
                    hash,
                    role.as_str(),
                    at,
                    i64::from(new_banks),
                    i64::from(can_manage_banks),
                    i64::from(ai)
                ],
            )?;
            let id = c.last_insert_rowid();
            note_known_users(c)?;
            if !set_bank_access(c, id, banks)? {
                // Returning an error rolls the insert back: the account is not created
                return Err(DbError::UnknownBanks);
            }
            Ok(UserChange::Done(id))
        })
        .or_else(|e| match e {
            DbError::UnknownBanks => Ok(UserChange::UnknownBanks),
            e => Err(e),
        })
    }

    /// `POST /api/users/update` by `actor`. Nobody can disable or demote themselves; the last enabled administrator cannot be
    /// disabled or demoted. Disabling or a change of role deletes the user's sessions; a rename keeps them, and so does a
    /// change of the banks, of "include banks imported later", of the bank management permission or of AI+ (read from the
    /// database on every request, so it applies at once; the AI service re-checks AI+ during a running answer too). An unknown bank id refuses the whole change ([`UserChange::UnknownBanks`],
    /// nothing written). Returns the user's name now (and the old one after a rename), read in the same transaction.
    pub fn update_user(
        &self,
        actor: i64,
        id: i64,
        change: &UserUpdate,
    ) -> Result<UserChange<UpdatedUser>, DbError> {
        let g = self.lock();
        run_tx(&g, |c| {
            let Some(u) = user_by_id(c, id)? else {
                return Ok(UserChange::NotFound);
            };
            let role = change.role.unwrap_or(u.role);
            let disabled = change.disabled.unwrap_or(u.disabled);
            let loses_admin =
                u.role == Role::Admin && !u.disabled && (role != Role::Admin || disabled);
            if id == actor && (disabled || role != u.role) {
                return Ok(UserChange::NotYourself);
            }
            if loses_admin && other_enabled_admins(c, id)? == 0 {
                return Ok(UserChange::LastAdmin);
            }
            if let Some(name) = &change.username {
                if name_taken(c, name, id)? {
                    return Ok(UserChange::Taken);
                }
            }
            let username = change.username.clone().unwrap_or_else(|| u.username.clone());
            let ai = change.ai.unwrap_or(u.ai_enabled);
            c.execute(
                "UPDATE users SET username=?1, role=?2, disabled=?3, new_banks=?4, can_manage_banks=?5, ai_enabled=?6 \
                 WHERE id=?7",
                params![
                    username,
                    role.as_str(),
                    i64::from(disabled),
                    i64::from(change.new_banks.unwrap_or(u.new_banks)),
                    i64::from(change.can_manage_banks.unwrap_or(u.can_manage_banks)),
                    i64::from(ai),
                    id
                ],
            )?;
            if let Some(banks) = &change.banks {
                if !set_bank_access(c, id, banks)? {
                    return Err(DbError::UnknownBanks);
                }
            }
            if role != u.role || (disabled && !u.disabled) {
                delete_sessions_of(c, id)?;
            }
            // `username` is COLLATE NOCASE: a change of case only is a rename too (the stored text changed)
            let renamed_from = (username != u.username).then_some(u.username);
            Ok(UserChange::Done(UpdatedUser {
                username,
                renamed_from,
                ai_changed: (ai != u.ai_enabled).then_some(ai),
            }))
        })
        .or_else(|e| match e {
            DbError::UnknownBanks => Ok(UserChange::UnknownBanks),
            e => Err(e),
        })
    }

    /// `POST /api/users/password` by `actor`: sets the (hashed) password of another user, who must change it at the next
    /// sign-in, and deletes their sessions. Your own password is changed with `POST /api/auth/password` instead. Returns the
    /// user's name (read in the same transaction).
    pub fn reset_user_password(
        &self,
        actor: i64,
        id: i64,
        hash: &str,
    ) -> Result<UserChange<String>, DbError> {
        let at = self.clock.now();
        let g = self.lock();
        run_tx(&g, |c| {
            let Some(u) = user_by_id(c, id)? else {
                return Ok(UserChange::NotFound);
            };
            if id == actor {
                return Ok(UserChange::NotYourself);
            }
            c.execute(
                "UPDATE users SET password_hash=?1, password_changed_at=?2, must_change_password=1 WHERE id=?3",
                params![hash, at, id],
            )?;
            delete_sessions_of(c, id)?;
            Ok(UserChange::Done(u.username))
        })
    }

    /// `POST /api/users/delete` by `actor`: deletes the user with their sessions, exam records, statistics, settings and bank
    /// assignments; the banks they own now belong to `actor`. Returns the deleted user's name (read in the same transaction).
    pub fn delete_user(&self, actor: i64, id: i64) -> Result<UserChange<String>, DbError> {
        let g = self.lock();
        run_tx(&g, |c| {
            let Some(u) = user_by_id(c, id)? else {
                return Ok(UserChange::NotFound);
            };
            if id == actor {
                return Ok(UserChange::NotYourself);
            }
            if u.role == Role::Admin && !u.disabled && other_enabled_admins(c, id)? == 0 {
                return Ok(UserChange::LastAdmin);
            }
            delete_user_rows(c, id, actor)?;
            Ok(UserChange::Done(u.username))
        })
    }

    /// The `reset-password` command: sets the (hashed) password of `username`, enables the account, requires a new password at
    /// the next sign-in and deletes its sessions. `Ok(None)` = no such user; otherwise the stored username.
    pub fn cli_reset_password(
        &self,
        username: &str,
        hash: &str,
    ) -> Result<Option<String>, DbError> {
        let at = self.clock.now();
        let g = self.lock();
        run_tx(&g, |c| {
            let Some(u) = user_by_name(c, username)? else {
                return Ok(None);
            };
            c.execute(
                "UPDATE users SET password_hash=?1, password_changed_at=?2, must_change_password=1, disabled=0 WHERE id=?3",
                params![hash, at, u.id],
            )?;
            delete_sessions_of(c, u.id)?;
            Ok(Some(u.username))
        })
    }
}
