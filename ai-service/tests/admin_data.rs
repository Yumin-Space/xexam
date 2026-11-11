//! Administrators' views (audited), clear with the persisted outbox, retention, reconcile, crash fix-up, WAL checkpoint,
//! usage statistics.
mod common;

use common::*;
use serde_json::{json, Value};
use std::time::Duration;

#[tokio::test]
async fn users_with_ai_and_audited_views() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    let kim = t.user("kim", false, false);
    let _zed = t.user("zed", false, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    let ev = t
        .send_all(&sam, json!({"text": "a question"}))
        .await
        .expect("s");
    let chat = ev[0].1["chatId"].as_i64().expect("id");
    let (_, _) = t
        .post("/api/ai/memories/add", &sam, json!({"content": "a note"}))
        .await;

    let (s, v) = t.get("/api/ai/admin/users", &admin).await;
    assert_eq!(s, 200, "{v}");
    let rows = v["users"].as_array().expect("rows");
    let names: Vec<&str> = rows
        .iter()
        .map(|r| r["username"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(
        names,
        vec!["admin", "sam"],
        "users with the AI+ service (administrators included), by name"
    );
    let sam_row = &rows[1];
    let keys: Vec<&str> = sam_row
        .as_object()
        .expect("obj")
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
            "tokensOut"
        ]
    );
    assert_eq!(
        (
            sam_row["ai"].clone(),
            sam_row["chats"].clone(),
            sam_row["messages"].clone(),
            sam_row["memories"].clone()
        ),
        (json!(true), json!(1), json!(2), json!(1))
    );
    assert_eq!(
        (sam_row["tokensIn"].clone(), sam_row["tokensOut"].clone()),
        (json!(21), json!(13))
    );
    assert_eq!(rows[0]["role"], json!("admin"));

    // viewing a user: the audit line reaches the main System log before the data is returned
    let before = t.backend.logs().len();
    let (s, v) = t
        .get(&format!("/api/ai/admin/user?id={}", sam.id), &admin)
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["chats"].as_array().map(Vec::len), Some(1));
    assert_eq!(v["memories"][0]["content"], json!("a note"));
    assert!(t
        .backend
        .log_contains("user \"admin\" viewed the AI chats and memories of user \"sam\""));
    assert_eq!(t.backend.logs().len(), before + 1);
    let (s, v) = t
        .get(
            &format!("/api/ai/admin/chat?user={}&id={chat}", sam.id),
            &admin,
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["messages"].as_array().map(Vec::len), Some(2));
    assert!(t
        .backend
        .log_contains("user \"admin\" viewed an AI chat of user \"sam\" (2 messages)"));
    // a chat named with the wrong owner: 404 and no audit line
    let n = t.backend.logs().len();
    let (s, _) = t
        .get(
            &format!("/api/ai/admin/chat?user={}&id={chat}", kim.id),
            &admin,
        )
        .await;
    assert_eq!(s, 404);
    assert_eq!(t.backend.logs().len(), n);
    // the log call fails: no data
    t.backend.st().log_fail = true;
    let (s, v) = t
        .get(&format!("/api/ai/admin/user?id={}", sam.id), &admin)
        .await;
    assert_eq!(
        (s, v),
        (
            503,
            json!({"error": "AI 暂时无法使用，请稍后再试或联系管理员。", "code": "ai.backendDown"})
        )
    );
    let (s, v) = t
        .get(
            &format!("/api/ai/admin/chat?user={}&id={chat}", sam.id),
            &admin,
        )
        .await;
    assert_eq!((s, v["code"].clone()), (503, json!("ai.backendDown")));
    t.backend.st().log_fail = false;
    // a user that does not exist in the main app
    let (s, _) = t.get("/api/ai/admin/user?id=9999", &admin).await;
    assert_eq!(s, 404);
    // a user without data and without AI+ can still be looked at
    let (s, v) = t
        .get(&format!("/api/ai/admin/user?id={}", kim.id), &admin)
        .await;
    assert_eq!((s, v["chats"].clone()), (200, json!([])));
}

