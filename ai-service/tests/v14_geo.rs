//! v1.4 external IP-location lookup, the ai-service side: the administrators' routes (settings, test, save, enable,
//! delete), the sealed key, the failures of a provider, the AI independence, the upgrade of schema 7, and the secrets.
//! Every provider is a local stub on 127.0.0.1; every place is made up.
mod common;

use common::geokit::*;
use common::*;
use serde_json::{json, Value};
use std::time::Duration;
use xexam_ai_service::config::{JournalMode, Timeouts};
use xexam_ai_service::store::Store;

/// The key of the tests (obviously fake, only unreserved characters).
const GEO_KEY: &str = "TESTKEY-geo-0123456789abcdef";
const MASTER_A: &str = "0101010101010101010101010101010101010101010101010101010101010101";
const MASTER_B: &str = "0202020202020202020202020202020202020202020202020202020202020202";

const TEST: &str = "/api/ai/admin/geo/test";
const SAVE: &str = "/api/ai/admin/geo/save";
const ENABLE: &str = "/api/ai/admin/geo/enable";
const DELETE: &str = "/api/ai/admin/geo/delete";
const GET: &str = "/api/ai/admin/geo";

fn mapping() -> Value {
    json!({"country": "", "countryCode": "country", "region": "region", "city": "city",
           "okPath": "", "okValue": "", "errorPath": "", "messagePath": ""})
}

fn draft(url: &str) -> Value {
    json!({"preset": "custom", "urlTemplate": url, "keyHeaderName": "", "keyHeaderValue": "",
           "mapping": mapping()})
}

fn draft_with(url: &str, f: impl FnOnce(&mut Value)) -> Value {
    let mut d = draft(url);
    f(&mut d);
    d
}

fn opts_with_master(hex: &str) -> Opts {
    Opts {
        secret_key: Some(hex.to_string()),
        ..Opts::default()
    }
}

fn geo_rows(t: &TestApp) -> i64 {
    t.db()
        .query_row("SELECT COUNT(*) FROM geo_settings", [], |r| r.get(0))
        .expect("count")
}

fn count(t: &TestApp, table: &str) -> i64 {
    t.db()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .expect("count")
}

/// The events delivered so far.
async fn events(t: &TestApp) -> Vec<Value> {
    t.app.0.deliver_outbox().await;
    t.backend.st().activity.clone()
}

fn action_events(all: &[Value], action: &str) -> Vec<Value> {
    all.iter()
        .filter(|e| e["action"] == json!(action))
        .cloned()
        .collect()
}

// —— access and shapes ————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn the_geo_routes_need_an_administrator_and_an_exact_body() {
    let t = start().await;
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    let good = draft(&stub.template("ok"));
    // a plain user: 403 on every route
    let (s, v) = t.get(GET, &sam).await;
    assert_eq!((s, v["code"].clone()), (403, json!("ai.adminOnly")));
    for path in [TEST, SAVE, ENABLE, DELETE] {
        let (s, v) = t.post(path, &sam, good.clone()).await;
        assert_eq!(
            (s, v["code"].clone()),
            (403, json!("ai.adminOnly")),
            "{path}"
        );
    }
    // no identity: refused like every other route
    let r = t.raw(axum::http::Method::GET, GET, None, None, None).await;
    assert_eq!(r.status().as_u16(), 503);
    // wrong methods
    let r = t
        .raw(
            axum::http::Method::GET,
            TEST,
            Some(t.ident(&admin)),
            None,
            None,
        )
        .await;
    assert_eq!(r.status().as_u16(), 405);
    // the draft shape is exact: nothing reaches the provider
    let bad_bodies = [
        draft_with(&stub.template("ok"), |d| d["extra"] = json!(1)),
        draft_with(&stub.template("ok"), |d| {
            d.as_object_mut().expect("o").remove("mapping");
        }),
        draft_with(&stub.template("ok"), |d| {
            d["mapping"].as_object_mut().expect("o").remove("city");
        }),
        draft_with(&stub.template("ok"), |d| d["mapping"]["bonus"] = json!("x")),
        draft_with(&stub.template("ok"), |d| d["mapping"]["city"] = json!(3)),
        draft_with(&stub.template("ok"), |d| d["mapping"] = json!("city")),
        draft_with(&stub.template("ok"), |d| d["preset"] = json!("bogus")),
        draft_with(&stub.template("ok"), |d| d["preset"] = json!(1)),
        draft_with(&stub.template("ok"), |d| d["key"] = json!(12)),
        draft_with(&stub.template("ok"), |d| d["urlTemplate"] = json!(null)),
        draft_with(&stub.template("ok"), |d| d["keyHeaderName"] = json!(null)),
        draft_with(&stub.template("ok"), |d| {
            d.as_object_mut().expect("o").remove("keyHeaderValue");
        }),
    ];
    for path in [TEST, SAVE] {
        for b in &bad_bodies {
            let (s, v) = t.post(path, &admin, b.clone()).await;
            assert_eq!(
                (s, v["code"].clone()),
                (400, json!("ai.badRequest")),
                "{path} {b}"
            );
        }
    }
    // enable and delete take an empty object only
    for path in [ENABLE, DELETE] {
        let (s, v) = t.post(path, &admin, json!({"x": 1})).await;
        assert_eq!(
            (s, v["code"].clone()),
            (400, json!("ai.badRequest")),
            "{path}"
        );
    }
    assert_eq!(stub.count(), 0);
    assert_eq!(geo_rows(&t), 0);
}

#[tokio::test]
async fn nothing_is_saved_at_first() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let (s, v) = t.get(GET, &admin).await;
    assert_eq!(s, 200);
    assert_eq!(
        v,
        json!({"settings": null, "testIp": "8.8.8.8", "perMinute": 30, "timeoutMs": 3000})
    );
}

