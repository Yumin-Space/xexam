//! Identity statements, the token, the access matrix, scoping, request rules and the validation of internal answers.
mod common;

use axum::http::Method;
use common::*;
use serde_json::{json, Value};

#[tokio::test]
async fn invalid_identities_answer_backend_down() {
    let t = start().await;
    let sam = t.user("sam", false, true);
    let now = xexam_ai_service::util::unix_now();
    let good = t.ident(&sam);
    let (p64, sig) = good.split_once('.').expect("dot");
    let payload = |f: &dyn Fn(&mut serde_json::Map<String, Value>)| {
        let mut m =
            json!({"v": 1, "id": sam.id, "stamp": sam.stamp, "username": "sam", "role": "user",
                            "aiAllowed": true, "sid": sam.sid, "exp": now + 60})
            .as_object()
            .cloned()
            .unwrap_or_default();
        f(&mut m);
        xexam_ai_service::identity::sign(&Value::Object(m).to_string(), TOKEN.as_bytes())
    };
    let cases: Vec<(&str, Option<String>)> = vec![
        ("missing", None),
        (
            "bad signature",
            Some(format!("{p64}.{}", sig.chars().rev().collect::<String>())),
        ),
        (
            "other token",
            Some(t.ident_with(&sam, now + 60, "another-token-another-token-0000000")),
        ),
        ("expired", Some(t.ident_with(&sam, now - 5, TOKEN))),
        ("too far ahead", Some(t.ident_with(&sam, now + 500, TOKEN))),
        (
            "unknown field",
            Some(payload(&|m| {
                m.insert("extra".into(), json!(1));
            })),
        ),
        (
            "wrong role",
            Some(payload(&|m| {
                m.insert("role".into(), json!("root"));
            })),
        ),
        (
            "bad stamp",
            Some(payload(&|m| {
                m.insert("stamp".into(), json!("XYZ"));
            })),
        ),
        (
            "bad sid",
            Some(payload(&|m| {
                m.insert("sid".into(), json!("1234"));
            })),
        ),
        (
            "string id",
            Some(payload(&|m| {
                m.insert("id".into(), json!("7"));
            })),
        ),
        ("two dots", Some(format!("{good}.x"))),
    ];
    for (what, ident) in cases {
        let r = t
            .raw(Method::GET, "/api/ai/status", ident, None, None)
            .await;
        let (s, v) = read_json(r).await;
        assert_eq!(s, 503, "{what}");
        assert_eq!(
            v,
            json!({"error": "AI 暂时无法使用，请稍后再试或联系管理员。", "code": "ai.backendDown"}),
            "{what}"
        );
    }
    assert!(captured_contains("refused a request without a valid identity (requests must come through Nginx; XEXAM_INTERNAL_TOKEN must be the same for both services)"));
    // at most one line a minute (eleven refusals above, one line)
    assert!(!t.app.0.identity_log.allow());
    let (s, _) = t.get("/api/ai/status", &sam).await;
    assert_eq!(s, 200);
    // the statement never reaches a log line
    assert!(!captured().iter().any(|l| l.contains(p64)));
}

#[tokio::test]
async fn a_rotated_token_file_is_read_again_once() {
    let t = start_with(Opts {
        token_env: false,
        ..Opts::default()
    })
    .await;
    let sam = t.user("sam", false, true);
    let (s, _) = t.get("/api/ai/status", &sam).await;
    assert_eq!(s, 200);
    // the backend got a new token (a new link volume): the next statement is signed with it
    let rotated = "rotated-internal-token-0123456789abcdef-xyz";
    std::fs::write(
        t.dir.path().join("link").join("internal.token"),
        format!("{rotated}\n"),
    )
    .expect("write");
    let ident = t.ident_with(&sam, xexam_ai_service::util::unix_now() + 60, rotated);
    let (s, v) = read_json(
        t.raw(Method::GET, "/api/ai/status", Some(ident), None, None)
            .await,
    )
    .await;
    assert_eq!(s, 200, "re-read once and accepted: {v}");
    // a third token within 5 s: no new re-read, refused
    let third = "third-internal-token-0123456789abcdef-xyzw";
    std::fs::write(t.dir.path().join("link").join("internal.token"), third).expect("write");
    let ident = t.ident_with(&sam, xexam_ai_service::util::unix_now() + 60, third);
    let (s, _) = read_json(
        t.raw(Method::GET, "/api/ai/status", Some(ident), None, None)
            .await,
    )
    .await;
    assert_eq!(s, 503);
}

