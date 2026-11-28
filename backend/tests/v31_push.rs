//! v3.1 (P4b): the push engine on the backend: subscribe validation and moves, the ten-per-user cap, the queue (coalescing,
//! expiry times, quiet hours at enqueue and at lease), the lease with its four re-checks and the encrypted record, the results
//! and the late-result guard, the triggers (version, banks, grants), the housekeeping, the key clean-up, and the courier
//! routes with a fake courier.
//!
//! Real database and store, real router. A test that needs the crypto package (P4a: key, encryption, allow-list, texts) or
//! the devices package (a session that carries its device) says so and returns early until they are merged; at the
//! integration gate they all run. Only made-up data is used.
#[path = "api_common/mod.rs"]
mod apic;
#[path = "common/mod.rs"]
mod dbc;
#[path = "push_common/mod.rs"]
mod pc;

use base64::Engine as _;
use pc::{crypto_ready, decrypt, skipped, ts, Env, Out, DAY, FCM, HOUR, TOKEN};
use rusqlite::params;
use serde_json::{json, Value};
use xexam_backend::auth::Role;
use xexam_backend::database::store::{
    push_backoff_secs, PushJobResult, PushOutcome, PushSubscribe, PushSubscribed, PushTestOutcome,
};
use xexam_backend::push::b64_lenient;
use xexam_backend::security::base64url_nopad;

const MSG_BAD_REQUEST: &str = "请求的格式不对。";
const MSG_STALE: &str = "推送密钥已更换，请重新开启通知。";
const MSG_BAD_ENDPOINT: &str = "这个浏览器的推送地址不受支持。";
const MSG_BAD_KEYS: &str = "推送订阅的密钥不对。";
const MSG_OFF: &str = "你的账号已关闭通知，请先在设置里打开。";
const MSG_NO_DEVICE: &str = "这台设备还没有开启通知。";
const MSG_BUSY: &str = "测试通知发得太频繁了，请稍后再试。";

/// A way to arrange the database for one case of a table test.
type Setup<'a> = Box<dyn Fn(&Env) + 'a>;

fn noon() -> i64 {
    ts(2026, 6, 10, 12, 0)
}

fn sub_req(n: usize) -> PushSubscribe {
    let ua = pc::new_ua();
    PushSubscribe {
        endpoint: format!("{FCM}route-{n}"),
        p256dh: base64url_nopad(&ua.public),
        auth: base64url_nopad(&ua.auth),
        key_id: "0123456789abcdef".to_string(),
        expires_at: None,
    }
}

fn sub_count(e: &Env) -> i64 {
    e.int("SELECT COUNT(*) FROM push_subscriptions")
}

fn rows_of_kind(e: &Env, sub: i64, kind: &str) -> Vec<Out> {
    e.outbox_of(sub)
        .into_iter()
        .filter(|o| o.kind == kind)
        .collect()
}

/// A user with one device (zone `UTC`) and one subscription: `(user, device, subscription, key pair)`.
fn with_sub(e: &Env, user: i64) -> (i64, i64, pc::Ua) {
    let dev = e.device(user, Some("UTC"));
    let (sub, ua) = e.sub(dev);
    (dev, sub, ua)
}

/// A leased row for tests of the results (no crypto needed): state `leased`, `tries` as given.
fn leased_row(e: &Env, sub: i64, tries: i64) -> i64 {
    let id = e.raw_out(
        sub,
        "test",
        Some("test"),
        &json!({"k": "test"}),
        e.now(),
        600,
    );
    e.exec(
        "UPDATE push_outbox SET state = 'leased', tries = ?1, next_at = ?2 WHERE id = ?3",
        params![tries, e.now() + 120, id],
    );
    id
}

fn result(
    id: i64,
    tries: i64,
    outcome: PushOutcome,
    status: Option<i64>,
    retry_after: Option<i64>,
) -> PushJobResult {
    PushJobResult {
        id,
        tries,
        outcome,
        status,
        retry_after,
    }
}

// ---------------------------------------------------------------- subscriptions (store)

#[test]
fn a_new_subscription_is_stored_and_a_reconfirm_within_a_day_writes_nothing() {
    let e = Env::at(noon());
    let kim = e.user("kim");
    let dev = e.device(kim, Some("UTC"));
    let req = sub_req(1);
    let now = e.now();
    assert_eq!(
        e.store
            .push_subscribe(kim, dev, &req, now)
            .expect("subscribe"),
        PushSubscribed::New
    );
    let row = |e: &Env| {
        e.raw()
            .query_row(
                "SELECT endpoint, p256dh, auth, key_id, created_at, confirmed_at, expires_at, failures FROM push_subscriptions WHERE device_id = ?1",
                params![dev],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, i64>(5)?,
                        r.get::<_, Option<i64>>(6)?,
                        r.get::<_, i64>(7)?,
                    ))
                },
            )
            .expect("row")
    };
    let first = row(&e);
    assert_eq!(first.0, req.endpoint);
    assert_eq!((first.1.len(), first.2.len()), (87, 22));
    assert_eq!((first.4, first.5, first.6, first.7), (now, now, None, 0));
    // Within a day: nothing is written
    let r = e
        .store
        .push_subscribe(kim, dev, &req, now + HOUR)
        .expect("again");
    assert_eq!(r, PushSubscribed::Unchanged);
    assert_eq!(row(&e).5, now);
    // After a day: confirmed again (with the new expiry time)
    let mut later = req.clone();
    later.expires_at = Some(now + 30 * DAY);
    let r = e
        .store
        .push_subscribe(kim, dev, &later, now + DAY + 1)
        .expect("after a day");
    assert_eq!(r, PushSubscribed::Confirmed);
    let second = row(&e);
    assert_eq!(
        (second.5, second.6, second.4),
        (now + DAY + 1, Some(now + 30 * DAY), now)
    );
    assert_eq!(sub_count(&e), 1);
}

#[test]
fn an_endpoint_moves_to_the_device_that_confirms_it_last() {
    let e = Env::at(noon());
    let (kim, lee) = (e.user("kim"), e.user("lee"));
    let (dk, dl) = (e.device(kim, Some("UTC")), e.device(lee, Some("UTC")));
    let req = sub_req(7);
    let now = e.now();
    e.store.push_subscribe(kim, dk, &req, now).expect("kim");
    let sub_id = e.int("SELECT id FROM push_subscriptions");
    // A queued push of the old holder goes with the move
    let id = e.raw_out(
        sub_id,
        "test",
        Some("test"),
        &json!({"k": "test"}),
        now,
        600,
    );
    assert_eq!(e.outbox().len(), 1);
    // Lee had a subscription of their own: it is replaced
    let (own, _ua) = e.sub(dl);
    e.raw_out(own, "test", Some("test"), &json!({"k": "test"}), now, 600);
    let r = e
        .store
        .push_subscribe(lee, dl, &req, now + 5)
        .expect("move");
    assert_eq!(r, PushSubscribed::New);
    assert_eq!(sub_count(&e), 1);
    let holder = |e: &Env| {
        e.int(&format!(
            "SELECT device_id FROM push_subscriptions WHERE endpoint = '{}'",
            req.endpoint
        ))
    };
    assert_eq!(holder(&e), dl);
    assert!(
        e.outbox().is_empty(),
        "the old holder's and the replaced queued rows are gone (row {id})"
    );
    // Moved back to kim
    e.store
        .push_subscribe(kim, dk, &req, now + 10)
        .expect("back");
    assert_eq!(sub_count(&e), 1);
    assert_eq!(holder(&e), dk);
}

#[test]
fn a_device_has_one_subscription_and_a_new_endpoint_replaces_the_old_one() {
    let e = Env::at(noon());
    let kim = e.user("kim");
    let dev = e.device(kim, Some("UTC"));
    let now = e.now();
    e.store
        .push_subscribe(kim, dev, &sub_req(1), now)
        .expect("one");
    let old = e.int("SELECT id FROM push_subscriptions");
    e.raw_out(old, "test", Some("test"), &json!({"k": "test"}), now, 600);
    let second = sub_req(2);
    assert_eq!(
        e.store
            .push_subscribe(kim, dev, &second, now + 1)
            .expect("two"),
        PushSubscribed::New
    );
    assert_eq!(sub_count(&e), 1);
    assert_eq!(
        e.text("SELECT endpoint FROM push_subscriptions"),
        second.endpoint
    );
    assert!(e.outbox().is_empty());
}

#[test]
fn a_user_keeps_ten_subscriptions_and_the_oldest_confirmed_go() {
    let e = Env::at(noon());
    let kim = e.user("kim");
    let lee = e.user("lee");
    let lee_dev = e.device(lee, Some("UTC"));
    e.store
        .push_subscribe(lee, lee_dev, &sub_req(500), e.now())
        .expect("lee");
    let mut devices = Vec::new();
    for i in 0..12 {
        let dev = e.device(kim, Some("UTC"));
        devices.push(dev);
        e.store
            .push_subscribe(kim, dev, &sub_req(i), e.now() + i as i64)
            .expect("subscribe");
        assert!(sub_count(&e) <= 11, "kim keeps at most ten, lee one");
    }
    let kept: Vec<i64> = {
        let c = e.raw();
        let mut st = c
            .prepare("SELECT s.device_id FROM push_subscriptions s JOIN devices d ON d.id = s.device_id WHERE d.user_id = ?1 ORDER BY s.confirmed_at")
            .expect("prepare");
        st.query_map(params![kim], |r| r.get(0))
            .expect("query")
            .map(|r| r.expect("row"))
            .collect()
    };
    assert_eq!(
        kept,
        devices[2..].to_vec(),
        "the two oldest were dropped, the newest kept"
    );
    // Another user's subscription is untouched
    assert_eq!(
        e.int(&format!(
            "SELECT COUNT(*) FROM push_subscriptions WHERE device_id = {lee_dev}"
        )),
        1
    );
}

#[test]
fn the_switch_and_the_owner_of_the_device_are_checked_before_anything_is_stored() {
    let e = Env::at(noon());
    let (kim, lee) = (e.user("kim"), e.user("lee"));
    let dev = e.device(kim, Some("UTC"));
    e.set_setting(kim, "push", "false");
    assert_eq!(
        e.store
            .push_subscribe(kim, dev, &sub_req(1), e.now())
            .expect("off"),
        PushSubscribed::Off
    );
    assert_eq!(sub_count(&e), 0);
    e.set_setting(kim, "push", "true");
    // Lee cannot subscribe kim's device
    assert_eq!(
        e.store
            .push_subscribe(lee, dev, &sub_req(2), e.now())
            .expect("foreign"),
        PushSubscribed::NoDevice
    );
    assert_eq!(sub_count(&e), 0);
    // A missing `push` row means on
    e.exec("DELETE FROM user_settings WHERE key = 'push'", []);
    assert_eq!(
        e.store
            .push_subscribe(kim, dev, &sub_req(3), e.now())
            .expect("on"),
        PushSubscribed::New
    );
    // A disabled account is refused
    e.exec("UPDATE users SET disabled = 1 WHERE id = ?1", params![kim]);
    assert!(e
        .store
        .push_subscribe(kim, dev, &sub_req(4), e.now())
        .is_err());
}

