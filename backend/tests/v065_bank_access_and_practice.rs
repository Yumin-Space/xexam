//! v0.6.5: question banks per user, bank managers and bank owners, practice records — schema v4 (migration from v3 with its
//! backup, fresh databases, fix-ups on open), which banks a plain user sees on every route that reads bank data, the bank
//! management permission and ownership on every bank route, and the `kind` of a study record.
//!
//! Like tests/v06_users_and_sign_in.rs, these tests use the real router with the real engine and database and sign in through
//! the API. Only made-up data is used.
#[path = "api_common/mod.rs"]
mod apic;
#[path = "common/mod.rs"]
mod dbc;

use apic::{build, send, Resp};
use axum::body::Body;
use axum::Router;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicI64;
use std::sync::Arc;
use xexam_backend::api::auth::{Access, ROUTES};
use xexam_backend::api::{build_router, AppState, Inner};
use xexam_backend::auth::{self, LoginThrottle, ThrottleLimits};
use xexam_backend::database::schema::{
    MIGRATE_V2_INDEXES, MIGRATE_V2_TABLES, MIGRATE_V3_INDEXES, MIGRATE_V3_TABLES, SCHEMA,
    V2_COLUMNS, V3_COLUMNS,
};
use xexam_backend::database::{scoped_id, Store, UserChange};
use xexam_backend::error::DbError;
use xexam_backend::models::{
    Attempt, AttemptKind, AttemptResult, BankAccess, BankScope, ImportTarget, Viewer,
};
use xexam_backend::services::{PendingStore, RealEngine};

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";

const MSG_ADMIN_ONLY: &str = "只有管理员可以进行这个操作。";
const MSG_BAD_REQUEST: &str = "请求的格式不对。";
const MSG_BANK_NOT_FOUND: &str = "这个题库不存在（可能已被删除），请刷新页面后再试。";
const MSG_PENDING_GONE: &str = "这次上传的预览已经过期或已被处理，请重新上传。";
const MSG_EXAM_QUESTIONS_GONE: &str = "这场考试的题目已经被删除，成绩没有保存。";
const MSG_BAD_RESULTS: &str = "成绩的格式不对";
const MSG_NO_BANK_PERMISSION: &str = "你没有管理题库的权限，请联系管理员。";
const MSG_NOT_BANK_OWNER: &str = "这个题库不是你上传的，只能更新或删除你自己上传的题库。";
const MSG_UNKNOWN_BANKS: &str = "选择的题库中有不存在的（可能已被删除），请刷新页面后再试。";

const ADMIN_PW: &str = "pantry-admin-1";

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
    let state = AppState(Arc::new(Inner {
        cfg: apic::default_config(),
        store: store.clone(),
        pending: PendingStore::new(),
        parse_gate: Arc::new(tokio::sync::Semaphore::new(2)),
        shutdown: None,
        engine: Arc::new(RealEngine::new(store.clone())),
        hash_gate: Arc::new(tokio::sync::Semaphore::new(auth::HASH_CONCURRENCY)),
        throttle: LoginThrottle::new(ThrottleLimits::default()),
        last_session_purge: AtomicI64::new(0),
        default_admin_hashes: Default::default(),
        logs: Default::default(),
        rejections: Default::default(),
        failed_sign_ins: Default::default(),
        name_locks: Default::default(),
        // TEST-ONLY: these tests import the committed 3-column demo fixture (tests/baseline/fixtures/demo.xlsx) through the
        // API; production code always refuses the old layouts (tests/v07_bank_access_and_layouts.rs checks that)
        old_layouts: xexam_backend::services::OldLayouts::AllowForTests,
        ai_link: Default::default(),
    }));
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

/// `POST /api/upload` of `data`; `target`: the `X-Bank-Target` header (`None`: no header, the legacy bank).
async fn upload(a: &App, cookie: &str, target: Option<&str>, name: &str, data: &[u8]) -> Resp {
    let len = data.len().to_string();
    let mut hs: Vec<(&str, &[u8])> = vec![
        ("cookie", cookie.as_bytes()),
        ("origin", ORIGIN.as_bytes()),
        ("content-type", b"application/octet-stream"),
        ("x-filename", name.as_bytes()),
        ("content-length", len.as_bytes()),
    ];
    if let Some(t) = target {
        hs.push(("x-bank-target", t.as_bytes()));
    }
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

fn demo_xlsx() -> Vec<u8> {
    std::fs::read(dbc::baseline("fixtures/demo.xlsx")).expect("demo.xlsx")
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

/// Signs in as the default administrator and sets a real password. Returns the cookie.
async fn admin(a: &App) -> String {
    let c = cookie_of(&login(a, "admin", "admin").await);
    let r = post(a, "/api/auth/password", &c, json!({ "password": ADMIN_PW })).await;
    assert_eq!(r.status, 200, "{}", r.text());
    cookie_of(&r)
}

/// Creates a user with the extra fields of `extra` (merged into the create body), signs in as them and sets their own
/// password. Returns (id, cookie).
async fn user(a: &App, admin_cookie: &str, name: &str, extra: Value) -> (i64, String) {
    let mut body = json!({"username": name, "password": "initial-pass", "role": "user"});
    if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    let r = post(a, "/api/users/create", admin_cookie, body).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let id = user_row(&r.json(), name)["id"].as_i64().expect("id");
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

/// The row of `name` in a `{"users": [...]}` answer.
fn user_row(users: &Value, name: &str) -> Value {
    users["users"]
        .as_array()
        .expect("users")
        .iter()
        .find(|u| u["username"] == name)
        .cloned()
        .expect("user in the list")
}

/// Imports a made-up bank of `n` single-choice questions (sheet `sheet`) as a new bank, as `import` would (owner: the first
/// administrator). Returns (bank id, question ids).
fn add_bank(store: &Store, file: &str, sheet: &str, n: usize) -> (i64, Vec<String>) {
    let parser_ids: Vec<String> = (0..n).map(|i| format!("{sheet}-{i}")).collect();
    let qs: Vec<_> = parser_ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            dbc::single(
                sheet,
                id,
                &format!("{sheet} made-up question {i}?"),
                i as i64 + 1,
            )
        })
        .collect();
    let (_, bank) = store
        .apply_import_for(ImportTarget::New, file, &[dbc::sheet(sheet, n as i64)], &qs)
        .expect("import")
        .expect("bank");
    let ids = parser_ids.iter().map(|p| scoped_id(bank, p)).collect();
    (bank, ids)
}

fn attempt_body(results: &[(&str, bool)], kind: Option<&str>) -> Value {
    let results: Vec<Value> = results
        .iter()
        .map(|(id, c)| json!({"id": id, "correct": c, "answer": ["A"]}))
        .collect();
    let mut body = json!({"results": results, "sheets": ["Herbs"], "mode": "all", "seconds": 30});
    if let Some(k) = kind {
        body["kind"] = json!(k);
    }
    body
}

/// The bank ids in a `/api/banks` answer.
fn bank_ids(banks: &Value) -> Vec<i64> {
    banks["banks"]
        .as_array()
        .expect("banks")
        .iter()
        .filter_map(|b| b["id"].as_i64())
        .collect()
}

/// The question ids in a `/api/bank` answer.
fn question_ids(bank: &Value) -> Vec<String> {
    bank["questions"]
        .as_array()
        .expect("questions")
        .iter()
        .filter_map(|q| q["id"].as_str().map(str::to_string))
        .collect()
}

fn keys(v: &Value) -> Vec<String> {
    v.as_object().expect("object").keys().cloned().collect()
}

fn backups(dir: &Path) -> Vec<PathBuf> {
    match std::fs::read_dir(dir.join("backups")) {
        Ok(rd) => rd.map(|e| e.expect("entry").path()).collect(),
        Err(_) => Vec::new(),
    }
}

/// A schema-3 database (as v0.6 wrote it) with made-up data: two administrators (the second one disabled) and a plain user,
/// two banks, questions, statistics and one exam record per user.
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
    let hash = auth::hash_password("pantry-pass-1").expect("hash");
    c.execute(
        "INSERT INTO users(id, username, password_hash, role, disabled, created_at, password_changed_at, must_change_password) VALUES \
         (1, 'off', ?1, 'admin', 1, '2026-01-01 10:00', '2026-01-01 10:00', 0), \
         (2, 'boss', ?1, 'admin', 0, '2026-01-01 10:00', '2026-01-01 10:00', 0), \
         (3, 'cook', ?1, 'user', 0, '2026-01-01 10:00', '2026-01-01 10:00', 0)",
        [hash],
    )
    .expect("users");
    c.execute_batch(
        r#"
        INSERT INTO banks(id, name, filename, legacy_ids, created_at, updated_at) VALUES
          (1, 'Herbs', 'Herbs.xlsx', 1, '2026-01-01 10:00', '2026-01-01 10:00'),
          (2, 'Spices', 'Spices.xlsx', 0, '2026-01-02 10:00', '2026-01-02 10:00');
        INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id) VALUES
          ('h1', 'Herbs', 1, 0, '{"kind": "single", "stem": "Basil or mint?"}', 1, '2026-01-01 10:00', '2026-01-01 10:00', 1),
          ('s1', 'Spices', 1, 0, '{"kind": "single", "stem": "Pepper or salt?"}', 1, '2026-01-02 10:00', '2026-01-02 10:00', 2);
        INSERT INTO user_stats(user_id, question_id, attempts, correct, wrong, streak, last, at) VALUES
          (2, 'h1', 1, 0, 1, 0, 0, '2026-01-03 10:00'),
          (3, 's1', 2, 1, 1, 0, 0, '2026-01-03 10:00');
        INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, banks, user_id) VALUES
          ('2026-01-03 10:00', '["Herbs"]', 'all', 1, 0, 0, 20, '[{"id": 1, "name": "Herbs"}]', 2),
          ('2026-01-03 11:00', '["Spices"]', 'all', 1, 1, 100, 25, '[{"id": 2, "name": "Spices"}]', 3);
        INSERT INTO attempt_items(attempt_id, seq, question_id, answer, correct) VALUES (1, 0, 'h1', '["A"]', 0), (2, 0, 's1', '["B"]', 1);
        INSERT INTO meta(key, value) VALUES('schema', '3');
        "#,
    )
    .expect("v3 data");
}

