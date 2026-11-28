//! v3.1: bank categories (contracts B.10, B.13, A.7, D18) and the two bank-list optimisations E.1 / E.2.
//!
//! Real router, real engine and database; every test signs in through the API. Only made-up data: category `Science` holding
//! bank `General knowledge`, category `Everyday life` holding bank `Kitchen basics`.
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
use xexam_backend::api::{build_router, AppState, Inner};
use xexam_backend::auth::{self, LoginThrottle, ThrottleLimits};
use xexam_backend::database::store::{
    ATTEMPT_BANKS_ALL_SQL, ATTEMPT_BANKS_USER_SQL, ATTEMPT_IDS_ALL_SQL, ATTEMPT_IDS_USER_SQL,
    ATTEMPT_STAMP_ALL_SQL, ATTEMPT_STAMP_USER_SQL, STATS_STAMP_ALL_SQL, STATS_STAMP_USER_SQL,
};
use xexam_backend::database::Store;
use xexam_backend::models::ImportTarget;
use xexam_backend::services::{PendingStore, RealEngine};

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";
const ADMIN_PW: &str = "pantry-admin-1";

const MSG_ADMIN_ONLY: &str = "只有管理员可以进行这个操作。";
const MSG_BAD_REQUEST: &str = "请求的格式不对。";
const MSG_BANK_NOT_FOUND: &str = "这个题库不存在（可能已被删除），请刷新页面后再试。";
const MSG_NAME_BAD: &str = "分类名称需要 1–60 个字，不能包含控制字符。";
const MSG_TAKEN: &str = "已经有同名的分类了。";
const MSG_NOT_FOUND: &str = "这个分类不存在（可能已被删除），请刷新页面后再试。";
const MSG_ORDER: &str = "分类或题库的顺序已经变了，请刷新页面后再试。";

// ---------------------------------------------------------------- helpers

struct App {
    state: AppState,
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
        old_layouts: xexam_backend::services::OldLayouts::Refuse,
        ai_link: Default::default(),
        activity: Default::default(),
        gates: Default::default(),
        push: Default::default(),
    }));
    // The activity writer runs, so the rows of the category routes can be read back
    state.0.activity.start(store.clone());
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

async fn call(
    a: &App,
    method: &str,
    path: &str,
    cookie: &str,
    body: Option<&str>,
    extra: &[(&str, &str)],
) -> Resp {
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
    for (k, v) in extra {
        hs.push((k, v.as_bytes().to_vec()));
    }
    let hrefs: Vec<(&str, &[u8])> = hs.iter().map(|(k, v)| (*k, v.as_slice())).collect();
    send(&a.router, build(method, path, Some(HOST), &hrefs, body)).await
}

async fn get(a: &App, path: &str, cookie: &str) -> Resp {
    call(a, "GET", path, cookie, None, &[]).await
}

