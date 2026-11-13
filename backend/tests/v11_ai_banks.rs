//! v1.1: AI-generated banks on the backend side: the token routes `POST /internal/ai/gen/source`, `/gen/seeds`, `/gen/create`
//! and `/gen/lookup` (gate, validation, session binding, permission, visibility, sampling, difficulty, caps, idempotent
//! create, unique names, owner, sharing both ways, the guard of the sharing, body limits), the `ai` key of
//! `GET /api/banks`, the refusal of Excel imports into an AI bank, deletion, and the questions of an AI bank in
//! `/api/bank`, records and the notebook. Made-up kitchen questions and users only; an obviously fake token.
#[path = "api_common/mod.rs"]
mod apic;
#[path = "common/mod.rs"]
mod dbc;

use apic::{build, send, Resp};
use axum::body::Body;
use axum::Router;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use xexam_backend::api::internal::InternalToken;
use xexam_backend::api::{build_router, AppState};
use xexam_backend::auth::{self, Role};
use xexam_backend::database::store::{
    ai_question_id, build_question, AiBankOutcome, AiBankReq, AiFile, AiMode, AiQuestion, GenKind,
    Tri, AI_BANKS_AUTO_SHARE,
};
use xexam_backend::database::{Store, UserChange};
use xexam_backend::logs::{BufferLayer, LogBuffer};
use xexam_backend::models::BankAccess;
use xexam_backend::models::{
    Answer, Explain, ExplainText, ImportTarget, Item, Kind, Opt, Question, Zh,
};
use xexam_backend::services::RealEngine;

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";
const NGINX_HOST: &str = "backend:8765";
const TOKEN: &str = "fake-internal-token-for-tests-0123456789";
const MSG_BAD_REQUEST: &str = "请求的格式不对。";
const MSG_AI_BANK_IMPORT: &str = "AI 生成的题库不能用 Excel 更新。";
const JOB: &str = "0123456789abcdef0123456789abcdef";

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
    assert!(state
        .0
        .ai_link
        .set_token(InternalToken::new(TOKEN).expect("token")));
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
    sid: String,
    stamp: String,
}

/// A user with a session. `banks`: what they see now; `new_banks`: "include banks imported later".
fn make_user(
    a: &App,
    name: &str,
    role: Role,
    banks: &[i64],
    new_banks: bool,
    manage: bool,
    ai: bool,
) -> Who {
    let id = match a
        .store
        .create_user_opts(
            name,
            "placeholder-hash",
            role,
            &BankAccess::Only(banks.to_vec()),
            new_banks,
            manage,
            ai,
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
    let sid = auth::token_hash(&token);
    a.store
        .start_session(id, &sid, &u.password_hash, auth::unix_now())
        .expect("start")
        .expect("enabled");
    Who {
        id,
        cookie: format!("{}={token}", auth::SESSION_COOKIE),
        sid,
        stamp: u.uid,
    }
}

/// The default administrator (id 1) with a session.
fn admin(a: &App) -> Who {
    let u = a.store.user_by_id(1).expect("lookup").expect("admin");
    a.store
        .change_own_password(
            1,
            &u.password_hash,
            &u.password_hash,
            &auth::token_hash(&auth::new_session_token()),
            auth::unix_now(),
        )
        .expect("clear")
        .expect("user");
    let token = auth::new_session_token();
    let sid = auth::token_hash(&token);
    a.store
        .start_session(1, &sid, &u.password_hash, auth::unix_now())
        .expect("start")
        .expect("enabled");
    Who {
        id: 1,
        cookie: format!("{}={token}", auth::SESSION_COOKIE),
        sid,
        stamp: u.uid,
    }
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

async fn post(a: &App, path: &str, body: &Value) -> Resp {
    internal(a, "POST", path, Some(TOKEN), &body.to_string()).await
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

// ---- data

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

/// A made-up six-column question (English base, Chinese and Japanese translations, an English and a Chinese explanation).
fn six(stem: &str, multi: bool) -> Question {
    Question {
        kind: if multi { Kind::Multi } else { Kind::Single },
        tag: String::new(),
        stem: stem.into(),
        options: vec![opt("A", "Whisk"), opt("B", "Ladle"), opt("C", "Peeler")],
        items: vec![],
        answer: Answer::Letters(if multi {
            vec!["A".into(), "B".into()]
        } else {
            vec!["B".into()]
        }),
        translation: "RAW-TRANSLATION".into(),
        zh: Some(zh(&format!("{stem} 中文"), &["打蛋器", "汤勺", "削皮器"])),
        english: String::new(),
        en: None,
        lang: "en".into(),
        ja: Some(zh(
            &format!("{stem} 日本語"),
            &["泡立て器", "おたま", "ピーラー"],
        )),
        explanation: "COMPAT".into(),
        mnemonic: String::new(),
        note: String::new(),
        explain: Some(Explain {
            en: Some(ExplainText {
                explanation: "A ladle serves soup.".into(),
                mnemonic: "L for ladle".into(),
                note: String::new(),
            }),
            zh: Some(ExplainText {
                explanation: "汤勺用来盛汤。".into(),
                mnemonic: String::new(),
                note: String::new(),
            }),
            ja: None,
        }),
        id: String::new(),
        sheet: String::new(),
        row: 0,
    }
}

fn hotspot() -> Question {
    let mut q = six("Judge each statement.", false);
    q.kind = Kind::Hotspot;
    q.options = vec![];
    q.items = vec![Item {
        text: "Water is wet.".into(),
        choices: vec!["Yes".into(), "No".into()],
    }];
    q.answer = Answer::Indexes(vec![0]);
    q
}

fn old_layout() -> Question {
    let mut q = six("どの道具？", false);
    q.lang = String::new();
    q
}

fn qdata(q: &Question) -> String {
    let mut v = serde_json::to_value(q).expect("json");
    for k in ["id", "sheet", "row"] {
        v.as_object_mut().expect("object").remove(k);
    }
    v.to_string()
}

fn put_bank(c: &Connection, name: &str, owner: i64) -> i64 {
    c.execute(
        "INSERT INTO banks(name, filename, legacy_ids, created_at, updated_at, owner_id) \
         VALUES(?1, 'f.xlsx', 0, '2026-01-01 10:00', '2026-01-01 10:00', ?2)",
        params![name, owner],
    )
    .expect("bank");
    c.last_insert_rowid()
}

fn put_q(c: &Connection, id: &str, bank: i64, sheet: &str, seq: i64, active: bool, q: &Question) {
    c.execute(
        "INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id) \
         VALUES(?1, ?2, ?3, ?3, ?4, ?5, '2026-01-01 10:00', '2026-01-01 10:00', ?6)",
        params![id, sheet, seq, qdata(q), i64::from(active), bank],
    )
    .expect("question");
}

fn grant(c: &Connection, user: i64, bank: i64) {
    c.execute(
        "INSERT INTO user_banks(user_id, bank_id) VALUES(?1, ?2)",
        params![user, bank],
    )
    .expect("grant");
}

fn put_stat(c: &Connection, user: i64, qid: &str, attempts: i64, wrong: i64, streak: i64) {
    c.execute(
        "INSERT INTO user_stats(user_id, question_id, attempts, correct, wrong, streak, last, at) \
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, 0, '2026-01-02 10:00')",
        params![user, qid, attempts, attempts - wrong, wrong, streak],
    )
    .expect("stat");
}

/// The shared library. Users: `alex` (AI+, may manage banks), `bob` (AI+, may not), `cara` (may manage, no AI+), `dan` (plain,
/// "include later", sees the main bank), `eve` ("include later", sees nothing). Banks: `main` "Kitchen basics" (owner alex:
/// sheet Ovens with 30 valid questions, the first 4 multiple-choice, plus a hotspot and a retired one; sheet Knives with 27),
/// `other` "Pantry" (owner admin, sheet Shelves 12; alex sees it but does not manage it), `many` "Many sheets" (owner alex, 60
/// sheets of one question), `lone` "Spices" (owner cara, 3 questions).
struct Lib {
    admin: Who,
    alex: Who,
    bob: Who,
    cara: Who,
    dan: Who,
    eve: Who,
    main: i64,
    other: i64,
    many: i64,
    lone: i64,
}

fn library(a: &App) -> Lib {
    let admin = admin(a);
    let alex = make_user(a, "alexandra_kitchen", Role::User, &[], false, true, true);
    let bob = make_user(a, "bob", Role::User, &[], false, false, true);
    let cara = make_user(a, "cara", Role::User, &[], false, true, false);
    let dan = make_user(a, "dan", Role::User, &[], true, false, false);
    let eve = make_user(a, "eve", Role::User, &[], true, false, false);
    let c = raw(&a.db);
    let main = put_bank(&c, "Kitchen basics", alex.id);
    let mut seq = 0;
    let mut next = || {
        seq += 1;
        seq
    };
    for i in 1..=30 {
        let stem = if i == 3 {
            "Z".repeat(200)
        } else {
            format!("Which oven is number {i}?")
        };
        put_q(
            &c,
            &format!("ov{i}"),
            main,
            "Ovens",
            next(),
            true,
            &six(&stem, i <= 4),
        );
    }
    put_q(&c, "hot1", main, "Ovens", next(), true, &hotspot());
    put_q(
        &c,
        "gone1",
        main,
        "Ovens",
        next(),
        false,
        &six("Retired oven?", false),
    );
    put_q(&c, "old1", main, "Ovens", next(), true, &old_layout());
    for i in 1..=27 {
        put_q(
            &c,
            &format!("kn{i}"),
            main,
            "Knives",
            next(),
            true,
            &six(&format!("Which knife is number {i}?"), false),
        );
    }
    let other = put_bank(&c, "Pantry", 1);
    for i in 1..=12 {
        put_q(
            &c,
            &format!("sh{i}"),
            other,
            "Shelves",
            i,
            true,
            &six(&format!("Which shelf is number {i}?"), false),
        );
    }
    let many = put_bank(&c, "Many sheets", alex.id);
    for i in 1..=60 {
        put_q(
            &c,
            &format!("ms{i}"),
            many,
            &format!("Sheet {i}"),
            i,
            true,
            &six(&format!("Which item is number {i}?"), false),
        );
    }
    let lone = put_bank(&c, "Spices", cara.id);
    for i in 1..=3 {
        put_q(
            &c,
            &format!("sp{i}"),
            lone,
            "Jars",
            i,
            true,
            &six(&format!("Which spice is number {i}?"), false),
        );
    }
    for b in [main, other, many] {
        grant(&c, alex.id, b);
    }
    grant(&c, bob.id, main);
    grant(&c, cara.id, lone);
    grant(&c, dan.id, main);
    Lib {
        admin,
        alex,
        bob,
        cara,
        dan,
        eve,
        main,
        other,
        many,
        lone,
    }
}

fn bank_body(w: &Who, bank: i64, min: i64, max: i64) -> Value {
    json!({"sid": w.sid, "source": "bank", "bank": bank, "min": min, "max": max})
}

fn sheets_body(w: &Who, source: &str, sheets: &[(i64, &str)], min: i64, max: i64) -> Value {
    json!({"sid": w.sid, "source": source, "min": min, "max": max,
           "sheets": sheets.iter().map(|(b, s)| json!({"bank": b, "sheet": s})).collect::<Vec<_>>()})
}

fn tri(s: &str) -> Value {
    json!({"en": s, "zh": format!("{s} 中文"), "ja": format!("{s} 日本語")})
}

/// One valid generated question number `i`: three out of four are single-choice, every third a multiple-choice one.
fn gen_q(i: usize) -> Value {
    let multi = i % 3 == 2;
    json!({
        "kind": if multi { "multi" } else { "single" },
        "stem": tri(&format!("Which pan is number {i} in the new set?")),
        "options": (0..4).map(|k| tri(&format!("Pan {i}{k}"))).collect::<Vec<_>>(),
        "answer": if multi { json!(["A", "C"]) } else { json!(["B"]) },
        "explanation": tri(&format!("Pan {i} fits because it is made for that.")),
    })
}

fn create_body(w: &Who, job: &str, name: &str, from: &[i64], n: usize) -> Value {
    json!({
        "job": job,
        "user": w.id,
        "stamp": w.stamp,
        "name": name,
        "source": "bank",
        "from": from,
        "goal": n,
        "at": "2026-11-12 09:30:00",
        "sheet": "AI",
        "questions": (0..n).map(gen_q).collect::<Vec<_>>(),
    })
}

fn job_key(n: u32) -> String {
    format!("{n:032x}")
}

fn ai_req(w: &Who, job: &str, name: &str, from: &[i64], n: usize) -> AiBankReq {
    let t = |s: &str| Tri {
        en: s.to_string(),
        zh: format!("{s} 中文"),
        ja: format!("{s} 日本語"),
    };
    AiBankReq {
        job: job.to_string(),
        user: w.id,
        stamp: w.stamp.clone(),
        name: name.to_string(),
        kind: GenKind::Bank,
        from: from.to_vec(),
        goal: n,
        at: "2026-11-12 09:30:00".into(),
        sheet: "AI".into(),
        questions: (0..n)
            .map(|i| AiQuestion {
                multi: false,
                stem: t(&format!("Which pan is number {i}?")),
                options: (0..3).map(|k| t(&format!("Pan {i}{k}"))).collect(),
                answer: vec!["B".into()],
                explanation: t("It fits."),
            })
            .collect(),
        file: None,
        skipped: Vec::new(),
    }
}

fn created(o: AiBankOutcome) -> i64 {
    match o {
        AiBankOutcome::Created { bank_id, .. } => bank_id,
        other => panic!("{other:?}"),
    }
}

fn banks_with_access(c: &Connection, bank: i64) -> BTreeSet<i64> {
    let mut st = c
        .prepare("SELECT user_id FROM user_banks WHERE bank_id=?1")
        .expect("prepare");
    let rows = st
        .query_map(params![bank], |r| r.get::<_, i64>(0))
        .expect("query");
    rows.map(|r| r.expect("row")).collect()
}

/// The log buffer of a process-wide subscriber (a per-thread one races with the callsite cache when other tests of this binary
/// log the same line on other threads). Other tests log other bank names, so a test looks for its own line.
fn global_logs() -> Arc<LogBuffer> {
    use tracing_subscriber::layer::SubscriberExt;
    static LOGS: std::sync::OnceLock<Arc<LogBuffer>> = std::sync::OnceLock::new();
    LOGS.get_or_init(|| {
        let lines = Arc::new(LogBuffer::new(10_000));
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::registry().with(BufferLayer::new(lines.clone())),
        );
        lines
    })
    .clone()
}

fn plan(c: &Connection, sql: &str) -> String {
    let mut st = c
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("explain");
    let rows = st.query_map([], |r| r.get::<_, String>(3)).expect("query");
    rows.map(|r| r.expect("row"))
        .collect::<Vec<_>>()
        .join(" | ")
}

// ---------------------------------------------------------------- the gate

#[tokio::test]
async fn the_token_gate_answers_404_text_like_an_unknown_path() {
    let a = app();
    let l = library(&a);
    let bodies = [
        (
            "/internal/ai/gen/source",
            bank_body(&l.alex, l.main, 30, 100),
        ),
        (
            "/internal/ai/gen/seeds",
            json!({"sid": l.alex.sid, "ids": ["ov1"]}),
        ),
        (
            "/internal/ai/gen/create",
            create_body(&l.alex, JOB, "n", &[l.main], 1),
        ),
        ("/internal/ai/gen/lookup", json!({"job": JOB})),
    ];
    for (path, body) in &bodies {
        for (method, token) in [
            ("POST", None),
            ("POST", Some("wrong-token-wrong-token-wrong-token-1")),
            ("POST", Some("")),
            ("GET", Some(TOKEN)),
            ("PUT", Some(TOKEN)),
        ] {
            let r = internal(&a, method, path, token, &body.to_string()).await;
            assert_eq!(r.status, 404, "{method} {path} {token:?}");
            assert_eq!(r.text(), "not found");
        }
        // the right token and method: a real answer
        let r = post(&a, path, body).await;
        assert_eq!(r.status, 200, "{path}: {}", r.text());
        assert_eq!(r.json()["ok"], json!(true), "{path}: {}", r.text());
    }
    // another content type is refused like on the other token routes
    let r = send(
        &a.router,
        build(
            "POST",
            "/internal/ai/gen/lookup",
            Some(NGINX_HOST),
            &[
                ("x-xexam-internal-token", TOKEN.as_bytes()),
                ("content-type", b"text/plain"),
            ],
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(r.status, 415);
}

// ---------------------------------------------------------------- source: bank mode

#[tokio::test]
async fn the_bank_source_is_a_stratified_listing_with_difficulty_labels() {
    let a = app();
    let l = library(&a);
    {
        let c = raw(&a.db);
        // ov1: 10 attempts of two users, 1 wrong -> easy; ov2 and ov4: 40 % and exactly 50 % -> mid; ov5: 90 % -> hard;
        // ov6: only 4 attempts -> null; ov7: no statistics -> null; ov8: exactly 20 % -> mid (only below 20 % is easy)
        put_stat(&c, l.alex.id, "ov1", 4, 1, 0);
        put_stat(&c, l.bob.id, "ov1", 6, 0, 3);
        put_stat(&c, l.alex.id, "ov2", 10, 4, 0);
        put_stat(&c, l.alex.id, "ov4", 10, 5, 0);
        put_stat(&c, l.alex.id, "ov5", 10, 9, 0);
        put_stat(&c, l.alex.id, "ov6", 4, 4, 0);
        put_stat(&c, l.alex.id, "ov8", 10, 2, 0);
    }
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &bank_body(&l.alex, l.main, 30, 2000),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let j = r.json();
    assert_eq!(
        keys(&j),
        ["ok", "user", "total", "banks", "strata", "items", "previous"]
    );
    assert_eq!(
        j["user"],
        json!({"id": l.alex.id, "username": "alexandra_kitchen", "role": "user"})
    );
    // 30 + 27 valid questions: the hotspot, the retired one and the old-layout one are not source questions
    assert_eq!(j["total"], json!(57));
    assert_eq!(
        j["banks"],
        json!([{"id": l.main, "name": "Kitchen basics"}])
    );
    assert_eq!(
        j["strata"],
        json!([
            {"bank": l.main, "sheet": "Ovens", "n": 30, "multi": 4},
            {"bank": l.main, "sheet": "Knives", "n": 27, "multi": 0},
        ])
    );
    let items = j["items"].as_array().expect("items");
    assert_eq!(items.len(), 57);
    assert_eq!(
        keys(&items[0]),
        ["id", "s", "kind", "opts", "right", "diff", "stem"]
    );
    let by = |id: &str| {
        items
            .iter()
            .find(|i| i["id"] == json!(id))
            .unwrap_or_else(|| panic!("{id}"))
            .clone()
    };
    assert_eq!(by("ov1")["s"], json!(0));
    assert_eq!(by("kn1")["s"], json!(1));
    assert_eq!(
        (
            by("ov1")["kind"].clone(),
            by("ov1")["opts"].clone(),
            by("ov1")["right"].clone()
        ),
        (json!("multi"), json!(3), json!(2))
    );
    assert_eq!(
        (by("ov9")["kind"].clone(), by("ov9")["right"].clone()),
        (json!("single"), json!(1))
    );
    assert_eq!(
        by("ov1")["stem"],
        json!({"en": "Which oven is number 1?", "zh": "Which oven is number 1? 中文", "ja": "Which oven is number 1? 日本語"})
    );
    // a 200-character stem is cut to 160 characters, the last one `…`
    let long = by("ov3")["stem"]["en"].as_str().expect("en").to_string();
    assert_eq!(long.chars().count(), 160);
    assert!(long.ends_with('…') && long.starts_with("ZZZ"));
    // the labels come from the sum over all users
    for (id, want) in [
        ("ov1", json!("easy")),
        ("ov2", json!("mid")),
        ("ov4", json!("mid")),
        ("ov5", json!("hard")),
        ("ov6", Value::Null),
        ("ov7", Value::Null),
        ("ov8", json!("mid")),
    ] {
        assert_eq!(by(id)["diff"], want, "{id}");
    }
    // the order: strata by bank, then the sheet's first seq; items by stratum, then seq
    let order: Vec<&str> = items
        .iter()
        .map(|i| i["id"].as_str().expect("id"))
        .collect();
    assert_eq!(&order[..3], ["ov1", "ov2", "ov3"]);
    assert_eq!(order[30], "kn1");
    assert_eq!(j["previous"], json!([]));
}

#[tokio::test]
async fn a_big_source_is_sampled_per_stratum_deterministically() {
    let a = app();
    let l = library(&a);
    let ask = || async {
        post(
            &a,
            "/internal/ai/gen/source",
            &bank_body(&l.alex, l.main, 30, 20),
        )
        .await
        .json()
    };
    let j = ask().await;
    assert_eq!(
        j["total"],
        json!(57),
        "total counts every eligible question, not only the returned ones"
    );
    let items = j["items"].as_array().expect("items");
    assert_eq!(items.len(), 20);
    // Hamilton over 30 and 27: 11 and 9; the strata keep their full counts
    assert_eq!(items.iter().filter(|i| i["s"] == json!(0)).count(), 11);
    assert_eq!(items.iter().filter(|i| i["s"] == json!(1)).count(), 9);
    assert_eq!(j["strata"][0]["n"], json!(30));
    assert_eq!(j["strata"][1]["n"], json!(27));
    // every k-th by seq: ascending inside the stratum, ids belong to it
    let ov: Vec<&str> = items
        .iter()
        .filter(|i| i["s"] == json!(0))
        .map(|i| i["id"].as_str().expect("id"))
        .collect();
    assert!(ov.iter().all(|id| id.starts_with("ov")));
    let nums: Vec<u32> = ov.iter().map(|id| id[2..].parse().expect("n")).collect();
    assert!(nums.windows(2).all(|w| w[0] < w[1]), "{nums:?}");
    // deterministic
    assert_eq!(ask().await, j);
    // one place per stratum while it allows: max 2 gives one of each
    let two = post(
        &a,
        "/internal/ai/gen/source",
        &bank_body(&l.alex, l.main, 30, 2),
    )
    .await
    .json();
    assert_eq!(two["items"].as_array().expect("items").len(), 2);
    assert_eq!(two["items"][0]["s"], json!(0));
    assert_eq!(two["items"][1]["s"], json!(1));
}

#[tokio::test]
async fn too_few_is_answered_before_the_difficulty_statement_runs() {
    let a = app();
    let l = library(&a);
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &bank_body(&l.alex, l.main, 58, 2000),
    )
    .await;
    assert_eq!(
        r.json(),
        json!({"ok": false, "reason": "tooFew", "have": 57})
    );
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &bank_body(&l.alex, l.main, 57, 2000),
    )
    .await;
    assert_eq!(r.json()["ok"], json!(true));
    // with `user_stats` out of the way, the difficulty statement cannot run: a found source fails, `tooFew` still answers
    raw(&a.db)
        .execute_batch("ALTER TABLE user_stats RENAME TO user_stats_away")
        .expect("rename");
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &bank_body(&l.alex, l.main, 58, 2000),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(
        r.json(),
        json!({"ok": false, "reason": "tooFew", "have": 57})
    );
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &bank_body(&l.alex, l.main, 30, 2000),
    )
    .await;
    assert_eq!(
        r.status, 500,
        "the one difficulty statement needs the table"
    );
    raw(&a.db)
        .execute_batch("ALTER TABLE user_stats_away RENAME TO user_stats")
        .expect("rename back");
    // the difficulty is ONE statement over the returned ids (no per-item query): its plan reads user_stats once
    let c = raw(&a.db);
    let p = plan(
        &c,
        "SELECT question_id, SUM(attempts), SUM(wrong) FROM user_stats WHERE question_id IN (SELECT value FROM json_each('[\"ov1\"]')) GROUP BY question_id",
    );
    assert!(p.contains("user_stats"), "{p}");
}

#[tokio::test]
async fn the_source_refusals_are_all_http_200() {
    let a = app();
    let l = library(&a);
    let refused = |reason: &str| json!({"ok": false, "reason": reason});
    // a session that does not exist
    let mut ghost = bank_body(&l.alex, l.main, 30, 100);
    ghost["sid"] = json!("0".repeat(64));
    let r = post(&a, "/internal/ai/gen/source", &ghost).await;
    assert_eq!((r.status.as_u16(), r.json()), (200, refused("session")));
    // not an administrator and not (AI+ on and may manage banks)
    for w in [&l.bob, &l.cara, &l.dan] {
        let r = post(&a, "/internal/ai/gen/source", &bank_body(w, l.main, 1, 100)).await;
        assert_eq!(
            (r.status.as_u16(), r.json()),
            (200, refused("notAllowed")),
            "{}",
            w.id
        );
    }
    // bank mode: a bank that does not exist, one the caller cannot see, one they see but do not manage -> notFound
    for bank in [9999, l.lone, l.other] {
        let r = post(
            &a,
            "/internal/ai/gen/source",
            &bank_body(&l.alex, bank, 1, 100),
        )
        .await;
        assert_eq!(
            (r.status.as_u16(), r.json()),
            (200, refused("notFound")),
            "bank {bank}"
        );
    }
    // an administrator manages every bank
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &bank_body(&l.admin, l.other, 1, 100),
    )
    .await;
    assert_eq!(r.json()["total"], json!(12));
    // scope / wrong: a missing sheet, a sheet of a bank they cannot see -> notFound; one bad entry refuses the request
    for sheets in [
        vec![(l.main, "Nope")],
        vec![(9999, "Ovens")],
        vec![(l.lone, "Jars")],
        vec![(l.main, "Ovens"), (l.lone, "Jars")],
    ] {
        for source in ["scope", "wrong"] {
            let r = post(
                &a,
                "/internal/ai/gen/source",
                &sheets_body(&l.alex, source, &sheets, 1, 100),
            )
            .await;
            assert_eq!(
                (r.status.as_u16(), r.json()),
                (200, refused("notFound")),
                "{source} {sheets:?}"
            );
        }
    }
    // a scope sheet of a visible bank the caller does not manage is fine (only visibility counts there)
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &sheets_body(&l.alex, "scope", &[(l.other, "Shelves")], 1, 100),
    )
    .await;
    assert_eq!(r.json()["total"], json!(12));
}

