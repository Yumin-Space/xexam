//! State reports to the main backend (round-2 contracts 2.2, 2.6, 2.7): the start report, a report right after every
//! state change, the payload, one report at a time, the retry after a failure, the stopping report, and what a stale
//! link changes (Status, the AiUser routes; reading and deleting still work).
mod common;

use common::*;
use serde_json::json;
use std::time::{Duration, Instant};

fn count(t: &TestApp) -> usize {
    t.backend.heartbeats().len()
}

#[tokio::test]
async fn the_start_report_is_compact_and_has_the_four_keys_in_order() {
    let t = start().await;
    // TestApp sends the start report right after App::open, as the heartbeat task does once the token is there
    let raw = t.backend.st().heartbeat_raw.clone();
    assert_eq!(
        raw,
        vec![r#"{"version":"0.8","enabled":false,"suspended":[],"stopping":false}"#.to_string()]
    );
    assert!(t.app.0.link_fresh());
    // no log line for heartbeats (only the "connected" line of the link, once)
    assert_eq!(captured_count("heartbeat"), 0);
}

#[tokio::test]
async fn every_state_change_is_reported_before_the_answer() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    let (s, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"provider": "openai", "baseUrl": format!("{}/v1", t.stub.url), "apiKey": KEY, "model": "demo-chat-small"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let (s, v) = t.post("/api/ai/admin/verify", &admin, json!({})).await;
    assert_eq!((s, v["ok"].clone()), (200, json!(true)), "{v}");

    // enable: reported at once with the new state
    let n = count(&t);
    let (s, v) = t
        .post("/api/ai/admin/enable", &admin, json!({"enabled": true}))
        .await;
    assert_eq!((s, v["enabled"].clone()), (200, json!(true)), "{v}");
    assert_eq!(count(&t), n + 1);
    assert_eq!(t.backend.last_heartbeat()["enabled"], json!(true));

    // disable, then enable again
    let (s, _) = t
        .post("/api/ai/admin/enable", &admin, json!({"enabled": false}))
        .await;
    assert_eq!(s, 200);
    assert_eq!(count(&t), n + 2);
    assert_eq!(t.backend.last_heartbeat()["enabled"], json!(false));
    t.verify_and_enable(&admin).await;
    assert_eq!(t.backend.last_heartbeat()["enabled"], json!(true));

    // a settings change that switches AI off (a new test is needed)
    let n = count(&t);
    let (s, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"model": "demo-chat-large"}),
        )
        .await;
    assert_eq!((s, v["switchedOff"].clone()), (200, json!(true)), "{v}");
    assert_eq!(count(&t), n + 1);
    assert_eq!(t.backend.last_heartbeat()["enabled"], json!(false));
    // a settings change that does not switch anything off sends no report
    let n = count(&t);
    let (s, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"retentionDays": 30}),
        )
        .await;
    assert_eq!((s, v["switchedOff"].clone()), (200, json!(false)), "{v}");
    assert_eq!(count(&t), n);

    // a failed test that switches AI off (the provider refuses the key)
    t.verify_and_enable(&admin).await;
    assert_eq!(t.backend.last_heartbeat()["enabled"], json!(true));
    t.stub.force(Some((
        401,
        r#"{"error":{"message":"bad key","type":"invalid_request_error","code":"invalid_api_key"}}"#,
    )));
    let n = count(&t);
    let (s, v) = t.post("/api/ai/admin/verify", &admin, json!({})).await;
    assert_eq!((s, v["switchedOff"].clone()), (200, json!(true)), "{v}");
    assert_eq!(count(&t), n + 1);
    assert_eq!(t.backend.last_heartbeat()["enabled"], json!(false));
    t.stub.force(None);

    // suspend and resume carry the user's id
    t.verify_and_enable(&admin).await;
    let n = count(&t);
    let (s, _) = t
        .post("/api/ai/admin/suspend", &admin, json!({"userId": sam.id}))
        .await;
    assert_eq!(s, 200);
    assert_eq!(count(&t), n + 1);
    assert_eq!(
        t.backend.last_heartbeat(),
        json!({"version": "0.8", "enabled": true, "suspended": [sam.id], "stopping": false})
    );
    let (s, _) = t
        .post("/api/ai/admin/resume", &admin, json!({"userId": sam.id}))
        .await;
    assert_eq!(s, 200);
    assert_eq!(count(&t), n + 2);
    assert_eq!(t.backend.last_heartbeat()["suspended"], json!([]));
}