// ---------------------------------------------------------------- schema v4

#[test]
fn v3_database_migrates_to_v4_with_a_backup_everyone_keeping_every_bank() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    write_v3_database(&db);
    let s = Store::open(&db).expect("open");
    assert!(!s.created_default_admin(), "there are users already");
    // the backup: xexam-schema3-<stamp>.db, still schema 3, without the v4 additions
    let backup = s.migration_backup().expect("backup").to_path_buf();
    let name = backup.file_name().and_then(|n| n.to_str()).expect("name");
    assert!(
        name.starts_with("xexam-schema3-")
            && name.len() == "xexam-schema3-20260101-000000.db".len(),
        "{name}"
    );
    assert_eq!(backups(dir.path()).len(), 1);
    {
        let b = raw(&backup);
        assert_eq!(text(&b, "SELECT value FROM meta WHERE key='schema'"), "3");
        assert_eq!(
            count(
                &b,
                "SELECT COUNT(*) FROM sqlite_master WHERE name='user_banks'"
            ),
            0
        );
        assert_eq!(count(&b, "SELECT COUNT(*) FROM attempts"), 2);
    }
    let c = raw(&db);
    assert_eq!(text(&c, "SELECT value FROM meta WHERE key='schema'"), "5");
    // every user that existed keeps seeing every bank (a row for each, 3 users x 2 banks) and gets the banks imported later
    // (v0.7: the two settings are independent); nobody gets the bank permission
    assert_eq!(count(&c, "SELECT COUNT(*) FROM users WHERE new_banks=1"), 3);
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM users WHERE can_manage_banks=1"),
        0
    );
    assert_eq!(count(&c, "SELECT COUNT(*) FROM user_banks"), 6);
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM pragma_table_info('users') WHERE name='all_banks'"
        ),
        0,
        "the pre-release column is nowhere"
    );
    // every bank belongs to the first ENABLED administrator (boss, id 2)
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM banks WHERE owner_id IS NOT 2"),
        0
    );
    // every record is an exam
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM attempts WHERE kind='exam'"),
        2
    );
    // what the API shows: the plain user still sees both banks, and their record says "exam"
    let cook = s.viewer(3).expect("viewer");
    assert_eq!(cook.scope, BankScope::Only([1, 2].into()));
    assert!(!cook.admin && !cook.can_manage);
    assert_eq!(bank_ids(&s.banks_as(&cook).expect("banks")), [1, 2]);
    assert_eq!(s.records(3).expect("records")["records"][0]["kind"], "exam");
    assert_eq!(
        s.progress(3).expect("progress")["history"][0]["kind"],
        "exam"
    );
    let banks = s.banks().expect("banks");
    assert_eq!(banks["banks"][0]["owner"], "boss");
    assert_eq!(banks["banks"][1]["ownerId"], 2);
    drop(c);
    drop(s);
    // reopening: nothing to do, the file stays byte-identical, no second backup
    let bytes = dbc::db_bytes(&db);
    let s = Store::open(&db).expect("reopen");
    assert!(s.migration_backup().is_none());
    s.banks().expect("banks");
    drop(s);
    assert_eq!(dbc::db_bytes(&db), bytes);
    assert_eq!(backups(dir.path()).len(), 1);
}

#[test]
fn a_v3_database_whose_backup_cannot_be_written_is_left_unchanged() {
    let dir = dbc::tmp();
    let db = dir.path().join("xexam.db");
    write_v3_database(&db);
    std::fs::write(dir.path().join("backups"), b"not a directory").expect("write");
    let before = dbc::db_bytes(&db);
    let err = Store::open(&db).err().expect("the open fails");
    assert!(matches!(err, DbError::Backup(_)), "{err}");
    assert_eq!(dbc::db_bytes(&db), before, "byte-identical");
    assert_eq!(
        text(&raw(&db), "SELECT value FROM meta WHERE key='schema'"),
        "3"
    );
}

#[test]
fn a_fresh_database_is_schema_4_and_its_default_administrator_sees_everything() {
    let dir = dbc::tmp();
    let db = dir.path().join("x.db");
    let s = Store::open(&db).expect("open");
    assert!(s.created_default_admin());
    assert!(s.migration_backup().is_none(), "nothing to back up");
    let c = raw(&db);
    assert_eq!(text(&c, "SELECT value FROM meta WHERE key='schema'"), "5");
    // the default administrator exists at the v4 step of a new database: it gets "banks imported later" like every user
    assert_eq!(count(&c, "SELECT new_banks FROM users WHERE id=1"), 1);
    assert_eq!(s.viewer(1).expect("viewer"), Viewer::admin(1));
    // a bank created without a signed-in user (the `import` command) belongs to the first administrator
    let (bank, _) = add_bank(&s, "Herbs.xlsx", "Herbs", 1);
    assert_eq!(
        count(&c, &format!("SELECT owner_id FROM banks WHERE id={bank}")),
        1
    );
}