#[tokio::test]
async fn the_source_requests_are_validated() {
    let a = app();
    let l = library(&a);
    let good = bank_body(&l.alex, l.main, 30, 100);
    let with = |k: &str, v: Value| {
        let mut b = good.clone();
        b[k] = v;
        b
    };
    let mut cases: Vec<Value> = vec![
        with("extra", json!(1)),
        with("bank", json!(0)),
        with("bank", json!("1")),
        with("min", json!(0)),
        with("max", json!(2001)),
        with("source", json!("all")),
        with("sid", json!("short")),
        with("sid", json!("A".repeat(64))),
    ];
    let mut no_min = good.clone();
    no_min.as_object_mut().expect("o").remove("min");
    cases.push(no_min);
    // the key set follows the source: sheets on a bank request, a bank on a scope request
    cases.push(sheets_body(&l.alex, "bank", &[(l.main, "Ovens")], 1, 10));
    cases.push(json!({"sid": l.alex.sid, "source": "scope", "bank": l.main, "min": 1, "max": 10}));
    cases.push(sheets_body(&l.alex, "scope", &[], 1, 10));
    for body in cases {
        let r = post(&a, "/internal/ai/gen/source", &body).await;
        assert_eq!(r.status, 400, "{body}");
        assert_eq!(r.json(), json!({"error": MSG_BAD_REQUEST}));
    }
}

// ---------------------------------------------------------------- source: scope and wrong

