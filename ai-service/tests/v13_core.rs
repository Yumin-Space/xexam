//! v1.3 core: the model routes read from the database, who may use what (admins and the global switches, the caps, the
//! daily budget), the health of a connection, the activity events and their delivery, the route table. Everything is
//! made up.
mod common;

use common::*;
use serde_json::{json, Value};
use xexam_ai_service::activity::{self, Event};
use xexam_ai_service::chat::Kind;
use xexam_ai_service::features::Feature;
use xexam_ai_service::identity::{ClientInfo, Identity, Role};
use xexam_ai_service::provider::UpCode;
use xexam_ai_service::route::{Fallback, Pick, Slot};
use xexam_ai_service::store::conn;

fn identity(w: &Who) -> Identity {
    Identity {
        id: w.id,
        stamp: w.stamp.clone(),
        username: w.username.clone(),
        role: if w.admin { Role::Admin } else { Role::User },
        ai_allowed: w.ai || w.admin,
        sid: w.sid.clone(),
    }
}

fn count(t: &TestApp, sql: &str) -> i64 {
    t.db().query_row(sql, [], |r| r.get(0)).expect("count")
}

/// Adds a pool model to connection 1 and returns its pool id.
fn add_model(t: &TestApp, model: &str, status: &str) -> i64 {
    t.app
        .0
        .store
        .write(|tx| {
            conn::upsert_pool(
                tx,
                &conn::NewPool {
                    connection_id: 1,
                    model,
                    manual: false,
                    status,
                    error_code: None,
                    usage_option: true,
                    max_tokens: 8192,
                    context: Some(32_000),
                    latency_ms: Some(5),
                    tested_at: Some(1),
                },
            )
        })
        .expect("model")
}

fn set_slot(t: &TestApp, slot: &str, pool: Option<i64>) {
    t.app
        .0
        .store
        .write(|tx| conn::set_slot(tx, slot, pool, "test"))
        .expect("slot");
}

fn set_global_off(t: &TestApp, bits: i64) {
    t.app
        .0
        .store
        .write(|tx| conn::set_features_off(tx, bits))
        .expect("bits");
}

// —— the routes ———————————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn a_route_is_the_pick_then_the_slot_then_the_default_and_never_another_model() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    let app = &t.app.0;
    // only the default is set: every slot falls back to it
    for s in Slot::ALL {
        let r = app.route(s, None).expect("route");
        assert_eq!(r.model, "demo-chat-small");
        assert_eq!(
            r.fallback,
            if s == Slot::Default {
                Fallback::None
            } else {
                Fallback::Default
            }
        );
        assert_eq!((r.connection_id, r.connection_name.as_str()), (1, "main"));
    }
    // a slot with its own model
    let large = add_model(&t, "demo-chat-large", "ok");
    set_slot(&t, "chat", Some(large));
    let r = app.route(Slot::Chat, None).expect("chat");
    assert_eq!(
        (r.model.as_str(), r.fallback, r.pool_id),
        ("demo-chat-large", Fallback::None, Some(large))
    );
    assert_eq!(r.context, Some(32_000));
    assert!(r.key.starts_with("c1r"), "{}", r.key);
    // its own model fails: the default is used (a warning in the settings, no other model)
    t.db()
        .execute(
            "UPDATE models SET status = 'failed' WHERE model = 'demo-chat-large'",
            [],
        )
        .expect("fail");
    let r = app.route(Slot::Chat, None).expect("chat");
    assert_eq!(
        (r.model.as_str(), r.fallback),
        ("demo-chat-small", Fallback::Default)
    );
    // the default fails too: nothing resolves for chat, although a third usable model sits in the pool
    add_model(&t, "demo-third", "ok");
    t.db()
        .execute(
            "UPDATE models SET status = 'failed' WHERE model = 'demo-chat-small'",
            [],
        )
        .expect("fail");
    assert_eq!(
        app.route(Slot::Chat, None).err().map(|e| e.code),
        Some("ai.disabled")
    );
    assert_eq!(
        app.route(Slot::Default, None).err().map(|e| e.code),
        Some("ai.disabled")
    );
    assert!(!app.route_exists(Slot::Chat));
    assert!(!app.effective().expect("effective"), "no slot resolves");
    // a slot whose own model is fine still works, and AI is effective again because one slot resolves
    t.db()
        .execute(
            "UPDATE models SET status = 'ok' WHERE model = 'demo-chat-large'",
            [],
        )
        .expect("ok");
    assert!(app.route_exists(Slot::Chat));
    assert!(!app.route_exists(Slot::Study));
    assert!(app.effective().expect("effective"));
}

