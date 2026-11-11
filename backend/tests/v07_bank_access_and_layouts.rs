//! v0.7, before its release: the question bank access of a plain user is two independent settings, and only the six-column
//! workbook layout may be imported, while the questions of the old 3- / 4-column layouts are retired.
//!
//! - "The chosen existing banks" (their `user_banks` rows) and "Include banks imported later" (`users.new_banks`): a new bank
//!   gets a row for every user with the flag on, in the same transaction; existing banks are never added by the flag. The
//!   pre-release v4 shape (`users.all_banks`) is converted on open, after a backup.
//! - Every import entry point (upload for a new bank or an update, the import confirm, the `import` command) refuses a 3- or
//!   4-column workbook before anything is staged or written.
//! - `Store::retire_old_layout_questions` (run by the server and the command-line program on every start) retires every
//!   active question of the old layouts, keeping study records, the notebook and the banks.
//!
//! Real router, real engine and database, production rules (the old layouts are refused). Made-up data only.
#[path = "api_common/mod.rs"]
mod apic;
#[path = "common/mod.rs"]
mod dbc;

use apic::{build, send, Resp};
use axum::body::Body;
use axum::Router;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use xexam_backend::api::{build_router, AppState};
use xexam_backend::database::schema::{
    MIGRATE_V2_INDEXES, MIGRATE_V2_TABLES, MIGRATE_V3_INDEXES, MIGRATE_V3_TABLES, SCHEMA,
    V2_COLUMNS, V3_COLUMNS,
};
use xexam_backend::database::{RetiredBank, Store};
use xexam_backend::models::{Attempt, AttemptResult, BankScope, ImportTarget};
use xexam_backend::services::{self, OldLayouts, RealEngine};

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";
const ADMIN_PW: &str = "garden-admin-1";
const MSG_OLD_LAYOUT: &str = "只能导入六列格式（A–C 列：英文、中文、日文题目；D–F 列：英文、中文、日文答案）。这个文件是旧的三列或四列格式，请改成六列格式后再导入。";

// ---------------------------------------------------------------- made-up workbooks

const WB: &str = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Soup" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
const RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Target="worksheets/a.xml"/></Relationships>"#;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A one-sheet .xlsx ("Soup") whose rows are the given cell texts, from column A on.
fn xlsx(rows: &[Vec<String>]) -> Vec<u8> {
    let mut sheet = String::new();
    for (i, cols) in rows.iter().enumerate() {
        let r = i + 1;
        sheet.push_str(&format!(r#"<row r="{r}">"#));
        for (k, text) in cols.iter().enumerate() {
            let letter = (b'A' + u8::try_from(k).expect("column")) as char;
            sheet.push_str(&format!(
                r#"<c r="{letter}{r}" t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#,
                esc(text)
            ));
        }
        sheet.push_str("</row>");
    }
    let xml = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{sheet}</sheetData></worksheet>"#
    );
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (name, bytes) in [
        ("xl/workbook.xml", WB.as_bytes()),
        ("xl/_rels/workbook.xml.rels", RELS.as_bytes()),
        ("xl/worksheets/a.xml", xml.as_bytes()),
    ] {
        w.start_file(name, opts).expect("start");
        w.write_all(bytes).expect("write");
    }
    w.finish().expect("finish").into_inner()
}

/// A six-column workbook of `n` made-up single-choice questions (`tag` makes the questions of two workbooks differ).
fn six_columns(n: usize, tag: &str) -> Vec<u8> {
    let rows: Vec<Vec<String>> = (1..=n)
        .map(|i| {
            vec![
                format!("A cook makes soup every evening ({tag} {i}). Which spoon is used for tasting?\n\nA. The wooden spoon\n\nB. The ladle\n\nC. The teaspoon"),
                format!("【中文翻译】\n一位厨师每天晚上做汤（{tag} {i}）。尝味道用哪把勺子？\nA. 木勺\nB. 汤勺\nC. 茶匙"),
                format!("料理人が毎晩スープを作ります（{tag} {i}）。味見に使うのはどのスプーンですか。\n\nA. 木のスプーン\n\nB. お玉\n\nC. ティースプーン"),
                "【Answer】C\n\n【Explanation】\nA small spoon is enough for tasting.".to_string(),
                "【答案】C\n\n【解析·怎么选】\n尝味道用小勺就够了。".to_string(),
                "【正解】C\n\n【解説】\n味見には小さなスプーンで十分です。".to_string(),
            ]
        })
        .collect();
    xlsx(&rows)
}

/// A four-column workbook (A English, B Chinese, C Japanese, D the answer): the old layout of v0.6.
fn four_columns() -> Vec<u8> {
    xlsx(&[vec![
        "This is a sufficiently long English question about soup. Which is correct?\n\nA. Alpha\n\nB. Beta\n\nC. Gamma".to_string(),
        "【中文翻译】\n这是一个关于汤的足够长的中文题目。哪个正确？\nA. 甲\nB. 乙\nC. 丙".to_string(),
        "これはスープについての十分に長い日本語の問題文です。どれが正しいですか。\n\nA. 甲です\n\nB. 乙です\n\nC. 丙です".to_string(),
        "【答案】B\n\n【解析·怎么选】因为乙。".to_string(),
    ]])
}

/// The committed three-column demo workbook (made-up questions), the layout of v0.4 and earlier.
fn three_columns() -> Vec<u8> {
    std::fs::read(dbc::baseline("fixtures/demo.xlsx")).expect("demo.xlsx")
}

// ---------------------------------------------------------------- helpers

struct App {
    router: Router,
    state: AppState,
    store: Arc<Store>,
    db: PathBuf,
    _dir: tempfile::TempDir,
}

/// The real router with the production rules: `AppState::new` (old layouts refused).
fn app() -> App {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    let store = Arc::new(Store::open(&db).expect("store"));
    let engine = Arc::new(RealEngine::new(store.clone()));
    let state = AppState::new(apic::default_config(), store.clone(), engine, None);
    assert_eq!(state.0.old_layouts, OldLayouts::Refuse);
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

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).expect("count")
}

