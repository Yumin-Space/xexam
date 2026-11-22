//! The v2.5 AI study assessment: the stream, the stored mapping (`summary` + `assess@` ref = wire kind `assess`), the
//! capsule (latest assessment), the v2.6 per-day limit (own feature, `assessPerDay`, admins exempt), the refusals. All records and texts are made up.
use super::util::*;
use serde_json::{json, Value};

const ASK: &str = "/api/ai/assess/ask";

const LATEST: &str = "/api/ai/assess/latest";

/// Pretends the capsule was written on an earlier day, so the day is free again.
fn age_capsule(e: &Env) {
    e.t.db()
        .execute("UPDATE assess_latest SET day = '2000-01-01'", [])
        .expect("age");
}

/// Three records of the user (the minimum), the newest at 2026-11-21 10:30.
fn three_records(e: &Env) {
    for (id, kind, at) in [
        (1, "practice", "2026-11-19 08:00"),
        (2, "exam", "2026-11-20 19:15"),
        (3, "practice", "2026-11-21 10:30"),
    ] {
        e.t.add_record(record(
            &e.user,
            id,
            kind,
            None,
            at,
            vec![
                item("q1", json!(["B"]), true),
                item("q2", json!(["A"]), false),
            ],
        ));
    }
}

#[tokio::test]
async fn an_assessment_streams_and_lands_in_a_day_chat() {
    let e = env().await;
    three_records(&e);
    let ev = ask_to(&e.t, &e.user, ASK, json!({"lang": "en"})).await;
    assert_eq!(terminal_name(&ev), "done");
    let s = start_of(&ev);
    assert_eq!(s["chatKind"], "assess");
    assert_eq!(s["kind"], "assess");
    assert_eq!(s["recordId"], Value::Null);
    assert_eq!(s["sample"], Value::Null);
    assert_eq!(s["chatTitle"], "2026-11-22");
    assert_eq!(
        s["basis"],
        json!({"days": 30, "records": 3, "answered": 6, "latestAt": "2026-11-21 10:30"})
    );
    // stored as a summary chat with the day ref and no record; the user message is the machine line
    let (kind, rf, rec): (String, String, Option<i64>) =
        e.t.db()
            .query_row("SELECT kind, ref, record_id FROM chats", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .expect("chat");
    assert_eq!(
        (kind.as_str(), rf.as_str(), rec),
        ("summary", "assess@2026-11-22", None)
    );
    let line: String = q1(&e.t, "SELECT content FROM messages WHERE role = 'user'");
    assert_eq!(line, "[AI assessment] latest=2026-11-21 10:30 30d 3r 6q");
    let kinds: i64 = q1(&e.t, "SELECT COUNT(*) FROM messages WHERE kind = 'summary'");
    assert_eq!(kinds, 2);
    let purpose: Option<String> = q1(&e.t, "SELECT purpose FROM usage WHERE kind = 'chat'");
    assert_eq!(purpose.as_deref(), Some("summary"));
    // the prompt: numbers only, no question text
    let req = e.t.stub.last();
    let msgs = req.body["messages"].as_array().expect("messages");
    assert_eq!(msgs.len(), 2);
    let sys = msgs[0]["content"].as_str().expect("system");
    assert!(sys.contains("study coach inside Xexam"));
    let user = msgs[1]["content"].as_str().expect("user");
    assert!(user.starts_with("<stats>\n") && user.contains("answered: 6"));
    assert!(!user.contains("tool"));
    // the wire kind is `assess` in the list and in the chat
    let (_, list) = e.t.get("/api/ai/chats", &e.user).await;
    assert_eq!(list["chats"][0]["kind"], "assess");
    let (_, chat) =
        e.t.get(&format!("/api/ai/chat?id={}", s["chatId"]), &e.user)
            .await;
    assert_eq!(chat["chat"]["kind"], "assess");
    assert_eq!(chat["messages"][0]["kind"], "assess");
    assert_eq!(chat["messages"][1]["kind"], "assess");
    assert_eq!(chat["messages"][1]["recordId"], Value::Null);
    // the old saved route is gone; the capsule holds the answer
    let (st, _) =
        e.t.get("/api/ai/assess/saved?at=202611211030", &e.user)
            .await;
    assert_eq!(st, 404);
    let (st, v) = e.t.get(LATEST, &e.user).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["assess"]["text"], text(&ev));
    assert_eq!(v["assess"]["day"], v["today"]);
    assert_eq!(v["remaining"], 2);
    assert_eq!(v["assess"]["latestAt"], "2026-11-21 10:30");
    assert_eq!(v["assess"]["records"], 3);
    assert_eq!(v["assess"]["answered"], 6);
    // the record summary route never sees this chat
    let (_, rs) =
        e.t.get("/api/ai/summary/saved?record=3&at=202611211030", &e.user)
            .await;
    assert_eq!(rs["chatId"], Value::Null);
    // another user has nothing saved
    let other = e.t.user("kim", false, true);
    let (_, none) = e.t.get(LATEST, &other).await;
    assert_eq!(none["assess"], Value::Null);
    assert_eq!(
        (none["perDay"].clone(), none["remaining"].clone()),
        (json!(3), json!(3))
    );
}

