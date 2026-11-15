//! v1.3 AI generation from the whole source (S6): the corpus pages, the prefix that is byte-identical for every call, the
//! plan call and its local fallback, retries that differ, chunks, the questions shown while the job runs, the activity
//! event. The in-process app, the fake backend's corpus route and a scripted provider. Every text is made up.
mod common;

use common::genkit::*;
use common::*;
use serde_json::{json, Value};
use std::time::Duration;

struct E {
    t: TestApp,
    admin: Who,
    user: Who,
}

async fn env_with(sheets: usize, per_sheet: usize) -> E {
    use std::sync::atomic::Ordering::SeqCst;
    let t = start().await;
    let admin = t.user("root", true, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    t.post(
        "/api/ai/admin/limits",
        &admin,
        json!({"genUnlimited": 0, "importUnlimited": 0}),
    )
    .await;
    let user = t.user("alexandra_k", false, true);
    t.backend.st().gen = kitchen(sheets, per_sheet);
    t.stub.st().script = Some(good_model());
    t.app.0.knobs.backoff_scale.store(0, SeqCst);
    t.app.0.knobs.place_poll_ms.store(20, SeqCst);
    t.relax_caps();
    E { t, admin, user }
}

async fn env() -> E {
    env_with(2, 20).await
}

const BANK: fn() -> Value = || json!({"source": "bank", "bankId": 3});

async fn start_job(e: &E, body: Value) -> (u16, Value) {
    e.t.post("/api/ai/gen/start", &e.user, body).await
}

async fn wait_state(e: &E, id: i64, states: &[&str]) -> Value {
    for _ in 0..800 {
        let (s, v) = e.t.get(&format!("/api/ai/gen/job?id={id}"), &e.user).await;
        assert_eq!(s, 200, "{v}");
        if states.contains(&v["job"]["state"].as_str().unwrap_or("")) {
            return v["job"].clone();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the job never reached {states:?}");
}

async fn run_to_end(e: &E, body: Value) -> Value {
    let (s, v) = start_job(e, body).await;
    assert_eq!(s, 202, "{v}");
    let id = v["job"]["id"].as_i64().expect("id");
    wait_state(e, id, &["done", "failed", "cancelled"]).await
}

fn q1<T: rusqlite::types::FromSql>(t: &TestApp, sql: &str) -> T {
    t.db().query_row(sql, [], |r| r.get(0)).expect(sql)
}

/// The text of a message content: a string, or the text parts of a content-part array joined like the plain form.
fn content_text(c: &Value) -> Option<String> {
    match c {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => Some(
            a.iter()
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n\n"),
        ),
        _ => None,
    }
}

/// (system, user turn) of every generation call the stub got (OpenAI-compatible form), in order.
fn gen_calls(t: &TestApp) -> Vec<(String, String)> {
    t.stub
        .requests()
        .iter()
        .filter_map(|r| {
            let m = r.body["messages"].as_array()?;
            let sys = content_text(&m.first()?["content"])?;
            if !sys.starts_with("You write new multiple-choice exam questions for Xexam") {
                return None;
            }
            Some((sys, content_text(&m.last()?["content"])?))
        })
        .collect()
}

/// The prefix of a call's user turn (up to and including the end fence of the source) and the task after it.
fn split_prefix(user: &str) -> (String, String) {
    let at = user.find("<<<END SOURCE ").expect("a prefix");
    let end = at + user[at..].find(">>>").expect("fence") + 3;
    (user[..end].to_string(), user[end..].to_string())
}

fn task_kind(task: &str) -> &'static str {
    if task.contains("TASK PLAN") {
        "plan"
    } else if task.contains("TASK WRITE") {
        "write"
    } else if task.contains("TASK GLOBAL") {
        "global"
    } else {
        "other"
    }
}

fn set_context(e: &E, tokens: i64) {
    e.t.db()
        .execute("UPDATE models SET context = ?1", [tokens])
        .expect("context");
}

#[tokio::test]
async fn every_call_of_a_job_starts_with_the_same_prefix_that_holds_the_whole_source() {
    let e = env().await;
    let done = run_to_end(&e, BANK()).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["made"], 20);
    let calls = gen_calls(&e.t);
    assert_eq!(calls.len(), 5, "one plan call and four writing calls");
    let parts: Vec<(String, String)> = calls.iter().map(|(_, u)| split_prefix(u)).collect();
    // the plan call goes first, alone; the writing calls follow
    assert_eq!(task_kind(&parts[0].1), "plan");
    assert!(parts[1..].iter().all(|p| task_kind(&p.1) == "write"));
    // the prefix is byte-identical in every call, and so is the system text
    assert!(
        parts.iter().all(|p| p.0 == parts[0].0),
        "one prefix per job"
    );
    assert!(calls.iter().all(|c| c.0 == calls[0].0));
    // never sampled: all 40 source questions are in it, with their three languages
    let nums: Vec<usize> = parts[0]
        .0
        .lines()
        .filter_map(|l| l.strip_prefix('Q'))
        .filter_map(|l| l.split(' ').next()?.parse().ok())
        .collect();
    assert_eq!(nums, (1..=40).collect::<Vec<usize>>());
    assert!(
        parts[0].0.contains("\"zh\":\"")
            && parts[0].0.contains("\"ja\":\"")
            && parts[0].0.contains("\"en\":\"")
    );
    // the corpus was read in one page of at most 200 and asked for with the source selection
    let req = e.t.backend.st().gen.corpus_requests.clone();
    assert_eq!(req.len(), 1);
    assert_eq!(
        (
            req[0]["source"].clone(),
            req[0]["after"].clone(),
            req[0]["limit"].clone()
        ),
        (json!("bank"), json!(0), json!(200))
    );
    // the plan row has its own purpose and the connection of the job
    let (plans, with_conn): (i64, i64) = (
        q1(
            &e.t,
            "SELECT COUNT(*) FROM gen_calls WHERE purpose = 'plan'",
        ),
        q1(
            &e.t,
            "SELECT COUNT(*) FROM gen_calls WHERE connection_id = 1 AND connection <> ''",
        ),
    );
    assert_eq!((plans, with_conn), (1, 5));
    // the question fence of a second job has another nonce: the prefix differs between jobs
    let again = run_to_end(&e, BANK()).await;
    assert_eq!(again["state"], "done", "{again}");
    let all = gen_calls(&e.t);
    let second = split_prefix(&all[5].1).0;
    assert_ne!(second, parts[0].0, "another job has another nonce");
}

#[tokio::test]
async fn the_writing_calls_name_refs_topics_and_angles_from_the_plan() {
    let e = env().await;
    let done = run_to_end(&e, BANK()).await;
    assert_eq!(done["state"], "done", "{done}");
    let calls = gen_calls(&e.t);
    let write = split_prefix(&calls[1].1).1;
    assert!(write.contains("\"refs\":[\"Q") && write.contains("\"angle\":\"a fresh scenario\""));
    assert!(
        write.contains("\"topic\":\"first topic\"") || write.contains("\"topic\":\"second topic\"")
    );
    // the source itself is never repeated after the prefix
    assert!(!write.contains("\"sheet\":"));
}

#[tokio::test]
async fn a_plan_that_cannot_be_used_never_fails_the_job_and_the_retry_differs() {
    let e = env().await;
    let inner = good_model();
    e.t.stub.st().script = Some(std::sync::Arc::new(move |sys: &str, user: &str| {
        let task = user.rsplit_once("<<<END SOURCE").map_or(user, |(_, t)| t);
        if sys.starts_with("You write new multiple-choice") && task.contains("TASK PLAN") {
            return Reply::Text("I would rather not plan.".into());
        }
        inner(sys, user)
    }));
    let done = run_to_end(&e, BANK()).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["made"], 20);
    let calls = gen_calls(&e.t);
    let plans: Vec<&(String, String)> = calls
        .iter()
        .filter(|c| task_kind(&split_prefix(&c.1).1) == "plan")
        .collect();
    assert_eq!(plans.len(), 2, "one differing retry, then the local plan");
    let (p1, t1) = split_prefix(&plans[0].1);
    let (p2, t2) = split_prefix(&plans[1].1);
    assert_eq!(p1, p2, "the cached prefix bytes do not change");
    assert_ne!(t1, t2, "the second send differs");
    let last = t2.lines().last().unwrap_or("");
    let parts: Vec<&str> = last.split(' ').collect();
    assert_eq!(&parts[..2], ["attempt", "2"], "{last}");
    assert_eq!(parts[2].len(), 8);
    // the local plan names the slot's own seed
    let write = calls
        .iter()
        .map(|c| split_prefix(&c.1).1)
        .find(|t| task_kind(t) == "write")
        .expect("write");
    assert!(write.contains("\"refs\":[\"Q") && !write.contains("\"angle\""));
    let rows: i64 = q1(
        &e.t,
        "SELECT COUNT(*) FROM gen_calls WHERE purpose = 'plan'",
    );
    assert_eq!(rows, 2);
}

