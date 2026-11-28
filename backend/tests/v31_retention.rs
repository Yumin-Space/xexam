//! v3.1: retention — the refused sign-in rows of the activity log (fixed 90 days after their latest merged attempt, accepted
//! sign-ins exempt as before) and the hourly housekeeping of unused devices (30 days without a session, at most 20 rows per
//! user).
//!
//! Real store, made-up users; the times are passed in, never waited for.
#[path = "common/mod.rs"]
mod dbc;

use rusqlite::Connection;
use std::path::Path;
use xexam_backend::auth::{self, Role};
use xexam_backend::database::{ActivityRow, ActivitySource, Merge, Store, UserChange};

const DAY: i64 = 86_400;

fn raw(p: &Path) -> Connection {
    Connection::open(p).expect("raw open")
}

fn row(action: &str, ts: i64, ok: bool) -> ActivityRow {
    ActivityRow {
        ts,
        actor: "system".into(),
        user_id: None,
        username: None,
        role: None,
        action: action.to_string(),
        object: None,
        detail: None,
        ok,
        reason: if ok { None } else { Some("badPassword".into()) },
        ip: Some("203.0.113.5".into()),
        country: Some("ZZ".into()),
        region: None,
        city: None,
        ua: None,
        source: ActivitySource::Backend,
        src_id: None,
        merge: Merge::None,
    }
}

fn count(db: &Path, sql: &str) -> i64 {
    raw(db).query_row(sql, [], |r| r.get(0)).expect("count")
}

// ---------------------------------------------------------------- E.5: refused sign-ins

#[test]
fn only_refused_sign_ins_past_ninety_days_go_and_accepted_ones_stay() {
    let dir = dbc::tmp();
    let db = dir.path().join("x.db");
    let store = Store::open(&db).expect("store");
    let now = auth::unix_now();
    let mut batch = Vec::new();
    // 89 and 91 days old, accepted and refused (each with its own timestamp so none merges)
    for (days, ok) in [(89, true), (89, false), (91, true), (91, false)] {
        batch.push(row("auth.signin", now - days * DAY, ok));
    }
    store.activity_insert(&batch).expect("insert");
    assert_eq!(count(&db, "SELECT COUNT(*) FROM activity_log"), 4);
    // a `days` setting of 7: ordinary rows older than a week would go, accepted sign-ins never do
    let deleted = store
        .activity_retention(now, 7, 100_000)
        .expect("retention");
    assert_eq!(deleted, 1, "only the 91-day refused sign-in");
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM activity_log WHERE action = 'auth.signin' AND result = 'ok'"
        ),
        2
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM activity_log WHERE result = 'fail'"
        ),
        1,
        "the 89-day refused sign-in stays"
    );
    assert_eq!(
        count(
            &db,
            &format!(
                "SELECT COUNT(*) FROM activity_log WHERE result = 'fail' AND ts = {}",
                now - 89 * DAY
            )
        ),
        1
    );
    // a second run has nothing left to do
    assert_eq!(store.activity_retention(now, 7, 100_000).expect("again"), 0);
}

#[test]
fn a_refused_row_still_being_merged_into_stays_whatever_its_first_time() {
    let dir = dbc::tmp();
    let db = dir.path().join("x.db");
    let store = Store::open(&db).expect("store");
    let now = auth::unix_now();
    store
        .activity_insert(&[row("auth.signin", now - 100 * DAY, false)])
        .expect("insert");
    // the burst is still going on: its latest merged attempt was 10 days ago
    raw(&db)
        .execute(
            "UPDATE activity_log SET last_ts = ?1, count = 40",
            [now - 10 * DAY],
        )
        .expect("merge");
    assert_eq!(
        store
            .activity_retention(now, 7, 100_000)
            .expect("retention"),
        0
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM activity_log"), 1);
    // once its last attempt is past 90 days it goes
    raw(&db)
        .execute("UPDATE activity_log SET last_ts = ?1", [now - 91 * DAY])
        .expect("age");
    assert_eq!(
        store
            .activity_retention(now, 7, 100_000)
            .expect("retention"),
        1
    );
}

#[test]
fn the_refused_rule_is_fixed_and_ignores_the_days_setting_and_other_actions() {
    let dir = dbc::tmp();
    let db = dir.path().join("x.db");
    let store = Store::open(&db).expect("store");
    let now = auth::unix_now();
    store
        .activity_insert(&[
            row("auth.signin", now - 30 * DAY, false),
            row("auth.expired", now - 95 * DAY, false),
        ])
        .expect("insert");
    // a long `days` setting: the 30-day refused sign-in stays (fixed 90 days), the other failure follows `days`
    assert_eq!(
        store
            .activity_retention(now, 365, 100_000)
            .expect("retention"),
        0
    );
    // a short one: the ordinary failure goes, the refused sign-in still stays
    assert_eq!(
        store
            .activity_retention(now, 7, 100_000)
            .expect("retention"),
        1
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM activity_log WHERE action = 'auth.signin'"
        ),
        1
    );
}

