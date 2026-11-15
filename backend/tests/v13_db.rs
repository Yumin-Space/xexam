//! v1.3, main database schema 8 (builder BE1): the migration from 7, sessions (read-only lookup, touch, purge), the sign-in
//! info switch, the activity log (merging, filters, retention, clearing), the system settings, the idempotent attempt save,
//! `/api/bank` reading only active rows, and the audit of the library stamp behind the ETag. Made-up data only.
#[path = "common/mod.rs"]
mod dbc;

use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use xexam_backend::auth::Role;
use xexam_backend::database::{
    scoped_id, ActivityFilter, ActivityRow, ActivitySource, FixedClock, Merge, NewSession,
    SigninInfo, Store, Touch, UserChange, UserFilter,
};
use xexam_backend::models::{
    Attempt, AttemptKind, AttemptResult, BankAccess, BankScope, ImportTarget, SessionTag,
};

const NOW: i64 = 1_794_000_000;

fn open(db: &Path) -> Store {
    Store::open_with_clock(db, Arc::new(FixedClock("2026-11-14 07:20".to_string()))).expect("open")
}

fn raw(p: &Path) -> Connection {
    Connection::open(p).expect("raw open")
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).expect("count")
}

fn text(c: &Connection, sql: &str) -> String {
    use rusqlite::types::Value as V;
    match c.query_row(sql, [], |r| r.get::<_, V>(0)).expect("text") {
        V::Text(t) => t,
        V::Integer(i) => i.to_string(),
        other => format!("{other:?}"),
    }
}

fn user(s: &Store, name: &str, role: Role) -> i64 {
    match s
        .create_user_opts(
            name,
            "placeholder-hash",
            role,
            &BankAccess::All,
            true,
            false,
            false,
        )
        .expect("create")
    {
        UserChange::Done(id) => id,
        other => panic!("{other:?}"),
    }
}

/// Everything the schema-8 step added, removed again: a fresh database becomes a schema-7 one.
const DOWNGRADE_TO_7: &str = "DROP INDEX idx_attempts_session; DROP TABLE activity_log; DROP TABLE system_settings; \
    ALTER TABLE users DROP COLUMN last_active_ip; ALTER TABLE users DROP COLUMN last_active_country; ALTER TABLE users DROP COLUMN last_active_region; ALTER TABLE users DROP COLUMN last_active_city; ALTER TABLE users DROP COLUMN last_active_ua; ALTER TABLE users DROP COLUMN last_active_at; ALTER TABLE users DROP COLUMN last_login_ip; \
    ALTER TABLE users DROP COLUMN last_login_country; ALTER TABLE users DROP COLUMN last_login_region; \
    ALTER TABLE users DROP COLUMN last_login_city; ALTER TABLE sessions DROP COLUMN started_at; \
    ALTER TABLE sessions DROP COLUMN remember; ALTER TABLE sessions DROP COLUMN ip; \
    ALTER TABLE sessions DROP COLUMN ua; UPDATE meta SET value = '7' WHERE key = 'schema';";

fn master(p: &Path) -> Vec<(String, String, Option<String>)> {
    let c = raw(p);
    let mut st = c
        .prepare("SELECT type, name, sql FROM sqlite_master ORDER BY rowid")
        .expect("prepare");
    let rows = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query");
    rows.map(|r| r.expect("row")).collect()
}

// ---------------------------------------------------------------- migration

#[test]
fn a_v7_file_migrates_to_v8_with_a_backup_and_the_backfill() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let fresh = dir.path().join("fresh").join("xexam.db");
    drop(open(&fresh));
    {
        let s = open(&db);
        let id = user(&s, "ann", Role::User);
        s.create_session(&NewSession {
            token_hash: "tok1",
            user_id: id,
            now: NOW,
            expires_at: NOW + 100,
            remember: true,
            ip: None,
            ua: None,
        })
        .expect("session");
        s.note_signin(id, "2026-11-01 08:00", None).expect("note");
    }
    {
        let c = raw(&db);
        c.execute_batch(DOWNGRADE_TO_7).expect("downgrade");
        // a fresh database has a backup-less open; this one now has data
        c.execute(
            "UPDATE users SET last_login_at = '2026-11-01 08:00' WHERE username = 'ann'",
            [],
        )
        .expect("login");
    }
    let s = open(&db);
    let backup = s
        .migration_backup()
        .expect("a backup was written")
        .to_path_buf();
    let name = backup.file_name().and_then(|n| n.to_str()).expect("name");
    assert!(name.starts_with("xexam-schema7-"), "{name}");
    assert_eq!(
        text(&raw(&backup), "SELECT value FROM meta WHERE key='schema'"),
        "7"
    );
    drop(s);
    let c = raw(&db);
    assert_eq!(text(&c, "SELECT value FROM meta WHERE key='schema'"), "9");
    // the backfill
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM sessions WHERE started_at IS NULL"),
        0
    );
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM sessions WHERE remember = 1 AND ip IS NULL AND ua IS NULL"
        ),
        1
    );
    assert_eq!(
        text(&c, "SELECT last_active_at FROM users WHERE username='ann'"),
        "2026-11-01 08:00:00"
    );
    // the first row of the log
    assert_eq!(count(&c, "SELECT COUNT(*) FROM activity_log"), 1);
    assert_eq!(
        text(
            &c,
            "SELECT actor || '|' || action || '|' || detail || '|' || result FROM activity_log"
        ),
        "system|system.migrate|{\"from\":7,\"to\":9}|ok"
    );
    // fresh == migrated
    assert_eq!(master(&db), master(&fresh));
    // nothing to do on the next open
    let before = dbc::db_bytes(&db);
    drop(open(&db));
    drop(c);
    assert_eq!(count(&raw(&db), "SELECT COUNT(*) FROM activity_log"), 1);
    let _ = before;
}