#[test]
fn the_test_notification_checks_switch_then_subscription_then_throttle() {
    let e = Env::at(noon());
    let kim = e.user("kim");
    let dev = e.device(kim, Some("UTC"));
    let mut claimed = 0;
    // No subscription yet
    let r = e.store.push_test(kim, dev, e.now(), || {
        claimed += 1;
        true
    });
    assert_eq!(r.expect("test"), PushTestOutcome::NoDevice);
    assert_eq!(claimed, 0, "the throttle slot is claimed last");
    let (sub, _ua) = e.sub(dev);
    e.set_setting(kim, "push", "false");
    assert_eq!(
        e.store.push_test(kim, dev, e.now(), || true).expect("off"),
        PushTestOutcome::Off
    );
    e.set_setting(kim, "push", "true");
    assert_eq!(
        e.store
            .push_test(kim, dev, e.now(), || false)
            .expect("busy"),
        PushTestOutcome::Busy
    );
    assert!(e.outbox().is_empty());
    assert_eq!(
        e.store
            .push_test(kim, dev, e.now(), || true)
            .expect("queued"),
        PushTestOutcome::Queued
    );
    let rows = e.outbox_of(sub);
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(
        (r.kind.as_str(), r.topic.as_deref(), r.urgency.as_str()),
        ("test", Some("test"), "high")
    );
    // No quiet hours for a test: due at once, kept for ten minutes
    assert_eq!(
        (r.next_at, r.expires_at, r.state.as_str()),
        (e.now(), e.now() + 600, "queued")
    );
    // A second one is coalesced into the queued row
    e.store
        .push_test(kim, dev, e.now() + 20, || true)
        .expect("again");
    assert_eq!(e.outbox_of(sub).len(), 1);
    // At 03:00 local time the test is still due now
    let night = ts(2026, 6, 11, 3, 0);
    let e2 = Env::at(night);
    let kim2 = e2.user("kim");
    let dev2 = e2.device(kim2, Some("UTC"));
    let (sub2, _ua) = e2.sub(dev2);
    e2.store
        .push_test(kim2, dev2, night, || true)
        .expect("night");
    assert_eq!(e2.outbox_of(sub2)[0].next_at, night);
}

// ---------------------------------------------------------------- the HTTP routes

#[tokio::test]
async fn the_config_route_gives_the_key_and_courier_presence() {
    let e = Env::new();
    let r = e.call("GET", "/api/push/config", None, None).await;
    assert_eq!(r.status, 401, "{}", r.text());
    let cookie = e.login("kim").await;
    let r = e.get("/api/push/config", &cookie).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let j = r.json();
    assert_eq!(j["courier"], json!(false));
    if crypto_ready() {
        let key = e.key().expect("key");
        assert_eq!(j["publicKey"], json!(key.public_b64()));
        assert_eq!(j["publicKey"].as_str().expect("key").len(), 87);
        assert_eq!(j["keyId"].as_str().expect("id").len(), 16);
        assert_eq!(j["keyId"], json!(key.key_id()));
    } else {
        assert_eq!(
            (j["publicKey"].clone(), j["keyId"].clone()),
            (Value::Null, Value::Null)
        );
    }
    // A poll of the courier shows
    e.state.0.push.note_poll(e.now());
    let j = e.get("/api/push/config", &cookie).await.json();
    assert_eq!(j["courier"], json!(true));
    assert_eq!(
        j.as_object().expect("object").keys().collect::<Vec<_>>(),
        ["publicKey", "keyId", "courier"]
    );
    // Without a key the answer says so
    e.state.0.push.set_key(None, String::new());
    let j = e.get("/api/push/config", &cookie).await.json();
    assert_eq!(
        (j["publicKey"].clone(), j["keyId"].clone()),
        (Value::Null, Value::Null)
    );
}

#[tokio::test]
async fn the_subscribe_route_validates_in_order_and_stores_the_canonical_form() {
    if !crypto_ready() {
        return skipped("the push crypto (key, allow-list, public-key check)");
    }
    let e = Env::new();
    if !e.device_linking_works().await {
        return skipped("the devices package (a session that carries its device)");
    }
    let kim = e.user("kim");
    let cookie = e.login("kim").await;
    let dev = e.device(kim, Some("UTC"));
    e.link_session(&cookie, dev);
    let key_id = e.key_id();
    let ua = pc::new_ua();
    let good = |endpoint: &str| {
        json!({
            "endpoint": endpoint,
            "keys": {"p256dh": base64url_nopad(&ua.public), "auth": base64url_nopad(&ua.auth)},
            "expirationTime": null,
            "keyId": key_id,
        })
    };
    let coded = |r: &apic::Resp, status: u16, msg: &str, code: &str| {
        assert_eq!(r.status.as_u16(), status, "{}", r.text());
        assert_eq!(r.json(), json!({"error": msg, "code": code}));
    };

    // shapes
    for body in [
        json!({}),
        json!({"endpoint": 5, "keys": {"p256dh": "a", "auth": "b"}, "keyId": key_id}),
        json!({"endpoint": format!("{FCM}x"), "keys": "no", "keyId": key_id}),
        json!({"endpoint": format!("{FCM}x"), "keys": {"p256dh": "a"}, "keyId": key_id}),
        json!({"endpoint": format!("{FCM}x"), "keys": {"p256dh": "a", "auth": "b"}}),
        {
            let mut b = good(&format!("{FCM}x"));
            b["expirationTime"] = json!("soon");
            b
        },
        {
            let mut b = good(&format!("{FCM}x"));
            b["expirationTime"] = json!(-5);
            b
        },
    ] {
        let r = e.post("/api/push/subscribe", &cookie, body.clone()).await;
        assert_eq!(r.status, 400, "{body}: {}", r.text());
        assert_eq!(r.json(), json!({"error": MSG_BAD_REQUEST}), "{body}");
    }
    // the key id comes before the endpoint, the endpoint before the keys, the keys before the switch
    let mut b = good("https://evil.example/x");
    b["keyId"] = json!("0000000000000000");
    coded(
        &e.post("/api/push/subscribe", &cookie, b).await,
        409,
        MSG_STALE,
        "push.staleKey",
    );
    for endpoint in [
        "https://evil.example/x",
        "http://fcm.googleapis.com/fcm/send/x",
        "https://127.0.0.1/x",
        "https://fcm.googleapis.com",
    ] {
        let mut b = good(endpoint);
        b["keys"]["auth"] = json!("short");
        coded(
            &e.post("/api/push/subscribe", &cookie, b).await,
            400,
            MSG_BAD_ENDPOINT,
            "push.badEndpoint",
        );
    }
    let bad_keys: Vec<(&str, Value)> = vec![
        (
            "p256dh too short",
            json!({"p256dh": base64url_nopad(&ua.public[..64]), "auth": base64url_nopad(&ua.auth)}),
        ),
        ("p256dh not 0x04", {
            let mut p = ua.public;
            p[0] = 0x02;
            json!({"p256dh": base64url_nopad(&p), "auth": base64url_nopad(&ua.auth)})
        }),
        ("not on the curve", {
            let mut p = [0x44u8; 65];
            p[0] = 0x04;
            json!({"p256dh": base64url_nopad(&p), "auth": base64url_nopad(&ua.auth)})
        }),
        (
            "auth 15 bytes",
            json!({"p256dh": base64url_nopad(&ua.public), "auth": base64url_nopad(&ua.auth[..15])}),
        ),
        (
            "p256dh not base64",
            json!({"p256dh": "***", "auth": base64url_nopad(&ua.auth)}),
        ),
    ];
    e.set_setting(kim, "push", "false");
    for (label, keys) in bad_keys {
        let mut b = good(&format!("{FCM}x"));
        b["keys"] = keys;
        let r = e.post("/api/push/subscribe", &cookie, b).await;
        assert_eq!(r.status, 400, "{label}: {}", r.text());
        assert_eq!(
            r.json(),
            json!({"error": MSG_BAD_KEYS, "code": "push.badKeys"}),
            "{label}"
        );
    }
    // everything valid but the switch is off: 409 and nothing stored
    coded(
        &e.post("/api/push/subscribe", &cookie, good(&format!("{FCM}x")))
            .await,
        409,
        MSG_OFF,
        "push.off",
    );
    assert_eq!(sub_count(&e), 0);
    e.set_setting(kim, "push", "true");

    // success; padding and the standard alphabet are tolerated, the stored form is canonical
    let mut b = good(&format!("{FCM}ok"));
    b["keys"]["p256dh"] = json!(base64::engine::general_purpose::STANDARD.encode(ua.public));
    b["keys"]["auth"] = json!(base64::engine::general_purpose::URL_SAFE.encode(ua.auth));
    b["expirationTime"] = json!(1_900_000_000_999_i64);
    let r = e.post("/api/push/subscribe", &cookie, b.clone()).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(r.json().get("device").is_some(), "{}", r.text());
    assert_eq!(sub_count(&e), 1);
    assert_eq!(
        e.text("SELECT p256dh FROM push_subscriptions"),
        base64url_nopad(&ua.public)
    );
    assert_eq!(
        e.text("SELECT auth FROM push_subscriptions"),
        base64url_nopad(&ua.auth)
    );
    assert_eq!(e.text("SELECT key_id FROM push_subscriptions"), key_id);
    assert_eq!(
        e.int("SELECT expires_at FROM push_subscriptions"),
        1_900_000_000
    );
    assert_eq!(e.int("SELECT failures FROM push_subscriptions"), 0);
    assert_eq!(e.int("SELECT device_id FROM push_subscriptions"), dev);
    // a brand-new subscription is recorded once, a re-confirm is not
    assert_eq!(e.entries("settings.pushOn").await.len(), 1);
    let confirmed = e.int("SELECT confirmed_at FROM push_subscriptions");
    e.advance(60);
    let r = e.post("/api/push/subscribe", &cookie, b).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(e.entries("settings.pushOn").await.len(), 1);
    assert_eq!(
        e.int("SELECT confirmed_at FROM push_subscriptions"),
        confirmed
    );
    // a float expirationTime is accepted too
    let mut b2 = good(&format!("{FCM}float"));
    b2["expirationTime"] = json!(1_900_000_000_500.5);
    assert_eq!(e.post("/api/push/subscribe", &cookie, b2).await.status, 200);
    assert_eq!(
        e.int("SELECT expires_at FROM push_subscriptions"),
        1_900_000_000
    );
    // the route needs a session
    let r = e
        .call(
            "POST",
            "/api/push/subscribe",
            None,
            Some(good(&format!("{FCM}z")).to_string()),
        )
        .await;
    assert_eq!(r.status, 401);
}

#[tokio::test]
async fn the_test_route_answers_off_no_device_busy_and_queues_one() {
    let e = Env::new();
    if !e.device_linking_works().await {
        return skipped("the devices package (a session that carries its device)");
    }
    let kim = e.user("kim");
    let cookie = e.login("kim").await;
    let dev = e.device(kim, Some("UTC"));
    e.link_session(&cookie, dev);
    let coded = |r: &apic::Resp, status: u16, msg: &str, code: &str| {
        assert_eq!(r.status.as_u16(), status, "{}", r.text());
        assert_eq!(r.json(), json!({"error": msg, "code": code}));
    };
    coded(
        &e.post("/api/push/test", &cookie, json!({})).await,
        409,
        MSG_NO_DEVICE,
        "push.noDevice",
    );
    let (sub, _ua) = e.sub(dev);
    e.set_setting(kim, "push", "false");
    coded(
        &e.post("/api/push/test", &cookie, json!({})).await,
        409,
        MSG_OFF,
        "push.off",
    );
    e.set_setting(kim, "push", "true");
    let r = e.post("/api/push/test", &cookie, json!({})).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json(), json!({"ok": true, "courier": false}));
    assert_eq!(e.outbox_of(sub).len(), 1);
    coded(
        &e.post("/api/push/test", &cookie, json!({})).await,
        429,
        MSG_BUSY,
        "push.testBusy",
    );
    // Ten seconds later it works again (the clock is the store's)
    e.advance(11);
    e.state.0.push.note_poll(e.now());
    let r = e.post("/api/push/test", &cookie, json!({})).await;
    assert_eq!(r.json(), json!({"ok": true, "courier": true}));
    // `settings.pushTest` is not an "important" action: only an administrator's tests reach the activity log
    assert!(e.entries("settings.pushTest").await.is_empty());
    let r = e
        .call("POST", "/api/push/test", None, Some("{}".into()))
        .await;
    assert_eq!(r.status, 401);
}

#[tokio::test]
async fn a_session_without_a_device_gets_a_plain_bad_request() {
    let e = Env::new();
    if e.device_linking_works().await {
        // The devices package links the session at its first request; this check is for the unlinked case only
        return skipped("the devices package links sessions, so every session has a device");
    }
    let cookie = e.login("kim").await;
    for path in ["/api/push/subscribe", "/api/push/test", "/api/study/saved"] {
        let r = e.post(path, &cookie, json!({})).await;
        assert_eq!(r.status, 400, "{path}: {}", r.text());
    }
}

