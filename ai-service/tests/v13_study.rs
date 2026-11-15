//! v1.3 shared hints / explanations (`study_shared`) and the summary limits. All questions, records and texts are made
//! up. The tests run with `TZ=UTC`.
mod common;

#[path = "study_api/util.rs"]
mod util;

use serde_json::{json, Value};
use util::*;
use xexam_ai_service::features::Feature;
use xexam_ai_service::store::conn;

/// An app with sharing on (the shared study tests of v1.2 keep it off), a second AI+ user and an AI+ admin.
async fn shared_env() -> (Env, Who) {
    let e = env().await;
    set_share(&e.t, true);
    let kim = e.t.user("kim", false, true);
    (e, kim)
}

fn set_share(t: &TestApp, on: bool) {
    let mut l = t.app.0.limits();
    l.study_share = i64::from(on);
    t.app.0.set_limits(l);
}

fn rows(t: &TestApp) -> i64 {
    q1(t, "SELECT COUNT(*) FROM study_shared")
}

/// Waits for the shared row of the first answer (it is written just after the stream ends).
async fn stored(t: &TestApp, n: i64) {
    eventually(|| rows(t) >= n).await;
}

fn chat_requests(t: &TestApp) -> usize {
    t.stub
        .requests()
        .iter()
        .filter(|r| r.path.ends_with("/chat/completions"))
        .count()
}

fn with(mut body: Value, k: &str, v: Value) -> Value {
    body[k] = v;
    body
}

