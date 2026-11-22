//! v2.5: `GET /api/dashboard` (leaderboard + most practised). Made-up users, banks and questions only; e-mail-shaped sample
//! names use the reserved `example.invalid` domain.
#[path = "api_common/mod.rs"]
mod apic;
#[path = "common/mod.rs"]
mod dbc;

use apic::{build, send, Resp};
use axum::body::Body;
use axum::Router;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use xexam_backend::api::{build_router, AppState};
use xexam_backend::auth::{self, Role};
use xexam_backend::database::{FixedClock, FnClock, Store, UserChange};
use xexam_backend::logs::LogBuffer;
use xexam_backend::models::{
    Answer, Attempt, AttemptKind, AttemptResult, BankAccess, Kind, Question,
};
use xexam_backend::services::RealEngine;

const HOST: &str = "127.0.0.1:8765";
const NOW: &str = "2026-11-22 14:05";

struct App {
    router: Router,
    store: Arc<Store>,
    db: PathBuf,
    _dir: tempfile::TempDir,
}

fn app_with(clock: Arc<dyn xexam_backend::database::Clock>) -> App {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let store = Arc::new(Store::open_with_clock(&db, clock).expect("store"));
    let engine = Arc::new(RealEngine::new(store.clone()));
    let state = AppState::new_with_logs(
        apic::default_config(),
        store.clone(),
        engine,
        None,
        Arc::new(LogBuffer::default()),
    );
    App {
        router: build_router(state),
        store,
        db,
        _dir: dir,
    }
}

fn app() -> App {
    app_with(Arc::new(FixedClock(NOW.to_string())))
}

fn raw(p: &Path) -> Connection {
    Connection::open(p).expect("raw open")
}

struct Who {
    id: i64,
    cookie: String,
}

fn add_user(a: &App, name: &str, role: Role) -> i64 {
    match a
        .store
        .create_user_opts(
            name,
            "placeholder-hash",
            role,
            &BankAccess::Only(Vec::new()),
            false,
            false,
            false,
        )
        .expect("create")
    {
        UserChange::Done(id) => id,
        other => panic!("{other:?}"),
    }
}

fn start(a: &App, id: i64, clear_flag: bool) -> String {
    let u = a.store.user_by_id(id).expect("lookup").expect("user");
    if clear_flag {
        a.store
            .change_own_password(
                id,
                &u.password_hash,
                &u.password_hash,
                &auth::token_hash(&auth::new_session_token()),
                auth::unix_now(),
            )
            .expect("clear")
            .expect("user");
    }
    let token = auth::new_session_token();
    a.store
        .start_session(
            id,
            &auth::token_hash(&token),
            &u.password_hash,
            auth::unix_now(),
        )
        .expect("start")
        .expect("enabled");
    format!("{}={token}", auth::SESSION_COOKIE)
}

fn user(a: &App, name: &str) -> Who {
    let id = add_user(a, name, Role::User);
    Who {
        id,
        cookie: start(a, id, true),
    }
}

fn admin(a: &App) -> Who {
    Who {
        id: 1,
        cookie: start(a, 1, true),
    }
}

async fn dash(a: &App, cookie: &str) -> Resp {
    send(
        &a.router,
        build(
            "GET",
            "/api/dashboard",
            Some(HOST),
            &[("cookie", cookie.as_bytes())],
            Body::empty(),
        ),
    )
    .await
}

async fn dash_json(a: &App, w: &Who) -> Value {
    let r = dash(a, &w.cookie).await;
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()
}

fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().expect("object").keys().cloned().collect();
    k.sort();
    k
}

fn put_bank(c: &Connection, name: &str) -> i64 {
    c.execute(
        "INSERT INTO banks(name, filename, legacy_ids, created_at, updated_at, owner_id) \
         VALUES(?1, 'f.xlsx', 0, '2026-01-01 10:00', '2026-01-01 10:00', 1)",
        params![name],
    )
    .expect("bank");
    c.last_insert_rowid()
}