// —— validation ———————————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn an_invalid_draft_is_refused_before_any_call() {
    let t = start().await;
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    let ok = stub.template("ok");
    let cases: Vec<(Value, &str, &str)> = vec![
        (
            draft("ftp://provider.example/{ip}"),
            "ai.geo.badUrl",
            "urlTemplate",
        ),
        (
            draft("https://provider.example/json"),
            "ai.geo.badUrl",
            "urlTemplate",
        ),
        (
            draft("https://provider.example/{ip}/{other}"),
            "ai.geo.badUrl",
            "urlTemplate",
        ),
        (
            draft("https://{ip}.provider.example/"),
            "ai.geo.badUrl",
            "urlTemplate",
        ),
        (
            draft("https://user@provider.example/{ip}"),
            "ai.geo.badUrl",
            "urlTemplate",
        ),
        (
            draft("https://provider.example/{ip}#x"),
            "ai.geo.badUrl",
            "urlTemplate",
        ),
        (
            draft_with(&ok, |d| {
                d["keyHeaderName"] = json!("Host");
                d["keyHeaderValue"] = json!("{key}");
                d["key"] = json!(GEO_KEY);
            }),
            "ai.geo.badHeader",
            "keyHeaderName",
        ),
        (
            draft_with(&ok, |d| {
                d["keyHeaderName"] = json!("Authorization");
                d["keyHeaderValue"] = json!("Bearer");
                d["key"] = json!(GEO_KEY);
            }),
            "ai.geo.badHeader",
            "keyHeaderValue",
        ),
        (
            draft_with(&ok, |d| d["keyHeaderValue"] = json!("{key}")),
            "ai.geo.badHeader",
            "keyHeaderValue",
        ),
        (
            draft_with(&ok, |d| d["key"] = json!("has a space")),
            "ai.geo.badKey",
            "key",
        ),
        (
            draft_with(&ok, |d| d["key"] = json!("")),
            "ai.geo.badKey",
            "key",
        ),
        (
            draft_with(&ok, |d| d["key"] = json!("k".repeat(401))),
            "ai.geo.badKey",
            "key",
        ),
        (
            draft(&format!("{}/{{key}}/{{ip}}", stub.url)),
            "ai.geo.keyRequired",
            "key",
        ),
        (
            draft_with(&ok, |d| {
                d["keyHeaderName"] = json!("Authorization");
                d["keyHeaderValue"] = json!("Bearer {key}");
            }),
            "ai.geo.keyRequired",
            "key",
        ),
        (
            draft_with(&ok, |d| {
                d["mapping"] = json!({"country": "", "countryCode": "", "region": "", "city": "",
                    "okPath": "", "okValue": "", "errorPath": "", "messagePath": ""});
            }),
            "ai.geo.badMapping",
            "mapping.country",
        ),
        (
            draft_with(&ok, |d| d["mapping"]["city"] = json!("a b")),
            "ai.geo.badMapping",
            "mapping.city",
        ),
        (
            draft_with(&ok, |d| d["mapping"]["okPath"] = json!("status")),
            "ai.geo.badMapping",
            "mapping.okValue",
        ),
        (
            draft_with(&ok, |d| d["mapping"]["messagePath"] = json!("a..b")),
            "ai.geo.badMapping",
            "mapping.messagePath",
        ),
    ];
    for path in [TEST, SAVE] {
        for (body, code, field) in &cases {
            let (s, v) = t.post(path, &admin, body.clone()).await;
            assert_eq!(s, 400, "{path} {body}");
            assert_eq!(v["code"], json!(code), "{path} {body}");
            assert_eq!(v["field"], json!(field), "{path} {body}");
            assert!(v["error"].as_str().is_some_and(|e| !e.is_empty()));
        }
    }
    assert_eq!(stub.count(), 0, "no invalid draft reaches a provider");
    assert_eq!(geo_rows(&t), 0);
}

#[tokio::test]
async fn a_key_in_the_query_only_is_optional_and_is_left_out_without_one() {
    let t = start().await;
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    let url = format!("{}/ok/{{ip}}?lang=en&token={{key}}", stub.url);
    let (s, v) = t.post(TEST, &admin, draft(&url)).await;
    assert_eq!((s, v["ok"].clone()), (200, json!(true)), "{v}");
    let seen = stub.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].target(), "/ok/8.8.8.8?lang=en");
    // with a key it is sent percent-encoded in the URL
    let (s, v) = t
        .post(
            TEST,
            &admin,
            draft_with(&url, |d| d["key"] = json!("a+b/c")),
        )
        .await;
    assert_eq!((s, v["ok"].clone()), (200, json!(true)), "{v}");
    assert_eq!(
        stub.requests()[1].target(),
        "/ok/8.8.8.8?lang=en&token=a%2Bb%2Fc"
    );
}

#[tokio::test]
async fn a_saved_key_never_follows_a_draft_to_another_origin() {
    let t = start_with(opts_with_master(MASTER_A)).await;
    let a = geo_stub().await;
    let b = geo_stub().await;
    let admin = t.user("admin", true, false);
    let url_a = format!("{}/ok/{{ip}}?token={{key}}", a.url);
    let (s, v) = t
        .post(
            SAVE,
            &admin,
            draft_with(&url_a, |d| d["key"] = json!(GEO_KEY)),
        )
        .await;
    assert_eq!((s, v["ok"].clone()), (200, json!(true)), "{v}");
    let url_b = format!("{}/ok/{{ip}}?token={{key}}", b.url);
    let (s, v) = t.post(TEST, &admin, draft(&url_b)).await;
    assert_eq!((s, v["ok"].clone()), (200, json!(true)), "{v}");
    let (s, v) = t.post(SAVE, &admin, draft(&url_b)).await;
    assert_eq!((s, v["ok"].clone()), (200, json!(true)), "{v}");
    let seen = b.requests();
    assert_eq!(seen.len(), 2);
    for r in &seen {
        assert!(!r.target().contains("token="), "{}", r.target());
        assert!(!r.target().contains(GEO_KEY));
    }
    let (_, v) = t.get(GET, &admin).await;
    assert_eq!(v["settings"]["key"]["set"], json!(false), "{v}");
    // a key in the path cannot be dropped: the draft needs a new key
    let url_c = format!("{}/{{key}}/{{ip}}", a.url);
    let (_, v) = t.post(SAVE, &admin, draft(&url_c)).await;
    assert_eq!(v["code"], json!("ai.geo.keyRequired"), "{v}");
}

// —— the test route ———————————————————————————————————————————————————————————————————————————————————————————————————

fn ipapi_mapping() -> Value {
    json!({"country": "country", "countryCode": "countryCode", "region": "regionName", "city": "city",
           "okPath": "status", "okValue": "success", "errorPath": "", "messagePath": "message"})
}

fn ipapico_mapping() -> Value {
    json!({"country": "country_name", "countryCode": "country_code", "region": "region", "city": "city",
           "okPath": "", "okValue": "", "errorPath": "error", "messagePath": "reason"})
}

