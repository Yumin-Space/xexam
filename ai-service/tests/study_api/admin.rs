//! Study chats in the administrators' views, clears, retention, user deletion and the usage by purpose.
use super::util::*;
use serde_json::{json, Value};

async fn two_study_chats(e: &Env) -> (i64, i64) {
    e.t.add_record(record(
        &e.user,
        42,
        "exam",
        Some(K2),
        "2026-11-11 21:58",
        vec![
            item("q1", json!(["A"]), false),
            item("q2", json!(["A"]), false),
        ],
    ));
    let a = start_of(&ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await);
    let b = start_of(
        &ask_to(
            &e.t,
            &e.user,
            SUMMARY_ASK,
            json!({"recordId": 42, "lang": "en"}),
        )
        .await,
    );
    (
        a["chatId"].as_i64().expect("chat"),
        b["chatId"].as_i64().expect("chat"),
    )
}

#[tokio::test]
async fn the_admin_views_show_study_chats_with_their_kind_and_fields() {
    let e = env().await;
    let (practice, summary) = two_study_chats(&e).await;
    let (st, v) =
        e.t.get(&format!("/api/ai/admin/user?id={}", e.user.id), &e.admin)
            .await;
    assert_eq!(st, 200, "{v}");
    let kinds: Vec<(i64, String)> = v["chats"]
        .as_array()
        .expect("chats")
        .iter()
        .map(|c| {
            (
                c["id"].as_i64().expect("id"),
                c["kind"].as_str().expect("kind").to_string(),
            )
        })
        .collect();
    assert!(
        kinds.contains(&(practice, "practice".to_string())),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&(summary, "summary".to_string())),
        "{kinds:?}"
    );
    let (st, v) =
        e.t.get(
            &format!("/api/ai/admin/chat?user={}&id={summary}", e.user.id),
            &e.admin,
        )
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["chat"]["kind"], "summary");
    assert_eq!(v["chat"]["recordId"], 42);
    let m = &v["messages"][1];
    assert_eq!(m["kind"], "summary");
    assert_eq!(m["recordId"], 42);
    assert_eq!(m["sample"], json!({"used": 2, "usable": 2, "wrong": 2}));
    assert_eq!(m["questionId"], Value::Null);
}

#[tokio::test]
async fn clearing_and_deleting_remove_study_chats() {
    let e = env().await;
    let (practice, summary) = two_study_chats(&e).await;
    let (st, v) =
        e.t.post(
            "/api/ai/admin/clear",
            &e.admin,
            json!({"userId": e.user.id, "what": "chat", "chatId": practice}),
        )
        .await;
    assert_eq!((st, v), (200, json!({"deleted": 1})));
    // the user deletes the other one
    let (st, v) =
        e.t.post("/api/ai/chats/delete", &e.user, json!({"id": summary}))
            .await;
    assert_eq!((st, v), (200, json!({"deleted": 1})));
    let n: i64 = q1(&e.t, "SELECT COUNT(*) FROM messages");
    assert_eq!(n, 0);
    // a record chat asked for again is created again
    let again = start_of(&ask(&e.t, &e.user, hint_body(K1, "q1", 1)).await);
    assert_eq!(again["newChat"], true);
}

#[tokio::test]
async fn retention_deletes_old_study_chats_and_a_deleted_user_takes_them_along() {
    let e = env().await;
    let (practice, summary) = two_study_chats(&e).await;
    e.t.db()
        .execute("UPDATE chats SET updated_at = 1 WHERE id = ?1", [practice])
        .expect("age");
    let n = xexam_ai_service::retention::sweep_once(&e.t.app);
    assert_eq!(n, 1);
    let left: Vec<i64> =
        e.t.db()
            .prepare("SELECT id FROM chats")
            .expect("prepare")
            .query_map([], |r| r.get(0))
            .expect("query")
            .map(|x| x.expect("id"))
            .collect();
    assert_eq!(left, vec![summary]);
    // the user is deleted in the main app: the next request of an account that reuses the id removes the data
    e.t.delete_backend_user(&e.user);
    assert!(e.t.app.0.purge_user(e.user.id).await);
    let n: i64 = q1(&e.t, "SELECT COUNT(*) FROM chats");
    assert_eq!(n, 0);
}

#[tokio::test]
async fn usage_by_purpose_counts_the_four_kinds_in_a_fixed_order() {
    let e = env().await;
    let _ = two_study_chats(&e).await;
    let _ = ask(&e.t, &e.user, explain_body(K1, "q2", 2)).await;
    let _ = ask(&e.t, &e.user, hint_body(K1, "q-leak", 3)).await;
    let _ =
        e.t.send_all(&e.user, json!({"text": "hello", "lang": "en"}))
            .await;
    settle(&e.t).await;
    let (st, v) = e.t.get("/api/ai/admin/usage", &e.admin).await;
    assert_eq!(st, 200, "{v}");
    let rows = v["byPurpose"].as_array().expect("byPurpose");
    let by: Vec<(String, i64, i64)> = rows
        .iter()
        .map(|r| {
            (
                r["purpose"].as_str().expect("purpose").to_string(),
                r["calls"].as_i64().expect("calls"),
                r["errors"].as_i64().expect("errors"),
            )
        })
        .collect();
    // the administrator's own test calls are not chat rows; the user's: 1 chat, 2 hints (one withheld), 1 explanation, 1 summary
    assert_eq!(
        by,
        vec![
            ("chat".to_string(), 1, 0),
            ("hint".to_string(), 2, 1),
            ("explain".to_string(), 1, 0),
            ("summary".to_string(), 1, 0)
        ]
    );
    for r in rows {
        let keys: Vec<&str> = r
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "purpose",
                "calls",
                "tokensIn",
                "tokensOut",
                "errors",
                "tokensCached"
            ]
        );
    }
}