#[tokio::test]
async fn scope_mode_reads_the_listed_sheets_and_skips_ai_banks() {
    let a = app();
    let l = library(&a);
    // listed twice: ignored; listed in any order: the strata are ordered by bank, then first seq
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &sheets_body(
            &l.alex,
            "scope",
            &[
                (l.other, "Shelves"),
                (l.main, "Knives"),
                (l.main, "Ovens"),
                (l.main, "Knives"),
            ],
            30,
            100,
        ),
    )
    .await;
    let j = r.json();
    assert_eq!(j["total"], json!(69));
    let strata: Vec<(i64, &str, i64)> = j["strata"]
        .as_array()
        .expect("strata")
        .iter()
        .map(|s| {
            (
                s["bank"].as_i64().expect("b"),
                s["sheet"].as_str().expect("s"),
                s["n"].as_i64().expect("n"),
            )
        })
        .collect();
    assert_eq!(
        strata,
        vec![
            (l.main, "Ovens", 30),
            (l.main, "Knives", 27),
            (l.other, "Shelves", 12)
        ]
    );
    assert_eq!(
        j["banks"],
        json!([{"id": l.main, "name": "Kitchen basics"}, {"id": l.other, "name": "Pantry"}])
    );
    // a scope holding a sheet of an AI bank is refused (A-OWNER-52b), whatever else is ticked
    let ai = created(
        a.store
            .create_ai_bank(&ai_req(&l.alex, &job_key(1), "AI made", &[l.main], 20))
            .expect("create"),
    );
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &sheets_body(&l.alex, "scope", &[(ai, "AI"), (l.main, "Knives")], 1, 100),
    )
    .await;
    assert_eq!(r.json(), json!({"ok": false, "reason": "aiSource"}));
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &sheets_body(&l.alex, "wrong", &[(ai, "AI"), (l.main, "Knives")], 1, 100),
    )
    .await;
    assert_eq!(r.json(), json!({"ok": false, "reason": "aiSource"}));
    // only AI sheets listed: refused too
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &sheets_body(&l.alex, "scope", &[(ai, "AI")], 1, 100),
    )
    .await;
    assert_eq!(r.json(), json!({"ok": false, "reason": "aiSource"}));
    // bank mode on an AI bank is a refusal of its own
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &bank_body(&l.alex, ai, 1, 100),
    )
    .await;
    assert_eq!(r.json(), json!({"ok": false, "reason": "aiSource"}));
    // an AI bank is never an administrator's source either
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &bank_body(&l.admin, ai, 1, 100),
    )
    .await;
    assert_eq!(r.json(), json!({"ok": false, "reason": "aiSource"}));
}

#[tokio::test]
async fn a_scope_of_sixty_sheets_is_read_sheet_by_sheet() {
    let a = app();
    let l = library(&a);
    let names: Vec<String> = (1..=60).map(|i| format!("Sheet {i}")).collect();
    let sheets: Vec<(i64, &str)> = names.iter().map(|n| (l.many, n.as_str())).collect();
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &sheets_body(&l.alex, "scope", &sheets, 30, 2000),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let j = r.json();
    assert_eq!(j["total"], json!(60));
    assert_eq!(j["strata"].as_array().expect("strata").len(), 60);
    // max below the number of strata: one each for the first `max` strata of equal size
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &sheets_body(&l.alex, "scope", &sheets, 30, 10),
    )
    .await;
    let j = r.json();
    assert_eq!(j["items"].as_array().expect("items").len(), 10);
    let used: BTreeSet<i64> = j["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["s"].as_i64().expect("s"))
        .collect();
    assert_eq!(used.len(), 10, "one item from each of ten different strata");
    // 200 listed sheets are accepted (each must exist and be visible: the 60 real ones repeated are duplicates, ignored)
    let mut long: Vec<(i64, &str)> = Vec::new();
    for _ in 0..4 {
        long.extend(sheets.iter().copied().take(50));
    }
    assert_eq!(long.len(), 200);
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &sheets_body(&l.alex, "scope", &long, 1, 2000),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["total"], json!(50));
}

#[tokio::test]
async fn wrong_mode_uses_the_callers_own_notebook() {
    let a = app();
    let l = library(&a);
    {
        let c = raw(&a.db);
        for i in 1..=12 {
            put_stat(&c, l.alex.id, &format!("ov{i}"), 3, 1, 0);
        }
        put_stat(&c, l.alex.id, "kn1", 3, 1, 1);
        put_stat(&c, l.alex.id, "kn2", 3, 2, 0);
        // not in the notebook: mastered (streak 2), never wrong, a hotspot, a retired question
        put_stat(&c, l.alex.id, "ov13", 5, 1, 2);
        put_stat(&c, l.alex.id, "ov14", 3, 0, 3);
        put_stat(&c, l.alex.id, "hot1", 3, 1, 0);
        put_stat(&c, l.alex.id, "gone1", 3, 1, 0);
        // another user's notebook does not count
        for i in 3..=9 {
            put_stat(&c, l.bob.id, &format!("kn{i}"), 3, 3, 0);
        }
    }
    let both = [(l.main, "Ovens"), (l.main, "Knives")];
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &sheets_body(&l.alex, "wrong", &both, 10, 2000),
    )
    .await;
    let j = r.json();
    assert_eq!(j["total"], json!(14));
    assert_eq!(
        j["strata"],
        json!([
            {"bank": l.main, "sheet": "Ovens", "n": 12, "multi": 4},
            {"bank": l.main, "sheet": "Knives", "n": 2, "multi": 0},
        ])
    );
    let ids: Vec<&str> = j["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["id"].as_str().expect("id"))
        .collect();
    assert!(
        ids.contains(&"ov12")
            && ids.contains(&"kn2")
            && !ids.contains(&"ov13")
            && !ids.contains(&"kn3"),
        "{ids:?}"
    );
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &sheets_body(&l.alex, "wrong", &both, 15, 2000),
    )
    .await;
    assert_eq!(
        r.json(),
        json!({"ok": false, "reason": "tooFew", "have": 14})
    );
    // Hamilton over 12 and 2 with max 10: 9 and 1
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &sheets_body(&l.alex, "wrong", &both, 10, 10),
    )
    .await;
    let items = r.json()["items"].clone();
    assert_eq!(
        items
            .as_array()
            .expect("items")
            .iter()
            .filter(|i| i["s"] == json!(0))
            .count(),
        9
    );
    // the same sheets in scope mode count the whole sheet
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &sheets_body(&l.alex, "scope", &both, 10, 2000),
    )
    .await;
    assert_eq!(r.json()["total"], json!(57));
}

#[tokio::test]
async fn the_source_carries_the_stems_of_earlier_ai_banks_of_the_same_sources() {
    let a = app();
    let l = library(&a);
    let b1 = created(
        a.store
            .create_ai_bank(&ai_req(&l.alex, &job_key(1), "First", &[l.main], 3))
            .expect("one"),
    );
    let b2 = created(
        a.store
            .create_ai_bank(&ai_req(
                &l.alex,
                &job_key(2),
                "Second",
                &[l.main, l.many],
                2,
            ))
            .expect("two"),
    );
    // a bank made from other sources, and one the caller cannot see
    created(
        a.store
            .create_ai_bank(&ai_req(&l.alex, &job_key(3), "Elsewhere", &[l.many], 2))
            .expect("three"),
    );
    assert!(b2 > b1);
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &bank_body(&l.alex, l.main, 30, 2000),
    )
    .await;
    let prev = r.json()["previous"].clone();
    // newest banks first, inside a bank by seq; the stems are the English and the Chinese ones
    assert_eq!(
        prev,
        json!([
            {"en": "Which pan is number 0?", "zh": "Which pan is number 0? 中文"},
            {"en": "Which pan is number 1?", "zh": "Which pan is number 1? 中文"},
            {"en": "Which pan is number 0?", "zh": "Which pan is number 0? 中文"},
            {"en": "Which pan is number 1?", "zh": "Which pan is number 1? 中文"},
            {"en": "Which pan is number 2?", "zh": "Which pan is number 2? 中文"},
        ])
    );
    // a plain user's list never includes a bank outside their scope
    let c = raw(&a.db);
    c.execute(
        "DELETE FROM user_banks WHERE user_id=?1 AND bank_id=?2",
        params![l.alex.id, l.other],
    )
    .expect("revoke");
    // (alex owns the AI banks, so they stay visible to him; an administrator sees them too)
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &bank_body(&l.admin, l.main, 30, 2000),
    )
    .await;
    assert_eq!(r.json()["previous"].as_array().expect("p").len(), 5);
}

#[tokio::test]
async fn previous_stems_are_capped_at_two_hundred() {
    let a = app();
    let l = library(&a);
    for n in 1..=3u32 {
        created(
            a.store
                .create_ai_bank(&ai_req(
                    &l.alex,
                    &job_key(n),
                    &format!("Bank {n}"),
                    &[l.main],
                    100,
                ))
                .expect("create"),
        );
    }
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &bank_body(&l.alex, l.main, 30, 2000),
    )
    .await;
    assert_eq!(r.json()["previous"].as_array().expect("p").len(), 200);
}

// ---------------------------------------------------------------- seeds