#[test]
fn many_refused_rows_all_go() {
    let dir = dbc::tmp();
    let db = dir.path().join("x.db");
    let store = Store::open(&db).expect("store");
    let now = auth::unix_now();
    let batch: Vec<ActivityRow> = (0..120)
        .map(|i| row("auth.signin", now - 200 * DAY - i, false))
        .collect();
    store.activity_insert(&batch).expect("insert");
    assert_eq!(count(&db, "SELECT COUNT(*) FROM activity_log"), 120);
    assert_eq!(
        store
            .activity_retention(now, 7, 100_000)
            .expect("retention"),
        120
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM activity_log"), 0);
}

// ---------------------------------------------------------------- C.13: devices

fn store_with_users() -> (tempfile::TempDir, std::path::PathBuf, Store) {
    let dir = dbc::tmp();
    let db = dir.path().join("x.db");
    let store = Store::open(&db).expect("store");
    let hash = auth::hash_password("kitchen-pass-1").expect("hash");
    for name in ["kim", "lee"] {
        match store.create_user(name, &hash, Role::User).expect("user") {
            UserChange::Done(_) => {}
            other => panic!("{other:?}"),
        }
    }
    (dir, db, store)
}

fn device(db: &Path, user: i64, n: i64, active_at: i64) -> i64 {
    let c = raw(db);
    c.execute(
        "INSERT INTO devices(user_id, cookie_hash, created_at, active_at) VALUES(?1, ?2, ?3, ?3)",
        rusqlite::params![user, format!("{n:064x}"), active_at],
    )
    .expect("device");
    c.last_insert_rowid()
}

fn session_on(db: &Path, user: i64, device: i64, tag: &str) {
    raw(db)
        .execute(
            "INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at, started_at, remember, device_id) \
             VALUES(?1, ?2, '2026-11-28 00:00', 1000, 2000, 1000, 1, ?3)",
            rusqlite::params![auth::token_hash(tag), user, device],
        )
        .expect("session");
}

#[test]
fn a_device_unused_for_thirty_days_without_a_session_is_deleted() {
    let (_dir, db, store) = store_with_users();
    let now = 100 * DAY;
    let old = device(&db, 2, 1, now - 31 * DAY);
    let edge = device(&db, 2, 2, now - 29 * DAY);
    let kept = device(&db, 2, 3, now - 60 * DAY);
    session_on(&db, 2, kept, "old but still signed in");
    assert_eq!(store.devices_housekeeping(now).expect("housekeeping"), 1);
    let ids: Vec<i64> = {
        let c = raw(&db);
        let mut st = c.prepare("SELECT id FROM devices ORDER BY id").expect("p");
        st.query_map([], |r| r.get(0))
            .expect("q")
            .map(|r| r.expect("id"))
            .collect()
    };
    assert_eq!(ids, vec![edge, kept]);
    assert!(!ids.contains(&old));
    // nothing more to do
    assert_eq!(store.devices_housekeeping(now).expect("again"), 0);
}

#[test]
fn a_device_with_an_expired_session_row_that_the_purge_has_not_taken_yet_stays() {
    let (_dir, db, store) = store_with_users();
    let now = 100 * DAY;
    let d = device(&db, 2, 1, now - 45 * DAY);
    // the session's `expires_at` is long past, but the row exists: the device is not "without a session row"
    session_on(&db, 2, d, "expired row");
    assert_eq!(store.devices_housekeeping(now).expect("housekeeping"), 0);
    // once the hourly purge deleted the row, the next housekeeping takes the unused device
    raw(&db).execute("DELETE FROM sessions", []).expect("purge");
    assert_eq!(store.devices_housekeeping(now).expect("housekeeping"), 1);
}

#[test]
fn a_user_keeps_at_most_twenty_devices_the_oldest_unused_ones_without_a_session_go_first() {
    let (_dir, db, store) = store_with_users();
    let now = 100 * DAY;
    // 23 recent devices of kim (ids ascending = activity ascending), the oldest three have no session... except the very
    // oldest, which holds a session and therefore stays
    let mut ids = Vec::new();
    for i in 0..23 {
        ids.push(device(&db, 2, i + 1, now - 10 * DAY + i));
    }
    session_on(&db, 2, ids[0], "the oldest has a session");
    // lee has two: untouched
    device(&db, 3, 100, now - 10 * DAY);
    device(&db, 3, 101, now - 9 * DAY);
    assert_eq!(store.devices_housekeeping(now).expect("housekeeping"), 3);
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM devices WHERE user_id = 2"),
        20
    );
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM devices WHERE user_id = 3"),
        2
    );
    // ids[0] (session) stays; the next three oldest went
    let gone: Vec<i64> = ids[1..4].to_vec();
    for g in gone {
        assert_eq!(
            count(&db, &format!("SELECT COUNT(*) FROM devices WHERE id = {g}")),
            0
        );
    }
    assert_eq!(
        count(
            &db,
            &format!("SELECT COUNT(*) FROM devices WHERE id = {}", ids[0])
        ),
        1
    );
}

#[test]
fn devices_that_all_hold_sessions_are_never_trimmed() {
    let (_dir, db, store) = store_with_users();
    let now = 100 * DAY;
    for i in 0..22 {
        let d = device(&db, 2, i + 1, now - 10 * DAY + i);
        session_on(&db, 2, d, &format!("session {i}"));
    }
    assert_eq!(store.devices_housekeeping(now).expect("housekeeping"), 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM devices"), 22);
}

#[test]
fn housekeeping_leaves_the_foreign_keys_whole() {
    let (_dir, db, store) = store_with_users();
    let now = 100 * DAY;
    for i in 0..5 {
        device(&db, 2, i + 1, now - 40 * DAY);
    }
    let d = device(&db, 2, 50, now - 1);
    session_on(&db, 2, d, "live");
    assert_eq!(store.devices_housekeeping(now).expect("housekeeping"), 5);
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM pragma_foreign_key_check"),
        0
    );
}
