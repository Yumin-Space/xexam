//! The activity log, write side (v1.3): who did what, kept in `activity_log` (schema v8) and shown to the administrators.
//!
//! Everything goes through one [`Sink`]:
//! - [`Sink::record`] never blocks: it puts an [`Entry`] on a bounded queue (2048). A full queue drops the entry and counts
//!   it; the writer turns the count into one `system.logDropped` row.
//! - One writer task takes rows off the queue and writes them in batches (100 rows or 500 ms, one transaction on a blocking
//!   thread). It is flushed by [`Sink::flush`] and stopped, after a last flush, by [`Sink::stop`] before the store closes.
//! - `ip`, `country` and `region` and `city` are written ONLY while the sign-in info switch (`users.signin_info`, the eye) is
//!   open (backend and AI events alike; the English log line leaves the address out too). While it is closed nothing new is
//!   recorded, and the read side (`Store::activity_page`, the users answer) also hides what was stored earlier; opening the
//!   eye shows it again. Closing deletes nothing: stored values of sign-in rows (which never expire) go only with the explicit
//!   `POST /api/users/signin-info/clear`, in both states.
//! - Entries anyone can cause without signing in (actor `anon`, failed sign-ins) are limited to 120 rows a minute; the rest
//!   is counted into one `system.signinFlood` row per minute. Page visits are limited to 30 writes per user per minute.
//! - Merging of page visits and refused sign-ins happens in the store (`ActivityRow::merge`).
//! - Retention runs hourly in the writer task (days from the settings, at most 500 000 rows).
//!
//! Never in an entry: passwords, tokens, cookies, keys, typed names without an account, question / answer / chat text. The
//! handlers choose the `object` and the `detail` keys from the allow-lists of the contract (`routes.md` / `rust-contracts.md`).
use crate::api::AppState;
use crate::auth::{unix_now, Role};
use crate::database::{ActivityRow, ActivitySource, Merge, Store};
use crate::security::{strip_ctrl, truncate_chars};
use axum::http::{HeaderMap, HeaderValue};
use serde_json::Value;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// Capacity of the queue between the handlers and the writer.
pub const QUEUE_CAP: usize = 2048;
/// The writer inserts a batch at this many rows ...
pub const BATCH_ROWS: usize = 100;
/// ... or at this interval (milliseconds).
pub const BATCH_MILLIS: u64 = 500;
/// Entries without a session (and failed sign-ins) written per minute; the rest is counted.
pub const ANON_ROWS_PER_MIN: u32 = 120;
/// Page-visit writes per user per minute.
pub const VISITS_PER_MIN: u32 = 30;
/// Rows the log keeps at most.
pub const ROWS_CAP: i64 = 500_000;
/// Seconds between two retention runs.
pub const RETENTION_EVERY_SECS: i64 = 3600;
const OBJECT_MAX: usize = 200;
const UA_MAX: usize = 256;
const PLACE_MAX: usize = 64;

/// What the visitor sent about itself, as Nginx sets it (always overwritten there): `X-Real-IP`, `User-Agent` and the
/// location headers. Never `X-Forwarded-For`, never the `CF-*` headers directly. Whether those headers may be believed is
/// decided before this type reads them, see [`apply_peer_rule`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientInfo {
    pub ip: Option<IpAddr>,
    pub ua: Option<String>,
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
}

/// Text from a header: valid UTF-8, control characters removed, trimmed, cut to `max` characters; empty = `None`.
fn header_text(h: &HeaderMap, name: &str, max: usize) -> Option<String> {
    let raw = h.get(name)?;
    let s = String::from_utf8(raw.as_bytes().to_vec()).ok()?;
    let clean: String = s.chars().filter(|c| !c.is_control()).collect();
    let t = truncate_chars(clean.trim(), max);
    let t = t.trim().to_string();
    (!t.is_empty()).then_some(t)
}

impl ClientInfo {
    /// Reads the headers (see the type). A missing or invalid header gives `None`, never an error.
    pub fn from_headers(h: &HeaderMap) -> ClientInfo {
        let ip = h
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<IpAddr>().ok())
            .map(normalize_ip);
        let country = h
            .get("x-geo-country")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| s.len() == 2 && s.bytes().all(|b| b.is_ascii_alphabetic()))
            .map(|s| s.to_ascii_uppercase())
            .filter(|s| s != "XX" && s != "T1");
        ClientInfo {
            ip,
            ua: header_text(h, "user-agent", UA_MAX),
            country,
            region: header_text(h, "x-geo-region", PLACE_MAX),
            city: header_text(h, "x-geo-city", PLACE_MAX),
        }
    }
}

/// The address of a connection with an IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) turned into plain IPv4.
pub fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    }
}

/// May `X-Real-IP` and `X-Geo-*` be believed for this connection? Yes when the backend is told it sits behind the bundled
/// Nginx (`XEXAM_BEHIND_PROXY=1`, set in the image), or when the TCP peer is a loopback address (a reverse proxy or a dev
/// server on the same machine). Without a peer address (a router without connect info) nothing changes: yes.
pub fn trusts_forwarded(behind_proxy: bool, peer: Option<SocketAddr>) -> bool {
    match peer {
        None => true,
        Some(p) => behind_proxy || normalize_ip(p.ip()).is_loopback(),
    }
}