#[test]
fn a_fresh_database_has_no_migrate_row_and_no_ip_anywhere() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = open(&db);
    assert!(s.migration_backup().is_none());
    drop(s);
    let c = raw(&db);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM activity_log"), 0);
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM users WHERE last_login_ip IS NOT NULL"
        ),
        0
    );
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM sessions WHERE ip IS NOT NULL"),
        0
    );
}

// ---------------------------------------------------------------- sessions

#[test]
fn a_v12_session_row_is_read_and_fixed_by_the_first_touch() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = open(&db);
    let id = user(&s, "ann", Role::User);
    // what a v1.2 binary inserts: no new columns
    raw(&db)
        .execute(
            "INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at) VALUES('old', ?1, 'x', 1000, 5000)",
            params![id],
        )
        .expect("insert");
    let (u, row) = s.session_user_ro("old").expect("lookup").expect("found");
    assert_eq!(u.username, "ann");
    assert_eq!(
        (
            row.started_at,
            row.last_seen_at,
            row.expires_at,
            row.remember
        ),
        (None, 1000, 5000, true)
    );
    assert_eq!((row.ip, row.ua), (None, None));
    // a read writes nothing
    assert_eq!(
        count(
            &raw(&db),
            "SELECT COUNT(*) FROM sessions WHERE started_at IS NULL"
        ),
        1
    );
    s.touch_session(
        "old",
        id,
        2000,
        &Touch {
            session: true,
            active: false,
            expires_at: 9000,
        },
    )
    .expect("touch");
    let (_, row) = s.session_user_ro("old").expect("lookup").expect("found");
    assert_eq!(
        (row.started_at, row.last_seen_at, row.expires_at),
        (Some(1000), 2000, 9000),
        "started_at = the old last_seen_at"
    );
    // an expired row is still returned (BE2 applies the role-based deadline)
    s.touch_session(
        "old",
        id,
        2001,
        &Touch {
            session: true,
            active: false,
            expires_at: 5,
        },
    )
    .expect("touch");
    assert!(s.session_user_ro("old").expect("lookup").is_some());
    assert!(s.session_user_ro("nope").expect("lookup").is_none());
    // a disabled user has no session
    raw(&db)
        .execute("UPDATE users SET disabled=1 WHERE id=?1", params![id])
        .expect("disable");
    assert!(s.session_user_ro("old").expect("lookup").is_none());
}

#[test]
fn create_touch_purge_and_the_cap_of_sessions() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = open(&db);
    let id = user(&s, "ann", Role::User);
    for i in 0..22 {
        let t = format!("t{i:02}");
        s.create_session(&NewSession {
            token_hash: &t,
            user_id: id,
            now: NOW,
            expires_at: NOW + 1000 + i,
            remember: i % 2 == 0,
            ip: Some("203.0.113.7"),
            ua: Some("Mozilla/5.0 test"),
        })
        .expect("create");
    }
    let c = raw(&db);
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM sessions"),
        20,
        "at most 20 per user"
    );
    assert!(
        s.session_user_ro("t00").expect("lookup").is_none(),
        "the ones expiring first went"
    );
    let (_, row) = s.session_user_ro("t21").expect("lookup").expect("found");
    assert_eq!(
        (
            row.started_at,
            row.remember,
            row.ip.as_deref(),
            row.ua.as_deref()
        ),
        (
            Some(NOW),
            false,
            Some("203.0.113.7"),
            Some("Mozilla/5.0 test")
        )
    );
    // nothing written when both gates say no
    s.touch_session(
        "t21",
        id,
        NOW + 5,
        &Touch {
            session: false,
            active: false,
            expires_at: 1,
        },
    )
    .expect("noop");
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM sessions WHERE last_seen_at <> ?1"
                .replace("?1", &NOW.to_string())
                .as_str()
        ),
        0
    );
    // the active minute only touches the user
    s.touch_session(
        "t21",
        id,
        NOW + 61,
        &Touch {
            session: false,
            active: true,
            expires_at: 1,
        },
    )
    .expect("active");
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM sessions WHERE last_seen_at <> ?1"
                .replace("?1", &NOW.to_string())
                .as_str()
        ),
        0
    );
    assert_eq!(
        text(&c, "SELECT length(last_active_at) FROM users WHERE id=2"),
        "19"
    );
    // purge returns what it deleted
    let ended = s.purge_expired_sessions_ended(NOW + 1010).expect("purge");
    assert_eq!(
        ended.len(),
        8,
        "t02 ..= t09 expire before now + 1010; t10 stays"
    );
    assert!(ended.iter().all(|e| e.user_id == id
        && e.started_at == Some(NOW)
        && e.ip.as_deref() == Some("203.0.113.7")));
    assert_eq!(count(&c, "SELECT COUNT(*) FROM sessions"), 12);
    s.delete_session("t21").expect("delete");
    assert!(s.session_user_ro("t21").expect("lookup").is_none());
}

// ---------------------------------------------------------------- sign-in info