#[tokio::test]
async fn an_admin_pick_may_name_a_model_outside_the_pool_and_a_dead_pick_is_dropped() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    let app = &t.app.0;
    let pick = Pick::parse_text("1:vendor:demo-model:v2").expect("pick");
    assert_eq!(pick.model, "vendor:demo-model:v2");
    let r = app.route(Slot::Chat, Some(&pick)).expect("pick");
    assert_eq!(r.model, "vendor:demo-model:v2");
    assert_eq!((r.pool_id, r.usage_option, r.max_tokens), (None, 1, 8192));
    assert!(!r.pick_dropped);
    // the connection does not exist: the chat default is used and the pick is reported as dropped
    let gone = Pick {
        connection_id: 9,
        model: "x".into(),
    };
    let r = app.route(Slot::Chat, Some(&gone)).expect("chat");
    assert_eq!(
        (r.model.as_str(), r.pick_dropped),
        ("demo-chat-small", true)
    );
    // a broken connection does not take picks either
    app.mark_broken(1, "main", "ai.up.auth");
    assert!(app.route(Slot::Chat, Some(&pick)).is_err());
}

#[tokio::test]
async fn effective_needs_the_switch_the_key_state_and_a_route() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    let app = &t.app.0;
    assert!(app.effective().expect("on"));
    t.db()
        .execute("UPDATE settings SET enabled = 0", [])
        .expect("off");
    assert!(!app.effective().expect("off"));
    t.db()
        .execute("UPDATE settings SET enabled = 1", [])
        .expect("on");
    assert!(app.effective().expect("on"));
    // keys sealed with another encryption key: the model is not usable
    t.db()
        .execute(
            "UPDATE connection_secrets SET key_id = '0000000000000000'",
            [],
        )
        .expect("foreign");
    assert!(!app.effective().expect("foreign key"));
}

#[tokio::test]
async fn a_key_that_does_not_open_is_a_key_problem_and_stops_the_service() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    let app = &t.app.0;
    let route = app.route(Slot::Chat, None).expect("route");
    let target = app.target(&route).expect("target");
    assert_eq!(target.api_key.as_ref().map(|k| k.as_str()), Some(KEY));
    drop(target);
    // damaged ciphertext under the right key id
    t.db()
        .execute(
            "UPDATE connection_secrets SET ciphertext = zeroblob(33)",
            [],
        )
        .expect("damage");
    assert!(app.key_problem().is_none(), "found only when it is used");
    let e = app.target(&route).err().expect("refused");
    assert_eq!(e.code, "ai.keyProblem");
    assert!(app.key_problem().is_some());
    assert!(!app.effective().expect("effective"));
    // entering a good key and re-checking clears it
    t.seed_route(Seed {
        provider: "openai",
        url: format!("{}/v1", t.stub.url),
        api_key: Some(KEY),
        gateway_key: None,
        gateway_auth: "bearer",
        model: "demo-chat-small",
    });
    app.recheck_keys();
    assert!(app.key_problem().is_none());
    assert!(app.effective().expect("effective"));
}

