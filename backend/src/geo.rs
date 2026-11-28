//! Where the place of a sign-in address comes from (v1.4).
//!
//! Two sources, chosen by the administrator (`system_settings` key `geo.source`, cached here):
//! - `cloudflare` (default): the place is whatever Cloudflare's request headers say (Nginx sets `X-Geo-*`). Nothing is sent
//!   anywhere.
//! - `api`: the place comes ONLY from an external lookup that ai-service runs for this backend. The backend has no internet
//!   by design and never calls ai-service: ai-service pulls work with a long poll (`POST /internal/ai/geo/next`), looks the
//!   addresses up at the configured provider and posts the answers back (`POST /internal/ai/geo/result`). In this mode
//!   Cloudflare's place headers are ignored for the place (the visitor address still comes from the trusted-proxy rule).
//!
//! This module holds everything the backend needs for that, in memory:
//! - [`is_public`]: the reserved-address filter. Private, loopback, link-local, CGNAT, documentation and other reserved
//!   addresses are never queued, so they can never leave the machine.
//! - The cache (one entry per address, [`CACHE_TTL_SECS`] for a place, [`CACHE_FAIL_SECS`] for a failure, at most
//!   [`CACHE_MAX`] entries). Activity rows are written synchronously and read their place from the cache only
//!   ([`Geo::place_for`]); nothing but a successful sign-in and the once-a-minute last-activity write ever asks for a
//!   lookup ([`Geo::request`]).
//! - The job queue (at most [`QUEUE_MAX`], stale jobs dropped), the jobs in flight, the per-minute cap on hand-outs
//!   ([`LOOKUPS_PER_MIN`]) and the generation counter that makes `clear` drop late results.
//! - The backfill lane (v1.6): when the source is switched to the API (or its settings are saved again), every stored public
//!   address that still has no place is looked up once ([`Geo::start_backfill`]), most recently seen first, after the live
//!   jobs and within the same per-minute cap. The lane is not bounded by [`QUEUE_MAX`] and not dropped by the stale rule; a
//!   `rateLimited` answer pauses it for [`BACKFILL_PAUSE_SECS`]; it lives in memory only and is emptied when the eye
//!   closes, when the source goes back to Cloudflare and on "clear records".
//!
//! Nothing is queued, handed out or cached while the sign-in info switch (the eye) is closed. No address, key or provider
//! text is ever written to the log by this module.
use crate::activity::ClientInfo;
use crate::auth::{token_hash, unix_now};
use crate::database::Place;
use crate::security::{strip_ctrl, truncate_chars};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::Notify;

/// Seconds a found place (or a success without fields) stays in the cache.
pub const CACHE_TTL_SECS: i64 = 86_400;
/// Seconds a failure stays in the cache (not for `rateLimited` and `notConfigured`).
pub const CACHE_FAIL_SECS: i64 = 900;
/// Most entries in the cache.
pub const CACHE_MAX: usize = 4096;
/// Most jobs waiting to be handed out; further requests are dropped (and counted).
pub const QUEUE_MAX: usize = 256;
/// A job that waited this long (seconds) is dropped.
pub const QUEUE_STALE_SECS: i64 = 120;
/// A job handed out without a result for this long (seconds) is forgotten.
pub const INFLIGHT_STALE_SECS: i64 = 60;
/// Jobs handed out per wall-clock minute; the rest waits in the queue.
pub const LOOKUPS_PER_MIN: u32 = 30;
/// Jobs in one `next` answer.
pub const BATCH_MAX: usize = 10;
/// Seconds the whole backfill pauses after a `rateLimited` answer for one of its lookups.
pub const BACKFILL_PAUSE_SECS: i64 = 900;
/// Most attempts per backfill address (timeouts, unreachable answers and lost hand-outs are retried).
pub const BACKFILL_MAX_ATTEMPTS: u8 = 3;
/// Upper bound of `waitMs` of a `next` request.
pub const POLL_WAIT_MAX_MS: u64 = 20_000;
/// Longest part of a place (characters).
const PLACE_MAX: usize = 64;

/// Who gets the place of a lookup when it arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// `users.last_login_*` of this user
    Signin(i64),
    /// `users.last_active_*` of this user
    Active(i64),
    /// v3.1: `devices.signin_*` of this device
    DeviceSignin(i64),
    /// v3.1: `devices.active_*` of this device
    DeviceActive(i64),
    /// Every record of the address that still has no place (the backfill lane, v1.6)
    Backfill,
}

/// The failure vocabulary shared with ai-service (`/internal/ai/geo/result`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailCode {
    Timeout,
    Unreachable,
    Redirect,
    HttpStatus,
    NotJson,
    TooLarge,
    ProviderFail,
    NoFields,
    KeyProblem,
    RateLimited,
    NotConfigured,
    Refused,
}

impl FailCode {
    /// The code as written on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            FailCode::Timeout => "timeout",
            FailCode::Unreachable => "unreachable",
            FailCode::Redirect => "redirect",
            FailCode::HttpStatus => "httpStatus",
            FailCode::NotJson => "notJson",
            FailCode::TooLarge => "tooLarge",
            FailCode::ProviderFail => "providerFail",
            FailCode::NoFields => "noFields",
            FailCode::KeyProblem => "keyProblem",
            FailCode::RateLimited => "rateLimited",
            FailCode::NotConfigured => "notConfigured",
            FailCode::Refused => "refused",
        }
    }

    /// The code of a wire text; `None` for anything unknown.
    pub fn parse(s: &str) -> Option<FailCode> {
        Some(match s {
            "timeout" => FailCode::Timeout,
            "unreachable" => FailCode::Unreachable,
            "redirect" => FailCode::Redirect,
            "httpStatus" => FailCode::HttpStatus,
            "notJson" => FailCode::NotJson,
            "tooLarge" => FailCode::TooLarge,
            "providerFail" => FailCode::ProviderFail,
            "noFields" => FailCode::NoFields,
            "keyProblem" => FailCode::KeyProblem,
            "rateLimited" => FailCode::RateLimited,
            "notConfigured" => FailCode::NotConfigured,
            "refused" => FailCode::Refused,
            _ => return None,
        })
    }
}

/// What ai-service reports for one lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The place (possibly with no field at all)
    Place(Place),
    Failed(FailCode),
}

/// What [`Geo::complete`] hands to the caller that writes it into the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completed {
    pub ip: String,
    pub place: Place,
    /// Unix seconds: activity rows of the address written since then get the place
    pub since: i64,
    pub signin_users: Vec<i64>,
    pub active_users: Vec<i64>,
    /// v3.1: the devices whose `signin_*` / `active_*` columns get the place (`Store::geo_apply_devices`)
    pub signin_devices: Vec<i64>,
    pub active_devices: Vec<i64>,
    /// The lookup belongs to the backfill: fill EVERY placeless record of the address (`Store::geo_fill_all`), whatever
    /// its age or owner, instead of only the recent rows and `signin_users` / `active_users`
    pub backfill: bool,
}

