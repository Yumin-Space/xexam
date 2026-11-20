//! v2.1 thinking-strength family per connection: the connection answer and bodies (`effortFamily`), the effective level of
//! a model call (the stored choice of the connection's family, else the model default), the AI Chat strength of an
//! administrator (`effort` next to `model`, `chats.pick_effort`, `pickEffort`), and the refusal memo per model.
//! Everything is made up.
mod common;

use common::*;
use serde_json::{json, Value};

const EFFORT: &str = "/api/ai/admin/models/effort";
const SAVE: &str = "/api/ai/admin/connections/save";

const NAMED: &str = r#"{"error":{"message":"Unrecognized request argument supplied: reasoning_effort","type":"invalid_request_error","param":"reasoning_effort"}}"#;

struct E {
    t: TestApp,
    admin: Who,
    user: Who,
}

async fn env() -> E {
    let t = start().await;
    let admin = t.user("root", true, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let user = t.user("lee", false, true);
    t.relax_caps();
    E { t, admin, user }
}

async fn free_on(e: &E) {
    let (c, v) =
        e.t.post("/api/ai/free", &e.admin, json!({"free": true}))
            .await;
    assert_eq!(c, 200, "{v}");
}

fn calls_from(t: &TestApp, from: usize) -> Vec<Value> {
    t.stub
        .requests()
        .into_iter()
        .skip(from)
        .filter(|r| !r.path.ends_with("/models"))
        .map(|r| r.body)
        .collect()
}

/// The level a body carries, whatever the family writes it as.
fn level_of(b: &Value) -> Option<String> {
    b["reasoning_effort"]
        .as_str()
        .or_else(|| b["reasoning"]["effort"].as_str())
        .or_else(|| b["output_config"]["effort"].as_str())
        .or_else(|| b["extra_body"]["google"]["thinking_config"]["thinking_level"].as_str())
        .map(str::to_string)
}

async fn settings(t: &TestApp, admin: &Who) -> Value {
    let (s, v) = t.get("/api/ai/admin/settings", admin).await;
    assert_eq!(s, 200, "{v}");
    v
}

async fn chat_read(t: &TestApp, who: &Who, chat: i64) -> Value {
    let (c, v) = t.get(&format!("/api/ai/chat?id={chat}"), who).await;
    assert_eq!(c, 200, "{v}");
    v
}

/// Sends as `who`; returns the chat id, the answer id, the stream events and the model calls it made.
async fn send(
    e: &E,
    who: &Who,
    chat: Option<i64>,
    extra: Value,
) -> Result<(i64, i64, Vec<(String, Value)>, Vec<Value>), (u16, Value)> {
    let mut body = json!({"text": "hello", "lang": "en"});
    if let Some(c) = chat {
        body["chatId"] = json!(c);
    }
    for (k, v) in extra.as_object().expect("object") {
        body[k] = v.clone();
    }
    let from = e.t.stub.count();
    let ev = e.t.send_all(who, body).await?;
    let start = ev[0].1.clone();
    Ok((
        start["chatId"].as_i64().expect("chat"),
        start["assistantMessageId"].as_i64().unwrap_or(0),
        ev,
        calls_from(&e.t, from),
    ))
}

// —— connections ——————————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn connection_save_takes_and_answers_the_family() {
    let e = env().await;
    let url = format!("{}/open/v1", e.t.stub.url);
    // create without a family: inferred (OpenAI wire, no Gemini model names)
    let (c, v) =
        e.t.post(
            SAVE,
            &e.admin,
            json!({"name": "second", "provider": "openai", "baseUrl": url, "apiKey": "sk-demo-1111"}),
        )
        .await;
    assert_eq!(c, 200, "{v}");
    assert_eq!(v["saved"], true, "{v}");
    let id2 = v["connectionId"].as_i64().expect("id");
    let by_id = |s: &Value, id: i64| -> Value {
        s["connections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == json!(id))
            .cloned()
            .expect("connection")
    };
    assert_eq!(by_id(&v["settings"], id2)["effortFamily"], "openai");
    // create with a family: kept
    let (c, v) =
        e.t.post(
            SAVE,
            &e.admin,
            json!({"name": "third", "provider": "openai", "baseUrl": url, "apiKey": "sk-demo-1111",
                   "effortFamily": "google"}),
        )
        .await;
    assert_eq!(c, 200, "{v}");
    let id3 = v["connectionId"].as_i64().expect("id");
    assert_eq!(by_id(&v["settings"], id3)["effortFamily"], "google");
    // an Anthropic connection is inferred as Anthropic
    let (c, v) =
        e.t.post(
            SAVE,
            &e.admin,
            json!({"name": "fourth", "provider": "anthropic",
                   "baseUrl": format!("{}/anthropic", e.t.stub.url), "apiKey": "sk-demo-1111"}),
        )
        .await;
    assert_eq!(c, 200, "{v}");
    if v["saved"] == json!(true) {
        let id4 = v["connectionId"].as_i64().expect("id");
        assert_eq!(by_id(&v["settings"], id4)["effortFamily"], "anthropic");
    }
    // update: no family keeps it, a family changes it (name-only save: no test), the slot efforts stay untouched
    let s = settings(&e.t, &e.admin).await;
    let rev = by_id(&s, id3)["rev"].as_i64().expect("rev");
    let (c, v) =
        e.t.post(
            SAVE,
            &e.admin,
            json!({"id": id3, "rev": rev, "name": "third", "provider": "openai", "baseUrl": url}),
        )
        .await;
    assert_eq!(c, 200, "{v}");
    assert_eq!(by_id(&v["settings"], id3)["effortFamily"], "google");
    let rev = by_id(&v["settings"], id3)["rev"].as_i64().expect("rev");
    let (c, v) =
        e.t.post(
            SAVE,
            &e.admin,
            json!({"id": id3, "rev": rev, "name": "third", "provider": "openai", "baseUrl": url,
                   "effortFamily": "responses"}),
        )
        .await;
    assert_eq!(c, 200, "{v}");
    assert_eq!(by_id(&v["settings"], id3)["effortFamily"], "responses");
    // the event names the family and what changed
    e.t.app.0.deliver_outbox().await;
    let evs: Vec<Value> =
        e.t.backend
            .st()
            .activity
            .iter()
            .filter(|v| v["action"] == json!("ai.settings.connectionUpdate"))
            .cloned()
            .collect();
    let last = evs.last().expect("an update event");
    assert_eq!(last["detail"]["family"], "responses");
    assert!(last["detail"]["changed"]
        .as_array()
        .unwrap()
        .contains(&json!("effortFamily")));
    // bad values
    for bad in [
        json!("Google"),
        json!("gemma"),
        json!("deepseek"),
        json!(""),
        json!(3),
        json!(["openai"]),
    ] {
        let (c, v) =
            e.t.post(
                SAVE,
                &e.admin,
                json!({"name": "x", "provider": "openai", "baseUrl": url, "apiKey": "sk-demo-1111",
                       "effortFamily": bad}),
            )
            .await;
        assert_eq!(c, 400, "{bad}: {v}");
    }
}

#[tokio::test]
async fn a_family_change_keeps_stored_slot_efforts_and_shows_the_default() {
    let e = env().await;
    let (c, _) =
        e.t.post(EFFORT, &e.admin, json!({"slot": "chat", "level": "xhigh"}))
            .await;
    assert_eq!(c, 200);
    let url = format!("{}/v1", e.t.stub.url);
    let s = settings(&e.t, &e.admin).await;
    let rev = s["connections"][0]["rev"].as_i64().expect("rev");
    let (c, v) =
        e.t.post(
            SAVE,
            &e.admin,
            json!({"id": 1, "rev": rev, "name": "main", "provider": "openai", "baseUrl": url,
                   "effortFamily": "anthropic"}),
        )
        .await;
    assert_eq!(c, 200, "{v}");
    let slot = &v["settings"]["slots"]["chat"];
    assert_eq!(
        slot["effort"],
        json!({"family": "openai", "level": "xhigh"})
    );
    assert_eq!(
        slot["effortEffective"],
        json!({"family": "anthropic", "level": "medium"})
    );
    // the call sends the Anthropic parameter with the model default
    let (_, _, ev, calls) = send(&e, &e.user, None, json!({})).await.expect("send");
    assert_eq!(terminal(&ev).0, "done");
    assert!(!calls.is_empty());
    for b in &calls {
        assert_eq!(b["output_config"]["effort"], "medium", "{b}");
        assert!(b.get("reasoning_effort").is_none(), "{b}");
    }
}

// —— AI Chat strength (administrators, free mode) ————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn chat_effort_is_sent_stored_and_read() {
    let e = env().await;
    free_on(&e).await;
    // a send with a level: the call carries it, the chat stores it, the read shows it
    let (chat, answer, ev, calls) = send(&e, &e.admin, None, json!({"effort": "xhigh"}))
        .await
        .expect("send");
    assert_eq!(terminal(&ev).0, "done", "{ev:?}");
    assert!(calls
        .iter()
        .all(|b| level_of(b).as_deref() == Some("xhigh")));
    assert_eq!(
        chat_read(&e.t, &e.admin, chat).await["chat"]["pickEffort"],
        "xhigh"
    );
    // no `effort` key: the stored level is kept and used
    let (_, _, _, calls) = send(&e, &e.admin, Some(chat), json!({}))
        .await
        .expect("send");
    assert!(calls
        .iter()
        .all(|b| level_of(b).as_deref() == Some("xhigh")));
    // regenerate and edit take the field too
    let from = e.t.stub.count();
    let mut s =
        e.t.open_stream(
            "/api/ai/regenerate",
            &e.admin,
            json!({"chatId": chat, "messageId": answer, "lang": "en", "effort": "low"}),
        )
        .await
        .expect("regenerate");
    let ev = s.all().await;
    assert_eq!(terminal(&ev).0, "done", "{ev:?}");
    assert!(calls_from(&e.t, from)
        .iter()
        .all(|b| level_of(b).as_deref() == Some("low")));
    assert_eq!(
        chat_read(&e.t, &e.admin, chat).await["chat"]["pickEffort"],
        "low"
    );
    // null clears it: the model default (unknown model: medium) goes out
    let (_, _, _, calls) = send(&e, &e.admin, Some(chat), json!({"effort": Value::Null}))
        .await
        .expect("send");
    assert!(calls
        .iter()
        .all(|b| level_of(b).as_deref() == Some("medium")));
    assert_eq!(
        chat_read(&e.t, &e.admin, chat).await["chat"]["pickEffort"],
        Value::Null
    );
}

