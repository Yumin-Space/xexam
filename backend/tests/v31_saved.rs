//! v3.1 (P4b): the saved-session marker (`POST /api/study/saved`, `/discard`, the clearing by `POST /api/attempt`) and the
//! reminder tick: the throttle, `timeLeft` 0, one session per device, the D10 schedule with its night shifts and the cap of
//! three reminders, and what leaves a reminder unqueued.
//!
//! Real database and store; the HTTP tests also use the real router and engine. A test that needs the devices package (a
//! session that carries its device) or the crypto package says so and returns early until they are merged. Only made-up
//! data is used.
#[path = "api_common/mod.rs"]
mod apic;
#[path = "common/mod.rs"]
mod dbc;
#[path = "push_common/mod.rs"]
mod pc;

use pc::{crypto_ready, skipped, ts, Env, DAY, HOUR};
use rusqlite::params;
use serde_json::{json, Value};
use xexam_backend::database::store::SavedReport;
use xexam_backend::push::scheduler::tick_once;

const MSG_BAD_REQUEST: &str = "请求的格式不对。";
const KEY1: &str = "0123456789abcdef0123456789abcdef";
const KEY2: &str = "fedcba9876543210fedcba9876543210";

fn report(key: &str, kind: &str, answered: i64, total: i64, time_left: Option<i64>) -> SavedReport {
    SavedReport {
        key: key.to_string(),
        kind: kind.to_string(),
        answered,
        total,
        started_at: 1_780_000_000,
        time_left,
    }
}

/// `(device_id, kind, answered, total, time_left, saved_at, remind_at, reminders)` of the marker of `(user, key)`.
type Marker = (
    Option<i64>,
    String,
    i64,
    i64,
    Option<i64>,
    i64,
    Option<i64>,
    i64,
);

fn marker(e: &Env, user: i64, key: &str) -> Option<Marker> {
    use rusqlite::OptionalExtension;
    e.raw()
        .query_row(
            "SELECT device_id, kind, answered, total, time_left, saved_at, remind_at, reminders FROM saved_sessions \
             WHERE user_id = ?1 AND session_key = ?2",
            params![user, key],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            },
        )
        .optional()
        .expect("marker")
}

fn marker_count(e: &Env) -> i64 {
    e.int("SELECT COUNT(*) FROM saved_sessions")
}

/// A signed device of `kim` with a subscription: `(user, device, subscription)`.
fn kim_with_subscription(e: &Env) -> (i64, i64, i64) {
    let kim = e.user("kim");
    let dev = e.device(kim, Some("UTC"));
    let (sub, _ua) = e.sub(dev);
    (kim, dev, sub)
}

// ---------------------------------------------------------------- the report

#[test]
fn a_report_stores_the_marker_and_schedules_the_first_reminder() {
    let t0 = ts(2026, 6, 10, 12, 0);
    let e = Env::at(t0);
    let kim = e.user("kim");
    let dev = e.device(kim, Some("UTC"));
    assert!(e
        .store
        .saved_report(kim, dev, &report(KEY1, "practice", 3, 20, None), t0)
        .expect("report"));
    assert_eq!(
        marker(&e, kim, KEY1),
        Some((
            Some(dev),
            "practice".into(),
            3,
            20,
            None,
            t0,
            Some(t0 + 3 * HOUR),
            0
        ))
    );
    // A timed exam keeps its time left
    assert!(e
        .store
        .saved_report(kim, dev, &report(KEY2, "exam", 5, 50, Some(1800)), t0 + 10)
        .expect("report"));
    let m = marker(&e, kim, KEY2).expect("exam marker");
    assert_eq!((m.1.as_str(), m.4), ("exam", Some(1800)));
    // The first one was replaced (one saved session per browser)
    assert_eq!(marker(&e, kim, KEY1), None);
}