fn admin_records(e: &Env) {
    for (id, kind, at) in [
        (11, "practice", "2026-11-19 08:00"),
        (12, "exam", "2026-11-20 19:15"),
        (13, "practice", "2026-11-21 10:30"),
    ] {
        e.t.add_record(record(
            &e.admin,
            id,
            kind,
            None,
            at,
            vec![item("q1", json!(["B"]), true)],
        ));
    }
}

async fn set_limits(e: &Env, body: Value) {
    let (st, v) = e.t.post("/api/ai/admin/limits", &e.admin, body).await;
    assert_eq!(st, 200, "{v}");
}

#[tokio::test]
async fn the_per_day_limit_counts_successful_runs_and_refuses_before_anything_is_taken() {
    let e = env().await;
    three_records(&e);
    // the default: 3 a day, each run counted
    let (_, v) = e.t.get(LATEST, &e.user).await;
    assert_eq!(
        (v["perDay"].clone(), v["remaining"].clone()),
        (json!(3), json!(3))
    );
    set_limits(&e, json!({"assessPerDay": 2})).await;
    for left in [1, 0] {
        let ev = ask_to(&e.t, &e.user, ASK, json!({})).await;
        assert_eq!(terminal_name(&ev), "done");
        let (_, v) = e.t.get(LATEST, &e.user).await;
        assert_eq!(
            (v["perDay"].clone(), v["remaining"].clone()),
            (json!(2), json!(left))
        );
    }
    let n: i64 = q1(&e.t, "SELECT runs FROM assess_latest");
    assert_eq!(n, 2);
    let calls = e.t.stub.count();
    let usage = e.t.usage_rows();
    let (st, v) = refused(&e.t, &e.user, ASK, json!({})).await;
    assert_eq!((st, &v["code"]), (409, &json!("ai.assessDaily")));
    assert_eq!(v["error"], "今天的 AI 学习评估次数已用完，明天再来吧。");
    assert_eq!((e.t.stub.count(), e.t.usage_rows()), (calls, usage));
    // the next day it works again: the count starts over, a new chat, the capsule is replaced
    age_capsule(&e);
    e.t.db()
        .execute("UPDATE chats SET ref = 'assess@2000-01-01'", [])
        .expect("old chats");
    let (_, v) = e.t.get(LATEST, &e.user).await;
    assert_eq!(v["remaining"], 2);
    let s3 = start_of(&ask_to(&e.t, &e.user, ASK, json!({"lang": "ja"})).await);
    assert_eq!(s3["newChat"], true);
    assert_eq!(s3["lang"], "ja");
    let (_, v) = e.t.get(LATEST, &e.user).await;
    assert_eq!(v["assess"]["day"], v["today"]);
    assert_eq!(v["remaining"], 1);
    let n: i64 = q1(&e.t, "SELECT runs FROM assess_latest");
    assert_eq!(n, 1);
    // 0 = no assessments at all
    set_limits(&e, json!({"assessPerDay": 0})).await;
    let (_, v) = e.t.get(LATEST, &e.user).await;
    assert_eq!(
        (v["perDay"].clone(), v["remaining"].clone()),
        (json!(0), json!(0))
    );
    let (st, v) = refused(&e.t, &e.user, ASK, json!({})).await;
    assert_eq!((st, &v["code"]), (403, &json!("ai.featureOff")));
}

