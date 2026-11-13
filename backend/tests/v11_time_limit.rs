//! v1.1: the time limit and the not-answered rule of exams (`POST /api/attempt`, `GET /api/records`, `GET /api/record`,
//! `Store::record_attempt_tagged`), schema 7 (the four columns, their CHECKs, the migration from 6) and the bank scope read of
//! `/api/bank`. Made-up questions and users only.
//!
//! The rule under test (owner decisions A-OWNER-13 / 13e): in every exam saved by v1.1 (timed or not, handed in by hand or
//! at time-up) a result that is not correct and has no answer is unanswered: not wrong, not in the wrong-answer notebook,
//! stored with answer NULL and correct 0, derived (never a column) in the record views; score = correct / all. A practice
//! session and every record written before v1.1 keep the v1.0 rule.
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
use std::sync::Arc;
use xexam_backend::api::{build_router, AppState};
use xexam_backend::auth::{self, Role};
use xexam_backend::database::{Store, UserChange};
use xexam_backend::logs::LogBuffer;
use xexam_backend::models::{
    Answer, Attempt, AttemptEnd, AttemptKind, AttemptResult, BankAccess, BankScope, Explain,
    ExplainText, Kind, Opt, Question, SessionTag, Zh,
};
use xexam_backend::services::RealEngine;

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";

// ---------------------------------------------------------------- helpers

struct App {
    router: Router,
    store: Arc<Store>,
    db: PathBuf,
    _dir: tempfile::TempDir,
}

fn app() -> App {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let store = Arc::new(Store::open(&db).expect("store"));
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

fn raw(p: &Path) -> Connection {
    Connection::open(p).expect("raw open")
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).expect("count")
}

fn text(c: &Connection, sql: &str) -> String {
    c.query_row(sql, [], |r| r.get(0)).expect("text")
}

struct Who {
    id: i64,
    cookie: String,
}

/// A signed-in plain user who sees `banks`.
fn user(a: &App, name: &str, banks: &[i64]) -> Who {
    let id = match a
        .store
        .create_user_opts(
            name,
            "placeholder-hash",
            Role::User,
            &BankAccess::Only(banks.to_vec()),
            false,
            false,
            false,
        )
        .expect("create")
    {
        UserChange::Done(id) => id,
        other => panic!("{other:?}"),
    };
    let u = a.store.user_by_id(id).expect("lookup").expect("user");
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
    Who {
        id,
        cookie: format!("{}={token}", auth::SESSION_COOKIE),
    }
}