#[test]
fn a_report_within_a_minute_only_refreshes_the_counts() {
    let t0 = ts(2026, 6, 10, 12, 0);
    let e = Env::at(t0);
    let kim = e.user("kim");
    let dev = e.device(kim, Some("UTC"));
    e.store
        .saved_report(kim, dev, &report(KEY1, "exam", 1, 20, Some(3000)), t0)
        .expect("report");
    // 30 seconds later: counts and time left only; saved_at and the schedule stay
    e.store
        .saved_report(kim, dev, &report(KEY1, "exam", 4, 20, Some(2900)), t0 + 30)
        .expect("report");
    let m = marker(&e, kim, KEY1).expect("marker");
    assert_eq!(
        (m.2, m.4, m.5, m.6),
        (4, Some(2900), t0, Some(t0 + 3 * HOUR))
    );
    // An answered count can never exceed the stored total (the CHECK of the table)
    e.store
        .saved_report(kim, dev, &report(KEY1, "exam", 20, 20, Some(10)), t0 + 40)
        .expect("report");
    assert_eq!(marker(&e, kim, KEY1).expect("marker").2, 20);
    // Time is up inside the throttle window: the reminder is cancelled
    e.store
        .saved_report(kim, dev, &report(KEY1, "exam", 20, 20, Some(0)), t0 + 50)
        .expect("report");
    let m = marker(&e, kim, KEY1).expect("marker");
    assert_eq!((m.4, m.6), (Some(0), None));
    // A minute after the last full write the schedule restarts, whatever was counted before
    e.exec("UPDATE saved_sessions SET reminders = 2", []);
    e.store
        .saved_report(kim, dev, &report(KEY1, "exam", 6, 20, Some(2000)), t0 + 61)
        .expect("report");
    let m = marker(&e, kim, KEY1).expect("marker");
    assert_eq!((m.5, m.6, m.7), (t0 + 61, Some(t0 + 61 + 3 * HOUR), 0));
}

#[test]
fn time_up_means_no_reminders_at_all() {
    let t0 = ts(2026, 6, 10, 12, 0);
    let e = Env::at(t0);
    let (kim, dev, sub) = kim_with_subscription(&e);
    e.store
        .saved_report(kim, dev, &report(KEY1, "exam", 10, 10, Some(0)), t0)
        .expect("report");
    assert_eq!(marker(&e, kim, KEY1).expect("marker").6, None);
    assert_eq!(e.store.push_remind_tick(t0 + 10 * DAY).expect("tick"), 0);
    assert!(e.outbox_of(sub).is_empty());
    // A marker whose time ran out after it was written (remind_at still set) is silenced by the tick
    e.store
        .saved_report(kim, dev, &report(KEY2, "exam", 2, 10, Some(600)), t0 + 100)
        .expect("report");
    e.exec("UPDATE saved_sessions SET time_left = 0", []);
    assert_eq!(e.store.push_remind_tick(t0 + 5 * HOUR).expect("tick"), 0);
    assert_eq!(marker(&e, kim, KEY2).expect("marker").6, None);
    assert!(e.outbox_of(sub).is_empty());
}

#[test]
fn a_new_session_replaces_the_old_one_of_the_same_device_only() {
    let t0 = ts(2026, 6, 10, 12, 0);
    let e = Env::at(t0);
    let kim = e.user("kim");
    let d1 = e.device(kim, Some("UTC"));
    let d2 = e.device(kim, Some("UTC"));
    e.store
        .saved_report(kim, d1, &report(KEY1, "practice", 1, 5, None), t0)
        .expect("report");
    e.store
        .saved_report(kim, d2, &report(KEY2, "practice", 2, 5, None), t0)
        .expect("report");
    // Both browsers keep their own saved session
    assert_eq!(marker_count(&e), 2);
    let key3 = "11111111111111111111111111111111";
    e.store
        .saved_report(kim, d1, &report(key3, "practice", 0, 5, None), t0 + 5)
        .expect("report");
    assert_eq!(marker(&e, kim, KEY1), None);
    assert!(marker(&e, kim, key3).is_some());
    assert!(marker(&e, kim, KEY2).is_some());
}