#[test]
fn the_signin_info_switch_only_hides_and_the_clear_wipes() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = open(&db);
    let id = user(&s, "ann", Role::User);
    let info = SigninInfo {
        ip: Some("203.0.113.7"),
        country: Some("ZZ"),
        region: Some("Example Region"),
        city: Some("Example City"),
    };
    // off (the default): the API answers null
    s.note_signin(id, "2026-11-14 07:20", None).expect("note");
    let v = s.users().expect("users");
    assert_eq!(v["signinInfo"], false);
    let ann = &v["users"][1];
    assert_eq!(
        (
            &ann["lastLoginIp"],
            &ann["lastLoginLocation"],
            &ann["lastLoginAt"],
            &ann["lastActiveAt"]
        ),
        (
            &Value::Null,
            &Value::Null,
            &json!("2026-11-14 07:20"),
            &json!("2026-11-14 07:20")
        )
    );
    // a stored value is never shown while the switch is off
    s.note_signin(id, "2026-11-14 07:21", Some(&info))
        .expect("note");
    assert_eq!(
        s.users().expect("users")["users"][1]["lastLoginIp"],
        Value::Null
    );
    // on
    s.set_signin_info(true, Some(1)).expect("on");
    assert!(s.signin_info_enabled().expect("flag"));
    let ann = s.users().expect("users")["users"][1].clone();
    assert_eq!(ann["lastLoginIp"], "203.0.113.7");
    assert_eq!(
        ann["lastLoginLocation"],
        json!({"country": "ZZ", "region": "Example Region", "city": "Example City"})
    );
    // a later sign-in with the country only replaces all fields (no mixing of two sign-ins)
    s.note_signin(
        id,
        "2026-11-14 08:00",
        Some(&SigninInfo {
            ip: Some("203.0.113.8"),
            country: Some("ZZ"),
            region: None,
            city: None,
        }),
    )
    .expect("note");
    let ann = s.users().expect("users")["users"][1].clone();
    assert_eq!(
        ann["lastLoginLocation"],
        json!({"country": "ZZ", "region": null, "city": null})
    );
    // sessions and log rows with an address
    s.create_session(&NewSession {
        token_hash: "a",
        user_id: id,
        now: NOW,
        expires_at: NOW + 10,
        remember: true,
        ip: Some("203.0.113.8"),
        ua: Some("ua"),
    })
    .expect("session");
    s.activity_insert(&[row(NOW, "auth.signin", Some("ip"))])
        .expect("log");
    // off again: display only, nothing stored is touched
    s.set_signin_info(false, Some(1)).expect("off");
    const USERS_WITH_INFO: &str = "SELECT COUNT(*) FROM users WHERE last_login_ip IS NOT NULL OR last_login_country IS NOT NULL OR last_login_region IS NOT NULL OR last_login_city IS NOT NULL";
    const LOG_WITH_INFO: &str = "SELECT COUNT(*) FROM activity_log WHERE ip IS NOT NULL OR country IS NOT NULL OR region IS NOT NULL OR city IS NOT NULL";
    let c = raw(&db);
    assert_eq!(count(&c, USERS_WITH_INFO), 1);
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM sessions WHERE ip IS NOT NULL"),
        1
    );
    assert_eq!(count(&c, LOG_WITH_INFO), 1);
    // hidden on read while off: the users answer and the log page
    let v = s.users().expect("users");
    assert_eq!(v["signinInfo"], false);
    assert_eq!(v["users"][1]["lastLoginIp"], Value::Null);
    assert_eq!(v["users"][1]["lastLoginLocation"], Value::Null);
    let page = s.activity_page(&ActivityFilter::default()).expect("page");
    assert!(!page.signin_info);
    assert!(page
        .entries
        .iter()
        .all(|e| e.ip.is_none() && e.country.is_none()));
    // a sign-in while off is recorded, and shown once the switch is on again
    s.note_signin(id, "2026-11-14 09:00", Some(&info))
        .expect("note");
    s.set_signin_info(true, Some(1)).expect("on");
    assert_eq!(
        s.users().expect("users")["users"][1]["lastLoginIp"],
        "203.0.113.7"
    );
    let page = s.activity_page(&ActivityFilter::default()).expect("page");
    assert!(page.signin_info);
    assert!(page.entries.iter().any(|e| e.ip.is_some()));
    // the explicit clear: users and sessions in one step, the log in batches; the device and the switch stay
    assert_eq!(s.clear_signin_info().expect("clear"), 2);
    assert_eq!(s.clear_signin_info().expect("clear"), 0);
    assert_eq!(s.clear_activity_ip(100).expect("clear"), 1);
    assert_eq!(s.clear_activity_ip(100).expect("clear"), 0);
    let c = raw(&db);
    assert_eq!(count(&c, USERS_WITH_INFO), 0);
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM sessions WHERE ip IS NOT NULL"),
        0
    );
    assert_eq!(count(&c, LOG_WITH_INFO), 0);
    assert_eq!(
        text(&c, "SELECT ua FROM sessions"),
        "ua",
        "the device stays"
    );
    assert_eq!(
        text(
            &c,
            "SELECT value FROM system_settings WHERE key='users.signin_info'"
        ),
        "1"
    );
    let ann = s.users().expect("users")["users"][1].clone();
    assert_eq!(ann["lastLoginIp"], Value::Null);
    assert_eq!(ann["lastLoginAt"], "2026-11-14 09:00");
}

// ---------------------------------------------------------------- activity log

fn row(ts: i64, action: &str, object: Option<&str>) -> ActivityRow {
    ActivityRow {
        ts,
        actor: "user".into(),
        user_id: Some(2),
        username: Some("ann".into()),
        role: Some("user".into()),
        action: action.into(),
        object: object.map(str::to_string),
        detail: None,
        ok: true,
        reason: None,
        ip: Some("203.0.113.7".into()),
        country: Some("ZZ".into()),
        region: Some("Example Region".into()),
        city: Some("Example City".into()),
        ua: Some("Mozilla/5.0 test".into()),
        source: ActivitySource::Backend,
        src_id: None,
        merge: Merge::None,
    }
}

fn visit(ts: i64, page: &str) -> ActivityRow {
    ActivityRow {
        merge: Merge::PageView,
        ..row(ts, "page.view", Some(page))
    }
}