#[tokio::test]
async fn suspended_ids_are_sorted_and_admins_are_never_listed() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let b = t.user_with_id(40, "bea", false, true);
    let a = t.user_with_id(12, "al", false, false);
    for who in [&b, &a] {
        let (s, v) = t
            .post("/api/ai/admin/suspend", &admin, json!({"userId": who.id}))
            .await;
        assert_eq!(s, 200, "{v}");
    }
    let (s, _) = t
        .post("/api/ai/admin/suspend", &admin, json!({"userId": admin.id}))
        .await;
    assert_eq!(s, 409);
    assert!(t.app.0.heartbeat_now().await);
    assert_eq!(t.backend.last_heartbeat()["suspended"], json!([12, 40]));
}

#[tokio::test]
async fn one_report_at_a_time_and_the_newest_state_last() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    t.backend.st().heartbeat_delay_ms = 300;
    t.backend.st().heartbeat_max_in_flight = 0;
    let n = count(&t);
    let first = {
        let app = t.app.clone();
        tokio::spawn(async move { app.0.heartbeat_now().await })
    };
    // the first report is on its way (enabled: true) when AI is switched off
    eventually(|| t.backend.st().heartbeat_in_flight == 1).await;
    let (s, _) = t
        .post("/api/ai/admin/enable", &admin, json!({"enabled": false}))
        .await;
    assert_eq!(s, 200);
    let more: Vec<_> = (0..3)
        .map(|_| {
            let app = t.app.clone();
            tokio::spawn(async move { app.0.heartbeat_now().await })
        })
        .collect();
    assert!(first.await.expect("join"));
    for h in more {
        assert!(h.await.expect("join"));
    }
    let beats = t.backend.heartbeats();
    assert_eq!(beats.len(), n + 5);
    assert_eq!(
        t.backend.st().heartbeat_max_in_flight,
        1,
        "never two at once"
    );
    assert_eq!(beats[n]["enabled"], json!(true));
    for b in &beats[n + 1..] {
        assert_eq!(
            b["enabled"],
            json!(false),
            "an older state never comes last"
        );
    }
}

#[tokio::test]
async fn refused_or_odd_answers_are_failures() {
    let t = start().await;
    for (mode, why) in [
        (Mode::Reject, "404: the token was refused"),
        (Mode::Status500, "500"),
        (Mode::NonJson, "not JSON"),
        (Mode::ExtraField, "an extra key"),
        (Mode::WrongType, "ok is not a boolean"),
    ] {
        t.backend.set_mode(mode);
        assert!(!t.app.0.heartbeat_now().await, "{why}");
    }
    t.backend.set_mode(Mode::Normal);
    t.backend.st().heartbeat_fail = true;
    assert!(!t.app.0.heartbeat_now().await);
    t.backend.st().heartbeat_fail = false;
    assert!(t.app.0.heartbeat_now().await);
    // a backend that does not answer: bounded by backend_total
    t.backend.set_mode(Mode::Hang);
    let started = Instant::now();
    assert!(!t.app.0.heartbeat_now().await);
    assert!(started.elapsed() < Duration::from_secs(5));
    t.backend.set_mode(Mode::Normal);
}

#[tokio::test]
async fn the_task_retries_after_a_failure_and_reports_at_once_when_notified() {
    let t = start().await;
    t.backend.st().heartbeat_fail = true;
    let attempts = |t: &TestApp| t.backend.st().heartbeat_raw.len();
    let before = attempts(&t);
    let task = tokio::spawn(xexam_ai_service::retention::heartbeat_loop(
        t.app.clone(),
        t.app.0.shutdown.clone(),
    ));
    // the first attempt at once, then again after heartbeat_retry (300 ms in the tests)
    eventually(|| attempts(&t) >= before + 3).await;
    let accepted = count(&t);
    t.backend.st().heartbeat_fail = false;
    eventually(|| count(&t) == accepted + 1).await;
    // after an accepted report the next one waits heartbeat_every (25 s): nothing more for a while
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert_eq!(count(&t), accepted + 1);
    // a state change notified without waiting (a key problem found while running) is reported at once
    t.app.0.notify_heartbeat();
    eventually(|| count(&t) == accepted + 2).await;
    // shutdown ends the task
    t.app.0.begin_shutdown();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the task ends on shutdown")
        .expect("join");
}