#[tokio::test]
async fn the_test_route_reports_the_place_or_the_exact_failure() {
    let t = start().await;
    let stub = geo_stub().await;
    let other = geo_stub().await;
    let admin = t.user("admin", true, false);
    // a place
    let (s, v) = t.post(TEST, &admin, draft(&stub.template("ok"))).await;
    assert_eq!(s, 200);
    assert_eq!(v["ok"], json!(true));
    assert_eq!(v["testIp"], json!("8.8.8.8"));
    assert_eq!(
        v["place"],
        json!({"country": null, "countryCode": "ZZ", "region": "Example Region", "city": "Example City"})
    );
    assert!(v["ms"].as_i64().is_some_and(|ms| ms >= 0));
    assert!(v.get("failure").is_none());
    assert_eq!(stub.requests()[0].path, "/ok/8.8.8.8");
    assert_eq!(
        stub.requests()[0].header("accept"),
        Some("application/json")
    );
    // ip-api shape
    let (_, v) = t
        .post(
            TEST,
            &admin,
            draft_with(&stub.template("ipapi"), |d| d["mapping"] = ipapi_mapping()),
        )
        .await;
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["place"]["country"], json!("Exampleland"));
    assert_eq!(v["place"]["region"], json!("Example Region"));
    // failures: (behaviour, mapping, code, status, message)
    let fails: Vec<(String, Value, &str, Value, Value)> = vec![
        (
            stub.template("ipapi-fail"),
            ipapi_mapping(),
            "providerFail",
            json!(null),
            json!("reserved range"),
        ),
        (
            stub.template("ipapico-err"),
            ipapico_mapping(),
            "providerFail",
            json!(null),
            json!("RateLimited"),
        ),
        (
            stub.template("s500"),
            mapping(),
            "httpStatus",
            json!(500),
            json!(null),
        ),
        (
            stub.template("s404"),
            mapping(),
            "httpStatus",
            json!(404),
            json!(null),
        ),
        (
            stub.template("text"),
            mapping(),
            "notJson",
            json!(null),
            json!(null),
        ),
        (
            stub.template("array"),
            mapping(),
            "notJson",
            json!(null),
            json!(null),
        ),
        (
            stub.template("empty"),
            mapping(),
            "noFields",
            json!(null),
            json!(null),
        ),
        (
            stub.template("big"),
            mapping(),
            "tooLarge",
            json!(null),
            json!(null),
        ),
        (
            format!("{}/redir/{}/{{ip}}", stub.url, other.port),
            mapping(),
            "redirect",
            json!(null),
            json!(null),
        ),
    ];
    for (url, map, code, status, message) in fails {
        let (s, v) = t
            .post(TEST, &admin, draft_with(&url, |d| d["mapping"] = map))
            .await;
        assert_eq!(s, 200, "{url}");
        assert_eq!(v["ok"], json!(false), "{url}");
        assert_eq!(v["testIp"], json!("8.8.8.8"));
        assert_eq!(
            v["failure"],
            json!({"code": code, "status": status, "message": message}),
            "{url}"
        );
        assert!(v.get("place").is_none());
    }
    // the redirect was refused: the second host was never contacted
    assert_eq!(other.count(), 0);
    assert_eq!(
        stub.requests()
            .iter()
            .filter(|r| r.path.starts_with("/redir/"))
            .count(),
        1
    );
    // nothing listens on that port: unreachable
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        l.local_addr().expect("addr").port()
    };
    let (s, v) = t
        .post(
            TEST,
            &admin,
            draft(&format!("http://127.0.0.1:{closed}/ok/{{ip}}")),
        )
        .await;
    assert_eq!(s, 200);
    assert_eq!(v["failure"]["code"], json!("unreachable"), "{v}");
    // a test saves nothing
    assert_eq!(geo_rows(&t), 0);
}

#[tokio::test]
async fn a_slow_provider_times_out() {
    let t = start_with(Opts {
        timeouts: Timeouts {
            geo_total: Duration::from_millis(300),
            ..fast_timeouts()
        },
        ..Opts::default()
    })
    .await;
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    let (s, v) = t.post(TEST, &admin, draft(&stub.template("slow"))).await;
    assert_eq!(s, 200);
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["failure"]["code"], json!("timeout"), "{v}");
    assert!(
        v["ms"].as_i64().is_some_and(|ms| (250..1500).contains(&ms)),
        "{v}"
    );
    // the settings view tells the limit
    let (_, g) = t.get(GET, &admin).await;
    assert_eq!(g["timeoutMs"], json!(300));
}

#[tokio::test]
async fn the_per_minute_cap_applies_to_the_test_button() {
    let t = start().await;
    t.app
        .0
        .knobs
        .geo_per_minute
        .store(3, std::sync::atomic::Ordering::SeqCst);
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    let d = draft(&stub.template("ok"));
    for i in 0..3 {
        let (_, v) = t.post(TEST, &admin, d.clone()).await;
        assert_eq!(v["ok"], json!(true), "test {i}: {v}");
    }
    let (s, v) = t.post(TEST, &admin, d.clone()).await;
    assert_eq!(s, 200);
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["failure"]["code"], json!("rateLimited"), "{v}");
    assert_eq!(stub.count(), 3, "the fourth never reached the provider");
    let (_, g) = t.get(GET, &admin).await;
    assert_eq!(g["perMinute"], json!(3));
}

#[tokio::test]
async fn only_one_test_runs_at_a_time() {
    let t = start().await;
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    let slow = draft(&stub.template("slow"));
    let quick = draft(&stub.template("ok"));
    let (first, second) = tokio::join!(t.post(TEST, &admin, slow), async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let a = t.post(TEST, &admin, quick.clone()).await;
        let b = t.post(SAVE, &admin, quick.clone()).await;
        (a, b)
    });
    assert_eq!(first.0, 200);
    assert_eq!(first.1["ok"], json!(true), "{}", first.1);
    for (s, v) in [second.0, second.1] {
        assert_eq!(s, 409, "{v}");
        assert_eq!(v["code"], json!("ai.geo.busy"));
    }
    // free again afterwards
    let (s, v) = t.post(TEST, &admin, quick).await;
    assert_eq!((s, v["ok"].clone()), (200, json!(true)));
}