// ---------------------------------------------------------------- bank pushes

fn make_manager(e: &Env, user: i64) {
    e.exec(
        "UPDATE users SET can_manage_banks = 1 WHERE id = ?1",
        params![user],
    );
}

struct Fx {
    admin: i64,
    ann: i64,
    kim: i64,
    lee: i64,
    subs: Vec<(String, i64)>,
}

impl Fx {
    fn sub_of(&self, who: &str) -> i64 {
        self.subs.iter().find(|(n, _)| n == who).expect("sub").1
    }
    fn who_got(&self, e: &Env, kind: &str) -> Vec<&str> {
        self.subs
            .iter()
            .filter(|(_, s)| !rows_of_kind(e, *s, kind).is_empty())
            .map(|(n, _)| n.as_str())
            .collect()
    }
}

/// admin (1), a second administrator `ann`, the bank manager `kim`, the plain user `lee`; each has one subscribed device.
fn fixture(e: &Env) -> Fx {
    let admin = e.user("admin");
    let ann = e.new_user("ann", Role::Admin);
    let kim = e.user("kim");
    let lee = e.user("lee");
    make_manager(e, kim);
    let mut subs = Vec::new();
    for (name, id) in [("admin", admin), ("ann", ann), ("kim", kim), ("lee", lee)] {
        let (_, sub, _) = with_sub(e, id);
        subs.push((name.to_string(), sub));
    }
    Fx {
        admin,
        ann,
        kim,
        lee,
        subs,
    }
}

#[test]
fn the_first_tick_only_sets_the_marks_so_old_banks_are_not_announced() {
    let e = Env::at(noon());
    let fx = fixture(&e);
    e.import_new(None, "Herbs", 2);
    assert_eq!(
        e.opt_int("SELECT COUNT(*) FROM meta WHERE key = 'push_banks_seen'"),
        Some(0)
    );
    assert_eq!(e.store.push_bank_tick(e.now()).expect("tick"), 0);
    assert!(e.outbox().is_empty());
    let banks = e.int("SELECT MAX(id) FROM banks");
    assert_eq!(
        e.text("SELECT value FROM meta WHERE key = 'push_banks_seen'"),
        banks.to_string()
    );
    let imports = e.int("SELECT MAX(id) FROM imports");
    assert_eq!(
        e.text("SELECT value FROM meta WHERE key = 'push_imports_seen'"),
        imports.to_string()
    );
    // A second tick with nothing new writes nothing and queues nothing
    assert_eq!(e.store.push_bank_tick(e.now()).expect("tick"), 0);
    assert!(e.outbox().is_empty());
    let _ = fx;
}

#[test]
fn a_new_bank_goes_to_the_importer_and_every_administrator() {
    let e = Env::at(noon());
    let fx = fixture(&e);
    e.mark_banks_seen();
    let bank = e.import_new(Some(&e.viewer(fx.kim)), "Herbs", 3);
    assert!(e.store.push_bank_tick(e.now()).expect("tick") >= 3);
    assert_eq!(
        fx.who_got(&e, "bank"),
        ["admin", "ann", "kim"],
        "lee holds no right to the bank"
    );
    for who in ["admin", "ann", "kim"] {
        let rows = rows_of_kind(&e, fx.sub_of(who), "bank");
        assert_eq!(rows.len(), 1, "{who}");
        let r = &rows[0];
        assert_eq!(r.topic, Some(format!("bank-{bank}")));
        assert_eq!(
            r.payload,
            json!({"k": "bank.new", "bank": bank, "name": "Herbs", "total": 3})
        );
        assert_eq!(
            (r.urgency.as_str(), r.state.as_str(), r.next_at),
            ("low", "queued", e.now())
        );
        assert_eq!(r.expires_at, e.now() + 3 * DAY);
    }
    // Ticking again announces nothing twice
    assert_eq!(e.store.push_bank_tick(e.now()).expect("tick"), 0);
    assert_eq!(e.outbox().len(), 3);
}

#[test]
fn a_command_line_import_is_announced_too() {
    let e = Env::at(noon());
    let fx = fixture(&e);
    e.mark_banks_seen();
    // "include banks imported later": lee gets a user_banks row in the importing transaction
    e.exec(
        "UPDATE users SET new_banks = 1 WHERE id = ?1",
        params![fx.lee],
    );
    let bank = e.import_new(None, "Spices", 4);
    e.store.push_bank_tick(e.now()).expect("tick");
    assert_eq!(fx.who_got(&e, "bank"), ["admin", "ann", "lee"]);
    let r = &rows_of_kind(&e, fx.sub_of("lee"), "bank")[0];
    assert_eq!(r.payload["bank"], bank);
    assert_eq!(r.payload["k"], "bank.new");
}

#[test]
fn an_import_into_an_existing_bank_is_announced_as_an_update_with_the_added_count() {
    let e = Env::at(noon());
    let fx = fixture(&e);
    let bank = e.import_new(Some(&e.viewer(fx.kim)), "Herbs", 3);
    e.grant(fx.lee, bank);
    e.mark_banks_seen();
    e.import_more(Some(&e.viewer(fx.kim)), bank, "Herbs", 5);
    let added = e.int("SELECT added + restored FROM imports ORDER BY id DESC LIMIT 1");
    assert_eq!(added, 2);
    e.store.push_bank_tick(e.now()).expect("tick");
    assert_eq!(fx.who_got(&e, "bank"), ["admin", "ann", "kim", "lee"]);
    for who in ["admin", "kim", "lee"] {
        let r = &rows_of_kind(&e, fx.sub_of(who), "bank")[0];
        assert_eq!(
            r.payload,
            json!({"k": "bank.updated", "bank": bank, "name": "Herbs", "added": 2, "total": 5}),
            "{who}"
        );
    }
    // An import that changes nothing says nothing
    let before = e.outbox().len();
    e.import_more(Some(&e.viewer(fx.kim)), bank, "Herbs", 5);
    assert_eq!(
        e.int("SELECT added + restored FROM imports ORDER BY id DESC LIMIT 1"),
        0
    );
    e.store.push_bank_tick(e.now()).expect("tick");
    assert_eq!(e.outbox().len(), before);
}

#[test]
fn two_updates_before_delivery_coalesce_into_one_row() {
    let e = Env::at(noon());
    let fx = fixture(&e);
    let bank = e.import_new(None, "Herbs", 3);
    e.mark_banks_seen();
    e.import_more(None, bank, "Herbs", 5);
    e.store.push_bank_tick(e.now()).expect("tick");
    let first = rows_of_kind(&e, fx.sub_of("admin"), "bank")[0].clone();
    assert_eq!(first.payload["added"], 2);
    e.set_now(noon() + 1800);
    e.import_more(None, bank, "Herbs", 9);
    e.store.push_bank_tick(e.now()).expect("tick");
    let rows = rows_of_kind(&e, fx.sub_of("admin"), "bank");
    assert_eq!(rows.len(), 1, "one row per subscription and topic");
    assert_eq!(rows[0].id, first.id);
    assert_eq!(
        rows[0].payload,
        json!({"k": "bank.updated", "bank": bank, "name": "Herbs", "added": 6, "total": 9})
    );
    // expires_at = max(old, max(created_at, next_at) + TTL of the new one): the later push extends it
    assert_eq!(rows[0].expires_at, noon() + 1800 + 3 * DAY);
    assert!(rows[0].expires_at > first.expires_at);
    assert_eq!(
        rows[0].next_at, first.next_at,
        "next_at is kept unless the new one is earlier"
    );
    // A leased row is never touched: the next update gets a row of its own
    e.exec(
        "UPDATE push_outbox SET state = 'leased', tries = 1, next_at = ?1",
        params![e.now() + 120],
    );
    e.set_now(noon() + 3600);
    e.import_more(None, bank, "Herbs", 10);
    e.store.push_bank_tick(e.now()).expect("tick");
    let rows = rows_of_kind(&e, fx.sub_of("admin"), "bank");
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (rows[0].state.as_str(), rows[0].payload["added"].as_i64()),
        ("leased", Some(6))
    );
    assert_eq!(
        (rows[1].state.as_str(), rows[1].payload["added"].as_i64()),
        ("queued", Some(1))
    );
}

#[test]
fn an_update_after_a_queued_new_bank_stays_a_new_bank_with_the_newest_total() {
    let e = Env::at(noon());
    let fx = fixture(&e);
    e.mark_banks_seen();
    let bank = e.import_new(None, "Herbs", 3);
    e.store.push_bank_tick(e.now()).expect("tick");
    e.import_more(None, bank, "Herbs", 6);
    e.store.push_bank_tick(e.now()).expect("tick");
    let rows = rows_of_kind(&e, fx.sub_of("admin"), "bank");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].payload,
        json!({"k": "bank.new", "bank": bank, "name": "Herbs", "total": 6})
    );
}

#[test]
fn an_ai_generated_bank_goes_only_to_the_users_holding_it() {
    let e = Env::at(noon());
    let fx = fixture(&e);
    e.mark_banks_seen();
    // Created by lee, shared with kim; the administrators see every bank but are not told about a personal one
    let bank = e.ai_bank("Kitchen basics", fx.lee, &[fx.lee, fx.kim]);
    e.store.push_bank_tick(e.now()).expect("tick");
    assert_eq!(fx.who_got(&e, "bank"), ["kim", "lee"]);
    assert_eq!(
        rows_of_kind(&e, fx.sub_of("lee"), "bank")[0].payload,
        json!({"k": "bank.new", "bank": bank, "name": "Kitchen basics", "total": 1})
    );
    // ...although the reverse of viewer_of lists the administrators for it
    let viewers = e.store.push_bank_viewers(bank).expect("viewers");
    assert!(viewers.contains(&fx.admin) && viewers.contains(&fx.ann));
}

#[test]
fn bank_viewers_is_the_reverse_of_viewer_of_for_every_user() {
    let e = Env::at(noon());
    let fx = fixture(&e);
    let zed = e.new_user("zed", Role::User);
    let b1 = e.import_new(Some(&e.viewer(fx.kim)), "Herbs", 2);
    let b2 = e.import_new(None, "Spices", 2);
    let b3 = e.ai_bank("Kitchen basics", fx.lee, &[fx.lee]);
    e.grant(fx.lee, b1);
    e.grant(zed, b2);
    // kim keeps owning b1 but loses the right to manage: no longer a viewer of b1 unless granted
    for (label, manage_kim) in [("manager", true), ("not a manager", false)] {
        e.exec(
            "UPDATE users SET can_manage_banks = ?1 WHERE id = ?2",
            params![manage_kim, fx.kim],
        );
        for bank in [b1, b2, b3] {
            let viewers = e.store.push_bank_viewers(bank).expect("viewers");
            for user in [fx.admin, fx.ann, fx.kim, fx.lee, zed] {
                let v = e.viewer(user);
                let sees = v.admin || v.scope.allows(Some(bank));
                assert_eq!(
                    viewers.contains(&user),
                    sees,
                    "{label}: user {user}, bank {bank}"
                );
            }
        }
    }
    // Disabled users are left out
    e.exec("UPDATE users SET disabled = 1 WHERE id = ?1", params![zed]);
    assert!(!e
        .store
        .push_bank_viewers(b2)
        .expect("viewers")
        .contains(&zed));
}