async fn call(a: &App, method: &str, path: &str, cookie: &str, body: Option<&str>) -> Resp {
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

async fn get(a: &App, path: &str, cookie: &str) -> Resp {
    call(a, "GET", path, cookie, None).await
}

async fn post(a: &App, path: &str, cookie: &str, body: Value) -> Resp {
    call(a, "POST", path, cookie, Some(&body.to_string())).await
}

/// `POST /api/upload`; `target`: the `X-Bank-Target` header.
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

/// Uploads and confirms; returns the bank id.
async fn import(a: &App, cookie: &str, target: &str, name: &str, data: &[u8]) -> i64 {
    let r = upload(a, cookie, target, name, data).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let token = r.json()["token"].as_str().expect("token").to_string();
    let r = post(a, "/api/import/confirm", cookie, json!({ "token": token })).await;
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()["bankId"].as_i64().expect("bank id")
}

fn cookie_of(r: &Resp) -> String {
    let sc = r.header("set-cookie").expect("set-cookie");
    sc.split(';').next().expect("pair").to_string()
}

async fn login(a: &App, name: &str, password: &str) -> Resp {
    let body = json!({"username": name, "password": password}).to_string();
    let len = body.len().to_string();
    let hs: Vec<(&str, &[u8])> = vec![
        ("origin", ORIGIN.as_bytes()),
        ("content-type", b"application/json"),
        ("content-length", len.as_bytes()),
    ];
    send(
        &a.router,
        build("POST", "/api/auth/login", Some(HOST), &hs, Body::from(body)),
    )
    .await
}

async fn admin(a: &App) -> String {
    let c = cookie_of(&login(a, "admin", "admin").await);
    let r = post(a, "/api/auth/password", &c, json!({ "password": ADMIN_PW })).await;
    assert_eq!(r.status, 200, "{}", r.text());
    cookie_of(&r)
}

/// Creates a plain user with the fields of `extra`, signs in as them and sets their own password. Returns (id, cookie).
async fn user(a: &App, admin_cookie: &str, name: &str, extra: Value) -> (i64, String) {
    let mut body = json!({"username": name, "password": "initial-pass", "role": "user"});
    if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    let r = post(a, "/api/users/create", admin_cookie, body).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let id = row(&r.json(), name)["id"].as_i64().expect("id");
    let c = cookie_of(&login(a, name, "initial-pass").await);
    let r = post(
        a,
        "/api/auth/password",
        &c,
        json!({ "password": format!("{name}-own-pass") }),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    (id, cookie_of(&r))
}

fn row(users: &Value, name: &str) -> Value {
    users["users"]
        .as_array()
        .expect("users")
        .iter()
        .find(|u| u["username"] == name)
        .cloned()
        .expect("user in the list")
}

/// The bank ids `cookie` sees in `GET /api/banks`.
async fn seen(a: &App, cookie: &str) -> Vec<i64> {
    get(a, "/api/banks", cookie).await.json()["banks"]
        .as_array()
        .expect("banks")
        .iter()
        .filter_map(|b| b["id"].as_i64())
        .collect()
}

fn backups(dir: &Path) -> Vec<PathBuf> {
    match std::fs::read_dir(dir.join("backups")) {
        Ok(rd) => rd.map(|e| e.expect("entry").path()).collect(),
        Err(_) => Vec::new(),
    }
}

fn cli(args: &[&str]) -> std::process::Output {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_xexam-backend"));
    for (k, _) in std::env::vars() {
        if k.starts_with("XEXAM_") {
            cmd.env_remove(k);
        }
    }
    cmd.args(args).output().expect("run xexam-backend")
}

/// A schema-3 database (as v0.6 wrote it): administrators `boss` (1) and `lee` (2), plain user `cook` (3); banks Herbs (1,
/// legacy) and Spices (2) with old-layout questions (no `lang`), Spices also with one six-column question (`"lang": "en"`);
/// statistics and one exam record per question of the old layout.
fn write_v3_database(db: &Path) {
    let c = raw(db);
    c.execute_batch(SCHEMA).expect("v1");
    c.execute_batch(MIGRATE_V2_TABLES).expect("v2 tables");
    for (table, _, definition) in V2_COLUMNS {
        c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {definition};"))
            .expect("v2 column");
    }
    c.execute_batch(MIGRATE_V2_INDEXES).expect("v2 index");
    c.execute_batch(MIGRATE_V3_TABLES).expect("v3 tables");
    for (table, _, definition) in V3_COLUMNS {
        c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {definition};"))
            .expect("v3 column");
    }
    c.execute_batch(MIGRATE_V3_INDEXES).expect("v3 index");
    c.execute_batch(
        r#"
        INSERT INTO users(id, username, password_hash, role, disabled, created_at, password_changed_at, must_change_password) VALUES
          (1, 'boss', 'x', 'admin', 0, '2026-01-01 10:00', '2026-01-01 10:00', 0),
          (2, 'lee', 'x', 'admin', 0, '2026-01-01 10:00', '2026-01-01 10:00', 0),
          (3, 'cook', 'x', 'user', 0, '2026-01-01 10:00', '2026-01-01 10:00', 0);
        INSERT INTO banks(id, name, filename, legacy_ids, created_at, updated_at) VALUES
          (1, 'Herbs', 'Herbs.xlsx', 1, '2026-01-01 10:00', '2026-01-01 10:00'),
          (2, 'Spices', 'Spices.xlsx', 0, '2026-01-02 10:00', '2026-01-02 10:00');
        INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id) VALUES
          ('h1', 'Herbs', 1, 0, '{"kind": "single", "stem": "Basil or mint?"}', 1, '2026-01-01 10:00', '2026-01-01 10:00', 1),
          ('h2', 'Herbs', 2, 1, '{"kind": "single", "stem": "Thyme or sage?"}', 1, '2026-01-01 10:00', '2026-01-01 10:00', 1),
          ('h0', 'Herbs', 3, 2, '{"kind": "single", "stem": "Retired before?"}', 0, '2026-01-01 10:00', '2026-01-01 10:00', 1),
          ('s1', 'Spices', 1, 0, '{"kind": "single", "stem": "Pepper or salt?"}', 1, '2026-01-02 10:00', '2026-01-02 10:00', 2),
          ('s6', 'Spices', 2, 1, '{"kind": "single", "stem": "Which spice is red?", "lang": "en"}', 1, '2026-01-02 10:00', '2026-01-02 10:00', 2);
        INSERT INTO user_stats(user_id, question_id, attempts, correct, wrong, streak, last, at) VALUES
          (1, 'h1', 1, 0, 1, 0, 0, '2026-01-03 10:00'),
          (3, 's1', 2, 1, 1, 0, 0, '2026-01-03 10:00');
        INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, banks, user_id) VALUES
          ('2026-01-03 10:00', '["Herbs"]', 'all', 1, 0, 0, 20, '[{"id": 1, "name": "Herbs"}]', 1),
          ('2026-01-03 11:00', '["Spices"]', 'all', 1, 1, 100, 25, '[{"id": 2, "name": "Spices"}]', 3);
        INSERT INTO attempt_items(attempt_id, seq, question_id, answer, correct) VALUES (1, 0, 'h1', '["A"]', 0), (2, 0, 's1', '["B"]', 1);
        INSERT INTO meta(key, value) VALUES('schema', '3');
        "#,
    )
    .expect("v3 data");
}