/// Applies the trust rule to the request headers, once per request, before any code reads them. Untrusted connection: the
/// forwarded headers are removed and `X-Real-IP` becomes the connecting address (so a LAN visitor of the standalone binary
/// shows its own address; the location stays empty and a client cannot invent either). Trusted connection: the headers
/// stay, and a missing or invalid `X-Real-IP` falls back to the connecting address. No peer: untouched.
pub fn apply_peer_rule(h: &mut HeaderMap, peer: Option<SocketAddr>, behind_proxy: bool) {
    let Some(peer) = peer else { return };
    let trusted = trusts_forwarded(behind_proxy, Some(peer));
    if !trusted {
        for name in ["x-real-ip", "x-geo-country", "x-geo-region", "x-geo-city"] {
            h.remove(name);
        }
    }
    let valid = trusted
        && h.get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| s.trim().parse::<IpAddr>().is_ok());
    if !valid {
        if let Ok(v) = HeaderValue::from_str(&normalize_ip(peer.ip()).to_string()) {
            h.insert("x-real-ip", v);
        }
    }
}

/// Who did it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    User {
        id: i64,
        username: String,
        role: Role,
    },
    /// Nobody signed in
    Anon,
    /// The service itself
    System,
}

impl Actor {
    /// The actor of a signed-in request.
    pub fn of(u: &crate::database::SessionUser) -> Actor {
        Actor::User {
            id: u.id,
            username: u.username.clone(),
            role: u.role,
        }
    }
}

/// One thing that happened.
#[derive(Debug, Clone)]
pub struct Entry {
    pub actor: Actor,
    /// A code of the contract (`users.create`, `page.view`, ...); the labels live in the frontend
    pub action: &'static str,
    pub object: Option<String>,
    /// Allow-listed keys only (counts, names of changed fields, ...)
    pub detail: Option<Value>,
    pub ok: bool,
    /// For a failure: the reason code
    pub reason: Option<String>,
    pub client: Option<ClientInfo>,
    /// Unix seconds; `None` = now
    pub at: Option<i64>,
}

impl Entry {
    /// A successful entry without object and detail.
    pub fn new(actor: Actor, action: &'static str) -> Entry {
        Entry {
            actor,
            action,
            object: None,
            detail: None,
            ok: true,
            reason: None,
            client: None,
            at: None,
        }
    }

    /// An entry of the service itself.
    pub fn system(action: &'static str, detail: Option<Value>) -> Entry {
        Entry {
            detail,
            ..Entry::new(Actor::System, action)
        }
    }

    pub fn object(mut self, o: impl Into<String>) -> Entry {
        self.object = Some(o.into());
        self
    }

    pub fn detail(mut self, d: Value) -> Entry {
        self.detail = Some(d);
        self
    }

    pub fn client(mut self, c: &ClientInfo) -> Entry {
        self.client = Some(c.clone());
        self
    }

    /// Marks the entry as failed with a reason code.
    pub fn fail(mut self, reason: impl Into<String>) -> Entry {
        self.ok = false;
        self.reason = Some(reason.into());
        self
    }
}

/// What a handler attaches to its response (as an extension) so the hook in `dispatch` can complete the entry of its route.
#[derive(Debug, Clone, Default)]
pub struct ActivityNote {
    pub object: Option<String>,
    pub detail: Option<Value>,
}

/// Attaches a note to a response.
pub fn with_note(
    mut resp: axum::response::Response,
    object: Option<String>,
    detail: Option<Value>,
) -> axum::response::Response {
    resp.extensions_mut()
        .insert(ActivityNote { object, detail });
    resp
}

/// How a row folds into the latest matching row (decided by the action and the result).
fn merge_of(action: &str, ok: bool, reason: Option<&str>) -> Merge {
    if action == "page.view" {
        Merge::PageView
    } else if action == "auth.signin" && !ok && matches!(reason, Some("locked" | "unknownUser")) {
        Merge::Refused
    } else {
        Merge::None
    }
}

/// The database row of an entry. `with_ip` is the sign-in info switch: while it is closed the row stores no address and no
/// location (the user agent is kept).
fn to_row(e: Entry, now: i64, with_ip: bool) -> ActivityRow {
    let (actor, user_id, username, role) = match &e.actor {
        Actor::User { id, username, role } => (
            "user",
            Some(*id),
            Some(username.clone()),
            Some(role.as_str().to_string()),
        ),
        Actor::Anon => ("anon", None, None, None),
        Actor::System => ("system", None, None, None),
    };
    let merge = merge_of(e.action, e.ok, e.reason.as_deref());
    let mut c = e.client.unwrap_or_default();
    if !with_ip {
        c.ip = None;
        c.country = None;
        c.region = None;
        c.city = None;
    }
    ActivityRow {
        ts: e.at.unwrap_or(now),
        actor: actor.to_string(),
        user_id,
        username,
        role,
        action: e.action.to_string(),
        object: e
            .object
            .map(|o| truncate_chars(&strip_ctrl(&o), OBJECT_MAX)),
        detail: e.detail.map(|d| d.to_string()),
        ok: e.ok,
        reason: e.reason.map(|r| truncate_chars(&strip_ctrl(&r), 80)),
        ip: c.ip.map(|i| i.to_string()),
        country: c.country,
        region: c.region,
        city: c.city,
        ua: c.ua,
        source: ActivitySource::Backend,
        src_id: None,
        merge,
    }
}

enum Msg {
    Row(Box<ActivityRow>),
    Flush(oneshot::Sender<()>),
    Stop(oneshot::Sender<()>),
}

#[derive(Default)]
struct Flood {
    minute: i64,
    rows: u32,
    dropped: u32,
    /// The overflow of a finished minute, not yet written: (minute, rows refused)
    pending: Option<(i64, u32)>,
}

/// State shared by the senders and the writer.
struct Shared {
    geo: crate::geo::Geo,
    signin_info: AtomicBool,
    user_visits: AtomicBool,
    dropped: AtomicU64,
    flood: Mutex<Flood>,
    visits: Mutex<HashMap<i64, (i64, u32)>>,
    last_retention: AtomicI64,
    rx: Mutex<Option<mpsc::Receiver<Msg>>>,
}