#[tokio::test]
async fn the_second_send_of_a_batch_differs_and_a_gateway_is_asked_to_skip_its_cache() {
    let e = env().await;
    e.t.seed_route(Seed {
        provider: "gateway",
        url: format!("{}/gw/openai", e.t.stub.url),
        api_key: None,
        gateway_key: Some(GW_KEY),
        gateway_auth: "cf-aig-authorization",
        model: "demo-chat-small",
    });
    e.t.verify_and_enable(&e.admin).await;
    let inner = good_model();
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let s2 = seen.clone();
    // the first writing answer is garbage: its slots go round again
    e.t.stub.st().script = Some(std::sync::Arc::new(move |sys: &str, user: &str| {
        let task = user.rsplit_once("<<<END SOURCE").map_or(user, |(_, t)| t);
        if sys.starts_with("You write new multiple-choice") && task.contains("TASK WRITE") {
            let n = s2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                return Reply::Text("not json".into());
            }
        }
        inner(sys, user)
    }));
    let done = run_to_end(&e, BANK()).await;
    assert_eq!(done["state"], "done", "{done}");
    let reqs: Vec<StubRequest> =
        e.t.stub
            .requests()
            .into_iter()
            .filter(|r| r.path.ends_with("/chat/completions"))
            .filter(|r| {
                r.body["messages"]
                    .as_array()
                    .and_then(|m| m.first())
                    .and_then(|m| content_text(&m["content"]))
                    .is_some_and(|s| {
                        s.starts_with("You write new multiple-choice exam questions for Xexam")
                    })
            })
            .collect();
    let users: Vec<String> = reqs
        .iter()
        .map(|r| {
            r.body["messages"]
                .as_array()
                .and_then(|m| m.last())
                .and_then(|m| content_text(&m["content"]))
                .unwrap_or_default()
        })
        .collect();
    let retries: Vec<usize> = (0..users.len())
        .filter(|&i| {
            users[i]
                .lines()
                .last()
                .is_some_and(|l| l.starts_with("attempt "))
        })
        .collect();
    assert!(
        !retries.is_empty(),
        "the repeated batch carries an attempt line"
    );
    for &i in &retries {
        assert_eq!(reqs[i].header("cf-aig-skip-cache"), Some("true"));
        let (prefix, _) = split_prefix(&users[i]);
        assert_eq!(
            prefix,
            split_prefix(&users[0]).0,
            "the cached prefix is unchanged"
        );
    }
    // the first sends carry no such header
    assert!(reqs
        .iter()
        .enumerate()
        .filter(|(i, _)| !retries.contains(i))
        .all(|(_, r)| r.header("cf-aig-skip-cache").is_none()));
}

