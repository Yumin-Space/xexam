//! v1.3 marker-secret test for the AI service: a flow that touches the shared study answers, the chat (send, regenerate,
//! edit), a connection save with a key and the admin views leaves none of the markers (key, question text, chat text,
//! answer text, internal token, session id) in any delivered activity event, printed log line or forwarded log line.
//! All texts are made up. The tests run with `TZ=UTC`.
mod common;

#[path = "study_api/util.rs"]
mod util;

use serde_json::json;
use util::*;

const KEY_MARK: &str = "sk-demo-KEYMARK-4471-zzzz";
const QUESTION_MARK: &str = "QUESTIONMARK-3319";
const CHAT_MARK: &str = "CHATMARK-8842";
const EDIT_MARK: &str = "EDITMARK-5507";

#[tokio::test]
async fn no_marker_reaches_an_event_or_a_log_line_in_the_v13_flows() {
    let e = env().await;
    // sharing on: the second ask is a shared hit
    let mut l = e.t.app.0.limits();
    l.study_share = 1;
    e.t.app.0.set_limits(l);
    let kim = e.t.user("kim", false, true);
    e.t.add_question(single_question(
        "qm",
        &format!("{QUESTION_MARK} which made-up tool fits?"),
    ));

    // shared study answers: a miss (model call) and a hit (copy)
    let first = ask(&e.t, &e.user, hint_body(K1, "qm", 1)).await;
    assert_eq!(terminal_name(&first), "done");
    eventually(|| q1::<i64>(&e.t, "SELECT COUNT(*) FROM study_shared") >= 1).await;
    let second = ask(&e.t, &kim, hint_body(K2, "qm", 1)).await;
    assert_eq!(start_of(&second)["shared"], true);
    let explained = ask(&e.t, &kim, explain_body(K2, "qm", 1)).await;
    assert_eq!(terminal_name(&explained), "done");

    // chat: send, regenerate, edit
    let sent =
        e.t.send_all(&e.user, json!({"text": CHAT_MARK}))
            .await
            .expect("stream");
    let chat = sent[0].1["chatId"].as_i64().expect("chat");
    let user_msg = sent[0].1["userMessageId"].as_i64().expect("question");
    let answer = sent[0].1["assistantMessageId"].as_i64().expect("answer");
    let again = ask_to(
        &e.t,
        &e.user,
        "/api/ai/regenerate",
        json!({"chatId": chat, "messageId": answer, "lang": "en"}),
    )
    .await;
    assert_eq!(terminal_name(&again), "done");
    let edited = ask_to(
        &e.t,
        &e.user,
        "/api/ai/edit",
        json!({"chatId": chat, "messageId": user_msg, "text": EDIT_MARK, "lang": "en"}),
    )
    .await;
    assert_eq!(terminal_name(&edited), "done");

    // admin: a connection saved with a key, the limits, a chat view
    let (s, _) =
        e.t.post(
            "/api/ai/admin/connections/save",
            &e.admin,
            json!({"name": "Spare", "provider": "openai",
                   "baseUrl": format!("{}/v1", e.t.stub.url), "apiKey": KEY_MARK}),
        )
        .await;
    assert!(s == 200 || s == 400 || s == 409, "status {s}");
    let (s, r) =
        e.t.post(
            "/api/ai/admin/limits",
            &e.admin,
            json!({"callsPerMinute": 4}),
        )
        .await;
    assert_eq!(s, 200, "{r}");
    let (s, _) =
        e.t.get(
            &format!("/api/ai/admin/chat?user={}&id={chat}", e.user.id),
            &e.admin,
        )
        .await;
    assert_eq!(s, 200);

    // deliver everything and collect all the places a secret could leak to
    e.t.app.0.deliver_outbox().await;
    xexam_ai_service::logs::flush_once(&e.t.app.0.link, xexam_ai_service::logs::queue()).await;
    let events: Vec<String> =
        e.t.backend
            .st()
            .activity
            .iter()
            .map(ToString::to_string)
            .collect();
    assert!(!events.is_empty(), "the flow produced activity events");
    let lines: Vec<String> = captured()
        .into_iter()
        .chain(e.t.backend.logs())
        .chain(events.iter().cloned())
        .collect();
    let forbidden = [
        KEY_MARK,
        "KEYMARK",
        QUESTION_MARK,
        CHAT_MARK,
        EDIT_MARK,
        "short answer",
        e.user.sid.as_str(),
        kim.sid.as_str(),
        TOKEN,
        K1,
        K2,
    ];
    for line in &lines {
        for f in forbidden {
            assert!(!line.contains(f), "{f:?} leaked: {line}");
        }
    }
}