/// The write side of the activity log (see the module documentation). Cheap to clone.
#[derive(Clone)]
pub struct Sink {
    tx: mpsc::Sender<Msg>,
    shared: Arc<Shared>,
}

impl Default for Sink {
    fn default() -> Sink {
        Sink::new()
    }
}

impl Sink {
    /// A sink without a writer; [`Sink::start`] starts one. Until then entries wait in the queue.
    pub fn new() -> Sink {
        let (tx, rx) = mpsc::channel(QUEUE_CAP);
        Sink {
            tx,
            shared: Arc::new(Shared {
                geo: crate::geo::Geo::new(),
                signin_info: AtomicBool::new(false),
                user_visits: AtomicBool::new(false),
                dropped: AtomicU64::new(0),
                flood: Mutex::new(Flood::default()),
                visits: Mutex::new(HashMap::new()),
                last_retention: AtomicI64::new(0),
                rx: Mutex::new(Some(rx)),
            }),
        }
    }

    /// The cached `users.signin_info` switch (whether stored addresses and locations are shown and new ones recorded).
    pub fn signin_info(&self) -> bool {
        self.shared.signin_info.load(Ordering::Relaxed)
    }

    /// The cached `activity.user_visits` switch.
    pub fn user_visits(&self) -> bool {
        self.shared.user_visits.load(Ordering::Relaxed)
    }

    /// v1.4: the place source, the lookup cache and the queue of external location lookups ([`crate::geo`]). Cheap to clone.
    pub fn geo(&self) -> &crate::geo::Geo {
        &self.shared.geo
    }

    /// Reads the switches (the eye, the visit logging and the place source) from the database into the cache (a failed
    /// read keeps the old values).
    pub fn reload_settings(&self, store: &Store) {
        match store.signin_info_enabled() {
            Ok(v) => {
                self.shared.signin_info.store(v, Ordering::Relaxed);
                self.shared.geo.set_eye(v);
            }
            Err(e) => tracing::warn!("could not read the sign-in info switch: {e}"),
        }
        match store.geo_source_is_api() {
            Ok(v) => self.shared.geo.set_api(v),
            Err(e) => tracing::warn!("could not read the location source: {e}"),
        }
        match store.activity_user_visits() {
            Ok(v) => self.shared.user_visits.store(v, Ordering::Relaxed),
            Err(e) => tracing::warn!("could not read the visit logging switch: {e}"),
        }
    }

    /// Entries lost because the queue was full since the writer last reported them.
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }

    /// Queues an entry. Never blocks; a full queue drops the entry and counts it. Also writes one English log line.
    pub fn record(&self, mut e: Entry) {
        let now = unix_now();
        // v1.4: the place of the row follows the source rule (Cloudflare headers, or the cached lookup of the address)
        if let Some(c) = e.client.as_mut() {
            let p = self.shared.geo.place_for(c);
            c.country = p.country;
            c.region = p.region;
            c.city = p.city;
        }
        let anon_flood = matches!(e.actor, Actor::Anon) || (e.action == "auth.signin" && !e.ok);
        if anon_flood && !self.flood_admit(now) {
            return;
        }
        // Entries anyone can cause without signing in are summed up by the burst logs, never one line each; system rows
        // report what the service already logged itself
        if !anon_flood && e.actor != Actor::System {
            self.log_line(&e);
        }
        self.push(to_row(e, now, self.signin_info()));
    }

    fn push(&self, row: ActivityRow) {
        if self.tx.try_send(Msg::Row(Box::new(row))).is_err() {
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn log_line(&self, e: &Entry) {
        let who = match &e.actor {
            Actor::User { username, .. } => format!("user={username:?} "),
            Actor::Anon => "user=anonymous ".to_string(),
            Actor::System => String::new(),
        };
        let object = e
            .object
            .as_deref()
            .map(|o| format!("object={:?} ", truncate_chars(&strip_ctrl(o), 80)))
            .unwrap_or_default();
        let reason = e
            .reason
            .as_deref()
            .map(|r| format!(" reason={:?}", truncate_chars(&strip_ctrl(r), 40)))
            .unwrap_or_default();
        // The address is in the line only while the sign-in info switch is open (as in the database row)
        let ip = match (&e.client, self.signin_info()) {
            (Some(ClientInfo { ip: Some(ip), .. }), true) => format!(" ip={ip}"),
            _ => String::new(),
        };
        tracing::info!(
            "activity {who}action={} {object}result={}{reason}{ip}",
            e.action,
            if e.ok { "ok" } else { "fail" },
        );
    }

    /// Claims one row of the per-minute budget of entries nobody signed in for.
    fn flood_admit(&self, now: i64) -> bool {
        let mut f = self.shared.flood.lock().unwrap_or_else(|p| p.into_inner());
        let minute = now.div_euclid(60);
        if f.minute != minute {
            // A new minute: the overflow of the old one waits for the writer (see `take_flood_report`)
            if f.dropped > 0 {
                f.pending = Some((f.minute, f.pending.map_or(0, |p| p.1) + f.dropped));
            }
            f.minute = minute;
            f.rows = 0;
            f.dropped = 0;
        }
        if f.rows < ANON_ROWS_PER_MIN {
            f.rows += 1;
            true
        } else {
            f.dropped = f.dropped.saturating_add(1);
            false
        }
    }

    /// The overflow of a finished minute as one `system.signinFlood` entry, once the minute has passed.
    fn take_flood_report(&self, now: i64) -> Option<Entry> {
        let mut f = self.shared.flood.lock().unwrap_or_else(|p| p.into_inner());
        if f.dropped > 0 && f.minute != now.div_euclid(60) {
            f.pending = Some((f.minute, f.pending.map_or(0, |p| p.1) + f.dropped));
            f.dropped = 0;
        }
        let (minute, n) = f.pending.take()?;
        Some(Entry {
            at: Some(minute * 60),
            ..Entry::system(
                "system.signinFlood",
                Some(serde_json::json!({ "dropped": n })),
            )
        })
    }

    /// Claims one page-visit write of `user` (at most [`VISITS_PER_MIN`] per minute).
    pub fn visit_allowed(&self, user: i64, now: i64) -> bool {
        let minute = now.div_euclid(60);
        let mut v = self.shared.visits.lock().unwrap_or_else(|p| p.into_inner());
        if v.len() > 10_000 {
            v.retain(|_, (m, _)| *m == minute);
        }
        let e = v.entry(user).or_insert((minute, 0));
        if e.0 != minute {
            *e = (minute, 0);
        }
        if e.1 >= VISITS_PER_MIN {
            return false;
        }
        e.1 += 1;
        true
    }

    /// Starts the writer task for `store` (once; later calls do nothing). Needs a running tokio runtime.
    pub fn start(&self, store: Arc<Store>) {
        let rx = self
            .shared
            .rx
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        let Some(rx) = rx else { return };
        self.reload_settings(&store);
        tokio::spawn(writer(self.shared.clone(), self.clone(), store, rx));
    }

    /// The writer was never started: the receiver is still parked here, so nothing would answer a message.
    fn no_writer(&self) -> bool {
        self.shared
            .rx
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
    }

    /// Waits until everything queued before this call is in the database (no-op without a writer).
    pub async fn flush(&self) {
        if self.no_writer() {
            return;
        }
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Msg::Flush(tx)).await.is_ok() {
            let _ = rx.await;
        }
    }

    /// Flushes and stops the writer (graceful shutdown, before the store closes). Releases the writer's store handle.
    pub async fn stop(&self) {
        if self.no_writer() {
            return;
        }
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Msg::Stop(tx)).await.is_ok() {
            let _ = rx.await;
        }
    }

    /// Writes rows now (events of the AI service: the sender must know they are stored).
    pub async fn write_now(
        &self,
        store: &Arc<Store>,
        rows: Vec<ActivityRow>,
    ) -> Result<(), crate::error::DbError> {
        let store = store.clone();
        tokio::task::spawn_blocking(move || store.activity_insert(&rows))
            .await
            .map_err(|_| crate::error::DbError::Join)?
    }
}