async fn post(a: &App, path: &str, cookie: &str, body: Value) -> Resp {
    call(a, "POST", path, cookie, Some(&body.to_string()), &[]).await
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
    let r = post(
        a,
        "/api/auth/password",
        &c,
        json!({ "password": ADMIN_PW, "displayName": "Admin" }),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    cookie_of(&r)
}

/// Creates a plain user (extra create fields in `extra`), signs in as them and sets their own password. Returns the cookie.
async fn user(a: &App, admin_cookie: &str, name: &str, extra: Value) -> String {
    let mut body = json!({"username": name, "password": "initial-pass", "role": "user"});
    if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    let r = post(a, "/api/users/create", admin_cookie, body).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let c = cookie_of(&login(a, name, "initial-pass").await);
    let r = post(
        a,
        "/api/auth/password",
        &c,
        json!({ "password": format!("{name}-own-pass"), "displayName": format!("{name} display") }),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    cookie_of(&r)
}

/// Imports a made-up bank of `n` single-choice questions as a new bank, as the `import` command would. Returns the bank id.
fn add_bank(store: &Store, file: &str, sheet: &str, n: usize) -> i64 {
    let qs: Vec<_> = (0..n)
        .map(|i| {
            dbc::single(
                sheet,
                &format!("{sheet}-{i}"),
                &format!("{sheet} made-up question {i}?"),
                i as i64 + 1,
            )
        })
        .collect();
    let (_, bank) = store
        .apply_import_for(ImportTarget::New, file, &[dbc::sheet(sheet, n as i64)], &qs)
        .expect("import")
        .expect("bank");
    bank
}

/// The `(categoryId, sort)` of every bank id.
fn placement(banks: &Value, id: i64) -> (Value, i64) {
    let b = banks["banks"]
        .as_array()
        .expect("banks")
        .iter()
        .find(|b| b["id"] == id)
        .expect("bank in the list");
    (b["categoryId"].clone(), b["sort"].as_i64().expect("sort"))
}

fn category_names(banks: &Value) -> Vec<String> {
    banks["categories"]
        .as_array()
        .expect("categories")
        .iter()
        .map(|c| c["name"].as_str().expect("name").to_string())
        .collect()
}

/// One category operation as the administrator; returns the answer (asserting 200).
async fn cat_ok(a: &App, c: &str, body: Value) -> Value {
    let r = post(a, "/api/banks/categories", c, body).await;
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()
}

/// Creates a category and returns its id.
async fn create_cat(a: &App, c: &str, name: &str) -> i64 {
    let answer = cat_ok(a, c, json!({"op": "create", "name": name})).await;
    answer["categories"]
        .as_array()
        .expect("categories")
        .iter()
        .find(|x| x["name"] == name)
        .expect("created")["id"]
        .as_i64()
        .expect("id")
}

async fn move_ok(a: &App, c: &str, ids: &[i64], category: Option<i64>) -> Value {
    let r = post(
        a,
        "/api/banks/category",
        c,
        json!({"ids": ids, "categoryId": category}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()
}

fn bank_ids_in(banks: &Value, category: Option<i64>) -> Vec<i64> {
    let mut rows: Vec<(bool, i64, i64)> = banks["banks"]
        .as_array()
        .expect("banks")
        .iter()
        .filter(|b| b["categoryId"].as_i64() == category)
        .map(|b| {
            let sort = b["sort"].as_i64().expect("sort");
            (sort == 0, sort, b["id"].as_i64().expect("id"))
        })
        .collect();
    rows.sort();
    rows.into_iter().map(|r| r.2).collect()
}

// ---------------------------------------------------------------- the list and the permission

#[tokio::test]
async fn banks_answer_carries_categories_and_placement() {
    let a = app();
    let c = admin(&a).await;
    let b1 = add_bank(&a.store, "general.xlsx", "Quiz", 2);
    let r = get(&a, "/api/banks", &c).await;
    assert_eq!(r.status, 200);
    let body = r.json();
    assert_eq!(body["categories"], json!([]));
    let bank = &body["banks"][0];
    let keys: Vec<&str> = bank
        .as_object()
        .expect("bank")
        .keys()
        .map(String::as_str)
        .collect();
    let tail: Vec<&str> = keys[keys.len() - 3..].to_vec();
    assert_eq!(tail, ["manageable", "categoryId", "sort"]);
    assert_eq!(bank["categoryId"], Value::Null);
    assert_eq!(bank["sort"], 0);
    assert_eq!(bank["id"], b1);

    let sci = create_cat(&a, &c, "Science").await;
    let body = move_ok(&a, &c, &[b1], Some(sci)).await;
    assert_eq!(
        body["categories"],
        json!([{"id": sci, "name": "Science", "sort": 0}])
    );
    assert_eq!(placement(&body, b1), (json!(sci), 1));
}

#[tokio::test]
async fn only_administrators_may_edit_categories() {
    let a = app();
    let c = admin(&a).await;
    let b1 = add_bank(&a.store, "general.xlsx", "Quiz", 1);
    let maker = user(&a, &c, "maker", json!({"canManageBanks": true})).await;
    for cookie in [&maker, &user(&a, &c, "plain", json!({})).await] {
        let r = post(
            &a,
            "/api/banks/categories",
            cookie,
            json!({"op": "create", "name": "X"}),
        )
        .await;
        assert_eq!(r.status, 403, "{}", r.text());
        assert_eq!(r.error(), MSG_ADMIN_ONLY);
        let r = post(
            &a,
            "/api/banks/category",
            cookie,
            json!({"ids": [b1], "categoryId": null}),
        )
        .await;
        assert_eq!(r.status, 403);
    }
    assert_eq!(
        raw(&a.db)
            .query_row("SELECT COUNT(*) FROM bank_categories", [], |r| r
                .get::<_, i64>(0))
            .expect("count"),
        0
    );
}

#[tokio::test]
async fn plain_users_see_only_categories_that_hold_a_visible_bank() {
    let a = app();
    let c = admin(&a).await;
    let b1 = add_bank(&a.store, "general.xlsx", "Quiz", 1);
    let b2 = add_bank(&a.store, "kitchen.xlsx", "Cook", 1);
    let b3 = add_bank(&a.store, "loose.xlsx", "Loose", 1);
    let sci = create_cat(&a, &c, "Science").await;
    let life = create_cat(&a, &c, "Everyday life").await;
    let empty = create_cat(&a, &c, "Empty").await;
    move_ok(&a, &c, &[b1], Some(sci)).await;
    move_ok(&a, &c, &[b2], Some(life)).await;
    let u = user(&a, &c, "reader", json!({"banks": [b2, b3]})).await;
    let body = get(&a, "/api/banks", &u).await.json();
    assert_eq!(category_names(&body), ["Everyday life"]);
    assert_eq!(body["banks"].as_array().expect("banks").len(), 2);
    assert_eq!(placement(&body, b2), (json!(life), 1));
    // b3 was renumbered when the others left its group
    assert_eq!(placement(&body, b3), (Value::Null, 1));
    // the administrator sees all three, including the empty one, in (sort, id) order
    let body = get(&a, "/api/banks", &c).await.json();
    assert_eq!(category_names(&body), ["Science", "Everyday life", "Empty"]);
    let _ = empty;
}

// ---------------------------------------------------------------- create / rename

#[tokio::test]
async fn create_cleans_the_name_and_appends() {
    let a = app();
    let c = admin(&a).await;
    let body = cat_ok(
        &a,
        &c,
        json!({"op": "create", "name": "  Everyday \t\n life  "}),
    )
    .await;
    assert_eq!(body["categories"][0]["name"], "Everyday life");
    assert_eq!(body["categories"][0]["sort"], 0);
    let body = cat_ok(&a, &c, json!({"op": "create", "name": "Science"})).await;
    assert_eq!(body["categories"][1]["sort"], 1);
    let body = cat_ok(&a, &c, json!({"op": "create", "name": "日本語の分類"})).await;
    assert_eq!(body["categories"][2]["sort"], 2);
    // 60 characters are fine, 61 are not (characters, not bytes)
    cat_ok(&a, &c, json!({"op": "create", "name": "é".repeat(60)})).await;
    let r = post(
        &a,
        "/api/banks/categories",
        &c,
        json!({"op": "create", "name": "é".repeat(61)}),
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (400, MSG_NAME_BAD.to_string())
    );
    assert_eq!(r.json()["code"], "banks.categoryName");
}

#[tokio::test]
async fn bad_names_and_bodies_are_refused() {
    let a = app();
    let c = admin(&a).await;
    for name in [
        "",
        "   ",
        "bell\u{7}",
        "zero\u{200B}width",
        "rtl\u{202E}",
        "nul\u{0}",
    ] {
        let r = post(
            &a,
            "/api/banks/categories",
            &c,
            json!({"op": "create", "name": name}),
        )
        .await;
        assert_eq!(r.status, 400, "{name:?}");
        assert_eq!(r.json()["code"], "banks.categoryName", "{name:?}");
    }
    for body in [
        json!({}),
        json!({"op": 1}),
        json!({"op": "explode"}),
        json!({"op": "create"}),
        json!({"op": "create", "name": 5}),
        json!({"op": "rename", "name": "X"}),
        json!({"op": "rename", "id": "1", "name": "X"}),
        json!({"op": "rename", "id": true, "name": "X"}),
        json!({"op": "delete"}),
        json!({"op": "order"}),
        json!({"op": "order", "ids": "1"}),
        json!({"op": "order", "ids": [1, "2"]}),
        json!({"op": "bankOrder", "ids": []}),
        json!({"op": "bankOrder", "categoryId": "1", "ids": []}),
    ] {
        let r = post(&a, "/api/banks/categories", &c, body.clone()).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (400, MSG_BAD_REQUEST.to_string()),
            "{body}"
        );
    }
    let r = call(&a, "POST", "/api/banks/categories", &c, Some("[1]"), &[]).await;
    assert_eq!(r.status, 400);
    assert_eq!(
        raw(&a.db)
            .query_row("SELECT COUNT(*) FROM bank_categories", [], |r| r
                .get::<_, i64>(0))
            .expect("count"),
        0
    );
}

#[tokio::test]
async fn names_are_unique_without_regard_to_ascii_case() {
    let a = app();
    let c = admin(&a).await;
    let sci = create_cat(&a, &c, "Science").await;
    let life = create_cat(&a, &c, "Everyday life").await;
    let r = post(
        &a,
        "/api/banks/categories",
        &c,
        json!({"op": "create", "name": "SCIENCE"}),
    )
    .await;
    assert_eq!((r.status.as_u16(), r.error()), (409, MSG_TAKEN.to_string()));
    assert_eq!(r.json()["code"], "banks.categoryTaken");
    let r = post(
        &a,
        "/api/banks/categories",
        &c,
        json!({"op": "rename", "id": life, "name": "science"}),
    )
    .await;
    assert_eq!(r.status, 409);
    // renaming to its own name (other case) is fine
    let body = cat_ok(
        &a,
        &c,
        json!({"op": "rename", "id": sci, "name": "SCIENCE"}),
    )
    .await;
    assert_eq!(body["categories"][0]["name"], "SCIENCE");
    let body = cat_ok(
        &a,
        &c,
        json!({"op": "rename", "id": life, "name": "  Home  "}),
    )
    .await;
    assert_eq!(category_names(&body), ["SCIENCE", "Home"]);
    let r = post(
        &a,
        "/api/banks/categories",
        &c,
        json!({"op": "rename", "id": 9999, "name": "Nope"}),
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (404, MSG_NOT_FOUND.to_string())
    );
    assert_eq!(r.json()["code"], "banks.categoryNotFound");
}

#[tokio::test]
async fn rename_sets_updated_at_from_the_clock() {
    let dir = dbc::tmp();
    let db = dir.path().join("x.db");
    let store = dbc::open_fixed(&db);
    store
        .category_apply(&xexam_backend::database::store::CategoryOp::Create {
            name: "Science".into(),
        })
        .expect("create");
    store
        .category_apply(&xexam_backend::database::store::CategoryOp::Rename {
            id: 1,
            name: "Sciences".into(),
        })
        .expect("rename");
    let c = raw(&db);
    let (created, updated, name): (i64, i64, String) = c
        .query_row(
            "SELECT created_at, updated_at, name FROM bank_categories",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("row");
    assert_eq!(name, "Sciences");
    assert_eq!(created, store.unix_now());
    assert_eq!(updated, store.unix_now());
}

// ---------------------------------------------------------------- order

#[tokio::test]
async fn order_needs_exactly_the_set_of_categories() {
    let a = app();
    let c = admin(&a).await;
    let x = create_cat(&a, &c, "Alpha").await;
    let y = create_cat(&a, &c, "Beta").await;
    let z = create_cat(&a, &c, "Gamma").await;
    let body = cat_ok(&a, &c, json!({"op": "order", "ids": [z, x, y]})).await;
    assert_eq!(category_names(&body), ["Gamma", "Alpha", "Beta"]);
    let sorts: Vec<i64> = body["categories"]
        .as_array()
        .expect("categories")
        .iter()
        .map(|c| c["sort"].as_i64().expect("sort"))
        .collect();
    assert_eq!(sorts, [0, 1, 2]);
    for ids in [
        json!([x, y]),
        json!([x, y, z, z]),
        json!([x, x, y]),
        json!([x, y, z, 777]),
        json!([]),
    ] {
        let r = post(
            &a,
            "/api/banks/categories",
            &c,
            json!({"op": "order", "ids": ids}),
        )
        .await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (409, MSG_ORDER.to_string()),
            "{ids}"
        );
        assert_eq!(r.json()["code"], "banks.categoryOrder");
    }
    // nothing changed
    let body = get(&a, "/api/banks", &c).await.json();
    assert_eq!(category_names(&body), ["Gamma", "Alpha", "Beta"]);
}

#[tokio::test]
async fn bank_order_writes_one_to_n_and_checks_the_group() {
    let a = app();
    let c = admin(&a).await;
    let b1 = add_bank(&a.store, "one.xlsx", "S1", 1);
    let b2 = add_bank(&a.store, "two.xlsx", "S2", 1);
    let b3 = add_bank(&a.store, "three.xlsx", "S3", 1);
    let sci = create_cat(&a, &c, "Science").await;
    move_ok(&a, &c, &[b1, b2], Some(sci)).await;
    let body = cat_ok(
        &a,
        &c,
        json!({"op": "bankOrder", "categoryId": sci, "ids": [b2, b1]}),
    )
    .await;
    assert_eq!(placement(&body, b2), (json!(sci), 1));
    assert_eq!(placement(&body, b1), (json!(sci), 2));
    assert_eq!(bank_ids_in(&body, Some(sci)), [b2, b1]);
    // the uncategorised group is ordered the same way (only b3 is there)
    let body = cat_ok(
        &a,
        &c,
        json!({"op": "bankOrder", "categoryId": null, "ids": [b3]}),
    )
    .await;
    assert_eq!(placement(&body, b3), (Value::Null, 1));
    for (cat, ids) in [
        (json!(sci), json!([b2])),
        (json!(sci), json!([b2, b1, b3])),
        (json!(sci), json!([b2, b2])),
        (json!(sci), json!([b2, b3])),
        (Value::Null, json!([b3, b3])),
        (Value::Null, json!([])),
    ] {
        let r = post(
            &a,
            "/api/banks/categories",
            &c,
            json!({"op": "bankOrder", "categoryId": cat, "ids": ids}),
        )
        .await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (409, MSG_ORDER.to_string()),
            "{cat} {ids}"
        );
    }
    let r = post(
        &a,
        "/api/banks/categories",
        &c,
        json!({"op": "bankOrder", "categoryId": 4242, "ids": []}),
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (404, MSG_NOT_FOUND.to_string())
    );
}

// ---------------------------------------------------------------- move

#[tokio::test]
async fn move_appends_in_the_given_order_and_renumbers_the_groups_left() {
    let a = app();
    let c = admin(&a).await;
    let b: Vec<i64> = (1..=5)
        .map(|i| add_bank(&a.store, &format!("b{i}.xlsx"), &format!("S{i}"), 1))
        .collect();
    let sci = create_cat(&a, &c, "Science").await;
    let life = create_cat(&a, &c, "Everyday life").await;
    // b1, b2 into Science (given order), b3 into Life; b4, b5 stay uncategorised and unplaced
    let body = move_ok(&a, &c, &[b[1], b[0]], Some(sci)).await;
    assert_eq!(bank_ids_in(&body, Some(sci)), [b[1], b[0]]);
    assert_eq!(placement(&body, b[1]), (json!(sci), 1));
    assert_eq!(placement(&body, b[0]), (json!(sci), 2));
    move_ok(&a, &c, &[b[2]], Some(life)).await;
    // the groups they left were renumbered: the unplaced uncategorised banks got fixed places 1..n
    let body = get(&a, "/api/banks", &c).await.json();
    assert_eq!(placement(&body, b[3]), (Value::Null, 1));
    assert_eq!(placement(&body, b[4]), (Value::Null, 2));
    // b4 (uncategorised) into Science lands after the two placed ones; Uncategorised closes up
    let body = move_ok(&a, &c, &[b[3]], Some(sci)).await;
    assert_eq!(bank_ids_in(&body, Some(sci)), [b[1], b[0], b[3]]);
    assert_eq!(placement(&body, b[3]), (json!(sci), 3));
    assert_eq!(placement(&body, b[4]), (Value::Null, 1));
    // moving banks already in the target keeps the others in order and appends the moved ones
    let body = move_ok(&a, &c, &[b[1]], Some(sci)).await;
    assert_eq!(bank_ids_in(&body, Some(sci)), [b[0], b[3], b[1]]);
    assert_eq!(placement(&body, b[0]).1, 1);
    assert_eq!(placement(&body, b[1]).1, 3);
    // out of a category to Uncategorised: appended after b5
    let body = move_ok(&a, &c, &[b[0]], None).await;
    assert_eq!(bank_ids_in(&body, None), [b[4], b[0]]);
    assert_eq!(placement(&body, b[0]), (Value::Null, 2));
    assert_eq!(placement(&body, b[3]), (json!(sci), 1));
    assert_eq!(placement(&body, b[1]), (json!(sci), 2));
    // a bank from two source groups at once
    let body = move_ok(&a, &c, &[b[2], b[4]], Some(sci)).await;
    assert_eq!(bank_ids_in(&body, Some(sci)), [b[3], b[1], b[2], b[4]]);
    assert_eq!(bank_ids_in(&body, Some(life)), Vec::<i64>::new());
    assert_eq!(bank_ids_in(&body, None), [b[0]]);
    assert_eq!(placement(&body, b[0]), (Value::Null, 1));
}

#[tokio::test]
async fn move_never_touches_the_bank_update_time() {
    let dir = dbc::tmp();
    let db = dir.path().join("x.db");
    let store = dbc::open_fixed(&db);
    let b1 = add_bank(&store, "one.xlsx", "S1", 1);
    let b2 = add_bank(&store, "two.xlsx", "S2", 1);
    use xexam_backend::database::store::CategoryOp;
    store
        .category_apply(&CategoryOp::Create {
            name: "Science".into(),
        })
        .expect("create");
    let c = raw(&db);
    c.execute("UPDATE banks SET updated_at='2020-01-01 00:00'", [])
        .expect("age");
    let before = store.library_stamp().expect("stamp");
    store
        .banks_move_to_category(&[b2, b1], Some(1))
        .expect("move");
    store
        .category_apply(&CategoryOp::BankOrder {
            category_id: Some(1),
            ids: vec![b1, b2],
        })
        .expect("order");
    store
        .category_apply(&CategoryOp::Delete { id: 1 })
        .expect("delete");
    let stamps: Vec<String> = {
        let mut st = c.prepare("SELECT updated_at FROM banks").expect("prepare");
        st.query_map([], |r| r.get(0))
            .expect("query")
            .map(|r| r.expect("row"))
            .collect()
    };
    assert_eq!(stamps, ["2020-01-01 00:00", "2020-01-01 00:00"]);
    assert_eq!(store.library_stamp().expect("stamp"), before);
}

#[tokio::test]
async fn move_refusals() {
    let a = app();
    let c = admin(&a).await;
    let b1 = add_bank(&a.store, "one.xlsx", "S1", 1);
    let sci = create_cat(&a, &c, "Science").await;
    let mv = |body: Value| {
        let (a, c) = (&a, &c);
        async move { post(a, "/api/banks/category", c, body).await }
    };
    let r = mv(json!({"ids": [b1, 9999], "categoryId": sci})).await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (404, MSG_BANK_NOT_FOUND.to_string())
    );
    let r = mv(json!({"ids": [b1], "categoryId": 9999})).await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (404, MSG_NOT_FOUND.to_string())
    );
    assert_eq!(r.json()["code"], "banks.categoryNotFound");
    for body in [
        json!({"ids": [b1, b1], "categoryId": sci}),
        json!({"ids": [], "categoryId": sci}),
        json!({"ids": [b1]}),
        json!({"categoryId": sci}),
        json!({"ids": "1", "categoryId": sci}),
        json!({"ids": [b1, "x"], "categoryId": sci}),
        json!({"ids": [b1], "categoryId": "1"}),
    ] {
        let r = mv(body.clone()).await;
        assert_eq!(
            (r.status.as_u16(), r.error()),
            (400, MSG_BAD_REQUEST.to_string()),
            "{body}"
        );
    }
    // 1000 ids are the most one call may name (they do not exist here, so the 1001-id call is refused before that check)
    let many: Vec<i64> = (10_000..11_001).collect();
    let r = mv(json!({"ids": many, "categoryId": sci})).await;
    assert_eq!(r.status, 400);
    let r = mv(json!({"ids": &many[..1000], "categoryId": sci})).await;
    assert_eq!(r.status, 404);
    // nothing was written by any refusal
    let body = get(&a, "/api/banks", &c).await.json();
    assert_eq!(placement(&body, b1), (Value::Null, 0));
}