// —— a connection's health ————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn three_auth_errors_in_a_row_break_a_connection_and_a_success_resets_the_count() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    let app = &t.app.0;
    let route = app.route(Slot::Chat, None).expect("route");
    let status = || -> String {
        t.db()
            .query_row("SELECT status FROM connections WHERE id = 1", [], |r| {
                r.get(0)
            })
            .expect("status")
    };
    app.note_outcome(&route, Some(UpCode::Auth));
    app.note_outcome(&route, Some(UpCode::Auth));
    // other errors change nothing, a success resets
    app.note_outcome(&route, Some(UpCode::RateLimit));
    app.note_outcome(&route, None);
    app.note_outcome(&route, Some(UpCode::Auth));
    app.note_outcome(&route, Some(UpCode::Auth));
    assert_eq!(status(), "ok");
    app.note_outcome(&route, Some(UpCode::Auth));
    assert_eq!(status(), "broken");
    // broken: nothing resolves, one event row was queued, one report is on its way
    assert!(app.route(Slot::Chat, None).is_err());
    let (action, line): (String, String) = {
        let c = t.db();
        c.query_row(
            "SELECT 'ai.system.connection', message FROM audit_outbox ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("event")
    };
    let v: Value = serde_json::from_str(&line).expect("json");
    assert_eq!(v["action"], json!(action));
    assert_eq!(
        (v["result"].clone(), v["reason"].clone()),
        (json!("fail"), json!("ai.up.auth"))
    );
    assert_eq!(v["object"], json!("main"));
    assert_eq!(v["userId"], Value::Null);
    // a second mark does not repeat the event
    let before = count(&t, "SELECT COUNT(*) FROM audit_outbox");
    app.mark_broken(1, "main", "ai.up.auth");
    assert_eq!(count(&t, "SELECT COUNT(*) FROM audit_outbox"), before);
}

#[tokio::test]
async fn live_calls_that_end_in_auth_errors_break_the_connection() {
    let t = start().await;
    t.relax_caps();
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    t.stub.force(Some((
        401,
        r#"{"error":{"message":"bad key","code":"invalid_api_key"}}"#,
    )));
    for i in 0..3 {
        let ev = t
            .send_all(&sam, json!({"text": format!("q{i}")}))
            .await
            .expect("stream");
        assert_eq!(terminal(&ev).1["code"], json!("ai.up.auth"));
    }
    t.stub.force(None);
    let status: String = t
        .db()
        .query_row("SELECT status FROM connections WHERE id = 1", [], |r| {
            r.get(0)
        })
        .expect("status");
    assert_eq!(status, "broken");
    let (s, v) = t.post("/api/ai/send", &sam, json!({"text": "again"})).await;
    assert_eq!((s, v["code"].clone()), (403, json!("ai.disabled")));
}

// —— who may use what ——————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn global_switches_stop_users_but_never_admins() {
    let t = start().await;
    t.relax_caps();
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let app = &t.app.0;
    let (a, s) = (identity(&admin), identity(&sam));
    for f in [Feature::Chat, Feature::Hint, Feature::Import] {
        assert!(app.feature_allowed(&a, f) && app.feature_allowed(&s, f));
    }
    // chat and hint off for everyone
    set_global_off(&t, Feature::Chat.bit() | Feature::Hint.bit());
    assert!(
        app.feature_allowed(&a, Feature::Chat),
        "admins are not affected"
    );
    assert!(app.feature_allowed(&a, Feature::Hint));
    assert!(!app.feature_allowed(&s, Feature::Chat));
    assert!(!app.feature_allowed(&s, Feature::Hint));
    assert!(
        app.feature_allowed(&s, Feature::Explain),
        "the other bits are untouched"
    );
    assert!(app.admit(&a, Feature::Chat).is_ok());
    assert_eq!(
        app.admit(&s, Feature::Chat).err().map(|e| e.code),
        Some("ai.featureOff")
    );
    // over HTTP: the user is refused, the admin's call runs
    let (code, v) = t.post("/api/ai/send", &sam, json!({"text": "hi"})).await;
    assert_eq!((code, v["code"].clone()), (403, json!("ai.featureOff")));
    let ev = t
        .send_all(&admin, json!({"text": "hi"}))
        .await
        .expect("admin");
    assert_eq!(terminal(&ev).0, "done");
    // the per-user bit does not apply to admins either, and still applies to users when the global one is clear
    set_global_off(&t, 0);
    t.app
        .0
        .store
        .write(|tx| {
            xexam_ai_service::store::data::set_features_off(
                tx,
                sam.id,
                &sam.stamp,
                &sam.username,
                Feature::Chat.bit(),
                1,
            )
        })
        .expect("user bit");
    let (code, v) = t.post("/api/ai/send", &sam, json!({"text": "hi"})).await;
    assert_eq!((code, v["code"].clone()), (403, json!("ai.featureOff")));
    let adm = t
        .send_all(&admin, json!({"text": "hi"}))
        .await
        .expect("admin");
    assert_eq!(terminal(&adm).0, "done");
    set_global_off(&t, Feature::Explain.bit());
    assert!(!app.feature_allowed(&s, Feature::Explain));
    assert!(app.feature_allowed(&a, Feature::Explain));
}

