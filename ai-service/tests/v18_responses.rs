//! v1.8 OpenAI Responses API connections: the format is stored as `format = 'openai'` plus `responses = 1` (no table
//! rebuild), the admin routes show and accept `"responses"` for a gateway only, a chat answer streams end to end from
//! `response.output_text.delta` events, a cut answer is marked, a refused `max_output_tokens` falls back once, and a
//! generation call carries its schema as `text.format`. Everything is made up.
mod common;

use common::genkit::*;
use common::*;
use serde_json::{json, Value};
use std::sync::atomic::Ordering::SeqCst;
use std::time::Duration;
use xexam_ai_service::config::JournalMode;
use xexam_ai_service::store::conn;
use xexam_ai_service::store::{Store, DB_FILE};

const SAVE: &str = "/api/ai/admin/connections/save";

fn chat_requests(t: &TestApp) -> Vec<StubRequest> {
    t.stub
        .requests()
        .into_iter()
        .filter(|r| r.path.ends_with("/responses"))
        .collect()
}

fn conn_of(s: &Value, id: i64) -> &Value {
    s["connections"]
        .as_array()
        .expect("connections")
        .iter()
        .find(|c| c["id"] == json!(id))
        .expect("connection")
}

fn stored(t: &TestApp, id: i64) -> (String, i64) {
    t.db()
        .query_row(
            "SELECT format, responses FROM connections WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row")
}

#[tokio::test]
async fn a_chat_answer_streams_end_to_end_over_the_responses_api() {
    let t = start().await;
    t.relax_caps();
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_responses(&admin, "demo-chat-small").await;
    let ev = t
        .send_all(
            &sam,
            json!({"chatId": null, "text": "What is a list?", "lang": "en"}),
        )
        .await
        .expect("stream");
    assert_eq!(text(&ev), ANSWER, "text comes from the deltas only, once");
    assert_eq!(
        terminal(&ev).1,
        json!({"status": "done", "tokensIn": 21, "tokensOut": 13, "truncated": false})
    );
    let req = chat_requests(&t).pop().expect("a Responses request");
    assert_eq!(req.path, "/gw/v1/responses");
    assert_eq!(
        req.header("authorization"),
        Some(format!("Bearer {GW_KEY}").as_str())
    );
    assert_eq!(req.header("accept"), Some("text/event-stream"));
    assert_eq!(req.header("anthropic-version"), None);
    assert!(req.body["instructions"]
        .as_str()
        .unwrap_or("")
        .starts_with("You are the study assistant inside Xexam"));
    assert_eq!(
        req.body["input"][0],
        json!({"role": "user", "content": "What is a list?"})
    );
    assert_eq!(req.body["stream"], json!(true));
    assert_eq!(req.body["store"], json!(false));
    assert_eq!(req.body["max_output_tokens"], json!(3000));
    for gone in [
        "messages",
        "stream_options",
        "max_tokens",
        "max_completion_tokens",
        "response_format",
    ] {
        assert!(req.body.get(gone).is_none(), "{gone} must not be sent");
    }
    assert!(!req.body.to_string().contains("cache_control"));
    // the answer is stored once
    let chat = ev[0].1["chatId"].as_i64().expect("chat id");
    let (st, v) = t.get(&format!("/api/ai/chat?id={chat}"), &sam).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["messages"][1]["content"], json!(ANSWER));
}