// ---------------------------------------------------------------- delete (A.7)

#[tokio::test]
async fn deleting_a_category_appends_its_banks_to_uncategorised_and_renumbers() {
    let a = app();
    let c = admin(&a).await;
    let loose1 = add_bank(&a.store, "loose1.xlsx", "L1", 1);
    let loose2 = add_bank(&a.store, "loose2.xlsx", "L2", 1);
    let k1 = add_bank(&a.store, "k1.xlsx", "K1", 1);
    let k2 = add_bank(&a.store, "k2.xlsx", "K2", 1);
    let sci = create_cat(&a, &c, "Science").await;
    let life = create_cat(&a, &c, "Everyday life").await;
    let third = create_cat(&a, &c, "Third").await;
    move_ok(&a, &c, &[k2, k1], Some(life)).await;
    // Uncategorised now holds loose1, loose2 (renumbered 1, 2 by the move); delete Life
    let body = cat_ok(&a, &c, json!({"op": "delete", "id": life})).await;
    assert_eq!(bank_ids_in(&body, None), [loose1, loose2, k2, k1]);
    for (i, id) in [loose1, loose2, k2, k1].iter().enumerate() {
        assert_eq!(placement(&body, *id), (Value::Null, i as i64 + 1));
    }
    // the remaining categories are renumbered 0..n-1 in their order
    assert_eq!(category_names(&body), ["Science", "Third"]);
    let sorts: Vec<i64> = body["categories"]
        .as_array()
        .expect("categories")
        .iter()
        .map(|c| c["sort"].as_i64().expect("sort"))
        .collect();
    assert_eq!(sorts, [0, 1]);
    // deleting the first category renumbers the others
    let body = cat_ok(&a, &c, json!({"op": "delete", "id": sci})).await;
    assert_eq!(
        body["categories"],
        json!([{"id": third, "name": "Third", "sort": 0}])
    );
    let r = post(
        &a,
        "/api/banks/categories",
        &c,
        json!({"op": "delete", "id": sci}),
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.error()),
        (404, MSG_NOT_FOUND.to_string())
    );
    // the foreign key is satisfied: no bank points at a deleted category
    let n: i64 = raw(&a.db)
        .query_row(
            "SELECT COUNT(*) FROM banks WHERE category_id IS NOT NULL AND category_id NOT IN (SELECT id FROM bank_categories)",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(n, 0);
}