#[test]
fn page_visits_merge_within_30_minutes_and_stop_after_24_hours() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = open(&db);
    s.activity_insert(&[
        visit(NOW, "home"),
        visit(NOW + 60, "home"),
        visit(NOW + 120, "banks"),
    ])
    .expect("log");
    let c = raw(&db);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM activity_log"), 2);
    assert_eq!(
        text(
            &c,
            "SELECT count || '/' || (last_ts - ts) FROM activity_log WHERE object='home'"
        ),
        "2/60"
    );
    // 29 minutes after the last one: still the same row; 31 minutes after that: a new row
    s.activity_insert(&[visit(NOW + 60 + 1740, "home")])
        .expect("log");
    assert_eq!(
        text(&c, "SELECT count FROM activity_log WHERE object='home'"),
        "3"
    );
    s.activity_insert(&[visit(NOW + 60 + 1740 + 1900, "home")])
        .expect("log");
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM activity_log WHERE object='home'"),
        2
    );
    // a row stops merging 24 h after its own ts, however busy it was: visits every 25 minutes
    let mut t = NOW + 100_000;
    s.activity_insert(&[visit(t, "records")]).expect("log");
    for _ in 0..70 {
        t += 1500;
        s.activity_insert(&[visit(t, "records")]).expect("log");
    }
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM activity_log WHERE object='records'"
        ),
        2
    );
    assert_eq!(
        count(
            &c,
            "SELECT MAX(last_ts - ts) < 86400 FROM activity_log WHERE object='records'"
        ),
        1
    );
    // another user's visit never merges
    let other = ActivityRow {
        user_id: Some(3),
        ..visit(NOW + 100_100, "records")
    };
    s.activity_insert(&[other]).expect("log");
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM activity_log WHERE object='records'"
        ),
        3
    );
}

#[test]
fn refused_signins_merge_within_60_seconds_per_target_and_address() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = open(&db);
    let refused = |ts: i64, object: &str, ip: Option<&str>| ActivityRow {
        actor: "anon".into(),
        user_id: None,
        username: None,
        role: None,
        ok: false,
        reason: Some("locked".into()),
        ip: ip.map(str::to_string),
        merge: Merge::Refused,
        ..row(ts, "auth.signin", Some(object))
    };
    s.activity_insert(&[
        refused(NOW, "unknown", Some("203.0.113.7")),
        refused(NOW + 30, "unknown", Some("203.0.113.7")),
        refused(NOW + 59, "unknown", Some("203.0.113.7")),
        refused(NOW + 59, "unknown", Some("203.0.113.8")),
        refused(NOW + 59, "ann", Some("203.0.113.7")),
        refused(NOW + 200, "unknown", Some("203.0.113.7")),
    ])
    .expect("log");
    let c = raw(&db);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM activity_log"), 4);
    assert_eq!(
        text(
            &c,
            "SELECT count FROM activity_log WHERE ts = ?1"
                .replace("?1", &NOW.to_string())
                .as_str()
        ),
        "3"
    );
    // without an address (the switch is off) the same rule applies to rows without one
    s.activity_insert(&[refused(NOW + 300, "x", None), refused(NOW + 310, "x", None)])
        .expect("log");
    assert_eq!(
        text(&c, "SELECT count FROM activity_log WHERE object='x'"),
        "2"
    );
}

#[test]
fn refused_signins_of_different_users_or_reasons_do_not_merge() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = open(&db);
    let refused = |ts: i64, uid: i64, name: &str, reason: &str| ActivityRow {
        actor: "anon".into(),
        user_id: Some(uid),
        username: Some(name.into()),
        role: None,
        ok: false,
        reason: Some(reason.into()),
        ip: None,
        merge: Merge::Refused,
        ..row(ts, "auth.signin", None)
    };
    s.activity_insert(&[
        refused(NOW, 1, "ann", "locked"),
        refused(NOW + 1, 2, "bob", "locked"),
        refused(NOW + 2, 1, "ann", "disabled"),
        refused(NOW + 3, 1, "ann", "locked"),
    ])
    .expect("log");
    let c = raw(&db);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM activity_log"), 3);
    assert_eq!(
        text(
            &c,
            "SELECT count FROM activity_log WHERE username='ann' AND reason='locked'"
        ),
        "2"
    );
}

#[test]
fn ai_events_are_idempotent_and_cut_to_their_limits() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = open(&db);
    let ev = ActivityRow {
        source: ActivitySource::Ai,
        src_id: Some("a".repeat(32)),
        object: Some("o".repeat(500)),
        ua: Some("u".repeat(500)),
        detail: Some(format!("{{\"x\":\"{}\"}}", "d".repeat(3000))),
        ..row(NOW, "ai.settings.enable", None)
    };
    s.activity_insert(&[ev.clone(), ev.clone()]).expect("log");
    s.activity_insert(&[ev]).expect("redelivered");
    let c = raw(&db);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM activity_log"), 1);
    assert_eq!(count(&c, "SELECT length(object) = 200 AND length(ua) = 256 AND detail IS NULL AND source = 'ai' FROM activity_log"), 1);
}