#[tokio::test]
async fn clear_writes_its_audit_line_through_the_outbox() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let mut chats = Vec::new();
    for i in 0..3 {
        let ev = t
            .send_all(&sam, json!({"text": format!("q{i}")}))
            .await
            .expect("s");
        chats.push(ev[0].1["chatId"].as_i64().expect("id"));
    }
    t.post("/api/ai/memories/add", &sam, json!({"content": "n1"}))
        .await;
    t.post("/api/ai/memories/add", &sam, json!({"content": "n2"}))
        .await;
    // shape rules
    for body in [
        json!({"userId": sam.id, "what": "chat"}),
        json!({"userId": sam.id, "what": "chats", "chatId": chats[0]}),
        json!({"userId": sam.id, "what": "memories", "chatId": chats[0]}),
        json!({"userId": sam.id, "what": "everything"}),
        json!({"what": "chats"}),
    ] {
        let (s, _) = t.post("/api/ai/admin/clear", &admin, body.clone()).await;
        assert_eq!(s, 400, "{body}");
    }
    let other = t.user("kim", false, true);
    let (s, _) = t
        .post(
            "/api/ai/admin/clear",
            &admin,
            json!({"userId": other.id, "what": "chat", "chatId": chats[0]}),
        )
        .await;
    assert_eq!(s, 404, "a chat of another owner");
    // one chat, delivered at once
    let (s, v) = t
        .post(
            "/api/ai/admin/clear",
            &admin,
            json!({"userId": sam.id, "what": "chat", "chatId": chats[0]}),
        )
        .await;
    assert_eq!((s, v), (200, json!({"deleted": 1})));
    assert!(t
        .backend
        .log_contains("user \"admin\" cleared one AI chat of user \"sam\""));
    let pending: i64 = t
        .db()
        .query_row("SELECT COUNT(*) FROM audit_outbox", [], |r| r.get(0))
        .expect("n");
    assert_eq!(pending, 0);
    // the main service cannot take the line: the clear still succeeds, the line waits in the outbox
    t.backend.st().log_fail = true;
    let (s, v) = t
        .post(
            "/api/ai/admin/clear",
            &admin,
            json!({"userId": sam.id, "what": "chats"}),
        )
        .await;
    assert_eq!((s, v), (200, json!({"deleted": 2})));
    let (s, v) = t
        .post(
            "/api/ai/admin/clear",
            &admin,
            json!({"userId": sam.id, "what": "memories"}),
        )
        .await;
    assert_eq!((s, v), (200, json!({"deleted": 2})));
    let queued: Vec<String> = {
        let db = t.db();
        let mut st = db
            .prepare("SELECT message FROM audit_outbox ORDER BY id")
            .expect("prep");
        let rows = st.query_map([], |r| r.get(0)).expect("q");
        rows.collect::<Result<Vec<String>, _>>().expect("rows")
    };
    assert_eq!(
        queued,
        vec![
            "user \"admin\" cleared all 2 AI chats of user \"sam\"".to_string(),
            "user \"admin\" cleared all 2 AI memories of user \"sam\"".to_string()
        ]
    );
    assert!(!t.backend.log_contains("cleared all 2 AI chats"));
    t.backend.st().log_fail = false;
    assert_eq!(t.app.0.deliver_outbox().await, 2);
    let logs = t.backend.logs();
    let a = logs
        .iter()
        .position(|l| l.contains("cleared all 2 AI chats"))
        .expect("chats line");
    let b = logs
        .iter()
        .position(|l| l.contains("cleared all 2 AI memories"))
        .expect("memories line");
    assert!(a < b, "oldest first");
    let pending: i64 = t
        .db()
        .query_row("SELECT COUNT(*) FROM audit_outbox", [], |r| r.get(0))
        .expect("n");
    assert_eq!(pending, 0);
    // two deliveries at once (an admin route and the background loop) never send the same line twice
    for m in ["queued demo line one", "queued demo line two"] {
        t.db()
            .execute("INSERT INTO audit_outbox (at, message) VALUES (1, ?1)", [m])
            .expect("queue");
    }
    let (x, y) = tokio::join!(t.app.0.deliver_outbox(), t.app.0.deliver_outbox());
    assert_eq!(x + y, 2);
    for m in ["queued demo line one", "queued demo line two"] {
        assert_eq!(
            t.backend.logs().iter().filter(|l| l.as_str() == m).count(),
            1,
            "{m}"
        );
    }
    let (_, v) = t.get("/api/ai/chats", &sam).await;
    assert_eq!(v["chats"], json!([]));
}