#[test]
fn open_gives_ownerless_banks_to_the_first_administrator_and_drops_stale_assignments() {
    let dir = dbc::tmp();
    let db = dir.path().join("x.db");
    let s = Store::open(&db).expect("open");
    let (b1, _) = add_bank(&s, "Herbs.xlsx", "Herbs", 1);
    let (b2, _) = add_bank(&s, "Spices.xlsx", "Spices", 1);
    drop(s);
    {
        // what an older binary could leave behind after a rollback
        let c = raw(&db);
        c.execute_batch(&format!(
            "UPDATE banks SET owner_id=NULL WHERE id={b1}; UPDATE banks SET owner_id=77 WHERE id={b2}; \
             DELETE FROM user_banks; INSERT INTO user_banks(user_id, bank_id) VALUES (1, {b1}), (55, {b1}), (1, 99);"
        ))
        .expect("damage");
    }
    let s = Store::open(&db).expect("reopen");
    let c = raw(&db);
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM banks WHERE owner_id IS NOT 1"),
        0
    );
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM user_banks"),
        1,
        "only the valid row"
    );
    drop(s);
    let bytes = dbc::db_bytes(&db);
    drop(Store::open(&db).expect("third open"));
    assert_eq!(dbc::db_bytes(&db), bytes, "nothing left to fix");
}

#[test]
fn users_an_older_binary_creates_after_a_rollback_get_every_bank() {
    let dir = dbc::tmp();
    let db = dir.path().join("x.db");
    let s = Store::open(&db).expect("open");
    let (herbs, _) = add_bank(&s, "Herbs.xlsx", "Herbs", 1);
    let hash = s
        .user_by_name("admin")
        .expect("lookup")
        .expect("admin")
        .password_hash;
    // a user created by this version with no bank keeps no bank
    let mine = match s
        .create_user_with(
            "newbie",
            &hash,
            auth::Role::User,
            &BankAccess::Only(Vec::new()),
            false,
            false,
        )
        .expect("create")
    {
        UserChange::Done(id) => id,
        other => panic!("{other:?}"),
    };
    drop(s);
    {
        // what v0.6 does after a rollback: it inserts users without user_banks rows or new_banks (the default 0 applies)
        let c = raw(&db);
        c.execute_batch(
            "INSERT INTO users(username, password_hash, role, disabled, created_at, password_changed_at, must_change_password) \
             VALUES ('dave', 'x', 'user', 0, '2026-01-01 00:00:00', '2026-01-01 00:00:00', 1);",
        )
        .expect("rollback user");
    }
    let s = Store::open(&db).expect("reopen");
    let c = raw(&db);
    let dave = count(&c, "SELECT id FROM users WHERE username='dave'");
    assert_eq!(
        count(&c, "SELECT new_banks FROM users WHERE username='dave'"),
        1,
        "the rollback user gets the banks imported later, as on v0.6"
    );
    assert_eq!(
        s.viewer(dave).expect("viewer").scope,
        BankScope::Only([herbs].into()),
        "and every existing bank"
    );
    assert_eq!(
        count(&c, &format!("SELECT new_banks FROM users WHERE id={mine}")),
        0,
        "a user this version created keeps their choice"
    );
    assert_eq!(
        count(
            &c,
            &format!("SELECT COUNT(*) FROM user_banks WHERE user_id={mine}")
        ),
        0
    );
    assert_eq!(
        text(&c, "SELECT value FROM meta WHERE key='users_known'"),
        count(&c, "SELECT MAX(id) FROM users").to_string()
    );
    drop(s);
    let bytes = dbc::db_bytes(&db);
    drop(Store::open(&db).expect("third open"));
    assert_eq!(dbc::db_bytes(&db), bytes, "nothing left to fix");
    // a database first opened by a build without the mark: only the mark is written, nobody's banks change
    raw(&db)
        .execute_batch(&format!(
            "DELETE FROM meta WHERE key='users_known'; UPDATE users SET new_banks=0 WHERE username='dave'; \
             DELETE FROM user_banks WHERE user_id={dave};"
        ))
        .expect("no mark");
    drop(Store::open(&db).expect("fourth open"));
    let c = raw(&db);
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM users WHERE new_banks=1 AND role='user'"
        ),
        0
    );
    assert_eq!(
        count(
            &c,
            &format!("SELECT COUNT(*) FROM user_banks WHERE user_id={dave}")
        ),
        0
    );
    assert_eq!(
        text(&c, "SELECT value FROM meta WHERE key='users_known'"),
        count(&c, "SELECT MAX(id) FROM users").to_string()
    );
}

#[test]
fn banks_an_older_binary_creates_after_a_rollback_reach_the_users_with_banks_imported_later() {
    let dir = dbc::tmp();
    let db = dir.path().join("x.db");
    let s = Store::open(&db).expect("open");
    let (herbs, _) = add_bank(&s, "Herbs.xlsx", "Herbs", 1);
    let hash = s
        .user_by_name("admin")
        .expect("lookup")
        .expect("admin")
        .password_hash;
    let made = |name: &str, later: bool| match s
        .create_user_with(
            name,
            &hash,
            auth::Role::User,
            &BankAccess::Only(vec![herbs]),
            later,
            false,
        )
        .expect("create")
    {
        UserChange::Done(id) => id,
        other => panic!("{other:?}"),
    };
    let flagged = made("flagged", true);
    let plain = made("plain", false);
    drop(s);
    // what v0.6 does after a rollback: it inserts a bank without an owner and without user_banks rows
    raw(&db)
        .execute_batch(
            "INSERT INTO banks(name, filename, legacy_ids, created_at, updated_at) \
             VALUES ('Spices', 'Spices.xlsx', 0, '2026-01-05 10:00', '2026-01-05 10:00');",
        )
        .expect("rollback bank");
    let s = Store::open(&db).expect("reopen");
    let c = raw(&db);
    let spices = count(&c, "SELECT id FROM banks WHERE name='Spices'");
    assert_eq!(
        s.viewer(flagged).expect("viewer").scope,
        BankScope::Only([herbs, spices].into()),
        "the bank reaches the user with banks imported later"
    );
    assert_eq!(
        s.viewer(plain).expect("viewer").scope,
        BankScope::Only([herbs].into()),
        "and nobody else"
    );
    assert_eq!(
        text(&c, "SELECT value FROM meta WHERE key='banks_known'"),
        spices.to_string()
    );
    assert_eq!(
        count(&c, &format!("SELECT owner_id FROM banks WHERE id={spices}")),
        1,
        "the first administrator owns it"
    );
    drop(s);
    let bytes = dbc::db_bytes(&db);
    drop(Store::open(&db).expect("third open"));
    assert_eq!(dbc::db_bytes(&db), bytes, "nothing left to fix");
}

