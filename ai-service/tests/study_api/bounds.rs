//! Bounds: one running answer per user (409 `ai.busy` with `running`), the limits, two first asks of one session, the
//! shape of an ask, the refusals of retry and memory suggest.
use super::util::*;
use serde_json::{json, Value};

#[tokio::test]
async fn busy_names_the_running_answer_and_never_stops_it() {
    let e = env().await;
    let mut s =
        e.t.open_stream(STUDY_ASK, &e.user, hint_body(K1, "q-slow", 7))
            .await
            .expect("stream");
    let (_, start) = s.until("start").await;
    // another hint of the same session
    let (st, v) = refused(&e.t, &e.user, STUDY_ASK, hint_body(K1, "q1", 1)).await;
    assert_eq!((st, &v["code"]), (409, &json!("ai.busy")));
    assert_eq!(v["error"], "上一个回答还在生成，请等它结束或按停止。");
    assert_eq!(
        v["running"],
        json!({"chatId": start["chatId"], "messageId": start["assistantMessageId"], "chatKind": "practice",
               "kind": "hint", "questionId": "q-slow", "questionNo": 7, "source": "practice", "recordId": null})
    );
    // `running` comes last
    let keys: Vec<&str> = v
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys.last(), Some(&"running"));
    // another session and an AI Chat message are refused the same way
    let (st, v) = refused(&e.t, &e.user, STUDY_ASK, hint_body(K2, "q1", 1)).await;
    assert_eq!((st, &v["code"]), (409, &json!("ai.busy")));
    assert_eq!(v["running"]["questionId"], "q-slow");
    let (st, v) =
        e.t.post("/api/ai/send", &e.user, json!({"text": "hi", "lang": "en"}))
            .await;
    assert_eq!((st, &v["code"]), (409, &json!("ai.busy")));
    // the running answer is still running (nothing was stopped)
    let (_, status) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(status["running"]["chatId"], start["chatId"]);
    e.t.stub.release();
    let rest = s.all().await;
    assert_eq!(terminal_name(&rest), "done");
    settle(&e.t).await;
    // after it ended the user can ask again
    assert_eq!(
        terminal_name(&ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await),
        "done"
    );
}

#[tokio::test]
async fn busy_while_a_memory_suggestion_holds_the_place_says_null() {
    let e = env().await;
    // the place is taken by another kind of call: no answer to name
    let place =
        e.t.app
            .0
            .registry
            .reserve(e.user.id, xexam_ai_service::chat::Kind::Suggest, None, 1)
            .expect("reserve");
    let (st, v) = refused(&e.t, &e.user, STUDY_ASK, hint_body(K1, "q1", 1)).await;
    assert_eq!((st, &v["code"]), (409, &json!("ai.busy")));
    assert_eq!(v["running"], Value::Null);
    drop(place);
}

#[tokio::test]
async fn the_message_limit_and_the_chat_limit() {
    let e = env().await;
    e.t.relax_caps();
    set_limit(&e.t, &e.admin, "messagesPerChat", 20).await;
    for i in 0..10 {
        let ev = ask(&e.t, &e.user, hint_body(K1, "q1", i + 1)).await;
        assert_eq!(terminal_name(&ev), "done", "{i}");
    }
    let (st, v) = refused(&e.t, &e.user, STUDY_ASK, hint_body(K1, "q1", 11)).await;
    assert_eq!(
        (st, &v["code"], &v["limit"]),
        (409, &json!("ai.messageLimit"), &json!(20))
    );
    let n: i64 = q1(&e.t, "SELECT COUNT(*) FROM messages");
    assert_eq!(n, 20, "nothing was written by the refused ask");
    // chats: ten is the smallest limit; ten sessions exist, the eleventh is refused, the first still works
    set_limit(&e.t, &e.admin, "chatsPerUser", 10).await;
    set_limit(&e.t, &e.admin, "messagesPerChat", 400).await;
    for i in 1..10 {
        let key = format!("{i:032x}");
        assert_eq!(
            terminal_name(&ask(&e.t, &e.user, hint_body(&key, "q1", 1)).await),
            "done"
        );
    }
    let (st, v) = refused(&e.t, &e.user, STUDY_ASK, hint_body(K2, "q1", 1)).await;
    assert_eq!(
        (st, &v["code"], &v["limit"]),
        (409, &json!("ai.chatLimit"), &json!(10))
    );
    assert_eq!(
        terminal_name(&ask(&e.t, &e.user, hint_body(K1, "q1", 12)).await),
        "done"
    );
    let n: i64 = q1(&e.t, "SELECT COUNT(*) FROM chats");
    assert_eq!(n, 10);
}

