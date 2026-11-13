//! v1.1 per-user AI+ feature switches (FP-1 … FP-5, addendum P-9 … P-13): the six keys, the server rule on every model
//! route, the admin route, administrators never refused. All data is made up.
mod common;

use common::genkit::*;
use common::*;
use serde_json::{json, Value};

const ALL_ON: fn() -> Value = || json!({"chat": true, "hint": true, "explain": true, "summary": true, "generate": true, "import": true});

fn only_off(key: &str) -> Value {
    let mut v = ALL_ON();
    v[key] = json!(false);
    v
}

struct E {
    t: TestApp,
    admin: Who,
    user: Who,
}

async fn env() -> E {
    let t = start().await;
    let admin = t.user("root", true, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let user = t.user("lee", false, true);
    t.relax_caps();
    t.backend.st().gen = kitchen(2, 20);
    t.stub.st().script = Some(good_model());
    t.add_question(single_question("q1", "Which tool fits the made-up job?"));
    t.add_record(FakeRecord {
        user: user.id,
        id: 1,
        at: "2026-11-12 09:30".into(),
        kind: "practice".into(),
        key: None,
        started_at: None,
        detail: true,
        items: vec![FakeItem {
            qid: "q1".into(),
            answer: json!(["A"]),
            correct: false,
        }],
    });
    E { t, admin, user }
}

async fn set_features(e: &E, who: &Who, features: Value) -> (u16, Value) {
    e.t.post(
        "/api/ai/admin/features",
        &e.admin,
        json!({"userId": who.id, "features": features}),
    )
    .await
}

fn hint_body() -> Value {
    json!({"kind": "hint", "mode": "practice", "key": "0123456789abcdef0123456789abcdef",
           "startedAt": xexam_ai_service::util::unix_now() - 3600, "questionId": "q1", "no": 1,
           "qlang": "en", "source": "practice", "lang": "en"})
}

fn explain_body() -> Value {
    let mut b = hint_body();
    b["kind"] = json!("explain");
    b
}

async fn refused_stream(t: &TestApp, who: &Who, path: &str, body: Value) -> (u16, Value) {
    match t.open_stream(path, who, body).await {
        Ok(_) => panic!("expected a refusal"),
        Err(e) => e,
    }
}

#[tokio::test]
async fn the_status_has_the_six_keys_for_ai_plus_users_only() {
    let e = env().await;
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(v["features"], ALL_ON());
    let keys: Vec<&String> = v["features"].as_object().expect("obj").keys().collect();
    assert_eq!(
        keys,
        ["chat", "hint", "explain", "summary", "generate", "import"]
    );
    let (s, _) = set_features(&e, &e.user, only_off("hint")).await;
    assert_eq!(s, 200);
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(v["features"]["hint"], false);
    assert_eq!(v["features"]["chat"], true);
    // administrators: all true; a user without AI+: no key at all
    let (_, v) = e.t.get("/api/ai/status", &e.admin).await;
    assert_eq!(v["features"], ALL_ON());
    let plain = e.t.user("plain", false, false);
    let (_, v) = e.t.get("/api/ai/status", &plain).await;
    assert!(v.get("features").is_none());
    // the features are there even when AI is switched off
    let (s, _) =
        e.t.post("/api/ai/admin/enable", &e.admin, json!({"enabled": false}))
            .await;
    assert_eq!(s, 200);
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(v["enabled"], false);
    assert_eq!(v["why"], "off");
    assert_eq!(v["features"]["hint"], false);
}

#[tokio::test]
async fn every_model_route_is_refused_with_its_feature_and_counts_nothing() {
    let e = env().await;
    // an answer and a saved record exist so the later routes have something to work on
    let sent =
        e.t.send_all(&e.user, json!({"text": "hello", "lang": "en"}))
            .await
            .expect("send");
    let msg_id = sent
        .iter()
        .find_map(|(n, v)| (n == "start").then(|| v["userMessageId"].as_i64()))
        .flatten()
        .expect("message id");
    // a cap of one call a minute: a refusal must not use it up
    let (s, v) =
        e.t.post(
            "/api/ai/admin/limits",
            &e.admin,
            json!({"callsPerMinute": 100}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let usage_before = e.t.usage_rows();
    let cases: Vec<(&str, &str, Value, &str)> = vec![
        (
            "chat",
            "/api/ai/send",
            json!({"text": "again", "lang": "en"}),
            "chat",
        ),
        (
            "chat",
            "/api/ai/memories/suggest",
            json!({"messageId": msg_id, "lang": "en"}),
            "chat",
        ),
        ("hint", "/api/ai/study/ask", hint_body(), "hint"),
        ("explain", "/api/ai/study/ask", explain_body(), "explain"),
        (
            "summary",
            "/api/ai/summary/ask",
            json!({"recordId": 1, "lang": "en"}),
            "summary",
        ),
    ];
    for (key, path, body, feature) in &cases {
        let (s, _) = set_features(&e, &e.user, only_off(key)).await;
        assert_eq!(s, 200);
        let (st, v) = if path.contains("suggest") {
            e.t.post(path, &e.user, body.clone()).await
        } else {
            refused_stream(&e.t, &e.user, path, body.clone()).await
        };
        assert_eq!(
            (st, &v["code"], &v["feature"]),
            (403, &json!("ai.featureOff"), &json!(feature)),
            "{path} with {key} off: {v}"
        );
    }
    assert_eq!(
        e.t.usage_rows(),
        usage_before,
        "a refusal writes no usage row"
    );
    // generation and import
    let (s, _) = set_features(&e, &e.user, only_off("generate")).await;
    assert_eq!(s, 200);
    let (st, v) =
        e.t.post(
            "/api/ai/gen/start",
            &e.user,
            json!({"source": "bank", "bankId": 3}),
        )
        .await;
    assert_eq!(
        (st, &v["code"], &v["feature"]),
        (403, &json!("ai.featureOff"), &json!("generate"))
    );
    let (s, _) = set_features(&e, &e.user, only_off("import")).await;
    assert_eq!(s, 200);
    let (st, v) =
        e.t.upload(&e.user, "notes.txt", b"hello there".to_vec())
            .await;
    assert_eq!(
        (st, &v["code"], &v["feature"]),
        (403, &json!("ai.featureOff"), &json!("import"))
    );
    let (st, v) =
        e.t.post(
            "/api/ai/import/confirm",
            &e.user,
            json!({"id": 1, "name": "Notes"}),
        )
        .await;
    assert_eq!(
        (st, &v["code"], &v["feature"]),
        (403, &json!("ai.featureOff"), &json!("import"))
    );
}

#[tokio::test]
async fn a_refusal_never_uses_a_call_cap_and_the_next_allowed_call_passes() {
    let e = env().await;
    let (s, v) =
        e.t.post(
            "/api/ai/admin/limits",
            &e.admin,
            json!({"callsPerMinute": 1, "callsPerHour": 1, "callsPerDay": 1}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    set_features(&e, &e.user, only_off("chat")).await;
    for _ in 0..3 {
        let (st, v) =
            e.t.open_stream("/api/ai/send", &e.user, json!({"text": "x", "lang": "en"}))
                .await
                .err()
                .expect("refused");
        assert_eq!((st, &v["code"]), (403, &json!("ai.featureOff")));
    }
    set_features(&e, &e.user, ALL_ON()).await;
    let ev =
        e.t.send_all(&e.user, json!({"text": "now it works", "lang": "en"}))
            .await
            .expect("allowed");
    assert_eq!(terminal(&ev).0, "done");
    // and now the cap is used
    let (st, v) =
        e.t.open_stream(
            "/api/ai/send",
            &e.user,
            json!({"text": "again", "lang": "en"}),
        )
        .await
        .err()
        .expect("capped");
    assert_eq!((st, &v["code"]), (429, &json!("ai.callLimit")));
}

#[tokio::test]
async fn generate_and_import_are_separate_permissions() {
    let e = env().await;
    // generate off, import on: an import is allowed, a generation refused
    set_features(&e, &e.user, only_off("generate")).await;
    let (st, v) =
        e.t.post(
            "/api/ai/gen/start",
            &e.user,
            json!({"source": "bank", "bankId": 3}),
        )
        .await;
    assert_eq!((st, &v["feature"]), (403, &json!("generate")));
    let file = {
        let z = xexam_ai_service::extract::testgen::docx(
            &xexam_ai_service::extract::testgen::DocxSpec::paragraphs(
                &questions_lines(3)
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            ),
        );
        z
    };
    let (st, v) = e.t.upload(&e.user, "course.docx", file.clone()).await;
    assert_eq!(st, 202, "{v}");
    let id = v["job"]["id"].as_i64().expect("id");
    let (s, _) =
        e.t.post("/api/ai/gen/cancel", &e.user, json!({"id": id}))
            .await;
    assert_eq!(s, 200);
    // the reverse
    set_features(&e, &e.user, only_off("import")).await;
    let (st, v) = e.t.upload(&e.user, "course.docx", file).await;
    assert_eq!((st, &v["feature"]), (403, &json!("import")));
    let (st, v) =
        e.t.post(
            "/api/ai/gen/start",
            &e.user,
            json!({"source": "bank", "bankId": 3}),
        )
        .await;
    assert_eq!(st, 202, "{v}");
}

#[tokio::test]
async fn ungated_routes_pass_with_every_feature_off_and_administrators_are_never_refused() {
    let e = env().await;
    set_features(
        &e,
        &e.user,
        json!({"chat": false, "hint": false, "explain": false, "summary": false, "generate": false, "import": false}),
    )
    .await;
    for path in ["/api/ai/chats", "/api/ai/memories", "/api/ai/status"] {
        let (s, v) = e.t.get(path, &e.user).await;
        assert_eq!(s, 200, "{path}: {v}");
    }
    let (s, v) =
        e.t.post(
            "/api/ai/memories/add",
            &e.user,
            json!({"content": "likes short answers"}),
        )
        .await;
    assert_eq!(s, 200, "adding a memory by hand needs no model: {v}");
    let (s, _) = e.t.get("/api/ai/study/saved?mode=practice&key=0123456789abcdef0123456789abcdef&source=practice&question=q1&kind=hint", &e.user).await;
    assert_eq!(s, 200);
    // an administrator whose stored bits are set is not refused (the bits are never read for administrators)
    e.t.db()
        .execute(
            "INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at, features_off) VALUES (?1, ?2, 'root', 1, 1, 63)
             ON CONFLICT(id) DO UPDATE SET features_off = 63",
            rusqlite::params![e.admin.id, e.admin.stamp],
        )
        .expect("bits");
    let ev =
        e.t.send_all(&e.admin, json!({"text": "admin hello", "lang": "en"}))
            .await
            .expect("allowed");
    assert_eq!(terminal(&ev).0, "done");
}

#[tokio::test]
async fn the_admin_route_audits_creates_rows_and_refuses_bad_requests() {
    let e = env().await;
    // an AI+ user who never used AI has no row yet: the route creates it
    let fresh = e.t.user("fresh", false, true);
    let rows: i64 =
        e.t.db()
            .query_row(
                "SELECT COUNT(*) FROM users WHERE id = ?1",
                [fresh.id],
                |r| r.get(0),
            )
            .expect("count");
    assert_eq!(rows, 0);
    let mut f = ALL_ON();
    f["hint"] = json!(false);
    f["generate"] = json!(false);
    let (s, v) = set_features(&e, &fresh, f.clone()).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["user"]["features"], f);
    let stamp: String =
        e.t.db()
            .query_row("SELECT stamp FROM users WHERE id = ?1", [fresh.id], |r| {
                r.get(0)
            })
            .expect("row");
    assert_eq!(stamp, fresh.stamp);
    // the audit line names the changes only; the same call again writes nothing new
    assert!(
        e.t.backend.log_contains(
            "admin \"root\" changed the AI features of \"fresh\": hint off, generate off"
        ),
        "{:?}",
        e.t.backend.logs()
    );
    let before = e.t.backend.logs().len();
    let (s, _) = set_features(&e, &fresh, f.clone()).await;
    assert_eq!(s, 200);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(e.t.backend.logs().len(), before, "unchanged: no audit line");
    // the user list carries the keys (administrators: all true)
    let (_, v) = e.t.get("/api/ai/admin/users", &e.admin).await;
    let row = v["users"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["username"] == "fresh")
        .expect("row")
        .clone();
    assert_eq!(row["features"], f);
    let admin_row = v["users"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["username"] == "root")
        .expect("row")
        .clone();
    assert_eq!(admin_row["features"], ALL_ON());
    // refusals
    let (s, v) = set_features(&e, &e.admin, ALL_ON()).await;
    assert_eq!((s, &v["code"]), (409, &json!("ai.adminAlways")));
    let (s, v) =
        e.t.post(
            "/api/ai/admin/features",
            &e.admin,
            json!({"userId": fresh.id, "features": {"chat": true}}),
        )
        .await;
    assert_eq!((s, &v["code"]), (400, &json!("ai.badRequest")));
    let five =
        json!({"chat": true, "hint": true, "explain": true, "summary": true, "generate": true});
    let (s, v) =
        e.t.post(
            "/api/ai/admin/features",
            &e.admin,
            json!({"userId": fresh.id, "features": five}),
        )
        .await;
    assert_eq!(
        (s, &v["code"]),
        (400, &json!("ai.badRequest")),
        "the five-key v1.0 shape is refused"
    );
    let (s, v) =
        e.t.post(
            "/api/ai/admin/features",
            &e.admin,
            json!({"userId": fresh.id}),
        )
        .await;
    assert_eq!((s, &v["code"]), (400, &json!("ai.badRequest")));
    let (s, v) = set_features(
        &e,
        &Who {
            id: 99_999,
            ..fresh.clone()
        },
        ALL_ON(),
    )
    .await;
    assert_eq!(
        (s, &v["code"]),
        (404, &json!("ai.notFound")),
        "an id the main service does not know"
    );
    let (s, v) =
        e.t.post(
            "/api/ai/admin/features",
            &e.user,
            json!({"userId": fresh.id, "features": ALL_ON()}),
        )
        .await;
    assert_eq!((s, &v["code"]), (403, &json!("ai.adminOnly")));
    // the ticks persist when AI+ is taken away and given back, and when the user is paused
    e.t.set_ai(&fresh, false);
    e.t.set_ai(&fresh, true);
    let (_, v) = e.t.get("/api/ai/status", &fresh).await;
    assert_eq!(v["features"], f);
    let (s, _) =
        e.t.post(
            "/api/ai/admin/suspend",
            &e.admin,
            json!({"userId": fresh.id}),
        )
        .await;
    assert_eq!(s, 200);
    let (s, v) = set_features(&e, &fresh, ALL_ON()).await;
    assert_eq!(s, 200, "a paused user's boxes stay editable: {v}");
}

#[tokio::test]
async fn a_paused_user_is_refused_as_paused_whatever_the_features_say() {
    let e = env().await;
    let (s, _) =
        e.t.post(
            "/api/ai/admin/suspend",
            &e.admin,
            json!({"userId": e.user.id}),
        )
        .await;
    assert_eq!(s, 200);
    set_features(&e, &e.user, only_off("chat")).await;
    let (st, v) =
        e.t.open_stream("/api/ai/send", &e.user, json!({"text": "x", "lang": "en"}))
            .await
            .err()
            .expect("refused");
    assert_eq!((st, &v["code"]), (403, &json!("ai.suspended")));
    let (st, v) =
        e.t.post(
            "/api/ai/gen/start",
            &e.user,
            json!({"source": "bank", "bankId": 3}),
        )
        .await;
    assert_eq!((st, &v["code"]), (403, &json!("ai.suspended")));
}

#[tokio::test]
async fn why_says_off_or_link_when_ai_cannot_be_used() {
    let e = env().await;
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(v["why"], Value::Null);
    // effective, but the main service has not accepted a report for a minute
    e.t.app
        .0
        .link
        .age_report_for_tests(std::time::Duration::from_secs(120));
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(
        (v["enabled"].clone(), v["why"].clone()),
        (json!(false), json!("link"))
    );
    assert!(
        v["features"].is_object() && v["gen"].is_object(),
        "features and gen survive a stale link"
    );
}

#[tokio::test]
async fn an_admin_switches_the_free_mode_of_administrator_accounts_only() {
    let e = env().await;
    let other = e.t.user("kai", true, true);
    // another administrator: on, shown in its row and in its own status, then off again
    let (s, v) =
        e.t.post(
            "/api/ai/admin/free",
            &e.admin,
            json!({"userId": other.id, "free": true}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["user"]["freeMode"], json!(true));
    let (_, st) = e.t.get("/api/ai/status", &other).await;
    assert_eq!(st["freeMode"], json!(true));
    let (_, list) = e.t.get("/api/ai/admin/users", &e.admin).await;
    let row = list["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == json!(other.id))
        .unwrap()
        .clone();
    assert_eq!(row["freeMode"], json!(true));
    let (s, v) =
        e.t.post(
            "/api/ai/admin/free",
            &e.admin,
            json!({"userId": other.id, "free": false}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["user"]["freeMode"], json!(false));
    // its own account too
    let (s, _) =
        e.t.post(
            "/api/ai/admin/free",
            &e.admin,
            json!({"userId": e.admin.id, "free": true}),
        )
        .await;
    assert_eq!(s, 200);
    // an ordinary account never gets free mode, and its row has no switch
    let (s, v) =
        e.t.post(
            "/api/ai/admin/free",
            &e.admin,
            json!({"userId": e.user.id, "free": true}),
        )
        .await;
    assert_eq!((s, v["code"].clone()), (409, json!("ai.freeAdminOnly")));
    let (_, list) = e.t.get("/api/ai/admin/users", &e.admin).await;
    let row = list["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == json!(e.user.id))
        .unwrap()
        .clone();
    assert!(row.get("freeMode").is_none());
    // a non-admin caller, an unknown id, extra keys
    let (s, _) =
        e.t.post(
            "/api/ai/admin/free",
            &e.user,
            json!({"userId": other.id, "free": true}),
        )
        .await;
    assert_eq!(s, 403);
    let (s, _) =
        e.t.post(
            "/api/ai/admin/free",
            &e.admin,
            json!({"userId": 9999, "free": true}),
        )
        .await;
    assert_eq!(s, 404);
    let (s, _) =
        e.t.post(
            "/api/ai/admin/free",
            &e.admin,
            json!({"userId": other.id, "free": true, "x": 1}),
        )
        .await;
    assert_eq!(s, 400);
}