#[tokio::test]
async fn cached_tokens_are_recorded_without_adding_them_to_the_input() {
    let t = start().await;
    t.relax_caps();
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_responses(&admin, "demo-cached").await;
    let ev = t
        .send_all(&sam, json!({"chatId": null, "text": "Hello", "lang": "en"}))
        .await
        .expect("stream");
    let (name, done) = terminal(&ev);
    assert_eq!(name, "done");
    assert_eq!(done["tokensIn"], json!(21), "input already includes cached");
    let cached: Option<i64> = t
        .db()
        .query_row(
            "SELECT tokens_cached FROM usage WHERE kind = 'chat' ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("usage row");
    assert_eq!(cached, Some(8));
}

#[tokio::test]
async fn an_incomplete_answer_is_marked_cut() {
    let t = start().await;
    t.relax_caps();
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_responses(&admin, "demo-length").await;
    let ev = t
        .send_all(&sam, json!({"text": "x"}))
        .await
        .expect("stream");
    let (name, done) = terminal(&ev);
    assert_eq!(name, "done");
    assert_eq!(done["truncated"], json!(true));
    assert_eq!(text(&ev), ANSWER);
}

#[tokio::test]
async fn a_model_that_refuses_max_output_tokens_is_sent_without_it_once() {
    let t = start().await;
    t.relax_caps();
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_responses(&admin, "demo-nomax").await;
    for _ in 0..2 {
        let ev = t
            .send_all(&sam, json!({"text": "Hello"}))
            .await
            .expect("stream");
        assert_eq!(terminal(&ev).0, "done", "{ev:?}");
        assert_eq!(text(&ev), ANSWER);
    }
    let reqs: Vec<StubRequest> = chat_requests(&t)
        .into_iter()
        .filter(|r| r.body["instructions"].as_str() != Some("Reply with the single word OK."))
        .collect();
    // first message: refused, then retried without the field; second message: remembered, no refusal
    assert_eq!(reqs.len(), 3, "{reqs:?}");
    assert!(reqs[0].body.get("max_output_tokens").is_some());
    assert!(reqs[1].body.get("max_output_tokens").is_none());
    assert!(reqs[2].body.get("max_output_tokens").is_none());
    assert!(captured_contains(
        "the AI provider refused max_output_tokens; sending no token limit"
    ));
}

#[tokio::test]
async fn a_generation_call_carries_its_schema_as_text_format() {
    let t = start().await;
    let admin = t.user("root", true, true);
    t.configure_responses(&admin, "demo-chat-small").await;
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
    let (s, v) = t
        .post(
            "/api/ai/gen/start",
            &user,
            json!({"source": "bank", "bankId": 3}),
        )
        .await;
    assert_eq!(s, 202, "{v}");
    let id = v["job"]["id"].as_i64().expect("id");
    let mut state = String::new();
    for _ in 0..800 {
        let (s, v) = t.get(&format!("/api/ai/gen/job?id={id}"), &user).await;
        assert_eq!(s, 200, "{v}");
        state = v["job"]["state"].as_str().unwrap_or("").to_string();
        if ["done", "failed", "cancelled"].contains(&state.as_str()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(state, "done");
    let reqs = chat_requests(&t);
    let with_schema: Vec<&StubRequest> = reqs
        .iter()
        .filter(|r| r.body.get("text").is_some())
        .collect();
    assert!(!with_schema.is_empty(), "a call carries a schema");
    for r in with_schema {
        let f = &r.body["text"]["format"];
        assert_eq!(f["type"], json!("json_schema"));
        assert_eq!(f["name"], json!("answer"));
        assert_eq!(f["strict"], json!(true));
        assert!(f["schema"].is_object());
        assert!(r.body.get("response_format").is_none());
    }
    assert!(reqs.iter().all(|r| r.body.get("messages").is_none()));
}

#[tokio::test]
async fn the_admin_routes_show_and_accept_responses_for_a_gateway_only() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    let gw = format!("{}/gw/v1", t.stub.url);
    let (s, v) = t
        .post(
            SAVE,
            &admin,
            json!({"name": "gateway", "provider": "gateway", "baseUrl": gw, "gatewayKey": GW_KEY,
                   "gatewayAuth": "bearer", "format": "responses"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let id = v["connectionId"].as_i64().expect("id");
    assert_eq!(conn_of(&v["settings"], id)["format"], json!("responses"));
    assert_eq!(stored(&t, id), ("openai".to_string(), 1));
    // a provider key on a Bearer Responses gateway clashes like a Chat Completions one
    let (s, v) = t
        .post(
            SAVE,
            &admin,
            json!({"id": id, "rev": 1, "name": "gateway", "provider": "gateway", "baseUrl": gw,
                   "apiKey": KEY, "gatewayAuth": "bearer", "format": "responses"}),
        )
        .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (400, Some("ai.headerClash")),
        "{v}"
    );
    // the provider `openai` stays Chat Completions whatever the format field says
    let (s, v) = t
        .post(
            SAVE,
            &admin,
            json!({"name": "plain", "provider": "openai", "baseUrl": format!("{}/open/v1", t.stub.url),
                   "apiKey": KEY, "format": "responses"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let pid = v["connectionId"].as_i64().expect("id");
    assert_eq!(conn_of(&v["settings"], pid)["format"], json!("openai"));
    assert_eq!(stored(&t, pid), ("openai".to_string(), 0));
    // moving the gateway back to another format clears the flag
    let rev = conn_of(&v["settings"], id)["rev"].as_i64().expect("rev");
    let (s, v) = t
        .post(
            SAVE,
            &admin,
            json!({"id": id, "rev": rev, "name": "gateway", "provider": "gateway", "baseUrl": gw,
                   "format": "openai"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(conn_of(&v["settings"], id)["format"], json!("openai"));
    assert_eq!(stored(&t, id), ("openai".to_string(), 0));
}

#[test]
fn storage_round_trips_responses_and_a_url_never_infers_it() {
    use xexam_ai_service::settings::{infer_format, Format};
    for u in [
        "https://example.com/v1",
        "https://example.com/v1/responses",
        "https://gw.example.com/x/responses",
        "https://gw.example.com/x/openai",
    ] {
        assert_ne!(infer_format(u), Format::Responses, "{u}");
    }
    assert_eq!(Format::parse("responses"), Some(Format::Responses));
    assert_eq!(Format::Responses.as_str(), "responses");
    let dir = tempfile::tempdir().expect("dir");
    let store = Store::open(dir.path(), JournalMode::Delete).expect("open");
    let id = store
        .write(|tx| {
            conn::insert_connection(
                tx,
                &conn::NewConn {
                    name: "r",
                    provider: "gateway",
                    base_url: "https://example.com/v1",
                    format: "responses",
                    gateway_auth: "bearer",
                    lists_models: true,
                    effort_family: "openai",
                    by: "test",
                    now: 1,
                },
            )
        })
        .expect("insert");
    let row = store
        .read(|c| conn::connection(c, id))
        .expect("read")
        .expect("row");
    assert_eq!(row.format, "responses");
    let raw: (String, i64) = store
        .read(|c| {
            c.query_row(
                "SELECT format, responses FROM connections WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .expect("raw");
    assert_eq!(raw, ("openai".to_string(), 1));
    // saving another format writes responses = 0
    store
        .write(|tx| {
            conn::update_connection(
                tx,
                id,
                row.rev,
                &conn::ConnChange {
                    name: "r",
                    provider: "gateway",
                    base_url: "https://example.com/v1",
                    format: "anthropic",
                    gateway_auth: "bearer",
                    lists_models: true,
                    effort_family: "openai",
                    by: "test",
                    now: 2,
                },
            )
        })
        .expect("update");
    let row = store
        .read(|c| conn::connection(c, id))
        .expect("read")
        .expect("row");
    assert_eq!(row.format, "anthropic");
}

#[test]
fn a_file_without_the_column_gets_it_at_open_and_keeps_its_rows() {
    let dir = tempfile::tempdir().expect("dir");
    drop(Store::open(dir.path(), JournalMode::Delete).expect("open"));
    let path = dir.path().join(DB_FILE);
    {
        let c = rusqlite::Connection::open(&path).expect("open");
        c.execute(
            "INSERT INTO connections (name, provider, base_url, created_at, updated_at) VALUES ('old', 'openai', 'https://example.com/v1', 1, 1)",
            [],
        )
        .expect("insert");
        c.execute_batch("ALTER TABLE connections DROP COLUMN responses;")
            .expect("drop");
    }
    let store = Store::open(dir.path(), JournalMode::Delete).expect("repair");
    let rows = store.read(conn::connections).expect("read");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].format, "openai");
}