#[tokio::test]
async fn administrators_are_exempt_unless_the_limit_says_otherwise() {
    let e = env().await;
    admin_records(&e);
    set_limits(&e, json!({"assessPerDay": 1})).await;
    // exempt (the default): no limit shown, any number of runs
    for _ in 0..3 {
        assert_eq!(
            terminal_name(&ask_to(&e.t, &e.admin, ASK, json!({})).await),
            "done"
        );
        let (_, v) = e.t.get(LATEST, &e.admin).await;
        assert_eq!(
            (v["perDay"].clone(), v["remaining"].clone()),
            (Value::Null, Value::Null)
        );
    }
    // not exempt: the same limit as everyone
    set_limits(&e, json!({"assessAdminsExempt": 0})).await;
    let (_, v) = e.t.get(LATEST, &e.admin).await;
    assert_eq!(
        (v["perDay"].clone(), v["remaining"].clone()),
        (json!(1), json!(0))
    );
    let (st, v) = refused(&e.t, &e.admin, ASK, json!({})).await;
    assert_eq!((st, &v["code"]), (409, &json!("ai.assessDaily")));
    age_capsule(&e);
    assert_eq!(
        terminal_name(&ask_to(&e.t, &e.admin, ASK, json!({})).await),
        "done"
    );
    let (st, _) = refused(&e.t, &e.admin, ASK, json!({})).await;
    assert_eq!(st, 409);
}

