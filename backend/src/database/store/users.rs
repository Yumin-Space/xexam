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
//! - Schema v10 (v1.6): the permission to download the banks a user can see (`can_download_banks`), stored like
//!   `can_manage_banks` (kept across role changes) and always true for administrators.
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
    /// v1.6: the stored "can download question banks" permission (`users.can_download_banks`)
    pub can_download_banks: bool,
    /// v0.8: the stored AI+ switch (`users.ai_enabled`)
    pub ai_enabled: bool,
    /// v0.8: the identity stamp (`users.uid`, 32 lower-case hex digits; `""` only for a user an older binary created and
    /// the next open has not stamped yet)
    pub uid: String,
    /// v2.6: the public display name (`users.display_name`); `None` = not chosen yet (the user is asked at the next sign-in)
    pub display_name: Option<String>,
}

/// A session to store ([`Store::create_session`]).
#[derive(Debug, Clone, Copy)]
pub struct NewSession<'a> {
    pub token_hash: &'a str,
    pub user_id: i64,
    /// Unix seconds: the sign-in (`started_at` and `last_seen_at`)
    pub now: i64,
    pub expires_at: i64,
    pub remember: bool,
    /// `None` while the sign-in info switch is off
    pub ip: Option<&'a str>,
    pub ua: Option<&'a str>,
}

/// The stored facts of a session ([`Store::session_user_ro`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub user_id: i64,
    /// `None` = unknown (a session a v1.2 binary created, fixed at the first touch)
    pub started_at: Option<i64>,
    pub last_seen_at: i64,
    pub expires_at: i64,
    pub remember: bool,
    pub ip: Option<String>,
    pub ua: Option<String>,
}

/// What [`Store::touch_session`] writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Touch {
    /// Refresh the session row (`last_seen_at`, `expires_at`)
    pub session: bool,
    /// Set `users.last_active_at`
    pub active: bool,
    /// The new deadline (unix seconds) when `session`
    pub expires_at: i64,
}

/// v1.4: what a last-activity write also stores (see [`Store::touch_session_with`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActiveFacts {
    /// Cleaned User-Agent: set when present (written also while the eye is closed), else the stored one stays
    pub ua: Option<String>,
    /// `Some((ip, place))` only while the eye is open and the client address is known: the three place parts are
    /// written as given (NULL when unknown, so a row never mixes two places); `None` leaves IP and place unchanged
    pub ip_place: Option<(String, Place)>,
}

/// A place: country (code or name), region, city. Every part may be missing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Place {
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
}

impl Place {
    /// Nothing known.
    pub fn is_empty(&self) -> bool {
        self.country.is_none() && self.region.is_none() && self.city.is_none()
    }
}

/// A session deleted by [`Store::purge_expired_sessions_ended`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndedSession {
    pub user_id: i64,
    pub ip: Option<String>,
    pub ua: Option<String>,
    pub expires_at: i64,
    pub started_at: Option<i64>,
    pub last_seen_at: i64,
    pub remember: bool,
}

/// The sign-in info of one accepted sign-in ([`Store::note_signin`]); the caller passes it only while the sign-in info switch is open.
#[derive(Debug, Clone, Copy, Default)]
pub struct SigninInfo<'a> {
    pub ip: Option<&'a str>,
    pub country: Option<&'a str>,
    pub region: Option<&'a str>,
    pub city: Option<&'a str>,
}

