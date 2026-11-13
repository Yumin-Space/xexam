//! v0.9: what the AI service reads about questions and study records — the token routes `POST /internal/ai/question` and
//! `POST /internal/ai/record` (gate, validation, session binding, bank scope, shapes, projections, answers, sampling), the
//! `recordId` / `key` / `startedAt` of `POST /api/attempt`, the `sessionKey` / `startedAt` of `GET /api/record`, and the
//! schema 5 → 6 step. Made-up questions and users only; an obviously fake token.
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
use xexam_backend::api::internal::InternalToken;
use xexam_backend::api::{build_router, AppState};
use xexam_backend::auth::{self, Role};
use xexam_backend::database::store::{QuestionAsk, StudyLang};
use xexam_backend::database::{Store, UserChange};
use xexam_backend::logs::LogBuffer;
use xexam_backend::models::BankAccess;
use xexam_backend::models::{
    Answer, AttemptKind, Explain, ExplainText, Item, Kind, Opt, Question, Zh,
};
use xexam_backend::services::RealEngine;

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";
const NGINX_HOST: &str = "backend:8765";
const TOKEN: &str = "fake-internal-token-for-tests-0123456789";
const MSG_BAD_REQUEST: &str = "请求的格式不对。";
const KEY: &str = "0123456789abcdef0123456789abcdef";

// ---------------------------------------------------------------- helpers

struct App {
    router: Router,
    store: Arc<Store>,
    db: PathBuf,
    _dir: tempfile::TempDir,
}

fn app_with_link(link: bool) -> App {
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
    if link {
        assert!(state
            .0
            .ai_link
            .set_token(InternalToken::new(TOKEN).expect("token")));
    }
    App {
        router: build_router(state),
        store,
        db,
        _dir: dir,
    }
}

fn app() -> App {
    app_with_link(true)
}

fn raw(p: &Path) -> Connection {
    Connection::open(p).expect("raw open")
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).expect("count")
}

/// A user with the store (a placeholder password hash: these users never sign in through the API).
fn add_user(a: &App, name: &str, role: Role, ai: bool) -> i64 {
    match a
        .store
        .create_user_opts(
            name,
            "placeholder-hash",
            role,
            &BankAccess::Only(Vec::new()),
            false,
            false,
            ai,
        )
        .expect("create")
    {
        UserChange::Done(id) => id,
        other => panic!("{other:?}"),
    }
}

/// Clears the "must change the password" flag of user `id`.
fn clear_flag(a: &App, id: i64) {
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
}

/// Starts a session for user `id`: the cookie and the `sid` (the token hash).
fn session(a: &App, id: i64) -> (String, String) {
    let u = a.store.user_by_id(id).expect("lookup").expect("user");
    let token = auth::new_session_token();
    let hash = auth::token_hash(&token);
    a.store
        .start_session(id, &hash, &u.password_hash, auth::unix_now())
        .expect("start")
        .expect("enabled");
    (format!("{}={token}", auth::SESSION_COOKIE), hash)
}

/// A ready user: created, password flag cleared, one session.
struct Who {
    id: i64,
    cookie: String,
    sid: String,
}

fn user(a: &App, name: &str, ai: bool) -> Who {
    let id = add_user(a, name, Role::User, ai);
    clear_flag(a, id);
    let (cookie, sid) = session(a, id);
    Who { id, cookie, sid }
}

/// The default administrator (id 1) with a session.
fn admin(a: &App) -> Who {
    clear_flag(a, 1);
    let (cookie, sid) = session(a, 1);
    Who { id: 1, cookie, sid }
}

async fn internal(a: &App, method: &str, path: &str, token: Option<&str>, body: &str) -> Resp {
    let len = body.len().to_string();
    let mut hs: Vec<(&str, &[u8])> = vec![
        ("content-type", b"application/json"),
        ("content-length", len.as_bytes()),
    ];
    if let Some(t) = token {
        hs.push(("x-xexam-internal-token", t.as_bytes()));
    }
    send(
        &a.router,
        build(
            method,
            path,
            Some(NGINX_HOST),
            &hs,
            Body::from(body.to_string()),
        ),
    )
    .await
}

/// `POST /internal/ai/question` with the test token.
async fn ask_question(a: &App, body: Value) -> Resp {
    internal(
        a,
        "POST",
        "/internal/ai/question",
        Some(TOKEN),
        &body.to_string(),
    )
    .await
}

/// `POST /internal/ai/record` with the test token.
async fn ask_record(a: &App, body: Value) -> Resp {
    internal(
        a,
        "POST",
        "/internal/ai/record",
        Some(TOKEN),
        &body.to_string(),
    )
    .await
}

fn qbody(w: &Who, qid: &str) -> Value {
    json!({"sid": w.sid, "id": w.id, "questionId": qid, "qlang": "en", "elang": "en"})
}

fn rbody(w: &Who, record: i64, max: i64) -> Value {
    json!({"sid": w.sid, "id": w.id, "recordId": record, "max": max, "elang": "en"})
}

fn keys(v: &Value) -> Vec<&str> {
    v.as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect()
}

fn not_found_bytes() -> Vec<u8> {
    // the API's JSON writer puts a space after every `:` and `,`
    br#"{"ok": false, "reason": "notFound"}"#.to_vec()
}

// ---- data