#[tokio::test]
async fn deleting_a_category_fixes_the_places_of_unplaced_banks_first() {
    let a = app();
    let c = admin(&a).await;
    // three unplaced uncategorised banks (sort 0) plus one in a category
    let u1 = add_bank(&a.store, "u1.xlsx", "U1", 1);
    let k1 = add_bank(&a.store, "k1.xlsx", "K1", 1);
    let u2 = add_bank(&a.store, "u2.xlsx", "U2", 1);
    let life = create_cat(&a, &c, "Everyday life").await;
    // put k1 in the category by raw SQL so the uncategorised banks stay unplaced (sort 0)
    raw(&a.db)
        .execute(
            "UPDATE banks SET category_id=?1, sort=1 WHERE id=?2",
            rusqlite::params![life, k1],
        )
        .expect("place");
    let body = get(&a, "/api/banks", &c).await.json();
    assert_eq!(placement(&body, u1), (Value::Null, 0));
    let body = cat_ok(&a, &c, json!({"op": "delete", "id": life})).await;
    assert_eq!(bank_ids_in(&body, None), [u1, u2, k1]);
    assert_eq!([u1, u2, k1].map(|id| placement(&body, id).1), [1, 2, 3]);
}

// ---------------------------------------------------------------- new banks land at the end

