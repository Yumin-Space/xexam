//! AI settings: validation, key re-entry, fingerprint rules, test / enable gating, key mismatch, models, limits.
mod common;

use common::*;
use serde_json::{json, Value};

fn hex_key(b: u8) -> String {
    xexam_ai_service::util::hex(&[b; 32])
}

#[tokio::test]
async fn defaults_and_validation_codes() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let (code, v) = t.get("/api/ai/admin/settings", &admin).await;
    assert_eq!(code, 200);
    let keys: Vec<&str> = v
        .as_object()
        .expect("obj")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec![
            "provider",
            "baseUrl",
            "format",
            "gatewayAuth",
            "model",
            "manualModel",
            "enabled",
            "verified",
            "verifiedAt",
            "retentionDays",
            "keys",
            "keyProblem",
            "updatedAt",
            "updatedBy",
            "defaults"
        ]
    );
    assert_eq!(v["provider"], json!("openai"));
    assert_eq!(v["baseUrl"], json!("https://api.openai.com/v1"));
    assert_eq!(
        v["keys"]["apiKey"],
        json!({"set": false, "hint": "", "unreadable": false})
    );
    assert_eq!(v["keyProblem"], Value::Null);
    assert_eq!(v["retentionDays"], json!(90));
    assert_eq!(
        v["defaults"],
        json!({"openai": "https://api.openai.com/v1", "anthropic": "https://api.anthropic.com"})
    );

    let cases = [
        (json!({"provider": "other"}), 400, "ai.badRequest", None),
        (
            json!({"baseUrl": "ftp://example.com"}),
            400,
            "ai.badUrl",
            None,
        ),
        (
            json!({"baseUrl": "https://user:pw@example.com/v1"}),
            400,
            "ai.badUrl",
            None,
        ),
        (
            json!({"baseUrl": "https://example.com/v1?key=1"}),
            400,
            "ai.badUrl",
            None,
        ),
        (json!({"format": "xml"}), 400, "ai.badRequest", None),
        (json!({"gatewayAuth": "basic"}), 400, "ai.badRequest", None),
        (
            json!({"apiKey": "has space"}),
            400,
            "ai.badKey",
            Some("apiKey"),
        ),
        (
            json!({"gatewayKey": "x".repeat(401)}),
            400,
            "ai.badKey",
            Some("gatewayKey"),
        ),
        (
            json!({"model": "no spaces allowed"}),
            400,
            "ai.badRequest",
            None,
        ),
        (json!({"retentionDays": 0}), 400, "ai.badRetention", None),
        (json!({"retentionDays": "30"}), 400, "ai.badRetention", None),
        (json!({"manualModel": "yes"}), 400, "ai.badRequest", None),
        (json!({"surprise": 1}), 400, "ai.badRequest", None),
        (
            json!({"provider": "gateway", "baseUrl": "https://ai-gateway.example.com/openai", "gatewayAuth": "bearer", "apiKey": KEY}),
            400,
            "ai.headerClash",
            Some("apiKey"),
        ),
    ];
    for (body, status, code, field) in cases {
        let (s, v) = t.post("/api/ai/admin/settings", &admin, body.clone()).await;
        assert_eq!(s, status, "{body} → {v}");
        assert_eq!(v["code"], json!(code), "{body}");
        assert_eq!(v.get("field").and_then(Value::as_str), field, "{body}");
        assert!(v["error"].as_str().is_some_and(|e| !e.is_empty()));
    }
    // nothing was saved by any of them
    let (_, after) = t.get("/api/ai/admin/settings", &admin).await;
    assert_eq!(after["provider"], json!("openai"));
    assert_eq!(after["updatedBy"], json!(""));
    // a valid save: hint, http allowed, retention alone logs its own line
    let (s, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"baseUrl": format!("{}/v1/", t.stub.url), "apiKey": KEY}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["baseUrl"], json!(format!("{}/v1", t.stub.url)));
    assert_eq!(
        v["keys"]["apiKey"],
        json!({"set": true, "hint": "…1234", "unreadable": false})
    );
    assert_eq!(v["switchedOff"], json!(false));
    assert_eq!(v["updatedBy"], json!("admin"));
    assert!(!v.to_string().contains(KEY));
    assert!(captured_contains(
        "user \"admin\" saved the AI settings (provider: OpenAI-compatible API, model: none; the API key was replaced; the base URL was changed)"
    ));
    let (s, _) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"retentionDays": 30}),
        )
        .await;
    assert_eq!(s, 200);
    assert!(captured_contains(
        "user \"admin\" set the AI chat retention to 30 days"
    ));
    // short keys get no hint
    let (_, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"apiKey": "sk-short"}),
        )
        .await;
    assert_eq!(v["keys"]["apiKey"]["hint"], json!(""));
    // non-admins never reach the settings
    let sam = t.user("sam", false, true);
    let (s, v) = t.get("/api/ai/admin/settings", &sam).await;
    assert_eq!((s, v["code"].clone()), (403, json!("ai.adminOnly")));
}