fn grant(c: &Connection, user: i64, bank: i64) {
    c.execute(
        "INSERT INTO user_banks(user_id, bank_id) VALUES(?1, ?2)",
        params![user, bank],
    )
    .expect("grant");
}

fn sample_question(stem: &str) -> Question {
    Question {
        kind: Kind::Single,
        tag: String::new(),
        stem: stem.into(),
        options: vec![],
        items: vec![],
        answer: Answer::Letters(vec!["A".into()]),
        translation: String::new(),
        zh: None,
        english: String::new(),
        en: None,
        lang: "en".into(),
        ja: None,
        explanation: String::new(),
        mnemonic: String::new(),
        note: String::new(),
        explain: None,
        id: String::new(),
        sheet: String::new(),
        row: 0,
    }
}

fn put_q(c: &Connection, id: &str, bank: Option<i64>, active: bool, seq: i64) {
    let mut v = serde_json::to_value(sample_question("Sample stem")).expect("json");
    for k in ["id", "sheet", "row"] {
        v.as_object_mut().expect("object").remove(k);
    }
    c.execute(
        "INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id) \
         VALUES(?1, 'S', ?2, ?2, ?3, ?4, '2026-01-01 10:00', '2026-01-01 10:00', ?5)",
        params![id, seq, v.to_string(), i64::from(active), bank],
    )
    .expect("question");
}

/// Statistics of one user: `answered` attempts, `good` of them correct (one row, made-up question id).
fn put_stats(c: &Connection, user: i64, answered: i64, good: i64) {
    c.execute(
        "INSERT INTO user_stats(user_id, question_id, attempts, correct, wrong, streak) VALUES(?1, ?2, ?3, ?4, ?5, 0)",
        params![user, format!("stat-{user}"), answered, good, answered - good],
    )
    .expect("stats");
}

/// A record at `at` with items `(question id, answered, correct)`.
fn put_record(c: &Connection, user: i64, at: &str, items: &[(&str, bool, bool)]) {
    c.execute(
        "INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, banks, user_id, kind) \
         VALUES(?1, '[\"S\"]', 'all', ?2, 0, 0, 60, '[]', ?3, 'practice')",
        params![at, items.len() as i64, user],
    )
    .expect("attempt");
    let id = c.last_insert_rowid();
    for (seq, (qid, answered, correct)) in items.iter().enumerate() {
        let answer = answered.then_some("[\"A\"]");
        c.execute(
            "INSERT INTO attempt_items(attempt_id, seq, question_id, answer, correct) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![id, seq as i64, qid, answer, i64::from(*correct)],
        )
        .expect("item");
    }
}

fn names(j: &Value) -> Vec<String> {
    j["ranking"]["top"]
        .as_array()
        .expect("top")
        .iter()
        .map(|e| e["name"].as_str().expect("name").to_string())
        .collect()
}