#[tokio::test]
async fn clearing_waits_for_a_running_answer() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-hold").await;
    let mut s = t
        .open_stream("/api/ai/send", &sam, json!({"text": "x"}))
        .await
        .expect("s");
    s.until("delta").await;
    let (code, v) = t
        .post(
            "/api/ai/admin/clear",
            &admin,
            json!({"userId": sam.id, "what": "chats"}),
        )
        .await;
    assert_eq!((code, v), (200, json!({"deleted": 1})));
    assert_eq!(s.until("error").await.1["code"], json!("ai.notFound"));
    let msgs: i64 = t
        .db()
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .expect("n");
    assert_eq!(msgs, 0);
    t.stub.release();
}

#[tokio::test]
async fn retention_sweep() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-hold").await;
    let (s, _) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"retentionDays": 2}),
        )
        .await;
    assert_eq!(s, 200);
    let (_, st) = t.get("/api/ai/status", &sam).await;
    assert_eq!(st["retentionDays"], json!(2));
    // two chats; one becomes old; a third, also old, is running
    t.post(
        "/api/ai/memories/add",
        &sam,
        json!({"content": "kept forever"}),
    )
    .await;
    let mut run = t
        .open_stream("/api/ai/send", &sam, json!({"text": "running"}))
        .await
        .expect("s");
    let (_, start) = run.until("start").await;
    run.until("delta").await;
    let running = start["chatId"].as_i64().expect("id");
    let db = t.db();
    let old = xexam_ai_service::util::unix_now() - 3 * 86_400;
    db.execute(
        "INSERT INTO chats (user_id, title, created_at, updated_at) VALUES (?1, 'old', ?2, ?2)",
        rusqlite::params![sam.id, old],
    )
    .expect("insert");
    let old_chat = db.last_insert_rowid();
    db.execute("INSERT INTO messages (chat_id, role, content, created_at) VALUES (?1, 'user', 'old text', ?2)", rusqlite::params![old_chat, old])
        .expect("insert");
    db.execute(
        "UPDATE chats SET updated_at = ?2 WHERE id = ?1",
        rusqlite::params![running, old],
    )
    .expect("age");
    db.execute(
        "INSERT INTO tasks (kind, state, run_after, created_at, updated_at) VALUES ('demo', 'done', 0, ?1, ?1), ('demo', 'queued', 0, ?1, ?1)",
        [old - 40 * 86_400],
    )
    .expect("tasks");
    let signal = t.app.0.store.checkpoint_signal();
    let notified = signal.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();
    assert_eq!(xexam_ai_service::retention::sweep_once(&t.app), 1);
    assert!(captured_contains(
        "AI retention: deleted 1 chats older than 2 days"
    ));
    tokio::time::timeout(Duration::from_secs(1), notified)
        .await
        .expect("checkpoint scheduled");
    let left: Vec<i64> = {
        let mut st = db
            .prepare("SELECT id FROM chats ORDER BY id")
            .expect("prep");
        let rows = st.query_map([], |r| r.get(0)).expect("q");
        rows.collect::<Result<Vec<i64>, _>>().expect("rows")
    };
    assert_eq!(left, vec![running], "the running chat is kept");
    let orphan: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE chat_id = ?1",
            [old_chat],
            |r| r.get(0),
        )
        .expect("n");
    assert_eq!(orphan, 0, "messages cascade");
    let tasks: Vec<String> = {
        let mut st = db.prepare("SELECT state FROM tasks").expect("prep");
        let rows = st.query_map([], |r| r.get(0)).expect("q");
        rows.collect::<Result<Vec<String>, _>>().expect("rows")
    };
    assert_eq!(tasks, vec!["queued".to_string()]);
    let mems: i64 = db
        .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
        .expect("n");
    assert_eq!(mems, 1, "memories are not subject to retention");
    t.stub.release();
    run.until("done").await;
}