async fn write_batch(store: &Arc<Store>, batch: &mut Vec<ActivityRow>) {
    if batch.is_empty() {
        return;
    }
    let rows = std::mem::take(batch);
    let n = rows.len();
    let s = store.clone();
    match tokio::task::spawn_blocking(move || s.activity_insert(&rows)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!("could not write {n} activity rows: {e}"),
        Err(_) => tracing::warn!("could not write {n} activity rows: the task ended abnormally"),
    }
}

/// System rows the writer adds from its own bookkeeping (dropped entries, sign-in flood), and the hourly retention.
async fn housekeeping(
    shared: &Arc<Shared>,
    sink: &Sink,
    store: &Arc<Store>,
    batch: &mut Vec<ActivityRow>,
) {
    let now = unix_now();
    let dropped = shared.dropped.swap(0, Ordering::Relaxed);
    if dropped > 0 {
        batch.push(to_row(
            Entry::system(
                "system.logDropped",
                Some(serde_json::json!({ "count": dropped })),
            ),
            now,
            false,
        ));
    }
    if let Some(e) = sink.take_flood_report(now) {
        batch.push(to_row(e, now, false));
    }
    let last = shared.last_retention.load(Ordering::Relaxed);
    if now.saturating_sub(last) >= RETENTION_EVERY_SECS {
        shared.last_retention.store(now, Ordering::Relaxed);
        write_batch(store, batch).await;
        let s = store.clone();
        let res = tokio::task::spawn_blocking(move || {
            let days = s.activity_retention_days()?;
            s.activity_retention(unix_now(), days, ROWS_CAP)
        })
        .await;
        match res {
            Ok(Ok(0)) => {}
            Ok(Ok(n)) => {
                tracing::info!("activity log: removed {n} old rows");
                batch.push(to_row(
                    Entry::system("system.logPurge", Some(serde_json::json!({ "deleted": n }))),
                    unix_now(),
                    false,
                ));
            }
            Ok(Err(e)) => tracing::warn!("activity log retention failed: {e}"),
            Err(_) => tracing::warn!("activity log retention failed: the task ended abnormally"),
        }
    }
}

async fn writer(shared: Arc<Shared>, sink: Sink, store: Arc<Store>, mut rx: mpsc::Receiver<Msg>) {
    let mut batch: Vec<ActivityRow> = Vec::new();
    let mut tick = tokio::time::interval(Duration::from_millis(BATCH_MILLIS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            m = rx.recv() => match m {
                None => {
                    write_batch(&store, &mut batch).await;
                    break;
                }
                Some(Msg::Row(r)) => {
                    batch.push(*r);
                    if batch.len() >= BATCH_ROWS {
                        write_batch(&store, &mut batch).await;
                    }
                }
                Some(Msg::Flush(done)) => {
                    housekeeping(&shared, &sink, &store, &mut batch).await;
                    write_batch(&store, &mut batch).await;
                    let _ = done.send(());
                }
                Some(Msg::Stop(done)) => {
                    rx.close();
                    while let Some(m) = rx.recv().await {
                        if let Msg::Row(r) = m {
                            batch.push(*r);
                        }
                    }
                    housekeeping(&shared, &sink, &store, &mut batch).await;
                    write_batch(&store, &mut batch).await;
                    let _ = done.send(());
                    break;
                }
            },
            _ = tick.tick() => {
                housekeeping(&shared, &sink, &store, &mut batch).await;
                write_batch(&store, &mut batch).await;
            }
        }
    }
}