fn link_test_config(
    backend_url: &str,
    dir: &std::path::Path,
    env_token: Option<&str>,
) -> xexam_ai_service::config::Config {
    xexam_ai_service::config::Config {
        data_dir: dir.join("ai"),
        host: "127.0.0.1".into(),
        port: 0,
        backend_url: backend_url.to_string(),
        link_dir: dir.join("link"),
        internal_token: env_token.map(|t| zeroize::Zeroizing::new(t.to_string())),
        secret_key: None,
        log_filter: None,
        journal: xexam_ai_service::config::JournalMode::Wal,
        version: "0.8".into(),
        timeouts: fast_timeouts(),
    }
}

/// Closes the AI database file of an `App` opened directly by a test before its `TempDir` is removed (on Windows an
/// open file would leave the directory behind).
fn close(app: &xexam_ai_service::app::AppState) {
    app.0.begin_shutdown();
    app.0.store.release_file();
}

#[tokio::test]
async fn a_too_short_env_token_turns_the_link_off_and_ignores_the_file() {
    init_logs();
    let backend = fake_backend().await;
    let dir = tempfile::tempdir().expect("dir");
    // A valid token file exists, but XEXAM_INTERNAL_TOKEN is set and too short: like the main backend, the link stays
    // off instead of falling back to the file
    std::fs::create_dir_all(dir.path().join("link")).expect("dir");
    std::fs::write(dir.path().join("link").join("internal.token"), TOKEN).expect("write");
    let cfg = link_test_config(&backend.url, dir.path(), Some("too-short"));
    let app = xexam_ai_service::app::App::open(cfg).expect("open");
    assert!(captured_contains(
        "XEXAM_INTERNAL_TOKEN is too short (at least 32 characters); AI link off"
    ));
    assert!(!app.0.link.load());
    assert!(!app.0.link.has_token());
    let router = xexam_ai_service::api::build_router(app.clone());
    use tower::ServiceExt;
    let req = axum::http::Request::builder()
        .uri("/api/ai/status")
        .header("x-xexam-identity", "x.y")
        .body(axum::body::Body::empty())
        .expect("req");
    let (s, v) = read_json(router.clone().oneshot(req).await.expect("resp")).await;
    assert_eq!((s, v["code"].clone()), (503, json!("ai.backendDown")));
    let req = axum::http::Request::builder()
        .uri("/healthz")
        .body(axum::body::Body::empty())
        .expect("req");
    let (s, _) = read_json(router.oneshot(req).await.expect("resp")).await;
    assert_eq!(s, 200);
    close(&app);
}

#[tokio::test]
async fn env_token_whitespace_and_reasons_follow_the_backend() {
    init_logs();
    let backend = fake_backend().await;
    let dir = tempfile::tempdir().expect("dir");
    std::fs::create_dir_all(dir.path().join("link")).expect("dir");
    std::fs::write(dir.path().join("link").join("internal.token"), TOKEN).expect("write");
    // A space inside (40 characters): the log line names the space, not the length, and the link is off
    let inner = format!("{} {}", "a".repeat(20), "b".repeat(19));
    let cfg = link_test_config(&backend.url, dir.path(), Some(&inner));
    let app = xexam_ai_service::app::App::open(cfg).expect("open");
    assert!(captured_contains(
        "XEXAM_INTERNAL_TOKEN may contain only printable ASCII characters without spaces; AI link off"
    ));
    assert!(!app.0.link.load());
    assert!(!app.0.link.has_token());
    close(&app);
    // Whitespace around the value is removed, as the backend does: the same token on both sides
    let dir = tempfile::tempdir().expect("dir");
    let cfg = link_test_config(&backend.url, dir.path(), Some(&format!(" {TOKEN}\r\n")));
    let app = xexam_ai_service::app::App::open(cfg).expect("open");
    assert!(app.0.link.from_env());
    assert_eq!(
        app.0.link.token().as_deref().map(String::as_str),
        Some(TOKEN)
    );
    close(&app);
}