#[test]
fn a_bank_without_active_questions_and_a_disabled_user_are_not_announced() {
    let e = Env::at(noon());
    let fx = fixture(&e);
    e.mark_banks_seen();
    let empty = e.import_new(None, "Herbs", 2);
    e.exec(
        "UPDATE questions SET active = 0 WHERE bank_id = ?1",
        params![empty],
    );
    e.exec(
        "UPDATE users SET disabled = 1 WHERE id = ?1",
        params![fx.ann],
    );
    let full = e.import_new(None, "Spices", 2);
    e.store.push_bank_tick(e.now()).expect("tick");
    assert_eq!(fx.who_got(&e, "bank"), ["admin"]);
    assert_eq!(
        rows_of_kind(&e, fx.sub_of("admin"), "bank")[0].payload["bank"],
        full
    );
    // The empty bank is not announced later either: its id is below the mark now
    e.exec(
        "UPDATE questions SET active = 1 WHERE bank_id = ?1",
        params![empty],
    );
    e.store.push_bank_tick(e.now()).expect("tick");
    assert_eq!(rows_of_kind(&e, fx.sub_of("admin"), "bank").len(), 1);
}

#[test]
fn bank_pushes_wait_for_the_end_of_the_quiet_hours() {
    if !crypto_ready() {
        return skipped("the push package (quiet hours)");
    }
    let night = ts(2026, 6, 11, 2, 0);
    let e = Env::at(night);
    let fx = fixture(&e);
    e.mark_banks_seen();
    e.import_new(None, "Herbs", 2);
    e.store.push_bank_tick(night).expect("tick");
    let r = &rows_of_kind(&e, fx.sub_of("admin"), "bank")[0];
    assert_eq!(r.next_at, ts(2026, 6, 11, 8, 0));
    // The full TTL counts from the release
    assert_eq!(r.expires_at, ts(2026, 6, 11, 8, 0) + 3 * DAY);
    // Not due before 08:00
    assert!(!e.store.push_due(ts(2026, 6, 11, 7, 59)).expect("due"));
    assert!(e.store.push_due(ts(2026, 6, 11, 8, 0)).expect("due"));
    assert_eq!(
        e.store.push_next_due(night).expect("next"),
        Some(ts(2026, 6, 11, 8, 0))
    );
    // A user at UTC+9: 02:00 UTC is 11:00 there, so it is sendable at once
    let e = Env::at(night);
    let admin = e.user("admin");
    let dev = e.device(admin, Some("UTC"));
    let (sub, _ua) = e.sub(dev);
    e.set_setting(admin, "tz", "\"Etc/GMT-9\"");
    e.mark_banks_seen();
    e.import_new(None, "Herbs", 2);
    e.store.push_bank_tick(night).expect("tick");
    assert_eq!(rows_of_kind(&e, sub, "bank")[0].next_at, night);
}

// ---------------------------------------------------------------- version trigger

fn start_row(e: &Env, version: &str) {
    e.exec(
        "INSERT INTO activity_log(ts, last_ts, count, actor, action, detail, result) VALUES(?1, ?1, 1, 'system', 'system.start', ?2, 'ok')",
        params![e.now(), json!({"version": version}).to_string()],
    );
}

fn app_version(e: &Env) -> Option<String> {
    e.raw()
        .query_row(
            "SELECT value FROM meta WHERE key = 'app_version'",
            [],
            |r| r.get(0),
        )
        .ok()
}

#[test]
fn the_version_push_follows_the_cases_of_the_contract() {
    let e = Env::at(noon());
    let kim = e.user("kim");
    let (_, sub, _) = with_sub(&e, kim);
    let check = |e: &Env, v: &str| e.store.push_version_check(v, e.now()).expect("check");

    // A fresh install (no start row, nothing announced): nothing, and the running version is noted
    assert_eq!(check(&e, "3.1"), None);
    assert_eq!(app_version(&e).as_deref(), Some("3.1"));
    assert!(e.outbox().is_empty());
    // Same version again: nothing
    assert_eq!(check(&e, "3.1"), None);
    // 3.1 -> 3.2: queued to the eligible subscription, held like any push
    assert_eq!(
        check(&e, "3.2"),
        Some((Some("3.1".to_string()), "3.2".to_string(), 1))
    );
    assert_eq!(app_version(&e).as_deref(), Some("3.2"));
    let rows = rows_of_kind(&e, sub, "version");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].payload,
        json!({"k": "version", "from": "3.1", "to": "3.2"})
    );
    assert_eq!(rows[0].topic.as_deref(), Some("version"));
    assert_eq!(
        (rows[0].urgency.as_str(), rows[0].expires_at),
        ("low", e.now() + 3 * DAY)
    );
    // A downgrade tells nobody and keeps the maximum
    assert_eq!(check(&e, "3.1"), None);
    assert_eq!(app_version(&e).as_deref(), Some("3.2"));
    // The comparison is by dotted numbers: 3.10 is above 3.9
    e.exec(
        "UPDATE meta SET value = '3.9' WHERE key = 'app_version'",
        [],
    );
    assert!(check(&e, "3.10").is_some());
    e.exec(
        "UPDATE meta SET value = '3.10' WHERE key = 'app_version'",
        [],
    );
    assert_eq!(check(&e, "3.9"), None);
    // A version that is not a number never counts as higher
    assert_eq!(check(&e, "3.11-rc1"), None);
}

#[test]
fn the_upgrade_from_a_binary_that_never_announced_uses_the_last_start_row() {
    let e = Env::at(noon());
    let kim = e.user("kim");
    let (_, sub, _) = with_sub(&e, kim);
    // v3.0 -> v3.1: no `app_version` yet, the start row of the old binary says 3.0; no device had a subscription then, so in
    // production nobody is reached (here one exists on purpose)
    start_row(&e, "3.0");
    let r = e.store.push_version_check("3.1", e.now()).expect("check");
    assert_eq!(r, Some((Some("3.0".to_string()), "3.1".to_string(), 1)));
    assert_eq!(rows_of_kind(&e, sub, "version").len(), 1);
    assert_eq!(app_version(&e).as_deref(), Some("3.1"));
    // The same version at the next start: nothing
    start_row(&e, "3.1");
    assert_eq!(
        e.store.push_version_check("3.1", e.now()).expect("check"),
        None
    );

    // A last start of a higher or equal version with nothing announced: noted, no push
    let e = Env::at(noon());
    start_row(&e, "3.1");
    assert_eq!(
        e.store.push_version_check("3.1", e.now()).expect("check"),
        None
    );
    assert_eq!(app_version(&e).as_deref(), Some("3.1"));
    let e = Env::at(noon());
    start_row(&e, "3.5");
    assert_eq!(
        e.store.push_version_check("3.1", e.now()).expect("check"),
        None
    );

    // The release reaches nobody when nobody subscribed (D26)
    let e = Env::at(noon());
    start_row(&e, "3.0");
    assert_eq!(
        e.store.push_version_check("3.1", e.now()).expect("check"),
        Some((Some("3.0".to_string()), "3.1".to_string(), 0))
    );
    assert!(e.outbox().is_empty());
}

#[test]
fn the_version_push_reaches_only_eligible_subscriptions() {
    let e = Env::at(noon());
    let (kim, lee) = (e.user("kim"), e.user("lee"));
    let ann = e.new_user("ann", Role::User);
    let bob = e.new_user("bob", Role::User);
    let (_, s_kim, _) = with_sub(&e, kim);
    let (_, s_lee, _) = with_sub(&e, lee);
    let (d_ann, s_ann, _) = with_sub(&e, ann);
    let (_, s_bob, _) = with_sub(&e, bob);
    e.set_setting(lee, "push", "false");
    e.exec(
        "UPDATE devices SET signed_out_at = ?1 WHERE id = ?2",
        params![e.now(), d_ann],
    );
    e.exec("UPDATE users SET disabled = 1 WHERE id = ?1", params![bob]);
    e.store.push_version_check("3.0", e.now()).expect("first");
    let r = e.store.push_version_check("3.1", e.now()).expect("second");
    assert_eq!(r, Some((Some("3.0".to_string()), "3.1".to_string(), 1)));
    assert_eq!(rows_of_kind(&e, s_kim, "version").len(), 1);
    for s in [s_lee, s_ann, s_bob] {
        assert!(rows_of_kind(&e, s, "version").is_empty());
    }
}

#[test]
fn the_start_up_marks_skip_what_other_binaries_did() {
    let e = Env::at(noon());
    e.import_new(None, "Herbs", 2);
    // marks missing: set to the current ids
    e.store.push_version_check("3.1", e.now()).expect("check");
    let banks = e.int("SELECT MAX(id) FROM banks").to_string();
    assert_eq!(
        e.text("SELECT value FROM meta WHERE key = 'push_banks_seen'"),
        banks
    );
    // the same version ran before: marks untouched
    start_row(&e, "3.1");
    e.exec(
        "UPDATE meta SET value = '0' WHERE key = 'push_banks_seen'",
        [],
    );
    e.store.push_version_check("3.1", e.now()).expect("check");
    assert_eq!(
        e.text("SELECT value FROM meta WHERE key = 'push_banks_seen'"),
        "0"
    );
    // another version ran before: marks move up
    start_row(&e, "3.0");
    e.store.push_version_check("3.1", e.now()).expect("check");
    assert_eq!(
        e.text("SELECT value FROM meta WHERE key = 'push_banks_seen'"),
        banks
    );
}

// ---------------------------------------------------------------- the lease

#[test]
fn a_leased_message_is_encrypted_signed_and_marked_leased() {
    if !crypto_ready() {
        return skipped("the push crypto and texts");
    }
    let e = Env::at(noon());
    let kim = e.user("kim");
    let (_, sub, ua) = with_sub(&e, kim);
    let id = e.raw_out(
        sub,
        "test",
        Some("test"),
        &json!({"k": "test"}),
        e.now(),
        600,
    );
    let jobs = e.lease();
    assert_eq!(jobs.len(), 1);
    let j = &jobs[0];
    assert_eq!((j.id, j.tries), (id, 1));
    assert_eq!(
        j.endpoint,
        e.text("SELECT endpoint FROM push_subscriptions")
    );
    assert_eq!(
        (j.ttl, j.urgency.as_str(), j.topic.as_deref()),
        (600, "normal", Some("test"))
    );
    assert!(
        j.authorization.starts_with("vapid t="),
        "{}",
        j.authorization
    );
    assert!(j.authorization.contains(", k="));
    assert!(!j.authorization.contains(['\r', '\n']) && j.authorization.len() <= 1024);
    let record = b64_lenient(&j.body).expect("body is base64url");
    assert!(record.len() <= 4096);
    let plain = String::from_utf8(decrypt(ua, &record)).expect("utf-8");
    assert_eq!(
        plain,
        r#"{"v":1,"title":"Xexam","body":"This is a test notification.","tag":"test","url":"/#/settings","lang":"en"}"#
    );
    let row = e.out(id);
    assert_eq!(
        (row.state.as_str(), row.tries, row.next_at),
        ("leased", 1, e.now() + 120)
    );
    // Within the lease nothing is handed out again; after it, the same row comes back with tries + 1 (at least once)
    assert!(e.lease().is_empty());
    e.advance(121);
    let again = e.lease();
    assert_eq!((again.len(), again[0].id, again[0].tries), (1, id, 2));
}

#[test]
fn the_tag_is_one_per_kind_and_never_the_topic() {
    if !crypto_ready() {
        return skipped("the push crypto and texts");
    }
    let e = Env::at(noon());
    let kim = e.user("kim");
    let bank = e.import_new(None, "Herbs", 3);
    e.grant(kim, bank);
    let key = "0123456789abcdef0123456789abcdef";
    let cases = [
        (
            "bank.new",
            json!({"k": "bank.new", "bank": bank, "name": "Herbs", "total": 3}),
            format!("bank-{bank}"),
            "bank",
            "bank",
        ),
        (
            "bank.grantMany",
            json!({"k": "bank.grantMany", "n": 3}),
            "bank-access".to_string(),
            "bank",
            "bank",
        ),
        (
            "version",
            json!({"k": "version", "from": "3.0", "to": "3.1"}),
            "version".to_string(),
            "version",
            "version",
        ),
        (
            "remind",
            json!({"k": "remind", "kind": "practice", "answered": 2, "total": 9, "n": 1, "key": key}),
            "remind".to_string(),
            "remind",
            "remind",
        ),
    ];
    for (label, payload, topic, kind, tag) in cases {
        // one subscription per device: every case gets its own device
        let dev = e.device(kim, Some("UTC"));
        let (sub, ua) = e.sub(dev);
        if label == "remind" {
            e.exec(
                "INSERT INTO saved_sessions(user_id, session_key, device_id, kind, answered, total, started_at, saved_at, remind_at, reminders) \
                 VALUES(?1, ?2, ?3, 'practice', 2, 9, 1780000000, ?4, NULL, 1)",
                params![kim, key, dev, e.now()],
            );
        }
        e.raw_out(sub, kind, Some(&topic), &payload, e.now(), 3600);
        let jobs = e.lease();
        let j = jobs
            .iter()
            .find(|j| j.topic.as_deref() == Some(topic.as_str()))
            .unwrap_or_else(|| panic!("{label}: no job"));
        let v: Value = serde_json::from_slice(&decrypt(ua, &b64_lenient(&j.body).expect("b64")))
            .expect("json");
        assert_eq!(v["tag"], tag, "{label}");
        if kind == "bank" {
            assert_ne!(
                v["tag"],
                json!(topic),
                "{label}: the tag is not the per-bank topic"
            );
        }
        assert_eq!(v["title"], "Xexam");
        assert_eq!(v["v"], 1);
    }
}

