//! Hints, explanations: the stream, what is stored, the guard on the wire, the persistence rule.
use super::util::*;
use serde_json::{json, Value};
use std::time::Duration;
use xexam_ai_service::util::local_minute;

/// role, kind, question id, source, question number, lang, status, content
type PairRow = (String, String, String, String, i64, String, String, String);

fn assistant_content(t: &TestApp, id: i64) -> String {
    t.db()
        .query_row("SELECT content FROM messages WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .expect("content")
}

#[tokio::test]
async fn a_hint_streams_and_is_saved_with_its_place() {
    let e = env().await;
    let started = now() - 3600;
    let ev = ask(&e.t, &e.user, hint_body(K1, "q1", 3)).await;
    let names = names(&ev);
    assert_eq!(names.first().map(String::as_str), Some("start"));
    assert_eq!(names.last().map(String::as_str), Some("done"));
    assert_eq!(text(&ev), CLEAN_HINT);
    let s = start_of(&ev);
    assert_eq!(s["chatKind"], "practice");
    assert_eq!(s["kind"], "hint");
    assert_eq!(s["questionId"], "q1");
    assert_eq!(s["questionNo"], 3);
    assert_eq!(s["source"], "practice");
    assert_eq!(s["recordId"], Value::Null);
    assert_eq!(s["sample"], Value::Null);
    assert_eq!(s["lang"], "en");
    assert_eq!(s["newChat"], true);
    assert_eq!(s["retry"], false);
    assert_eq!(s["free"], false);
    // the stored title holds only the time (the suffix comes from the frontend catalog)
    assert_eq!(s["chatTitle"], local_minute(started));
    let keys: Vec<&str> = s
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "chatId",
            "chatTitle",
            "userMessageId",
            "assistantMessageId",
            "model",
            "newChat",
            "retry",
            "free",
            "chatKind",
            "kind",
            "questionId",
            "questionNo",
            "source",
            "recordId",
            "sample",
            "lang"
        ]
    );
    let chat = s["chatId"].as_i64().expect("chat id");
    let (kind, rf, title, rec): (String, String, String, Option<i64>) =
        e.t.db()
            .query_row(
                "SELECT kind, ref, title, record_id FROM chats WHERE id = ?1",
                [chat],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .expect("chat row");
    assert_eq!((kind.as_str(), rf.as_str(), rec), ("practice", K1, None));
    assert_eq!(title, local_minute(started));
    // the pair
    let rows: Vec<PairRow> = e
        .t
        .db()
        .prepare(
            "SELECT role, kind, question_id, source, question_no, lang, status, content FROM messages
             WHERE chat_id = ?1 ORDER BY id",
        )
        .expect("prepare")
        .query_map([chat], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        })
        .expect("query")
        .map(|x| x.expect("row"))
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "user");
    assert_eq!(rows[1].0, "assistant");
    for r in &rows {
        assert_eq!(
            (r.1.as_str(), r.2.as_str(), r.3.as_str(), r.4, r.5.as_str()),
            ("hint", "q1", "practice", 3, "en")
        );
    }
    assert_eq!(rows[1].6, "done");
    assert_eq!(rows[1].7, CLEAN_HINT);
    // the stored user message is the question without tags and without any solution
    assert!(rows[0].7.starts_with("Which tool fits the made-up job?"));
    assert!(!rows[0].7.contains("<question") && !rows[0].7.contains("Bravo is the made-up"));
    // one usage row: kind chat, purpose hint
    let (kind, purpose, status): (String, Option<String>, String) =
        e.t.db()
            .query_row(
                "SELECT kind, purpose, status FROM usage WHERE kind = 'chat'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("usage");
    assert_eq!(
        (kind.as_str(), purpose.as_deref(), status.as_str()),
        ("chat", Some("hint"), "ok")
    );
    // exactly one system prompt and one user turn went upstream; the language line closes the system prompt
    let req = e.t.stub.last();
    let msgs = req.body["messages"].as_array().expect("messages");
    assert_eq!(msgs.len(), 2);
    let sys = msgs[0]["content"].as_str().expect("system");
    assert!(sys
        .ends_with("Always reply in English (the language chosen in the user's Xexam settings)."));
    assert!(!sys.contains("Today is"));
    let user = msgs[1]["content"].as_str().expect("user");
    assert!(user.starts_with("<question number=\"3\""));
    assert!(!user.contains("answer_key") && !user.contains("Bravo is the made-up"));
    // AI Chat lists it with its kind
    let (s, v) = e.t.get("/api/ai/chats", &e.user).await;
    assert_eq!(s, 200);
    assert_eq!(v["chats"][0]["kind"], "practice");
    assert_eq!(v["chats"][0]["recordId"], Value::Null);
}