// ---------------------------------------------------------------- the dispatch hook

/// The activity code of every POST route: `(path, action, user_important)`. An administrator's POST is always recorded (a path
/// that is not listed becomes `admin.request` with the path as object, so a future route is never silently unlogged); a plain
/// user's only when `user_important`. An empty action means the route records itself (sign-in, sign-out, page visits).
pub const ROUTE_ACTIONS: &[(&str, &str, bool)] = &[
    ("/api/auth/login", "", false),
    ("/api/auth/logout", "", false),
    ("/api/auth/password", "auth.password", true),
    ("/api/upload", "bank.upload", true),
    ("/api/import/confirm", "bank.import", true),
    ("/api/import/cancel", "bank.importCancel", true),
    ("/api/banks/delete", "bank.delete", true),
    ("/api/banks/sheets/delete", "bank.sheetsDelete", true),
    ("/api/attempt", "study.submit", true),
    ("/api/records/delete", "records.delete", true),
    ("/api/reset", "progress.reset", true),
    ("/api/settings", "settings.update", false),
    ("/api/settings/reset", "settings.reset", false),
    ("/api/shutdown", "system.shutdown", false),
    ("/api/users/create", "users.create", false),
    ("/api/users/update", "users.update", false),
    ("/api/users/password", "users.password", false),
    ("/api/users/delete", "users.delete", false),
    ("/api/users/unlock", "users.unlock", false),
    ("/api/users/signin-info", "users.signinInfo", false),
    (
        "/api/users/signin-info/clear",
        "users.signinInfoClear",
        false,
    ),
    ("/api/users/geo-source", "users.geoSource", false),
    ("/api/activity/settings", "logs.settings", false),
    ("/api/activity/visit", "", false),
];

/// The reason code of a failed request, from its status.
fn reason_of(status: u16) -> &'static str {
    match status {
        400 => "badRequest",
        401 => "signInRequired",
        403 => "forbidden",
        404 => "notFound",
        409 => "conflict",
        413 => "tooLarge",
        415 => "unsupported",
        429 => "tooManyRequests",
        _ => "error",
    }
}

/// Called by `dispatch` after a POST handler ran for a signed-in user: records the entry of its route (see
/// [`ROUTE_ACTIONS`]) and strips the handler's [`ActivityNote`] from the response.
pub fn after_post(
    sink: &Sink,
    client: &ClientInfo,
    path: &str,
    user: &crate::database::SessionUser,
    resp: &mut axum::response::Response,
) {
    let note = resp.extensions_mut().remove::<ActivityNote>();
    let admin = user.role == Role::Admin;
    let (action, important) = match ROUTE_ACTIONS.iter().find(|(p, ..)| *p == path) {
        Some((_, "", _)) => return,
        Some((_, a, imp)) => (*a, *imp),
        None => ("admin.request", false),
    };
    if !admin && !important {
        return;
    }
    let status = resp.status().as_u16();
    let mut e = Entry::new(Actor::of(user), action);
    // The clearing of sign-in info stores no address or device itself: the log is clean right after it
    if action != "users.signinInfoClear" {
        e = e.client(client);
    }
    if action == "admin.request" {
        e.object = Some(path.to_string());
    }
    if (200..300).contains(&status) {
        if let Some(n) = note {
            if n.object.is_some() {
                e.object = n.object;
            }
            e.detail = n.detail;
        }
    } else {
        e = e.fail(reason_of(status));
    }
    sink.record(e);
}

/// The `system.guard` entry for one report of the Host / Origin check (counts only: the refused text is the visitor's).
pub fn guard_entry(report: &crate::logs::BurstReport) -> Entry {
    let n = match report {
        crate::logs::BurstReport::One(_) => 1,
        crate::logs::BurstReport::Many { count, .. } => *count,
    };
    Entry::system("system.guard", Some(serde_json::json!({ "count": n })))
}

