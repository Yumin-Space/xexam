//! Suspend / Resume of a user's AI+ function (round-2 contracts 3.2–3.5): the routes and their checks, the audit line
//! through the outbox, idempotency, the access rule for the user, the running answer stopped at once, the list of
//! users with the AI+ service, and a suspension that outlives switching the service off and on. All content is made up.
mod common;

use common::*;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const OFF_LINE: &str = "AI function turned off for user \"sam\" by admin \"admin\"";
const ON_LINE: &str = "AI function turned on for user \"sam\" by admin \"admin\"";

fn row_of<'a>(list: &'a Value, name: &str) -> Option<&'a Value> {
    list["users"]
        .as_array()
        .and_then(|a| a.iter().find(|r| r["username"] == json!(name)))
}

fn function_enabled(t: &TestApp, id: i64) -> Option<i64> {
    t.db()
        .query_row(
            "SELECT function_enabled FROM users WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .ok()
}

#[tokio::test]
async fn the_routes_check_caller_body_and_target() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let other_admin = t.user("root", true, false);
    let sam = t.user("sam", false, true);
    for path in ["/api/ai/admin/suspend", "/api/ai/admin/resume"] {
        // only administrators
        let (s, v) = t.post(path, &sam, json!({"userId": sam.id})).await;
        assert_eq!(
            (s, v),
            (
                403,
                json!({"error": "只有管理员可以这样做。", "code": "ai.adminOnly"})
            ),
            "{path}"
        );
        // only a positive integer userId
        for bad in [
            json!({}),
            json!({"userId": "1"}),
            json!({"userId": 0}),
            json!({"userId": -3}),
            json!({"userId": 1.5}),
            json!({"userId": null}),
            json!({"userId": sam.id, "extra": true}),
        ] {
            let (s, v) = t.post(path, &admin, bad.clone()).await;
            assert_eq!(
                (s, v["code"].clone()),
                (400, json!("ai.badRequest")),
                "{path} {bad}"
            );
        }
        // GET is not a route
        let (s, _) = t.get(path, &admin).await;
        assert_eq!(s, 405, "{path}");
        // a user the main app does not know
        let (s, v) = t.post(path, &admin, json!({"userId": 9999})).await;
        assert_eq!(
            (s, v["code"].clone()),
            (404, json!("ai.notFound")),
            "{path}"
        );
        // administrators always have AI
        for target in [&admin, &other_admin] {
            let (s, v) = t.post(path, &admin, json!({"userId": target.id})).await;
            assert_eq!(
                (s, v),
                (
                    409,
                    json!({"error": "管理员始终可以使用 AI，不能暂停。", "code": "ai.adminAlways"})
                ),
                "{path}"
            );
        }
        // the main backend cannot be asked
        t.backend.set_mode(Mode::Status500);
        let (s, v) = t.post(path, &admin, json!({"userId": sam.id})).await;
        assert_eq!(
            (s, v["code"].clone()),
            (503, json!("ai.backendDown")),
            "{path}"
        );
        t.backend.set_mode(Mode::Normal);
    }
    // nothing was written for an administrator, nothing was logged
    assert_eq!(function_enabled(&t, admin.id), None);
    assert!(!t.backend.log_contains("AI function turned"));
}