#[test]
fn each_recheck_expires_the_row_without_handing_it_out() {
    if !crypto_ready() {
        return skipped("the push crypto and texts");
    }
    // (a) eligibility: key, switch, disabled, signed out, unused for 30 days, subscription gone
    for case in [
        "stale key",
        "switch off",
        "disabled",
        "signed out",
        "unused 31 days",
    ] {
        let e = Env::at(noon());
        let kim = e.user("kim");
        let dev = e.device(kim, Some("UTC"));
        let key_id = if case == "stale key" {
            "ffffffffffffffff".to_string()
        } else {
            e.key_id()
        };
        let (sub, _ua) = e.sub_with(dev, &key_id);
        let id = e.raw_out(
            sub,
            "test",
            Some("test"),
            &json!({"k": "test"}),
            e.now(),
            600,
        );
        match case {
            "switch off" => e.set_setting(kim, "push", "false"),
            "disabled" => e.exec("UPDATE users SET disabled = 1 WHERE id = ?1", params![kim]),
            "signed out" => e.exec("UPDATE devices SET signed_out_at = ?1", params![e.now()]),
            "unused 31 days" => e.exec(
                "UPDATE devices SET active_at = ?1",
                params![e.now() - 31 * DAY],
            ),
            _ => {}
        }
        assert!(e.lease().is_empty(), "{case}");
        let row = e.out(id);
        assert_eq!(
            (row.state.as_str(), row.done_at),
            ("expired", Some(e.now())),
            "{case}"
        );
    }
    // (b) a bank push: the bank is gone, or the user may not see it any more
    let e = Env::at(noon());
    let (kim, lee) = (e.user("kim"), e.user("lee"));
    let bank = e.import_new(None, "Herbs", 2);
    let other = e.import_new(None, "Spices", 2);
    e.grant(kim, bank);
    e.grant(lee, other);
    let (_, s_kim, _) = with_sub(&e, kim);
    let (_, s_lee, _) = with_sub(&e, lee);
    let payload = |b: i64| json!({"k": "bank.new", "bank": b, "name": "x", "total": 2});
    let ok = e.raw_out(
        s_kim,
        "bank",
        Some(&format!("bank-{bank}")),
        &payload(bank),
        e.now(),
        3600,
    );
    let lost = e.raw_out(
        s_lee,
        "bank",
        Some(&format!("bank-{bank}")),
        &payload(bank),
        e.now(),
        3600,
    );
    let gone = e.raw_out(
        s_lee,
        "bank",
        Some("bank-999"),
        &payload(999),
        e.now(),
        3600,
    );
    let jobs = e.lease();
    assert_eq!(jobs.iter().map(|j| j.id).collect::<Vec<_>>(), [ok]);
    assert_eq!(e.out(lost).state, "expired", "lee never held that bank");
    assert_eq!(e.out(gone).state, "expired", "the bank does not exist");
    // a grant push for several banks is not re-checked
    let many = e.raw_out(
        s_lee,
        "bank",
        Some("bank-access"),
        &json!({"k": "bank.grantMany", "n": 2}),
        e.now(),
        3600,
    );
    assert_eq!(e.lease().iter().map(|j| j.id).collect::<Vec<_>>(), [many]);
    // an AI-generated bank: the audience is its holders, not the administrators
    let ai = e.ai_bank("Kitchen basics", lee, &[lee]);
    let admin = e.user("admin");
    let (_, s_admin, _) = with_sub(&e, admin);
    let to_admin = e.raw_out(
        s_admin,
        "bank",
        Some(&format!("bank-{ai}")),
        &payload(ai),
        e.now(),
        3600,
    );
    let to_lee = e.raw_out(
        s_lee,
        "bank",
        Some(&format!("bank-{ai}")),
        &payload(ai),
        e.now(),
        3600,
    );
    assert_eq!(e.lease().iter().map(|j| j.id).collect::<Vec<_>>(), [to_lee]);
    assert_eq!(e.out(to_admin).state, "expired");

    // (c) a reminder: the marker, its device, the record of the session, the time left
    let e = Env::at(noon());
    let kim = e.user("kim");
    let (dk, sk, _) = with_sub(&e, kim);
    let key = "0123456789abcdef0123456789abcdef";
    let payload =
        json!({"k": "remind", "kind": "exam", "answered": 1, "total": 5, "n": 1, "key": key});
    let marker = |e: &Env, device: Option<i64>, time_left: Option<i64>| {
        e.exec("DELETE FROM saved_sessions", []);
        e.exec(
            "INSERT INTO saved_sessions(user_id, session_key, device_id, kind, answered, total, started_at, time_left, saved_at, remind_at, reminders) \
             VALUES(?1, ?2, ?3, 'exam', 1, 5, 1780000000, ?4, ?5, NULL, 1)",
            params![kim, key, device, time_left, e.now()],
        );
    };
    let other_dev = e.device(kim, Some("UTC"));
    let cases: Vec<(&str, Setup<'_>, bool)> = vec![
        (
            "marker present",
            Box::new(|e| marker(e, Some(dk), Some(900))),
            true,
        ),
        (
            "no time limit",
            Box::new(|e| marker(e, Some(dk), None)),
            true,
        ),
        (
            "marker gone",
            Box::new(|e| e.exec("DELETE FROM saved_sessions", [])),
            false,
        ),
        (
            "another device holds it",
            Box::new(move |e| marker(e, Some(other_dev), Some(900))),
            false,
        ),
        ("no device", Box::new(|e| marker(e, None, Some(900))), false),
        (
            "time is up",
            Box::new(|e| marker(e, Some(dk), Some(0))),
            false,
        ),
        (
            "a record exists",
            Box::new(|e| {
                marker(e, Some(dk), Some(900));
                e.exec(
                    "INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, user_id, kind, session_key) \
                     VALUES('2026-06-10 12:30', '[]', 'all', 5, 5, 100, 60, ?1, 'practice', ?2)",
                    params![kim, key],
                );
            }),
            false,
        ),
    ];
    for (label, setup, delivered) in cases {
        e.exec("DELETE FROM attempts", []);
        e.exec("DELETE FROM push_outbox", []);
        setup(&e);
        let id = e.raw_out(sk, "remind", Some("remind"), &payload, e.now(), 3600);
        let jobs = e.lease();
        assert_eq!(jobs.len(), usize::from(delivered), "{label}");
        assert_eq!(
            e.out(id).state,
            if delivered { "leased" } else { "expired" },
            "{label}"
        );
    }

    // (d) a payload that does not render
    let e = Env::at(noon());
    let kim = e.user("kim");
    let (_, sub, _) = with_sub(&e, kim);
    let id = e.raw_out(
        sub,
        "bank",
        Some("bank-1"),
        &json!({"k": "nope"}),
        e.now(),
        600,
    );
    assert!(e.lease().is_empty());
    assert_eq!(e.out(id).state, "expired");
}

#[test]
fn a_row_past_its_ttl_is_never_handed_out_and_the_batch_is_twenty() {
    if !crypto_ready() {
        return skipped("the push crypto and texts");
    }
    let e = Env::at(noon());
    let kim = e.user("kim");
    let (_, sub, _) = with_sub(&e, kim);
    // created 100 s ago, expired exactly now: not handed out, not leased, not expired yet (the hourly pass does that)
    e.exec(
        "INSERT INTO push_outbox(subscription_id, kind, topic, payload, created_at, expires_at, next_at) VALUES(?1, 'test', 'test', '{\"k\":\"test\"}', ?2, ?3, ?4)",
        params![sub, e.now() - 100, e.now(), e.now() - 50],
    );
    assert!(e.lease().is_empty());
    let row = e.outbox().pop().expect("row");
    assert_eq!((row.state.as_str(), row.tries), ("queued", 0));
    assert_eq!(
        e.store
            .push_housekeeping(e.now())
            .expect("housekeeping")
            .expired,
        1
    );
    // 25 due rows: 20, then 5
    e.exec("DELETE FROM push_outbox", []);
    for i in 0..25 {
        e.raw_out(
            sub,
            "test",
            Some(&format!("t{i}")),
            &json!({"k": "test"}),
            e.now() - 10,
            600,
        );
    }
    assert_eq!(e.lease().len(), 20);
    assert_eq!(e.lease().len(), 5);
    assert!(e.lease().is_empty());
}

#[test]
fn quiet_hours_are_applied_again_at_lease_time_except_for_the_test() {
    if !crypto_ready() {
        return skipped("the push crypto and texts");
    }
    let night = ts(2026, 6, 11, 3, 0);
    let e = Env::at(night);
    let kim = e.user("kim");
    let (_, sub, _) = with_sub(&e, kim);
    // queued at noon the day before, sent in the night: held to 08:00, tries unchanged, expiry kept
    let bank = e.import_new(None, "Herbs", 2);
    e.grant(kim, bank);
    let id = e.raw_out(
        sub,
        "bank",
        Some(&format!("bank-{bank}")),
        &json!({"k": "bank.new", "bank": bank, "name": "Herbs", "total": 2}),
        night - 1000,
        3 * DAY,
    );
    let test = e.raw_out(
        sub,
        "test",
        Some("test"),
        &json!({"k": "test"}),
        night - 1000,
        600,
    );
    let jobs = e.lease();
    assert_eq!(
        jobs.iter().map(|j| j.id).collect::<Vec<_>>(),
        [test],
        "the test ignores quiet hours"
    );
    let row = e.out(id);
    assert_eq!(
        (row.state.as_str(), row.tries, row.next_at),
        ("queued", 0, ts(2026, 6, 11, 8, 0))
    );
    e.set_now(ts(2026, 6, 11, 8, 0));
    assert_eq!(e.lease().iter().map(|j| j.id).collect::<Vec<_>>(), [id]);
    // the zone may have changed since the enqueue: at UTC+9 03:00 UTC is 12:00
    let e = Env::at(night);
    let kim = e.user("kim");
    let (_, sub, _) = with_sub(&e, kim);
    let bank = e.import_new(None, "Herbs", 2);
    e.grant(kim, bank);
    e.set_setting(kim, "tz", "\"Etc/GMT-9\"");
    let id = e.raw_out(
        sub,
        "bank",
        Some(&format!("bank-{bank}")),
        &json!({"k": "bank.new", "bank": bank, "name": "Herbs", "total": 2}),
        night - 10,
        3600,
    );
    assert_eq!(e.lease().iter().map(|j| j.id).collect::<Vec<_>>(), [id]);
}