#[tokio::test]
async fn key_reentry_rule() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    let before = t.stub.count();
    let other_port = "http://127.0.0.1:1/v1".to_string();
    let (s, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"baseUrl": other_port}),
        )
        .await;
    assert_eq!(
        (s, v["code"].clone(), v["field"].clone()),
        (400, json!("ai.keyReenter"), json!("apiKey"))
    );
    let (_, now) = t.get("/api/ai/admin/settings", &admin).await;
    assert_eq!(now["baseUrl"], json!(format!("{}/v1", t.stub.url)));
    assert_eq!(
        now["enabled"],
        json!(true),
        "a refused update changes nothing"
    );
    let (s, _) = t.post("/api/ai/admin/models", &admin, json!({})).await;
    assert_eq!(s, 200);
    assert_eq!(
        t.stub.count(),
        before + 1,
        "only the models call reached the stub"
    );
    // provider change needs the key again
    let (s, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"provider": "anthropic", "baseUrl": format!("{}/anthropic", t.stub.url)}),
        )
        .await;
    assert_eq!((s, v["code"].clone()), (400, json!("ai.keyReenter")));
    // a path-only change on the same origin keeps the key (and switches AI off: new fingerprint)
    let (s, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"baseUrl": format!("{}/open/v1", t.stub.url)}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["keys"]["apiKey"]["set"], json!(true));
    assert_eq!(v["switchedOff"], json!(true));
    // the default port written out is the same origin
    let port = t.stub.url.rsplit(':').next().unwrap_or("0").to_string();
    let same = format!("http://127.0.0.1:{port}/v1");
    let (s, _) = t
        .post("/api/ai/admin/settings", &admin, json!({"baseUrl": same}))
        .await;
    assert_eq!(s, 200);
    // with the key typed again, or removed, the origin may change
    let (s, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"baseUrl": "https://example.com/v1", "apiKey": null}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["keys"]["apiKey"]["set"], json!(false));
    assert!(captured_contains("the API key was removed"));
}