/// The same database in the shape a pre-release build of v0.7 wrote: schema 4 with `users.all_banks`. boss and cook have
/// `all_banks = 1` (no rows), lee `all_banks = 0` with a row for Spices only.
fn write_prerelease_v4_database(db: &Path) {
    write_v3_database(db);
    raw(db)
        .execute_batch(
            r#"
            CREATE TABLE user_banks (user_id INTEGER NOT NULL, bank_id INTEGER NOT NULL, PRIMARY KEY (user_id, bank_id));
            CREATE INDEX idx_user_banks_bank ON user_banks(bank_id);
            ALTER TABLE users ADD COLUMN all_banks INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE users ADD COLUMN can_manage_banks INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE banks ADD COLUMN owner_id INTEGER;
            ALTER TABLE attempts ADD COLUMN kind TEXT NOT NULL DEFAULT 'exam' CHECK (kind IN ('exam', 'practice'));
            CREATE INDEX idx_banks_owner ON banks(owner_id);
            UPDATE banks SET owner_id = 1;
            UPDATE users SET all_banks = 1 WHERE id IN (1, 3);
            INSERT INTO user_banks(user_id, bank_id) VALUES (2, 2);
            INSERT INTO meta(key, value) VALUES ('users_known', '3');
            UPDATE meta SET value = '4' WHERE key = 'schema';
            "#,
        )
        .expect("pre-release v4");
}