#[tokio::test]
async fn the_assessment_is_its_own_feature() {
    let e = env().await;
    three_records(&e);
    // the global switch
    let (st, v) =
        e.t.post(
            "/api/ai/admin/feature-switches",
            &e.admin,
            json!({"assess": false}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let (st, v) = refused(&e.t, &e.user, ASK, json!({})).await;
    assert_eq!((st, &v["code"]), (403, &json!("ai.featureOff")));
    assert_eq!(v["feature"], "assess");
    // summary stays on: the record summary is not affected
    let (st, _) =
        e.t.post(
            "/api/ai/admin/feature-switches",
            &e.admin,
            json!({"assess": true, "summary": false}),
        )
        .await;
    assert_eq!(st, 200);
    assert_eq!(
        terminal_name(&ask_to(&e.t, &e.user, ASK, json!({})).await),
        "done"
    );
}

#[tokio::test]
async fn assessment_calls_are_reported_as_their_own_usage_feature() {
    let e = env().await;
    three_records(&e);
    assert_eq!(
        terminal_name(&ask_to(&e.t, &e.user, ASK, json!({})).await),
        "done"
    );
    // stored with purpose summary (the CHECK), reported as assess
    let p: String = q1(&e.t, "SELECT purpose FROM usage ORDER BY id DESC LIMIT 1");
    assert_eq!(p, "summary");
    let m: i64 = q1(&e.t, "SELECT assess FROM usage ORDER BY id DESC LIMIT 1");
    assert_eq!(m, 1);
    let (st, v) = e.t.get("/api/ai/admin/usage?days=7", &e.admin).await;
    assert_eq!(st, 200, "{v}");
    let rows = v["byPurpose"].as_array().expect("rows");
    let calls = |name: &str| {
        rows.iter()
            .find(|r| r["purpose"] == name)
            .map(|r| r["calls"].clone())
    };
    assert_eq!(calls("assess"), Some(json!(1)));
    assert_eq!(calls("summary"), Some(json!(0)));
}

#[tokio::test]
async fn a_failed_assessment_keeps_the_old_text_and_does_not_use_up_the_day() {
    let e = env().await;
    three_records(&e);
    let boom = r#"{"error":{"type":"api_error","message":"internal detail"}}"#;
    e.t.stub.force(Some((500, boom)));
    let ev = ask_to(&e.t, &e.user, ASK, json!({})).await;
    assert_eq!(terminal_name(&ev), "error");
    e.t.stub.force(None);
    let (_, v) = e.t.get(LATEST, &e.user).await;
    assert_eq!(v["assess"], Value::Null);
    assert_eq!(v["remaining"], 3);
    // so a retry the same day works, and then the day is used
    assert_eq!(
        terminal_name(&ask_to(&e.t, &e.user, ASK, json!({})).await),
        "done"
    );
    let (_, v) = e.t.get(LATEST, &e.user).await;
    let kept = v["assess"]["text"].clone();
    assert!(kept.as_str().is_some_and(|t| !t.is_empty()));
    // a later failure (on a free day) leaves that text in place
    age_capsule(&e);
    e.t.stub.force(Some((500, boom)));
    assert_eq!(
        terminal_name(&ask_to(&e.t, &e.user, ASK, json!({})).await),
        "error"
    );
    e.t.stub.force(None);
    let (_, v) = e.t.get(LATEST, &e.user).await;
    assert_eq!(v["assess"]["text"], kept);
    assert_eq!(v["assess"]["day"], "2000-01-01");
    assert_eq!(v["remaining"], 3);
}

#[tokio::test]
async fn the_capsule_outlives_the_chat_and_needs_no_ai_plus_to_read() {
    let e = env().await;
    three_records(&e);
    let ev = ask_to(&e.t, &e.user, ASK, json!({})).await;
    let chat = start_of(&ev)["chatId"].as_i64().expect("id");
    let (st, v) =
        e.t.post("/api/ai/chats/delete", &e.user, json!({"id": chat}))
            .await;
    assert_eq!(st, 200, "{v}");
    let (st, v) =
        e.t.post("/api/ai/chats/delete", &e.user, json!({"all": true}))
            .await;
    assert_eq!(st, 200, "{v}");
    let n: i64 = q1(&e.t, "SELECT COUNT(*) FROM chats");
    assert_eq!(n, 0);
    // AI+ switched off: the text still reads, only asking is refused
    e.t.set_ai(&e.user, false);
    let (st, v) = e.t.get(LATEST, &e.user).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["assess"]["text"], text(&ev));
    let (st, v) = refused(&e.t, &e.user, ASK, json!({})).await;
    assert_eq!((st, &v["code"]), (403, &json!("ai.notAllowed")));
}

#[tokio::test]
async fn latest_has_the_exact_shape_and_only_admins_see_the_model() {
    let e = env().await;
    three_records(&e);
    // nothing yet
    let (st, v) = e.t.get(LATEST, &e.user).await;
    assert_eq!(st, 200);
    assert_eq!(v["assess"], Value::Null);
    let mut top: Vec<&String> = v.as_object().expect("object").keys().collect();
    top.sort();
    assert_eq!(top, ["assess", "perDay", "remaining", "running", "today"]);
    assert_eq!(v["today"].as_str().map(str::len), Some(10));
    let ev = ask_to(&e.t, &e.user, ASK, json!({})).await;
    let (_, v) = e.t.get(LATEST, &e.user).await;
    let mut keys: Vec<&String> = v["assess"].as_object().expect("object").keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "answered",
            "createdAt",
            "day",
            "latestAt",
            "records",
            "text"
        ]
    );
    assert!(v["assess"]["createdAt"].as_i64().is_some_and(|n| n > 0));
    assert_eq!(v["assess"]["text"], text(&ev));
    assert!(!v.to_string().contains("\"model\""), "{v}");
    // an administrator reads the model of the assessment
    admin_records(&e);
    ask_to(&e.t, &e.admin, ASK, json!({})).await;
    let (_, v) = e.t.get(LATEST, &e.admin).await;
    assert_eq!(v["assess"]["model"], "demo-chat-small");
    // reading is a GET only
    let (st, _) = e.t.post(LATEST, &e.user, json!({})).await;
    assert!(st == 404 || st == 405, "{st}");
}