#[test]
fn the_page_filters_paginates_and_hides_visits_by_default() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = open(&db);
    s.set_signin_info(true, None).expect("on");
    let sys = ActivityRow {
        actor: "system".into(),
        user_id: None,
        username: None,
        role: None,
        ip: None,
        country: None,
        region: None,
        city: None,
        ..row(NOW, "system.start", None)
    };
    let fail = ActivityRow {
        ok: false,
        reason: Some("badPassword".into()),
        ..row(NOW + 2, "auth.signin", None)
    };
    let mut rows = vec![
        sys,
        row(NOW + 1, "auth.signin", None),
        fail,
        row(NOW + 3, "users.create", Some("bob")),
        visit(NOW + 4, "home"),
    ];
    rows.push(ActivityRow {
        user_id: Some(3),
        username: Some("gone".into()),
        ..row(NOW + 5, "bank.delete", Some("b"))
    });
    rows.push(ActivityRow {
        source: ActivitySource::Ai,
        src_id: Some("b".repeat(32)),
        ..row(NOW + 6, "ai.gen.create", Some("bank"))
    });
    s.activity_insert(&rows).expect("log");
    let all = s.activity_page(&ActivityFilter::default()).expect("page");
    let ids: Vec<&str> = all.entries.iter().map(|e| e.action.as_str()).collect();
    assert_eq!(
        ids,
        [
            "ai.gen.create",
            "bank.delete",
            "users.create",
            "auth.signin",
            "auth.signin",
            "system.start"
        ],
        "newest first, no visits"
    );
    assert_eq!(all.next, None);
    assert_eq!(all.newest, Some(all.entries[0].id));
    // visits only with their group or action
    let v = s
        .activity_page(&ActivityFilter {
            group: Some("visit".into()),
            ..Default::default()
        })
        .expect("page");
    assert_eq!(v.entries.len(), 1);
    let v = s
        .activity_page(&ActivityFilter {
            action: Some("page.view".into()),
            ..Default::default()
        })
        .expect("page");
    assert_eq!(v.entries.len(), 1);
    // group, result, user
    let g = |group: &str| {
        s.activity_page(&ActivityFilter {
            group: Some(group.into()),
            ..Default::default()
        })
        .expect("page")
        .entries
        .len()
    };
    assert_eq!(
        (
            g("auth"),
            g("users"),
            g("banks"),
            g("ai"),
            g("system"),
            g("study"),
            g("nope")
        ),
        (2, 1, 1, 1, 1, 0, 0)
    );
    let f = s
        .activity_page(&ActivityFilter {
            ok: Some(false),
            ..Default::default()
        })
        .expect("page");
    assert_eq!(f.entries.len(), 1);
    assert_eq!(f.entries[0].reason.as_deref(), Some("badPassword"));
    let u = s
        .activity_page(&ActivityFilter {
            user: Some(UserFilter::Id(3)),
            ..Default::default()
        })
        .expect("page");
    assert_eq!(u.entries.len(), 1);
    assert!(u.entries[0].deleted, "user 3 does not exist");
    let sysf = s
        .activity_page(&ActivityFilter {
            user: Some(UserFilter::System),
            ..Default::default()
        })
        .expect("page");
    assert_eq!(sysf.entries.len(), 1);
    let anon = s
        .activity_page(&ActivityFilter {
            user: Some(UserFilter::Anon),
            ..Default::default()
        })
        .expect("page");
    assert!(anon.entries.is_empty());
    // keyset: pages of 2, `next` is the last id, `after` reads what is newer
    let p1 = s
        .activity_page(&ActivityFilter {
            limit: Some(2),
            ..Default::default()
        })
        .expect("page");
    assert_eq!(p1.entries.len(), 2);
    assert_eq!(p1.next, Some(p1.entries[1].id));
    let p2 = s
        .activity_page(&ActivityFilter {
            limit: Some(2),
            before: p1.next,
            ..Default::default()
        })
        .expect("page");
    assert_eq!(p2.entries[0].action, "users.create");
    let last = s
        .activity_page(&ActivityFilter {
            limit: Some(2),
            before: p2.next,
            ..Default::default()
        })
        .expect("page");
    assert_eq!(last.entries.len(), 2);
    assert_eq!(last.next, None);
    let newer = s
        .activity_page(&ActivityFilter {
            after: Some(all.entries[2].id),
            ..Default::default()
        })
        .expect("page");
    assert_eq!(newer.entries.len(), 2);
    let none = s
        .activity_page(&ActivityFilter {
            after: all.newest,
            ..Default::default()
        })
        .expect("page");
    assert!(none.entries.is_empty());
    assert_eq!(none.newest, all.newest, "newest stays the sent after");
    // the entry as JSON (ip and location while the switch is on), AI source
    let first = all.entries[0].to_json();
    assert_eq!(first["source"], "ai");
    assert_eq!(first["ip"], "203.0.113.7");
    assert_eq!(
        first["location"],
        json!({"country": "ZZ", "region": "Example Region", "city": "Example City"})
    );
    assert_eq!(first["result"], "ok");
    assert_eq!(all.entries[5].to_json()["location"], Value::Null);
    assert_eq!(first["time"].as_str().map(str::len), Some(19));
}

#[test]
fn facets_list_users_with_their_current_or_snapshot_name() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = open(&db);
    let ann = user(&s, "ann", Role::User); // id 2
    assert_eq!(ann, 2);
    s.activity_insert(&[
        row(NOW, "auth.signin", None),
        ActivityRow {
            user_id: Some(9),
            username: Some("Gone".into()),
            ..row(NOW + 1, "auth.signin", None)
        },
        ActivityRow {
            user_id: Some(9),
            username: Some("gone-new".into()),
            ..row(NOW + 2, "auth.signout", None)
        },
    ])
    .expect("log");
    let f = s.activity_facets().expect("facets");
    let got: Vec<(i64, &str, bool)> = f
        .iter()
        .map(|u| (u.id, u.username.as_str(), u.deleted))
        .collect();
    assert_eq!(got, [(2, "ann", false), (9, "gone-new", true)]);
    let page = s
        .activity_page(&ActivityFilter {
            facets: true,
            ..Default::default()
        })
        .expect("page");
    assert_eq!(
        page.to_json()["facets"]["users"][1],
        json!({"id": 9, "username": "gone-new", "deleted": true})
    );
    assert!(s
        .activity_page(&ActivityFilter::default())
        .expect("page")
        .to_json()
        .get("facets")
        .is_none());
}