#[tokio::test]
async fn a_source_that_does_not_fit_is_cut_into_chunks_with_a_global_plan_and_a_prefix_each() {
    let e = env().await;
    // the answer reserve is 18 000 tokens whatever the settings say: this context leaves room for about half the bank
    set_context(&e, 22_000);
    let (s, v) = start_job(&e, BANK()).await;
    assert_eq!(s, 202, "{v}");
    let id = v["job"]["id"].as_i64().expect("id");
    let done = wait_state(&e, id, &["done", "failed", "cancelled"]).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["made"], 20);
    let (_, j) =
        e.t.get(&format!("/api/ai/gen/job?id={id}&after=0"), &e.user)
            .await;
    let chunks = j["job"]["chunks"].as_i64().expect("chunks");
    assert!(chunks >= 2, "{j}");
    assert!(j["job"]["prefixTokens"].as_i64().unwrap_or(0) > 1000);
    let calls = gen_calls(&e.t);
    // the global plan: no cached prefix, sent first
    assert_eq!(
        task_kind(&calls[0].1),
        "global",
        "{}",
        &calls[0].1[..80.min(calls[0].1.len())]
    );
    assert!(!calls[0].1.contains("<<<END SOURCE"));
    // every chunk has its own prefix (own nonce, own numbering) and its own plan call
    let mut prefixes: Vec<String> = Vec::new();
    for (_, u) in &calls[1..] {
        if u.contains("<<<END SOURCE") {
            let p = split_prefix(u).0;
            if !prefixes.contains(&p) {
                prefixes.push(p);
            }
        }
    }
    assert_eq!(prefixes.len() as i64, chunks);
    let numbers: Vec<Vec<usize>> = prefixes
        .iter()
        .map(|p| {
            p.lines()
                .filter_map(|l| l.strip_prefix('Q'))
                .filter_map(|l| l.split(' ').next()?.parse().ok())
                .collect()
        })
        .collect();
    let mut all: Vec<usize> = numbers.concat();
    all.sort_unstable();
    assert_eq!(
        all,
        (1..=40).collect::<Vec<usize>>(),
        "every question is in exactly one chunk"
    );
    // a writing call never mixes chunks: every ref of a call lies inside its own prefix
    for (_, u) in calls.iter().filter(|(_, u)| u.contains("TASK WRITE")) {
        let (p, task) = split_prefix(u);
        let at = prefixes
            .iter()
            .position(|x| *x == p)
            .expect("a chunk prefix");
        let slots_json = task
            .split_once("SLOTS\n")
            .and_then(|(_, t)| t.lines().next())
            .unwrap_or("[]");
        let slots: Vec<Value> = serde_json::from_str(slots_json).expect("slots");
        for s in &slots {
            for r in s["refs"].as_array().into_iter().flatten() {
                let n: usize = r.as_str().unwrap_or("Q0")[1..].parse().unwrap_or(0);
                assert!(numbers[at].contains(&n), "ref Q{n} outside its chunk");
            }
        }
    }
    let plans: i64 = q1(
        &e.t,
        "SELECT COUNT(*) FROM gen_calls WHERE purpose = 'plan'",
    );
    assert_eq!(
        plans,
        chunks + 1,
        "a plan call per chunk and the global plan"
    );
    // the questions of the finished job are in the created bank (20, from all chunks)
    assert_eq!(
        e.t.backend.st().gen.created[0]["questions"]
            .as_array()
            .map(Vec::len),
        Some(20)
    );
}