#[tokio::test]
async fn admit_checks_ai_suspension_the_feature_and_its_slot_in_order() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let app = &t.app.0;
    let s = identity(&sam);
    assert!(app.admit(&s, Feature::Generate).is_ok());
    // the slot of the feature has nothing and the default is gone: disabled (also for an admin)
    t.db()
        .execute("UPDATE models SET status = 'failed'", [])
        .expect("fail");
    assert_eq!(
        app.admit(&s, Feature::Generate).err().map(|e| e.code),
        Some("ai.disabled")
    );
    assert_eq!(
        app.admit(&identity(&admin), Feature::Chat)
            .err()
            .map(|e| e.code),
        Some("ai.disabled")
    );
    t.db()
        .execute("UPDATE models SET status = 'ok'", [])
        .expect("ok");
    // a model of the generate slot only: generation is admitted (its slot resolves), chat falls back to the default
    let own = add_model(&t, "demo-gen", "ok");
    set_slot(&t, "generate", Some(own));
    t.db()
        .execute(
            "UPDATE models SET status = 'failed' WHERE model = 'demo-chat-small'",
            [],
        )
        .expect("fail");
    assert!(app.admit(&s, Feature::Generate).is_ok());
    assert!(app.admit(&s, Feature::Chat).is_err());
}

#[tokio::test]
async fn admins_are_exempt_from_the_caps_only_while_the_limit_says_so() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let mut l = t.app.0.limits();
    assert_eq!(
        l.calls_admins_exempt, 1,
        "a new database starts with the exemption on"
    );
    l.calls_per_minute = 1;
    l.calls_per_hour = 5;
    l.calls_per_day = 10;
    t.app.0.set_limits(l);
    // exempt: three calls in a minute, the usage rows are still written
    for i in 0..3 {
        let ev = t
            .send_all(&admin, json!({"text": format!("a{i}")}))
            .await
            .expect("admin");
        assert_eq!(terminal(&ev).0, "done", "call {i}");
    }
    assert_eq!(
        count(
            &t,
            "SELECT COUNT(*) FROM usage WHERE user_id IS NOT NULL AND kind = 'chat'"
        ),
        3
    );
    // a user is limited by the same caps
    let ev = t.send_all(&sam, json!({"text": "s1"})).await.expect("sam");
    assert_eq!(terminal(&ev).0, "done");
    let (code, v) = t.post("/api/ai/send", &sam, json!({"text": "s2"})).await;
    assert_eq!((code, v["code"].clone()), (429, json!("ai.callLimit")));
    // the exemption off: the admin is limited like everyone
    let mut l = t.app.0.limits();
    l.calls_admins_exempt = 0;
    t.app.0.set_limits(l);
    let (code, v) = t.post("/api/ai/send", &admin, json!({"text": "a4"})).await;
    assert_eq!((code, v["code"].clone()), (429, json!("ai.callLimit")));
}

