//! v1.3: the study routes (hint, explanation, summary) discard the model's reasoning exactly as before: it reaches no
//! frame, no stored chat, no shared answer, no saved route and no guard. The reasoning below names the correct option,
//! so a guard that saw it would withhold the hint.
use super::util::*;
use serde_json::{json, Value};

/// Made-up reasoning words; the stub sends them as `reasoning_content` or as a leading span.
const SECRET: &str = "the correct option is Bravo tool, zebra-thought-7781";

async fn ask_raw(
    t: &TestApp,
    who: &Who,
    path: &str,
    body: Value,
) -> (Vec<(String, Value)>, String) {
    let mut s = t
        .open_stream(path, who, body)
        .await
        .unwrap_or_else(|(s, v)| panic!("expected a stream, got {s} {v}"));
    let ev = s.all().await;
    let raw = s.raw.clone();
    settle(t).await;
    (ev, raw)
}

fn disk_text(t: &TestApp) -> String {
    let mut all = String::new();
    for suffix in ["", "-wal"] {
        let p = format!("{}{suffix}", t.db_path().display());
        if let Ok(b) = std::fs::read(&p) {
            all.push_str(&String::from_utf8_lossy(&b));
        }
    }
    all
}

#[tokio::test]
async fn study_answers_never_see_reasoning() {
    let e = env().await;
    // sharing on, so the shared rows are checked too
    let mut l = e.t.app.0.limits();
    l.study_share = 1;
    e.t.app.0.set_limits(l);
    e.t.add_record(record(
        &e.user,
        42,
        "exam",
        Some(K2),
        "2026-11-11 21:58",
        vec![item("q1", json!(["A"]), false)],
    ));
    e.t.stub.st().reasoning = Some(SECRET.to_string());
    let mut raws = Vec::new();
    for (path, body, kind) in [
        (STUDY_ASK, hint_body(K1, "q1", 1), "hint"),
        (STUDY_ASK, explain_body(K1, "q1", 1), "explain"),
        (
            SUMMARY_ASK,
            json!({"recordId": 42, "lang": "en"}),
            "summary",
        ),
    ] {
        let (ev, raw) = ask_raw(&e.t, &e.user, path, body).await;
        assert_eq!(terminal_name(&ev), "done", "{kind}: {ev:?}");
        assert_ne!(terminal(&ev).1["code"], json!("ai.hintLeak"), "{kind}");
        for n in names(&ev) {
            assert!(
                ["start", "thinking", "delta", "done"].contains(&n.as_str()),
                "{kind}: frame {n}"
            );
        }
        assert!(
            !names(&ev).iter().any(|n| n.starts_with("reasoning")),
            "{kind}"
        );
        assert!(!raw.contains("zebra-thought-7781"), "{kind}: {raw}");
        raws.push(raw);
    }
    let kept: i64 = q1(
        &e.t,
        "SELECT COUNT(*) FROM messages WHERE reasoning IS NOT NULL",
    );
    assert_eq!(kept, 0);
    let shared: String = q1(
        &e.t,
        "SELECT COALESCE(group_concat(content), '') FROM study_shared",
    );
    assert!(!shared.contains("zebra-thought-7781"));
    let n: i64 = q1(&e.t, "SELECT COUNT(*) FROM study_shared");
    assert!(n >= 2, "the hint and the explanation were shared: {n}");
    // the saved routes carry the answer only, no reasoning key
    for kind in ["hint", "explain"] {
        let (s, v) = e
            .t
            .get(
                &format!("/api/ai/study/saved?mode=practice&key={K1}&source=practice&question=q1&kind={kind}"),
                &e.user,
            )
            .await;
        assert_eq!(s, 200, "{v}");
        let text = v.to_string();
        assert!(!text.contains("zebra-thought-7781"), "{kind}: {text}");
        assert!(!text.contains("reasoning"), "{kind}: {text}");
        assert!(v["latest"]["content"]
            .as_str()
            .is_some_and(|c| !c.is_empty()));
    }
    assert!(!disk_text(&e.t).contains("zebra-thought-7781"));
    assert!(!captured_contains("zebra-thought-7781"));
}

#[tokio::test]
async fn a_leading_thought_or_thinking_span_in_a_hint_is_removed_before_the_guard() {
    for tag in ["thought", "thinking", "think"] {
        let e = env().await;
        {
            let mut st = e.t.stub.st();
            st.reasoning = Some(SECRET.to_string());
            st.reasoning_tag = Some(tag);
        }
        let (ev, raw) = ask_raw(&e.t, &e.user, STUDY_ASK, hint_body(K1, "q1", 1)).await;
        assert_eq!(terminal_name(&ev), "done", "{tag}: {ev:?}");
        assert_eq!(text(&ev), CLEAN_HINT, "{tag}: the hint is whole and clean");
        assert!(!raw.contains(&format!("<{tag}>")), "{tag}: {raw}");
        assert!(!raw.contains("zebra-thought-7781"), "{tag}: {raw}");
        let stored: String = q1(
            &e.t,
            "SELECT content FROM messages WHERE kind = 'hint' AND role = 'assistant'",
        );
        assert_eq!(stored, CLEAN_HINT, "{tag}");
        assert!(!disk_text(&e.t).contains("zebra-thought-7781"), "{tag}");
    }
    assert!(!captured_contains("zebra-thought-7781"));
}
