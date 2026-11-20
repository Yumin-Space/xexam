//! v1.6: the bank download route `GET /api/banks/download?id=&format=xlsx|docx`, who may use it (`users.can_download_banks`,
//! schema v10), the activity event, the Excel round trip through the app and the migration to schema 10.
//! Real router, real engine and database. Made-up questions and users only.
#[path = "api_common/mod.rs"]
mod apic;
#[path = "common/mod.rs"]
mod dbc;

use apic::{build, send, Resp};
use axum::body::Body;
use axum::Router;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use xexam_backend::api::response::{MSG_BANK_NOT_FOUND, MSG_NO_DOWNLOAD_PERMISSION};
use xexam_backend::api::{build_router, AppState};
use xexam_backend::auth::{self, Role};
use xexam_backend::database::{NewSession, Store, UserChange};
use xexam_backend::models::{BankAccess, BankScope, Viewer};
use xexam_backend::services::RealEngine;

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";
const XLSX_MIME: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const DOCX_MIME: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";

// ---------------------------------------------------------------- made-up workbook (two sheets)

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A two-sheet six-column workbook; `tag` makes the questions of two workbooks differ.
fn sample(tag: &str) -> Vec<u8> {
    let header = [
        "Question (English)",
        "中文题目",
        "日本語の問題",
        "Answer (English)",
        "【答案】和解析",
        "【正解】と解説",
    ];
    let single = |i: usize| -> Vec<String> {
        vec![
            format!(
                "Which colour is the sky on a clear day ({tag} {i})?\nA. Blue\nB. Green\nC. Red"
            ),
            format!("晴天时天空是什么颜色（{tag} {i}）？\nA. 蓝色\nB. 绿色\nC. 红色"),
            format!("晴れた日の空は何色ですか（{tag} {i}）。\nA. 青\nB. 緑\nC. 赤"),
            "【Answer】A (sure)\n【Explanation】Sunlight scatters.".to_string(),
            "【答案】A\n【解析】阳光散射。".to_string(),
            "【正解】A\n【解説】日光が散乱します。".to_string(),
        ]
    };
    let hot = |i: usize| -> Vec<String> {
        vec![
            format!("HOTSPOT\nDecide whether each statement is true ({tag} {i}).\n1. Water is wet.\tYes / No\n2. Fire is cold.\tYes / No"),
            "判断每个说法是否正确。\n1. 水是湿的。\u{3000}是 / 否\n2. 火是冷的。\u{3000}是 / 否".to_string(),
            "各文が正しいか判断してください。\n1. 水は濡れている。\u{3000}はい / いいえ\n2. 火は冷たい。\u{3000}はい / いいえ".to_string(),
            "【Answer】1. Yes; 2. No".to_string(),
            "【答案】1. 是；2. 否".to_string(),
            "【正解】1. はい；2. いいえ".to_string(),
        ]
    };
    let sheets: Vec<(&str, Vec<Vec<String>>)> = vec![
        (
            "Colours",
            vec![
                header.map(String::from).to_vec(),
                single(1),
                single(2),
                hot(1),
            ],
        ),
        (
            "Facts",
            vec![header.map(String::from).to_vec(), single(3), hot(2)],
        ),
    ];
    let mut wb = String::from(
        r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>"#,
    );
    let mut rels = String::from(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    let mut parts: Vec<(String, String)> = Vec::new();
    for (i, (name, rows)) in sheets.iter().enumerate() {
        let n = i + 1;
        wb.push_str(&format!(
            r#"<sheet name="{name}" sheetId="{n}" r:id="rId{n}"/>"#
        ));
        rels.push_str(&format!(
            r#"<Relationship Id="rId{n}" Target="worksheets/s{n}.xml"/>"#
        ));
        let mut sheet = String::new();
        for (r0, cols) in rows.iter().enumerate() {
            let r = r0 + 1;
            sheet.push_str(&format!(r#"<row r="{r}">"#));
            for (k, text) in cols.iter().enumerate() {
                let letter = (b'A' + u8::try_from(k).expect("col")) as char;
                sheet.push_str(&format!(
                    r#"<c r="{letter}{r}" t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#,
                    esc(text)
                ));
            }
            sheet.push_str("</row>");
        }
        parts.push((
            format!("xl/worksheets/s{n}.xml"),
            format!(
                r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{sheet}</sheetData></worksheet>"#
            ),
        ));
    }
    wb.push_str("</sheets></workbook>");
    rels.push_str("</Relationships>");
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (name, body) in [
        ("xl/workbook.xml".to_string(), wb),
        ("xl/_rels/workbook.xml.rels".to_string(), rels),
    ]
    .into_iter()
    .chain(parts)
    {
        w.start_file(name, opts).expect("start");
        w.write_all(body.as_bytes()).expect("write");
    }
    w.finish().expect("finish").into_inner()
}

// ---------------------------------------------------------------- the app

struct App {
    router: Router,
    state: AppState,
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
    App {
        router: build_router(state.clone()),
        state,
        store,
        db,
        _dir: dir,
    }
}

fn raw(p: &Path) -> Connection {
    Connection::open(p).expect("raw open")
}

/// A made-up plain user who sees `banks`, with the given manage / download permissions (created without signing in).
fn add_user(a: &App, name: &str, banks: &[i64], manage: bool, download: bool) -> i64 {
    match a
        .store
        .create_user_full(
            name,
            "placeholder-hash",
            Role::User,
            &BankAccess::Only(banks.to_vec()),
            false,
            manage,
            false,
            download,
        )
        .expect("create")
    {
        UserChange::Done(id) => id,
        other => panic!("{other:?}"),
    }
}

/// A session for user `id`; returns the `Cookie` value.
fn session(a: &App, id: i64) -> String {
    let u = a.store.user_by_id(id).expect("lookup").expect("user");
    // clears a pending password change (the default administrator starts with one)
    a.store
        .change_own_password(
            id,
            &u.password_hash,
            &u.password_hash,
            &auth::token_hash(&auth::new_session_token()),
            auth::unix_now(),
        )
        .expect("flag")
        .expect("user");
    let token = auth::new_session_token();
    let now = auth::unix_now();
    a.store
        .create_session(&NewSession {
            token_hash: &auth::token_hash(&token),
            user_id: id,
            now,
            expires_at: now + 3600,
            remember: false,
            ip: None,
            ua: None,
        })
        .expect("session");
    format!("{}={token}", auth::SESSION_COOKIE)
}

fn admin(a: &App) -> String {
    session(a, 1)
}

async fn call(a: &App, method: &str, path: &str, cookie: Option<&str>) -> Resp {
    let hs: Vec<(&str, &[u8])> = cookie
        .map(|c| ("cookie", c.as_bytes()))
        .into_iter()
        .collect();
    send(
        &a.router,
        build(method, path, Some(HOST), &hs, Body::empty()),
    )
    .await
}

async fn get(a: &App, path: &str, cookie: &str) -> Resp {
    call(a, "GET", path, Some(cookie)).await
}

async fn post(a: &App, path: &str, cookie: &str, body: &Value) -> Resp {
    let text = body.to_string();
    let len = text.len().to_string();
    let hs: Vec<(&str, &[u8])> = vec![
        ("cookie", cookie.as_bytes()),
        ("origin", ORIGIN.as_bytes()),
        ("content-type", b"application/json"),
        ("content-length", len.as_bytes()),
    ];
    send(
        &a.router,
        build("POST", path, Some(HOST), &hs, Body::from(text)),
    )
    .await
}

async fn upload(a: &App, cookie: &str, target: &str, name: &str, data: &[u8]) -> Resp {
    let len = data.len().to_string();
    let hs: Vec<(&str, &[u8])> = vec![
        ("cookie", cookie.as_bytes()),
        ("origin", ORIGIN.as_bytes()),
        ("content-type", b"application/octet-stream"),
        ("x-filename", name.as_bytes()),
        ("x-bank-target", target.as_bytes()),
        ("content-length", len.as_bytes()),
    ];
    send(
        &a.router,
        build(
            "POST",
            "/api/upload",
            Some(HOST),
            &hs,
            Body::from(data.to_vec()),
        ),
    )
    .await
}

/// Uploads and confirms as `cookie`; returns the bank id.
async fn import(a: &App, cookie: &str, target: &str, name: &str, data: &[u8]) -> i64 {
    let r = upload(a, cookie, target, name, data).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let token = r.json()["token"].as_str().expect("token").to_string();
    let r = post(a, "/api/import/confirm", cookie, &json!({ "token": token })).await;
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()["bankId"].as_i64().expect("bank id")
}

fn dl(id: i64, format: &str) -> String {
    format!("/api/banks/download?id={id}&format={format}")
}

/// (action, username, object, detail)
type ActivityRow = (String, Option<String>, Option<String>, Option<String>);

fn activity_rows(a: &App) -> Vec<ActivityRow> {
    raw(&a.db)
        .prepare("SELECT action, username, object, detail FROM activity_log WHERE action='bank.download' ORDER BY id")
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

// ---------------------------------------------------------------- the route

#[tokio::test]
async fn an_administrator_downloads_the_excel_file() {
    let a = app();
    let adm = admin(&a);
    let id = import(&a, &adm, "new", "Soup bank.xlsx", &sample("a")).await;
    let r = get(&a, &dl(id, "xlsx"), &adm).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.header("content-type").as_deref(), Some(XLSX_MIME));
    let cd = r.header("content-disposition").expect("disposition");
    assert!(cd.starts_with("attachment;"), "{cd}");
    assert!(cd.contains("filename*=UTF-8''"), "{cd}");
    assert_eq!(
        r.header("content-length").as_deref(),
        Some(r.body.len().to_string().as_str())
    );
    assert_eq!(r.header("cache-control").as_deref(), Some("no-store"));
    assert_eq!(
        r.header("x-content-type-options").as_deref(),
        Some("nosniff")
    );
    assert!(r.body.starts_with(b"PK"));
    let mut z = zip::ZipArchive::new(Cursor::new(r.body.clone())).expect("zip");
    assert!(z.by_name("xl/worksheets/sheet2.xml").is_ok());
}

#[tokio::test]
async fn an_administrator_downloads_the_word_file() {
    let a = app();
    let adm = admin(&a);
    let id = import(&a, &adm, "new", "Soup bank.xlsx", &sample("a")).await;
    let r = get(&a, &dl(id, "docx"), &adm).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.header("content-type").as_deref(), Some(DOCX_MIME));
    let cd = r.header("content-disposition").expect("disposition");
    assert!(
        cd.starts_with("attachment;") && cd.contains("filename*=UTF-8''"),
        "{cd}"
    );
    assert!(cd.contains("Soup%20bank.docx"), "{cd}");
    assert!(r.body.starts_with(b"PK"));
    let mut z = zip::ZipArchive::new(Cursor::new(r.body.clone())).expect("zip");
    let mut doc = String::new();
    z.by_name("word/document.xml")
        .expect("document.xml")
        .read_to_string(&mut doc)
        .expect("read");
    assert!(!doc.is_empty());
}

#[tokio::test]
async fn who_may_download() {
    let a = app();
    let adm = admin(&a);
    let b1 = import(&a, &adm, "new", "one.xlsx", &sample("one")).await;
    let b2 = import(&a, &adm, "new", "two.xlsx", &sample("two")).await;
    let plain = add_user(&a, "pia", &[b1], false, false);
    let manager = add_user(&a, "max", &[b1], true, false);
    let (plain_c, manager_c) = (session(&a, plain), session(&a, manager));

    // no session
    let r = call(&a, "GET", &dl(b1, "xlsx"), None).await;
    assert_eq!(r.status, 401);
    // a plain user and a bank manager without the right
    for c in [&plain_c, &manager_c] {
        let r = get(&a, &dl(b1, "xlsx"), c).await;
        assert_eq!(r.status, 403, "{}", r.text());
        assert_eq!(r.error(), MSG_NO_DOWNLOAD_PERMISSION);
        // an invisible bank gives the same answer: the right is checked first
        assert_eq!(get(&a, &dl(b2, "xlsx"), c).await.status, 403);
    }
    // the right is given by an administrator
    let r = post(
        &a,
        "/api/users/update",
        &adm,
        &json!({"id": plain, "canDownloadBanks": true}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = get(&a, &dl(b1, "xlsx"), &plain_c).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.header("content-type").as_deref(), Some(XLSX_MIME));
    // a bank the user cannot see answers like a bank that does not exist
    let hidden = get(&a, &dl(b2, "xlsx"), &plain_c).await;
    let missing = get(&a, &dl(9999, "xlsx"), &plain_c).await;
    assert_eq!(hidden.status, 404);
    assert_eq!(hidden.error(), MSG_BANK_NOT_FOUND);
    assert_eq!(
        (hidden.status, &hidden.body),
        (missing.status, &missing.body)
    );
    // the right is read on every request: taking it away applies at once, without a new sign-in
    let r = post(
        &a,
        "/api/users/update",
        &adm,
        &json!({"id": plain, "canDownloadBanks": false}),
    )
    .await;
    assert_eq!(r.status, 200);
    let r = get(&a, &dl(b1, "xlsx"), &plain_c).await;
    assert_eq!(r.status, 403);
    assert_eq!(r.error(), MSG_NO_DOWNLOAD_PERMISSION);
    // the stored right of an administrator does not matter: administrators always may
    raw(&a.db)
        .execute("UPDATE users SET can_download_banks = 0 WHERE id = 1", [])
        .expect("update");
    assert_eq!(get(&a, &dl(b2, "xlsx"), &adm).await.status, 200);
}

#[tokio::test]
async fn bad_queries_are_400() {
    let a = app();
    let adm = admin(&a);
    let id = import(&a, &adm, "new", "one.xlsx", &sample("one")).await;
    for path in [
        format!("/api/banks/download?id={id}&format=pdf"),
        format!("/api/banks/download?id={id}&format=XLSX"),
        format!("/api/banks/download?id={id}"),
        "/api/banks/download?format=xlsx".to_string(),
        "/api/banks/download?id=abc&format=xlsx".to_string(),
        format!("/api/banks/download?id={id}&format=xlsx&format=xlsx"),
        format!("/api/banks/download?id={id}&id={id}&format=xlsx"),
        "/api/banks/download".to_string(),
    ] {
        let r = get(&a, &path, &adm).await;
        assert_eq!(r.status, 400, "{path}: {}", r.text());
    }
    assert_eq!(get(&a, &dl(9999, "docx"), &adm).await.status, 404);
}

#[tokio::test]
async fn head_has_no_body_and_leaves_no_activity_row() {
    let a = app();
    let adm = admin(&a);
    let id = import(&a, &adm, "new", "one.xlsx", &sample("one")).await;
    let r = call(&a, "HEAD", &dl(id, "xlsx"), Some(&adm)).await;
    assert_eq!(r.status, 200);
    assert!(r.body.is_empty());
    assert_eq!(r.header("content-type").as_deref(), Some(XLSX_MIME));
    a.state.0.activity.flush().await;
    assert!(activity_rows(&a).is_empty());
}

#[tokio::test]
async fn a_download_is_one_activity_event() {
    let a = app();
    let adm = admin(&a);
    let id = import(&a, &adm, "new", "Soup bank.xlsx", &sample("one")).await;
    let user = add_user(&a, "pia", &[id], false, true);
    let c = session(&a, user);
    let r = get(&a, &dl(id, "xlsx"), &c).await;
    assert_eq!(r.status, 200, "{}", r.text());
    // a refused request writes no download row
    assert_eq!(get(&a, &dl(id, "pdf"), &c).await.status, 400);
    assert_eq!(get(&a, &dl(9999, "xlsx"), &c).await.status, 404);
    a.state.0.activity.flush().await;
    let rows = activity_rows(&a);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let (action, who, object, detail) = rows.into_iter().next().expect("row");
    assert_eq!(action, "bank.download");
    assert_eq!(who.as_deref(), Some("pia"));
    assert_eq!(object.as_deref(), Some("Soup bank"));
    let detail: Value = serde_json::from_str(&detail.expect("detail")).expect("json");
    assert_eq!(
        detail,
        json!({"format": "xlsx", "langs": ["en", "zh", "ja"], "sheets": 2, "questions": 5})
    );
}

// ---------------------------------------------------------------- the round trip through the app

#[tokio::test]
async fn the_excel_download_imports_again_without_changes() {
    let a = app();
    let adm = admin(&a);
    let id = import(&a, &adm, "new", "Soup bank.xlsx", &sample("one")).await;
    let file = get(&a, &dl(id, "xlsx"), &adm).await;
    assert_eq!(file.status, 200);
    // Uploaded as an update of the same bank: the ids are kept, so nothing is added or removed
    let r = upload(&a, &adm, &id.to_string(), "Soup bank.xlsx", &file.body).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let plan = r.json();
    assert_eq!(plan["added"], 0, "{plan}");
    assert_eq!(plan["removed"], 0, "{plan}");
    assert_eq!(plan["skipped"], 0, "{plan}");
    let sheets = plan["sheets"].as_array().expect("sheets");
    assert_eq!(sheets.len(), 2);
    for s in sheets {
        assert_eq!(s["added"], 0, "{s}");
        assert_eq!(s["removed"], 0, "{s}");
        assert!(s.get("warnings").is_none(), "{s}");
    }
    // The store hands out the same questions the download was made of
    let v = Viewer::admin(1);
    let bank = a
        .store
        .export_bank_as(&v, id)
        .expect("export")
        .expect("bank");
    assert_eq!(bank.name, "Soup bank");
    assert_eq!(
        bank.sheets
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["Colours", "Facts"]
    );
    assert_eq!(bank.question_count(), 5);
    assert!(bank
        .sheets
        .iter()
        .flat_map(|s| &s.questions)
        .all(|q| q.id.is_empty()));
}

#[tokio::test]
async fn export_bank_as_checks_the_scope_and_skips_retired_questions() {
    let a = app();
    let adm = admin(&a);
    let id = import(&a, &adm, "new", "Soup bank.xlsx", &sample("one")).await;
    let all = Viewer::admin(1);
    assert!(a
        .store
        .export_bank_as(&all, id + 1)
        .expect("export")
        .is_none());
    let blind = Viewer {
        user: 2,
        admin: false,
        can_manage: false,
        scope: BankScope::Only(Default::default()),
    };
    assert!(a
        .store
        .export_bank_as(&blind, id)
        .expect("export")
        .is_none());
    let sees = Viewer {
        scope: BankScope::Only([id].into_iter().collect()),
        ..blind
    };
    assert_eq!(
        a.store
            .export_bank_as(&sees, id)
            .expect("export")
            .expect("bank")
            .question_count(),
        5
    );
    // A retired question is not exported, and a sheet left empty disappears
    raw(&a.db)
        .execute(
            "UPDATE questions SET active=0 WHERE bank_id=?1 AND sheet='Facts'",
            [id],
        )
        .expect("retire");
    let bank = a
        .store
        .export_bank_as(&all, id)
        .expect("export")
        .expect("bank");
    assert_eq!(bank.sheets.len(), 1);
    assert_eq!(bank.question_count(), 3);
    raw(&a.db)
        .execute("UPDATE questions SET active=0 WHERE bank_id=?1", [id])
        .expect("retire all");
    let bank = a
        .store
        .export_bank_as(&all, id)
        .expect("export")
        .expect("bank");
    assert!(bank.sheets.is_empty());
    // An empty bank still downloads: one sheet with the header row
    let r = get(&a, &dl(id, "xlsx"), &adm).await;
    assert_eq!(r.status, 200);
}

// ---------------------------------------------------------------- users API

#[tokio::test]
async fn can_download_banks_in_the_users_api_and_the_auth_state() {
    let a = app();
    let adm = admin(&a);
    let r = post(
        &a,
        "/api/users/create",
        &adm,
        &json!({"username": "dora", "password": "made-up-pass-1", "role": "user"}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let row = |v: &Value, name: &str| -> Value {
        v["users"]
            .as_array()
            .expect("users")
            .iter()
            .find(|u| u["username"] == name)
            .cloned()
            .expect("row")
    };
    let list = get(&a, "/api/users", &adm).await.json();
    assert_eq!(row(&list, "dora")["canDownloadBanks"], false);
    assert_eq!(row(&list, "admin")["canDownloadBanks"], false);
    // the key follows canManageBanks
    let text = get(&a, "/api/users", &adm).await.text();
    let (m, d) = (
        text.find("\"canManageBanks\"").expect("manage"),
        text.find("\"canDownloadBanks\"").expect("download"),
    );
    assert!(m < d && d < text.find("\"ai\"").expect("ai"));

    // a created user with the right
    let r = post(
        &a,
        "/api/users/create",
        &adm,
        &json!({"username": "eli", "password": "made-up-pass-1", "role": "user", "canDownloadBanks": true}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(row(&r.json(), "eli")["canDownloadBanks"], true);
    // a wrong type is refused, on create and on update
    let r = post(
        &a,
        "/api/users/create",
        &adm,
        &json!({"username": "fay", "password": "made-up-pass-1", "role": "user", "canDownloadBanks": "yes"}),
    )
    .await;
    assert_eq!(r.status, 400);
    assert!(a.store.user_by_name("fay").expect("db").is_none());
    let eli = a.store.user_by_name("eli").expect("db").expect("eli").id;
    let r = post(
        &a,
        "/api/users/update",
        &adm,
        &json!({"id": eli, "canDownloadBanks": 1}),
    )
    .await;
    assert_eq!(r.status, 400);
    // a valid change is applied and listed in the activity detail
    let r = post(
        &a,
        "/api/users/update",
        &adm,
        &json!({"id": eli, "canDownloadBanks": false}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(row(&r.json(), "eli")["canDownloadBanks"], false);
    let r = post(
        &a,
        "/api/users/update",
        &adm,
        &json!({"id": eli, "canDownloadBanks": true}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(row(&r.json(), "eli")["canDownloadBanks"], true);
    // an update that omits the key leaves it alone; the stored value survives a role change
    let r = post(
        &a,
        "/api/users/update",
        &adm,
        &json!({"id": eli, "username": "eli2"}),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(row(&r.json(), "eli2")["canDownloadBanks"], true);
    let r = post(
        &a,
        "/api/users/update",
        &adm,
        &json!({"id": eli, "role": "admin"}),
    )
    .await;
    assert_eq!(r.status, 200);
    let r = post(
        &a,
        "/api/users/update",
        &adm,
        &json!({"id": eli, "role": "user"}),
    )
    .await;
    assert_eq!(row(&r.json(), "eli2")["canDownloadBanks"], true);

    // the auth state: true for an administrator, the stored value for a user
    let state = get(&a, "/api/auth/state", &adm).await.json();
    assert_eq!(state["user"]["canDownloadBanks"], true);
    let dora = a.store.user_by_name("dora").expect("db").expect("dora").id;
    let dc = session(&a, dora);
    assert_eq!(
        get(&a, "/api/auth/state", &dc).await.json()["user"]["canDownloadBanks"],
        false
    );
    let ec = session(&a, eli);
    assert_eq!(
        get(&a, "/api/auth/state", &ec).await.json()["user"]["canDownloadBanks"],
        true
    );

    // the update's activity detail names the field
    a.state.0.activity.flush().await;
    let details: Vec<String> = raw(&a.db)
        .prepare(
            "SELECT COALESCE(detail, '') FROM activity_log WHERE action='users.update' ORDER BY id",
        )
        .expect("prepare")
        .query_map([], |r| r.get(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert!(
        details.iter().any(|d| d.contains("\"canDownloadBanks\"")),
        "{details:?}"
    );
    assert!(
        details.iter().any(|d| !d.contains("canDownloadBanks")),
        "{details:?}"
    );
}

// ---------------------------------------------------------------- migration

#[test]
fn a_fresh_database_is_the_current_schema_and_a_schema_9_file_migrates_with_one_row() {
    let dir = dbc::tmp();
    let fresh = dir.path().join("fresh").join("xexam.db");
    drop(dbc::open_fixed(&fresh));
    let schema = |c: &Connection| -> String {
        c.query_row("SELECT value FROM meta WHERE key='schema'", [], |r| {
            r.get(0)
        })
        .expect("schema")
    };
    let c = raw(&fresh);
    assert_eq!(schema(&c), xexam_backend::database::schema::SCHEMA_VERSION);
    let n: i64 = c
        .query_row("SELECT COUNT(*) FROM activity_log", [], |r| r.get(0))
        .expect("count");
    assert_eq!(n, 0);
    drop(c);

    let db = dir.path().join("xexam.db");
    // A genuine schema-9 file (what a v1.5 binary leaves; v2.0: built from the schema history)
    dbc::historic(&db, 9);
    raw(&db)
        .execute_batch(
            "INSERT INTO users(id, username, password_hash, role, created_at, password_changed_at, new_banks) VALUES                (1, 'admin', 'placeholder-hash', 'admin', '2026-11-01 07:00', '2026-11-01 07:00', 1),                (2, 'ann', 'placeholder-hash', 'user', '2026-11-01 07:00', '2026-11-01 07:00', 1);",
        )
        .expect("schema-9 users");
    let s = dbc::open_fixed(&db);
    let backup = s.migration_backup().expect("a backup").to_path_buf();
    assert!(backup
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("xexam-schema9-")));
    let ann = s.user_by_name("ann").expect("db").expect("ann");
    assert!(!ann.can_download_banks);
    drop(s);
    let c = raw(&db);
    assert_eq!(schema(&c), xexam_backend::database::schema::SCHEMA_VERSION);
    let stored: i64 = c
        .query_row(
            "SELECT can_download_banks FROM users WHERE username='ann'",
            [],
            |r| r.get(0),
        )
        .expect("column");
    assert_eq!(stored, 0);
    let rows: Vec<String> = c
        .prepare("SELECT detail FROM activity_log WHERE action = 'system.migrate'")
        .expect("prepare")
        .query_map([], |r| r.get(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(rows, vec![r#"{"from":9,"to":11}"#.to_string()]);
    // fresh and migrated databases have the same users columns, and the next open writes nothing
    let cols = |c: &Connection| -> Vec<String> {
        c.prepare("SELECT name FROM pragma_table_info('users') ORDER BY name")
            .expect("prepare")
            .query_map([], |r| r.get(0))
            .expect("query")
            .map(|r| r.expect("row"))
            .collect()
    };
    assert_eq!(cols(&c), cols(&raw(&fresh)));
    drop(c);
    let s = dbc::open_fixed(&db);
    assert!(s.migration_backup().is_none());
}

// ---------------------------------------------------------------- v1.9: the `langs` query

fn dl_langs(id: i64, format: &str, langs: &str) -> String {
    format!("/api/banks/download?id={id}&format={format}&langs={langs}")
}

#[tokio::test]
async fn a_language_subset_changes_the_file_name_and_the_activity_detail() {
    let a = app();
    let adm = admin(&a);
    let id = import(&a, &adm, "new", "Soup bank.xlsx", &sample("one")).await;
    // zh only: Excel, suffix `_zh`
    let r = get(&a, &dl_langs(id, "xlsx", "zh"), &adm).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let cd = r.header("content-disposition").expect("disposition");
    assert!(cd.contains("filename=\"Soup bank_zh.xlsx\""), "{cd}");
    assert!(cd.contains("Soup%20bank_zh.xlsx"), "{cd}");
    // en,ja given in another order and as an encoded comma: the suffix keeps en, zh, ja order
    let r = get(&a, &dl_langs(id, "docx", "ja%2Cen"), &adm).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let cd = r.header("content-disposition").expect("disposition");
    assert!(cd.contains("filename=\"Soup bank_en-ja.docx\""), "{cd}");
    let mut z = zip::ZipArchive::new(Cursor::new(r.body.clone())).expect("zip");
    let mut doc = String::new();
    z.by_name("word/document.xml")
        .expect("document.xml")
        .read_to_string(&mut doc)
        .expect("read");
    assert!(doc.contains("日本語") && doc.contains("晴れた日") && !doc.contains("晴天时"));
    // all three given explicitly = the plain name
    let r = get(&a, &dl_langs(id, "xlsx", "en,zh,ja"), &adm).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let cd = r.header("content-disposition").expect("disposition");
    assert!(cd.contains("filename=\"Soup bank.xlsx\""), "{cd}");
    // no `langs` = unchanged v1.8 name
    let r = get(&a, &dl(id, "docx"), &adm).await;
    let cd = r.header("content-disposition").expect("disposition");
    assert!(cd.contains("filename=\"Soup bank.docx\""), "{cd}");
    a.state.0.activity.flush().await;
    let details: Vec<Value> = activity_rows(&a)
        .into_iter()
        .map(|r| serde_json::from_str(&r.3.expect("detail")).expect("json"))
        .collect();
    assert_eq!(
        details,
        vec![
            json!({"format": "xlsx", "langs": ["zh"], "sheets": 2, "questions": 5}),
            json!({"format": "docx", "langs": ["en", "ja"], "sheets": 2, "questions": 5}),
            json!({"format": "xlsx", "langs": ["en", "zh", "ja"], "sheets": 2, "questions": 5}),
            json!({"format": "docx", "langs": ["en", "zh", "ja"], "sheets": 2, "questions": 5}),
        ]
    );
}

#[tokio::test]
async fn bad_langs_are_400_before_the_bank_lookup_and_leave_no_activity_row() {
    let a = app();
    let adm = admin(&a);
    let id = import(&a, &adm, "new", "one.xlsx", &sample("one")).await;
    for langs in [
        "", "fr", "ZH", "zh,zh", "zh,,en", "en,", "en%20,zh", "zh-CN",
    ] {
        let r = get(&a, &dl_langs(id, "xlsx", langs), &adm).await;
        assert_eq!(r.status, 400, "langs={langs:?}: {}", r.text());
    }
    // given twice
    let twice = format!("/api/banks/download?id={id}&format=xlsx&langs=zh&langs=en");
    assert_eq!(get(&a, &twice, &adm).await.status, 400);
    // checked before the lookup: a missing bank with bad langs is 400, not 404
    assert_eq!(
        get(&a, &dl_langs(9999, "xlsx", "fr"), &adm).await.status,
        400
    );
    assert_eq!(
        get(&a, &dl_langs(9999, "xlsx", "zh"), &adm).await.status,
        404
    );
    a.state.0.activity.flush().await;
    assert!(activity_rows(&a).is_empty());
}

#[tokio::test]
async fn head_with_langs_matches_get_headers_and_has_no_body() {
    let a = app();
    let adm = admin(&a);
    let id = import(&a, &adm, "new", "one.xlsx", &sample("one")).await;
    let r = call(&a, "HEAD", &dl_langs(id, "docx", "en,zh"), Some(&adm)).await;
    assert_eq!(r.status, 200);
    assert!(r.body.is_empty());
    let cd = r.header("content-disposition").expect("disposition");
    assert!(cd.contains("filename=\"one_en-zh.docx\""), "{cd}");
    assert_eq!(
        call(&a, "HEAD", &dl_langs(id, "docx", "xx"), Some(&adm))
            .await
            .status,
        400
    );
}