#[tokio::test]
async fn a_source_above_the_chunk_ceiling_is_refused_with_its_numbers_and_nothing_is_written() {
    let e = env().await;
    set_context(&e, 22_000);
    let (s, _) =
        e.t.post("/api/ai/admin/limits", &e.admin, json!({"genMaxChunks": 1}))
            .await;
    assert_eq!(s, 200);
    let (s, v) = start_job(&e, BANK()).await;
    assert_eq!((s, &v["code"]), (409, &json!("ai.gen.tooBig")), "{v}");
    assert_eq!(v["limit"], 1);
    assert!(v["chunks"].as_i64().unwrap_or(0) >= 2);
    assert!(v["tokens"].as_i64().unwrap_or(0) > 1000);
    assert_eq!(q1::<i64>(&e.t, "SELECT COUNT(*) FROM tasks"), 0);
    assert!(gen_calls(&e.t).is_empty(), "no model call");
}

#[tokio::test]
async fn the_corpus_is_read_page_by_page_and_a_source_that_changes_is_refused() {
    let e = env_with(2, 60).await; // 120 questions
    e.t.backend.st().gen.corpus_page = 50;
    let done = run_to_end(&e, BANK()).await;
    assert_eq!(done["state"], "done", "{done}");
    let afters: Vec<i64> =
        e.t.backend
            .st()
            .gen
            .corpus_requests
            .iter()
            .map(|r| r["after"].as_i64().unwrap_or(-1))
            .collect();
    assert_eq!(afters, vec![0, 50, 100], "pages of the corpus in order");
    let calls = gen_calls(&e.t);
    let nums = split_prefix(&calls[0].1)
        .0
        .lines()
        .filter(|l| l.starts_with('Q'))
        .count();
    assert_eq!(
        nums, 120,
        "more than the old 100-seed ceiling: nothing is sampled"
    );
    // a bank edited between two pages
    let e2 = env_with(2, 60).await;
    e2.t.backend.st().gen.corpus_page = 50;
    e2.t.backend.st().gen.corpus_drift = true;
    let (s, v) = start_job(&e2, BANK()).await;
    assert_eq!((s, &v["code"]), (404, &json!("ai.gen.notFound")), "{v}");
    assert_eq!(q1::<i64>(&e2.t, "SELECT COUNT(*) FROM tasks"), 0);
}

