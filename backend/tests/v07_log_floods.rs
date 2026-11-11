//! v0.7: what anyone who can reach the server can make it log without signing in (failed sign-ins under made-up names, locks of
//! usernames without an account, requests rejected by the Host / Origin guard) is summed up instead of logged one line each,
//! so a flood cannot push the other lines out of the System log; and those lines are English. This binary holds only this
//! test, so it can count the lines of the process-wide log capture without other tests writing into it. Made-up data only.
#[path = "api_common/mod.rs"]
mod apic;

use apic::{build, send};
use axum::body::Body;
use serde_json::json;
use std::sync::Arc;
use xexam_backend::api::build_router;
use xexam_backend::logs::{BufferLayer, LogBuffer};

const HOST: &str = "127.0.0.1:8765";
const ORIGIN: &str = "http://127.0.0.1:8765";

async fn login(router: &axum::Router, name: &str, origin: &str) -> u16 {
    let body = json!({"username": name, "password": "wrong-password-x"}).to_string();
    let len = body.len().to_string();
    send(
        router,
        build(
            "POST",
            "/api/auth/login",
            Some(HOST),
            &[
                ("origin", origin.as_bytes()),
                ("content-type", b"application/json"),
                ("content-length", len.as_bytes()),
            ],
            Body::from(body),
        ),
    )
    .await
    .status
    .as_u16()
}

/// Sends the sign-ins of `names` (one each), eight at a time; returns how many got each status.
async fn login_all(
    router: &axum::Router,
    names: Vec<String>,
) -> std::collections::BTreeMap<u16, usize> {
    let mut out = std::collections::BTreeMap::new();
    for chunk in names.chunks(8) {
        let mut set = tokio::task::JoinSet::new();
        for name in chunk {
            let (r, n) = (router.clone(), name.clone());
            set.spawn(async move { login(&r, &n, ORIGIN).await });
        }
        while let Some(s) = set.join_next().await {
            *out.entry(s.expect("task")).or_insert(0) += 1;
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn floods_anyone_can_cause_without_signing_in_are_summed_up_in_english() {
    use tracing_subscriber::layer::SubscriberExt;
    let buf = Arc::new(LogBuffer::new(100_000));
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry().with(BufferLayer::new(buf.clone())),
    )
    .expect("the only global subscriber");
    let messages = |from: u64| -> Vec<String> {
        buf.since(Some(from))
            .lines
            .into_iter()
            .map(|l| l.message)
            .collect()
    };
    let a = apic::app();
    let router = build_router(a.state.clone());

    // 1. 300 failed sign-ins, each under its own made-up name: the throttle refuses none of them (every name has its own
    //    bucket and none fails twice), and the log gets one line for the first, not 300
    let start = buf.since(None).last;
    let names: Vec<String> = (0..300).map(|i| format!("nobody{i:04x}")).collect();
    let statuses = login_all(&router, names).await;
    assert_eq!(statuses.get(&401), Some(&300), "{statuses:?}");
    let lines = messages(start);
    // v0.7: never the typed name (it may be a password typed into the wrong field)
    assert_eq!(
        lines,
        ["failed sign-in with a username that has no account"],
        "{lines:?}"
    );

    // 2. Twenty made-up names, each guessed until its lock starts: one lock line, not twenty (locks of accounts stay one line
    //    each, see v06_users_and_sign_in). A fresh server: the 300 failures above put the throttle under pressure, which would
    //    hold back a name's second guess for a minute. Its first failed sign-in is logged, the other 99 wait for the summary.
    // (the app is kept until the end: it owns the temporary directory of its store)
    let locks_app = apic::app();
    let locks = build_router(locks_app.state.clone());
    let start = buf.since(None).last;
    for _round in 0..5 {
        let names: Vec<String> = (0..20).map(|i| format!("ghost{i:02}")).collect();
        let statuses = login_all(&locks, names).await;
        assert_eq!(statuses.get(&401), Some(&20), "{statuses:?}");
    }
    let lines = messages(start);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(
        lines[0],
        "failed sign-in with a username that has no account"
    );
    assert!(
        lines[1].starts_with(
            "sign-in of a username without an account from other browsers locked for 60 s"
        ),
        "{}",
        lines[1]
    );

    // 3. A sign-in from a foreign Origin: rejected by the guard, and the log line describes why in English (the Chinese reason
    //    is only the text of the 403, which the frontend translates)
    let start = buf.since(None).last;
    assert_eq!(login(&router, "admin", "http://evil.example").await, 403);
    let lines = messages(start);
    assert_eq!(
        lines,
        ["rejected POST /api/auth/login: Origin host \"evil.example\" differs from the Host \"127.0.0.1\" and is not in the allowed origins"],
        "{lines:?}"
    );
    // within the minute, further rejections wait for the summary
    let start = buf.since(None).last;
    assert_eq!(login(&router, "admin", "http://evil.example").await, 403);
    assert_eq!(messages(start), Vec::<String>::new());
    // another method (the Host check only), on a fresh server so that its first rejection is logged at once
    let fresh_app = apic::app();
    let fresh = build_router(fresh_app.state.clone());
    let start = buf.since(None).last;
    let r = send(
        &fresh,
        build("PUT", "/api/info", Some("evil.example"), &[], Body::empty()),
    )
    .await;
    assert_eq!(r.status, 403);
    assert!(
        r.text().contains("被拒绝"),
        "the response keeps its message"
    );
    assert_eq!(
        messages(start),
        ["rejected PUT /api/info: Host \"evil.example\" is not in the allowed list"]
    );
    let all = messages(0);
    assert!(all.iter().all(|m| m.is_ascii()), "{all:?}");
}