#[tokio::test]
async fn a_new_bank_lands_at_the_end_of_uncategorised() {
    let a = app();
    let c = admin(&a).await;
    let b1 = add_bank(&a.store, "one.xlsx", "S1", 1);
    let b2 = add_bank(&a.store, "two.xlsx", "S2", 1);
    let sci = create_cat(&a, &c, "Science").await;
    // a round trip through Science places both uncategorised banks: b1 at 1 (renumbered), b2 appended at 2
    move_ok(&a, &c, &[b2], Some(sci)).await;
    move_ok(&a, &c, &[b2], None).await;
    let body = get(&a, "/api/banks", &c).await.json();
    assert_eq!(placement(&body, b1), (Value::Null, 1));
    assert_eq!(placement(&body, b2), (Value::Null, 2));
    // a bank created later (any path) is unplaced (sort 0) and shows last
    let b3 = add_bank(&a.store, "three.xlsx", "S3", 1);
    let body = get(&a, "/api/banks", &c).await.json();
    assert_eq!(placement(&body, b3), (Value::Null, 0));
    assert_eq!(bank_ids_in(&body, None), [b1, b2, b3]);
    // and it stays behind banks that are placed later, whatever their ids
    move_ok(&a, &c, &[b3], Some(sci)).await;
    move_ok(&a, &c, &[b1], Some(sci)).await;
    let b4 = add_bank(&a.store, "four.xlsx", "S4", 1);
    let body = get(&a, "/api/banks", &c).await.json();
    assert_eq!(bank_ids_in(&body, None), [b2, b4]);
    assert_eq!(bank_ids_in(&body, Some(sci)), [b3, b1]);
}