#[tokio::test]
async fn seeds_are_the_capped_full_texts_in_request_order() {
    let a = app();
    let l = library(&a);
    {
        let c = raw(&a.db);
        let mut big = six("S".repeat(700).as_str(), false);
        big.options[0].text = "O".repeat(200);
        if let Some(z) = big.zh.as_mut() {
            z.options.as_mut().expect("o")[1] = "令".repeat(200);
        }
        big.explain
            .as_mut()
            .expect("e")
            .en
            .as_mut()
            .expect("en")
            .explanation = "E".repeat(500);
        put_q(&c, "big1", l.main, "Ovens", 900, true, &big);
        put_stat(&c, l.alex.id, "ov5", 10, 9, 0);
    }
    let ids = [
        "ov5", "kn2", "nope", "gone1", "hot1", "old1", "sh1", "sp1", "big1", "ov5",
    ];
    let r = post(
        &a,
        "/internal/ai/gen/seeds",
        &json!({"sid": l.alex.sid, "ids": ids}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let j = r.json();
    assert_eq!(keys(&j), ["ok", "items"]);
    let got: Vec<&str> = j["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["id"].as_str().expect("id"))
        .collect();
    // in request order; an unknown, retired, hotspot, old-layout id is left out; sp1 is a bank alex cannot see; sh1 is visible
    assert_eq!(got, vec!["ov5", "kn2", "sh1", "big1", "ov5"]);
    let first = &j["items"][0];
    assert_eq!(
        keys(first),
        [
            "id",
            "kind",
            "sheet",
            "diff",
            "stem",
            "options",
            "answer",
            "explanation"
        ]
    );
    assert_eq!(first["kind"], json!("single"));
    assert_eq!(first["sheet"], json!("Ovens"));
    assert_eq!(first["diff"], json!("hard"));
    assert_eq!(first["answer"], json!(["B"]));
    assert_eq!(keys(&first["options"][0]), ["key", "en", "zh", "ja"]);
    assert_eq!(
        first["options"][1],
        json!({"key": "B", "en": "Ladle", "zh": "汤勺", "ja": "おたま"})
    );
    assert_eq!(
        first["explanation"],
        json!({"en": "A ladle serves soup.", "zh": "汤勺用来盛汤。", "ja": ""})
    );
    assert_eq!(first["stem"]["zh"], json!("Which oven is number 5? 中文"));
    // the caps: stem 600, option 150, explanation 400 characters per language, the last one `…`
    let big = &j["items"][3];
    let n = |v: &Value| v.as_str().expect("str").chars().count();
    assert_eq!(n(&big["stem"]["en"]), 600);
    assert!(big["stem"]["en"].as_str().expect("s").ends_with('…'));
    assert_eq!(n(&big["options"][0]["en"]), 150);
    assert_eq!(n(&big["options"][1]["zh"]), 150);
    assert_eq!(n(&big["explanation"]["en"]), 400);
    // a multiple-choice question
    let r = post(
        &a,
        "/internal/ai/gen/seeds",
        &json!({"sid": l.alex.sid, "ids": ["ov1"]}),
    )
    .await;
    assert_eq!(r.json()["items"][0]["kind"], json!("multi"));
    assert_eq!(r.json()["items"][0]["answer"], json!(["A", "B"]));
    // an AI bank's question is never a seed
    let ai = created(
        a.store
            .create_ai_bank(&ai_req(&l.alex, &job_key(1), "AI made", &[l.main], 3))
            .expect("create"),
    );
    let q: String = text(
        &raw(&a.db),
        &format!("SELECT id FROM questions WHERE bank_id={ai} ORDER BY seq LIMIT 1"),
    );
    let r = post(
        &a,
        "/internal/ai/gen/seeds",
        &json!({"sid": l.alex.sid, "ids": [q]}),
    )
    .await;
    assert_eq!(r.json(), json!({"ok": true, "items": []}));
}

#[tokio::test]
async fn seeds_refusals_and_validation() {
    let a = app();
    let l = library(&a);
    let r = post(
        &a,
        "/internal/ai/gen/seeds",
        &json!({"sid": "0".repeat(64), "ids": ["ov1"]}),
    )
    .await;
    assert_eq!(r.json(), json!({"ok": false, "reason": "session"}));
    for w in [&l.bob, &l.cara, &l.dan] {
        let r = post(
            &a,
            "/internal/ai/gen/seeds",
            &json!({"sid": w.sid, "ids": ["ov1"]}),
        )
        .await;
        assert_eq!(
            r.json(),
            json!({"ok": false, "reason": "notAllowed"}),
            "{}",
            w.id
        );
    }
    // 100 ids are accepted (unknown ones are left out), 101 are not
    let hundred: Vec<String> = (0..100).map(|i| format!("x{i}")).collect();
    let r = post(
        &a,
        "/internal/ai/gen/seeds",
        &json!({"sid": l.alex.sid, "ids": hundred}),
    )
    .await;
    assert_eq!(r.json(), json!({"ok": true, "items": []}));
    let mut bad: Vec<Value> = vec![
        json!({"sid": l.alex.sid, "ids": (0..101).map(|i| format!("x{i}")).collect::<Vec<_>>()}),
        json!({"sid": l.alex.sid, "ids": []}),
        json!({"sid": l.alex.sid, "ids": ["a b"]}),
        json!({"sid": l.alex.sid, "ids": ["é"]}),
        json!({"sid": l.alex.sid, "ids": [5]}),
        json!({"sid": l.alex.sid, "ids": ["a".repeat(65)]}),
        json!({"sid": l.alex.sid, "ids": "ov1"}),
        json!({"sid": l.alex.sid}),
        json!({"sid": l.alex.sid, "ids": ["ov1"], "x": 1}),
    ];
    bad.push(json!({"ids": ["ov1"]}));
    for body in bad {
        let r = post(&a, "/internal/ai/gen/seeds", &body).await;
        assert_eq!(r.status, 400, "{body}");
    }
}

// ---------------------------------------------------------------- create

#[tokio::test]
async fn create_makes_the_bank_with_its_owner_facts_and_questions() {
    let lines = global_logs();
    let a = app();
    let l = library(&a);
    let name = "Kitchen basics_alexandr_20261112093000";
    let r = post(
        &a,
        "/internal/ai/gen/create",
        &create_body(&l.alex, JOB, name, &[l.main], 20),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let j = r.json();
    assert_eq!(keys(&j), ["ok", "bankId", "name", "created"]);
    assert_eq!(
        (j["ok"].clone(), j["name"].clone(), j["created"].clone()),
        (json!(true), json!(name), json!(true))
    );
    let bank = j["bankId"].as_i64().expect("bankId");
    let c = raw(&a.db);
    let row: (String, String, i64, Option<i64>, String, String, String) = c
        .query_row(
            "SELECT name, filename, legacy_ids, owner_id, ai_job, ai_meta, created_at FROM banks WHERE id=?1",
            params![bank],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .expect("bank row");
    assert_eq!(
        (row.0.as_str(), row.1.as_str(), row.2, row.3, row.4.as_str()),
        (name, "", 0, Some(l.alex.id), JOB)
    );
    assert_eq!(
        row.5,
        format!(
            r#"{{"v":1,"source":"bank","from":[{{"id":{},"name":"Kitchen basics"}}],"author":"alexandra_kitchen","authorId":{},"made":20,"goal":20,"at":"2026-11-12 09:30:00"}}"#,
            l.main, l.alex.id
        )
    );
    // 20 questions, one sheet "AI", seq 0 ..., row 1 ..., active, no import row
    assert_eq!(
        count(
            &c,
            &format!("SELECT COUNT(*) FROM questions WHERE bank_id={bank}")
        ),
        20
    );
    assert_eq!(count(&c, &format!("SELECT COUNT(DISTINCT sheet) FROM questions WHERE bank_id={bank} AND sheet='AI' AND active=1")), 1);
    assert_eq!(
        count(
            &c,
            &format!("SELECT MIN(seq) FROM questions WHERE bank_id={bank}")
        ),
        0
    );
    assert_eq!(
        count(
            &c,
            &format!("SELECT MAX(seq) FROM questions WHERE bank_id={bank}")
        ),
        19
    );
    assert_eq!(
        count(
            &c,
            &format!("SELECT MIN(row_no) FROM questions WHERE bank_id={bank}")
        ),
        1
    );
    assert_eq!(
        count(
            &c,
            &format!("SELECT MAX(row_no) FROM questions WHERE bank_id={bank}")
        ),
        20
    );
    assert_eq!(
        count(
            &c,
            &format!("SELECT COUNT(*) FROM imports WHERE bank_id={bank}")
        ),
        0
    );
    // a stored question is the six-column form the store builds, with the bank-scoped id
    let want = AiQuestion {
        multi: false,
        stem: Tri {
            en: "Which pan is number 0 in the new set?".into(),
            zh: "Which pan is number 0 in the new set? 中文".into(),
            ja: "Which pan is number 0 in the new set? 日本語".into(),
        },
        options: (0..4)
            .map(|k| Tri {
                en: format!("Pan 0{k}"),
                zh: format!("Pan 0{k} 中文"),
                ja: format!("Pan 0{k} 日本語"),
            })
            .collect(),
        answer: vec!["B".into()],
        explanation: Tri {
            en: "Pan 0 fits because it is made for that.".into(),
            zh: "Pan 0 fits because it is made for that. 中文".into(),
            ja: "Pan 0 fits because it is made for that. 日本語".into(),
        },
    };
    let id = ai_question_id(bank, "AI", &want);
    let data: String = text(
        &c,
        &format!("SELECT data FROM questions WHERE id='{id}' AND seq=0"),
    );
    let stored: Question = serde_json::from_str(&data).expect("question");
    assert_eq!(stored, build_question(&want));
    assert!(stored.is_six_column());
    assert_eq!(stored.translation, "Which pan is number 0 in the new set? 中文\nA. Pan 00 中文\nB. Pan 01 中文\nC. Pan 02 中文\nD. Pan 03 中文");
    assert_eq!(
        stored.explanation,
        "Pan 0 fits because it is made for that. 中文"
    );
    // the third question is multiple-choice
    let third: String = text(
        &c,
        &format!("SELECT data FROM questions WHERE bank_id={bank} AND seq=2"),
    );
    let third: Question = serde_json::from_str(&third).expect("question");
    assert_eq!(
        (third.kind, third.answer),
        (Kind::Multi, Answer::Letters(vec!["A".into(), "C".into()]))
    );
    // the log line: English, the username, no question text
    let msgs: Vec<String> = lines
        .since(None)
        .lines
        .into_iter()
        .map(|x| x.message)
        .collect();
    assert!(
        msgs.contains(&format!(
            r#"AI bank "{name}" created for user "alexandra_kitchen" (20 questions)"#
        )),
        "{msgs:?}"
    );
    assert!(msgs.iter().all(|m| !m.contains("Which pan")), "{msgs:?}");
    // the author (a bank manager) sees it as the owner and may manage it; the administrator sees it too
    let r = api(&a, "GET", "/api/banks", &l.alex.cookie, None)
        .await
        .json();
    let mine = r["banks"]
        .as_array()
        .expect("banks")
        .iter()
        .find(|b| b["id"] == json!(bank))
        .expect("listed")
        .clone();
    assert_eq!(mine["manageable"], json!(true));
    assert_eq!(mine["total"], json!(20));
    assert_eq!(
        mine["sheets"],
        json!([{"name": "AI", "usable": 20, "inactive": 0}])
    );
}

#[tokio::test]
async fn create_is_idempotent_by_job_and_names_are_made_unique() {
    let a = app();
    let l = library(&a);
    let body = create_body(&l.alex, JOB, "Same name", &[l.main], 20);
    let first = post(&a, "/internal/ai/gen/create", &body).await.json();
    assert_eq!(first["created"], json!(true));
    let c = raw(&a.db);
    let (banks, questions) = (
        count(&c, "SELECT COUNT(*) FROM banks"),
        count(&c, "SELECT COUNT(*) FROM questions"),
    );
    // the same job key again: the same bank, nothing written
    let again = post(&a, "/internal/ai/gen/create", &body).await.json();
    assert_eq!(
        again,
        json!({"ok": true, "bankId": first["bankId"], "name": "Same name", "created": false})
    );
    assert_eq!(
        (
            count(&c, "SELECT COUNT(*) FROM banks"),
            count(&c, "SELECT COUNT(*) FROM questions")
        ),
        (banks, questions)
    );
    // even for a body that differs (a retry after the account changed its mind)
    let other = post(
        &a,
        "/internal/ai/gen/create",
        &create_body(&l.alex, JOB, "Other name", &[l.main], 20),
    )
    .await
    .json();
    assert_eq!(other["created"], json!(false));
    assert_eq!(other["name"], json!("Same name"));
    // another job with the same name: " (2)", " (3)"
    for (n, want) in [(2, "Same name (2)"), (3, "Same name (3)")] {
        let j = post(
            &a,
            "/internal/ai/gen/create",
            &create_body(&l.alex, &job_key(n), "Same name", &[l.main], 20),
        )
        .await
        .json();
        assert_eq!(
            (j["created"].clone(), j["name"].clone()),
            (json!(true), json!(want))
        );
    }
    // a name that an imported bank already has
    let j = post(
        &a,
        "/internal/ai/gen/create",
        &create_body(&l.alex, &job_key(9), "Pantry", &[l.main], 20),
    )
    .await
    .json();
    assert_eq!(j["name"], json!("Pantry (2)"));
    // the lookup (API-I4) finds the bank of a job key and nothing else
    let r = post(&a, "/internal/ai/gen/lookup", &json!({"job": JOB})).await;
    assert_eq!(
        r.json(),
        json!({"ok": true, "bank": {"id": first["bankId"], "name": "Same name", "made": 20, "goal": 20}})
    );
    let r = post(&a, "/internal/ai/gen/lookup", &json!({"job": job_key(777)})).await;
    assert_eq!(r.json(), json!({"ok": true, "bank": null}));
    for bad in [
        json!({}),
        json!({"job": "abc"}),
        json!({"job": JOB, "x": 1}),
        json!({"job": "G".repeat(32)}),
    ] {
        assert_eq!(
            post(&a, "/internal/ai/gen/lookup", &bad).await.status,
            400,
            "{bad}"
        );
    }
}

#[tokio::test]
async fn create_checks_the_account_itself() {
    let a = app();
    let l = library(&a);
    let refused = |reason: &str| json!({"ok": false, "reason": reason});
    let create = |w: &Who, stamp: &str, job: &str| {
        let mut b = create_body(w, job, "Checked", &[l.main], 3);
        b["stamp"] = json!(stamp);
        b
    };
    // no user with that id and stamp
    let r = post(
        &a,
        "/internal/ai/gen/create",
        &create(&l.alex, &"0".repeat(32), &job_key(1)),
    )
    .await;
    assert_eq!((r.status.as_u16(), r.json()), (200, refused("user")));
    let mut ghost = create(&l.alex, &l.alex.stamp, &job_key(1));
    ghost["user"] = json!(99_999);
    assert_eq!(
        post(&a, "/internal/ai/gen/create", &ghost).await.json(),
        refused("user")
    );
    // the id of another user with this user's stamp does not pass either
    let mut mixed = create(&l.alex, &l.alex.stamp, &job_key(1));
    mixed["user"] = json!(l.bob.id);
    assert_eq!(
        post(&a, "/internal/ai/gen/create", &mixed).await.json(),
        refused("user")
    );
    // not an administrator and not (AI+ on and may manage banks)
    for w in [&l.bob, &l.cara, &l.dan] {
        let r = post(
            &a,
            "/internal/ai/gen/create",
            &create(w, &w.stamp, &job_key(2)),
        )
        .await;
        assert_eq!(r.json(), refused("notAllowed"), "{}", w.id);
    }
    // an administrator may (AI+ is not stored for them)
    let r = post(
        &a,
        "/internal/ai/gen/create",
        &create(&l.admin, &l.admin.stamp, &job_key(3)),
    )
    .await;
    assert_eq!(r.json()["created"], json!(true));
    // a disabled account
    raw(&a.db)
        .execute(
            "UPDATE users SET disabled=1 WHERE id=?1",
            params![l.alex.id],
        )
        .expect("disable");
    let r = post(
        &a,
        "/internal/ai/gen/create",
        &create(&l.alex, &l.alex.stamp, &job_key(4)),
    )
    .await;
    assert_eq!(r.json(), refused("user"));
    raw(&a.db)
        .execute(
            "UPDATE users SET disabled=0 WHERE id=?1",
            params![l.alex.id],
        )
        .expect("enable");
    // the permission was taken away
    raw(&a.db)
        .execute(
            "UPDATE users SET can_manage_banks=0 WHERE id=?1",
            params![l.alex.id],
        )
        .expect("take away");
    let r = post(
        &a,
        "/internal/ai/gen/create",
        &create(&l.alex, &l.alex.stamp, &job_key(5)),
    )
    .await;
    assert_eq!(r.json(), refused("notAllowed"));
    raw(&a.db)
        .execute(
            "UPDATE users SET can_manage_banks=1 WHERE id=?1",
            params![l.alex.id],
        )
        .expect("give back");
    // a bank made before the suspension below: a retry of its job is still answered with it (API-I3 step 1 first)
    let made = post(
        &a,
        "/internal/ai/gen/create",
        &create(&l.alex, &l.alex.stamp, &job_key(7)),
    )
    .await
    .json();
    assert_eq!(made["created"], json!(true));
    // suspended in AI settings (the last heartbeat) is refused too, without writing anything
    let hb = format!(
        r#"{{"version":"1.1.0","enabled":true,"suspended":[{}],"stopping":false}}"#,
        l.alex.id
    );
    let r = internal(&a, "POST", "/internal/ai/heartbeat", Some(TOKEN), &hb).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let banks = count(&raw(&a.db), "SELECT COUNT(*) FROM banks");
    let r = post(
        &a,
        "/internal/ai/gen/create",
        &create(&l.alex, &l.alex.stamp, &job_key(6)),
    )
    .await;
    assert_eq!(r.json(), refused("notAllowed"));
    assert_eq!(count(&raw(&a.db), "SELECT COUNT(*) FROM banks"), banks);
    let again = post(
        &a,
        "/internal/ai/gen/create",
        &create(&l.alex, &l.alex.stamp, &job_key(7)),
    )
    .await
    .json();
    assert_eq!(
        again,
        json!({"ok": true, "bankId": made["bankId"], "name": made["name"], "created": false})
    );
    assert_eq!(count(&raw(&a.db), "SELECT COUNT(*) FROM banks"), banks);
    // nothing was written by any refusal
    assert_eq!(
        count(
            &raw(&a.db),
            "SELECT COUNT(*) FROM banks WHERE ai_job IS NOT NULL"
        ),
        2,
        "only the administrator's and the one made before the suspension"
    );
}

#[tokio::test]
async fn the_create_body_is_validated_with_the_generators_limits() {
    let a = app();
    let l = library(&a);
    let good = create_body(&l.alex, JOB, "Valid", &[l.main], 3);
    let bad = |mutate: &dyn Fn(&mut Value)| {
        let mut b = good.clone();
        mutate(&mut b);
        b
    };
    let cases: Vec<(&str, Value)> = vec![
        ("an extra key", bad(&|b| b["extra"] = json!(1))),
        ("fewer questions than goal", bad(&|b| b["goal"] = json!(4))),
        ("more questions than goal", bad(&|b| b["goal"] = json!(2))),
        (
            "a stem of 801 characters",
            bad(&|b| b["questions"][0]["stem"]["en"] = json!("x".repeat(801))),
        ),
        (
            "an option of 301 characters",
            bad(&|b| b["questions"][1]["options"][0]["zh"] = json!("令".repeat(301))),
        ),
        (
            "an explanation of 1201 characters",
            bad(&|b| b["questions"][2]["explanation"]["ja"] = json!("e".repeat(1201))),
        ),
        (
            "two options",
            bad(&|b| b["questions"][0]["options"] = json!([tri("a"), tri("b")])),
        ),
        (
            "seven options",
            bad(&|b| {
                b["questions"][0]["options"] =
                    json!((0..7).map(|i| tri(&format!("o{i}"))).collect::<Vec<_>>())
            }),
        ),
        (
            "an answer letter beyond the options",
            bad(&|b| b["questions"][0]["answer"] = json!(["E"])),
        ),
        (
            "a single with two letters",
            bad(&|b| b["questions"][0]["answer"] = json!(["A", "B"])),
        ),
        (
            "a name with a bidi override",
            bad(&|b| b["name"] = json!("Evil\u{202E}name")),
        ),
        (
            "a name with a zero-width space",
            bad(&|b| b["name"] = json!("Zero\u{200B}width")),
        ),
        (
            "a name over 120 characters",
            bad(&|b| b["name"] = json!("n".repeat(121))),
        ),
        (
            "from with a duplicate",
            bad(&|b| b["from"] = json!([l.main, l.main])),
        ),
        ("a bad job", bad(&|b| b["job"] = json!("XYZ"))),
        ("a bad timestamp", bad(&|b| b["at"] = json!("2026-11-12"))),
    ];
    for (what, body) in cases {
        let r = post(&a, "/internal/ai/gen/create", &body).await;
        assert_eq!(r.status, 400, "{what}: {}", r.text());
        assert_eq!(r.json(), json!({"error": MSG_BAD_REQUEST}), "{what}");
    }
    // 200 source banks are accepted (a scope may span as many banks as sheets)
    let from: Vec<i64> = (1..=200).collect();
    let r = post(
        &a,
        "/internal/ai/gen/create",
        &bad(&|b| b["from"] = json!(from)),
    )
    .await;
    assert_eq!(r.json()["created"], json!(true), "{}", r.text());
    assert_eq!(
        count(
            &raw(&a.db),
            "SELECT COUNT(*) FROM banks WHERE ai_job IS NOT NULL"
        ),
        1,
        "nothing else was written"
    );
}

#[tokio::test]
async fn a_hundred_questions_at_the_maximum_lengths_fit_and_ten_mib_plus_one_does_not() {
    let a = app();
    let l = library(&a);
    // 100 questions, every text at its maximum, four-byte characters: about 4.6 MB
    let wide = |tag: &str, n: usize| -> Value {
        let pad = "😀".repeat(n.saturating_sub(tag.chars().count()));
        json!({"en": format!("{tag}{pad}"), "zh": format!("{tag}{pad}"), "ja": format!("{tag}{pad}")})
    };
    let questions: Vec<Value> = (0..100)
        .map(|i| {
            json!({
                "kind": "multi",
                "stem": wide(&format!("{i:03}"), 800),
                "options": (0..6).map(|k| wide(&format!("{i:03}o{k}"), 300)).collect::<Vec<_>>(),
                "answer": ["A", "B", "C", "D", "E"],
                "explanation": wide("x", 1200),
            })
        })
        .collect();
    let body = json!({
        "job": JOB, "user": l.alex.id, "stamp": l.alex.stamp, "name": "Maximal", "source": "scope", "from": [l.main],
        "goal": 100, "at": "2026-11-12 09:30:00", "sheet": "AI", "questions": questions,
    })
    .to_string();
    assert!(
        body.len() > 4_400_000 && body.len() < 4_800_000,
        "{}",
        body.len()
    );
    let r = internal(&a, "POST", "/internal/ai/gen/create", Some(TOKEN), &body).await;
    assert_eq!(
        r.status,
        200,
        "{}",
        r.text().chars().take(200).collect::<String>()
    );
    assert_eq!(r.json()["created"], json!(true));
    let c = raw(&a.db);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM questions WHERE bank_id IN (SELECT id FROM banks WHERE ai_job IS NOT NULL)"), 100);
    // 10 MiB + 1 byte: 413 (like the other token routes' limits), nothing written
    let mut over = String::from(r#"{"a":1}"#);
    over.push_str(&" ".repeat(10 * 1024 * 1024 + 1 - over.len()));
    assert_eq!(over.len(), 10 * 1024 * 1024 + 1);
    let r = internal(&a, "POST", "/internal/ai/gen/create", Some(TOKEN), &over).await;
    assert_eq!(r.status, 413);
    // exactly 10 MiB is read (and refused as a bad request, not as too large)
    let mut edge = String::from(r#"{"a":1}"#);
    edge.push_str(&" ".repeat(10 * 1024 * 1024 - edge.len()));
    let r = internal(&a, "POST", "/internal/ai/gen/create", Some(TOKEN), &edge).await;
    assert_eq!(r.status, 400);
    // the other gen routes keep the 64 KiB limit
    let mut big = String::from(r#"{"job":""#);
    big.push_str(&"a".repeat(70 * 1024));
    big.push_str(r#""}"#);
    assert_eq!(
        internal(&a, "POST", "/internal/ai/gen/lookup", Some(TOKEN), &big)
            .await
            .status,
        413
    );
}

// ---------------------------------------------------------------- sharing

/// The users that see bank `bank` through `viewer_of` (the rule the sharing guard must mirror).
fn sees(a: &App, user: i64, bank: i64) -> bool {
    a.store
        .viewer(user)
        .expect("viewer")
        .scope
        .allows(Some(bank))
}

#[test]
fn the_sharing_switch_is_on_by_default_and_the_decision_is_recorded() {
    // owner decision A-OWNER-13d: AI banks are shared like any new bank
    const { assert!(AI_BANKS_AUTO_SHARE) };
}

#[tokio::test]
async fn a_new_ai_bank_reaches_only_the_include_later_users_who_see_every_source_bank() {
    let a = app();
    let l = library(&a);
    let c = raw(&a.db);
    // sources: `main` and `other`. Users with "include later": dan (sees main only), eve (sees nothing), gina (sees both),
    // hank (sees main and owns other as a bank manager), ivy (manager, owns neither, sees both), jon (sees both, flag off),
    // and a second administrator.
    let gina = make_user(
        &a,
        "gina",
        Role::User,
        &[l.main, l.other],
        true,
        false,
        false,
    );
    let hank = make_user(&a, "hank", Role::User, &[l.main], true, true, false);
    c.execute(
        "UPDATE banks SET owner_id=?1 WHERE id=?2",
        params![hank.id, l.other],
    )
    .expect("owner");
    let ivy = make_user(&a, "ivy", Role::User, &[l.main, l.other], true, true, false);
    let jon = make_user(
        &a,
        "jon",
        Role::User,
        &[l.main, l.other],
        false,
        false,
        false,
    );
    let root = make_user(&a, "root2", Role::Admin, &[], true, false, false);
    let from = [l.main, l.other];
    let bank = created(
        a.store
            .create_ai_bank(&ai_req(&l.alex, &job_key(1), "Shared", &from, 3))
            .expect("create"),
    );
    let got = banks_with_access(&c, bank);
    for u in [gina.id, hank.id, ivy.id, root.id] {
        assert!(
            got.contains(&u),
            "user {u} sees both sources and has the flag on: {got:?}"
        );
    }
    for u in [l.dan.id, l.eve.id, jon.id, l.alex.id, l.bob.id, l.cara.id] {
        assert!(
            !got.contains(&u),
            "user {u}: dan and eve cannot see `other`, jon has the flag off: {got:?}"
        );
    }
    // the rule equals `viewer_of` for every user of the library: with the flag on, a row exactly when every source is seen
    let flagged: Vec<i64> = {
        let mut st = c
            .prepare("SELECT id FROM users WHERE new_banks <> 0")
            .expect("prepare");
        let rows = st.query_map([], |r| r.get::<_, i64>(0)).expect("query");
        rows.map(|r| r.expect("row")).collect()
    };
    for u in flagged {
        // (`viewer_of` is evaluated after the bank exists: the new bank itself is not one of `from`)
        let sees_all = from.iter().all(|b| sees(&a, u, *b));
        // a row grants the user the bank (the author is exempt: alex is not flagged)
        assert_eq!(got.contains(&u), sees_all, "user {u}");
        assert_eq!(
            sees(&a, u, bank),
            sees_all,
            "user {u} sees the new bank exactly when they see every source"
        );
    }
    // the author and administrators see it without a row; unrelated users do not
    assert!(sees(&a, l.alex.id, bank) && sees(&a, 1, bank));
    assert!(!sees(&a, l.dan.id, bank) && !sees(&a, l.eve.id, bank) && !sees(&a, jon.id, bank));
    // the known-bank mark moved with the creation (the next open's rollback fix-up would otherwise share it)
    assert_eq!(
        text(&c, "SELECT value FROM meta WHERE key='banks_known'"),
        bank.to_string()
    );
    // a source bank deleted before the creation is visible to nobody: then only the author and administrators get it
    let gone = put_bank(&c, "Short lived", 1);
    grant(&c, gina.id, gone);
    a.store.delete_bank(gone, false).expect("delete");
    let bank2 = created(
        a.store
            .create_ai_bank(&ai_req(&l.alex, &job_key(2), "Orphan", &[l.main, gone], 3))
            .expect("create"),
    );
    let admins: BTreeSet<i64> = {
        let mut st = c
            .prepare("SELECT id FROM users WHERE role='admin' AND new_banks <> 0")
            .expect("prepare");
        let rows = st.query_map([], |r| r.get::<_, i64>(0)).expect("query");
        rows.map(|r| r.expect("row")).collect()
    };
    assert!(admins.contains(&root.id));
    assert_eq!(
        banks_with_access(&c, bank2),
        admins,
        "only the administrators with the flag: every plain user lacks the deleted source bank"
    );
}

#[tokio::test]
async fn with_the_switch_off_nobody_gets_a_row_but_the_bank_is_known() {
    let a = app();
    let l = library(&a);
    let c = raw(&a.db);
    let bank = created(
        a.store
            .create_ai_bank_shared(
                &ai_req(&l.alex, &job_key(1), "Private", &[l.main], 3),
                false,
            )
            .expect("create"),
    );
    assert!(banks_with_access(&c, bank).is_empty());
    assert!(
        sees(&a, l.alex.id, bank) && sees(&a, 1, bank),
        "the author and administrators still see it"
    );
    assert!(!sees(&a, l.dan.id, bank));
    assert_eq!(
        text(&c, "SELECT value FROM meta WHERE key='banks_known'"),
        bank.to_string(),
        "known either way"
    );
    // with the switch on the same request shares it
    let bank2 = created(
        a.store
            .create_ai_bank_shared(&ai_req(&l.alex, &job_key(2), "Shared", &[l.main], 3), true)
            .expect("create"),
    );
    assert!(banks_with_access(&c, bank2).contains(&l.dan.id));
    assert_eq!(
        text(&c, "SELECT value FROM meta WHERE key='banks_known'"),
        bank2.to_string()
    );
    // an empty source list shares with nobody
    let bank3 = created(
        a.store
            .create_ai_bank_shared(&ai_req(&l.alex, &job_key(3), "Rootless", &[], 3), true)
            .expect("create"),
    );
    assert!(banks_with_access(&c, bank3).is_empty());
}

#[tokio::test]
async fn an_administrator_removes_an_ai_bank_from_a_user_like_any_other_bank() {
    let a = app();
    let l = library(&a);
    let bank = created(
        a.store
            .create_ai_bank(&ai_req(&l.alex, &job_key(1), "Shared", &[l.main], 3))
            .expect("create"),
    );
    // dan has "include later" and sees the main bank: the AI bank came with the default
    let listed = |cookie: String| {
        let a = &a;
        async move {
            let banks = api(a, "GET", "/api/banks", &cookie, None).await.json();
            let ids: Vec<i64> = banks["banks"]
                .as_array()
                .expect("banks")
                .iter()
                .map(|b| b["id"].as_i64().expect("id"))
                .collect();
            let all = api(a, "GET", "/api/bank", &cookie, None).await.json();
            let in_bank = all["questions"]
                .as_array()
                .expect("q")
                .iter()
                .filter(|q| q["bank"] == json!(bank))
                .count();
            (ids, in_bank)
        }
    };
    let (ids, n) = listed(l.dan.cookie.clone()).await;
    assert!(ids.contains(&bank), "{ids:?}");
    assert_eq!(n, 3);
    // the administrator saves dan's banks without the AI bank (User management's ordinary field)
    let body = json!({"id": l.dan.id, "banks": [l.main]}).to_string();
    let r = api(
        &a,
        "POST",
        "/api/users/update",
        &l.admin.cookie,
        Some(&body),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let (ids, n) = listed(l.dan.cookie.clone()).await;
    assert!(!ids.contains(&bank), "{ids:?}");
    assert_eq!(n, 0, "gone from /api/bank too");
    // the author and the administrator still see it
    assert!(listed(l.alex.cookie.clone()).await.0.contains(&bank));
    assert!(listed(l.admin.cookie.clone()).await.0.contains(&bank));
    // another bank created later does not bring it back (only an administrator's own assignment does); dan still has the flag
    let next = created(
        a.store
            .create_ai_bank(&ai_req(&l.alex, &job_key(2), "Next", &[l.main], 3))
            .expect("create"),
    );
    let (ids, _) = listed(l.dan.cookie.clone()).await;
    assert!(ids.contains(&next) && !ids.contains(&bank), "{ids:?}");
    // and an administrator may give it back
    let body = json!({"id": l.dan.id, "banks": [l.main, bank]}).to_string();
    assert_eq!(
        api(
            &a,
            "POST",
            "/api/users/update",
            &l.admin.cookie,
            Some(&body)
        )
        .await
        .status,
        200
    );
    assert!(listed(l.dan.cookie.clone()).await.0.contains(&bank));
}

// ---------------------------------------------------------------- the bank list, imports, deletion, study

#[tokio::test]
async fn the_bank_list_names_the_source_banks_the_viewer_can_see() {
    let a = app();
    let l = library(&a);
    let bank = created(
        a.store
            .create_ai_bank(&ai_req(
                &l.alex,
                &job_key(1),
                "From two",
                &[l.main, l.other],
                20,
            ))
            .expect("create"),
    );
    let at = |who: &Who| {
        let a = &a;
        let cookie = who.cookie.clone();
        async move { api(a, "GET", "/api/banks", &cookie, None).await.json() }
    };
    let find = |banks: &Value, id: i64| {
        banks["banks"]
            .as_array()
            .expect("banks")
            .iter()
            .find(|b| b["id"] == json!(id))
            .cloned()
    };
    // the administrator: every source bank; the key comes last, after `manageable`
    let admin_list = at(&l.admin).await;
    let mine = find(&admin_list, bank).expect("listed");
    let ks = keys(&mine);
    assert_eq!(ks[ks.len() - 2..], ["manageable", "ai"]);
    assert_eq!(
        mine["ai"],
        json!({"source": "bank", "from": ["Kitchen basics", "Pantry"], "author": "alexandra_kitchen", "made": 20, "goal": 20, "at": "2026-11-12 09:30:00"})
    );
    assert!(mine["ai"].get("authorId").is_none() && !mine.to_string().contains("authorId"));
    // another bank has no `ai`
    assert!(find(&admin_list, l.main).expect("main").get("ai").is_none());
    // a user who sees only one of the sources is not told the other's name (here: gave them the AI bank by hand)
    raw(&a.db)
        .execute(
            "INSERT INTO user_banks(user_id, bank_id) VALUES(?1, ?2)",
            params![l.dan.id, bank],
        )
        .expect("grant");
    let dan_list = at(&l.dan).await;
    let seen = find(&dan_list, bank).expect("listed");
    assert_eq!(
        seen["ai"]["from"],
        json!(["Kitchen basics"]),
        "Pantry is outside dan's scope"
    );
    assert_eq!(
        seen["ai"]["author"],
        json!("alexandra_kitchen"),
        "the author is shown to everyone who sees the bank"
    );
    assert_eq!(seen["manageable"], json!(false));
    assert!(!dan_list.to_string().contains("Pantry"));
}

#[tokio::test]
async fn an_excel_import_into_an_ai_bank_is_refused_at_preview_and_at_confirm() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let store = Arc::new(Store::open_with_clock(&db, dbc::fixed(dbc::FIXED_NOW)).expect("store"));
    let engine = Arc::new(RealEngine::new(store.clone()));
    let mut state = AppState::new(apic::default_config(), store.clone(), engine, None);
    // TEST-ONLY: the demo fixture is a 3-column workbook (production code refuses old layouts)
    Arc::get_mut(&mut state.0)
        .expect("a new state is not shared yet")
        .old_layouts = xexam_backend::services::OldLayouts::AllowForTests;
    let app = apic::TestApp {
        router: apic::signed_in_router(&state),
        state,
        engine: apic::FakeEngine::new(),
        shutdown_rx: None,
        tmp: None,
    };
    let demo = std::fs::read(dbc::baseline("fixtures/demo.xlsx")).expect("demo.xlsx");
    let upload = |target: Option<String>| {
        let app = &app;
        let demo = demo.clone();
        async move {
            let mut hs: Vec<(&str, Vec<u8>)> = vec![
                ("content-type", b"application/octet-stream".to_vec()),
                ("x-filename", b"demo.xlsx".to_vec()),
            ];
            if let Some(t) = target {
                hs.push(("x-bank-target", t.into_bytes()));
            }
            let refs: Vec<(&str, &[u8])> = hs.iter().map(|(k, v)| (*k, v.as_slice())).collect();
            apic::post_raw(app, "/api/upload", demo, &refs).await
        }
    };
    // a normal bank to update, and an AI bank
    let p = upload(Some("new".into())).await;
    assert_eq!(p.status, 200, "{}", p.text());
    let tok = p.json()["token"].as_str().expect("token").to_string();
    let done = apic::post_json(
        &app,
        "/api/import/confirm",
        &format!(r#"{{"token": "{tok}"}}"#),
    )
    .await;
    assert_eq!(done.status, 200, "{}", done.text());
    let normal = done.json()["bankId"].as_i64().expect("bankId");
    let admin = store.viewer(1).expect("viewer");
    assert!(admin.admin);
    let uid = store.user_by_id(1).expect("u").expect("admin").uid;
    let req = AiBankReq {
        user: 1,
        stamp: uid,
        ..ai_req_for(1, &job_key(1))
    };
    let ai = created(store.create_ai_bank(&req).expect("create"));
    // preview: 400, nothing staged
    let r = upload(Some(ai.to_string())).await;
    assert_eq!(
        (r.status.as_u16(), r.json()),
        (400, json!({"error": MSG_AI_BANK_IMPORT})),
        "{}",
        r.text()
    );
    // an upload that was staged for a normal bank when it became an AI bank is refused at confirm, nothing written
    let p = upload(Some(normal.to_string())).await;
    assert_eq!(p.status, 200, "{}", p.text());
    let tok = p.json()["token"].as_str().expect("token").to_string();
    let c = raw(&db);
    c.execute(
        "UPDATE banks SET ai_job=?1, ai_meta='{}' WHERE id=?2",
        params![job_key(2), normal],
    )
    .expect("make it an AI bank");
    drop(c);
    let after_update = dbc::db_bytes(&db);
    let r = apic::post_json(
        &app,
        "/api/import/confirm",
        &format!(r#"{{"token": "{tok}"}}"#),
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.json()),
        (400, json!({"error": MSG_AI_BANK_IMPORT})),
        "{}",
        r.text()
    );
    assert_eq!(dbc::db_bytes(&db), after_update, "nothing written");
    // the store refuses it as well
    let err = store
        .apply_import_as(&admin, ImportTarget::Bank(ai), "x.xlsx", &[], &[])
        .expect_err("refused");
    assert!(
        matches!(err, xexam_backend::error::DbError::AiBank),
        "{err:?}"
    );
    // deleting an AI bank works like any other
    let gone = store
        .delete_bank(ai, false)
        .expect("delete")
        .expect("found");
    assert_eq!(gone.id, ai);
    assert_eq!(
        count(
            &raw(&db),
            &format!("SELECT COUNT(*) FROM questions WHERE bank_id={ai}")
        ),
        0
    );
    drop(app);
}

fn ai_req_for(user: i64, job: &str) -> AiBankReq {
    let t = |s: &str| Tri {
        en: s.into(),
        zh: format!("{s} 中文"),
        ja: format!("{s} 日本語"),
    };
    AiBankReq {
        job: job.to_string(),
        user,
        stamp: String::new(),
        name: "Admin AI bank".into(),
        kind: GenKind::Scope,
        from: vec![],
        goal: 1,
        at: "2026-11-12 09:30:00".into(),
        sheet: "AI".into(),
        questions: vec![AiQuestion {
            multi: false,
            stem: t("Which pan is it?"),
            options: vec![t("Wok"), t("Pot"), t("Pan")],
            answer: vec!["C".into()],
            explanation: t("A pan."),
        }],
        file: None,
        skipped: Vec::new(),
    }
}

#[tokio::test]
async fn the_questions_of_an_ai_bank_are_ordinary_questions_in_the_bank_records_and_the_notebook() {
    let a = app();
    let l = library(&a);
    let bank = created(
        a.store
            .create_ai_bank(&ai_req(&l.alex, &job_key(1), "Study me", &[l.main], 3))
            .expect("create"),
    );
    // /api/bank: the three questions, sheet AI, the bank id, in seq order
    let all = api(&a, "GET", "/api/bank", &l.alex.cookie, None)
        .await
        .json();
    let mine: Vec<&Value> = all["questions"]
        .as_array()
        .expect("q")
        .iter()
        .filter(|q| q["bank"] == json!(bank))
        .collect();
    assert_eq!(mine.len(), 3);
    assert_eq!(mine[0]["sheet"], json!("AI"));
    assert_eq!(mine[0]["stem"], json!("Which pan is number 0?"));
    assert_eq!(mine[0]["lang"], json!("en"));
    assert!(all["sheets"]
        .as_array()
        .expect("s")
        .iter()
        .any(|s| s["name"] == json!("AI") && s["usable"] == json!(3)));
    // an exam and a practice session over them are saved, and a wrong answer reaches the notebook
    let ids: Vec<String> = mine
        .iter()
        .map(|q| q["id"].as_str().expect("id").to_string())
        .collect();
    let body = format!(
        r#"{{"results":[{{"id":"{}","correct":true,"answer":["B"]}},{{"id":"{}","correct":false,"answer":["A"]}},{{"id":"{}","correct":false}}],"sheets":["AI"],"mode":"all","seconds":9,"limit":600,"end":"timeUp"}}"#,
        ids[0], ids[1], ids[2]
    );
    let r = api(&a, "POST", "/api/attempt", &l.alex.cookie, Some(&body)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let p = r.json();
    assert_eq!(
        p["questions"][&ids[1]]["wrong"],
        json!(1),
        "answered wrong: in the notebook"
    );
    assert!(
        p["questions"].get(ids[2].as_str()).is_none(),
        "unanswered: not in the notebook"
    );
    let rec = api(
        &a,
        "GET",
        &format!("/api/record?id={}", p["recordId"]),
        &l.alex.cookie,
        None,
    )
    .await
    .json();
    assert_eq!(
        rec["record"]["banks"],
        json!([{"id": bank, "name": "Study me", "deleted": false}])
    );
    assert_eq!(rec["record"]["unanswered"], json!(1));
    assert_eq!(rec["items"][0]["question"]["bank"], json!(bank));
    // the bank list counts its own records; deleting the bank with records removes them
    let banks = api(&a, "GET", "/api/banks", &l.alex.cookie, None)
        .await
        .json();
    let listed = banks["banks"]
        .as_array()
        .expect("banks")
        .iter()
        .find(|b| b["id"] == json!(bank))
        .expect("bank")
        .clone();
    assert_eq!(
        (listed["records"].clone(), listed["studied"].clone()),
        (json!(1), json!(2))
    );
    let del = api(
        &a,
        "POST",
        "/api/banks/delete",
        &l.alex.cookie,
        Some(&json!({"id": bank, "withRecords": true}).to_string()),
    )
    .await;
    assert_eq!(del.status, 200, "{}", del.text());
    let c = raw(&a.db);
    assert_eq!(
        count(&c, &format!("SELECT COUNT(*) FROM banks WHERE id={bank}")),
        0
    );
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM attempts WHERE kind='exam'"),
        0
    );
}

#[tokio::test]
async fn the_bank_scope_read_leaves_the_source_banks_of_ai_banks_out_for_users_who_cannot_see_them()
{
    // DR1-T8: `ai.from` never names a source bank outside the viewer's scope; administrators get all of them
    let a = app();
    let l = library(&a);
    let bank = created(
        a.store
            .create_ai_bank(&ai_req(&l.alex, &job_key(1), "From main", &[l.main], 3))
            .expect("create"),
    );
    let hide = make_user(&a, "hide", Role::User, &[bank], false, false, false);
    let r = api(&a, "GET", "/api/banks", &hide.cookie, None)
        .await
        .json();
    let entry = r["banks"]
        .as_array()
        .expect("banks")
        .iter()
        .find(|b| b["id"] == json!(bank))
        .expect("listed")
        .clone();
    assert_eq!(
        entry["ai"]["from"],
        json!([]),
        "the viewer cannot see Kitchen basics"
    );
    assert!(!r.to_string().contains("Kitchen basics"));
    let r = api(&a, "GET", "/api/banks", &l.admin.cookie, None)
        .await
        .json();
    let entry = r["banks"]
        .as_array()
        .expect("banks")
        .iter()
        .find(|b| b["id"] == json!(bank))
        .expect("listed")
        .clone();
    assert_eq!(entry["ai"]["from"], json!(["Kitchen basics"]));
}

// ---------------------------------------------------------------- v1.1 file import (API-I5, API-I3a) and sheet deletion

fn file_q(i: usize, options: usize, multi: bool, all: bool) -> Value {
    let letters = ["A", "B", "C", "D", "E", "F", "G", "H"];
    let answer: Vec<&str> = if multi {
        letters[..if all { options } else { 2 }].to_vec()
    } else {
        vec!["B"]
    };
    json!({
        "kind": if multi { "multi" } else { "single" },
        "stem": tri(&format!("Which whisk is number {i} in the file?")),
        "options": (0..options).map(|k| tri(&format!("Whisk {i}{k}"))).collect::<Vec<_>>(),
        "answer": answer,
        "explanation": tri(&format!("Whisk {i} is the one.")),
    })
}

fn file_body(w: &Who, job: &str, mode: &str, n: usize, skipped: &[i64], goal: usize) -> Value {
    let mut b = json!({
        "job": job, "user": w.id, "stamp": w.stamp, "name": "Kitchen notes_alexandr_20261112093000",
        "source": "file", "from": [], "file": {"name": "Kitchen notes.pdf", "type": "pdf"}, "mode": mode,
        "goal": goal, "at": "2026-11-12 09:30:00", "sheet": "AI",
        "questions": (0..n).map(|i| file_q(i, 4, false, false)).collect::<Vec<_>>(),
    });
    if mode == "questions" {
        b["skipped"] = json!(skipped);
    }
    b
}

fn find_bank(list: &Value, id: i64) -> Value {
    list["banks"]
        .as_array()
        .expect("banks")
        .iter()
        .find(|b| b["id"] == json!(id))
        .cloned()
        .expect("listed")
}

#[tokio::test]
async fn import_check_answers_the_permission_and_binds_the_session() {
    let a = app();
    let l = library(&a);
    let check = |sid: &str| json!({"sid": sid});
    let r = post(&a, "/internal/ai/import/check", &check(&l.alex.sid)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(
        r.json(),
        json!({"ok": true, "user": {"id": l.alex.id, "username": "alexandra_kitchen", "role": "user"}})
    );
    let r = post(&a, "/internal/ai/import/check", &check(&l.admin.sid)).await;
    assert_eq!(r.json()["user"]["role"], json!("admin"));
    // AI+ but may not manage banks, and managing but no AI+: not allowed
    for w in [&l.bob, &l.cara] {
        let r = post(&a, "/internal/ai/import/check", &check(&w.sid)).await;
        assert_eq!(r.json(), json!({"ok": false, "reason": "notAllowed"}));
    }
    // no such session
    let r = post(&a, "/internal/ai/import/check", &check(&"0".repeat(64))).await;
    assert_eq!(r.json(), json!({"ok": false, "reason": "session"}));
    // exactly one key, a string; wrong shapes are 400
    for bad in [
        json!({}),
        json!({"sid": l.alex.sid, "x": 1}),
        json!({"sid": 5}),
    ] {
        let r = post(&a, "/internal/ai/import/check", &bad).await;
        assert_eq!(r.status, 400, "{bad}");
    }
    // token gate like the other routes
    let r = internal(
        &a,
        "POST",
        "/internal/ai/import/check",
        None,
        &check(&l.alex.sid).to_string(),
    )
    .await;
    assert_eq!(r.status, 404);
    // nothing written
    assert_eq!(
        count(
            &raw(&a.db),
            "SELECT COUNT(*) FROM banks WHERE ai_job IS NOT NULL"
        ),
        0
    );
}

#[tokio::test]
async fn import_check_reports_whether_a_typed_bank_name_is_taken() {
    let a = app();
    let l = library(&a);
    let existing: String = raw(&a.db)
        .query_row("SELECT name FROM banks LIMIT 1", [], |r| r.get(0))
        .expect("a bank");
    let r = post(
        &a,
        "/internal/ai/import/check",
        &json!({"sid": l.alex.sid, "name": format!("  {existing} ")}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["nameTaken"], json!(true));
    let r = post(
        &a,
        "/internal/ai/import/check",
        &json!({"sid": l.alex.sid, "name": "A name nobody uses (v11)"}),
    )
    .await;
    assert_eq!(r.json()["nameTaken"], json!(false));
    // a name of the wrong type is a 400; without a name the answer has no nameTaken
    let r = post(
        &a,
        "/internal/ai/import/check",
        &json!({"sid": l.alex.sid, "name": 5}),
    )
    .await;
    assert_eq!(r.status, 400);
    let r = post(&a, "/internal/ai/import/check", &json!({"sid": l.alex.sid})).await;
    assert!(r.json().get("nameTaken").is_none());
}

#[tokio::test]
async fn a_file_import_in_questions_mode_makes_the_bank_with_the_file_facts() {
    let lines = global_logs();
    let a = app();
    let l = library(&a);
    // 3 of 5 converted; numbers 2 and 4 skipped
    let job = job_key(70);
    let r = post(
        &a,
        "/internal/ai/gen/create",
        &file_body(&l.alex, &job, "questions", 3, &[2, 4], 5),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["created"], json!(true));
    let bank = r.json()["bankId"].as_i64().expect("bank");
    let c = raw(&a.db);
    let meta: String = text(&c, &format!("SELECT ai_meta FROM banks WHERE id={bank}"));
    assert_eq!(
        meta,
        format!(
            r#"{{"v":1,"source":"file","from":[],"file":{{"name":"Kitchen notes.pdf","type":"pdf"}},"mode":"questions","skipped":[2,4],"author":"alexandra_kitchen","authorId":{},"made":3,"goal":5,"at":"2026-11-12 09:30:00"}}"#,
            l.alex.id
        )
    );
    // the ids carry the row: scoped_id(bank, make_id("AI<row>", stem, first option))
    let ids: Vec<String> = {
        let mut st = c
            .prepare(&format!(
                "SELECT id FROM questions WHERE bank_id={bank} ORDER BY seq"
            ))
            .expect("prepare");
        st.query_map([], |r| r.get::<_, String>(0))
            .expect("rows")
            .map(|r| r.expect("id"))
            .collect()
    };
    assert_eq!(ids.len(), 3);
    for (i, id) in ids.iter().enumerate() {
        let want = xexam_backend::database::store::scoped_id(
            bank,
            &xexam_backend::parser::make_id(
                &format!("AI{}", i + 1),
                &format!("Which whisk is number {i} in the file?"),
                &format!("Whisk {i}0"),
            ),
        );
        assert_eq!(*id, want);
    }
    // the bank list: key order source, from, file, mode, skipped, author, made, goal, at
    let list = api(&a, "GET", "/api/banks", &l.admin.cookie, None)
        .await
        .json();
    let mine = find_bank(&list, bank);
    assert_eq!(
        keys(&mine["ai"]),
        ["source", "from", "file", "mode", "skipped", "author", "made", "goal", "at"]
    );
    assert_eq!(mine["ai"]["file"], json!("Kitchen notes.pdf"));
    assert_eq!(mine["ai"]["skipped"], json!([2, 4]));
    assert_eq!(mine["ai"]["made"], json!(3));
    // SEC-10: the log line names the bank id, never the name (it holds the file stem)
    let msgs: Vec<String> = lines
        .since(None)
        .lines
        .into_iter()
        .map(|x| x.message)
        .collect();
    assert!(
        msgs.contains(&format!(
            r#"AI bank #{bank} created for user "alexandra_kitchen" (3 questions, file import)"#
        )),
        "{msgs:?}"
    );
    assert!(
        msgs.iter()
            .all(|m| !m.contains("Kitchen notes") && !m.contains("Which whisk")),
        "no file name and no text in a log line"
    );
    // idempotent by job key
    let again = post(
        &a,
        "/internal/ai/gen/create",
        &file_body(&l.alex, &job, "questions", 3, &[2, 4], 5),
    )
    .await;
    assert_eq!(
        (
            again.json()["created"].clone(),
            again.json()["bankId"].clone()
        ),
        (json!(false), json!(bank))
    );
}

#[tokio::test]
async fn a_file_import_in_material_mode_has_no_skipped_key() {
    let a = app();
    let l = library(&a);
    let r = post(
        &a,
        "/internal/ai/gen/create",
        &file_body(&l.alex, &job_key(71), "material", 20, &[], 20),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let bank = r.json()["bankId"].as_i64().expect("bank");
    let meta: Value = serde_json::from_str(&text(
        &raw(&a.db),
        &format!("SELECT ai_meta FROM banks WHERE id={bank}"),
    ))
    .expect("meta");
    assert_eq!(
        keys(&meta),
        ["v", "source", "from", "file", "mode", "author", "authorId", "made", "goal", "at"]
    );
    let list = api(&a, "GET", "/api/banks", &l.admin.cookie, None)
        .await
        .json();
    let mine = find_bank(&list, bank);
    assert_eq!(
        keys(&mine["ai"]),
        ["source", "from", "file", "mode", "author", "made", "goal", "at"]
    );
    assert_eq!(mine["ai"]["mode"], json!("material"));
}

#[tokio::test]
async fn the_file_create_body_is_validated_by_mode() {
    let a = app();
    let l = library(&a);
    let counter = std::cell::Cell::new(100u32);
    let status = |b: Value| {
        counter.set(counter.get() + 1);
        let mut b = b;
        b["job"] = json!(job_key(counter.get()));
        let a = &a;
        async move { post(a, "/internal/ai/gen/create", &b).await.status.as_u16() }
    };
    let w = &l.alex;
    // keys: file and mode only for a file; skipped only for questions mode
    assert_eq!(status(file_body(w, "x", "questions", 3, &[], 3)).await, 200);
    assert_eq!(
        status(file_body(w, "x", "material", 20, &[], 20)).await,
        200
    );
    let mut b = file_body(w, "x", "material", 20, &[], 20);
    b["skipped"] = json!([]);
    assert_eq!(status(b).await, 400, "skipped in material mode");
    let mut b = file_body(w, "x", "questions", 3, &[], 3);
    b.as_object_mut().expect("o").remove("skipped");
    assert_eq!(
        status(b).await,
        400,
        "skipped is required in questions mode"
    );
    let mut b = create_body(w, "x", "Plain", &[l.main], 3);
    b["file"] = json!({"name": "a.pdf", "type": "pdf"});
    assert_eq!(status(b).await, 400, "file with a bank source");
    let mut b = create_body(w, "x", "Plain", &[l.main], 3);
    b["mode"] = json!("questions");
    assert_eq!(status(b).await, 400, "mode with a bank source");
    let mut b = file_body(w, "x", "questions", 3, &[], 3);
    b["from"] = json!([l.main]);
    assert_eq!(status(b).await, 400, "from must be empty for a file");
    let mut b = file_body(w, "x", "questions", 3, &[], 3);
    b["mode"] = json!("both");
    assert_eq!(status(b).await, 400);
    // file facts
    for bad in [
        json!({"name": "", "type": "pdf"}),
        json!({"name": "a\u{202E}b.pdf", "type": "pdf"}),
        json!({"name": "a\nb.pdf", "type": "pdf"}),
        json!({"name": "x".repeat(201), "type": "pdf"}),
        json!({"name": "a.doc", "type": "doc"}),
        json!({"name": "a.pdf"}),
        json!({"name": "a.pdf", "type": "pdf", "size": 3}),
    ] {
        let mut b = file_body(w, "x", "questions", 3, &[], 3);
        b["file"] = bad.clone();
        assert_eq!(status(b).await, 400, "{bad}");
    }
    let mut b = file_body(w, "x", "questions", 3, &[], 3);
    b["file"] = json!({"name": "令".repeat(200), "type": "docx"});
    assert_eq!(status(b).await, 200, "a 200-character CJK file name");
    // skipped rules: questions + skipped = goal, distinct sorted integers 1..=goal
    let cases: [(usize, &[i64], usize, &str); 7] = [
        (3, &[4], 5, "3 + 1 != 5"),
        (3, &[2, 2], 5, "duplicate"),
        (3, &[4, 2], 5, "unsorted"),
        (3, &[0, 2], 5, "zero"),
        (3, &[2, 6], 5, "above goal"),
        (0, &[1, 2], 2, "none converted"),
        (3, &[], 4, "3 + 0 != 4"),
    ];
    for (n, skipped, goal, why) in cases {
        assert_eq!(
            status(file_body(w, "x", "questions", n, skipped, goal)).await,
            400,
            "{why}"
        );
    }
    // material mode: exactly goal questions
    assert_eq!(
        status(file_body(w, "x", "material", 19, &[], 20)).await,
        400
    );
    // option counts and answers
    let one = |q: Value, mode: &str| {
        let mut b = file_body(w, "x", mode, 1, &[], 1);
        b["questions"] = json!([q]);
        b
    };
    for (opts, ok) in [(1, false), (2, true), (8, true), (9, false)] {
        assert_eq!(
            status(one(file_q(0, opts, false, false), "questions")).await,
            if ok { 200 } else { 400 },
            "{opts} options in questions mode"
        );
    }
    // material mode keeps the generator's limits (3 ... 6 options, never every option right)
    for (opts, multi, all) in [(2, false, false), (7, false, false), (4, true, true)] {
        let mut b = file_body(w, "x", "material", 20, &[], 20);
        b["questions"][0] = file_q(0, opts, multi, all);
        assert_eq!(
            status(b).await,
            400,
            "{opts} {multi} {all} in material mode"
        );
    }
    assert_eq!(
        status(one(file_q(0, 4, true, true), "questions")).await,
        200,
        "multi with every option right"
    );
    assert_eq!(
        status(one(file_q(0, 4, true, false), "questions")).await,
        200
    );
    let mut q = file_q(0, 4, true, false);
    q["answer"] = json!(["B"]);
    assert_eq!(
        status(one(q, "questions")).await,
        400,
        "multi needs 2 or more"
    );
    // text lengths: stem 2 000 ok / 2 001 refused (questions mode), 800 / 801 in material mode
    let long = |chars: usize| {
        let mut q = file_q(0, 4, false, false);
        q["stem"] =
            json!({"en": "s".repeat(chars), "zh": "s".repeat(chars), "ja": "s".repeat(chars)});
        q
    };
    assert_eq!(status(one(long(2000), "questions")).await, 200);
    assert_eq!(status(one(long(2001), "questions")).await, 400);
    let mut b = file_body(w, "x", "material", 20, &[], 20);
    b["questions"][0] = long(800);
    assert_eq!(status(b).await, 200);
    let mut b = file_body(w, "x", "material", 20, &[], 20);
    b["questions"][0] = long(801);
    assert_eq!(status(b).await, 400);
    let mut q = file_q(0, 4, false, false);
    q["options"][0] = json!({"en": "o".repeat(401), "zh": "o", "ja": "o"});
    assert_eq!(status(one(q, "questions")).await, 400, "option 401");
    let mut q = file_q(0, 4, false, false);
    q["explanation"] = json!({"en": "e".repeat(1501), "zh": "e", "ja": "e"});
    assert_eq!(status(one(q, "questions")).await, 400, "explanation 1 501");
    // generated banks still refuse a repeated question
    let mut b = create_body(w, "x", "Rep", &[l.main], 2);
    b["questions"] = json!([gen_q(0), gen_q(0)]);
    assert_eq!(status(b).await, 400, "generated banks refuse duplicates");
}

#[tokio::test]
async fn a_repeated_file_question_gets_distinct_stored_ids() {
    let a = app();
    let l = library(&a);
    let mut b = file_body(&l.alex, &job_key(80), "questions", 2, &[], 2);
    b["questions"] = json!([file_q(0, 4, false, false), file_q(0, 4, false, false)]);
    let r = post(&a, "/internal/ai/gen/create", &b).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let bank = r.json()["bankId"].as_i64().expect("bank");
    assert_eq!(
        count(
            &raw(&a.db),
            &format!("SELECT COUNT(DISTINCT id) FROM questions WHERE bank_id={bank}")
        ),
        2
    );
}

#[tokio::test]
async fn the_largest_file_meta_fits_the_check_and_the_worst_payload_is_accepted() {
    let a = app();
    let l = library(&a);
    // SEC-13: a 200-character CJK file name and 99 skipped numbers (1 converted of 100)
    let skipped: Vec<i64> = (2..=100).collect();
    let mut b = file_body(&l.alex, &job_key(90), "questions", 1, &skipped, 100);
    b["file"] = json!({"name": "令".repeat(200), "type": "docx"});
    let r = post(&a, "/internal/ai/gen/create", &b).await;
    assert_eq!(r.status, 200, "{}", r.text());
    // the worst valid file payload: 100 questions, 8 options, every text at the converted maximum, four-byte characters
    let wide = |tag: &str, n: usize| -> Value {
        let pad = "😀".repeat(n.saturating_sub(tag.chars().count()));
        json!({"en": format!("{tag}{pad}"), "zh": format!("{tag}{pad}"), "ja": format!("{tag}{pad}")})
    };
    let questions: Vec<Value> = (0..100)
        .map(|i| {
            json!({
                "kind": "multi",
                "stem": wide(&format!("{i:03}"), 2000),
                "options": (0..8).map(|k| wide(&format!("{i:03}o{k}"), 400)).collect::<Vec<_>>(),
                "answer": ["A", "B", "C", "D", "E", "F", "G", "H"],
                "explanation": wide("x", 1500),
            })
        })
        .collect();
    let body = json!({
        "job": job_key(91), "user": l.alex.id, "stamp": l.alex.stamp, "name": "Worst file", "source": "file", "from": [],
        "file": {"name": "令".repeat(200), "type": "pdf"}, "mode": "questions", "skipped": [],
        "goal": 100, "at": "2026-11-12 09:30:00", "sheet": "AI", "questions": questions,
    })
    .to_string();
    assert!(
        body.len() > 8_000_000 && body.len() < 10 * 1024 * 1024,
        "{}",
        body.len()
    );
    let r = internal(&a, "POST", "/internal/ai/gen/create", Some(TOKEN), &body).await;
    assert_eq!(
        r.status,
        200,
        "{}",
        r.text().chars().take(200).collect::<String>()
    );
    assert_eq!(r.json()["created"], json!(true));
}

fn file_req(w: &Who, job: &str) -> AiBankReq {
    AiBankReq {
        kind: GenKind::File,
        from: vec![],
        file: Some((
            AiFile {
                name: "Notes.md".into(),
                ty: "md".into(),
            },
            AiMode::Material,
        )),
        ..ai_req(w, job, "Notes_alexandr_20261112093000", &[], 20)
    }
}

#[tokio::test]
async fn file_banks_are_shared_with_the_include_later_users_both_ways() {
    let a = app();
    let l = library(&a);
    // from is [], so every "include later" user gets the bank while the switch is on
    let bank = created(
        a.store
            .create_ai_bank(&file_req(&l.alex, &job_key(95)))
            .expect("create"),
    );
    let got = banks_with_access(&raw(&a.db), bank);
    assert!(
        got.contains(&l.dan.id) && got.contains(&l.eve.id),
        "{got:?}"
    );
    // switch off: nobody but the author and the administrators
    let off = created(
        a.store
            .create_ai_bank_shared(&file_req(&l.alex, &job_key(96)), false)
            .expect("create"),
    );
    let got = banks_with_access(&raw(&a.db), off);
    assert!(
        !got.contains(&l.dan.id) && !got.contains(&l.eve.id),
        "{got:?}"
    );
    assert!(sees(&a, l.alex.id, off) && sees(&a, 1, off));
}

#[tokio::test]
async fn create_refuses_a_user_who_may_no_longer_make_banks() {
    let a = app();
    let l = library(&a);
    // SEC-8: bank management was removed during the wait
    raw(&a.db)
        .execute(
            "UPDATE users SET can_manage_banks=0 WHERE id=?1",
            params![l.alex.id],
        )
        .expect("update");
    let r = post(
        &a,
        "/internal/ai/gen/create",
        &file_body(&l.alex, &job_key(97), "material", 20, &[], 20),
    )
    .await;
    assert_eq!(r.json(), json!({"ok": false, "reason": "notAllowed"}));
    assert_eq!(
        count(
            &raw(&a.db),
            "SELECT COUNT(*) FROM banks WHERE ai_job IS NOT NULL"
        ),
        0
    );
}

#[tokio::test]
async fn the_source_route_refuses_a_file_source() {
    let a = app();
    let l = library(&a);
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &json!({"sid": l.alex.sid, "source": "file", "bank": l.main, "min": 1, "max": 10}),
    )
    .await;
    assert_eq!(r.status, 400);
}

#[tokio::test]
async fn an_ai_bank_is_never_a_source_even_with_a_visible_id() {
    // M-9: API-I1 bank mode -> aiSource; API-I2 seeds of AI-bank questions are not returned
    let a = app();
    let l = library(&a);
    let ai = created(
        a.store
            .create_ai_bank(&ai_req(&l.alex, &job_key(98), "AI one", &[l.main], 20))
            .expect("create"),
    );
    let r = post(
        &a,
        "/internal/ai/gen/source",
        &bank_body(&l.alex, ai, 1, 100),
    )
    .await;
    assert_eq!(r.json(), json!({"ok": false, "reason": "aiSource"}));
    let ids: Vec<String> = {
        let c = raw(&a.db);
        let mut st = c
            .prepare(&format!(
                "SELECT id FROM questions WHERE bank_id={ai} LIMIT 3"
            ))
            .expect("p");
        st.query_map([], |r| r.get::<_, String>(0))
            .expect("q")
            .map(|r| r.expect("id"))
            .collect()
    };
    let r = post(
        &a,
        "/internal/ai/gen/seeds",
        &json!({"sid": l.alex.sid, "ids": ids}),
    )
    .await;
    assert_eq!(r.json()["items"], json!([]), "{}", r.text());
}

// ---- C-B1: POST /api/banks/sheets/delete

async fn sheets_delete(a: &App, who: &Who, body: Value) -> Resp {
    api(
        a,
        "POST",
        "/api/banks/sheets/delete",
        &who.cookie,
        Some(&body.to_string()),
    )
    .await
}

fn sheet_count(c: &Connection, bank: i64, sheet: &str) -> i64 {
    count(
        c,
        &format!("SELECT COUNT(*) FROM questions WHERE bank_id={bank} AND sheet='{sheet}'"),
    )
}

#[tokio::test]
async fn deleting_sheets_removes_their_questions_and_answers_like_bank_delete() {
    let a = app();
    let l = library(&a);
    let c = raw(&a.db);
    put_stat(&c, l.alex.id, "kn1", 4, 2, 0);
    put_stat(&c, l.alex.id, "ov1", 4, 2, 0);
    put_stat(&c, l.dan.id, "kn2", 3, 1, 0);
    drop(c);
    let r = sheets_delete(&a, &l.alex, json!({"id": l.main, "sheets": ["Knives"]})).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(keys(&r.json()), ["banks", "bank", "progress"]);
    let c = raw(&a.db);
    assert_eq!(sheet_count(&c, l.main, "Knives"), 0);
    assert_eq!(
        sheet_count(&c, l.main, "Ovens"),
        33,
        "the other sheet is untouched"
    );
    // wrong-answer statistics of the deleted questions are gone for everyone, the others stay
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM user_stats WHERE question_id IN ('kn1','kn2')"
        ),
        0
    );
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM user_stats WHERE question_id='ov1'"
        ),
        1
    );
    // the bank list no longer has the sheet
    let mine = find_bank(&r.json(), l.main);
    let names: Vec<String> = mine["sheets"]
        .as_array()
        .expect("sheets")
        .iter()
        .map(|s| s["name"].as_str().expect("n").to_string())
        .collect();
    assert_eq!(names, ["Ovens"]);
}

#[tokio::test]
async fn deleting_sheets_keeps_attempts_unless_asked_and_drops_the_emptied_ones() {
    let a = app();
    let l = library(&a);
    let c = raw(&a.db);
    for id in [1, 2] {
        c.execute(
            "INSERT INTO attempts(id, at, sheets, mode, total, correct, percent, seconds, user_id, banks) \
             VALUES(?1, '2026-01-01 10:00', '[\"Knives\"]', 'all', 2, 1, 50, 5, ?2, '[]')",
            params![id, l.alex.id],
        )
        .expect("attempt");
    }
    let item = |attempt: i64, seq: i64, q: &str| {
        c.execute(
            "INSERT INTO attempt_items(attempt_id, seq, question_id, answer, correct) VALUES(?1, ?2, ?3, '[0]', 1)",
            params![attempt, seq, q],
        )
        .expect("item");
    };
    item(1, 0, "kn1");
    item(1, 1, "kn2");
    item(2, 0, "kn3");
    item(2, 1, "ov1");
    drop(c);
    // withRecords false: attempts and items stay as stored
    let r = sheets_delete(
        &a,
        &l.alex,
        json!({"id": l.main, "sheets": ["Knives"], "withRecords": false}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let c = raw(&a.db);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempts"), 2);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempt_items"), 4);
    // the sheet comes back (a later import), and is deleted again with its records
    put_q(
        &c,
        "kn1",
        l.main,
        "Knives",
        900,
        true,
        &six("Which knife again?", false),
    );
    put_q(
        &c,
        "kn2",
        l.main,
        "Knives",
        901,
        true,
        &six("Which blade again?", false),
    );
    put_q(
        &c,
        "kn3",
        l.main,
        "Knives",
        902,
        true,
        &six("Which cleaver again?", false),
    );
    drop(c);
    let r = sheets_delete(
        &a,
        &l.alex,
        json!({"id": l.main, "sheets": ["Knives"], "withRecords": true}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let c = raw(&a.db);
    // attempt 1 lost every item and is gone; attempt 2 keeps its Ovens item
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempts"), 1);
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM attempt_items WHERE question_id='ov1'"
        ),
        1
    );
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempt_items"), 1);
}

#[tokio::test]
async fn deleting_sheets_is_for_manageable_banks_only_and_refuses_the_whole_bank_and_ai_banks() {
    let a = app();
    let l = library(&a);
    let main_rows = |c: &Connection| {
        count(
            c,
            &format!("SELECT COUNT(*) FROM questions WHERE bank_id={}", l.main),
        )
    };
    let before = main_rows(&raw(&a.db));
    // not manageable (alex sees but does not own "Pantry"), unknown bank, unknown sheet: 404
    for body in [
        json!({"id": l.other, "sheets": ["Shelves"]}),
        json!({"id": 9999, "sheets": ["Knives"]}),
        json!({"id": l.main, "sheets": ["Nope"]}),
        json!({"id": l.main, "sheets": ["Knives", "Nope"]}),
    ] {
        let r = sheets_delete(&a, &l.alex, body.clone()).await;
        assert_eq!(r.status, 404, "{body}: {}", r.text());
    }
    // bob may not manage banks at all
    let r = sheets_delete(&a, &l.bob, json!({"id": l.main, "sheets": ["Knives"]})).await;
    assert_eq!(r.status, 403, "{}", r.text());
    // every sheet named: 400 with the new message
    let r = sheets_delete(
        &a,
        &l.alex,
        json!({"id": l.main, "sheets": ["Knives", "Ovens"]}),
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.json()),
        (400, json!({"error": "要删除全部工作表，请删除整个题库。"}))
    );
    // an AI bank
    let ai = created(
        a.store
            .create_ai_bank(&ai_req(&l.alex, &job_key(99), "AI two", &[l.main], 20))
            .expect("create"),
    );
    let r = sheets_delete(&a, &l.alex, json!({"id": ai, "sheets": ["AI"]})).await;
    assert_eq!(
        (r.status.as_u16(), r.json()),
        (
            400,
            json!({"error": "AI 题集只有一个工作表，请删除整个题库。"})
        )
    );
    // body shape: exactly the three keys, 1 ... 200 distinct strings
    for body in [
        json!({"id": l.main}),
        json!({"id": l.main, "sheets": []}),
        json!({"id": l.main, "sheets": ["Knives", "Knives"]}),
        json!({"id": l.main, "sheets": [1]}),
        json!({"id": l.main, "sheets": ["Knives"], "withRecords": 1}),
        json!({"id": l.main, "sheets": ["Knives"], "x": 1}),
        json!({"id": "3", "sheets": ["Knives"]}),
        json!({"sheets": ["Knives"]}),
    ] {
        let r = sheets_delete(&a, &l.alex, body.clone()).await;
        assert_eq!(r.status, 400, "{body}: {}", r.text());
    }
    let too_many: Vec<String> = (0..201).map(|i| format!("S{i}")).collect();
    let r = sheets_delete(&a, &l.alex, json!({"id": l.many, "sheets": too_many})).await;
    assert_eq!(r.status, 400);
    assert_eq!(
        main_rows(&raw(&a.db)),
        before,
        "nothing was written by any refusal"
    );
    // the administrator may delete from any bank; 59 of 60 sheets is allowed
    let names: Vec<String> = (1..=59).map(|i| format!("Sheet {i}")).collect();
    let r = sheets_delete(&a, &l.admin, json!({"id": l.many, "sheets": names})).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(
        count(
            &raw(&a.db),
            &format!("SELECT COUNT(*) FROM questions WHERE bank_id={}", l.many)
        ),
        1
    );
    // signed out: 401
    let r = api(
        &a,
        "POST",
        "/api/banks/sheets/delete",
        "",
        Some(&json!({"id": l.main, "sheets": ["Knives"]}).to_string()),
    )
    .await;
    assert_eq!(r.status, 401);
}

#[tokio::test]
async fn deleting_a_sheet_drops_the_pending_excel_import_of_that_bank() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let store = Arc::new(Store::open_with_clock(&db, dbc::fixed(dbc::FIXED_NOW)).expect("store"));
    let engine = Arc::new(RealEngine::new(store.clone()));
    let mut state = AppState::new(apic::default_config(), store.clone(), engine, None);
    Arc::get_mut(&mut state.0)
        .expect("a new state is not shared yet")
        .old_layouts = xexam_backend::services::OldLayouts::AllowForTests;
    let app = apic::TestApp {
        router: apic::signed_in_router(&state),
        state,
        engine: apic::FakeEngine::new(),
        shutdown_rx: None,
        tmp: None,
    };
    let c = raw(&db);
    let bank = put_bank(&c, "Two sheets", 1);
    put_q(
        &c,
        "t1",
        bank,
        "One",
        1,
        true,
        &six("Which tin is one?", false),
    );
    put_q(
        &c,
        "t2",
        bank,
        "Two",
        2,
        true,
        &six("Which tin is two?", false),
    );
    drop(c);
    let demo = std::fs::read(dbc::baseline("fixtures/demo.xlsx")).expect("demo.xlsx");
    let target = bank.to_string();
    let p = apic::post_raw(
        &app,
        "/api/upload",
        demo,
        &[
            ("content-type", b"application/octet-stream"),
            ("x-filename", b"demo.xlsx"),
            ("x-bank-target", target.as_bytes()),
        ],
    )
    .await;
    assert_eq!(p.status, 200, "{}", p.text());
    let tok = p.json()["token"].as_str().expect("token").to_string();
    let r = apic::post_json(
        &app,
        "/api/banks/sheets/delete",
        &json!({"id": bank, "sheets": ["Two"]}).to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = apic::post_json(
        &app,
        "/api/import/confirm",
        &format!(r#"{{"token": "{tok}"}}"#),
    )
    .await;
    assert_ne!(r.status, 200, "the staged upload is gone: {}", r.text());
    assert_eq!(
        count(
            &raw(&db),
            &format!("SELECT COUNT(*) FROM questions WHERE bank_id={bank}")
        ),
        1
    );
}
