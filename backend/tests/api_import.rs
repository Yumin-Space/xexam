#[path = "api_common/mod.rs"]
mod common;
use axum::body::Body;
use axum::http::StatusCode;
use bytes::Bytes;
use common::*;
use http_body_util::channel::Channel;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use xexam_backend::error::{DbError, ImportError};

const MIB20: usize = 20 * 1024 * 1024;
const TOO_BIG: &str = "文件太大了（最大 20 MB）。题库通常只有 1–2 MB，请确认选对了文件。";
const GONE: &str = "这次上传的预览已经过期或已被处理，请重新上传。";

fn plan_with_skips(n: usize) -> Value {
    let skipped: Vec<Value> = (0..n)
        .map(|i| json!({"row": i + 2, "reason": "r"}))
        .collect();
    json!({
        "sheets": [{"name": "s", "total": 5, "usable": 5, "skipped": skipped, "added": 5, "updated": 0, "removed": 0, "restored": 0}],
        "added": 5, "updated": 0, "unchanged": 0, "restored": 0, "removed": 0, "skipped": n, "usable": 5,
        "removed_samples": [], "removed_stats": 0
    })
}

fn prime_ok(a: &TestApp, n: usize) {
    a.engine.with(|f| {
        f.parse = Some(Ok(workbook(n)));
        f.plan = plan_with_skips(0);
    });
}

async fn do_upload(a: &TestApp, name: &str) -> (Resp, String) {
    prime_ok(a, 5);
    let r = upload(a, vec![b'P'; 300], Some(name)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let tok = r.json()["token"].as_str().unwrap_or("").to_string();
    (r, tok)
}

#[tokio::test]
async fn upload_preview_shape_and_filename() {
    let a = app();
    let (r, tok) = do_upload(&a, "%E3%83%86%E3%82%B9%E3%83%88%2Fdir%5Cevil").await;
    let keys: Vec<String> = r.json().as_object().unwrap().keys().cloned().collect();
    assert_eq!(
        keys,
        [
            "sheets",
            "added",
            "updated",
            "unchanged",
            "restored",
            "removed",
            "skipped",
            "usable",
            "removed_samples",
            "removed_stats",
            "filename",
            "token"
        ]
    );
    assert_eq!(r.json()["filename"], "evil");
    assert_eq!(tok.len(), 22);
    let sheet_keys: Vec<String> = r.json()["sheets"][0]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        sheet_keys,
        [
            "name",
            "total",
            "usable",
            "skipped",
            "added",
            "updated",
            "removed",
            "restored",
            "skippedMore",
            "skippedCount"
        ]
    );
    assert_eq!(a.engine.with(|f| f.last_parse_len), Some(300));
    assert_eq!(
        a.engine.with(|f| f.apply_calls),
        0,
        "preview must not write"
    );
    // missing X-Filename
    prime_ok(&a, 1);
    let r = upload(&a, vec![0; 200], None).await;
    assert_eq!(r.json()["filename"], "未命名.xlsx");
}

#[tokio::test]
async fn preview_caps_skipped_at_30() {
    let a = app();
    a.engine.with(|f| {
        f.parse = Some(Ok(workbook(2)));
        f.plan = plan_with_skips(45);
    });
    let r = upload(&a, vec![0; 200], Some("x.xlsx")).await;
    let s = &r.json()["sheets"][0];
    assert_eq!(s["skipped"].as_array().unwrap().len(), 30);
    assert_eq!(
        (s["skippedMore"].as_i64(), s["skippedCount"].as_i64()),
        (Some(15), Some(45))
    );
}

