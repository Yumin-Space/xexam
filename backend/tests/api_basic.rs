#[path = "api_common/mod.rs"]
mod common;
use axum::body::Body;
use axum::http::StatusCode;
use common::*;
use serde_json::json;
use xexam_backend::error::ImportError;

fn assert_std_headers(r: &Resp, ctype: &str) {
    assert_eq!(r.header("content-type").as_deref(), Some(ctype));
    assert_eq!(r.header("cache-control").as_deref(), Some("no-store"));
    assert_eq!(
        r.header("x-content-type-options").as_deref(),
        Some("nosniff")
    );
    assert!(r.headers.get("server").is_none(), "no Server header");
    assert!(r.headers.get("x-powered-by").is_none());
}

#[tokio::test]
async fn info_is_byte_exact_python_format() {
    let a = app();
    a.engine.with(|f| f.count = 7);
    let r = get(&a, "/api/info").await;
    assert_eq!(r.status, 200);
    assert_std_headers(&r, "application/json; charset=utf-8");
    let t = r.text();
    assert!(t.starts_with(r#"{"version": "0.3.0", "arch": ""#), "{t}");
    // v0.7.5: the public route no longer says how many questions exist
    assert!(t.ends_with(r#", "canShutdown": false}"#), "{t}");
    assert_eq!(
        r.json()
            .as_object()
            .expect("object")
            .keys()
            .collect::<Vec<_>>(),
        ["version", "arch", "canShutdown"]
    );
    assert_eq!(
        r.header("content-length").unwrap(),
        r.body.len().to_string()
    );
}

#[tokio::test]
async fn head_mirrors_get_without_body() {
    let a = app();
    for p in ["/api/info", "/api/bank", "/api/progress", "/api/imports"] {
        let g = get(&a, p).await;
        let h = send(
            &a.router,
            build("HEAD", p, Some("localhost"), &[], Body::empty()),
        )
        .await;
        assert_eq!(h.status, g.status, "{p}");
        assert!(h.body.is_empty(), "{p}");
        assert_eq!(
            h.header("content-length"),
            g.header("content-length"),
            "{p}"
        );
        assert_eq!(h.header("content-type"), g.header("content-type"), "{p}");
    }
    // HEAD also passes the Host guard
    let h = send(
        &a.router,
        build(
            "HEAD",
            "/api/info",
            Some("evil.example.com"),
            &[],
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(h.status, 403);
    assert!(h.body.is_empty());
}

#[tokio::test]
async fn get_routes_return_engine_data_in_order() {
    let a = app();
    a.engine.with(|f| {
        f.bank = json!({"file": "a.xlsx", "mtime": "2026-01-01 00:00", "error": "", "questions": [{"kind": "single", "stem": "日本語\n\"q\""}], "sheets": []});
        f.imports = vec![xexam_backend::models::ImportRecord { id: 1, at: "t".into(), filename: "f".into(), added: 1, updated: 2, removed: 3, restored: 4, skipped: 5, total: 6 }];
    });
    let b = get(&a, "/api/bank").await;
    assert_eq!(b.text(), "{\"file\": \"a.xlsx\", \"mtime\": \"2026-01-01 00:00\", \"error\": \"\", \"questions\": [{\"kind\": \"single\", \"stem\": \"日本語\\n\\\"q\\\"\"}], \"sheets\": []}");
    let i = get(&a, "/api/imports").await;
    assert_eq!(
        i.text(),
        r#"{"imports": [{"id": 1, "at": "t", "filename": "f", "added": 1, "updated": 2, "removed": 3, "restored": 4, "skipped": 5, "total": 6}]}"#
    );
    let p = get(&a, "/api/progress").await;
    assert_eq!(
        p.text(),
        r#"{"version": 1, "questions": {}, "history": [], "masterStreak": 2}"#
    );
}

#[tokio::test]
async fn unknown_paths_and_methods() {
    let a = app();
    for p in [
        "/api/nope",
        "/api/bank/",
        "/api/upload",
        "/api/shutdown",
        "/",
        "/index.html",
        "/API/INFO",
    ] {
        let r = get(&a, p).await;
        assert_eq!(
            (r.status, r.text().as_str()),
            (StatusCode::NOT_FOUND, "not found"),
            "{p}"
        );
        assert_std_headers(&r, "text/plain; charset=utf-8");
    }
    let r = post_json(&a, "/api/nope", "{}").await;
    assert_eq!(
        (r.status, r.error().as_str()),
        (StatusCode::NOT_FOUND, "not found")
    );
    let r = post_json(&a, "/api/info", "{}").await;
    assert_eq!(r.status, 404);
    // Unknown POST path + wrong type: 415 comes before 404
    let r = post_raw(
        &a,
        "/api/nope",
        b"{}".to_vec(),
        &[("content-type", b"text/plain")],
    )
    .await;
    assert_eq!(
        (r.status, r.error().as_str()),
        (StatusCode::UNSUPPORTED_MEDIA_TYPE, "bad content type")
    );
    for m in ["PUT", "DELETE", "PATCH", "OPTIONS", "TRACE", "CONNECT"] {
        let r = send(
            &a.router,
            build(m, "/api/info", Some("localhost"), &[], Body::empty()),
        )
        .await;
        assert_eq!(r.status, 405, "{m}");
        assert_eq!(r.header("allow").as_deref(), Some("GET, HEAD, POST"));
        assert!(r.headers.get("access-control-allow-origin").is_none());
    }
}

#[tokio::test]
async fn host_guard_get() {
    let a = app();
    let mut cases: Vec<(Option<&str>, u16)> = vec![
        (Some("localhost"), 200),
        (Some("localhost:1"), 200),
        (Some("LOCALHOST."), 200),
        (Some("127.0.0.1:9"), 200),
        (Some("[::1]:9"), 200),
        (Some("evil.example.com"), 403),
        (Some("192.168.1.20:8765"), 403),
        (Some("localhost:1:2"), 403),
        (Some("localhost.evil.com"), 403),
        (Some(""), 403),
        (None, 403),
    ];
    cases.push((Some("evil\t.com"), 403));
    for (h, want) in cases {
        let r = send(&a.router, build("GET", "/api/info", h, &[], Body::empty())).await;
        assert_eq!(r.status.as_u16(), want, "{h:?}");
        if want == 403 {
            assert_std_headers(&r, "text/plain; charset=utf-8");
            assert!(r.text().starts_with("Forbidden host: "), "{}", r.text());
            assert!(
                r.text().contains("XEXAM_ALLOWED_HOSTS")
                    && r.text().contains("XEXAM_ALLOW_ANY_HOST=1")
            );
        }
    }
    // Echo: normalized, control characters removed
    let r = send(
        &a.router,
        build("GET", "/api/info", Some("evil\t.com"), &[], Body::empty()),
    )
    .await;
    assert!(
        r.text().starts_with("Forbidden host: evil.com。"),
        "{}",
        r.text()
    );
    // Static paths also pass the Host guard
    let r = send(
        &a.router,
        build("GET", "/", Some("evil.example.com"), &[], Body::empty()),
    )
    .await;
    assert_eq!(r.status, 403);
    // Overlong Host: the echo is at most 80 characters
    let long = format!("{}.com", "a".repeat(200));
    let r = send(
        &a.router,
        build("GET", "/api/info", Some(&long), &[], Body::empty()),
    )
    .await;
    let t = r.text();
    let seen = t
        .strip_prefix("Forbidden host: ")
        .unwrap()
        .split('。')
        .next()
        .unwrap();
    assert_eq!(seen.chars().count(), 80);
}

#[tokio::test]
async fn forwarded_headers_are_never_trusted() {
    let a = app();
    let h: &[(&str, &[u8])] = &[
        ("x-forwarded-host", b"localhost"),
        ("forwarded", b"host=localhost"),
        ("x-real-ip", b"127.0.0.1"),
    ];
    let r = send(
        &a.router,
        build(
            "GET",
            "/api/info",
            Some("evil.example.com"),
            h,
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(r.status, 403);
    // Conversely: Host is valid, X-Forwarded-Host is something else, no effect
    let r = send(
        &a.router,
        build(
            "GET",
            "/api/info",
            Some("localhost"),
            &[("x-forwarded-host", b"evil.example.com")],
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(r.status, 200);
    let r = send(
        &a.router,
        build(
            "POST",
            "/api/import/cancel",
            Some("evil.example.com"),
            &[
                ("content-type", b"application/json"),
                ("x-forwarded-host", b"localhost"),
                ("origin", b"http://localhost"),
            ],
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(r.status, 403);
}

#[tokio::test]
async fn allowed_hosts_origins_and_any_host() {
    let a = app_with(
        config_with(&[("XEXAM_ALLOWED_HOSTS", "xexam.example.com")]),
        None,
    );
    let ok = |h: &'static str| async {
        send(
            &a.router,
            build("GET", "/api/info", Some(h), &[], Body::empty()),
        )
        .await
        .status
        .as_u16()
    };
    assert_eq!(ok("xexam.example.com").await, 200);
    assert_eq!(ok("XEXAM.Example.COM:443").await, 200);
    assert_eq!(ok("other.example.com").await, 403);
    // An allowed Host is also a valid Origin
    let post = |h: &'static str, o: &'static str| async {
        send(
            &a.router,
            build(
                "POST",
                "/api/import/cancel",
                Some(h),
                &[
                    ("content-type", b"application/json"),
                    ("origin", o.as_bytes()),
                ],
                Body::from("{}"),
            ),
        )
        .await
    };
    assert_eq!(
        post("xexam.example.com", "https://xexam.example.com")
            .await
            .status,
        200
    );
    assert_eq!(
        post("xexam.example.com", "https://evil.example.com")
            .await
            .status,
        403
    );
    assert_eq!(
        post("xexam.example.com", "https://xexam.example.com.evil.com")
            .await
            .status,
        403
    );
    assert_eq!(
        post("xexam.example.com", "https://user@xexam.example.com")
            .await
            .status,
        403
    );
    // Scenario 3: Host is rewritten to an IP, Origin is still the public domain
    let a3 = app_with(
        config_with(&[("XEXAM_ALLOWED_HOSTS", "192.168.1.20")]),
        None,
    );
    let r = send(
        &a3.router,
        build(
            "POST",
            "/api/import/cancel",
            Some("192.168.1.20:8765"),
            &[
                ("content-type", b"application/json"),
                ("origin", b"https://xexam.example.com"),
            ],
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(r.status, 403);
    assert!(
        r.error()
            .contains("请把「xexam.example.com」加进 XEXAM_ALLOWED_ORIGINS"),
        "{}",
        r.error()
    );
    let a3b = app_with(
        config_with(&[
            ("XEXAM_ALLOWED_HOSTS", "192.168.1.20"),
            ("XEXAM_ALLOWED_ORIGINS", "https://xexam.example.com"),
        ]),
        None,
    );
    let r = send(
        &a3b.router,
        build(
            "POST",
            "/api/import/cancel",
            Some("192.168.1.20:8765"),
            &[
                ("content-type", b"application/json"),
                ("origin", b"https://xexam.example.com"),
            ],
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(r.status, 200);
    // XEXAM_ALLOWED_ORIGINS only allows the Origin; it does not become an allowed Host
    let r = send(
        &a3b.router,
        build(
            "GET",
            "/api/info",
            Some("xexam.example.com"),
            &[],
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(r.status, 403);
    // Host is not checked: GET to any domain is allowed, but the Origin check stays as before
    let any = app_with(config_with(&[("XEXAM_ALLOW_ANY_HOST", "1")]), None);
    assert_eq!(
        send(
            &any.router,
            build(
                "GET",
                "/api/info",
                Some("anything.example.net"),
                &[],
                Body::empty()
            )
        )
        .await
        .status,
        200
    );
    assert_eq!(
        send(
            &any.router,
            build("GET", "/api/info", None, &[], Body::empty())
        )
        .await
        .status,
        200
    );
    let r = send(
        &any.router,
        build(
            "POST",
            "/api/import/cancel",
            Some("anything.example.net"),
            &[
                ("content-type", b"application/json"),
                ("origin", b"https://evil.example.com"),
            ],
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(r.status, 403);
    let off = app_with(config_with(&[("XEXAM_ALLOW_ANY_HOST", "false")]), None);
    assert_eq!(
        send(
            &off.router,
            build(
                "GET",
                "/api/info",
                Some("anything.example.net"),
                &[],
                Body::empty()
            )
        )
        .await
        .status,
        403
    );
}

#[tokio::test]
async fn post_deny_messages_match_python() {
    let a = app();
    let h = |host: &'static str, origin: Option<&'static str>| {
        let mut hs: Vec<(&str, &[u8])> = vec![("content-type", b"application/json")];
        if let Some(o) = origin {
            hs.push(("origin", o.as_bytes()));
        }
        build(
            "POST",
            "/api/import/cancel",
            Some(host),
            &hs,
            Body::from("{}"),
        )
    };
    let r = send(&a.router, h("evil.example.com", None)).await;
    assert_eq!(r.status, 403);
    assert_std_headers(&r, "application/json; charset=utf-8");
    assert_eq!(r.error(), "被拒绝：这个请求的 Host 是「evil.example.com」，不在允许的列表里。请把它加进 XEXAM_ALLOWED_HOSTS（或设 XEXAM_ALLOW_ANY_HOST=1，只在前面有身份验证时）。");
    // Host and Origin both wrong: report Host
    let r = send(
        &a.router,
        h("evil.example.com", Some("https://evil.example")),
    )
    .await;
    assert!(r.error().contains("这个请求的 Host 是"), "{}", r.error());
    for o in [
        "null",
        "file://",
        "ftp://localhost",
        "http://user@localhost",
        "",
        "https://",
    ] {
        let r = send(&a.router, h("127.0.0.1:8701", Some(o))).await;
        assert_eq!(r.status, 403, "{o:?}");
        assert_eq!(
            r.error(),
            format!("被拒绝：这个请求的 Origin 是「{o}」，不是一个网址。")
        );
    }
    let r = send(
        &a.router,
        h("127.0.0.1:8701", Some("https://evil.example.com")),
    )
    .await;
    assert_eq!(
        r.error(),
        "被拒绝：这个请求来自「evil.example.com」，和服务看到的 Host「127.0.0.1」不是同一个主机。如果你在 Cloudflare Tunnel 等反向代理后面且代理改写了 Host，请把「evil.example.com」加进 XEXAM_ALLOWED_ORIGINS。"
    );
    // Accepted form: compare only the hostname
    for o in [
        "http://127.0.0.1/path",
        "https://127.0.0.1:1234",
        "HTTP://127.0.0.1:5",
        "http://127.0.0.1:8701",
    ] {
        assert_eq!(
            send(&a.router, h("127.0.0.1:8701", Some(o))).await.status,
            200,
            "{o}"
        );
    }
    assert_eq!(send(&a.router, h("127.0.0.1:8701", None)).await.status, 200);
    // Origin comes before Content-Type: upload with text/plain + malicious Origin gets 403, not 415
    let r = post_raw(
        &a,
        "/api/upload",
        vec![],
        &[
            ("content-type", b"text/plain"),
            ("origin", b"https://evil.example.com"),
        ],
    )
    .await;
    assert_eq!(r.status, 403);
}

#[tokio::test]
async fn content_type_rules() {
    let a = app();
    let try_ct = |ct: Option<&'static str>| {
        let a = &a;
        async move {
            let hs: Vec<(&str, &[u8])> = ct
                .map(|c| vec![("content-type", c.as_bytes())])
                .unwrap_or_default();
            post_raw(a, "/api/import/cancel", b"{}".to_vec(), &hs)
                .await
                .status
                .as_u16()
        }
    };
    assert_eq!(try_ct(Some("application/json")).await, 200);
    assert_eq!(try_ct(Some("application/json; charset=utf-8")).await, 200);
    assert_eq!(try_ct(Some("Application/JSON")).await, 200); // v0.2 compares the main type before the semicolon, case-insensitively
    assert_eq!(try_ct(Some("text/plain")).await, 415);
    assert_eq!(try_ct(None).await, 415);
    assert_eq!(try_ct(Some("text/plain; x=application/json")).await, 415);
    assert_eq!(try_ct(Some("application/jsonp")).await, 415);
    // Upload: substring match (same as the old version)
    a.engine.with(|f| f.parse = Some(Err(ImportError::Empty)));
    let r = post_raw(
        &a,
        "/api/upload",
        vec![],
        &[(
            "content-type",
            b"multipart/form-data; x=application/octet-stream",
        )],
    )
    .await;
    assert_eq!(
        (r.status, r.error().as_str()),
        (StatusCode::BAD_REQUEST, "文件是空的，或者没有上传完整。")
    );
    let r = post_raw(
        &a,
        "/api/upload",
        vec![0; 10],
        &[("content-type", b"application/json")],
    )
    .await;
    assert_eq!(r.status, 415);
    let r = post_raw(&a, "/api/upload", vec![0; 10], &[]).await;
    assert_eq!(r.status, 415);
}

#[tokio::test]
async fn json_body_rules() {
    let a = app();
    let cancel = |b: &[u8]| {
        post_raw(
            &a,
            "/api/import/cancel",
            b.to_vec(),
            &[("content-type", b"application/json")],
        )
    };
    assert_eq!(cancel(b"").await.status, 200); // empty body = {}
    assert_eq!(cancel(b"\xEF\xBB\xBF{}").await.status, 200); // BOM
    assert_eq!(cancel(br#"{"x": NaN}"#).await.status, 200);
    assert_eq!(cancel(br#"{"a":1,"a":2}"#).await.status, 200);
    for bad in [
        &b"{not json"[..],
        b"[1,2]",
        br#""x""#,
        b"null",
        b"{\"a\":\"\xff\"}",
        b"123",
        b"true",
    ] {
        let r = cancel(bad).await;
        assert_eq!(
            (r.status, r.error().as_str()),
            (StatusCode::BAD_REQUEST, "bad json"),
            "{:?}",
            String::from_utf8_lossy(bad)
        );
    }
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    assert_eq!(cancel(deep.as_bytes()).await.status, 400);
    // Exactly 1,000,000 bytes passes, 1,000,001 gives 413
    let pad = |n: usize| format!("{{\"pad\":\"{}\"}}", "a".repeat(n - 10));
    let exact = pad(1_000_000);
    assert_eq!(exact.len(), 1_000_000);
    assert_eq!(cancel(exact.as_bytes()).await.status, 200);
    let over = pad(1_000_001);
    let r = cancel(over.as_bytes()).await;
    assert_eq!(
        (r.status, r.error().as_str()),
        (StatusCode::PAYLOAD_TOO_LARGE, "too large")
    );
    // Negative Content-Length -> 413; non-numeric -> uses the actual byte count
    let r = send(
        &a.router,
        build(
            "POST",
            "/api/attempt",
            Some("localhost"),
            &[
                ("content-type", b"application/json"),
                ("content-length", b"-5"),
            ],
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(r.status, 413);
    let r = send(
        &a.router,
        build(
            "POST",
            "/api/import/cancel",
            Some("localhost"),
            &[
                ("content-type", b"application/json"),
                ("content-length", b"abc"),
            ],
            Body::from("{}"),
        ),
    )
    .await;
    assert_eq!(r.status, 200);
}
