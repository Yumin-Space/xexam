#[path = "api_common/mod.rs"]
mod common;
use common::*;
use serde_json::json;

#[tokio::test]
async fn shutdown_disabled_is_403_with_docker_message() {
    let a = app();
    let r = post_json(&a, "/api/shutdown", "{}").await;
    assert_eq!(r.status, 403);
    assert_eq!(r.error(), "在 Docker 里请用 docker compose stop 停止容器。");
}

#[tokio::test]
async fn shutdown_enabled_replies_ok_and_signals() {
    let mut cfg = default_config();
    cfg.allow_shutdown = true;
    let mut a = app_with(cfg, None);
    let mut rx = a.shutdown_rx.take().expect("rx");
    let r = post_json(&a, "/api/shutdown", "{}").await;
    assert_eq!(
        (r.status.as_u16(), r.text().as_str()),
        (200, r#"{"ok": true}"#)
    );
    assert!(rx.try_recv().is_ok(), "graceful shutdown must be triggered");
    let info = get(&a, "/api/info").await;
    assert!(
        info.text().contains(r#""canShutdown": true"#),
        "{}",
        info.text()
    );
}

#[tokio::test]
async fn attempt_passes_cleaned_request_to_the_store_and_returns_progress() {
    let a = app();
    a.engine.with(|f| f.attempt_result = json!({"version": 1, "questions": {"a": {"attempts": 1}}, "history": [], "masterStreak": 2}));
    let r = post_json(&a, "/api/attempt", r#"{"results":[{"id":"a","correct":true},{"id":5,"correct":true}],"sheets":["s",1],"mode":"weak","seconds":"77","extra":1}"#).await;
    assert_eq!(r.status, 200);
    assert_eq!(
        r.text(),
        r#"{"version": 1, "questions": {"a": {"attempts": 1}}, "history": [], "masterStreak": 2}"#
    );
    let att = a.engine.with(|f| f.last_attempt.clone()).expect("recorded");
    assert_eq!(
        (
            att.results.len(),
            att.sheets.clone(),
            att.mode.as_str(),
            att.seconds
        ),
        (1, vec!["s".to_string()], "weak", 77)
    );
    // error paths: storage is never touched
    a.engine.with(|f| f.last_attempt = None);
    for (body, msg) in [
        ("{}", "没有成绩可保存"),
        (r#"{"results": 5}"#, "成绩的格式不对"),
        (r#"{"results": {"a": 1}}"#, "成绩的格式不对"),
        (r#"{"results": "abc"}"#, "成绩的格式不对"),
    ] {
        let r = post_json(&a, "/api/attempt", body).await;
        assert_eq!(
            (r.status.as_u16(), r.error().as_str()),
            (400, msg),
            "{body}"
        );
    }
    assert!(a.engine.with(|f| f.last_attempt.is_none()));
    // Infinity / NaN `seconds` no longer drop the connection; they become 0
    let r = post_json(
        &a,
        "/api/attempt",
        r#"{"results":[{"id":"a","correct":true}],"seconds":Infinity}"#,
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(
        a.engine.with(|f| f.last_attempt.clone()).map(|x| x.seconds),
        Some(0)
    );
}

#[tokio::test]
async fn reset_returns_progress_and_works_twice() {
    let a = app();
    for _ in 0..2 {
        let r = post_json(&a, "/api/reset", "{}").await;
        assert_eq!(
            r.text(),
            r#"{"version": 1, "questions": {}, "history": [], "masterStreak": 2}"#
        );
    }
    assert_eq!(a.engine.with(|f| f.reset_calls), 2);
}

#[tokio::test]
async fn database_failure_is_a_fixed_500_without_details() {
    let a = app();
    a.engine.with(|f| f.fail_all_db = true);
    for p in ["/api/info", "/api/bank", "/api/progress", "/api/imports"] {
        let r = get(&a, p).await;
        assert_eq!(r.status, 500, "{p}");
        assert_eq!(r.error(), "服务器内部出错，请稍后再试。");
        assert!(!r.text().to_lowercase().contains("sqlite"));
    }
    assert_eq!(
        post_json(
            &a,
            "/api/attempt",
            r#"{"results":[{"id":"a","correct":true}]}"#
        )
        .await
        .status,
        500
    );
}