// —— save, key, enable, delete ————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn a_failed_test_saves_nothing_and_a_pass_saves_the_settings() {
    let t = start_with(opts_with_master(MASTER_A)).await;
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    let (s, v) = t.post(SAVE, &admin, draft(&stub.template("s500"))).await;
    assert_eq!(s, 200);
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["failure"]["code"], json!("httpStatus"));
    assert_eq!(v["failure"]["status"], json!(500));
    assert!(v.get("settings").is_none());
    assert_eq!(geo_rows(&t), 0);
    let (_, g) = t.get(GET, &admin).await;
    assert_eq!(g["settings"], json!(null));

    let url = format!("{}/ok/{{ip}}?token={{key}}", stub.url);
    let body = draft_with(&url, |d| {
        d["preset"] = json!("ipinfo");
        d["key"] = json!(GEO_KEY);
    });
    let (s, v) = t.post(SAVE, &admin, body).await;
    assert_eq!(s, 200);
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["place"]["city"], json!("Example City"));
    let settings = v["settings"].clone();
    assert_eq!(settings["preset"], json!("ipinfo"));
    assert_eq!(settings["urlTemplate"], json!(url));
    assert_eq!(settings["host"], json!("127.0.0.1"));
    assert_eq!(settings["insecureHttp"], json!(true));
    assert_eq!(settings["mapping"], mapping());
    assert_eq!(
        settings["key"],
        json!({"set": true, "hint": "…cdef", "unreadable": false})
    );
    assert!(settings["testedAt"]
        .as_i64()
        .is_some_and(|n| n > 1_700_000_000));
    assert_eq!(settings["testedAt"], settings["updatedAt"]);
    // the key reached the provider (once, in the URL) and nowhere else
    assert_eq!(
        stub.requests().last().expect("request").target(),
        format!("/ok/8.8.8.8?token={GEO_KEY}")
    );
    assert!(!v.to_string().contains(GEO_KEY));
    // GET: the same settings, never the key
    let (_, g) = t.get(GET, &admin).await;
    assert_eq!(g["settings"], settings);
    assert!(!g.to_string().contains(GEO_KEY));
    // the key is sealed in the database
    let (key_id, nonce, ct, hint): (String, Vec<u8>, Vec<u8>, String) = t
        .db()
        .query_row(
            "SELECT key_id, key_nonce, key_ciphertext, key_hint FROM geo_settings WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("row");
    assert_eq!(key_id, t.app.0.key_id());
    assert_eq!(nonce.len(), 24);
    assert!(!ct.windows(GEO_KEY.len()).any(|w| w == GEO_KEY.as_bytes()));
    assert_eq!(hint, "…cdef");
    let by: Option<i64> = t
        .db()
        .query_row("SELECT updated_by FROM geo_settings", [], |r| r.get(0))
        .expect("by");
    assert_eq!(by, Some(admin.id));
}

#[tokio::test]
async fn the_key_is_kept_replaced_or_removed() {
    let t = start_with(opts_with_master(MASTER_A)).await;
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    let url = format!("{}/ok/{{ip}}?token={{key}}", stub.url);
    let with_key = |key: Option<Value>| {
        draft_with(&url, |d| {
            if let Some(k) = key {
                d["key"] = k;
            }
        })
    };
    // set
    let (_, v) = t.post(SAVE, &admin, with_key(Some(json!(GEO_KEY)))).await;
    assert_eq!(v["settings"]["key"]["hint"], json!("…cdef"), "{v}");
    let first_ct: Vec<u8> = t
        .db()
        .query_row("SELECT key_ciphertext FROM geo_settings", [], |r| r.get(0))
        .expect("ct");
    // keep: the test uses the saved key, the stored key is unchanged
    let (_, v) = t.post(SAVE, &admin, with_key(None)).await;
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["settings"]["key"]["set"], json!(true));
    assert_eq!(v["settings"]["key"]["hint"], json!("…cdef"));
    assert_eq!(
        stub.requests().last().expect("request").target(),
        format!("/ok/8.8.8.8?token={GEO_KEY}")
    );
    let kept_ct: Vec<u8> = t
        .db()
        .query_row("SELECT key_ciphertext FROM geo_settings", [], |r| r.get(0))
        .expect("ct");
    assert_eq!(first_ct, kept_ct);
    // a test without a key field also uses the saved one
    let (_, v) = t.post(TEST, &admin, with_key(None)).await;
    assert_eq!(v["ok"], json!(true));
    assert_eq!(
        stub.requests().last().expect("request").target(),
        format!("/ok/8.8.8.8?token={GEO_KEY}")
    );
    // replace: another key, another hint and ciphertext
    let (_, v) = t
        .post(
            SAVE,
            &admin,
            with_key(Some(json!("OTHERKEY-geo-fedcba9876543210"))),
        )
        .await;
    assert_eq!(v["settings"]["key"]["hint"], json!("…3210"));
    let new_ct: Vec<u8> = t
        .db()
        .query_row("SELECT key_ciphertext FROM geo_settings", [], |r| r.get(0))
        .expect("ct");
    assert_ne!(new_ct, first_ct);
    // a short key has no hint
    let (_, v) = t.post(SAVE, &admin, with_key(Some(json!("short")))).await;
    assert_eq!(
        v["settings"]["key"],
        json!({"set": true, "hint": "", "unreadable": false})
    );
    // remove: a template that needs the key in its path is refused, a query-only one is fine
    let (s, v) = t.post(SAVE, &admin, with_key(Some(json!(null)))).await;
    assert_eq!(s, 200);
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(
        v["settings"]["key"],
        json!({"set": false, "hint": "", "unreadable": false})
    );
    let n: i64 = t
        .db()
        .query_row(
            "SELECT COUNT(*) FROM geo_settings WHERE key_id IS NOT NULL OR key_nonce IS NOT NULL OR key_ciphertext IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(n, 0);
    assert_eq!(
        stub.requests().last().expect("request").target(),
        "/ok/8.8.8.8"
    );
    let path_key = draft(&format!("{}/ok/{{key}}/{{ip}}", stub.url));
    let (s, v) = t.post(SAVE, &admin, path_key.clone()).await;
    assert_eq!((s, v["code"].clone()), (400, json!("ai.geo.keyRequired")));
    let (s, v) = t
        .post(
            SAVE,
            &admin,
            draft_with(&format!("{}/ok/{{key}}/{{ip}}", stub.url), |d| {
                d["key"] = json!(null)
            }),
        )
        .await;
    assert_eq!((s, v["code"].clone()), (400, json!("ai.geo.keyRequired")));
}

#[tokio::test]
async fn a_key_in_a_header_goes_only_into_the_header() {
    let t = start().await;
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    let body = draft_with(&stub.template("ok"), |d| {
        d["keyHeaderName"] = json!("Authorization");
        d["keyHeaderValue"] = json!("Bearer {key}");
        d["key"] = json!(GEO_KEY);
    });
    let (_, v) = t.post(TEST, &admin, body).await;
    assert_eq!(v["ok"], json!(true), "{v}");
    let req = stub.requests().pop().expect("request");
    assert_eq!(req.target(), "/ok/8.8.8.8");
    assert_eq!(
        req.header("authorization"),
        Some(&*format!("Bearer {GEO_KEY}"))
    );
}