#[tokio::test]
async fn suspend_and_resume_are_audited_once_and_idempotent() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;

    let (s, v) = t
        .post("/api/ai/admin/suspend", &admin, json!({"userId": sam.id}))
        .await;
    assert_eq!(s, 200, "{v}");
    let keys: Vec<&str> = v["user"]
        .as_object()
        .expect("row")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec![
            "id",
            "username",
            "role",
            "ai",
            "aiAllowed",
            "disabled",
            "exists",
            "suspended",
            "chats",
            "messages",
            "memories",
            "lastUsedAt",
            "tokensIn",
            "tokensOut",
            "features"
        ]
    );
    assert_eq!(
        (
            v["user"]["id"].clone(),
            v["user"]["suspended"].clone(),
            v["user"]["exists"].clone()
        ),
        (json!(sam.id), json!(true), json!(true))
    );
    assert_eq!(function_enabled(&t, sam.id), Some(0));
    // the audit line reached the main System log through the outbox, and the local log with target audit
    assert_eq!(
        t.backend
            .logs()
            .iter()
            .filter(|l| l.as_str() == OFF_LINE)
            .count(),
        1
    );
    assert!(captured_contains(&format!("INFO audit: {OFF_LINE}")));
    let pending: i64 = t
        .db()
        .query_row("SELECT COUNT(*) FROM audit_outbox", [], |r| r.get(0))
        .expect("outbox");
    assert_eq!(pending, 0);

    // the same again: 200, no second line
    let (s, v) = t
        .post("/api/ai/admin/suspend", &admin, json!({"userId": sam.id}))
        .await;
    assert_eq!((s, v["user"]["suspended"].clone()), (200, json!(true)));
    assert_eq!(
        t.backend
            .logs()
            .iter()
            .filter(|l| l.as_str() == OFF_LINE)
            .count(),
        1
    );

    // resume, twice
    for _ in 0..2 {
        let (s, v) = t
            .post("/api/ai/admin/resume", &admin, json!({"userId": sam.id}))
            .await;
        assert_eq!((s, v["user"]["suspended"].clone()), (200, json!(false)));
    }
    assert_eq!(
        t.backend
            .logs()
            .iter()
            .filter(|l| l.as_str() == ON_LINE)
            .count(),
        1
    );
    assert_eq!(function_enabled(&t, sam.id), Some(1));

    // resuming a user that never was suspended writes no line
    let kim = t.user("kim", false, true);
    let (s, _) = t
        .post("/api/ai/admin/resume", &admin, json!({"userId": kim.id}))
        .await;
    assert_eq!(s, 200);
    assert!(!t
        .backend
        .log_contains("AI function turned on for user \"kim\""));
}

