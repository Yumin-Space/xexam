//! v1.0 per-user caps on AI model calls (spec section 3): 5 / minute, 20 / hour, 100 / day by default, everyone including
//! administrators, minute and hour rolling, the day resetting at 0:00 of the server's time zone (`TZ=UTC` in these tests).
//! Counted: every `chat` / `memory` usage row of the user plus the calls still running; not counted: the administrator's
//! connection test and model list. All names, questions and texts are made up.
mod common;

#[path = "study_api/util.rs"]
mod util;

use axum::http::Method;
use serde_json::{json, Value};
use util::*;
use xexam_ai_service::store::stats::{self, UsageRow};

const CHAT: &str = "/api/ai/send";
const SUGGEST: &str = "/api/ai/memories/suggest";

fn chat_body(text: &str) -> Value {
    json!({"chatId": null, "text": text, "lang": "en"})
}

/// A POST that keeps the headers: (status, `Retry-After`, JSON).
async fn post_raw(t: &TestApp, who: &Who, path: &str, body: Value) -> (u16, Option<String>, Value) {
    let r = t
        .raw(
            Method::POST,
            path,
            Some(t.ident(who)),
            Some(body.to_string().into_bytes()),
            Some("application/json"),
        )
        .await;
    let retry = r
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let (s, v) = read_json(r).await;
    (s, retry, v)
}

