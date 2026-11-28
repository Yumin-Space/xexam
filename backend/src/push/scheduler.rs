//! Push start-up work, the periodic ticks and the housekeeping (contracts C.8).
//!
//! [`start`] runs once from `main.rs` right after the application state is built and before `system.start` is recorded: it
//! loads the VAPID key, removes subscriptions of an old key and runs the version trigger. [`spawn`] then starts the
//! 60-second scheduler: the bank tick, the reminder tick and, every 60th tick, the housekeeping. The scheduler is not started
//! by the test helpers; tests call [`tick_once`] with a pinned time.

use crate::api::AppState;
use crate::push::vapid;
use std::time::Duration;

/// Seconds between two ticks.
pub const TICK_SECS: u64 = 60;
/// The first tick comes this long after start (seconds).
pub const FIRST_TICK_SECS: u64 = 5;
/// Housekeeping runs on every this many ticks (once an hour), and on the first.
pub const HOUSEKEEPING_EVERY: u64 = 60;
/// Longest `XEXAM_PUSH_SUBJECT` accepted (bytes).
pub const SUBJECT_MAX: usize = 200;

/// What [`start`] did, for the caller's log lines.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StartReport {
    /// `(from, to, devices)` when a version push was queued
    pub version_push: Option<(Option<String>, String, usize)>,
}

/// Whether `s` is an acceptable `sub` claim: starts with `mailto:` or `https://`, printable ASCII without spaces, at most
/// [`SUBJECT_MAX`] bytes.
pub fn subject_ok(s: &str) -> bool {
    s.len() <= SUBJECT_MAX
        && (s.starts_with("mailto:") || s.starts_with("https://"))
        && s.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

/// The `sub` claim: the configured one when valid, else the derived default (with a warning when nothing better than
/// `mailto:admin@localhost` exists).
fn subject_for(cfg: &crate::config::Config) -> String {
    if let Some(s) = cfg.push_subject.as_deref() {
        if subject_ok(s) {
            return s.to_string();
        }
        tracing::warn!(
            "push: XEXAM_PUSH_SUBJECT is not valid (a mailto: or https: address without spaces, at most {SUBJECT_MAX} bytes); using the default"
        );
    }
    let (subject, derived) = vapid::default_subject(cfg);
    if !derived {
        tracing::warn!(
            "push: set XEXAM_PUSH_SUBJECT (a mailto: or https: address); some push services refuse the default"
        );
    }
    subject
}

/// Loads the VAPID key, cleans subscriptions of an old key, runs the version trigger. Failures are logged and leave push off
/// (the caller never stops the server for them).
pub fn start(st: &AppState) -> Result<StartReport, String> {
    let cfg = &st.0.cfg;
    let subject = subject_for(cfg);
    let env_value = std::env::var(vapid::KEY_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty());
    let key = match vapid::Vapid::load_or_create(&cfg.data_dir, env_value.as_deref()) {
        Ok((key, source)) => {
            tracing::info!(
                "push: VAPID key {} ({})",
                key.key_id(),
                match source {
                    vapid::KeySource::Env => "from the environment",
                    vapid::KeySource::File => "from the key file",
                    vapid::KeySource::Created => "new key file created",
                }
            );
            Some(key)
        }
        Err(e) => {
            tracing::warn!("push: off, the VAPID key cannot be used: {e}");
            None
        }
    };
    let key_id = key.as_ref().map(vapid::Vapid::key_id);
    st.0.push.set_key(key, subject);
    let store = &st.0.store;
    if let Some(id) = key_id {
        match store.push_key_cleanup(&id) {
            Ok(0) => {}
            Ok(n) => tracing::info!("push: removed {n} subscription(s) of an old key"),
            Err(e) => tracing::warn!("push: cleaning subscriptions of an old key failed: {e}"),
        }
    }
    let now = store.unix_now();
    let version_push = store
        .push_version_check(&cfg.version, now)
        .map_err(|e| format!("push: the version check failed: {e}"))?;
    if let Some((from, to, devices)) = &version_push {
        tracing::info!(
            "push: version {} -> {to}: queued for {devices} device(s)",
            from.as_deref().unwrap_or("?")
        );
    }
    Ok(StartReport { version_push })
}

/// One scheduler tick (public for tests); `housekeeping` also runs the hourly clean-up. Blocking: the callers use a blocking
/// thread.
pub fn tick_once(st: &AppState, now: i64, housekeeping: bool) {
    let store = &st.0.store;
    let mut wake = false;
    match store.push_bank_tick(now) {
        Ok(n) => wake |= n > 0,
        Err(e) => tracing::warn!("push: bank tick failed: {e}"),
    }
    match store.push_remind_tick(now) {
        Ok(n) => wake |= n > 0,
        Err(e) => tracing::warn!("push: reminder tick failed: {e}"),
    }
    match store.push_due(now) {
        Ok(due) => wake |= due,
        Err(e) => tracing::warn!("push: due check failed: {e}"),
    }
    if wake {
        st.0.push.wake();
    }
    if housekeeping {
        match store.push_housekeeping(now) {
            Ok(h) if h.expired + h.deleted + h.subscriptions + h.saved > 0 => tracing::info!(
                "push: housekeeping expired {}, deleted {} row(s), {} subscription(s), {} saved session(s)",
                h.expired,
                h.deleted,
                h.subscriptions,
                h.saved
            ),
            Ok(_) => {}
            Err(e) => tracing::warn!("push: housekeeping failed: {e}"),
        }
        match store.devices_housekeeping(now) {
            Ok(0) => {}
            Ok(n) => tracing::info!("devices: removed {n} unused device(s)"),
            Err(e) => tracing::warn!("devices: housekeeping failed: {e}"),
        }
    }
}

/// Spawns the 60-second scheduler task (first tick 5 seconds after start).
pub fn spawn(st: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(FIRST_TICK_SECS),
            Duration::from_secs(TICK_SECS),
        );
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut n: u64 = 0;
        loop {
            interval.tick().await;
            if st.0.push.is_closed() {
                break;
            }
            let housekeeping = n.is_multiple_of(HOUSEKEEPING_EVERY);
            n += 1;
            let st2 = st.clone();
            let now = st.0.store.unix_now();
            if let Err(e) =
                tokio::task::spawn_blocking(move || tick_once(&st2, now, housekeeping)).await
            {
                tracing::warn!("push: the scheduler tick ended abnormally: {e}");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_subject_rules() {
        assert!(subject_ok("mailto:admin@xexam.example.com"));
        assert!(subject_ok("https://xexam.example.com"));
        assert!(!subject_ok("http://xexam.example.com"));
        assert!(!subject_ok("mailto:a b@x.example"));
        assert!(!subject_ok("admin@xexam.example.com"));
        assert!(!subject_ok("mailto:é@xexam.example.com"));
        assert!(!subject_ok(&format!("https://{}", "a".repeat(200))));
        assert!(subject_ok(&format!("https://{}", "a".repeat(192))));
    }
}