#[tokio::test]
async fn fingerprint_rules_and_enable_gating() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let (s, _) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"baseUrl": format!("{}/v1", t.stub.url), "apiKey": KEY}),
        )
        .await;
    assert_eq!(s, 200);
    let (s, v) = t
        .post("/api/ai/admin/enable", &admin, json!({"enabled": true}))
        .await;
    assert_eq!((s, v["code"].clone()), (409, json!("ai.notVerified")));
    let (s, v) = t.post("/api/ai/admin/verify", &admin, json!({})).await;
    assert_eq!((s, v["code"].clone()), (400, json!("ai.modelMissing")));
    let (s, _) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"model": "demo-chat-small"}),
        )
        .await;
    assert_eq!(s, 200);
    let (s, v) = t.post("/api/ai/admin/verify", &admin, json!({})).await;
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
            "ok",
            "code",
            "httpStatus",
            "latencyMs",
            "model",
            "verifiedAt",
            "enabled",
            "switchedOff"
        ]
    );
    assert_eq!(v["ok"], json!(true));
    assert_eq!(v["enabled"], json!(false));
    assert!(v["verifiedAt"].is_i64());
    assert!(captured_contains(
        "user \"admin\" tested the AI settings: OK (model demo-chat-small, "
    ));
    let (s, v) = t
        .post("/api/ai/admin/enable", &admin, json!({"enabled": true}))
        .await;
    assert_eq!(
        (s, v["enabled"].clone(), v["verified"].clone()),
        (200, json!(true), json!(true))
    );
    assert!(captured_contains("user \"admin\" enabled AI"));
    // retention and manualModel are not part of the fingerprint
    let (_, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"retentionDays": 7, "manualModel": true}),
        )
        .await;
    assert_eq!(
        (v["enabled"].clone(), v["switchedOff"].clone()),
        (json!(true), json!(false))
    );
    // every field that must clear it
    let changes = [
        json!({"model": "demo-chat-large"}),
        json!({"apiKey": "sk-demo-0000-9999"}),
        json!({"baseUrl": format!("{}/open/v1", t.stub.url)}),
    ];
    for c in changes {
        t.verify_and_enable(&admin).await;
        let (s, v) = t.post("/api/ai/admin/settings", &admin, c.clone()).await;
        assert_eq!(s, 200, "{c}");
        assert_eq!(v["switchedOff"], json!(true), "{c}");
        assert_eq!(v["enabled"], json!(false), "{c}");
        assert_eq!(v["verified"], json!(false), "{c}");
        assert_eq!(v["verifiedAt"], Value::Null, "{c}");
    }
    assert!(captured_contains(
        "AI was switched off: its settings changed and need a new test (by user \"admin\")"
    ));
    let row: (i64, i64) = t
        .db()
        .query_row("SELECT usage_option, max_tokens FROM settings", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("row");
    assert_eq!(row, (1, 8192));
    // a gateway: format and auth scheme are part of it
    let (s, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"provider": "gateway", "baseUrl": format!("{}/gw/openai", t.stub.url), "gatewayAuth": "cf-aig-authorization",
                   "gatewayKey": GW_KEY, "apiKey": null}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["format"], json!("openai"));
    t.verify_and_enable(&admin).await;
    let (_, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"gatewayAuth": "x-api-key"}),
        )
        .await;
    assert_eq!(v["switchedOff"], json!(true));
}

#[tokio::test]
async fn a_failed_test_switches_off_only_for_configuration_errors() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    // transient: rate limit → state kept
    t.stub.force(Some((429, "{}")));
    let (s, v) = t.post("/api/ai/admin/verify", &admin, json!({})).await;
    assert_eq!(s, 200);
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["code"], json!("ai.up.rateLimit"));
    assert_eq!(v["httpStatus"], json!(429));
    assert_eq!(v["enabled"], json!(true));
    assert_eq!(v["switchedOff"], json!(false));
    assert!(captured_contains(
        "user \"admin\" tested the AI settings: failed (ai.up.rateLimit, HTTP 429)"
    ));
    // configuration: auth → AI off
    t.stub.force(Some((401, "{}")));
    let (_, v) = t.post("/api/ai/admin/verify", &admin, json!({})).await;
    assert_eq!(v["code"], json!("ai.up.auth"));
    assert_eq!(v["enabled"], json!(false));
    assert_eq!(v["switchedOff"], json!(true));
    assert_eq!(v["verifiedAt"], Value::Null);
    assert!(captured_contains(
        "AI was switched off: the test failed (ai.up.auth) (by user \"admin\")"
    ));
    t.stub.force(None);
    // connect failure: no HTTP status
    let (s, _) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"baseUrl": "http://127.0.0.1:1/v1", "apiKey": KEY}),
        )
        .await;
    assert_eq!(s, 200);
    let (_, v) = t.post("/api/ai/admin/verify", &admin, json!({})).await;
    assert_eq!(
        (v["code"].clone(), v["httpStatus"].clone()),
        (json!("ai.up.connect"), Value::Null)
    );
}