fn popular_ids(j: &Value) -> Vec<String> {
    j["popular"]["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["questionId"].as_str().expect("id").to_string())
        .collect()
}

// ---------------------------------------------------------------- tests

#[tokio::test]
async fn empty_install_has_the_exact_shape() {
    let a = app();
    let adm = admin(&a);
    let j = dash_json(&a, &adm).await;
    assert_eq!(keys(&j), ["popular", "ranking"]);
    assert_eq!(keys(&j["ranking"]), ["me", "top", "users"]);
    assert_eq!(keys(&j["ranking"]["me"]), ["answered", "correct", "rank"]);
    assert_eq!(keys(&j["popular"]), ["days", "items", "since"]);
    assert_eq!(j["ranking"]["top"], json!([]));
    assert_eq!(j["ranking"]["users"], json!(0));
    assert_eq!(j["ranking"]["me"]["rank"], Value::Null);
    assert_eq!(j["ranking"]["me"]["answered"], json!(0));
    assert_eq!(j["ranking"]["me"]["correct"], json!(0));
    assert_eq!(j["popular"]["items"], json!([]));
    assert_eq!(j["popular"]["days"], json!(30));
    assert_eq!(j["popular"]["since"], json!("2026-10-23 14:05"));
}

#[tokio::test]
async fn entries_have_the_exact_keys_and_need_a_session() {
    let a = app();
    let adm = admin(&a);
    let c = raw(&a.db);
    let bank = put_bank(&c, "Sample bank");
    put_q(&c, "q1", Some(bank), true, 1);
    put_stats(&c, adm.id, 10, 7);
    put_record(&c, adm.id, "2026-11-20 10:00", &[("q1", true, true)]);
    let j = dash_json(&a, &adm).await;
    assert_eq!(
        keys(&j["ranking"]["top"][0]),
        ["answered", "correct", "name", "rank", "self"]
    );
    assert_eq!(
        keys(&j["popular"]["items"][0]),
        ["bankId", "correct", "questionId", "times"]
    );
    let r = send(
        &a.router,
        build("GET", "/api/dashboard", Some(HOST), &[], Body::empty()),
    )
    .await;
    assert_eq!(r.status, 401);
}

#[tokio::test]
async fn ranking_orders_by_correct_then_fewer_answers_then_id() {
    let a = app();
    let adm = admin(&a);
    let p = user(&a, "pat");
    let q = user(&a, "quinn");
    let r = user(&a, "rene");
    let s = user(&a, "sam");
    let t = user(&a, "tess");
    let c = raw(&a.db);
    put_stats(&c, p.id, 50, 40);
    put_stats(&c, q.id, 40, 40); // all correct: first
    put_stats(&c, r.id, 60, 40); // same correct as pat, more answers
    put_stats(&c, s.id, 50, 40); // identical to pat: the lower id first
    put_stats(&c, t.id, 90, 10);
    let j = dash_json(&a, &adm).await;
    assert_eq!(names(&j), ["quinn", "pat", "sam"]);
    assert_eq!(j["ranking"]["users"], json!(5));
    // the viewer (an administrator without answers) is not ranked
    assert_eq!(j["ranking"]["me"]["rank"], Value::Null);
}

#[tokio::test]
async fn disabled_and_unanswered_users_are_not_ranked() {
    let a = app();
    let adm = admin(&a);
    let on = user(&a, "olga");
    let off = user(&a, "omar");
    let none = user(&a, "nina");
    let c = raw(&a.db);
    put_stats(&c, on.id, 5, 3);
    put_stats(&c, off.id, 99, 99);
    put_stats(&c, none.id, 0, 0);
    c.execute("UPDATE users SET disabled=1 WHERE id=?1", params![off.id])
        .expect("disable");
    let j = dash_json(&a, &adm).await;
    assert_eq!(names(&j), ["olga"]);
    assert_eq!(j["ranking"]["users"], json!(1));
}

#[tokio::test]
async fn names_are_masked_for_everyone_and_domains_never_leave() {
    let a = app();
    let adm = admin(&a);
    let e1 = user(&a, "alice@example.invalid");
    let e2 = user(&a, "bo@example.invalid");
    let plain = user(&a, "carol");
    let c = raw(&a.db);
    put_stats(&c, e1.id, 30, 30);
    put_stats(&c, e2.id, 20, 20);
    put_stats(&c, plain.id, 10, 10);
    for viewer in [&adm, &plain] {
        let r = dash(&a, &viewer.cookie).await;
        assert_eq!(r.status, 200);
        let text = r.text();
        assert!(
            !text.contains("example") && !text.contains("invalid"),
            "{text}"
        );
        assert!(!text.contains('@'), "{text}");
        let j = r.json();
        let top = &j["ranking"]["top"];
        assert_eq!(top[0]["name"], json!("a***e"));
        assert_eq!(top[1]["name"], json!("b***"));
        assert_eq!(top[2]["name"], json!("carol"));
        assert_eq!(top[2]["self"], json!(viewer.id == plain.id));
    }
    // the e-mail user sees their own full name, and only that row says self
    let r = dash_json(&a, &e1).await;
    assert_eq!(
        r["ranking"]["top"][0]["name"],
        json!("alice@example.invalid")
    );
    assert_eq!(r["ranking"]["top"][0]["self"], json!(true));
    assert_eq!(r["ranking"]["top"][1]["self"], json!(false));
}

#[tokio::test]
async fn me_is_reported_when_outside_the_top_three() {
    let a = app();
    let ids: Vec<Who> = ["u1", "u2", "u3", "u4", "u5"]
        .iter()
        .map(|n| user(&a, n))
        .collect();
    let c = raw(&a.db);
    for (i, w) in ids.iter().enumerate() {
        put_stats(&c, w.id, 100, 90 - 10 * i as i64);
    }
    let j = dash_json(&a, &ids[3]).await;
    assert_eq!(j["ranking"]["users"], json!(5));
    assert_eq!(names(&j), ["u1", "u2", "u3"]);
    assert_eq!(
        j["ranking"]["me"],
        json!({"rank": 4, "answered": 100, "correct": 60})
    );
}

#[tokio::test]
async fn unanswered_items_count_in_neither_block() {
    let a = app();
    let w = user(&a, "una");
    let c = raw(&a.db);
    let bank = put_bank(&c, "Sample bank");
    grant(&c, w.id, bank);
    put_q(&c, "q1", Some(bank), true, 1);
    put_q(&c, "q2", Some(bank), true, 2);
    // q1 answered wrong, q2 unanswered (answer NULL, correct 0)
    put_record(
        &c,
        w.id,
        "2026-11-21 09:00",
        &[("q1", true, false), ("q2", false, false)],
    );
    // user_stats counts only answered items by construction
    put_stats(&c, w.id, 1, 0);
    let j = dash_json(&a, &w).await;
    assert_eq!(popular_ids(&j), ["q1"]);
    assert_eq!(j["popular"]["items"][0]["times"], json!(1));
    assert_eq!(j["ranking"]["me"]["answered"], json!(1));
}

#[tokio::test]
async fn reset_removes_the_user_and_pruning_keeps_the_ranking() {
    // a ticking clock, one new minute per call
    let n = Arc::new(AtomicUsize::new(0));
    let a = app_with(Arc::new(FnClock(move || {
        let i = n.fetch_add(1, Ordering::SeqCst);
        format!("2026-11-22 {:02}:{:02}", (i / 60) % 24, i % 60)
    })));
    let w = user(&a, "rita");
    let other = user(&a, "otto");
    {
        let c = raw(&a.db);
        let bank = put_bank(&c, "Sample bank");
        put_q(&c, "q1", Some(bank), true, 1);
    }
    for _ in 0..501 {
        a.store
            .record_attempt(
                w.id,
                &Attempt {
                    results: vec![AttemptResult {
                        id: "q1".into(),
                        correct: true,
                    }],
                    sheets: vec!["S".into()],
                    mode: "all".into(),
                    seconds: 5,
                    kind: AttemptKind::Practice,
                    limit: None,
                    end: None,
                },
            )
            .expect("record");
    }
    let c = raw(&a.db);
    let records: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM attempts WHERE user_id=?1",
            params![w.id],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(records, 500, "the oldest record was pruned");
    let j = dash_json(&a, &other).await;
    assert_eq!(names(&j), ["rita"]);
    assert_eq!(j["ranking"]["top"][0]["answered"], json!(501));
    assert_eq!(j["ranking"]["top"][0]["correct"], json!(501));
    // reset
    let text = json!({}).to_string();
    let len = text.len().to_string();
    let r = send(
        &a.router,
        build(
            "POST",
            "/api/reset",
            Some(HOST),
            &[
                ("cookie", w.cookie.as_bytes()),
                ("origin", b"http://127.0.0.1:8765"),
                ("content-type", b"application/json"),
                ("content-length", len.as_bytes()),
            ],
            Body::from(text),
        ),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let j = dash_json(&a, &other).await;
    assert_eq!(j["ranking"]["top"], json!([]));
    assert_eq!(j["popular"]["items"], json!([]));
}

#[tokio::test]
async fn the_window_is_thirty_days() {
    let a = app();
    let w = user(&a, "wanda");
    let c = raw(&a.db);
    let bank = put_bank(&c, "Sample bank");
    grant(&c, w.id, bank);
    put_q(&c, "old", Some(bank), true, 1);
    put_q(&c, "new", Some(bank), true, 2);
    put_record(&c, w.id, "2026-10-22 12:00", &[("old", true, true)]); // 31 days back
    put_record(&c, w.id, "2026-10-24 12:00", &[("new", true, true)]); // 29 days back
    let j = dash_json(&a, &w).await;
    assert_eq!(j["popular"]["since"], json!("2026-10-23 14:05"));
    assert_eq!(popular_ids(&j), ["new"]);
}

#[tokio::test]
async fn most_practised_follows_the_bank_scope() {
    let a = app();
    let adm = admin(&a);
    let b = user(&a, "bruno");
    let c = raw(&a.db);
    let bank1 = put_bank(&c, "Sample one");
    let bank2 = put_bank(&c, "Sample two");
    put_q(&c, "one", Some(bank1), true, 1);
    put_q(&c, "two", Some(bank2), true, 2);
    grant(&c, b.id, bank1);
    // bank 2 is practised most
    for _ in 0..3 {
        put_record(&c, adm.id, "2026-11-20 10:00", &[("two", true, true)]);
    }
    put_record(&c, adm.id, "2026-11-20 11:00", &[("one", true, true)]);
    let jb = dash_json(&a, &b).await;
    assert_eq!(popular_ids(&jb), ["one"]);
    assert!(!jb.to_string().contains("two"));
    let ja = dash_json(&a, &adm).await;
    assert_eq!(popular_ids(&ja), ["two", "one"]);
    assert_eq!(ja["popular"]["items"][0]["times"], json!(3));
    assert_eq!(ja["popular"]["items"][0]["bankId"], json!(bank2));
}

#[tokio::test]
async fn retired_and_orphan_questions_are_excluded() {
    let a = app();
    let w = user(&a, "wren");
    let c = raw(&a.db);
    let bank = put_bank(&c, "Sample bank");
    grant(&c, w.id, bank);
    put_q(&c, "live", Some(bank), true, 1);
    put_q(&c, "retired", Some(bank), false, 2);
    put_q(&c, "orphan", None, true, 3);
    put_record(
        &c,
        w.id,
        "2026-11-21 09:00",
        &[
            ("live", true, true),
            ("retired", true, true),
            ("orphan", true, true),
            ("gone", true, true),
        ],
    );
    let j = dash_json(&a, &w).await;
    assert_eq!(popular_ids(&j), ["live"]);
}

#[tokio::test]
async fn ties_are_stable_and_capped_at_three() {
    let a = app();
    let adm = admin(&a);
    let c = raw(&a.db);
    let bank = put_bank(&c, "Sample bank");
    for i in 1..=5 {
        put_q(&c, &format!("t{i}"), Some(bank), true, i);
    }
    // every question practised once, correct: the order falls back to bank, then seq
    for i in (1..=5).rev() {
        put_record(
            &c,
            adm.id,
            "2026-11-21 09:00",
            &[(&format!("t{i}"), true, true)],
        );
    }
    for _ in 0..2 {
        let j = dash_json(&a, &adm).await;
        assert_eq!(popular_ids(&j), ["t1", "t2", "t3"]);
    }
}

#[tokio::test]
async fn a_password_change_session_is_refused() {
    let a = app();
    let id = add_user(&a, "newbie", Role::User);
    let cookie = start(&a, id, false);
    let r = dash(&a, &cookie).await;
    assert_eq!(r.status, 403, "{}", r.text());
}
