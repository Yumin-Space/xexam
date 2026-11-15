//! External IP-location lookup (v1.4): the one place in the system that may send a visitor address to a provider.
//!
//! The main backend has no internet access by design, so the outbound call runs here. The backend keeps the queue, the
//! 24 h cache, the reserved-address filter and the per-minute cap of its hand-outs; this service pulls work from it over
//! the token-authenticated link (`worker`), looks the addresses up with the settings of `geo_settings` (an
//! administrator's "test and save") and posts the places back. Lookups never go through the AI quota, the AI usage
//! statistics or the AI switch: they need only this service to run.
//!
//! * `filter`   - the reserved addresses that are never sent (4.1)
//! * `template` - validation of a draft and building of the request (5.1, 5.2)
//! * `mapping`  - reading the provider's JSON answer (5.3)
//! * `client`   - the HTTP call (4.6)
//! * `worker`   - the background task that serves the backend's queue (4.9)
use crate::app::{App, Knobs};
use crate::config::Timeouts;
use crate::secret::{SecretError, GEO_KEY_AAD};
use crate::store::geo::{GeoRow, SealedKey};
use serde_json::{json, Value};
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use zeroize::Zeroizing;

pub mod client;
pub mod filter;
pub mod mapping;
pub mod template;
pub mod worker;

/// The fixed public example address the test button looks up.
pub const GEO_TEST_IP: &str = "8.8.8.8";
/// Lookups per wall-clock minute (the test button and the worker share them).
pub const GEO_PER_MIN: u64 = 30;
/// Largest provider answer read, in bytes.
pub const GEO_MAX_BODY: usize = 64 * 1024;
/// Failure lines written per minute; the rest are summed into one line.
const LOG_LINES_PER_MIN: u32 = 10;

/// Why a lookup failed (the shared vocabulary of 4.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureCode {
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

impl FailureCode {
    pub const ALL: [FailureCode; 12] = [
        FailureCode::Timeout,
        FailureCode::Unreachable,
        FailureCode::Redirect,
        FailureCode::HttpStatus,
        FailureCode::NotJson,
        FailureCode::TooLarge,
        FailureCode::ProviderFail,
        FailureCode::NoFields,
        FailureCode::KeyProblem,
        FailureCode::RateLimited,
        FailureCode::NotConfigured,
        FailureCode::Refused,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            FailureCode::Timeout => "timeout",
            FailureCode::Unreachable => "unreachable",
            FailureCode::Redirect => "redirect",
            FailureCode::HttpStatus => "httpStatus",
            FailureCode::NotJson => "notJson",
            FailureCode::TooLarge => "tooLarge",
            FailureCode::ProviderFail => "providerFail",
            FailureCode::NoFields => "noFields",
            FailureCode::KeyProblem => "keyProblem",
            FailureCode::RateLimited => "rateLimited",
            FailureCode::NotConfigured => "notConfigured",
            FailureCode::Refused => "refused",
        }
    }

    pub fn parse(s: &str) -> Option<FailureCode> {
        FailureCode::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

/// A failed lookup: the code, the HTTP status (`httpStatus` only) and the provider's text (`providerFail` only, cleaned
/// and without the key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub code: FailureCode,
    pub status: Option<u16>,
    pub message: Option<String>,
}

impl Failure {
    pub fn code(code: FailureCode) -> Failure {
        Failure {
            code,
            status: None,
            message: None,
        }
    }

    /// `{"code", "status", "message"}` of the test answers.
    pub fn to_json(&self) -> Value {
        json!({
            "code": self.code.as_str(),
            "status": self.status,
            "message": self.message,
        })
    }
}

/// A place as a provider gave it (cleaned). Any part may be missing; not all four.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GeoPlace {
    pub country: Option<String>,
    pub country_code: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
}

impl GeoPlace {
    pub fn is_empty(&self) -> bool {
        self.country.is_none()
            && self.country_code.is_none()
            && self.region.is_none()
            && self.city.is_none()
    }

    /// `{"country", "countryCode", "region", "city"}`, each a string or null.
    pub fn to_json(&self) -> Value {
        json!({
            "country": self.country,
            "countryCode": self.country_code,
            "region": self.region,
            "city": self.city,
        })
    }
}

/// Counts the lookups of the current wall-clock minute.
#[derive(Debug, Default)]
struct MinuteCap {
    minute: i64,
    used: u64,
}

impl MinuteCap {
    /// One more lookup at `now` (unix s) against `limit` per minute; `false` when the minute is used up.
    fn take(&mut self, now: i64, limit: u64) -> bool {
        let minute = now.div_euclid(60);
        if minute != self.minute {
            self.minute = minute;
            self.used = 0;
        }
        if self.used >= limit {
            return false;
        }
        self.used += 1;
        true
    }
}