#[tokio::test]
async fn verify_falls_back_once_and_chat_uses_the_same_request() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    // OpenAI format: the model refuses stream_options → retried without it, stored
    t.configure_openai(&admin, "demo-nousage").await;
    let opt: i64 = t
        .db()
        .query_row("SELECT usage_option FROM settings", [], |r| r.get(0))
        .expect("row");
    assert_eq!(opt, 0);
    let ev = t.send_all(&sam, json!({"text": "hi"})).await.expect("s");
    assert_eq!(terminal(&ev).0, "done");
    assert!(t.stub.last().body.get("stream_options").is_none());
    assert_eq!(
        terminal(&ev).1["tokensIn"],
        Value::Null,
        "no usage reported without the option"
    );
    // Anthropic format: 8192 refused → 4096, also for chat
    let (s, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"provider": "anthropic", "baseUrl": format!("{}/anthropic", t.stub.url), "apiKey": KEY, "model": "demo-4k"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    t.verify_and_enable(&admin).await;
    let mt: i64 = t
        .db()
        .query_row("SELECT max_tokens FROM settings", [], |r| r.get(0))
        .expect("row");
    assert_eq!(mt, 4096);
    let ev = t.send_all(&sam, json!({"text": "hi"})).await.expect("s");
    assert_eq!(terminal(&ev).0, "done");
    assert_eq!(t.stub.last().body["max_tokens"], json!(4096));
    let verify_rows: i64 = t
        .db()
        .query_row(
            "SELECT COUNT(*) FROM usage WHERE kind = 'verify'",
            [],
            |r| r.get(0),
        )
        .expect("n");
    assert_eq!(verify_rows, 4, "two calls per verify with a fallback");
}

#[tokio::test]
async fn a_changed_encryption_key_is_detected_and_fixed_by_entering_the_keys() {
    let mut t = start_with(Opts {
        secret_key: Some(hex_key(1)),
        ..Opts::default()
    })
    .await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    assert!(captured_contains(
        "AI encryption key: from XEXAM_AI_SECRET_KEY"
    ));
    t.restart(Opts {
        secret_key: Some(hex_key(2)),
        ..Opts::default()
    })
    .await;
    let (_, v) = t.get("/api/ai/admin/settings", &admin).await;
    assert_eq!(v["keyProblem"], json!("mismatch"));
    assert_eq!(
        v["keys"]["apiKey"],
        json!({"set": true, "hint": "…1234", "unreadable": true})
    );
    assert_eq!(v["enabled"], json!(false));
    let stored: i64 = t
        .db()
        .query_row("SELECT enabled FROM settings", [], |r| r.get(0))
        .expect("row");
    assert_eq!(stored, 0, "written once at start");
    assert!(captured_contains("The saved AI keys cannot be decrypted: the AI encryption key was changed or lost. AI is off until an administrator enters the keys again"));
    let (_, st) = t.get("/api/ai/status", &admin).await;
    assert_eq!(
        (st["enabled"].clone(), st["keyProblem"].clone()),
        (json!(false), json!(true))
    );
    let (_, st) = t.get("/api/ai/status", &sam).await;
    assert_eq!(st["keyProblem"], json!(false), "only administrators see it");
    let (s, v) = t
        .post("/api/ai/admin/enable", &admin, json!({"enabled": true}))
        .await;
    assert_eq!((s, v["code"].clone()), (409, json!("ai.keyProblem")));
    let (s, v) = t.post("/api/ai/admin/verify", &admin, json!({})).await;
    assert_eq!((s, v["code"].clone()), (409, json!("ai.keyProblem")));
    let (s, v) = t.post("/api/ai/admin/models", &admin, json!({})).await;
    assert_eq!((s, v["code"].clone()), (409, json!("ai.keyProblem")));
    // entering the key again fixes it
    let (s, v) = t
        .post("/api/ai/admin/settings", &admin, json!({"apiKey": KEY}))
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["keyProblem"], Value::Null);
    assert_eq!(v["keys"]["apiKey"]["unreadable"], json!(false));
    t.verify_and_enable(&admin).await;
    assert!(t
        .send_all(&sam, json!({"text": "works again"}))
        .await
        .is_ok());
}

