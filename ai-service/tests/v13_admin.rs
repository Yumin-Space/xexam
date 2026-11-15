//! v1.3 admin routes: the settings object, connections (test before save, key rotation, key reuse confirmation, delete,
//! order), the model pool (list cache, pick, test, remove), enable, the global feature switches (admins unaffected),
//! limits, usage, clear, the chat viewer's `at`, one activity event per POST, the 90-second budget. Everything is made up.
mod common;

use common::*;
use serde_json::{json, Value};
use std::time::Duration;
use xexam_ai_service::store::conn;
use xexam_ai_service::store::data::{self, NewMessage};

const BAD_KEY_BODY: &str = r#"{"error":{"type":"authentication_error","message":"bad key"}}"#;
const KEY2: &str = "sk-demo-1111-2222";
const WRONG: &str = "sk-wrong-0000-1111";

fn v1(t: &TestApp) -> String {
    format!("{}/v1", t.stub.url)
}

/// An address on the stub that accepts any key.
fn open(t: &TestApp) -> String {
    format!("{}/open/v1", t.stub.url)
}

async fn setup() -> (TestApp, Who) {
    let t = start().await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    (t, admin)
}

async fn settings(t: &TestApp, admin: &Who) -> Value {
    let (s, v) = t.get("/api/ai/admin/settings", admin).await;
    assert_eq!(s, 200, "{v}");
    v
}

/// The activity events delivered so far (the delivery task does not run in the tests).
async fn events(t: &TestApp) -> Vec<Value> {
    t.app.0.deliver_outbox().await;
    t.backend.st().activity.clone()
}

/// The admin events (settings, users, admin) among the delivered ones.
async fn admin_events(t: &TestApp) -> Vec<Value> {
    events(t)
        .await
        .into_iter()
        .filter(|e| {
            let a = e["action"].as_str().unwrap_or("");
            a.starts_with("ai.settings.")
                || a.starts_with("ai.users.")
                || a.starts_with("ai.admin.")
        })
        .collect()
}

fn stub_paths(t: &TestApp) -> Vec<String> {
    t.stub
        .requests()
        .iter()
        .map(|r| format!("{} {}", r.method, r.path))
        .collect()
}

fn count_calls(t: &TestApp, what: &str) -> usize {
    stub_paths(t).iter().filter(|p| p.as_str() == what).count()
}

fn conn_ids(s: &Value) -> Vec<i64> {
    s["connections"]
        .as_array()
        .expect("connections")
        .iter()
        .map(|c| c["id"].as_i64().expect("id"))
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

async fn save_new(t: &TestApp, admin: &Who, name: &str, url: &str, key: &str) -> (u16, Value) {
    t.post(
        "/api/ai/admin/connections/save",
        admin,
        json!({"name": name, "provider": "openai", "baseUrl": url, "apiKey": key}),
    )
    .await
}

// —— settings object ———————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn the_settings_object_has_the_v13_shape_and_never_a_key() {
    let (t, admin) = setup().await;
    let s = settings(&t, &admin).await;
    for k in [
        "enabled",
        "effective",
        "state",
        "pauseReason",
        "keyProblem",
        "retentionDays",
        "updatedAt",
        "updatedBy",
        "defaults",
        "maxConnections",
        "connections",
        "pool",
        "slots",
        "featureSwitches",
    ] {
        assert!(s.get(k).is_some(), "missing {k}");
    }
    assert_eq!(s["state"], json!("on"));
    assert_eq!(s["maxConnections"], json!(8));
    assert_eq!(s["defaults"]["openai"], json!("https://api.openai.com/v1"));
    let c = conn_of(&s, 1);
    assert_eq!(c["name"], json!("main"));
    assert_eq!(c["provider"], json!("openai"));
    assert_eq!(c["rev"], json!(1));
    assert_eq!(c["host"], json!(t.stub.url.trim_start_matches("http://")));
    assert_eq!(c["keys"]["apiKey"]["set"], json!(true));
    assert_eq!(c["keys"]["apiKey"]["hint"], json!("…1234"));
    assert_eq!(c["keys"]["apiKey"]["unreadable"], json!(false));
    assert_eq!(c["keys"]["gatewayKey"]["set"], json!(false));
    assert_eq!(c["usedBy"], json!(["default"]));
    assert_eq!(c["insecureHttp"], json!(false));
    let p = &s["pool"][0];
    assert_eq!(p["model"], json!("demo-chat-small"));
    assert_eq!(p["connectionName"], json!("main"));
    assert_eq!(p["usedBy"], json!(["default"]));
    assert_eq!(s["slots"]["default"]["using"]["fallback"], json!("none"));
    assert_eq!(s["slots"]["chat"]["modelId"], Value::Null);
    assert_eq!(s["slots"]["chat"]["using"]["fallback"], json!("default"));
    assert_eq!(s["featureSwitches"]["hint"], json!(true));
    for slot in ["default", "chat", "study", "summary", "generate", "import"] {
        let f = s["slots"][slot]["using"]["fallback"].as_str().unwrap_or("");
        assert!(f == "none" || f == "default", "{slot}: {f}");
    }
    assert!(!s.to_string().contains(KEY), "no key in the settings");
}