async fn set_caps(t: &TestApp, admin: &Who, minute: i64, hour: i64, day: i64) {
    let (s, v) = t
        .post(
            "/api/ai/admin/limits",
            admin,
            json!({"callsPerMinute": minute, "callsPerHour": hour, "callsPerDay": day}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
}

/// Makes the AI database know the user (no model call), so usage rows inserted by the test keep their user id.
async fn touch(t: &TestApp, who: &Who) {
    let (s, v) = t
        .post(
            "/api/ai/memories/add",
            who,
            json!({"content": "A made-up note."}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
}

fn counted_rows(t: &TestApp, user: i64) -> i64 {
    t.db()
        .query_row(
            "SELECT COUNT(*) FROM usage WHERE user_id = ?1 AND kind IN ('chat', 'memory')",
            [user],
            |r| r.get(0),
        )
        .expect("count")
}

fn row_counts(t: &TestApp) -> (i64, i64, i64) {
    let n = |sql: &str| -> i64 { t.db().query_row(sql, [], |r| r.get(0)).expect(sql) };
    (
        n("SELECT COUNT(*) FROM chats"),
        n("SELECT COUNT(*) FROM messages"),
        n("SELECT COUNT(*) FROM usage"),
    )
}

/// Inserts one finished made-up call that started at `start` (latency 0).
fn insert_call(t: &TestApp, user: i64, start: i64, kind: &'static str, code: Option<&str>) {
    let u = UsageRow {
        user_id: Some(user),
        at: start,
        kind,
        provider: "openai",
        model: "demo-chat-small".into(),
        tokens_in: None,
        tokens_out: None,
        tokens_cached: None,
        tokens_cache_write: None,
        latency_ms: 0,
        first_byte_ms: None,
        status: if code.is_some() { "error" } else { "ok" },
        error_code: code.map(str::to_string),
    };
    stats::insert_usage(&t.db(), &u).expect("insert");
}

fn day() -> xexam_ai_service::quota::Day {
    xexam_ai_service::util::day_bounds(now())
}

/// Waits until a few seconds before the next 0:00 have passed, so a test never straddles the day change.
async fn clear_of_midnight() {
    let d = day();
    if d.next - now() < 5 {
        tokio::time::sleep(std::time::Duration::from_secs(
            u64::try_from(d.next - now() + 1).unwrap_or(1),
        ))
        .await;
    }
}

#[tokio::test]
async fn the_minute_cap_refuses_the_third_ask_and_writes_nothing() {
    let e = env().await;
    set_caps(&e.t, &e.admin, 2, 3, 4).await;
    for (q, no) in [("q1", 1), ("q2", 2)] {
        let ev = ask(&e.t, &e.user, hint_body(K1, q, no)).await;
        assert_eq!(terminal_name(&ev), "done");
    }
    let before = row_counts(&e.t);
    let t0 = now();
    let (s, retry, v) = post_raw(&e.t, &e.user, STUDY_ASK, hint_body(K1, "qh", 3)).await;
    assert_eq!(s, 429, "{v}");
    assert_eq!(v["code"], json!("ai.callLimit"));
    assert_eq!(v["cap"], json!("minute"));
    assert_eq!(v["limit"], json!(2));
    let retry_at = v["retryAt"].as_i64().expect("retryAt");
    let retry_after = v["retryAfter"].as_i64().expect("retryAfter");
    assert!((t0 + 1..=t0 + 61).contains(&retry_at), "{v}");
    assert!((1..=60).contains(&retry_after), "{v}");
    assert_eq!(retry, Some(retry_after.to_string()), "Retry-After header");
    assert_eq!(
        v["error"],
        json!("你的 AI 请求额度已用完（每分钟 2 次），请联系管理员设置 AI 请求额度上限。")
    );
    assert_eq!(row_counts(&e.t), before, "no chat, message or usage row");
    // the chat route refuses the same way and keeps nothing either
    let (s, _, v) = post_raw(&e.t, &e.user, CHAT, chat_body("Hello")).await;
    assert_eq!((s, &v["code"]), (429, &json!("ai.callLimit")));
    assert_eq!(row_counts(&e.t), before);
    // the status shows the numbers and the block
    let (_, st) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(st["calls"]["used"]["minute"], json!(2));
    assert_eq!(st["calls"]["used"]["hour"], json!(2));
    assert_eq!(st["calls"]["used"]["day"], json!(2));
    assert_eq!(
        st["calls"]["caps"],
        json!({"minute": 2, "hour": 3, "day": 4})
    );
    assert_eq!(
        st["calls"]["blocked"],
        json!({"cap": "minute", "limit": 2, "retryAt": retry_at})
    );
    // lowering blocks at once, raising frees at once
    set_caps(&e.t, &e.admin, 3, 3, 4).await;
    let (_, st) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(st["calls"]["blocked"], Value::Null);
    let ev = ask(&e.t, &e.user, hint_body(K1, "qh", 3)).await;
    assert_eq!(terminal_name(&ev), "done");
    let (s, _, v) = post_raw(&e.t, &e.user, STUDY_ASK, hint_body(K1, "q1", 4)).await;
    // minute and hour (3 each) are both full: the one that frees last is named
    assert_eq!((s, &v["cap"]), (429, &json!("hour")), "{v}");
    set_caps(&e.t, &e.admin, 3, 9, 9).await;
    let (_, st) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(st["calls"]["blocked"]["cap"], json!("minute"));
}

#[tokio::test]
async fn the_day_cap_resets_at_midnight_and_names_tomorrow() {
    clear_of_midnight().await;
    let e = env().await;
    set_caps(&e.t, &e.admin, 3, 3, 3).await;
    touch(&e.t, &e.user).await;
    let d = day();
    for _ in 0..3 {
        insert_call(&e.t, e.user.id, d.start, "chat", None);
    }
    let (s, retry, v) = post_raw(&e.t, &e.user, CHAT, chat_body("Hello")).await;
    assert_eq!(s, 429, "{v}");
    assert_eq!(v["cap"], json!("day"));
    assert_eq!(v["limit"], json!(3));
    assert_eq!(v["retryAt"], json!(d.next), "the next 0:00");
    assert!(
        v["error"].as_str().unwrap_or("").ends_with(
            "（每天 3 次），请联系管理员设置 AI 请求额度上限。明天 0 点后可以再次使用。"
        ),
        "{v}"
    );
    assert!(retry.is_some());
    assert_eq!(counted_rows(&e.t, e.user.id), 3, "nothing was written");
    // three calls that started yesterday (outside the hour window) do not count today
    e.t.db().execute("DELETE FROM usage", []).expect("clear");
    for _ in 0..3 {
        insert_call(&e.t, e.user.id, d.start - 7200, "chat", None);
    }
    let ev =
        e.t.send_all(&e.user, chat_body("Hello"))
            .await
            .expect("passes");
    assert_eq!(terminal(&ev).0, "done");
    let (_, st) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(st["calls"]["used"]["day"], json!(1));
    assert_eq!(st["calls"]["used"]["hour"], json!(1));
}

#[tokio::test]
async fn administrators_are_limited_too_and_free_mode_counts() {
    let e = env().await;
    set_caps(&e.t, &e.admin, 2, 5, 10).await;
    let ev = e.t.send_all(&e.admin, chat_body("One")).await.expect("one");
    assert_eq!(terminal(&ev).0, "done");
    let (s, v) =
        e.t.post("/api/ai/free", &e.admin, json!({"free": true}))
            .await;
    assert_eq!(s, 200, "{v}");
    let ev =
        e.t.send_all(&e.admin, chat_body("Two"))
            .await
            .expect("free");
    assert_eq!(ev[0].1["free"], json!(true));
    assert_eq!(terminal(&ev).0, "done");
    let (s, _, v) = post_raw(&e.t, &e.admin, CHAT, chat_body("Three")).await;
    assert_eq!(
        (s, &v["code"], &v["cap"]),
        (429, &json!("ai.callLimit"), &json!("minute"))
    );
    // the connection test and the model list are not counted and work while blocked
    let rows = counted_rows(&e.t, e.admin.id);
    let (s, v) = e.t.post("/api/ai/admin/verify", &e.admin, json!({})).await;
    assert_eq!((s, &v["ok"]), (200, &json!(true)), "{v}");
    let (s, v) = e.t.post("/api/ai/admin/models", &e.admin, json!({})).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(counted_rows(&e.t, e.admin.id), rows);
    let (_, st) = e.t.get("/api/ai/status", &e.admin).await;
    assert_eq!(st["calls"]["used"]["minute"], json!(2));
    assert_eq!(st["calls"]["used"]["day"], json!(2));
}

#[tokio::test]
async fn retries_and_failed_calls_count() {
    let e = env().await;
    set_caps(&e.t, &e.admin, 2, 5, 10).await;
    e.t.stub.force(Some((
        500,
        r#"{"error":{"message":"made-up failure","type":"server_error"}}"#,
    )));
    let ev =
        e.t.send_all(&e.user, chat_body("One"))
            .await
            .expect("stream");
    assert_eq!(terminal(&ev).0, "error");
    let chat = ev[0].1["chatId"].as_i64().expect("chat");
    e.t.stub.force(None);
    let ev =
        e.t.open_stream(
            "/api/ai/retry",
            &e.user,
            json!({"chatId": chat, "lang": "en"}),
        )
        .await
        .expect("retry")
        .all()
        .await;
    assert_eq!(terminal(&ev).0, "done");
    assert_eq!(
        counted_rows(&e.t, e.user.id),
        2,
        "the failed call and the retry"
    );
    let (s, _, v) = post_raw(&e.t, &e.user, CHAT, chat_body("Three")).await;
    assert_eq!((s, &v["cap"]), (429, &json!("minute")), "{v}");
}

#[tokio::test]
async fn a_usage_row_with_the_connect_code_counts() {
    let e = env().await;
    set_caps(&e.t, &e.admin, 1, 5, 10).await;
    touch(&e.t, &e.user).await;
    insert_call(&e.t, e.user.id, now() - 10, "chat", Some("connect"));
    let (s, _, v) = post_raw(&e.t, &e.user, CHAT, chat_body("Hello")).await;
    assert_eq!((s, &v["cap"]), (429, &json!("minute")), "{v}");
}

#[tokio::test]
async fn memory_suggestions_count_and_are_refused_by_the_cap() {
    let e = env().await;
    set_caps(&e.t, &e.admin, 2, 5, 10).await;
    let ev =
        e.t.send_all(&e.user, chat_body("I like short answers"))
            .await
            .expect("send");
    let msg = ev[0].1["userMessageId"].as_i64().expect("message");
    let (s, v) =
        e.t.post(SUGGEST, &e.user, json!({"messageId": msg, "lang": "en"}))
            .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(counted_rows(&e.t, e.user.id), 2);
    let kinds: i64 =
        e.t.db()
            .query_row(
                "SELECT COUNT(*) FROM usage WHERE kind = 'memory'",
                [],
                |r| r.get(0),
            )
            .expect("count");
    assert_eq!(kinds, 1);
    let (s, _, v) = post_raw(
        &e.t,
        &e.user,
        SUGGEST,
        json!({"messageId": msg, "lang": "en"}),
    )
    .await;
    assert_eq!((s, &v["code"]), (429, &json!("ai.callLimit")), "{v}");
    let (s, _, v) = post_raw(&e.t, &e.user, CHAT, chat_body("Next")).await;
    assert_eq!((s, &v["cap"]), (429, &json!("minute")), "{v}");
}

#[tokio::test]
async fn a_memory_suggestion_whose_client_disconnects_still_writes_its_row() {
    let e = env().await;
    let ev =
        e.t.send_all(&e.user, chat_body("Remember my made-up habit"))
            .await
            .expect("send");
    let msg = ev[0].1["userMessageId"].as_i64().expect("message");
    e.t.set_model(&e.admin, "demo-hold").await;
    let before = counted_rows(&e.t, e.user.id);
    let req = e.t.raw(
        Method::POST,
        SUGGEST,
        Some(e.t.ident(&e.user)),
        Some(
            json!({"messageId": msg, "lang": "en"})
                .to_string()
                .into_bytes(),
        ),
        Some("application/json"),
    );
    // the browser goes away while the provider is slow: the handler future is dropped
    let gone = tokio::time::timeout(std::time::Duration::from_millis(600), req).await;
    assert!(gone.is_err(), "the held answer is still running");
    // the call still runs: it counts as running, then as a row once the provider answers
    let (_, st) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(st["calls"]["used"]["minute"], json!(before + 1), "{st}");
    e.t.stub.release();
    let t = &e.t;
    let uid = e.user.id;
    eventually(|| counted_rows(t, uid) == before + 1).await;
    let (kind, status): (String, String) =
        e.t.db()
            .query_row(
                "SELECT kind, status FROM usage WHERE user_id = ?1 ORDER BY id DESC LIMIT 1",
                [e.user.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("row");
    assert_eq!((kind.as_str(), status.as_str()), ("memory", "ok"));
    let (_, st) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(
        st["calls"]["used"]["minute"],
        json!(before + 1),
        "counted once, as a row"
    );
}

#[tokio::test]
async fn running_calls_count_and_status_and_admin_numbers_agree() {
    let e = env().await;
    set_limit(&e.t, &e.admin, "perUserRunning", 2).await;
    let kim = e.t.user("kim", false, true);
    // two finished calls today, plus one of another user
    for text in ["One", "Two"] {
        let ev = e.t.send_all(&e.user, chat_body(text)).await.expect("send");
        assert_eq!(terminal(&ev).0, "done");
    }
    let ev = e.t.send_all(&kim, chat_body("Other")).await.expect("kim");
    assert_eq!(terminal(&ev).0, "done");
    // the administrator's own test and model list leave the numbers alone
    let (s, v) = e.t.post("/api/ai/admin/models", &e.admin, json!({})).await;
    assert_eq!(s, 200, "{v}");
    // a third call is running
    e.t.set_model(&e.admin, "demo-hold").await;
    let mut third =
        e.t.open_stream(CHAT, &e.user, chat_body("Three"))
            .await
            .expect("third");
    third.until("start").await;
    let (_, st) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(
        st["calls"]["used"],
        json!({"minute": 3, "hour": 3, "day": 3}),
        "{st}"
    );
    assert_eq!(
        st["calls"]["caps"],
        json!({"minute": 5, "hour": 20, "day": 100})
    );
    assert_eq!(st["calls"]["blocked"], Value::Null);
    // the administrators' list, the single row and the usage page say the same
    let (s, list) = e.t.get("/api/ai/admin/users", &e.admin).await;
    assert_eq!(s, 200, "{list}");
    let row = |name: &str| -> Value {
        list["users"]
            .as_array()
            .and_then(|a| a.iter().find(|r| r["username"] == json!(name)))
            .cloned()
            .unwrap_or(Value::Null)
    };
    assert_eq!(
        row("lee")["calls"]["used"],
        json!({"minute": 3, "hour": 3, "day": 3})
    );
    assert_eq!(row("lee")["calls"]["blocked"], Value::Null);
    assert!(row("lee")["calls"].get("caps").is_none());
    assert_eq!(row("kim")["calls"]["used"]["day"], json!(1));
    assert_eq!(
        row("root")["calls"]["used"]["day"],
        json!(0),
        "test / model list rows do not count"
    );
    let (s, one) =
        e.t.get(&format!("/api/ai/admin/user?id={}", e.user.id), &e.admin)
            .await;
    assert_eq!(s, 200, "{one}");
    assert_eq!(one["user"]["calls"]["used"]["day"], json!(3));
    let (s, usage) = e.t.get("/api/ai/admin/usage?days=7", &e.admin).await;
    assert_eq!(s, 200, "{usage}");
    let today = |name: &str| -> Value {
        usage["byUser"]
            .as_array()
            .and_then(|a| a.iter().find(|r| r["username"] == json!(name)))
            .map_or(Value::Null, |r| r["today"].clone())
    };
    assert_eq!(today("lee"), json!(3));
    assert_eq!(today("kim"), json!(1));
    assert_eq!(today("root"), json!(0));
    e.t.stub.release();
    let ev = third.all().await;
    assert_eq!(terminal(&ev).0, "done");
    let (_, st) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(
        st["calls"]["used"]["day"],
        json!(3),
        "a finished call is counted once"
    );
}

#[tokio::test]
async fn a_running_call_counts_against_the_cap() {
    let e = env().await;
    set_limit(&e.t, &e.admin, "perUserRunning", 3).await;
    set_caps(&e.t, &e.admin, 2, 5, 10).await;
    e.t.set_model(&e.admin, "demo-hold").await;
    let mut a =
        e.t.open_stream(CHAT, &e.user, chat_body("A"))
            .await
            .expect("a");
    a.until("start").await;
    let mut b =
        e.t.open_stream(CHAT, &e.user, chat_body("B"))
            .await
            .expect("b");
    b.until("start").await;
    let (s, _, v) = post_raw(&e.t, &e.user, CHAT, chat_body("C")).await;
    assert_eq!((s, &v["cap"]), (429, &json!("minute")), "{v}");
    let (_, st) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(st["calls"]["used"]["minute"], json!(2));
    assert_eq!(st["calls"]["blocked"]["cap"], json!("minute"));
    // let both held answers reach the stub before releasing them
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    e.t.stub.release();
    a.all().await;
    b.all().await;
}

#[tokio::test]
async fn calls_are_there_for_a_paused_user_and_absent_without_the_service() {
    let e = env().await;
    let (s, v) =
        e.t.post(
            "/api/ai/admin/suspend",
            &e.admin,
            json!({"userId": e.user.id}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let (_, st) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(st["allowed"], json!(false));
    assert_eq!(
        st["calls"]["used"],
        json!({"minute": 0, "hour": 0, "day": 0}),
        "{st}"
    );
    let nia = e.t.user("nia", false, false);
    let (s, st) = e.t.get("/api/ai/status", &nia).await;
    assert_eq!(s, 200, "{st}");
    assert!(st.get("calls").is_none(), "{st}");
}

#[tokio::test]
async fn one_log_line_per_episode_and_no_line_per_refusal() {
    let e = env().await;
    let user = e.t.user("caplogger", false, true);
    set_caps(&e.t, &e.admin, 1, 5, 10).await;
    let ev = e.t.send_all(&user, chat_body("One")).await.expect("one");
    assert_eq!(terminal(&ev).0, "done");
    for _ in 0..3 {
        let (s, _, v) = post_raw(&e.t, &user, CHAT, chat_body("More")).await;
        assert_eq!((s, &v["cap"]), (429, &json!("minute")), "{v}");
    }
    let line = "user \"caplogger\" reached the AI request cap (1 per minute)";
    assert_eq!(captured_count(line), 1);
}

#[tokio::test]
async fn the_order_rule_refuses_the_update_as_a_whole() {
    let e = env().await;
    let (s, v) =
        e.t.post(
            "/api/ai/admin/limits",
            &e.admin,
            json!({"callsPerMinute": 30}),
        )
        .await;
    assert_eq!(
        (s, &v["code"], &v["field"]),
        (400, &json!("ai.limitOrder"), &json!("callsPerHour")),
        "{v}"
    );
    assert_eq!(
        v["error"],
        json!("每小时的次数不能少于每分钟的次数，每天的次数不能少于每小时的次数。")
    );
    let (s, v) =
        e.t.post(
            "/api/ai/admin/limits",
            &e.admin,
            json!({"callsPerHour": 200}),
        )
        .await;
    assert_eq!((s, &v["field"]), (400, &json!("callsPerDay")), "{v}");
    let (_, got) = e.t.get("/api/ai/admin/limits", &e.admin).await;
    assert_eq!(
        got["limits"]["callsPerMinute"],
        json!(5),
        "nothing was saved"
    );
    assert_eq!(got["limits"]["callsPerHour"], json!(20));
    // a consistent update in one request passes, equal values included
    let (s, v) =
        e.t.post(
            "/api/ai/admin/limits",
            &e.admin,
            json!({"callsPerMinute": 30, "callsPerHour": 30, "callsPerDay": 30}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
}

#[tokio::test]
async fn of_two_simultaneous_asks_at_the_boundary_exactly_one_passes() {
    let e = env().await;
    set_limit(&e.t, &e.admin, "perUserRunning", 2).await;
    set_limit(&e.t, &e.admin, "callsPerMinute", 1).await;
    for i in 0..20 {
        let who = e.t.user(&format!("racer{i}"), false, true);
        let (a, b) = tokio::join!(
            e.t.open_stream(CHAT, &who, chat_body("Left")),
            e.t.open_stream(CHAT, &who, chat_body("Right")),
        );
        let (passed, refused): (Vec<_>, Vec<_>) = [a, b].into_iter().partition(Result::is_ok);
        assert_eq!((passed.len(), refused.len()), (1, 1), "round {i}");
        if let Some(Ok(mut s)) = passed.into_iter().next() {
            s.all().await;
        }
        let Some(Err((status, v))) = refused.into_iter().next() else {
            panic!("one refusal");
        };
        assert_eq!(
            (status, &v["cap"]),
            (429, &json!("minute")),
            "round {i}: {v}"
        );
    }
}

#[tokio::test]
async fn defaults_are_five_twenty_and_a_hundred() {
    let e = env().await;
    let (_, st) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(
        st["calls"]["caps"],
        json!({"minute": 5, "hour": 20, "day": 100}),
        "{st}"
    );
    let (_, l) = e.t.get("/api/ai/admin/limits", &e.admin).await;
    assert_eq!(l["limits"]["callsPerDay"], json!(100));
}
