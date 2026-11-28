//! v3.1 devices: one row per browser per user (contracts A.2, A.7, B.3, B.5, B.7, C.13).
//!
//! - A device is the pair (user, `xexam_did` cookie); the database stores only the SHA-256 of the cookie value
//!   (`devices.cookie_hash`). A session belongs to a device (`sessions.device_id`), or to none (a session made before v3.1,
//!   by an older binary or by a test helper) until [`Store::device_link`] links it.
//! - A device row outlives its sessions: expiry and the 20-session cap do not sign it out (it can keep receiving pushes);
//!   an explicit sign-out sets `signed_out_at`, deletes the sessions and the push subscription. An unused device (no
//!   session, `active_at` older than 30 days) is deleted by [`Store::devices_housekeeping`]; one user keeps at most
//!   [`MAX_DEVICES_PER_USER`] rows.
//! - IP and place columns follow the sign-in info switch exactly like `users.last_*`: written only while the eye is open,
//!   the three place parts always together, the place by the `geo.source` rule.
//! - Delete order (every foreign key is `ON DELETE NO ACTION`): sessions, then the push subscription with its outbox rows
//!   and the saved-session markers (`store::push`), then the device.

use super::push::{push_remove_device, saved_delete_device};
use super::system_settings::{flag, get, GEO_SOURCE, SIGNIN_INFO};
use super::users::{ensure_active, format_unix_second, location_json, NewSession, SigninInfo};
use super::{run_tx, Store};
use crate::auth::{self, Role, SessionTimes, MAX_SESSIONS_PER_USER};
use crate::database::Place;
use crate::error::DbError;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;

/// A browser belongs to the user who signed in last: removes the push subscriptions of other users' devices that share
/// the browser cookie (their queued rows go first), so the previous account's pushes stop on this browser.
fn drop_other_users_subs(c: &Connection, cookie_hash: &str, user_id: i64) -> Result<(), DbError> {
    let ids: Vec<i64> = {
        let mut st = c.prepare("SELECT id FROM devices WHERE cookie_hash=?1 AND user_id<>?2")?;
        let rows = st.query_map(params![cookie_hash, user_id], |r| r.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for id in ids {
        push_remove_device(c, id)?;
    }
    Ok(())
}

/// At most this many device rows per user; the oldest unused ones without a session go first (D2).
pub const MAX_DEVICES_PER_USER: i64 = 20;
/// A device with no session is deleted when it was last active more than this long ago (30 days, D2).
pub const DEVICE_UNUSED_SECS: i64 = 30 * 86_400;
/// `devices.ua` is at most this many characters (the table CHECK).
const UA_MAX: usize = 256;

/// What a sign-in tells the device row ([`Store::create_session_on_device`]).
#[derive(Debug, Clone, Copy)]
pub struct DeviceSignin<'a> {
    /// SHA-256 (hex) of the `xexam_did` cookie value
    pub cookie_hash: &'a str,
    /// Cleaned User-Agent (written also while the eye is closed)
    pub ua: Option<&'a str>,
    /// `Some` only while the sign-in info eye is open; a field the request lacked becomes NULL (never mixed places)
    pub info: Option<SigninInfo<'a>>,
}

/// The result of [`Store::device_link`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Linked {
    pub device_id: i64,
    /// This call's `UPDATE` linked the session (the caller may send the cookie); false = it was linked already
    pub changed: bool,
}

/// Outcome of a change addressed to one device of a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceChange<T> {
    Done(T),
    /// No such device for that user (404)
    NotFound,
}

fn cut_ua(ua: &str) -> String {
    ua.chars().take(UA_MAX).collect()
}

/// The device of `user_id` with this cookie hash.
pub(super) fn find_device(
    c: &Connection,
    user_id: i64,
    cookie_hash: &str,
) -> Result<Option<i64>, DbError> {
    Ok(c.query_row(
        "SELECT id FROM devices WHERE user_id=?1 AND cookie_hash=?2",
        params![user_id, cookie_hash],
        |r| r.get(0),
    )
    .optional()?)
}