#[test]
fn a_device_of_another_user_stores_nothing_and_a_disabled_user_is_refused() {
    let t0 = ts(2026, 6, 10, 12, 0);
    let e = Env::at(t0);
    let (kim, lee) = (e.user("kim"), e.user("lee"));
    let lees = e.device(lee, Some("UTC"));
    assert!(!e
        .store
        .saved_report(kim, lees, &report(KEY1, "practice", 1, 5, None), t0)
        .expect("report"));
    assert_eq!(marker_count(&e), 0);
    let kims = e.device(kim, Some("UTC"));
    e.exec("UPDATE users SET disabled = 1 WHERE id = ?1", params![kim]);
    let r = e
        .store
        .saved_report(kim, kims, &report(KEY1, "practice", 1, 5, None), t0);
    assert!(
        matches!(r, Err(xexam_backend::error::DbError::NoUser)),
        "{r:?}"
    );
    assert_eq!(marker_count(&e), 0);
}

#[test]
fn discard_and_done_remove_only_that_marker() {
    let t0 = ts(2026, 6, 10, 12, 0);
    let e = Env::at(t0);
    let (kim, lee) = (e.user("kim"), e.user("lee"));
    let (dk, dl) = (e.device(kim, None), e.device(lee, None));
    e.store
        .saved_report(kim, dk, &report(KEY1, "practice", 1, 5, None), t0)
        .expect("report");
    e.store
        .saved_report(lee, dl, &report(KEY1, "practice", 1, 5, None), t0)
        .expect("report");
    // The same key of another user is untouched; no row is not an error
    e.store.saved_discard(kim, KEY1).expect("discard");
    e.store.saved_discard(kim, KEY1).expect("discard again");
    assert_eq!(marker(&e, kim, KEY1), None);
    assert!(marker(&e, lee, KEY1).is_some());
    e.store.saved_done(lee, "no-such-key").expect("done");
    assert!(marker(&e, lee, KEY1).is_some());
    e.store.saved_done(lee, KEY1).expect("done");
    assert_eq!(marker_count(&e), 0);
}

// ---------------------------------------------------------------- the reminder tick

/// Marks every queued row as sent, as if the courier had delivered it (no crypto needed).
fn deliver_all(e: &Env, now: i64) {
    e.exec(
        "UPDATE push_outbox SET state = 'sent', done_at = ?1 WHERE state = 'queued'",
        params![now],
    );
}

#[test]
fn the_night_shift_keeps_three_separate_reminders_saved_at_21_00() {
    if !crypto_ready() {
        return skipped("the push package (quiet hours)");
    }
    let saved = ts(2026, 6, 10, 21, 0);
    let e = Env::at(saved);
    let (kim, dev, sub) = kim_with_subscription(&e);
    e.store
        .saved_report(kim, dev, &report(KEY1, "practice", 4, 30, None), saved)
        .expect("report");
    assert_eq!(
        marker(&e, kim, KEY1).expect("m").6,
        Some(ts(2026, 6, 11, 0, 0))
    );
    // Nothing is due before 00:00
    assert_eq!(
        e.store
            .push_remind_tick(ts(2026, 6, 10, 23, 59))
            .expect("tick"),
        0
    );
    assert!(e.outbox().is_empty());
    // Reminder 1, due 00:00: queued, held to 08:00, kept for its full 6 h after the release
    let t1 = ts(2026, 6, 11, 0, 0);
    e.set_now(t1);
    assert_eq!(
        e.store.push_remind_tick(t1).expect("tick"),
        0,
        "held, not sendable now"
    );
    let rows = e.outbox_of(sub);
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r.kind, "remind");
    assert_eq!(r.topic.as_deref(), Some("remind"));
    assert_eq!(r.next_at, ts(2026, 6, 11, 8, 0));
    assert_eq!(r.expires_at, ts(2026, 6, 11, 14, 0));
    assert_eq!(r.payload["n"], 1);
    assert_eq!(r.payload["key"], KEY1);
    assert_eq!(r.payload["answered"], 4);
    assert_eq!(r.payload["total"], 30);
    assert_eq!(r.payload["kind"], "practice");
    // Reminder 2 is due at 11:00 (3 h after the release), reminder 3 at 14:00
    let m = marker(&e, kim, KEY1).expect("m");
    assert_eq!((m.7, m.6), (1, Some(ts(2026, 6, 11, 11, 0))));
    // 08:00: the courier delivers reminder 1
    deliver_all(&e, ts(2026, 6, 11, 8, 0));
    let t2 = ts(2026, 6, 11, 10, 59);
    e.set_now(t2);
    assert_eq!(
        e.store.push_remind_tick(t2).expect("tick"),
        0,
        "not due yet"
    );
    let t2 = ts(2026, 6, 11, 11, 0);
    e.set_now(t2);
    assert_eq!(
        e.store.push_remind_tick(t2).expect("tick"),
        1,
        "sendable at once"
    );
    let rows = e.outbox_of(sub);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (rows[1].payload["n"].as_i64(), rows[1].next_at),
        (Some(2), t2)
    );
    let m = marker(&e, kim, KEY1).expect("m");
    assert_eq!((m.7, m.6), (2, Some(ts(2026, 6, 11, 14, 0))));
    deliver_all(&e, t2);
    let t3 = ts(2026, 6, 11, 14, 0);
    e.set_now(t3);
    assert_eq!(e.store.push_remind_tick(t3).expect("tick"), 1);
    assert_eq!(e.outbox_of(sub).len(), 3);
    assert_eq!(e.outbox_of(sub)[2].payload["n"], 3);
    // A fourth never comes
    let m = marker(&e, kim, KEY1).expect("m");
    assert_eq!((m.7, m.6), (3, None));
    deliver_all(&e, t3);
    assert_eq!(e.store.push_remind_tick(t3 + 30 * DAY).expect("tick"), 0);
    assert_eq!(e.outbox_of(sub).len(), 3);
}