#[tokio::test]
async fn a_missing_token_file_means_backend_down_until_it_appears() {
    init_logs();
    let backend = fake_backend().await;
    let dir = tempfile::tempdir().expect("dir");
    let cfg = link_test_config(&backend.url, dir.path(), None);
    let app = xexam_ai_service::app::App::open(cfg).expect("open");
    let router = xexam_ai_service::api::build_router(app.clone());
    use tower::ServiceExt;
    let req = axum::http::Request::builder()
        .uri("/api/ai/status")
        .header("x-xexam-identity", "x.y")
        .body(axum::body::Body::empty())
        .expect("req");
    let (s, v) = read_json(router.clone().oneshot(req).await.expect("resp")).await;
    assert_eq!((s, v["code"].clone()), (503, json!("ai.backendDown")));
    assert!(captured_contains(
        "AI link: waiting for the internal token at "
    ));
    // /healthz never depends on the backend or the token
    let req = axum::http::Request::builder()
        .uri("/healthz")
        .body(axum::body::Body::empty())
        .expect("req");
    let (s, v) = read_json(router.oneshot(req).await.expect("resp")).await;
    assert_eq!(s, 200);
    assert_eq!(v["ok"], json!(true));
    assert_eq!(v["version"], json!("0.8"));
    assert_eq!(v["arch"], json!(std::env::consts::ARCH));
    // the file appears: the link loads it
    std::fs::create_dir_all(dir.path().join("link")).expect("dir");
    std::fs::write(dir.path().join("link").join("internal.token"), TOKEN).expect("write");
    assert!(app.0.link.load());
    close(&app);
}

#[tokio::test]
async fn a_reused_user_id_loses_the_old_data_before_anything_is_served() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    let old = t.user_with_id(7, "olduser", false, true);
    t.send_all(&old, json!({"text": "old question"}))
        .await
        .expect("s");
    let (s, _) = t
        .post("/api/ai/memories/add", &old, json!({"content": "old note"}))
        .await;
    assert_eq!(s, 200);
    // the main database was restored: id 7 now belongs to another account (another stamp)
    let new = Who {
        stamp: "ffffffffffffffffffffffffffffffff".into(),
        username: "newuser".into(),
        ..old.clone()
    };
    t.backend.st().users.get_mut(&7).expect("u").stamp = new.stamp.clone();
    let (s, v) = t.get("/api/ai/chats", &new).await;
    assert_eq!((s, v), (200, json!({"chats": []})));
    let (_, v) = t.get("/api/ai/memories", &new).await;
    assert_eq!(v["memories"], json!([]));
    let db = t.db();
    let rows: i64 = db
        .query_row("SELECT (SELECT COUNT(*) FROM chats) + (SELECT COUNT(*) FROM memories) + (SELECT COUNT(*) FROM users WHERE id = 7)", [], |r| r.get(0))
        .expect("n");
    assert_eq!(rows, 0);
    let anon: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM usage WHERE kind = 'chat' AND user_id IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("n");
    assert_eq!(anon, 1, "usage rows stay, anonymised");
    assert!(captured_contains(
        "Removed the AI data of a user id that now belongs to another account (id 7)"
    ));
    // a renamed user keeps the data and the name is refreshed
    let sam = t.user("sam", false, true);
    t.send_all(&sam, json!({"text": "hi"})).await.expect("s");
    let renamed = Who {
        username: "samuel".into(),
        ..sam.clone()
    };
    let (_, v) = t.get("/api/ai/chats", &renamed).await;
    assert_eq!(v["chats"].as_array().map(Vec::len), Some(1));
    let name: String = t
        .db()
        .query_row("SELECT username FROM users WHERE id = ?1", [sam.id], |r| {
            r.get(0)
        })
        .expect("row");
    assert_eq!(name, "samuel");
}