// ---------------------------------------------------------------- storage

#[test]
fn a_prerelease_v4_database_is_converted_on_open_after_a_backup() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    write_prerelease_v4_database(&db);
    let s = Store::open(&db).expect("open");
    assert!(s.converted_prerelease());
    let backup = s.migration_backup().expect("backup").to_path_buf();
    let name = backup.file_name().and_then(|n| n.to_str()).expect("name");
    assert!(name.starts_with("xexam-schema4-"), "{name}");
    assert_eq!(backups(dir.path()).len(), 1);
    // the backup is the pre-release shape, untouched
    assert_eq!(
        count(
            &raw(&backup),
            "SELECT COUNT(*) FROM pragma_table_info('users') WHERE name='all_banks'"
        ),
        1
    );
    let c = raw(&db);
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM pragma_table_info('users') WHERE name='all_banks'"
        ),
        0,
        "all_banks is kept nowhere"
    );
    // all_banks = 1: every existing bank and the banks imported later; all_banks = 0: the flag off, the rows kept
    let flags: Vec<(i64, i64)> = c
        .prepare("SELECT id, new_banks FROM users ORDER BY id")
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(flags, [(1, 1), (2, 0), (3, 1)]);
    assert_eq!(
        s.viewer(3).expect("viewer").scope,
        BankScope::Only([1, 2].into())
    );
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM user_banks WHERE user_id=2"),
        1
    );
    let known: String = c
        .query_row("SELECT value FROM meta WHERE key='banks_known'", [], |r| {
            r.get(0)
        })
        .expect("mark");
    assert_eq!(known, "2");
    // nothing else changed: questions, records, statistics
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM questions WHERE active=1"),
        4
    );
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempts"), 2);
    drop(c);
    drop(s);
    // the next open has nothing to do
    let bytes = dbc::db_bytes(&db);
    let s = Store::open(&db).expect("reopen");
    assert!(!s.converted_prerelease());
    assert!(s.migration_backup().is_none());
    drop(s);
    assert_eq!(dbc::db_bytes(&db), bytes);
    assert_eq!(backups(dir.path()).len(), 1);
}

#[test]
fn a_user_an_older_binary_added_to_a_prerelease_database_gets_every_bank_on_conversion() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    write_prerelease_v4_database(&db);
    // after the last pre-release open, an older binary (v0.6) created `dana` as it does: no access rows, `all_banks` left
    // at its default 0, the mark (`users_known` = 3) not moved
    raw(&db)
        .execute_batch(
            "INSERT INTO users(id, username, password_hash, role, disabled, created_at, password_changed_at, \
             must_change_password) VALUES (4, 'dana', 'x', 'user', 0, '2026-02-01 10:00', '2026-02-01 10:00', 0);",
        )
        .expect("older binary user");
    let s = Store::open(&db).expect("open");
    assert!(s.converted_prerelease());
    let c = raw(&db);
    // dana sees every bank and the banks imported later, as every user did in the older version
    assert_eq!(
        s.viewer(4).expect("viewer").scope,
        BankScope::Only([1, 2].into())
    );
    let flags: Vec<(i64, i64)> = c
        .prepare("SELECT id, new_banks FROM users ORDER BY id")
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(flags, [(1, 1), (2, 0), (3, 1), (4, 1)]);
    // the restricted pre-release user keeps only their own row
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM user_banks WHERE user_id=2"),
        1
    );
    let known: String = c
        .query_row("SELECT value FROM meta WHERE key='users_known'", [], |r| {
            r.get(0)
        })
        .expect("mark");
    assert_eq!(known, "4");
    drop(c);
    drop(s);
    // the next open has nothing to do
    let bytes = dbc::db_bytes(&db);
    let s = Store::open(&db).expect("reopen");
    assert!(!s.converted_prerelease());
    drop(s);
    assert_eq!(dbc::db_bytes(&db), bytes);
}