#[tokio::test]
async fn the_daily_budget_stops_users_but_not_admins_and_zero_is_off() {
    let t = start().await;
    t.relax_caps();
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let set = |budget: i64| {
        let mut l = t.app.0.limits();
        l.daily_token_budget = budget;
        t.app.0.set_limits(l);
    };
    // off (0): nothing is refused however much was used
    for i in 0..2 {
        let ev = t
            .send_all(&sam, json!({"text": format!("q{i}")}))
            .await
            .expect("sam");
        assert_eq!(terminal(&ev).0, "done");
    }
    let used = count(
        &t,
        "SELECT COALESCE(SUM(COALESCE(tokens_in, 0) + COALESCE(tokens_out, 0)), 0) FROM usage",
    );
    assert!(used > 0);
    // a budget below today's use: users are refused with ai.budget, admins carry on, and the counts are untouched
    set(used);
    let rows = count(&t, "SELECT COUNT(*) FROM usage");
    let (code, v) = t.post("/api/ai/send", &sam, json!({"text": "more"})).await;
    assert_eq!((code, v["code"].clone()), (429, json!("ai.budget")));
    assert_eq!(
        count(&t, "SELECT COUNT(*) FROM usage"),
        rows,
        "a refusal writes nothing"
    );
    let (code, v) = t
        .post("/api/ai/memories/suggest", &sam, json!({"messageId": 1}))
        .await;
    assert!(code == 429 || code == 404, "{code} {v}");
    let ev = t
        .send_all(&admin, json!({"text": "admin call"}))
        .await
        .expect("admin");
    assert_eq!(terminal(&ev).0, "done");
    // a budget above the use lets the user in again
    set(used + 1_000_000);
    let ev = t
        .send_all(&sam, json!({"text": "again"}))
        .await
        .expect("sam");
    assert_eq!(terminal(&ev).0, "done");
}

#[tokio::test]
async fn take_places_gives_back_the_places_it_took_when_it_refuses() {
    let t = start().await;
    t.relax_caps();
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let app = &t.app.0;
    let s = identity(&sam);
    set_global_off(&t, Feature::Hint.bit());
    for _ in 0..3 {
        let e = app
            .take_places(&s, Kind::Chat, None, Feature::Hint)
            .err()
            .expect("refused");
        assert_eq!(e.code, "ai.featureOff");
    }
    // nothing stayed reserved: a normal call still gets its place
    let p = app
        .take_places(&s, Kind::Chat, None, Feature::Chat)
        .expect("places");
    drop(p);
    assert!(app.running_of(sam.id).is_none());
}

// —— the history of a chat ————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn consecutive_requests_of_a_long_chat_share_their_start() {
    let t = start().await;
    t.relax_caps();
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    let mut chat = Value::Null;
    let mut seen: Vec<Vec<String>> = Vec::new();
    for i in 0..14 {
        let text = format!("{i:02}-{}", "x".repeat(1500));
        let ev = t
            .send_all(&admin, json!({"chatId": chat, "text": text, "lang": "en"}))
            .await
            .expect("stream");
        chat = ev[0].1["chatId"].clone();
        let msgs = t.stub.last().body["messages"]
            .as_array()
            .cloned()
            .expect("messages");
        seen.push(
            msgs.iter()
                .skip(1)
                .map(|m| m["content"].as_str().unwrap_or("").to_string())
                .collect(),
        );
    }
    // a request without its new question is the start of the next one, except at the moves of the window and the line
    let mut broken = 0;
    for w in seen.windows(2) {
        let old = &w[0][..w[0].len() - 1];
        if w[1].len() < old.len() || &w[1][..old.len()] != old {
            broken += 1;
        }
    }
    assert!(
        broken <= 2,
        "{broken} requests did not extend the previous one"
    );
    // the first turn of every request is a user message
    for r in &seen {
        assert!(
            r[0].starts_with(|c: char| c.is_ascii_digit()),
            "starts with a question"
        );
    }
}

// —— activity events ——————————————————————————————————————————————————————————————————————————————————————————————————

fn event(t: &TestApp, w: &Who, action: &'static str) -> Event {
    let _ = t;
    Event::new(
        Some(&identity(w)),
        action,
        &ClientInfo {
            ip: Some("203.0.113.7".into()),
            ua: Some("Demo/1.0".into()),
        },
    )
    .object("main")
    .detail(
        json!({"provider": "openai", "host": "api.example.com", "apiKey": "sk-marker-secret-0000"}),
    )
}