#[tokio::test]
async fn the_corpus_refusals_map_to_the_start_errors() {
    let e = env().await;
    e.t.backend.st().gen.corpus_refusal = Some("tooMany".into());
    let (s, v) = start_job(&e, BANK()).await;
    assert_eq!((s, &v["code"]), (409, &json!("ai.gen.tooBig")), "{v}");
    e.t.backend.st().gen.corpus_refusal = Some("notAllowed".into());
    let (s, v) = start_job(&e, BANK()).await;
    assert_eq!((s, &v["code"]), (403, &json!("ai.gen.forbidden")), "{v}");
    e.t.backend.st().gen.corpus_refusal = None;
    // fewer than the minimum
    e.t.backend.st().gen.corpus_page = 0;
    let (s, _) =
        e.t.post(
            "/api/ai/admin/limits",
            &e.admin,
            json!({"genMinSource": 50}),
        )
        .await;
    assert_eq!(s, 200);
    let (s, v) = start_job(&e, BANK()).await;
    assert_eq!((s, &v["code"]), (422, &json!("ai.gen.tooFew")), "{v}");
    assert_eq!(q1::<i64>(&e.t, "SELECT COUNT(*) FROM tasks"), 0);
}

#[tokio::test]
async fn the_questions_are_listed_while_the_job_runs_and_removed_when_it_ends() {
    let e = env().await;
    // the create call keeps failing: the job stays in its saving phase for a while with every question written
    e.t.backend.st().gen.create_down = true;
    let (s, v) = start_job(&e, BANK()).await;
    assert_eq!(s, 202, "{v}");
    let id = v["job"]["id"].as_i64().expect("id");
    let mut seen: Option<Value> = None;
    for _ in 0..600 {
        let (s, j) =
            e.t.get(&format!("/api/ai/gen/job?id={id}&after=0&lang=zh"), &e.user)
                .await;
        assert_eq!(s, 200, "{j}");
        if j["job"]["phase"] == "saving" && j["items"].as_array().is_some_and(|a| a.len() == 20) {
            seen = Some(j);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let j = seen.expect("the written questions were listed");
    assert_eq!(j["last"], 20);
    assert_eq!(j["gone"], json!([]));
    assert_eq!(
        (
            j["job"]["chunks"].clone(),
            j["job"]["made"].clone(),
            j["job"]["goal"].clone()
        ),
        (json!(1), json!(20), json!(20))
    );
    let first = &j["items"][0];
    assert_eq!(first["no"], 1);
    assert!(first["kind"] == "single" || first["kind"] == "multi");
    let stem = first["stem"].as_str().unwrap_or("");
    assert!(
        stem.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c)),
        "texts in the asked language: {stem}"
    );
    assert!(first["options"].as_array().is_some_and(|o| o.len() >= 4));
    assert!(first["options"][0]["text"].is_string() && first["options"][0]["correct"].is_boolean());
    // the cursor: only newer questions come, 20 at most per answer
    let (_, j2) =
        e.t.get(&format!("/api/ai/gen/job?id={id}&after=15"), &e.user)
            .await;
    let nos: Vec<i64> = j2["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|i| i["no"].as_i64())
        .collect();
    assert_eq!(nos, vec![16, 17, 18, 19, 20]);
    let (_, j3) =
        e.t.get(&format!("/api/ai/gen/job?id={id}&after=20"), &e.user)
            .await;
    assert_eq!(
        (j3["items"].clone(), j3["last"].clone()),
        (json!([]), json!(20))
    );
    // the rows are written in the same transaction as their call row: one row per accepted question
    assert_eq!(q1::<i64>(&e.t, "SELECT COUNT(*) FROM gen_items"), 20);
    // the job fails at the save (about ten seconds), and the rows go with it
    let end = wait_state(&e, id, &["failed", "done"]).await;
    assert_eq!(end["error"], "ai.gen.save", "{end}");
    for _ in 0..100 {
        if q1::<i64>(&e.t, "SELECT COUNT(*) FROM gen_items") == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(q1::<i64>(&e.t, "SELECT COUNT(*) FROM gen_items"), 0);
}

fn insert_job(e: &E, user_id: i64, key: &str) -> i64 {
    use xexam_ai_service::store::{data, jobs};
    let now = xexam_ai_service::util::unix_now();
    e.t.app
        .0
        .store
        .write(|tx| {
            let who = identity_of(&e.user);
            data::ensure_user(tx, &who, now)?;
            let payload = json!({"name": "Kitchen basics_x", "source": "bank", "chunks": 3, "prefixTokens": 9000});
            match jobs::insert(
                tx,
                &jobs::NewJob {
                    user_id,
                    job_key: key,
                    goal: 10,
                    payload: &payload,
                    charged: true,
                    phase: Some("writing"),
                    now,
                },
            ) {
                Ok(id) => Ok(id),
                Err(_) => Err(rusqlite::Error::InvalidQuery),
            }
        })
        .expect("job")
}

fn identity_of(w: &Who) -> xexam_ai_service::identity::Identity {
    use xexam_ai_service::identity::{Identity, Role};
    Identity {
        id: w.id,
        stamp: w.stamp.clone(),
        username: w.username.clone(),
        role: if w.admin { Role::Admin } else { Role::User },
        ai_allowed: w.ai || w.admin,
        sid: w.sid.clone(),
    }
}

#[tokio::test]
async fn the_job_route_serves_the_owner_only_with_language_fallback_gone_numbers_and_a_bad_query_refused(
) {
    use xexam_ai_service::store::items;
    let e = env().await;
    let id = insert_job(&e, e.user.id, &"a".repeat(32));
    let now = xexam_ai_service::util::unix_now();
    let data = |en: &str, zh: &str| {
        json!({
            "kind": "single",
            "stem": {"en": en, "zh": zh, "ja": ""},
            "options": [
                {"en": "first", "zh": "", "ja": "", "correct": false},
                {"en": "second", "zh": "第二", "ja": "", "correct": true}
            ],
            "explanation": {"en": "because", "zh": "", "ja": ""},
        })
        .to_string()
    };
    e.t.app
        .0
        .store
        .write(|tx| -> rusqlite::Result<()> {
            for k in 1..=25 {
                items::items_add(
                    tx,
                    id,
                    k,
                    &data(&format!("stem {k}"), &format!("题 {k}")),
                    now,
                )?;
            }
            items::items_remove(tx, id, 3)?;
            Ok(())
        })
        .expect("items");
    // without `after`: today's answer, no items
    let (s, v) = e.t.get(&format!("/api/ai/gen/job?id={id}"), &e.user).await;
    assert_eq!(s, 200);
    assert!(v.get("items").is_none() && v["job"]["id"] == id);
    // with `after`: 20 items at most, tombstones skipped and listed as gone, texts in the asked language
    let (s, v) =
        e.t.get(&format!("/api/ai/gen/job?id={id}&after=0&lang=zh"), &e.user)
            .await;
    assert_eq!(s, 200, "{v}");
    let list = v["items"].as_array().expect("items");
    assert_eq!(list.len(), 20);
    assert_eq!(list[0]["no"], 1);
    assert_eq!(list[2]["no"], 4, "number 3 was dropped");
    assert_eq!(v["last"], 21);
    assert_eq!(v["gone"], json!([3]));
    assert_eq!(list[0]["stem"], "题 1");
    assert_eq!(
        list[0]["options"][0]["text"], "first",
        "a missing language falls back to English"
    );
    assert_eq!(list[0]["options"][1]["text"], "第二");
    assert_eq!(list[0]["options"][1]["correct"], true);
    assert_eq!(list[0]["explanation"], "because");
    assert_eq!(
        (v["job"]["chunks"].clone(), v["job"]["prefixTokens"].clone()),
        (json!(3), json!(9000))
    );
    assert_eq!(v["job"]["phase"], "writing");
    // another user's job is a missing one
    let other = e.t.user("bo_other", false, true);
    let (s, v) =
        e.t.get(&format!("/api/ai/gen/job?id={id}&after=0"), &other)
            .await;
    assert_eq!((s, &v["code"]), (404, &json!("ai.gen.notFound")));
    let (s, v) =
        e.t.get(&format!("/api/ai/gen/job?id={id}&after=0"), &e.admin)
            .await;
    assert_eq!(
        (s, &v["code"]),
        (404, &json!("ai.gen.notFound")),
        "item text is the owner's alone"
    );
    // bad queries
    for q in [
        "after=x",
        "after=",
        "after=-1",
        "after=0&lang=fr",
        "after=0&lang=",
    ] {
        let (s, v) =
            e.t.get(&format!("/api/ai/gen/job?id={id}&{q}"), &e.user)
                .await;
        assert_eq!((s, &v["code"]), (400, &json!("ai.badRequest")), "{q}");
    }
    // the administrators' list never carries item text
    let (s, v) = e.t.get("/api/ai/admin/gen?limit=10", &e.admin).await;
    assert_eq!(s, 200);
    assert!(!v.to_string().contains("stem 1"));
}

#[tokio::test]
async fn a_cancel_during_the_writing_removes_the_listed_questions() {
    let e = env().await;
    e.t.stub.st().head_delay_ms = 600;
    let (s, v) = start_job(&e, BANK()).await;
    assert_eq!(s, 202, "{v}");
    let id = v["job"]["id"].as_i64().expect("id");
    // wait for the plan call to be answered and the writing to start
    for _ in 0..400 {
        let (_, j) = e.t.get(&format!("/api/ai/gen/job?id={id}"), &e.user).await;
        if j["job"]["phase"] == "writing" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let (s, v) =
        e.t.post("/api/ai/gen/cancel", &e.user, json!({"id": id}))
            .await;
    assert_eq!(s, 200, "{v}");
    for _ in 0..200 {
        if q1::<i64>(&e.t, "SELECT COUNT(*) FROM gen_items") == 0
            && q1::<i64>(
                &e.t,
                "SELECT COUNT(*) FROM gen_calls WHERE status = 'stopped'",
            ) >= 1
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(q1::<i64>(&e.t, "SELECT COUNT(*) FROM gen_items"), 0);
    assert!(e.t.backend.st().gen.created.is_empty());
}

#[tokio::test]
async fn a_finished_job_leaves_one_activity_event_with_the_bank_name_and_the_count() {
    let e = env().await;
    let done = run_to_end(&e, BANK()).await;
    assert_eq!(done["state"], "done", "{done}");
    assert!(e.t.app.0.deliver_outbox().await >= 1);
    let st = e.t.backend.st();
    let ev: Vec<&Value> = st
        .activity
        .iter()
        .filter(|a| a["action"] == "ai.gen.create")
        .collect();
    assert_eq!(ev.len(), 1, "{:?}", st.activity);
    assert_eq!(ev[0]["object"], done["name"]);
    assert_eq!(ev[0]["detail"]["questions"], 20);
    assert_eq!(ev[0]["username"], "alexandra_k");
    assert!(ev[0]["reason"].is_null() || ev[0]["result"] == "ok");
    // no question text, plan text or source text travels in the event
    assert!(!ev[0].to_string().contains("\"stem\""));
}

#[tokio::test]
async fn a_failed_job_leaves_a_failed_event_with_its_code() {
    let e = env().await;
    e.t.stub.st().script = Some(std::sync::Arc::new(|_, _| {
        Reply::Status(500, "boom".into())
    }));
    let done = run_to_end(&e, BANK()).await;
    assert_eq!(done["state"], "failed", "{done}");
    assert!(e.t.app.0.deliver_outbox().await >= 1);
    let st = e.t.backend.st();
    let ev: Vec<&Value> = st
        .activity
        .iter()
        .filter(|a| a["action"] == "ai.gen.create")
        .collect();
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0]["result"], "fail");
    assert_eq!(ev[0]["reason"], "ai.up.server");
}

#[tokio::test]
async fn the_goal_may_be_three_hundred_questions_for_a_file_import_plan_and_a_generation() {
    // the planner takes goals up to 300 (v1.3), the same figure as the import's per-file cap
    assert_eq!(xexam_ai_service::gen::MAX_GOAL, 300);
}

#[tokio::test]
async fn a_cache_floor_decides_whether_markers_are_sent_for_a_generation() {
    // an unknown model gets the largest minimum (4096 tokens): a small source (the rules and 6 questions) is below it
    let e = env_with(1, 6).await;
    let (s, v) =
        e.t.post("/api/ai/admin/limits", &e.admin, json!({"genMinSource": 5}))
            .await;
    assert_eq!(s, 200, "{v}");
    e.t.configure_anthropic(&e.admin, "demo-claude").await;
    let done = run_to_end(&e, BANK()).await;
    assert_eq!(done["state"], "done", "{done}");
    let bodies: Vec<Value> =
        e.t.stub
            .requests()
            .iter()
            .filter(|r| r.path.contains("/anthropic/"))
            .filter(|r| {
                r.body["system"][0]["text"].as_str().is_some_and(|s| {
                    s.starts_with("You write new multiple-choice exam questions for Xexam")
                })
            })
            .map(|r| r.body.clone())
            .collect();
    assert!(bodies.len() >= 5, "{}", bodies.len());
    for b in &bodies {
        assert!(
            !b.to_string().contains("cache_control"),
            "below the minimum: no marker"
        );
    }
    // a Sonnet model caches from 1024 tokens: the system text and the 40 questions are above it
    let e2 = env().await;
    e2.t.configure_anthropic(&e2.admin, "demo-sonnet").await;
    let done = run_to_end(&e2, BANK()).await;
    assert_eq!(done["state"], "done", "{done}");
    let bodies: Vec<Value> =
        e2.t.stub
            .requests()
            .iter()
            .filter(|r| r.path.contains("/anthropic/"))
            .filter(|r| {
                r.body["system"][0]["text"].as_str().is_some_and(|s| {
                    s.starts_with("You write new multiple-choice exam questions for Xexam")
                })
            })
            .map(|r| r.body.clone())
            .collect();
    assert!(bodies.len() >= 5);
    for b in &bodies {
        let c = &b["messages"][0]["content"];
        assert_eq!(
            c[0]["cache_control"],
            json!({"type": "ephemeral"}),
            "the prefix carries the breakpoint"
        );
        assert!(c[0]["text"]
            .as_str()
            .unwrap_or("")
            .starts_with("SOURCE BANK\n<<<SOURCE "));
        assert!(
            c[1].get("cache_control").is_none(),
            "the task after it does not"
        );
        assert_eq!(c[0]["text"], bodies[0]["messages"][0]["content"][0]["text"]);
    }
}