#[test]
fn the_night_shift_saved_at_23_30() {
    if !crypto_ready() {
        return skipped("the push package (quiet hours)");
    }
    let saved = ts(2026, 6, 10, 23, 30);
    let e = Env::at(saved);
    let (kim, dev, sub) = kim_with_subscription(&e);
    e.store
        .saved_report(kim, dev, &report(KEY1, "exam", 7, 40, Some(1500)), saved)
        .expect("report");
    // Reminder 1 at 02:30, held to 08:00; nothing else before 08:00
    let t1 = ts(2026, 6, 11, 2, 30);
    e.set_now(t1);
    e.store.push_remind_tick(t1).expect("tick");
    let rows = e.outbox_of(sub);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].next_at, ts(2026, 6, 11, 8, 0));
    assert_eq!(rows[0].payload["kind"], "exam");
    for (h, m) in [(3, 0), (5, 30), (7, 59)] {
        let t = ts(2026, 6, 11, h, m);
        e.set_now(t);
        e.store.push_remind_tick(t).expect("tick");
        assert_eq!(e.outbox_of(sub).len(), 1, "nothing new at {h}:{m:02}");
    }
    // Then 11:00 and 14:00 (reminder 1 was delivered at 08:00)
    deliver_all(&e, ts(2026, 6, 11, 8, 0));
    for (n, h) in [(2, 11), (3, 14)] {
        let t = ts(2026, 6, 11, h, 0);
        e.set_now(t);
        e.store.push_remind_tick(t).expect("tick");
        let rows = e.outbox_of(sub);
        assert_eq!(rows.last().expect("row").payload["n"], n);
        deliver_all(&e, t);
    }
    assert_eq!(e.outbox_of(sub).len(), 3);
}

#[test]
fn a_daytime_save_reminds_after_three_hours_then_every_three_hours() {
    let saved = ts(2026, 6, 10, 9, 0);
    let e = Env::at(saved);
    let (kim, dev, sub) = kim_with_subscription(&e);
    e.store
        .saved_report(kim, dev, &report(KEY1, "practice", 1, 9, None), saved)
        .expect("report");
    for (n, h) in [(1, 12), (2, 15), (3, 18)] {
        let t = ts(2026, 6, 10, h, 0);
        e.set_now(t);
        assert_eq!(e.store.push_remind_tick(t - 1).expect("tick"), 0);
        assert_eq!(
            e.store.push_remind_tick(t).expect("tick"),
            1,
            "reminder {n}"
        );
        let last = e.outbox_of(sub).pop().expect("row");
        assert_eq!((last.payload["n"].as_i64(), last.next_at), (Some(n), t));
        deliver_all(&e, t);
    }
    assert_eq!(marker(&e, kim, KEY1).expect("m").6, None);
}