#[tokio::test]
async fn the_stopping_report_is_the_last_one() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    t.app.0.begin_shutdown();
    t.app.0.heartbeat_stopping().await;
    assert_eq!(
        t.backend.last_heartbeat(),
        json!({"version": "0.8", "enabled": true, "suspended": [], "stopping": true})
    );
    // nothing is sent after it
    let n = count(&t);
    assert!(!t.app.0.heartbeat_now().await);
    assert_eq!(count(&t), n);
    // a backend that does not answer: the stopping report gives up after 2 s
    t.backend.set_mode(Mode::Hang);
    let started = Instant::now();
    t.app.0.heartbeat_stopping().await;
    assert!(started.elapsed() < Duration::from_millis(2900));
    t.backend.set_mode(Mode::Normal);
}

#[tokio::test]
async fn a_stale_link_blocks_ai_use_but_not_reading_or_deleting() {
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
    let (s, v) = t
        .post("/api/ai/memories/add", &sam, json!({"content": "a note"}))
        .await;
    assert_eq!(s, 200, "{v}");
    let mem = v["memory"]["id"].as_i64().expect("id");
    let (_, v) = t.get("/api/ai/status", &sam).await;
    assert_eq!(
        (v["enabled"].clone(), v["allowed"].clone()),
        (json!(true), json!(true))
    );

    // no report accepted for more than 60 s
    t.app.0.link.age_report_for_tests(Duration::from_secs(61));
    assert!(!t.app.0.link_fresh());
    for who in [&sam, &admin] {
        let (s, v) = t.get("/api/ai/status", who).await;
        assert_eq!(s, 200);
        assert_eq!(
            (v["enabled"].clone(), v["allowed"].clone()),
            (json!(false), json!(false)),
            "{v}"
        );
    }
    for (path, body) in [
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
                503,
                json!({"error": "AI 暂时无法使用，请稍后再试或联系管理员。", "code": "ai.backendDown"})
            ),
            "{path}"
        );
    }
    // reading and deleting one's own data still work, and so do the admin routes
    let (s, v) = t.get("/api/ai/chats", &sam).await;
    assert_eq!((s, v["chats"].as_array().map(Vec::len)), (200, Some(1)));
    let (s, _) = t.get(&format!("/api/ai/chat?id={chat}"), &sam).await;
    assert_eq!(s, 200);
    let (s, v) = t.get("/api/ai/memories", &sam).await;
    assert_eq!((s, v["memories"].as_array().map(Vec::len)), (200, Some(1)));
    let (s, _) = t.get("/api/ai/admin/settings", &admin).await;
    assert_eq!(s, 200);
    let (s, v) = t.get("/api/ai/admin/users", &admin).await;
    assert_eq!(s, 200, "{v}");
    // the Settings answer keeps the stored effective state
    let (_, v) = t.get("/api/ai/admin/settings", &admin).await;
    assert_eq!(v["enabled"], json!(true));
    let (s, v) = t
        .post("/api/ai/memories/delete", &sam, json!({"id": mem}))
        .await;
    assert_eq!((s, v["deleted"].clone()), (200, json!(1)));
    let (s, v) = t
        .post("/api/ai/chats/delete", &sam, json!({"id": chat}))
        .await;
    assert_eq!((s, v["deleted"].clone()), (200, json!(1)));

    // the next accepted report makes AI usable again
    assert!(t.app.0.heartbeat_now().await);
    let (_, v) = t.get("/api/ai/status", &sam).await;
    assert_eq!(v["enabled"], json!(true));
    let ev = t.send_all(&sam, json!({"text": "again"})).await.expect("s");
    assert_eq!(terminal(&ev).0, "done");
}

#[tokio::test]
async fn a_restarted_service_is_not_usable_before_its_first_report() {
    let mut t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    t.restart_without_report(Opts::default());
    let (s, v) = t.post("/api/ai/send", &sam, json!({"text": "hi"})).await;
    assert_eq!((s, v["code"].clone()), (503, json!("ai.backendDown")));
    let (_, v) = t.get("/api/ai/status", &sam).await;
    assert_eq!(v["enabled"], json!(false));
    // the order of the checks: AI+ service off comes first, also while the link is stale
    let (s, _) = t
        .post("/api/ai/admin/enable", &admin, json!({"enabled": false}))
        .await;
    assert_eq!(s, 200);
    t.app.0.link.age_report_for_tests(Duration::from_secs(61));
    let (s, v) = t.post("/api/ai/send", &sam, json!({"text": "hi"})).await;
    assert_eq!(
        (s, v),
        (
            403,
            json!({"error": "AI+ 功能暂时不可用。", "code": "ai.disabled"})
        )
    );
}