#[tokio::test]
async fn a_key_that_cannot_be_opened_is_reported_and_blocks_enable() {
    let mut t = start_with(opts_with_master(MASTER_A)).await;
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    let url = format!("{}/ok/{{ip}}?token={{key}}", stub.url);
    let body = draft_with(&url, |d| d["key"] = json!(GEO_KEY));
    let (_, v) = t.post(SAVE, &admin, body).await;
    assert_eq!(v["ok"], json!(true), "{v}");
    // the encryption key changed
    t.restart(opts_with_master(MASTER_B)).await;
    let (_, g) = t.get(GET, &admin).await;
    assert_eq!(
        g["settings"]["key"],
        json!({"set": true, "hint": "…cdef", "unreadable": true}),
        "{g}"
    );
    let (s, v) = t.post(ENABLE, &admin, json!({})).await;
    assert_eq!((s, v["code"].clone()), (409, json!("ai.geo.keyProblem")));
    assert!(t.backend.st().geo.source_calls.is_empty());
    // a test that keeps the key fails without a call
    let before = stub.count();
    let (_, v) = t.post(TEST, &admin, draft(&url)).await;
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["failure"]["code"], json!("keyProblem"), "{v}");
    assert_eq!(stub.count(), before);
    // entering the key again repairs it
    let (_, v) = t
        .post(
            SAVE,
            &admin,
            draft_with(&url, |d| d["key"] = json!(GEO_KEY)),
        )
        .await;
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["settings"]["key"]["unreadable"], json!(false));
    let (s, v) = t.post(ENABLE, &admin, json!({})).await;
    assert_eq!((s, v["source"].clone()), (200, json!("api")));
}

#[tokio::test]
async fn enable_needs_saved_settings_and_delete_goes_back_to_cloudflare() {
    let t = start().await;
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    // nothing saved
    let (s, v) = t.post(ENABLE, &admin, json!({})).await;
    assert_eq!((s, v["code"].clone()), (409, json!("ai.geo.testFirst")));
    assert!(t.backend.st().geo.source_calls.is_empty());
    let (_, v) = t.post(SAVE, &admin, draft(&stub.template("ok"))).await;
    assert_eq!(v["ok"], json!(true));
    // the backend is unreachable for the switch: nothing changes
    t.backend.st().geo.source_down = true;
    let (s, v) = t.post(ENABLE, &admin, json!({})).await;
    assert_eq!((s, v["code"].clone()), (503, json!("ai.geo.linkDown")));
    let (s, v) = t.post(DELETE, &admin, json!({})).await;
    assert_eq!((s, v["code"].clone()), (503, json!("ai.geo.linkDown")));
    assert_eq!(
        geo_rows(&t),
        1,
        "nothing is deleted while the switch cannot be made"
    );
    let (_, g) = t.get(GET, &admin).await;
    assert!(g["settings"].is_object());
    t.backend.st().geo.source_down = false;
    // enable
    let (s, v) = t.post(ENABLE, &admin, json!({})).await;
    assert_eq!((s, v), (200, json!({"source": "api"})));
    assert_eq!(t.backend.st().geo.source_name(), "api");
    assert_eq!(
        t.backend.st().geo.source_calls,
        vec![json!({"source": "api"})]
    );
    // delete: first the switch, then the settings
    let (s, v) = t.post(DELETE, &admin, json!({})).await;
    assert_eq!(
        (s, v),
        (200, json!({"settings": null, "source": "cloudflare"}))
    );
    assert_eq!(t.backend.st().geo.source_name(), "cloudflare");
    assert_eq!(
        t.backend.st().geo.source_calls.last().cloned(),
        Some(json!({"source": "cloudflare"}))
    );
    assert_eq!(geo_rows(&t), 0);
    let (_, g) = t.get(GET, &admin).await;
    assert_eq!(g["settings"], json!(null));
    // enable is refused again, and a second delete is harmless
    let (s, v) = t.post(ENABLE, &admin, json!({})).await;
    assert_eq!((s, v["code"].clone()), (409, json!("ai.geo.testFirst")));
    let (s, _) = t.post(DELETE, &admin, json!({})).await;
    assert_eq!(s, 200);
}

#[tokio::test]
async fn a_delete_removes_the_sealed_key_from_the_database() {
    let t = start_with(opts_with_master(MASTER_A)).await;
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    let url = format!("{}/ok/{{ip}}?token={{key}}", stub.url);
    let (_, v) = t
        .post(
            SAVE,
            &admin,
            draft_with(&url, |d| d["key"] = json!(GEO_KEY)),
        )
        .await;
    assert_eq!(v["ok"], json!(true));
    assert_eq!(geo_rows(&t), 1);
    let (s, _) = t.post(DELETE, &admin, json!({})).await;
    assert_eq!(s, 200);
    assert_eq!(geo_rows(&t), 0);
    // a restart finds nothing in memory either
    let (_, g) = t.get(GET, &admin).await;
    assert_eq!(g["settings"], json!(null));
}

#[tokio::test]
async fn settings_survive_a_restart() {
    let mut t = start_with(opts_with_master(MASTER_A)).await;
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    let (_, v) = t.post(SAVE, &admin, draft(&stub.template("ok"))).await;
    assert_eq!(v["ok"], json!(true));
    let (_, before) = t.get(GET, &admin).await;
    t.restart(opts_with_master(MASTER_A)).await;
    let (_, after) = t.get(GET, &admin).await;
    assert_eq!(before, after);
    assert!(after["settings"].is_object());
}

// —— independence from AI+ and its counters ———————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn it_works_with_ai_off_and_never_touches_the_ai_counters() {
    let t = start().await;
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    // AI+ is off and no connection is configured
    let snap = t.app.0.snapshot().expect("snapshot");
    assert!(!snap.enabled);
    assert!(!t.app.0.route_exists(xexam_ai_service::route::Slot::Default));
    let limits_before = t.app.0.limits();
    let (s, v) = t.post(SAVE, &admin, draft(&stub.template("ok"))).await;
    assert_eq!((s, v["ok"].clone()), (200, json!(true)), "{v}");
    let (s, v) = t.post(TEST, &admin, draft(&stub.template("s500"))).await;
    assert_eq!((s, v["ok"].clone()), (200, json!(false)));
    let (s, _) = t.post(ENABLE, &admin, json!({})).await;
    assert_eq!(s, 200);
    // the worker's lookups
    let _worker = xexam_ai_service::geo::worker::spawn(t.app.clone(), t.app.0.shutdown.clone());
    t.app
        .0
        .knobs
        .geo_wait_ms
        .store(100, std::sync::atomic::Ordering::SeqCst);
    t.backend.st().geo.push_job("0123456789abcdef", "1.1.1.1");
    eventually(|| !t.backend.st().geo.results.is_empty()).await;
    // no AI usage, no generation call, no limit changed, no AI switch moved
    assert_eq!(t.usage_rows(), 0);
    assert_eq!(count(&t, "gen_calls"), 0);
    assert_eq!(count(&t, "tasks"), 0);
    assert_eq!(t.app.0.limits(), limits_before);
    assert!(!t.app.0.snapshot().expect("snapshot").enabled);
    assert_eq!(count(&t, "chats"), 0);
}

