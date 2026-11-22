//! v1.3 backend-misc: `POST /internal/ai/gen/corpus` (three languages, paging, caps, access, `tooFew` / `tooMany`), the ETag
//! and 304 of `GET /api/bank` / `GET /api/banks`, and the idempotent `POST /api/attempt`. Made-up kitchen questions and
//! users only; an obviously fake token.
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
use xexam_backend::database::{Store, UserChange};
use xexam_backend::models::BankAccess;
use xexam_backend::models::{Answer, Explain, ExplainText, Kind, Opt, Question, Zh};
use xexam_backend::services::RealEngine;

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";
const NGINX_HOST: &str = "backend:8765";
const TOKEN: &str = "fake-internal-token-for-tests-0123456789";

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
    let state = AppState::new(apic::default_config(), store.clone(), engine, None);
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

struct Who {
    id: i64,
    cookie: String,
    sid: String,
}

fn sign_in(a: &App, id: i64) -> Who {
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
    a.store
        .set_display_name(id, &format!("User {}", id))
        .expect("name");
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
    }
}

fn make_user(a: &App, name: &str, manage: bool, ai: bool) -> Who {
    let id = match a
        .store
        .create_user_opts(
            name,
            "placeholder-hash",
            Role::User,
            &BankAccess::Only(vec![]),
            false,
            manage,
            ai,
        )
        .expect("create")
    {
        UserChange::Done(id) => id,
        other => panic!("{other:?}"),
    };
    sign_in(a, id)
}

async fn post(a: &App, path: &str, body: &Value) -> Resp {
    let text = body.to_string();
    let len = text.len().to_string();
    let hs: Vec<(&str, &[u8])> = vec![
        ("content-type", b"application/json"),
        ("content-length", len.as_bytes()),
        ("x-xexam-internal-token", TOKEN.as_bytes()),
    ];
    send(
        &a.router,
        build("POST", path, Some(NGINX_HOST), &hs, Body::from(text)),
    )
    .await
}