// —— connections ——————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn a_new_connection_is_listed_before_it_is_saved_and_a_failed_test_saves_nothing() {
    let (t, admin) = setup().await;
    let (s, v) = save_new(&t, &admin, "second", &open(&t), KEY2).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], json!(true));
    assert_eq!(v["saved"], json!(true));
    assert_eq!(v["connectionId"], json!(2));
    assert_eq!(v["listed"], json!(true));
    assert_eq!(
        v["modelCount"],
        json!(5),
        "the stub lists five distinct models"
    );
    assert_eq!(v["tests"], json!([]), "a new connection is only listed");
    assert_eq!(conn_ids(&v["settings"]), vec![1, 2]);
    assert_eq!(count_calls(&t, "GET /open/v1/models"), 1);
    assert_eq!(
        stub_paths(&t)
            .iter()
            .filter(|p| p.contains("/open/") && p.contains("completions"))
            .count(),
        0
    );

    // a wrong key: nothing is saved
    let (s, v) = save_new(&t, &admin, "bad", &v1(&t), WRONG).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["saved"], json!(false));
    assert_eq!(v["code"], json!("ai.up.auth"));
    assert_eq!(v["httpStatus"], json!(401));
    assert_eq!(v["failedModel"], Value::Null);
    assert_eq!(conn_ids(&v["settings"]), vec![1, 2]);
    let rows: i64 = t
        .db()
        .query_row(
            "SELECT COUNT(*) FROM connections WHERE name = 'bad'",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn key_rotation_tests_one_in_use_model_and_keeps_the_old_key_on_failure() {
    let (t, admin) = setup().await;
    let rev = conn_of(&settings(&t, &admin).await, 1)["rev"]
        .as_i64()
        .expect("rev");
    // a failing rotation keeps the old key
    let (s, v) = t
        .post(
            "/api/ai/admin/connections/save",
            &admin,
            json!({"id": 1, "rev": rev, "name": "main", "provider": "openai", "baseUrl": v1(&t), "apiKey": WRONG}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["saved"], json!(false));
    assert_eq!(v["code"], json!("ai.up.auth"));
    let c = conn_of(&v["settings"], 1);
    assert_eq!(c["rev"], json!(rev));
    assert_eq!(c["keys"]["apiKey"]["hint"], json!("…1234"));
    // the old key still works
    let (s, v) = t
        .post("/api/ai/admin/connections/test", &admin, json!({"id": 1}))
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], json!(true), "{v}");

    // a good rotation: list + one call of the in-use model, rev goes up, the hint changes
    let before = count_calls(&t, "POST /v1/chat/completions");
    let (s, v) = t
        .post(
            "/api/ai/admin/connections/save",
            &admin,
            json!({"id": 1, "rev": rev, "name": "main", "provider": "openai", "baseUrl": v1(&t), "apiKey": KEY2}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["saved"], json!(true));
    assert_eq!(v["tests"].as_array().map(Vec::len), Some(1));
    assert_eq!(v["tests"][0]["model"], json!("demo-chat-small"));
    assert_eq!(count_calls(&t, "POST /v1/chat/completions"), before + 1);
    let c = conn_of(&v["settings"], 1);
    assert_eq!(c["rev"], json!(rev + 1));
    assert_eq!(c["keys"]["apiKey"]["hint"], json!("…2222"));

    // the stale revision is refused
    let (s, v) = t
        .post(
            "/api/ai/admin/connections/save",
            &admin,
            json!({"id": 1, "rev": rev, "name": "renamed", "provider": "openai", "baseUrl": v1(&t)}),
        )
        .await;
    assert_eq!(s, 409, "{v}");
    assert_eq!(v["code"], json!("ai.conflict"));
}

#[tokio::test]
async fn a_name_only_save_does_not_call_the_provider() {
    let (t, admin) = setup().await;
    let before = t.stub.count();
    let (s, v) = t
        .post(
            "/api/ai/admin/connections/save",
            &admin,
            json!({"id": 1, "rev": 1, "name": "primary", "provider": "openai", "baseUrl": v1(&t)}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["saved"], json!(true));
    assert_eq!(t.stub.count(), before, "nothing was sent");
    assert_eq!(conn_of(&v["settings"], 1)["name"], json!("primary"));
    assert_eq!(conn_of(&v["settings"], 1)["rev"], json!(2));
}

#[tokio::test]
async fn copying_the_keys_of_another_connection_needs_the_host_confirmed_for_any_scheme() {
    let (t, admin) = setup().await;
    let body = |confirm: Option<&str>| {
        let mut b =
            json!({"name": "copy", "provider": "openai", "baseUrl": v1(&t), "reuseKeysFrom": 1});
        if let Some(c) = confirm {
            b["confirmHost"] = json!(c);
        }
        b
    };
    let host = t.stub.url.trim_start_matches("http://").to_string();
    let (s, v) = t
        .post("/api/ai/admin/connections/save", &admin, body(None))
        .await;
    assert_eq!(s, 409, "{v}");
    assert_eq!(v["code"], json!("ai.keyReuseConfirm"));
    assert_eq!(v["host"], json!(host));
    assert_eq!(v["oldHost"], json!(host));
    assert_eq!(v["slots"], json!([]));
    let (s, v) = t
        .post(
            "/api/ai/admin/connections/save",
            &admin,
            body(Some("other.example.com")),
        )
        .await;
    assert_eq!(s, 409, "{v}");
    assert_eq!(v["code"], json!("ai.keyReuseConfirm"));
    // the stub is a plain-http address: the reuse is allowed (no restriction on the scheme)
    assert!(t.stub.url.starts_with("http://"));
    let (s, v) = t
        .post("/api/ai/admin/connections/save", &admin, body(Some(&host)))
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], json!(true), "{v}");
    let id = v["connectionId"].as_i64().expect("id");
    assert_eq!(
        conn_of(&v["settings"], id)["keys"]["apiKey"]["hint"],
        json!("…1234")
    );
    let ev = admin_events(&t).await;
    let reuse = ev
        .iter()
        .find(|e| e["action"] == json!("ai.settings.keyReuse"))
        .expect("keyReuse event");
    assert_eq!(reuse["object"], json!("copy"));
    assert_eq!(reuse["detail"]["newHost"], json!(host));

    // moving a connection to another origin without typing the key again also asks (host name differs)
    let moved = format!(
        "http://localhost:{}/v1",
        t.stub.url.rsplit(':').next().unwrap_or("1")
    );
    let (s, v) = t
        .post(
            "/api/ai/admin/connections/save",
            &admin,
            json!({"id": 1, "rev": 1, "name": "main", "provider": "openai", "baseUrl": moved}),
        )
        .await;
    assert_eq!(s, 409, "{v}");
    assert_eq!(v["code"], json!("ai.keyReuseConfirm"));
    assert!(v["host"].as_str().unwrap_or("").starts_with("localhost:"));
    assert_eq!(v["slots"], json!(["default"]));
}