fn opt(key: &str, text: &str) -> Opt {
    Opt {
        key: key.to_string(),
        text: text.to_string(),
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

/// A made-up six-column single-choice question (English base, Chinese and Japanese translations).
fn six(stem: &str) -> Question {
    Question {
        kind: Kind::Single,
        tag: "TAG-NEVER-SENT".into(),
        stem: stem.into(),
        options: vec![opt("A", "Apple"), opt("B", "Banana"), opt("C", "Cherry")],
        items: vec![],
        answer: Answer::Letters(vec!["B".into()]),
        translation: "RAW-TRANSLATION-NEVER-SENT".into(),
        zh: Some(zh("哪种水果是黄色的？", &["苹果", "香蕉", "樱桃"])),
        english: String::new(),
        en: None,
        lang: "en".into(),
        ja: Some(zh(
            "黄色い果物はどれ？",
            &["りんご", "バナナ", "さくらんぼ"],
        )),
        explanation: "COMPAT".into(),
        mnemonic: String::new(),
        note: String::new(),
        explain: Some(Explain {
            en: Some(ExplainText {
                explanation: "Bananas are yellow.".into(),
                mnemonic: "B for banana".into(),
                note: "confidence: high".into(),
            }),
            zh: Some(ExplainText {
                explanation: "香蕉是黄色的。".into(),
                mnemonic: String::new(),
                note: String::new(),
            }),
            ja: None,
        }),
        id: String::new(),
        sheet: "SHEET-NEVER-SENT".into(),
        row: 9,
    }
}

fn hotspot() -> Question {
    let mut q = six("Judge each statement.");
    q.kind = Kind::Hotspot;
    q.options = vec![];
    q.items = vec![
        Item {
            text: "Water is wet.".into(),
            choices: vec!["Yes".into(), "No".into()],
        },
        Item {
            text: "Fire is cold.".into(),
            choices: vec!["Yes".into(), "No".into()],
        },
        Item {
            text: "Ice is cold.".into(),
            choices: vec!["Yes".into(), "No".into()],
        },
    ];
    q.answer = Answer::Indexes(vec![0, 1, 0]);
    q.zh = Some(Zh {
        stem: "判断每句话。".into(),
        options: None,
        items: Some(vec![
            "水是湿的。".into(),
            "火是冷的。".into(),
            "冰是冷的。".into(),
        ]),
        choices: Some(vec![
            vec!["是".into(), "否".into()],
            vec!["是".into(), "否".into()],
            vec!["是".into(), "否".into()],
        ]),
    });
    q.ja = None;
    q
}

/// A retired question of the old four-column layout: Japanese base text, an English structure, one compatibility
/// explanation.
fn four_column() -> Question {
    let mut q = six("どの果物が黄色い？");
    q.lang = String::new();
    q.options = vec![opt("A", "りんご"), opt("B", "バナナ")];
    q.zh = Some(zh("哪种水果是黄色的？", &["苹果", "香蕉"]));
    q.en = Some(zh("Which fruit is yellow?", &["Apple", "Banana"]));
    q.ja = None;
    q.explain = None;
    q.explanation = "バナナは黄色い。".into();
    q.mnemonic = "覚え方".into();
    q.note = "確信度: 高".into();
    q
}

/// The JSON stored in `questions.data`: every field except id / sheet / row.
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

fn grant(c: &Connection, user: i64, bank: i64) {
    c.execute(
        "INSERT INTO user_banks(user_id, bank_id) VALUES(?1, ?2)",
        params![user, bank],
    )
    .expect("grant");
}

fn put_q(c: &Connection, id: &str, bank: Option<i64>, active: bool, q: &Question, seq: i64) {
    c.execute(
        "INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id) \
         VALUES(?1, 'SHEET-NEVER-SENT', ?2, ?2, ?3, ?4, '2026-01-01 10:00', '2026-01-01 10:00', ?5)",
        params![id, seq, qdata(q), i64::from(active), bank],
    )
    .expect("question");
}

/// One record item: question id, the stored answer (`None` = unanswered) and whether it was correct.
type Row = (String, Option<String>, bool);

fn row(qid: &str, answer: Option<&str>, correct: bool) -> Row {
    (qid.to_string(), answer.map(str::to_string), correct)
}

fn put_record(
    c: &Connection,
    user: i64,
    kind: &str,
    at: &str,
    items: &[Row],
    key: Option<&str>,
    started: Option<i64>,
) -> i64 {
    let good = items.iter().filter(|r| r.2).count();
    c.execute(
        "INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, banks, user_id, kind, session_key, started_at) \
         VALUES(?1, '[\"S\"]', 'all', ?2, ?3, 0, 60, '[]', ?4, ?5, ?6, ?7)",
        params![at, items.len() as i64, good as i64, user, kind, key, started],
    )
    .expect("attempt");
    let id = c.last_insert_rowid();
    for (seq, (qid, answer, correct)) in items.iter().enumerate() {
        c.execute(
            "INSERT INTO attempt_items(attempt_id, seq, question_id, answer, correct) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![id, seq as i64, qid, answer, i64::from(*correct)],
        )
        .expect("item");
    }
    id
}

// ---------------------------------------------------------------- the gate

#[tokio::test]
async fn the_token_gate_answers_404_text_like_an_unknown_path() {
    let a = app();
    let w = user(&a, "gina", true);
    let body = qbody(&w, "q1").to_string();
    let rec = rbody(&w, 1, 5).to_string();
    let info = json!({"sid": w.sid, "id": w.id, "recordId": 1}).to_string();
    for (path, b) in [
        ("/internal/ai/question", &body),
        ("/internal/ai/record", &rec),
        ("/internal/ai/record-info", &info),
    ] {
        for (method, token) in [
            ("POST", None),
            ("POST", Some("wrong-token-wrong-token-wrong-token-1")),
            ("POST", Some("")),
            ("GET", Some(TOKEN)),
            ("PUT", Some(TOKEN)),
        ] {
            let r = internal(&a, method, path, token, b).await;
            assert_eq!(r.status, 404, "{method} {path} {token:?}");
            assert_eq!(r.text(), "not found");
            assert!(r
                .header("content-type")
                .is_some_and(|c| c.starts_with("text/plain")));
        }
        // Not reachable as /api/internal/... (sign in first: the API's own unknown-path answer)
        let r = send(
            &a.router,
            build(
                "POST",
                &format!("/api{path}"),
                Some(HOST),
                &[
                    ("cookie", w.cookie.as_bytes()),
                    ("origin", ORIGIN.as_bytes()),
                    ("content-type", b"application/json"),
                    ("content-length", b.len().to_string().as_bytes()),
                    ("x-xexam-internal-token", TOKEN.as_bytes()),
                ],
                Body::from(b.clone()),
            ),
        )
        .await;
        assert_eq!(r.status, 404, "{path}");
        assert!(!r.text().contains("\"ok\""), "{}", r.text());
    }
    // An unknown path under /internal/ai/
    let r = internal(&a, "POST", "/internal/ai/questions", Some(TOKEN), &body).await;
    assert_eq!(r.status, 404);
    // The link off: no token loaded, so even the right token string is refused
    let off = app_with_link(false);
    let w2 = user(&off, "hana", true);
    let r = internal(
        &off,
        "POST",
        "/internal/ai/question",
        Some(TOKEN),
        &qbody(&w2, "q1").to_string(),
    )
    .await;
    assert_eq!(r.status, 404);
    assert_eq!(r.text(), "not found");
}

#[tokio::test]
async fn the_body_must_be_exactly_the_documented_keys() {
    let a = app();
    let w = user(&a, "ivan", true);
    let sid = w.sid.clone();
    let good_q = qbody(&w, "abc123");
    let r = ask_question(&a, good_q.clone()).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = ask_record(&a, rbody(&w, 1, 5)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let with = |base: &Value, k: &str, v: Value| {
        let mut b = base.clone();
        b[k] = v;
        b
    };
    let without = |base: &Value, k: &str| {
        let mut b = base.clone();
        b.as_object_mut().expect("object").remove(k);
        b
    };
    let mut bad_q: Vec<Value> = vec![
        with(&good_q, "extra", json!(1)),
        with(&good_q, "sid", json!("A".repeat(64))),
        with(&good_q, "sid", json!(sid[..63].to_string())),
        with(&good_q, "sid", json!(5)),
        with(&good_q, "id", json!(0)),
        with(&good_q, "id", json!(-3)),
        with(&good_q, "id", json!("1")),
        with(&good_q, "id", json!(1.5)),
        with(&good_q, "questionId", json!("")),
        with(&good_q, "questionId", json!("x".repeat(65))),
        with(&good_q, "questionId", json!("a%20b")),
        with(&good_q, "questionId", json!("a&b")),
        with(&good_q, "questionId", json!("a b")),
        with(&good_q, "questionId", json!(7)),
        with(&good_q, "qlang", json!("fr")),
        with(&good_q, "qlang", json!("zh-CN")),
        with(&good_q, "qlang", json!(null)),
        with(&good_q, "elang", json!("EN")),
        with(&good_q, "record", json!({"id": 1})),
        with(&good_q, "record", json!({"id": 1, "kind": "quiz"})),
        with(&good_q, "record", json!({"id": 0, "kind": "exam"})),
        with(&good_q, "record", json!({"id": 1, "kind": "exam", "x": 1})),
        with(&good_q, "record", json!("exam")),
    ];
    for k in ["sid", "id", "questionId", "qlang", "elang"] {
        bad_q.push(without(&good_q, k));
    }
    for b in bad_q {
        let r = ask_question(&a, b.clone()).await;
        assert_eq!(r.status, 400, "{b}");
        assert_eq!(r.json(), json!({"error": MSG_BAD_REQUEST}), "{b}");
    }
    // A null record is the same as none
    let r = ask_question(&a, with(&good_q, "record", Value::Null)).await;
    assert_eq!(r.status, 200);
    let good_r = rbody(&w, 1, 5);
    let mut bad_r: Vec<Value> = vec![
        with(&good_r, "extra", json!(1)),
        with(&good_r, "sid", json!("zz")),
        with(&good_r, "id", json!(0)),
        with(&good_r, "recordId", json!(0)),
        with(&good_r, "recordId", json!("1")),
        with(&good_r, "max", json!(0)),
        with(&good_r, "max", json!(71)),
        with(&good_r, "max", json!(-1)),
        with(&good_r, "max", json!(2.5)),
        with(&good_r, "max", json!("5")),
        with(&good_r, "elang", json!("fr")),
        with(&good_r, "elang", json!(1)),
    ];
    for k in ["sid", "id", "recordId", "max", "elang"] {
        bad_r.push(without(&good_r, k));
    }
    for b in bad_r {
        let r = ask_record(&a, b.clone()).await;
        assert_eq!(r.status, 400, "{b}");
    }
    // The bounds of max are fine
    for m in [1, 70] {
        let r = ask_record(&a, with(&good_r, "max", json!(m))).await;
        assert_eq!(r.status, 200, "max {m}");
    }
    // Content type and size are the dispatcher's: not JSON -> 415, over 64 KiB -> 413
    let r = send(
        &a.router,
        build(
            "POST",
            "/internal/ai/question",
            Some(NGINX_HOST),
            &[
                ("content-type", b"text/plain"),
                ("x-xexam-internal-token", TOKEN.as_bytes()),
            ],
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(r.status, 415);
    let big = format!("{{\"pad\": \"{}\"}}", "x".repeat(70 * 1024));
    let r = internal(&a, "POST", "/internal/ai/question", Some(TOKEN), &big).await;
    assert_eq!(r.status, 413);
}

// ---------------------------------------------------------------- session binding

#[tokio::test]
async fn the_call_is_bound_to_the_callers_live_session() {
    let a = app();
    let c = raw(&a.db);
    let bank = put_bank(&c, "Fruit");
    put_q(
        &c,
        "q1",
        Some(bank),
        true,
        &six("Which fruit is yellow?"),
        1,
    );
    let ana = user(&a, "ana", true);
    grant(&c, ana.id, bank);

    // valid session: ok (also for the record route)
    let r = ask_question(&a, qbody(&ana, "q1")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["ok"], json!(true));
    let rec = put_record(
        &c,
        ana.id,
        "practice",
        "2026-03-04 05:06",
        &[row("q1", Some("[\"A\"]"), false)],
        None,
        None,
    );
    assert_eq!(
        ask_record(&a, rbody(&ana, rec, 70)).await.json()["ok"],
        json!(true)
    );

    let session_refused = json!({"ok": false, "reason": "session"});
    // another user's id with this session
    let bo = user(&a, "bo", true);
    let mut b = qbody(&ana, "q1");
    b["id"] = json!(bo.id);
    assert_eq!(ask_question(&a, b).await.json(), session_refused);
    // a session hash that does not exist
    let mut b = qbody(&ana, "q1");
    b["sid"] = json!("0".repeat(64));
    assert_eq!(ask_question(&a, b).await.json(), session_refused);
    let mut b = rbody(&ana, rec, 70);
    b["sid"] = json!("0".repeat(64));
    assert_eq!(ask_record(&a, b).await.json(), session_refused);
    // expired
    c.execute(
        "UPDATE sessions SET expires_at = 1 WHERE token_hash = ?1",
        params![bo.sid],
    )
    .expect("expire");
    assert_eq!(
        ask_question(&a, qbody(&bo, "q1")).await.json(),
        session_refused
    );
    // disabled user
    let cy = user(&a, "cy", true);
    c.execute(
        "UPDATE users SET disabled = 1 WHERE id = ?1",
        params![cy.id],
    )
    .expect("disable");
    assert_eq!(
        ask_question(&a, qbody(&cy, "q1")).await.json(),
        session_refused
    );
    // password change pending
    let di = user(&a, "di", true);
    c.execute(
        "UPDATE users SET must_change_password = 1 WHERE id = ?1",
        params![di.id],
    )
    .expect("flag");
    assert_eq!(
        ask_question(&a, qbody(&di, "q1")).await.json(),
        session_refused
    );
    assert_eq!(
        ask_record(&a, rbody(&di, 1, 5)).await.json(),
        session_refused
    );

    // a plain user without the AI+ feature
    let ed = user(&a, "ed", false);
    grant(&c, ed.id, bank);
    let refused = json!({"ok": false, "reason": "notAllowed"});
    assert_eq!(ask_question(&a, qbody(&ed, "q1")).await.json(), refused);
    assert_eq!(ask_record(&a, rbody(&ed, 1, 5)).await.json(), refused);
    // ... and the administrator always may
    let root = admin(&a);
    let r = ask_question(&a, qbody(&root, "q1")).await;
    assert_eq!(r.json()["ok"], json!(true), "{}", r.text());
}

#[tokio::test]
async fn a_call_writes_nothing_not_even_a_session_refresh() {
    let a = app();
    let c = raw(&a.db);
    let bank = put_bank(&c, "Fruit");
    put_q(
        &c,
        "q1",
        Some(bank),
        true,
        &six("Which fruit is yellow?"),
        1,
    );
    let w = user(&a, "fay", true);
    grant(&c, w.id, bank);
    let rec = put_record(
        &c,
        w.id,
        "practice",
        "2026-03-04 05:06",
        &[row("q1", Some("[\"A\"]"), false)],
        None,
        None,
    );
    // Old enough for the refresh window: every other route would refresh it
    c.execute(
        "UPDATE sessions SET last_seen_at = last_seen_at - 7200 WHERE token_hash = ?1",
        params![w.sid],
    )
    .expect("age");
    let session_row = || -> (i64, i64) {
        c.query_row(
            "SELECT last_seen_at, expires_at FROM sessions WHERE token_hash = ?1",
            params![w.sid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("session row")
    };
    let before = session_row();
    let bytes_before = dbc::db_bytes(&a.db);
    assert_eq!(ask_question(&a, qbody(&w, "q1")).await.status, 200);
    assert_eq!(ask_record(&a, rbody(&w, rec, 70)).await.status, 200);
    assert_eq!(ask_question(&a, qbody(&w, "missing")).await.status, 200);
    assert_eq!(session_row(), before, "no refresh");
    assert_eq!(dbc::db_bytes(&a.db), bytes_before, "no write at all");
}

// ---------------------------------------------------------------- bank scope

#[tokio::test]
async fn a_question_is_found_only_inside_the_callers_banks_with_one_answer_for_everything_else() {
    let a = app();
    let c = raw(&a.db);
    let granted = put_bank(&c, "Granted");
    let other = put_bank(&c, "Other");
    let gone = put_bank(&c, "Gone");
    put_q(
        &c,
        "g1",
        Some(granted),
        true,
        &six("In the granted bank"),
        1,
    );
    put_q(&c, "o1", Some(other), true, &six("In another bank"), 2);
    put_q(&c, "d1", Some(gone), true, &six("In a deleted bank"), 3);
    put_q(&c, "n1", None, true, &six("A question without a bank"), 4);
    put_q(&c, "r1", Some(granted), false, &four_column(), 5);
    let ula = user(&a, "ula", true);
    grant(&c, ula.id, granted);
    grant(&c, ula.id, gone);
    // the bank row is deleted; the question and the grant stay behind (as after an interrupted delete by an older binary)
    c.execute("DELETE FROM banks WHERE id = ?1", params![gone])
        .expect("delete bank");

    let r = ask_question(&a, qbody(&ula, "g1")).await;
    assert_eq!(r.json()["ok"], json!(true), "{}", r.text());
    assert_eq!(r.json()["question"]["active"], json!(true));
    // retired: still found, active false
    let r = ask_question(&a, qbody(&ula, "r1")).await;
    assert_eq!(r.json()["ok"], json!(true), "{}", r.text());
    assert_eq!(r.json()["question"]["active"], json!(false));

    let expected = not_found_bytes();
    for qid in ["o1", "d1", "n1", "no-such-question", "G1", "g2"] {
        let r = ask_question(&a, qbody(&ula, qid)).await;
        assert_eq!(r.status, 200, "{qid}");
        assert_eq!(r.body, expected, "{qid}: byte-identical");
    }
    // An administrator sees every bank, including a question without a bank
    let root = admin(&a);
    for qid in ["g1", "o1", "n1", "r1"] {
        let r = ask_question(&a, qbody(&root, qid)).await;
        assert_eq!(r.json()["ok"], json!(true), "{qid}: {}", r.text());
    }
    assert_eq!(
        ask_question(&a, qbody(&root, "no-such-question"))
            .await
            .body,
        expected
    );
    // A bank manager sees the banks they own too
    let mgr = user(&a, "max", true);
    c.execute(
        "UPDATE users SET can_manage_banks = 1 WHERE id = ?1",
        params![mgr.id],
    )
    .expect("manage");
    c.execute(
        "UPDATE banks SET owner_id = ?1 WHERE id = ?2",
        params![mgr.id, other],
    )
    .expect("own");
    assert_eq!(
        ask_question(&a, qbody(&mgr, "o1")).await.json()["ok"],
        json!(true)
    );
    assert_eq!(ask_question(&a, qbody(&mgr, "g1")).await.body, expected);
}

// ---------------------------------------------------------------- shapes and projections

#[tokio::test]
async fn the_question_answer_has_exactly_the_documented_shape_and_never_the_raw_columns() {
    let a = app();
    let c = raw(&a.db);
    let bank = put_bank(&c, "Fruit");
    put_q(
        &c,
        "q1",
        Some(bank),
        true,
        &six("Which fruit is yellow?"),
        1,
    );
    let w = user(&a, "gus", true);
    grant(&c, w.id, bank);
    c.execute(
        "INSERT INTO user_settings(user_id, key, value) VALUES(?1, 'uiLang', '\"ja\"')",
        params![w.id],
    )
    .expect("setting");

    let r = ask_question(&a, qbody(&w, "q1")).await;
    let v = r.json();
    assert_eq!(
        keys(&v),
        ["ok", "uiLang", "question", "solution", "item", "record"]
    );
    assert_eq!(v["uiLang"], json!("ja"));
    assert_eq!(v["item"], Value::Null);
    assert_eq!(v["record"], Value::Null);
    assert_eq!(
        keys(&v["question"]),
        ["id", "kind", "lang", "stem", "options", "items", "active", "cut"]
    );
    assert_eq!(
        keys(&v["solution"]),
        [
            "answer",
            "explanation",
            "mnemonic",
            "note",
            "explainLang",
            "keyTexts"
        ]
    );
    let text = r.text();
    for never in [
        "TAG-NEVER-SENT",
        "RAW-TRANSLATION-NEVER-SENT",
        "SHEET-NEVER-SENT",
        "\"bank",
        "tag",
        "translation",
        "english",
    ] {
        assert!(!text.contains(never), "{never} in {text}");
    }
    // the hint-safe half holds neither the key nor an explanation
    let qtext = v["question"].to_string();
    assert!(
        !qtext.contains("Bananas are yellow") && !qtext.contains("\"answer\""),
        "{qtext}"
    );
    assert_eq!(v["solution"]["answer"], json!(["B"]));
    assert_eq!(v["solution"]["explanation"], json!("Bananas are yellow."));
    assert_eq!(v["solution"]["explainLang"], json!("en"));
    assert_eq!(
        v["solution"]["keyTexts"],
        json!(["Banana", "香蕉", "バナナ"])
    );
    // uiLang defaults to en for a user who never saved one
    let other = user(&a, "hal", true);
    grant(&c, other.id, bank);
    assert_eq!(
        ask_question(&a, qbody(&other, "q1")).await.json()["uiLang"],
        json!("en")
    );
}

#[tokio::test]
async fn languages_explanations_old_layouts_and_hotspots_through_the_route() {
    let a = app();
    let c = raw(&a.db);
    let bank = put_bank(&c, "Mixed");
    put_q(
        &c,
        "s1",
        Some(bank),
        true,
        &six("Which fruit is yellow?"),
        1,
    );
    put_q(&c, "h1", Some(bank), true, &hotspot(), 2);
    put_q(&c, "f1", Some(bank), false, &four_column(), 3);
    let w = user(&a, "ian", true);
    grant(&c, w.id, bank);
    let ask = |qid: &str, qlang: &str, elang: &str| {
        let mut b = qbody(&w, qid);
        b["qlang"] = json!(qlang);
        b["elang"] = json!(elang);
        b
    };
    // Chinese question text, Japanese explanation preferred (none written): falls back to the question's language
    let v = ask_question(&a, ask("s1", "zh", "ja")).await.json();
    assert_eq!(v["question"]["lang"], json!("zh"));
    assert_eq!(v["question"]["stem"], json!("哪种水果是黄色的？"));
    assert_eq!(v["solution"]["explainLang"], json!("zh"));
    assert_eq!(v["solution"]["explanation"], json!("香蕉是黄色的。"));
    // Japanese question text
    let v = ask_question(&a, ask("s1", "ja", "en")).await.json();
    assert_eq!(v["question"]["lang"], json!("ja"));
    assert_eq!(
        v["question"]["options"][1],
        json!({"key": "B", "text": "バナナ"})
    );
    assert_eq!(v["solution"]["explainLang"], json!("en"));
    // The retired four-column question: Japanese base, English structure, compatibility explanation
    let v = ask_question(&a, ask("f1", "en", "en")).await.json();
    assert_eq!(v["question"]["lang"], json!("en"));
    assert_eq!(v["question"]["options"][0]["text"], json!("Apple"));
    assert_eq!(v["question"]["active"], json!(false));
    assert_eq!(v["solution"]["explainLang"], json!(""));
    assert_eq!(v["solution"]["explanation"], json!("バナナは黄色い。"));
    assert_eq!(v["solution"]["note"], json!("確信度: 高"));
    assert_eq!(
        v["solution"]["keyTexts"],
        json!(["バナナ", "Banana", "香蕉"]),
        "base language first"
    );
    let v = ask_question(&a, ask("f1", "ja", "ja")).await.json();
    assert_eq!(v["question"]["lang"], json!("ja"));
    // Hotspot: items and choices per language, answer indexes, key texts
    let v = ask_question(&a, ask("h1", "zh", "en")).await.json();
    assert_eq!(v["question"]["kind"], json!("hotspot"));
    assert_eq!(v["question"]["options"], json!([]));
    assert_eq!(
        v["question"]["items"][0],
        json!({"text": "水是湿的。", "choices": ["是", "否"]})
    );
    assert_eq!(v["solution"]["answer"], json!([0, 1, 0]));
    assert_eq!(v["solution"]["keyTexts"], json!(["Yes", "是", "No", "否"]));
    // Japanese is missing on this question: the base language and its own choices
    let v = ask_question(&a, ask("h1", "ja", "en")).await.json();
    assert_eq!(v["question"]["lang"], json!("en"));
    assert_eq!(v["question"]["items"][1]["choices"], json!(["Yes", "No"]));
}

#[tokio::test]
async fn long_texts_are_cut_with_the_full_caps() {
    let a = app();
    let c = raw(&a.db);
    let bank = put_bank(&c, "Long");
    let mut q = six(&"あ".repeat(7000));
    q.options = (0..10)
        .map(|i| opt(&((b'A' + i) as char).to_string(), &"い".repeat(2000)))
        .collect();
    q.zh = None;
    q.ja = None;
    q.explain = Some(Explain {
        en: Some(ExplainText {
            explanation: "う".repeat(7000),
            mnemonic: "え".repeat(2000),
            note: "お".repeat(900),
        }),
        zh: None,
        ja: None,
    });
    put_q(&c, "long", Some(bank), true, &q, 1);
    let w = user(&a, "jo", true);
    grant(&c, w.id, bank);
    let v = ask_question(&a, qbody(&w, "long")).await.json();
    let n = |s: &Value| s.as_str().map(|s| s.chars().count());
    assert_eq!(n(&v["question"]["stem"]), Some(6000));
    assert_eq!(v["question"]["cut"], json!(true));
    assert_eq!(v["question"]["options"].as_array().map(Vec::len), Some(8));
    assert_eq!(n(&v["question"]["options"][0]["text"]), Some(1500));
    assert_eq!(n(&v["solution"]["explanation"]), Some(6000));
    assert_eq!(n(&v["solution"]["mnemonic"]), Some(1500));
    assert_eq!(n(&v["solution"]["note"]), Some(500));
    assert!(v["question"]["stem"]
        .as_str()
        .is_some_and(|s| s.ends_with('…')));
}

// ---------------------------------------------------------------- a record with the question

#[tokio::test]
async fn a_question_with_its_record_gives_the_users_answer_and_the_session_identity() {
    let a = app();
    let c = raw(&a.db);
    let bank = put_bank(&c, "Fruit");
    put_q(
        &c,
        "q1",
        Some(bank),
        true,
        &six("Which fruit is yellow?"),
        1,
    );
    put_q(&c, "q2", Some(bank), true, &six("Which fruit is red?"), 2);
    put_q(&c, "q3", Some(bank), true, &hotspot(), 3);
    let w = user(&a, "kit", true);
    grant(&c, w.id, bank);
    let items = [
        row("q1", Some("[\"A\"]"), false),
        row("q2", None, false),
        row("q3", Some("[0, null, 1]"), false),
    ];
    let rec = put_record(
        &c,
        w.id,
        "practice",
        "2026-03-04 05:06",
        &items,
        Some(KEY),
        Some(1_780_000_000),
    );
    let with_record = |qid: &str, kind: &str, id: i64| {
        let mut b = qbody(&w, qid);
        b["record"] = json!({"id": id, "kind": kind});
        b
    };
    let v = ask_question(&a, with_record("q1", "practice", rec))
        .await
        .json();
    assert_eq!(
        v["item"],
        json!({"seq": 0, "answer": ["A"], "correct": false})
    );
    assert_eq!(
        v["record"],
        json!({"id": rec, "at": "2026-03-04 05:06", "kind": "practice", "key": KEY, "startedAt": 1_780_000_000})
    );
    assert_eq!(keys(&v["item"]), ["seq", "answer", "correct"]);
    assert_eq!(keys(&v["record"]), ["id", "at", "kind", "key", "startedAt"]);
    // unanswered: item.answer is null
    let v = ask_question(&a, with_record("q2", "practice", rec))
        .await
        .json();
    assert_eq!(
        v["item"],
        json!({"seq": 1, "answer": null, "correct": false})
    );
    // hotspot answers pass through with their nulls
    let v = ask_question(&a, with_record("q3", "practice", rec))
        .await
        .json();
    assert_eq!(v["item"]["answer"], json!([0, null, 1]));
    // ... but an index past the choices, a string and an entry past the item count become null
    let odd = put_record(
        &c,
        w.id,
        "practice",
        "2026-03-04 05:07",
        &[row("q3", Some("[0, 5, \"A\", 1]"), false)],
        None,
        None,
    );
    let v = ask_question(&a, with_record("q3", "practice", odd))
        .await
        .json();
    assert_eq!(v["item"]["answer"], json!([0, null, null, null]));

    // not found: the wrong kind, the question is not in the record, a record that does not exist, another user's record
    let expected = not_found_bytes();
    assert_eq!(
        ask_question(&a, with_record("q1", "exam", rec)).await.body,
        expected
    );
    put_q(&c, "q9", Some(bank), true, &six("Not in the record"), 9);
    assert_eq!(
        ask_question(&a, with_record("q9", "practice", rec))
            .await
            .body,
        expected
    );
    assert_eq!(
        ask_question(&a, with_record("q1", "practice", rec + 100))
            .await
            .body,
        expected
    );
    let nia = user(&a, "nia", true);
    grant(&c, nia.id, bank);
    let mut b = qbody(&nia, "q1");
    b["record"] = json!({"id": rec, "kind": "practice"});
    assert_eq!(
        ask_question(&a, b.clone()).await.body,
        expected,
        "another user's record"
    );
    // Even an administrator cannot reach another user's record
    let root = admin(&a);
    b["sid"] = json!(root.sid);
    b["id"] = json!(root.id);
    assert_eq!(ask_question(&a, b).await.body, expected);
    // A record of an older version: null key and start time
    let old = put_record(
        &c,
        w.id,
        "exam",
        "2026-01-02 03:04",
        &[row("q1", Some("[\"B\"]"), true)],
        None,
        None,
    );
    let v = ask_question(&a, with_record("q1", "exam", old))
        .await
        .json();
    assert_eq!(v["record"]["key"], Value::Null);
    assert_eq!(v["record"]["startedAt"], Value::Null);
    assert_eq!(v["item"]["correct"], json!(true));
    // A letter that is no longer an option key becomes null (a re-import changed the options)
    let stale = put_record(
        &c,
        w.id,
        "exam",
        "2026-01-02 03:05",
        &[row("q1", Some("[\"Z\", \"B\", 1]"), false)],
        None,
        None,
    );
    let v = ask_question(&a, with_record("q1", "exam", stale))
        .await
        .json();
    assert_eq!(v["item"]["answer"], json!([null, "B", null]));
    // The question twice in one record: the lowest seq
    let twice = put_record(
        &c,
        w.id,
        "exam",
        "2026-01-02 03:06",
        &[
            row("q2", Some("[\"C\"]"), false),
            row("q1", None, false),
            row("q1", Some("[\"A\"]"), false),
        ],
        None,
        None,
    );
    let v = ask_question(&a, with_record("q1", "exam", twice))
        .await
        .json();
    assert_eq!(v["item"]["seq"], json!(1));
}

#[test]
fn the_store_answers_a_question_ask_without_a_route() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let store = Store::open(&db).expect("store");
    let c = raw(&db);
    put_q(&c, "q1", Some(1), true, &six("Which fruit is yellow?"), 1);
    let ask = QuestionAsk {
        question_id: "q1".into(),
        qlang: StudyLang::En,
        elang: StudyLang::En,
        record: None,
    };
    // the default administrator (id 1) sees every bank, even a bank id that has no row
    let v = store.study_question(1, &ask).expect("ok").expect("found");
    assert_eq!(v["ok"], json!(true));
    // a user id that does not exist sees nothing
    assert!(store.study_question(999, &ask).expect("ok").is_none());
    let with_record = QuestionAsk {
        record: Some((1, AttemptKind::Exam)),
        ..ask
    };
    assert!(store.study_question(1, &with_record).expect("ok").is_none());
}

// ---------------------------------------------------------------- the record route

#[tokio::test]
async fn the_record_route_counts_wrong_answers_and_never_unanswered_ones() {
    let a = app();
    let c = raw(&a.db);
    let bank = put_bank(&c, "Fruit");
    for i in 1..=5 {
        put_q(
            &c,
            &format!("q{i}"),
            Some(bank),
            true,
            &six(&format!("Question {i}?")),
            i,
        );
    }
    let w = user(&a, "lou", true);
    grant(&c, w.id, bank);
    let items = [
        row("q1", Some("[\"A\"]"), false),
        row("q2", Some("[]"), false),
        row("q3", None, false),
        row("q4", Some("[\"B\"]"), true),
        row("q5", Some("[\"C\", \"Z\"]"), false),
    ];
    let rec = put_record(
        &c,
        w.id,
        "exam",
        "2026-03-04 05:06",
        &items,
        Some(KEY),
        Some(1_780_000_000),
    );
    let r = ask_record(&a, rbody(&w, rec, 70)).await;
    let v = r.json();
    assert_eq!(
        keys(&v),
        [
            "ok",
            "uiLang",
            "record",
            "wrong",
            "usable",
            "items",
            "budgetCut"
        ],
        "{}",
        r.text()
    );
    assert_eq!(v["budgetCut"], json!(false));
    assert_eq!(
        v["record"],
        json!({"id": rec, "at": "2026-03-04 05:06", "kind": "exam", "mode": "all", "total": 5, "correct": 1,
               "answered": 4, "detail": true, "key": KEY, "startedAt": 1_780_000_000})
    );
    assert_eq!(v["wrong"], json!(3), "q3 is unanswered: not counted");
    assert_eq!(v["usable"], json!(3));
    let got = v["items"].as_array().expect("items");
    assert_eq!(got.len(), 3);
    let seqs: Vec<i64> = got.iter().filter_map(|i| i["seq"].as_i64()).collect();
    assert_eq!(seqs, [0, 1, 4], "in record order");
    assert_eq!(keys(&got[0]), ["seq", "answer", "question", "solution"]);
    assert_eq!(got[0]["answer"], json!(["A"]));
    assert_eq!(got[1]["answer"], json!([]));
    assert_eq!(
        got[2]["answer"],
        json!(["C", null]),
        "an entry that fits no option is null"
    );
    // compact projection: no key texts, no mnemonic / note, language = elang when the question has it
    assert_eq!(got[0]["solution"]["keyTexts"], json!([]));
    assert_eq!(got[0]["solution"]["mnemonic"], json!(""));
    assert_eq!(got[0]["solution"]["note"], json!(""));
    assert_eq!(got[0]["question"]["lang"], json!("en"));
    let mut zhb = rbody(&w, rec, 70);
    zhb["elang"] = json!("zh");
    let zhv = ask_record(&a, zhb).await.json();
    assert_eq!(zhv["items"][0]["question"]["lang"], json!("zh"));
    assert_eq!(zhv["items"][0]["solution"]["explainLang"], json!("zh"));
    // max smaller than the wrong count: that many, distinct, in order
    let v = ask_record(&a, rbody(&w, rec, 2)).await.json();
    assert_eq!(v["wrong"], json!(3));
    assert_eq!(v["usable"], json!(3));
    let seqs: Vec<i64> = v["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|i| i["seq"].as_i64())
        .collect();
    assert_eq!(seqs.len(), 2);
    assert!(seqs.windows(2).all(|p| p[0] < p[1]));
    // another user's record, and a record that does not exist
    let expected = not_found_bytes();
    let other = user(&a, "mia", true);
    assert_eq!(ask_record(&a, rbody(&other, rec, 70)).await.body, expected);
    assert_eq!(ask_record(&a, rbody(&w, rec + 50, 70)).await.body, expected);
    let root = admin(&a);
    assert_eq!(
        ask_record(&a, rbody(&root, rec, 70)).await.body,
        expected,
        "not even an administrator"
    );
}

#[tokio::test]
async fn a_record_without_detail_and_one_without_wrong_answers_give_empty_samples() {
    let a = app();
    let c = raw(&a.db);
    let bank = put_bank(&c, "Fruit");
    put_q(&c, "q1", Some(bank), true, &six("Question 1?"), 1);
    let w = user(&a, "ned", true);
    grant(&c, w.id, bank);
    // saved before v0.5: no attempt_items
    c.execute(
        "INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, banks, user_id, kind) \
         VALUES('2026-01-01 00:00', '[]', 'weak', 4, 2, 50, 9, '[]', ?1, 'exam')",
        params![w.id],
    )
    .expect("old record");
    let old = c.last_insert_rowid();
    let v = ask_record(&a, rbody(&w, old, 70)).await.json();
    assert_eq!(
        (v["wrong"].clone(), v["usable"].clone(), v["items"].clone()),
        (json!(0), json!(0), json!([]))
    );
    assert_eq!(v["record"]["detail"], json!(false));
    assert_eq!(v["record"]["answered"], Value::Null);
    assert_eq!(v["record"]["key"], Value::Null);
    // only unanswered and correct items
    let rec = put_record(
        &c,
        w.id,
        "practice",
        "2026-03-04 05:06",
        &[row("q1", None, false), row("q1", Some("[\"B\"]"), true)],
        None,
        None,
    );
    let v = ask_record(&a, rbody(&w, rec, 70)).await.json();
    assert_eq!(
        (v["wrong"].clone(), v["usable"].clone(), v["items"].clone()),
        (json!(0), json!(0), json!([]))
    );
    assert_eq!(v["record"]["answered"], json!(1));
    assert_eq!(v["record"]["detail"], json!(true));
}

#[tokio::test]
async fn invisible_and_deleted_questions_count_as_wrong_but_are_never_returned() {
    let a = app();
    let c = raw(&a.db);
    let mine = put_bank(&c, "Mine");
    let hidden = put_bank(&c, "Hidden");
    put_q(&c, "m1", Some(mine), true, &six("Mine 1?"), 1);
    put_q(&c, "h1", Some(hidden), true, &six("Hidden 1?"), 2);
    put_q(&c, "n1", None, true, &six("No bank?"), 3);
    put_q(&c, "m2", Some(mine), false, &six("Retired mine?"), 4);
    let w = user(&a, "oli", true);
    grant(&c, w.id, mine);
    let items = [
        row("m1", Some("[\"A\"]"), false),
        row("h1", Some("[\"A\"]"), false),
        row("n1", Some("[\"A\"]"), false),
        row("gone", Some("[\"A\"]"), false),
        row("m2", Some("[\"A\"]"), false),
    ];
    let rec = put_record(&c, w.id, "practice", "2026-03-04 05:06", &items, None, None);
    let v = ask_record(&a, rbody(&w, rec, 70)).await.json();
    assert_eq!(v["wrong"], json!(5));
    assert_eq!(v["usable"], json!(2), "m1 and the retired m2");
    let ids: Vec<&str> = v["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|i| i["question"]["id"].as_str())
        .collect();
    assert_eq!(ids, ["m1", "m2"]);
    assert_eq!(v["items"][1]["question"]["active"], json!(false));
    let text = v.to_string();
    for never in ["Hidden 1?", "No bank?"] {
        assert!(!text.contains(never), "{never}");
    }

    // An administrator's record whose wrong question was deleted: counted in wrong, not in usable, never sampled
    let root = admin(&a);
    let items = [
        row("m1", Some("[\"A\"]"), false),
        row("deleted-q", Some("[\"B\"]"), false),
        row("h1", Some("[\"C\"]"), false),
    ];
    let rec = put_record(&c, root.id, "exam", "2026-03-04 05:07", &items, None, None);
    for _ in 0..5 {
        let v = ask_record(&a, rbody(&root, rec, 70)).await.json();
        assert_eq!(
            (v["wrong"].clone(), v["usable"].clone()),
            (json!(3), json!(2))
        );
        let ids: Vec<&str> = v["items"]
            .as_array()
            .expect("items")
            .iter()
            .filter_map(|i| i["question"]["id"].as_str())
            .collect();
        assert_eq!(ids, ["m1", "h1"]);
    }
}

#[tokio::test]
async fn a_hundred_wrong_answers_give_seventy_distinct_ones_in_record_order() {
    let a = app();
    let c = raw(&a.db);
    let bank = put_bank(&c, "Big");
    let w = user(&a, "pat", true);
    grant(&c, w.id, bank);
    let mut items = Vec::new();
    for i in 0..100 {
        put_q(
            &c,
            &format!("b{i:03}"),
            Some(bank),
            true,
            &six(&format!("Question {i}?")),
            i,
        );
        items.push(row(&format!("b{i:03}"), Some("[\"A\"]"), false));
    }
    items.push(row("b000", None, false));
    items.push(row("b001", None, false));
    items.push(row("b002", Some("[\"B\"]"), true));
    let rec = put_record(&c, w.id, "practice", "2026-03-04 05:06", &items, None, None);
    let mut seen_sets = Vec::new();
    for _ in 0..2 {
        let v = ask_record(&a, rbody(&w, rec, 70)).await.json();
        assert_eq!(
            (v["wrong"].clone(), v["usable"].clone()),
            (json!(100), json!(100))
        );
        let seqs: Vec<i64> = v["items"]
            .as_array()
            .expect("items")
            .iter()
            .filter_map(|i| i["seq"].as_i64())
            .collect();
        assert_eq!(seqs.len(), 70);
        assert!(
            seqs.windows(2).all(|p| p[0] < p[1]),
            "distinct, in seq order"
        );
        assert!(
            seqs.iter().all(|s| (0..100).contains(s)),
            "only the answered-wrong items"
        );
        seen_sets.push(seqs);
    }
    // 70 of 100 drawn twice at random: the chance of the same set twice is astronomically small
    assert_ne!(seen_sets[0], seen_sets[1], "a random draw");
    // A fixed draw (the injected picker): always 0 takes the first 70 positions
    let v = a
        .store
        .study_record_with(w.id, rec, 70, StudyLang::En, &mut |_| 0)
        .expect("ok")
        .expect("found");
    let seqs: Vec<i64> = v["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|i| i["seq"].as_i64())
        .collect();
    assert_eq!(seqs, (0..70).collect::<Vec<i64>>());
    // max 100 is capped at 70 by the store too
    let v = a
        .store
        .study_record_with(w.id, rec, 500, StudyLang::En, &mut |_| 0)
        .expect("ok")
        .expect("found");
    assert_eq!(v["items"].as_array().map(Vec::len), Some(70));
}

#[tokio::test]
async fn seventy_maximal_items_fit_the_size_budget() {
    let a = app();
    let c = raw(&a.db);
    let bank = put_bank(&c, "Maximal");
    let w = user(&a, "quin", true);
    grant(&c, w.id, bank);
    // Single / multiple choice: a full stem, eight full options, a full explanation, in 3-byte characters
    let mut single = six(&"あ".repeat(1500));
    single.options = (0..8)
        .map(|i| opt(&((b'A' + i) as char).to_string(), &"い".repeat(500)))
        .collect();
    single.zh = None;
    single.ja = None;
    single.explain = Some(Explain {
        en: Some(ExplainText {
            explanation: "う".repeat(900),
            mnemonic: "え".repeat(900),
            note: "お".repeat(900),
        }),
        zh: None,
        ja: None,
    });
    let mut items = Vec::new();
    for i in 0..70 {
        put_q(&c, &format!("s{i:02}"), Some(bank), true, &single, i);
        items.push(row(&format!("s{i:02}"), Some("[\"A\"]"), false));
    }
    let rec = put_record(&c, w.id, "practice", "2026-03-04 05:06", &items, None, None);
    let r = ask_record(&a, rbody(&w, rec, 70)).await;
    assert_eq!(r.json()["items"].as_array().map(Vec::len), Some(70));
    assert!(r.body.len() < 1024 * 1024, "{} bytes", r.body.len());
    let v = r.json();
    assert_eq!(
        v["budgetCut"],
        json!(true),
        "70 x 4 000 characters is over the 120 000 budget"
    );
    assert!(text_total(&v) <= 120_000, "{}", text_total(&v));
    assert!(v["items"]
        .as_array()
        .expect("items")
        .iter()
        .all(|i| i["question"]["cut"] == json!(true)));
    // every option keeps its letter
    assert_eq!(v["items"][69]["question"]["options"][7]["key"], json!("H"));
    // Hotspot: 20 items of 300 characters with 10 choices of 80 each is the largest compact item; 70 of them are cut to the
    // same text budget, keeping every statement and every choice
    let mut hot = hotspot();
    hot.stem = "あ".repeat(1500);
    hot.items = (0..25)
        .map(|_| Item {
            text: "い".repeat(500),
            choices: (0..12).map(|_| "う".repeat(100)).collect(),
        })
        .collect();
    hot.answer = Answer::Indexes(vec![0; 25]);
    hot.zh = None;
    let mut items = Vec::new();
    for i in 0..70 {
        put_q(&c, &format!("h{i:02}"), Some(bank), true, &hot, 100 + i);
        items.push(row(&format!("h{i:02}"), Some("[0]"), false));
    }
    let rec = put_record(&c, w.id, "practice", "2026-03-04 05:07", &items, None, None);
    let r = ask_record(&a, rbody(&w, rec, 70)).await;
    let v = r.json();
    assert_eq!(v["items"].as_array().map(Vec::len), Some(70));
    assert_eq!(v["budgetCut"], json!(true));
    assert!(text_total(&v) <= 120_000, "{}", text_total(&v));
    assert!(r.body.len() < 1024 * 1024, "{} bytes", r.body.len());
    assert_eq!(
        v["items"][0]["question"]["items"].as_array().map(Vec::len),
        Some(20)
    );
    assert_eq!(
        v["items"][0]["question"]["items"][0]["choices"]
            .as_array()
            .map(Vec::len),
        Some(10)
    );
    // A record within the budget is not touched
    let few = put_record(
        &c,
        w.id,
        "practice",
        "2026-03-04 05:08",
        &[
            row("s00", Some("[\"A\"]"), false),
            row("s01", Some("[\"A\"]"), false),
        ],
        None,
        None,
    );
    let v = ask_record(&a, rbody(&w, few, 70)).await.json();
    assert_eq!(
        v["budgetCut"],
        json!(false),
        "two items are within the budget (their own caps still apply)"
    );
}

/// Characters of question and answer text in a record answer (what the budget counts).
fn text_total(v: &Value) -> usize {
    let n = |s: &Value| s.as_str().map_or(0, |s| s.chars().count());
    let mut total = 0;
    for it in v["items"].as_array().expect("items") {
        let q = &it["question"];
        total += n(&q["stem"]);
        for o in q["options"].as_array().expect("options") {
            total += n(&o["text"]);
        }
        for i in q["items"].as_array().expect("items") {
            total += n(&i["text"]);
            total += i["choices"]
                .as_array()
                .expect("choices")
                .iter()
                .map(n)
                .sum::<usize>();
        }
        for f in ["explanation", "mnemonic", "note"] {
            total += n(&it["solution"][f]);
        }
    }
    total
}

#[tokio::test]
async fn record_info_names_only_the_callers_own_record_and_needs_no_ai_service() {
    let a = app();
    let c = raw(&a.db);
    let bank = put_bank(&c, "Fruit");
    put_q(
        &c,
        "q1",
        Some(bank),
        true,
        &six("Which fruit is yellow?"),
        1,
    );
    let w = user(&a, "sal", true);
    let rec = put_record(
        &c,
        w.id,
        "exam",
        "2026-03-04 05:06",
        &[row("q1", Some("[\"A\"]"), false)],
        Some(KEY),
        Some(1_780_000_000),
    );
    let info = |w: &Who, id: i64| json!({"sid": w.sid, "id": w.id, "recordId": id});
    let r = internal(
        &a,
        "POST",
        "/internal/ai/record-info",
        Some(TOKEN),
        &info(&w, rec).to_string(),
    )
    .await;
    let v = r.json();
    assert_eq!(keys(&v), ["ok", "uiLang", "record"], "{}", r.text());
    assert_eq!(
        v["record"],
        json!({"id": rec, "at": "2026-03-04 05:06", "kind": "exam", "key": KEY, "startedAt": 1_780_000_000})
    );
    assert_eq!(keys(&v["record"]), ["id", "at", "kind", "key", "startedAt"]);
    // a pre-v0.9 record: nulls
    let old = put_record(
        &c,
        w.id,
        "practice",
        "2026-01-02 03:04",
        &[row("q1", None, false)],
        None,
        None,
    );
    let v = internal(
        &a,
        "POST",
        "/internal/ai/record-info",
        Some(TOKEN),
        &info(&w, old).to_string(),
    )
    .await
    .json();
    assert_eq!(
        (v["record"]["key"].clone(), v["record"]["startedAt"].clone()),
        (Value::Null, Value::Null)
    );
    // not found: another user's record, a missing one (the same bytes), not even for an administrator
    let expected = not_found_bytes();
    let other = user(&a, "tam", false);
    let r = internal(
        &a,
        "POST",
        "/internal/ai/record-info",
        Some(TOKEN),
        &info(&other, rec).to_string(),
    )
    .await;
    assert_eq!(r.body, expected);
    let r = internal(
        &a,
        "POST",
        "/internal/ai/record-info",
        Some(TOKEN),
        &info(&w, rec + 99).to_string(),
    )
    .await;
    assert_eq!(r.body, expected);
    let root = admin(&a);
    let r = internal(
        &a,
        "POST",
        "/internal/ai/record-info",
        Some(TOKEN),
        &info(&root, rec).to_string(),
    )
    .await;
    assert_eq!(r.body, expected);
    // the user without the AI+ feature still gets their own record's identity (they may read their saved chats)
    let mine = put_record(
        &c,
        other.id,
        "exam",
        "2026-03-04 05:09",
        &[row("q1", None, false)],
        None,
        None,
    );
    let v = internal(
        &a,
        "POST",
        "/internal/ai/record-info",
        Some(TOKEN),
        &info(&other, mine).to_string(),
    )
    .await
    .json();
    assert_eq!(v["ok"], json!(true));
    // session errors and validation
    let mut b = info(&w, rec);
    b["sid"] = json!("0".repeat(64));
    let r = internal(
        &a,
        "POST",
        "/internal/ai/record-info",
        Some(TOKEN),
        &b.to_string(),
    )
    .await;
    assert_eq!(r.json(), json!({"ok": false, "reason": "session"}));
    for bad in [
        json!({"sid": w.sid, "id": w.id}),
        json!({"sid": w.sid, "id": w.id, "recordId": 0}),
        json!({"sid": w.sid, "id": w.id, "recordId": 1, "max": 5}),
        json!({"sid": w.sid, "id": 0, "recordId": 1}),
    ] {
        let r = internal(
            &a,
            "POST",
            "/internal/ai/record-info",
            Some(TOKEN),
            &bad.to_string(),
        )
        .await;
        assert_eq!(r.status, 400, "{bad}");
    }
}

// ---------------------------------------------------------------- POST /api/attempt, GET /api/record

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

#[tokio::test]
async fn the_attempt_answer_carries_the_record_id_and_the_session_is_stored() {
    let a = app();
    let c = raw(&a.db);
    let bank = put_bank(&c, "Fruit");
    put_q(
        &c,
        "q1",
        Some(bank),
        true,
        &six("Which fruit is yellow?"),
        1,
    );
    put_q(&c, "q2", Some(bank), true, &six("Which fruit is red?"), 2);
    let w = user(&a, "rex", true);
    grant(&c, w.id, bank);
    let results = r#""results":[{"id":"q1","correct":false,"answer":["A"]},{"id":"q2","correct":true,"answer":["A"]}],"sheets":["S"],"mode":"all","seconds":30,"kind":"practice""#;

    // the first one with a key and a start time
    let body = format!(r#"{{{results},"key":"{KEY}","startedAt":1780000000}}"#);
    let r = api(&a, "POST", "/api/attempt", &w.cookie, Some(&body)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let j = r.json();
    let ks = keys(&j);
    assert_eq!(ks.last().copied(), Some("recordId"), "{ks:?}");
    let id1 = j["recordId"].as_i64().expect("recordId");
    assert_eq!(count(&c, "SELECT MAX(id) FROM attempts"), id1);
    let stored: (Option<String>, Option<i64>) = c
        .query_row(
            "SELECT session_key, started_at FROM attempts WHERE id = ?1",
            params![id1],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row");
    assert_eq!(stored, (Some(KEY.to_string()), Some(1_780_000_000)));

    // invalid values are stored as NULL and never refused; each is judged on its own
    for (extra, want) in [
        (r#","key":"NOT-A-KEY","startedAt":"soon""#, (None, None)),
        (r#","key":5,"startedAt":-1"#, (None, None)),
        (r#","startedAt":1780000001"#, (None, Some(1_780_000_001))),
        (r#","key":null"#, (None, None)),
        (r#""#, (None, None)),
    ] {
        let r = api(
            &a,
            "POST",
            "/api/attempt",
            &w.cookie,
            Some(&format!("{{{results}{extra}}}")),
        )
        .await;
        assert_eq!(r.status, 200, "{extra}: {}", r.text());
        let id = r.json()["recordId"].as_i64().expect("recordId");
        assert!(id > id1);
        let stored: (Option<String>, Option<i64>) = c
            .query_row(
                "SELECT session_key, started_at FROM attempts WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("row");
        assert_eq!(stored, (want.0.map(str::to_string), want.1), "{extra}");
    }

    // GET /api/record names the session (the list does not)
    let r = api(&a, "GET", &format!("/api/record?id={id1}"), &w.cookie, None).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let detail = r.json();
    let rec = &detail["record"];
    assert_eq!(rec["sessionKey"], json!(KEY));
    assert_eq!(rec["startedAt"], json!(1_780_000_000));
    assert_eq!(keys(rec).last().copied(), Some("startedAt"));
    let r = api(&a, "GET", "/api/records", &w.cookie, None).await;
    let listed = r.json();
    let first = &listed["records"][0];
    assert!(
        first.get("sessionKey").is_none() && first.get("startedAt").is_none(),
        "{first}"
    );
    // the internal route reads the same identity back
    let v = ask_record(&a, rbody(&w, id1, 70)).await.json();
    assert_eq!(v["record"]["key"], json!(KEY));
    assert_eq!(v["wrong"], json!(1));
    assert_eq!(v["usable"], json!(1));
}

// ---------------------------------------------------------------- schema 5 -> 6

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

#[test]
fn a_schema_5_database_is_upgraded_after_one_backup_and_keeps_every_record() {
    let dir = dbc::tmp();
    let fresh = dir.path().join("fresh").join("xexam.db");
    std::fs::create_dir_all(fresh.parent().expect("parent")).expect("mkdir");
    drop(Store::open(&fresh).expect("fresh"));
    // A v5 database: what a fresh one is minus the two v6 columns, stamped "5", with some data
    let db = dir.path().join("xexam.db");
    let s = Store::open(&db).expect("open");
    drop(s);
    {
        let c = raw(&db);
        let bank = put_bank(&c, "Fruit");
        put_q(
            &c,
            "q1",
            Some(bank),
            true,
            &six("Which fruit is yellow?"),
            1,
        );
        let rec = put_record(
            &c,
            1,
            "exam",
            "2026-03-04 05:06",
            &[row("q1", Some("[\"A\"]"), false)],
            None,
            None,
        );
        assert!(rec > 0);
        // (v1.1: a fresh database also has the v7 columns and indexes, which go first: `ended`'s CHECK names `time_limit`)
        c.execute_batch(
            "DROP INDEX idx_banks_ai_job; DROP INDEX idx_questions_layout; \
             ALTER TABLE attempts DROP COLUMN ended; ALTER TABLE attempts DROP COLUMN time_limit; \
             ALTER TABLE banks DROP COLUMN ai_meta; ALTER TABLE banks DROP COLUMN ai_job; \
             ALTER TABLE attempts DROP COLUMN session_key; \
             ALTER TABLE attempts DROP COLUMN started_at; \
             UPDATE meta SET value = '5' WHERE key = 'schema';",
        )
        .expect("downgrade");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM pragma_table_info('attempts') WHERE name IN ('session_key', 'started_at')"), 0);
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
            .is_some_and(|n| n.starts_with("xexam-schema5-")),
        "{backup:?}"
    );
    assert_eq!(backups(dir.path()).len(), 1);
    // the backup is the v5 state (no v6 columns), the upgraded file has them
    assert_eq!(
        count(&raw(&backup), "SELECT COUNT(*) FROM pragma_table_info('attempts') WHERE name IN ('session_key', 'started_at')"),
        0
    );
    let c = raw(&db);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM pragma_table_info('attempts') WHERE name IN ('session_key', 'started_at')"), 2);
    let v: String = c
        .query_row("SELECT value FROM meta WHERE key='schema'", [], |r| {
            r.get(0)
        })
        .expect("schema");
    assert_eq!(v, "7");
    // the existing record is intact and has no session
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM attempts WHERE session_key IS NULL AND started_at IS NULL"
        ),
        1
    );
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempt_items"), 1);
    let detail = s.record(1, 1).expect("record").expect("exists");
    assert_eq!(detail["record"]["sessionKey"], Value::Null);
    drop(c);
    drop(s);
    // a fresh database and an upgraded one have the same structure
    assert_eq!(
        sqlite_master(&db)
            .iter()
            .filter(|r| r.1 == "attempts")
            .collect::<Vec<_>>(),
        sqlite_master(&fresh)
            .iter()
            .filter(|r| r.1 == "attempts")
            .collect::<Vec<_>>()
    );
    // the next open has nothing to do
    let bytes = dbc::db_bytes(&db);
    let s = Store::open(&db).expect("reopen");
    assert!(s.migration_backup().is_none());
    drop(s);
    assert_eq!(dbc::db_bytes(&db), bytes, "byte-identical");
    assert_eq!(backups(dir.path()).len(), 1);
}

#[test]
fn a_schema_6_database_is_read_by_the_columns_only_this_version_knows() {
    // The new columns are nullable: a v0.8 binary that treats "6" as current never reads them, and its own INSERT into
    // attempts (without the two columns) leaves them NULL.
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    drop(Store::open(&db).expect("open"));
    let c = raw(&db);
    c.execute(
        "INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, banks, user_id, kind) \
         VALUES('2026-03-04 05:06', '[]', 'all', 1, 0, 0, 1, '[]', 1, 'exam')",
        [],
    )
    .expect("v0.8 style insert");
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM attempts WHERE session_key IS NULL AND started_at IS NULL"
        ),
        1
    );
}