/// What to do with one failure line.
#[derive(Debug, Default, PartialEq, Eq)]
struct LogPlan {
    /// The previous minute held back this many lines: write one summary line first
    summary: Option<u64>,
    /// Write the line itself
    line: bool,
}

/// At most [`LOG_LINES_PER_MIN`] failure lines per minute; the rest are counted and summed up when the minute ends.
#[derive(Debug, Default)]
struct FailLog {
    minute: i64,
    logged: u32,
    held: u64,
}

impl FailLog {
    fn roll(&mut self, now: i64) -> Option<u64> {
        let minute = now.div_euclid(60);
        if minute == self.minute {
            return None;
        }
        self.minute = minute;
        self.logged = 0;
        let held = std::mem::take(&mut self.held);
        (held > 0).then_some(held)
    }

    /// A failure at `now`.
    fn plan(&mut self, now: i64) -> LogPlan {
        let summary = self.roll(now);
        if self.logged < LOG_LINES_PER_MIN {
            self.logged += 1;
            LogPlan {
                summary,
                line: true,
            }
        } else {
            self.held += 1;
            LogPlan {
                summary,
                line: false,
            }
        }
    }
}

/// The state of the lookup feature inside [`App`]: the HTTP client, the saved settings in memory, the per-minute count,
/// the "one test at a time" flag and the failure-line limiter.
pub struct GeoState {
    client: reqwest::Client,
    /// One lookup, in total (`Timeouts::geo_total`)
    pub total: Duration,
    config: RwLock<Option<Arc<GeoRow>>>,
    cap: Mutex<MinuteCap>,
    testing: AtomicBool,
    fails: Mutex<FailLog>,
}

/// Held while a test or a save runs; dropping it frees the place.
pub struct TestGuard<'a>(&'a AtomicBool);

impl Drop for TestGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl GeoState {
    pub fn new(version: &str, t: &Timeouts) -> Result<GeoState, String> {
        Ok(GeoState {
            client: client::build_client(version, t.geo_total)?,
            total: t.geo_total,
            config: RwLock::new(None),
            cap: Mutex::new(MinuteCap::default()),
            testing: AtomicBool::new(false),
            fails: Mutex::new(FailLog::default()),
        })
    }

    /// The saved settings as they are in memory.
    pub fn config(&self) -> Option<Arc<GeoRow>> {
        self.config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Replaces the settings in memory (after the database was written).
    pub fn set_config(&self, row: Option<GeoRow>) {
        *self.config.write().unwrap_or_else(|e| e.into_inner()) = row.map(Arc::new);
    }

    /// Takes one of the minute's lookups (`false` = used up).
    pub fn take_slot(&self, now: i64, limit: u64) -> bool {
        self.cap
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take(now, limit)
    }

    /// Starts a test or save; `None` while another one runs.
    pub fn begin_test(&self) -> Option<TestGuard<'_>> {
        // `then`, not `then_some`: a guard built and dropped on the busy path would free the place of the running test
        (!self.testing.swap(true, Ordering::SeqCst)).then(|| TestGuard(&self.testing))
    }

    /// One failure line `geo lookup via <host> failed: <code>`, limited to 10 per minute. Never the address, the URL, the
    /// key or the provider's text.
    pub fn log_failure(&self, host: &str, code: FailureCode, now: i64) {
        let plan = self
            .fails
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .plan(now);
        Self::write_plan(plan, host, code);
    }

    /// Writes the summary of a finished minute when no new failure came to do it.
    pub fn flush_log(&self, now: i64) {
        let summary = self
            .fails
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .roll(now);
        if let Some(n) = summary {
            tracing::warn!("geo lookup: {n} more failures in the last minute were not logged");
        }
    }

    fn write_plan(plan: LogPlan, host: &str, code: FailureCode) {
        if let Some(n) = plan.summary {
            tracing::warn!("geo lookup: {n} more failures in the last minute were not logged");
        }
        if plan.line {
            tracing::warn!("geo lookup via {host} failed: {}", code.as_str());
        }
    }
}

/// Opens a sealed key; `Err` when there is no master key or the key does not open.
pub fn open_key(app: &App, key: &SealedKey) -> Result<Zeroizing<String>, SecretError> {
    let master = app.master().ok_or(SecretError)?;
    master.open_aad(GEO_KEY_AAD, &key.key_id, &key.nonce, &key.ciphertext)
}

/// The key of the saved settings: `Ok(None)` without one, `Err` when it cannot be opened.
pub fn saved_key(app: &App, row: &GeoRow) -> Result<Option<Zeroizing<String>>, SecretError> {
    row.key.as_ref().map(|k| open_key(app, k)).transpose()
}

