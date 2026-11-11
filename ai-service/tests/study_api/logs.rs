//! Nothing of a question, an answer, a chat title, a session key or a record time is ever logged.
use super::util::*;
use serde_json::json;

#[tokio::test]
async fn no_study_content_reaches_the_logs() {
    let e = env().await;
    let secret_key = "c0ffee00c0ffee00c0ffee00c0ffee00";
    let mut q = single_question("q-log", "Zq-unique-stem-9f31 which made-up tool fits?");
    q.solution["explanation"] = json!("Zq-unique-explanation-77aa");
    e.t.add_question(q);
    e.t.add_record(record(
        &e.user,
        61,
        "exam",
        Some(secret_key),
        "2031-05-06 07:08",
        vec![item("q-log", json!(["A"]), false)],
    ));
    let _ = ask(&e.t, &e.user, hint_body(secret_key, "q-log", 1)).await;
    let _ = ask(&e.t, &e.user, explain_body(secret_key, "q-log", 1)).await;
    let _ = ask(&e.t, &e.user, hint_body(K3, "q-leak", 1)).await;
    let _ = ask(&e.t, &e.user, result_body("exam", 61, "q-log", 1)).await;
    let _ = ask_to(
        &e.t,
        &e.user,
        SUMMARY_ASK,
        json!({"recordId": 61, "lang": "en"}),
    )
    .await;
    // failures of the data routes
    e.t.backend.st().study_switch = StudySwitch::Unreadable;
    let _ = refused(&e.t, &e.user, STUDY_ASK, hint_body(K1, "q-log", 1)).await;
    e.t.backend.st().study_switch = StudySwitch::Normal;
    let hint_text = &CLEAN_HINT[..40];
    for l in captured() {
        for needle in [
            "Zq-unique-stem-9f31",
            "Zq-unique-explanation-77aa",
            secret_key,
            "2031-05-06 07:08",
            hint_text,
            "Bravo",
            "Items:",
        ] {
            assert!(!l.contains(needle), "{needle} in a log line: {l}");
        }
    }
    for l in e.t.backend.logs() {
        for needle in [
            "Zq-unique-stem-9f31",
            secret_key,
            "2031-05-06 07:08",
            hint_text,
        ] {
            assert!(!l.contains(needle), "{needle} in a forwarded line: {l}");
        }
    }
}