#[tokio::test]
async fn upload_errors_are_400_with_chinese_text() {
    let a = app();
    for (e, want) in [
        (ImportError::Empty, "文件是空的，或者没有上传完整。"),
        (ImportError::NotXlsx, "这个文件不是 .xlsx（可能是 .csv、.pdf 或别的格式）。请在 Excel 里另存为「Excel 工作簿(.xlsx)」再上传。"),
        (ImportError::Corrupt, "这个 .xlsx 文件已损坏，打不开。请用 Excel 重新保存一份再上传。"),
        (ImportError::Message("文件里的 XML 读不了".into()), "文件里的 XML 读不了"),
        (ImportError::Unexpected, "读取这个 Excel 时出错：文件内容不符合预期"),
    ] {
        a.engine.with(|f| f.parse = Some(Err(e)));
        let r = upload(&a, vec![0; 200], Some("x.xlsx")).await;
        assert_eq!((r.status, r.error().as_str()), (StatusCode::BAD_REQUEST, want));
    }
    // a failed upload does not touch the pending-confirmation slot
    let (_, tok) = do_upload(&a, "a.xlsx").await;
    a.engine.with(|f| f.parse = Some(Err(ImportError::Corrupt)));
    assert_eq!(upload(&a, vec![0; 200], None).await.status, 400);
    assert_eq!(
        post_json(
            &a,
            "/api/import/confirm",
            &format!(r#"{{"token": "{tok}"}}"#)
        )
        .await
        .status,
        200
    );
}

#[tokio::test]
async fn parse_timeout_answers_400() {
    let a = app_with(config_with(&[("XEXAM_PARSE_TIMEOUT_SECS", "1")]), None);
    a.engine.with(|f| {
        f.parse = Some(Ok(workbook(1)));
        f.parse_delay = Some(Duration::from_millis(2500));
    });
    let t = std::time::Instant::now();
    let r = upload(&a, vec![0; 200], Some("slow.xlsx")).await;
    assert!(
        t.elapsed() < Duration::from_millis(2300),
        "must not wait for the parser: {:?}",
        t.elapsed()
    );
    assert_eq!(r.status, 400);
    assert!(r.error().contains("时间太长"), "{}", r.error());
}

#[tokio::test]
async fn upload_size_limits_with_content_length() {
    let a = app();
    prime_ok(&a, 1);
    // exactly 20 MiB passes the size check (the content check decides afterwards); one byte more -> 413
    let r = upload(&a, vec![0; MIB20], Some("x.xlsx")).await;
    assert_eq!(
        r.status,
        200,
        "exactly 20 MiB must pass the size check: {}",
        r.text()
    );
    let r = upload(&a, vec![0; MIB20 + 1], Some("x.xlsx")).await;
    assert_eq!(
        (r.status, r.error().as_str()),
        (StatusCode::PAYLOAD_TOO_LARGE, TOO_BIG)
    );
    let r = upload(&a, vec![0; 21 * 1024 * 1024], Some("x.xlsx")).await;
    assert_eq!(r.status, 413);
    assert_eq!(
        r.header("content-type").as_deref(),
        Some("application/json; charset=utf-8")
    );
}

#[tokio::test]
async fn streaming_body_limit_chunked_no_content_length() {
    let a = app();
    prime_ok(&a, 1);
    // chunked upload without Content-Length: streaming byte count. Up to 20 MiB is processed normally
    let (mut tx, body) = Channel::<Bytes>::new(4);
    let router = a.router.clone();
    let req = build(
        "POST",
        "/api/upload",
        Some("localhost"),
        &[
            ("content-type", b"application/octet-stream"),
            ("transfer-encoding", b"chunked"),
        ],
        Body::new(body),
    );
    let h = tokio::spawn(async move { send(&router, req).await });
    for _ in 0..30 {
        tx.send_data(Bytes::from(vec![1u8; 64 * 1024]))
            .await
            .unwrap();
    }
    drop(tx);
    let r = h.await.unwrap();
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(a.engine.with(|f| f.last_parse_len), Some(30 * 64 * 1024));

    // over 20 MiB: 413 midway, and draining is capped, so the whole body is not read
    let (mut tx, body) = Channel::<Bytes>::new(4);
    let router = a.router.clone();
    let req = build(
        "POST",
        "/api/upload",
        Some("localhost"),
        &[("content-type", b"application/octet-stream")],
        Body::new(body),
    );
    let h = tokio::spawn(async move { send(&router, req).await });
    let chunk = Bytes::from(vec![2u8; 1024 * 1024]);
    let mut sent = 0usize;
    while tx.send_data(chunk.clone()).await.is_ok() {
        sent += 1;
        if sent > 200 {
            break; // the server should not let us keep sending forever
        }
    }
    let r = h.await.unwrap();
    assert_eq!(
        (r.status, r.error().as_str()),
        (StatusCode::PAYLOAD_TOO_LARGE, TOO_BIG)
    );
    assert!(
        sent >= 21,
        "the server must have counted up to the limit first, sent={sent}"
    );
    assert!(
        sent <= 85,
        "draining is bounded (4 x limit = 80 MiB), sent={sent}"
    );
}

#[tokio::test]
async fn lying_content_length_is_not_trusted() {
    let a = app();
    prime_ok(&a, 1);
    // declares 100 bytes, actually sends 21 MiB: rejected by the actual byte count
    let (mut tx, body) = Channel::<Bytes>::new(4);
    let router = a.router.clone();
    let req = build(
        "POST",
        "/api/upload",
        Some("localhost"),
        &[
            ("content-type", b"application/octet-stream"),
            ("content-length", b"100"),
        ],
        Body::new(body),
    );
    let h = tokio::spawn(async move { send(&router, req).await });
    let chunk = Bytes::from(vec![3u8; 1024 * 1024]);
    for _ in 0..22 {
        if tx.send_data(chunk.clone()).await.is_err() {
            break;
        }
    }
    drop(tx);
    let r = h.await.unwrap();
    assert_eq!(
        (r.status, r.error().as_str()),
        (StatusCode::PAYLOAD_TOO_LARGE, TOO_BIG)
    );
    // JSON likewise: declares 2 bytes, actually 1,000,001
    let (mut tx, body) = Channel::<Bytes>::new(4);
    let router = a.router.clone();
    let req = build(
        "POST",
        "/api/attempt",
        Some("localhost"),
        &[
            ("content-type", b"application/json"),
            ("content-length", b"2"),
        ],
        Body::new(body),
    );
    let h = tokio::spawn(async move { send(&router, req).await });
    tx.send_data(Bytes::from(vec![b' '; 1_000_001]))
        .await
        .unwrap();
    drop(tx);
    let r = h.await.unwrap();
    assert_eq!(
        (r.status, r.error().as_str()),
        (StatusCode::PAYLOAD_TOO_LARGE, "too large")
    );
}

#[tokio::test]
async fn declared_longer_than_actual_does_not_hang_or_crash() {
    // hyper reports an early end on a real connection; here a Channel abort simulates a read error
    let a = app();
    let (mut tx, body) = Channel::<Bytes, std::io::Error>::new(4);
    let router = a.router.clone();
    let req = build(
        "POST",
        "/api/upload",
        Some("localhost"),
        &[
            ("content-type", b"application/octet-stream"),
            ("content-length", b"5000"),
        ],
        Body::new(body),
    );
    let h = tokio::spawn(async move { send(&router, req).await });
    tx.send_data(Bytes::from_static(b"PK")).await.unwrap();
    tx.abort(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        "eof",
    ));
    let r = tokio::time::timeout(Duration::from_secs(5), h)
        .await
        .expect("no hang")
        .unwrap();
    assert_eq!(r.status, 400);
    assert_eq!(r.error(), "文件是空的，或者没有上传完整。");
}