#[tokio::test]
async fn an_invalid_env_key_keeps_the_service_running_with_ai_off() {
    let t = start_with(Opts {
        secret_key: Some("not-a-key".into()),
        ..Opts::default()
    })
    .await;
    let admin = t.user("admin", true, false);
    assert!(captured_contains(
        "XEXAM_AI_SECRET_KEY is not a valid key (expected 32 bytes as 64 hex digits or base64); AI is off"
    ));
    let (_, v) = t.get("/api/ai/admin/settings", &admin).await;
    assert_eq!(v["keyProblem"], json!("invalidEnv"));
    let (s, v) = t
        .post("/api/ai/admin/settings", &admin, json!({"apiKey": KEY}))
        .await;
    assert_eq!((s, v["code"].clone()), (409, json!("ai.keyProblem")));
    let (s, _) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"retentionDays": 12}),
        )
        .await;
    assert_eq!(s, 200);
    // no key file was created when the variable is set
    assert!(!t.dir.path().join("ai").join("secret.key").exists());
}

#[tokio::test]
async fn a_key_file_is_created_and_reused() {
    let mut t = start().await;
    let admin = t.user("admin", true, false);
    let path = t.dir.path().join("ai").join("secret.key");
    assert!(path.exists());
    assert!(captured_contains("Created a new AI key file "));
    t.configure_openai(&admin, "demo-chat-small").await;
    t.restart(Opts::default()).await;
    let (_, v) = t.get("/api/ai/admin/settings", &admin).await;
    assert_eq!(v["keyProblem"], Value::Null);
    assert_eq!(v["enabled"], json!(true));
    assert!(captured_contains(&format!(
        "AI encryption key: {}",
        path.display()
    )));
    // a broken file: AI off with its own problem
    std::fs::write(&path, "garbage-garbage-garbage-garbage-garbage-garbage\n").expect("write");
    t.restart(Opts::default()).await;
    let (_, v) = t.get("/api/ai/admin/settings", &admin).await;
    assert_eq!(v["keyProblem"], json!("invalidFile"));
    assert_eq!(v["enabled"], json!(false));
    assert!(captured_contains("is not valid; AI is off. Move it out of the AI data volume and restart the ai-service container to create a new one, then enter the AI keys again"));
}