#[test]
fn retention_deletes_by_age_in_batches_then_by_cap() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = open(&db);
    {
        let c = raw(&db);
        // 5 200 old rows (more than one batch of 5 000), 30 recent ones
        c.execute_batch("BEGIN").expect("begin");
        for i in 0..5200 {
            c.execute(
                "INSERT INTO activity_log(ts, last_ts, actor, action, result) VALUES(?1, ?1, 'system', 'system.start', 'ok')",
                params![NOW - 200 * 86_400 + i],
            )
            .expect("old");
        }
        for i in 0..30 {
            c.execute(
                "INSERT INTO activity_log(ts, last_ts, actor, action, result) VALUES(?1, ?1, 'system', 'system.stop', 'ok')",
                params![NOW - 100 + i],
            )
            .expect("new");
        }
        c.execute_batch("COMMIT").expect("commit");
    }
    assert_eq!(
        s.activity_retention(NOW, 90, 500_000).expect("retention"),
        5200
    );
    let c = raw(&db);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM activity_log"), 30);
    // the cap removes the oldest rows beyond it
    assert_eq!(s.activity_retention(NOW, 90, 20).expect("cap"), 10);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM activity_log"), 20);
    assert_eq!(
        count(&c, "SELECT MIN(ts) FROM activity_log"),
        NOW - 100 + 10
    );
    assert_eq!(s.activity_retention(NOW, 90, 20).expect("again"), 0);
}

// ---------------------------------------------------------------- attempts

fn attempt(ids: &[(&str, bool)], kind: AttemptKind) -> Attempt {
    Attempt {
        results: ids
            .iter()
            .map(|(id, correct)| AttemptResult {
                id: id.to_string(),
                correct: *correct,
            })
            .collect(),
        sheets: vec!["Cooking".to_string()],
        mode: "all".into(),
        seconds: 9,
        kind,
        limit: None,
        end: None,
    }
}

#[test]
fn a_repeated_attempt_save_writes_nothing_and_returns_the_same_record() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = open(&db);
    dbc::seed_questions(&db, &["q1", "q2"]);
    let tag = SessionTag {
        key: Some("a".repeat(32)),
        started_at: Some(1_794_000_000),
    };
    let a = attempt(&[("q1", true), ("q2", false)], AttemptKind::Exam);
    let first = s
        .record_attempt_tagged(1, &BankScope::All, &a, &[], &tag)
        .expect("first");
    let id = first["recordId"].as_i64().expect("id");
    let c = raw(&db);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempts"), 1);
    let stats = text(&c, "SELECT group_concat(question_id || ':' || attempts, ',') FROM (SELECT * FROM user_stats ORDER BY question_id)");
    let again = s
        .record_attempt_tagged(1, &BankScope::All, &a, &[], &tag)
        .expect("retry");
    assert_eq!(again["recordId"], first["recordId"]);
    assert_eq!(again["duplicate"], true);
    assert!(first.get("duplicate").is_none());
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempts"), 1);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempt_items"), 2);
    assert_eq!(
        text(&c, "SELECT group_concat(question_id || ':' || attempts, ',') FROM (SELECT * FROM user_stats ORDER BY question_id)"),
        stats,
        "the statistics were not counted twice"
    );
    // another kind, another start time, another key, another user, or no tag: a new record
    let practice = attempt(&[("q1", true)], AttemptKind::Practice);
    assert_ne!(
        s.record_attempt_tagged(1, &BankScope::All, &practice, &[], &tag)
            .expect("kind")["recordId"],
        json!(id)
    );
    let later = SessionTag {
        started_at: Some(1_794_000_001),
        ..tag.clone()
    };
    assert_ne!(
        s.record_attempt_tagged(1, &BankScope::All, &a, &[], &later)
            .expect("start")["recordId"],
        json!(id)
    );
    let other_key = SessionTag {
        key: Some("b".repeat(32)),
        ..tag.clone()
    };
    assert_ne!(
        s.record_attempt_tagged(1, &BankScope::All, &a, &[], &other_key)
            .expect("key")["recordId"],
        json!(id)
    );
    let bob = user(&s, "bob", Role::User);
    assert_ne!(
        s.record_attempt_tagged(bob, &BankScope::All, &a, &[], &tag)
            .expect("user")["recordId"],
        json!(id)
    );
    s.record_attempt_tagged(1, &BankScope::All, &a, &[], &SessionTag::default())
        .expect("untagged");
    s.record_attempt_tagged(1, &BankScope::All, &a, &[], &SessionTag::default())
        .expect("untagged twice");
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempts"), 7);
}

// ---------------------------------------------------------------- /api/bank reads active rows only (B8)

fn import(s: &Store, file: &str, sheets: &[(&str, &[(&str, &str)])]) -> i64 {
    import_into(s, ImportTarget::New, file, sheets)
}

fn import_into(
    s: &Store,
    target: ImportTarget,
    file: &str,
    sheets: &[(&str, &[(&str, &str)])],
) -> i64 {
    let mut parsed = Vec::new();
    let mut qs = Vec::new();
    for (name, items) in sheets {
        parsed.push(dbc::sheet(name, i64::try_from(items.len()).unwrap_or(0)));
        for (i, (id, stem)) in items.iter().enumerate() {
            qs.push(dbc::single(
                name,
                id,
                stem,
                i64::try_from(i).unwrap_or(0) + 2,
            ));
        }
    }
    s.apply_import_for(target, file, &parsed, &qs)
        .expect("import")
        .expect("target")
        .1
}