// —— secrets ——————————————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn the_key_never_appears_in_an_answer_an_event_a_log_line_a_failure_message_or_the_database_file(
) {
    let t = start_with(opts_with_master(MASTER_A)).await;
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    let mut answers: Vec<Value> = Vec::new();
    // a provider that echoes the key it was sent, in the URL and in a header
    let echo_url = format!("{}/echo/{{ip}}?token={{key}}", stub.url);
    let echo_map = json!({"country": "", "countryCode": "country", "region": "", "city": "city",
        "okPath": "", "okValue": "", "errorPath": "error", "messagePath": "message"});
    let (_, v) = t
        .post(
            TEST,
            &admin,
            draft_with(&echo_url, |d| {
                d["mapping"] = echo_map.clone();
                d["key"] = json!(GEO_KEY);
            }),
        )
        .await;
    assert_eq!(v["failure"]["code"], json!("providerFail"), "{v}");
    assert_eq!(v["failure"]["message"], json!("denied for ***"), "{v}");
    answers.push(v);
    let (_, v) = t
        .post(
            TEST,
            &admin,
            draft_with(&stub.template("echo"), |d| {
                d["mapping"] = echo_map.clone();
                d["keyHeaderName"] = json!("Authorization");
                d["keyHeaderValue"] = json!("Bearer {key}");
                d["key"] = json!(GEO_KEY);
            }),
        )
        .await;
    assert_eq!(
        v["failure"]["message"],
        json!("denied for Bearer ***"),
        "{v}"
    );
    answers.push(v);
    // a failing save, a passing save, a view, a failing enable, a delete
    let url = format!("{}/ok/{{ip}}?token={{key}}", stub.url);
    let (_, v) = t
        .post(
            SAVE,
            &admin,
            draft_with(&format!("{}/s500/{{ip}}?token={{key}}", stub.url), |d| {
                d["key"] = json!(GEO_KEY)
            }),
        )
        .await;
    answers.push(v);
    let (_, v) = t
        .post(
            SAVE,
            &admin,
            draft_with(&url, |d| d["key"] = json!(GEO_KEY)),
        )
        .await;
    assert_eq!(v["ok"], json!(true));
    answers.push(v);
    let (_, v) = t.get(GET, &admin).await;
    answers.push(v);
    let (_, v) = t
        .post(
            TEST,
            &admin,
            draft_with(&url, |d| d["key"] = json!("bad key")),
        )
        .await;
    answers.push(v);
    let (_, v) = t.post(ENABLE, &admin, json!({})).await;
    answers.push(v);
    for a in &answers {
        assert!(!a.to_string().contains(GEO_KEY), "{a}");
    }
    // events
    let all = events(&t).await;
    let geo_events: Vec<&Value> = all
        .iter()
        .filter(|e| {
            e["action"]
                .as_str()
                .is_some_and(|a| a.starts_with("ai.geo."))
        })
        .collect();
    assert_eq!(geo_events.len(), 5, "{geo_events:?}");
    assert!(!serde_json::to_string(&all).expect("json").contains(GEO_KEY));
    // logs
    assert!(!captured_contains(GEO_KEY));
    assert!(!captured_contains("TESTKEY"));
    // the database file and its journal never hold the key in clear
    let path = t.db_path();
    for file in [
        path.clone(),
        std::path::PathBuf::from(format!("{}-wal", path.display())),
    ] {
        let bytes = std::fs::read(&file).unwrap_or_default();
        assert!(
            !bytes
                .windows(GEO_KEY.len())
                .any(|w| w == GEO_KEY.as_bytes()),
            "{}",
            file.display()
        );
    }
    // the secret key file of the service is not what holds it either
    let key_file = t.dir.path().join("ai").join("secret.key");
    if let Ok(bytes) = std::fs::read(key_file) {
        assert!(!bytes
            .windows(GEO_KEY.len())
            .any(|w| w == GEO_KEY.as_bytes()));
    }
}

#[tokio::test]
async fn the_geo_events_name_only_the_host_the_preset_and_what_happened_to_the_key() {
    let t = start_with(opts_with_master(MASTER_A)).await;
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    let url = format!("{}/ok/{{ip}}?token={{key}}", stub.url);
    t.post(TEST, &admin, draft(&stub.template("s500"))).await;
    t.post(
        SAVE,
        &admin,
        draft_with(&url, |d| {
            d["preset"] = json!("ipinfo");
            d["key"] = json!(GEO_KEY);
        }),
    )
    .await;
    t.post(SAVE, &admin, draft(&stub.template("s500"))).await;
    t.post(ENABLE, &admin, json!({})).await;
    t.post(DELETE, &admin, json!({})).await;
    let all = events(&t).await;
    let tests = action_events(&all, "ai.geo.test");
    assert_eq!(tests.len(), 1);
    assert_eq!(tests[0]["result"], json!("fail"));
    assert_eq!(tests[0]["reason"], json!("httpStatus"));
    assert_eq!(
        tests[0]["detail"],
        json!({"host": "127.0.0.1", "code": "httpStatus"})
    );
    assert_eq!(tests[0]["username"], json!("admin"));
    let saves = action_events(&all, "ai.geo.save");
    assert_eq!(saves.len(), 2);
    assert_eq!(saves[0]["result"], json!("ok"));
    assert_eq!(
        saves[0]["detail"],
        json!({"host": "127.0.0.1", "change": "ipinfo", "secret": "set"})
    );
    assert_eq!(saves[1]["result"], json!("fail"));
    assert_eq!(
        saves[1]["detail"],
        json!({"host": "127.0.0.1", "change": "custom", "secret": "kept", "code": "httpStatus"})
    );
    let enables = action_events(&all, "ai.geo.enable");
    assert_eq!(enables.len(), 1);
    assert_eq!(enables[0]["detail"], json!({"host": "127.0.0.1"}));
    let deletes = action_events(&all, "ai.geo.delete");
    assert_eq!(deletes.len(), 1);
    assert_eq!(deletes[0]["detail"], json!(null));
    assert_eq!(deletes[0]["result"], json!("ok"));
}

// —— the worker ———————————————————————————————————————————————————————————————————————————————————————————————————————

async fn worker_app(stub: &GeoStub, kind: &str) -> (TestApp, Who, tokio::task::JoinHandle<()>) {
    let t = start().await;
    let admin = t.user("admin", true, false);
    if !kind.is_empty() {
        let (_, v) = t.post(SAVE, &admin, draft(&stub.template(kind))).await;
        assert_eq!(v["ok"], json!(true), "{v}");
    }
    t.app
        .0
        .knobs
        .geo_wait_ms
        .store(100, std::sync::atomic::Ordering::SeqCst);
    let h = xexam_ai_service::geo::worker::spawn(t.app.clone(), t.app.0.shutdown.clone());
    (t, admin, h)
}

fn results(t: &TestApp) -> Vec<Value> {
    t.backend.st().geo.results.clone()
}

