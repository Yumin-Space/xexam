//! v1.9 per-connection on/off switch and independent global feature switches: an OFF connection keeps its settings and
//! choices, is left out of the pickers, and the features that use it fail with `ai.connectionOff` (409) until it is on
//! again. Everything is made up.
mod common;

#[path = "study_api/util.rs"]
mod util;

use serde_json::{json, Value};
use util::*;

const KEY2: &str = "sk-demo-1111-2222";
const ENABLED: &str = "/api/ai/admin/connections/enabled";

/// An address on the stub that accepts any key.
fn open(t: &TestApp) -> String {
    format!("{}/open/v1", t.stub.url)
}

async fn settings(t: &TestApp, admin: &Who) -> Value {
    let (s, v) = t.get("/api/ai/admin/settings", admin).await;
    assert_eq!(s, 200, "{v}");
    v
}

fn conn_of(s: &Value, id: i64) -> &Value {
    s["connections"]
        .as_array()
        .expect("connections")
        .iter()
        .find(|c| c["id"] == json!(id))
        .expect("connection")
}

/// The test environment plus a second connection (id 2) that serves the hint slot.
async fn env_two() -> Env {
    let e = env().await;
    let (s, v) =
        e.t.post(
            "/api/ai/admin/connections/save",
            &e.admin,
            json!({"name": "second", "provider": "openai", "baseUrl": open(&e.t), "apiKey": KEY2}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["connectionId"], json!(2));
    let (s, v) =
        e.t.post(
            "/api/ai/admin/models/pick",
            &e.admin,
            json!({"slot": "hint", "connectionId": 2, "model": "demo-chat-large", "manual": false}),
        )
        .await;
    assert_eq!((s, v["ok"].clone()), (200, json!(true)), "{v}");
    e
}

async fn switch(e: &Env, id: i64, on: bool) -> (u16, Value) {
    e.t.post(ENABLED, &e.admin, json!({"id": id, "enabled": on}))
        .await
}

fn stored_pick(t: &TestApp, chat: i64) -> (Option<i64>, Option<String>) {
    t.db()
        .query_row(
            "SELECT pick_conn, pick_model FROM chats WHERE id = ?1",
            [chat],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("pick")
}

#[tokio::test]
async fn every_connection_starts_on_and_the_switch_keeps_everything() {
    let e = env_two().await;
    let s = settings(&e.t, &e.admin).await;
    assert_eq!(conn_of(&s, 1)["enabled"], json!(true));
    assert_eq!(conn_of(&s, 2)["enabled"], json!(true));
    assert_eq!(s["slots"].as_object().expect("slots").len(), 8);
    assert!(s["slots"].get("study").is_none());
    let rev = conn_of(&s, 2)["rev"].clone();
    let (st, v) = switch(&e, 2, false).await;
    assert_eq!(st, 200, "{v}");
    let c = conn_of(&v["settings"], 2);
    assert_eq!(c["enabled"], json!(false));
    assert_eq!(c["rev"], rev, "the switch does not bump the revision");
    assert_eq!(c["keys"]["apiKey"]["set"], json!(true), "the key is kept");
    assert_eq!(c["usedBy"], json!(["hint"]), "the choice is kept");
    assert_eq!(c["status"], conn_of(&s, 2)["status"]);
    // a second identical call answers 200 as well
    let (st, v) = switch(&e, 2, false).await;
    assert_eq!(st, 200, "{v}");
    // the slot that uses it is not usable now, and the others still are
    let s = settings(&e.t, &e.admin).await;
    assert_eq!(s["slots"]["hint"]["using"], Value::Null);
    assert_ne!(s["slots"]["explain"]["using"], Value::Null);
    assert!(s["slots"]["hint"]["modelId"].is_i64(), "the model is kept");
    // back on
    let (st, v) = switch(&e, 2, true).await;
    assert_eq!(st, 200, "{v}");
    let s = settings(&e.t, &e.admin).await;
    assert_ne!(s["slots"]["hint"]["using"], Value::Null);
}

#[tokio::test]
async fn an_off_connection_is_listed_without_models_and_without_a_request() {
    let e = env_two().await;
    switch(&e, 2, false).await;
    let before = e.t.stub.count();
    let (s, v) = e.t.get("/api/ai/admin/models?refresh=1", &e.admin).await;
    assert_eq!(s, 200, "{v}");
    let list = v["connections"].as_array().expect("connections");
    let off = list.iter().find(|c| c["id"] == json!(2)).expect("second");
    assert_eq!(off["enabled"], json!(false));
    assert_eq!(off["models"], json!([]));
    assert_eq!(off["listed"], json!(false));
    assert_eq!(off["error"], Value::Null);
    let on = list.iter().find(|c| c["id"] == json!(1)).expect("main");
    assert_eq!(on["enabled"], json!(true));
    assert!(!on["models"].as_array().expect("models").is_empty());
    // only the connection that is on was asked
    let asked = e.t.stub.requests().into_iter().skip(before).count();
    assert_eq!(asked, 1, "one model listing, none for the OFF connection");
}

#[tokio::test]
async fn picks_on_an_off_connection_are_refused() {
    let e = env_two().await;
    switch(&e, 2, false).await;
    for slot in ["chat", "hint"] {
        let (s, v) =
            e.t.post(
                "/api/ai/admin/models/pick",
                &e.admin,
                json!({"slot": slot, "connectionId": 2, "model": "demo-chat-large"}),
            )
            .await;
        assert_eq!(
            (s, v["code"].clone()),
            (409, json!("ai.connectionOff")),
            "{v}"
        );
    }
    // the removed slot is a plain bad request
    let (s, _) =
        e.t.post(
            "/api/ai/admin/models/pick",
            &e.admin,
            json!({"slot": "study", "connectionId": 1, "model": "demo-chat-small"}),
        )
        .await;
    assert_eq!(s, 400);
}

#[tokio::test]
async fn a_feature_on_an_off_connection_fails_and_works_again_when_it_is_back_on() {
    let e = env_two().await;
    switch(&e, 2, false).await;
    let (s, v) = refused(&e.t, &e.user, STUDY_ASK, hint_body(K1, "q1", 1)).await;
    assert_eq!(
        (s, v["code"].clone()),
        (409, json!("ai.connectionOff")),
        "{v}"
    );
    // explain follows the default (connection 1): it works
    let ev = ask(&e.t, &e.user, explain_body(K1, "q1", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    // the user's status reports the hint as not usable
    let (s, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["features"]["hint"], json!(false), "{v}");
    assert_eq!(v["features"]["explain"], json!(true), "{v}");
    switch(&e, 2, true).await;
    let ev = ask(&e.t, &e.user, hint_body(K1, "q1", 2)).await;
    assert_eq!(terminal_name(&ev), "done");
}

#[tokio::test]
async fn a_slot_that_follows_an_off_default_fails_without_a_fallback() {
    let e = env_two().await;
    // connection 1 holds the default: explain and chat follow it
    switch(&e, 1, false).await;
    let (s, v) = refused(&e.t, &e.user, STUDY_ASK, explain_body(K1, "q1", 1)).await;
    assert_eq!(
        (s, v["code"].clone()),
        (409, json!("ai.connectionOff")),
        "{v}"
    );
    // hint has its own model on connection 2
    let ev = ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    // connection 1 still lists its models for the settings page
    let s = settings(&e.t, &e.admin).await;
    assert_eq!(s["slots"]["default"]["using"], Value::Null);
    assert_eq!(s["state"], json!("on"), "another slot still resolves");
}

#[tokio::test]
async fn an_admin_chat_with_a_stored_pick_on_an_off_connection_fails_and_keeps_the_pick() {
    let e = env_two().await;
    let (c, _) =
        e.t.post("/api/ai/free", &e.admin, json!({"free": true}))
            .await;
    assert_eq!(c, 200);
    let ev = ask_to(
        &e.t,
        &e.admin,
        "/api/ai/send",
        json!({"text": "hello", "lang": "en"}),
    )
    .await;
    let chat = ev[0].1["chatId"].as_i64().expect("chat");
    let (c, v) =
        e.t.post(
            "/api/ai/chat/model",
            &e.admin,
            json!({"chatId": chat, "model": {"connectionId": 2, "model": "demo-chat-large"}}),
        )
        .await;
    assert_eq!(c, 200, "{v}");
    switch(&e, 2, false).await;
    for (path, body) in [
        (
            "/api/ai/send",
            json!({"chatId": chat, "text": "again", "lang": "en"}),
        ),
        (
            "/api/ai/send",
            json!({"text": "x", "lang": "en", "model": {"connectionId": 2, "model": "demo-chat-large"}}),
        ),
    ] {
        let (s, v) = refused(&e.t, &e.admin, path, body).await;
        assert_eq!(
            (s, v["code"].clone()),
            (409, json!("ai.connectionOff")),
            "{v}"
        );
    }
    // picking it in the menu is refused too
    let (c, v) =
        e.t.post(
            "/api/ai/chat/model",
            &e.admin,
            json!({"chatId": chat, "model": {"connectionId": 2, "model": "demo-chat-large"}}),
        )
        .await;
    assert_eq!(
        (c, v["code"].clone()),
        (409, json!("ai.connectionOff")),
        "{v}"
    );
    assert_eq!(
        stored_pick(&e.t, chat),
        (Some(2), Some("demo-chat-large".into()))
    );
    // back on: the same chat answers from the picked model
    switch(&e, 2, true).await;
    let ev = ask_to(
        &e.t,
        &e.admin,
        "/api/ai/send",
        json!({"chatId": chat, "text": "once more", "lang": "en"}),
    )
    .await;
    assert_eq!(terminal_name(&ev), "done");
    assert_eq!(e.t.stub.last().body["model"], json!("demo-chat-large"));
}

#[tokio::test]
async fn the_switch_body_is_strict_and_leaves_one_event_each_way() {
    let e = env_two().await;
    for body in [
        json!({}),
        json!({"enabled": true}),
        json!({"id": 2}),
        json!({"id": 2, "enabled": "false"}),
        json!({"id": "2", "enabled": false}),
        json!({"id": 0, "enabled": false}),
        json!({"id": 2, "enabled": false, "extra": 1}),
    ] {
        let (s, v) = e.t.post(ENABLED, &e.admin, body.clone()).await;
        assert_eq!(
            (s, v["code"].clone()),
            (400, json!("ai.badRequest")),
            "{body}"
        );
    }
    let (s, v) = switch(&e, 99, false).await;
    assert_eq!((s, v["code"].clone()), (404, json!("ai.notFound")), "{v}");
    // a user may not
    let (s, v) =
        e.t.post(ENABLED, &e.user, json!({"id": 2, "enabled": false}))
            .await;
    assert_eq!((s, v["code"].clone()), (403, json!("ai.adminOnly")), "{v}");
    switch(&e, 2, false).await;
    switch(&e, 2, true).await;
    e.t.app.0.deliver_outbox().await;
    let acts: Vec<String> =
        e.t.backend
            .st()
            .activity
            .iter()
            .filter_map(|a| a["action"].as_str().map(str::to_string))
            .filter(|a| a.starts_with("ai.settings.connection"))
            .collect();
    let on = acts
        .iter()
        .filter(|a| *a == "ai.settings.connectionOn")
        .count();
    let off = acts
        .iter()
        .filter(|a| *a == "ai.settings.connectionOff")
        .count();
    assert_eq!((on, off), (1, 1), "{acts:?}");
}

#[tokio::test]
async fn global_switches_are_independent_and_apply_to_administrators() {
    let e = env().await;
    let (s, v) =
        e.t.post(
            "/api/ai/admin/feature-switches",
            &e.admin,
            json!({"hint": false}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["featureSwitches"]["hint"], json!(false));
    assert_eq!(v["featureSwitches"]["explain"], json!(true));
    // an administrator is refused, and explain still works for them
    let (s, v) = refused(&e.t, &e.admin, STUDY_ASK, hint_body(K2, "q1", 1)).await;
    assert_eq!((s, v["code"].clone()), (403, json!("ai.featureOff")), "{v}");
    let ev = ask(&e.t, &e.admin, explain_body(K2, "q1", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    // the administrator's status follows the global switch
    let (s, v) = e.t.get("/api/ai/status", &e.admin).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["features"]["hint"], json!(false), "{v}");
    assert_eq!(v["features"]["explain"], json!(true), "{v}");
    // the per-user boxes still pair hint and explain
    let (s, v) =
        e.t.post(
            "/api/ai/admin/features",
            &e.admin,
            json!({"userId": e.user.id, "features": {"chat": true, "hint": false, "explain": true,
                   "summary": true, "assess": true, "generate": true, "import": true}}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let f = &v["user"]["features"];
    assert_eq!(
        (f["hint"].clone(), f["explain"].clone()),
        (json!(false), json!(false)),
        "{v}"
    );
}

#[tokio::test]
async fn an_admin_pick_on_an_on_connection_works_while_the_chat_default_is_off() {
    let e = env_two().await;
    let (c, _) =
        e.t.post("/api/ai/free", &e.admin, json!({"free": true}))
            .await;
    assert_eq!(c, 200);
    // the chat slot follows connection 1; switching it off leaves connection 2 (hint slot) on
    let (s, v) = switch(&e, 1, false).await;
    assert_eq!(s, 200, "{v}");
    // no pick: the default route is off
    let (s, v) = refused(
        &e.t,
        &e.admin,
        "/api/ai/send",
        json!({"text": "x", "lang": "en"}),
    )
    .await;
    assert_eq!(
        (s, v["code"].clone()),
        (409, json!("ai.connectionOff")),
        "{v}"
    );
    // a pick on the connection that is on works
    let ev = ask_to(
        &e.t,
        &e.admin,
        "/api/ai/send",
        json!({"text": "hello", "lang": "en", "model": {"connectionId": 2, "model": "demo-chat-large"}}),
    )
    .await;
    assert_eq!(terminal_name(&ev), "done");
    assert_eq!(e.t.stub.last().body["model"], json!("demo-chat-large"));
}

#[tokio::test]
async fn effort_family_is_reported_even_when_the_connection_is_off() {
    let e = env_two().await;
    let s = settings(&e.t, &e.admin).await;
    assert_eq!(s["slots"]["hint"]["effortFamily"], json!("openai"));
    assert_eq!(s["slots"]["chat"]["effortFamily"], json!("openai"));
    switch(&e, 2, false).await;
    let s = settings(&e.t, &e.admin).await;
    assert_eq!(s["slots"]["hint"]["using"], Value::Null);
    assert_eq!(
        s["slots"]["hint"]["effortFamily"],
        json!("openai"),
        "the target connection's family stays visible"
    );
    // the family is the one POST models/effort checks against
    let (st, v) =
        e.t.post(
            "/api/ai/admin/models/effort",
            &e.admin,
            json!({"slot": "hint", "level": "low"}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
}