#[test]
fn the_text_follows_the_users_language() {
    if !crypto_ready() {
        return skipped("the push crypto and texts");
    }
    for (lang, tag, needle) in [
        ("zh", "zh-CN", "测试"),
        ("ja", "ja", "テスト"),
        ("en", "en", "test notification"),
    ] {
        let e = Env::at(noon());
        let kim = e.user("kim");
        e.set_setting(kim, "uiLang", &format!("\"{lang}\""));
        let (_, sub, ua) = with_sub(&e, kim);
        e.raw_out(
            sub,
            "test",
            Some("test"),
            &json!({"k": "test"}),
            e.now(),
            600,
        );
        let jobs = e.lease();
        let v: Value =
            serde_json::from_slice(&decrypt(ua, &b64_lenient(&jobs[0].body).expect("b64")))
                .expect("json");
        assert_eq!(v["lang"], tag);
        assert!(
            v["body"].as_str().expect("body").contains(needle),
            "{lang}: {v}"
        );
    }
    // a missing or broken language setting means English
    let e = Env::at(noon());
    let kim = e.user("kim");
    e.set_setting(kim, "uiLang", "\"fr\"");
    let (_, sub, ua) = with_sub(&e, kim);
    e.raw_out(
        sub,
        "test",
        Some("test"),
        &json!({"k": "test"}),
        e.now(),
        600,
    );
    let jobs = e.lease();
    let v: Value = serde_json::from_slice(&decrypt(ua, &b64_lenient(&jobs[0].body).expect("b64")))
        .expect("json");
    assert_eq!(v["lang"], "en");
}

// ---------------------------------------------------------------- results