/// `UPDATE`s that clear the stored sign-in IP and location of users and sessions (inside the caller's transaction);
/// returns the rows changed.
fn clear_signin_rows(c: &Connection) -> Result<i64, DbError> {
    // v1.4: the last-activity address and place go with the sign-in ones, in the same statement (so a user counts once);
    // the device, `last_active_ua`, is not an address and stays
    let users = c.execute(
        "UPDATE users SET last_login_ip=NULL, last_login_country=NULL, last_login_region=NULL, last_login_city=NULL,          last_active_ip=NULL, last_active_country=NULL, last_active_region=NULL, last_active_city=NULL          WHERE last_login_ip IS NOT NULL OR last_login_country IS NOT NULL OR last_login_region IS NOT NULL          OR last_login_city IS NOT NULL OR last_active_ip IS NOT NULL OR last_active_country IS NOT NULL          OR last_active_region IS NOT NULL OR last_active_city IS NOT NULL",
        [],
    )?;
    let sessions = c.execute("UPDATE sessions SET ip=NULL WHERE ip IS NOT NULL", [])?;
    Ok(i64::try_from(users + sessions).unwrap_or(i64::MAX))
}

/// Unix seconds as `YYYY-MM-DD HH:MM:SS` in the local time zone (the format of `last_login_at`, with seconds).
pub(super) fn format_unix_second(secs: i64) -> String {
    jiff::Timestamp::from_second(secs)
        .map(|t| super::super::timefmt::format_second(&t.to_zoned(jiff::tz::TimeZone::system())))
        .unwrap_or_default()
}

/// The signed-in user of a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionUser {
    pub id: i64,
    pub username: String,
    pub role: Role,
    pub must_change_password: bool,
    /// v2.6: the public display name; `None` = not chosen yet
    pub display_name: Option<String>,
    /// May import banks and update / delete their own: always true for administrators (whatever is stored), else the stored
    /// permission. Read from the database on every request, so a change applies at once.
    pub can_manage_banks: bool,
    /// May download the banks they can see: always true for administrators, else the stored permission. Read from the
    /// database on every request.
    pub can_download_banks: bool,
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
    /// The account must finish its setup before anything else: a pending password change, or no display name yet. Such a
    /// session may only reach the routes flagged `during_password_change` and is refused by every AI-internal call.
    pub fn setup_required(&self) -> bool {
        self.must_change_password || self.display_name.is_none()
    }
    /// `{"id", "username", "displayName", "needsName", "role", "mustChangePassword", "canManageBanks", "canDownloadBanks",
    /// "aiAllowed"}` (`GET /api/auth/state` and friends). The identity stamp is not part of it.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "username": self.username,
            "displayName": self.display_name,
            "needsName": self.display_name.is_none(),
            "role": self.role.as_str(),
            "mustChangePassword": self.must_change_password,
            "canManageBanks": self.can_manage_banks,
            "canDownloadBanks": self.can_download_banks,
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
            display_name: u.display_name.clone(),
            can_manage_banks: u.role == Role::Admin || u.can_manage_banks,
            can_download_banks: u.role == Role::Admin || u.can_download_banks,
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
    /// v1.6: the "can download question banks" permission
    pub can_download_banks: Option<bool>,
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
const USER_COLUMN_COUNT: usize = 14;

const USER_COLUMNS: &str = "id, username, password_hash, role, disabled, created_at, last_login_at, must_change_password, \
     new_banks, can_manage_banks, ai_enabled, uid, can_download_banks, display_name";

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
        can_download_banks: r.get::<_, i64>(12)? != 0,
        display_name: r.get(13)?,
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

/// Whether another user (not `except`) already uses this text as a login or as a display name (v2.6: logins and display
/// names are exclusive across all users, so nobody can sign up as, or be shown as, somebody else).
fn name_taken(c: &Connection, name: &str, except: i64) -> Result<bool, DbError> {
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE (username=?1 OR display_name=?1 COLLATE NOCASE) AND id<>?2)",
        params![name, except],
        |r| r.get::<_, bool>(0),
    )?)
}

/// Whether another user (not `except`) already has this display name or uses it as a login, ignoring ASCII letter case.
fn display_name_taken(c: &Connection, name: &str, except: i64) -> Result<bool, DbError> {
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE (display_name=?1 COLLATE NOCASE OR username=?1) AND id<>?2)",
        params![name, except],
        |r| r.get::<_, bool>(0),
    )?)
}

