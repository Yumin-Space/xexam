//! v3.1 web push storage: subscriptions, the outbox, saved sessions, the triggers, the lease and the results (contracts C.2,
//! C.5, C.8, B.8, B.9).
//!
//! - **Subscriptions**: one per device (`device_id UNIQUE`), the endpoint is globally unique and moves to the device (and
//!   user) that confirms it last ([`Store::push_subscribe`]). Every delete path removes the outbox rows first (all foreign
//!   keys are `ON DELETE NO ACTION`).
//! - **Outbox**: queue and seven-day delivery log in one table. Enqueue coalesces with a queued row of the same
//!   subscription and topic; the lease ([`Store::push_lease`]) re-checks everything that may have changed since the enqueue,
//!   applies the quiet hours and encrypts the message; results ([`Store::push_result`]) are accepted only for the row, the
//!   lease and the try number they belong to.
//! - **Triggers**: bank tick (new and updated banks), reminder tick (unfinished practice / exam), version check (start-up),
//!   grants (called by the users store inside its transaction), and the test notification.
//! - **Saved sessions**: the server-side marker of an unfinished practice / exam that drives the reminders.
//!
//! Every method takes the time as a parameter (the callers read the store clock), so tests pin it.

use super::users::ensure_active;
use super::{get_meta, invalid_input, Store};
use crate::error::DbError;
use crate::push::{ece, quiet, texts, vapid};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// A reminder is due this long after the last save (seconds), and each next one this long after the previous (D10).
pub const REMIND_AFTER_SECS: i64 = 3 * 3600;
/// At most this many reminders per saved session.
pub const MAX_REMINDERS: i64 = 3;
/// How long a leased row stays leased before it is handed out again (seconds).
pub const LEASE_SECS: i64 = 120;
/// Most jobs of one lease call.
pub const LEASE_BATCH: usize = 20;
/// A device unused for this long stops receiving pushes (seconds).
pub const DEVICE_IDLE_SECS: i64 = 30 * 86_400;
/// A saved-session report within this many seconds of the last full write only refreshes the counts.
pub const SAVED_THROTTLE_SECS: i64 = 60;
/// A saved session nobody finished is forgotten after this long (seconds).
pub const SAVED_KEEP_SECS: i64 = 14 * 86_400;
/// At most this many push subscriptions per user.
pub const SUBSCRIPTIONS_PER_USER: usize = 10;
/// A re-confirm of the same endpoint on the same device within this long writes nothing (seconds).
pub const RECONFIRM_SECS: i64 = 86_400;
/// Finished outbox rows are kept this long (the delivery log, seconds).
pub const OUTBOX_KEEP_SECS: i64 = 7 * 86_400;
/// Rows handled per statement of the housekeeping (the lock is released between batches).
pub const HOUSEKEEPING_BATCH: i64 = 500;
/// Gives up after this many tries (a `retry` result at the fifth try fails the row).
pub const MAX_TRIES: i64 = 5;
/// Longest name inside a notification (characters).
pub const NAME_MAX_CHARS: usize = 60;

const TTL_REMIND: i64 = 6 * 3600;
const TTL_BANK: i64 = 3 * 86_400;
const TTL_VERSION: i64 = 3 * 86_400;
const TTL_TEST: i64 = 600;
const MAX_PAYLOAD_BYTES: usize = 1024;
/// Candidates looked at per lease call (rows that fail a re-check do not count against the batch).
const LEASE_SCAN: i64 = 200;

// ---------------------------------------------------------------- public shapes

/// What a client sent to `POST /api/push/subscribe`, validated and canonical (base64url without padding).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushSubscribe {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub key_id: String,
    /// `PushSubscription.expirationTime` / 1000, rounded down
    pub expires_at: Option<i64>,
}

/// What [`Store::push_subscribe`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushSubscribed {
    /// The account's notification switch is off: nothing was stored
    Off,
    /// The device is not one of the user's
    NoDevice,
    /// The same endpoint was confirmed again within a day: nothing was written
    Unchanged,
    /// The same endpoint was confirmed again (after a day, or with other keys)
    Confirmed,
    /// A new subscription for this device (also an endpoint moved here from another device)
    New,
}

/// What [`Store::push_test`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushTestOutcome {
    Off,
    NoDevice,
    Busy,
    Queued,
}

/// A saved-session report of `POST /api/study/saved`, already shape-checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedReport {
    pub key: String,
    /// `exam` or `practice`
    pub kind: String,
    pub answered: i64,
    pub total: i64,
    pub started_at: i64,
    pub time_left: Option<i64>,
}

/// One message handed to the courier ([`Store::push_lease`]).
#[derive(Clone, PartialEq, Eq)]
pub struct PushJob {
    pub id: i64,
    pub tries: i64,
    pub endpoint: String,
    pub ttl: i64,
    pub urgency: String,
    pub topic: Option<String>,
    pub authorization: String,
    /// The encrypted record, base64url without padding
    pub body: String,
}

impl std::fmt::Debug for PushJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the endpoint, the Authorization value or the body
        write!(f, "PushJob {{ id: {}, tries: {} }}", self.id, self.tries)
    }
}

/// What the push service answered, as the courier reports it (contracts C.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOutcome {
    Sent,
    Gone,
    Auth,
    Retry,
    Deferred,
    Rejected,
}

impl PushOutcome {
    pub fn parse(s: &str) -> Option<PushOutcome> {
        Some(match s {
            "sent" => PushOutcome::Sent,
            "gone" => PushOutcome::Gone,
            "auth" => PushOutcome::Auth,
            "retry" => PushOutcome::Retry,
            "deferred" => PushOutcome::Deferred,
            "rejected" => PushOutcome::Rejected,
            _ => return None,
        })
    }
}

/// One entry of `POST /internal/ai/push/result`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PushJobResult {
    pub id: i64,
    pub tries: i64,
    pub outcome: PushOutcome,
    pub status: Option<i64>,
    pub retry_after: Option<i64>,
}

/// What [`Store::push_result`] did with one result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushResultEffect {
    /// The result matched a leased row (otherwise it was late or a duplicate and is ignored)
    pub applied: bool,
    /// The host of the subscription's endpoint (for the `system.pushAuth` record)
    pub host: Option<String>,
    /// The subscription was deleted by this result
    pub subscription_deleted: bool,
}

/// What [`Store::push_version_check`] queued: `(from, to, devices)`.
pub type PushVersionPush = (Option<String>, String, usize);

/// What one housekeeping run removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushHousekeeping {
    pub expired: usize,
    pub deleted: usize,
    pub subscriptions: usize,
    pub saved: usize,
}

/// Seconds to wait before the next try after the failed attempt `tries` (1 → 30 s, 2 → 120 s, 3 → 600 s, 4 and more → 3600 s)
/// plus `jitter_pct` percent (0..=20).
pub fn push_backoff_secs(tries: i64, jitter_pct: u32) -> i64 {
    let base: i64 = match tries {
        i64::MIN..=1 => 30,
        2 => 120,
        3 => 600,
        _ => 3600,
    };
    base + base * i64::from(jitter_pct.min(20)) / 100
}

fn jitter_pct() -> u32 {
    let mut b = [0u8; 1];
    if getrandom::fill(&mut b).is_err() {
        return 0;
    }
    u32::from(b[0]) % 21
}