#[tokio::test]
async fn chat_effort_is_checked_and_gated() {
    let e = env().await;
    // outside free mode a level is refused (null is fine)
    let (c, v) = send(&e, &e.admin, None, json!({"effort": "high"}))
        .await
        .expect_err("not free");
    assert_eq!((c, v["code"].as_str()), (400, Some("ai.badRequest")), "{v}");
    send(&e, &e.admin, None, json!({"effort": Value::Null}))
        .await
        .expect("null is fine");
    free_on(&e).await;
    // a level of another family, or garbage: 400 and no call
    let before = e.t.stub.count();
    for bad in [
        json!("HIGH"),
        json!("none"),
        json!("High"),
        json!(""),
        json!(5),
    ] {
        let (c, v) = send(&e, &e.admin, None, json!({"effort": bad}))
            .await
            .expect_err("bad level");
        assert_eq!(
            (c, v["code"].as_str()),
            (400, Some("ai.badRequest")),
            "{bad}: {v}"
        );
    }
    assert_eq!(e.t.stub.count(), before, "nothing was called");
    // a plain user may not name a level, but may send null
    let (c, v) = send(&e, &e.user, None, json!({"effort": "high"}))
        .await
        .expect_err("user");
    assert_eq!((c, v["code"].as_str()), (403, Some("ai.notAllowed")), "{v}");
    let (_, _, ev, calls) = send(&e, &e.user, None, json!({"effort": Value::Null}))
        .await
        .expect("null");
    assert_eq!(terminal(&ev).0, "done");
    assert!(calls
        .iter()
        .all(|b| level_of(b).as_deref() == Some("medium")));
    // the read of a plain user never shows it
    let (chat, _, _, _) = send(&e, &e.user, None, json!({})).await.expect("send");
    assert!(chat_read(&e.t, &e.user, chat).await["chat"]
        .get("pickEffort")
        .is_none());
}