#[test]
fn bank_lists_active_questions_and_counts_retired_ones_per_sheet() {
    let dir = dbc::tmp();
    let s = open(&dir.path().join("xexam.db"));
    let b1 = import(
        &s,
        "one.xlsx",
        &[
            ("Pans", &[("p1", "Which pan?"), ("p2", "Which lid?")]),
            ("Pots", &[("t1", "Which pot?")]),
        ],
    );
    // the re-import keeps only Pans: the Pots question and p2 are retired, Pots has only retired rows
    import_into(
        &s,
        ImportTarget::Bank(b1),
        "one.xlsx",
        &[("Pans", &[("p1", "Which pan?")])],
    );
    let b2 = import(&s, "two.xlsx", &[("Pans", &[("x1", "Which whisk?")])]);
    let v = s.bank().expect("bank");
    let qs: Vec<(&str, i64)> = v["questions"]
        .as_array()
        .expect("qs")
        .iter()
        .map(|q| {
            (
                q["id"].as_str().expect("id"),
                q["bank"].as_i64().expect("bank"),
            )
        })
        .collect();
    assert_eq!(qs.len(), 2, "{qs:?}");
    assert!(qs.contains(&(scoped_id(b1, "p1").as_str(), b1)), "{qs:?}");
    assert!(qs.contains(&(scoped_id(b2, "x1").as_str(), b2)), "{qs:?}");
    let sheets: Vec<(&str, i64, i64)> = v["sheets"]
        .as_array()
        .expect("sheets")
        .iter()
        .map(|x| {
            (
                x["name"].as_str().expect("n"),
                x["usable"].as_i64().expect("u"),
                x["inactive"].as_i64().expect("i"),
            )
        })
        .collect();
    // sheets merge by name across banks, in order of first appearance, retired-only sheets included
    assert_eq!(sheets, [("Pans", 2, 1), ("Pots", 0, 1)], "{v}");
    assert!(v["sheets"][1].get("total").is_some());
    // a limited viewer
    let only = s
        .bank_as(&BankScope::Only([b2].into_iter().collect()))
        .expect("bank");
    assert_eq!(only["questions"].as_array().map(Vec::len), Some(1));
    assert_eq!(only["sheets"].as_array().map(Vec::len), Some(1));
    let none = s
        .bank_as(&BankScope::Only(Default::default()))
        .expect("bank");
    assert_eq!(none["questions"], json!([]));
    assert_eq!(none["sheets"], json!([]));
}

// ---------------------------------------------------------------- library stamp audit (A4)

#[test]
fn the_library_stamp_changes_after_every_event_that_changes_the_bank_answer() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = open(&db);
    let admin = s.viewer(1).expect("viewer");
    let stamp = || s.library_stamp().expect("stamp");
    let mut seen = vec![stamp()];
    let mut step = |what: &str, now: String| {
        assert!(
            !seen.contains(&now),
            "the stamp did not change after: {what}"
        );
        seen.push(now);
    };
    assert_eq!(stamp(), stamp(), "stable across two reads");
    // import confirm: new bank
    let b1 = import(
        &s,
        "one.xlsx",
        &[
            ("Pans", &[("p1", "Which pan?"), ("p2", "Which lid?")]),
            ("Pots", &[("t1", "Which pot?")]),
        ],
    );
    step("a new bank", stamp());
    // re-import: the same sheets and counts, one question text changed
    import_into(
        &s,
        ImportTarget::Bank(b1),
        "one.xlsx",
        &[
            ("Pans", &[("p1", "Which pan now?"), ("p2", "Which lid?")]),
            ("Pots", &[("t1", "Which pot?")]),
        ],
    );
    step("a re-import with changed text and the same counts", stamp());
    // re-import that retires the Pots question
    import_into(
        &s,
        ImportTarget::Bank(b1),
        "one.xlsx",
        &[("Pans", &[("p1", "Which pan now?"), ("p2", "Which lid?")])],
    );
    step("a re-import that retires a sheet", stamp());
    // delete the retired-only sheet: the active count stays, the answer's sheet list changes
    assert!(s
        .delete_sheets_as(&admin, b1, &["Pots".to_string()], false)
        .expect("delete")
        .is_some());
    step("deleting a sheet that only has retired questions", stamp());
    // delete a sheet with active questions
    let b2 = import(
        &s,
        "two.xlsx",
        &[
            ("Whisks", &[("w1", "Which whisk?")]),
            ("Bowls", &[("o1", "Which bowl?")]),
        ],
    );
    step("a second bank", stamp());
    assert!(s
        .delete_sheets_as(&admin, b2, &["Bowls".to_string()], false)
        .expect("delete")
        .is_some());
    step("deleting a sheet with active questions", stamp());
    // retire old-layout questions (the questions of this test have no `lang`, so they are old-layout)
    assert!(!s.retire_old_layout_questions().expect("retire").is_empty());
    step("retiring old-layout questions", stamp());
    // delete a bank
    assert!(s.delete_bank(b2, false).expect("delete").is_some());
    step("deleting a bank", stamp());
}

#[test]
fn an_ai_bank_changes_the_stamp() {
    use xexam_backend::database::store::{AiBankOutcome, AiBankReq, AiQuestion, GenKind, Tri};
    let dir = dbc::tmp();
    let s = open(&dir.path().join("xexam.db"));
    let before = s.library_stamp().expect("stamp");
    let t = |x: &str| Tri {
        en: x.into(),
        zh: format!("{x} 中文"),
        ja: format!("{x} 日本語"),
    };
    let uid = s.user_by_id(1).expect("u").expect("admin").uid;
    let out = s
        .create_ai_bank(&AiBankReq {
            job: format!("{:032x}", 1),
            user: 1,
            stamp: uid,
            name: "AI pans".into(),
            kind: GenKind::Bank,
            from: vec![],
            goal: 1,
            at: "2026-11-12 09:30:00".into(),
            sheet: "AI".into(),
            questions: vec![AiQuestion {
                multi: false,
                stem: t("Which pan?"),
                options: vec![t("A1"), t("B1")],
                answer: vec!["B".into()],
                explanation: t("Fits."),
            }],
            file: None,
            skipped: vec![],
        })
        .expect("create");
    assert!(matches!(out, AiBankOutcome::Created { .. }), "{out:?}");
    assert_ne!(s.library_stamp().expect("stamp"), before);
}

