//! v1.9 `XEXAM_AI_USER_AGENT_NAME`: the default names the service, a value is sent as is, `none` sends no
//! `User-Agent` header at all (on chat calls, models lists and connection tests). Everything is made up.
mod common;

use common::*;
use serde_json::json;
use xexam_ai_service::config::AiUserAgent;

fn opts(ua: AiUserAgent) -> Opts {
    Opts {
        ai_user_agent: ua,
        ..Opts::default()
    }
}

async fn chat_once(t: &TestApp) {
    t.relax_caps();
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let ev = t
        .send_all(
            &sam,
            json!({"chatId": null, "text": "What is a list?", "lang": "en"}),
        )
        .await
        .expect("stream");
    assert_eq!(text(&ev), ANSWER);
}

fn chat_request(t: &TestApp) -> StubRequest {
    t.stub
        .requests()
        .into_iter()
        .rfind(|r| r.path == "/v1/chat/completions")
        .expect("a chat request")
}

#[tokio::test]
async fn the_default_user_agent_names_the_service_and_its_version() {
    let t = start_with(opts(AiUserAgent::Default)).await;
    chat_once(&t).await;
    // the test configuration reports version 0.8
    assert_eq!(
        chat_request(&t).header("user-agent"),
        Some("xexam-ai-service/0.8")
    );
}

#[tokio::test]
async fn a_configured_value_is_sent_as_is() {
    let t = start_with(opts(AiUserAgent::Value(
        "MyGateway-Client/2.0 (made up)".into(),
    )))
    .await;
    chat_once(&t).await;
    assert_eq!(
        chat_request(&t).header("user-agent"),
        Some("MyGateway-Client/2.0 (made up)")
    );
}

#[tokio::test]
async fn none_sends_no_user_agent_on_chat_models_and_connection_tests() {
    let t = start_with(opts(AiUserAgent::Off)).await;
    chat_once(&t).await;
    assert_eq!(chat_request(&t).header("user-agent"), None);

    let admin = t.user("admin2", true, false);
    let (s, v) = t.get("/api/ai/admin/models?refresh=1", &admin).await;
    assert_eq!(s, 200, "{v}");
    let (s, v) = t
        .post("/api/ai/admin/connections/test", &admin, json!({"id": 1}))
        .await;
    assert_eq!(s, 200, "{v}");
    let all = t.stub.requests();
    assert!(
        all.iter().any(|r| r.path.ends_with("/models")),
        "the models list reached the stub"
    );
    for r in &all {
        assert_eq!(
            r.header("user-agent"),
            None,
            "{} {} carries no User-Agent",
            r.method,
            r.path
        );
    }
}