#[test]
fn a_new_bank_reaches_only_the_users_with_banks_imported_later() {
    let dir = dbc::tmp();
    let s = Store::open(&dir.path().join("x.db")).expect("open");
    let six = |id: &str| {
        let mut q = dbc::single("Soup", id, &format!("Made-up question {id}?"), 1);
        q.lang = "en".to_string();
        q
    };
    let (_, first) = s
        .apply_import_for(
            ImportTarget::New,
            "First.xlsx",
            &[dbc::sheet("Soup", 1)],
            &[six("a")],
        )
        .expect("import")
        .expect("bank");
    let hash = auth_hash();
    let made = |name: &str, banks: Vec<i64>, later: bool| match s
        .create_user_with(
            name,
            &hash,
            xexam_backend::auth::Role::User,
            &xexam_backend::models::BankAccess::Only(banks),
            later,
            false,
        )
        .expect("create")
    {
        xexam_backend::database::UserChange::Done(id) => id,
        other => panic!("{other:?}"),
    };
    let later = made("later", vec![], true);
    let fixed = made("fixed", vec![first], false);
    // existing banks are never added by the flag
    assert_eq!(
        s.viewer(later).expect("v").scope,
        BankScope::Only(Default::default())
    );
    // a new bank (as an import as a new bank does) and the legacy bank the `import` command creates
    let (_, second) = s
        .apply_import_for(
            ImportTarget::New,
            "Second.xlsx",
            &[dbc::sheet("Soup", 1)],
            &[six("b")],
        )
        .expect("import")
        .expect("bank");
    s.apply_import("Third.xlsx", &[dbc::sheet("Soup", 1)], &[six("c")])
        .expect("import command");
    let third = s.banks().expect("banks")["banks"][2]["id"]
        .as_i64()
        .expect("id");
    assert_eq!(
        s.viewer(later).expect("v").scope,
        BankScope::Only([second, third].into())
    );
    assert_eq!(
        s.viewer(fixed).expect("v").scope,
        BankScope::Only([first].into())
    );
}

fn auth_hash() -> String {
    xexam_backend::auth::hash_password("initial-pass").expect("hash")
}

// ---------------------------------------------------------------- the users API and the flag

#[tokio::test]
async fn new_banks_reach_flagged_users_when_an_administrator_or_a_bank_manager_imports() {
    let a = app();
    let c = admin(&a).await;
    let first = import(&a, &c, "new", "First.xlsx", &six_columns(2, "first")).await;
    let (_, later) = user(&a, &c, "later", json!({"newBanks": true})).await;
    let (_, plain) = user(&a, &c, "plain", json!({"banks": [first]})).await;
    let (maker_id, maker) = user(&a, &c, "maker", json!({"canManageBanks": true})).await;
    assert_eq!(
        seen(&a, &later).await,
        Vec::<i64>::new(),
        "existing banks are not added"
    );
    // an administrator imports a new bank
    let second = import(&a, &c, "new", "Second.xlsx", &six_columns(2, "second")).await;
    assert_eq!(seen(&a, &later).await, [second]);
    assert_eq!(seen(&a, &plain).await, [first]);
    assert_eq!(seen(&a, &maker).await, Vec::<i64>::new());
    // a bank manager imports a new bank: it reaches the flagged users too, and its owner sees it as their own
    let third = import(&a, &maker, "new", "Third.xlsx", &six_columns(2, "third")).await;
    assert_eq!(seen(&a, &later).await, [second, third]);
    assert_eq!(seen(&a, &plain).await, [first]);
    assert_eq!(seen(&a, &maker).await, [third]);
    // updating a bank is not a new bank: nobody gains anything
    import(
        &a,
        &c,
        &first.to_string(),
        "First v2.xlsx",
        &six_columns(3, "first"),
    )
    .await;
    assert_eq!(seen(&a, &later).await, [second, third]);
    // what /api/users says: the rows and the flag
    let users = get(&a, "/api/users", &c).await.json();
    let l = row(&users, "later");
    assert_eq!(
        (l["banks"].clone(), l["newBanks"].clone()),
        (json!([second, third]), json!(true))
    );
    let m = row(&users, "maker");
    assert_eq!(
        (
            m["banks"].clone(),
            m["newBanks"].clone(),
            m["ownedBanks"].clone(),
            m["id"].clone()
        ),
        (json!([]), json!(false), json!([third]), json!(maker_id))
    );
}