#[tokio::test]
async fn a_limited_users_library_stamp_says_nothing_about_banks_they_cannot_see() {
    let a = app();
    let c = admin(&a).await;
    let (b1, _) = add_bank(&a.store, "Herbs.xlsx", "Herbs", 2);
    let (_, nc) = user(&a, &c, "nobody", json!({})).await;
    let (_, sc) = user(&a, &c, "some", json!({"banks": [b1]})).await;
    let (_, mc) = user(&a, &c, "maker", json!({"canManageBanks": true})).await;
    let stamp = |v: Value| v["library"].as_str().expect("stamp").to_string();
    let read = |cookie: String| {
        let a = &a;
        async move { stamp(get(a, "/api/auth/state", &cookie).await.json()) }
    };
    let (n0, s0, m0, a0) = (
        read(nc.clone()).await,
        read(sc.clone()).await,
        read(mc.clone()).await,
        read(c.clone()).await,
    );
    // an opaque digest for everyone: no counters or times in clear text
    for st in [&n0, &s0, &m0, &a0] {
        assert!(
            st.len() == 32 && st.bytes().all(|b| b.is_ascii_hexdigit()),
            "{st}"
        );
    }
    assert!(
        n0 != s0 && s0 != a0 && n0 != m0,
        "each scope has its own stamp"
    );
    // another bank, by the administrator: invisible to the limited users, so their stamps stay
    let (b2, _) = add_bank(&a.store, "Spices.xlsx", "Spices", 3);
    assert_eq!(read(nc.clone()).await, n0);
    assert_eq!(read(sc.clone()).await, s0);
    assert_eq!(read(mc.clone()).await, m0);
    assert_ne!(read(c.clone()).await, a0);
    // the bank they see is replaced: their stamp changes (also with the very same questions: a new import)
    let qs = vec![dbc::single(
        "Herbs",
        "Herbs-0",
        "Herbs made-up question 0?",
        1,
    )];
    a.store
        .apply_import_for(
            ImportTarget::Bank(b1),
            "Herbs.xlsx",
            &[dbc::sheet("Herbs", 1)],
            &qs,
        )
        .expect("replace")
        .expect("replaced");
    let s1 = read(sc.clone()).await;
    assert_ne!(s1, s0);
    assert_eq!(read(nc.clone()).await, n0);
    // a hidden bank is deleted: still nothing for them
    let r = post(&a, "/api/banks/delete", &c, json!({"id": b2})).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(read(sc.clone()).await, s1);
    assert_eq!(read(nc).await, n0);
}

#[tokio::test]
async fn only_administrators_learn_who_owns_a_bank() {
    let a = app();
    let c = admin(&a).await;
    add_bank(&a.store, "Herbs.xlsx", "Herbs", 1);
    let (_, rc) = user(&a, &c, "reader", json!({"banks": "all"})).await;
    let banks = get(&a, "/api/banks", &rc).await.json();
    let list = banks["banks"].as_array().expect("banks");
    assert_eq!(list.len(), 1);
    assert_eq!(
        (list[0]["owner"].clone(), list[0]["ownerId"].clone()),
        (json!(null), json!(null))
    );
    let all = get(&a, "/api/banks", &c).await.json();
    assert_eq!(
        (
            all["banks"][0]["owner"].clone(),
            all["banks"][0]["ownerId"].clone()
        ),
        (json!("admin"), json!(1))
    );
}

// ---------------------------------------------------------------- the users API

#[tokio::test]
async fn users_get_banks_and_the_bank_permission_on_create_and_update() {
    let a = app();
    let c = admin(&a).await;
    let (b1, _) = add_bank(&a.store, "Herbs.xlsx", "Herbs", 2);
    let (b2, _) = add_bank(&a.store, "Spices.xlsx", "Spices", 2);
    // defaults: no bank, no permission
    let r = post(
        &a,
        "/api/users/create",
        &c,
        json!({"username": "cook", "password": "initial-pass", "role": "user"}),
    )
    .await;
    assert_eq!(r.status, 200);
    let cook = user_row(&r.json(), "cook");
    assert_eq!(
        (
            cook["banks"].clone(),
            cook["canManageBanks"].clone(),
            cook["ownedBanks"].clone()
        ),
        (json!([]), json!(false), json!([]))
    );
    assert_eq!(cook["newBanks"], false);
    let admin_row = user_row(&r.json(), "admin");
    // the default administrator of a new database has "banks imported later", so it got both banks when they were created
    assert_eq!(
        (admin_row["banks"].clone(), admin_row["newBanks"].clone()),
        (json!([b1, b2]), json!(true))
    );
    assert_eq!(admin_row["ownedBanks"], json!([b1, b2]));
    let cook_id = cook["id"].as_i64().expect("id");
    // a list (sorted, without duplicates), "all", the permission
    let r = post(
        &a,
        "/api/users/create",
        &c,
        json!({"username": "baker", "password": "initial-pass", "role": "user", "banks": [b2, b1, b2], "canManageBanks": true}),
    )
    .await;
    assert_eq!(r.status, 200);
    let baker = user_row(&r.json(), "baker");
    assert_eq!(
        (baker["banks"].clone(), baker["canManageBanks"].clone()),
        (json!([b1, b2]), json!(true))
    );
    let r = post(
        &a,
        "/api/users/create",
        &c,
        json!({"username": "chef", "password": "initial-pass", "role": "admin", "banks": "all"}),
    )
    .await;
    // "all": every existing bank, plus the banks imported later
    let chef = user_row(&r.json(), "chef");
    assert_eq!(
        (chef["banks"].clone(), chef["newBanks"].clone()),
        (json!([b1, b2]), json!(true))
    );
    // the two settings are independent: a list with newBanks, and "all" without it
    let r = post(
        &a,
        "/api/users/create",
        &c,
        json!({"username": "sous", "password": "initial-pass", "role": "user", "banks": [b1], "newBanks": true}),
    )
    .await;
    let sous = user_row(&r.json(), "sous");
    assert_eq!(
        (sous["banks"].clone(), sous["newBanks"].clone()),
        (json!([b1]), json!(true))
    );
    let r = post(
        &a,
        "/api/users/create",
        &c,
        json!({"username": "porter", "password": "initial-pass", "role": "user", "banks": "all", "newBanks": false}),
    )
    .await;
    let porter = user_row(&r.json(), "porter");
    assert_eq!(
        (porter["banks"].clone(), porter["newBanks"].clone()),
        (json!([b1, b2]), json!(false))
    );
    // validation: nothing is created
    for (body, msg) in [
        (json!({"banks": [b1, 999]}), MSG_UNKNOWN_BANKS),
        (json!({"banks": [0]}), MSG_UNKNOWN_BANKS),
        (json!({"banks": [-3]}), MSG_UNKNOWN_BANKS),
        (json!({"banks": ["1"]}), MSG_BAD_REQUEST),
        (json!({"banks": [true]}), MSG_BAD_REQUEST),
        (json!({"banks": "some"}), MSG_BAD_REQUEST),
        (json!({"banks": 1}), MSG_BAD_REQUEST),
        (json!({"banks": null}), MSG_BAD_REQUEST),
        (json!({"canManageBanks": "yes"}), MSG_BAD_REQUEST),
        (json!({"canManageBanks": 1}), MSG_BAD_REQUEST),
        (json!({"newBanks": "yes"}), MSG_BAD_REQUEST),
        (json!({"newBanks": 1}), MSG_BAD_REQUEST),
        (json!({"newBanks": null}), MSG_BAD_REQUEST),
        (json!({"banks": [b1], "newBanks": []}), MSG_BAD_REQUEST),
    ] {
        let mut full = json!({"username": "ghost", "password": "initial-pass", "role": "user"});
        for (k, v) in body.as_object().expect("o") {
            full[k] = v.clone();
        }
        let r = post(&a, "/api/users/create", &c, full).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (400, msg.to_string()),
            "{body}"
        );
    }
    assert!(a.store.user_by_name("ghost").expect("lookup").is_none());
    // update: the banks, the permission; an unknown id changes nothing (not even the other fields)
    let r = post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": cook_id, "banks": [b2], "canManageBanks": true}),
    )
    .await;
    let row = user_row(&r.json(), "cook");
    assert_eq!(
        (row["banks"].clone(), row["canManageBanks"].clone()),
        (json!([b2]), json!(true))
    );
    let r = post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": cook_id, "username": "cook2", "banks": [b1, 404]}),
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (400, MSG_UNKNOWN_BANKS.to_string())
    );
    let r = get(&a, "/api/users", &c).await;
    let row = user_row(&r.json(), "cook");
    assert_eq!(row["banks"], json!([b2]), "unchanged");
    // a role change keeps both settings; "all" and back
    let r = post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": cook_id, "role": "admin"}),
    )
    .await;
    let row = user_row(&r.json(), "cook");
    assert_eq!(
        (
            row["role"].clone(),
            row["banks"].clone(),
            row["canManageBanks"].clone()
        ),
        (json!("admin"), json!([b2]), json!(true))
    );
    let r = post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": cook_id, "role": "user", "banks": "all"}),
    )
    .await;
    let row = user_row(&r.json(), "cook");
    assert_eq!(
        (row["banks"].clone(), row["newBanks"].clone()),
        (json!([b1, b2]), json!(true))
    );
    // only what is given changes: the flag alone, then the banks alone
    let r = post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": cook_id, "newBanks": false}),
    )
    .await;
    let row = user_row(&r.json(), "cook");
    assert_eq!(
        (row["banks"].clone(), row["newBanks"].clone()),
        (json!([b1, b2]), json!(false))
    );
    let r = post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": cook_id, "newBanks": true}),
    )
    .await;
    let r2 = post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": cook_id, "banks": [b2]}),
    )
    .await;
    assert_eq!(r.status, 200);
    let row = user_row(&r2.json(), "cook");
    assert_eq!(
        (row["banks"].clone(), row["newBanks"].clone()),
        (json!([b2]), json!(true))
    );
    let r = post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": cook_id, "banks": [], "newBanks": false, "canManageBanks": false}),
    )
    .await;
    let row = user_row(&r.json(), "cook");
    assert_eq!(
        (
            row["banks"].clone(),
            row["newBanks"].clone(),
            row["canManageBanks"].clone()
        ),
        (json!([]), json!(false), json!(false))
    );
    // a wrong type changes nothing
    let r = post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": cook_id, "banks": [b1], "newBanks": "yes"}),
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (400, MSG_BAD_REQUEST.to_string())
    );
    assert_eq!(
        user_row(&get(&a, "/api/users", &c).await.json(), "cook")["banks"],
        json!([])
    );
    // only administrators
    let (_, uc) = user(
        &a,
        &c,
        "waiter",
        json!({"banks": "all", "canManageBanks": true}),
    )
    .await;
    for (path, body) in [
        (
            "/api/users/create",
            json!({"username": "x", "password": "initial-pass", "role": "user"}),
        ),
        ("/api/users/update", json!({"id": cook_id, "banks": "all"})),
    ] {
        let r = post(&a, path, &uc, body).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (403, MSG_ADMIN_ONLY.to_string()),
            "{path}"
        );
    }
}

