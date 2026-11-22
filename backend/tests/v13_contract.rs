//! v1.3 contract test, backend half: the answer of `POST /internal/ai/gen/corpus` for a made-up kitchen bank is compared with
//! the shared golden file `tests/fixtures/gen_corpus_page.json`; ai-service's `tests/v13_contract.rs` feeds the same file to
//! its `parse_corpus`, so the two sides cannot drift apart. Set `XEXAM_UPDATE_FIXTURES=1` to rewrite the file after an
//! intended contract change (then review the diff). Made-up questions and users only.
#![allow(dead_code)]
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

#[tokio::test]
async fn the_corpus_answer_equals_the_golden_file_of_both_services() {
    let a = app();
    let l = library(&a, 6);
    let c = raw(&a.db);
    // One question without a Japanese text: its `ja` keys are absent in the file
    let mut no_ja = six("A plain question", false);
    no_ja.ja = None;
    put_q(&c, "noja1", l.bank, "Ovens", 7, true, &no_ja);
    let r = post(
        &a,
        "/internal/ai/gen/corpus",
        &corpus_body(&l.alex, l.bank, 5, 0, 200),
    )
    .await;
    assert_eq!(r.status, 200);
    let got: Value = serde_json::from_slice(&r.body).expect("json");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/gen_corpus_page.json");
    if std::env::var("XEXAM_UPDATE_FIXTURES").is_ok() {
        let mut text = serde_json::to_string_pretty(&got).expect("pretty");
        text.push('\n');
        std::fs::write(&path, text).expect("write the golden file");
    }
    let want: Value =
        serde_json::from_slice(&std::fs::read(&path).expect("golden file")).expect("golden json");
    assert_eq!(
        got, want,
        "the corpus answer changed: update the golden file on purpose"
    );
}