#[tokio::test]
async fn too_few_records_and_other_refusals() {
    let e = env().await;
    e.t.add_record(record(
        &e.user,
        1,
        "practice",
        None,
        "2026-11-19 08:00",
        vec![item("q1", json!(["B"]), true)],
    ));
    let (s, v) = refused(&e.t, &e.user, ASK, json!({})).await;
    assert_eq!((s, &v["code"]), (409, &json!("ai.assessFew")));
    assert_eq!(v["error"], "学习记录太少，至少需要 3 条。");
    let n: i64 = q1(&e.t, "SELECT COUNT(*) FROM chats");
    assert_eq!(n, 0);
    // bad bodies
    for b in [
        json!({"x": 1}),
        json!({"lang": "fr"}),
        json!({"lang": 1}),
        json!({"recordId": 1}),
    ] {
        let (s, v) = refused(&e.t, &e.user, ASK, b.clone()).await;
        assert_eq!((s, &v["code"]), (400, &json!("ai.badRequest")), "{b}");
    }
    // a user without AI+
    let plain = e.t.user("ann", false, false);
    let (s, v) = refused(&e.t, &plain, ASK, json!({})).await;
    assert_eq!((s, &v["code"]), (403, &json!("ai.notAllowed")));
}

#[tokio::test]
async fn an_unreadable_backend_answer_fails_only_this_request() {
    let e = env().await;
    three_records(&e);
    e.t.backend.st().study_switch = StudySwitch::ExtraField;
    let (s, v) = refused(&e.t, &e.user, ASK, json!({})).await;
    assert_eq!(s, 503, "{v}");
    e.t.backend.st().study_switch = StudySwitch::Normal;
    assert_eq!(
        terminal_name(&ask_to(&e.t, &e.user, ASK, json!({})).await),
        "done"
    );
}

#[tokio::test]
async fn an_ordinary_user_never_sees_the_model() {
    let e = env().await;
    three_records(&e);
    let ev = ask_to(&e.t, &e.user, ASK, json!({})).await;
    assert!(start_of(&ev).get("model").is_none());
    let (_, v) = e.t.get(LATEST, &e.user).await;
    assert!(!v.to_string().contains("\"model\""), "{v}");
}

#[tokio::test]
async fn latest_says_running_while_today_assessment_is_still_written() {
    let e = env().await;
    three_records(&e);
    // the stub holds an assessment whose newest record is at 23:07, so the answer waits after its text
    e.t.add_record(record(
        &e.user,
        4,
        "practice",
        None,
        "2026-11-21 23:07",
        vec![item("q1", json!(["B"]), true)],
    ));
    let (_, before) = e.t.get(LATEST, &e.user).await;
    assert_eq!(before["running"], false);
    let mut stream =
        e.t.open_stream(ASK, &e.user, json!({}))
            .await
            .expect("stream");
    let (_, during) = e.t.get(LATEST, &e.user).await;
    assert_eq!(during["running"], true, "{during}");
    assert_eq!(during["remaining"], 3);
    // another user is not running
    let other = e.t.user("kim", false, true);
    let (_, theirs) = e.t.get(LATEST, &other).await;
    assert_eq!(theirs["running"], false);
    e.t.stub.release();
    assert_eq!(terminal_name(&stream.all().await), "done");
    settle(&e.t).await;
    let (_, after) = e.t.get(LATEST, &e.user).await;
    assert_eq!(after["running"], false);
}