#[tokio::test]
async fn reconcile_removes_deleted_users_and_anonymises_usage() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    let kim = t.user("kim", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    t.send_all(&sam, json!({"text": "q"})).await.expect("s");
    t.send_all(&kim, json!({"text": "q"})).await.expect("s");
    t.post("/api/ai/memories/add", &sam, json!({"content": "note"}))
        .await;
    t.delete_backend_user(&sam);
    // kim's id now belongs to someone else (a restore)
    t.backend.st().users.get_mut(&kim.id).expect("kim").stamp =
        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into();
    let answer = xexam_ai_service::retention::reconcile_once(&t.app, false, &[])
        .await
        .expect("reconcile");
    assert!(answer.iter().all(|u| u.id != sam.id));
    assert!(captured_contains("Removed the AI data of 1 deleted users"));
    assert!(captured_contains(&format!(
        "Removed the AI data of a user id that now belongs to another account (id {})",
        kim.id
    )));
    let db = t.db();
    let left: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM users WHERE id IN (?1, ?2)",
            [sam.id, kim.id],
            |r| r.get(0),
        )
        .expect("n");
    assert_eq!(left, 0);
    let content: i64 = db
        .query_row("SELECT (SELECT COUNT(*) FROM chats) + (SELECT COUNT(*) FROM messages) + (SELECT COUNT(*) FROM memories)", [], |r| r.get(0))
        .expect("n");
    assert_eq!(content, 0);
    let anon: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM usage WHERE kind = 'chat' AND user_id IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("n");
    assert_eq!(anon, 2);
    let (_, u) = t.get("/api/ai/admin/usage?days=7", &admin).await;
    let deleted = u["byUser"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["userId"].is_null())
        .cloned()
        .expect("deleted row");
    assert_eq!(
        (deleted["username"].clone(), deleted["calls"].clone()),
        (Value::Null, json!(2))
    );
    // the backend unreachable: skipped, nothing removed
    t.backend.set_mode(Mode::Status500);
    assert!(
        xexam_ai_service::retention::reconcile_once(&t.app, false, &[])
            .await
            .is_err()
    );
    t.backend.set_mode(Mode::Normal);
    // the admin list reconciles first
    let lee = t.user("lee", false, true);
    t.send_all(&lee, json!({"text": "q"})).await.expect("s");
    t.delete_backend_user(&lee);
    let (_, v) = t.get("/api/ai/admin/users", &admin).await;
    assert!(v["users"]
        .as_array()
        .expect("rows")
        .iter()
        .all(|r| r["username"] != json!("lee")));
}

#[tokio::test]
async fn a_user_deleted_during_an_answer_is_awaited() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-hold").await;
    let mut s = t
        .open_stream("/api/ai/send", &sam, json!({"text": "x"}))
        .await
        .expect("s");
    s.until("delta").await;
    // the re-check would also stop it; the reconcile must not wait for that, it stops it itself
    t.delete_backend_user(&sam);
    xexam_ai_service::retention::reconcile_once(&t.app, false, &[])
        .await
        .expect("reconcile");
    let code = s.until("error").await.1["code"].clone();
    assert!(code == json!("ai.notAllowed"), "{code}");
    let db = t.db();
    let n: i64 = db
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .expect("n");
    assert_eq!(n, 0);
    let usage: i64 = db
        .query_row("SELECT COUNT(*) FROM usage WHERE kind = 'chat'", [], |r| {
            r.get(0)
        })
        .expect("n");
    assert_eq!(usage, 1, "exactly one usage row, anonymised");
    t.stub.release();
}

#[tokio::test]
async fn crash_fixup_of_streaming_rows() {
    let mut t = start().await;
    let sam = t.user("sam", false, true);
    let db = t.db();
    db.execute("INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at) VALUES (?1, ?2, 'sam', 1, 1)", rusqlite::params![sam.id, sam.stamp])
        .expect("user");
    db.execute(
        "INSERT INTO chats (user_id, title, created_at, updated_at) VALUES (?1, 't', 1, 1)",
        [sam.id],
    )
    .expect("chat");
    let chat = db.last_insert_rowid();
    db.execute("INSERT INTO messages (chat_id, role, content, status, created_at) VALUES (?1, 'assistant', 'half', 'streaming', 1)", [chat])
        .expect("msg");
    drop(db);
    t.restart(Opts::default()).await;
    let (_, v) = t.get(&format!("/api/ai/chat?id={chat}"), &sam).await;
    assert_eq!(v["messages"][0]["status"], json!("error"));
    assert_eq!(v["messages"][0]["error"], json!("ai.interrupted"));
    assert_eq!(v["messages"][0]["content"], json!("half"));
    // a clean database is not written again on the next open
    let before = std::fs::metadata(t.db_path())
        .expect("meta")
        .modified()
        .expect("mtime");
    std::thread::sleep(Duration::from_millis(50));
    t.restart(Opts::default()).await;
    let after = std::fs::metadata(t.db_path())
        .expect("meta")
        .modified()
        .expect("mtime");
    assert_eq!(before, after);
}