fn sub_facts(e: &Env, sub: i64) -> (i64, Option<i64>, Option<i64>, Option<i64>) {
    e.raw()
        .query_row(
            "SELECT failures, last_ok_at, last_fail_at, last_status FROM push_subscriptions WHERE id = ?1",
            params![sub],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("subscription")
}

#[test]
fn a_sent_result_finishes_the_row_and_resets_the_failures() {
    let e = Env::at(noon());
    let kim = e.user("kim");
    let (_, sub, _) = with_sub(&e, kim);
    e.exec("UPDATE push_subscriptions SET failures = 4", []);
    let id = leased_row(&e, sub, 1);
    let eff = e
        .store
        .push_result(
            e.now(),
            &result(id, 1, PushOutcome::Sent, Some(201), None),
            Some("x"),
        )
        .expect("result");
    assert!(eff.applied);
    assert_eq!(eff.host.as_deref(), Some("fcm.googleapis.com"));
    let row = e.out(id);
    assert_eq!(
        (row.state.as_str(), row.status, row.done_at),
        ("sent", Some(201), Some(e.now()))
    );
    assert_eq!(sub_facts(&e, sub), (0, Some(e.now()), None, Some(201)));
}

#[test]
fn gone_deletes_the_subscription_and_its_queue() {
    let e = Env::at(noon());
    let kim = e.user("kim");
    let (_, sub, _) = with_sub(&e, kim);
    let id = leased_row(&e, sub, 1);
    e.raw_out(
        sub,
        "test",
        Some("other"),
        &json!({"k": "test"}),
        e.now(),
        600,
    );
    let eff = e
        .store
        .push_result(
            e.now(),
            &result(id, 1, PushOutcome::Gone, Some(410), None),
            None,
        )
        .expect("result");
    assert!(eff.applied && eff.subscription_deleted);
    assert_eq!(sub_count(&e), 0);
    assert!(e.outbox().is_empty());
}

#[test]
fn auth_deletes_a_stale_subscription_and_counts_a_current_one() {
    let e = Env::at(noon());
    let (kim, lee) = (e.user("kim"), e.user("lee"));
    let dk = e.device(kim, Some("UTC"));
    let dl = e.device(lee, Some("UTC"));
    let (stale, _) = e.sub_with(dk, "ffffffffffffffff");
    let (current, _) = e.sub_with(dl, "0123456789abcdef");
    let a = leased_row(&e, stale, 1);
    let b = leased_row(&e, current, 1);
    let eff = e
        .store
        .push_result(
            e.now(),
            &result(a, 1, PushOutcome::Auth, Some(403), None),
            Some("0123456789abcdef"),
        )
        .expect("stale");
    assert!(eff.subscription_deleted);
    assert_eq!(
        e.int(&format!(
            "SELECT COUNT(*) FROM push_subscriptions WHERE id = {stale}"
        )),
        0
    );
    let eff = e
        .store
        .push_result(
            e.now(),
            &result(b, 1, PushOutcome::Auth, Some(401), None),
            Some("0123456789abcdef"),
        )
        .expect("current");
    assert!(eff.applied && !eff.subscription_deleted);
    assert_eq!(e.out(b).state, "failed");
    assert_eq!(sub_facts(&e, current), (1, None, Some(e.now()), Some(401)));
}

#[test]
fn retry_backs_off_by_try_number_and_gives_up_after_five() {
    let e = Env::at(noon());
    let kim = e.user("kim");
    let (_, sub, _) = with_sub(&e, kim);
    for (tries, base) in [(1, 30), (2, 120), (3, 600), (4, 3600)] {
        let id = leased_row(&e, sub, tries);
        e.store
            .push_result(
                e.now(),
                &result(id, tries, PushOutcome::Retry, Some(503), None),
                None,
            )
            .expect("retry");
        let row = e.out(id);
        assert_eq!(
            (row.state.as_str(), row.tries, row.done_at),
            ("queued", tries, None),
            "try {tries}"
        );
        let wait = row.next_at - e.now();
        assert!(
            wait >= base && wait <= base + base / 5,
            "try {tries}: waits {wait} s, want {base} + 0..20 %"
        );
        e.exec("DELETE FROM push_outbox", []);
    }
    assert_eq!(sub_facts(&e, sub).0, 4);
    let id = leased_row(&e, sub, 5);
    e.store
        .push_result(
            e.now(),
            &result(id, 5, PushOutcome::Retry, Some(500), None),
            None,
        )
        .expect("fifth");
    assert_eq!(e.out(id).state, "failed");
    assert_eq!(sub_facts(&e, sub).0, 5);
    // The table of the contract
    assert_eq!(
        [1, 2, 3, 4, 7].map(|t| push_backoff_secs(t, 0)),
        [30, 120, 600, 3600, 3600]
    );
    // A row past its expiry is not retried
    e.exec("DELETE FROM push_outbox", []);
    let id = leased_row(&e, sub, 1);
    e.exec(
        "UPDATE push_outbox SET created_at = ?1, expires_at = ?2 WHERE id = ?3",
        params![e.now() - 100, e.now(), id],
    );
    e.store
        .push_result(
            e.now(),
            &result(id, 1, PushOutcome::Retry, Some(503), None),
            None,
        )
        .expect("expired");
    assert_eq!(e.out(id).state, "failed");
}

#[test]
fn a_retry_with_retry_after_waits_exactly_that_and_is_not_a_failure() {
    let e = Env::at(noon());
    let kim = e.user("kim");
    let (_, sub, _) = with_sub(&e, kim);
    let id = leased_row(&e, sub, 2);
    e.store
        .push_result(
            e.now(),
            &result(id, 2, PushOutcome::Retry, Some(429), Some(90)),
            None,
        )
        .expect("retry");
    let row = e.out(id);
    assert_eq!(
        (row.state.as_str(), row.next_at, row.tries),
        ("queued", e.now() + 90, 2)
    );
    assert_eq!(sub_facts(&e, sub), (0, None, None, Some(429)));
    // deferred: the try does not count, 60 s when nothing is said
    let id = leased_row(&e, sub, 3);
    e.store
        .push_result(
            e.now(),
            &result(id, 3, PushOutcome::Deferred, None, None),
            None,
        )
        .expect("deferred");
    let row = e.out(id);
    assert_eq!(
        (row.state.as_str(), row.tries, row.next_at),
        ("queued", 2, e.now() + 60)
    );
    let id = leased_row(&e, sub, 1);
    e.store
        .push_result(
            e.now(),
            &result(id, 1, PushOutcome::Deferred, Some(429), Some(300)),
            None,
        )
        .expect("deferred");
    let row = e.out(id);
    assert_eq!((row.tries, row.next_at), (0, e.now() + 300));
    assert_eq!(sub_facts(&e, sub).0, 0);
}

#[test]
fn rejected_fails_only_that_message() {
    let e = Env::at(noon());
    let kim = e.user("kim");
    let (_, sub, _) = with_sub(&e, kim);
    let id = leased_row(&e, sub, 1);
    let other = e.raw_out(
        sub,
        "test",
        Some("other"),
        &json!({"k": "test"}),
        e.now(),
        600,
    );
    e.store
        .push_result(
            e.now(),
            &result(id, 1, PushOutcome::Rejected, Some(400), None),
            None,
        )
        .expect("rejected");
    assert_eq!(e.out(id).state, "failed");
    assert_eq!(e.out(other).state, "queued");
    assert_eq!(sub_count(&e), 1);
}

#[test]
fn late_and_duplicate_results_change_nothing() {
    let e = Env::at(noon());
    let kim = e.user("kim");
    let (_, sub, _) = with_sub(&e, kim);
    let id = leased_row(&e, sub, 2);
    let none = |r: PushJobResult| {
        let eff = e.store.push_result(e.now(), &r, None).expect("result");
        assert!(!eff.applied && eff.host.is_none() && !eff.subscription_deleted);
    };
    // another try number (the row was leased again since)
    none(result(id, 1, PushOutcome::Sent, Some(201), None));
    none(result(id, 3, PushOutcome::Gone, Some(410), None));
    assert_eq!(e.out(id).state, "leased");
    assert_eq!(sub_count(&e), 1);
    // an id that does not exist
    none(result(99_999, 1, PushOutcome::Sent, Some(201), None));
    // the right one works once; the duplicate is ignored
    assert!(
        e.store
            .push_result(
                e.now(),
                &result(id, 2, PushOutcome::Sent, Some(201), None),
                None
            )
            .expect("sent")
            .applied
    );
    none(result(id, 2, PushOutcome::Gone, Some(410), None));
    assert_eq!((e.out(id).state.as_str(), sub_count(&e)), ("sent", 1));
    // a queued row (not leased) takes no result
    let q = e.raw_out(sub, "test", Some("q"), &json!({"k": "test"}), e.now(), 600);
    none(result(q, 0, PushOutcome::Sent, Some(201), None));
    none(result(q, 1, PushOutcome::Sent, Some(201), None));
    assert_eq!(e.out(q).state, "queued");
    // a row whose lease ran out and was handed out again: the first courier's late answer is refused
    let id = leased_row(&e, sub, 1);
    e.exec(
        "UPDATE push_outbox SET tries = 2 WHERE id = ?1",
        params![id],
    );
    none(result(id, 1, PushOutcome::Sent, Some(201), None));
    assert_eq!(e.out(id).state, "leased");
}

// ---------------------------------------------------------------- housekeeping and key clean-up

#[test]
fn housekeeping_expires_deletes_and_drops_dead_subscriptions() {
    let now = noon();
    let e = Env::at(now);
    let (kim, lee) = (e.user("kim"), e.user("lee"));
    let (d_kim, s_kim, _) = with_sub(&e, kim);
    let (d_lee, s_lee, _) = with_sub(&e, lee);
    let out = |state: &str, created: i64, expires: i64, done: Option<i64>| {
        e.exec(
            "INSERT INTO push_outbox(subscription_id, kind, topic, payload, created_at, expires_at, state, next_at, done_at) VALUES(?1, 'test', 't', '{\"k\":\"test\"}', ?2, ?3, ?4, ?2, ?5)",
            params![s_kim, created, expires, state, done],
        );
        e.int("SELECT MAX(id) FROM push_outbox")
    };
    let overdue = out("queued", now - 7200, now - 3600, None);
    let leased_overdue = out("leased", now - 7200, now - 1, None);
    let waiting = out("queued", now - 7200, now + 3600, None);
    let old_sent = out("sent", now - 9 * DAY, now - 8 * DAY, Some(now - 8 * DAY));
    let recent_failed = out("failed", now - 7 * DAY, now - 6 * DAY, Some(now - 6 * DAY));
    let old_expired = out("expired", now - 9 * DAY, now - 8 * DAY, Some(now - 8 * DAY));
    let h = e.store.push_housekeeping(now).expect("housekeeping");
    assert_eq!(
        (h.expired, h.deleted, h.subscriptions, h.saved),
        (2, 2, 0, 0)
    );
    assert_eq!(e.out(overdue).state, "expired");
    assert_eq!(e.out(leased_overdue).state, "expired");
    assert_eq!(e.out(waiting).state, "queued");
    assert_eq!(e.out(recent_failed).state, "failed");
    let ids: Vec<i64> = e.outbox().iter().map(|o| o.id).collect();
    assert!(!ids.contains(&old_sent) && !ids.contains(&old_expired));
    // Subscriptions: past their expiry, ten failures and silent for a week, device unused for 30 days
    e.exec(
        "UPDATE push_subscriptions SET expires_at = ?1 WHERE id = ?2",
        params![now - 1, s_kim],
    );
    let h = e.store.push_housekeeping(now).expect("housekeeping");
    assert_eq!(h.subscriptions, 1);
    assert_eq!(sub_count(&e), 1);
    assert!(
        e.outbox().is_empty(),
        "the outbox of a deleted subscription goes first"
    );
    e.exec(
        "UPDATE push_subscriptions SET failures = 10, last_ok_at = ?1",
        params![now - 8 * DAY],
    );
    assert_eq!(
        e.store
            .push_housekeeping(now)
            .expect("housekeeping")
            .subscriptions,
        1
    );
    let (_, s2, _) = with_sub(&e, kim);
    e.exec(
        "UPDATE push_subscriptions SET failures = 10, last_ok_at = ?1 WHERE id = ?2",
        params![now - 6 * DAY, s2],
    );
    assert_eq!(
        e.store
            .push_housekeeping(now)
            .expect("housekeeping")
            .subscriptions,
        0,
        "recent success keeps it"
    );
    e.exec(
        "UPDATE devices SET active_at = ?1 WHERE id = ?2",
        params![now - 31 * DAY, d_kim],
    );
    let (_, s3, _) = {
        let dev = d_kim;
        // d_kim has no subscription row left (it was deleted above); give it one again
        let (s, u) = e.sub(dev);
        (dev, s, u)
    };
    assert_eq!(
        e.store
            .push_housekeeping(now)
            .expect("housekeeping")
            .subscriptions,
        1
    );
    assert_eq!(
        e.int(&format!(
            "SELECT COUNT(*) FROM push_subscriptions WHERE id = {s3}"
        )),
        0
    );
    let _ = (d_lee, s_lee);
    // Saved sessions: older than 14 days, or with a record
    let key = |i: u8| format!("{i:032x}");
    let user = kim;
    for (i, saved) in [(1u8, now - 15 * DAY), (2, now - 13 * DAY), (3, now - DAY)] {
        e.exec(
            "INSERT INTO saved_sessions(user_id, session_key, kind, answered, total, started_at, saved_at) VALUES(?1, ?2, 'practice', 1, 5, 1780000000, ?3)",
            params![user, key(i), saved],
        );
    }
    e.exec(
        "INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, user_id, kind, session_key) VALUES('2026-06-10 12:30', '[]', 'all', 5, 5, 100, 60, ?1, 'practice', ?2)",
        params![user, key(3)],
    );
    let h = e.store.push_housekeeping(now).expect("housekeeping");
    assert_eq!(h.saved, 2);
    assert_eq!(e.text("SELECT session_key FROM saved_sessions"), key(2));
}

#[test]
fn housekeeping_works_through_batches_of_five_hundred() {
    let now = noon();
    let e = Env::at(now);
    let kim = e.user("kim");
    let (_, sub, _) = with_sub(&e, kim);
    {
        let mut c = e.raw();
        let tx = c.transaction().expect("tx");
        for i in 0..1_250 {
            tx.execute(
                "INSERT INTO push_outbox(subscription_id, kind, topic, payload, created_at, expires_at, state, next_at, done_at) VALUES(?1, 'test', 't', '{\"k\":\"test\"}', ?2, ?3, 'sent', ?2, ?4)",
                params![sub, now - 20 * DAY, now - 19 * DAY, now - 10 * DAY - i],
            )
            .expect("insert");
        }
        for i in 0..620 {
            tx.execute(
                "INSERT INTO push_outbox(subscription_id, kind, topic, payload, created_at, expires_at, state, next_at) VALUES(?1, 'test', 't', '{\"k\":\"test\"}', ?2, ?3, 'queued', ?2)",
                params![sub, now - 3 * DAY - i, now - 2 * DAY],
            )
            .expect("insert");
        }
        tx.commit().expect("commit");
    }
    let h = e.store.push_housekeeping(now).expect("housekeeping");
    assert_eq!((h.expired, h.deleted), (620, 1_250));
    assert_eq!(
        e.int("SELECT COUNT(*) FROM push_outbox WHERE state = 'expired'"),
        620
    );
    assert_eq!(
        e.int("SELECT COUNT(*) FROM push_outbox WHERE state = 'sent'"),
        0
    );
}

#[test]
fn a_changed_key_removes_the_subscriptions_of_the_old_one() {
    let e = Env::at(noon());
    let (kim, lee) = (e.user("kim"), e.user("lee"));
    let dk = e.device(kim, Some("UTC"));
    let dl = e.device(lee, Some("UTC"));
    let (old, _) = e.sub_with(dk, "ffffffffffffffff");
    let (cur, _) = e.sub_with(dl, "0123456789abcdef");
    e.raw_out(
        old,
        "test",
        Some("test"),
        &json!({"k": "test"}),
        e.now(),
        600,
    );
    e.raw_out(
        cur,
        "test",
        Some("test"),
        &json!({"k": "test"}),
        e.now(),
        600,
    );
    assert_eq!(
        e.store
            .push_key_cleanup("0123456789abcdef")
            .expect("cleanup"),
        1
    );
    assert_eq!(sub_count(&e), 1);
    assert_eq!(e.outbox().len(), 1);
    assert_eq!(e.outbox()[0].sub, cur);
    assert_eq!(
        e.store.push_key_cleanup("0123456789abcdef").expect("again"),
        0
    );
}

// ---------------------------------------------------------------- start-up

#[tokio::test]
async fn start_loads_or_creates_the_key_and_runs_the_version_check() {
    let e = Env::at(noon());
    // the test key installed by the helper is replaced by what `start` loads from the data directory
    e.state.0.push.set_key(None, String::new());
    let report = xexam_backend::push::scheduler::start(&e.state).expect("start");
    assert_eq!(
        report.version_push, None,
        "a fresh database announces nothing"
    );
    if !crypto_ready() {
        assert!(
            e.state.0.push.key().is_none(),
            "no usable key: push stays off"
        );
        return;
    }
    let key = e.state.0.push.key().expect("key loaded");
    let file = e.dir.path().join("push-vapid.key");
    assert!(
        file.exists(),
        "the key file was created in the data directory"
    );
    assert_eq!(
        e.state.0.push.subject(),
        "mailto:admin@localhost",
        "nothing better is configured"
    );
    // a restart loads the same key
    let id = key.key_id();
    e.state.0.push.set_key(None, String::new());
    xexam_backend::push::scheduler::start(&e.state).expect("restart");
    assert_eq!(e.state.0.push.key().expect("key").key_id(), id);
    // subscriptions of another key are removed at start, those of this key stay
    let kim = e.user("kim");
    let dk = e.device(kim, Some("UTC"));
    let lee = e.user("lee");
    let dl = e.device(lee, Some("UTC"));
    e.sub_with(dk, "ffffffffffffffff");
    e.sub_with(dl, &id);
    xexam_backend::push::scheduler::start(&e.state).expect("third start");
    assert_eq!(sub_count(&e), 1);
    assert_eq!(e.text("SELECT key_id FROM push_subscriptions"), id);
    // a configured subject is used
    let e = Env::at_with(
        noon(),
        &[("XEXAM_PUSH_SUBJECT", "https://xexam.example.com")],
    );
    xexam_backend::push::scheduler::start(&e.state).expect("start");
    assert_eq!(e.state.0.push.subject(), "https://xexam.example.com");
    let e = Env::at_with(noon(), &[("XEXAM_PUSH_SUBJECT", "not a subject")]);
    xexam_backend::push::scheduler::start(&e.state).expect("start");
    assert_eq!(
        e.state.0.push.subject(),
        "mailto:admin@localhost",
        "an invalid subject falls back to the default"
    );
    // derived from the configured origins when there is a real host name
    let e = Env::at_with(noon(), &[("XEXAM_ALLOWED_HOSTS", "xexam.example.com")]);
    xexam_backend::push::scheduler::start(&e.state).expect("start");
    assert_eq!(e.state.0.push.subject(), "https://xexam.example.com");
}

#[test]
fn tick_once_runs_the_triggers_and_the_hourly_housekeeping() {
    let e = Env::at(noon());
    let fx = fixture(&e);
    e.mark_banks_seen();
    e.import_new(None, "Herbs", 2);
    // an old finished row to be removed by the housekeeping pass
    e.exec(
        "INSERT INTO push_outbox(subscription_id, kind, topic, payload, created_at, expires_at, state, next_at, done_at) VALUES(?1, 'test', 't', '{\"k\":\"test\"}', ?2, ?3, 'sent', ?2, ?4)",
        params![fx.sub_of("kim"), e.now() - 30 * DAY, e.now() - 29 * DAY, e.now() - 20 * DAY],
    );
    xexam_backend::push::scheduler::tick_once(&e.state, e.now(), false);
    assert_eq!(fx.who_got(&e, "bank"), ["admin", "ann"]);
    assert_eq!(
        e.int("SELECT COUNT(*) FROM push_outbox WHERE kind = 'test'"),
        1,
        "no housekeeping on a normal tick"
    );
    xexam_backend::push::scheduler::tick_once(&e.state, e.now(), true);
    assert_eq!(
        e.int("SELECT COUNT(*) FROM push_outbox WHERE kind = 'test'"),
        0
    );
}

// ---------------------------------------------------------------- the courier routes

async fn next(e: &Env, wait_ms: u64) -> apic::Resp {
    e.internal(
        "/internal/ai/push/next",
        Some(TOKEN),
        &json!({"waitMs": wait_ms}).to_string(),
    )
    .await
}

#[tokio::test]
async fn the_courier_routes_are_hidden_without_the_token_and_check_their_bodies() {
    let e = Env::at(noon());
    for path in ["/internal/ai/push/next", "/internal/ai/push/result"] {
        for token in [None, Some("wrong-token-wrong-token-wrong-token-0000")] {
            let r = e.internal(path, token, "{}").await;
            assert_eq!(r.status, 404, "{path}");
            assert_eq!(r.text(), "not found");
        }
    }
    let post = |path: &'static str, body: Value| {
        let e = &e;
        async move { e.internal(path, Some(TOKEN), &body.to_string()).await }
    };
    for body in [
        json!({}),
        json!({"waitMs": -1}),
        json!({"waitMs": 20_001}),
        json!({"waitMs": "5"}),
        json!({"waitMs": 5, "x": 1}),
        json!({"waitMs": 1.5}),
    ] {
        let r = post("/internal/ai/push/next", body.clone()).await;
        assert_eq!(r.status, 400, "{body}: {}", r.text());
    }
    let ok = json!({"id": 1, "tries": 1, "outcome": "sent", "status": 201, "retryAfter": null});
    let with = |k: &str, v: Value| {
        let mut o = ok.clone();
        o[k] = v;
        o
    };
    let bad: Vec<(&str, Value)> = vec![
        ("no results", json!({})),
        ("empty list", json!({"results": []})),
        ("21 results", json!({"results": vec![ok.clone(); 21]})),
        ("not an object", json!({"results": [5]})),
        ("extra key", json!({"results": [with("extra", json!(1))]})),
        (
            "missing key",
            json!({"results": [{"id": 1, "tries": 1, "outcome": "sent", "status": 201}]}),
        ),
        ("id zero", json!({"results": [with("id", json!(0))]})),
        ("tries zero", json!({"results": [with("tries", json!(0))]})),
        (
            "unknown outcome",
            json!({"results": [with("outcome", json!("done"))]}),
        ),
        (
            "status 1000",
            json!({"results": [with("status", json!(1000))]}),
        ),
        (
            "status negative",
            json!({"results": [with("status", json!(-1))]}),
        ),
        (
            "status string",
            json!({"results": [with("status", json!("201"))]}),
        ),
        (
            "retryAfter 0",
            json!({"results": [with("retryAfter", json!(0))]}),
        ),
        (
            "retryAfter 3601",
            json!({"results": [with("retryAfter", json!(3601))]}),
        ),
        ("extra top key", json!({"results": [ok.clone()], "more": 1})),
    ];
    for (label, body) in bad {
        let r = post("/internal/ai/push/result", body).await;
        assert_eq!(r.status, 400, "{label}: {}", r.text());
    }
    // an unknown or late result is accepted and ignored
    let r = post("/internal/ai/push/result", json!({"results": [ok.clone()]})).await;
    assert_eq!((r.status.as_u16(), r.json()), (200, json!({"ok": true})));
    for outcome in ["sent", "gone", "auth", "retry", "deferred", "rejected"] {
        let r = post(
            "/internal/ai/push/result",
            json!({"results": [with("outcome", json!(outcome))]}),
        )
        .await;
        assert_eq!(r.status, 200, "{outcome}: {}", r.text());
    }
}