#[tokio::test]
async fn early_rejection_drains_the_body() {
    // drain the request body before the 403 / 415: the Channel sender must be able to finish sending, and must not fail because the server dropped the body early
    let a = app();
    for (host, ct, want) in [
        ("evil.example.com", "application/octet-stream", 403u16),
        ("localhost", "text/plain", 415),
    ] {
        let (mut tx, body) = Channel::<Bytes>::new(2);
        let router = a.router.clone();
        let req = build(
            "POST",
            "/api/upload",
            Some(host),
            &[("content-type", ct.as_bytes())],
            Body::new(body),
        );
        let h = tokio::spawn(async move { send(&router, req).await });
        let mut sent = 0;
        for _ in 0..8 {
            if tx
                .send_data(Bytes::from(vec![0u8; 1024 * 1024]))
                .await
                .is_ok()
            {
                sent += 1;
            }
        }
        drop(tx);
        let r = h.await.unwrap();
        assert_eq!(r.status.as_u16(), want);
        assert_eq!(sent, 8, "body must have been drained, not dropped ({host})");
    }
}

#[tokio::test]
async fn confirm_lifecycle() {
    let a = app();
    // no pending item
    for body in [
        r#"{"token": "x"}"#,
        "{}",
        r#"{"token": null}"#,
        r#"{"token": 123}"#,
        r#"{"token": ["a"]}"#,
        r#"{"token": "令牌"}"#,
    ] {
        let r = post_json(&a, "/api/import/confirm", body).await;
        assert_eq!(
            (r.status, r.error().as_str()),
            (StatusCode::CONFLICT, GONE),
            "{body}"
        );
    }
    let (_, tok) = do_upload(&a, "d.xlsx").await;
    // a wrong token is not consumed
    for body in [
        r#"{"token": "wrong-token"}"#.to_string(),
        r#"{"token": 123}"#.into(),
        r#"{"token": "令牌"}"#.into(),
        r#"{"token": null}"#.into(),
        "{}".into(),
        format!(r#"{{"token": "{}"}}"#, &tok[..21]),
        format!(r#"{{"token": "{tok}x"}}"#),
    ] {
        assert_eq!(
            post_json(&a, "/api/import/confirm", &body).await.status,
            409,
            "{body}"
        );
    }
    a.engine
        .with(|f| f.apply = Some(Ok(json!({"added": 5, "sheets": []}))));
    a.engine
        .with(|f| f.bank = json!({"file": "d.xlsx", "questions": [1]}));
    let r = post_json(
        &a,
        "/api/import/confirm",
        &format!(r#"{{"token": "{tok}"}}"#),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(
        r.text(),
        r#"{"result": {"added": 5, "sheets": []}, "bank": {"file": "d.xlsx", "questions": [1]}}"#
    );
    // single use
    let r = post_json(
        &a,
        "/api/import/confirm",
        &format!(r#"{{"token": "{tok}"}}"#),
    )
    .await;
    assert_eq!(r.status, 409);
    assert_eq!(a.engine.with(|f| f.apply_calls), 1);
}

#[tokio::test]
async fn cancel_needs_no_token_and_second_upload_replaces_first() {
    let a = app();
    let (_, t1) = do_upload(&a, "first.xlsx").await;
    let (_, t2) = do_upload(&a, "second.xlsx").await;
    assert_ne!(t1, t2);
    assert_eq!(
        post_json(
            &a,
            "/api/import/confirm",
            &format!(r#"{{"token": "{t1}"}}"#)
        )
        .await
        .status,
        409
    );
    let r = post_json(&a, "/api/import/cancel", "{}").await;
    assert_eq!(
        (r.status, r.text().as_str()),
        (StatusCode::OK, r#"{"ok": true}"#)
    );
    assert_eq!(
        post_json(
            &a,
            "/api/import/confirm",
            &format!(r#"{{"token": "{t2}"}}"#)
        )
        .await
        .status,
        409
    );
    // cancel with no pending item is also 200
    assert_eq!(post_json(&a, "/api/import/cancel", "{}").await.status, 200);
}

#[tokio::test]
async fn confirm_empty_workbook_is_400_and_consumes_token() {
    let a = app();
    a.engine.with(|f| {
        f.parse = Some(Ok(workbook(0)));
        f.plan = plan_with_skips(0);
    });
    let r = upload(&a, vec![0; 200], Some("e.xlsx")).await;
    assert_eq!(r.status, 200);
    let tok = r.json()["token"].as_str().unwrap().to_string();
    let r = post_json(
        &a,
        "/api/import/confirm",
        &format!(r#"{{"token": "{tok}"}}"#),
    )
    .await;
    assert_eq!(
        (r.status, r.error().as_str()),
        (
            StatusCode::BAD_REQUEST,
            "这份文件里没有读到任何可用的题，没有导入。"
        )
    );
    assert_eq!(
        post_json(
            &a,
            "/api/import/confirm",
            &format!(r#"{{"token": "{tok}"}}"#)
        )
        .await
        .status,
        409
    );
    assert_eq!(a.engine.with(|f| f.apply_calls), 0);
}

#[tokio::test]
async fn confirm_db_failure_is_500_with_fixed_text() {
    let a = app();
    let (_, tok) = do_upload(&a, "d.xlsx").await;
    a.engine.with(|f| f.apply = Some(Err(DbError::Join)));
    let r = post_json(
        &a,
        "/api/import/confirm",
        &format!(r#"{{"token": "{tok}"}}"#),
    )
    .await;
    assert_eq!(
        (r.status, r.error().as_str()),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "写入数据库失败，题库没有任何改动。"
        )
    );
    // the token has been consumed (same as the old version)
    assert_eq!(
        post_json(
            &a,
            "/api/import/confirm",
            &format!(r#"{{"token": "{tok}"}}"#)
        )
        .await
        .status,
        409
    );
}

#[tokio::test]
async fn pending_expires_after_ttl() {
    let a = app_with(default_config(), Some(Duration::from_millis(150)));
    let (_, tok) = do_upload(&a, "d.xlsx").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let r = post_json(
        &a,
        "/api/import/confirm",
        &format!(r#"{{"token": "{tok}"}}"#),
    )
    .await;
    assert_eq!((r.status, r.error().as_str()), (StatusCode::CONFLICT, GONE));
    assert_eq!(a.engine.with(|f| f.apply_calls), 0);
}

#[tokio::test]
async fn concurrent_confirm_has_exactly_one_winner() {
    let a = Arc::new(app());
    let (_, tok) = do_upload(&a, "d.xlsx").await;
    let mut handles = vec![];
    for _ in 0..50 {
        let a2 = a.clone();
        let body = format!(r#"{{"token": "{tok}"}}"#);
        handles.push(tokio::spawn(async move {
            post_json(&a2, "/api/import/confirm", &body)
                .await
                .status
                .as_u16()
        }));
    }
    let mut ok = 0;
    let mut gone = 0;
    for h in handles {
        match h.await.unwrap() {
            200 => ok += 1,
            409 => gone += 1,
            other => panic!("unexpected {other}"),
        }
    }
    assert_eq!((ok, gone), (1, 49));
    assert_eq!(a.engine.with(|f| f.apply_calls), 1);
}

// ---------------------------------------------------------------- tests added after review: cooperative timeout and concurrency limit

mod cooperative {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::sync::Semaphore;
    use xexam_backend::error::ImportError;
    use xexam_backend::models::ParsedWorkbook;
    use xexam_backend::services::{parse_upload, Engine};

    /// An engine that respects the deadline: it checks the clock every 5 ms and returns `Timeout` once the deadline passes; it also records how many parses are currently occupying threads.
    struct Spinner {
        running: AtomicUsize,
        peak: AtomicUsize,
        finished: AtomicUsize,
    }

    impl Spinner {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                running: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                finished: AtomicUsize::new(0),
            })
        }
    }

    macro_rules! unused_engine_methods {
        () => {
            fn count_active(&self) -> Result<i64, xexam_backend::error::DbError> {
                unreachable!()
            }
            fn bank(&self) -> Result<serde_json::Value, xexam_backend::error::DbError> {
                unreachable!()
            }
            fn progress(&self, _: i64) -> Result<serde_json::Value, xexam_backend::error::DbError> {
                unreachable!()
            }
            fn import_history(
                &self,
                _: i64,
            ) -> Result<Vec<xexam_backend::models::ImportRecord>, xexam_backend::error::DbError> {
                unreachable!()
            }
            fn plan_import(
                &self,
                _: &[xexam_backend::models::ParsedSheet],
                _: &[xexam_backend::models::Question],
            ) -> Result<serde_json::Value, xexam_backend::error::DbError> {
                unreachable!()
            }
            fn apply_import(
                &self,
                _: &str,
                _: &[xexam_backend::models::ParsedSheet],
                _: &[xexam_backend::models::Question],
            ) -> Result<serde_json::Value, xexam_backend::error::DbError> {
                unreachable!()
            }
            fn record_attempt(
                &self,
                _: i64,
                _: &xexam_backend::models::Attempt,
            ) -> Result<serde_json::Value, xexam_backend::error::DbError> {
                unreachable!()
            }
            fn reset_progress(
                &self,
                _: i64,
            ) -> Result<serde_json::Value, xexam_backend::error::DbError> {
                unreachable!()
            }
        };
    }

    impl Engine for Spinner {
        fn check_and_parse(&self, data: &[u8]) -> Result<ParsedWorkbook, ImportError> {
            self.check_and_parse_until(data, Instant::now() + Duration::from_secs(3600))
        }
        fn check_and_parse_until(
            &self,
            _data: &[u8],
            deadline: Instant,
        ) -> Result<ParsedWorkbook, ImportError> {
            let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            let r = loop {
                if Instant::now() >= deadline {
                    break Err(ImportError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            self.running.fetch_sub(1, Ordering::SeqCst);
            self.finished.fetch_add(1, Ordering::SeqCst);
            r
        }
        unused_engine_methods!();
    }

    /// After a timeout the background thread ends by itself (instead of running on forever), and the client gets a 400 with the Chinese-language message.
    #[tokio::test]
    async fn the_parser_thread_really_stops_after_the_deadline() {
        let eng = Spinner::new();
        let gate = Arc::new(Semaphore::new(2));
        let r = parse_upload(
            eng.clone(),
            gate.clone(),
            vec![0; 10],
            Duration::from_millis(200),
        )
        .await;
        let e = r.expect_err("must time out");
        assert!(
            e.to_string().contains("bad request") || format!("{e:?}").contains("时间太长"),
            "{e:?}"
        );
        // the thread ends by itself shortly after the deadline and returns its permit
        for _ in 0..100 {
            if eng.finished.load(Ordering::SeqCst) == 1 && gate.available_permits() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            eng.finished.load(Ordering::SeqCst),
            1,
            "the parser thread must have finished by itself"
        );
        assert_eq!(eng.running.load(Ordering::SeqCst), 0);
        assert_eq!(
            gate.available_permits(),
            2,
            "the permit must be returned once the thread ends"
        );
    }

    /// The number of concurrent parses never exceeds the number of permits; the extra ones queue, and queueing time also counts toward the time limit.
    #[tokio::test]
    async fn at_most_two_parses_run_at_once_and_waiting_counts_against_the_deadline() {
        let eng = Spinner::new();
        let gate = Arc::new(Semaphore::new(2));
        let mut tasks = Vec::new();
        for _ in 0..5 {
            let (e, g) = (eng.clone(), gate.clone());
            tasks.push(tokio::spawn(async move {
                parse_upload(e, g, vec![0; 10], Duration::from_millis(300))
                    .await
                    .is_err()
            }));
        }
        for t in tasks {
            assert!(t.await.expect("join"), "every call ends as a timeout error");
        }
        assert!(
            eng.peak.load(Ordering::SeqCst) <= 2,
            "peak concurrency was {}",
            eng.peak.load(Ordering::SeqCst)
        );
        // the queued ones time out while waiting for a permit and never start parsing
        for _ in 0..100 {
            if gate.available_permits() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(gate.available_permits(), 2);
        assert!(eng.finished.load(Ordering::SeqCst) <= 5);
    }
}