#[tokio::test]
async fn auth_state_says_whether_the_user_may_manage_banks_and_the_library_follows_their_access() {
    let a = app();
    let c = admin(&a).await;
    let (b1, _) = add_bank(&a.store, "Herbs.xlsx", "Herbs", 1);
    add_bank(&a.store, "Spices.xlsx", "Spices", 1);
    let (id, uc) = user(&a, &c, "cook", json!({})).await;
    let state = get(&a, "/api/auth/state", &uc).await.json();
    assert_eq!(state["user"]["canManageBanks"], false);
    assert_eq!(
        get(&a, "/api/auth/state", &c).await.json()["user"]["canManageBanks"],
        true
    );
    let mut seen = vec![state["library"].as_str().expect("stamp").to_string()];
    for change in [
        json!({"id": id, "banks": [b1]}),
        json!({"id": id, "canManageBanks": true}),
        json!({"id": id, "banks": "all"}),
        json!({"id": id, "canManageBanks": false}),
    ] {
        assert_eq!(
            post(&a, "/api/users/update", &c, change.clone())
                .await
                .status,
            200
        );
        let state = get(&a, "/api/auth/state", &uc).await.json();
        let stamp = state["library"].as_str().expect("stamp").to_string();
        assert!(!seen.contains(&stamp), "{change}: the stamp must change");
        seen.push(stamp);
    }
    // "include banks imported later" alone changes nothing the user sees: the stamp stays (v0.7)
    let before = seen.last().cloned();
    let r = post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": id, "newBanks": false}),
    )
    .await;
    assert_eq!(r.status, 200);
    let state = get(&a, "/api/auth/state", &uc).await.json();
    assert_eq!(state["library"].as_str().map(str::to_string), before);
}

// ---------------------------------------------------------------- what a plain user sees

#[tokio::test]
async fn a_plain_user_sees_only_their_banks_on_every_route() {
    let a = app();
    let c = admin(&a).await;
    let (b1, herbs) = add_bank(&a.store, "Herbs.xlsx", "Herbs", 2);
    let (b2, spices) = add_bank(&a.store, "Spices.xlsx", "Spices", 2);
    let (id, uc) = user(&a, &c, "cook", json!({"banks": [b1]})).await;
    // a user without any bank sees an empty library
    let (_, nc) = user(&a, &c, "newbie", json!({})).await;
    let empty = get(&a, "/api/bank", &nc).await.json();
    assert_eq!(
        (empty["questions"].clone(), empty["sheets"].clone()),
        (json!([]), json!([]))
    );
    assert_eq!(
        get(&a, "/api/banks", &nc).await.json(),
        json!({"banks": []})
    );

    // /api/bank and /api/banks: only Herbs
    let bank = get(&a, "/api/bank", &uc).await.json();
    assert_eq!(question_ids(&bank), herbs);
    assert_eq!(bank["sheets"].as_array().map(Vec::len), Some(1));
    assert_eq!(bank["sheets"][0]["name"], "Herbs");
    assert_eq!(
        bank["file"], "Herbs.xlsx",
        "the latest import of a visible bank"
    );
    let banks = get(&a, "/api/banks", &uc).await.json();
    assert_eq!(bank_ids(&banks), [b1]);
    assert_eq!(banks["banks"][0]["manageable"], false);
    // (v0.7) a plain user is not told who owns a bank: the user list is the administrators' alone
    assert_eq!(
        (
            banks["banks"][0]["owner"].clone(),
            banks["banks"][0]["ownerId"].clone()
        ),
        (json!(null), json!(null))
    );
    let admin_view = get(&a, "/api/banks", &c).await.json();
    assert_eq!(admin_view["banks"][0]["owner"], "admin");
    // the administrator sees both
    assert_eq!(bank_ids(&get(&a, "/api/banks", &c).await.json()), [b1, b2]);

    // /api/attempt: a question of a hidden bank counts like a deleted one
    let r = post(
        &a,
        "/api/attempt",
        &uc,
        attempt_body(&[(&herbs[0], false), (&spices[0], false)], None),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let p = r.json();
    assert_eq!(p["notCounted"], json!([spices[0]]));
    assert!(p["questions"].get(&spices[0]).is_none());
    assert_eq!(p["history"][0]["total"], 1);
    let c2 = raw(&a.db);
    assert_eq!(
        count(
            &c2,
            &format!("SELECT COUNT(*) FROM user_stats WHERE user_id={id}")
        ),
        1,
        "no statistics for the hidden question"
    );
    // only hidden questions: nothing is saved
    let r = post(
        &a,
        "/api/attempt",
        &uc,
        attempt_body(&[(&spices[1], true)], None),
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (409, MSG_EXAM_QUESTIONS_GONE.to_string())
    );

    // the user sees Spices for a while and studies it, then loses it
    post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": id, "banks": [b1, b2]}),
    )
    .await;
    let r = post(
        &a,
        "/api/attempt",
        &uc,
        attempt_body(&[(&herbs[1], true), (&spices[1], false)], Some("practice")),
    )
    .await;
    assert_eq!(r.status, 200);
    assert!(r.json().get("notCounted").is_none());
    post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": id, "banks": [b1]}),
    )
    .await;
    // /api/progress: the notebook counts only visible questions; the history stays complete
    let p = get(&a, "/api/progress", &uc).await.json();
    let mut qs: Vec<String> = p["questions"]
        .as_object()
        .expect("q")
        .keys()
        .cloned()
        .collect();
    qs.sort();
    let mut want = herbs.clone();
    want.sort();
    assert_eq!(qs, want);
    assert_eq!(p["history"].as_array().map(Vec::len), Some(2));
    assert_eq!(p["history"][0]["kind"], "practice");
    // the statistics are kept: they come back with the bank
    assert_eq!(
        count(
            &c2,
            &format!("SELECT COUNT(*) FROM user_stats WHERE user_id={id}")
        ),
        3
    );
    // /api/records keeps both records; /api/record leaves the hidden question's text out (like a deleted one)
    let records = get(&a, "/api/records", &uc).await.json();
    assert_eq!(records["records"].as_array().map(Vec::len), Some(2));
    let rid = records["records"][0]["id"].as_i64().expect("id");
    let detail = get(&a, &format!("/api/record?id={rid}"), &uc).await.json();
    assert_eq!(detail["record"]["kind"], "practice");
    let items = detail["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["question"]["id"], json!(herbs[1]));
    assert_eq!(items[1]["questionId"], json!(spices[1]));
    assert_eq!(items[1]["question"], Value::Null);
    assert_eq!(items[1]["correct"], false, "the answer stays");
    assert!(
        !detail.to_string().contains("Spices made-up question"),
        "no hidden question text"
    );
    // "all" again: everything is back
    post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": id, "banks": "all"}),
    )
    .await;
    let detail = get(&a, &format!("/api/record?id={rid}"), &uc).await.json();
    assert_eq!(detail["items"][1]["question"]["id"], json!(spices[1]));
    assert_eq!(
        get(&a, "/api/progress", &uc).await.json()["questions"]
            .as_object()
            .map(|m| m.len()),
        Some(3)
    );
    assert_eq!(
        question_ids(&get(&a, "/api/bank", &uc).await.json()).len(),
        4
    );
}