fn insert_device(
    c: &Connection,
    user_id: i64,
    cookie_hash: &str,
    now: i64,
    ua: Option<&str>,
) -> Result<i64, DbError> {
    let ua = ua.map(cut_ua);
    c.execute(
        "INSERT INTO devices(user_id, cookie_hash, created_at, active_at, ua) VALUES(?1,?2,?3,?3,?4)",
        params![user_id, cookie_hash, now, ua],
    )?;
    Ok(c.last_insert_rowid())
}

/// Deletes one device row child-first (A.7): its sessions, its push subscription and queued pushes, its saved-session
/// markers, then the row.
pub(super) fn delete_device(c: &Connection, device_id: i64) -> Result<(), DbError> {
    c.execute(
        "DELETE FROM sessions WHERE device_id=?1",
        params![device_id],
    )?;
    push_remove_device(c, device_id)?;
    saved_delete_device(c, device_id)?;
    c.execute("DELETE FROM devices WHERE id=?1", params![device_id])?;
    Ok(())
}

/// Keeps at most [`MAX_DEVICES_PER_USER`] rows of `user_id`: the oldest unused ones without a session go first, never
/// `keep`. Returns how many were deleted.
fn trim_devices(c: &Connection, user_id: i64, keep: Option<i64>) -> Result<usize, DbError> {
    let count: i64 = c.query_row(
        "SELECT COUNT(*) FROM devices WHERE user_id=?1",
        params![user_id],
        |r| r.get(0),
    )?;
    let excess = count - MAX_DEVICES_PER_USER;
    if excess <= 0 {
        return Ok(0);
    }
    let ids: Vec<i64> = {
        let mut st = c.prepare(
            "SELECT d.id FROM devices d WHERE d.user_id=?1 AND d.id IS NOT ?2 \
             AND NOT EXISTS(SELECT 1 FROM sessions s WHERE s.device_id = d.id) \
             ORDER BY d.active_at, d.id LIMIT ?3",
        )?;
        let rows = st.query_map(params![user_id, keep, excess], |r| r.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for id in &ids {
        delete_device(c, *id)?;
    }
    Ok(ids.len())
}

/// Signs one device out (A.7): its sessions are deleted, then its push subscription (with the queued pushes), then
/// `signed_out_at` is set (kept when it already is). Returns whether the device had a session or a subscription.
pub(super) fn signout_device_rows(
    c: &Connection,
    device_id: i64,
    now: i64,
) -> Result<bool, DbError> {
    let had_session: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE device_id=?1)",
        params![device_id],
        |r| r.get(0),
    )?;
    c.execute(
        "DELETE FROM sessions WHERE device_id=?1",
        params![device_id],
    )?;
    let had_sub = push_remove_device(c, device_id)?;
    c.execute(
        "UPDATE devices SET signed_out_at=COALESCE(signed_out_at, ?1) WHERE id=?2",
        params![now, device_id],
    )?;
    Ok(had_session || had_sub)
}