// ---------------------------------------------------------------- the reserved-address filter

/// `(base, prefix length)` of every IPv4 range that is never looked up.
const RESERVED_V4: &[([u8; 4], u8)] = &[
    ([0, 0, 0, 0], 8),
    ([10, 0, 0, 0], 8),
    ([100, 64, 0, 0], 10),
    ([127, 0, 0, 0], 8),
    ([169, 254, 0, 0], 16),
    ([172, 16, 0, 0], 12),
    ([192, 0, 0, 0], 24),
    ([192, 0, 2, 0], 24),
    ([192, 31, 196, 0], 24),
    ([192, 52, 193, 0], 24),
    ([192, 88, 99, 0], 24),
    ([192, 168, 0, 0], 16),
    ([192, 175, 48, 0], 24),
    ([198, 18, 0, 0], 15),
    ([198, 51, 100, 0], 24),
    ([203, 0, 113, 0], 24),
    ([224, 0, 0, 0], 4),
    ([240, 0, 0, 0], 4),
];

fn v4_in(ip: Ipv4Addr, base: [u8; 4], prefix: u8) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    };
    (u32::from(ip) & mask) == (u32::from(Ipv4Addr::from(base)) & mask)
}

fn v6_public(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    // Only global unicast 2000::/3 can be public
    if s[0] & 0xe000 != 0x2000 {
        return false;
    }
    // 2001::/23 (protocol assignments, Teredo, benchmarking, ORCHID, ...)
    if s[0] == 0x2001 && s[1] < 0x0200 {
        return false;
    }
    // 2001:db8::/32 documentation
    if s[0] == 0x2001 && s[1] == 0x0db8 {
        return false;
    }
    // 2002::/16 6to4
    if s[0] == 0x2002 {
        return false;
    }
    // 3fff::/20 documentation
    if s[0] == 0x3fff && s[1] < 0x1000 {
        return false;
    }
    true
}

/// May this address be sent to an external provider? `false` for every private, loopback, link-local, CGNAT, documentation
/// and otherwise reserved address (an IPv4-mapped IPv6 address is judged as the IPv4 address it carries).
pub fn is_public(ip: IpAddr) -> bool {
    match crate::activity::normalize_ip(ip) {
        IpAddr::V4(v4) => !RESERVED_V4.iter().any(|(b, p)| v4_in(v4, *b, *p)),
        IpAddr::V6(v6) => v6_public(v6),
    }
}

// ---------------------------------------------------------------- places

/// A place text: control characters removed, trimmed, cut to 64 characters; empty = `None`.
pub fn clean_text(s: &str) -> Option<String> {
    let t = truncate_chars(strip_ctrl(s).trim(), PLACE_MAX);
    let t = t.trim().to_string();
    (!t.is_empty()).then_some(t)
}

/// The stored place of a provider answer: `country` = the country code uppercased when it is exactly two ASCII letters and
/// not `XX` / `T1`, else the cleaned country name; region and city cleaned.
pub fn clean_place(
    country: Option<&str>,
    country_code: Option<&str>,
    region: Option<&str>,
    city: Option<&str>,
) -> Place {
    let code = country_code
        .and_then(clean_text)
        .filter(|c| c.len() == 2 && c.bytes().all(|b| b.is_ascii_alphabetic()))
        .map(|c| c.to_ascii_uppercase())
        .filter(|c| c != "XX" && c != "T1");
    Place {
        country: code.or_else(|| country.and_then(clean_text)),
        region: region.and_then(clean_text),
        city: city.and_then(clean_text),
    }
}

// ---------------------------------------------------------------- state

#[derive(Debug, Clone)]
struct Job {
    id: String,
    ip: IpAddr,
    gen: u64,
    queued_at: i64,
    targets: Vec<Target>,
    /// Backfill jobs: hand-outs used so far, this one included
    attempts: u8,
}

struct InFlight {
    job: Job,
    handed_at: i64,
}

struct CacheEntry {
    place: Place,
    expires: i64,
}

#[derive(Default)]
struct State {
    cache: HashMap<IpAddr, CacheEntry>,
    queue: VecDeque<Job>,
    inflight: HashMap<String, InFlight>,
    /// The backfill lane: addresses (with the attempts used) still to be handed out, next one first
    backfill: VecDeque<(IpAddr, u8)>,
    /// Every address the running backfill has not finished (waiting, in flight or retried); dedupes the lane
    backfill_set: HashSet<IpAddr>,
    /// No backfill hand-out before this unix second (after a `rateLimited` answer)
    backfill_paused_until: i64,
    gen: u64,
    /// The wall-clock minute `handed` counts for
    minute: i64,
    handed: u32,
    /// Requests dropped or jobs that went stale since the last log line
    skipped: u32,
    /// The minute of the last log line about skipped lookups
    logged_minute: i64,
}

struct Inner {
    api: AtomicBool,
    eye: AtomicBool,
    closed: AtomicBool,
    notify: Notify,
    state: Mutex<State>,
}

/// The place source and the lookup bookkeeping (see the module documentation). Cheap to clone.
#[derive(Clone)]
pub struct Geo(Arc<Inner>);

impl Default for Geo {
    fn default() -> Geo {
        Geo::new()
    }
}