// ---------------------------------------------------------------- stamps (B.13)

#[tokio::test]
async fn category_edits_change_the_banks_etag_and_the_categories_stamp_only() {
    let a = app();
    let c = admin(&a).await;
    let b1 = add_bank(&a.store, "one.xlsx", "S1", 1);
    let b2 = add_bank(&a.store, "two.xlsx", "S2", 1);
    let etag = |path: &'static str| {
        let (a, c) = (&a, &c);
        async move { get(a, path, c).await.header("etag").expect("etag") }
    };
    let state = || {
        let (a, c) = (&a, &c);
        async move { get(a, "/api/auth/state", c).await.json() }
    };
    let (banks0, bank0, st0) = (
        etag("/api/banks").await,
        etag("/api/bank").await,
        state().await,
    );
    let (lib0, cat0) = (st0["library"].clone(), st0["categories"].clone());
    assert!(cat0.as_str().is_some_and(|s| !s.is_empty()), "{st0}");

    let sci = create_cat(&a, &c, "Science").await;
    let (banks1, bank1, st1) = (
        etag("/api/banks").await,
        etag("/api/bank").await,
        state().await,
    );
    assert_ne!(banks1, banks0);
    assert_eq!(bank1, bank0);
    assert_eq!(st1["library"], lib0);
    assert_ne!(st1["categories"], cat0);

    // each kind of edit moves the stamps, none touches the library stamp
    let mut seen = vec![(banks1, st1["categories"].clone())];
    let edits: Vec<(&str, Value)> = vec![
        (
            "/api/banks/category",
            json!({"ids": [b1], "categoryId": sci}),
        ),
        (
            "/api/banks/category",
            json!({"ids": [b2], "categoryId": sci}),
        ),
        (
            "/api/banks/categories",
            json!({"op": "bankOrder", "categoryId": sci, "ids": [b2, b1]}),
        ),
        (
            "/api/banks/categories",
            json!({"op": "rename", "id": sci, "name": "Sciences"}),
        ),
        (
            "/api/banks/categories",
            json!({"op": "create", "name": "Everyday life"}),
        ),
    ];
    for (path, body) in edits {
        let r = post(&a, path, &c, body.clone()).await;
        assert_eq!(r.status, 200, "{body}: {}", r.text());
        let (e, s) = (etag("/api/banks").await, state().await);
        assert!(
            seen.iter().all(|(old, _)| *old != e),
            "ETag of /api/banks did not change for {body}"
        );
        assert!(
            seen.iter().all(|(_, old)| *old != s["categories"]),
            "categories stamp did not change for {body}"
        );
        assert_eq!(s["library"], lib0, "{body}");
        assert_eq!(etag("/api/bank").await, bank0, "{body}");
        seen.push((e, s["categories"].clone()));
    }
    // and a conditional GET is answered 304 with the current tag
    let tag = etag("/api/banks").await;
    let r = call(
        &a,
        "GET",
        "/api/banks",
        &c,
        None,
        &[("if-none-match", &tag)],
    )
    .await;
    assert_eq!(r.status, 304);
}

#[tokio::test]
async fn the_categories_stamp_of_a_plain_user_follows_only_what_they_see() {
    let a = app();
    let c = admin(&a).await;
    let b1 = add_bank(&a.store, "one.xlsx", "S1", 1);
    let b2 = add_bank(&a.store, "two.xlsx", "S2", 1);
    let sci = create_cat(&a, &c, "Science").await;
    let hidden = create_cat(&a, &c, "Hidden").await;
    move_ok(&a, &c, &[b1], Some(sci)).await;
    move_ok(&a, &c, &[b2], Some(hidden)).await;
    let u = user(&a, &c, "reader", json!({"banks": [b1]})).await;
    let stamp = |cookie: String| {
        let a = &a;
        async move { get(a, "/api/auth/state", &cookie).await.json()["categories"].clone() }
    };
    let s0 = stamp(u.clone()).await;
    assert!(s0.as_str().is_some_and(|s| !s.is_empty()));
    // an edit of a category the user cannot see leaves their stamp alone
    cat_ok(
        &a,
        &c,
        json!({"op": "rename", "id": hidden, "name": "Secret"}),
    )
    .await;
    move_ok(&a, &c, &[b2], None).await;
    assert_eq!(stamp(u.clone()).await, s0);
    // an edit of their own category moves it
    cat_ok(
        &a,
        &c,
        json!({"op": "rename", "id": sci, "name": "Sciences"}),
    )
    .await;
    assert_ne!(stamp(u.clone()).await, s0);
}

