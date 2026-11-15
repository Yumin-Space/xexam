//! v1.3 cross-area: a generation job keeps the route it pinned at its start when an administrator changes the model of
//! its slot while it runs. The in-process app and a scripted provider. Every text is made up.
mod common;

use common::genkit::*;
use common::*;
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn a_generation_job_keeps_its_pinned_model_when_the_slot_changes_mid_job() {
    use std::sync::atomic::Ordering::SeqCst;
    let t = start().await;
    let admin = t.user("root", true, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    t.post(
        "/api/ai/admin/limits",
        &admin,
        json!({"genUnlimited": 0, "importUnlimited": 0}),
    )
    .await;
    let user = t.user("alexandra_k", false, true);
    t.backend.st().gen = kitchen(2, 20);
    t.stub.st().script = Some(good_model());
    t.app.0.knobs.backoff_scale.store(0, SeqCst);
    t.app.0.knobs.place_poll_ms.store(20, SeqCst);
    t.relax_caps();
    // every upstream call takes a while, so the job is still writing when the model is changed
    t.stub.st().head_delay_ms = 400;

    let (s, v) = t
        .post(
            "/api/ai/gen/start",
            &user,
            json!({"source": "bank", "bankId": 3}),
        )
        .await;
    assert_eq!(s, 202, "{v}");
    let id = v["job"]["id"].as_i64().expect("id");
    for _ in 0..400 {
        let (_, j) = t.get(&format!("/api/ai/gen/job?id={id}"), &user).await;
        if j["job"]["phase"] == "writing" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    // the default slot (which the generation slot follows) now names another model of the same connection
    let (s, v) = t
        .post(
            "/api/ai/admin/models/pick",
            &admin,
            json!({"slot": "default", "connectionId": 1, "model": "demo-chat-large", "manual": false}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], json!(true), "{v}");
    for _ in 0..800 {
        let (_, j) = t.get(&format!("/api/ai/gen/job?id={id}"), &user).await;
        if ["done", "failed", "cancelled"].contains(&j["job"]["state"].as_str().unwrap_or("")) {
            assert_eq!(j["job"]["state"], "done", "{j}");
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let models: Vec<String> = t
        .stub
        .requests()
        .iter()
        .filter(|r| {
            let c = &r.body["messages"][0]["content"];
            c.as_str()
                .or_else(|| c[0]["text"].as_str())
                .is_some_and(|c| c.starts_with("You write new multiple-choice exam questions"))
        })
        .map(|r| r.body["model"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(models.len() >= 5, "plan and writing calls: {models:?}");
    assert!(
        models.iter().all(|m| m == "demo-chat-small"),
        "every call of the job uses the pinned model: {models:?}"
    );
}