#[tokio::test]
async fn an_event_goes_through_the_outbox_to_the_activity_route() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.app
        .0
        .activity(event(&t, &admin, "ai.settings.connectionAdd"));
    t.app.0.activity(event(&t, &admin, "ai.settings.key"));
    assert_eq!(count(&t, "SELECT COUNT(*) FROM audit_outbox"), 2);
    // a plain audit line travels the old way, in the same delivery
    t.app
        .0
        .store
        .write(|tx| xexam_ai_service::store::stats::outbox_add(tx, 1, "an old plain audit line"))
        .expect("plain");
    assert_eq!(t.app.0.deliver_outbox().await, 3);
    let st = t.backend.st();
    assert_eq!(st.activity.len(), 2);
    assert_eq!(st.activity_batches, 1, "one batch for both events");
    let e = &st.activity[0];
    let keys: Vec<&str> = e
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec![
            "id", "at", "userId", "stamp", "username", "action", "object", "detail", "result",
            "reason", "ip", "ua"
        ]
    );
    assert_eq!(e["userId"], json!(admin.id));
    assert_eq!(e["stamp"], json!(admin.stamp));
    assert_eq!(e["ip"], json!("203.0.113.7"));
    assert_eq!(
        e["detail"],
        json!({"provider": "openai", "host": "api.example.com"})
    );
    assert!(!serde_json::to_string(&st.activity)
        .expect("json")
        .contains("sk-marker"));
    assert!(st.logs.iter().any(|(_, m)| m == "an old plain audit line"));
    assert!(
        !st.logs.iter().any(|(_, m)| m.contains("\"action\"")),
        "events never go to the plain log"
    );
    drop(st);
    assert_eq!(count(&t, "SELECT COUNT(*) FROM audit_outbox"), 0);
}

#[tokio::test]
async fn a_backend_without_the_route_keeps_the_events_and_a_refused_event_is_dropped() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.app
        .0
        .activity(event(&t, &admin, "ai.settings.connectionAdd"));
    // the route is missing (404): nothing is lost and the link stays up
    t.backend.st().activity_status = 404;
    assert_eq!(t.app.0.deliver_outbox().await, 0);
    assert_eq!(count(&t, "SELECT COUNT(*) FROM audit_outbox"), 1);
    assert!(
        t.app.0.link.is_connected(),
        "a missing route does not cost the link"
    );
    // a bad event refuses its batch: it is dropped, the good one behind it arrives
    t.backend.st().activity_status = 200;
    t.backend.st().activity_bad_action = Some("ai.settings.bad".into());
    t.app.0.activity(event(&t, &admin, "ai.settings.bad"));
    t.app.0.activity(event(&t, &admin, "ai.settings.model"));
    assert_eq!(t.app.0.deliver_outbox().await, 3);
    let actions: Vec<String> = t
        .backend
        .st()
        .activity
        .iter()
        .map(|e| e["action"].as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(
        actions,
        vec!["ai.settings.connectionAdd", "ai.settings.model"]
    );
    assert_eq!(count(&t, "SELECT COUNT(*) FROM audit_outbox"), 0);
    // a transient failure (500) keeps everything for the next round
    t.backend.st().activity_status = 500;
    t.app.0.activity(event(&t, &admin, "ai.settings.limits"));
    assert_eq!(t.app.0.deliver_outbox().await, 0);
    assert_eq!(count(&t, "SELECT COUNT(*) FROM audit_outbox"), 1);
}

#[tokio::test]
async fn events_too_old_for_the_backend_are_not_sent() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let old = activity::line_of(
        &event(&t, &admin, "ai.settings.limits"),
        xexam_ai_service::util::unix_now() - 7 * 86_400,
    );
    t.app
        .0
        .store
        .write(|tx| xexam_ai_service::store::stats::outbox_add(tx, 1, &old))
        .expect("old");
    t.app.0.activity(event(&t, &admin, "ai.settings.model"));
    assert_eq!(t.app.0.deliver_outbox().await, 2);
    let st = t.backend.st();
    assert_eq!(st.activity.len(), 1);
    assert_eq!(st.activity[0]["action"], json!("ai.settings.model"));
}