#[tokio::test]
async fn a_second_user_gets_the_stored_answer_without_a_model_call() {
    let (e, kim) = shared_env().await;
    let first = ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await;
    assert_eq!(terminal_name(&first), "done");
    assert_eq!(start_of(&first)["shared"], false);
    stored(&e.t, 1).await;
    let calls = chat_requests(&e.t);
    let usage: i64 = q1(&e.t, "SELECT COUNT(*) FROM usage");

    let second = ask(&e.t, &kim, hint_body(K2, "q1", 4)).await;
    assert_eq!(terminal_name(&second), "done");
    assert_eq!(start_of(&second)["shared"], true);
    assert_eq!(names(&second), ["start", "delta", "done"]);
    assert_eq!(text(&second), text(&first));
    // nothing was generated and nothing counted
    assert_eq!(chat_requests(&e.t), calls);
    assert_eq!(q1::<i64>(&e.t, "SELECT COUNT(*) FROM usage"), usage);
    assert_eq!(
        q1::<i64>(&e.t, "SELECT hits FROM study_shared"),
        1,
        "one hit"
    );
    // the copy is an ordinary finished pair in kim's own session chat
    let (chat_rows, status): (i64, String) = e.t.db()
        .query_row(
            "SELECT COUNT(*), MIN(m.status) FROM messages m JOIN chats c ON c.id = m.chat_id WHERE c.user_id = ?1",
            [kim.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((chat_rows, status.as_str()), (2, "done"));
    let (s, saved) =
        e.t.get(
            &format!(
                "/api/ai/study/saved?mode=practice&source=practice&question=q1&kind=hint&key={K2}"
            ),
            &kim,
        )
        .await;
    assert_eq!(s, 200, "{saved}");
    assert_eq!(saved["latest"]["content"], text(&first));
}

#[tokio::test]
async fn a_hit_takes_no_place_and_no_cap() {
    let (e, kim) = shared_env().await;
    ask(&e.t, &e.user, explain_body(K1, "q1", 1)).await;
    stored(&e.t, 1).await;
    // kim may make one model call a minute: a miss uses it up, yet the shared answer is still served
    let mut l = e.t.app.0.limits();
    l.calls_per_minute = 1;
    e.t.app.0.set_limits(l);
    let miss = ask(&e.t, &kim, explain_body(K2, "q2", 5)).await;
    assert_eq!(start_of(&miss)["shared"], false);
    let ev = ask(&e.t, &kim, explain_body(K2, "q1", 2)).await;
    assert_eq!(start_of(&ev)["shared"], true);
    // while a real call is refused by the cap
    let (s, v) = refused(&e.t, &kim, STUDY_ASK, explain_body(K2, "qh", 3)).await;
    assert_eq!((s, v["code"].as_str()), (429, Some("ai.callLimit")), "{v}");
}

#[tokio::test]
async fn the_key_is_content_and_language() {
    let (e, kim) = shared_env().await;
    ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await;
    stored(&e.t, 1).await;
    let calls = chat_requests(&e.t);
    // another answer language: a miss
    let ev = ask(
        &e.t,
        &kim,
        with(hint_body(K2, "q1", 1), "lang", json!("zh")),
    )
    .await;
    assert_eq!(start_of(&ev)["shared"], false);
    assert_eq!(chat_requests(&e.t), calls + 1);
    stored(&e.t, 2).await;
    // an edited question: a miss
    e.t.add_question(single_question(
        "q1",
        "Which tool fits the edited made-up job?",
    ));
    let ev = ask(&e.t, &kim, hint_body(K2, "q1", 1)).await;
    assert_eq!(start_of(&ev)["shared"], false);
    stored(&e.t, 3).await;
    assert_eq!(rows(&e.t), 3);
    // hint and explanation are separate entries
    let ev = ask(&e.t, &kim, explain_body(K2, "q1", 1)).await;
    assert_eq!(start_of(&ev)["shared"], false);
}

#[tokio::test]
async fn fresh_replaces_the_answer_and_counts() {
    let (e, kim) = shared_env().await;
    ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await;
    stored(&e.t, 1).await;
    let usage: i64 = q1(&e.t, "SELECT COUNT(*) FROM usage");
    let calls = chat_requests(&e.t);
    let ev = ask(
        &e.t,
        &kim,
        with(hint_body(K2, "q1", 1), "fresh", json!(true)),
    )
    .await;
    assert_eq!(terminal_name(&ev), "done");
    assert_eq!(start_of(&ev)["shared"], false);
    assert_eq!(chat_requests(&e.t), calls + 1);
    assert_eq!(q1::<i64>(&e.t, "SELECT COUNT(*) FROM usage"), usage + 1);
    eventually(|| q1::<i64>(&e.t, "SELECT gen FROM study_shared") == 2).await;
    assert_eq!(rows(&e.t), 1);
    // `fresh` must be a boolean
    for bad in [json!("true"), json!(1), Value::Null] {
        let (s, v) = refused(
            &e.t,
            &kim,
            STUDY_ASK,
            with(hint_body(K2, "q1", 1), "fresh", bad),
        )
        .await;
        assert_eq!((s, v["code"].as_str()), (400, Some("ai.badRequest")), "{v}");
    }
}

#[tokio::test]
async fn a_failed_regenerate_leaves_the_stored_answer_alone() {
    let (e, _) = shared_env().await;
    let first = ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await;
    stored(&e.t, 1).await;
    e.t.stub.force(Some((500, "boom")));
    let ev = ask(
        &e.t,
        &e.user,
        with(hint_body(K1, "q1", 1), "fresh", json!(true)),
    )
    .await;
    e.t.stub.force(None);
    assert_eq!(terminal_name(&ev), "error");
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let (content, gen): (String, i64) =
        e.t.db()
            .query_row("SELECT content, gen FROM study_shared", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
    assert_eq!((content, gen), (text(&first), 1));
}

#[tokio::test]
async fn incomplete_answers_are_never_stored() {
    let (e, _) = shared_env().await;
    // a hint the guard withdrew (it named the answer), a failed call
    for qid in ["q-leak", "q-err"] {
        let ev = ask(&e.t, &e.user, hint_body(K1, qid, 1)).await;
        assert_eq!(terminal_name(&ev), "error", "{qid}");
    }
    // an over-long hint is cut: done, but truncated
    let ev = ask(&e.t, &e.user, hint_body(K1, "q-long", 1)).await;
    assert_eq!(terminal(&ev).1["truncated"], true);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert_eq!(rows(&e.t), 0);
}

#[tokio::test]
async fn the_switch_off_reads_and_writes_nothing() {
    let (e, kim) = shared_env().await;
    ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await;
    stored(&e.t, 1).await;
    set_share(&e.t, false);
    let calls = chat_requests(&e.t);
    let ev = ask(&e.t, &kim, hint_body(K2, "q1", 1)).await;
    assert_eq!(start_of(&ev)["shared"], false);
    assert_eq!(chat_requests(&e.t), calls + 1);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let (rows_now, gen, hits): (i64, i64, i64) =
        e.t.db()
            .query_row(
                "SELECT COUNT(*), MAX(gen), MAX(hits) FROM study_shared",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
    assert_eq!((rows_now, gen, hits), (1, 1, 0));
}

#[tokio::test]
async fn two_racing_misses_leave_one_row() {
    let (e, kim) = shared_env().await;
    let mut a =
        e.t.open_stream(STUDY_ASK, &e.user, hint_body(K1, "q1", 1))
            .await
            .unwrap();
    let mut b =
        e.t.open_stream(STUDY_ASK, &kim, hint_body(K2, "q1", 1))
            .await
            .unwrap();
    a.all().await;
    b.all().await;
    stored(&e.t, 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(rows(&e.t), 1);
}

#[tokio::test]
async fn the_gates_apply_before_the_lookup_and_admins_pass_the_global_switch() {
    let (e, kim) = shared_env().await;
    ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await;
    stored(&e.t, 1).await;
    let global = |bits: i64| {
        e.t.app
            .0
            .store
            .write(|tx| conn::set_features_off(tx, bits))
            .unwrap();
    };
    // the hint switch is off for everyone: a normal user is refused even though the answer exists
    global(Feature::Hint.bit());
    let (s, v) = refused(&e.t, &kim, STUDY_ASK, hint_body(K2, "q1", 1)).await;
    assert_eq!(
        (s, v["code"].as_str(), v["feature"].as_str()),
        (403, Some("ai.featureOff"), Some("hint")),
        "{v}"
    );
    // the explanation switch is still on
    let ev = ask(&e.t, &kim, explain_body(K2, "q1", 1)).await;
    assert_eq!(terminal_name(&ev), "done");
    // an administrator is not affected by the global switch and is served the shared answer
    let ev = ask(&e.t, &e.admin, hint_body(K2, "q1", 2)).await;
    assert_eq!(start_of(&ev)["shared"], true);
    global(0);
    // a suspended user is refused
    e.t.app
        .0
        .store
        .write(|tx| {
            xexam_ai_service::store::data::set_function_enabled(
                tx,
                kim.id,
                &kim.stamp,
                &kim.username,
                false,
                now(),
            )
        })
        .unwrap();
    let (s, v) = refused(&e.t, &kim, STUDY_ASK, hint_body(K2, "q1", 1)).await;
    assert_eq!((s, v["code"].as_str()), (403, Some("ai.suspended")), "{v}");
}

#[tokio::test]
async fn the_result_page_shares_the_practice_explanation() {
    let (e, kim) = shared_env().await;
    e.t.add_record(record(
        &kim,
        7,
        "exam",
        Some(K2),
        "2026-11-11 21:58",
        vec![
            item("q1", json!(["B"]), false),
            item("q2", json!(["B"]), false),
        ],
    ));
    // the first explanation comes from the practice page
    ask(&e.t, &e.user, explain_body(K1, "q1", 1)).await;
    stored(&e.t, 1).await;
    let calls = chat_requests(&e.t);
    let ev = ask(&e.t, &kim, result_body("exam", 7, "q1", 1)).await;
    assert_eq!(start_of(&ev)["shared"], true);
    assert_eq!(start_of(&ev)["source"], "result");
    assert_eq!(chat_requests(&e.t), calls);
    // a result-page miss builds the non-personal prompt (no sentence about the user's own answer)
    let ev = ask(&e.t, &kim, result_body("exam", 7, "q2", 2)).await;
    assert_eq!(terminal_name(&ev), "done");
    let prompt = e.t.stub.last().body["messages"].to_string();
    assert!(!prompt.contains("user_answer"), "{prompt}");
    assert!(!prompt.contains("The user answered"), "{prompt}");
}

#[tokio::test]
async fn the_reply_length_is_the_reply_limit() {
    let (e, _) = shared_env().await;
    let mut l = e.t.app.0.limits();
    l.reply_max_tokens = 1234;
    e.t.app.0.set_limits(l);
    ask(&e.t, &e.user, hint_body(K1, "q2", 1)).await;
    assert_eq!(e.t.stub.last().body["max_tokens"], json!(1234));
}

fn wrong_record(user: &Who, n: usize) -> FakeRecord {
    let items = (0..n).map(|_| item("q1", json!(["A"]), false)).collect();
    record(user, 42, "exam", Some(K2), "2026-11-11 21:58", items)
}

#[tokio::test]
async fn the_summary_lists_at_most_summary_max_items() {
    let (e, _) = shared_env().await;
    let mut l = e.t.app.0.limits();
    l.summary_max_items = 10;
    e.t.app.0.set_limits(l);
    e.t.add_record(wrong_record(&e.user, 15));
    let ev = ask_to(
        &e.t,
        &e.user,
        SUMMARY_ASK,
        json!({"recordId": 42, "lang": "en"}),
    )
    .await;
    assert_eq!(terminal_name(&ev), "done");
    assert_eq!(
        start_of(&ev)["sample"],
        json!({"used": 10, "usable": 15, "wrong": 15})
    );
}

#[test]
fn the_summary_prompt_budget_scales_with_the_limit() {
    use xexam_ai_service::prompt::Lang;
    use xexam_ai_service::study::{
        summary_prompt_with, KeyAnswer, OptionText, QKind, RecordInfo, RecordKind, Sample,
        Solution, StudyQuestion, SummaryItem, UserEntry,
    };
    let q = StudyQuestion {
        id: "q1".into(),
        kind: QKind::Single,
        lang: Lang::En,
        stem: "x".repeat(3_000),
        options: vec![OptionText {
            key: "A".into(),
            text: "one".into(),
        }],
        items: vec![],
        active: true,
        cut: false,
    };
    let sol = Solution {
        answer: KeyAnswer::Letters(vec!["A".into()]),
        explanation: String::new(),
        mnemonic: String::new(),
        note: String::new(),
        explain_lang: None,
        key_texts: vec![],
    };
    let items: Vec<SummaryItem> = (0..10)
        .map(|i| SummaryItem {
            seq: i as u32,
            question: q.clone(),
            solution: sol.clone(),
            answer: vec![UserEntry::Text("B".into())],
        })
        .collect();
    let rec = RecordInfo {
        at: "2026-11-11 21:58".into(),
        kind: RecordKind::Exam,
    };
    let sample = Sample::new(10, 10, 10);
    let small = summary_prompt_with(&rec, &items, sample, Lang::En, 7_500).1;
    let large = summary_prompt_with(&rec, &items, sample, Lang::En, 52_500).1;
    assert!(
        small.chars().count() < 7_500 + 10 * 400,
        "cut to the small budget"
    );
    assert!(
        large.chars().count() > small.chars().count() * 3,
        "the large budget keeps more"
    );
}