#[tokio::test]
async fn validation_names_limits_and_header_clash() {
    let (t, admin) = setup().await;
    let post = |b: Value| {
        let (t, admin) = (&t, &admin);
        async move { t.post("/api/ai/admin/connections/save", admin, b).await }
    };
    let (s, v) = post(json!({"name": "", "provider": "openai", "baseUrl": open(&t)})).await;
    assert_eq!((s, v["code"].as_str()), (400, Some("ai.badName")));
    let (s, v) = post(json!({"name": "x", "provider": "openai", "baseUrl": "ftp://x"})).await;
    assert_eq!((s, v["code"].as_str()), (400, Some("ai.badUrl")));
    let (s, v) = post(json!({"name": "x", "provider": "other", "baseUrl": open(&t)})).await;
    assert_eq!((s, v["code"].as_str()), (400, Some("ai.badRequest")));
    let (s, v) = post(json!({"name": "x", "provider": "openai", "apiKey": "has space"})).await;
    assert_eq!((s, v["code"].as_str()), (400, Some("ai.badKey")));
    // the same name, in another case
    let (s, v) = post(json!({"name": "MAIN", "provider": "openai", "baseUrl": open(&t)})).await;
    assert_eq!((s, v["code"].as_str()), (409, Some("ai.nameTaken")));
    // a gateway needs its key, and a provider key on top of a Bearer / OpenAI gateway clashes
    let gw = format!("{}/gw/openai", t.stub.url);
    let (s, v) = post(json!({"name": "gw", "provider": "gateway", "baseUrl": gw})).await;
    assert_eq!((s, v["code"].as_str()), (422, Some("ai.keyMissing")));
    let (s, v) = post(
        json!({"name": "gw", "provider": "gateway", "baseUrl": gw, "gatewayKey": GW_KEY, "apiKey": KEY2, "gatewayAuth": "bearer"}),
    )
    .await;
    assert_eq!((s, v["code"].as_str()), (400, Some("ai.headerClash")));
    // an unknown field
    let (s, _) = post(json!({"name": "x", "provider": "openai", "model": "m"})).await;
    assert_eq!(s, 400);
    // at most 8 connections
    t.app
        .0
        .store
        .write(|tx| -> rusqlite::Result<()> {
            for i in 2..=8 {
                conn::insert_connection(
                    tx,
                    &conn::NewConn {
                        name: &format!("c{i}"),
                        provider: "openai",
                        base_url: "https://example.com/v1",
                        format: "openai",
                        gateway_auth: "bearer",
                        lists_models: true,
                        by: "test",
                        now: 1,
                    },
                )?;
            }
            Ok(())
        })
        .expect("fill");
    let (s, v) = post(json!({"name": "ninth", "provider": "openai", "baseUrl": open(&t)})).await;
    assert_eq!((s, v["code"].as_str()), (409, Some("ai.connectionLimit")));
    assert_eq!(v["limit"], json!(8));
}