#[test]
fn a_held_reminder_is_replaced_by_a_newer_one_when_the_courier_is_down() {
    if !crypto_ready() {
        return skipped("the push package (quiet hours)");
    }
    let saved = ts(2026, 6, 10, 21, 0);
    let e = Env::at(saved);
    let (kim, dev, sub) = kim_with_subscription(&e);
    e.store
        .saved_report(kim, dev, &report(KEY1, "practice", 4, 30, None), saved)
        .expect("report");
    let t1 = ts(2026, 6, 11, 0, 0);
    e.store.push_remind_tick(t1).expect("tick");
    // Nothing delivered: reminder 2 (11:00) finds reminder 1 still queued and replaces it (newest only, as on the device)
    e.store
        .push_remind_tick(ts(2026, 6, 11, 11, 0))
        .expect("tick");
    let rows = e.outbox_of(sub);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].payload["n"], 2);
    assert_eq!(rows[0].state, "queued");
    // next_at kept at the earlier release, expiry kept at the later one
    assert_eq!(rows[0].next_at, ts(2026, 6, 11, 8, 0));
    assert_eq!(rows[0].expires_at, ts(2026, 6, 11, 17, 0));
}

#[test]
fn every_save_resets_the_schedule() {
    let t0 = ts(2026, 6, 10, 12, 0);
    let e = Env::at(t0);
    let (kim, dev, _sub) = kim_with_subscription(&e);
    e.store
        .saved_report(kim, dev, &report(KEY1, "practice", 1, 9, None), t0)
        .expect("report");
    let t1 = t0 + 3 * HOUR;
    e.set_now(t1);
    e.store.push_remind_tick(t1).expect("tick");
    assert_eq!(marker(&e, kim, KEY1).expect("m").7, 1);
    let t2 = t1 + 10 * 60;
    e.set_now(t2);
    e.store
        .saved_report(kim, dev, &report(KEY1, "practice", 2, 9, None), t2)
        .expect("report");
    let m = marker(&e, kim, KEY1).expect("m");
    assert_eq!((m.7, m.6), (0, Some(t2 + 3 * HOUR)));
}

#[test]
fn a_finished_session_drops_its_marker_and_queues_nothing() {
    let t0 = ts(2026, 6, 10, 12, 0);
    let e = Env::at(t0);
    let (kim, dev, sub) = kim_with_subscription(&e);
    e.store
        .saved_report(kim, dev, &report(KEY1, "practice", 8, 9, None), t0)
        .expect("report");
    e.exec(
        "INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, user_id, kind, session_key) \
         VALUES('2026-06-10 12:30', '[]', 'all', 9, 9, 100, 60, ?1, 'practice', ?2)",
        params![kim, KEY1],
    );
    assert_eq!(e.store.push_remind_tick(t0 + 4 * HOUR).expect("tick"), 0);
    assert_eq!(marker_count(&e), 0);
    assert!(e.outbox_of(sub).is_empty());
}

#[test]
fn nothing_is_queued_for_a_device_that_may_not_receive_but_the_schedule_goes_on() {
    let t0 = ts(2026, 6, 10, 12, 0);
    let e = Env::at(t0);
    let kim = e.user("kim");
    // No subscription at all
    let bare = e.device(kim, Some("UTC"));
    e.store
        .saved_report(kim, bare, &report(KEY1, "practice", 1, 9, None), t0)
        .expect("report");
    e.store.push_remind_tick(t0 + 3 * HOUR).expect("tick");
    assert!(e.outbox().is_empty());
    let m = marker(&e, kim, KEY1).expect("m");
    assert_eq!((m.7, m.6), (1, Some(t0 + 6 * HOUR)));
    // A signed-out device, a device unused for 31 days, an account with the switch off, a disabled account
    for case in ["signed_out", "idle", "switch_off", "disabled"] {
        let e = Env::at(t0);
        let (kim, dev, sub) = kim_with_subscription(&e);
        e.store
            .saved_report(kim, dev, &report(KEY1, "practice", 1, 9, None), t0)
            .expect("report");
        match case {
            "signed_out" => e.exec("UPDATE devices SET signed_out_at = ?1", params![t0]),
            "idle" => e.exec("UPDATE devices SET active_at = ?1", params![t0 - 31 * DAY]),
            "switch_off" => e.set_setting(kim, "push", "false"),
            _ => e.exec("UPDATE users SET disabled = 1 WHERE id = ?1", params![kim]),
        }
        e.store.push_remind_tick(t0 + 3 * HOUR).expect("tick");
        assert!(e.outbox_of(sub).is_empty(), "{case}");
        assert_eq!(marker(&e, kim, KEY1).expect("m").7, 1, "{case}");
    }
}