#[tokio::test]
async fn models_listing() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let (_, _) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"baseUrl": format!("{}/v1", t.stub.url), "apiKey": KEY}),
        )
        .await;
    let (s, v) = t.post("/api/ai/admin/models", &admin, json!({})).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["listed"], json!(true));
    assert_eq!(
        v["models"],
        json!([
            "demo-chat-large",
            "demo-chat-small",
            "demo-error",
            "demo-length",
            "demo-nousage"
        ])
    );
    assert!(captured_contains(
        "user \"admin\" listed the AI models: 5 found"
    ));
    let n = t.stub.count();
    let (_, again) = t.post("/api/ai/admin/models", &admin, json!({})).await;
    assert_eq!(again, v);
    assert_eq!(t.stub.count(), n, "cached for the same fingerprint");
    // a service without a model list
    let (_, _) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"baseUrl": format!("{}/nolist/v1", t.stub.url)}),
        )
        .await;
    let (s, v) = t.post("/api/ai/admin/models", &admin, json!({})).await;
    assert_eq!((s, v), (200, json!({"models": [], "listed": false})));
    assert!(captured_contains(
        "user \"admin\" listed the AI models: the service does not list models"
    ));
    // Anthropic: API order; the key is required
    let (s, _) = t
        .post("/api/ai/admin/settings", &admin, json!({"provider": "anthropic", "baseUrl": format!("{}/anthropic", t.stub.url), "apiKey": null}))
        .await;
    assert_eq!(s, 200);
    let (s, v) = t.post("/api/ai/admin/models", &admin, json!({})).await;
    assert_eq!(
        (s, v["code"].clone(), v["field"].clone()),
        (400, json!("ai.keyMissing"), json!("apiKey"))
    );
    let (_, _) = t
        .post("/api/ai/admin/settings", &admin, json!({"apiKey": KEY}))
        .await;
    let (s, v) = t.post("/api/ai/admin/models", &admin, json!({})).await;
    assert_eq!(
        (s, v["models"].clone()),
        (200, json!(["demo-claude", "demo-4k", "demo-hold"]))
    );
    let req = t.stub.last();
    assert_eq!(req.path, "/anthropic/v1/models");
    // wrong key → 502 with the code; the masked key in the body is never shown
    let (_, _) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"apiKey": "wrong-key-00000000"}),
        )
        .await;
    let (s, v) = t.post("/api/ai/admin/models", &admin, json!({})).await;
    assert_eq!((s, v["code"].clone()), (502, json!("ai.up.auth")));
    assert!(!v.to_string().contains("sk-de****"));
    // redirects are never followed
    let (_, _) = t
        .post("/api/ai/admin/settings", &admin, json!({"provider": "openai", "baseUrl": format!("{}/redirect/v1", t.stub.url), "apiKey": KEY}))
        .await;
    let before = t.stub.count();
    let (s, v) = t.post("/api/ai/admin/models", &admin, json!({})).await;
    assert_eq!((s, v["code"].clone()), (502, json!("ai.up.protocol")));
    assert_eq!(
        t.stub.count(),
        before + 1,
        "the redirect target was not requested"
    );
    // gateway: the gateway key is required
    let (_, _) = t
        .post("/api/ai/admin/settings", &admin, json!({"provider": "gateway", "baseUrl": format!("{}/gw/openai", t.stub.url), "gatewayAuth": "cf-aig-authorization", "apiKey": null}))
        .await;
    let (s, v) = t.post("/api/ai/admin/models", &admin, json!({})).await;
    assert_eq!((s, v["field"].clone()), (400, json!("gatewayKey")));
}