#[tokio::test]
async fn a_gateway_connection_saves_with_its_gateway_key() {
    let (t, admin) = setup().await;
    let gw = format!("{}/gw/openai", t.stub.url);
    let (s, v) = t
        .post(
            "/api/ai/admin/connections/save",
            &admin,
            json!({"name": "gateway", "provider": "gateway", "baseUrl": gw, "gatewayKey": GW_KEY, "gatewayAuth": "cf-aig-authorization"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], json!(true), "{v}");
    let id = v["connectionId"].as_i64().expect("id");
    let c = conn_of(&v["settings"], id);
    assert_eq!(c["format"], json!("openai"));
    assert_eq!(c["keys"]["gatewayKey"]["set"], json!(true));
    assert_eq!(c["keys"]["apiKey"]["set"], json!(false));
}

#[tokio::test]
async fn order_and_delete() {
    let (t, admin) = setup().await;
    let (_, v) = save_new(&t, &admin, "second", &open(&t), KEY2).await;
    assert_eq!(v["connectionId"], json!(2));
    let (s, v) = t
        .post(
            "/api/ai/admin/connections/order",
            &admin,
            json!({"ids": [2, 1]}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(conn_ids(&v["settings"]), vec![2, 1]);
    let (s, _) = t
        .post(
            "/api/ai/admin/connections/order",
            &admin,
            json!({"ids": [2, 2]}),
        )
        .await;
    assert_eq!(s, 400);

    // a model of the second connection for the chat slot
    let (s, v) = t
        .post(
            "/api/ai/admin/models/pick",
            &admin,
            json!({"slot": "chat", "connectionId": 2, "model": "demo-chat-large", "manual": false}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], json!(true), "{v}");
    // used by a slot: refused until forced
    let (s, v) = t
        .post("/api/ai/admin/connections/delete", &admin, json!({"id": 2}))
        .await;
    assert_eq!(s, 409, "{v}");
    assert_eq!(v["code"], json!("ai.connectionInUse"));
    assert_eq!(v["usedBy"], json!(["chat"]));
    // the default lives on connection 1: deleting it needs a new default
    let (s, v) = t
        .post(
            "/api/ai/admin/connections/delete",
            &admin,
            json!({"id": 1, "force": true}),
        )
        .await;
    assert_eq!(s, 409, "{v}");
    assert_eq!(v["code"], json!("ai.defaultNeeded"));
    let (s, v) = t
        .post(
            "/api/ai/admin/connections/delete",
            &admin,
            json!({"id": 1, "force": true, "newDefault": {"connectionId": 2, "model": "demo-chat-large"}}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let s2 = &v["settings"];
    assert_eq!(conn_ids(s2), vec![2]);
    assert_eq!(s2["slots"]["default"]["using"]["fallback"], json!("none"));
    assert_eq!(s2["state"], json!("on"));
    // forcing the delete of the last connection switches AI+ off
    let (s, v) = t
        .post(
            "/api/ai/admin/connections/delete",
            &admin,
            json!({"id": 2, "force": true}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(conn_ids(&v["settings"]), Vec::<i64>::new());
    assert_eq!(v["settings"]["enabled"], json!(false));
    assert_eq!(v["settings"]["state"], json!("setup"));
}

#[tokio::test]
async fn a_connection_test_sets_the_status_and_a_config_failure_breaks_it() {
    let (t, admin) = setup().await;
    let (s, v) = t
        .post("/api/ai/admin/connections/test", &admin, json!({"id": 1}))
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["saved"], json!(false));
    assert_eq!(v["tests"][0]["model"], json!("demo-chat-small"));
    // the provider refuses the key now
    t.stub.force(Some((401, BAD_KEY_BODY)));
    let (s, v) = t
        .post("/api/ai/admin/connections/test", &admin, json!({"id": 1}))
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["code"], json!("ai.up.auth"));
    assert_eq!(v["failedModel"], json!("demo-chat-small"));
    let c = conn_of(&v["settings"], 1);
    assert_eq!(c["status"], json!("broken"));
    assert_eq!(c["errorCode"], json!("ai.up.auth"));
    assert_eq!(v["settings"]["pool"][0]["status"], json!("failed"));
    assert_eq!(v["settings"]["state"], json!("paused"));
    assert_eq!(v["settings"]["pauseReason"], json!("noRoute"));
    // it works again after a passing test
    t.stub.force(None);
    let (_, v) = t
        .post("/api/ai/admin/connections/test", &admin, json!({"id": 1}))
        .await;
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(conn_of(&v["settings"], 1)["status"], json!("ok"));
    assert_eq!(v["settings"]["pool"][0]["status"], json!("ok"));
    assert_eq!(v["settings"]["state"], json!("on"));
    let ev = events(&t).await;
    let acts: Vec<&str> = ev.iter().filter_map(|e| e["action"].as_str()).collect();
    assert!(acts.contains(&"ai.system.connection"), "{acts:?}");
}

#[tokio::test]
async fn a_slow_provider_answers_ai_up_timeout_and_changes_no_status() {
    let mut o = Opts::default();
    o.timeouts.verify_total = Duration::from_secs(1);
    let t = start_with(o).await;
    let admin = t.user("admin", true, false);
    t.configure_openai(&admin, "demo-chat-small").await;
    t.stub.st().head_delay_ms = 4000;
    let (s, v) = t
        .post("/api/ai/admin/connections/test", &admin, json!({"id": 1}))
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["code"], json!("ai.up.timeout"));
    assert_eq!(v["failedModel"], json!("demo-chat-small"));
    // a timeout is transient: nothing is marked broken or failed
    assert_eq!(conn_of(&v["settings"], 1)["status"], json!("ok"));
    assert_eq!(v["settings"]["pool"][0]["status"], json!("ok"));
}

#[tokio::test]
async fn one_test_per_connection_at_a_time() {
    let (t, admin) = setup().await;
    t.stub.st().head_delay_ms = 1500;
    let first = t.post("/api/ai/admin/connections/test", &admin, json!({"id": 1}));
    let second = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        t.post("/api/ai/admin/connections/test", &admin, json!({"id": 1}))
            .await
    };
    let ((s1, v1), (s2, v2)) = tokio::join!(first, second);
    assert_eq!(s1, 200, "{v1}");
    assert_eq!(s2, 409, "{v2}");
    assert_eq!(v2["code"], json!("ai.busy"));
    // after the first one the connection is free again
    t.stub.st().head_delay_ms = 0;
    let (s, _) = t
        .post("/api/ai/admin/connections/test", &admin, json!({"id": 1}))
        .await;
    assert_eq!(s, 200);
}

// —— models ———————————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn the_model_list_is_cached_for_ten_minutes_until_refreshed() {
    let (t, admin) = setup().await;
    let (s, v) = t.get("/api/ai/admin/models?refresh=1", &admin).await;
    assert_eq!(s, 200, "{v}");
    let c = &v["connections"][0];
    assert_eq!(c["id"], json!(1));
    assert_eq!(c["listed"], json!(true));
    assert_eq!(c["error"], Value::Null);
    let m = c["models"]
        .as_array()
        .expect("models")
        .iter()
        .find(|m| m["id"] == json!("demo-chat-small"))
        .expect("model");
    assert!(m["poolId"].is_number());
    assert_eq!(m["status"], json!("ok"));
    assert_eq!(count_calls(&t, "GET /v1/models"), 1);
    let (_, _) = t.get("/api/ai/admin/models?refresh=0", &admin).await;
    let (_, _) = t.get("/api/ai/admin/models", &admin).await;
    assert_eq!(count_calls(&t, "GET /v1/models"), 1, "cached");
    let (_, _) = t.get("/api/ai/admin/models?refresh=1", &admin).await;
    assert_eq!(count_calls(&t, "GET /v1/models"), 2);
    let (s, _) = t.get("/api/ai/admin/models?refresh=x", &admin).await;
    assert_eq!(s, 400);
    // every list is a usage row of kind `models` with the connection
    let rows: i64 = t
        .db()
        .query_row(
            "SELECT COUNT(*) FROM usage WHERE kind = 'models' AND connection_id = 1 AND connection = 'main'",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(rows, 2);
}

#[tokio::test]
async fn pick_tests_unless_fresh_and_a_failed_pick_changes_nothing() {
    let (t, admin) = setup().await;
    let pick = |slot: &str, model: &str| json!({"slot": slot, "connectionId": 1, "model": model, "manual": false});
    let before = count_calls(&t, "POST /v1/chat/completions");
    let (s, v) = t
        .post(
            "/api/ai/admin/models/pick",
            &admin,
            pick("chat", "demo-chat-large"),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(count_calls(&t, "POST /v1/chat/completions"), before + 1);
    let pool_id = v["settings"]["slots"]["chat"]["modelId"]
        .as_i64()
        .expect("slot");
    assert_eq!(
        v["settings"]["slots"]["chat"]["using"]["fallback"],
        json!("none")
    );
    // a model that just passed is not tested again
    let (_, v) = t
        .post(
            "/api/ai/admin/models/pick",
            &admin,
            pick("study", "demo-chat-large"),
        )
        .await;
    assert_eq!(v["ok"], json!(true));
    assert_eq!(count_calls(&t, "POST /v1/chat/completions"), before + 1);
    assert_eq!(v["settings"]["slots"]["study"]["modelId"], json!(pool_id));
    // a failing pick: no slot change, a new model is not added to the pool
    t.stub.force(Some((401, BAD_KEY_BODY)));
    let (s, v) = t
        .post(
            "/api/ai/admin/models/pick",
            &admin,
            pick("summary", "demo-length"),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["code"], json!("ai.up.auth"));
    assert_eq!(v["httpStatus"], json!(401));
    assert_eq!(v["settings"]["slots"]["summary"]["modelId"], Value::Null);
    assert!(v["settings"]["pool"]
        .as_array()
        .expect("pool")
        .iter()
        .all(|m| m["model"] != json!("demo-length")));
    t.stub.force(None);
    // clear a slot; the default cannot be cleared
    let (s, v) = t
        .post(
            "/api/ai/admin/models/pick",
            &admin,
            json!({"slot": "chat", "clear": true}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["settings"]["slots"]["chat"]["modelId"], Value::Null);
    let (s, v) = t
        .post(
            "/api/ai/admin/models/pick",
            &admin,
            json!({"slot": "default", "clear": true}),
        )
        .await;
    assert_eq!((s, v["code"].as_str()), (400, Some("ai.badRequest")));
    // bad slot / model
    let (s, _) = t
        .post("/api/ai/admin/models/pick", &admin, pick("nope", "m"))
        .await;
    assert_eq!(s, 400);
    let (s, v) = t
        .post(
            "/api/ai/admin/models/pick",
            &admin,
            json!({"slot": "chat", "connectionId": 99, "model": "m"}),
        )
        .await;
    assert_eq!((s, v["code"].as_str()), (400, Some("ai.badModel")));
}

#[tokio::test]
async fn an_unusable_default_warns_but_nothing_is_switched_and_ai_stays_on() {
    let (t, admin) = setup().await;
    let (_, v) = t
        .post(
            "/api/ai/admin/models/pick",
            &admin,
            json!({"slot": "chat", "connectionId": 1, "model": "demo-chat-large", "manual": false}),
        )
        .await;
    assert_eq!(v["ok"], json!(true), "{v}");
    // the default model fails (a configuration error found by a test)
    t.app
        .0
        .store
        .write(|tx| conn::set_pool_status(tx, 1, "failed", Some("ai.up.notFound"), None, Some(1)))
        .expect("fail the default");
    let s = settings(&t, &admin).await;
    assert_eq!(
        s["slots"]["default"]["using"],
        Value::Null,
        "the warning: the default has no route"
    );
    assert_eq!(
        s["slots"]["study"]["using"],
        Value::Null,
        "nothing else is switched in"
    );
    assert_eq!(s["slots"]["chat"]["using"]["fallback"], json!("none"));
    assert_eq!(s["state"], json!("on"), "another slot still resolves");
    assert_eq!(s["effective"], json!(true));
}

#[tokio::test]
async fn model_test_and_remove() {
    let (t, admin) = setup().await;
    let (s, v) = t
        .post(
            "/api/ai/admin/models/test",
            &admin,
            json!({"connectionId": 1, "model": "demo-chat-small"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["tests"][0]["model"], json!("demo-chat-small"));
    t.stub.force(Some((401, BAD_KEY_BODY)));
    let (_, v) = t
        .post(
            "/api/ai/admin/models/test",
            &admin,
            json!({"connectionId": 1, "model": "demo-chat-small"}),
        )
        .await;
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["code"], json!("ai.up.auth"));
    assert_eq!(v["failedModel"], json!("demo-chat-small"));
    assert_eq!(v["settings"]["pool"][0]["status"], json!("failed"));
    t.stub.force(None);
    // in use: refused; unused: removed
    let (s, v) = t
        .post("/api/ai/admin/models/remove", &admin, json!({"poolId": 1}))
        .await;
    assert_eq!(s, 409, "{v}");
    assert_eq!(v["code"], json!("ai.modelInUse"));
    assert_eq!(v["usedBy"], json!(["default"]));
    let (_, v) = t
        .post(
            "/api/ai/admin/models/pick",
            &admin,
            json!({"slot": "chat", "connectionId": 1, "model": "demo-chat-large", "manual": true}),
        )
        .await;
    assert_eq!(v["ok"], json!(true), "{v}");
    let large = v["settings"]["slots"]["chat"]["modelId"]
        .as_i64()
        .expect("id");
    assert_eq!(
        v["settings"]["pool"]
            .as_array()
            .expect("pool")
            .iter()
            .find(|m| m["id"] == json!(large))
            .map(|m| m["manual"].clone()),
        Some(json!(true))
    );
    let (_, v) = t
        .post(
            "/api/ai/admin/models/pick",
            &admin,
            json!({"slot": "chat", "clear": true}),
        )
        .await;
    assert_eq!(v["ok"], json!(true));
    let (s, v) = t
        .post(
            "/api/ai/admin/models/remove",
            &admin,
            json!({"poolId": large}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["settings"]["pool"].as_array().map(Vec::len), Some(1));
    let (s, _) = t
        .post(
            "/api/ai/admin/models/remove",
            &admin,
            json!({"poolId": large}),
        )
        .await;
    assert_eq!(s, 404);
}

// —— enable, switches ——————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn enable_needs_a_tested_default_and_a_failing_test_leaves_ai_off() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let (_, v) = save_new(&t, &admin, "main", &v1(&t), KEY).await;
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["settings"]["state"], json!("setup"));
    // no default model yet
    let (s, v) = t
        .post("/api/ai/admin/enable", &admin, json!({"enabled": true}))
        .await;
    assert_eq!((s, v["code"].as_str()), (422, Some("ai.modelMissing")));
    // a failing test leaves AI+ off
    t.stub.force(Some((401, BAD_KEY_BODY)));
    let (s, v) = t
        .post(
            "/api/ai/admin/enable",
            &admin,
            json!({"enabled": true, "default": {"connectionId": 1, "model": "demo-chat-small"}}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["code"], json!("ai.up.auth"));
    assert_eq!(v["settings"]["enabled"], json!(false));
    t.stub.force(None);
    let (s, v) = t
        .post(
            "/api/ai/admin/enable",
            &admin,
            json!({"enabled": true, "default": {"connectionId": 1, "model": "demo-chat-small"}}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["settings"]["enabled"], json!(true));
    assert_eq!(v["settings"]["effective"], json!(true));
    assert_eq!(v["settings"]["state"], json!("on"));
    assert_eq!(
        v["settings"]["slots"]["default"]["using"]["fallback"],
        json!("none")
    );
    // off again
    let (_, v) = t
        .post("/api/ai/admin/enable", &admin, json!({"enabled": false}))
        .await;
    assert_eq!(v["settings"]["state"], json!("off"));
    // on again with the default already set
    let (_, v) = t
        .post("/api/ai/admin/enable", &admin, json!({"enabled": true}))
        .await;
    assert_eq!(v["settings"]["state"], json!("on"));
}

#[tokio::test]
async fn a_global_switch_stops_normal_users_but_never_an_admin() {
    let (t, admin) = setup().await;
    let sam = t.user("sam", false, true);
    t.relax_caps();
    let (s, v) = t
        .post(
            "/api/ai/admin/feature-switches",
            &admin,
            json!({"chat": false}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["featureSwitches"]["chat"], json!(false));
    assert_eq!(v["featureSwitches"]["hint"], json!(true));
    // a normal user is refused
    let (s, v) = t.post("/api/ai/send", &sam, json!({"text": "hello"})).await;
    assert_eq!(s, 403, "{v}");
    assert_eq!(v["code"], json!("ai.featureOff"));
    // an administrator is not affected: the call runs to its end
    let ev = t
        .send_all(&admin, json!({"text": "hello"}))
        .await
        .expect("send");
    assert_eq!(
        names(&ev).last().map(String::as_str),
        Some("done"),
        "{:?}",
        names(&ev)
    );
    // switching it on again
    let (_, v) = t
        .post(
            "/api/ai/admin/feature-switches",
            &admin,
            json!({"chat": true}),
        )
        .await;
    assert_eq!(v["featureSwitches"]["chat"], json!(true));
    let ev = t
        .send_all(&sam, json!({"text": "hello"}))
        .await
        .expect("send");
    assert_eq!(names(&ev).last().map(String::as_str), Some("done"));
    // validation
    let (s, _) = t
        .post(
            "/api/ai/admin/feature-switches",
            &admin,
            json!({"nope": true}),
        )
        .await;
    assert_eq!(s, 400);
    let (s, _) = t
        .post(
            "/api/ai/admin/feature-switches",
            &admin,
            json!({"chat": "off"}),
        )
        .await;
    assert_eq!(s, 400);
    let (s, _) = t
        .post(
            "/api/ai/admin/feature-switches",
            &sam,
            json!({"chat": false}),
        )
        .await;
    assert_eq!(s, 403);
}

// —— limits, usage, clear ——————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn limits_carry_the_v13_keys_and_each_change_is_an_event() {
    let (t, admin) = setup().await;
    let (s, v) = t.get("/api/ai/admin/limits", &admin).await;
    assert_eq!(s, 200, "{v}");
    let l = &v["limits"];
    assert_eq!(l["callsAdminsExempt"], json!(1));
    assert_eq!(l["studyShare"], json!(1));
    assert_eq!(l["studyShareDays"], json!(180));
    assert_eq!(l["genMaxChunks"], json!(8));
    assert_eq!(l["summaryMaxItems"], json!(40));
    assert_eq!(l["dailyTokenBudget"], json!(0));
    assert_eq!(l["importMaxQuestions"], json!(100));
    assert_eq!(l["memoryTokens"], json!(200));
    assert_eq!(l["memoriesPerUser"], json!(30));
    assert!(
        l.get("hintMaxTokens").is_none(),
        "per-feature answer lengths are not built"
    );
    let (s, v) = t
        .post(
            "/api/ai/admin/limits",
            &admin,
            json!({"dailyTokenBudget": 5000, "callsAdminsExempt": 0}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["limits"]["dailyTokenBudget"], json!(5000));
    assert_eq!(v["limits"]["callsAdminsExempt"], json!(0));
    let (s, v) = t
        .post(
            "/api/ai/admin/limits",
            &admin,
            json!({"importMaxQuestions": 301}),
        )
        .await;
    assert_eq!((s, v["code"].as_str()), (400, Some("ai.limitRange")));
    let ev = admin_events(&t).await;
    let e = ev
        .iter()
        .find(|e| e["action"] == json!("ai.settings.limits"))
        .expect("event");
    assert_eq!(
        e["detail"]["keys"],
        json!(["callsAdminsExempt", "dailyTokenBudget"])
    );
}

#[tokio::test]
async fn usage_has_connections_the_shared_counters_and_the_budget() {
    let (t, admin) = setup().await;
    let (_, v) = t
        .post("/api/ai/admin/connections/test", &admin, json!({"id": 1}))
        .await;
    assert_eq!(v["ok"], json!(true), "{v}");
    let ev = t
        .send_all(&admin, json!({"text": "hello"}))
        .await
        .expect("send");
    assert_eq!(names(&ev).last().map(String::as_str), Some("done"));
    let (s, v) = t.get("/api/ai/admin/usage?days=7", &admin).await;
    assert_eq!(s, 200, "{v}");
    let by_model = v["byModel"].as_array().expect("byModel");
    assert!(by_model
        .iter()
        .any(|m| m["model"] == json!("demo-chat-small")
            && m["connectionId"] == json!(1)
            && m["connection"] == json!("main")));
    assert!(by_model.iter().any(|m| m["model"] == json!("")
        && m["other"] == json!(true)
        && m["connection"] == json!("main")));
    let by_conn = v["byConnection"].as_array().expect("byConnection");
    assert_eq!(by_conn[0]["connection"], json!("main"));
    assert_eq!(by_conn[0]["connectionId"], json!(1));
    assert!(by_conn[0]["calls"].as_i64().unwrap_or(0) >= 2);
    assert_eq!(v["shared"], json!({"rows": 0, "hits": 0, "hitsToday": 0}));
    assert_eq!(v["budget"]["limit"], json!(0));
    assert!(v["budget"]["usedToday"].is_number());
}

#[tokio::test]
async fn clearing_the_shared_answers_is_one_event_with_the_count() {
    let (t, admin) = setup().await;
    t.app
        .0
        .store
        .write(|tx| -> rusqlite::Result<()> {
            for (i, q) in ["q1", "q2"].iter().enumerate() {
                xexam_ai_service::store::shared::shared_put_first(
                    tx,
                    &xexam_ai_service::store::shared::SharedKey {
                        question_id: q,
                        content_hash: &format!("{i:032x}"),
                        kind: "hint",
                        lang: "en",
                    },
                    &xexam_ai_service::store::shared::NewShared {
                        content: "made-up hint",
                        model: "demo-chat-small",
                        connection: "main",
                        tokens_in: None,
                        tokens_out: None,
                        now: 1,
                    },
                )?;
            }
            Ok(())
        })
        .expect("shared rows");
    let (_, v) = t.get("/api/ai/admin/usage?days=7", &admin).await;
    assert_eq!(v["shared"]["rows"], json!(2));
    let (s, v) = t
        .post("/api/ai/admin/clear", &admin, json!({"what": "shared"}))
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["deleted"], json!(2));
    let (_, v) = t.get("/api/ai/admin/usage?days=7", &admin).await;
    assert_eq!(v["shared"]["rows"], json!(0));
    let ev = admin_events(&t).await;
    let e = ev
        .iter()
        .find(|e| e["action"] == json!("ai.admin.clear"))
        .expect("event");
    assert_eq!(e["object"], json!("shared"));
    assert_eq!(e["detail"]["count"], json!(2));
    let (s, _) = t
        .post(
            "/api/ai/admin/clear",
            &admin,
            json!({"what": "shared", "userId": 1}),
        )
        .await;
    assert_eq!(s, 400);
}

// —— the chat viewer ————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn the_viewer_reads_a_branch_through_at_and_never_moves_the_leaf() {
    let (t, admin) = setup().await;
    let sam = t.user("sam", false, true);
    t.relax_caps();
    let ev = t
        .send_all(&sam, json!({"text": "hello"}))
        .await
        .expect("send");
    let chat = ev[0].1["chatId"].as_i64().expect("chat id");
    // a second answer to the same question: a new branch, now the leaf
    let (q, a1) = {
        let c = t.db();
        let ids: Vec<i64> = c
            .prepare("SELECT id FROM messages WHERE chat_id = ?1 ORDER BY id")
            .and_then(|mut s| s.query_map([chat], |r| r.get(0))?.collect())
            .expect("ids");
        (ids[0], ids[1])
    };
    let a2 = t
        .app
        .0
        .store
        .write(|tx| {
            data::insert_message(
                tx,
                &NewMessage {
                    chat,
                    role: "assistant",
                    content: "second made-up answer",
                    status: "done",
                    model: Some("demo-chat-small"),
                    free: false,
                    now: 5,
                    parent: Some(q),
                    connection: Some("main"),
                },
            )
        })
        .expect("second answer");
    let leaf = |t: &TestApp| -> Option<i64> {
        t.db()
            .query_row("SELECT leaf_id FROM chats WHERE id = ?1", [chat], |r| {
                r.get(0)
            })
            .expect("leaf")
    };
    assert_eq!(leaf(&t), Some(a2));
    let path = format!("/api/ai/admin/chat?user={}&id={chat}", sam.id);
    let (s, v) = t.get(&path, &admin).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["chat"]["leafId"], json!(a2));
    assert_eq!(v["messages"][1]["id"], json!(a2));
    assert_eq!(v["messages"][1]["versions"], json!([a1, a2]));
    assert_eq!(v["messages"][1]["connection"], json!("main"));
    assert_eq!(v["messages"][1]["parentId"], json!(q));
    // `at` = the first answer: that branch, read only
    let (s, v) = t.get(&format!("{path}&at={a1}"), &admin).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["messages"][1]["id"], json!(a1));
    assert_eq!(v["chat"]["leafId"], json!(a1));
    assert_eq!(leaf(&t), Some(a2), "the viewer never writes the leaf");
    // a message of no such chat
    let (s, _) = t.get(&format!("{path}&at=999999"), &admin).await;
    assert_eq!(s, 404);
    // every view is an event that carries counts only
    let ev = admin_events(&t).await;
    let views: Vec<&Value> = ev
        .iter()
        .filter(|e| e["action"] == json!("ai.admin.viewChats"))
        .collect();
    assert_eq!(views.len(), 2);
    assert_eq!(views[0]["object"], json!("sam"));
    assert_eq!(views[0]["detail"]["messages"], json!(2));
    assert_eq!(views[0]["detail"]["stored"], json!(3));
    // the view is refused when the event cannot be delivered
    t.backend.st().activity_status = 500;
    let (s, v) = t.get(&path, &admin).await;
    assert_eq!(s, 503, "{v}");
    assert_eq!(v["code"], json!("ai.backendDown"));
}

// —— events ———————————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn every_admin_post_leaves_exactly_one_event_and_never_a_secret() {
    init_logs();
    let (t, admin) = setup().await;
    let sam = t.user("sam", false, true);
    let marker = "sk-marker-ZZSECRET-77001";
    let seen = std::cell::Cell::new(0usize);
    let one = |label: &str, evs: Vec<Value>, action: &str| {
        let new: Vec<&Value> = evs.iter().skip(seen.get()).collect();
        assert_eq!(
            new.len(),
            1,
            "{label}: {:?}",
            new.iter().map(|e| &e["action"]).collect::<Vec<_>>()
        );
        assert_eq!(new[0]["action"], json!(action), "{label}");
        assert_eq!(new[0]["userId"], json!(admin.id), "{label}");
        seen.set(evs.len());
    };
    macro_rules! step {
        ($label:expr, $action:expr, $post:expr) => {{
            let (s, v) = $post.await;
            assert_eq!(s, 200, "{}: {v}", $label);
            let evs = admin_events(&t).await;
            one($label, evs, $action);
        }};
    }
    step!(
        "retention",
        "ai.settings.retention",
        t.post(
            "/api/ai/admin/settings",
            &admin,
            json!({"retentionDays": 30})
        )
    );
    step!(
        "limits",
        "ai.settings.limits",
        t.post("/api/ai/admin/limits", &admin, json!({"genMaxChunks": 5}))
    );
    step!(
        "features",
        "ai.settings.features",
        t.post(
            "/api/ai/admin/feature-switches",
            &admin,
            json!({"hint": false})
        )
    );
    step!(
        "connection add (marker key)",
        "ai.settings.connectionAdd",
        t.post(
            "/api/ai/admin/connections/save",
            &admin,
            json!({"name": "second", "provider": "openai", "baseUrl": open(&t), "apiKey": marker})
        )
    );
    step!(
        "connection update",
        "ai.settings.connectionUpdate",
        t.post(
            "/api/ai/admin/connections/save",
            &admin,
            json!({"id": 2, "rev": 1, "name": "second one", "provider": "openai", "baseUrl": open(&t)})
        )
    );
    step!(
        "key",
        "ai.settings.key",
        t.post(
            "/api/ai/admin/connections/save",
            &admin,
            json!({"id": 2, "rev": 2, "name": "second one", "provider": "openai", "baseUrl": open(&t), "apiKey": "sk-marker-ZZSECRET-77002"})
        )
    );
    step!(
        "connection test",
        "ai.settings.connectionTest",
        t.post("/api/ai/admin/connections/test", &admin, json!({"id": 2}))
    );
    step!(
        "order",
        "ai.settings.connectionOrder",
        t.post(
            "/api/ai/admin/connections/order",
            &admin,
            json!({"ids": [2, 1]})
        )
    );
    step!(
        "model pick",
        "ai.settings.model",
        t.post(
            "/api/ai/admin/models/pick",
            &admin,
            json!({"slot": "chat", "connectionId": 2, "model": "demo-chat-large", "manual": false})
        )
    );
    step!(
        "model test",
        "ai.settings.modelTest",
        t.post(
            "/api/ai/admin/models/test",
            &admin,
            json!({"connectionId": 2, "model": "demo-chat-large"})
        )
    );
    step!(
        "model clear",
        "ai.settings.model",
        t.post(
            "/api/ai/admin/models/pick",
            &admin,
            json!({"slot": "chat", "clear": true})
        )
    );
    step!(
        "model remove",
        "ai.settings.modelRemove",
        t.post("/api/ai/admin/models/remove", &admin, json!({"poolId": 2}))
    );
    step!(
        "disable",
        "ai.settings.disable",
        t.post("/api/ai/admin/enable", &admin, json!({"enabled": false}))
    );
    step!(
        "enable",
        "ai.settings.enable",
        t.post("/api/ai/admin/enable", &admin, json!({"enabled": true}))
    );
    step!(
        "clear shared",
        "ai.admin.clear",
        t.post("/api/ai/admin/clear", &admin, json!({"what": "shared"}))
    );
    // a user the admin works on
    let sam_id = sam.id;
    step!(
        "user features",
        "ai.users.features",
        t.post(
            "/api/ai/admin/features",
            &admin,
            json!({"userId": sam_id, "features": {"chat": true, "hint": false, "explain": true, "summary": true, "generate": true, "import": true}})
        )
    );
    step!(
        "suspend",
        "ai.users.features",
        t.post("/api/ai/admin/suspend", &admin, json!({"userId": sam_id}))
    );
    step!(
        "resume",
        "ai.users.features",
        t.post("/api/ai/admin/resume", &admin, json!({"userId": sam_id}))
    );
    step!(
        "free",
        "ai.users.features",
        t.post(
            "/api/ai/admin/free",
            &admin,
            json!({"userId": admin.id, "free": true})
        )
    );
    step!(
        "connection remove",
        "ai.settings.connectionRemove",
        t.post("/api/ai/admin/connections/delete", &admin, json!({"id": 2}))
    );
    // no key, hint or typed secret anywhere an event or a log line goes
    let all = serde_json::to_string(&t.backend.st().activity).expect("json");
    assert!(!all.contains("ZZSECRET") && !all.contains(KEY) && !all.contains(KEY2));
    assert!(!all.contains("…1234"), "no key hint in an event");
    assert!(!captured_contains("ZZSECRET"));
    assert!(t.backend.logs().iter().all(|l| !l.contains("ZZSECRET")));
    // the feature event names the keys and the user, the key event the secret and the change
    let evs = t.backend.st().activity.clone();
    let key = evs
        .iter()
        .find(|e| e["action"] == json!("ai.settings.key"))
        .expect("key event");
    assert_eq!(key["object"], json!("second one"));
    assert_eq!(
        key["detail"],
        json!({"secret": "apiKey", "change": "replace"})
    );
    let uf = evs
        .iter()
        .find(|e| e["action"] == json!("ai.users.features") && e["detail"].get("off").is_some())
        .expect("features event");
    assert_eq!(uf["object"], json!("sam"));
    // hint and explain are one switch: a request that names one moves both
    assert_eq!(uf["detail"]["off"], json!(["hint", "explain"]));
    let sf = evs
        .iter()
        .find(|e| e["action"] == json!("ai.settings.features"))
        .expect("switches event");
    assert_eq!(sf["detail"]["off"], json!(["hint", "explain"]));
}

#[tokio::test]
async fn a_failed_test_is_a_failed_event_with_the_code() {
    let (t, admin) = setup().await;
    let (_, v) = save_new(&t, &admin, "bad", &v1(&t), WRONG).await;
    assert_eq!(v["ok"], json!(false));
    let ev = admin_events(&t).await;
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0]["action"], json!("ai.settings.connectionAdd"));
    assert_eq!(ev[0]["result"], json!("fail"));
    assert_eq!(ev[0]["reason"], json!("ai.up.auth"));
    assert_eq!(ev[0]["object"], json!("bad"));
}

#[tokio::test]
async fn the_admin_routes_are_for_administrators_only() {
    let (t, admin) = setup().await;
    let sam = t.user("sam", false, true);
    for (path, body) in [
        (
            "/api/ai/admin/connections/save",
            json!({"name": "x", "provider": "openai"}),
        ),
        ("/api/ai/admin/connections/test", json!({"id": 1})),
        ("/api/ai/admin/connections/delete", json!({"id": 1})),
        ("/api/ai/admin/connections/order", json!({"ids": [1]})),
        (
            "/api/ai/admin/models/pick",
            json!({"slot": "chat", "clear": true}),
        ),
        (
            "/api/ai/admin/models/test",
            json!({"connectionId": 1, "model": "m"}),
        ),
        ("/api/ai/admin/models/remove", json!({"poolId": 1})),
        ("/api/ai/admin/feature-switches", json!({"chat": false})),
        ("/api/ai/admin/enable", json!({"enabled": false})),
    ] {
        let (s, v) = t.post(path, &sam, body).await;
        assert_eq!(s, 403, "{path}: {v}");
    }
    let (s, _) = t.get("/api/ai/admin/models", &sam).await;
    assert_eq!(s, 403);
    // nothing changed
    let s = settings(&t, &admin).await;
    assert_eq!(s["state"], json!("on"));
    assert_eq!(conn_ids(&s), vec![1]);
}

// —— hint + analysis: one switch ————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn the_hint_and_explain_switches_always_change_together() {
    let (t, admin) = setup().await;
    let pair = |v: &Value| {
        (
            v["featureSwitches"]["hint"].clone(),
            v["featureSwitches"]["explain"].clone(),
        )
    };
    let post = |body: Value| t.post("/api/ai/admin/feature-switches", &admin, body);
    // what the page sends: both keys
    let (s, v) = post(json!({"hint": false, "explain": false})).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(pair(&v), (json!(false), json!(false)));
    assert_eq!(v["featureSwitches"]["chat"], json!(true));
    // one key alone moves both, in either direction
    let (_, v) = post(json!({"explain": true})).await;
    assert_eq!(pair(&v), (json!(true), json!(true)));
    let (_, v) = post(json!({"hint": false})).await;
    assert_eq!(pair(&v), (json!(false), json!(false)));
    let (_, v) = post(json!({"hint": true, "summary": false})).await;
    assert_eq!(pair(&v), (json!(true), json!(true)));
    assert_eq!(v["featureSwitches"]["summary"], json!(false));
    // two different values in one request: the one that changes decides
    let (_, v) = post(json!({"hint": true, "explain": false})).await;
    assert_eq!(pair(&v), (json!(false), json!(false)));
    let bits: i64 = t
        .db()
        .query_row("SELECT features_off FROM settings WHERE id = 1", [], |r| {
            r.get(0)
        })
        .expect("bits");
    assert_eq!(bits, 2 | 4 | 8);
}