#[tokio::test]
async fn deleting_a_bank_or_a_user_removes_their_assignments_and_hands_banks_over() {
    let a = app();
    let c = admin(&a).await;
    let (b1, _) = add_bank(&a.store, "Herbs.xlsx", "Herbs", 1);
    let (b2, _) = add_bank(&a.store, "Spices.xlsx", "Spices", 1);
    let (cook, _) = user(&a, &c, "cook", json!({"banks": [b1, b2]})).await;
    let (baker, bc) = user(
        &a,
        &c,
        "baker",
        json!({"banks": [b1], "canManageBanks": true}),
    )
    .await;
    // baker imports a bank of their own
    let r = upload(&a, &bc, Some("new"), "Baking.xlsx", &demo_xlsx()).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let token = r.json()["token"].as_str().expect("token").to_string();
    let r = post(&a, "/api/import/confirm", &bc, json!({ "token": token })).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let b3 = r.json()["bankId"].as_i64().expect("bank id");
    post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": cook, "banks": [b1, b2, b3]}),
    )
    .await;
    let c2 = raw(&a.db);
    // (the default administrator has "banks imported later" and got all three banks too)
    assert_eq!(
        count(&c2, "SELECT COUNT(*) FROM user_banks WHERE user_id <> 1"),
        4
    );
    // a deleted bank leaves no assignment behind
    let r = post(&a, "/api/banks/delete", &c, json!({"id": b2})).await;
    assert_eq!(r.status, 200);
    assert_eq!(
        count(
            &c2,
            &format!("SELECT COUNT(*) FROM user_banks WHERE bank_id={b2}")
        ),
        0
    );
    assert_eq!(
        user_row(&get(&a, "/api/users", &c).await.json(), "cook")["banks"],
        json!([b1, b3])
    );
    // a deleted user: their assignments go, their banks move to the administrator who deleted them
    let r = post(&a, "/api/users/delete", &c, json!({"id": baker})).await;
    assert_eq!(r.status, 200);
    assert_eq!(
        count(
            &c2,
            &format!("SELECT COUNT(*) FROM user_banks WHERE user_id={baker}")
        ),
        0
    );
    assert_eq!(
        count(&c2, &format!("SELECT owner_id FROM banks WHERE id={b3}")),
        1
    );
    assert_eq!(user_row(&r.json(), "admin")["ownedBanks"], json!([b1, b3]));
    assert_eq!(
        user_row(&r.json(), "cook")["banks"],
        json!([b1, b3]),
        "other users keep their banks"
    );
}

// ---------------------------------------------------------------- bank managers and owners

#[test]
fn the_bank_manager_routes_are_exactly_the_bank_management_routes() {
    let mut routes: Vec<(&str, &str)> = ROUTES
        .iter()
        .filter(|r| r.access == Access::BankManager)
        .map(|r| (r.method, r.path))
        .collect();
    routes.sort();
    assert_eq!(
        routes,
        [
            ("GET", "/api/imports"),
            ("POST", "/api/banks/delete"),
            ("POST", "/api/import/cancel"),
            ("POST", "/api/import/confirm"),
            ("POST", "/api/upload"),
        ]
    );
}

#[tokio::test]
async fn a_user_without_the_permission_gets_403_on_every_bank_management_route() {
    let a = app();
    let c = admin(&a).await;
    let (b1, _) = add_bank(&a.store, "Herbs.xlsx", "Herbs", 1);
    let (id, uc) = user(&a, &c, "cook", json!({"banks": "all"})).await;
    let check = |r: Resp, what: &str| {
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (403, MSG_NO_BANK_PERMISSION.to_string()),
            "{what}"
        );
    };
    check(get(&a, "/api/imports", &uc).await, "imports");
    check(
        upload(&a, &uc, Some("new"), "x.xlsx", &demo_xlsx()).await,
        "upload new",
    );
    check(
        upload(&a, &uc, Some(&b1.to_string()), "x.xlsx", &demo_xlsx()).await,
        "upload into a bank",
    );
    check(
        post(&a, "/api/import/confirm", &uc, json!({"token": "x"})).await,
        "confirm",
    );
    check(
        post(&a, "/api/import/cancel", &uc, json!({})).await,
        "cancel",
    );
    check(
        post(&a, "/api/banks/delete", &uc, json!({"id": b1})).await,
        "delete",
    );
    assert_eq!(
        bank_ids(&a.store.banks().expect("banks")),
        [b1],
        "nothing deleted"
    );
    // given the permission, the same session may (read from the database on every request)
    post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": id, "canManageBanks": true}),
    )
    .await;
    assert_eq!(get(&a, "/api/imports", &uc).await.status, 200);
}