#[tokio::test]
async fn gateway_headers_for_each_scheme() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    let base = format!("{}/gw/openai", t.stub.url);
    // cf-aig-authorization, with an upstream key (OpenAI format → Authorization)
    let (s, v) = t
        .post("/api/ai/admin/settings", &admin, json!({"provider": "gateway", "baseUrl": base, "gatewayAuth": "cf-aig-authorization", "gatewayKey": GW_KEY, "apiKey": KEY, "model": "demo-chat-small"}))
        .await;
    assert_eq!(s, 200, "{v}");
    t.verify_and_enable(&admin).await;
    t.send_all(&sam, json!({"text": "x"})).await.expect("s");
    let r = t.stub.last();
    assert_eq!(r.path, "/gw/openai/chat/completions");
    assert_eq!(
        r.header("cf-aig-authorization"),
        Some(format!("Bearer {GW_KEY}").as_str())
    );
    assert_eq!(
        r.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    // bearer without an upstream key (BYOK)
    let (s, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"gatewayAuth": "bearer", "apiKey": null}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    t.verify_and_enable(&admin).await;
    t.send_all(&sam, json!({"text": "x"})).await.expect("s");
    let r = t.stub.last();
    assert_eq!(
        r.header("authorization"),
        Some(format!("Bearer {GW_KEY}").as_str())
    );
    assert!(r.header("cf-aig-authorization").is_none());
    // x-api-key with an upstream key in the OpenAI format
    let (s, _) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"gatewayAuth": "x-api-key", "apiKey": KEY}),
        )
        .await;
    assert_eq!(s, 200);
    t.verify_and_enable(&admin).await;
    t.send_all(&sam, json!({"text": "x"})).await.expect("s");
    let r = t.stub.last();
    assert_eq!(r.header("x-api-key"), Some(GW_KEY));
    assert_eq!(
        r.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    // Anthropic format through the gateway: the path keeps the keys (same origin), format inferred only on first save
    let (s, v) = t
        .post("/api/ai/admin/settings", &admin, json!({"baseUrl": format!("{}/gw/anthropic", t.stub.url), "format": "anthropic", "gatewayAuth": "cf-aig-authorization", "model": "demo-claude"}))
        .await;
    assert_eq!(s, 200, "{v}");
    t.verify_and_enable(&admin).await;
    t.send_all(&sam, json!({"text": "x"})).await.expect("s");
    let r = t.stub.last();
    assert_eq!(r.path, "/gw/anthropic/v1/messages");
    assert_eq!(r.header("x-api-key"), Some(KEY));
    assert_eq!(r.header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(
        r.header("cf-aig-authorization"),
        Some(format!("Bearer {GW_KEY}").as_str())
    );
}

#[tokio::test]
async fn safety_limits_routes() {
    let mut t = start().await;
    let admin = t.user("admin", true, false);
    let (s, v) = t.get("/api/ai/admin/limits", &admin).await;
    assert_eq!(s, 200);
    assert_eq!(
        v["limits"],
        json!({"perUserRunning": 1, "globalRunning": 16, "chatsPerUser": 1000, "messagesPerChat": 400,
               "memoriesPerUser": 50, "memoryChars": 300, "messageChars": 16000})
    );
    assert_eq!(v["defaults"], v["limits"]);
    assert_eq!(v["ranges"]["chatsPerUser"], json!([10, 100000]));
    assert_eq!(v["ranges"]["messageChars"], json!([1000, 32000]));
    for (body, code, field) in [
        (
            json!({"chatsPerUser": 5}),
            "ai.limitRange",
            Some("chatsPerUser"),
        ),
        (
            json!({"globalRunning": 65}),
            "ai.limitRange",
            Some("globalRunning"),
        ),
        (
            json!({"perUserRunning": 1.5}),
            "ai.limitRange",
            Some("perUserRunning"),
        ),
        (
            json!({"memoryChars": "300"}),
            "ai.limitRange",
            Some("memoryChars"),
        ),
        (
            json!({"chatsPerUser": 2000, "messageChars": 999}),
            "ai.limitRange",
            Some("messageChars"),
        ),
        (json!({"bogus": 1}), "ai.badRequest", None),
    ] {
        let (s, v) = t.post("/api/ai/admin/limits", &admin, body.clone()).await;
        assert_eq!(s, 400, "{body}");
        assert_eq!(v["code"], json!(code), "{body}");
        assert_eq!(v.get("field").and_then(Value::as_str), field, "{body}");
    }
    let (_, v) = t.get("/api/ai/admin/limits", &admin).await;
    assert_eq!(v["limits"]["chatsPerUser"], json!(1000), "all or nothing");
    let (s, v) = t
        .post(
            "/api/ai/admin/limits",
            &admin,
            json!({"chatsPerUser": 2000, "messageChars": 20000}),
        )
        .await;
    assert_eq!(s, 200);
    assert_eq!(v["limits"]["chatsPerUser"], json!(2000));
    assert!(captured_contains("AI limits changed by admin \"admin\": chatsPerUser 1000 -> 2000, messageChars 16000 -> 20000"));
    // limits are not part of the fingerprint
    t.configure_openai(&admin, "demo-chat-small").await;
    let (_, _) = t
        .post("/api/ai/admin/limits", &admin, json!({"globalRunning": 4}))
        .await;
    let (_, st) = t.get("/api/ai/admin/settings", &admin).await;
    assert_eq!(st["enabled"], json!(true));
    // kept across a restart, gate sized from it
    t.restart(Opts::default()).await;
    let (_, v) = t.get("/api/ai/admin/limits", &admin).await;
    assert_eq!(v["limits"]["chatsPerUser"], json!(2000));
    assert_eq!(v["limits"]["globalRunning"], json!(4));
    assert_eq!(t.app.0.gate.size(), 4);
    let sam = t.user("sam", false, true);
    let (s, v) = t
        .post("/api/ai/admin/limits", &sam, json!({"chatsPerUser": 20}))
        .await;
    assert_eq!((s, v["code"].clone()), (403, json!("ai.adminOnly")));
}