#[tokio::test]
async fn two_concurrent_first_asks_of_one_session_make_one_chat() {
    let e = env().await;
    set_limit(&e.t, &e.admin, "perUserRunning", 2).await;
    let hold = e.t.app.0.hold_study_asks();
    // both asks use a question the stub answers slowly, so the first one is still running when the second one decides
    let a =
        e.t.open_stream(STUDY_ASK, &e.user, hint_body(K1, "q-slow", 1));
    let b =
        e.t.open_stream(STUDY_ASK, &e.user, explain_body(K1, "q-slow", 2));
    let control = async {
        // both asks hold a place and wait before their write transaction
        let both = hold.arrived.acquire_many(2).await.expect("arrived");
        both.forget();
        hold.release.add_permits(1);
        eventually(|| {
            let n: i64 =
                e.t.db()
                    .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
                    .expect("count");
            n == 2
        })
        .await;
        hold.release.add_permits(1);
        e.t.app.0.release_study_asks();
    };
    let (ra, rb, ()) = tokio::join!(a, b, control);
    let (ok, no): (Vec<_>, Vec<_>) = [ra, rb].into_iter().partition(Result::is_ok);
    assert_eq!((ok.len(), no.len()), (1, 1));
    let (st, v) = no
        .into_iter()
        .next()
        .expect("refusal")
        .err()
        .expect("error");
    assert_eq!((st, &v["code"]), (409, &json!("ai.busy")));
    let mut good = ok.into_iter().next().expect("stream").expect("ok");
    e.t.stub.release();
    assert_eq!(terminal_name(&good.all().await), "done");
    settle(&e.t).await;
    let chats: i64 = q1(&e.t, "SELECT COUNT(*) FROM chats");
    let msgs: i64 = q1(&e.t, "SELECT COUNT(*) FROM messages");
    assert_eq!((chats, msgs), (1, 2));
}

#[tokio::test]
async fn an_ask_has_a_strict_shape() {
    let e = env().await;
    let base = hint_body(K1, "q1", 1);
    let with = |k: &str, v: Value| {
        let mut b = base.clone();
        b[k] = v;
        b
    };
    let without = |k: &str| {
        let mut b = base.clone();
        b.as_object_mut().expect("object").remove(k);
        b
    };
    let mut bad: Vec<Value> = vec![
        with("extra", json!(1)),
        with("kind", json!("other")),
        with("mode", json!("quiz")),
        with("key", json!("not-hex")),
        with("key", json!(K1.to_uppercase())),
        with("startedAt", json!(0)),
        with("startedAt", json!("1")),
        with("startedAt", json!(1.5)),
        with("questionId", json!("a b")),
        with("questionId", json!("")),
        with("questionId", json!("x".repeat(65))),
        with("no", json!(0)),
        with("no", json!(100_001)),
        with("no", json!("1")),
        with("qlang", json!("fr")),
        with("lang", json!("fr")),
        with("source", json!("page")),
        with("recordId", json!(0)),
        // a hint is practice + practice only
        with("mode", json!("exam")),
        with("source", json!("result")),
        // a practice source never carries a record
        with("recordId", json!(5)),
        // no record: key and startedAt are both required
        without("key"),
        without("startedAt"),
    ];
    for k in ["kind", "mode", "questionId", "no", "qlang", "source"] {
        bad.push(without(k));
    }
    // an exam needs a result source and a record id
    let mut exam = explain_body(K1, "q1", 1);
    exam["mode"] = json!("exam");
    bad.push(exam.clone());
    exam["source"] = json!("result");
    bad.push(exam);
    for b in bad {
        let (st, v) = refused(&e.t, &e.user, STUDY_ASK, b.clone()).await;
        assert_eq!((st, &v["code"]), (400, &json!("ai.badRequest")), "{b}");
    }
    // optional keys may be null; a record needs no key
    let mut ok = hint_body(K1, "q1", 1);
    ok["lang"] = Value::Null;
    ok["recordId"] = Value::Null;
    assert_eq!(terminal_name(&ask(&e.t, &e.user, ok).await), "done");
    // the exam never gets a hint
    let mut exam_hint = result_body("exam", 7, "q1", 1);
    exam_hint["kind"] = json!("hint");
    let (st, _) = refused(&e.t, &e.user, STUDY_ASK, exam_hint).await;
    assert_eq!(st, 400);
    // not a JSON object / not JSON content type
    let r =
        e.t.raw(
            axum::http::Method::POST,
            STUDY_ASK,
            Some(e.t.ident(&e.user)),
            Some(b"[]".to_vec()),
            Some("application/json"),
        )
        .await;
    assert_eq!(r.status().as_u16(), 400);
    // GET is not allowed on the ask routes, POST not on the saved routes
    let (st, _) = e.t.get(STUDY_ASK, &e.user).await;
    assert_eq!(st, 405);
    let (st, _) = e.t.post("/api/ai/study/saved", &e.user, json!({})).await;
    assert_eq!(st, 405);
}

#[tokio::test]
async fn a_study_answer_cannot_be_retried_or_remembered() {
    let e = env().await;
    // an answer that ended with an error (the stub fails after its text)
    let s = start_of(&ask(&e.t, &e.user, hint_body(K1, "q-err", 1)).await);
    let (st, v) =
        e.t.post(
            "/api/ai/retry",
            &e.user,
            json!({"chatId": s["chatId"], "lang": "en"}),
        )
        .await;
    assert_eq!((st, &v["code"]), (409, &json!("ai.notRetryable")));
    assert_eq!(v["error"], "这个回答不能重试。");
    let (st, v) =
        e.t.post(
            "/api/ai/memories/suggest",
            &e.user,
            json!({"messageId": s["userMessageId"], "lang": "en"}),
        )
        .await;
    assert_eq!((st, &v["code"]), (404, &json!("ai.notFound")));
}