#[tokio::test]
async fn a_second_ask_of_a_session_lands_in_the_same_chat() {
    let e = env().await;
    let a = start_of(&ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await);
    let b = start_of(&ask(&e.t, &e.user, explain_body(K1, "q2", 2)).await);
    assert_eq!(a["chatId"], b["chatId"]);
    assert_eq!(b["newChat"], false);
    let n: i64 = q1(&e.t, "SELECT COUNT(*) FROM chats");
    assert_eq!(n, 1);
    let m: i64 = q1(&e.t, "SELECT COUNT(*) FROM messages");
    assert_eq!(m, 4);
    // another session is another chat
    let c = start_of(&ask(&e.t, &e.user, hint_body(K2, "q1", 1)).await);
    assert_ne!(a["chatId"], c["chatId"]);
}

#[tokio::test]
async fn an_explanation_carries_the_key_and_is_stored_with_its_purpose() {
    let e = env().await;
    let ev = ask(&e.t, &e.user, explain_body(K1, "q1", 4)).await;
    assert_eq!(terminal_name(&ev), "done");
    assert!(text(&ev).starts_with("**Answer: B**"));
    assert_eq!(start_of(&ev)["kind"], "explain");
    let req = e.t.stub.last();
    let user = req.body["messages"][1]["content"].as_str().expect("user");
    assert!(user.contains("<answer_key>B</answer_key>"));
    assert!(user
        .contains("<bank_explanation>Bravo is the made-up tool for this job.</bank_explanation>"));
    // the practice page does not discuss the user's answer
    let sys = req.body["messages"][0]["content"].as_str().expect("system");
    assert!(!sys.contains("The user answered") && !sys.contains("did not answer"));
    let purpose: Option<String> = q1(&e.t, "SELECT purpose FROM usage WHERE kind = 'chat'");
    assert_eq!(purpose.as_deref(), Some("explain"));
}

