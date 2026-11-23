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
    let day = s["chatTitle"].as_str().expect("title").to_string();
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
        ("summary", format!("assess@{day}").as_str(), None)
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
    assert!(sys.contains("ONE paragraph of 3 or 4 short sentences"));
    assert!(
        user.starts_with("<facts>\n")
            && user.contains("- Questions answered in those sessions: 6, correct 3 (50%)")
    );
    assert!(!user.contains("tool") && !user.contains("Regenerate"));
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
    // the whole answer is asked for after the `start` frame: let the request reach the stub before it is released
    for _ in 0..200 {
        if e.t.stub.count() > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    e.t.stub.release();
    let all = stream.all().await;
    assert_eq!(terminal_name(&all), "done", "{all:?}");
    settle(&e.t).await;
    let (_, after) = e.t.get(LATEST, &e.user).await;
    assert_eq!(after["running"], false);
}

// —— v2.7: the whole answer is checked, retried and cleaned before it is shown ———————————————————————————————————————

const GOOD_EN: &str = "A small step taken each day adds up like drops filling a quiet pond. You have answered a good number of questions lately and most were right. Next week, spend a short session on the ones you missed. Go at your own pace and be kind to yourself.";
const COUNTING_EN: &str = "A(1) small(2) step(3) each(4) day(5) adds(6) up(7) like(8) drops(9) in(10) a(11) pond(12). Total length: 144 characters. Verify Constraints.";

/// Answers the assessment prompts with `replies` in order (the last one repeats); everything else falls through.
fn assess_script(
    e: &Env,
    replies: Vec<genkit::Reply>,
) -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = calls.clone();
    let replies = std::sync::Mutex::new(replies);
    e.t.stub.st().script = Some(std::sync::Arc::new(move |sys: &str, _user: &str| {
        if !sys.contains("study coach inside Xexam") {
            return genkit::Reply::Fall;
        }
        let n = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let r = replies.lock().unwrap_or_else(|x| x.into_inner());
        match r.get(n).or_else(|| r.last()) {
            Some(genkit::Reply::Text(t)) => genkit::Reply::Text(t.clone()),
            Some(genkit::Reply::Cut(t)) => genkit::Reply::Cut(t.clone()),
            Some(genkit::Reply::Status(s, b)) => genkit::Reply::Status(*s, b.clone()),
            _ => genkit::Reply::Fall,
        }
    }));
    calls
}

#[tokio::test]
async fn a_counting_reply_is_asked_again_with_the_retry_note_and_never_shown() {
    let e = env().await;
    three_records(&e);
    let calls = assess_script(
        &e,
        vec![
            genkit::Reply::Text(COUNTING_EN.into()),
            genkit::Reply::Text(format!("**{GOOD_EN}**")),
        ],
    );
    let ev = ask_to(&e.t, &e.user, ASK, json!({"lang": "en"})).await;
    assert_eq!(terminal_name(&ev), "done", "{ev:?}");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    // ONE delta with the cleaned text
    let deltas = ev.iter().filter(|(n, _)| n == "delta").count();
    assert_eq!(deltas, 1);
    assert_eq!(text(&ev), GOOD_EN);
    let reqs = e.t.stub.requests();
    assert_eq!(reqs.len(), 2);
    let sys = |i: usize| {
        reqs[i].body["messages"][0]["content"]
            .as_str()
            .unwrap_or("")
            .to_string()
    };
    assert!(!sys(0).contains("previous reply broke the rules"));
    assert!(sys(1).ends_with("Reply with only the final 3-4 sentences."));
    // the request is ONE counted usage row; the retry is a gen_calls row (still in the statistics, outside the caps)
    let rows: i64 = q1(&e.t, "SELECT COUNT(*) FROM usage WHERE assess = 1");
    assert_eq!(rows, 1);
    let retries: i64 = q1(
        &e.t,
        "SELECT COUNT(*) FROM gen_calls WHERE purpose = 'assess'",
    );
    assert_eq!(retries, 1);
    let (_, v) = e.t.get(LATEST, &e.user).await;
    assert_eq!(v["assess"]["text"], GOOD_EN);
    assert_eq!(v["remaining"], 2);
}