/// Dotted decimal version numbers of any length (`None` when a part is not a number).
fn version_parts(v: &str) -> Option<Vec<u64>> {
    v.trim()
        .split('.')
        .map(|p| p.parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()
}

/// `a` is a higher version than `b`: dotted decimal numbers, missing parts count as 0 (`2.3.1` > `2.3`, `3.10` > `3.9`);
/// a part that is not a number makes the comparison "not greater".
pub fn version_greater(a: &str, b: &str) -> bool {
    let (Some(a), Some(b)) = (version_parts(a), version_parts(b)) else {
        return false;
    };
    let n = a.len().max(b.len());
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

/// The host of an `https://` endpoint (lower case, no port), for logs and the per-host record.
pub fn endpoint_host(endpoint: &str) -> Option<String> {
    let rest = endpoint.strip_prefix("https://")?;
    let auth = rest.split('/').next()?;
    let host = auth.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// A name cut to [`NAME_MAX_CHARS`] characters, `…` appended when it was cut.
pub fn cut_name(name: &str) -> String {
    if name.chars().count() <= NAME_MAX_CHARS {
        name.to_string()
    } else {
        let mut s: String = name.chars().take(NAME_MAX_CHARS).collect();
        s.push('…');
        s
    }
}

// ---------------------------------------------------------------- small helpers

fn set_meta_if_changed(c: &Connection, key: &str, value: &str) -> Result<(), DbError> {
    if get_meta(c, key)?.as_deref() == Some(value) {
        return Ok(());
    }
    c.execute(
        "INSERT INTO meta(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn meta_i64(c: &Connection, key: &str) -> Result<Option<i64>, DbError> {
    Ok(get_meta(c, key)?.and_then(|v| v.trim().parse::<i64>().ok()))
}

/// The stored text of a user setting, parsed as a JSON string value.
fn setting_string(c: &Connection, user: i64, key: &str) -> Result<Option<String>, DbError> {
    let text: Option<String> = c
        .query_row(
            "SELECT value FROM user_settings WHERE user_id=?1 AND key=?2",
            params![user, key],
            |r| r.get(0),
        )
        .optional()?;
    Ok(text
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_str().map(str::to_string)))
}

/// The user's `tz` setting when it names a zone (`auto` and anything invalid: `None`).
fn user_tz(c: &Connection, user: i64) -> Result<Option<String>, DbError> {
    Ok(setting_string(c, user, crate::models::SETTING_TZ)?
        .filter(|s| s != "auto" && crate::models::tz_name_ok(s)))
}

/// The language of the user's notifications (`en` when missing or invalid).
fn ui_lang(c: &Connection, user: i64) -> Result<String, DbError> {
    Ok(setting_string(c, user, crate::models::SETTING_UI_LANG)?
        .filter(|s| matches!(s.as_str(), "en" | "zh" | "ja"))
        .unwrap_or_else(|| "en".to_string()))
}

/// The account notification switch (a missing row means on).
fn push_switch_on(c: &Connection, user: i64) -> Result<bool, DbError> {
    let off: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM user_settings WHERE user_id=?1 AND key='push' AND value='false')",
        params![user],
        |r| r.get(0),
    )?;
    Ok(!off)
}

/// A subscription that may receive pushes now (the eligibility rule of C.8, without the key check).
struct SubRef {
    sub_id: i64,
    device_tz: Option<String>,
}

const ELIGIBLE: &str = "u.disabled = 0 \
    AND NOT EXISTS (SELECT 1 FROM user_settings us WHERE us.user_id = u.id AND us.key = 'push' AND us.value = 'false') \
    AND d.signed_out_at IS NULL AND d.active_at >= ?2";

fn eligible_subs(c: &Connection, user: i64, now: i64) -> Result<Vec<SubRef>, DbError> {
    let sql = format!(
        "SELECT s.id, d.tz FROM push_subscriptions s JOIN devices d ON d.id = s.device_id \
         JOIN users u ON u.id = d.user_id WHERE u.id = ?1 AND {ELIGIBLE} ORDER BY s.id"
    );
    let mut st = c.prepare(&sql)?;
    let rows = st.query_map(params![user, now - DEVICE_IDLE_SECS], |r| {
        Ok(SubRef {
            sub_id: r.get(0)?,
            device_tz: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn eligible_sub_of_device(
    c: &Connection,
    device: i64,
    now: i64,
) -> Result<Option<SubRef>, DbError> {
    let sql = format!(
        "SELECT s.id, d.tz FROM push_subscriptions s JOIN devices d ON d.id = s.device_id \
         JOIN users u ON u.id = d.user_id WHERE d.id = ?1 AND {ELIGIBLE}"
    );
    Ok(
        c.query_row(&sql, params![device, now - DEVICE_IDLE_SECS], |r| {
            Ok(SubRef {
                sub_id: r.get(0)?,
                device_tz: r.get(1)?,
            })
        })
        .optional()?,
    )
}

/// Users that have at least one eligible subscription.
fn users_with_subscriptions(c: &Connection, now: i64) -> Result<Vec<i64>, DbError> {
    let sql = format!(
        "SELECT DISTINCT u.id FROM push_subscriptions s JOIN devices d ON d.id = s.device_id \
         JOIN users u ON u.id = d.user_id WHERE ?1 = ?1 AND {ELIGIBLE} ORDER BY u.id"
    );
    let mut st = c.prepare(&sql)?;
    let rows = st.query_map(params![now, now - DEVICE_IDLE_SECS], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Deletes a subscription: its outbox rows first, then the row itself.
fn delete_subscription(c: &Connection, sub_id: i64) -> Result<(), DbError> {
    c.execute(
        "DELETE FROM push_outbox WHERE subscription_id=?1",
        params![sub_id],
    )?;
    c.execute(
        "DELETE FROM push_subscriptions WHERE id=?1",
        params![sub_id],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- the queue

/// A message to queue (language-neutral payload; the text is rendered at lease time).
struct Msg {
    kind: &'static str,
    topic: String,
    payload: Value,
    urgency: &'static str,
    ttl: i64,
}

fn remind_msg(kind: &str, answered: i64, total: i64, n: i64, key: &str) -> Msg {
    Msg {
        kind: "remind",
        topic: "remind".to_string(),
        payload: json!({"k": "remind", "kind": kind, "answered": answered, "total": total, "n": n, "key": key}),
        urgency: "normal",
        ttl: TTL_REMIND,
    }
}

fn bank_new_msg(bank: i64, name: &str, total: i64) -> Msg {
    Msg {
        kind: "bank",
        topic: format!("bank-{bank}"),
        payload: json!({"k": "bank.new", "bank": bank, "name": cut_name(name), "total": total}),
        urgency: "low",
        ttl: TTL_BANK,
    }
}

fn bank_updated_msg(bank: i64, name: &str, added: i64, total: i64) -> Msg {
    Msg {
        kind: "bank",
        topic: format!("bank-{bank}"),
        payload: json!({"k": "bank.updated", "bank": bank, "name": cut_name(name), "added": added, "total": total}),
        urgency: "low",
        ttl: TTL_BANK,
    }
}

fn bank_many_msg(n: i64) -> Msg {
    Msg {
        kind: "bank",
        topic: "bank-access".to_string(),
        payload: json!({"k": "bank.grantMany", "n": n}),
        urgency: "low",
        ttl: TTL_BANK,
    }
}

fn version_msg(from: &str, to: &str) -> Msg {
    Msg {
        kind: "version",
        topic: "version".to_string(),
        payload: json!({"k": "version", "from": from, "to": to}),
        urgency: "low",
        ttl: TTL_VERSION,
    }
}

fn test_msg() -> Msg {
    Msg {
        kind: "test",
        topic: "test".to_string(),
        payload: json!({"k": "test"}),
        urgency: "high",
        ttl: TTL_TEST,
    }
}

/// The payload of a coalesced row: a newer `bank.updated` adds its `added` to the queued one's and takes the newest `total`
/// and `name`; one that meets a queued `bank.new` keeps being "new" with the newest total; every other kind replaces.
fn merge_payload(old: &Value, new: &Value) -> Value {
    let kind = |v: &Value| v.get("k").and_then(Value::as_str).unwrap_or("").to_string();
    let num = |v: &Value, k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0);
    if kind(new) != "bank.updated" {
        return new.clone();
    }
    match kind(old).as_str() {
        "bank.updated" => {
            let mut v = new.clone();
            v["added"] = json!(num(old, "added") + num(new, "added"));
            v
        }
        "bank.new" => {
            let mut v = old.clone();
            v["name"] = new.get("name").cloned().unwrap_or(Value::Null);
            v["total"] = json!(num(new, "total"));
            v
        }
        _ => new.clone(),
    }
}

/// Queues `m` for one subscription at `next_at` (or merges it into the queued row of the same topic).
fn enqueue(c: &Connection, sub_id: i64, m: &Msg, now: i64, next_at: i64) -> Result<(), DbError> {
    let expires = now.max(next_at) + m.ttl;
    let queued: Option<(i64, String, i64, i64)> = c
        .query_row(
            "SELECT id, payload, expires_at, next_at FROM push_outbox \
             WHERE subscription_id=?1 AND topic=?2 AND state='queued' ORDER BY id DESC LIMIT 1",
            params![sub_id, m.topic],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let payload = match &queued {
        Some((_, old, _, _)) => {
            let old: Value = serde_json::from_str(old).unwrap_or(Value::Null);
            merge_payload(&old, &m.payload)
        }
        None => m.payload.clone(),
    };
    let text = payload.to_string();
    if text.len() > MAX_PAYLOAD_BYTES {
        return Err(invalid_input("push payload too long"));
    }
    match queued {
        Some((id, _, old_expires, old_next)) => {
            c.execute(
                "UPDATE push_outbox SET kind=?1, payload=?2, urgency=?3, expires_at=?4, next_at=?5 WHERE id=?6",
                params![
                    m.kind,
                    text,
                    m.urgency,
                    old_expires.max(expires),
                    old_next.min(next_at),
                    id
                ],
            )?;
        }
        None => {
            c.execute(
                "INSERT INTO push_outbox(subscription_id, kind, topic, payload, urgency, created_at, expires_at, state, next_at, tries) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'queued', ?8, 0)",
                params![sub_id, m.kind, m.topic, text, m.urgency, now, expires, next_at],
            )?;
        }
    }
    Ok(())
}

/// Queues `m` for every eligible subscription of `user`, each held to the end of its quiet hours. Returns how many.
fn enqueue_for_user(c: &Connection, user: i64, m: &Msg, now: i64) -> Result<usize, DbError> {
    let subs = eligible_subs(c, user, now)?;
    if subs.is_empty() {
        return Ok(0);
    }
    let tz = user_tz(c, user)?;
    for s in &subs {
        let zone = quiet::zone_for(tz.as_deref(), s.device_tz.as_deref());
        enqueue(c, s.sub_id, m, now, quiet::release_at(now, &zone))?;
    }
    Ok(subs.len())
}

// ---------------------------------------------------------------- audiences

/// The users who may see `bank` (the reverse of `viewer_of`): administrators, users holding a `user_banks` row, and the
/// owner when they may manage banks. Disabled users are left out.
fn bank_viewers(c: &Connection, bank: i64) -> Result<Vec<i64>, DbError> {
    let mut st = c.prepare(
        "SELECT id FROM users WHERE disabled = 0 AND (role = 'admin' \
         OR id IN (SELECT user_id FROM user_banks WHERE bank_id = ?1) \
         OR (can_manage_banks = 1 AND id = (SELECT owner_id FROM banks WHERE id = ?1))) ORDER BY id",
    )?;
    let rows = st.query_map(params![bank], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Who is told about `bank` (D12): its viewers; for an AI-generated (personal) bank only the users holding a `user_banks`
/// row. `None` when the bank does not exist.
fn push_audience(c: &Connection, bank: i64) -> Result<Option<Vec<i64>>, DbError> {
    let ai: Option<bool> = c
        .query_row(
            "SELECT ai_job IS NOT NULL FROM banks WHERE id=?1",
            params![bank],
            |r| r.get(0),
        )
        .optional()?;
    match ai {
        None => Ok(None),
        Some(false) => Ok(Some(bank_viewers(c, bank)?)),
        Some(true) => {
            let mut st = c.prepare(
                "SELECT u.id FROM user_banks ub JOIN users u ON u.id = ub.user_id \
                 WHERE ub.bank_id = ?1 AND u.disabled = 0 ORDER BY u.id",
            )?;
            let rows = st.query_map(params![bank], |r| r.get(0))?;
            Ok(Some(rows.collect::<Result<_, _>>()?))
        }
    }
}

fn bank_facts(c: &Connection, bank: i64) -> Result<Option<(String, i64)>, DbError> {
    Ok(c
        .query_row(
            "SELECT name, (SELECT COUNT(*) FROM questions q WHERE q.bank_id = banks.id AND q.active = 1) \
             FROM banks WHERE id=?1",
            params![bank],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

// ---------------------------------------------------------------- helpers other stores call

/// Removes the push subscription of a device (its outbox rows first) and reports whether one existed.
pub(super) fn push_remove_device(c: &Connection, device_id: i64) -> Result<bool, DbError> {
    let sub: Option<i64> = c
        .query_row(
            "SELECT id FROM push_subscriptions WHERE device_id=?1",
            params![device_id],
            |r| r.get(0),
        )
        .optional()?;
    match sub {
        Some(id) => {
            delete_subscription(c, id)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Removes everything push-related of a user: outbox rows, subscriptions, saved sessions.
pub(super) fn push_delete_user(c: &Connection, user_id: i64) -> Result<(), DbError> {
    c.execute(
        "DELETE FROM push_outbox WHERE subscription_id IN \
         (SELECT s.id FROM push_subscriptions s JOIN devices d ON d.id = s.device_id WHERE d.user_id = ?1)",
        params![user_id],
    )?;
    c.execute(
        "DELETE FROM push_subscriptions WHERE device_id IN (SELECT id FROM devices WHERE user_id = ?1)",
        params![user_id],
    )?;
    c.execute(
        "DELETE FROM saved_sessions WHERE user_id=?1",
        params![user_id],
    )?;
    Ok(())
}

/// Removes the saved-session markers of a device.
pub(super) fn saved_delete_device(c: &Connection, device_id: i64) -> Result<(), DbError> {
    c.execute(
        "DELETE FROM saved_sessions WHERE device_id=?1",
        params![device_id],
    )?;
    Ok(())
}

/// Queues the "you can now use these banks" push(es) after an administrator granted existing banks (D11): one `bank.new`
/// for a single bank, one `bank.grantMany` for two or more; nothing for an administrator target. `bank_ids` are the banks
/// that were newly granted (ids that no longer exist are ignored).
pub(super) fn push_enqueue_grant(
    c: &Connection,
    user_id: i64,
    bank_ids: &[i64],
    now: i64,
) -> Result<(), DbError> {
    let role: Option<String> = c
        .query_row(
            "SELECT role FROM users WHERE id=?1",
            params![user_id],
            |r| r.get(0),
        )
        .optional()?;
    if role.as_deref() != Some("user") {
        return Ok(());
    }
    let mut seen = HashSet::new();
    let mut banks: Vec<(i64, String, i64)> = Vec::new();
    for id in bank_ids {
        if !seen.insert(*id) {
            continue;
        }
        if let Some((name, total)) = bank_facts(c, *id)? {
            banks.push((*id, name, total));
        }
    }
    let msg = match banks.as_slice() {
        [] => return Ok(()),
        [(id, name, total)] => bank_new_msg(*id, name, *total),
        many => bank_many_msg(many.len() as i64),
    };
    enqueue_for_user(c, user_id, &msg, now)?;
    Ok(())
}

// ---------------------------------------------------------------- Store

impl Store {
    // ------------------------------------------------------------ subscriptions

    /// `POST /api/push/subscribe`: stores the subscription of `device` (B.8). The endpoint moves here when another device
    /// holds it; this device's own earlier subscription is replaced; at most [`SUBSCRIPTIONS_PER_USER`] per user stay.
    pub fn push_subscribe(
        &self,
        user: i64,
        device: i64,
        req: &PushSubscribe,
        now: i64,
    ) -> Result<PushSubscribed, DbError> {
        self.tx(|c| {
            ensure_active(c, user)?;
            let mine: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1 AND user_id=?2)",
                params![device, user],
                |r| r.get(0),
            )?;
            if !mine {
                return Ok(PushSubscribed::NoDevice);
            }
            if !push_switch_on(c, user)? {
                return Ok(PushSubscribed::Off);
            }
            type Held = (i64, i64, i64, String, String, String);
            let by_endpoint: Option<Held> = c
                .query_row(
                    "SELECT id, device_id, confirmed_at, p256dh, auth, key_id FROM push_subscriptions WHERE endpoint=?1",
                    params![req.endpoint],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                        ))
                    },
                )
                .optional()?;
            let outcome = match by_endpoint {
                Some((id, dev, confirmed, p, a, k)) if dev == device => {
                    let same = p == req.p256dh && a == req.auth && k == req.key_id;
                    if same && now - confirmed < RECONFIRM_SECS {
                        return Ok(PushSubscribed::Unchanged);
                    }
                    c.execute(
                        "UPDATE push_subscriptions SET p256dh=?1, auth=?2, key_id=?3, confirmed_at=?4, expires_at=?5, failures=0 WHERE id=?6",
                        params![req.p256dh, req.auth, req.key_id, now, req.expires_at, id],
                    )?;
                    PushSubscribed::Confirmed
                }
                Some((id, _, _, _, _, _)) => {
                    // The endpoint moves to this device (and user): queued pushes of the old holder go
                    c.execute(
                        "DELETE FROM push_outbox WHERE subscription_id=?1",
                        params![id],
                    )?;
                    push_remove_device(c, device)?;
                    c.execute(
                        "UPDATE push_subscriptions SET device_id=?1, p256dh=?2, auth=?3, key_id=?4, confirmed_at=?5, \
                         expires_at=?6, failures=0, last_fail_at=NULL, last_status=NULL WHERE id=?7",
                        params![device, req.p256dh, req.auth, req.key_id, now, req.expires_at, id],
                    )?;
                    PushSubscribed::New
                }
                None => {
                    push_remove_device(c, device)?;
                    c.execute(
                        "INSERT INTO push_subscriptions(device_id, endpoint, p256dh, auth, key_id, created_at, confirmed_at, expires_at, failures) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, 0)",
                        params![device, req.endpoint, req.p256dh, req.auth, req.key_id, now, req.expires_at],
                    )?;
                    PushSubscribed::New
                }
            };
            // At most SUBSCRIPTIONS_PER_USER per user: the oldest confirmed beyond that go (never this one)
            let mut st = c.prepare(
                "SELECT s.id FROM push_subscriptions s JOIN devices d ON d.id = s.device_id \
                 WHERE d.user_id = ?1 AND s.device_id <> ?2 ORDER BY s.confirmed_at DESC, s.id DESC LIMIT -1 OFFSET ?3",
            )?;
            let extra: Vec<i64> = st
                .query_map(
                    params![user, device, (SUBSCRIPTIONS_PER_USER - 1) as i64],
                    |r| r.get(0),
                )?
                .collect::<Result<_, _>>()?;
            drop(st);
            for id in extra {
                delete_subscription(c, id)?;
            }
            Ok(outcome)
        })
    }

    /// Whether `device` has a push subscription row.
    pub fn push_has_subscription(&self, device: i64) -> Result<bool, DbError> {
        let g = self.lock();
        Ok(g.query_row(
            "SELECT EXISTS(SELECT 1 FROM push_subscriptions WHERE device_id=?1)",
            params![device],
            |r| r.get(0),
        )?)
    }

    /// `POST /api/push/test`: queues one test notification for the device (no quiet hours). `claim` is called last, after
    /// the switch and the subscription were checked, and says whether the per-device throttle lets this one through.
    pub fn push_test(
        &self,
        user: i64,
        device: i64,
        now: i64,
        claim: impl FnOnce() -> bool,
    ) -> Result<PushTestOutcome, DbError> {
        self.tx(|c| {
            ensure_active(c, user)?;
            if !push_switch_on(c, user)? {
                return Ok(PushTestOutcome::Off);
            }
            let sub: Option<i64> = c
                .query_row(
                    "SELECT s.id FROM push_subscriptions s JOIN devices d ON d.id = s.device_id \
                     WHERE s.device_id = ?1 AND d.user_id = ?2",
                    params![device, user],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(sub) = sub else {
                return Ok(PushTestOutcome::NoDevice);
            };
            if !claim() {
                return Ok(PushTestOutcome::Busy);
            }
            enqueue(c, sub, &test_msg(), now, now)?;
            Ok(PushTestOutcome::Queued)
        })
    }

    /// Start-up (rotation): deletes every subscription whose key is not `key_id` (outbox first). Returns how many.
    pub fn push_key_cleanup(&self, key_id: &str) -> Result<usize, DbError> {
        self.tx(|c| {
            let mut st = c.prepare("SELECT id FROM push_subscriptions WHERE key_id <> ?1")?;
            let ids: Vec<i64> = st
                .query_map(params![key_id], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            drop(st);
            for id in &ids {
                delete_subscription(c, *id)?;
            }
            Ok(ids.len())
        })
    }

    // ------------------------------------------------------------ saved sessions

    /// `POST /api/study/saved` (B.9). `Ok(false)`: `device` is not one of the user's. A report within a minute of the last
    /// full write only refreshes `answered` and `time_left`; every other one restarts the reminder schedule.
    pub fn saved_report(
        &self,
        user: i64,
        device: i64,
        r: &SavedReport,
        now: i64,
    ) -> Result<bool, DbError> {
        self.tx(|c| {
            ensure_active(c, user)?;
            let mine: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1 AND user_id=?2)",
                params![device, user],
                |r| r.get(0),
            )?;
            if !mine {
                return Ok(false);
            }
            let existing: Option<(Option<i64>, i64)> = c
                .query_row(
                    "SELECT device_id, saved_at FROM saved_sessions WHERE user_id=?1 AND session_key=?2",
                    params![user, r.key],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let times_up = r.time_left == Some(0);
            match existing {
                Some((Some(dev), saved_at))
                    if dev == device && saved_at > now - SAVED_THROTTLE_SECS =>
                {
                    c.execute(
                        "UPDATE saved_sessions SET answered=MIN(?1, total), time_left=?2, \
                         remind_at=CASE WHEN ?3 THEN NULL ELSE remind_at END WHERE user_id=?4 AND session_key=?5",
                        params![r.answered, r.time_left, times_up, user, r.key],
                    )?;
                }
                _ => {
                    let remind_at = if times_up {
                        None
                    } else {
                        Some(now + REMIND_AFTER_SECS)
                    };
                    c.execute(
                        "INSERT INTO saved_sessions(user_id, session_key, device_id, kind, answered, total, started_at, time_left, saved_at, remind_at, reminders) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0) \
                         ON CONFLICT(user_id, session_key) DO UPDATE SET device_id=excluded.device_id, kind=excluded.kind, \
                         answered=excluded.answered, total=excluded.total, started_at=excluded.started_at, \
                         time_left=excluded.time_left, saved_at=excluded.saved_at, remind_at=excluded.remind_at, reminders=0",
                        params![
                            user,
                            r.key,
                            device,
                            r.kind,
                            r.answered,
                            r.total,
                            r.started_at,
                            r.time_left,
                            now,
                            remind_at
                        ],
                    )?;
                }
            }
            // The browser holds one saved session per user: a new one replaced the old one
            c.execute(
                "DELETE FROM saved_sessions WHERE user_id=?1 AND device_id=?2 AND session_key<>?3",
                params![user, device, r.key],
            )?;
            Ok(true)
        })
    }

    /// `POST /api/study/saved/discard`: deletes the user's marker of this session (no row is not an error).
    pub fn saved_discard(&self, user: i64, key: &str) -> Result<(), DbError> {
        self.tx(|c| {
            c.execute(
                "DELETE FROM saved_sessions WHERE user_id=?1 AND session_key=?2",
                params![user, key],
            )?;
            Ok(())
        })
    }

    /// `POST /api/attempt`: the saved marker of this session is done (an answer was recorded).
    pub fn saved_done(&self, user_id: i64, key: &str) -> Result<(), DbError> {
        self.saved_discard(user_id, key)
    }

    // ------------------------------------------------------------ triggers

    /// Start-up version trigger (C.8): queues a push when the running version is higher than the announced one.
    /// Returns `(from, to, devices)` when one was queued.
    pub fn push_version_check(
        &self,
        running: &str,
        now: i64,
    ) -> Result<Option<PushVersionPush>, DbError> {
        self.tx(|c| {
            let last_detail: Option<Option<String>> = c
                .query_row(
                    "SELECT detail FROM activity_log WHERE action='system.start' ORDER BY id DESC LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            let last_ran: Option<String> = last_detail
                .flatten()
                .and_then(|d| serde_json::from_str::<Value>(&d).ok())
                .and_then(|v| v.get("version").and_then(Value::as_str).map(str::to_string));
            let announced = get_meta(c, "app_version")?;

            // No backlog from imports and banks another binary made while this version was not running
            let marks_missing = meta_i64(c, "push_imports_seen")?.is_none()
                || meta_i64(c, "push_banks_seen")?.is_none();
            if last_ran.as_deref() != Some(running) || marks_missing {
                let imports: i64 =
                    c.query_row("SELECT COALESCE(MAX(id), 0) FROM imports", [], |r| r.get(0))?;
                let banks: i64 =
                    c.query_row("SELECT COALESCE(MAX(id), 0) FROM banks", [], |r| r.get(0))?;
                set_meta_if_changed(c, "push_imports_seen", &imports.to_string())?;
                set_meta_if_changed(c, "push_banks_seen", &banks.to_string())?;
            }

            let from: Option<String> = match &announced {
                None => {
                    set_meta_if_changed(c, "app_version", running)?;
                    match &last_ran {
                        Some(lr) if version_greater(running, lr) => Some(lr.clone()),
                        _ => return Ok(None),
                    }
                }
                Some(a) if version_greater(running, a) => {
                    set_meta_if_changed(c, "app_version", running)?;
                    Some(a.clone())
                }
                Some(_) => return Ok(None),
            };
            let msg = version_msg(from.as_deref().unwrap_or(""), running);
            let mut devices = 0;
            for user in users_with_subscriptions(c, now)? {
                devices += enqueue_for_user(c, user, &msg, now)?;
            }
            Ok(Some((from, running.to_string(), devices)))
        })
    }

    /// Scheduler: pushes for new banks and for imports into existing banks since the last tick (C.8). Returns how many
    /// outbox rows were queued or merged.
    pub fn push_bank_tick(&self, now: i64) -> Result<usize, DbError> {
        self.tx(|c| {
            let max_banks: i64 =
                c.query_row("SELECT COALESCE(MAX(id), 0) FROM banks", [], |r| r.get(0))?;
            let max_imports: i64 =
                c.query_row("SELECT COALESCE(MAX(id), 0) FROM imports", [], |r| r.get(0))?;
            let seen_banks = meta_i64(c, "push_banks_seen")?;
            let seen_imports = meta_i64(c, "push_imports_seen")?;
            let (Some(seen_banks), Some(seen_imports)) = (seen_banks, seen_imports) else {
                set_meta_if_changed(c, "push_banks_seen", &max_banks.to_string())?;
                set_meta_if_changed(c, "push_imports_seen", &max_imports.to_string())?;
                return Ok(0);
            };
            let mut queued = 0;
            let mut new_banks: HashSet<i64> = HashSet::new();
            if max_banks > seen_banks {
                // Every bank created since the last tick is "new": its own import is not announced a second time, also when
                // the bank holds no active question and so gets no push of its own
                let mut st = c.prepare(
                    "SELECT b.id, EXISTS (SELECT 1 FROM questions q WHERE q.bank_id = b.id AND q.active = 1) \
                     FROM banks b WHERE b.id > ?1 AND b.id <= ?2 ORDER BY b.id",
                )?;
                let ids: Vec<(i64, bool)> = st
                    .query_map(params![seen_banks, max_banks], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })?
                    .collect::<Result<_, _>>()?;
                drop(st);
                for (id, has_questions) in ids {
                    new_banks.insert(id);
                    if !has_questions {
                        continue;
                    }
                    let (Some((name, total)), Some(audience)) =
                        (bank_facts(c, id)?, push_audience(c, id)?)
                    else {
                        continue;
                    };
                    let msg = bank_new_msg(id, &name, total);
                    for user in audience {
                        queued += enqueue_for_user(c, user, &msg, now)?;
                    }
                }
            }
            if max_imports > seen_imports {
                let mut st = c.prepare(
                    "SELECT bank_id, added + restored FROM imports WHERE id > ?1 AND id <= ?2 \
                     AND added + restored > 0 AND bank_id IS NOT NULL ORDER BY id",
                )?;
                let rows: Vec<(i64, i64)> = st
                    .query_map(params![seen_imports, max_imports], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })?
                    .collect::<Result<_, _>>()?;
                drop(st);
                for (bank, added) in rows {
                    if new_banks.contains(&bank) {
                        continue;
                    }
                    let (Some((name, total)), Some(audience)) =
                        (bank_facts(c, bank)?, push_audience(c, bank)?)
                    else {
                        continue;
                    };
                    let msg = bank_updated_msg(bank, &name, added, total);
                    for user in audience {
                        queued += enqueue_for_user(c, user, &msg, now)?;
                    }
                }
            }
            set_meta_if_changed(c, "push_banks_seen", &max_banks.to_string())?;
            set_meta_if_changed(c, "push_imports_seen", &max_imports.to_string())?;
            Ok(queued)
        })
    }

    /// Scheduler: the reminders of unfinished practices / exams that are due (C.8, D10). Returns how many were queued for
    /// sending right now (held ones do not count).
    pub fn push_remind_tick(&self, now: i64) -> Result<usize, DbError> {
        self.tx(|c| {
            type Marker = (
                i64,
                String,
                Option<i64>,
                String,
                i64,
                i64,
                Option<i64>,
                i64,
                i64,
            );
            let mut st = c.prepare(
                "SELECT user_id, session_key, device_id, kind, answered, total, time_left, saved_at, reminders \
                 FROM saved_sessions WHERE remind_at IS NOT NULL AND remind_at <= ?1 ORDER BY remind_at, user_id",
            )?;
            let due: Vec<Marker> = st
                .query_map(params![now], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                    ))
                })?
                .collect::<Result<_, _>>()?;
            drop(st);
            let mut queued = 0;
            for (user, key, device, kind, answered, total, time_left, saved_at, reminders) in due {
                let finished: bool = c.query_row(
                    "SELECT EXISTS(SELECT 1 FROM attempts WHERE user_id=?1 AND session_key=?2)",
                    params![user, key],
                    |r| r.get(0),
                )?;
                if finished {
                    c.execute(
                        "DELETE FROM saved_sessions WHERE user_id=?1 AND session_key=?2",
                        params![user, key],
                    )?;
                    continue;
                }
                if time_left == Some(0) {
                    c.execute(
                        "UPDATE saved_sessions SET remind_at=NULL WHERE user_id=?1 AND session_key=?2",
                        params![user, key],
                    )?;
                    continue;
                }
                let n = reminders + 1;
                let device_tz: Option<String> = match device {
                    Some(d) => c
                        .query_row("SELECT tz FROM devices WHERE id=?1", params![d], |r| {
                            r.get(0)
                        })
                        .optional()?
                        .flatten(),
                    None => None,
                };
                let zone = quiet::zone_for(user_tz(c, user)?.as_deref(), device_tz.as_deref());
                let release = quiet::release_at(now, &zone);
                if let Some(d) = device {
                    if let Some(sub) = eligible_sub_of_device(c, d, now)? {
                        let msg = remind_msg(&kind, answered, total, n, &key);
                        enqueue(c, sub.sub_id, &msg, now, release)?;
                        if release <= now {
                            queued += 1;
                        }
                    }
                }
                let next = if n < MAX_REMINDERS {
                    Some((saved_at + REMIND_AFTER_SECS * (n + 1)).max(release + REMIND_AFTER_SECS))
                } else {
                    None
                };
                c.execute(
                    "UPDATE saved_sessions SET reminders=?1, remind_at=?2 WHERE user_id=?3 AND session_key=?4",
                    params![n, next, user, key],
                )?;
            }
            Ok(queued)
        })
    }

    /// Whether some outbox row can be handed out now.
    pub fn push_due(&self, now: i64) -> Result<bool, DbError> {
        let g = self.lock();
        Ok(g.query_row(
            "SELECT EXISTS(SELECT 1 FROM push_outbox WHERE state IN ('queued', 'leased') AND next_at <= ?1 AND expires_at > ?1)",
            params![now],
            |r| r.get(0),
        )?)
    }

    /// The earliest time an outbox row becomes due (the courier's long poll sleeps until then); `None` when nothing waits.
    pub fn push_next_due(&self, now: i64) -> Result<Option<i64>, DbError> {
        let g = self.lock();
        Ok(g.query_row(
            "SELECT MIN(next_at) FROM push_outbox WHERE state IN ('queued', 'leased') AND expires_at > ?1",
            params![now],
            |r| r.get(0),
        )?)
    }

    /// The users who may see `bank` (the reverse of `viewer_of`, disabled users left out), for tests and checks.
    pub fn push_bank_viewers(&self, bank: i64) -> Result<Vec<i64>, DbError> {
        let g = self.lock();
        bank_viewers(&g, bank)
    }

    // ------------------------------------------------------------ lease and results

    /// `POST /internal/ai/push/next`: hands out up to `max` due messages, encrypted and signed (C.5, C.8). Every row is
    /// re-checked first; one that fails becomes `expired`. Quiet hours put a row back to `queued` until 08:00.
    pub fn push_lease(
        &self,
        now: i64,
        max: usize,
        key: &vapid::Vapid,
        subject: &str,
    ) -> Result<Vec<PushJob>, DbError> {
        let key_id = key.key_id();
        self.tx(|c| lease_in(c, now, max.min(LEASE_BATCH), key, &key_id, subject))
    }

    /// `POST /internal/ai/push/result`: applies one result (C.5). Late or duplicate results (the row is not leased, or leased
    /// with another try number) change nothing. `current_key_id` decides whether an `auth` answer deletes the subscription.
    pub fn push_result(
        &self,
        now: i64,
        r: &PushJobResult,
        current_key_id: Option<&str>,
    ) -> Result<PushResultEffect, DbError> {
        self.tx(|c| result_in(c, now, r, current_key_id))
    }

    // ------------------------------------------------------------ housekeeping

    /// Hourly clean-up (C.8): expires rows past their TTL, deletes finished rows after seven days, dead subscriptions and
    /// old saved sessions. Batches of 500 rows; the lock is released between batches.
    pub fn push_housekeeping(&self, now: i64) -> Result<PushHousekeeping, DbError> {
        let mut out = PushHousekeeping::default();
        loop {
            let n = self.tx(|c| {
                Ok(c.execute(
                    "UPDATE push_outbox SET state='expired', done_at=?1 WHERE id IN \
                     (SELECT id FROM push_outbox WHERE state IN ('queued', 'leased') AND expires_at <= ?1 LIMIT ?2)",
                    params![now, HOUSEKEEPING_BATCH],
                )?)
            })?;
            out.expired += n;
            if (n as i64) < HOUSEKEEPING_BATCH {
                break;
            }
        }
        loop {
            let n = self.tx(|c| {
                Ok(c.execute(
                    "DELETE FROM push_outbox WHERE id IN \
                     (SELECT id FROM push_outbox WHERE state IN ('sent', 'failed', 'expired') AND done_at < ?1 LIMIT ?2)",
                    params![now - OUTBOX_KEEP_SECS, HOUSEKEEPING_BATCH],
                )?)
            })?;
            out.deleted += n;
            if (n as i64) < HOUSEKEEPING_BATCH {
                break;
            }
        }
        loop {
            let n = self.tx(|c| {
                let mut st = c.prepare(
                    "SELECT s.id FROM push_subscriptions s JOIN devices d ON d.id = s.device_id \
                     WHERE (s.expires_at IS NOT NULL AND s.expires_at < ?1) \
                        OR (s.failures >= 10 AND COALESCE(s.last_ok_at, s.created_at) < ?2) \
                        OR d.active_at < ?3 LIMIT ?4",
                )?;
                let ids: Vec<i64> = st
                    .query_map(
                        params![
                            now,
                            now - OUTBOX_KEEP_SECS,
                            now - DEVICE_IDLE_SECS,
                            HOUSEKEEPING_BATCH
                        ],
                        |r| r.get(0),
                    )?
                    .collect::<Result<_, _>>()?;
                drop(st);
                for id in &ids {
                    delete_subscription(c, *id)?;
                }
                Ok(ids.len())
            })?;
            out.subscriptions += n;
            if (n as i64) < HOUSEKEEPING_BATCH {
                break;
            }
        }
        loop {
            let n = self.tx(|c| {
                Ok(c.execute(
                    "DELETE FROM saved_sessions WHERE rowid IN (SELECT rowid FROM saved_sessions \
                     WHERE saved_at < ?1 OR EXISTS (SELECT 1 FROM attempts a WHERE a.user_id = saved_sessions.user_id \
                     AND a.session_key = saved_sessions.session_key) LIMIT ?2)",
                    params![now - SAVED_KEEP_SECS, HOUSEKEEPING_BATCH],
                )?)
            })?;
            out.saved += n;
            if (n as i64) < HOUSEKEEPING_BATCH {
                break;
            }
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- lease internals

struct Candidate {
    id: i64,
    sub_id: i64,
    kind: String,
    topic: Option<String>,
    payload: String,
    urgency: String,
    expires_at: i64,
}

struct SubContext {
    endpoint: String,
    p256dh: String,
    auth: String,
    key_id: String,
    device_id: i64,
    user_id: i64,
    device_tz: Option<String>,
    signed_out: bool,
    active_at: i64,
    disabled: bool,
}

/// The time to live of a queued row of this kind (seconds).
fn ttl_of(kind: &str) -> i64 {
    match kind {
        "remind" => TTL_REMIND,
        "version" => TTL_VERSION,
        "test" => TTL_TEST,
        _ => TTL_BANK,
    }
}

fn finish_row(c: &Connection, id: i64, state: &str, now: i64) -> Result<(), DbError> {
    c.execute(
        "UPDATE push_outbox SET state=?1, done_at=?2 WHERE id=?3",
        params![state, now, id],
    )?;
    Ok(())
}

fn lease_in(
    c: &Connection,
    now: i64,
    max: usize,
    key: &vapid::Vapid,
    key_id: &str,
    subject: &str,
) -> Result<Vec<PushJob>, DbError> {
    let mut st = c.prepare(
        "SELECT id, subscription_id, kind, topic, payload, urgency, expires_at FROM push_outbox \
         WHERE state IN ('queued', 'leased') AND next_at <= ?1 AND expires_at > ?1 ORDER BY next_at, id LIMIT ?2",
    )?;
    let cands: Vec<Candidate> = st
        .query_map(params![now, LEASE_SCAN], |r| {
            Ok(Candidate {
                id: r.get(0)?,
                sub_id: r.get(1)?,
                kind: r.get(2)?,
                topic: r.get(3)?,
                payload: r.get(4)?,
                urgency: r.get(5)?,
                expires_at: r.get(6)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    drop(st);

    let mut jobs: Vec<PushJob> = Vec::new();
    let mut audiences: HashMap<i64, Option<Vec<i64>>> = HashMap::new();
    let mut auth_headers: HashMap<String, String> = HashMap::new();
    for cand in cands {
        if jobs.len() >= max {
            break;
        }
        let ctx: Option<SubContext> = c
            .query_row(
                "SELECT s.endpoint, s.p256dh, s.auth, s.key_id, d.id, d.user_id, d.tz, d.signed_out_at IS NOT NULL, d.active_at, u.disabled \
                 FROM push_subscriptions s JOIN devices d ON d.id = s.device_id JOIN users u ON u.id = d.user_id WHERE s.id = ?1",
                params![cand.sub_id],
                |r| {
                    Ok(SubContext {
                        endpoint: r.get(0)?,
                        p256dh: r.get(1)?,
                        auth: r.get(2)?,
                        key_id: r.get(3)?,
                        device_id: r.get(4)?,
                        user_id: r.get(5)?,
                        device_tz: r.get(6)?,
                        signed_out: r.get(7)?,
                        active_at: r.get(8)?,
                        disabled: r.get::<_, i64>(9)? != 0,
                    })
                },
            )
            .optional()?;
        // (a) the subscription is still eligible
        let Some(ctx) = ctx else {
            finish_row(c, cand.id, "expired", now)?;
            continue;
        };
        if ctx.disabled
            || ctx.signed_out
            || ctx.active_at < now - DEVICE_IDLE_SECS
            || ctx.key_id != key_id
            || !push_switch_on(c, ctx.user_id)?
        {
            finish_row(c, cand.id, "expired", now)?;
            continue;
        }
        let payload: Value = match serde_json::from_str(&cand.payload) {
            Ok(v) => v,
            Err(_) => {
                finish_row(c, cand.id, "expired", now)?;
                continue;
            }
        };
        let k = payload.get("k").and_then(Value::as_str).unwrap_or("");
        // (b) a bank push: the bank still exists and the user may still see it
        if matches!(k, "bank.updated" | "bank.new") {
            let bank = payload.get("bank").and_then(Value::as_i64).unwrap_or(0);
            let audience = match audiences.get(&bank) {
                Some(a) => a.clone(),
                None => {
                    let a = push_audience(c, bank)?;
                    audiences.insert(bank, a.clone());
                    a
                }
            };
            if !audience.is_some_and(|a| a.contains(&ctx.user_id)) {
                finish_row(c, cand.id, "expired", now)?;
                continue;
            }
        }
        // (c) a reminder: the marker still exists for this device, no record exists for the session, time is not up
        if k == "remind" {
            let marker_key = payload.get("key").and_then(Value::as_str).unwrap_or("");
            let marker: Option<(Option<i64>, Option<i64>)> = c
                .query_row(
                    "SELECT device_id, time_left FROM saved_sessions WHERE user_id=?1 AND session_key=?2",
                    params![ctx.user_id, marker_key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let finished: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM attempts WHERE user_id=?1 AND session_key=?2)",
                params![ctx.user_id, marker_key],
                |r| r.get(0),
            )?;
            let ok = match marker {
                Some((dev, left)) => {
                    dev == Some(ctx.device_id) && !finished && left.is_none_or(|l| l > 0)
                }
                None => false,
            };
            if !ok {
                finish_row(c, cand.id, "expired", now)?;
                continue;
            }
        }
        // (d) the text renders
        let lang = ui_lang(c, ctx.user_id)?;
        let Some(rendered) = texts::render(&payload, &lang) else {
            finish_row(c, cand.id, "expired", now)?;
            continue;
        };
        // Quiet hours (everything but the test notification)
        if cand.kind != "test" {
            let zone = quiet::zone_for(
                user_tz(c, ctx.user_id)?.as_deref(),
                ctx.device_tz.as_deref(),
            );
            let release = quiet::release_at(now, &zone);
            if release > now {
                c.execute(
                    "UPDATE push_outbox SET state='queued', next_at=?1, expires_at=MAX(expires_at, ?1 + ?3) WHERE id=?2",
                    params![release, cand.id, ttl_of(&cand.kind)],
                )?;
                continue;
            }
        }
        // Encrypt and sign
        let plain = json!({
            "v": 1,
            "title": rendered.title,
            "body": rendered.body,
            "tag": rendered.tag,
            "url": rendered.url,
            "lang": rendered.lang,
        })
        .to_string();
        let (Some(ua), Some(auth)) = (
            crate::push::b64_lenient(&ctx.p256dh),
            crate::push::b64_lenient(&ctx.auth),
        ) else {
            finish_row(c, cand.id, "expired", now)?;
            continue;
        };
        let (Ok(ua), Ok(auth)) = (
            <[u8; 65]>::try_from(ua.as_slice()),
            <[u8; 16]>::try_from(auth.as_slice()),
        ) else {
            finish_row(c, cand.id, "expired", now)?;
            continue;
        };
        let Ok(record) = ece::encrypt(&ua, &auth, plain.as_bytes()) else {
            finish_row(c, cand.id, "expired", now)?;
            continue;
        };
        let Some(aud) = vapid::audience_of(&ctx.endpoint) else {
            // The endpoint is no longer an allowed push service
            finish_row(c, cand.id, "failed", now)?;
            continue;
        };
        let authorization = match auth_headers.get(&aud) {
            Some(h) => h.clone(),
            None => match key.authorization(&aud, subject, now) {
                Ok(h) => {
                    auth_headers.insert(aud.clone(), h.clone());
                    h
                }
                Err(_) => {
                    finish_row(c, cand.id, "failed", now)?;
                    continue;
                }
            },
        };
        let tries: i64 = c.query_row(
            "UPDATE push_outbox SET state='leased', tries=tries+1, next_at=?1 WHERE id=?2 RETURNING tries",
            params![now + LEASE_SECS, cand.id],
            |r| r.get(0),
        )?;
        jobs.push(PushJob {
            id: cand.id,
            tries,
            endpoint: ctx.endpoint,
            ttl: (cand.expires_at - now).max(0),
            urgency: cand.urgency,
            topic: cand.topic,
            authorization,
            body: crate::security::base64url_nopad(&record),
        });
    }
    Ok(jobs)
}

fn result_in(
    c: &Connection,
    now: i64,
    r: &PushJobResult,
    current_key_id: Option<&str>,
) -> Result<PushResultEffect, DbError> {
    let row: Option<(i64, i64, String, String)> = c
        .query_row(
            "SELECT o.subscription_id, o.expires_at, s.key_id, s.endpoint FROM push_outbox o \
             JOIN push_subscriptions s ON s.id = o.subscription_id WHERE o.id=?1 AND o.state='leased' AND o.tries=?2",
            params![r.id, r.tries],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((sub, expires_at, sub_key, endpoint)) = row else {
        return Ok(PushResultEffect::default());
    };
    let mut effect = PushResultEffect {
        applied: true,
        host: endpoint_host(&endpoint),
        subscription_deleted: false,
    };
    let status = r.status;
    let fail_sub = |with_failure: bool| -> Result<(), DbError> {
        if with_failure {
            c.execute(
                "UPDATE push_subscriptions SET failures=failures+1, last_fail_at=?1, last_status=?2 WHERE id=?3",
                params![now, status.unwrap_or(0), sub],
            )?;
        } else {
            c.execute(
                "UPDATE push_subscriptions SET last_status=?1 WHERE id=?2",
                params![status.unwrap_or(0), sub],
            )?;
        }
        Ok(())
    };
    let set_row = |state: &str, next_at: Option<i64>, tries: i64| -> Result<(), DbError> {
        let done_at = if state == "queued" { None } else { Some(now) };
        c.execute(
            "UPDATE push_outbox SET state=?1, status=?2, tries=?3, next_at=COALESCE(?4, next_at), done_at=?5 WHERE id=?6",
            params![state, status, tries, next_at, done_at, r.id],
        )?;
        Ok(())
    };
    match r.outcome {
        PushOutcome::Sent => {
            set_row("sent", None, r.tries)?;
            c.execute(
                "UPDATE push_subscriptions SET failures=0, last_ok_at=?1, last_status=?2 WHERE id=?3",
                params![now, status, sub],
            )?;
        }
        PushOutcome::Gone => {
            delete_subscription(c, sub)?;
            effect.subscription_deleted = true;
        }
        PushOutcome::Rejected => set_row("failed", None, r.tries)?,
        PushOutcome::Auth => {
            if current_key_id.is_some_and(|k| k != sub_key) {
                delete_subscription(c, sub)?;
                effect.subscription_deleted = true;
            } else {
                set_row("failed", None, r.tries)?;
                fail_sub(true)?;
            }
        }
        PushOutcome::Retry => {
            if r.tries >= MAX_TRIES || expires_at <= now {
                set_row("failed", None, r.tries)?;
                fail_sub(r.retry_after.is_none())?;
            } else {
                let wait = match r.retry_after {
                    Some(s) => s,
                    None => push_backoff_secs(r.tries, jitter_pct()),
                };
                set_row("queued", Some(now + wait), r.tries)?;
                fail_sub(r.retry_after.is_none())?;
            }
        }
        PushOutcome::Deferred => {
            let wait = r.retry_after.unwrap_or(60);
            set_row("queued", Some(now + wait), (r.tries - 1).max(0))?;
        }
    }
    Ok(effect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_follows_the_table() {
        assert_eq!(push_backoff_secs(1, 0), 30);
        assert_eq!(push_backoff_secs(2, 0), 120);
        assert_eq!(push_backoff_secs(3, 0), 600);
        assert_eq!(push_backoff_secs(4, 0), 3600);
        assert_eq!(push_backoff_secs(9, 0), 3600);
        assert_eq!(push_backoff_secs(1, 20), 36);
        assert_eq!(push_backoff_secs(4, 99), 3600 + 720);
    }

    #[test]
    fn versions_compare_as_dotted_numbers() {
        assert!(version_greater("3.1", "3.0"));
        assert!(version_greater("2.3.1", "2.3"));
        assert!(version_greater("3.10", "3.9"));
        assert!(!version_greater("3.1", "3.1"));
        assert!(!version_greater("3.1.0", "3.1"));
        assert!(!version_greater("3.0", "3.1"));
        assert!(!version_greater("3.1-rc1", "3.0"));
        assert!(!version_greater("3.1", "x"));
        assert!(!version_greater("", "3.1"));
        assert!(version_greater("3.1.0.1", "3.1"));
    }

    #[test]
    fn names_are_cut_at_sixty_characters() {
        let short = "General knowledge";
        assert_eq!(cut_name(short), short);
        let long: String = "题".repeat(61);
        let cut = cut_name(&long);
        assert_eq!(cut.chars().count(), 61);
        assert!(cut.ends_with('…'));
        assert_eq!(cut_name(&"a".repeat(60)), "a".repeat(60));
    }

    #[test]
    fn hosts_come_from_the_endpoint() {
        assert_eq!(
            endpoint_host("https://FCM.googleapis.com:443/fcm/send/abc").as_deref(),
            Some("fcm.googleapis.com")
        );
        assert_eq!(endpoint_host("http://x/y"), None);
    }

    // ---- the helpers other stores call (they take an open transaction's connection)

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let s = Store::open(&dir.path().join("xexam.db")).expect("store");
        (dir, s)
    }

    fn exec(s: &Store, sql: &str, p: impl rusqlite::Params) {
        s.lock().execute(sql, p).expect(sql);
    }

    fn count(s: &Store, table: &str) -> i64 {
        s.lock()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .expect("count")
    }

    fn add_user(s: &Store, name: &str, role: &str) -> i64 {
        exec(
            s,
            "INSERT INTO users(username, password_hash, role, created_at, password_changed_at) VALUES(?1, 'x', ?2, '2026-06-10 12:00', '2026-06-10 12:00')",
            params![name, role],
        );
        s.lock()
            .query_row(
                "SELECT id FROM users WHERE username = ?1",
                params![name],
                |r| r.get(0),
            )
            .expect("user")
    }

    fn add_device(s: &Store, user: i64, n: usize, active_at: i64) -> i64 {
        exec(
            s,
            "INSERT INTO devices(user_id, cookie_hash, created_at, active_at, tz) VALUES(?1, ?2, ?3, ?3, 'UTC')",
            params![user, format!("{n:064x}"), active_at],
        );
        s.lock()
            .query_row("SELECT MAX(id) FROM devices", [], |r| r.get(0))
            .expect("device")
    }

    fn add_sub(s: &Store, device: i64, now: i64) -> i64 {
        exec(
            s,
            "INSERT INTO push_subscriptions(device_id, endpoint, p256dh, auth, key_id, created_at, confirmed_at) VALUES(?1, ?2, ?3, ?4, '0123456789abcdef', ?5, ?5)",
            params![
                device,
                format!("https://fcm.googleapis.com/fcm/send/{device}"),
                "A".repeat(87),
                "B".repeat(22),
                now
            ],
        );
        s.lock()
            .query_row("SELECT MAX(id) FROM push_subscriptions", [], |r| r.get(0))
            .expect("sub")
    }

    fn add_outbox(s: &Store, sub: i64, topic: &str, now: i64) {
        exec(
            s,
            "INSERT INTO push_outbox(subscription_id, kind, topic, payload, created_at, expires_at, next_at) VALUES(?1, 'test', ?2, ?5, ?3, ?4, ?3)",
            params![sub, topic, now, now + 600, r#"{"k":"test"}"#],
        );
    }

    fn add_saved(s: &Store, user: i64, device: Option<i64>, key: &str, now: i64) {
        exec(
            s,
            "INSERT INTO saved_sessions(user_id, session_key, device_id, kind, answered, total, started_at, saved_at) VALUES(?1, ?2, ?3, 'practice', 1, 5, 1780000000, ?4)",
            params![user, key, device, now],
        );
    }

    fn add_bank(s: &Store, name: &str, questions: usize) -> i64 {
        exec(
            s,
            "INSERT INTO banks(name, filename, legacy_ids, created_at, updated_at) VALUES(?1, ?1, 0, '2026-06-10 12:00', '2026-06-10 12:00')",
            params![name],
        );
        let id: i64 = s
            .lock()
            .query_row("SELECT MAX(id) FROM banks", [], |r| r.get(0))
            .expect("bank");
        for i in 0..questions {
            exec(
                s,
                "INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id) VALUES(?1, 'S', ?2, ?2, '{}', 1, '2026-06-10 12:00', '2026-06-10 12:00', ?3)",
                params![format!("{id}-{i}"), i as i64 + 1, id],
            );
        }
        id
    }

    const NOW: i64 = 1_781_000_000;
    const KEY_A: &str = "0123456789abcdef0123456789abcdef";
    const KEY_B: &str = "fedcba9876543210fedcba9876543210";

    #[test]
    fn removing_a_device_deletes_its_subscription_with_the_outbox_first() {
        let (_dir, s) = store();
        let user = add_user(&s, "kim", "user");
        let (d1, d2) = (add_device(&s, user, 1, NOW), add_device(&s, user, 2, NOW));
        let (s1, s2) = (add_sub(&s, d1, NOW), add_sub(&s, d2, NOW));
        add_outbox(&s, s1, "a", NOW);
        add_outbox(&s, s1, "b", NOW);
        add_outbox(&s, s2, "a", NOW);
        // foreign keys are on: this only works because the outbox goes first
        assert!(s.tx(|c| push_remove_device(c, d1)).expect("remove"));
        assert_eq!(
            (count(&s, "push_subscriptions"), count(&s, "push_outbox")),
            (1, 1)
        );
        // a device without a subscription: nothing, and `false`
        assert!(!s.tx(|c| push_remove_device(c, d1)).expect("again"));
        assert!(!s.tx(|c| push_remove_device(c, 9_999)).expect("unknown"));
        assert_eq!(count(&s, "push_subscriptions"), 1);
    }

    #[test]
    fn deleting_a_user_removes_outbox_subscriptions_and_saved_sessions_of_that_user_only() {
        let (_dir, s) = store();
        let (kim, lee) = (add_user(&s, "kim", "user"), add_user(&s, "lee", "user"));
        let (d1, d2, d3) = (
            add_device(&s, kim, 1, NOW),
            add_device(&s, kim, 2, NOW),
            add_device(&s, lee, 3, NOW),
        );
        let subs = [
            add_sub(&s, d1, NOW),
            add_sub(&s, d2, NOW),
            add_sub(&s, d3, NOW),
        ];
        for sub in subs {
            add_outbox(&s, sub, "a", NOW);
        }
        add_saved(&s, kim, Some(d1), KEY_A, NOW);
        add_saved(&s, kim, None, KEY_B, NOW);
        add_saved(&s, lee, Some(d3), KEY_A, NOW);
        s.tx(|c| push_delete_user(c, kim)).expect("delete");
        assert_eq!(
            (
                count(&s, "push_subscriptions"),
                count(&s, "push_outbox"),
                count(&s, "saved_sessions")
            ),
            (1, 1, 1)
        );
        // the user's devices are the devices store's to delete (child-first, after this helper)
        assert_eq!(count(&s, "devices"), 3);
        s.tx(|c| {
            c.execute("DELETE FROM devices WHERE user_id = ?1", params![kim])?;
            Ok(())
        })
        .expect("devices can be deleted now");
        s.tx(|c| push_delete_user(c, kim)).expect("again");
    }

    #[test]
    fn deleting_a_device_removes_its_saved_sessions_only() {
        let (_dir, s) = store();
        let kim = add_user(&s, "kim", "user");
        let (d1, d2) = (add_device(&s, kim, 1, NOW), add_device(&s, kim, 2, NOW));
        add_saved(&s, kim, Some(d1), KEY_A, NOW);
        add_saved(&s, kim, Some(d2), KEY_B, NOW);
        s.tx(|c| saved_delete_device(c, d1)).expect("delete");
        assert_eq!(count(&s, "saved_sessions"), 1);
        // now the device itself can go
        s.tx(|c| {
            c.execute("DELETE FROM devices WHERE id = ?1", params![d1])?;
            Ok(())
        })
        .expect("device deleted");
    }

    fn bank_rows(s: &Store, sub: i64) -> Vec<(String, Value)> {
        let g = s.lock();
        let mut st = g
            .prepare("SELECT topic, payload FROM push_outbox WHERE subscription_id = ?1 AND kind = 'bank' ORDER BY id")
            .expect("prepare");
        st.query_map(params![sub], |r| {
            let p: String = r.get(1)?;
            Ok((r.get(0)?, serde_json::from_str(&p).expect("json")))
        })
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
    }

    #[test]
    fn one_granted_bank_is_a_new_bank_push_and_several_are_one_summary() {
        let (_dir, s) = store();
        let lee = add_user(&s, "lee", "user");
        let sub = add_sub(&s, add_device(&s, lee, 1, NOW), NOW);
        let (b1, b2, b3) = (
            add_bank(&s, "Herbs", 3),
            add_bank(&s, "Spices", 2),
            add_bank(&s, "Teas", 1),
        );
        s.tx(|c| push_enqueue_grant(c, lee, &[b1], NOW))
            .expect("grant");
        let rows = bank_rows(&s, sub);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, format!("bank-{b1}"));
        assert_eq!(
            rows[0].1,
            json!({"k": "bank.new", "bank": b1, "name": "Herbs", "total": 3})
        );
        // two or more at once: one summary on its own topic; unknown ids and duplicates do not count
        s.tx(|c| push_enqueue_grant(c, lee, &[b2, b3, b3, 9_999], NOW))
            .expect("grant");
        let rows = bank_rows(&s, sub);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].0, "bank-access");
        assert_eq!(rows[1].1, json!({"k": "bank.grantMany", "n": 2}));
        // a second summary replaces the queued one (newest only)
        s.tx(|c| push_enqueue_grant(c, lee, &[b1, b2, b3], NOW + 5))
            .expect("grant");
        let rows = bank_rows(&s, sub);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].1, json!({"k": "bank.grantMany", "n": 3}));
        // one existing bank among unknown ones is a single "new bank"
        s.tx(|c| push_enqueue_grant(c, lee, &[9_998, b2], NOW + 6))
            .expect("grant");
        assert_eq!(bank_rows(&s, sub).len(), 3);
        // nothing at all
        let before = count(&s, "push_outbox");
        s.tx(|c| push_enqueue_grant(c, lee, &[], NOW))
            .expect("grant");
        s.tx(|c| push_enqueue_grant(c, lee, &[9_999], NOW))
            .expect("grant");
        assert_eq!(count(&s, "push_outbox"), before);
    }

    #[test]
    fn a_grant_reaches_nobody_for_an_administrator_a_disabled_or_a_subscriptionless_user() {
        let (_dir, s) = store();
        let admin = add_user(&s, "boss", "admin");
        let off = add_user(&s, "off", "user");
        let disabled = add_user(&s, "gone", "user");
        let bare = add_user(&s, "bare", "user");
        let b1 = add_bank(&s, "Herbs", 3);
        add_sub(&s, add_device(&s, admin, 1, NOW), NOW);
        add_sub(&s, add_device(&s, off, 2, NOW), NOW);
        add_sub(&s, add_device(&s, disabled, 3, NOW), NOW);
        add_device(&s, bare, 4, NOW);
        exec(
            &s,
            "INSERT INTO user_settings(user_id, key, value) VALUES(?1, 'push', 'false')",
            params![off],
        );
        exec(
            &s,
            "UPDATE users SET disabled = 1 WHERE id = ?1",
            params![disabled],
        );
        for user in [admin, off, disabled, bare, 9_999] {
            s.tx(|c| push_enqueue_grant(c, user, &[b1], NOW))
                .expect("grant");
        }
        assert_eq!(count(&s, "push_outbox"), 0);
        // a device unused for 31 days and a signed-out device are not reached either
        let lee = add_user(&s, "lee", "user");
        let stale = add_device(&s, lee, 5, NOW - 31 * 86_400);
        add_sub(&s, stale, NOW);
        let out = add_device(&s, lee, 6, NOW);
        add_sub(&s, out, NOW);
        exec(
            &s,
            "UPDATE devices SET signed_out_at = ?1 WHERE id = ?2",
            params![NOW, out],
        );
        s.tx(|c| push_enqueue_grant(c, lee, &[b1], NOW))
            .expect("grant");
        assert_eq!(count(&s, "push_outbox"), 0);
        // every eligible device of the user gets it
        let d_a = add_device(&s, lee, 7, NOW);
        let d_b = add_device(&s, lee, 8, NOW);
        add_sub(&s, d_a, NOW);
        add_sub(&s, d_b, NOW);
        s.tx(|c| push_enqueue_grant(c, lee, &[b1], NOW))
            .expect("grant");
        assert_eq!(count(&s, "push_outbox"), 2);
    }

    #[test]
    fn a_newer_update_adds_to_the_queued_one() {
        let old = json!({"k": "bank.updated", "bank": 3, "name": "A", "added": 2, "total": 10});
        let new = json!({"k": "bank.updated", "bank": 3, "name": "B", "added": 5, "total": 15});
        let m = merge_payload(&old, &new);
        assert_eq!(m["added"], 7);
        assert_eq!(m["total"], 15);
        assert_eq!(m["name"], "B");
        // A queued "new bank" stays new
        let old = json!({"k": "bank.new", "bank": 3, "name": "A", "total": 10});
        let m = merge_payload(&old, &new);
        assert_eq!(m["k"], "bank.new");
        assert_eq!(m["total"], 15);
        // Anything else replaces
        let m = merge_payload(
            &json!({"k": "version", "from": "1", "to": "2"}),
            &json!({"k": "version", "from": "1", "to": "3"}),
        );
        assert_eq!(m["to"], "3");
    }
}
