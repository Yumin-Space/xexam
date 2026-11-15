//! `system_settings` (schema v8): a small key / value table for switches that belong to the whole installation, not to one
//! user. A missing row means the default.
//!
//! Keys: `activity.retention_days` (default 90, 7..=3650), `activity.user_visits` (`0` / `1`, default `0`) and
//! `users.signin_info` (`0` / `1`, default `0`: whether the stored sign-in IP and approximate location are SHOWN and
//! whether new ones are RECORDED: while it is `0` nothing new is recorded and the stored ones are hidden).
use super::{run_tx, Store};
use crate::error::DbError;
use rusqlite::{params, Connection, OptionalExtension};

/// Days the activity log keeps its rows.
pub const ACTIVITY_RETENTION_DAYS: &str = "activity.retention_days";
/// `1` = visits of plain users are logged too (administrators' visits always are).
pub const ACTIVITY_USER_VISITS: &str = "activity.user_visits";
/// `1` = the eye is open: the recorded IP and approximate location are shown and new ones are recorded; `0` = hidden and
/// nothing new is recorded.
pub const SIGNIN_INFO: &str = "users.signin_info";

pub const DEFAULT_RETENTION_DAYS: i64 = 90;
pub const MIN_RETENTION_DAYS: i64 = 7;
pub const MAX_RETENTION_DAYS: i64 = 3650;

pub(super) fn get(c: &Connection, key: &str) -> Result<Option<String>, DbError> {
    Ok(c.query_row(
        "SELECT value FROM system_settings WHERE key=?1",
        params![key],
        |r| r.get(0),
    )
    .optional()?)
}

/// A `0` / `1` switch (anything but `1`, and a missing row, is off).
pub(super) fn flag(c: &Connection, key: &str) -> Result<bool, DbError> {
    Ok(get(c, key)?.as_deref() == Some("1"))
}

fn put(c: &Connection, key: &str, value: &str, at: &str, by: Option<i64>) -> Result<(), DbError> {
    c.execute(
        "INSERT INTO system_settings(key, value, updated_at, updated_by) VALUES(?1,?2,?3,?4) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at, updated_by=excluded.updated_by",
        params![key, value, at, by],
    )?;
    Ok(())
}

impl Store {
    /// The stored value of `key`, `None` = the default applies.
    pub fn system_setting(&self, key: &str) -> Result<Option<String>, DbError> {
        let g = self.lock();
        get(&g, key)
    }

    /// Stores `value` under `key` (`by` = the administrator who changed it).
    pub fn set_system_setting(
        &self,
        key: &str,
        value: &str,
        by: Option<i64>,
    ) -> Result<(), DbError> {
        let at = self.clock.now();
        let g = self.lock();
        run_tx(&g, |c| put(c, key, value, &at, by))
    }

    /// Days the activity log is kept: the stored value clamped to 7..=3650, default 90.
    pub fn activity_retention_days(&self) -> Result<i64, DbError> {
        let g = self.lock();
        Ok(get(&g, ACTIVITY_RETENTION_DAYS)?
            .and_then(|v| v.trim().parse::<i64>().ok())
            .unwrap_or(DEFAULT_RETENTION_DAYS)
            .clamp(MIN_RETENTION_DAYS, MAX_RETENTION_DAYS))
    }

    /// Whether visits of plain users are logged.
    pub fn activity_user_visits(&self) -> Result<bool, DbError> {
        let g = self.lock();
        flag(&g, ACTIVITY_USER_VISITS)
    }

    /// Whether the recorded sign-in IP and location are shown.
    pub fn signin_info_enabled(&self) -> Result<bool, DbError> {
        let g = self.lock();
        flag(&g, SIGNIN_INFO)
    }

    /// Turns the sign-in info switch on or off. Nothing stored is changed: while off, nothing new is recorded and the stored values
    /// are hidden; turning it on shows them again and recording resumes. Clearing is a separate action ([`Store::clear_signin_info`]).
    pub fn set_signin_info(&self, enabled: bool, by: Option<i64>) -> Result<(), DbError> {
        let at = self.clock.now();
        let g = self.lock();
        run_tx(&g, |c| {
            put(c, SIGNIN_INFO, if enabled { "1" } else { "0" }, &at, by)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::FixedClock;
    use std::sync::Arc;

    fn open() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let s = Store::open_with_clock(
            &dir.path().join("x.db"),
            Arc::new(FixedClock("2026-01-02 03:04".to_string())),
        )
        .expect("open");
        (dir, s)
    }

    #[test]
    fn defaults_set_and_clamp() {
        let (_d, s) = open();
        assert_eq!(s.system_setting("nope").expect("get"), None);
        assert_eq!(s.activity_retention_days().expect("days"), 90);
        assert!(!s.activity_user_visits().expect("v"));
        assert!(!s.signin_info_enabled().expect("i"));
        s.set_system_setting(ACTIVITY_RETENTION_DAYS, "2", Some(1))
            .expect("set");
        assert_eq!(s.activity_retention_days().expect("days"), 7);
        s.set_system_setting(ACTIVITY_RETENTION_DAYS, "99999", None)
            .expect("set");
        assert_eq!(s.activity_retention_days().expect("days"), 3650);
        s.set_system_setting(ACTIVITY_RETENTION_DAYS, "junk", None)
            .expect("set");
        assert_eq!(s.activity_retention_days().expect("days"), 90);
        s.set_system_setting(ACTIVITY_USER_VISITS, "1", None)
            .expect("set");
        assert!(s.activity_user_visits().expect("v"));
        assert_eq!(
            s.system_setting(ACTIVITY_USER_VISITS).expect("get"),
            Some("1".to_string())
        );
    }
}