#[tokio::test]
async fn activity_now_says_whether_the_backend_took_the_event() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    assert!(
        t.app
            .0
            .activity_now(event(&t, &admin, "ai.admin.viewChats"))
            .await
    );
    assert_eq!(t.backend.st().activity.len(), 1);
    t.backend.st().activity_status = 500;
    assert!(
        !t.app
            .0
            .activity_now(event(&t, &admin, "ai.admin.viewChats"))
            .await
    );
    assert_eq!(
        t.backend.st().activity.len(),
        1,
        "nothing is queued for a refused view"
    );
    assert_eq!(count(&t, "SELECT COUNT(*) FROM audit_outbox"), 0);
    // an invalid action is never sent
    t.backend.st().activity_status = 200;
    let bad = Event::new(None, "bank.upload", &ClientInfo::default());
    assert!(!t.app.0.activity_now(bad).await);
}

#[tokio::test]
async fn a_pause_and_a_resume_are_reported_once_each() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    let events = || {
        count(
            &t,
            "SELECT COUNT(*) FROM audit_outbox WHERE message LIKE '%ai.system.%'",
        )
    };
    t.app.0.heartbeat_now().await;
    assert_eq!(events(), 0);
    t.db()
        .execute("UPDATE models SET status = 'failed'", [])
        .expect("fail");
    t.app.0.heartbeat_now().await;
    t.app.0.heartbeat_now().await;
    assert_eq!(events(), 1, "paused once");
    t.db()
        .execute("UPDATE models SET status = 'ok'", [])
        .expect("ok");
    t.app.0.heartbeat_now().await;
    assert_eq!(events(), 2, "resumed once");
    let last: String = t
        .db()
        .query_row(
            "SELECT message FROM audit_outbox ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("last");
    assert!(last.contains("ai.system.resumed"));
}

// —— the route table —————————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn admin_routes_answer_403_to_users() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    for path in [
        "/api/ai/admin/connections/save",
        "/api/ai/admin/connections/test",
        "/api/ai/admin/connections/delete",
        "/api/ai/admin/connections/order",
        "/api/ai/admin/models/pick",
        "/api/ai/admin/models/test",
        "/api/ai/admin/models/remove",
        "/api/ai/admin/feature-switches",
    ] {
        let (s, v) = t.post(path, &sam, json!({})).await;
        assert_eq!(
            (s, v["code"].clone()),
            (403, json!("ai.adminOnly")),
            "{path}"
        );
    }
    // the removed routes are gone
    let (s, v) = t.post("/api/ai/admin/verify", &admin, json!({})).await;
    assert_eq!((s, v["code"].clone()), (404, json!("ai.notFound")));
    let (s, _) = t.post("/api/ai/admin/models", &admin, json!({})).await;
    assert_eq!(s, 405);
}

#[tokio::test]
async fn the_settings_route_takes_only_the_retention() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    let (s, v) = t.get("/api/ai/admin/settings", &admin).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        (
            v["enabled"].clone(),
            v["effective"].clone(),
            v["state"].clone()
        ),
        (json!(true), json!(true), json!("on"))
    );
    for body in [
        json!({"model": "x"}),
        json!({"apiKey": "sk-demo-NEVER-0000-1111"}),
        json!({"retentionDays": 0}),
        json!({"retentionDays": 3651}),
        json!({"retentionDays": "7"}),
    ] {
        let (s, v) = t.post("/api/ai/admin/settings", &admin, body.clone()).await;
        assert_eq!(s, 400, "{body}: {v}");
        assert!(!v.to_string().contains("NEVER"));
    }
    let (s, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"retentionDays": 45}),
        )
        .await;
    assert_eq!((s, v["retentionDays"].clone()), (200, json!(45)));
}