impl Geo {
    /// Cloudflare mode, eye closed, empty cache.
    pub fn new() -> Geo {
        Geo(Arc::new(Inner {
            api: AtomicBool::new(false),
            eye: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            notify: Notify::new(),
            state: Mutex::new(State::default()),
        }))
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The cached place source: `true` = the external lookup (`api`).
    pub fn is_api(&self) -> bool {
        self.0.api.load(Ordering::Relaxed)
    }

    /// Sets the cached place source. Switching to Cloudflare forgets the queue; switching to the API wakes a waiting poll.
    pub fn set_api(&self, api: bool) {
        let was = self.0.api.swap(api, Ordering::Relaxed);
        if was && !api {
            self.drop_pending();
        }
        if api {
            self.0.notify.notify_one();
        }
    }

    /// Sets the cached sign-in info switch. Closing it forgets everything queued or in flight.
    pub fn set_eye(&self, open: bool) {
        let was = self.0.eye.swap(open, Ordering::Relaxed);
        if was && !open {
            self.drop_pending();
        }
        if open {
            self.0.notify.notify_one();
        }
    }

    /// The queue, the backfill lane and the jobs in flight are forgotten (late results are ignored). The cache stays.
    pub fn drop_pending(&self) {
        let mut st = self.lock();
        st.queue.clear();
        st.inflight.clear();
        reset_backfill(&mut st);
    }

    /// The administrators' "clear records": the cache, the queue and the jobs in flight are emptied and the generation
    /// moves on, so a result that arrives later is dropped.
    pub fn clear(&self) {
        let mut st = self.lock();
        st.cache.clear();
        st.queue.clear();
        st.inflight.clear();
        reset_backfill(&mut st);
        st.gen = st.gen.wrapping_add(1);
    }

    /// Shutdown: a waiting `next` returns an empty answer and later ones return at once.
    pub fn close(&self) {
        self.0.closed.store(true, Ordering::Relaxed);
        self.0.notify.notify_waiters();
        self.0.notify.notify_one();
    }

    fn active(&self) -> bool {
        self.is_api() && self.0.eye.load(Ordering::Relaxed)
    }

    // ------------------------------------------------------------ reading

    /// The place of a request's client under the source rule: Cloudflare mode = that request's `X-Geo-*` values; API mode =
    /// the cached place of its address (public addresses only), never a call, never the Cloudflare headers.
    pub fn place_for(&self, client: &ClientInfo) -> Place {
        self.place_for_at(client, unix_now())
    }

    /// [`Geo::place_for`] at a given time (unix seconds).
    pub fn place_for_at(&self, client: &ClientInfo, now: i64) -> Place {
        if self.is_api() {
            client
                .ip
                .map(|ip| self.cached_place_at(ip, now))
                .unwrap_or_default()
        } else {
            Place {
                country: client.country.clone(),
                region: client.region.clone(),
                city: client.city.clone(),
            }
        }
    }

    /// API mode: the cached place of `ip` (empty when unknown, failed, stale or not a public address); Cloudflare mode:
    /// always empty (the AI events carry no Cloudflare place).
    pub fn api_place_of(&self, ip: IpAddr) -> Place {
        if self.is_api() {
            self.cached_place_at(ip, unix_now())
        } else {
            Place::default()
        }
    }

    fn cached_place_at(&self, ip: IpAddr, now: i64) -> Place {
        let ip = crate::activity::normalize_ip(ip);
        if !is_public(ip) {
            return Place::default();
        }
        let st = self.lock();
        match st.cache.get(&ip) {
            Some(e) if e.expires > now => e.place.clone(),
            _ => Place::default(),
        }
    }

    // ------------------------------------------------------------ asking

    /// Asks for a lookup of `ip` for `target` (see 4.3 of the spec): only in API mode, with the eye open, for a public
    /// address that has no fresh cache entry. A job for the address that is already queued or in flight gets the target
    /// added. Returns whether the target is (now) waiting for a result.
    pub fn request(&self, ip: IpAddr, target: Target, now: i64) -> bool {
        if !self.active() {
            return false;
        }
        let ip = crate::activity::normalize_ip(ip);
        if !is_public(ip) {
            return false;
        }
        let mut st = self.lock();
        purge_stale(&mut st, now);
        if st.cache.get(&ip).is_some_and(|e| e.expires > now) {
            return false;
        }
        if let Some(job) = st.queue.iter_mut().find(|j| j.ip == ip) {
            add_target(&mut job.targets, target);
            return true;
        }
        if let Some(f) = st.inflight.values_mut().find(|f| f.job.ip == ip) {
            add_target(&mut f.job.targets, target);
            return true;
        }
        if st.queue.len() >= QUEUE_MAX {
            st.skipped = st.skipped.saturating_add(1);
            log_skipped(&mut st, now);
            return false;
        }
        let gen = st.gen;
        st.queue.push_back(Job {
            id: new_job_id(),
            ip,
            gen,
            queued_at: now,
            targets: vec![target],
            attempts: 0,
        });
        drop(st);
        self.0.notify.notify_one();
        true
    }

    /// Hands out up to [`BATCH_MAX`] jobs now (within the per-minute cap); empty when nothing can be handed out.
    pub fn take(&self, now: i64) -> Vec<(String, IpAddr)> {
        self.take_inner(now).0
    }

    /// The jobs, and whether the per-minute cap (not an empty queue) is why there are none.
    fn take_inner(&self, now: i64) -> (Vec<(String, IpAddr)>, bool) {
        if !self.active() {
            return (Vec::new(), false);
        }
        let mut st = self.lock();
        purge_stale(&mut st, now);
        let minute = now.div_euclid(60);
        if st.minute != minute {
            st.minute = minute;
            st.handed = 0;
        }
        let backfill_ready = !st.backfill.is_empty() && now >= st.backfill_paused_until;
        if st.queue.is_empty() && !backfill_ready {
            return (Vec::new(), false);
        }
        let room = LOOKUPS_PER_MIN.saturating_sub(st.handed) as usize;
        if room == 0 {
            return (Vec::new(), true);
        }
        let n = room.min(BATCH_MAX);
        let mut out = Vec::with_capacity(n);
        // Live jobs (sign-ins, last activity) first
        while out.len() < n {
            let Some(job) = st.queue.pop_front() else {
                break;
            };
            out.push((job.id.clone(), job.ip));
            st.inflight.insert(
                job.id.clone(),
                InFlight {
                    job,
                    handed_at: now,
                },
            );
        }
        // Then the backfill: most recently seen address first
        if backfill_ready {
            let mut deferred = Vec::new();
            while out.len() < n {
                let Some((ip, attempts)) = st.backfill.pop_front() else {
                    break;
                };
                if !st.backfill_set.contains(&ip) {
                    continue;
                }
                // A live job for the address is out already: its answer serves the backfill too; come back to it later
                if st.inflight.values().any(|f| f.job.ip == ip) {
                    deferred.push((ip, attempts));
                    continue;
                }
                let job = Job {
                    id: new_job_id(),
                    ip,
                    gen: st.gen,
                    queued_at: now,
                    targets: vec![Target::Backfill],
                    attempts: attempts.saturating_add(1),
                };
                out.push((job.id.clone(), ip));
                st.inflight.insert(
                    job.id.clone(),
                    InFlight {
                        job,
                        handed_at: now,
                    },
                );
            }
            st.backfill.extend(deferred);
        }
        st.handed = st
            .handed
            .saturating_add(u32::try_from(out.len()).unwrap_or(u32::MAX));
        (out, false)
    }

    /// The long poll behind `POST /internal/ai/geo/next`: returns at once when a job can be handed out, else waits until
    /// one can, `wait_ms` passes or the backend shuts down (then the answer is empty).
    pub async fn next(&self, wait_ms: u64) -> Vec<(String, IpAddr)> {
        let wait = Duration::from_millis(wait_ms.min(POLL_WAIT_MAX_MS));
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            // Register for wake-ups before looking, so one that arrives in between is not lost
            let notified = self.0.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.0.closed.load(Ordering::Relaxed) {
                return Vec::new();
            }
            let (jobs, capped) = self.take_inner(unix_now());
            if !jobs.is_empty() {
                return jobs;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Vec::new();
            }
            let mut sleep = deadline - now;
            if capped {
                // Wake when the next minute starts (the cap resets then)
                let into = u64::try_from(unix_now().rem_euclid(60)).unwrap_or(0);
                sleep = sleep.min(Duration::from_secs(60 - into) + Duration::from_millis(5));
            }
            let _ = tokio::time::timeout(sleep, notified).await;
        }
    }