/// Starts the small monitors that write system rows: the AI link going up or down (`system.aiLink`). The handles are
/// aborted by the caller at shutdown.
pub fn spawn_monitors(state: &AppState) -> Vec<tokio::task::JoinHandle<()>> {
    let st = state.clone();
    let link = tokio::spawn(async move {
        let mut prev: Option<bool> = None;
        loop {
            let up = st.0.ai_link.state().connected;
            if prev != Some(up) && (prev.is_some() || up) {
                st.0.activity.record(Entry::system(
                    "system.aiLink",
                    Some(serde_json::json!({ "state": if up { "up" } else { "down" } })),
                ));
            }
            prev = Some(up);
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    vec![link]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::ActivityFilter;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&str, &[u8])]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).expect("name"),
                HeaderValue::from_bytes(v).expect("value"),
            );
        }
        h
    }

    fn open() -> (tempfile::TempDir, Arc<Store>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let s = Arc::new(Store::open(&dir.path().join("x.db")).expect("open"));
        (dir, s)
    }

    fn page(store: &Store) -> Vec<crate::database::ActivityEntry> {
        store
            .activity_page(&ActivityFilter::default())
            .expect("page")
            .entries
    }

    #[test]
    fn client_info_reads_only_the_nginx_headers() {
        let h = headers(&[
            ("x-real-ip", b"203.0.113.7"),
            ("user-agent", b"Agent/1.0 \t (test)"),
            ("x-geo-country", b"zz"),
            ("x-geo-region", "  Example Region\t ".as_bytes()),
            ("x-geo-city", "Exämple City".as_bytes()),
            ("x-forwarded-for", b"198.51.100.9"),
            ("cf-ipcountry", b"QQ"),
        ]);
        let c = ClientInfo::from_headers(&h);
        assert_eq!(c.ip, Some("203.0.113.7".parse().expect("ip")));
        assert_eq!(c.ua.as_deref(), Some("Agent/1.0  (test)"));
        assert_eq!(c.country.as_deref(), Some("ZZ"));
        assert_eq!(c.region.as_deref(), Some("Example Region"));
        assert_eq!(c.city.as_deref(), Some("Exämple City"));
        // IPv6 parses too
        let c = ClientInfo::from_headers(&headers(&[("x-real-ip", b"2001:db8::1")]));
        assert!(c.ip.is_some());
    }

    #[test]
    fn client_info_rejects_bad_values() {
        for bad in [&b"XX"[..], b"T1", b"Z", b"ZZZ", b"1A", b""] {
            let c = ClientInfo::from_headers(&headers(&[("x-geo-country", bad)]));
            assert_eq!(c.country, None, "{bad:?}");
        }
        let c = ClientInfo::from_headers(&headers(&[
            ("x-real-ip", b"not-an-ip"),
            ("x-geo-city", b"   "),
            ("x-geo-region", &[0xff, 0xfe]),
        ]));
        assert_eq!(c, ClientInfo::default());
        let long = "a".repeat(300);
        let c = ClientInfo::from_headers(&headers(&[
            ("user-agent", long.as_bytes()),
            ("x-geo-city", long.as_bytes()),
        ]));
        assert_eq!(c.ua.as_deref().map(str::len), Some(256));
        assert_eq!(c.city.as_deref().map(str::len), Some(64));
    }

    fn peer(s: &str) -> Option<SocketAddr> {
        Some(s.parse().expect("socket address"))
    }

    fn after_rule(
        pairs: &[(&str, &[u8])],
        p: Option<SocketAddr>,
        behind_proxy: bool,
    ) -> ClientInfo {
        let mut h = headers(pairs);
        apply_peer_rule(&mut h, p, behind_proxy);
        ClientInfo::from_headers(&h)
    }

    const FORGED: [(&str, &[u8]); 4] = [
        ("x-real-ip", b"198.51.100.9"),
        ("x-geo-country", b"ZZ"),
        ("x-geo-region", b"Example Region"),
        ("x-geo-city", b"Example City"),
    ];

    #[test]
    fn trust_rule_standalone_lan_peer_ignores_the_headers() {
        // Not behind the bundled Nginx, peer on the LAN: forged headers are dropped, the peer address is recorded
        let c = after_rule(&FORGED, peer("192.0.2.10:5555"), false);
        assert_eq!(c.ip, Some("192.0.2.10".parse().expect("ip")));
        assert_eq!((c.country, c.region, c.city), (None, None, None));
        // no headers at all: same
        let c = after_rule(&[], peer("192.0.2.10:5555"), false);
        assert_eq!(c.ip, Some("192.0.2.10".parse().expect("ip")));
        // IPv6 peer
        let c = after_rule(&FORGED, peer("[2001:db8::5]:1"), false);
        assert_eq!(c.ip, Some("2001:db8::5".parse().expect("ip")));
    }

    #[test]
    fn trust_rule_behind_proxy_keeps_the_headers() {
        let c = after_rule(&FORGED, peer("192.0.2.10:5555"), true);
        assert_eq!(c.ip, Some("198.51.100.9".parse().expect("ip")));
        assert_eq!(c.country.as_deref(), Some("ZZ"));
        assert_eq!(c.region.as_deref(), Some("Example Region"));
        assert_eq!(c.city.as_deref(), Some("Example City"));
        // trusted but the address header is missing or invalid: the peer address
        let c = after_rule(&[], peer("192.0.2.10:5555"), true);
        assert_eq!(c.ip, Some("192.0.2.10".parse().expect("ip")));
        let c = after_rule(&[("x-real-ip", b"nonsense")], peer("192.0.2.10:1"), true);
        assert_eq!(c.ip, Some("192.0.2.10".parse().expect("ip")));
    }

    #[test]
    fn trust_rule_loopback_peer_is_trusted_without_the_flag() {
        for p in ["127.0.0.1:1", "[::1]:1", "[::ffff:127.0.0.1]:1"] {
            let c = after_rule(&FORGED, peer(p), false);
            assert_eq!(c.ip, Some("198.51.100.9".parse().expect("ip")), "{p}");
            assert_eq!(c.country.as_deref(), Some("ZZ"), "{p}");
            // no header from a loopback proxy: the loopback address itself
            let c = after_rule(&[], peer(p), false);
            assert!(c.ip.expect("ip").is_loopback(), "{p}");
        }
    }

    #[test]
    fn trust_rule_normalises_mapped_addresses_and_keeps_user_agent() {
        let c = after_rule(
            &[("user-agent", b"Agent/1.0")],
            peer("[::ffff:192.0.2.77]:9"),
            false,
        );
        assert_eq!(c.ip, Some("192.0.2.77".parse().expect("ip")));
        assert_eq!(c.ua.as_deref(), Some("Agent/1.0"));
        // a mapped address in a trusted header is normalised too
        let c = after_rule(
            &[("x-real-ip", b"::ffff:203.0.113.4")],
            peer("192.0.2.1:1"),
            true,
        );
        assert_eq!(c.ip, Some("203.0.113.4".parse().expect("ip")));
        // no peer (router without connect info): headers are used as before
        let c = after_rule(&FORGED, None, false);
        assert_eq!(c.ip, Some("198.51.100.9".parse().expect("ip")));
        assert_eq!(c.city.as_deref(), Some("Example City"));
        assert!(!trusts_forwarded(false, peer("192.0.2.1:1")));
        assert!(trusts_forwarded(false, None));
    }

    #[test]
    fn merge_rules_follow_the_action() {
        assert_eq!(merge_of("page.view", true, None), Merge::PageView);
        assert_eq!(
            merge_of("auth.signin", false, Some("locked")),
            Merge::Refused
        );
        assert_eq!(
            merge_of("auth.signin", false, Some("unknownUser")),
            Merge::Refused
        );
        assert_eq!(
            merge_of("auth.signin", false, Some("badPassword")),
            Merge::None
        );
        assert_eq!(merge_of("auth.signin", true, None), Merge::None);
        assert_eq!(merge_of("users.create", true, None), Merge::None);
    }

    #[test]
    fn a_row_keeps_address_and_location_only_while_the_switch_is_open() {
        let c = ClientInfo {
            ip: Some("203.0.113.7".parse().expect("ip")),
            ua: Some("Agent".into()),
            country: Some("ZZ".into()),
            region: Some("R".into()),
            city: Some("C".into()),
        };
        let r = to_row(
            Entry::new(Actor::Anon, "x.y").client(&c),
            1_700_000_000,
            true,
        );
        assert_eq!(r.ip.as_deref(), Some("203.0.113.7"));
        assert_eq!(
            (r.country.as_deref(), r.region.as_deref(), r.city.as_deref()),
            (Some("ZZ"), Some("R"), Some("C"))
        );
        assert_eq!(r.ua.as_deref(), Some("Agent"));
        let r = to_row(
            Entry::new(Actor::Anon, "x.y").client(&c),
            1_700_000_000,
            false,
        );
        assert_eq!(
            (&r.ip, &r.country, &r.region, &r.city),
            (&None, &None, &None, &None)
        );
        assert_eq!(r.ua.as_deref(), Some("Agent"));
    }

    #[test]
    fn route_table_has_unique_paths() {
        let mut seen = std::collections::HashSet::new();
        for (p, ..) in ROUTE_ACTIONS {
            assert!(seen.insert(*p), "duplicate {p}");
        }
    }

    #[test]
    fn every_post_route_has_a_table_row() {
        for r in crate::api::auth::ROUTES
            .iter()
            .filter(|r| r.method == "POST")
        {
            assert!(
                ROUTE_ACTIONS.iter().any(|(p, ..)| *p == r.path),
                "POST {} has no row in ROUTE_ACTIONS",
                r.path
            );
        }
    }

    #[test]
    fn anonymous_flood_is_capped_and_reported_once() {
        let s = Sink::new();
        let t0 = 1_800_000_000;
        let ok = (0..200).filter(|_| s.flood_admit(t0)).count();
        assert_eq!(ok, ANON_ROWS_PER_MIN as usize);
        // nothing to report inside the minute
        assert!(s.take_flood_report(t0 + 5).is_none());
        let e = s.take_flood_report(t0 + 61).expect("report");
        assert_eq!(e.action, "system.signinFlood");
        assert_eq!(
            e.detail.as_ref().and_then(|d| d["dropped"].as_u64()),
            Some(80)
        );
        // the next minute starts with a fresh budget
        assert!(s.flood_admit(t0 + 61));
        assert!(s.take_flood_report(t0 + 130).is_none());
    }

    #[test]
    fn visits_are_limited_per_user_and_minute() {
        let s = Sink::new();
        let t = 1_800_000_000;
        let n = (0..50).filter(|_| s.visit_allowed(7, t)).count();
        assert_eq!(n, VISITS_PER_MIN as usize);
        assert!(s.visit_allowed(8, t));
        assert!(s.visit_allowed(7, t + 60));
    }

    #[tokio::test]
    async fn writer_stores_the_address_only_while_the_eye_is_open() {
        let (_d, store) = open();
        let sink = Sink::new();
        sink.start(store.clone());
        let c = ClientInfo {
            ip: Some("203.0.113.7".parse().expect("ip")),
            ua: Some("Agent".into()),
            country: Some("ZZ".into()),
            region: None,
            city: Some("Example City".into()),
        };
        let alice = Actor::User {
            id: 1,
            username: "alice".into(),
            role: Role::Admin,
        };
        // closed (default): nothing is recorded, the device is
        sink.record(
            Entry::new(alice.clone(), "users.create")
                .object("bob")
                .client(&c),
        );
        sink.flush().await;
        store.set_signin_info(true, Some(1)).expect("on");
        let rows = page(&store);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (&rows[0].ip, &rows[0].country, &rows[0].city),
            (&None, &None, &None)
        );
        assert_eq!(rows[0].ua.as_deref(), Some("Agent"));
        // open: recording resumes
        sink.reload_settings(&store);
        sink.record(Entry::new(alice.clone(), "users.update").client(&c));
        sink.flush().await;
        let rows = page(&store);
        assert_eq!(rows.len(), 2);
        let with = rows.iter().filter(|r| r.ip.is_some()).count();
        assert_eq!(with, 1);
        // closed again: the stored value is hidden, nothing new is stored, nothing is deleted
        store.set_signin_info(false, Some(1)).expect("off");
        sink.reload_settings(&store);
        sink.record(Entry::new(alice, "users.delete").client(&c));
        sink.flush().await;
        let rows = page(&store);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.ip.is_none() && r.city.is_none()));
        // reopened: the earlier record shows again, the one made while closed has none
        store.set_signin_info(true, Some(1)).expect("on");
        let rows = page(&store);
        assert_eq!(
            rows.iter()
                .filter(|r| r.ip.as_deref() == Some("203.0.113.7"))
                .count(),
            1
        );
        assert_eq!(rows.iter().filter(|r| r.ip.is_none()).count(), 2);
        sink.stop().await;
    }

    #[tokio::test]
    async fn full_queue_drops_and_the_writer_reports_it() {
        let (_d, store) = open();
        let sink = Sink::new();
        // no writer yet: the queue fills up
        for _ in 0..QUEUE_CAP + 10 {
            sink.record(Entry::system("system.start", None));
        }
        assert_eq!(sink.dropped(), 10);
        sink.start(store.clone());
        sink.flush().await;
        assert_eq!(sink.dropped(), 0);
        let rows = store
            .activity_page(&ActivityFilter {
                limit: Some(200),
                action: Some("system.logDropped".into()),
                ..Default::default()
            })
            .expect("page")
            .entries;
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].detail.as_ref().and_then(|d| d["count"].as_u64()),
            Some(10)
        );
        sink.stop().await;
    }

    #[tokio::test]
    async fn stop_flushes_what_is_queued() {
        let (_d, store) = open();
        let sink = Sink::new();
        sink.start(store.clone());
        for _ in 0..5 {
            sink.record(Entry::system("system.stop", None));
        }
        sink.stop().await;
        assert_eq!(page(&store).len(), 5);
    }

    #[tokio::test]
    async fn after_post_records_by_route_role_and_result() {
        let (_d, store) = open();
        let sink = Sink::new();
        sink.start(store.clone());
        let c = ClientInfo::default();
        let user = |role| crate::database::SessionUser {
            id: 3,
            username: "alice".into(),
            role,
            must_change_password: false,
            can_manage_banks: true,
            ai_allowed: false,
            uid: String::new(),
        };
        let resp = |status: u16| {
            axum::response::Response::builder()
                .status(status)
                .body(axum::body::Body::empty())
                .expect("response")
        };
        // admin: listed route with a note
        let mut r = with_note(
            resp(200),
            Some("bob".into()),
            Some(serde_json::json!({"role": "user"})),
        );
        after_post(&sink, &c, "/api/users/create", &user(Role::Admin), &mut r);
        assert!(r.extensions().get::<ActivityNote>().is_none());
        // admin: unlisted route
        after_post(
            &sink,
            &c,
            "/api/new/thing",
            &user(Role::Admin),
            &mut resp(200),
        );
        // admin: failure
        after_post(
            &sink,
            &c,
            "/api/users/delete",
            &user(Role::Admin),
            &mut resp(409),
        );
        // user: important only
        after_post(
            &sink,
            &c,
            "/api/settings",
            &user(Role::User),
            &mut resp(200),
        );
        after_post(&sink, &c, "/api/reset", &user(Role::User), &mut resp(200));
        // explicit routes record nothing here
        after_post(
            &sink,
            &c,
            "/api/auth/logout",
            &user(Role::Admin),
            &mut resp(200),
        );
        sink.flush().await;
        let mut rows = page(&store);
        rows.reverse();
        let got: Vec<(String, Option<String>, bool, Option<String>)> = rows
            .iter()
            .map(|r| (r.action.clone(), r.object.clone(), r.ok, r.reason.clone()))
            .collect();
        assert_eq!(
            got,
            vec![
                (
                    "users.create".to_string(),
                    Some("bob".to_string()),
                    true,
                    None
                ),
                (
                    "admin.request".to_string(),
                    Some("/api/new/thing".to_string()),
                    true,
                    None
                ),
                (
                    "users.delete".to_string(),
                    None,
                    false,
                    Some("conflict".to_string())
                ),
                ("progress.reset".to_string(), None, true, None),
            ]
        );
        sink.stop().await;
    }

    #[tokio::test]
    async fn the_entry_of_the_clear_stores_no_address_or_location() {
        let (_d, store) = open();
        let sink = Sink::new();
        store.set_signin_info(true, Some(1)).expect("open the eye");
        sink.start(store.clone());
        let c = ClientInfo {
            ip: Some("203.0.113.9".parse().expect("ip")),
            ua: Some("Demo/1".into()),
            country: Some("ZZ".into()),
            region: Some("Example Region".into()),
            city: Some("Example City".into()),
        };
        let admin = crate::database::SessionUser {
            id: 1,
            username: "admin".into(),
            role: Role::Admin,
            must_change_password: false,
            can_manage_banks: true,
            ai_allowed: false,
            uid: String::new(),
        };
        let mut resp = axum::response::Response::builder()
            .status(200)
            .body(axum::body::Body::empty())
            .expect("response");
        after_post(&sink, &c, "/api/users/signin-info/clear", &admin, &mut resp);
        let mut resp = axum::response::Response::builder()
            .status(200)
            .body(axum::body::Body::empty())
            .expect("response");
        after_post(&sink, &c, "/api/users/unlock", &admin, &mut resp);
        sink.flush().await;
        let rows = page(&store);
        let clear = rows
            .iter()
            .find(|r| r.action == "users.signinInfoClear")
            .expect("clear row");
        assert_eq!(
            (&clear.ip, &clear.country, &clear.region, &clear.city),
            (&None, &None, &None, &None)
        );
        // other entries still carry the client
        let other = rows
            .iter()
            .find(|r| r.action == "users.unlock")
            .expect("row");
        assert_eq!(other.ip.as_deref(), Some("203.0.113.9"));
        sink.stop().await;
    }
}