#[tokio::test]
async fn access_matrix_and_scoping() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    let kim = t.user("kim", false, false);
    // AI not set up: reading works for everyone, writing is refused
    let (s, v) = t.get("/api/ai/status", &sam).await;
    assert_eq!(s, 200);
    let keys: Vec<&str> = v
        .as_object()
        .expect("obj")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec![
            "enabled",
            "allowed",
            "suspended",
            "hasData",
            "running",
            "retentionDays",
            "memoryLimit",
            "memoryMaxChars",
            "maxMessageChars",
            "messageChars",
            "memoryChars",
            "keyProblem",
            "configured",
            "freeMode"
        ]
    );
    assert_eq!(
        (
            v["enabled"].clone(),
            v["allowed"].clone(),
            v["hasData"].clone(),
            v["running"].clone()
        ),
        (json!(false), json!(false), json!(false), Value::Null)
    );
    for (path, body) in [
        ("/api/ai/send", json!({"text": "hi"})),
        ("/api/ai/retry", json!({"chatId": 1})),
        ("/api/ai/memories/add", json!({"content": "x"})),
        ("/api/ai/memories/update", json!({"id": 1, "content": "x"})),
        ("/api/ai/memories/suggest", json!({"messageId": 1})),
    ] {
        let (s, v) = t.post(path, &sam, body).await;
        assert_eq!(
            (s, v["code"].clone()),
            (403, json!("ai.disabled")),
            "{path}"
        );
    }
    let (_, v) = t.get("/api/ai/status", &admin).await;
    assert_eq!(
        (v["configured"].clone(), v["keyProblem"].clone()),
        (json!(false), json!(false))
    );
    t.configure_openai(&admin, "demo-chat-small").await;
    let (_, v) = t.get("/api/ai/status", &admin).await;
    assert_eq!(
        (v["configured"].clone(), v["allowed"].clone()),
        (json!(true), json!(true))
    );
    let (_, v) = t.get("/api/ai/status", &kim).await;
    assert_eq!(
        (
            v["enabled"].clone(),
            v["allowed"].clone(),
            v["configured"].clone()
        ),
        (json!(true), json!(false), json!(false))
    );
    // a user without AI+ cannot write
    for (path, body) in [
        ("/api/ai/send", json!({"text": "hi"})),
        ("/api/ai/memories/add", json!({"content": "x"})),
    ] {
        let (s, v) = t.post(path, &kim, body).await;
        assert_eq!(
            (s, v["code"].clone()),
            (403, json!("ai.notAllowed")),
            "{path}"
        );
    }
    // sam writes; kim (and the admin) never see sam's data through the user routes
    let ev = t.send_all(&sam, json!({"text": "mine"})).await.expect("s");
    let chat = ev[0].1["chatId"].as_i64().expect("id");
    let (_, mem) = t
        .post("/api/ai/memories/add", &sam, json!({"content": "my note"}))
        .await;
    let mem_id = mem["memory"]["id"].as_i64().expect("id");
    for who in [&kim, &admin] {
        let (s, missing) = t.get("/api/ai/chat?id=999999", who).await;
        let (s2, foreign) = t.get(&format!("/api/ai/chat?id={chat}"), who).await;
        assert_eq!((s, s2), (404, 404));
        assert_eq!(
            missing, foreign,
            "a foreign id answers exactly like a missing one"
        );
        let (_, v) = t.get("/api/ai/chats", who).await;
        assert_eq!(v["chats"], json!([]));
        let (_, v) = t.get("/api/ai/memories", who).await;
        assert_eq!(v["memories"], json!([]));
        let (s, _) = t
            .post("/api/ai/chats/delete", who, json!({"id": chat}))
            .await;
        assert_eq!(s, 404);
        let (s, _) = t
            .post("/api/ai/memories/delete", who, json!({"id": mem_id}))
            .await;
        assert_eq!(s, 404);
    }
    let (s, _) = t
        .post(
            "/api/ai/memories/update",
            &admin,
            json!({"id": mem_id, "content": "hijack"}),
        )
        .await;
    assert_eq!(s, 404);
    let (s, _) = t
        .post("/api/ai/retry", &admin, json!({"chatId": chat}))
        .await;
    assert_eq!(s, 404);
    // AI+ switched off for sam: reading and deleting still work
    t.set_ai(&sam, false);
    let sam_off = Who {
        ai: false,
        ..sam.clone()
    };
    let (_, v) = t.get("/api/ai/status", &sam_off).await;
    assert_eq!(
        (v["allowed"].clone(), v["hasData"].clone()),
        (json!(false), json!(true))
    );
    let (s, v) = t.get(&format!("/api/ai/chat?id={chat}"), &sam_off).await;
    assert_eq!(s, 200, "{v}");
    let (s, _) = t
        .post("/api/ai/memories/add", &sam_off, json!({"content": "x"}))
        .await;
    assert_eq!(s, 403);
    // AI off globally: the same
    let (s, _) = t
        .post("/api/ai/admin/enable", &admin, json!({"enabled": false}))
        .await;
    assert_eq!(s, 200);
    let (s, v) = t
        .post("/api/ai/memories/delete", &sam_off, json!({"all": true}))
        .await;
    assert_eq!((s, v), (200, json!({"deleted": 1})));
    let (s, v) = t
        .post("/api/ai/chats/delete", &sam_off, json!({"all": true}))
        .await;
    assert_eq!((s, v), (200, json!({"deleted": 1})));
    // admin routes: administrators only
    for (m, path) in [
        (Method::GET, "/api/ai/admin/settings"),
        (Method::POST, "/api/ai/admin/settings"),
        (Method::POST, "/api/ai/admin/models"),
        (Method::POST, "/api/ai/admin/verify"),
        (Method::POST, "/api/ai/admin/enable"),
        (Method::GET, "/api/ai/admin/usage"),
        (Method::GET, "/api/ai/admin/users"),
        (Method::GET, "/api/ai/admin/user?id=1"),
        (Method::GET, "/api/ai/admin/chat?user=1&id=1"),
        (Method::POST, "/api/ai/admin/clear"),
        (Method::GET, "/api/ai/admin/limits"),
        (Method::POST, "/api/ai/admin/limits"),
    ] {
        let post = m == Method::POST;
        let r = t
            .raw(
                m,
                path,
                Some(t.ident(&sam)),
                post.then(|| b"{}".to_vec()),
                post.then_some("application/json"),
            )
            .await;
        let (s, v) = read_json(r).await;
        assert_eq!(
            (s, v["code"].clone()),
            (403, json!("ai.adminOnly")),
            "{path}"
        );
    }
}

