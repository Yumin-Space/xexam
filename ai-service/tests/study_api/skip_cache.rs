//! Cloudflare AI Gateway cache bypass: a regenerate sends `cf-aig-skip-cache: true`, a first ask does not, and no other
//! provider ever gets the header. All questions, records and texts are made up.
use super::util::*;
use serde_json::{json, Value};

const HEADER: &str = "cf-aig-skip-cache";

/// Admin: a Cloudflare AI Gateway (OpenAI-compatible route of the stub) with the `cf-aig-authorization` scheme.
async fn use_gateway(e: &Env) {
    let (s, v) =
        e.t.post(
            "/api/ai/admin/settings",
            &e.admin,
            json!({"provider": "gateway", "baseUrl": format!("{}/gw/openai", e.t.stub.url),
                   "gatewayAuth": "cf-aig-authorization", "gatewayKey": GW_KEY, "apiKey": null,
                   "model": "demo-chat-small"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    e.t.verify_and_enable(&e.admin).await;
}

/// The skip-cache header of every chat request the stub received so far, oldest first (the verify call included: it
/// is a plain first request, so it must carry none).
fn skips(e: &Env) -> Vec<Option<String>> {
    e.t.stub
        .requests()
        .iter()
        .filter(|r| r.path.ends_with("/chat/completions"))
        .map(|r| r.header(HEADER).map(str::to_string))
        .collect()
}

fn wrong_record(user: &Who) -> FakeRecord {
    record(
        user,
        42,
        "exam",
        Some(K2),
        "2026-11-11 21:58",
        vec![
            item("q1", json!(["A"]), false),
            item("q2", Value::Null, false),
        ],
    )
}

#[tokio::test]
async fn a_study_regenerate_skips_the_gateway_cache_and_a_first_ask_does_not() {
    let e = env().await;
    use_gateway(&e).await;
    let before = skips(&e).len();
    let true_ = Some("true".to_string());

    // first ask: cached
    let ev = ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    assert_eq!(skips(&e)[before..], [None]);
    // the same question / kind / source again: regenerate
    let ev = ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    assert_eq!(skips(&e)[before..], [None, true_.clone()]);
    // another kind of the same question is a first ask again
    ask(&e.t, &e.user, explain_body(K1, "q1", 1)).await;
    assert_eq!(skips(&e)[before..], [None, true_.clone(), None]);
    ask(&e.t, &e.user, explain_body(K1, "q1", 1)).await;
    assert_eq!(
        skips(&e)[before..],
        [None, true_.clone(), None, true_.clone()]
    );
    // another question is a first ask too
    ask(&e.t, &e.user, hint_body(K1, "q2", 2)).await;
    assert_eq!(skips(&e).last(), Some(&None));
}

#[tokio::test]
async fn an_ask_after_a_withdrawn_hint_skips_the_gateway_cache() {
    // The leaking hint came back whole from upstream, so the gateway may hold it: asking again must not get it back
    let e = env().await;
    use_gateway(&e).await;
    let before = skips(&e).len();
    let ev = ask(&e.t, &e.user, hint_body(K1, "q-leak", 1)).await;
    assert_eq!(terminal_name(&ev), "error");
    assert_eq!(skips(&e)[before..], [None]);
    ask(&e.t, &e.user, hint_body(K1, "q-leak", 1)).await;
    assert_eq!(skips(&e)[before..], [None, Some("true".to_string())]);
}

#[tokio::test]
async fn a_summary_regenerate_skips_the_gateway_cache() {
    let e = env().await;
    use_gateway(&e).await;
    e.t.add_record(wrong_record(&e.user));
    let before = skips(&e).len();
    let body = json!({"recordId": 42, "lang": "en"});
    let ev = ask_to(&e.t, &e.user, SUMMARY_ASK, body.clone()).await;
    assert_eq!(terminal_name(&ev), "done");
    assert_eq!(skips(&e)[before..], [None]);
    let ev = ask_to(&e.t, &e.user, SUMMARY_ASK, body).await;
    assert_eq!(terminal_name(&ev), "done");
    assert_eq!(skips(&e)[before..], [None, Some("true".to_string())]);
}

#[tokio::test]
async fn an_ai_chat_retry_skips_the_gateway_cache() {
    let e = env().await;
    use_gateway(&e).await;
    let before = skips(&e).len();
    e.t.stub.force(Some((
        500,
        r#"{"error":{"type":"api_error","message":"internal detail"}}"#,
    )));
    let ev =
        e.t.send_all(&e.user, json!({"text": "first try"}))
            .await
            .expect("stream");
    assert_eq!(terminal_name(&ev), "error");
    let chat = ev[0].1["chatId"].as_i64().expect("chatId");
    assert_eq!(skips(&e)[before..], [None]);
    e.t.stub.force(None);
    let mut s =
        e.t.open_stream("/api/ai/retry", &e.user, json!({"chatId": chat}))
            .await
            .expect("retry");
    let ev = s.all().await;
    assert_eq!(terminal_name(&ev), "done");
    settle(&e.t).await;
    assert_eq!(skips(&e)[before..], [None, Some("true".to_string())]);
}

#[tokio::test]
async fn other_providers_never_get_the_header() {
    // `env` configures plain OpenAI-compatible settings
    let e = env().await;
    ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await;
    ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await;
    e.t.add_record(wrong_record(&e.user));
    let body = json!({"recordId": 42, "lang": "en"});
    ask_to(&e.t, &e.user, SUMMARY_ASK, body.clone()).await;
    ask_to(&e.t, &e.user, SUMMARY_ASK, body).await;
    let all = skips(&e);
    assert!(all.len() >= 4, "{all:?}");
    assert!(all.iter().all(Option::is_none), "{all:?}");
}

// —— the regenerate note: the request of an attempt differs from every earlier one (a gateway that caches by content
// ignores the header) ——

const NOTE: &str = "Regenerate request #";

/// The chat requests the stub received so far (oldest first), as (system text, last message text).
fn sent(e: &Env) -> Vec<(String, String)> {
    e.t.stub
        .requests()
        .iter()
        .filter(|r| r.path.ends_with("/chat/completions"))
        .map(|r| {
            let m = r.body["messages"].as_array().expect("messages");
            let text = |v: &Value| v["content"].as_str().expect("text").to_string();
            (text(&m[0]), text(m.last().expect("last")))
        })
        .collect()
}

fn note_number(last: &str) -> Option<String> {
    last.find(NOTE).map(|i| {
        let rest = &last[i + NOTE.len()..];
        let n: String = rest.chars().take_while(char::is_ascii_digit).collect();
        assert!(
            rest[n.len()..].starts_with(
                ": write a new answer; do not repeat an earlier answer word for word.)"
            ),
            "{rest}"
        );
        assert!(last.ends_with(')'), "the note ends the last turn: {last}");
        n
    })
}

fn stored_notes(e: &Env) -> i64 {
    q1(
        &e.t,
        "SELECT COUNT(*) FROM messages WHERE content LIKE '%Regenerate request%'",
    )
}

#[tokio::test]
async fn a_study_regenerate_note_counts_the_attempts_and_is_never_stored() {
    let e = env().await;
    let before = sent(&e).len();
    for _ in 0..3 {
        let ev = ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await;
        assert_eq!(terminal_name(&ev), "done");
    }
    let all = sent(&e);
    let all = &all[before..];
    let numbers: Vec<_> = all.iter().map(|(_, l)| note_number(l)).collect();
    assert_eq!(numbers, [None, Some("2".into()), Some("3".into())]);
    // the system prompt (and so its prompt cache) is the same every time
    assert!(all.iter().all(|(s, _)| *s == all[0].0));
    // the attempts differ from each other only by the note
    assert_ne!(all[0].1, all[1].1);
    assert!(all[1].1.starts_with(&all[0].1));
    assert_eq!(stored_notes(&e), 0);
    // another kind of the same question starts at its own first ask
    ask(&e.t, &e.user, explain_body(K1, "q1", 1)).await;
    assert_eq!(note_number(&sent(&e).last().expect("req").1), None);
}

#[tokio::test]
async fn a_summary_regenerate_note_counts_the_attempts_and_is_never_stored() {
    let e = env().await;
    e.t.add_record(wrong_record(&e.user));
    let before = sent(&e).len();
    let body = json!({"recordId": 42, "lang": "en"});
    for _ in 0..3 {
        let ev = ask_to(&e.t, &e.user, SUMMARY_ASK, body.clone()).await;
        assert_eq!(terminal_name(&ev), "done");
    }
    let all = sent(&e);
    let all = &all[before..];
    let numbers: Vec<_> = all.iter().map(|(_, l)| note_number(l)).collect();
    assert_eq!(numbers, [None, Some("2".into()), Some("3".into())]);
    assert!(all.iter().all(|(s, _)| *s == all[0].0));
    assert_eq!(stored_notes(&e), 0);
}

#[tokio::test]
async fn an_ai_chat_retry_note_counts_the_attempts_and_is_never_stored() {
    let e = env().await;
    let before = sent(&e).len();
    let fail = || {
        e.t.stub.force(Some((
            500,
            r#"{"error":{"type":"api_error","message":"internal detail"}}"#,
        )))
    };
    fail();
    let ev =
        e.t.send_all(&e.user, json!({"text": "first try"}))
            .await
            .expect("stream");
    assert_eq!(terminal_name(&ev), "error");
    let chat = ev[0].1["chatId"].as_i64().expect("chatId");
    // two retries that fail again, then one that works
    for want in ["2", "3"] {
        let mut s =
            e.t.open_stream("/api/ai/retry", &e.user, json!({"chatId": chat}))
                .await
                .expect("retry");
        assert_eq!(terminal_name(&s.all().await), "error", "retry #{want}");
        settle(&e.t).await;
    }
    e.t.stub.force(None);
    let mut s =
        e.t.open_stream("/api/ai/retry", &e.user, json!({"chatId": chat}))
            .await
            .expect("retry");
    assert_eq!(terminal_name(&s.all().await), "done");
    settle(&e.t).await;
    let all = sent(&e);
    let all = &all[before..];
    let numbers: Vec<_> = all.iter().map(|(_, l)| note_number(l)).collect();
    assert_eq!(
        numbers,
        [None, Some("2".into()), Some("3".into()), Some("4".into())]
    );
    assert!(all.iter().all(|(s, _)| *s == all[0].0));
    assert_eq!(stored_notes(&e), 0);
    let (_, v) = e.t.get(&format!("/api/ai/chat?id={chat}"), &e.user).await;
    assert!(!v.to_string().contains("Regenerate request"), "{v}");
}