/// The lookup of the worker (4.6): reserved address, config, key, cap, request, answer, in this order.
pub async fn lookup_saved(app: &App, ip: IpAddr) -> Result<GeoPlace, Failure> {
    if !filter::is_public(ip) {
        return Err(Failure::code(FailureCode::Refused));
    }
    let Some(row) = app.geo.config() else {
        return Err(Failure::code(FailureCode::NotConfigured));
    };
    let key = saved_key(app, &row).map_err(|_| Failure::code(FailureCode::KeyProblem))?;
    lookup_with(app, &row.cfg, key.as_ref().map(|k| k.as_str()), ip).await
}

/// The part of a lookup that the test button shares with the worker: the per-minute cap, the request, the answer.
pub async fn lookup_with(
    app: &App,
    cfg: &template::ProviderCfg,
    key: Option<&str>,
    ip: IpAddr,
) -> Result<GeoPlace, Failure> {
    let limit = Knobs::get(&app.knobs.geo_per_minute);
    if !app.geo.take_slot(crate::util::unix_now(), limit) {
        return Err(Failure::code(FailureCode::RateLimited));
    }
    let built =
        template::build(cfg, ip, key).map_err(|_| Failure::code(FailureCode::NotConfigured))?;
    let json = client::fetch(&app.geo.client, app.geo.total, built).await?;
    let encoded = key.map(template::percent_encode).unwrap_or_default();
    let secrets: Vec<&str> = key
        .into_iter()
        .chain((!encoded.is_empty()).then_some(encoded.as_str()))
        .collect();
    mapping::interpret(&json, &cfg.mapping, &secrets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_codes_round_trip() {
        for c in FailureCode::ALL {
            assert_eq!(FailureCode::parse(c.as_str()), Some(c));
        }
        assert_eq!(FailureCode::parse("nope"), None);
        let names: Vec<&str> = FailureCode::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            names,
            [
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
                "refused"
            ]
        );
    }

    #[test]
    fn the_minute_cap_counts_per_wall_clock_minute() {
        let mut cap = MinuteCap::default();
        let t0 = 1_763_200_040; // second 20 of a wall-clock minute
        for i in 0..30 {
            assert!(cap.take(t0 + i % 30, 30), "lookup {i}");
        }
        // the 31st of the same minute is refused, however often it is asked
        assert!(!cap.take(t0 + 39, 30));
        assert!(!cap.take(t0 + 39, 30));
        // the next wall-clock minute starts again
        assert!(cap.take(t0 + 40, 30));
        // a lower limit applies at once
        let mut small = MinuteCap::default();
        assert!(small.take(t0, 2) && small.take(t0, 2) && !small.take(t0, 2));
        // a limit of zero never lets one through
        assert!(!MinuteCap::default().take(t0, 0));
    }

    #[test]
    fn failure_lines_are_limited_per_minute_and_summed_up() {
        let mut log = FailLog::default();
        let t0 = 1_763_200_040; // second 20 of a wall-clock minute
        for i in 0..10 {
            assert_eq!(
                log.plan(t0 + i),
                LogPlan {
                    summary: None,
                    line: true
                }
            );
        }
        for _ in 0..7 {
            assert_eq!(
                log.plan(t0 + 30),
                LogPlan {
                    summary: None,
                    line: false
                }
            );
        }
        // the next minute: one summary of the held lines, then lines again
        assert_eq!(
            log.plan(t0 + 45),
            LogPlan {
                summary: Some(7),
                line: true
            }
        );
        // a quiet minute produces no summary; flush alone reports the held ones once
        let mut log = FailLog::default();
        for i in 0..12 {
            log.plan(t0 + i);
        }
        assert_eq!(log.roll(t0 + 5), None);
        assert_eq!(log.roll(t0 + 70), Some(2));
        assert_eq!(log.roll(t0 + 140), None);
    }

    #[test]
    fn the_place_and_failure_json_shapes() {
        let p = GeoPlace {
            country: None,
            country_code: Some("ZZ".into()),
            region: Some("Example Region".into()),
            city: Some("Example City".into()),
        };
        assert_eq!(
            p.to_json(),
            json!({"country": null, "countryCode": "ZZ", "region": "Example Region", "city": "Example City"})
        );
        assert!(!p.is_empty());
        assert!(GeoPlace::default().is_empty());
        let f = Failure {
            code: FailureCode::HttpStatus,
            status: Some(500),
            message: None,
        };
        assert_eq!(
            f.to_json(),
            json!({"code": "httpStatus", "status": 500, "message": null})
        );
    }

    #[test]
    fn one_test_at_a_time() {
        let g = GeoState::new("1.4", &Timeouts::default()).expect("state");
        let first = g.begin_test().expect("free");
        assert!(g.begin_test().is_none());
        // a refused attempt must not free the place of the running test
        assert!(g.begin_test().is_none());
        drop(first);
        assert!(g.begin_test().is_some());
    }
}