/// What [`Store::change_own_password_as`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasswordChange {
    /// Written; `old_name` is the display name before (when one was sent with the password)
    Done {
        user: SessionUser,
        old_name: Option<String>,
    },
    /// The user is gone or disabled, or the password hash is not the one the caller verified (nothing written)
    Stale,
    /// The display name sent with the password belongs to someone else (nothing written)
    NameTaken,
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
        "INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at, started_at) VALUES(?1,?2,?3,?4,?5,?4)",
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

/// `{"country", "region", "city"}` of a stored place; `null` when all three parts are missing (an external provider may
/// give no country, so a place without a country is still a place).
pub(super) fn location_json(
    country: Option<String>,
    region: Option<String>,
    city: Option<String>,
) -> Value {
    if country.is_none() && region.is_none() && city.is_none() {
        Value::Null
    } else {
        json!({ "country": country, "region": region, "city": city })
    }
}

/// `{"users": [...], "signinInfo": bool}` for `GET /api/users`, ordered by id.
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
    // v1.3: IP and location are shown only while the sign-in info switch is on (while it is off nothing new is stored)
    let signin_info = super::system_settings::flag(c, super::system_settings::SIGNIN_INFO)?;
    let mut st = c.prepare(&format!(
        "SELECT {USER_COLUMNS}, (SELECT COUNT(*) FROM attempts a WHERE a.user_id = users.id), \
         last_active_at, last_login_ip, last_login_country, last_login_region, last_login_city, \
         last_active_ip, last_active_country, last_active_region, last_active_city, last_active_ua FROM users ORDER BY id"
    ))?;
    let rows = st.query_map([], |r| {
        let geo = (
            r.get::<_, Option<String>>(USER_COLUMN_COUNT + 2)?,
            r.get::<_, Option<String>>(USER_COLUMN_COUNT + 3)?,
            r.get::<_, Option<String>>(USER_COLUMN_COUNT + 4)?,
            r.get::<_, Option<String>>(USER_COLUMN_COUNT + 5)?,
        );
        let active = (
            r.get::<_, Option<String>>(USER_COLUMN_COUNT + 6)?,
            r.get::<_, Option<String>>(USER_COLUMN_COUNT + 7)?,
            r.get::<_, Option<String>>(USER_COLUMN_COUNT + 8)?,
            r.get::<_, Option<String>>(USER_COLUMN_COUNT + 9)?,
        );
        Ok((
            user_row(r)?,
            r.get::<_, i64>(USER_COLUMN_COUNT)?,
            r.get::<_, Option<String>>(USER_COLUMN_COUNT + 1)?,
            geo,
            active,
            r.get::<_, Option<String>>(USER_COLUMN_COUNT + 10)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (u, records, last_active, (ip, country, region, city), active, active_ua) = row?;
        let (ip, location) = if signin_info {
            (ip, location_json(country, region, city))
        } else {
            (None, Value::Null)
        };
        // v1.4: the last activity address and place follow the same eye; the device is not an address and is always shown
        let (active_ip, active_location) = if signin_info {
            (active.0, location_json(active.1, active.2, active.3))
        } else {
            (None, Value::Null)
        };
        out.push(json!({
            "id": u.id,
            "username": u.username,
            "displayName": u.display_name,
            "role": u.role.as_str(),
            "disabled": u.disabled,
            "createdAt": u.created_at,
            "lastLoginAt": u.last_login_at,
            "mustChangePassword": u.must_change_password,
            "records": records,
            "banks": assigned.remove(&u.id).unwrap_or_default(),
            "newBanks": u.new_banks,
            "canManageBanks": u.can_manage_banks,
            "canDownloadBanks": u.can_download_banks,
            "ai": u.ai_enabled,
            "ownedBanks": owned.remove(&u.id).unwrap_or_default(),
            "lastActiveAt": last_active,
            "lastLoginIp": ip,
            "lastLoginLocation": location,
            "lastActiveIp": active_ip,
            "lastActiveLocation": active_location,
            "lastActiveUa": active_ua,
        }));
    }
    Ok(json!({ "users": out, "signinInfo": signin_info }))
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
                 u.last_login_at, u.must_change_password, u.new_banks, u.can_manage_banks, u.ai_enabled, u.uid, u.can_download_banks, u.display_name \
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
                        can_download_banks: r.get::<_, i64>(14)? != 0,
                        display_name: r.get(15)?,
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

    /// v1.3, read only: the session `token_hash` with its user (the user must be enabled). Nothing is written and an
    /// expired row is returned like any other (the caller applies the role-based deadline, see `SessionRow::expires_at`,
    /// then calls [`Store::touch_session`], [`Store::delete_session`] or lets the purge take it). Replaces
    /// [`Store::session_user`] once the auth layer switched.
    pub fn session_user_ro(
        &self,
        token_hash: &str,
    ) -> Result<Option<(SessionUser, SessionRow)>, DbError> {
        let g = self.lock();
        let found = g
            .query_row(
                "SELECT s.user_id, s.started_at, s.last_seen_at, s.expires_at, s.remember, s.ip, s.ua, \
                 u.id, u.username, u.password_hash, u.role, u.disabled, u.created_at, u.last_login_at, \
                 u.must_change_password, u.new_banks, u.can_manage_banks, u.ai_enabled, u.uid, u.can_download_banks, u.display_name \
                 FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token_hash=?1",
                params![token_hash],
                |r| {
                    let row = SessionRow {
                        user_id: r.get(0)?,
                        started_at: r.get(1)?,
                        last_seen_at: r.get(2)?,
                        expires_at: r.get(3)?,
                        remember: r.get::<_, i64>(4)? != 0,
                        ip: r.get(5)?,
                        ua: r.get(6)?,
                    };
                    let user = UserRow {
                        id: r.get(7)?,
                        username: r.get(8)?,
                        password_hash: r.get(9)?,
                        role: Role::parse(&r.get::<_, String>(10)?).unwrap_or(Role::User),
                        disabled: r.get::<_, i64>(11)? != 0,
                        created_at: r.get(12)?,
                        last_login_at: r.get(13)?,
                        must_change_password: r.get::<_, i64>(14)? != 0,
                        new_banks: r.get::<_, i64>(15)? != 0,
                        can_manage_banks: r.get::<_, i64>(16)? != 0,
                        ai_enabled: r.get::<_, i64>(17)? != 0,
                        uid: r.get(18)?,
                        can_download_banks: r.get::<_, i64>(19)? != 0,
                        display_name: r.get(20)?,
                    };
                    Ok((user, row))
                },
            )
            .optional()?;
        Ok(found
            .filter(|(u, _)| !u.disabled)
            .map(|(u, row)| (SessionUser::from(&u), row)))
    }

    /// v1.3: stores a new session and keeps at most [`MAX_SESSIONS_PER_USER`] of the user (the ones that expire first are
    /// deleted). `started_at` and `last_seen_at` are `now`. The caller verified the password and decides the deadline;
    /// `ip` is stored as given; the caller passes `None` while the sign-in info switch is closed.
    pub fn create_session(&self, s: &NewSession<'_>) -> Result<(), DbError> {
        let at = self.clock.now();
        let g = self.lock();
        run_tx(&g, |c| {
            ensure_active(c, s.user_id)?;
            c.execute(
                "INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at, started_at, remember, ip, ua) \
                 VALUES(?1,?2,?3,?4,?5,?4,?6,?7,?8)",
                params![
                    s.token_hash,
                    s.user_id,
                    at,
                    s.now,
                    s.expires_at,
                    s.remember,
                    s.ip,
                    s.ua
                ],
            )?;
            c.execute(
                "DELETE FROM sessions WHERE user_id=?1 AND token_hash NOT IN \
                 (SELECT token_hash FROM sessions WHERE user_id=?1 ORDER BY expires_at DESC, rowid DESC LIMIT ?2)",
                params![s.user_id, MAX_SESSIONS_PER_USER],
            )?;
            Ok(())
        })
    }

    /// v1.3: what the auth layer decided to write for one request, in one lock and one transaction: with `t.session` the
    /// session gets `last_seen_at = now` and `expires_at = t.expires_at` (and `started_at = its old last_seen_at` when it
    /// had none, a session a v1.2 binary created); with `t.active` `users.last_active_at` = `now`. Nothing is written when
    /// both are false.
    pub fn touch_session(
        &self,
        token_hash: &str,
        user_id: i64,
        now: i64,
        t: &Touch,
    ) -> Result<(), DbError> {
        self.touch_session_with(token_hash, user_id, now, t, None)
    }

    /// [`Store::touch_session`] that also stores, in the same transaction and only with `t.active`, the facts of the
    /// request (v1.4: `last_active_ua`, and `last_active_ip` with its place while the eye is open).
    pub fn touch_session_with(
        &self,
        token_hash: &str,
        user_id: i64,
        now: i64,
        t: &Touch,
        facts: Option<&ActiveFacts>,
    ) -> Result<(), DbError> {
        if !t.session && !t.active {
            return Ok(());
        }
        let g = self.lock();
        run_tx(&g, |c| {
            if t.session {
                c.execute(
                    "UPDATE sessions SET started_at=COALESCE(started_at, last_seen_at), last_seen_at=?1, expires_at=?2 \
                     WHERE token_hash=?3",
                    params![now, t.expires_at, token_hash],
                )?;
            }
            if t.active {
                c.execute(
                    "UPDATE users SET last_active_at=?1 WHERE id=?2",
                    params![format_unix_second(now), user_id],
                )?;
                if let Some(f) = facts {
                    if let Some(ua) = &f.ua {
                        c.execute(
                            "UPDATE users SET last_active_ua=?1 WHERE id=?2",
                            params![ua, user_id],
                        )?;
                    }
                    if let Some((ip, p)) = &f.ip_place {
                        c.execute(
                            "UPDATE users SET last_active_ip=?1, last_active_country=?2, last_active_region=?3, \
                             last_active_city=?4 WHERE id=?5",
                            params![ip, p.country, p.region, p.city, user_id],
                        )?;
                    }
                }
            }
            Ok(())
        })
    }

    /// v1.3: deletes one session (sign-out, or an ended one found by the lookup).
    pub fn delete_session(&self, token_hash: &str) -> Result<(), DbError> {
        self.end_session(token_hash)
    }

    /// v1.3: deletes every session that expired before `now` and returns what was deleted (for the activity log). Replaces
    /// [`Store::purge_expired_sessions`] once the auth layer switched.
    pub fn purge_expired_sessions_ended(&self, now: i64) -> Result<Vec<EndedSession>, DbError> {
        let g = self.lock();
        let mut st = g.prepare(
            "DELETE FROM sessions WHERE expires_at < ?1 \
             RETURNING user_id, ip, ua, expires_at, started_at, last_seen_at, remember",
        )?;
        let rows = st.query_map(params![now], |r| {
            Ok(EndedSession {
                user_id: r.get(0)?,
                ip: r.get(1)?,
                ua: r.get(2)?,
                expires_at: r.get(3)?,
                started_at: r.get(4)?,
                last_seen_at: r.get(5)?,
                remember: r.get::<_, i64>(6)? != 0,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// v1.3: a sign-in of `user_id` was accepted: `last_login_at` and `last_active_at` = `now`, and the sign-in info:
    /// `Some` = store it (a field the request lacked becomes NULL, so the record never mixes two sign-ins), `None` = the
    /// caller has none (the eye is closed: nothing new is recorded), the stored one is kept. The switch itself is read by the caller.
    pub fn note_signin(
        &self,
        user_id: i64,
        now: &str,
        info: Option<&SigninInfo<'_>>,
    ) -> Result<(), DbError> {
        self.note_signin_with_ua(user_id, now, info, None)
    }

    /// [`Store::note_signin`] that also sets (v1.4) the last-activity facts from the sign-in: `last_active_ua` = `ua`
    /// when given (always: the device is not an address), and with `info` the last-activity IP and place = the sign-in's.
    pub fn note_signin_with_ua(
        &self,
        user_id: i64,
        now: &str,
        info: Option<&SigninInfo<'_>>,
        ua: Option<&str>,
    ) -> Result<(), DbError> {
        let g = self.lock();
        run_tx(&g, |c| {
            match info {
                Some(i) => c.execute(
                    "UPDATE users SET last_login_at=?1, last_active_at=?1, last_login_ip=?2, last_login_country=?3, \
                     last_login_region=?4, last_login_city=?5, last_active_ip=?2, last_active_country=?3, \
                     last_active_region=?4, last_active_city=?5 WHERE id=?6",
                    params![now, i.ip, i.country, i.region, i.city, user_id],
                )?,
                None => c.execute(
                    "UPDATE users SET last_login_at=?1, last_active_at=?1 WHERE id=?2",
                    params![now, user_id],
                )?,
            };
            if let Some(ua) = ua {
                c.execute(
                    "UPDATE users SET last_active_ua=?1 WHERE id=?2",
                    params![ua, user_id],
                )?;
            }
            Ok(())
        })
    }

    /// v1.3: clears every stored sign-in IP and location of the users and sessions (the administrators' explicit
    /// "clear saved IP addresses and locations"; the activity log follows in batches, [`Store::clear_activity_ip`]).
    /// Returns the rows changed.
    pub fn clear_signin_info(&self) -> Result<i64, DbError> {
        let g = self.lock();
        run_tx(&g, clear_signin_rows)
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
        Ok(
            match self.change_own_password_as(
                user_id,
                expected_hash,
                new_hash,
                &NewSession {
                    token_hash: new_token_hash,
                    user_id,
                    now,
                    expires_at: now.saturating_add(SESSION_TTL_SECS),
                    remember: true,
                    ip: None,
                    ua: None,
                },
                None,
            )? {
                PasswordChange::Done { user, .. } => Some(user),
                PasswordChange::Stale | PasswordChange::NameTaken => None,
            },
        )
    }

    /// [`Store::change_own_password`] with the full new session (`s.user_id` is the user): the caller passes the old
    /// session's `remember`, the deadline of the user's role and the address / device, so a password change does not turn
    /// a browser session into a remember session. v2.6: `display_name` (already cleaned) is written in the same
    /// transaction; a taken name refuses the whole change before anything is written.
    pub fn change_own_password_as(
        &self,
        user_id: i64,
        expected_hash: &str,
        new_hash: &str,
        s: &NewSession<'_>,
        display_name: Option<&str>,
    ) -> Result<PasswordChange, DbError> {
        let at = self.clock.now();
        let g = self.lock();
        run_tx(&g, |c| {
            let Some(before) = user_by_id(c, user_id)? else {
                return Ok(PasswordChange::Stale);
            };
            if before.disabled || before.password_hash != expected_hash {
                return Ok(PasswordChange::Stale);
            }
            if let Some(name) = display_name {
                if display_name_taken(c, name, user_id)? {
                    return Ok(PasswordChange::NameTaken);
                }
            }
            let n = c.execute(
                "UPDATE users SET password_hash=?1, password_changed_at=?2, must_change_password=0                  WHERE id=?3 AND password_hash=?4 AND disabled=0",
                params![new_hash, at, user_id, expected_hash],
            )?;
            if n == 0 {
                return Ok(PasswordChange::Stale);
            }
            if let Some(name) = display_name {
                c.execute(
                    "UPDATE users SET display_name=?1 WHERE id=?2",
                    params![name, user_id],
                )?;
            }
            delete_sessions_of(c, user_id)?;
            c.execute(
                "INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at, started_at, remember, ip, ua)                  VALUES(?1,?2,?3,?4,?5,?4,?6,?7,?8)",
                params![s.token_hash, user_id, at, s.now, s.expires_at, s.remember, s.ip, s.ua],
            )?;
            Ok(
                match user_by_id(c, user_id)?.as_ref().map(SessionUser::from) {
                    Some(user) => PasswordChange::Done {
                        user,
                        old_name: before.display_name,
                    },
                    None => PasswordChange::Stale,
                },
            )
        })
    }

    /// `POST /api/auth/name` (v2.6): sets or changes the display name (already cleaned). `Done` carries the previous name
    /// (`None` = first time) and the user as the session now sees them. Uniqueness is checked inside the write
    /// transaction, case-insensitively (SQLite folds ASCII letters only, like `username`); another user's name is
    /// `Taken`, your own name in another letter case is allowed (it changes the capitals).
    pub fn set_display_name(
        &self,
        user_id: i64,
        name: &str,
    ) -> Result<UserChange<(Option<String>, SessionUser)>, DbError> {
        let g = self.lock();
        run_tx(&g, |c| {
            let Some(before) = user_by_id(c, user_id)?.filter(|u| !u.disabled) else {
                return Ok(UserChange::NotFound);
            };
            if display_name_taken(c, name, user_id)? {
                return Ok(UserChange::Taken);
            }
            c.execute(
                "UPDATE users SET display_name=?1 WHERE id=?2",
                params![name, user_id],
            )?;
            Ok(
                match user_by_id(c, user_id)?.as_ref().map(SessionUser::from) {
                    Some(user) => UserChange::Done((before.display_name, user)),
                    None => UserChange::NotFound,
                },
            )
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

    /// [`Store::create_user_full`] without the download permission.
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
        self.create_user_full(
            username,
            hash,
            role,
            banks,
            new_banks,
            can_manage_banks,
            ai,
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
    /// `can_manage_banks`, the AI+ switch `ai` (v0.8) and the download permission `can_download_banks` (v1.6), and a new random identity stamp. `username` is already cleaned
    /// ([`auth::clean_username`]) and `hash` computed. Returns the new id; [`UserChange::UnknownBanks`] when a bank id does
    /// not exist (nothing written).
    #[allow(clippy::too_many_arguments)]
    pub fn create_user_full(
        &self,
        username: &str,
        hash: &str,
        role: Role,
        banks: &BankAccess,
        new_banks: bool,
        can_manage_banks: bool,
        ai: bool,
        can_download_banks: bool,
    ) -> Result<UserChange<i64>, DbError> {
        let at = self.clock.now();
        let g = self.lock();
        run_tx(&g, |c| {
            if name_taken(c, username, 0)? {
                return Ok(UserChange::Taken);
            }
            c.execute(
                "INSERT INTO users(username, password_hash, role, disabled, created_at, password_changed_at, must_change_password, \
                 new_banks, can_manage_banks, ai_enabled, uid, can_download_banks) \
                 VALUES(?1, ?2, ?3, 0, ?4, ?4, 1, ?5, ?6, ?7, lower(hex(randomblob(16))), ?8)",
                params![
                    username,
                    hash,
                    role.as_str(),
                    at,
                    i64::from(new_banks),
                    i64::from(can_manage_banks),
                    i64::from(ai),
                    i64::from(can_download_banks)
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
                "UPDATE users SET username=?1, role=?2, disabled=?3, new_banks=?4, can_manage_banks=?5, ai_enabled=?6, can_download_banks=?7 \
                 WHERE id=?8",
                params![
                    username,
                    role.as_str(),
                    i64::from(disabled),
                    i64::from(change.new_banks.unwrap_or(u.new_banks)),
                    i64::from(change.can_manage_banks.unwrap_or(u.can_manage_banks)),
                    i64::from(ai),
                    i64::from(change.can_download_banks.unwrap_or(u.can_download_banks)),
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
