//! Reply language (round 2, item 24): `lang` of send / retry / suggest is the Settings language; the system prompt says
//! "Always reply in {language} …" instead of "Reply in the language the user writes in."; missing = English; anything
//! else is refused before anything is saved or sent.
mod common;

use common::*;
use serde_json::{json, Value};

/// The language line of the round-2 contracts (5.2), written out here on purpose.
fn line(language: &str) -> String {
    format!(
        "Always reply in {language} (the language chosen in the user's Xexam settings), unless the user explicitly asks for another language in this message."
    )
}

/// The system text of an upstream request (OpenAI: the first message; Anthropic: `system`).
fn system_of(req: &StubRequest) -> String {
    if let Some(s) = req.body.get("system").and_then(Value::as_str) {
        return s.to_string();
    }
    // Anthropic: one text block (with the cache breakpoint)
    if let Some(s) = req.body["system"][0]["text"].as_str() {
        return s.to_string();
    }
    assert_eq!(
        req.body["messages"][0]["role"],
        json!("system"),
        "{:?}",
        req.body
    );
    req.body["messages"][0]["content"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn count(t: &TestApp, table: &str) -> i64 {
    t.db()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .expect("count")
}

/// The study-assistant prompt with exactly the language sentence replaced.
fn assert_study_prompt(sys: &str, language: &str) {
    assert!(
        sys.starts_with(
            "You are the study assistant inside Xexam, a self-hosted exam practice app. Answer clearly and concisely.\n"
        ),
        "{sys}"
    );
    // the language line closes the prompt (after the static text and the memories)
    assert!(
        sys.contains("\nIf you are not sure, say so. Use plain text; simple Markdown\n"),
        "{sys}"
    );
    assert!(sys.ends_with(&format!("\n\n{}", line(language))), "{sys}");
    assert!(
        !sys.contains("Reply in the language the user writes in"),
        "{sys}"
    );
    assert_eq!(sys.matches("Always reply in").count(), 1, "{sys}");
    assert!(sys.contains("Today is "), "{sys}");
}

#[tokio::test]
async fn send_puts_the_settings_language_into_the_system_prompt() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    for (lang, language) in [
        (Some("en"), "English"),
        (Some("zh"), "Simplified Chinese"),
        (Some("zh-CN"), "Simplified Chinese"),
        (Some("ja"), "Japanese"),
        (None, "English"),
    ] {
        let mut body = json!({"chatId": null, "text": "What is a set?"});
        if let Some(l) = lang {
            body["lang"] = json!(l);
        }
        let ev = t.send_all(&sam, body).await.expect("stream");
        assert_eq!(terminal(&ev).0, "done", "{lang:?}: {ev:?}");
        assert_eq!(text(&ev), ANSWER);
        assert_study_prompt(&system_of(&t.stub.last()), language);
    }
    // the language of each message is its own: the same chat in English, then in Japanese
    let ev = t
        .send_all(&sam, json!({"text": "First", "lang": "en"}))
        .await
        .expect("stream");
    let chat = ev[0].1["chatId"].as_i64().expect("chat");
    assert_study_prompt(&system_of(&t.stub.last()), "English");
    let ev = t
        .send_all(
            &sam,
            json!({"chatId": chat, "text": "Second", "lang": "ja"}),
        )
        .await
        .expect("stream");
    assert_eq!(terminal(&ev).0, "done");
    assert_study_prompt(&system_of(&t.stub.last()), "Japanese");
}

#[tokio::test]
async fn the_anthropic_format_carries_the_language_in_system() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    let (s, v) = t
        .post(
            "/api/ai/admin/settings",
            &admin,
            json!({"provider": "anthropic", "baseUrl": format!("{}/anthropic", t.stub.url), "apiKey": KEY, "model": "demo-claude"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    t.verify_and_enable(&admin).await;
    let ev = t
        .send_all(&sam, json!({"text": "Explain sets", "lang": "zh"}))
        .await
        .expect("stream");
    assert_eq!(terminal(&ev).0, "done");
    let req = t.stub.last();
    assert_eq!(req.path, "/anthropic/v1/messages");
    assert_study_prompt(&system_of(&req), "Simplified Chinese");
}

#[tokio::test]
async fn an_invalid_language_is_refused_before_anything_is_saved() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let ev = t
        .send_all(&sam, json!({"text": "A chat to retry and suggest from"}))
        .await
        .expect("stream");
    let chat = ev[0].1["chatId"].as_i64().expect("chat");
    let question = ev[0].1["userMessageId"].as_i64().expect("message");
    let calls = t.stub.count();
    let (chats, messages, usage) = (count(&t, "chats"), count(&t, "messages"), t.usage_rows());
    for bad in [
        json!("fr"),
        json!(5),
        json!(""),
        json!(null),
        json!("EN"),
        json!("zh-TW"),
        json!(["en"]),
        json!({"lang": "en"}),
        json!(true),
    ] {
        for (path, body) in [
            (
                "/api/ai/send",
                json!({"chatId": null, "text": "hi", "lang": bad}),
            ),
            (
                "/api/ai/send",
                json!({"chatId": chat, "text": "hi", "lang": bad}),
            ),
            ("/api/ai/retry", json!({"chatId": chat, "lang": bad})),
            (
                "/api/ai/memories/suggest",
                json!({"messageId": question, "lang": bad}),
            ),
        ] {
            let (code, v) = t.post(path, &sam, body).await;
            assert_eq!(code, 400, "{path} {bad}: {v}");
            assert_eq!(v["code"], json!("ai.badRequest"), "{path} {bad}: {v}");
        }
    }
    assert_eq!(t.stub.count(), calls, "nothing went upstream");
    assert_eq!(count(&t, "chats"), chats);
    assert_eq!(count(&t, "messages"), messages);
    assert_eq!(t.usage_rows(), usage);
}

#[tokio::test]
async fn retry_uses_the_language_of_its_own_request() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    t.stub.force(Some((
        500,
        r#"{"error":{"message":"made-up failure","type":"server_error"}}"#,
    )));
    let ev = t
        .send_all(&sam, json!({"text": "Hello there", "lang": "en"}))
        .await
        .expect("stream");
    assert_eq!(terminal(&ev).0, "error", "{ev:?}");
    let chat = ev[0].1["chatId"].as_i64().expect("chat");
    assert_study_prompt(&system_of(&t.stub.last()), "English");
    t.stub.force(None);
    // a refused language leaves the failed answer as it was
    let (code, _) = t
        .post("/api/ai/retry", &sam, json!({"chatId": chat, "lang": "de"}))
        .await;
    assert_eq!(code, 400);
    let (_, v) = t.get(&format!("/api/ai/chat?id={chat}"), &sam).await;
    assert_eq!(v["messages"][1]["status"], json!("error"));
    let ev = t
        .open_stream("/api/ai/retry", &sam, json!({"chatId": chat, "lang": "ja"}))
        .await
        .expect("retry")
        .all()
        .await;
    assert_eq!(ev[0].1["retry"], json!(true));
    assert_eq!(terminal(&ev).0, "done");
    assert_study_prompt(&system_of(&t.stub.last()), "Japanese");
    // without a language: English
    t.stub.force(Some((
        500,
        r#"{"error":{"message":"made-up failure","type":"server_error"}}"#,
    )));
    let ev = t
        .send_all(&sam, json!({"chatId": chat, "text": "Again", "lang": "zh"}))
        .await
        .expect("stream");
    assert_eq!(terminal(&ev).0, "error");
    t.stub.force(None);
    let ev = t
        .open_stream("/api/ai/retry", &sam, json!({"chatId": chat}))
        .await
        .expect("retry")
        .all()
        .await;
    assert_eq!(terminal(&ev).0, "done");
    assert_study_prompt(&system_of(&t.stub.last()), "English");
}

#[tokio::test]
async fn memory_suggestions_are_written_in_the_settings_language() {
    let t = start().await;
    let admin = t.user("admin", true, false);
    let sam = t.user("sam", false, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    let ev = t
        .send_all(
            &sam,
            json!({"text": "Please remember that I like short answers", "lang": "ja"}),
        )
        .await
        .expect("stream");
    let msg = ev[0].1["userMessageId"].as_i64().expect("message");
    for (lang, language) in [
        (Some("ja"), "Japanese"),
        (Some("zh-CN"), "Simplified Chinese"),
        (Some("zh"), "Simplified Chinese"),
        (Some("en"), "English"),
        (None, "English"),
    ] {
        let mut body = json!({"messageId": msg});
        if let Some(l) = lang {
            body["lang"] = json!(l);
        }
        let (code, v) = t.post("/api/ai/memories/suggest", &sam, body).await;
        assert_eq!(code, 200, "{lang:?}: {v}");
        assert!(v["suggestion"].as_str().is_some_and(|s| !s.is_empty()));
        let req = t.stub.last();
        assert_eq!(
            system_of(&req),
            format!(
                "Rewrite the user's request as one short note about the user to remember for later chats. Write the note in {language}. At most 200 characters. Reply with the note only."
            )
        );
        assert_eq!(
            req.body["messages"][1],
            json!({"role": "user", "content": "Please remember that I like short answers"})
        );
    }
    // the test call keeps its own prompt, without a language line
    let (code, v) = t.post("/api/ai/admin/verify", &admin, json!({})).await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(system_of(&t.stub.last()), "Reply with the single word OK.");
}