// ---------------------------------------------------------------- activity

#[tokio::test]
async fn category_edits_are_recorded_in_the_activity_log() {
    let a = app();
    let c = admin(&a).await;
    let b1 = add_bank(&a.store, "one.xlsx", "S1", 1);
    let sci = create_cat(&a, &c, "Science").await;
    move_ok(&a, &c, &[b1], Some(sci)).await;
    a.state.0.activity.flush().await;
    let conn = raw(&a.db);
    let mut st = conn
        .prepare("SELECT action, COALESCE(detail, '') FROM activity_log WHERE action IN ('bank.categories', 'bank.categoryMove') ORDER BY id")
        .expect("prepare");
    let rows: Vec<(String, String)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert!(rows.len() >= 2, "activity rows missing: {rows:?}");
    assert_eq!(rows[0].0, "bank.categories");
    let d: Value = serde_json::from_str(&rows[0].1).expect("detail");
    assert_eq!(d, json!({"op": "create", "name": "Science"}));
    assert_eq!(rows[1].0, "bank.categoryMove");
    let d: Value = serde_json::from_str(&rows[1].1).expect("detail");
    assert_eq!(d, json!({"count": 1, "categoryId": sci}));
}

// ---------------------------------------------------------------- E.1: query plans

fn plan(c: &Connection, sql: &str) -> String {
    let mut st = c
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("prepare");
    let params: Vec<i64> = if sql.contains("?1") { vec![1] } else { vec![] };
    st.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        r.get::<_, String>(3)
    })
    .expect("plan")
    .map(|r| r.expect("row"))
    .collect::<Vec<_>>()
    .join(" | ")
}

#[test]
fn the_one_user_statements_use_their_indexes() {
    let dir = dbc::tmp();
    let db = dir.path().join("x.db");
    let _store = dbc::open_fixed(&db);
    let c = raw(&db);
    for sql in [
        ATTEMPT_BANKS_USER_SQL,
        ATTEMPT_STAMP_USER_SQL,
        ATTEMPT_IDS_USER_SQL,
    ] {
        let p = plan(&c, sql);
        assert!(
            p.contains("USING INDEX idx_attempts_user")
                || p.contains("USING COVERING INDEX idx_attempts_user"),
            "{sql}: {p}"
        );
        assert!(!p.contains("SCAN attempts"), "{sql}: {p}");
    }
    // user_stats: its primary key starts with user_id
    let p = plan(&c, STATS_STAMP_USER_SQL);
    assert!(
        p.contains("USING") && p.contains("INDEX") || p.contains("PRIMARY KEY"),
        "{p}"
    );
    assert!(!p.contains("SCAN user_stats"), "{p}");
    // the administrator forms are plain scans and carry no placeholder
    for sql in [
        ATTEMPT_BANKS_ALL_SQL,
        ATTEMPT_STAMP_ALL_SQL,
        ATTEMPT_IDS_ALL_SQL,
        STATS_STAMP_ALL_SQL,
    ] {
        assert!(!sql.contains("?1") && !sql.contains("IS NULL"), "{sql}");
    }
}

#[tokio::test]
async fn records_and_stamps_stay_per_user_after_the_split() {
    let a = app();
    let c = admin(&a).await;
    let b1 = add_bank(&a.store, "one.xlsx", "Quiz", 2);
    let u1 = user(&a, &c, "alice", json!({"banks": [b1]})).await;
    let u2 = user(&a, &c, "bob", json!({"banks": [b1]})).await;
    let qid = xexam_backend::database::scoped_id(b1, "Quiz-0");
    let attempt_body = json!({"results": [{"id": qid, "correct": true, "answer": ["A"]}], "sheets": ["Quiz"], "mode": "all", "seconds": 5});
    let attempt = |cookie: String| {
        let (a, body) = (&a, attempt_body.clone());
        async move { post(a, "/api/attempt", &cookie, body).await }
    };
    assert_eq!(attempt(u1.clone()).await.status, 200);
    assert_eq!(attempt(u1.clone()).await.status, 200);
    assert_eq!(attempt(u2.clone()).await.status, 200);
    let records_of = |cookie: String| {
        let a = &a;
        async move { get(a, "/api/banks", &cookie).await.json()["banks"][0]["records"].clone() }
    };
    assert_eq!(records_of(u1.clone()).await, 2);
    assert_eq!(records_of(u2.clone()).await, 1);
    assert_eq!(records_of(c.clone()).await, 3);
    // a record of one user moves only that user's (and the administrator's) ETag
    let tag = |cookie: String| {
        let a = &a;
        async move {
            get(a, "/api/banks", &cookie)
                .await
                .header("etag")
                .expect("etag")
        }
    };
    let (t1, t2, ta) = (
        tag(u1.clone()).await,
        tag(u2.clone()).await,
        tag(c.clone()).await,
    );
    assert_eq!(attempt(u1.clone()).await.status, 200);
    assert_ne!(tag(u1.clone()).await, t1);
    assert_eq!(tag(u2.clone()).await, t2);
    assert_ne!(tag(c.clone()).await, ta);
}

// ---------------------------------------------------------------- E.2: aggregated sheets