#[tokio::test]
async fn the_worker_polls_looks_up_and_posts_the_place_with_the_job_id() {
    let stub = geo_stub().await;
    let (t, _admin, _h) = worker_app(&stub, "ok").await;
    let before = stub.count();
    t.backend.st().geo.push_job("0123456789abcdef", "1.1.1.1");
    eventually(|| !results(&t).is_empty()).await;
    assert_eq!(
        results(&t),
        vec![json!({"id": "0123456789abcdef", "ok": true,
            "place": {"country": null, "countryCode": "ZZ", "region": "Example Region", "city": "Example City"}})]
    );
    let seen = stub.requests();
    assert_eq!(seen.len(), before + 1);
    assert_eq!(seen.last().expect("request").path, "/ok/1.1.1.1");
    // the poll asked with the configured wait
    let first = t.backend.st().geo.next_bodies[0].clone();
    assert_eq!(first, json!({"waitMs": 100}));
}

#[tokio::test]
async fn a_batch_is_looked_up_and_answered_in_one_call_and_reserved_addresses_are_refused_without_a_call(
) {
    let stub = geo_stub().await;
    let (t, _admin, _h) = worker_app(&stub, "ok").await;
    let before = stub.count();
    {
        let mut s = t.backend.st();
        s.geo.push_job("aaaaaaaaaaaaaaa1", "8.8.8.8");
        s.geo.push_job("aaaaaaaaaaaaaaa2", "10.0.0.1");
        s.geo.push_job("aaaaaaaaaaaaaaa3", "203.0.113.5");
        s.geo.push_job("aaaaaaaaaaaaaaa4", "127.0.0.1");
        s.geo.push_job("aaaaaaaaaaaaaaa5", "::1");
        s.geo.push_job("aaaaaaaaaaaaaaa6", "2606:4700:4700::1111");
    }
    eventually(|| results(&t).len() == 6).await;
    let r = results(&t);
    let by_id = |id: &str| {
        r.iter()
            .find(|x| x["id"] == json!(id))
            .cloned()
            .expect("result")
    };
    for (id, ok) in [
        ("aaaaaaaaaaaaaaa1", true),
        ("aaaaaaaaaaaaaaa2", false),
        ("aaaaaaaaaaaaaaa3", false),
        ("aaaaaaaaaaaaaaa4", false),
        ("aaaaaaaaaaaaaaa5", false),
        ("aaaaaaaaaaaaaaa6", true),
    ] {
        let x = by_id(id);
        assert_eq!(x["ok"], json!(ok), "{id}");
        if !ok {
            assert_eq!(x, json!({"id": id, "ok": false, "code": "refused"}));
        }
    }
    // one call per batch
    assert_eq!(t.backend.st().geo.batches, vec![6]);
    // the provider saw the two public addresses only
    let paths: Vec<String> = stub
        .requests()
        .iter()
        .skip(before)
        .map(|q| q.path.clone())
        .collect();
    assert_eq!(paths.len(), 2, "{paths:?}");
    assert!(paths.contains(&"/ok/8.8.8.8".to_string()));
    assert!(paths.contains(&"/ok/2606:4700:4700::1111".to_string()));
}

#[tokio::test]
async fn without_settings_the_worker_answers_not_configured() {
    let stub = geo_stub().await;
    let (t, admin, _h) = worker_app(&stub, "").await;
    t.backend.st().geo.push_job("bbbbbbbbbbbbbbb1", "1.1.1.1");
    eventually(|| results(&t).len() == 1).await;
    assert_eq!(
        results(&t)[0],
        json!({"id": "bbbbbbbbbbbbbbb1", "ok": false, "code": "notConfigured"})
    );
    assert_eq!(stub.count(), 0);
    // once settings are saved the next job is looked up
    let (_, v) = t.post(SAVE, &admin, draft(&stub.template("ok"))).await;
    assert_eq!(v["ok"], json!(true));
    t.backend.st().geo.push_job("bbbbbbbbbbbbbbb2", "1.1.1.1");
    eventually(|| results(&t).len() == 2).await;
    assert_eq!(results(&t)[1]["ok"], json!(true));
}

#[tokio::test]
async fn a_failing_provider_yields_failure_codes_and_one_log_line_without_address_or_key() {
    let stub = geo_stub().await;
    let t = start().await;
    let admin = t.user("admin", true, false);
    // a template can only be saved after it passed its test, so the saved row is switched to a failing provider
    // behind the service's back (as if the provider broke later)
    let (_, v) = t.post(SAVE, &admin, draft(&stub.template("ok"))).await;
    assert_eq!(v["ok"], json!(true));
    t.db()
        .execute(
            "UPDATE geo_settings SET url_template = ?1",
            [stub.template("s500")],
        )
        .expect("update");
    t.app.0.reload_geo().expect("reload");
    t.app
        .0
        .knobs
        .geo_wait_ms
        .store(100, std::sync::atomic::Ordering::SeqCst);
    let _h = xexam_ai_service::geo::worker::spawn(t.app.clone(), t.app.0.shutdown.clone());
    t.backend.st().geo.push_job("ccccccccccccccc1", "1.1.1.1");
    eventually(|| results(&t).len() == 1).await;
    assert_eq!(
        results(&t)[0],
        json!({"id": "ccccccccccccccc1", "ok": false, "code": "httpStatus"})
    );
    let lines: Vec<String> = captured()
        .into_iter()
        .filter(|l| l.contains("geo lookup via"))
        .collect();
    assert!(!lines.is_empty());
    for l in &lines {
        assert!(l.contains("127.0.0.1 failed:"), "{l}");
        assert!(!l.contains("1.1.1.1"), "{l}");
        assert!(!l.contains("/s500/"), "{l}");
    }
}

#[tokio::test]
async fn the_worker_stays_inside_the_per_minute_cap() {
    let stub = geo_stub().await;
    let (t, _admin, _h) = worker_app(&stub, "ok").await;
    // the save above already used one lookup of this minute; three allow two more (three if the minute just turned)
    t.app
        .0
        .knobs
        .geo_per_minute
        .store(3, std::sync::atomic::Ordering::SeqCst);
    let before = stub.count();
    {
        let mut s = t.backend.st();
        for (i, ip) in ["1.1.1.1", "8.8.4.4", "9.9.9.9", "1.0.0.1"]
            .iter()
            .enumerate()
        {
            s.geo.push_job(&format!("dddddddddddddd{i:02}"), ip);
        }
    }
    eventually(|| results(&t).len() == 4).await;
    let ok = results(&t)
        .iter()
        .filter(|r| r["ok"] == json!(true))
        .count();
    let capped = results(&t)
        .iter()
        .filter(|r| r["ok"] == json!(false) && r["code"] == json!("rateLimited"))
        .count();
    assert!((2..=3).contains(&ok), "{ok}");
    assert_eq!(ok + capped, 4);
    assert_eq!(
        stub.count() - before,
        ok,
        "a capped lookup never reaches the provider"
    );
}