#[tokio::test]
async fn unticking_one_bank_while_banks_imported_later_are_included_removes_only_that_bank() {
    let a = app();
    let c = admin(&a).await;
    let general = import(
        &a,
        &c,
        "new",
        "General knowledge.xlsx",
        &six_columns(2, "general"),
    )
    .await;
    let kitchen = import(
        &a,
        &c,
        "new",
        "Kitchen basics.xlsx",
        &six_columns(2, "kitchen"),
    )
    .await;
    let (id, uc) = user(&a, &c, "alex", json!({"banks": "all"})).await;
    assert_eq!(seen(&a, &uc).await, [general, kitchen]);
    // the owner's example: General knowledge ticked, Kitchen basics not, banks imported later included
    let r = post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": id, "banks": [general]}),
    )
    .await;
    let alex = row(&r.json(), "alex");
    assert_eq!(
        (alex["banks"].clone(), alex["newBanks"].clone()),
        (json!([general]), json!(true))
    );
    assert_eq!(seen(&a, &uc).await, [general]);
    let later = import(&a, &c, "new", "Later.xlsx", &six_columns(1, "later")).await;
    assert_eq!(
        seen(&a, &uc).await,
        [general, later],
        "General knowledge and the new bank, not Kitchen basics"
    );
    // the exam questions follow: none of Kitchen basics
    let bank = get(&a, "/api/bank", &uc).await.json();
    assert!(bank["questions"]
        .as_array()
        .expect("questions")
        .iter()
        .all(|q| q["bank"] != json!(kitchen)));
}

// ---------------------------------------------------------------- only the six-column layout

#[tokio::test]
async fn uploads_of_the_old_layouts_are_refused_before_anything_is_staged_or_written() {
    let a = app();
    let c = admin(&a).await;
    let bank = import(&a, &c, "new", "Soup.xlsx", &six_columns(2, "soup")).await;
    // a staged six-column upload, to show that a refused upload does not replace it
    let r = upload(&a, &c, "new", "Staged.xlsx", &six_columns(1, "staged")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let token = r.json()["token"].as_str().expect("token").to_string();
    let before = dbc::db_bytes(&a.db);
    let update = bank.to_string();
    for (target, name, data) in [
        ("new", "three.xlsx", three_columns()),
        ("new", "four.xlsx", four_columns()),
        (update.as_str(), "three.xlsx", three_columns()),
        (update.as_str(), "four.xlsx", four_columns()),
    ] {
        let r = upload(&a, &c, target, name, &data).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (400, MSG_OLD_LAYOUT.to_string()),
            "{target} {name}"
        );
    }
    assert_eq!(dbc::db_bytes(&a.db), before, "nothing was written");
    // the staged upload is still the six-column one
    let r = post(&a, "/api/import/confirm", &c, json!({ "token": token })).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["result"]["added"], 1);
    // a six-column file still updates the bank
    import(
        &a,
        &c,
        &bank.to_string(),
        "Soup v2.xlsx",
        &six_columns(3, "soup"),
    )
    .await;
    let c2 = raw(&a.db);
    assert_eq!(
        count(
            &c2,
            &format!("SELECT COUNT(*) FROM questions WHERE bank_id={bank} AND active=1")
        ),
        3
    );
    assert_eq!(count(&c2, "SELECT COUNT(*) FROM imports"), 3);
}

#[tokio::test]
async fn the_import_confirm_refuses_an_old_layout_too() {
    let a = app();
    let c = admin(&a).await;
    let before = dbc::db_bytes(&a.db);
    // an old-layout workbook can only be staged by bypassing the upload route (production never does): the confirm still
    // refuses it and writes nothing
    let wb = xexam_backend::parser::parse_workbook(&three_columns()).expect("parse");
    assert!(wb.has_old_layout());
    let token = a.state.0.pending.put_named(
        1,
        "admin",
        ImportTarget::New,
        "three.xlsx".to_string(),
        wb.sheets,
        wb.questions,
    );
    let r = post(&a, "/api/import/confirm", &c, json!({ "token": token })).await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (400, MSG_OLD_LAYOUT.to_string())
    );
    assert_eq!(dbc::db_bytes(&a.db), before);
    assert_eq!(a.store.bank_count().expect("count"), 0);
}