#[tokio::test]
async fn deletes_schedule_a_checkpoint_that_empties_the_wal() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let ev = t
        .send_all(&sam, json!({"text": "to be deleted"}))
        .await
        .expect("s");
    let chat = ev[0].1["chatId"].as_i64().expect("id");
    let signal = t.app.0.store.checkpoint_signal();
    let notified = signal.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();
    let (s, _) = t
        .post("/api/ai/chats/delete", &sam, json!({"id": chat}))
        .await;
    assert_eq!(s, 200);
    tokio::time::timeout(Duration::from_secs(1), notified)
        .await
        .expect("scheduled");
    let wal = t.db_path().with_extension("db-wal");
    assert!(std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0) > 0);
    let runs = t.app.0.store.checkpoint_runs();
    assert!(!t.app.0.store.checkpoint_now().expect("checkpoint"));
    assert_eq!(t.app.0.store.checkpoint_runs(), runs + 1);
    assert_eq!(std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0), 0);
    // the background task coalesces: many deletes, one run within the 10-s window
    let tasks = xexam_ai_service::retention::spawn_all(&t.app);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let base = t.app.0.store.checkpoint_runs();
    for _ in 0..5 {
        t.app.0.store.schedule_checkpoint();
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(t.app.0.store.checkpoint_runs(), base + 1);
    for h in tasks {
        h.abort();
    }
}

#[tokio::test]
async fn usage_statistics() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    t.send_all(&sam, json!({"text": "q1"})).await.expect("s");
    t.send_all(&sam, json!({"text": "q2"})).await.expect("s");
    t.stub.force(Some((500, "{}")));
    t.send_all(&sam, json!({"text": "q3"})).await.expect("s");
    t.stub.force(None);
    let (s, v) = t.get("/api/ai/admin/usage?days=7", &admin).await;
    assert_eq!(s, 200, "{v}");
    let keys: Vec<&str> = v
        .as_object()
        .expect("obj")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec!["days", "from", "to", "unit", "totals", "byUser", "byModel", "byPeriod"]
    );
    assert_eq!(
        (v["days"].clone(), v["unit"].clone()),
        (json!(7), json!("day"))
    );
    assert_eq!(v["byPeriod"].as_array().map(Vec::len), Some(7));
    assert_eq!(v["byPeriod"][6]["period"], v["to"]);
    assert_eq!(v["byPeriod"][0]["period"], v["from"]);
    // 1 verify (admin) + 3 chats (sam)
    assert_eq!(
        v["totals"],
        json!({"calls": 4, "tokensIn": 21 * 3, "tokensOut": 13 * 3, "errors": 1, "tokensCached": 0})
    );
    assert_eq!(v["byUser"][0]["username"], json!("sam"));
    assert_eq!(v["byUser"][0]["calls"], json!(3));
    assert_eq!(v["byUser"][0]["errors"], json!(1));
    assert!(v["byUser"][0]["lastAt"].is_i64());
    assert_eq!(v["byModel"][0]["model"], json!("demo-chat-small"));
    assert_eq!(v["byPeriod"][6]["calls"], json!(4));
    let (_, d) = t.get("/api/ai/admin/usage", &admin).await;
    assert_eq!(
        (d["days"].clone(), d["byPeriod"].as_array().map(Vec::len)),
        (json!(7), Some(7))
    );
    let (_, m) = t.get("/api/ai/admin/usage?days=365", &admin).await;
    assert_eq!(m["unit"], json!("month"));
    let periods = m["byPeriod"].as_array().expect("p");
    assert!((12..=13).contains(&periods.len()));
    assert_eq!(periods.last().map(|p| p["calls"].clone()), Some(json!(4)));
    assert_eq!(periods[0]["period"].as_str().map(str::len), Some(7));
    let (_, m) = t.get("/api/ai/admin/usage?days=90", &admin).await;
    assert_eq!(m["unit"], json!("month"));
    for bad in ["5", "abc", "30&days=7"] {
        let (s, _) = t
            .get(&format!("/api/ai/admin/usage?days={bad}"), &admin)
            .await;
        assert_eq!(s, 400, "{bad}");
    }
    // A model listing uses no model: it counts under '' even while a model is saved
    let (s, l) = t.post("/api/ai/admin/models", &admin, json!({})).await;
    assert_eq!(s, 200, "{l}");
    let (_, v) = t.get("/api/ai/admin/usage?days=7", &admin).await;
    let rows = v["byModel"].as_array().expect("byModel");
    let calls = |m: &str| {
        rows.iter()
            .find(|r| r["model"] == json!(m))
            .map(|r| r["calls"].clone())
    };
    assert_eq!(
        (calls("demo-chat-small"), calls("")),
        (Some(json!(4)), Some(json!(1)))
    );
}