#[tokio::test]
async fn the_audit_line_is_kept_in_the_outbox_when_the_log_is_down() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.backend.st().log_fail = true;
    let (s, v) = t
        .post("/api/ai/admin/suspend", &admin, json!({"userId": sam.id}))
        .await;
    assert_eq!((s, v["user"]["suspended"].clone()), (200, json!(true)));
    // the flag and its line were written together
    let (flag, line): (i64, String) = t
        .db()
        .query_row(
            "SELECT (SELECT function_enabled FROM users WHERE id = ?1), (SELECT message FROM audit_outbox)",
            [sam.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row");
    assert_eq!((flag, line.as_str()), (0, OFF_LINE));
    assert!(!t.backend.log_contains(OFF_LINE));
    t.backend.st().log_fail = false;
    assert_eq!(t.app.0.deliver_outbox().await, 1);
    assert!(t.backend.log_contains(OFF_LINE));
}

#[tokio::test]
async fn a_suspended_user_can_read_and_delete_but_not_use_ai() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let ev = t
        .send_all(&sam, json!({"text": "a question"}))
        .await
        .expect("s");
    let chat = ev[0].1["chatId"].as_i64().expect("id");
    let user_msg = ev[0].1["userMessageId"].as_i64().expect("id");
    let (_, v) = t
        .post("/api/ai/memories/add", &sam, json!({"content": "a note"}))
        .await;
    let mem = v["memory"]["id"].as_i64().expect("id");
    let (s, _) = t
        .post("/api/ai/admin/suspend", &admin, json!({"userId": sam.id}))
        .await;
    assert_eq!(s, 200);

    let (_, v) = t.get("/api/ai/status", &sam).await;
    assert_eq!(
        (
            v["enabled"].clone(),
            v["allowed"].clone(),
            v["suspended"].clone()
        ),
        (json!(true), json!(false), json!(true))
    );
    for (path, body) in [
        ("/api/ai/send", json!({"text": "more"})),
        ("/api/ai/send", json!({"chatId": chat, "text": "more"})),
        ("/api/ai/retry", json!({"chatId": chat})),
        ("/api/ai/memories/add", json!({"content": "x"})),
        (
            "/api/ai/memories/update",
            json!({"id": mem, "content": "y"}),
        ),
        ("/api/ai/memories/suggest", json!({"messageId": user_msg})),
    ] {
        let (s, v) = t.post(path, &sam, body).await;
        assert_eq!(
            (s, v),
            (
                403,
                json!({"error": "管理员已暂停你的 AI+ 功能。", "code": "ai.suspended"})
            ),
            "{path}"
        );
    }
    // the order of the checks: a user without the AI+ service hears notAllowed first
    t.set_ai(&sam, false);
    let sam_off = Who {
        ai: false,
        ..sam.clone()
    };
    let (s, v) = t.post("/api/ai/send", &sam_off, json!({"text": "x"})).await;
    assert_eq!((s, v["code"].clone()), (403, json!("ai.notAllowed")));
    t.set_ai(&sam, true);
    // reading and deleting stay possible
    let (s, v) = t.get("/api/ai/chats", &sam).await;
    assert_eq!((s, v["chats"].as_array().map(Vec::len)), (200, Some(1)));
    let (s, v) = t.get(&format!("/api/ai/chat?id={chat}"), &sam).await;
    assert_eq!((s, v["messages"].as_array().map(Vec::len)), (200, Some(2)));
    let (s, v) = t.get("/api/ai/memories", &sam).await;
    assert_eq!((s, v["memories"].as_array().map(Vec::len)), (200, Some(1)));
    let (s, v) = t
        .post("/api/ai/memories/delete", &sam, json!({"id": mem}))
        .await;
    assert_eq!((s, v["deleted"].clone()), (200, json!(1)));
    let (s, v) = t
        .post("/api/ai/chats/delete", &sam, json!({"id": chat}))
        .await;
    assert_eq!((s, v["deleted"].clone()), (200, json!(1)));
    // an administrator is never affected
    let (_, v) = t.get("/api/ai/status", &admin).await;
    assert_eq!(
        (v["allowed"].clone(), v["suspended"].clone()),
        (json!(true), json!(false))
    );
    // resume: AI works again
    let (s, _) = t
        .post("/api/ai/admin/resume", &admin, json!({"userId": sam.id}))
        .await;
    assert_eq!(s, 200);
    let ev = t.send_all(&sam, json!({"text": "again"})).await.expect("s");
    assert_eq!(terminal(&ev).0, "done");
}