#[test]
fn check_layout_refuses_old_layouts_unless_a_test_allows_them() {
    let old = xexam_backend::parser::parse_workbook(&three_columns()).expect("parse");
    let four = xexam_backend::parser::parse_workbook(&four_columns()).expect("parse");
    let six = xexam_backend::parser::parse_workbook(&six_columns(2, "x")).expect("parse");
    assert!(old.has_old_layout() && four.has_old_layout() && !six.has_old_layout());
    assert_eq!(six.questions.len(), 2);
    for wb in [&old, &four] {
        let e = services::check_layout(&wb.questions, OldLayouts::Refuse, "x.xlsx")
            .expect_err("refused");
        assert_eq!(e.to_string(), MSG_OLD_LAYOUT);
        assert!(services::check_layout(&wb.questions, OldLayouts::AllowForTests, "x.xlsx").is_ok());
    }
    assert!(services::check_layout(&six.questions, OldLayouts::Refuse, "x.xlsx").is_ok());
}

#[test]
fn the_import_command_refuses_the_old_layouts_and_imports_six_columns() {
    let dir = dbc::tmp();
    let data = dir.path().to_str().expect("utf-8 path").to_string();
    let files = dbc::tmp();
    for (name, bytes) in [
        ("three.xlsx", three_columns()),
        ("four.xlsx", four_columns()),
    ] {
        let path = files.path().join(name);
        std::fs::write(&path, bytes).expect("write");
        let out = cli(&["import", path.to_str().expect("path"), "--data", &data]);
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        assert_eq!(out.status.code(), Some(1), "{name}: {stdout}");
        assert!(
            stdout.contains("Import failed: the workbook uses the old 3- or 4-column layout; only the six-column layout can be imported.")
                && stdout.contains("Nothing was imported."),
            "{name}: {stdout}"
        );
        let c = raw(&dir.path().join("xexam.db"));
        assert_eq!(count(&c, "SELECT COUNT(*) FROM banks"), 0, "{name}");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM questions"), 0, "{name}");
    }
    let path = files.path().join("six.xlsx");
    std::fs::write(&path, six_columns(3, "cli")).expect("write");
    let out = cli(&["import", path.to_str().expect("path"), "--data", &data]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("Imported six.xlsx: 3 added"), "{stdout}");
    let c = raw(&dir.path().join("xexam.db"));
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM questions WHERE active=1"),
        3
    );
}

// ---------------------------------------------------------------- the old layouts' questions are retired

