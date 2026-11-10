//! Users and sessions (schema v3, v0.6).
//!
//! - `users`: one row per account; `username` is unique case-insensitively (`COLLATE NOCASE`), `password_hash` is an Argon2id
//!   PHC string (hashing happens outside the database lock, in the API layer or the CLI).
//! - `sessions`: one row per signed-in browser, keyed by the SHA-256 of the cookie token; `last_seen_at` / `expires_at` are unix
//!   seconds. Expiry slides: a session is refreshed at most once per hour.
//! - Every check that protects the accounts (username taken, "not yourself", "the last enabled administrator") runs inside the
//!   same `BEGIN IMMEDIATE` transaction as the write it guards, so two requests can never both pass it.
use super::{run_tx, Store};
use crate::auth::{self, Role, MAX_SESSIONS_PER_USER, SESSION_REFRESH_SECS, SESSION_TTL_SECS};
use crate::error::DbError;
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
}

/// The signed-in user of a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionUser {
    pub id: i64,
    pub username: String,
    pub role: Role,
    pub must_change_password: bool,
}

impl SessionUser {
    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }
    /// `{"id", "username", "role", "mustChangePassword"}` (`GET /api/auth/state` and friends).
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "username": self.username,
            "role": self.role.as_str(),
            "mustChangePassword": self.must_change_password,
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
        }
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
}

/// Fields of `POST /api/users/update` (absent = unchanged).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserUpdate {
    pub username: Option<String>,
    pub role: Option<Role>,
    pub disabled: Option<bool>,
}

const USER_COLUMNS: &str =
    "id, username, password_hash, role, disabled, created_at, last_login_at, must_change_password";

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
    })
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

/// Creates the default administrator `admin` / `admin` (must change the password) inside the caller's transaction.
pub(super) fn insert_default_admin(c: &Connection, now: &str) -> Result<i64, DbError> {
    let hash = auth::hash_password(auth::DEFAULT_ADMIN_PASSWORD).map_err(DbError::Password)?;
    c.execute(
        "INSERT INTO users(username, password_hash, role, disabled, created_at, password_changed_at, must_change_password) \
         VALUES(?1, ?2, 'admin', 0, ?3, ?3, 1)",
        params![auth::DEFAULT_ADMIN_USERNAME, hash, now],
    )?;
    Ok(c.last_insert_rowid())
}

/// `{"users": [...]}` for `GET /api/users`, ordered by id.
fn users_json(c: &Connection) -> Result<Value, DbError> {
    let mut st = c.prepare(&format!(
        "SELECT {USER_COLUMNS}, (SELECT COUNT(*) FROM attempts a WHERE a.user_id = users.id) FROM users ORDER BY id"
    ))?;
    let rows = st.query_map([], |r| Ok((user_row(r)?, r.get::<_, i64>(8)?)))?;
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
        }));
    }
    Ok(json!({ "users": out }))
}

/// Deletes a user's own rows: sessions, exam records (with their detail), wrong-answer statistics, settings, then the user.
fn delete_user_rows(c: &Connection, id: i64) -> Result<(), DbError> {
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

    /// Whether an enabled administrator still has one of `hashes` (the hashes found to verify the default password at startup).
    /// A changed or reset password, a disabled, demoted or deleted account all end it; no hashing is needed.
    pub fn default_admin_pending(&self, hashes: &[String]) -> Result<bool, DbError> {
        let g = self.lock();
        let mut st = g.prepare(
            "SELECT EXISTS(SELECT 1 FROM users WHERE role='admin' AND disabled=0 AND password_hash=?1)",
        )?;
        for h in hashes {
            if st.query_row(params![h], |r| r.get::<_, bool>(0))? {
                return Ok(true);
            }
        }
        Ok(false)
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
            c.execute(
                "INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at) VALUES(?1,?2,?3,?4,?5)",
                params![token_hash, user_id, at, now, now.saturating_add(SESSION_TTL_SECS)],
            )?;
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
                "SELECT s.last_seen_at, s.expires_at, u.id, u.username, u.password_hash, u.role, u.disabled, u.created_at,                  u.last_login_at, u.must_change_password FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token_hash=?1",
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

    /// `POST /api/auth/password`: the signed-in user's new password (already hashed). Clears `must_change_password` and deletes
    /// the user's other sessions (the one with `keep_token_hash` stays). `expected_hash` is the password hash the caller read
    /// (and verified the current password against): when it changed in the meantime (an administrator's reset, a change from
    /// another session) nothing is written. `Ok(None)` = the user no longer exists, is disabled or has another password now.
    pub fn change_own_password(
        &self,
        user_id: i64,
        expected_hash: &str,
        new_hash: &str,
        keep_token_hash: &str,
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
            c.execute(
                "DELETE FROM sessions WHERE user_id=?1 AND token_hash<>?2",
                params![user_id, keep_token_hash],
            )?;
            Ok(user_by_id(c, user_id)?.as_ref().map(SessionUser::from))
        })
    }

    /// `GET /api/users`
    pub fn users(&self) -> Result<Value, DbError> {
        let g = self.lock();
        users_json(&g)
    }

    /// `POST /api/users/create`: a new enabled account that must choose a new password at its first sign-in. `username` is
    /// already cleaned ([`auth::clean_username`]) and `hash` computed. Returns the new id.
    pub fn create_user(
        &self,
        username: &str,
        hash: &str,
        role: Role,
    ) -> Result<UserChange<i64>, DbError> {
        let at = self.clock.now();
        let g = self.lock();
        run_tx(&g, |c| {
            if name_taken(c, username, 0)? {
                return Ok(UserChange::Taken);
            }
            c.execute(
                "INSERT INTO users(username, password_hash, role, disabled, created_at, password_changed_at, must_change_password) \
                 VALUES(?1, ?2, ?3, 0, ?4, ?4, 1)",
                params![username, hash, role.as_str(), at],
            )?;
            Ok(UserChange::Done(c.last_insert_rowid()))
        })
    }

    /// `POST /api/users/update` by `actor`. Nobody can disable or demote themselves; the last enabled administrator cannot be
    /// disabled or demoted. Disabling or a change of role deletes the user's sessions; a rename keeps them.
    pub fn update_user(
        &self,
        actor: i64,
        id: i64,
        change: &UserUpdate,
    ) -> Result<UserChange<()>, DbError> {
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
            c.execute(
                "UPDATE users SET username=?1, role=?2, disabled=?3 WHERE id=?4",
                params![
                    change.username.as_deref().unwrap_or(&u.username),
                    role.as_str(),
                    i64::from(disabled),
                    id
                ],
            )?;
            if role != u.role || (disabled && !u.disabled) {
                delete_sessions_of(c, id)?;
            }
            Ok(UserChange::Done(()))
        })
    }

    /// `POST /api/users/password` by `actor`: sets the (hashed) password of another user, who must change it at the next
    /// sign-in, and deletes their sessions. Your own password is changed with `POST /api/auth/password` instead. Returns the
    /// user's name (so the API can lift the login throttle for it).
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

    /// `POST /api/users/delete` by `actor`: deletes the user with their sessions, exam records, statistics and settings.
    pub fn delete_user(&self, actor: i64, id: i64) -> Result<UserChange<()>, DbError> {
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
            delete_user_rows(c, id)?;
            Ok(UserChange::Done(()))
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
