//! v1.3 encryption key problems (the AI keys are encrypted with `XEXAM_AI_SECRET_KEY` or a key file): a changed key is
//! detected at start and fixed by entering the key again on the connection, an invalid environment key and a broken key
//! file keep the service running with AI off. Made-up keys only.
mod common;

use common::*;
use serde_json::{json, Value};

fn hex_key(b: u8) -> String {
    xexam_ai_service::util::hex(&[b; 32])
}

#[tokio::test]
async fn a_changed_encryption_key_is_detected_and_fixed_by_entering_the_key_again() {
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
    assert_eq!(v["keyProblem"], json!("mismatch"), "{v}");
    assert_eq!(v["effective"], json!(false));
    assert!(captured_contains("The saved AI keys cannot be decrypted: the AI encryption key was changed or lost. AI is off until an administrator enters the keys again"));
    // only administrators see the problem; the answer never carries a key
    let (_, st) = t.get("/api/ai/status", &admin).await;
    assert_eq!(
        (st["enabled"].clone(), st["keyProblem"].clone()),
        (json!(false), json!(true))
    );
    let (_, st) = t.get("/api/ai/status", &sam).await;
    assert_eq!(st["keyProblem"], json!(false));
    assert!(!v.to_string().contains(KEY));
    // entering the key again on the connection fixes it
    let conn = v["connections"][0]["id"].clone();
    let rev = v["connections"][0]["rev"].clone();
    let (s, r) = t
        .post(
            "/api/ai/admin/connections/save",
            &admin,
            json!({"id": conn, "rev": rev, "name": v["connections"][0]["name"],
                   "provider": "openai", "baseUrl": format!("{}/v1", t.stub.url), "apiKey": KEY}),
        )
        .await;
    assert_eq!(s, 200, "{r}");
    assert_eq!(r["saved"], json!(true), "{r}");
    let (_, v) = t.get("/api/ai/admin/settings", &admin).await;
    assert_eq!(v["keyProblem"], Value::Null, "{v}");
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
    assert_eq!(v["keyProblem"], json!("invalidEnv"), "{v}");
    // keys cannot be saved without a usable encryption key, other settings still work
    let (s, v) = t
        .post(
            "/api/ai/admin/connections/save",
            &admin,
            json!({"name": "Main", "provider": "openai",
                   "baseUrl": format!("{}/v1", t.stub.url), "apiKey": KEY}),
        )
        .await;
    assert_eq!((s, v["code"].clone()), (409, json!("ai.keyProblem")), "{v}");
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
    assert_eq!(v["keyProblem"], Value::Null, "{v}");
    assert_eq!(v["enabled"], json!(true));
    assert!(captured_contains(&format!(
        "AI encryption key: {}",
        path.display()
    )));
    // a broken file: AI off with its own problem
    std::fs::write(&path, "garbage-garbage-garbage-garbage-garbage-garbage\n").expect("write");
    t.restart(Opts::default()).await;
    let (_, v) = t.get("/api/ai/admin/settings", &admin).await;
    assert_eq!(v["keyProblem"], json!("invalidFile"), "{v}");
    assert_eq!(v["effective"], json!(false));
    assert!(captured_contains("is not valid; AI is off. Move it out of the AI data volume and restart the ai-service container to create a new one, then enter the AI keys again"));
}