#[tokio::test]
async fn sheet_order_follows_the_first_seq_when_sheets_interleave() {
    let a = app();
    let c = admin(&a).await;
    let mut qs = Vec::new();
    for (sheet, n) in [("Alpha", 3), ("Beta", 3)] {
        for i in 0..n {
            qs.push(dbc::single(
                sheet,
                &format!("{sheet}-{i}"),
                &format!("{sheet} made-up {i}?"),
                i + 1,
            ));
        }
    }
    let (_, bank) = a
        .store
        .apply_import_for(
            ImportTarget::New,
            "mixed.xlsx",
            &[dbc::sheet("Alpha", 3), dbc::sheet("Beta", 3)],
            &qs,
        )
        .expect("import")
        .expect("bank");
    // Interleave the seq of the two sheets: Beta takes 1, 3, 5 and Alpha 2, 4, 6, and retire one Beta and one Alpha question
    let conn = raw(&a.db);
    for (sheet, seqs) in [("Beta", [1, 3, 5]), ("Alpha", [2, 4, 6])] {
        let ids: Vec<String> = {
            let mut st = conn
                .prepare("SELECT id FROM questions WHERE bank_id=?1 AND sheet=?2 ORDER BY seq")
                .expect("prepare");
            st.query_map(rusqlite::params![bank, sheet], |r| r.get(0))
                .expect("query")
                .map(|r| r.expect("row"))
                .collect()
        };
        for (id, seq) in ids.iter().zip(seqs) {
            conn.execute(
                "UPDATE questions SET seq=?1 WHERE id=?2",
                rusqlite::params![seq + 100, id],
            )
            .expect("seq");
        }
    }
    conn.execute(
        "UPDATE questions SET active=0 WHERE bank_id=?1 AND sheet='Beta' AND seq=105",
        [bank],
    )
    .expect("retire");
    conn.execute(
        "UPDATE questions SET active=0 WHERE bank_id=?1 AND sheet='Alpha' AND seq=102",
        [bank],
    )
    .expect("retire");
    let body = get(&a, "/api/banks", &c).await.json();
    let entry = &body["banks"][0];
    assert_eq!(
        entry["sheets"],
        json!([
            {"name": "Beta", "usable": 2, "inactive": 1},
            {"name": "Alpha", "usable": 2, "inactive": 1}
        ])
    );
    assert_eq!(
        (entry["total"].clone(), entry["inactive"].clone()),
        (json!(4), json!(2))
    );
    // `/api/bank` orders the same sheets the same way
    let lib = get(&a, "/api/bank", &c).await.json();
    let lib_text = lib.to_string();
    assert!(
        lib_text.find("Beta").expect("Beta") < lib_text.find("Alpha").expect("Alpha"),
        "{lib_text}"
    );
    // a bank with no question rows at all: empty sheets, zero totals
    let empty_bank: i64 = {
        conn.execute(
            "INSERT INTO banks(name, filename, legacy_ids, created_at, updated_at, owner_id) VALUES ('Empty', 'e.xlsx', 0, '2026-01-01 10:00', '2026-01-01 10:00', 1)",
            [],
        )
        .expect("bank");
        conn.last_insert_rowid()
    };
    let body = get(&a, "/api/banks", &c).await.json();
    let e = body["banks"]
        .as_array()
        .expect("banks")
        .iter()
        .find(|b| b["id"] == empty_bank)
        .expect("empty bank");
    assert_eq!(
        (
            e["sheets"].clone(),
            e["total"].clone(),
            e["inactive"].clone()
        ),
        (json!([]), json!(0), json!(0))
    );
}

// ---------------------------------------------------------------- A.7: the foreign-key graph

#[test]
fn deleting_a_bank_inside_a_category_still_works() {
    let dir = dbc::tmp();
    let db = dir.path().join("x.db");
    let store = dbc::open_fixed(&db);
    use xexam_backend::database::store::CategoryOp;
    let b1 = add_bank(&store, "one.xlsx", "S1", 1);
    store
        .category_apply(&CategoryOp::Create {
            name: "Science".into(),
        })
        .expect("create");
    store.banks_move_to_category(&[b1], Some(1)).expect("move");
    // the category is deleted while a bank sits in it: child-first, so the foreign key never fails
    store
        .category_apply(&CategoryOp::Delete { id: 1 })
        .expect("delete with a bank inside");
    let v = xexam_backend::models::Viewer::admin(0);
    assert!(store
        .delete_bank_as(&v, b1, false)
        .expect("delete")
        .is_some());
    // and a bank in a category can be deleted while the category stays
    let b2 = add_bank(&store, "two.xlsx", "S2", 1);
    store
        .category_apply(&CategoryOp::Create {
            name: "Again".into(),
        })
        .expect("create");
    store.banks_move_to_category(&[b2], Some(2)).expect("move");
    assert!(store
        .delete_bank_as(&v, b2, false)
        .expect("delete")
        .is_some());
    assert_eq!(
        store.banks().expect("banks")["categories"],
        json!([{"id": 2, "name": "Again", "sort": 0}])
    );
}

#[test]
fn the_store_refuses_like_the_api() {
    use xexam_backend::database::store::{CategoryError, CategoryOp};
    let dir = dbc::tmp();
    let db = dir.path().join("x.db");
    let store = dbc::open_fixed(&db);
    let b1 = add_bank(&store, "one.xlsx", "S1", 1);
    assert!(matches!(
        store.category_apply(&CategoryOp::Create {
            name: String::new()
        }),
        Err(CategoryError::NameBad)
    ));
    store
        .category_apply(&CategoryOp::Create {
            name: "Science".into(),
        })
        .expect("create");
    assert!(matches!(
        store.category_apply(&CategoryOp::Create {
            name: "science".into()
        }),
        Err(CategoryError::Taken)
    ));
    assert!(matches!(
        store.category_apply(&CategoryOp::Delete { id: 7 }),
        Err(CategoryError::NotFound)
    ));
    assert!(matches!(
        store.category_apply(&CategoryOp::Order { ids: vec![] }),
        Err(CategoryError::Order)
    ));
    assert!(matches!(
        store.banks_move_to_category(&[b1, b1], None),
        Err(CategoryError::BadRequest)
    ));
    assert!(matches!(
        store.banks_move_to_category(&[b1], Some(7)),
        Err(CategoryError::NotFound)
    ));
    assert!(matches!(
        store.banks_move_to_category(&[99], Some(1)),
        Err(CategoryError::BankNotFound)
    ));
}