#[tokio::test]
async fn a_stored_level_of_another_family_is_ignored_at_call_time() {
    let e = env().await;
    free_on(&e).await;
    let (chat, _, _, _) = send(&e, &e.admin, None, json!({"effort": "max"}))
        .await
        .expect("send");
    // the connection becomes Google: "max" is no Google level; no error, the model default goes out
    e.t.db()
        .execute(
            "UPDATE connections SET effort_family = 'google', rev = rev + 1 WHERE id = 1",
            [],
        )
        .expect("family");
    let (_, _, ev, calls) = send(&e, &e.admin, Some(chat), json!({}))
        .await
        .expect("send");
    assert_eq!(terminal(&ev).0, "done", "{ev:?}");
    assert!(calls
        .iter()
        .all(|b| level_of(b).as_deref() == Some("MEDIUM")));
    // garbage in the column is ignored the same way
    e.t.db()
        .execute("UPDATE chats SET pick_effort = 'turbo'", [])
        .expect("garbage");
    let (_, _, ev, _) = send(&e, &e.admin, Some(chat), json!({}))
        .await
        .expect("send");
    assert_eq!(terminal(&ev).0, "done", "{ev:?}");
}

#[tokio::test]
async fn the_chat_model_route_stores_the_level() {
    let e = env().await;
    let (chat, _, _, _) = send(&e, &e.admin, None, json!({})).await.expect("send");
    // outside free mode a level is refused
    let (c, v) =
        e.t.post(
            "/api/ai/chat/model",
            &e.admin,
            json!({"chatId": chat, "model": Value::Null, "effort": "high"}),
        )
        .await;
    assert_eq!((c, v["code"].as_str()), (400, Some("ai.badRequest")), "{v}");
    free_on(&e).await;
    let big = json!({"connectionId": 1, "model": "demo-chat-large"});
    let (c, v) =
        e.t.post(
            "/api/ai/chat/model",
            &e.admin,
            json!({"chatId": chat, "model": big, "effort": "xhigh"}),
        )
        .await;
    assert_eq!(c, 200, "{v}");
    assert_eq!(v["chat"]["pick"], big);
    assert_eq!(v["chat"]["pickEffort"], "xhigh");
    // clearing the pick keeps the level unless `effort: null` is sent too
    let (_, v) =
        e.t.post(
            "/api/ai/chat/model",
            &e.admin,
            json!({"chatId": chat, "model": Value::Null}),
        )
        .await;
    assert_eq!(v["chat"]["pick"], Value::Null);
    assert_eq!(v["chat"]["pickEffort"], "xhigh");
    let (_, v) =
        e.t.post(
            "/api/ai/chat/model",
            &e.admin,
            json!({"chatId": chat, "model": Value::Null, "effort": Value::Null}),
        )
        .await;
    assert_eq!(v["chat"]["pickEffort"], Value::Null);
    // an unknown level for the family of the pick is a 400 and changes nothing
    let (c, v) =
        e.t.post(
            "/api/ai/chat/model",
            &e.admin,
            json!({"chatId": chat, "model": big, "effort": "HIGH"}),
        )
        .await;
    assert_eq!((c, v["code"].as_str()), (400, Some("ai.badRequest")), "{v}");
    assert_eq!(
        chat_read(&e.t, &e.admin, chat).await["chat"]["pickEffort"],
        Value::Null
    );
    // a plain user may not
    let (c, _) =
        e.t.post(
            "/api/ai/chat/model",
            &e.user,
            json!({"chatId": chat, "model": Value::Null, "effort": "low"}),
        )
        .await;
    assert_eq!(c, 403);
}