    // ------------------------------------------------------------ answering

    /// Takes the result of job `id` in: forgets the job, caches the outcome and returns what has to be written into the
    /// database. `None` when nothing is to be written: an unknown or forgotten job, a result of an older generation (the
    /// records were cleared), a failure, an empty place, or the eye closed (then nothing at all, not even the cache).
    pub fn complete(&self, id: &str, outcome: Outcome, now: i64) -> Option<Completed> {
        let mut st = self.lock();
        let f = st.inflight.remove(id)?;
        if f.job.gen != st.gen || !self.0.eye.load(Ordering::Relaxed) {
            return None;
        }
        let ip = f.job.ip;
        let is_backfill = f.job.targets.contains(&Target::Backfill);
        match outcome {
            Outcome::Failed(code) => {
                if is_backfill {
                    let live = st.backfill_set.contains(&ip);
                    match code {
                        // The whole backfill waits; the address goes back to the front and this attempt does not count
                        FailCode::RateLimited => {
                            st.backfill_paused_until = now + BACKFILL_PAUSE_SECS;
                            if live {
                                st.backfill
                                    .push_front((ip, f.job.attempts.saturating_sub(1)));
                            }
                            return None;
                        }
                        FailCode::Timeout | FailCode::Unreachable
                            if f.job.attempts < BACKFILL_MAX_ATTEMPTS =>
                        {
                            if live {
                                st.backfill.push_back((ip, f.job.attempts));
                            }
                            return None;
                        }
                        _ => {
                            st.backfill_set.remove(&ip);
                        }
                    }
                }
                if !matches!(code, FailCode::RateLimited | FailCode::NotConfigured) {
                    cache_put(&mut st, ip, Place::default(), now + CACHE_FAIL_SECS, now);
                }
                None
            }
            Outcome::Place(place) => {
                cache_put(&mut st, ip, place.clone(), now + CACHE_TTL_SECS, now);
                // A live job that answers first also finishes the backfill of its address
                let in_backfill = st.backfill_set.remove(&ip);
                if in_backfill && !is_backfill {
                    st.backfill.retain(|(a, _)| *a != ip);
                }
                if place.is_empty() || !self.is_api() {
                    return None;
                }
                let mut signin_users = Vec::new();
                let mut active_users = Vec::new();
                let mut signin_devices = Vec::new();
                let mut active_devices = Vec::new();
                for t in &f.job.targets {
                    match t {
                        Target::Signin(u) => signin_users.push(*u),
                        Target::Active(u) => active_users.push(*u),
                        Target::DeviceSignin(d) => signin_devices.push(*d),
                        Target::DeviceActive(d) => active_devices.push(*d),
                        Target::Backfill => {}
                    }
                }
                Some(Completed {
                    ip: ip.to_string(),
                    place,
                    since: f.job.queued_at.saturating_sub(5),
                    signin_users,
                    active_users,
                    signin_devices,
                    active_devices,
                    backfill: is_backfill || in_backfill,
                })
            }
        }
    }

    // ------------------------------------------------------------ backfill

    /// Starts (or restarts) the backfill: `addresses` are the stored addresses that have no place, most recently seen
    /// first. Replaces a running backfill (its lookups in flight are forgotten). Only in API mode with the eye open;
    /// otherwise the lane just stays empty. Reserved addresses are skipped, an address counts once whatever its text form.
    pub fn start_backfill(&self, addresses: &[IpAddr]) {
        let mut st = self.lock();
        reset_backfill(&mut st);
        if !self.active() {
            return;
        }
        for a in addresses {
            let ip = crate::activity::normalize_ip(*a);
            if is_public(ip) && st.backfill_set.insert(ip) {
                st.backfill.push_back((ip, 0));
            }
        }
        let any = !st.backfill.is_empty();
        drop(st);
        if any {
            self.0.notify.notify_one();
        }
    }

    /// Addresses the running backfill has not finished yet (waiting, in flight or paused).
    pub fn backfill_left(&self) -> usize {
        self.lock().backfill_set.len()
    }

    /// Addresses waiting in the backfill lane (not handed out).
    pub fn backfill_queued(&self) -> usize {
        self.lock().backfill.len()
    }

    // ------------------------------------------------------------ for tests and diagnostics

    /// Entries in the cache.
    pub fn cache_len(&self) -> usize {
        self.lock().cache.len()
    }

    /// Jobs waiting.
    pub fn queue_len(&self) -> usize {
        self.lock().queue.len()
    }

    /// Jobs handed out without a result yet.
    pub fn inflight_len(&self) -> usize {
        self.lock().inflight.len()
    }

    /// The generation counter (moves on at every [`Geo::clear`]).
    pub fn generation(&self) -> u64 {
        self.lock().gen
    }

    /// Puts an entry into the cache (a place for [`CACHE_TTL_SECS`]); for tests and for warming.
    pub fn cache_place(&self, ip: IpAddr, place: Place, now: i64) {
        let mut st = self.lock();
        cache_put(&mut st, ip, place, now + CACHE_TTL_SECS, now);
    }
}

fn add_target(targets: &mut Vec<Target>, t: Target) {
    if !targets.contains(&t) {
        targets.push(t);
    }
}

/// 16 random lower-case hex digits.
fn new_job_id() -> String {
    token_hash(&crate::auth::new_session_token())[..16].to_string()
}

fn reset_backfill(st: &mut State) {
    st.backfill.clear();
    st.backfill_set.clear();
    st.backfill_paused_until = 0;
}

fn purge_stale(st: &mut State, now: i64) {
    let before = st.queue.len();
    st.queue
        .retain(|j| now.saturating_sub(j.queued_at) <= QUEUE_STALE_SECS);
    let dropped = before - st.queue.len();
    if dropped > 0 {
        st.skipped = st
            .skipped
            .saturating_add(u32::try_from(dropped).unwrap_or(u32::MAX));
        log_skipped(st, now);
    }
    let stale: Vec<String> = st
        .inflight
        .iter()
        .filter(|(_, f)| now.saturating_sub(f.handed_at) > INFLIGHT_STALE_SECS)
        .map(|(k, _)| k.clone())
        .collect();
    for k in stale {
        let Some(f) = st.inflight.remove(&k) else {
            continue;
        };
        // A backfill lookup that got no answer is tried again (at most BACKFILL_MAX_ATTEMPTS times in all)
        if f.job.targets.contains(&Target::Backfill)
            && f.job.gen == st.gen
            && st.backfill_set.contains(&f.job.ip)
        {
            if f.job.attempts < BACKFILL_MAX_ATTEMPTS {
                st.backfill.push_back((f.job.ip, f.job.attempts));
            } else {
                st.backfill_set.remove(&f.job.ip);
            }
        }
    }
}