#[tokio::test]
async fn a_leaking_hint_is_withheld_and_nothing_of_it_is_kept() {
    let e = env().await;
    let leaky = e.t.user("leaky-user-1", false, true);
    let ev = ask(&e.t, &leaky, hint_body(K1, "q-leak", 1)).await;
    let (name, data) = terminal(&ev);
    assert_eq!(name, "error");
    assert_eq!(data["code"], "ai.hintLeak");
    assert_eq!(data["withdraw"], true);
    assert_eq!(data["error"], "这个提示透露了答案，已被隐藏。请重新生成。");
    // no delta ever carried the verdict
    let shown = text(&ev);
    assert!(
        !shown.contains("The answer is B") && !shown.contains("answer is"),
        "{shown:?}"
    );
    let chat = start_of(&ev)["chatId"].as_i64().expect("chat");
    let (content, status, code): (String, String, Option<String>) = e
        .t
        .db()
        .query_row(
            "SELECT content, status, error_code FROM messages WHERE chat_id = ?1 AND role = 'assistant'",
            [chat],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("assistant");
    assert_eq!(
        (content.as_str(), status.as_str(), code.as_deref()),
        ("", "error", Some("ai.hintLeak"))
    );
    let (status, code, purpose): (String, Option<String>, Option<String>) =
        e.t.db()
            .query_row(
                "SELECT status, error_code, purpose FROM usage WHERE user_id = ?1",
                [leaky.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("usage");
    assert_eq!(
        (status.as_str(), code.as_deref(), purpose.as_deref()),
        ("error", Some("hintLeak"), Some("hint"))
    );
    // one log line per minute, then counted; no text in it
    let line = "AI hint withheld because it gave the answer away (user \"leaky-user-1\")";
    assert_eq!(captured_count(line), 1);
    let _ = ask(&e.t, &leaky, hint_body(K1, "q-leak", 1)).await;
    assert_eq!(captured_count(line), 1, "the second one is only counted");
    // the saved lookup shows the withheld answer as an error (the browser maps the code)
    let (s, v) = e
        .t
        .get(
            &format!("/api/ai/study/saved?mode=practice&key={K1}&source=practice&question=q-leak&kind=hint"),
            &leaky,
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["latest"]["status"], "error");
    assert_eq!(v["latest"]["error"], "ai.hintLeak");
    assert_eq!(v["latest"]["content"], "");
    assert_eq!(v["lastGood"], Value::Null);
}

#[tokio::test]
async fn a_think_tag_after_the_text_withholds_a_hint_but_not_an_explanation() {
    let e = env().await;
    let ev = ask(&e.t, &e.user, hint_body(K1, "q-think-late", 1)).await;
    let (name, data) = terminal(&ev);
    assert_eq!(name, "error");
    assert_eq!(data["code"], "ai.hintLeak");
    assert_eq!(data["withdraw"], true);
    assert!(!text(&ev).contains("<think>"), "no tag was shown");
    let chat = start_of(&ev)["chatId"].as_i64().expect("chat");
    let (content, status): (String, String) =
        e.t.db()
            .query_row(
                "SELECT content, status FROM messages WHERE chat_id = ?1 AND role = 'assistant'",
                [chat],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("assistant");
    assert_eq!((content.as_str(), status.as_str()), ("", "error"));
    // explanations are not checked
    let ev = ask(&e.t, &e.user, explain_body(K1, "q-think-late", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    assert!(text(&ev).contains("<think>"), "{:?}", text(&ev));
}

#[tokio::test]
async fn a_leak_on_a_statements_question_is_withheld_too() {
    let e = env().await;
    let ev = ask(&e.t, &e.user, hint_body(K1, "qh-leak", 2)).await;
    let (name, data) = terminal(&ev);
    assert_eq!(name, "error");
    assert_eq!(data["code"], "ai.hintLeak");
    assert!(!text(&ev).contains("Statement 2 is true"));
}

#[tokio::test]
async fn a_held_back_leak_followed_by_an_upstream_error_is_still_a_leak() {
    let e = env().await;
    let ev = ask(&e.t, &e.user, hint_body(K1, "q-leakerr", 1)).await;
    let (name, data) = terminal(&ev);
    assert_eq!(
        (name.as_str(), &data["code"]),
        ("error", &json!("ai.hintLeak"))
    );
    assert_eq!(data["withdraw"], true);
    let content: String = q1(
        &e.t,
        "SELECT content FROM messages WHERE role = 'assistant'",
    );
    assert_eq!(content, "");
}

#[tokio::test]
async fn a_clean_hint_cut_by_an_upstream_error_keeps_everything_released() {
    let e = env().await;
    let ev = ask(&e.t, &e.user, hint_body(K1, "q-err", 1)).await;
    let (name, data) = terminal(&ev);
    assert_eq!(name, "error");
    assert_eq!(data["code"], "ai.up.server");
    assert!(data.get("withdraw").is_none());
    // the held-back tail is released by the final check, stored and shown
    assert_eq!(text(&ev), CLEAN_HINT);
    let content: String = q1(
        &e.t,
        "SELECT content FROM messages WHERE role = 'assistant'",
    );
    assert_eq!(content, CLEAN_HINT);
}

#[tokio::test]
async fn stop_with_a_held_back_leak_gives_hint_leak_and_nothing_unchecked_is_ever_stored() {
    let e = env().await;
    let mut s =
        e.t.open_stream(STUDY_ASK, &e.user, hint_body(K1, "q-leakslow", 1))
            .await
            .expect("stream");
    let (_, start) = s.until("start").await;
    let msg = start["assistantMessageId"].as_i64().expect("message");
    let chat = start["chatId"].as_i64().expect("chat");
    // the stub is holding after the leaking sentence: several 100-ms flushes pass; the stored text is only released text
    for _ in 0..8 {
        tokio::time::sleep(Duration::from_millis(120)).await;
        let c = assistant_content(&e.t, msg);
        assert!(!c.contains("answer is"), "{c:?}");
        assert!(!c.contains("The answer"), "{c:?}");
    }
    let (st, v) =
        e.t.post("/api/ai/stop", &e.user, json!({"chatId": chat}))
            .await;
    assert_eq!(st, 200, "{v}");
    let rest = s.all().await;
    let (name, data) = terminal(&rest);
    assert_eq!(
        (name.as_str(), &data["code"]),
        ("error", &json!("ai.hintLeak"))
    );
    assert_eq!(assistant_content(&e.t, msg), "");
    e.t.stub.release();
}

#[tokio::test]
async fn the_total_deadline_with_a_held_back_leak_is_a_leak_too() {
    let mut timeouts = fast_timeouts();
    timeouts.total = Duration::from_millis(1500);
    let e = env_with(Opts {
        timeouts,
        ..Opts::default()
    })
    .await;
    let ev = ask(&e.t, &e.user, hint_body(K1, "q-leakslow", 1)).await;
    let (name, data) = terminal(&ev);
    assert_eq!(
        (name.as_str(), &data["code"]),
        ("error", &json!("ai.hintLeak"))
    );
    let content: String = q1(
        &e.t,
        "SELECT content FROM messages WHERE role = 'assistant'",
    );
    assert_eq!(content, "");
    e.t.stub.release();
}

#[tokio::test]
async fn a_hint_over_the_limit_ends_done_and_truncated() {
    let e = env().await;
    let ev = ask(&e.t, &e.user, hint_body(K1, "q-long", 1)).await;
    let (name, data) = terminal(&ev);
    assert_eq!(name, "done");
    assert_eq!(data["truncated"], true);
    let content: String = q1(
        &e.t,
        "SELECT content FROM messages WHERE role = 'assistant'",
    );
    assert_eq!(content.chars().count(), 6_000);
    assert_eq!(text(&ev), content);
    let truncated: i64 = q1(
        &e.t,
        "SELECT truncated FROM messages WHERE role = 'assistant'",
    );
    assert_eq!(truncated, 1);
}

#[tokio::test]
async fn a_client_that_goes_away_still_gets_the_answer_saved() {
    let e = env().await;
    let mut s =
        e.t.open_stream(STUDY_ASK, &e.user, hint_body(K1, "q-slow", 1))
            .await
            .expect("stream");
    let (_, start) = s.until("start").await;
    let msg = start["assistantMessageId"].as_i64().expect("message");
    drop(s);
    stub_waiting(&e.t).await;
    e.t.stub.release();
    eventually(|| {
        let st: String =
            e.t.db()
                .query_row("SELECT status FROM messages WHERE id = ?1", [msg], |r| {
                    r.get(0)
                })
                .expect("status");
        st == "done"
    })
    .await;
    assert!(assistant_content(&e.t, msg).starts_with("First work out"));
}

#[tokio::test]
async fn stop_ends_a_study_answer_as_stopped() {
    let e = env().await;
    let mut s =
        e.t.open_stream(STUDY_ASK, &e.user, explain_body(K1, "q-slow", 1))
            .await
            .expect("stream");
    let (_, start) = s.until("start").await;
    let (st, v) =
        e.t.post("/api/ai/stop", &e.user, json!({"chatId": start["chatId"]}))
            .await;
    assert_eq!((st, &v["stopped"]), (200, &json!(true)), "{v}");
    let rest = s.all().await;
    assert_eq!(terminal_name(&rest), "stopped");
    e.t.stub.release();
}

#[tokio::test]
async fn the_reply_language_is_the_asked_one_else_the_backends() {
    let e = env().await;
    // ja asked: the prompt says Japanese, the answer is stored as ja
    let mut b = hint_body(K1, "q1", 1);
    b["lang"] = json!("ja");
    let ev = ask(&e.t, &e.user, b).await;
    assert_eq!(start_of(&ev)["lang"], "ja");
    let sys = e.t.stub.last().body["messages"][0]["content"]
        .as_str()
        .expect("system")
        .to_string();
    assert!(sys
        .ends_with("Always reply in Japanese (the language chosen in the user's Xexam settings)."));
    // zh is accepted as an alias of zh-CN and stored as zh-CN
    let mut b = hint_body(K2, "q1", 1);
    b["lang"] = json!("zh");
    assert_eq!(start_of(&ask(&e.t, &e.user, b).await)["lang"], "zh-CN");
    // no lang: the backend's uiLang decides
    e.t.backend.st().ui_lang = "zh".to_string();
    let mut b = hint_body(K3, "q1", 1);
    b.as_object_mut().expect("object").remove("lang");
    let ev = ask(&e.t, &e.user, b).await;
    assert_eq!(start_of(&ev)["lang"], "zh-CN");
    let last = e.t.stub.last();
    let sys = last.body["messages"][0]["content"]
        .as_str()
        .expect("system");
    assert!(sys.ends_with(
        "Always reply in Simplified Chinese (the language chosen in the user's Xexam settings)."
    ));
    // the elang the backend saw
    let _ = last;
}

#[tokio::test]
async fn a_study_pair_shows_in_the_chat_with_its_fields() {
    let e = env().await;
    let ev = ask(&e.t, &e.user, hint_body(K1, "q1", 5)).await;
    let chat = start_of(&ev)["chatId"].as_i64().expect("chat");
    let (s, v) = e.t.get(&format!("/api/ai/chat?id={chat}"), &e.user).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["chat"]["kind"], "practice");
    let m = &v["messages"][1];
    assert_eq!(m["kind"], "hint");
    assert_eq!(m["questionId"], "q1");
    assert_eq!(m["questionNo"], 5);
    assert_eq!(m["source"], "practice");
    assert_eq!(m["lang"], "en");
    assert_eq!(m["recordId"], Value::Null);
    assert_eq!(m["sample"], Value::Null);
    // a follow-up typed in AI Chat keeps the chat kind in its start frame and is a normal chat pair
    let mut r =
        e.t.open_stream(
            "/api/ai/send",
            &e.user,
            json!({"chatId": chat, "text": "and why?", "lang": "en"}),
        )
        .await
        .expect("stream");
    let (_, st) = r.until("start").await;
    assert_eq!(st["chatKind"], "practice");
    let _ = r.all().await;
    settle(&e.t).await;
    let (_, v) = e.t.get(&format!("/api/ai/chat?id={chat}"), &e.user).await;
    assert_eq!(v["messages"][2]["kind"], "chat");
    assert_eq!(v["messages"][3]["kind"], "chat");
    // a plain new chat says "chat"
    let mut r =
        e.t.open_stream(
            "/api/ai/send",
            &e.user,
            json!({"text": "hello", "lang": "en"}),
        )
        .await
        .expect("stream");
    let (_, st) = r.until("start").await;
    assert_eq!(st["chatKind"], "chat");
    let _ = r.all().await;
    settle(&e.t).await;
}

#[tokio::test]
async fn an_inline_think_span_is_never_shown_or_stored() {
    let e = env().await;
    let ev = ask(&e.t, &e.user, hint_body(K1, "q-think", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    let shown = text(&ev);
    assert_eq!(shown, CLEAN_HINT);
    assert!(
        !shown.contains("think") && !shown.contains("first option"),
        "{shown:?}"
    );
    let chat = start_of(&ev)["chatId"].as_i64().expect("chat");
    let content: String =
        e.t.db()
            .query_row(
                "SELECT content FROM messages WHERE chat_id = ?1 AND role = 'assistant'",
                [chat],
                |r| r.get(0),
            )
            .expect("assistant");
    assert_eq!(content, CLEAN_HINT);
}