#[test]
fn the_scoped_stamp_follows_access_role_and_what_the_viewer_may_see() {
    let dir = dbc::tmp();
    let s = open(&dir.path().join("xexam.db"));
    let b1 = import(&s, "one.xlsx", &[("Pans", &[("p1", "Which pan?")])]);
    let b2 = import(&s, "two.xlsx", &[("Pots", &[("t1", "Which pot?")])]);
    let ann = match s
        .create_user_opts(
            "ann",
            "placeholder-hash",
            Role::User,
            &BankAccess::Only(vec![b1]),
            false,
            false,
            false,
        )
        .expect("create")
    {
        UserChange::Done(id) => id,
        other => panic!("{other:?}"),
    };
    let stamp = |s: &Store| {
        s.library_stamp_as(&s.viewer(ann).expect("viewer"))
            .expect("stamp")
    };
    let a0 = stamp(&s);
    assert_eq!(a0, stamp(&s));
    // a change in a bank she cannot see leaves her stamp alone
    import_into(
        &s,
        ImportTarget::Bank(b2),
        "two.xlsx",
        &[("Pots", &[("t1", "Which pot now?")])],
    );
    assert_eq!(stamp(&s), a0);
    // a change in her bank, a sheet that only has retired rows removed, access granted, permission given
    import_into(
        &s,
        ImportTarget::Bank(b1),
        "one.xlsx",
        &[("Pans", &[("p1", "Which pan now?")])],
    );
    let a1 = stamp(&s);
    assert_ne!(a1, a0);
    import_into(
        &s,
        ImportTarget::Bank(b1),
        "one.xlsx",
        &[("Lids", &[("l1", "Which lid?")])],
    );
    let a2 = stamp(&s);
    assert_ne!(a2, a1);
    let admin = s.viewer(1).expect("viewer");
    assert!(s
        .delete_sheets_as(&admin, b1, &["Pans".to_string()], false)
        .expect("delete")
        .is_some());
    let a3 = stamp(&s);
    assert_ne!(a3, a2, "the retired-only sheet is gone");
    s.update_user(
        1,
        ann,
        &xexam_backend::database::UserUpdate {
            banks: Some(BankAccess::Only(vec![b1, b2])),
            ..Default::default()
        },
    )
    .expect("update");
    let a4 = stamp(&s);
    assert_ne!(a4, a3, "access changed");
    s.update_user(
        1,
        ann,
        &xexam_backend::database::UserUpdate {
            can_manage_banks: Some(true),
            ..Default::default()
        },
    )
    .expect("update");
    assert_ne!(stamp(&s), a4, "permission changed");
    // the viewer becoming an administrator
    s.update_user(
        1,
        ann,
        &xexam_backend::database::UserUpdate {
            role: Some(Role::Admin),
            ..Default::default()
        },
    )
    .expect("promote");
    let as_admin = stamp(&s);
    assert_eq!(
        as_admin,
        s.library_stamp_as(&s.viewer(1).expect("v")).expect("stamp"),
        "an administrator's stamp is the shared one"
    );
    s.update_user(
        1,
        ann,
        &xexam_backend::database::UserUpdate {
            role: Some(Role::User),
            ..Default::default()
        },
    )
    .expect("demote");
    assert_ne!(stamp(&s), as_admin, "no longer an administrator");
}

#[test]
fn the_banks_stamp_also_follows_study_records_owners_and_names() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = open(&db);
    let b1 = import(
        &s,
        "one.xlsx",
        &[("Pans", &[("p1", "Which pan?"), ("p2", "Which lid?")])],
    );
    let admin = s.viewer(1).expect("viewer");
    let ann = match s
        .create_user_opts(
            "ann",
            "placeholder-hash",
            Role::User,
            &BankAccess::Only(vec![b1]),
            false,
            true,
            false,
        )
        .expect("create")
    {
        UserChange::Done(id) => id,
        other => panic!("{other:?}"),
    };
    let ann_v = || s.viewer(ann).expect("viewer");
    let bs_admin = || s.banks_stamp_as(&admin).expect("stamp");
    let bs_ann = || s.banks_stamp_as(&ann_v()).expect("stamp");
    let (a0, n0) = (bs_admin(), bs_ann());
    assert_eq!((a0.clone(), n0.clone()), (bs_admin(), bs_ann()), "stable");
    let (p1, p2) = (scoped_id(b1, "p1"), scoped_id(b1, "p2"));
    // the library stamp does not move on a study record, the banks stamp does (studied / records numbers)
    let lib = s.library_stamp().expect("stamp");
    s.record_attempt_tagged(
        ann,
        &BankScope::All,
        &attempt(&[(&p1, true)], AttemptKind::Practice),
        &[],
        &SessionTag::default(),
    )
    .expect("record");
    assert_eq!(
        s.library_stamp().expect("stamp"),
        lib,
        "/api/bank keeps its 304 after a study record"
    );
    let (a1, n1) = (bs_admin(), bs_ann());
    assert_ne!(
        a1, a0,
        "an administrator's numbers count every user's records"
    );
    assert_ne!(n1, n0);
    // another user's record moves the administrator's numbers, not ann's
    let bob = user(&s, "bob", Role::User);
    s.record_attempt_tagged(
        bob,
        &BankScope::All,
        &attempt(&[(&p2, false)], AttemptKind::Exam),
        &[],
        &SessionTag::default(),
    )
    .expect("record");
    assert_ne!(bs_admin(), a1);
    assert_eq!(bs_ann(), n1);
    // wrong-answer statistics changing without a new record: a reset
    let (a2, n2) = (bs_admin(), bs_ann());
    s.reset_progress(ann).expect("reset");
    assert_ne!(bs_ann(), n2);
    assert_ne!(bs_admin(), a2);
    // an owner leaving: the bank moves to an administrator
    let b2 = s
        .apply_import_as(
            &ann_v(),
            ImportTarget::New,
            "mine.xlsx",
            &[dbc::sheet("Mine", 1)],
            &[dbc::single("Mine", "m1", "Which mug?", 2)],
        )
        .expect("import")
        .expect("target")
        .1;
    assert!(b2 > 0);
    let a3 = bs_admin();
    assert!(matches!(
        s.delete_user(1, ann).expect("delete"),
        UserChange::Done(_)
    ));
    assert_ne!(bs_admin(), a3, "the owner of the bank changed");
}