#[test]
fn the_user_zone_decides_the_quiet_hours_of_a_reminder() {
    if !crypto_ready() {
        return skipped("the push package (quiet hours)");
    }
    // Saved at 12:00 UTC; the reminder is due 15:00 UTC = 00:00 in UTC+9: held to 08:00 local = 23:00 UTC
    let t0 = ts(2026, 6, 10, 12, 0);
    let e = Env::at(t0);
    let kim = e.user("kim");
    let dev = e.device(kim, Some("UTC"));
    let (sub, _ua) = e.sub(dev);
    e.set_setting(kim, "tz", "\"Etc/GMT-9\"");
    e.store
        .saved_report(kim, dev, &report(KEY1, "practice", 1, 9, None), t0)
        .expect("report");
    let t1 = t0 + 3 * HOUR;
    e.set_now(t1);
    e.store.push_remind_tick(t1).expect("tick");
    let rows = e.outbox_of(sub);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].next_at, ts(2026, 6, 10, 23, 0));
    // With `auto` the device's zone is used
    let e = Env::at(t0);
    let kim = e.user("kim");
    let dev = e.device(kim, Some("Etc/GMT-9"));
    let (sub, _ua) = e.sub(dev);
    e.store
        .saved_report(kim, dev, &report(KEY1, "practice", 1, 9, None), t0)
        .expect("report");
    e.store.push_remind_tick(t1).expect("tick");
    assert_eq!(e.outbox_of(sub)[0].next_at, ts(2026, 6, 10, 23, 0));
}

#[test]
fn tick_once_wakes_the_courier_when_something_is_due() {
    let t0 = ts(2026, 6, 10, 12, 0);
    let e = Env::at(t0);
    let (kim, dev, _sub) = kim_with_subscription(&e);
    e.store
        .saved_report(kim, dev, &report(KEY1, "practice", 1, 9, None), t0)
        .expect("report");
    let t1 = t0 + 3 * HOUR;
    e.set_now(t1);
    tick_once(&e.state, t1, false);
    assert_eq!(e.outbox().len(), 1);
    // The wake-up is kept for the next wait: a long wait returns at once
    let started = std::time::Instant::now();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    rt.block_on(e.state.0.push.wait(std::time::Duration::from_secs(30)));
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn a_reminder_that_is_delivered_carries_the_session_in_the_users_language() {
    if !crypto_ready() {
        return skipped("the push crypto and texts");
    }
    let saved = ts(2026, 6, 10, 21, 0);
    let e = Env::at(saved);
    let kim = e.user("kim");
    e.set_setting(kim, "uiLang", "\"zh\"");
    let dev = e.device(kim, Some("UTC"));
    let (sub, ua) = e.sub(dev);
    e.store
        .saved_report(kim, dev, &report(KEY1, "exam", 4, 30, Some(900)), saved)
        .expect("report");
    let t1 = ts(2026, 6, 11, 0, 0);
    e.set_now(t1);
    e.store.push_remind_tick(t1).expect("tick");
    // Held at 07:59, delivered at 08:00 with the cheer of reminder 1
    assert!(e.lease_at(ts(2026, 6, 11, 7, 59)).is_empty());
    assert_eq!(e.outbox_of(sub)[0].state, "queued");
    assert_eq!(e.outbox_of(sub)[0].tries, 0);
    e.set_now(ts(2026, 6, 11, 8, 0));
    let jobs = e.lease();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].topic.as_deref(), Some("remind"));
    assert_eq!(jobs[0].urgency, "normal");
    assert_eq!(jobs[0].ttl, 6 * 3600);
    let body = pc::decrypt(
        ua,
        &xexam_backend::push::b64_lenient(&jobs[0].body).expect("b64"),
    );
    let plain: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(plain["tag"], "remind");
    assert_eq!(plain["url"], "/#/quiz");
    assert_eq!(plain["lang"], "zh-CN");
    assert!(
        plain["body"].as_str().expect("body").contains("考试"),
        "{plain}"
    );
    assert!(
        plain["body"].as_str().expect("body").contains("4/30"),
        "{plain}"
    );
}