// —— the refusal memo is per model ————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn a_refused_strength_is_remembered_per_model_not_per_connection() {
    let e = env().await;
    free_on(&e).await;
    e.t.stub
        .refuse_if_body_has(Some(("reasoning_effort", 400, NAMED)));
    let large = json!({"model": {"connectionId": 1, "model": "demo-chat-large"}});
    // the first call with the large model: refused once, retried without the parameter
    let (chat, _, ev, calls) = send(&e, &e.admin, None, large.clone()).await.expect("send");
    assert_eq!(terminal(&ev).0, "done", "{ev:?}");
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(level_of(&calls[0]).is_some() && level_of(&calls[1]).is_none());
    // remembered for this model
    let (_, _, _, calls) = send(&e, &e.admin, Some(chat), large).await.expect("send");
    assert_eq!(calls.len(), 1);
    assert!(level_of(&calls[0]).is_none());
    // the default model of the same connection still tries the parameter first
    let (_, _, ev, calls) = send(&e, &e.admin, None, json!({"model": Value::Null}))
        .await
        .expect("send");
    assert_eq!(terminal(&ev).0, "done", "{ev:?}");
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(level_of(&calls[0]).is_some() && level_of(&calls[1]).is_none());
    // a settings change forgets all of it
    let (c, _) =
        e.t.post(EFFORT, &e.admin, json!({"slot": "chat", "level": "low"}))
            .await;
    assert_eq!(c, 200);
    let (_, _, _, calls) = send(
        &e,
        &e.admin,
        None,
        json!({"model": {"connectionId": 1, "model": "demo-chat-large"}}),
    )
    .await
    .expect("send");
    assert_eq!(calls.len(), 2, "tried again after the save: {calls:?}");
}