#[test]
fn old_layout_questions_are_retired_once_keeping_records_notebook_and_banks() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    write_v3_database(&db);
    let s = Store::open(&db).expect("open");
    assert!(s.migration_backup().is_some(), "v3 -> v4 with a backup");
    let retired = s.retire_old_layout_questions().expect("retire");
    assert_eq!(
        retired,
        [
            RetiredBank {
                id: Some(1),
                name: "Herbs".to_string(),
                retired: 2
            },
            RetiredBank {
                id: Some(2),
                name: "Spices".to_string(),
                retired: 1
            },
        ]
    );
    let c = raw(&db);
    // the old-layout questions are retired, the six-column one is untouched; nothing is deleted
    assert_eq!(count(&c, "SELECT COUNT(*) FROM questions"), 5);
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM questions WHERE active=1"),
        1
    );
    assert_eq!(
        count(&c, "SELECT active FROM questions WHERE id='s6'"),
        1,
        "six-column"
    );
    // retired after the bank's other questions, in their order
    let seqs: Vec<(String, i64)> = c
        .prepare("SELECT id, seq FROM questions WHERE bank_id=1 ORDER BY seq")
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(
        seqs,
        [
            ("h0".to_string(), 2),
            ("h1".to_string(), 3),
            ("h2".to_string(), 4)
        ]
    );
    // records, statistics (the notebook's data), banks and import history stay
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempts"), 2);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM attempt_items"), 2);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM user_stats"), 2);
    assert_eq!(count(&c, "SELECT COUNT(*) FROM banks"), 2);
    // what the pages read: no question of Herbs is offered, the bank page counts them as retired
    let banks = s.banks().expect("banks");
    assert_eq!(
        (
            banks["banks"][0]["total"].clone(),
            banks["banks"][0]["inactive"].clone()
        ),
        (json!(0), json!(3))
    );
    assert_eq!(
        (
            banks["banks"][1]["total"].clone(),
            banks["banks"][1]["inactive"].clone()
        ),
        (json!(1), json!(1))
    );
    let bank = s.bank().expect("bank");
    assert_eq!(bank["questions"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        s.records(3).expect("records")["records"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    // an exam that was in progress with a retired question is still counted, as after an import that retired it
    let p = s
        .record_attempt(
            3,
            &Attempt {
                results: vec![AttemptResult {
                    id: "s1".to_string(),
                    correct: true,
                }],
                sheets: vec!["Spices".to_string()],
                mode: "all".to_string(),
                seconds: 9,
                kind: Default::default(),
            },
        )
        .expect("record");
    assert_eq!(p["questions"]["s1"]["attempts"], 3);
    drop(c);
    drop(s);
    // a second start retires nothing and writes nothing
    let bytes = dbc::db_bytes(&db);
    let s = Store::open(&db).expect("reopen");
    assert_eq!(s.retire_old_layout_questions().expect("again"), []);
    drop(s);
    assert_eq!(dbc::db_bytes(&db), bytes);
    // an old-layout question an older binary imported after a rollback is retired at the next start
    raw(&db)
        .execute_batch(
            "INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id) VALUES \
             ('h3', 'Herbs', 4, 0, '{\"kind\": \"single\", \"stem\": \"Dill or fennel?\"}', 1, '2026-02-01 10:00', '2026-02-01 10:00', 1)",
        )
        .expect("rollback import");
    let s = Store::open(&db).expect("reopen");
    assert_eq!(
        s.retire_old_layout_questions().expect("retire"),
        [RetiredBank {
            id: Some(1),
            name: "Herbs".to_string(),
            retired: 1
        }]
    );
}

#[tokio::test]
async fn a_bank_whose_old_questions_were_retired_is_updated_with_a_six_column_file() {
    let a = app();
    let c = admin(&a).await;
    // a bank of the old layout, as an earlier version imported it
    let wb = xexam_backend::parser::parse_workbook(&three_columns()).expect("parse");
    let n_old = i64::try_from(wb.questions.len()).expect("n");
    let (_, bank) = a
        .store
        .apply_import_for(
            ImportTarget::New,
            "Old bank.xlsx",
            &wb.sheets,
            &wb.questions,
        )
        .expect("import")
        .expect("bank");
    let retired = a.store.retire_old_layout_questions().expect("retire");
    assert_eq!(retired[0].retired, n_old);
    assert_eq!(
        get(&a, "/api/banks", &c).await.json()["banks"][0]["total"],
        0,
        "the bank stays, without questions in use"
    );
    // the update: the new questions are added (their ids follow the English text), the old ones stay retired
    let r = upload(
        &a,
        &c,
        &bank.to_string(),
        "Old bank (six columns).xlsx",
        &six_columns(4, "new"),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let p = r.json();
    assert_eq!(
        (
            p["added"].clone(),
            p["removed"].clone(),
            p["restored"].clone()
        ),
        (json!(4), json!(0), json!(0))
    );
    let token = p["token"].as_str().expect("token").to_string();
    let r = post(&a, "/api/import/confirm", &c, json!({ "token": token })).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let b = &get(&a, "/api/banks", &c).await.json()["banks"][0];
    assert_eq!(
        (b["id"].clone(), b["total"].clone(), b["inactive"].clone()),
        (json!(bank), json!(4), json!(n_old))
    );
    // nothing is left to retire
    assert_eq!(a.store.retire_old_layout_questions().expect("again"), []);
}

#[test]
fn the_command_line_start_retires_the_old_layouts_and_logs_one_english_line_per_bank() {
    let dir = dbc::tmp();
    let data = dir.path().to_str().expect("utf-8 path").to_string();
    write_v3_database(&dir.path().join("xexam.db"));
    let out = cli(&["migrate-progress", "--data", &data]);
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        stderr.contains("retired 2 questions of bank \"Herbs\": they use the old 3- / 4-column layout; import the six-column version of the workbook"),
        "{stderr}"
    );
    assert!(
        stderr.contains("retired 1 question of bank \"Spices\": it uses the old 3- / 4-column layout; import the six-column version of the workbook"),
        "{stderr}"
    );
    assert!(
        stdout.contains("Not migrated: there is no old progress.json"),
        "{stdout}"
    );
    let c = raw(&dir.path().join("xexam.db"));
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM questions WHERE active=1"),
        1
    );
    // the next start retires nothing more
    let out = cli(&["migrate-progress", "--data", &data]);
    assert!(!String::from_utf8_lossy(&out.stderr).contains("retired"));
}