#[tokio::test]
async fn suspending_stops_a_running_answer_at_once() {
    let mut opts = Opts::default();
    // a long re-check interval: the stop must come from the route itself, not from the 2-s re-check
    opts.timeouts.access_interval = Duration::from_secs(20);
    let t = start_with(opts).await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-hold").await;
    let mut s = t
        .open_stream("/api/ai/send", &sam, json!({"text": "a long one"}))
        .await
        .expect("stream");
    let (_, start) = s.until("start").await;
    let answer = start["assistantMessageId"].as_i64().expect("id");
    s.until("delta").await;
    let started = Instant::now();
    let (st, _) = t
        .post("/api/ai/admin/suspend", &admin, json!({"userId": sam.id}))
        .await;
    assert_eq!(st, 200);
    let (name, data) = s.until("error").await;
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(name, "error");
    assert_eq!(
        data,
        json!({"status": "error", "code": "ai.suspended", "error": "管理员已暂停你的 AI+ 功能。"})
    );
    t.stub.release();
    let (status, code): (String, Option<String>) = t
        .db()
        .query_row(
            "SELECT status, error_code FROM messages WHERE id = ?1",
            [answer],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("answer");
    assert_eq!(
        (status.as_str(), code.as_deref()),
        ("error", Some("ai.suspended"))
    );
    let usage: String = t
        .db()
        .query_row(
            "SELECT status FROM usage WHERE kind = 'chat' ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("usage");
    assert_eq!(usage, "stopped");
    assert!(t.app.0.running_of(sam.id).is_none());
}

#[tokio::test]
async fn the_list_shows_users_with_the_service_and_keeps_suspensions() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    let kim = t.user("kim", false, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    let ev = t
        .send_all(&sam, json!({"text": "a question"}))
        .await
        .expect("s");
    assert_eq!(terminal(&ev).0, "done");
    let (s, _) = t
        .post("/api/ai/admin/suspend", &admin, json!({"userId": sam.id}))
        .await;
    assert_eq!(s, 200);

    // suspended users stay listed; users without the service are not listed
    let (s, v) = t.get("/api/ai/admin/users", &admin).await;
    assert_eq!(s, 200, "{v}");
    let names: Vec<&str> = v["users"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|r| r["username"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(names, vec!["admin", "sam"]);
    assert_eq!(
        row_of(&v, "sam").map(|r| r["suspended"].clone()),
        Some(json!(true))
    );
    assert_eq!(
        row_of(&v, "admin").map(|r| r["suspended"].clone()),
        Some(json!(false))
    );
    let (_, one) = t
        .get(&format!("/api/ai/admin/user?id={}", sam.id), &admin)
        .await;
    assert_eq!(one["user"]["suspended"], json!(true));

    // the AI+ service switched off in User management: no longer listed, the data stays and sam can still read it
    t.set_ai(&sam, false);
    let (_, v) = t.get("/api/ai/admin/users", &admin).await;
    assert!(row_of(&v, "sam").is_none(), "{v}");
    let sam_off = Who {
        ai: false,
        ..sam.clone()
    };
    let (s, v) = t.get("/api/ai/chats", &sam_off).await;
    assert_eq!((s, v["chats"].as_array().map(Vec::len)), (200, Some(1)));
    let (s, v) = t
        .get(&format!("/api/ai/admin/user?id={}", sam.id), &admin)
        .await;
    assert_eq!((s, v["chats"].as_array().map(Vec::len)), (200, Some(1)));
    // a suspended user's id stays in the reports while the service is off
    assert!(t.app.0.heartbeat_now().await);
    assert_eq!(t.backend.last_heartbeat()["suspended"], json!([sam.id]));

    // switched on again: listed again, still suspended (only Resume clears it)
    t.set_ai(&sam, true);
    let (_, v) = t.get("/api/ai/admin/users", &admin).await;
    assert_eq!(
        row_of(&v, "sam").map(|r| r["suspended"].clone()),
        Some(json!(true))
    );
    let (s, v) = t.post("/api/ai/send", &sam, json!({"text": "x"})).await;
    assert_eq!((s, v["code"].clone()), (403, json!("ai.suspended")));

    // a user without the service can be suspended too (not offered by the UI); still not listed
    let (s, v) = t
        .post("/api/ai/admin/suspend", &admin, json!({"userId": kim.id}))
        .await;
    assert_eq!((s, v["user"]["ai"].clone()), (200, json!(false)));
    let (_, v) = t.get("/api/ai/admin/users", &admin).await;
    assert!(row_of(&v, "kim").is_none());
}

#[tokio::test]
async fn an_ai_row_of_another_account_is_removed_before_the_flag_is_set() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    let old = t.user_with_id(7, "olduser", false, true);
    let ev = t
        .send_all(&old, json!({"text": "old question"}))
        .await
        .expect("s");
    assert_eq!(terminal(&ev).0, "done");
    // id 7 now belongs to another account (another stamp)
    t.backend.st().users.get_mut(&7).expect("u").stamp = "f".repeat(32);
    t.backend.st().users.get_mut(&7).expect("u").username = "newuser".into();
    let (s, v) = t
        .post("/api/ai/admin/suspend", &admin, json!({"userId": 7}))
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        (v["user"]["username"].clone(), v["user"]["chats"].clone()),
        (json!("newuser"), json!(0))
    );
    let (stamp, name, flag): (String, String, i64) = t
        .db()
        .query_row(
            "SELECT stamp, username, function_enabled FROM users WHERE id = 7",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("row");
    assert_eq!((stamp, name.as_str(), flag), ("f".repeat(32), "newuser", 0));
    assert!(t
        .backend
        .log_contains("AI function turned off for user \"newuser\" by admin \"admin\""));
}