async fn api(
    a: &App,
    method: &str,
    path: &str,
    cookie: &str,
    extra: &[(&str, &str)],
    body: Option<&str>,
) -> Resp {
    let mut hs: Vec<(&str, Vec<u8>)> = vec![("cookie", cookie.as_bytes().to_vec())];
    for (k, v) in extra {
        hs.push((k, v.as_bytes().to_vec()));
    }
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

fn header(r: &Resp, name: &str) -> Option<String> {
    r.headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
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

/// A made-up six-column question: English base, Chinese and Japanese translations, English and Chinese explanations.
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
        translation: String::new(),
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
                mnemonic: String::new(),
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

/// A manager `alex` (AI+), a plain `dan` (no AI+) and a bank of `n` questions in sheet `Ovens`, all visible to both.
struct Lib {
    alex: Who,
    dan: Who,
    bank: i64,
}

fn library(a: &App, n: usize) -> Lib {
    let alex = make_user(a, "alex", true, true);
    let dan = make_user(a, "dan", false, false);
    let c = raw(&a.db);
    let bank = put_bank(&c, "Kitchen basics", alex.id);
    grant(&c, alex.id, bank);
    grant(&c, dan.id, bank);
    for i in 1..=n {
        put_q(
            &c,
            &format!("ov{i:03}"),
            bank,
            "Ovens",
            i64::try_from(i).expect("n"),
            true,
            &six(&format!("Which oven is number {i}?"), i % 4 == 0),
        );
    }
    Lib { alex, dan, bank }
}

fn corpus_body(w: &Who, bank: i64, min: i64, after: i64, limit: i64) -> Value {
    json!({"sid": w.sid, "source": "bank", "bank": bank, "min": min, "after": after, "limit": limit})
}

// ---------------------------------------------------------------- corpus

#[tokio::test]
async fn the_corpus_is_all_three_languages_in_source_order() {
    let a = app();
    let l = library(&a, 6);
    let c = raw(&a.db);
    // One question without a Japanese translation: its `ja` keys are absent, never empty strings
    let mut no_ja = six("A plain question", false);
    no_ja.ja = None;
    put_q(&c, "noja1", l.bank, "Ovens", 7, true, &no_ja);
    // A retired question, a hotspot-like old layout question and an unusable one never appear
    put_q(
        &c,
        "gone1",
        l.bank,
        "Ovens",
        8,
        false,
        &six("Retired", false),
    );
    let mut old = six("Old layout", false);
    old.lang = String::new();
    put_q(&c, "old1", l.bank, "Ovens", 9, true, &old);
    let r = post(
        &a,
        "/internal/ai/gen/corpus",
        &corpus_body(&l.alex, l.bank, 5, 0, 200),
    )
    .await;
    assert_eq!(r.status, 200);
    let v: Value = serde_json::from_slice(&r.body).expect("json");
    assert_eq!(v["ok"], true);
    assert_eq!(v["user"], json!({"id": l.alex.id}));
    assert_eq!(v["total"], 7);
    assert!(v["next"].is_null());
    assert_eq!(
        v["banks"],
        json!([{"id": l.bank, "name": "Kitchen basics"}])
    );
    assert_eq!(
        v["strata"],
        json!([{"bank": l.bank, "sheet": "Ovens", "n": 7, "multi": 1}])
    );
    assert_eq!(v["previous"], json!([]));
    let items = v["items"].as_array().expect("items");
    assert_eq!(items.len(), 7);
    let ids: Vec<&str> = items
        .iter()
        .map(|i| i["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids[..3], ["ov001", "ov002", "ov003"]);
    let first = &items[0];
    assert_eq!(first["sheet"], "Ovens");
    assert_eq!(first["kind"], "single");
    assert_eq!(first["answer"], json!([1]));
    assert_eq!(first["stem"]["en"], "Which oven is number 1?");
    assert_eq!(first["stem"]["zh"], "Which oven is number 1? 中文");
    assert_eq!(first["stem"]["ja"], "Which oven is number 1? 日本語");
    assert_eq!(
        first["options"][1],
        json!({"en": "Ladle", "zh": "汤勺", "ja": "おたま"})
    );
    // An explanation the question lacks in one language is an absent key
    assert_eq!(
        first["explanation"],
        json!({"en": "A ladle serves soup.", "zh": "汤勺用来盛汤。"})
    );
    assert!(first["diff"].is_null());
    // The multiple-choice one (4th) has two right options
    assert_eq!(items[3]["kind"], "multi");
    assert_eq!(items[3]["answer"], json!([0, 1]));
    // No Japanese: the key is absent in the stem and every option
    let last = &items[6];
    assert_eq!(last["id"], "noja1");
    assert!(last["stem"].get("ja").is_none());
    assert!(last["options"][0].get("ja").is_none());
    assert_eq!(last["options"][0]["zh"], "打蛋器");
}

#[tokio::test]
async fn the_corpus_is_paged_and_the_extras_come_with_the_first_page_only() {
    let a = app();
    let l = library(&a, 5);
    let page = |after: i64, limit: i64| {
        let body = corpus_body(&l.alex, l.bank, 5, after, limit);
        let a = &a;
        async move {
            let r = post(a, "/internal/ai/gen/corpus", &body).await;
            serde_json::from_slice::<Value>(&r.body).expect("json")
        }
    };
    let p1 = page(0, 2).await;
    assert_eq!(p1["next"], 2);
    assert_eq!(p1["total"], 5);
    assert!(
        p1.get("strata").is_some() && p1.get("banks").is_some() && p1.get("previous").is_some()
    );
    let p2 = page(2, 2).await;
    assert_eq!(p2["next"], 4);
    for k in ["strata", "banks", "previous"] {
        assert!(p2.get(k).is_none(), "{k} only on the first page");
    }
    let p3 = page(4, 2).await;
    assert!(p3["next"].is_null());
    assert_eq!(p3["items"].as_array().expect("items").len(), 1);
    let ids = |v: &Value| -> Vec<String> {
        v["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|i| i["id"].as_str().expect("id").to_string())
            .collect()
    };
    let all: Vec<String> = [&p1, &p2, &p3].iter().flat_map(|p| ids(p)).collect();
    assert_eq!(all, ["ov001", "ov002", "ov003", "ov004", "ov005"]);
    // The default page size is 200 and an offset past the end is an empty last page
    let r = post(
        &a,
        "/internal/ai/gen/corpus",
        &json!({"sid": l.alex.sid, "source": "bank", "bank": l.bank, "min": 1}),
    )
    .await;
    let v: Value = serde_json::from_slice(&r.body).expect("json");
    assert_eq!(v["items"].as_array().expect("items").len(), 5);
    let past = page(50, 10).await;
    assert!(past["next"].is_null());
    assert_eq!(past["items"], json!([]));
}

#[tokio::test]
async fn corpus_texts_are_cut_per_language() {
    let a = app();
    let l = library(&a, 0);
    let c = raw(&a.db);
    let mut q = six(&"S".repeat(700), false);
    q.options[0].text = "O".repeat(200);
    put_q(&c, "long1", l.bank, "Ovens", 1, true, &q);
    let r = post(
        &a,
        "/internal/ai/gen/corpus",
        &corpus_body(&l.alex, l.bank, 1, 0, 10),
    )
    .await;
    let v: Value = serde_json::from_slice(&r.body).expect("json");
    let item = &v["items"][0];
    let en = item["stem"]["en"].as_str().expect("stem");
    assert_eq!(en.chars().count(), 600);
    assert!(en.ends_with('…'));
    assert_eq!(
        item["options"][0]["en"]
            .as_str()
            .expect("opt")
            .chars()
            .count(),
        150
    );
}

#[tokio::test]
async fn corpus_refusals_follow_the_source_route() {
    let a = app();
    let l = library(&a, 3);
    let c = raw(&a.db);
    let reason = |r: &Resp| -> String {
        assert_eq!(r.status, 200);
        let v: Value = serde_json::from_slice(&r.body).expect("json");
        assert_eq!(v["ok"], false);
        v["reason"].as_str().expect("reason").to_string()
    };
    // tooFew
    let r = post(
        &a,
        "/internal/ai/gen/corpus",
        &corpus_body(&l.alex, l.bank, 30, 0, 10),
    )
    .await;
    assert_eq!(reason(&r), "tooFew");
    let v: Value = serde_json::from_slice(&r.body).expect("json");
    assert_eq!(v["have"], 3);
    // not allowed: no AI+ / cannot manage
    let r = post(
        &a,
        "/internal/ai/gen/corpus",
        &corpus_body(&l.dan, l.bank, 1, 0, 10),
    )
    .await;
    assert_eq!(reason(&r), "notAllowed");
    // a bank the caller cannot see
    let other = put_bank(&c, "Other bank", 1);
    put_q(&c, "x1", other, "S", 1, true, &six("Hidden", false));
    let r = post(
        &a,
        "/internal/ai/gen/corpus",
        &corpus_body(&l.alex, other, 1, 0, 10),
    )
    .await;
    assert_eq!(reason(&r), "notFound");
    let r = post(
        &a,
        "/internal/ai/gen/corpus",
        &corpus_body(&l.alex, 9999, 1, 0, 10),
    )
    .await;
    assert_eq!(reason(&r), "notFound");
    // an unknown session
    let mut body = corpus_body(&l.alex, l.bank, 1, 0, 10);
    body["sid"] = json!("0".repeat(64));
    let r = post(&a, "/internal/ai/gen/corpus", &body).await;
    assert_eq!(reason(&r), "session");
    // an AI bank is never a source
    c.execute(
        "UPDATE banks SET ai_job='0123456789abcdef0123456789abcdef', ai_meta='{}' WHERE id=?1",
        params![l.bank],
    )
    .expect("ai bank");
    let r = post(
        &a,
        "/internal/ai/gen/corpus",
        &corpus_body(&l.alex, l.bank, 1, 0, 10),
    )
    .await;
    assert_eq!(reason(&r), "aiSource");
}

#[tokio::test]
async fn corpus_over_twenty_thousand_questions_is_too_many() {
    let a = app();
    let l = library(&a, 0);
    let mut c = raw(&a.db);
    let data = qdata(&six("Which oven?", false));
    let tx = c.transaction().expect("tx");
    for i in 1..=20_001 {
        tx.execute(
            "INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id) \
             VALUES(?1, 'Ovens', ?2, ?2, ?3, 1, '2026-01-01 10:00', '2026-01-01 10:00', ?4)",
            params![format!("q{i:06}"), i, data, l.bank],
        )
        .expect("question");
    }
    tx.commit().expect("commit");
    let r = post(
        &a,
        "/internal/ai/gen/corpus",
        &corpus_body(&l.alex, l.bank, 1, 0, 200),
    )
    .await;
    let v: Value = serde_json::from_slice(&r.body).expect("json");
    assert_eq!(v, json!({"ok": false, "reason": "tooMany", "max": 20000}));
    // exactly 20 000 is fine
    c.execute("DELETE FROM questions WHERE id='q020001'", [])
        .expect("delete");
    let r = post(
        &a,
        "/internal/ai/gen/corpus",
        &corpus_body(&l.alex, l.bank, 1, 0, 200),
    )
    .await;
    let v: Value = serde_json::from_slice(&r.body).expect("json");
    assert_eq!(v["total"], 20_000);
    assert_eq!(v["next"], 200);
}

#[tokio::test]
async fn the_wrong_corpus_holds_only_the_callers_open_wrong_questions() {
    let a = app();
    let l = library(&a, 4);
    let c = raw(&a.db);
    for (qid, wrong, streak) in [("ov001", 2, 0), ("ov003", 1, 1), ("ov004", 0, 0)] {
        c.execute(
            "INSERT INTO user_stats(user_id, question_id, attempts, correct, wrong, streak, last, at) \
             VALUES(?1, ?2, 3, 1, ?3, ?4, 0, '2026-01-02 10:00')",
            params![l.alex.id, qid, wrong, streak],
        )
        .expect("stat");
    }
    let body = json!({"sid": l.alex.sid, "source": "wrong", "min": 1,
        "sheets": [{"bank": l.bank, "sheet": "Ovens"}]});
    let r = post(&a, "/internal/ai/gen/corpus", &body).await;
    let v: Value = serde_json::from_slice(&r.body).expect("json");
    assert_eq!(v["total"], 2);
    let ids: Vec<&str> = v["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, ["ov001", "ov003"]);
    // A difficulty label appears from five attempts
    assert!(v["items"][0]["diff"].is_null());
}

#[tokio::test]
async fn corpus_requests_are_validated() {
    let a = app();
    let l = library(&a, 2);
    let ok = |b: Value| {
        let a = &a;
        async move { post(a, "/internal/ai/gen/corpus", &b).await.status }
    };
    let good = corpus_body(&l.alex, l.bank, 1, 0, 200);
    assert_eq!(ok(good.clone()).await, 200);
    for (key, bad) in [
        ("limit", json!(0)),
        ("limit", json!(201)),
        ("after", json!(-1)),
        ("after", json!(20001)),
        ("min", json!(0)),
        ("min", json!(2001)),
        ("source", json!("file")),
        ("source", json!("other")),
        ("bank", json!(0)),
        ("extra", json!(1)),
    ] {
        let mut b = good.clone();
        b[key] = bad.clone();
        let r = post(&a, "/internal/ai/gen/corpus", &b).await;
        assert_eq!(r.status, 400, "{key} = {bad}");
    }
    // `sheets` belongs to scope / wrong only, `bank` to bank only
    let mut b = good.clone();
    b["sheets"] = json!([{"bank": l.bank, "sheet": "Ovens"}]);
    assert_eq!(ok(b).await, 400);
    let scope = json!({"sid": l.alex.sid, "source": "scope", "min": 1, "bank": l.bank});
    assert_eq!(ok(scope).await, 400);
    // `lang` is gone in v1.3
    let mut b = good;
    b["lang"] = json!("zh");
    assert_eq!(ok(b).await, 400);
    // The token gate is the same as for every internal route
    let r = send(
        &a.router,
        build(
            "POST",
            "/internal/ai/gen/corpus",
            Some(NGINX_HOST),
            &[
                ("content-type", b"application/json"),
                ("content-length", b"2"),
            ],
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(r.status, 404);
}

// ---------------------------------------------------------------- ETag / 304

fn etag(r: &Resp) -> String {
    header(r, "etag").expect("an ETag")
}

#[tokio::test]
async fn bank_and_banks_carry_an_etag_and_answer_304() {
    let a = app();
    let l = library(&a, 3);
    let admin = sign_in(&a, 1);
    for who in [&l.alex, &admin] {
        for path in ["/api/bank", "/api/banks"] {
            let r = api(&a, "GET", path, &who.cookie, &[], None).await;
            assert_eq!(r.status, 200, "{path}");
            let tag = etag(&r);
            assert!(tag.starts_with('"') && tag.ends_with('"'), "{tag}");
            assert!(tag.contains(&format!("-{}-", who.id)), "{tag}");
            assert_eq!(
                header(&r, "cache-control").as_deref(),
                Some("private, no-cache")
            );
            assert!(!r.body.is_empty());
            // The same tag: 304, no body, the same headers
            let m = api(
                &a,
                "GET",
                path,
                &who.cookie,
                &[("if-none-match", &tag)],
                None,
            )
            .await;
            assert_eq!(m.status, 304, "{path}");
            assert!(m.body.is_empty());
            assert_eq!(etag(&m), tag);
            assert_eq!(
                header(&m, "cache-control").as_deref(),
                Some("private, no-cache")
            );
            // A weak copy (what a compressing proxy sends), a list and `*` match too
            let weak = format!("W/{tag}");
            for inm in [weak.as_str(), &format!("\"zz\", {tag}"), "*"] {
                let m = api(
                    &a,
                    "GET",
                    path,
                    &who.cookie,
                    &[("if-none-match", inm)],
                    None,
                )
                .await;
                assert_eq!(m.status, 304, "{path} {inm}");
            }
            // Another tag: the full answer
            let m = api(
                &a,
                "GET",
                path,
                &who.cookie,
                &[("if-none-match", "\"nope\"")],
                None,
            )
            .await;
            assert_eq!(m.status, 200);
            assert_eq!(m.body, r.body);
        }
    }
}

#[tokio::test]
async fn another_users_etag_never_matches() {
    let a = app();
    let l = library(&a, 3);
    let admin = sign_in(&a, 1);
    for path in ["/api/bank", "/api/banks"] {
        let mine = etag(&api(&a, "GET", path, &l.alex.cookie, &[], None).await);
        let theirs = etag(&api(&a, "GET", path, &l.dan.cookie, &[], None).await);
        let boss = etag(&api(&a, "GET", path, &admin.cookie, &[], None).await);
        assert!(mine != theirs && mine != boss && theirs != boss, "{path}");
        let r = api(
            &a,
            "GET",
            path,
            &l.dan.cookie,
            &[("if-none-match", &mine)],
            None,
        )
        .await;
        assert_eq!(r.status, 200, "{path}: alex's tag must not give dan a 304");
    }
}

#[tokio::test]
async fn every_stamp_event_changes_the_etag() {
    let a = app();
    let l = library(&a, 3);
    let admin = sign_in(&a, 1);
    let c = raw(&a.db);
    let tags = |path: &'static str, who: &Who| {
        let a = &a;
        let cookie = who.cookie.clone();
        async move { etag(&api(a, "GET", path, &cookie, &[], None).await) }
    };
    // The events: a question added, a question retired, a bank renamed, a new bank (granted), a bank deleted,
    // a study record saved (changes /api/banks only), a sheet of retired-only questions deleted
    let mut last = (
        tags("/api/bank", &l.dan).await,
        tags("/api/banks", &l.dan).await,
    );
    let mut check = |what: &str, bank_changes: bool, banks_changes: bool, now: (String, String)| {
        assert_eq!(last.0 != now.0, bank_changes, "{what}: /api/bank");
        assert_eq!(last.1 != now.1, banks_changes, "{what}: /api/banks");
        last = now;
    };
    put_q(
        &c,
        "extra1",
        l.bank,
        "Ovens",
        10,
        true,
        &six("One more", false),
    );
    c.execute(
        "UPDATE banks SET updated_at='2026-02-01 10:00' WHERE id=?1",
        params![l.bank],
    )
    .expect("touch");
    check(
        "question added",
        true,
        true,
        (
            tags("/api/bank", &l.dan).await,
            tags("/api/banks", &l.dan).await,
        ),
    );
    put_q(
        &c,
        "gone1",
        l.bank,
        "Retired",
        11,
        false,
        &six("Retired", false),
    );
    check(
        "retired-only sheet added",
        true,
        true,
        (
            tags("/api/bank", &l.dan).await,
            tags("/api/banks", &l.dan).await,
        ),
    );
    c.execute("DELETE FROM questions WHERE sheet='Retired'", [])
        .expect("delete");
    check(
        "retired-only sheet deleted",
        true,
        true,
        (
            tags("/api/bank", &l.dan).await,
            tags("/api/banks", &l.dan).await,
        ),
    );
    c.execute(
        "UPDATE banks SET name='Kitchen basics 2' WHERE id=?1",
        params![l.bank],
    )
    .expect("rename");
    check(
        "bank renamed",
        false,
        true,
        (
            tags("/api/bank", &l.dan).await,
            tags("/api/banks", &l.dan).await,
        ),
    );
    // A study record changes the numbers of /api/banks only
    let body = r#"{"results":[{"id":"ov001","correct":true,"answer":["B"]}],"sheets":["Ovens"],"mode":"all","seconds":5}"#;
    let r = api(&a, "POST", "/api/attempt", &l.dan.cookie, &[], Some(body)).await;
    assert_eq!(r.status, 200);
    check(
        "study record saved",
        false,
        true,
        (
            tags("/api/bank", &l.dan).await,
            tags("/api/banks", &l.dan).await,
        ),
    );
    let other = put_bank(&c, "Second bank", l.alex.id);
    grant(&c, l.dan.id, other);
    put_q(
        &c,
        "s1",
        other,
        "S",
        1,
        true,
        &six("In the second bank", false),
    );
    check(
        "bank added and granted",
        true,
        true,
        (
            tags("/api/bank", &l.dan).await,
            tags("/api/banks", &l.dan).await,
        ),
    );
    c.execute("DELETE FROM user_banks WHERE bank_id=?1", params![other])
        .expect("revoke");
    check(
        "bank taken away",
        true,
        true,
        (
            tags("/api/bank", &l.dan).await,
            tags("/api/banks", &l.dan).await,
        ),
    );
    // The administrator's tag follows every bank
    let t0 = tags("/api/bank", &admin).await;
    c.execute("DELETE FROM questions WHERE bank_id=?1", params![other])
        .expect("delete");
    c.execute("DELETE FROM banks WHERE id=?1", params![other])
        .expect("delete bank");
    assert_ne!(t0, tags("/api/bank", &admin).await);
}

#[tokio::test]
async fn head_and_unsigned_requests_keep_their_rules() {
    let a = app();
    let l = library(&a, 2);
    let r = api(&a, "GET", "/api/bank", &l.dan.cookie, &[], None).await;
    let tag = etag(&r);
    let h = api(&a, "HEAD", "/api/bank", &l.dan.cookie, &[], None).await;
    assert_eq!(h.status, 200);
    assert_eq!(etag(&h), tag);
    assert!(h.body.is_empty());
    // No session: the usual 401, never a 304
    let r = api(&a, "GET", "/api/bank", "", &[("if-none-match", &tag)], None).await;
    assert_eq!(r.status, 401);
}

// ---------------------------------------------------------------- idempotent attempt

#[tokio::test]
async fn a_repeated_attempt_save_answers_like_the_first_plus_duplicate() {
    let a = app();
    let l = library(&a, 3);
    let key = "0123456789abcdef0123456789abcdef";
    let body = format!(
        r#"{{"results":[{{"id":"ov001","correct":true,"answer":["B"]}},{{"id":"ov002","correct":false,"answer":["A"]}}],"sheets":["Ovens"],"mode":"all","seconds":9,"key":"{key}","startedAt":1790000000}}"#
    );
    let c = raw(&a.db);
    let records = |c: &Connection| -> i64 {
        c.query_row("SELECT COUNT(*) FROM attempts", [], |r| r.get(0))
            .expect("count")
    };
    let first = api(&a, "POST", "/api/attempt", &l.dan.cookie, &[], Some(&body)).await;
    assert_eq!(first.status, 200);
    let f: Value = serde_json::from_slice(&first.body).expect("json");
    assert!(f.get("duplicate").is_none());
    let id = f["recordId"].as_i64().expect("record id");
    assert_eq!(records(&c), 1);
    let stats: i64 = c
        .query_row("SELECT SUM(attempts) FROM user_stats", [], |r| r.get(0))
        .expect("stats");
    let again = api(&a, "POST", "/api/attempt", &l.dan.cookie, &[], Some(&body)).await;
    assert_eq!(again.status, 200);
    let g: Value = serde_json::from_slice(&again.body).expect("json");
    assert_eq!(g["duplicate"], true);
    assert_eq!(g["recordId"], id);
    // Nothing was written the second time
    assert_eq!(records(&c), 1);
    let stats2: i64 = c
        .query_row("SELECT SUM(attempts) FROM user_stats", [], |r| r.get(0))
        .expect("stats");
    assert_eq!(stats, stats2);
    // The answer equals the first one plus the flag, which sits before recordId
    let mut without = g.clone();
    without.as_object_mut().expect("object").remove("duplicate");
    assert_eq!(without, f);
    let text = again.text();
    assert!(text.find("\"duplicate\"").expect("flag") < text.find("\"recordId\"").expect("id"));
    // A different start time or another user is a new record
    let other_start = body.replace("1790000000", "1790000001");
    let r = api(
        &a,
        "POST",
        "/api/attempt",
        &l.dan.cookie,
        &[],
        Some(&other_start),
    )
    .await;
    let v: Value = serde_json::from_slice(&r.body).expect("json");
    assert!(v.get("duplicate").is_none());
    assert_eq!(records(&c), 2);
}