// ---------------------------------------------------------------- HTTP

fn saved_body(key: &str) -> Value {
    json!({"key": key, "kind": "practice", "answered": 2, "total": 10, "startedAt": 1_780_000_000, "timeLeft": null})
}

#[tokio::test]
async fn the_saved_routes_check_every_shape() {
    let e = Env::new();
    if !e.device_linking_works().await {
        return skipped("the devices package (a session that carries its device)");
    }
    let kim = e.user("kim");
    let cookie = e.login("kim").await;
    let dev = e.device(kim, Some("UTC"));
    e.link_session(&cookie, dev);

    let mut cases: Vec<(&str, Value)> = Vec::new();
    let mut with = |label: &'static str, f: &dyn Fn(&mut Value)| {
        let mut b = saved_body(KEY1);
        f(&mut b);
        cases.push((label, b));
    };
    with("key upper case", &|b| b["key"] = json!(KEY1.to_uppercase()));
    with("key too short", &|b| b["key"] = json!("abc"));
    with("key not a string", &|b| b["key"] = json!(5));
    with("kind unknown", &|b| b["kind"] = json!("quiz"));
    with("total zero", &|b| b["total"] = json!(0));
    with("total too big", &|b| b["total"] = json!(100_001));
    with("total a float", &|b| b["total"] = json!(10.5));
    with("answered negative", &|b| b["answered"] = json!(-1));
    with("answered above total", &|b| b["answered"] = json!(11));
    with("answered a bool", &|b| b["answered"] = json!(true));
    with("startedAt zero", &|b| b["startedAt"] = json!(0));
    with("startedAt too big", &|b| {
        b["startedAt"] = json!(1_i64 << 41)
    });
    with("timeLeft with practice", &|b| b["timeLeft"] = json!(100));
    with("timeLeft too big", &|b| {
        b["kind"] = json!("exam");
        b["timeLeft"] = json!(36_001);
    });
    with("timeLeft negative", &|b| {
        b["kind"] = json!("exam");
        b["timeLeft"] = json!(-1);
    });
    with("timeLeft a string", &|b| {
        b["kind"] = json!("exam");
        b["timeLeft"] = json!("5");
    });
    with("key missing", &|b| {
        b.as_object_mut().expect("object").remove("key");
    });
    for (label, body) in cases {
        let r = e.post("/api/study/saved", &cookie, body).await;
        assert_eq!(r.status, 400, "{label}: {}", r.text());
        assert_eq!(r.error(), MSG_BAD_REQUEST, "{label}");
    }
    assert_eq!(marker_count(&e), 0, "nothing was stored");

    // Valid ones
    let r = e.post("/api/study/saved", &cookie, saved_body(KEY1)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json(), json!({"ok": true}));
    let m = marker(&e, kim, KEY1).expect("marker");
    assert_eq!(
        (m.0, m.1.as_str(), m.2, m.3, m.4),
        (Some(dev), "practice", 2, 10, None)
    );
    let mut exam = saved_body(KEY2);
    exam["kind"] = json!("exam");
    exam["timeLeft"] = json!(36_000);
    let r = e.post("/api/study/saved", &cookie, exam).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(marker(&e, kim, KEY2).expect("marker").4, Some(36_000));
    // `timeLeft` may be left out
    let mut b = saved_body("22222222222222222222222222222222");
    b.as_object_mut().expect("object").remove("timeLeft");
    assert_eq!(e.post("/api/study/saved", &cookie, b).await.status, 200);
    assert_eq!(marker_count(&e), 1, "one saved session per browser");

    // Discard
    let r = e
        .post(
            "/api/study/saved/discard",
            &cookie,
            json!({"key": "22222222222222222222222222222222"}),
        )
        .await;
    assert_eq!((r.status.as_u16(), r.json()), (200, json!({"ok": true})));
    assert_eq!(marker_count(&e), 0);
    let r = e
        .post(
            "/api/study/saved/discard",
            &cookie,
            json!({"key": "22222222222222222222222222222222"}),
        )
        .await;
    assert_eq!(r.status, 200, "no marker is not an error");
    let r = e
        .post("/api/study/saved/discard", &cookie, json!({"key": "zz"}))
        .await;
    assert_eq!(r.status, 400);
}