#[tokio::test]
async fn an_older_backend_without_the_routes_makes_the_worker_back_off_and_keeps_the_link() {
    let stub = geo_stub().await;
    let (t, _admin, _h) = worker_app(&stub, "").await;
    assert!(t.app.0.link.is_connected());
    t.backend.st().geo.next_switch = NextSwitch::Http404;
    eventually(|| t.backend_calls("/internal/ai/geo/next") >= 1).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    // one call, then a long pause: no hot loop
    assert_eq!(t.backend_calls("/internal/ai/geo/next"), 1);
    // a 404 on a data route never marks the link lost
    assert!(t.app.0.link.is_connected());
    assert!(t.app.0.link.has_token());
}

#[tokio::test]
async fn a_failing_backend_makes_the_worker_wait_before_asking_again() {
    let stub = geo_stub().await;
    let (t, _admin, _h) = worker_app(&stub, "").await;
    t.backend.st().geo.next_switch = NextSwitch::Http500;
    eventually(|| t.backend_calls("/internal/ai/geo/next") >= 1).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(t.backend_calls("/internal/ai/geo/next"), 1);
}

// —— schema 8 —————————————————————————————————————————————————————————————————————————————————————————————————————————

fn master(c: &rusqlite::Connection) -> Vec<(String, String, Option<String>)> {
    let mut st = c
        .prepare("SELECT type, name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name")
        .expect("prepare");
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

#[test]
fn a_schema_7_file_is_backed_up_and_gets_only_the_geo_table() {
    let fresh = tempfile::tempdir().expect("dir");
    drop(Store::open(fresh.path(), JournalMode::Delete).expect("fresh"));
    let want =
        master(&rusqlite::Connection::open(fresh.path().join("xexam-ai.db")).expect("open fresh"));
    assert!(want.iter().any(|(_, n, _)| n == "geo_settings"));

    let dir = tempfile::tempdir().expect("dir");
    drop(Store::open(dir.path(), JournalMode::Delete).expect("create"));
    let path = dir.path().join("xexam-ai.db");
    {
        // what a v1.3 build left: no geo table, schema 7, and values the schema-7 steps would have changed
        let c = rusqlite::Connection::open(&path).expect("open");
        c.execute_batch(
            "DROP TABLE geo_settings;
             UPDATE meta SET value = '7' WHERE key = 'schema';
             UPDATE limits SET memory_tokens = 777, memories_per_user = 12 WHERE id = 1;
             INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at) VALUES (5, 's', 'sam', 1, 1);
             INSERT INTO chats (id, user_id, title, created_at, updated_at) VALUES (1, 5, 'made-up chat', 1, 1);
             INSERT INTO messages (id, chat_id, role, content, created_at, parent_id)
               VALUES (1, 1, 'user', 'made-up question', 1, NULL), (2, 1, 'assistant', 'made-up answer', 2, 1);
             UPDATE chats SET leaf_id = 2 WHERE id = 1;",
        )
        .expect("fixture");
    }
    let store = Store::open(dir.path(), JournalMode::Delete).expect("upgrade");
    let c = rusqlite::Connection::open(&path).expect("open");
    let v: String = c
        .query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
            r.get(0)
        })
        .expect("schema");
    assert_eq!(v, "9");
    // the same tables and indexes as a new file
    assert_eq!(master(&c), want);
    // one backup, still schema 7 and without the table
    let copies: Vec<_> = std::fs::read_dir(dir.path().join("backups"))
        .expect("backups")
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(copies.len(), 1);
    let name = copies[0].file_name().to_string_lossy().to_string();
    assert!(
        name.starts_with("xexam-ai-schema7-") && name.ends_with(".db"),
        "{name}"
    );
    let b = rusqlite::Connection::open(copies[0].path()).expect("backup");
    let bv: String = b
        .query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
            r.get(0)
        })
        .expect("schema");
    assert_eq!(bv, "7");
    assert!(!master(&b).iter().any(|(_, n, _)| n == "geo_settings"));
    // nothing from the earlier steps ran again: the limits and the chat line are untouched
    let (mt, mpu): (i64, i64) = c
        .query_row(
            "SELECT memory_tokens, memories_per_user FROM limits WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("limits");
    assert_eq!((mt, mpu), (777, 12));
    let (parent, leaf): (Option<i64>, Option<i64>) = c
        .query_row(
            "SELECT (SELECT parent_id FROM messages WHERE id = 2), (SELECT leaf_id FROM chats WHERE id = 1)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("chat");
    assert_eq!((parent, leaf), (Some(1), Some(2)));
    // the table is usable and empty
    let n: i64 = store
        .read(|c| c.query_row("SELECT COUNT(*) FROM geo_settings", [], |r| r.get(0)))
        .expect("count");
    assert_eq!(n, 0);
    drop(store);
    // reopening the upgraded file writes no second backup
    drop(Store::open(dir.path(), JournalMode::Delete).expect("reopen"));
    assert_eq!(
        std::fs::read_dir(dir.path().join("backups"))
            .expect("backups")
            .count(),
        1
    );
}

#[tokio::test]
async fn a_save_tells_the_backend_only_while_the_source_is_api() {
    let t = start().await;
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    // cloudflare: the source is read, nothing is switched or sent
    let (_, v) = t.post(SAVE, &admin, draft(&stub.template("ok"))).await;
    assert_eq!(v["ok"], json!(true), "{v}");
    assert!(t.backend.st().geo.source_calls.is_empty());
    assert_eq!(t.backend.st().geo.source_reads, 1);
    // api: exactly one {"source":"api"} call per save
    let (s, _) = t.post(ENABLE, &admin, json!({})).await;
    assert_eq!(s, 200);
    t.backend.st().geo.source_calls.clear();
    let (_, v) = t.post(SAVE, &admin, draft(&stub.template("ok"))).await;
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(
        t.backend.st().geo.source_calls,
        vec![json!({"source": "api"})]
    );
    // a failed test saves nothing and tells nobody
    t.backend.st().geo.source_calls.clear();
    let (_, v) = t.post(SAVE, &admin, draft(&stub.template("s500"))).await;
    assert_eq!(v["ok"], json!(false), "{v}");
    assert!(t.backend.st().geo.source_calls.is_empty());
}

#[tokio::test]
async fn a_save_succeeds_while_the_backend_link_is_down() {
    let t = start().await;
    let stub = geo_stub().await;
    let admin = t.user("admin", true, false);
    t.backend.st().geo.source_down = true;
    let (s, v) = t.post(SAVE, &admin, draft(&stub.template("ok"))).await;
    assert_eq!((s, v["ok"].clone()), (200, json!(true)), "{v}");
    assert_eq!(geo_rows(&t), 1);
    assert!(t.backend.st().geo.source_calls.is_empty());
}