/// Signs out every device of `user_id` except `keep_device`, and every session of the user without a device except the
/// one whose token hash is `keep_token`. Returns how many devices had a session or a subscription.
pub(super) fn signout_user_devices(
    c: &Connection,
    user_id: i64,
    now: i64,
    keep_device: Option<i64>,
    keep_token: Option<&str>,
) -> Result<i64, DbError> {
    let ids: Vec<i64> = {
        let mut st = c.prepare("SELECT id FROM devices WHERE user_id=?1 ORDER BY id")?;
        let rows = st.query_map(params![user_id], |r| r.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let mut count = 0;
    for id in ids {
        if Some(id) == keep_device {
            continue;
        }
        if signout_device_rows(c, id, now)? {
            count += 1;
        }
    }
    // Sessions without a device (made before v3.1, or by an older binary)
    match keep_token {
        Some(t) => c.execute(
            "DELETE FROM sessions WHERE user_id=?1 AND device_id IS NULL AND token_hash<>?2",
            params![user_id, t],
        )?,
        None => c.execute(
            "DELETE FROM sessions WHERE user_id=?1 AND device_id IS NULL",
            params![user_id],
        )?,
    };
    Ok(count)
}

/// The facts a sign-in writes into a device row while the eye is open: address and place of the sign-in and of the last
/// activity (the three place parts are written together).
fn write_signin_facts(
    c: &Connection,
    device_id: i64,
    info: &SigninInfo<'_>,
) -> Result<(), DbError> {
    c.execute(
        "UPDATE devices SET signin_ip=?1, signin_country=?2, signin_region=?3, signin_city=?4, \
         active_ip=?1, active_country=?2, active_region=?3, active_city=?4 WHERE id=?5",
        params![info.ip, info.country, info.region, info.city, device_id],
    )?;
    Ok(())
}

/// One address with its place: ip, country, region, city.
type Where = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// One device row as the list needs it.
struct Raw {
    id: i64,
    ua: Option<String>,
    app: bool,
    created_at: i64,
    signin_at: Option<i64>,
    signin: Where,
    active_at: i64,
    active: Where,
    signed_out_at: Option<i64>,
    has_sub: bool,
}

/// One list entry: the `DeviceJson`, `active_at`, the id and whether it is the current device (for the sort).
type Entry = (Value, i64, i64, bool);

/// The `DeviceJson` rows of one user (B.3), unsorted; `only` limits it to one device.
fn device_rows(
    c: &Connection,
    user_id: i64,
    only: Option<i64>,
    now: i64,
    eye: bool,
    current: Option<i64>,
) -> Result<Vec<Entry>, DbError> {
    // The user: the role decides the session deadlines, `disabled` and the push switch decide the "push" flag
    let Some((role, disabled, push_on)): Option<(Role, bool, bool)> = c
        .query_row(
            "SELECT role, disabled, NOT EXISTS(SELECT 1 FROM user_settings s WHERE s.user_id = users.id \
             AND s.key = 'push' AND s.value = 'false') FROM users WHERE id=?1",
            params![user_id],
            |r| {
                Ok((
                    Role::parse(&r.get::<_, String>(0)?).unwrap_or(Role::User),
                    r.get::<_, i64>(1)? != 0,
                    r.get::<_, i64>(2)? != 0,
                ))
            },
        )
        .optional()?
    else {
        return Ok(Vec::new());
    };
    let mut st = c.prepare(
        "SELECT d.id, d.ua, d.app, d.created_at, d.signin_at, d.signin_ip, d.signin_country, d.signin_region, d.signin_city, \
         d.active_at, d.active_ip, d.active_country, d.active_region, d.active_city, d.signed_out_at, \
         EXISTS(SELECT 1 FROM push_subscriptions p WHERE p.device_id = d.id) \
         FROM devices d WHERE d.user_id=?1 AND (?2 IS NULL OR d.id=?2)",
    )?;
    let raws = st
        .query_map(params![user_id, only], |r| {
            Ok(Raw {
                id: r.get(0)?,
                ua: r.get(1)?,
                app: r.get::<_, i64>(2)? != 0,
                created_at: r.get(3)?,
                signin_at: r.get(4)?,
                signin: (r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?),
                active_at: r.get(9)?,
                active: (r.get(10)?, r.get(11)?, r.get(12)?, r.get(13)?),
                signed_out_at: r.get(14)?,
                has_sub: r.get::<_, i64>(15)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut sessions =
        c.prepare("SELECT started_at, last_seen_at, remember FROM sessions WHERE device_id=?1")?;
    let mut out = Vec::new();
    for d in raws {
        let times = sessions
            .query_map(params![d.id], |r| {
                let last: i64 = r.get(1)?;
                Ok(SessionTimes {
                    started_at: r.get::<_, Option<i64>>(0)?.unwrap_or(last),
                    last_seen_at: last,
                    remember: r.get::<_, i64>(2)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        // The deadline of the user's CURRENT role, exactly as the session lookup computes it
        let live = times
            .iter()
            .any(|t| auth::session_deadline(role, t).0 > now);
        let state = if live {
            "active"
        } else if d.signed_out_at.is_some() {
            "signedOut"
        } else {
            "expired"
        };
        let eligible = push_on
            && !disabled
            && d.signed_out_at.is_none()
            && d.active_at >= now.saturating_sub(DEVICE_UNUSED_SECS);
        let (signin_ip, signin_loc, active_ip, active_loc) = if eye {
            (
                d.signin.0,
                location_json(d.signin.1, d.signin.2, d.signin.3),
                d.active.0,
                location_json(d.active.1, d.active.2, d.active.3),
            )
        } else {
            (None, Value::Null, None, Value::Null)
        };
        let is_current = current == Some(d.id);
        let json = json!({
            "id": d.id,
            "current": is_current,
            "state": state,
            "ua": d.ua,
            "app": d.app,
            "createdAt": format_unix_second(d.created_at),
            "signinAt": d.signin_at.map(format_unix_second),
            "signinIp": signin_ip,
            "signinLocation": signin_loc,
            "activeAt": format_unix_second(d.active_at),
            "activeIp": active_ip,
            "activeLocation": active_loc,
            "push": d.has_sub && eligible,
        });
        out.push((json, d.active_at, d.id, is_current));
    }
    Ok(out)
}

impl Store {
    /// The `DeviceJson` of one device of `user_id` (B.3); `current` is the requesting session's device id. `Null` when
    /// the user has no such device. The sign-in info eye is read inside.
    pub fn device_json(
        &self,
        user_id: i64,
        device_id: i64,
        current: Option<i64>,
    ) -> Result<Value, DbError> {
        let now = self.unix_now();
        let g = self.lock();
        let eye = flag(&g, SIGNIN_INFO)?;
        let rows = device_rows(&g, user_id, Some(device_id), now, eye, current)?;
        Ok(rows
            .into_iter()
            .next()
            .map(|(v, ..)| v)
            .unwrap_or(Value::Null))
    }

    /// `{"devices": [DeviceJson], "signinInfo": bool}` of `user_id`: the current device first, then `active_at` newest
    /// first, then the highest id first.
    pub fn devices_json(&self, user_id: i64, current: Option<i64>) -> Result<Value, DbError> {
        let now = self.unix_now();
        let g = self.lock();
        let eye = flag(&g, SIGNIN_INFO)?;
        let mut rows = device_rows(&g, user_id, None, now, eye, current)?;
        rows.sort_by(|a, b| {
            b.3.cmp(&a.3)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| b.2.cmp(&a.2))
        });
        let list: Vec<Value> = rows.into_iter().map(|(v, ..)| v).collect();
        Ok(json!({ "devices": list, "signinInfo": eye }))
    }

    /// The number of device rows of every user that has any (`"devices"` of `GET /api/users`).
    pub fn device_counts(&self) -> Result<HashMap<i64, i64>, DbError> {
        let g = self.lock();
        let mut st = g.prepare("SELECT user_id, COUNT(*) FROM devices GROUP BY user_id")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        Ok(rows.collect::<Result<HashMap<_, _>, _>>()?)
    }

    /// A sign-in, in ONE transaction: finds or creates the device of (`s.user_id`, cookie), sets its sign-in facts
    /// (`signin_at = active_at = s.now`, `signed_out_at` cleared, the User-Agent, and with the eye open the address and
    /// place), stores the session on it (`s.device_id` is ignored), keeps at most [`MAX_SESSIONS_PER_USER`] sessions and
    /// [`MAX_DEVICES_PER_USER`] devices (never the one just used). Returns the device id. [`DbError::NoUser`] when the
    /// user was deleted or disabled meanwhile.
    pub fn create_session_on_device(
        &self,
        s: &NewSession<'_>,
        d: &DeviceSignin<'_>,
    ) -> Result<i64, DbError> {
        let at = self.clock.now();
        let g = self.lock();
        run_tx(&g, |c| {
            ensure_active(c, s.user_id)?;
            let id = match find_device(c, s.user_id, d.cookie_hash)? {
                Some(id) => id,
                None => insert_device(c, s.user_id, d.cookie_hash, s.now, d.ua)?,
            };
            c.execute(
                "UPDATE devices SET signin_at=?1, active_at=?1, signed_out_at=NULL, ua=COALESCE(?2, ua) WHERE id=?3",
                params![s.now, d.ua.map(cut_ua), id],
            )?;
            if let Some(info) = &d.info {
                write_signin_facts(c, id, info)?;
            }
            drop_other_users_subs(c, d.cookie_hash, s.user_id)?;
            c.execute(
                "INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at, started_at, remember, ip, ua, device_id) \
                 VALUES(?1,?2,?3,?4,?5,?4,?6,?7,?8,?9)",
                params![
                    s.token_hash,
                    s.user_id,
                    at,
                    s.now,
                    s.expires_at,
                    s.remember,
                    s.ip,
                    s.ua,
                    id
                ],
            )?;
            c.execute(
                "DELETE FROM sessions WHERE user_id=?1 AND token_hash NOT IN \
                 (SELECT token_hash FROM sessions WHERE user_id=?1 ORDER BY expires_at DESC, rowid DESC LIMIT ?2)",
                params![s.user_id, MAX_SESSIONS_PER_USER],
            )?;
            trim_devices(c, s.user_id, Some(id))?;
            Ok(id)
        })
    }

    /// Links a session that has no device (B.7 "Linking") to the device of (`user_id`, `cookie_hash`). With `create` the
    /// device is created when it does not exist (written with `now` and `ua`); without it `Ok(None)` is the answer for an
    /// unknown cookie. The link is `UPDATE .. WHERE device_id IS NULL`, so only one of two racing requests changes the
    /// row (`changed`); the other reads the device the row has. `Ok(None)` also when the session is gone.
    pub fn device_link(
        &self,
        token_hash: &str,
        user_id: i64,
        cookie_hash: &str,
        create: bool,
        now: i64,
        ua: Option<&str>,
    ) -> Result<Option<Linked>, DbError> {
        let g = self.lock();
        run_tx(&g, |c| {
            let row: Option<Option<i64>> = c
                .query_row(
                    "SELECT device_id FROM sessions WHERE token_hash=?1 AND user_id=?2",
                    params![token_hash, user_id],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(linked) = row else {
                return Ok(None);
            };
            if let Some(device_id) = linked {
                return Ok(Some(Linked {
                    device_id,
                    changed: false,
                }));
            }
            let device = match find_device(c, user_id, cookie_hash)? {
                Some(id) => id,
                None if create => insert_device(c, user_id, cookie_hash, now, ua)?,
                None => return Ok(None),
            };
            let n = c.execute(
                "UPDATE sessions SET device_id=?1 WHERE token_hash=?2 AND device_id IS NULL",
                params![device, token_hash],
            )?;
            if n == 0 {
                // Lost the race: the row has a device now
                let id: Option<i64> = c
                    .query_row(
                        "SELECT device_id FROM sessions WHERE token_hash=?1",
                        params![token_hash],
                        |r| r.get(0),
                    )
                    .optional()?
                    .flatten();
                return Ok(id.map(|device_id| Linked {
                    device_id,
                    changed: false,
                }));
            }
            // A live session on the device: it is signed in
            c.execute(
                "UPDATE devices SET signed_out_at=NULL WHERE id=?1",
                params![device],
            )?;
            drop_other_users_subs(c, cookie_hash, user_id)?;
            trim_devices(c, user_id, Some(device))?;
            Ok(Some(Linked {
                device_id: device,
                changed: true,
            }))
        })
    }

    /// `POST /api/devices/hello`: stores the zone and the installed-app flag of the device and, with `remove_push`
    /// (the browser no longer grants the notification permission), removes the device's push subscription. Returns
    /// whether a subscription was removed. `NotFound` when the device is not the user's.
    pub fn device_hello(
        &self,
        user_id: i64,
        device_id: i64,
        tz: Option<&str>,
        app: bool,
        remove_push: bool,
    ) -> Result<DeviceChange<bool>, DbError> {
        let g = self.lock();
        run_tx(&g, |c| {
            let n = c.execute(
                "UPDATE devices SET tz=?1, app=?2 WHERE id=?3 AND user_id=?4",
                params![tz, i64::from(app), device_id, user_id],
            )?;
            if n == 0 {
                return Ok(DeviceChange::NotFound);
            }
            let removed = if remove_push {
                push_remove_device(c, device_id)?
            } else {
                false
            };
            Ok(DeviceChange::Done(removed))
        })
    }

    /// Signs one device of `user_id` out (A.7). Returns whether it had a session or a subscription.
    pub fn device_signout(
        &self,
        user_id: i64,
        device_id: i64,
    ) -> Result<DeviceChange<bool>, DbError> {
        let now = self.unix_now();
        let g = self.lock();
        run_tx(&g, |c| {
            let own: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1 AND user_id=?2)",
                params![device_id, user_id],
                |r| r.get(0),
            )?;
            if !own {
                return Ok(DeviceChange::NotFound);
            }
            Ok(DeviceChange::Done(signout_device_rows(c, device_id, now)?))
        })
    }

    /// Signs out every device of `user_id` except `keep_device`, and every device-less session except the one with the
    /// hash `keep_token`. Returns how many devices had a session or a subscription.
    pub fn devices_signout_user(
        &self,
        user_id: i64,
        keep_device: Option<i64>,
        keep_token: Option<&str>,
    ) -> Result<i64, DbError> {
        let now = self.unix_now();
        let g = self.lock();
        run_tx(&g, |c| {
            signout_user_devices(c, user_id, now, keep_device, keep_token)
        })
    }

    /// Sign-out of one session: the session is deleted, then its device is signed out (the device's other sessions,
    /// its subscription and queued pushes go too, `signed_out_at` is set). A session without a device only disappears.
    pub fn logout_session(&self, token_hash: &str) -> Result<(), DbError> {
        let now = self.unix_now();
        let g = self.lock();
        run_tx(&g, |c| {
            let device: Option<i64> = c
                .query_row(
                    "SELECT device_id FROM sessions WHERE token_hash=?1",
                    params![token_hash],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            c.execute(
                "DELETE FROM sessions WHERE token_hash=?1",
                params![token_hash],
            )?;
            if let Some(d) = device {
                signout_device_rows(c, d, now)?;
            }
            Ok(())
        })
    }

    /// Hourly clean-up (C.13): deletes devices without a session row that were last active more than 30 days ago, then
    /// per user the oldest unused devices without a session beyond [`MAX_DEVICES_PER_USER`]. Returns the number deleted.
    pub fn devices_housekeeping(&self, now: i64) -> Result<usize, DbError> {
        let cutoff = now.saturating_sub(DEVICE_UNUSED_SECS);
        let g = self.lock();
        let deleted = run_tx(&g, |c| {
            let old: Vec<i64> = {
                let mut st = c.prepare(
                    "SELECT d.id FROM devices d WHERE d.active_at < ?1 \
                     AND NOT EXISTS(SELECT 1 FROM sessions s WHERE s.device_id = d.id) ORDER BY d.id",
                )?;
                let rows = st.query_map(params![cutoff], |r| r.get(0))?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            let mut n = 0;
            for id in old {
                delete_device(c, id)?;
                n += 1;
            }
            let crowded: Vec<i64> = {
                let mut st = c.prepare(
                    "SELECT user_id FROM devices GROUP BY user_id HAVING COUNT(*) > ?1 ORDER BY user_id",
                )?;
                let rows = st.query_map(params![MAX_DEVICES_PER_USER], |r| r.get(0))?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            for user in crowded {
                n += trim_devices(c, user, None)?;
            }
            Ok(n)
        })?;
        Ok(deleted)
    }

    /// Writes the place of an address into the sign-in / activity columns of the given devices (like `geo_apply`): nothing
    /// for an empty place, with the eye closed or when the place source is not `api` (both read inside the transaction);
    /// a device is only filled while its stored address is still `ip` and the three place parts are still NULL.
    /// Returns whether a row changed.
    pub fn geo_apply_devices(
        &self,
        ip: &str,
        place: &Place,
        _since: i64,
        signin: &[i64],
        active: &[i64],
    ) -> Result<bool, DbError> {
        if place.is_empty() || (signin.is_empty() && active.is_empty()) {
            return Ok(false);
        }
        let g = self.lock();
        run_tx(&g, |c| {
            if !flag(c, SIGNIN_INFO)? || get(c, GEO_SOURCE)?.as_deref() != Some("api") {
                return Ok(false);
            }
            let mut changed = 0usize;
            for id in signin {
                changed += c.execute(
                    "UPDATE devices SET signin_country=?1, signin_region=?2, signin_city=?3 \
                     WHERE id=?4 AND signin_ip=?5 AND signin_country IS NULL AND signin_region IS NULL \
                     AND signin_city IS NULL",
                    params![place.country, place.region, place.city, id, ip],
                )?;
            }
            for id in active {
                changed += c.execute(
                    "UPDATE devices SET active_country=?1, active_region=?2, active_city=?3 \
                     WHERE id=?4 AND active_ip=?5 AND active_country IS NULL AND active_region IS NULL \
                     AND active_city IS NULL",
                    params![place.country, place.region, place.city, id, ip],
                )?;
            }
            Ok(changed > 0)
        })
    }
}