#[tokio::test]
async fn a_bank_manager_imports_updates_and_deletes_only_their_own_banks() {
    let a = app();
    let c = admin(&a).await;
    let (b1, herbs) = add_bank(&a.store, "Herbs.xlsx", "Herbs", 2);
    let (alice, ac) = user(&a, &c, "alice", json!({"canManageBanks": true})).await;
    let (_, bc) = user(&a, &c, "bob", json!({"canManageBanks": true})).await;

    // a new bank: owned by alice and visible to her at once (she was given no bank)
    let r = upload(&a, &ac, Some("new"), "Baking.xlsx", &demo_xlsx()).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let token = r.json()["token"].as_str().expect("token").to_string();
    let r = post(&a, "/api/import/confirm", &ac, json!({ "token": token })).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let mine = r.json()["bankId"].as_i64().expect("bank id");
    let own_questions = question_ids(&r.json()["bank"]);
    assert!(
        !own_questions.is_empty(),
        "the confirm answer shows her new bank"
    );
    assert!(
        own_questions.iter().all(|q| !herbs.contains(q)),
        "and not the administrator's"
    );
    let banks = get(&a, "/api/banks", &ac).await.json();
    assert_eq!(bank_ids(&banks), [mine]);
    assert_eq!(
        (
            banks["banks"][0]["owner"].clone(),
            banks["banks"][0]["ownerId"].clone(),
            banks["banks"][0]["manageable"].clone()
        ),
        // (v0.7) her own id says it is hers; usernames go to administrators only
        (json!(null), json!(alice), json!(true))
    );
    // the administrator sees who uploaded it and may manage it too
    let all = get(&a, "/api/banks", &c).await.json();
    assert_eq!(bank_ids(&all), [b1, mine]);
    assert_eq!(all["banks"][1]["owner"], "alice");
    assert_eq!(all["banks"][1]["manageable"], true);
    // bob sees nothing of it
    assert_eq!(
        get(&a, "/api/banks", &bc).await.json(),
        json!({"banks": []})
    );

    // updating: her own bank yes; the administrator's bank, another user's bank or the legacy bank no (before any parse)
    let r = upload(
        &a,
        &ac,
        Some(&mine.to_string()),
        "Baking v2.xlsx",
        &demo_xlsx(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let token = r.json()["token"].as_str().expect("token").to_string();
    for (who, target) in [(&ac, b1.to_string()), (&bc, mine.to_string())] {
        let r = upload(&a, who, Some(&target), "x.xlsx", &demo_xlsx()).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (403, MSG_NOT_BANK_OWNER.to_string()),
            "{target}"
        );
    }
    // an unknown bank: 404, as for an administrator
    let r = upload(&a, &bc, Some("999"), "x.xlsx", &demo_xlsx()).await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (404, MSG_BANK_NOT_FOUND.to_string())
    );
    // bob's upload and cancel do not touch alice's staged upload
    let r = upload(&a, &bc, Some("new"), "Bread.xlsx", &demo_xlsx()).await;
    assert_eq!(r.status, 200);
    let bob_token = r.json()["token"].as_str().expect("token").to_string();
    assert_eq!(
        post(&a, "/api/import/cancel", &bc, json!({})).await.status,
        200
    );
    // nobody can confirm another user's upload, even with its token
    let r = post(&a, "/api/import/confirm", &c, json!({ "token": token })).await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (409, MSG_PENDING_GONE.to_string())
    );
    let r = post(
        &a,
        "/api/import/confirm",
        &bc,
        json!({ "token": bob_token }),
    )
    .await;
    assert_eq!(r.status, 409, "cancelled");
    let r = post(&a, "/api/import/confirm", &ac, json!({ "token": token })).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["bankId"], mine);

    // the import history: alice sees her own bank's imports only; the administrator sees everything
    let mine_hist = get(&a, "/api/imports", &ac).await.json();
    let files: Vec<&str> = mine_hist["imports"]
        .as_array()
        .expect("imports")
        .iter()
        .filter_map(|i| i["filename"].as_str())
        .collect();
    assert_eq!(files, ["Baking v2.xlsx", "Baking.xlsx"]);
    assert_eq!(
        get(&a, "/api/imports", &c).await.json()["imports"]
            .as_array()
            .map(Vec::len),
        Some(3)
    );
    assert_eq!(
        get(&a, "/api/imports", &bc).await.json(),
        json!({"imports": []})
    );

    // deleting: not the administrator's bank, not by bob
    let r = post(&a, "/api/banks/delete", &ac, json!({"id": b1})).await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (403, MSG_NOT_BANK_OWNER.to_string())
    );
    let r = post(&a, "/api/banks/delete", &bc, json!({"id": mine})).await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (403, MSG_NOT_BANK_OWNER.to_string())
    );
    let r = post(&a, "/api/banks/delete", &ac, json!({"id": 999})).await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (404, MSG_BANK_NOT_FOUND.to_string())
    );
    assert_eq!(bank_ids(&a.store.banks().expect("banks")), [b1, mine]);
    // without the permission, her own bank is out of reach too (and out of her exams)
    post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": alice, "canManageBanks": false}),
    )
    .await;
    let r = post(&a, "/api/banks/delete", &ac, json!({"id": mine})).await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (403, MSG_NO_BANK_PERMISSION.to_string())
    );
    assert_eq!(
        get(&a, "/api/banks", &ac).await.json(),
        json!({"banks": []})
    );
    post(
        &a,
        "/api/users/update",
        &c,
        json!({"id": alice, "canManageBanks": true}),
    )
    .await;
    let r = post(&a, "/api/banks/delete", &ac, json!({"id": mine})).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["banks"], json!([]));
    assert_eq!(bank_ids(&a.store.banks().expect("banks")), [b1]);
}