#[tokio::test]
async fn request_rules() {
    let t = start().await;
    let sam = t.user("sam", false, true);
    let id = Some(t.ident(&sam));
    let (s, v) = read_json(
        t.raw(Method::PUT, "/api/ai/status", id.clone(), None, None)
            .await,
    )
    .await;
    assert_eq!((s, v["code"].clone()), (405, json!("ai.badRequest")));
    let r = t
        .raw(Method::DELETE, "/api/ai/chats", id.clone(), None, None)
        .await;
    assert_eq!(
        r.headers().get("allow").and_then(|v| v.to_str().ok()),
        Some("GET, POST")
    );
    let r = t
        .raw(Method::HEAD, "/api/ai/status", id.clone(), None, None)
        .await;
    assert_eq!(r.status().as_u16(), 405);
    let (s, _) = read_json(
        t.raw(
            Method::POST,
            "/api/ai/status",
            id.clone(),
            Some(b"{}".to_vec()),
            Some("application/json"),
        )
        .await,
    )
    .await;
    assert_eq!(s, 405);
    let (s, v) = read_json(
        t.raw(Method::GET, "/api/ai/nothing", id.clone(), None, None)
            .await,
    )
    .await;
    assert_eq!((s, v["code"].clone()), (404, json!("ai.notFound")));
    let (s, _) = read_json(
        t.raw(Method::GET, "/internal/ai/log", id.clone(), None, None)
            .await,
    )
    .await;
    assert_eq!(s, 404);
    // POST needs JSON
    let (s, v) = read_json(
        t.raw(
            Method::POST,
            "/api/ai/stop",
            id.clone(),
            Some(b"{}".to_vec()),
            Some("text/plain"),
        )
        .await,
    )
    .await;
    assert_eq!((s, v["code"].clone()), (415, json!("ai.badRequest")));
    let (s, _) = read_json(
        t.raw(
            Method::POST,
            "/api/ai/stop",
            id.clone(),
            Some(b"{}".to_vec()),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(s, 415);
    let (s, v) = read_json(
        t.raw(
            Method::POST,
            "/api/ai/stop",
            id.clone(),
            Some(b"".to_vec()),
            Some("application/json; charset=utf-8"),
        )
        .await,
    )
    .await;
    assert_eq!(
        (s, v),
        (200, json!({"ok": true, "stopped": false})),
        "empty body = {{}}"
    );
    let (s, v) = read_json(
        t.raw(
            Method::POST,
            "/api/ai/stop",
            id.clone(),
            Some(b"[1]".to_vec()),
            Some("application/json"),
        )
        .await,
    )
    .await;
    assert_eq!(
        (s, v),
        (400, json!({"error": "请求无效。", "code": "ai.badRequest"}))
    );
    let (s, _) = read_json(
        t.raw(
            Method::POST,
            "/api/ai/stop",
            id.clone(),
            Some(b"{bad".to_vec()),
            Some("application/json"),
        )
        .await,
    )
    .await;
    assert_eq!(s, 400);
    let big = format!("{{\"text\":\"{}\"}}", "a".repeat(130 * 1024));
    let (s, v) = read_json(
        t.raw(
            Method::POST,
            "/api/ai/send",
            id.clone(),
            Some(big.into_bytes()),
            Some("application/json"),
        )
        .await,
    )
    .await;
    assert_eq!((s, v["code"].clone()), (413, json!("ai.badRequest")));
    // ids are positive integers
    for q in [
        "/api/ai/chat",
        "/api/ai/chat?id=",
        "/api/ai/chat?id=0",
        "/api/ai/chat?id=-3",
        "/api/ai/chat?id=abc",
        "/api/ai/chat?id=1&id=2",
    ] {
        let (s, v) = read_json(t.raw(Method::GET, q, id.clone(), None, None).await).await;
        assert_eq!((s, v["code"].clone()), (400, json!("ai.badRequest")), "{q}");
    }
    for body in [
        json!({"id": "1"}),
        json!({"id": 0}),
        json!({"id": 1.5}),
        json!({}),
        json!({"all": false}),
    ] {
        let (s, _) = t.post("/api/ai/chats/delete", &sam, body.clone()).await;
        assert_eq!(s, 400, "{body}");
    }
    // every JSON answer has the same headers
    let r = t
        .raw(Method::GET, "/api/ai/chats", id.clone(), None, None)
        .await;
    let h = r.headers();
    assert_eq!(
        h.get("content-type").and_then(|v| v.to_str().ok()),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(
        h.get("cache-control").and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(
        h.get("x-content-type-options")
            .and_then(|v| v.to_str().ok()),
        Some("nosniff")
    );
    // compact JSON
    let (_, v) = t.get("/api/ai/chats", &sam).await;
    assert_eq!(v.to_string(), "{\"chats\":[]}");
}

#[tokio::test]
async fn internal_answers_must_match_their_schema() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let (s, v) = t.get("/api/ai/admin/users", &admin).await;
    assert_eq!(s, 200, "{v}");
    assert!(captured_contains("AI link: connected to the main service"));
    for mode in [
        Mode::ExtraField,
        Mode::WrongType,
        Mode::Status500,
        Mode::NonJson,
        Mode::Hang,
    ] {
        t.backend.set_mode(mode);
        let (s, v) = t.get("/api/ai/admin/users", &admin).await;
        assert_eq!(
            (s, v["code"].clone()),
            (503, json!("ai.backendDown")),
            "{mode:?}"
        );
    }
    // a 404 means the token was refused: re-read once, reported once
    t.backend.set_mode(Mode::Reject);
    let (s, _) = t.get("/api/ai/admin/users", &admin).await;
    assert_eq!(s, 503);
    assert!(captured_contains("AI link: the main service refused the internal token (XEXAM_INTERNAL_TOKEN must be the same for both services)"));
    t.backend.set_mode(Mode::Normal);
    let (s, _) = t.get("/api/ai/admin/users", &admin).await;
    assert_eq!(s, 200);
}