/// At most one English line a minute about dropped lookups (a count only, never an address).
fn log_skipped(st: &mut State, now: i64) {
    let minute = now.div_euclid(60);
    if st.skipped > 0 && st.logged_minute != minute {
        st.logged_minute = minute;
        tracing::warn!(
            "geo: {} location lookups skipped (queue full or too old)",
            st.skipped
        );
        st.skipped = 0;
    }
}

fn cache_put(st: &mut State, ip: IpAddr, place: Place, expires: i64, now: i64) {
    if !st.cache.contains_key(&ip) && st.cache.len() >= CACHE_MAX {
        st.cache.retain(|_, e| e.expires > now);
        if st.cache.len() >= CACHE_MAX {
            let mut order: Vec<(i64, IpAddr)> =
                st.cache.iter().map(|(k, e)| (e.expires, *k)).collect();
            order.sort_unstable();
            let excess = st.cache.len() + 1 - CACHE_MAX;
            for (_, k) in order.into_iter().take(excess) {
                st.cache.remove(&k);
            }
        }
    }
    st.cache.insert(ip, CacheEntry { place, expires });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("ip")
    }

    fn open() -> Geo {
        let g = Geo::new();
        g.set_api(true);
        g.set_eye(true);
        g
    }

    fn here() -> Place {
        Place {
            country: Some("ZZ".into()),
            region: Some("Example Region".into()),
            city: Some("Example City".into()),
        }
    }

    #[test]
    fn reserved_ranges_are_never_public() {
        // (first, last) of every reserved range
        let ranges: &[(&str, &str)] = &[
            ("0.0.0.0", "0.255.255.255"),
            ("10.0.0.0", "10.255.255.255"),
            ("100.64.0.0", "100.127.255.255"),
            ("127.0.0.0", "127.255.255.255"),
            ("169.254.0.0", "169.254.255.255"),
            ("172.16.0.0", "172.31.255.255"),
            ("192.0.0.0", "192.0.0.255"),
            ("192.0.2.0", "192.0.2.255"),
            ("192.31.196.0", "192.31.196.255"),
            ("192.52.193.0", "192.52.193.255"),
            ("192.88.99.0", "192.88.99.255"),
            ("192.168.0.0", "192.168.255.255"),
            ("192.175.48.0", "192.175.48.255"),
            ("198.18.0.0", "198.19.255.255"),
            ("198.51.100.0", "198.51.100.255"),
            ("203.0.113.0", "203.0.113.255"),
            ("224.0.0.0", "239.255.255.255"),
            ("240.0.0.0", "255.255.255.255"),
            ("::", "::"),
            ("::1", "::1"),
            ("::2", "::ffff:0:0"),
            ("64:ff9b::", "64:ff9b::ffff:ffff"),
            ("64:ff9b:1::", "64:ff9b:1:ffff:ffff:ffff:ffff:ffff"),
            ("100::", "100::ffff:ffff:ffff:ffff"),
            ("fc00::", "fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
            ("fe80::", "febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
            ("fec0::", "feff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
            ("ff00::", "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
            ("5f00::", "5f00:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
            ("2001::", "2001:1ff:ffff:ffff:ffff:ffff:ffff:ffff"),
            ("2001:db8::", "2001:db8:ffff:ffff:ffff:ffff:ffff:ffff"),
            ("2002::", "2002:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
            ("3fff::", "3fff:fff:ffff:ffff:ffff:ffff:ffff:ffff"),
        ];
        for (a, b) in ranges {
            assert!(!is_public(ip(a)), "{a} must not be public");
            assert!(!is_public(ip(b)), "{b} must not be public");
        }
    }

    #[test]
    fn public_addresses_and_mapped_forms() {
        for a in [
            "8.8.8.8",
            "1.1.1.1",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
            "100.63.255.255",
            "100.128.0.0",
            "172.15.255.255",
            "172.32.0.0",
            "198.17.255.255",
            "198.20.0.0",
            "2001:200::",
            "3fff:1000::",
            "2003::",
        ] {
            assert!(is_public(ip(a)), "{a} must be public");
        }
        for a in ["::ffff:10.0.0.1", "::ffff:127.0.0.1", "::ffff:100.64.0.1"] {
            assert!(!is_public(ip(a)), "{a} must not be public");
        }
    }

    #[test]
    fn places_are_cleaned_and_the_country_code_wins() {
        let p = clean_place(
            Some("Example Country"),
            Some("zz"),
            Some(" Example\u{7} Region "),
            Some(""),
        );
        assert_eq!(p.country.as_deref(), Some("ZZ"));
        assert_eq!(p.region.as_deref(), Some("Example Region"));
        assert_eq!(p.city, None);
        // Not a two-letter code, or one of the placeholders: the name is used
        for bad in ["ZZZ", "X1", "XX", "T1", ""] {
            let p = clean_place(Some("Example Country"), Some(bad), None, None);
            assert_eq!(p.country.as_deref(), Some("Example Country"), "{bad}");
        }
        let long = "x".repeat(200);
        assert_eq!(clean_text(&long).map(|s| s.chars().count()), Some(64));
        assert_eq!(clean_text("  \u{1}  "), None);
    }

    #[test]
    fn fail_codes_round_trip() {
        for c in [
            "timeout",
            "unreachable",
            "redirect",
            "httpStatus",
            "notJson",
            "tooLarge",
            "providerFail",
            "noFields",
            "keyProblem",
            "rateLimited",
            "notConfigured",
            "refused",
        ] {
            assert_eq!(FailCode::parse(c).map(FailCode::as_str), Some(c));
        }
        assert_eq!(FailCode::parse("Timeout"), None);
        assert_eq!(FailCode::parse("other"), None);
    }

    #[test]
    fn request_needs_api_mode_the_eye_and_a_public_address() {
        let g = Geo::new();
        assert!(!g.request(ip("8.8.8.8"), Target::Signin(1), 1000));
        g.set_api(true);
        assert!(
            !g.request(ip("8.8.8.8"), Target::Signin(1), 1000),
            "eye closed"
        );
        g.set_eye(true);
        assert!(
            !g.request(ip("192.0.2.1"), Target::Signin(1), 1000),
            "reserved"
        );
        assert!(
            !g.request(ip("10.0.0.1"), Target::Signin(1), 1000),
            "private"
        );
        assert!(g.request(ip("8.8.8.8"), Target::Signin(1), 1000));
        assert_eq!(g.queue_len(), 1);
        g.set_api(false);
        assert_eq!(g.queue_len(), 0, "Cloudflare mode forgets the queue");
        assert!(!g.request(ip("8.8.4.4"), Target::Signin(1), 1000));
    }

    #[test]
    fn same_address_shares_one_job_and_collects_targets() {
        let g = open();
        assert!(g.request(ip("8.8.8.8"), Target::Signin(1), 1000));
        assert!(g.request(ip("8.8.8.8"), Target::Active(1), 1000));
        assert!(g.request(ip("8.8.8.8"), Target::Active(1), 1000));
        assert!(g.request(ip("::ffff:8.8.8.8"), Target::Active(2), 1000));
        assert_eq!(g.queue_len(), 1);
        let jobs = g.take(1001);
        assert_eq!(jobs.len(), 1);
        // In flight: another request still joins it
        assert!(g.request(ip("8.8.8.8"), Target::Active(3), 1002));
        assert_eq!(g.queue_len(), 0);
        let done = g
            .complete(&jobs[0].0, Outcome::Place(here()), 1003)
            .expect("applies");
        assert_eq!(done.signin_users, vec![1]);
        assert_eq!(done.active_users, vec![1, 2, 3]);
        assert_eq!(done.ip, "8.8.8.8");
        assert_eq!(done.since, 995);
        assert_eq!(g.inflight_len(), 0);
    }

    #[test]
    fn a_fresh_cache_entry_stops_a_new_request() {
        let g = open();
        g.cache_place(ip("8.8.8.8"), here(), 1000);
        assert!(!g.request(ip("8.8.8.8"), Target::Signin(1), 1000 + CACHE_TTL_SECS - 1));
        assert!(g.request(ip("8.8.8.8"), Target::Signin(1), 1000 + CACHE_TTL_SECS));
    }

    #[test]
    fn cache_ttls_and_failure_rules() {
        let g = open();
        let mut ids = Vec::new();
        for (i, a) in ["8.8.8.1", "8.8.8.2", "8.8.8.3", "8.8.8.4"]
            .iter()
            .enumerate()
        {
            assert!(g.request(ip(a), Target::Signin(i as i64 + 1), 1000));
        }
        for (id, _) in g.take(1000) {
            ids.push(id);
        }
        let jobs: HashMap<_, _> = ids.iter().cloned().enumerate().collect();
        assert_eq!(jobs.len(), 4);
        // 0: a place, 1: a failure (cached 15 min), 2: rateLimited (not cached), 3: a success without fields (cached as empty)
        assert!(g.complete(&ids[0], Outcome::Place(here()), 1000).is_some());
        assert!(g
            .complete(&ids[1], Outcome::Failed(FailCode::Timeout), 1000)
            .is_none());
        assert!(g
            .complete(&ids[2], Outcome::Failed(FailCode::RateLimited), 1000)
            .is_none());
        assert!(g
            .complete(&ids[3], Outcome::Place(Place::default()), 1000)
            .is_none());
        assert_eq!(g.cache_len(), 3);
        assert_eq!(g.place_for_at(&client("8.8.8.1"), 1000), here());
        assert_eq!(
            g.place_for_at(&client("8.8.8.1"), 1000 + CACHE_TTL_SECS),
            Place::default()
        );
        // The failure is remembered for 15 minutes
        assert!(!g.request(ip("8.8.8.2"), Target::Signin(2), 1000 + CACHE_FAIL_SECS - 1));
        assert!(g.request(ip("8.8.8.2"), Target::Signin(2), 1000 + CACHE_FAIL_SECS));
        // rateLimited is not cached: asking again queues at once
        assert!(g.request(ip("8.8.8.3"), Target::Signin(3), 1001));
    }

    fn client(a: &str) -> ClientInfo {
        ClientInfo {
            ip: Some(ip(a)),
            ua: None,
            country: Some("QQ".into()),
            region: Some("Cloud Region".into()),
            city: Some("Cloud City".into()),
        }
    }

    #[test]
    fn place_follows_the_source_rule() {
        let g = Geo::new();
        // Cloudflare mode: the request's own headers, whatever the cache holds
        g.cache_place(ip("8.8.8.8"), here(), 1000);
        let p = g.place_for_at(&client("8.8.8.8"), 1000);
        assert_eq!(p.city.as_deref(), Some("Cloud City"));
        // API mode: the cache only; the headers are ignored; reserved and unknown addresses give nothing
        g.set_api(true);
        assert_eq!(g.place_for_at(&client("8.8.8.8"), 1000), here());
        assert_eq!(g.place_for_at(&client("8.8.4.4"), 1000), Place::default());
        g.cache_place(ip("192.0.2.9"), here(), 1000);
        assert_eq!(g.place_for_at(&client("192.0.2.9"), 1000), Place::default());
        let mut no_ip = client("8.8.8.8");
        no_ip.ip = None;
        assert_eq!(g.place_for_at(&no_ip, 1000), Place::default());
    }

    #[test]
    fn cache_is_bounded_and_evicts_the_earliest_expiry() {
        let g = open();
        let base = 1_000_000;
        for i in 0..CACHE_MAX {
            let a = IpAddr::V4(Ipv4Addr::from(0x0800_0000u32 + i as u32));
            g.cache_place(a, here(), base + i as i64);
        }
        assert_eq!(g.cache_len(), CACHE_MAX);
        g.cache_place(ip("9.9.9.9"), here(), base + 5000);
        assert_eq!(g.cache_len(), CACHE_MAX);
        // The entry with the earliest expiry (the first one) is the one that went
        let first = IpAddr::V4(Ipv4Addr::from(0x0800_0000u32));
        assert_eq!(g.cached_place_at(first, base), Place::default());
        assert_eq!(g.cached_place_at(ip("9.9.9.9"), base), here());
        // Expired entries go first
        g.cache_place(ip("9.9.9.8"), here(), base + CACHE_TTL_SECS + 10_000);
        assert!(g.cache_len() <= CACHE_MAX);
    }

    #[test]
    fn queue_is_bounded_and_stale_jobs_are_dropped() {
        let g = open();
        for i in 0..QUEUE_MAX {
            let a = IpAddr::V4(Ipv4Addr::from(0x0800_0000u32 + i as u32));
            assert!(g.request(a, Target::Signin(1), 1000));
        }
        assert!(
            !g.request(ip("9.9.9.9"), Target::Signin(1), 1000),
            "queue full"
        );
        assert_eq!(g.queue_len(), QUEUE_MAX);
        // Too old: dropped by the next request
        assert!(g.request(
            ip("9.9.9.9"),
            Target::Signin(1),
            1000 + QUEUE_STALE_SECS + 1
        ));
        assert_eq!(g.queue_len(), 1);
    }

    #[test]
    fn hand_outs_respect_the_batch_and_the_per_minute_cap() {
        let g = open();
        for i in 0..40 {
            let a = IpAddr::V4(Ipv4Addr::from(0x0800_0000u32 + i));
            assert!(g.request(a, Target::Signin(1), 600));
        }
        // minute 10 = 600..659: 3 batches of 10 = 30, then the cap
        let mut got = 0;
        for _ in 0..5 {
            got += g.take(610).len();
        }
        assert_eq!(got, LOOKUPS_PER_MIN as usize);
        assert_eq!(g.queue_len(), 10);
        assert!(g.take(659).is_empty());
        // A new minute resets the count
        assert_eq!(g.take(660).len(), 10);
        assert_eq!(g.queue_len(), 0);
    }

    #[test]
    fn stale_inflight_jobs_are_forgotten() {
        let g = open();
        assert!(g.request(ip("8.8.8.8"), Target::Signin(1), 1000));
        let id = g.take(1000).remove(0).0;
        assert_eq!(g.inflight_len(), 1);
        // Another request after the stale time purges it and queues a new job
        assert!(g.request(
            ip("8.8.8.8"),
            Target::Signin(1),
            1000 + INFLIGHT_STALE_SECS + 1
        ));
        assert_eq!(g.inflight_len(), 0);
        assert_eq!(g.queue_len(), 1);
        assert!(g.complete(&id, Outcome::Place(here()), 1100).is_none());
    }

    #[test]
    fn clear_bumps_the_generation_and_drops_late_results() {
        let g = open();
        g.cache_place(ip("8.8.4.4"), here(), 1000);
        assert!(g.request(ip("8.8.8.8"), Target::Signin(1), 1000));
        let id = g.take(1000).remove(0).0;
        assert!(g.request(ip("1.1.1.1"), Target::Signin(1), 1000));
        let gen = g.generation();
        g.clear();
        assert_eq!(g.generation(), gen + 1);
        assert_eq!((g.cache_len(), g.queue_len(), g.inflight_len()), (0, 0, 0));
        assert!(g.complete(&id, Outcome::Place(here()), 1001).is_none());
        assert_eq!(g.cache_len(), 0, "a late result is not cached either");
    }

    #[test]
    fn closing_the_eye_forgets_everything_and_blocks_results() {
        let g = open();
        assert!(g.request(ip("8.8.8.8"), Target::Signin(1), 1000));
        assert!(g.request(ip("1.1.1.1"), Target::Signin(1), 1000));
        let id = g.take(1000).remove(0).0;
        g.set_eye(false);
        assert_eq!((g.queue_len(), g.inflight_len()), (0, 0));
        assert!(g.complete(&id, Outcome::Place(here()), 1001).is_none());
        assert_eq!(g.cache_len(), 0);
        assert!(g.take(1002).is_empty());
    }

    // ------------------------------------------------------------ backfill (v1.6)

    fn addr(i: u32) -> IpAddr {
        IpAddr::V4(Ipv4Addr::from(0x0800_0000u32 + i))
    }

    fn addrs(n: u32) -> Vec<IpAddr> {
        (0..n).map(addr).collect()
    }

    #[test]
    fn backfill_hands_out_newest_first_one_job_per_distinct_address() {
        let g = open();
        // The same address twice (one in mapped form) and a reserved one: counted once / skipped
        g.start_backfill(&[
            addr(1),
            addr(2),
            addr(1),
            ip("::ffff:8.0.0.2"),
            ip("10.0.0.1"),
            addr(3),
        ]);
        assert_eq!(g.backfill_left(), 3);
        let jobs = g.take(1000);
        let got: Vec<IpAddr> = jobs.iter().map(|j| j.1).collect();
        assert_eq!(got, vec![addr(1), addr(2), addr(3)], "most recent first");
        assert_eq!(g.backfill_queued(), 0);
        assert_eq!(g.backfill_left(), 3, "in flight is still unfinished");
        for (id, a) in &jobs {
            let done = g
                .complete(id, Outcome::Place(here()), 1001)
                .expect("applies");
            assert!(done.backfill);
            assert_eq!(done.ip, a.to_string());
            assert!(done.signin_users.is_empty() && done.active_users.is_empty());
        }
        assert_eq!(g.backfill_left(), 0);
        assert!(g.take(1002).is_empty());
    }

    #[test]
    fn live_jobs_go_first_and_the_cap_holds_for_both() {
        let g = open();
        g.start_backfill(&addrs(50));
        assert!(g.request(ip("9.9.9.9"), Target::Signin(1), 600));
        let first = g.take(610);
        assert_eq!(first.len(), BATCH_MAX);
        assert_eq!(first[0].1, ip("9.9.9.9"), "the live job is first");
        assert_eq!(first[1].1, addr(0));
        // A live job asked for later still goes before the rest of the backfill
        assert!(g.request(ip("9.9.9.8"), Target::Signin(2), 611));
        let second = g.take(611);
        assert_eq!(second[0].1, ip("9.9.9.8"));
        let third = g.take(612);
        assert_eq!(
            first.len() + second.len() + third.len(),
            LOOKUPS_PER_MIN as usize
        );
        assert!(g.take(659).is_empty(), "the cap of 30 a minute holds");
        assert_eq!(g.take(660).len(), BATCH_MAX, "a new minute");
    }

    #[test]
    fn a_full_live_queue_does_not_drop_backfill_addresses() {
        let g = open();
        g.start_backfill(&addrs(300));
        assert_eq!(g.backfill_left(), 300, "not bounded by the queue limit");
        for i in 0..QUEUE_MAX {
            assert!(g.request(addr(10_000 + i as u32), Target::Signin(1), 1000));
        }
        assert!(
            !g.request(ip("9.9.9.9"), Target::Signin(1), 1000),
            "live queue full"
        );
        assert_eq!(g.backfill_left(), 300);
        // Far past the stale time of live jobs: they go, the backfill stays
        assert!(g.take(1000 + QUEUE_STALE_SECS + 1).len() <= BATCH_MAX);
        assert_eq!(g.queue_len(), 0);
        assert_eq!(g.backfill_left(), 300);
    }

    #[test]
    fn rate_limited_pauses_the_backfill_and_the_address_goes_back_to_the_front() {
        let g = open();
        g.start_backfill(&addrs(3));
        let jobs = g.take(1000);
        assert_eq!(jobs.len(), 3);
        assert!(g
            .complete(&jobs[0].0, Outcome::Failed(FailCode::RateLimited), 1000)
            .is_none());
        for j in &jobs[1..] {
            assert!(g.complete(&j.0, Outcome::Place(here()), 1000).is_some());
        }
        assert_eq!(g.backfill_queued(), 1);
        // Paused: nothing for 15 minutes, live jobs still flow
        assert!(g.take(1000 + BACKFILL_PAUSE_SECS - 1).is_empty());
        assert!(g.request(ip("9.9.9.9"), Target::Signin(1), 1100));
        assert_eq!(g.take(1100).len(), 1);
        assert_eq!(
            g.cache_len(),
            2,
            "rateLimited is not cached (only the two answers are)"
        );
        let again = g.take(1000 + BACKFILL_PAUSE_SECS + 1);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].1, jobs[0].1, "back at the front");
        // The rate-limited attempt did not count: two timeouts are retried, the third attempt's failure drops it
        let mut id = again[0].0.clone();
        for round in 0..2i64 {
            let t = 2000 + round * 100;
            assert!(g
                .complete(&id, Outcome::Failed(FailCode::Timeout), t)
                .is_none());
            let next = g.take(t + 1);
            assert_eq!(next.len(), 1, "round {round}");
            id = next[0].0.clone();
        }
        assert!(g
            .complete(&id, Outcome::Failed(FailCode::Timeout), 3000)
            .is_none());
        assert!(g.take(3001).is_empty());
    }

    #[test]
    fn timeouts_and_unreachable_retry_three_times_other_failures_drop() {
        let g = open();
        g.start_backfill(&[addr(1), addr(2)]);
        let jobs = g.take(1000);
        // addr(2): a hard failure drops it at once
        assert!(g
            .complete(&jobs[1].0, Outcome::Failed(FailCode::KeyProblem), 1001)
            .is_none());
        assert_eq!(g.backfill_left(), 1);
        // addr(1): unreachable, timeout, then the third attempt fails and it is dropped
        let mut id = jobs[0].0.clone();
        for (n, code) in [FailCode::Unreachable, FailCode::Timeout]
            .into_iter()
            .enumerate()
        {
            assert!(g.complete(&id, Outcome::Failed(code), 1002).is_none());
            assert_eq!(g.backfill_left(), 1, "attempt {n} is retried");
            let again = g.take(1003);
            assert_eq!(again.len(), 1);
            id = again[0].0.clone();
        }
        assert!(g
            .complete(&id, Outcome::Failed(FailCode::Timeout), 1004)
            .is_none());
        assert_eq!(g.backfill_left(), 0);
        assert!(g.take(1005).is_empty());
    }

    #[test]
    fn stale_inflight_backfill_jobs_are_retried_at_most_three_times() {
        let g = open();
        g.start_backfill(&[addr(1)]);
        let mut now = 1000;
        // Three hand-outs that nobody answers, a fourth never comes
        for attempt in 1..=3 {
            assert_eq!(g.take(now).len(), 1, "attempt {attempt}");
            assert_eq!(g.backfill_left(), 1);
            now += INFLIGHT_STALE_SECS + 1;
        }
        assert!(g.take(now).is_empty());
        assert_eq!(g.inflight_len(), 0);
        assert_eq!(g.backfill_left(), 0);
    }

    #[test]
    fn eye_close_cloudflare_and_clear_stop_the_backfill() {
        for stop in 0..3 {
            let g = open();
            g.start_backfill(&addrs(5));
            let jobs = g.take(1000);
            assert_eq!(jobs.len(), 5);
            g.start_backfill(&addrs(15));
            assert_eq!(g.backfill_left(), 15);
            match stop {
                0 => g.set_eye(false),
                1 => g.set_api(false),
                _ => g.clear(),
            }
            assert_eq!(g.backfill_left(), 0, "stop {stop}");
            assert_eq!(g.backfill_queued(), 0, "stop {stop}");
            assert!(g.take(1001).is_empty());
            assert!(g
                .complete(&jobs[0].0, Outcome::Place(here()), 1002)
                .is_none());
        }
        // Not active (eye closed): nothing is started
        let g = Geo::new();
        g.set_api(true);
        g.start_backfill(&addrs(3));
        assert_eq!(g.backfill_left(), 0);
    }

    #[test]
    fn restarting_replaces_the_lane() {
        let g = open();
        g.start_backfill(&addrs(10));
        assert_eq!(g.take(1000).len(), 10);
        g.start_backfill(&[addr(20), addr(21)]);
        assert_eq!(g.backfill_left(), 2);
        let jobs = g.take(1001);
        assert_eq!(
            jobs.iter().map(|j| j.1).collect::<Vec<_>>(),
            vec![addr(20), addr(21)]
        );
    }

    #[test]
    fn a_live_answer_for_a_backfill_address_finishes_it() {
        let g = open();
        g.start_backfill(&[addr(1), addr(2)]);
        assert!(g.request(addr(1), Target::Signin(7), 1000));
        let jobs = g.take(1000);
        // The live job for addr(1) goes first; the backfill does not look it up a second time
        assert_eq!(
            jobs.iter().map(|j| j.1).collect::<Vec<_>>(),
            vec![addr(1), addr(2)]
        );
        let done = g
            .complete(&jobs[0].0, Outcome::Place(here()), 1001)
            .expect("applies");
        assert!(done.backfill, "the handler fills every placeless row of it");
        assert_eq!(done.signin_users, vec![7]);
        assert_eq!(g.backfill_left(), 1);
        assert_eq!(g.backfill_queued(), 0);
        // A live job already out when the backfill reaches the address: the backfill waits for it
        let g = open();
        assert!(g.request(addr(1), Target::Signin(7), 1000));
        let live = g.take(1000);
        g.start_backfill(&[addr(1)]);
        assert!(
            g.take(1001).is_empty(),
            "no second lookup of an address in flight"
        );
        let done = g
            .complete(&live[0].0, Outcome::Place(here()), 1002)
            .expect("applies");
        assert!(done.backfill);
        assert_eq!(g.backfill_left(), 0);
        assert!(g.take(1003).is_empty());
    }

    #[test]
    fn a_backfill_answer_without_fields_just_finishes_the_address() {
        let g = open();
        g.start_backfill(&[addr(1)]);
        let jobs = g.take(1000);
        assert!(g
            .complete(&jobs[0].0, Outcome::Place(Place::default()), 1001)
            .is_none());
        assert_eq!(g.backfill_left(), 0);
    }

    #[tokio::test]
    async fn poll_waits_wakes_and_times_out() {
        let g = open();
        // Nothing waiting: empty after the wait
        let t = std::time::Instant::now();
        assert!(g.next(80).await.is_empty());
        assert!(t.elapsed() >= Duration::from_millis(70));
        // A pending job: at once
        assert!(g.request(ip("8.8.8.8"), Target::Signin(1), unix_now()));
        let t = std::time::Instant::now();
        assert_eq!(g.next(5000).await.len(), 1);
        assert!(t.elapsed() < Duration::from_millis(1000));
        // A job that arrives while waiting wakes the poll
        let g2 = g.clone();
        let waiter = tokio::spawn(async move { g2.next(5000).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(g.request(ip("1.1.1.1"), Target::Signin(1), unix_now()));
        let got = tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("woken")
            .expect("join");
        assert_eq!(got.len(), 1);
        // Shutdown ends a waiting poll
        let g3 = g.clone();
        let waiter = tokio::spawn(async move { g3.next(20_000).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        g.close();
        let got = tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("closed")
            .expect("join");
        assert!(got.is_empty());
        assert!(g.next(20_000).await.is_empty(), "closed: returns at once");
    }
}