#[tokio::test]
async fn next_answers_empty_after_the_wait_and_records_the_poll() {
    let e = Env::at(noon());
    assert!(!e.state.0.push.courier_connected(e.now()));
    let started = std::time::Instant::now();
    let r = next(&e, 300).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json(), json!({"jobs": []}));
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(250),
        "waited {:?}",
        started.elapsed()
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    assert!(
        e.state.0.push.courier_connected(e.now()),
        "the poll is the courier's presence"
    );
    // waitMs 0 answers at once
    let started = std::time::Instant::now();
    assert_eq!(next(&e, 0).await.json(), json!({"jobs": []}));
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

#[tokio::test]
async fn closing_the_handle_ends_a_long_poll() {
    let e = Env::at(noon());
    let push = e.state.0.push.clone();
    let router = e.router.clone();
    let poll = tokio::spawn(async move {
        let len = json!({"waitMs": 20_000}).to_string();
        let lenh = len.len().to_string();
        apic::send(
            &router,
            apic::build(
                "POST",
                "/internal/ai/push/next",
                Some(pc::NGINX_HOST),
                &[
                    ("content-type", b"application/json"),
                    ("content-length", lenh.as_bytes()),
                    ("x-xexam-internal-token", TOKEN.as_bytes()),
                ],
                axum::body::Body::from(len),
            ),
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let started = std::time::Instant::now();
    push.close();
    let r = tokio::time::timeout(std::time::Duration::from_secs(10), poll)
        .await
        .expect("the poll ended")
        .expect("task");
    assert_eq!(r.json(), json!({"jobs": []}));
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[tokio::test]
async fn a_fake_courier_leases_a_job_and_reports_it() {
    if !crypto_ready() {
        return skipped("the push crypto and texts");
    }
    let e = Env::at(noon());
    let kim = e.user("kim");
    let (_, sub, ua) = with_sub(&e, kim);
    let id = e.raw_out(
        sub,
        "test",
        Some("test"),
        &json!({"k": "test"}),
        e.now(),
        600,
    );
    // a long poll that is woken when something becomes due
    let r = next(&e, 0).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let jobs = r.json()["jobs"].as_array().expect("jobs").clone();
    assert_eq!(jobs.len(), 1);
    let j = jobs[0].as_object().expect("job");
    assert_eq!(
        j.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "id",
            "tries",
            "endpoint",
            "ttl",
            "urgency",
            "topic",
            "authorization",
            "body"
        ]
    );
    assert_eq!(
        (j["id"].as_i64(), j["tries"].as_i64(), j["ttl"].as_i64()),
        (Some(id), Some(1), Some(600))
    );
    assert_eq!(
        (j["urgency"].as_str(), j["topic"].as_str()),
        (Some("normal"), Some("test"))
    );
    assert!(j["authorization"]
        .as_str()
        .expect("auth")
        .starts_with("vapid t="));
    let plain: Value = serde_json::from_slice(&decrypt(
        ua,
        &b64_lenient(j["body"].as_str().expect("body")).expect("b64"),
    ))
    .expect("json");
    assert_eq!(plain["tag"], "test");
    // nothing else is due
    assert_eq!(next(&e, 0).await.json(), json!({"jobs": []}));
    // report it
    let res = json!({"results": [{"id": id, "tries": 1, "outcome": "sent", "status": 201, "retryAfter": null}]});
    let r = e
        .internal("/internal/ai/push/result", Some(TOKEN), &res.to_string())
        .await;
    assert_eq!((r.status.as_u16(), r.json()), (200, json!({"ok": true})));
    assert_eq!(e.out(id).state, "sent");
    // the same report again is ignored
    let r = e
        .internal("/internal/ai/push/result", Some(TOKEN), &res.to_string())
        .await;
    assert_eq!(r.status, 200);
    assert_eq!(e.out(id).status, Some(201));
}

#[tokio::test]
async fn a_poll_without_a_key_hands_out_nothing() {
    let e = Env::at(noon());
    let kim = e.user("kim");
    let (_, sub, _) = with_sub(&e, kim);
    e.raw_out(
        sub,
        "test",
        Some("test"),
        &json!({"k": "test"}),
        e.now(),
        600,
    );
    e.state.0.push.set_key(None, String::new());
    assert_eq!(next(&e, 0).await.json(), json!({"jobs": []}));
    assert_eq!(e.outbox()[0].state, "queued");
    assert!(e.state.0.push.courier_connected(e.now()));
}

#[tokio::test]
async fn auth_for_every_result_of_a_call_is_recorded_once_an_hour_per_host() {
    let e = Env::at(noon());
    let (kim, lee) = (e.user("kim"), e.user("lee"));
    let (_, s1, _) = with_sub(&e, kim);
    let (_, s2, _) = with_sub(&e, lee);
    let key_id = e.key_id();
    e.exec("UPDATE push_subscriptions SET key_id = ?1", params![key_id]);
    let auth = |a: i64, b: i64| {
        json!({"results": [
            {"id": a, "tries": 1, "outcome": "auth", "status": 403, "retryAfter": null},
            {"id": b, "tries": 1, "outcome": "auth", "status": 401, "retryAfter": null}
        ]})
    };
    let (a, b) = (leased_row(&e, s1, 1), leased_row(&e, s2, 1));
    let r = e
        .internal(
            "/internal/ai/push/result",
            Some(TOKEN),
            &auth(a, b).to_string(),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let rows = e.entries("system.pushAuth").await;
    assert_eq!(rows.len(), 1, "both subscriptions are on one host");
    assert_eq!(
        serde_json::from_str::<Value>(rows[0].1.as_deref().expect("detail")).expect("json"),
        json!({"host": "fcm.googleapis.com"})
    );
    // within the hour: not again
    let (a, b) = (leased_row(&e, s1, 1), leased_row(&e, s2, 1));
    e.advance(600);
    e.internal(
        "/internal/ai/push/result",
        Some(TOKEN),
        &auth(a, b).to_string(),
    )
    .await;
    assert_eq!(e.entries("system.pushAuth").await.len(), 1);
    // after the hour: again
    let (a, b) = (leased_row(&e, s1, 1), leased_row(&e, s2, 1));
    e.advance(3600);
    e.internal(
        "/internal/ai/push/result",
        Some(TOKEN),
        &auth(a, b).to_string(),
    )
    .await;
    assert_eq!(e.entries("system.pushAuth").await.len(), 2);
    // one auth alone, or a mix, is not recorded
    e.advance(2 * HOUR);
    let (a, b) = (leased_row(&e, s1, 1), leased_row(&e, s2, 1));
    let mixed = json!({"results": [
        {"id": a, "tries": 1, "outcome": "auth", "status": 403, "retryAfter": null},
        {"id": b, "tries": 1, "outcome": "sent", "status": 201, "retryAfter": null}
    ]});
    e.internal("/internal/ai/push/result", Some(TOKEN), &mixed.to_string())
        .await;
    let single = json!({"results": [{"id": leased_row(&e, s1, 1), "tries": 1, "outcome": "auth", "status": 403, "retryAfter": null}]});
    e.internal("/internal/ai/push/result", Some(TOKEN), &single.to_string())
        .await;
    assert_eq!(e.entries("system.pushAuth").await.len(), 2);
    // ... and a refused batch applies nothing at all
    let id = leased_row(&e, s2, 1);
    let batch = json!({"results": [
        {"id": id, "tries": 1, "outcome": "sent", "status": 201, "retryAfter": null},
        {"id": 5, "tries": 1, "outcome": "bogus", "status": 201, "retryAfter": null}
    ]});
    assert_eq!(
        e.internal("/internal/ai/push/result", Some(TOKEN), &batch.to_string())
            .await
            .status,
        400
    );
    assert_eq!(e.out(id).state, "leased");
}

// ---------------------------------------------------------------- grants through the users route

#[tokio::test]
async fn granting_banks_through_the_users_route_pushes_per_the_contract() {
    let e = Env::new();
    if !e.device_linking_works().await {
        return skipped(
            "the devices package (which also wires the grant push into the users store)",
        );
    }
    let admin_cookie = e.login("admin").await;
    let lee = e.user("lee");
    let (_, sub, _) = with_sub(&e, lee);
    let ann = e.new_user("ann", Role::Admin);
    let (_, s_ann, _) = with_sub(&e, ann);
    let b1 = e.import_new(None, "Herbs", 2);
    let b2 = e.import_new(None, "Spices", 2);
    let b3 = e.import_new(None, "Teas", 2);
    e.mark_banks_seen();
    // one bank: a "new bank" push; several: one summary
    let r = e
        .post(
            "/api/users/update",
            &admin_cookie,
            json!({"id": lee, "banks": [b1]}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let rows = rows_of_kind(&e, sub, "bank");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].payload["k"], "bank.new");
    assert_eq!(rows[0].topic, Some(format!("bank-{b1}")));
    e.exec("DELETE FROM push_outbox", []);
    let r = e
        .post(
            "/api/users/update",
            &admin_cookie,
            json!({"id": lee, "banks": [b1, b2, b3]}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let rows = rows_of_kind(&e, sub, "bank");
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].payload, json!({"k": "bank.grantMany", "n": 2}));
    assert_eq!(rows[0].topic.as_deref(), Some("bank-access"));
    // taking a bank away pushes nothing; an administrator target gets nothing
    e.exec("DELETE FROM push_outbox", []);
    e.post(
        "/api/users/update",
        &admin_cookie,
        json!({"id": lee, "banks": [b1]}),
    )
    .await;
    assert!(e.outbox().is_empty());
    e.post(
        "/api/users/update",
        &admin_cookie,
        json!({"id": ann, "banks": [b1, b2]}),
    )
    .await;
    assert!(rows_of_kind(&e, s_ann, "bank").is_empty());
}

#[test]
fn a_row_held_to_the_morning_by_the_lease_gets_its_full_time_to_live_from_the_release() {
    if !crypto_ready() {
        return skipped("the push crypto and texts");
    }
    let night = ts(2026, 6, 11, 3, 0);
    let e = Env::at(night);
    let kim = e.user("kim");
    let (_, sub, _) = with_sub(&e, kim);
    let bank = e.import_new(None, "Herbs", 2);
    e.grant(kim, bank);
    // Short expiry: it would end long before 08:00 without the extension
    let id = e.raw_out(
        sub,
        "bank",
        Some(&format!("bank-{bank}")),
        &json!({"k": "bank.new", "bank": bank, "name": "Herbs", "total": 2}),
        night - 1000,
        3600,
    );
    assert!(e.lease().is_empty());
    let row = e.out(id);
    assert_eq!(row.next_at, ts(2026, 6, 11, 8, 0));
    assert_eq!(row.expires_at, ts(2026, 6, 11, 8, 0) + 3 * DAY);
}