async fn api(a: &App, method: &str, path: &str, cookie: &str, body: Option<&str>) -> Resp {
    let mut hs: Vec<(&str, Vec<u8>)> = vec![("cookie", cookie.as_bytes().to_vec())];
    let body = match body {
        Some(b) => {
            hs.push(("origin", ORIGIN.as_bytes().to_vec()));
            hs.push(("content-type", b"application/json".to_vec()));
            hs.push(("content-length", b.len().to_string().into_bytes()));
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let hrefs: Vec<(&str, &[u8])> = hs.iter().map(|(k, v)| (*k, v.as_slice())).collect();
    send(&a.router, build(method, path, Some(HOST), &hrefs, body)).await
}

fn keys(v: &Value) -> Vec<&str> {
    v.as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect()
}

fn opt(key: &str, t: &str) -> Opt {
    Opt {
        key: key.to_string(),
        text: t.to_string(),
    }
}

fn zh(stem: &str, options: &[&str]) -> Zh {
    Zh {
        stem: stem.to_string(),
        options: Some(options.iter().map(|s| (*s).to_string()).collect()),
        items: None,
        choices: None,
    }
}

/// A made-up six-column single-choice question.
fn six(stem: &str) -> Question {
    Question {
        kind: Kind::Single,
        tag: String::new(),
        stem: stem.into(),
        options: vec![opt("A", "Whisk"), opt("B", "Ladle"), opt("C", "Peeler")],
        items: vec![],
        answer: Answer::Letters(vec!["B".into()]),
        translation: String::new(),
        zh: Some(zh("哪个用来盛汤？", &["打蛋器", "汤勺", "削皮器"])),
        english: String::new(),
        en: None,
        lang: "en".into(),
        ja: Some(zh(
            "スープをすくうのは？",
            &["泡立て器", "おたま", "ピーラー"],
        )),
        explanation: "汤勺用来盛汤。".into(),
        mnemonic: String::new(),
        note: String::new(),
        explain: Some(Explain {
            en: Some(ExplainText {
                explanation: "A ladle serves soup.".into(),
                mnemonic: String::new(),
                note: String::new(),
            }),
            zh: None,
            ja: None,
        }),
        id: String::new(),
        sheet: String::new(),
        row: 0,
    }
}

fn qdata(q: &Question) -> String {
    let mut v = serde_json::to_value(q).expect("json");
    for k in ["id", "sheet", "row"] {
        v.as_object_mut().expect("object").remove(k);
    }
    v.to_string()
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

fn put_q(c: &Connection, id: &str, bank: i64, sheet: &str, seq: i64, stem: &str) {
    c.execute(
        "INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id) \
         VALUES(?1, ?2, ?3, ?3, ?4, 1, '2026-01-01 10:00', '2026-01-01 10:00', ?5)",
        params![id, sheet, seq, qdata(&six(stem)), bank],
    )
    .expect("question");
}

/// A bank with these question ids (sheet "Utensils"); returns the bank id.
fn library(a: &App, ids: &[&str]) -> i64 {
    let c = raw(&a.db);
    let bank = put_bank(&c, "Kitchen basics");
    for (i, id) in ids.iter().enumerate() {
        put_q(
            &c,
            id,
            bank,
            "Utensils",
            i64::try_from(i).expect("small") + 1,
            &format!("Which tool is number {i}?"),
        );
    }
    bank
}

/// `{"results": [...]}` with these `(id, correct, answer json)`.
fn results(rows: &[(&str, bool, &str)]) -> String {
    let items: Vec<String> = rows
        .iter()
        .map(|(id, ok, ans)| {
            if ans.is_empty() {
                format!(r#"{{"id":"{id}","correct":{ok}}}"#)
            } else {
                format!(r#"{{"id":"{id}","correct":{ok},"answer":{ans}}}"#)
            }
        })
        .collect();
    format!(
        r#""results":[{}],"sheets":["Utensils"],"mode":"all","seconds":30"#,
        items.join(",")
    )
}

/// The seven results of the main scenarios: right, wrong, and five kinds of "no answer".
const MIX: [(&str, bool, &str); 8] = [
    ("q1", true, r#"["B"]"#),     // right
    ("q2", false, r#"["A"]"#),    // wrong, answered
    ("q3", false, ""),            // no answer key
    ("q4", false, "[]"),          // empty list
    ("q5", false, "[null,null]"), // hotspot with nothing chosen
    ("q6", false, r#"[0,null]"#), // hotspot partly answered: answered and wrong
    ("q7", true, ""),             // right without a stored answer (kept as correct)
    ("q8", false, r#""A""#),      // malformed answer: nothing usable
];

fn user_stats_ids(c: &Connection, user: i64) -> Vec<String> {
    let mut st = c
        .prepare("SELECT question_id FROM user_stats WHERE user_id=?1 ORDER BY question_id")
        .expect("prepare");
    let rows = st
        .query_map(params![user], |r| r.get::<_, String>(0))
        .expect("query");
    rows.map(|r| r.expect("row")).collect()
}

fn attempt_row(c: &Connection, id: i64) -> (i64, i64, i64, Option<i64>, Option<String>, String) {
    c.query_row(
        "SELECT total, correct, percent, time_limit, ended, kind FROM attempts WHERE id=?1",
        params![id],
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
    .expect("attempt")
}

fn items(c: &Connection, id: i64) -> Vec<(String, Option<String>, i64)> {
    let mut st = c
        .prepare("SELECT question_id, answer, correct FROM attempt_items WHERE attempt_id=?1 ORDER BY seq")
        .expect("prepare");
    let rows = st
        .query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query");
    rows.map(|r| r.expect("row")).collect()
}

fn mix_library(a: &App) {
    library(a, &["q1", "q2", "q3", "q4", "q5", "q6", "q7", "q8"]);
}

// ---------------------------------------------------------------- the route's cleaning

#[tokio::test]
async fn limit_and_end_are_stored_for_exams_and_never_refuse_a_request() {
    let a = app();
    mix_library(&a);
    let w = user(&a, "rex", &[1]);
    let c = raw(&a.db);
    let body = |extra: &str| format!(r#"{{{}{extra}}}"#, results(&[("q1", true, r#"["B"]"#)]));
    // (extra keys, stored time_limit, stored ended)
    let cases: [(&str, Option<i64>, &str); 10] = [
        ("", None, "submit"),
        (r#","limit":3600,"end":"timeUp""#, Some(3600), "timeUp"),
        (r#","limit":3600,"end":"submit""#, Some(3600), "submit"),
        (r#","limit":3600"#, Some(3600), "submit"),
        (r#","limit":60,"end":"timeUp""#, Some(60), "timeUp"),
        (r#","limit":36000,"end":"timeUp""#, Some(36000), "timeUp"),
        // time-up without a usable limit is a hand-in
        (r#","end":"timeUp""#, None, "submit"),
        (r#","limit":59,"end":"timeUp""#, None, "submit"),
        (r#","limit":"3600","end":"timeUp""#, None, "submit"),
        (r#","limit":3600.5,"end":"junk""#, None, "submit"),
    ];
    for (extra, limit, ended) in cases {
        let r = api(&a, "POST", "/api/attempt", &w.cookie, Some(&body(extra))).await;
        assert_eq!(r.status, 200, "{extra}: {}", r.text());
        let id = r.json()["recordId"].as_i64().expect("recordId");
        let row = attempt_row(&c, id);
        assert_eq!(
            (row.3, row.4.as_deref(), row.5.as_str()),
            (limit, Some(ended), "exam"),
            "{extra}"
        );
    }
    // a practice session never has a limit or an end, whatever is sent
    for extra in [
        r#","kind":"practice""#,
        r#","kind":"practice","limit":3600"#,
        r#","kind":"practice","limit":3600,"end":"timeUp""#,
        r#","kind":"practice","end":"submit""#,
    ] {
        let r = api(&a, "POST", "/api/attempt", &w.cookie, Some(&body(extra))).await;
        assert_eq!(r.status, 200, "{extra}: {}", r.text());
        let id = r.json()["recordId"].as_i64().expect("recordId");
        let row = attempt_row(&c, id);
        assert_eq!(
            (row.3, row.4, row.5.as_str()),
            (None, None, "practice"),
            "{extra}"
        );
    }
}

// ---------------------------------------------------------------- the not-answered rule

/// Saves MIX through the API with `extra` keys, returns (record id, record summary, record items).
async fn save_mix(a: &App, w: &Who, extra: &str) -> (i64, Value, Vec<Value>) {
    let body = format!("{{{}{extra}}}", results(&MIX));
    let r = api(a, "POST", "/api/attempt", &w.cookie, Some(&body)).await;
    assert_eq!(r.status, 200, "{extra}: {}", r.text());
    let id = r.json()["recordId"].as_i64().expect("recordId");
    let r = api(a, "GET", &format!("/api/record?id={id}"), &w.cookie, None).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let d = r.json();
    let list = api(a, "GET", "/api/records", &w.cookie, None).await.json();
    // the list and the detail agree on the new keys
    let in_list = list["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|x| x["id"].as_i64() == Some(id))
        .expect("listed")
        .clone();
    for k in ["limit", "ended", "unanswered"] {
        assert_eq!(in_list.get(k), d["record"].get(k), "{k}");
    }
    (
        id,
        d["record"].clone(),
        d["items"].as_array().expect("items").clone(),
    )
}

/// The three unanswered kinds of MIX (q3 no key, q4 `[]`, q5 `[null,null]`, q8 malformed) and what stays as it is.
fn check_not_answered_rule(c: &Connection, user: i64, id: i64, record: &Value, rows: &[Value]) {
    // eight kept results; correct = q1 and q7
    let row = attempt_row(c, id);
    assert_eq!(
        (row.0, row.1, row.2),
        (8, 2, 25),
        "score = correct / ALL, unanswered included"
    );
    // the unanswered four (q3, q4, q5, q8): stored answer NULL, correct 0; the others as sent
    let stored = items(c, id);
    let by_id = |q: &str| stored.iter().find(|i| i.0 == q).expect("item").clone();
    for q in ["q3", "q4", "q5", "q8"] {
        assert_eq!((by_id(q).1, by_id(q).2), (None, 0), "{q}");
    }
    assert_eq!(by_id("q1").1.as_deref(), Some(r#"["B"]"#));
    assert_eq!(
        (by_id("q2").1.as_deref(), by_id("q2").2),
        (Some(r#"["A"]"#), 0)
    );
    assert_eq!(
        (by_id("q6").1.as_deref(), by_id("q6").2),
        (Some("[0, null]"), 0),
        "partly answered is answered"
    );
    assert_eq!(
        (by_id("q7").1, by_id("q7").2),
        (None, 1),
        "a correct result is never unanswered"
    );
    // no statistics for the unanswered ones, not even an empty row; the rest are written as today
    assert_eq!(user_stats_ids(c, user), vec!["q1", "q2", "q6", "q7"]);
    let wrong: i64 = c
        .query_row(
            "SELECT SUM(wrong) FROM user_stats WHERE user_id=?1",
            params![user],
            |r| r.get(0),
        )
        .expect("sum");
    assert_eq!(wrong, 2, "only q2 and q6 are wrong answers");
    // the record views derive the unanswered count and flag the items
    assert_eq!(record["unanswered"], json!(4));
    let flagged: Vec<&str> = rows
        .iter()
        .filter(|i| i["unanswered"] == json!(true))
        .map(|i| i["questionId"].as_str().expect("id"))
        .collect();
    assert_eq!(flagged, vec!["q3", "q4", "q5", "q8"]);
    // the flag sits right after "correct" and only on those items
    let first_flagged = rows
        .iter()
        .find(|i| i["questionId"] == json!("q3"))
        .expect("q3");
    assert_eq!(
        keys(first_flagged),
        [
            "seq",
            "questionId",
            "answer",
            "correct",
            "unanswered",
            "question"
        ]
    );
    let plain = rows
        .iter()
        .find(|i| i["questionId"] == json!("q1"))
        .expect("q1");
    assert_eq!(
        keys(plain),
        ["seq", "questionId", "answer", "correct", "question"]
    );
}

#[tokio::test]
async fn a_time_up_exam_leaves_unanswered_questions_out_of_the_statistics() {
    let a = app();
    mix_library(&a);
    let w = user(&a, "rex", &[1]);
    let (id, record, rows) = save_mix(&a, &w, r#","limit":3600,"end":"timeUp""#).await;
    check_not_answered_rule(&raw(&a.db), w.id, id, &record, &rows);
    // key order: limit, ended, unanswered right after "kind", then (detail only) the session keys
    let ks = keys(&record);
    let kind = ks.iter().position(|k| *k == "kind").expect("kind");
    assert_eq!(&ks[kind + 1..kind + 4], ["limit", "ended", "unanswered"]);
    assert_eq!(record["limit"], json!(3600));
    assert_eq!(record["ended"], json!("timeUp"));
    assert_eq!(ks.last().copied(), Some("startedAt"));
    // the answer of the route is unchanged: progress, no notCounted, recordId last
    let body = format!(r#"{{{},"limit":600,"end":"timeUp"}}"#, results(&MIX));
    let r = api(&a, "POST", "/api/attempt", &w.cookie, Some(&body)).await;
    assert_eq!(
        keys(&r.json()),
        [
            "version",
            "questions",
            "history",
            "masterStreak",
            "recordId"
        ]
    );
    // the history of /api/progress carries the same score
    assert_eq!(r.json()["history"][0]["percent"], json!(25));
}

#[tokio::test]
async fn a_timed_exam_handed_in_by_hand_follows_the_same_rule() {
    let a = app();
    mix_library(&a);
    let w = user(&a, "rex", &[1]);
    let (id, record, rows) = save_mix(&a, &w, r#","limit":3600,"end":"submit""#).await;
    check_not_answered_rule(&raw(&a.db), w.id, id, &record, &rows);
    assert_eq!(record["ended"], json!("submit"));
    assert_eq!(record["limit"], json!(3600));
}

#[tokio::test]
async fn an_untimed_exam_follows_the_same_rule_with_or_without_end() {
    let a = app();
    mix_library(&a);
    for (name, extra) in [
        ("ann", r#","end":"submit""#),
        ("bob", ""),
        ("cat", r#","end":"junk""#),
    ] {
        let w = user(&a, name, &[1]);
        let (id, record, rows) = save_mix(&a, &w, extra).await;
        check_not_answered_rule(&raw(&a.db), w.id, id, &record, &rows);
        assert_eq!(record["ended"], json!("submit"), "{extra}");
        assert!(
            record.get("limit").is_none(),
            "an untimed exam has no limit key: {record}"
        );
        let ks = keys(&record);
        let kind = ks.iter().position(|k| *k == "kind").expect("kind");
        assert_eq!(&ks[kind + 1..kind + 3], ["ended", "unanswered"], "{extra}");
    }
}

#[tokio::test]
async fn thirty_right_of_forty_is_seventy_five_percent() {
    let a = app();
    let ids: Vec<String> = (1..=40).map(|i| format!("p{i}")).collect();
    let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    library(&a, &id_refs);
    let w = user(&a, "rex", &[1]);
    let rows: Vec<(&str, bool, &str)> = id_refs
        .iter()
        .enumerate()
        .map(|(i, id)| {
            if i < 30 {
                (*id, true, r#"["B"]"#)
            } else {
                (*id, false, "")
            }
        })
        .collect();
    for extra in [
        r#","limit":1800,"end":"timeUp""#,
        r#","limit":1800,"end":"submit""#,
        "",
    ] {
        let body = format!("{{{}{extra}}}", results(&rows));
        let r = api(&a, "POST", "/api/attempt", &w.cookie, Some(&body)).await;
        assert_eq!(r.status, 200, "{extra}");
        let id = r.json()["recordId"].as_i64().expect("id");
        let c = raw(&a.db);
        let row = attempt_row(&c, id);
        assert_eq!((row.0, row.1, row.2), (40, 30, 75), "{extra}");
        let d = api(&a, "GET", &format!("/api/record?id={id}"), &w.cookie, None)
            .await
            .json();
        assert_eq!(d["record"]["unanswered"], json!(10), "{extra}");
    }
    // ten unanswered questions never reached the notebook: no row for p31 ... p40
    let c = raw(&a.db);
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM user_stats WHERE wrong > 0"),
        0
    );
    assert_eq!(count(&c, "SELECT COUNT(*) FROM user_stats"), 30);
}

#[tokio::test]
async fn a_practice_session_keeps_the_v10_rule() {
    let a = app();
    mix_library(&a);
    let w = user(&a, "rex", &[1]);
    let (id, record, rows) =
        save_mix(&a, &w, r#","kind":"practice","limit":3600,"end":"timeUp""#).await;
    let c = raw(&a.db);
    let row = attempt_row(&c, id);
    assert_eq!((row.3, row.4, row.5.as_str()), (None, None, "practice"));
    // every empty answer is wrong and written to the statistics
    assert_eq!(
        user_stats_ids(&c, w.id),
        vec!["q1", "q2", "q3", "q4", "q5", "q6", "q7", "q8"]
    );
    let wrong: i64 = c
        .query_row(
            "SELECT SUM(wrong) FROM user_stats WHERE user_id=?1",
            params![w.id],
            |r| r.get(0),
        )
        .expect("sum");
    assert_eq!(wrong, 6);
    // `[]` and `[null, null]` stay stored (only a malformed one is NULL, as before)
    let stored = items(&c, id);
    let find = |q: &str| stored.iter().find(|i| i.0 == q).expect("item").clone();
    assert_eq!(find("q4").1.as_deref(), Some("[]"));
    assert_eq!(find("q5").1.as_deref(), Some("[null, null]"));
    assert_eq!(find("q3").1, None);
    // no new keys, no item flags
    assert_eq!(
        keys(&record),
        [
            "id",
            "at",
            "banks",
            "sheets",
            "mode",
            "total",
            "correct",
            "percent",
            "seconds",
            "answered",
            "detail",
            "kind",
            "sessionKey",
            "startedAt"
        ]
    );
    assert!(rows.iter().all(|i| i.get("unanswered").is_none()));
    assert_eq!(attempt_row(&c, id).2, 25);
}

#[test]
fn an_attempt_without_end_is_a_record_from_before_v11() {
    // What a record written before v1.1 stands for: `Attempt` with `end: None`. Its null answers are wrong and counted, and
    // the views never re-derive anything: no new keys, no flags, no ended.
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = Store::open(&db).expect("open");
    {
        let c = raw(&db);
        let bank = put_bank(&c, "Kitchen basics");
        for (i, q) in ["q1", "q2", "q3"].iter().enumerate() {
            put_q(
                &c,
                q,
                bank,
                "Utensils",
                i64::try_from(i).expect("small") + 1,
                q,
            );
        }
    }
    let attempt = |kind: AttemptKind, limit: Option<i64>, end: Option<AttemptEnd>| Attempt {
        results: ["q1", "q2", "q3"]
            .iter()
            .map(|id| AttemptResult {
                id: (*id).to_string(),
                correct: false,
            })
            .collect(),
        sheets: vec!["Utensils".into()],
        mode: "all".into(),
        seconds: 5,
        kind,
        limit,
        end,
    };
    let answers = vec![None, Some(json!([])), Some(json!([null]))];
    let p = s
        .record_attempt_tagged(
            1,
            &BankScope::All,
            &attempt(AttemptKind::Exam, None, None),
            &answers,
            &SessionTag::default(),
        )
        .expect("record");
    let id = p["recordId"].as_i64().expect("id");
    let c = raw(&db);
    let row = attempt_row(&c, id);
    assert_eq!((row.0, row.1, row.2, row.3, row.4), (3, 0, 0, None, None));
    assert_eq!(
        user_stats_ids(&c, 1),
        vec!["q1", "q2", "q3"],
        "wrong as before"
    );
    // a limit without `end` is not stored (the CHECK would refuse it): the attempt is the old kind of record
    let id2 = s
        .record_attempt_tagged(
            1,
            &BankScope::All,
            &attempt(AttemptKind::Exam, Some(600), None),
            &answers,
            &SessionTag::default(),
        )
        .expect("record")["recordId"]
        .as_i64()
        .expect("id");
    assert_eq!(
        (attempt_row(&c, id2).3, attempt_row(&c, id2).4),
        (None, None)
    );
    let d = s.record(1, id).expect("record").expect("exists");
    assert_eq!(
        keys(&d["record"]),
        [
            "id",
            "at",
            "banks",
            "sheets",
            "mode",
            "total",
            "correct",
            "percent",
            "seconds",
            "answered",
            "detail",
            "kind",
            "sessionKey",
            "startedAt"
        ]
    );
    assert!(d["items"]
        .as_array()
        .expect("items")
        .iter()
        .all(|i| i.get("unanswered").is_none()));
    let list = s.records(1).expect("records");
    assert!(list["records"]
        .as_array()
        .expect("records")
        .iter()
        .all(|r| r.get("ended").is_none() && r.get("unanswered").is_none()));
    // the store keeps the CHECKs quiet for misuse: TimeUp without a limit is a hand-in, a practice never has `ended`
    let id3 = s
        .record_attempt_tagged(
            1,
            &BankScope::All,
            &attempt(AttemptKind::Exam, None, Some(AttemptEnd::TimeUp)),
            &answers,
            &SessionTag::default(),
        )
        .expect("record")["recordId"]
        .as_i64()
        .expect("id");
    assert_eq!(attempt_row(&c, id3).4.as_deref(), Some("submit"));
    let id4 = s
        .record_attempt_tagged(
            1,
            &BankScope::All,
            &attempt(AttemptKind::Practice, Some(600), Some(AttemptEnd::Submit)),
            &answers,
            &SessionTag::default(),
        )
        .expect("record")["recordId"]
        .as_i64()
        .expect("id");
    assert_eq!(
        (attempt_row(&c, id4).3, attempt_row(&c, id4).4),
        (None, None)
    );
    let id5 = s
        .record_attempt_tagged(
            1,
            &BankScope::All,
            &attempt(AttemptKind::Exam, Some(9), Some(AttemptEnd::TimeUp)),
            &answers,
            &SessionTag::default(),
        )
        .expect("record")["recordId"]
        .as_i64()
        .expect("id");
    assert_eq!(
        (attempt_row(&c, id5).3, attempt_row(&c, id5).4.as_deref()),
        (None, Some("submit"))
    );
}

#[tokio::test]
async fn a_record_of_v10_in_the_same_database_keeps_its_old_shape() {
    // An exam saved by v1.0 (ended NULL) and one saved by v1.1 side by side: only the second has the new keys, and the
    // first one's null answers are still wrong.
    let a = app();
    mix_library(&a);
    let w = user(&a, "rex", &[1]);
    {
        let c = raw(&a.db);
        c.execute(
            "INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, banks, user_id, kind) \
             VALUES('2026-03-04 05:06', '[\"Utensils\"]', 'all', 2, 1, 50, 9, '[]', ?1, 'exam')",
            params![w.id],
        )
        .expect("old record");
        let id = c.last_insert_rowid();
        for (seq, (q, ans, ok)) in [("q1", Some(r#"["B"]"#), 1), ("q2", None, 0)]
            .iter()
            .enumerate()
        {
            c.execute(
                "INSERT INTO attempt_items(attempt_id, seq, question_id, answer, correct) VALUES(?1, ?2, ?3, ?4, ?5)",
                params![id, i64::try_from(seq).expect("small"), q, ans, ok],
            )
            .expect("item");
        }
    }
    let (new_id, _, _) = save_mix(&a, &w, "").await;
    let list = api(&a, "GET", "/api/records", &w.cookie, None).await.json();
    let recs = list["records"].as_array().expect("records");
    let old = recs.iter().find(|r| r["id"] != json!(new_id)).expect("old");
    assert!(
        old.get("ended").is_none() && old.get("unanswered").is_none() && old.get("limit").is_none(),
        "{old}"
    );
    let d = api(
        &a,
        "GET",
        &format!("/api/record?id={}", old["id"]),
        &w.cookie,
        None,
    )
    .await
    .json();
    assert!(
        d["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|i| i.get("unanswered").is_none()),
        "{d}"
    );
    // the old record's null answer stays a wrong, counted question
    assert_eq!(old["total"], json!(2));
    assert_eq!(old["answered"], json!(1));
}

// ---------------------------------------------------------------- schema 7

fn sqlite_master(p: &Path) -> Vec<(String, String, String)> {
    let c = raw(p);
    let mut st = c
        .prepare("SELECT type, name, COALESCE(sql, '') FROM sqlite_master ORDER BY type, name")
        .expect("prepare");
    let rows = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query");
    rows.map(|r| r.expect("row")).collect()
}

fn backups(dir: &Path) -> Vec<PathBuf> {
    let d = dir.join("backups");
    if !d.is_dir() {
        return Vec::new();
    }
    let mut v: Vec<PathBuf> = std::fs::read_dir(d)
        .expect("backups")
        .map(|e| e.expect("entry").path())
        .collect();
    v.sort();
    v
}

#[test]
fn a_fresh_database_is_schema_7_with_four_columns_and_two_indexes() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    drop(Store::open(&db).expect("open"));
    let c = raw(&db);
    assert_eq!(text(&c, "SELECT value FROM meta WHERE key='schema'"), "7");
    let cols = |t: &str| -> Vec<String> {
        let mut st = c
            .prepare(&format!("SELECT name FROM pragma_table_info('{t}')"))
            .expect("prepare");
        let rows = st.query_map([], |r| r.get::<_, String>(0)).expect("query");
        rows.map(|r| r.expect("row")).collect()
    };
    let attempts = cols("attempts");
    assert_eq!(&attempts[attempts.len() - 2..], ["time_limit", "ended"]);
    let banks = cols("banks");
    assert_eq!(&banks[banks.len() - 2..], ["ai_job", "ai_meta"]);
    assert!(
        !cols("attempt_items").iter().any(|c| c == "unanswered"),
        "unanswered is derived, never stored"
    );
    assert_eq!(count(&c, "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name IN ('idx_banks_ai_job', 'idx_questions_layout')"), 2);
    let unique: String = text(
        &c,
        "SELECT sql FROM sqlite_master WHERE name='idx_banks_ai_job'",
    );
    assert!(
        unique.contains("UNIQUE") && unique.contains("WHERE ai_job IS NOT NULL"),
        "{unique}"
    );
}

#[test]
fn the_column_checks_are_a_safety_net_with_the_right_shape() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    drop(Store::open(&db).expect("open"));
    let c = raw(&db);
    let put = |kind: &str, limit: &str, ended: &str| -> bool {
        c.execute(
            &format!(
                "INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, banks, user_id, kind, time_limit, ended) \
                 VALUES('2026-03-04 05:06', '[]', 'all', 1, 0, 0, 1, '[]', 1, '{kind}', {limit}, {ended})"
            ),
            [],
        )
        .is_ok()
    };
    // accepted: NULL / NULL (an exam written by v1.0, a practice), an untimed exam handed in, a timed one either way
    assert!(put("exam", "NULL", "NULL"));
    assert!(put("practice", "NULL", "NULL"));
    assert!(put("exam", "NULL", "'submit'"));
    assert!(put("exam", "3600", "'submit'"));
    assert!(put("exam", "3600", "'timeUp'"));
    assert!(put("exam", "60", "'timeUp'"));
    assert!(put("exam", "36000", "'timeUp'"));
    // refused: practice with a limit or an end, a limit without `ended`, time-up without a limit, an end outside the two
    // values, a limit below 60 or above 36000
    assert!(!put("practice", "3600", "'submit'"));
    assert!(!put("practice", "NULL", "'submit'"));
    assert!(!put("practice", "3600", "NULL"));
    assert!(!put("exam", "3600", "NULL"));
    assert!(!put("exam", "NULL", "'timeUp'"));
    assert!(!put("exam", "NULL", "'finished'"));
    assert!(!put("exam", "3600", "'finished'"));
    assert!(!put("exam", "59", "'timeUp'"));
    assert!(!put("exam", "36001", "'submit'"));
    // banks: the facts exist exactly when the job key does
    let bank = |job: &str, meta: &str| -> bool {
        c.execute(
            &format!("INSERT INTO banks(name, filename, legacy_ids, created_at, updated_at, ai_job, ai_meta) VALUES('b', '', 0, 'x', 'x', {job}, {meta})"),
            [],
        )
        .is_ok()
    };
    assert!(bank("NULL", "NULL"));
    assert!(bank("'job1'", "'{}'"));
    assert!(!bank("'job2'", "NULL"));
    assert!(!bank("NULL", "'{}'"));
    assert!(!bank("'job1'", "'{}'"), "one bank per job key");
}

#[test]
fn a_schema_6_database_is_upgraded_after_one_backup_and_keeps_every_record() {
    let dir = dbc::tmp();
    let fresh = dir.path().join("fresh").join("xexam.db");
    std::fs::create_dir_all(fresh.parent().expect("parent")).expect("mkdir");
    drop(Store::open(&fresh).expect("fresh"));
    let db = dir.path().join("xexam.db");
    drop(Store::open(&db).expect("open"));
    {
        let c = raw(&db);
        let bank = put_bank(&c, "Kitchen basics");
        put_q(&c, "q1", bank, "Utensils", 1, "Which tool is one?");
        c.execute(
            "INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, banks, user_id, kind) \
             VALUES('2026-03-04 05:06', '[\"Utensils\"]', 'all', 1, 0, 0, 9, '[]', 1, 'exam')",
            [],
        )
        .expect("record");
        c.execute(
            "INSERT INTO attempt_items(attempt_id, seq, question_id, answer, correct) VALUES(1, 0, 'q1', NULL, 0)",
            [],
        )
        .expect("item");
        // a v6 database: a fresh one minus the v7 objects (the CHECK of `ended` names `time_limit`, so it goes first)
        c.execute_batch(
            "DROP INDEX idx_banks_ai_job; DROP INDEX idx_questions_layout; \
             ALTER TABLE attempts DROP COLUMN ended; ALTER TABLE attempts DROP COLUMN time_limit; \
             ALTER TABLE banks DROP COLUMN ai_meta; ALTER TABLE banks DROP COLUMN ai_job; \
             UPDATE meta SET value = '6' WHERE key = 'schema';",
        )
        .expect("downgrade");
    }
    assert!(backups(dir.path()).is_empty());
    let s = Store::open(&db).expect("upgrade");
    let backup = s
        .migration_backup()
        .expect("a backup was written")
        .to_path_buf();
    assert!(
        backup
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("xexam-schema6-")),
        "{backup:?}"
    );
    assert_eq!(backups(dir.path()).len(), 1);
    // the backup is the v6 state, the upgraded file has the v7 objects
    let v7 =
        "SELECT COUNT(*) FROM pragma_table_info('attempts') WHERE name IN ('time_limit', 'ended')";
    assert_eq!(count(&raw(&backup), v7), 0);
    let c = raw(&db);
    assert_eq!(count(&c, v7), 2);
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM pragma_table_info('banks') WHERE name IN ('ai_job', 'ai_meta')"
        ),
        2
    );
    assert_eq!(text(&c, "SELECT value FROM meta WHERE key='schema'"), "7");
    // the old record is intact: untimed, no `ended` (v1.0 rule), and the views show no new keys
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM attempts WHERE time_limit IS NULL AND ended IS NULL"
        ),
        1
    );
    let d = s.record(1, 1).expect("record").expect("exists");
    assert!(d["record"].get("ended").is_none() && d["record"].get("unanswered").is_none());
    assert!(d["items"][0].get("unanswered").is_none());
    drop(c);
    drop(s);
    // a fresh database and an upgraded one have the same structure
    assert_eq!(sqlite_master(&db), sqlite_master(&fresh));
    // the next open has nothing to do
    let bytes = dbc::db_bytes(&db);
    let s = Store::open(&db).expect("reopen");
    assert!(s.migration_backup().is_none());
    drop(s);
    assert_eq!(dbc::db_bytes(&db), bytes, "byte-identical");
    assert_eq!(backups(dir.path()).len(), 1);
}

#[test]
fn a_v10_binary_can_still_write_to_a_schema_7_database() {
    // The new columns are nullable and the unique index only covers real job keys: a v1.0 binary's own INSERTs (without
    // the new columns) pass the CHECKs and leave NULL / NULL.
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    drop(Store::open(&db).expect("open"));
    let c = raw(&db);
    c.execute(
        "INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, banks, user_id, kind) \
         VALUES('2026-03-04 05:06', '[]', 'all', 1, 0, 0, 1, '[]', 1, 'practice')",
        [],
    )
    .expect("v1.0 style insert");
    c.execute(
        "INSERT INTO banks(name, filename, legacy_ids, created_at, updated_at) VALUES('a', 'a.xlsx', 0, 'x', 'x')",
        [],
    )
    .expect("v1.0 bank");
    c.execute(
        "INSERT INTO banks(name, filename, legacy_ids, created_at, updated_at) VALUES('b', 'b.xlsx', 0, 'x', 'x')",
        [],
    )
    .expect("a second imported bank (both ai_job NULL)");
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM attempts WHERE time_limit IS NULL AND ended IS NULL"
        ),
        1
    );
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM banks WHERE ai_job IS NULL AND ai_meta IS NULL"
        ),
        2
    );
}

// ---------------------------------------------------------------- the bank scope read (DR1-9, DR1-10)

fn plan(c: &Connection, sql: &str, p: &[&dyn rusqlite::ToSql]) -> String {
    let mut st = c
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("explain");
    let rows = st.query_map(p, |r| r.get::<_, String>(3)).expect("query");
    rows.map(|r| r.expect("row"))
        .collect::<Vec<_>>()
        .join(" | ")
}

#[test]
fn a_limited_viewer_reads_only_their_banks_and_the_bank_list_reads_the_covering_index() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let s = Store::open(&db).expect("open");
    let c = raw(&db);
    let (b1, b2, b3) = (
        put_bank(&c, "One"),
        put_bank(&c, "Two"),
        put_bank(&c, "Three"),
    );
    for (i, (bank, sheet)) in [(b1, "S1"), (b2, "S2"), (b2, "S2"), (b3, "S3"), (b2, "S4")]
        .iter()
        .enumerate()
    {
        let n = i64::try_from(i).expect("small");
        put_q(
            &c,
            &format!("q{n}"),
            *bank,
            sheet,
            n + 1,
            &format!("Which tool is {n}?"),
        );
    }
    // the plan the limited read uses: the bank index, never a scan of the table
    let sql = "SELECT id, sheet, row_no, data, active, bank_id FROM questions WHERE bank_id IN (SELECT value FROM json_each(?1)) ORDER BY bank_id, seq";
    let p = plan(&c, sql, &[&"[2]"]);
    assert!(
        p.contains("idx_questions_bank") || p.contains("idx_questions_layout"),
        "{p}"
    );
    assert!(!p.contains("SCAN questions"), "{p}");
    // the bank list's aggregate reads only the covering index
    let p = plan(
        &c,
        "SELECT bank_id, sheet, active FROM questions ORDER BY seq",
        &[],
    );
    assert!(p.contains("COVERING INDEX idx_questions_layout"), "{p}");
    // the output of a limited read = the administrator's read restricted to those banks, same order, same sheets
    let all = s.bank_as(&BankScope::All).expect("all");
    let only = s
        .bank_as(&BankScope::Only([b2].into_iter().collect()))
        .expect("only");
    let want: Vec<&Value> = all["questions"]
        .as_array()
        .expect("questions")
        .iter()
        .filter(|q| q["bank"] == json!(b2))
        .collect();
    assert_eq!(
        only["questions"]
            .as_array()
            .expect("questions")
            .iter()
            .collect::<Vec<_>>(),
        want
    );
    let sheets: Vec<(&str, i64)> = only["sheets"]
        .as_array()
        .expect("sheets")
        .iter()
        .map(|s| {
            (
                s["name"].as_str().expect("name"),
                s["usable"].as_i64().expect("usable"),
            )
        })
        .collect();
    assert_eq!(sheets, vec![("S2", 2), ("S4", 1)]);
    let two = s
        .bank_as(&BankScope::Only([b1, b3].into_iter().collect()))
        .expect("two");
    let ids: Vec<&str> = two["questions"]
        .as_array()
        .expect("q")
        .iter()
        .map(|q| q["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, vec!["q0", "q3"], "grouped by bank id, in seq order");
    let none = s
        .bank_as(&BankScope::Only(Default::default()))
        .expect("none");
    assert_eq!(none["questions"], json!([]));
}