#[tokio::test]
async fn a_bank_manager_deleting_their_bank_with_records_deletes_only_their_own_records() {
    let a = app();
    let c = admin(&a).await;
    let (alice, ac) = user(&a, &c, "alice", json!({"canManageBanks": true})).await;
    let (cook, uc) = user(&a, &c, "cook", json!({"banks": "all"})).await;
    let r = upload(&a, &ac, Some("new"), "Baking.xlsx", &demo_xlsx()).await;
    let token = r.json()["token"].as_str().expect("token").to_string();
    let r = post(&a, "/api/import/confirm", &ac, json!({ "token": token })).await;
    let mine = r.json()["bankId"].as_i64().expect("bank id");
    let qs = question_ids(&r.json()["bank"]);
    // alice, cook and the administrator study it
    for cookie in [&ac, &uc, &c] {
        let r = post(
            &a,
            "/api/attempt",
            cookie,
            attempt_body(&[(&qs[0], false), (&qs[1], true)], None),
        )
        .await;
        assert_eq!(r.status, 200, "{}", r.text());
    }
    let banks = get(&a, "/api/banks", &ac).await.json();
    assert_eq!(
        (
            banks["banks"][0]["records"].clone(),
            banks["banks"][0]["studied"].clone()
        ),
        (json!(1), json!(2)),
        "her own numbers"
    );
    let r = post(
        &a,
        "/api/banks/delete",
        &ac,
        json!({"id": mine, "withRecords": true}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["progress"]["questions"], json!({}));
    assert_eq!(r.json()["progress"]["history"], json!([]));
    let c2 = raw(&a.db);
    for (who, records, stats) in [(alice, 0, 0), (cook, 1, 2), (1, 1, 2)] {
        assert_eq!(
            count(
                &c2,
                &format!("SELECT COUNT(*) FROM attempts WHERE user_id={who}")
            ),
            records,
            "{who}"
        );
        assert_eq!(
            count(
                &c2,
                &format!("SELECT COUNT(*) FROM user_stats WHERE user_id={who}")
            ),
            stats,
            "{who}"
        );
    }
    // cook's record stays, its questions show as deleted
    let records = get(&a, "/api/records", &uc).await.json();
    let rid = records["records"][0]["id"].as_i64().expect("id");
    assert_eq!(records["records"][0]["banks"][0]["deleted"], true);
    let detail = get(&a, &format!("/api/record?id={rid}"), &uc).await.json();
    assert_eq!(detail["items"][0]["question"], Value::Null);
    assert_eq!(detail["items"].as_array().map(Vec::len), Some(2));
}

#[test]
fn ownership_is_checked_inside_the_write() {
    let dir = dbc::tmp();
    let s = Store::open(&dir.path().join("x.db")).expect("open");
    let (b1, ids) = add_bank(&s, "Herbs.xlsx", "Herbs", 2);
    let hash = auth::hash_password("pantry-pass-1").expect("hash");
    let UserChange::Done(alice) = s
        .create_user_with(
            "alice",
            &hash,
            auth::Role::User,
            &BankAccess::All,
            true,
            true,
        )
        .expect("create")
    else {
        panic!("created");
    };
    let v = s.viewer(alice).expect("viewer");
    assert!(v.can_manage && !v.admin && v.scope == BankScope::Only([b1].into()));
    let sh = [dbc::sheet("Herbs", 1)];
    let q = [dbc::single(
        "Herbs",
        "Herbs-0",
        "Changed made-up question?",
        1,
    )];
    assert!(matches!(
        s.apply_import_as(&v, ImportTarget::Bank(b1), "x.xlsx", &sh, &q),
        Err(DbError::NotBankOwner)
    ));
    assert!(matches!(
        s.delete_bank_as(&v, b1, true),
        Err(DbError::NotBankOwner)
    ));
    assert!(matches!(
        s.check_import_target(&v, ImportTarget::Bank(b1)),
        Err(DbError::NotBankOwner)
    ));
    assert!(!s
        .check_import_target(&v, ImportTarget::Bank(99))
        .expect("check"));
    assert!(s.check_import_target(&v, ImportTarget::New).expect("check"));
    // nothing was written: same questions, same import history
    assert_eq!(question_ids(&s.bank().expect("bank")), ids);
    assert_eq!(s.import_history(10).expect("history").len(), 1);
    // her own new bank she may update
    let (_, own) = s
        .apply_import_as(&v, ImportTarget::New, "Mine.xlsx", &sh, &q)
        .expect("new")
        .expect("bank");
    assert!(s
        .apply_import_as(&v, ImportTarget::Bank(own), "Mine.xlsx", &sh, &q)
        .expect("update")
        .is_some());
    // a legacy target while there is no legacy bank creates one, owned by her
    let (_, legacy) = s
        .apply_import_as(&v, ImportTarget::Legacy, "Old.xlsx", &sh, &q)
        .expect("legacy")
        .expect("bank");
    assert_eq!(s.banks().expect("banks")["banks"][2]["ownerId"], alice);
    assert_eq!(s.banks().expect("banks")["banks"][2]["id"], legacy);
    assert_eq!(s.banks().expect("banks")["banks"][2]["legacy"], true);
    // ...which then nobody but her (and administrators) may update through the legacy target
    let UserChange::Done(bob) = s
        .create_user_with(
            "bob",
            &hash,
            auth::Role::User,
            &BankAccess::Only(vec![]),
            false,
            true,
        )
        .expect("create")
    else {
        panic!("created");
    };
    let bob = s.viewer(bob).expect("viewer");
    assert!(matches!(
        s.check_import_target(&bob, ImportTarget::Legacy),
        Err(DbError::NotBankOwner)
    ));
    assert!(s
        .check_import_target(&Viewer::admin(1), ImportTarget::Legacy)
        .expect("admin"));
}

// ---------------------------------------------------------------- practice

#[tokio::test]
async fn practice_is_recorded_with_its_kind_and_feeds_the_notebook_like_an_exam() {
    let a = app();
    let c = admin(&a).await;
    let (_, herbs) = add_bank(&a.store, "Herbs.xlsx", "Herbs", 3);
    let (_, uc) = user(&a, &c, "cook", json!({"banks": "all"})).await;
    // an exam by default
    let r = post(
        &a,
        "/api/attempt",
        &uc,
        attempt_body(&[(&herbs[0], true)], None),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["history"][0]["kind"], "exam");
    // a practice session: a wrong answer enters the notebook, a right one counts toward leaving it
    let r = post(
        &a,
        "/api/attempt",
        &uc,
        attempt_body(&[(&herbs[1], false), (&herbs[0], true)], Some("practice")),
    )
    .await;
    assert_eq!(r.status, 200);
    let p = r.json();
    assert_eq!(p["history"][0]["kind"], "practice");
    assert_eq!(p["history"][1]["kind"], "exam");
    assert_eq!(
        (
            p["questions"][&herbs[1]]["wrong"].clone(),
            p["questions"][&herbs[1]]["last"].clone()
        ),
        (json!(1), json!(false))
    );
    assert_eq!(
        (
            p["questions"][&herbs[0]]["streak"].clone(),
            p["questions"][&herbs[0]]["attempts"].clone()
        ),
        (json!(2), json!(2))
    );
    // an explicit exam, then the records list and the detail say which is which
    post(
        &a,
        "/api/attempt",
        &uc,
        attempt_body(&[(&herbs[2], true)], Some("exam")),
    )
    .await;
    let records = get(&a, "/api/records", &uc).await.json();
    let kinds: Vec<&str> = records["records"]
        .as_array()
        .expect("records")
        .iter()
        .filter_map(|r| r["kind"].as_str())
        .collect();
    assert_eq!(kinds, ["exam", "practice", "exam"]);
    assert_eq!(
        keys(&records["records"][1]).last().map(String::as_str),
        Some("kind")
    );
    let rid = records["records"][1]["id"].as_i64().expect("id");
    let detail = get(&a, &format!("/api/record?id={rid}"), &uc).await.json();
    assert_eq!(detail["record"]["kind"], "practice");
    assert_eq!(detail["record"]["total"], 2);
    let history = get(&a, "/api/progress", &uc).await.json()["history"].clone();
    assert_eq!(
        history
            .as_array()
            .map(|h| h.iter().map(|x| x["kind"].clone()).collect::<Vec<_>>()),
        Some(vec![json!("exam"), json!("practice"), json!("exam")])
    );
    // anything else is refused, nothing is saved
    for kind in [
        json!("Practice"),
        json!("quiz"),
        json!(1),
        json!(true),
        json!(["practice"]),
    ] {
        let mut body = attempt_body(&[(&herbs[0], true)], None);
        body["kind"] = kind.clone();
        let r = post(&a, "/api/attempt", &uc, body).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (400, MSG_BAD_RESULTS.to_string()),
            "{kind}"
        );
    }
    assert_eq!(
        get(&a, "/api/records", &uc).await.json()["records"]
            .as_array()
            .map(Vec::len),
        Some(3)
    );
    // the stored value
    let c2 = raw(&a.db);
    assert_eq!(
        count(&c2, "SELECT COUNT(*) FROM attempts WHERE kind='practice'"),
        1
    );
}

#[test]
fn the_store_keeps_the_kind_of_an_attempt() {
    let dir = dbc::tmp();
    let s = Store::open(&dir.path().join("x.db")).expect("open");
    let (_, ids) = add_bank(&s, "Herbs.xlsx", "Herbs", 1);
    let attempt = Attempt {
        results: vec![AttemptResult {
            id: ids[0].clone(),
            correct: false,
        }],
        sheets: vec!["Herbs".to_string()],
        mode: "retry".to_string(),
        seconds: 5,
        kind: AttemptKind::Practice,
    };
    let p = s.record_attempt(1, &attempt).expect("record");
    assert_eq!(p["history"][0]["kind"], "practice");
    assert_eq!(
        s.records(1).expect("records")["records"][0]["kind"],
        "practice"
    );
    assert_eq!(
        s.record(1, 1).expect("record").expect("exists")["record"]["kind"],
        "practice"
    );
    // a viewer who cannot see the bank: nothing is written
    assert!(matches!(
        s.record_attempt_as(1, &BankScope::Only(Default::default()), &attempt, &[]),
        Err(DbError::QuestionsGone)
    ));
    assert_eq!(
        s.records(1).expect("records")["records"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
}