#[tokio::test]
async fn the_saved_routes_need_a_session_and_are_not_logged() {
    let e = Env::new();
    let r = e
        .call(
            "POST",
            "/api/study/saved",
            None,
            Some(saved_body(KEY1).to_string()),
        )
        .await;
    assert_eq!(r.status, 401, "{}", r.text());
    let r = e
        .call(
            "POST",
            "/api/study/saved/discard",
            None,
            Some(json!({"key": KEY1}).to_string()),
        )
        .await;
    assert_eq!(r.status, 401, "{}", r.text());
    if !e.device_linking_works().await {
        return skipped("the devices package (a session that carries its device)");
    }
    let kim = e.user("kim");
    let cookie = e.login("kim").await;
    let dev = e.device(kim, Some("UTC"));
    e.link_session(&cookie, dev);
    e.post("/api/study/saved", &cookie, saved_body(KEY1)).await;
    e.post("/api/study/saved/discard", &cookie, json!({"key": KEY1}))
        .await;
    // ROUTE_ACTIONS says "" for both: no activity row
    assert!(e.entries("study.saved").await.is_empty());
    let rows: i64 = e.int("SELECT COUNT(*) FROM activity_log WHERE action LIKE '%saved%'");
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn a_disabled_account_cannot_save() {
    let e = Env::new();
    if !e.device_linking_works().await {
        return skipped("the devices package (a session that carries its device)");
    }
    let kim = e.user("kim");
    let cookie = e.login("kim").await;
    let dev = e.device(kim, Some("UTC"));
    e.link_session(&cookie, dev);
    e.exec("UPDATE users SET disabled = 1 WHERE id = ?1", params![kim]);
    let r = e.post("/api/study/saved", &cookie, saved_body(KEY1)).await;
    assert_eq!(r.status, 401, "{}", r.text());
    assert_eq!(marker_count(&e), 0);
}

#[tokio::test]
async fn a_recorded_attempt_clears_the_marker_of_its_session() {
    let e = Env::new();
    if !e.device_linking_works().await {
        return skipped("the devices package (a session that carries its device)");
    }
    let kim = e.user("kim");
    let bank = e.import_new(None, "Herbs", 2);
    e.grant(kim, bank);
    let cookie = e.login("kim").await;
    let dev = e.device(kim, Some("UTC"));
    e.link_session(&cookie, dev);
    let qid = xexam_backend::database::scoped_id(bank, "Herbs-0");
    let attempt = |key: &str| {
        json!({
            "results": [{"id": qid, "correct": true, "answer": ["A"]}],
            "sheets": ["Herbs"], "mode": "all", "seconds": 30, "kind": "practice",
            "key": key, "startedAt": 1_780_000_000
        })
    };
    let r = e.post("/api/study/saved", &cookie, saved_body(KEY1)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    // An attempt of another session leaves it alone; a refused attempt does too
    let r = e.post("/api/attempt", &cookie, attempt(KEY2)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(marker(&e, kim, KEY1).is_some());
    let mut bad = attempt(KEY1);
    bad["results"] = json!("nope");
    assert_eq!(e.post("/api/attempt", &cookie, bad).await.status, 400);
    assert!(marker(&e, kim, KEY1).is_some());
    // The attempt of the saved session clears it
    let r = e.post("/api/attempt", &cookie, attempt(KEY1)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(marker(&e, kim, KEY1), None);
    // The reminder tick of a cleared marker has nothing to do
    assert_eq!(
        e.store.push_remind_tick(e.now() + 10 * DAY).expect("tick"),
        0
    );
}
