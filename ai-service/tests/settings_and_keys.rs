//! AI settings: the safety limits routes (the v1.2 single-connection settings tests were replaced by `v13_admin.rs`
//! (connections, models, enable) and `v13_keys.rs` (encryption key problems)).
mod common;

use common::*;
use serde_json::{json, Value};

#[tokio::test]
async fn safety_limits_routes() {
    let mut t = start().await;
    let admin = t.user("admin", true, false);
    let (s, v) = t.get("/api/ai/admin/limits", &admin).await;
    assert_eq!(s, 200);
    assert_eq!(
        v["limits"],
        json!({"perUserRunning": 1, "globalRunning": 16, "chatsPerUser": 1000, "messagesPerChat": 400,
               "memoriesPerUser": 30, "memoryTokens": 200,
               "callsPerMinute": 5, "callsPerHour": 20, "callsPerDay": 100, "replyMaxTokens": 3000,
               "genPerDay": 3, "genAdminsExempt": 1, "genMinSource": 30, "genMinWrong": 10,
               "genBankSingle": 14, "genBankMulti": 6, "genWrongSingle": 14, "genWrongMulti": 6,
               "genCallTokens": 20000, "genMaxCalls": 15, "importMaxCalls": 25, "recognizeMaxCalls": 20, "genUnlimited": 1, "importUnlimited": 1, "genRunning": 2, "importMaxMb": 20, "importMaxQuestions": 100,
               "callsAdminsExempt": 1, "studyShare": 1, "studyShareDays": 180, "genMaxChunks": 8,
               "summaryMaxItems": 40, "dailyTokenBudget": 0,
               "paceOn": 1, "paceRpm": 13, "paceTpm": 240000})
    );
    let keys: Vec<&String> = v["limits"].as_object().expect("object").keys().collect();
    assert_eq!(keys.len(), 36);
    assert_eq!(&keys[33..], &["paceOn", "paceRpm", "paceTpm"]);
    assert_eq!(
        &keys[6..10],
        &[
            "callsPerMinute",
            "callsPerHour",
            "callsPerDay",
            "replyMaxTokens"
        ]
    );
    assert_eq!(keys[10], "genPerDay");
    assert_eq!(keys[26], "importMaxQuestions");
    assert_eq!(v["ranges"]["callsPerMinute"], json!([1, 100]));
    assert_eq!(v["ranges"]["callsPerHour"], json!([1, 2000]));
    assert_eq!(v["ranges"]["callsPerDay"], json!([1, 20000]));
    assert_eq!(v["defaults"], v["limits"]);
    assert_eq!(v["ranges"]["chatsPerUser"], json!([10, 100000]));
    assert_eq!(v["ranges"]["replyMaxTokens"], json!([1, 250000]));
    assert!(v["limits"].get("messageChars").is_none());
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
            json!({"memoryTokens": "200"}),
            "ai.limitRange",
            Some("memoryTokens"),
        ),
        (
            json!({"chatsPerUser": 2000, "replyMaxTokens": 250001}),
            "ai.limitRange",
            Some("replyMaxTokens"),
        ),
        (
            json!({"chatsPerUser": 2000, "replyMaxTokens": 0}),
            "ai.limitRange",
            Some("replyMaxTokens"),
        ),
        // the per-message number is not an administrator's setting any more
        (json!({"messageChars": 16000}), "ai.badRequest", None),
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
            json!({"chatsPerUser": 2000, "replyMaxTokens": 16000}),
        )
        .await;
    assert_eq!(s, 200);
    assert_eq!(v["limits"]["chatsPerUser"], json!(2000));
    assert!(captured_contains("AI limits changed by admin \"admin\": chatsPerUser 1000 -> 2000, replyMaxTokens 3000 -> 16000"));
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

#[tokio::test]
async fn unlimited_switches_are_zero_or_one() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let (s, v) = t
        .post(
            "/api/ai/admin/limits",
            &admin,
            json!({"genUnlimited": 1, "importUnlimited": 1}),
        )
        .await;
    assert_eq!(s, 200);
    assert_eq!(v["limits"]["genUnlimited"], json!(1));
    assert_eq!(v["limits"]["importUnlimited"], json!(1));
    let (s, _) = t
        .post("/api/ai/admin/limits", &admin, json!({"genUnlimited": 2}))
        .await;
    assert_eq!(s, 400);
}