#[tokio::test]
async fn three_unusable_replies_end_with_the_unusable_code_and_save_nothing() {
    let e = env().await;
    three_records(&e);
    let calls = assess_script(&e, vec![genkit::Reply::Text(COUNTING_EN.into())]);
    let ev = ask_to(&e.t, &e.user, ASK, json!({"lang": "en"})).await;
    assert_eq!(terminal_name(&ev), "error", "{ev:?}");
    let err = ev.iter().find(|(n, _)| n == "error").expect("error frame");
    assert_eq!(err.1["code"], "ai.assess.unusable");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    let (st, status): (String, String) = (
        q1(&e.t, "SELECT status FROM messages WHERE role = 'assistant'"),
        q1(
            &e.t,
            "SELECT error_code FROM messages WHERE role = 'assistant'",
        ),
    );
    assert_eq!(
        (st.as_str(), status.as_str()),
        ("error", "ai.assess.unusable")
    );
    let saved: i64 = q1(&e.t, "SELECT COUNT(*) FROM assess_latest");
    assert_eq!(saved, 0);
    // 3 attempts: the caps see ONE call, the two retries are gen_calls rows
    let rows: i64 = q1(&e.t, "SELECT COUNT(*) FROM usage WHERE assess = 1");
    assert_eq!(rows, 1);
    let retries: i64 = q1(
        &e.t,
        "SELECT COUNT(*) FROM gen_calls WHERE purpose = 'assess'",
    );
    assert_eq!(retries, 2);
    let now = xexam_ai_service::util::unix_now();
    let starts = {
        let c = e.t.db();
        xexam_ai_service::store::stats::recent_starts(&c, e.user.id, now, now - 86_400)
            .expect("starts")
    };
    assert_eq!(starts.len(), 1, "{starts:?}");
    let (_, v) = e.t.get(LATEST, &e.user).await;
    assert_eq!(
        (v["assess"].clone(), v["remaining"].clone()),
        (Value::Null, json!(3))
    );
}

#[tokio::test]
async fn a_cut_reply_is_never_kept_even_when_it_ends_with_a_full_stop() {
    let e = env().await;
    three_records(&e);
    let calls = assess_script(
        &e,
        vec![
            genkit::Reply::Cut(GOOD_EN.into()),
            genkit::Reply::Text(GOOD_EN.into()),
        ],
    );
    let ev = ask_to(&e.t, &e.user, ASK, json!({"lang": "en"})).await;
    assert_eq!(terminal_name(&ev), "done", "{ev:?}");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    let cut = ev
        .iter()
        .find(|(n, _)| n == "done")
        .map(|(_, d)| d["truncated"].clone());
    assert_eq!(cut, Some(json!(false)));
}

#[tokio::test]
async fn the_second_run_of_a_day_carries_no_regenerate_note() {
    let e = env().await;
    three_records(&e);
    for _ in 0..2 {
        let ev = ask_to(&e.t, &e.user, ASK, json!({"lang": "en"})).await;
        assert_eq!(terminal_name(&ev), "done", "{ev:?}");
    }
    let reqs = e.t.stub.requests();
    assert_eq!(reqs.len(), 2);
    let user = reqs[1].body["messages"][1]["content"]
        .as_str()
        .unwrap_or("");
    assert!(
        user.starts_with("<facts>\n") && !user.contains("Regenerate"),
        "{user}"
    );
}

#[tokio::test]
async fn a_saved_capsule_that_fails_the_checks_reads_as_no_assessment() {
    let e = env().await;
    three_records(&e);
    assert_eq!(
        terminal_name(&ask_to(&e.t, &e.user, ASK, json!({})).await),
        "done"
    );
    // an old garbled text (per-character numbering) is hidden; nothing is deleted
    e.t.db()
        .execute(
            "UPDATE assess_latest SET content = '涓(1)涓(2)细(3)流(4)不(5)舍(6)昼(7)夜(8)地(9)雕(10)刻(11)'",
            [],
        )
        .expect("garble");
    let (st, v) = e.t.get(LATEST, &e.user).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["assess"], Value::Null);
    let kept: i64 = q1(&e.t, "SELECT COUNT(*) FROM assess_latest");
    assert_eq!(kept, 1);
}

#[tokio::test]
async fn a_hidden_capsule_of_today_does_not_use_up_the_day() {
    let e = env().await;
    three_records(&e);
    set_limits(&e, json!({"assessPerDay": 1})).await;
    assert_eq!(
        terminal_name(&ask_to(&e.t, &e.user, ASK, json!({})).await),
        "done"
    );
    // the one run of the day is now a garbled capsule (as an older version could have saved)
    e.t.db()
        .execute(
            "UPDATE assess_latest SET content = '涓(1)涓(2)细(3)流(4)不(5)舍(6)昼(7)夜(8)地(9)雕(10)刻(11)'",
            [],
        )
        .expect("garble");
    let (_, v) = e.t.get(LATEST, &e.user).await;
    assert_eq!(
        (v["assess"].clone(), v["remaining"].clone()),
        (Value::Null, json!(1))
    );
    // a new run is allowed, replaces the text, and then the day is used up
    assert_eq!(
        terminal_name(&ask_to(&e.t, &e.user, ASK, json!({})).await),
        "done"
    );
    let (_, v) = e.t.get(LATEST, &e.user).await;
    assert_ne!(v["assess"], Value::Null);
    assert_eq!(v["remaining"], 0);
}
