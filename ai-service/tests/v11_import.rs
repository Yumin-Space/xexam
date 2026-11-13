//! v1.1 AI file import (IJ-1 … IJ-5, API-U6 / U7, T-IM-AI-1 … 10): the in-process app, the real extraction child (the
//! service's own binary), the fake backend and a scripted provider. Every file and text is made up and generated here.
mod common;

use common::genkit::*;
use common::*;
use serde_json::{json, Value};
use std::sync::atomic::Ordering::SeqCst;
use std::sync::Arc;
use std::time::Duration;
use xexam_ai_service::extract::testgen::{self, DocxSpec};

struct E {
    t: TestApp,
    admin: Who,
    user: Who,
}

async fn env() -> E {
    let t = start().await;
    let admin = t.user("root", true, true);
    t.configure_openai(&admin, "demo-chat-small").await;
    // these tests exercise the call ceilings, so the v1.1 default (unlimited on) is switched off
    t.post(
        "/api/ai/admin/limits",
        &admin,
        json!({"genUnlimited": 0, "importUnlimited": 0}),
    )
    .await;
    let user = t.user("alexandra_k", false, true);
    t.backend.st().gen = kitchen(2, 20);
    t.stub.st().script = Some(good_model());
    t.app.0.knobs.backoff_scale.store(0, SeqCst);
    t.app.0.knobs.place_poll_ms.store(20, SeqCst);
    // the windowed path (v1.1) is what most of these tests exercise: a model context too small for any file; the
    // whole-file tests at the end use `env_whole`
    t.app.0.knobs.import_context_tokens.store(1, SeqCst);
    *t.app.0.knobs.exe_override.lock().unwrap() =
        Some(env!("CARGO_BIN_EXE_xexam-ai-service").into());
    t.relax_caps();
    E { t, admin, user }
}

fn docx_of(lines: &[String]) -> Vec<u8> {
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    testgen::docx(&DocxSpec::paragraphs(&refs))
}

/// Six questions: four with options (answer B), one without options, one true / false (answer True).
fn six() -> Vec<String> {
    let mut v = vec!["KIND:QUESTIONS zebrafishmarker notes".to_string()];
    for i in 1..=4 {
        v.push(format!(
            "Q{i}. Which kitchen device {} keeps a stock pot at a steady simmer?",
            en_text(300 + i, 3)
        ));
        for (k, l) in ["A", "B", "C", "D"].iter().enumerate() {
            v.push(format!(
                "{l}. {} {}",
                pw(500 + i * 10 + k),
                pw(600 + i * 10 + k)
            ));
        }
        v.push("Answer: B".to_string());
    }
    v.push("Q5. [open] Describe the device that keeps a stock pot at a steady simmer.".to_string());
    v.push("Q6. [tf] The pot lid keeps the heat in the pot while it simmers.".to_string());
    v.push("Answer: True".to_string());
    v
}

async fn job_of(t: &TestApp, who: &Who, id: i64) -> Value {
    let (s, v) = t.get(&format!("/api/ai/gen/job?id={id}"), who).await;
    assert_eq!(s, 200, "{v}");
    v["job"].clone()
}

/// Polls until the job's phase (while active) or state matches.
async fn wait_for(t: &TestApp, who: &Who, id: i64, phase: Option<&str>, states: &[&str]) -> Value {
    for _ in 0..800 {
        let j = job_of(t, who, id).await;
        let state = j["state"].as_str().unwrap_or("");
        if states.contains(&state) || (phase.is_some() && j["phase"].as_str() == phase) {
            return j;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let j = job_of(t, who, id).await;
    panic!("the job never reached {phase:?} / {states:?}: {j}");
}

async fn upload_ok(e: &E, who: &Who, name: &str, bytes: Vec<u8>) -> i64 {
    let (s, v) = e.t.upload(who, name, bytes).await;
    assert_eq!(s, 202, "{v}");
    assert_eq!(v["job"]["source"], "file");
    v["job"]["id"].as_i64().expect("id")
}

fn q1<T: rusqlite::types::FromSql>(t: &TestApp, sql: &str) -> T {
    t.db().query_row(sql, [], |r| r.get(0)).expect(sql)
}

fn tasks(t: &TestApp) -> i64 {
    q1(t, "SELECT COUNT(*) FROM tasks")
}

/// The bank name the tests type at the confirm step
const TYPED_NAME: &str = "Kitchen notes (typed)";

async fn confirm(e: &E, who: &Who, id: i64) -> (u16, Value) {
    e.t.post(
        "/api/ai/import/confirm",
        who,
        json!({"id": id, "name": TYPED_NAME}),
    )
    .await
}

fn names(dir: &std::path::Path) -> Vec<String> {
    fn walk(d: &std::path::Path, out: &mut Vec<String>) {
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else {
                    let n = p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();
                    if !n.starts_with("xexam-ai.db") {
                        out.push(p.display().to_string());
                    }
                }
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, &mut v);
    v.sort();
    v
}

// —— T-IM-AI-1 ————————————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn the_refusals_of_the_upload_in_their_order_write_nothing() {
    let e = env().await;
    let ok_file = docx_of(&six());
    // content type, then the identity-based refusals
    let (s, v) =
        e.t.upload_with(&e.user, "a.docx", ok_file.clone(), "application/json", &[])
            .await;
    assert_eq!((s, &v["code"]), (415, &json!("ai.badRequest")));
    let plain = e.t.user("plain", false, false);
    let (s, v) = e.t.upload(&plain, "a.docx", ok_file.clone()).await;
    assert_eq!((s, &v["code"]), (403, &json!("ai.notAllowed")));
    let (s, _) =
        e.t.post(
            "/api/ai/admin/suspend",
            &e.admin,
            json!({"userId": e.user.id}),
        )
        .await;
    assert_eq!(s, 200);
    let (s, v) = e.t.upload(&e.user, "a.docx", ok_file.clone()).await;
    assert_eq!((s, &v["code"]), (403, &json!("ai.suspended")));
    e.t.post(
        "/api/ai/admin/resume",
        &e.admin,
        json!({"userId": e.user.id}),
    )
    .await;
    // the feature `import` off while `generate` is on
    e.t.post(
        "/api/ai/admin/features",
        &e.admin,
        json!({"userId": e.user.id, "features": {"chat": true, "hint": true, "explain": true, "summary": true, "generate": true, "import": false}}),
    )
    .await;
    let (s, v) = e.t.upload(&e.user, "a.docx", ok_file.clone()).await;
    assert_eq!(
        (s, &v["code"], &v["feature"]),
        (403, &json!("ai.featureOff"), &json!("import"))
    );
    e.t.post(
        "/api/ai/admin/features",
        &e.admin,
        json!({"userId": e.user.id, "features": {"chat": true, "hint": true, "explain": true, "summary": true, "generate": true, "import": true}}),
    )
    .await;
    // file types
    let (s, v) = e.t.upload(&e.user, "old.doc", b"x".to_vec()).await;
    assert_eq!(
        (s, &v["code"], &v["ext"]),
        (415, &json!("ai.import.legacy"), &json!("doc"))
    );
    let (s, v) = e.t.upload(&e.user, "picture.PNG", b"x".to_vec()).await;
    assert_eq!((s, &v["code"]), (415, &json!("ai.import.image")));
    let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
    let (s, v) = e.t.upload(&e.user, "scan.pdf", png.to_vec()).await;
    assert_eq!(
        (s, &v["code"]),
        (415, &json!("ai.import.image")),
        "content beats the name"
    );
    let (s, v) = e.t.upload(&e.user, "program.exe", b"x".to_vec()).await;
    assert_eq!(
        (s, &v["code"], &v["ext"]),
        (415, &json!("ai.import.type"), &json!("exe"))
    );
    // size and emptiness (the limit is a setting)
    let (s, v) = e.t.upload(&e.user, "empty.txt", Vec::new()).await;
    assert_eq!((s, &v["code"]), (400, &json!("ai.import.empty")));
    e.t.post("/api/ai/admin/limits", &e.admin, json!({"importMaxMb": 1}))
        .await;
    let (s, v) =
        e.t.upload(&e.user, "big.txt", vec![b'a'; (1 << 20) + 1])
            .await;
    assert_eq!(
        (s, &v["code"], &v["maxMb"]),
        (413, &json!("ai.import.tooLarge"), &json!(1))
    );
    e.t.post("/api/ai/admin/limits", &e.admin, json!({"importMaxMb": 20}))
        .await;
    // a missing X-Filename
    let r =
        e.t.raw(
            axum::http::Method::POST,
            "/api/ai/import/upload",
            Some(e.t.ident(&e.user)),
            Some(ok_file.clone()),
            Some("application/octet-stream"),
        )
        .await;
    assert_eq!(r.status().as_u16(), 400);
    // the backend says no
    e.t.backend.st().gen.check_refusal = Some("notAllowed".into());
    let (s, v) = e.t.upload(&e.user, "a.docx", ok_file.clone()).await;
    assert_eq!((s, &v["code"]), (403, &json!("ai.gen.forbidden")));
    e.t.backend.st().gen.check_refusal = None;
    // quota used up
    e.t.post("/api/ai/admin/limits", &e.admin, json!({"genPerDay": 0}))
        .await;
    let (s, v) = e.t.upload(&e.user, "a.docx", ok_file.clone()).await;
    assert_eq!((s, &v["code"]), (429, &json!("ai.gen.quota")));
    e.t.post("/api/ai/admin/limits", &e.admin, json!({"genPerDay": 3}))
        .await;
    // the memory places for pending uploads are all taken
    {
        let _held =
            e.t.app
                .0
                .import_pending
                .clone()
                .try_acquire_many_owned(4)
                .expect("permits");
        let (s, v) = e.t.upload(&e.user, "a.docx", ok_file.clone()).await;
        assert_eq!((s, &v["code"]), (503, &json!("ai.serverBusy")));
    }
    assert_eq!(tasks(&e.t), 0, "no refusal wrote a row");
    assert!(e.t.app.0.jobs.is_empty());
    // busy with a running generation
    e.t.stub.st().head_delay_ms = 500;
    let (s, _) =
        e.t.post(
            "/api/ai/gen/start",
            &e.user,
            json!({"source": "bank", "bankId": 3}),
        )
        .await;
    assert_eq!(s, 202);
    let (s, v) = e.t.upload(&e.user, "a.docx", ok_file).await;
    assert_eq!((s, &v["code"]), (409, &json!("ai.gen.busy")));
    assert_eq!(v["job"]["source"], "bank");
    e.t.stub.st().head_delay_ms = 0;
}

#[tokio::test]
async fn a_second_parallel_upload_of_the_same_user_is_busy_with_a_null_job() {
    let e = env().await;
    let t = &e.t;
    // hold the first upload in the backend check (the guard is taken before it)
    t.backend.set_mode(Mode::Hang);
    let file = docx_of(&six());
    let first = {
        let (t2, user2, f2) = (t.router(), e.user.clone(), file.clone());
        let ident = t.ident(&user2);
        tokio::spawn(async move {
            use tower::ServiceExt;
            let req = axum::http::Request::builder()
                .method("POST")
                .uri("/api/ai/import/upload")
                .header("x-xexam-identity", ident)
                .header("content-type", "application/octet-stream")
                .header("content-length", f2.len())
                .header("x-filename", "a.docx")
                .body(axum::body::Body::from(f2))
                .expect("request");
            let _ = t2.oneshot(req).await;
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (s, v) = t.upload(&e.user, "b.docx", file).await;
    assert_eq!(
        (s, &v["code"], &v["job"]),
        (409, &json!("ai.gen.busy"), &Value::Null)
    );
    first.abort();
    t.backend.set_mode(Mode::Normal);
}

// —— T-IM-AI-2 questions flow, T-IM-AI-10 nothing stored ——————————————————————————————————————————————————————————————

#[tokio::test]
async fn the_questions_flow_recognises_waits_for_the_confirm_converts_and_creates_the_bank() {
    let e = env().await;
    let before = names(e.t.dir.path());
    let id = upload_ok(&e, &e.user, "Kitchen notes (v2).docx", docx_of(&six())).await;
    let j = wait_for(&e.t, &e.user, id, Some("confirm"), &["failed", "cancelled"]).await;
    assert_eq!(j["phase"], "confirm", "{j}");
    assert_eq!(
        (j["mode"].clone(), j["goal"].clone(), j["charged"].clone()),
        (json!("questions"), json!(6), json!(false))
    );
    assert_eq!(j["file"]["name"], "Kitchen notes (v2).docx");
    assert_eq!(j["file"]["type"], "docx");
    assert_eq!(j["read"], json!({"done": 1, "total": 1}));
    let now = xexam_ai_service::util::unix_now();
    let by = j["confirmBy"].as_i64().expect("confirmBy");
    assert!((by - now - 1800).abs() < 10, "30 minutes");
    assert!(j["name"]
        .as_str()
        .unwrap_or("")
        .starts_with("Kitchen notes (v2)_alexandr_"));
    // nothing is charged and nothing was written to the main service before the confirm
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(v["gen"]["used"], 0);
    assert!(e.t.backend.st().gen.created.is_empty());
    // the recognising calls are outside the per-user call caps (A-OWNER-56); one window: the numbered questions decide
    // the type, so there is no classification call
    assert_eq!(v["calls"]["used"]["day"], 0);
    let recognise: i64 = q1(
        &e.t,
        "SELECT COUNT(*) FROM gen_calls WHERE purpose = 'recognise'",
    );
    assert_eq!(recognise, 1);
    assert_eq!(
        v["gen"]["job"]["id"], id,
        "the status carries the waiting job"
    );
    // confirm
    let (s, v) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        (v["job"]["charged"].clone(), v["gen"]["used"].clone()),
        (json!(true), json!(1))
    );
    let done = wait_for(&e.t, &e.user, id, None, &["done", "failed"]).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(
        (
            done["made"].clone(),
            done["goal"].clone(),
            done["skipped"].clone()
        ),
        (json!(6), json!(6), json!([]))
    );
    assert!(done["bankId"].as_i64().is_some());
    // the create body of API-I3a; the bank carries the name typed at the confirm step (A-OWNER-57)
    let created = e.t.backend.st().gen.created.clone();
    let body = created.first().expect("create");
    assert_eq!(body["name"], TYPED_NAME);
    let keys: Vec<&str> = body
        .as_object()
        .expect("obj")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "job",
            "user",
            "stamp",
            "name",
            "source",
            "from",
            "file",
            "mode",
            "skipped",
            "goal",
            "at",
            "sheet",
            "questions"
        ]
    );
    assert_eq!(
        (
            body["source"].clone(),
            body["from"].clone(),
            body["mode"].clone()
        ),
        (json!("file"), json!([]), json!("questions"))
    );
    assert_eq!(
        body["file"],
        json!({"name": "Kitchen notes (v2).docx", "type": "docx"})
    );
    assert_eq!(
        (body["goal"].clone(), body["skipped"].clone()),
        (json!(6), json!([]))
    );
    let qs = body["questions"].as_array().expect("questions");
    assert_eq!(qs.len(), 6);
    // kept options keep their words and their order; the stated answer B is the one marked
    let first = &qs[0];
    assert_eq!(first["options"].as_array().map(Vec::len), Some(4));
    assert_eq!(first["answer"], json!(["B"]));
    assert!(
        first["options"][0]["en"]
            .as_str()
            .unwrap_or("")
            .contains(&pw(510)),
        "{first}"
    );
    // the open question got four made options and one planned correct letter
    let open = &qs[4];
    assert_eq!(open["options"].as_array().map(Vec::len), Some(4));
    assert_eq!(open["answer"].as_array().map(Vec::len), Some(1));
    // the true / false question has the two fixed options, answer A (True)
    let tf = &qs[5];
    assert_eq!(
        tf["options"][0],
        json!({"en": "True", "zh": "正确", "ja": "正しい"})
    );
    assert_eq!(
        tf["options"][1],
        json!({"en": "False", "zh": "错误", "ja": "誤り"})
    );
    assert_eq!(tf["answer"], json!(["A"]));

    // the writing calls are import rows outside the caps
    let writes: i64 = q1(
        &e.t,
        "SELECT COUNT(*) FROM gen_calls WHERE purpose = 'import'",
    );
    assert!(writes >= 2, "{writes}");
    // A-OWNER-67: the job object carries the exact sums of its own call rows (recognition included)
    let (all_calls, all_in, all_out): (i64, i64, i64) = e
        .t
        .db()
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(tokens_in), 0), COALESCE(SUM(tokens_out), 0) FROM gen_calls WHERE task_id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("sums");
    assert!(all_calls > writes, "{all_calls}");
    let job = job_of(&e.t, &e.user, id).await;
    assert_eq!(
        job["usage"],
        json!({"calls": all_calls, "tokensIn": all_in, "tokensOut": all_out, "tokensCached": 0, "tokensCacheWrite": 0})
    );
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(
        v["calls"]["used"]["day"], 0,
        "no import call counts against the caps"
    );
    // T-IM-AI-10: the file never touched the disk and no text was stored or logged
    assert_eq!(names(e.t.dir.path()), before);
    let c = e.t.db();
    let tables: Vec<String> = {
        let mut st = c
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
            .expect("tables");
        st.query_map([], |r| r.get(0))
            .expect("rows")
            .map(|r| r.expect("name"))
            .collect()
    };
    for t in &tables {
        let cols: Vec<String> = {
            let mut st = c.prepare(&format!("SELECT name FROM pragma_table_info('{t}') WHERE type LIKE 'TEXT%' OR type = ''")).expect("cols");
            st.query_map([], |r| r.get(0))
                .expect("rows")
                .map(|r| r.expect("col"))
                .collect()
        };
        for col in cols {
            let n: i64 = c
                .query_row(
                    &format!("SELECT COUNT(*) FROM {t} WHERE {col} LIKE '%zebrafishmarker%'"),
                    [],
                    |r| r.get(0),
                )
                .expect("count");
            assert_eq!(n, 0, "{t}.{col} holds file text");
        }
    }
    for line in captured().iter().chain(e.t.backend.logs().iter()) {
        assert!(
            !line.contains("Kitchen notes"),
            "the file name is in a log line: {line}"
        );
        assert!(
            !line.contains("zebrafishmarker"),
            "file text in a log line: {line}"
        );
    }
    assert!(captured_contains("started an AI file import (docx,"));
    assert!(captured_contains(
        "AI file import recognised (questions, 6)"
    ));
    assert!(captured_contains(
        "finished an AI file import (6 of 6 questions,"
    ));
}

// —— T-IM-AI-3 ——————————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn a_material_file_makes_exactly_twenty_questions_in_the_bank_composition() {
    let e = env().await;
    let text = material_lines(30).join("\n");
    let id = upload_ok(&e, &e.user, "handbook.md", text.into_bytes()).await;
    let j = wait_for(&e.t, &e.user, id, Some("confirm"), &["failed", "cancelled"]).await;
    assert_eq!(
        (j["mode"].clone(), j["goal"].clone()),
        (json!("material"), json!(20)),
        "{j}"
    );
    let (s, v) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200, "{v}");
    let done = wait_for(&e.t, &e.user, id, None, &["done", "failed"]).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["skipped"], Value::Null, "material mode lists nothing");
    let st = e.t.backend.st();
    let body = st.gen.created.first().expect("create");
    assert_eq!(body["mode"], "material");
    assert!(body.get("skipped").is_none());
    assert_eq!(body["goal"], 20);
    let qs = body["questions"].as_array().expect("qs");
    assert_eq!(qs.len(), 20);
    assert_eq!(qs.iter().filter(|q| q["kind"] == "multi").count(), 6);
    assert_eq!(body["file"]["type"], "md");
}

// —— T-IM-AI-4 quota ————————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn declining_is_free_and_leaves_a_seen_cancelled_job() {
    let e = env().await;
    let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
    wait_for(&e.t, &e.user, id, Some("confirm"), &["failed"]).await;
    let (s, v) =
        e.t.post("/api/ai/gen/cancel", &e.user, json!({"id": id}))
            .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        (v["job"]["state"].clone(), v["job"]["charged"].clone()),
        (json!("cancelled"), json!(false))
    );
    let seen: i64 = q1(&e.t, "SELECT seen FROM tasks");
    assert_eq!(seen, 1);
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(
        (v["gen"]["used"].clone(), v["gen"]["unseen"].clone()),
        (json!(0), json!([]))
    );
    // the recognising calls are not counted by the call caps (A-OWNER-56)
    assert_eq!(v["calls"]["used"]["day"], 0);
    // the runner is gone and the user may upload again
    for _ in 0..100 {
        if e.t.app.0.jobs.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(e.t.app.0.jobs.is_empty());
    let id2 = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
    assert!(id2 > id);
}

#[tokio::test]
async fn an_unconfirmed_import_expires_unseen_and_free() {
    let e = env().await;
    e.t.app.0.knobs.confirm_secs.store(1, SeqCst);
    let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
    let j = wait_for(&e.t, &e.user, id, None, &["cancelled", "failed", "done"]).await;
    assert_eq!(
        (j["state"].clone(), j["error"].clone(), j["charged"].clone()),
        (json!("cancelled"), json!("ai.import.expired"), json!(false)),
        "{j}"
    );
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(
        v["gen"]["unseen"].as_array().map(Vec::len),
        Some(1),
        "one card"
    );
    assert_eq!(v["gen"]["used"], 0);
    // a confirm after the expiry is refused as not waiting
    let (s, v) = confirm(&e, &e.user, id).await;
    assert_eq!((s, &v["code"]), (409, &json!("ai.import.notWaiting")));
    // the card can be dismissed
    let (s, v) =
        e.t.post("/api/ai/gen/dismiss", &e.user, json!({"id": id}))
            .await;
    assert_eq!((s, v), (200, json!({"ok": true})));
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(v["gen"]["unseen"], json!([]));
}

#[tokio::test]
async fn a_cancel_during_the_extraction_kills_the_child_and_is_free() {
    let e = env().await;
    e.t.app.0.knobs.hang_child.store(true, SeqCst);
    let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let j = job_of(&e.t, &e.user, id).await;
    assert_eq!(
        (j["state"].clone(), j["phase"].clone()),
        (json!("queued"), json!("reading"))
    );
    let started = std::time::Instant::now();
    let (s, v) =
        e.t.post("/api/ai/gen/cancel", &e.user, json!({"id": id}))
            .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["job"]["charged"], false);
    for _ in 0..200 {
        if e.t.app.0.jobs.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(e.t.app.0.jobs.is_empty(), "the runner ended");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "no wait for the 30 s kill"
    );
    assert_eq!(q1::<i64>(&e.t, "SELECT COUNT(*) FROM gen_calls"), 0);
}

#[tokio::test]
async fn a_cancel_after_the_confirm_keeps_the_unit_only_once_writing_started() {
    // while writing: kept
    let e = env().await;
    let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
    wait_for(&e.t, &e.user, id, Some("confirm"), &["failed"]).await;
    e.t.stub.st().head_delay_ms = 3000;
    let (s, _) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200);
    for _ in 0..200 {
        if job_of(&e.t, &e.user, id).await["state"] == "running" && e.t.stub.count() > 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let (s, v) =
        e.t.post("/api/ai/gen/cancel", &e.user, json!({"id": id}))
            .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        (v["job"]["state"].clone(), v["job"]["charged"].clone()),
        (json!("cancelled"), json!(true))
    );
    // while still queued for a place: not charged
    let e2 = env().await;
    e2.t.post("/api/ai/admin/limits", &e2.admin, json!({"genRunning": 1}))
        .await;
    let id = upload_ok(&e2, &e2.user, "course.docx", docx_of(&six())).await;
    wait_for(&e2.t, &e2.user, id, Some("confirm"), &["failed"]).await;
    let other = e2.t.user("blake", false, true);
    e2.t.stub.st().head_delay_ms = 1500;
    let (s, _) =
        e2.t.post(
            "/api/ai/gen/start",
            &other,
            json!({"source": "bank", "bankId": 3}),
        )
        .await;
    assert_eq!(s, 202);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (s, _) = confirm(&e2, &e2.user, id).await;
    assert_eq!(s, 200);
    let j = job_of(&e2.t, &e2.user, id).await;
    assert_eq!(
        (j["state"].clone(), j["phase"].clone(), j["charged"].clone()),
        (json!("queued"), json!("writing"), json!(true))
    );
    let (s, v) =
        e2.t.post("/api/ai/gen/cancel", &e2.user, json!({"id": id}))
            .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        (v["job"]["state"].clone(), v["job"]["charged"].clone()),
        (json!("cancelled"), json!(false))
    );
    e2.t.stub.st().head_delay_ms = 0;
}

#[tokio::test]
async fn a_confirm_when_the_units_are_used_up_is_refused_and_the_job_keeps_waiting() {
    let e = env().await;
    let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
    wait_for(&e.t, &e.user, id, Some("confirm"), &["failed"]).await;
    e.t.post("/api/ai/admin/limits", &e.admin, json!({"genPerDay": 0}))
        .await;
    let (s, v) = confirm(&e, &e.user, id).await;
    assert_eq!((s, &v["code"]), (429, &json!("ai.gen.quota")));
    assert_eq!(job_of(&e.t, &e.user, id).await["phase"], "confirm");
    e.t.post("/api/ai/admin/limits", &e.admin, json!({"genPerDay": 3}))
        .await;
    let (s, v) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200, "{v}");
    let done = wait_for(&e.t, &e.user, id, None, &["done", "failed"]).await;
    assert_eq!(done["state"], "done");
    // other refusals of the confirm
    let (s, v) = confirm(&e, &e.user, id).await;
    assert_eq!((s, &v["code"]), (409, &json!("ai.import.notWaiting")));
    let (s, v) = confirm(&e, &e.admin, id).await;
    assert_eq!((s, &v["code"]), (404, &json!("ai.gen.notFound")));
    let (s, v) =
        e.t.post(
            "/api/ai/import/confirm",
            &e.user,
            json!({"id": id, "name": "n", "x": 1}),
        )
        .await;
    assert_eq!((s, &v["code"]), (400, &json!("ai.badRequest")));
}

#[tokio::test]
async fn the_confirm_needs_a_free_bank_name_and_creates_nothing_otherwise() {
    let e = env().await;
    let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
    wait_for(&e.t, &e.user, id, Some("confirm"), &["failed"]).await;
    let post = |body: Value| e.t.post("/api/ai/import/confirm", &e.user, body);
    // missing, empty, blank: ai.import.nameEmpty; too long / control characters: ai.gen.badName
    for body in [
        json!({"id": id}),
        json!({"id": id, "name": ""}),
        json!({"id": id, "name": " \t "}),
        json!({"id": id, "name": null}),
    ] {
        let (s, v) = post(body.clone()).await;
        assert_eq!(
            (s, &v["code"]),
            (400, &json!("ai.import.nameEmpty")),
            "{body}"
        );
    }
    let (s, v) = post(json!({"id": id, "name": "x".repeat(121)})).await;
    assert_eq!(
        (s, &v["code"], &v["max"]),
        (400, &json!("ai.gen.badName"), &json!(120))
    );
    let (s, v) = post(json!({"id": id, "name": "a\u{7}b"})).await;
    assert_eq!((s, &v["code"]), (400, &json!("ai.gen.badName")));
    let (s, _) = post(json!({"id": id, "name": 5})).await;
    assert_eq!(s, 400);
    // a clash: refused, the job keeps waiting, nothing charged, nothing created
    e.t.backend.st().gen.taken_names.push("Taken bank".into());
    let (s, v) = post(json!({"id": id, "name": "  Taken bank "})).await;
    assert_eq!((s, &v["code"]), (409, &json!("ai.import.nameTaken")));
    let j = job_of(&e.t, &e.user, id).await;
    assert_eq!(
        (j["phase"].clone(), j["charged"].clone()),
        (json!("confirm"), json!(false))
    );
    assert!(e.t.backend.st().gen.created.is_empty());
    // the pre-filled suggestion can be longer than a Home name: 120 characters pass the name check (they only clash here)
    let long = "y".repeat(120);
    e.t.backend.st().gen.taken_names.push(long.clone());
    let (s, v) = post(json!({"id": id, "name": long})).await;
    assert_eq!((s, &v["code"]), (409, &json!("ai.import.nameTaken")));
    // a free name goes through (trimmed) and names the bank
    let (s, v) = post(json!({"id": id, "name": "  Fresh name  "})).await;
    assert_eq!(s, 200, "{v}");
    let done = wait_for(&e.t, &e.user, id, None, &["done", "failed"]).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["name"], "Fresh name");
    let created = e.t.backend.st().gen.created.clone();
    assert_eq!(created[0]["name"], "Fresh name");
}

#[tokio::test]
async fn the_confirm_checks_the_bank_permission_again() {
    let e = env().await;
    let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
    wait_for(&e.t, &e.user, id, Some("confirm"), &["failed"]).await;
    e.t.backend.st().gen.check_refusal = Some("notAllowed".into());
    let (s, v) = confirm(&e, &e.user, id).await;
    assert_eq!((s, &v["code"]), (403, &json!("ai.gen.forbidden")));
    assert_eq!(
        job_of(&e.t, &e.user, id).await["phase"],
        "confirm",
        "the job keeps waiting"
    );
    e.t.backend.st().gen.check_refusal = Some("session".into());
    let (s, v) = confirm(&e, &e.user, id).await;
    assert_eq!((s, &v["code"]), (403, &json!("ai.notAllowed")));
}

// —— T-IM-AI-5 failures ———————————————————————————————————————————————————————————————————————————————————————————————

async fn failure_of(e: &E, name: &str, bytes: Vec<u8>) -> Value {
    let id = upload_ok(e, &e.user, name, bytes).await;
    let j = wait_for(&e.t, &e.user, id, None, &["failed", "done", "cancelled"]).await;
    assert_eq!(j["state"], "failed", "{j}");
    assert_eq!(j["charged"], false, "{j}");
    assert!(e.t.backend.st().gen.created.is_empty(), "no bank");
    assert_eq!(q1::<i64>(&e.t, "SELECT charged FROM tasks"), 0);
    j
}

#[tokio::test]
async fn extraction_failures_are_free_and_make_no_ai_call() {
    for (name, bytes, code) in [
        ("blank.txt", b"   \n  \n".to_vec(), "ai.import.noText"),
        ("empty.pdf", testgen::pdf_empty(2), "ai.import.noText"),
        (
            "locked.pdf",
            testgen::pdf_encrypted_junk(),
            "ai.import.protected",
        ),
        (
            "garbage.docx",
            b"this is not a zip file at all".to_vec(),
            "ai.import.corrupt",
        ),
        (
            "bomb.docx",
            testgen::zip_of(&[("word/document.xml", &vec![b' '; 40 << 20][..])]),
            "ai.import.tooComplex",
        ),
    ] {
        let e = env().await;
        let j = failure_of(&e, name, bytes).await;
        assert_eq!(j["error"], code, "{name}: {j}");
        assert_eq!(
            q1::<i64>(&e.t, "SELECT COUNT(*) FROM gen_calls"),
            0,
            "{name}: no model call"
        );
        // the other users of the service are unaffected
        let ev =
            e.t.send_all(&e.user, json!({"text": "still alive", "lang": "en"}))
                .await
                .expect("chat works");
        assert_eq!(terminal(&ev).0, "done");
    }
}

#[tokio::test]
async fn a_hanging_extraction_is_killed_and_reported_as_a_read_timeout() {
    let e = env().await;
    e.t.app.0.knobs.hang_child.store(true, SeqCst);
    e.t.app.0.knobs.kill_secs.store(1, SeqCst);
    let started = std::time::Instant::now();
    let j = failure_of(&e, "course.docx", docx_of(&six())).await;
    assert_eq!(j["error"], "ai.import.readTimeout");
    assert!(started.elapsed() < Duration::from_secs(15));
}

#[tokio::test]
async fn recognising_failures_are_free() {
    // nothing to learn from
    let e = env().await;
    let j = failure_of(
        &e,
        "form.txt",
        b"KIND:NONE a list of names and numbers only".to_vec(),
    )
    .await;
    assert_eq!(j["error"], "ai.import.noContent");
    // an unreadable classification, twice
    let e = env().await;
    e.t.stub.st().script = Some(Arc::new(|sys: &str, _| {
        if sys.starts_with("You sort") {
            Reply::Text("I think it is probably a quiz.".into())
        } else {
            Reply::Fall
        }
    }));
    // (no numbered lines: the classification decides, so it is the first call)
    let j = failure_of(
        &e,
        "course.txt",
        b"KIND:QUESTIONS plain prose about simmering pots".to_vec(),
    )
    .await;
    assert_eq!(j["error"], "ai.import.unclear");
    assert_eq!(
        q1::<i64>(
            &e.t,
            "SELECT COUNT(*) FROM gen_calls WHERE purpose = 'recognise'"
        ),
        2
    );
    // three provider errors in a row
    let e = env().await;
    e.t.stub.st().script = Some(Arc::new(|_, _| Reply::Status(500, "boom".into())));
    let j = failure_of(&e, "course.docx", docx_of(&six())).await;
    assert_eq!(j["error"], "ai.up.server");
    assert_eq!(
        q1::<i64>(
            &e.t,
            "SELECT COUNT(*) FROM gen_calls WHERE status = 'error'"
        ),
        3
    );
}

#[tokio::test]
async fn a_file_over_the_limit_takes_the_first_questions_and_says_so() {
    let e = env().await;
    e.t.post(
        "/api/ai/admin/limits",
        &e.admin,
        json!({"importMaxQuestions": 5}),
    )
    .await;
    // A-OWNER-47: 12 questions, limit 5: the first 5 are imported, nothing is refused
    let id = upload_ok(&e, &e.user, "long.docx", docx_of(&questions_lines(12))).await;
    let j = wait_for(&e.t, &e.user, id, Some("confirm"), &["failed", "cancelled"]).await;
    assert_eq!(j["phase"], "confirm", "{j}");
    assert_eq!(
        (j["goal"].clone(), j["total"].clone(), j["taken"].clone()),
        (json!(5), json!(12), json!(5))
    );
    let (s, v) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200, "{v}");
    let done = wait_for(&e.t, &e.user, id, None, &["done", "failed"]).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(
        (
            done["made"].clone(),
            done["total"].clone(),
            done["taken"].clone()
        ),
        (json!(5), json!(12), json!(5))
    );
}

#[tokio::test]
async fn a_file_within_the_limit_has_total_equal_to_taken() {
    let e = env().await;
    let id = upload_ok(&e, &e.user, "six.docx", docx_of(&six())).await;
    let j = wait_for(&e.t, &e.user, id, Some("confirm"), &["failed", "cancelled"]).await;
    assert_eq!(
        (j["total"].clone(), j["taken"].clone()),
        (json!(6), json!(6))
    );
}

/// `n` questions of about 2 800 characters each (three questions per window), made up.
fn long_questions(n: usize) -> Vec<String> {
    let mut v = vec!["KIND:QUESTIONS long notes".to_string()];
    for i in 1..=n {
        v.push(format!(
            "Q{i}. Which kitchen device {} keeps a stock pot at a steady simmer?",
            en_text(300 + i, 3)
        ));
        for l in ["A", "B", "C"] {
            v.push(format!("{l}. {}", "pot ".repeat(225)));
        }
        v.push("Answer: B".to_string());
    }
    v
}

#[tokio::test]
async fn the_windows_are_read_in_waves_with_progress_and_no_classification_call() {
    let e = env().await;
    let id = upload_ok(&e, &e.user, "long.docx", docx_of(&long_questions(12))).await;
    let j = wait_for(&e.t, &e.user, id, Some("confirm"), &["failed", "cancelled"]).await;
    assert_eq!(j["mode"], "questions", "{j}");
    assert_eq!(j["goal"], 12);
    let total = j["read"]["total"].as_i64().expect("total");
    assert!(total >= 3, "{j}");
    assert_eq!(j["read"]["done"], j["read"]["total"], "{j}");
    // one call per window, nothing else: the first windows decided the type
    let rows: i64 = q1(
        &e.t,
        "SELECT COUNT(*) FROM gen_calls WHERE purpose = 'recognise'",
    );
    assert_eq!(rows, total);
    let classify: i64 = q1(
        &e.t,
        "SELECT COUNT(*) FROM gen_calls WHERE purpose = 'recognise' AND asked = 0",
    );
    assert_eq!(
        classify, total,
        "the segmentation rows carry no asked count"
    );
}

#[tokio::test]
async fn parallel_windows_are_never_held_back_by_the_per_user_caps() {
    let e = env().await;
    e.t.post(
        "/api/ai/admin/limits",
        &e.admin,
        json!({"callsPerMinute": 1, "callsPerHour": 1, "callsPerDay": 1}),
    )
    .await;
    assert_eq!(e.t.app.0.limits().calls_per_hour, 1, "the caps were saved");
    // the file needs 4 or more window calls; the caps allow one: import ignores them (A-OWNER-56)
    let id = upload_ok(&e, &e.user, "long.docx", docx_of(&long_questions(12))).await;
    let j = wait_for(&e.t, &e.user, id, Some("confirm"), &["failed"]).await;
    assert_eq!(j["state"], "running", "{j}");
    assert!(
        q1::<i64>(
            &e.t,
            "SELECT COUNT(*) FROM gen_calls WHERE purpose = 'recognise'"
        ) >= 4
    );
}

#[tokio::test]
async fn a_questions_file_that_needs_more_windows_than_the_ceiling_is_refused_before_any_call() {
    let e = env().await;
    // the ceiling 8 leaves 6 windows (60 000 characters) for a questions file
    let (s, v) =
        e.t.post(
            "/api/ai/admin/limits",
            &e.admin,
            json!({"recognizeMaxCalls": 8}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["limits"]["recognizeMaxCalls"], 8);
    let j = failure_of(&e, "huge.docx", docx_of(&long_questions(30))).await;
    assert_eq!(
        (j["error"].clone(), j["errorArg"].clone()),
        (json!("ai.import.tooMuchText"), json!(60_000)),
        "{j}"
    );
    assert_eq!(q1::<i64>(&e.t, "SELECT COUNT(*) FROM gen_calls"), 0);
    // the default ceiling is 20 (range 8 to 40)
    let (_, v) = e.t.get("/api/ai/admin/limits", &e.admin).await;
    assert_eq!(v["defaults"]["recognizeMaxCalls"], 20);
    assert_eq!(v["ranges"]["recognizeMaxCalls"], json!([8, 40]));
    let (s, _) =
        e.t.post(
            "/api/ai/admin/limits",
            &e.admin,
            json!({"recognizeMaxCalls": 41}),
        )
        .await;
    assert_eq!(s, 400);
}

#[tokio::test]
async fn the_recognising_calls_stop_at_the_ceiling() {
    let e = env().await;
    e.t.post(
        "/api/ai/admin/limits",
        &e.admin,
        json!({"recognizeMaxCalls": 8}),
    )
    .await;
    // an unreadable classification answer is asked twice; with every segmentation answer unreadable too, the retries
    // never pass the ceiling of 8
    e.t.stub.st().script = Some(Arc::new(|_, _| Reply::Text("not json at all".into())));
    let j = failure_of(&e, "long.docx", docx_of(&long_questions(12))).await;
    assert_eq!(j["error"], "ai.import.unclear", "{j}");
    let n: i64 = q1(
        &e.t,
        "SELECT COUNT(*) FROM gen_calls WHERE purpose = 'recognise'",
    );
    assert!((1..=8).contains(&n), "{n}");
}

#[tokio::test]
async fn a_question_with_too_long_a_source_text_is_named_by_its_number() {
    let e = env().await;
    let mut lines = vec!["KIND:QUESTIONS notes".to_string()];
    lines.push("Q1. A short question that is fine?".to_string());
    lines.push("A. one".to_string());
    lines.push("B. two".to_string());
    lines.push("Q2. A very long question follows.".to_string());
    for i in 0..4 {
        lines.push(format!("{} {}", "long".repeat(200), i));
    }
    let j = failure_of(&e, "long.docx", docx_of(&lines)).await;
    assert_eq!(
        (j["error"].clone(), j["errorArg"].clone()),
        (json!("ai.import.tooLongQuestion"), json!(2))
    );
}

#[tokio::test]
async fn when_every_conversion_fails_the_import_fails_without_a_bank() {
    let e = env().await;
    let inner = good_model();
    e.t.stub.st().script = Some(Arc::new(move |sys: &str, user: &str| {
        if sys.starts_with("You convert exam questions") {
            Reply::Text("{\"questions\": []}".into())
        } else {
            inner(sys, user)
        }
    }));
    let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
    wait_for(&e.t, &e.user, id, Some("confirm"), &["failed"]).await;
    let (s, _) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200);
    let j = wait_for(&e.t, &e.user, id, None, &["failed", "done"]).await;
    assert_eq!(
        (j["state"].clone(), j["error"].clone(), j["charged"].clone()),
        (json!("failed"), json!("ai.gen.quality"), json!(false))
    );
    assert!(e.t.backend.st().gen.created.is_empty());
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(v["gen"]["used"], 0, "a failed import is never counted");
}

// —— T-IM-AI-6 conversion checks —————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn a_rewritten_stem_is_redone_and_a_question_failing_three_times_is_skipped_and_listed() {
    let e = env().await;
    let inner = good_model();
    let seen_q2 = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let s2 = seen_q2.clone();
    e.t.stub.st().script = Some(Arc::new(move |sys: &str, user: &str| {
        if !sys.starts_with("You convert exam questions") {
            return inner(sys, user);
        }
        let Reply::Text(t) = inner(sys, user) else {
            return Reply::Fall;
        };
        let mut v: Value = serde_json::from_str(&t).expect("json");
        for q in v["questions"].as_array_mut().expect("qs") {
            let slot = q["slot"].as_u64().unwrap_or(0);
            // slot 2: reworded the first time only; slot 3: always reworded
            let reword = slot == 3 || (slot == 2 && s2.fetch_add(1, SeqCst) == 0);
            if reword {
                q["stem"]["en"] = json!(
                    "What apparatus holds a large cooking vessel at an even low heat setting"
                );
            }
        }
        Reply::Text(v.to_string())
    }));
    let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
    wait_for(&e.t, &e.user, id, Some("confirm"), &["failed"]).await;
    let (s, _) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200);
    let done = wait_for(&e.t, &e.user, id, None, &["done", "failed"]).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(
        (
            done["made"].clone(),
            done["goal"].clone(),
            done["skipped"].clone()
        ),
        (json!(5), json!(6), json!([3]))
    );
    assert_eq!(done["charged"], true, "a partial import counts");
    let st = e.t.backend.st();
    let body = st.gen.created.first().expect("create");
    assert_eq!(body["skipped"], json!([3]));
    assert_eq!(body["goal"], 6);
    assert_eq!(body["questions"].as_array().map(Vec::len), Some(5));
    assert!(seen_q2.load(SeqCst) >= 2, "slot 2 was asked again");
}

// —— A-OWNER-66 no endless import: a slot is given up after three failed tries ——————————————————————————————————————————

/// A provider that never delivers the slots in `drop`: they are left out of every conversion answer; an answer left
/// with no question at all is plain prose (unreadable).
fn never_delivers(e: &E, drop: &'static [u64]) {
    never_delivers_when(e, drop, |sys, _| {
        sys.starts_with("You convert exam questions")
    });
}

/// The same on the whole-file path: only the TASK CONVERT calls lose their slots.
fn never_delivers_whole(e: &E, drop: &'static [u64]) {
    never_delivers_when(e, drop, |sys, user| {
        sys.starts_with("You work on one file of a user of Xexam")
            && task_of(user).contains("TASK CONVERT")
    });
}

fn never_delivers_when(e: &E, drop: &'static [u64], hit: fn(&str, &str) -> bool) {
    let inner = good_model();
    e.t.stub.st().script = Some(Arc::new(move |sys: &str, user: &str| {
        if !hit(sys, user) {
            return inner(sys, user);
        }
        let Reply::Text(t) = inner(sys, user) else {
            return Reply::Fall;
        };
        let mut v: Value = serde_json::from_str(&t).expect("json");
        let list = v["questions"].as_array_mut().expect("qs");
        list.retain(|q| !drop.contains(&q["slot"].as_u64().unwrap_or(0)));
        if list.is_empty() {
            return Reply::Text("Sorry, I could not do that one.".into());
        }
        Reply::Text(v.to_string())
    }));
}

async fn partial_import_ends_after_three_tries(unlimited: i64) {
    let e = env().await;
    e.t.post(
        "/api/ai/admin/limits",
        &e.admin,
        json!({"importUnlimited": unlimited}),
    )
    .await;
    never_delivers(&e, &[3]);
    let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
    wait_for(&e.t, &e.user, id, Some("confirm"), &["failed"]).await;
    let (s, _) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200);
    let done = wait_for(&e.t, &e.user, id, None, &["done", "failed"]).await;
    assert_eq!(done["state"], "done", "unlimited {unlimited}: {done}");
    assert_eq!(
        (
            done["made"].clone(),
            done["goal"].clone(),
            done["skipped"].clone()
        ),
        (json!(5), json!(6), json!([3])),
        "unlimited {unlimited}"
    );
    let st = e.t.backend.st();
    let body = st.gen.created.first().expect("create");
    assert_eq!(body["skipped"], json!([3]));
    assert_eq!(body["questions"].as_array().map(Vec::len), Some(5));
    // one batch that lost slot 3, then slot 3 alone three times: never an endless loop
    assert!(
        e.t.stub.count() <= 12,
        "unlimited {unlimited}: {} calls",
        e.t.stub.count()
    );
}

#[tokio::test]
async fn a_slot_the_model_never_delivers_is_given_up_after_three_tries_with_unlimited_off() {
    partial_import_ends_after_three_tries(0).await;
}

#[tokio::test]
async fn a_slot_the_model_never_delivers_is_given_up_after_three_tries_with_unlimited_on() {
    partial_import_ends_after_three_tries(1).await;
}

#[tokio::test]
async fn a_conversion_that_returns_nothing_readable_is_repeated_one_slot_per_call_then_every_slot_is_given_up(
) {
    for unlimited in [0, 1] {
        let e = env().await;
        e.t.post(
            "/api/ai/admin/limits",
            &e.admin,
            json!({"importUnlimited": unlimited}),
        )
        .await;
        // slots 1 and 2 are delivered, the rest never: the bigger call loses 3..6, each then fails three times alone
        never_delivers(&e, &[3, 4, 5, 6]);
        let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
        wait_for(&e.t, &e.user, id, Some("confirm"), &["failed"]).await;
        let (s, _) = confirm(&e, &e.user, id).await;
        assert_eq!(s, 200);
        let done = wait_for(&e.t, &e.user, id, None, &["done", "failed"]).await;
        assert_eq!(done["state"], "done", "unlimited {unlimited}: {done}");
        assert_eq!(
            (done["made"].clone(), done["skipped"].clone()),
            (json!(2), json!([3, 4, 5, 6])),
            "unlimited {unlimited}"
        );
    }
}

#[tokio::test]
async fn a_cancel_that_keeps_the_finished_questions_creates_the_partial_bank() {
    let e = env().await;
    e.t.post(
        "/api/ai/admin/limits",
        &e.admin,
        json!({"importUnlimited": 1}),
    )
    .await;
    never_delivers(&e, &[3, 4, 5, 6]);
    let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
    wait_for(&e.t, &e.user, id, Some("confirm"), &["failed"]).await;
    e.t.stub.st().head_delay_ms = 300;
    let (s, _) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200);
    for _ in 0..400 {
        let j = job_of(&e.t, &e.user, id).await;
        if j["made"].as_i64().unwrap_or(0) >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let (s, v) =
        e.t.post(
            "/api/ai/gen/cancel",
            &e.user,
            json!({"id": id, "keep": true}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let done = wait_for(&e.t, &e.user, id, None, &["done", "failed", "cancelled"]).await;
    e.t.stub.st().head_delay_ms = 0;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["made"], 2);
    assert_eq!(done["skipped"], json!([3, 4, 5, 6]));
    assert_eq!(done["charged"], true);
    {
        let st = e.t.backend.st();
        let body = st.gen.created.first().expect("the bank was created");
        assert_eq!(body["questions"].as_array().map(Vec::len), Some(2));
        assert_eq!(body["skipped"], json!([3, 4, 5, 6]));
        assert_eq!(body["goal"], 6);
    }
    // a non-boolean flag is a plain bad request
    let (s, _) =
        e.t.post(
            "/api/ai/gen/cancel",
            &e.user,
            json!({"id": id, "keep": "yes"}),
        )
        .await;
    assert_eq!(s, 400);
}

// —— T-IM-AI-7 restart, purge ————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn a_restart_in_the_confirm_phase_interrupts_the_job_without_a_charge() {
    let mut e = env().await;
    let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
    wait_for(&e.t, &e.user, id, Some("confirm"), &["failed"]).await;
    e.t.restart(Opts::default()).await;
    let j = job_of(&e.t, &e.user, id).await;
    assert_eq!(
        (j["state"].clone(), j["error"].clone(), j["charged"].clone()),
        (json!("failed"), json!("ai.gen.interrupted"), json!(false))
    );
}

#[tokio::test]
async fn purging_a_user_during_recognising_stops_the_import() {
    let e = env().await;
    e.t.stub.st().head_delay_ms = 500;
    let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(e.t.app.0.purge_user(e.user.id).await);
    assert!(e.t.app.0.jobs.is_empty());
    assert_eq!(tasks(&e.t), 0);
    let _ = id;
}

// —— T-IM-AI-8 the shared quota ——————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn generations_and_imports_share_the_three_a_day_and_one_job_at_a_time() {
    let e = env().await;
    for _ in 0..2 {
        let (s, v) =
            e.t.post(
                "/api/ai/gen/start",
                &e.user,
                json!({"source": "bank", "bankId": 3}),
            )
            .await;
        assert_eq!(s, 202, "{v}");
        wait_for(
            &e.t,
            &e.user,
            v["job"]["id"].as_i64().expect("id"),
            None,
            &["done"],
        )
        .await;
    }
    let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
    wait_for(&e.t, &e.user, id, Some("confirm"), &["failed"]).await;
    // one AI-bank job per user across both kinds
    let (s, v) =
        e.t.post(
            "/api/ai/gen/start",
            &e.user,
            json!({"source": "bank", "bankId": 3}),
        )
        .await;
    assert_eq!((s, &v["code"]), (409, &json!("ai.gen.busy")));
    assert_eq!(v["job"]["source"], "file");
    let (s, _) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200);
    wait_for(&e.t, &e.user, id, None, &["done"]).await;
    // 2 generations + 1 confirmed import = 3: the next upload and generation are refused
    let (_, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(
        (v["gen"]["used"].clone(), v["gen"]["left"].clone()),
        (json!(3), json!(0))
    );
    let (s, v) = e.t.upload(&e.user, "course.docx", docx_of(&six())).await;
    assert_eq!((s, &v["code"]), (429, &json!("ai.gen.quota")));
    let (s, v) =
        e.t.post(
            "/api/ai/gen/start",
            &e.user,
            json!({"source": "bank", "bankId": 3}),
        )
        .await;
    assert_eq!((s, &v["code"]), (429, &json!("ai.gen.quota")));
    // an administrator is exempt
    let id = upload_ok(&e, &e.admin, "course.docx", docx_of(&six())).await;
    wait_for(&e.t, &e.admin, id, Some("confirm"), &["failed"]).await;
}

// —— T-IM-AI-9 the call caps —————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn import_calls_are_outside_the_caps_never_refused_and_never_counted() {
    let e = env().await;
    e.t.post(
        "/api/ai/admin/limits",
        &e.admin,
        json!({"callsPerMinute": 1, "callsPerHour": 20, "callsPerDay": 100}),
    )
    .await;
    let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
    wait_for(&e.t, &e.user, id, Some("confirm"), &["failed"]).await;
    // the recognising call did not use the minute cap: it is still free for chat (the stub answers it)
    let (st, v) = e.t.get("/api/ai/status", &e.user).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["calls"]["used"]["minute"], 0, "{v}");
    assert!(v["calls"]["blocked"].is_null(), "{v}");
    // the writing calls are not held back either
    let (s, _) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200);
    let done = wait_for(&e.t, &e.user, id, None, &["done", "failed"]).await;
    assert_eq!(done["state"], "done", "{done}");
    // the admin usage: today (the capped calls) stays 0, the import row is recognise + import
    let (s, v) = e.t.get("/api/ai/admin/usage?days=7", &e.admin).await;
    assert_eq!(s, 200, "{v}");
    let row = v["byUser"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["username"] == "alexandra_k")
        .expect("user")
        .clone();
    assert_eq!(row["today"], 0);
    let import_row = v["byPurpose"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["purpose"] == "import")
        .expect("import")
        .clone();
    let all: i64 = q1(
        &e.t,
        "SELECT COUNT(*) FROM gen_calls WHERE purpose IN ('recognise', 'import')",
    );
    assert_eq!(import_row["calls"].as_i64(), Some(all));
    assert!(all > 2);
}

#[tokio::test]
async fn full_per_user_caps_do_not_fail_a_classification() {
    let e = env().await;
    e.t.post(
        "/api/ai/admin/limits",
        &e.admin,
        json!({"callsPerMinute": 1, "callsPerHour": 1, "callsPerDay": 100}),
    )
    .await;
    // no numbered lines: the classification is the first call, the segmentation the second; both pass
    let id = upload_ok(
        &e,
        &e.user,
        "course.txt",
        b"KIND:QUESTIONS plain prose about simmering pots".to_vec(),
    )
    .await;
    let j = wait_for(&e.t, &e.user, id, Some("confirm"), &["failed"]).await;
    assert_eq!(j["state"], "running", "{j}");
    assert!(
        q1::<i64>(
            &e.t,
            "SELECT COUNT(*) FROM gen_calls WHERE purpose = 'recognise'"
        ) >= 1
    );
}

// —— IX in the real child process ————————————————————————————————————————————————————————————————————————————————————

#[test]
fn the_child_works_with_an_empty_environment_and_no_data_directory() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let dir = tempfile::tempdir().expect("dir");
    let mut child = Command::new(env!("CARGO_BIN_EXE_xexam-ai-service"))
        .args([
            "extract",
            "--ext",
            "txt",
            "--lang",
            "en",
            "--max-bytes",
            "10000",
            "--max-chars",
            "10000",
        ])
        .env_clear()
        .current_dir(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"Which tool keeps a stock pot at a steady simmer on the stove?")
        .expect("write");
    let out = child.wait_with_output().expect("output");
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.starts_with("XEXAM-EXTRACT-RESULT "), "{text}");
    assert!(text.contains("\"ok\":true"));
    assert!(
        std::fs::read_dir(dir.path()).expect("dir").next().is_none(),
        "nothing written to disk"
    );
}

#[test]
fn the_child_protocol_through_the_parent() {
    use xexam_ai_service::extract::child::{run_child_blocking, run_child_with, ChildArgs};
    use xexam_ai_service::extract::{ExtractError, LangHint};
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_xexam-ai-service"));
    let docx = docx_of(&six());
    let doc = run_child_blocking(
        exe,
        &ChildArgs::new("docx", LangHint::En, docx.len() as u64),
        docx.clone(),
        Duration::from_secs(30),
    )
    .expect("docx");
    assert_eq!(doc.kind.as_str(), "docx");
    assert!(doc.sections[0].blocks.iter().any(|b| b.starts_with("Q1.")));
    // stdin over the maximum
    let r = run_child_blocking(
        exe,
        &ChildArgs::new("docx", LangHint::En, 100),
        docx.clone(),
        Duration::from_secs(30),
    );
    assert_eq!(r.map(|_| ()), Err(ExtractError::TooLarge));
    // the hang hook is killed after the test timeout
    let started = std::time::Instant::now();
    let r = run_child_with(
        exe,
        &ChildArgs::new("docx", LangHint::En, docx.len() as u64),
        docx.clone(),
        Duration::from_secs(1),
        true,
        &std::sync::atomic::AtomicBool::new(false),
    );
    assert_eq!(r.map(|_| ()), Err(ExtractError::Timeout));
    assert!(started.elapsed() < Duration::from_secs(10));
    // a library failure is a normal answer, not a crashed service
    let r = run_child_blocking(
        exe,
        &ChildArgs::new("pdf", LangHint::En, 1000),
        b"%PDF-1.4 junk".to_vec(),
        Duration::from_secs(30),
    );
    assert_eq!(r.map(|_| ()), Err(ExtractError::Corrupt));
    // a crafted zip end record claiming 2^32 entries, and a 42.zip-style member
    let mut lie = testgen::zip_of(&[("word/document.xml", b"<a/>")]);
    let eocd = lie.len() - 22;
    lie[eocd + 10] = 0xFF;
    lie[eocd + 11] = 0xFF;
    let r = run_child_blocking(
        exe,
        &ChildArgs::new("docx", LangHint::En, lie.len() as u64),
        lie,
        Duration::from_secs(30),
    );
    assert!(matches!(
        r,
        Err(ExtractError::Corrupt) | Err(ExtractError::TooComplex)
    ));
    let bomb = testgen::zip_of(&[("word/document.xml", &vec![b' '; 40 << 20][..])]);
    let r = run_child_blocking(
        exe,
        &ChildArgs::new("docx", LangHint::En, bomb.len() as u64),
        bomb,
        Duration::from_secs(30),
    );
    assert_eq!(r.map(|_| ()), Err(ExtractError::TooComplex));
}

#[cfg(target_os = "linux")]
#[test]
fn the_rss_watchdog_ends_a_memory_bomb_with_too_complex() {
    use xexam_ai_service::extract::child::{run_child_blocking, ChildArgs};
    use xexam_ai_service::extract::{ExtractError, LangHint};
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_xexam-ai-service"));
    let pdf = testgen::pdf_flate_bomb(1200);
    let mut args = ChildArgs::new("pdf", LangHint::En, pdf.len() as u64);
    args.max_rss_mb = Some(64);
    let r = run_child_blocking(exe, &args, pdf, Duration::from_secs(30));
    assert_eq!(r.map(|_| ()), Err(ExtractError::TooComplex));
    // the next normal extraction works
    let docx = docx_of(&six());
    assert!(run_child_blocking(
        exe,
        &ChildArgs::new("docx", LangHint::En, docx.len() as u64),
        docx,
        Duration::from_secs(30)
    )
    .is_ok());
}

#[test]
fn a_flate_bomb_and_a_recursion_bomb_end_the_child_and_the_next_extraction_works() {
    use xexam_ai_service::extract::child::{run_child_blocking, ChildArgs};
    use xexam_ai_service::extract::LangHint;
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_xexam-ai-service"));
    // 150 MiB of inflated content (the Linux-only test uses a 1.2 GiB bomb against the memory watchdog)
    let bomb = testgen::pdf_flate_bomb(150);
    let r = run_child_blocking(
        exe,
        &ChildArgs::new("pdf", LangHint::En, bomb.len() as u64),
        bomb,
        Duration::from_secs(30),
    );
    assert!(r.is_err(), "a page of blanks has no text: {r:?}");
    // 100 000 nested arrays: whatever the parser does (an error, a refusal, a crash of the child), the parent only sees
    // an extraction error
    let nested = testgen::pdf_nested_arrays(100_000);
    let r = run_child_blocking(
        exe,
        &ChildArgs::new("pdf", LangHint::En, nested.len() as u64),
        nested,
        Duration::from_secs(30),
    );
    assert!(r.is_err(), "{r:?}");
    // the service's next extraction works
    let docx = docx_of(&six());
    let ok = run_child_blocking(
        exe,
        &ChildArgs::new("docx", LangHint::En, docx.len() as u64),
        docx,
        Duration::from_secs(30),
    );
    assert!(ok.is_ok(), "{ok:?}");
}

// —— A-OWNER-70 / A-OWNER-71 (sprint 13): the whole file once, as a cached prefix ————————————————————————————————————————

/// `env()` with the whole-file path on (the other tests keep the windowed path by a tiny model context).
async fn env_whole() -> E {
    let e = env().await;
    e.t.app.0.knobs.import_context_tokens.store(0, SeqCst);
    e
}

/// (system, user) of every whole-file call the stub got (OpenAI-compatible form), in order.
fn whole_calls(t: &TestApp) -> Vec<(String, String)> {
    t.stub
        .requests()
        .iter()
        .filter_map(|r| {
            let m = r.body["messages"].as_array()?;
            let sys = m.first()?["content"].as_str()?;
            if !sys.starts_with("You work on one file of a user of Xexam") {
                return None;
            }
            let user = m.iter().rev().find(|x| x["role"] == "user")?["content"].as_str()?;
            Some((sys.to_string(), user.to_string()))
        })
        .collect()
}

/// The part of a user turn up to and including the end fence: the cached prefix.
fn prefix_of(user: &str) -> &str {
    let i = user.find("<<<END FILE").expect("the fence");
    let j = user[i..].find(">>>").expect("the fence end");
    &user[..i + j + 3]
}

fn task_of(user: &str) -> &str {
    user.rsplit_once("<<<END FILE").map_or(user, |(_, t)| t)
}

/// Calls of the windowed prompts (classification / segmentation) the stub got.
fn windowed_calls(t: &TestApp) -> usize {
    t.stub
        .requests()
        .iter()
        .filter(|r| {
            r.body["messages"][0]["content"].as_str().is_some_and(|s| {
                s.starts_with("You sort a user's file")
                    || s.starts_with("You find the exam questions")
            })
        })
        .count()
}

#[tokio::test]
async fn a_file_of_seventy_unnumbered_questions_is_counted_seventy_in_one_call_and_cut_to_fifty() {
    let e = env_whole().await;
    let id = upload_ok(&e, &e.user, "workbook.docx", docx_of(&unnumbered_lines())).await;
    let j = wait_for(&e.t, &e.user, id, Some("confirm"), &["failed", "cancelled"]).await;
    assert_eq!(j["phase"], "confirm", "{j}");
    // the exact count, the first 50 are taken; `found` names the same number as `total` for the front end
    assert_eq!(
        (
            j["total"].clone(),
            j["found"].clone(),
            j["taken"].clone(),
            j["goal"].clone(),
            j["approx"].clone()
        ),
        (json!(70), json!(70), json!(50), json!(50), json!(false))
    );
    assert_eq!(j["read"], json!({"done": 1, "total": 1}));
    // ONE recognising call on the whole file, no window, no classification
    let calls = whole_calls(&e.t);
    assert_eq!(calls.len(), 1);
    assert!(task_of(&calls[0].1).contains("TASK RECOGNISE"));
    assert_eq!(windowed_calls(&e.t), 0);
    let rec: i64 = q1(
        &e.t,
        "SELECT COUNT(*) FROM gen_calls WHERE purpose = 'recognise'",
    );
    assert_eq!(rec, 1);
    let (s, v) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200, "{v}");
    let done = wait_for(&e.t, &e.user, id, None, &["done", "failed"]).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(
        (
            done["made"].clone(),
            done["total"].clone(),
            done["found"].clone(),
            done["taken"].clone()
        ),
        (json!(50), json!(70), json!(70), json!(50))
    );
    // the bank holds the FIRST fifty, each with its own text
    let created = e.t.backend.st().gen.created.clone();
    let qs = created[0]["questions"]
        .as_array()
        .expect("questions")
        .clone();
    assert_eq!(qs.len(), 50);
    for (k, q) in qs.iter().enumerate() {
        let stem = q["stem"]["en"].as_str().unwrap_or("");
        assert!(stem.contains(&pw(100_000 + k)), "{k}: {stem}");
    }
    assert!(!qs.iter().any(|q| q["stem"]["en"]
        .as_str()
        .unwrap_or("")
        .contains(&pw(100_050))));
}

#[tokio::test]
async fn every_call_of_a_job_starts_with_the_same_bytes_and_writing_waits_for_call_one() {
    let e = env_whole().await;
    // every call waits a while before it answers, so calls that run together overlap
    e.t.stub.st().head_delay_ms = 250;
    let log: Arc<std::sync::Mutex<Vec<(bool, std::time::Instant)>>> = Arc::default();
    let inner = good_model();
    let seen = log.clone();
    e.t.stub.st().script = Some(Arc::new(move |system: &str, user: &str| {
        let rec = task_of(user).contains("TASK RECOGNISE");
        seen.lock().unwrap().push((rec, std::time::Instant::now()));
        inner(system, user)
    }));
    let id = upload_ok(&e, &e.user, "workbook.docx", docx_of(&unnumbered_lines())).await;
    wait_for(&e.t, &e.user, id, Some("confirm"), &["failed", "cancelled"]).await;
    let (s, v) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200, "{v}");
    let done = wait_for(&e.t, &e.user, id, None, &["done", "failed"]).await;
    assert_eq!(done["state"], "done", "{done}");
    // byte-identical prefix (rules and file) in every call, whatever the task
    let calls = whole_calls(&e.t);
    assert!(calls.len() >= 4, "{}", calls.len());
    let first = prefix_of(&calls[0].1).to_string();
    assert!(first.starts_with("FILE\n<<<FILE ") && first.contains("\nL1: KIND:QUESTIONS"));
    for (sys, user) in &calls {
        assert_eq!(sys, &calls[0].0);
        assert_eq!(prefix_of(user), first);
        assert!(user[first.len()..].starts_with("\n\nTASK "));
    }
    assert!(task_of(&calls[0].1).contains("TASK RECOGNISE"));
    assert!(calls[1..]
        .iter()
        .all(|(_, u)| task_of(u).contains("TASK CONVERT")));
    // the writing calls name lines and never repeat the file
    assert!(calls[1..]
        .iter()
        .all(|(_, u)| task_of(u).contains("\"lines\":\"L") && task_of(u).len() < 3000));
    // parallel only after call 1: every writing call starts after the recognition call returned (250 ms), up to four
    // at the same time
    let log = log.lock().unwrap().clone();
    let rec_at = log.iter().find(|(r, _)| *r).expect("recognise").1;
    let writes: Vec<std::time::Instant> =
        log.iter().filter(|(r, _)| !*r).map(|(_, t)| *t).collect();
    assert!(writes
        .iter()
        .all(|t| t.duration_since(rec_at).as_millis() >= 200));
    let together = |t0: std::time::Instant| {
        writes
            .iter()
            .filter(|t| **t >= t0 && t.duration_since(t0).as_millis() < 120)
            .count()
    };
    let most = writes.iter().map(|t| together(*t)).max().unwrap_or(0);
    assert!((2..=4).contains(&most), "calls together: {most}");
}

#[tokio::test]
async fn an_anthropic_request_carries_the_breakpoint_on_the_prefix_and_the_task_follows() {
    let e = env_whole().await;
    e.t.configure_anthropic(&e.admin, "demo-claude").await;
    let id = upload_ok(&e, &e.user, "Kitchen notes.docx", docx_of(&six())).await;
    wait_for(&e.t, &e.user, id, Some("confirm"), &["failed", "cancelled"]).await;
    let (s, v) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200, "{v}");
    let done = wait_for(&e.t, &e.user, id, None, &["done", "failed"]).await;
    assert_eq!(done["state"], "done", "{done}");
    let bodies: Vec<Value> =
        e.t.stub
            .requests()
            .iter()
            .filter(|r| r.path.contains("/anthropic/"))
            .filter(|r| {
                r.body["system"][0]["text"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("You work on one file of a user of Xexam"))
            })
            .map(|r| r.body.clone())
            .collect();
    assert!(bodies.len() >= 2, "{}", bodies.len());
    for b in &bodies {
        // the rules block and the file block both carry a breakpoint; the task block after them does not
        assert_eq!(
            b["system"][0]["cache_control"],
            json!({"type": "ephemeral"})
        );
        let c = &b["messages"][0]["content"];
        assert_eq!(c[0]["cache_control"], json!({"type": "ephemeral"}));
        assert!(c[0]["text"]
            .as_str()
            .unwrap_or("")
            .starts_with("FILE\n<<<FILE "));
        assert!(c[0]["text"].as_str().unwrap_or("").ends_with(">>>"));
        assert!(c[1].get("cache_control").is_none());
        assert!(c[1]["text"].as_str().unwrap_or("").starts_with("TASK "));
        assert_eq!(c[0]["text"], bodies[0]["messages"][0]["content"][0]["text"]);
        assert_eq!(b["system"], bodies[0]["system"]);
    }
}

#[tokio::test]
async fn cached_tokens_of_the_import_calls_are_recorded_and_shown_in_the_job() {
    let e = env_whole().await;
    e.t.stub.st().cached_tokens = Some(15);
    e.t.stub.st().cache_write_tokens = Some(7);
    let id = upload_ok(&e, &e.user, "Kitchen notes.docx", docx_of(&six())).await;
    wait_for(&e.t, &e.user, id, Some("confirm"), &["failed", "cancelled"]).await;
    let (s, v) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200, "{v}");
    let done = wait_for(&e.t, &e.user, id, None, &["done", "failed"]).await;
    assert_eq!(done["state"], "done", "{done}");
    let calls = done["usage"]["calls"].as_i64().expect("calls");
    assert!(calls >= 2);
    assert_eq!(done["usage"]["tokensCached"], json!(15 * calls), "{done}");
    assert_eq!(
        done["usage"]["tokensCacheWrite"],
        json!(7 * calls),
        "{done}"
    );
    for purpose in ["recognise", "import"] {
        let missing: i64 = q1(
            &e.t,
            &format!("SELECT COUNT(*) FROM gen_calls WHERE purpose = '{purpose}' AND COALESCE(tokens_cached, -1) != 15"),
        );
        assert_eq!(missing, 0, "{purpose}");
    }
    // AI 用量 sums them as well
    let (s, v) = e.t.get("/api/ai/admin/usage", &e.admin).await;
    assert_eq!(s, 200, "{v}");
    assert!(
        v["totals"]["tokensCached"].as_i64().unwrap_or(0) >= 15 * calls,
        "{v}"
    );
    assert!(
        v["totals"]["tokensCacheWrite"].as_i64().unwrap_or(0) >= 7 * calls,
        "{v}"
    );
}

#[tokio::test]
async fn a_model_that_misses_questions_makes_the_count_about_n() {
    let e = env_whole().await;
    // a model that skips every ninth question: 63 of 70; the local count of "A." lines says 70
    let inner = good_model();
    e.t.stub.st().script = Some(Arc::new(move |system: &str, user: &str| {
        if task_of(user).contains("TASK RECOGNISE") {
            Reply::Text(whole_recognise_answer(user, 9))
        } else {
            inner(system, user)
        }
    }));
    let id = upload_ok(&e, &e.user, "workbook.docx", docx_of(&unnumbered_lines())).await;
    let j = wait_for(&e.t, &e.user, id, Some("confirm"), &["failed", "cancelled"]).await;
    assert_eq!(
        (j["total"].clone(), j["taken"].clone(), j["approx"].clone()),
        (json!(63), json!(50), json!(true)),
        "{j}"
    );
}

#[tokio::test]
async fn an_unreadable_recognition_is_asked_once_more_then_the_windows_take_over() {
    let e = env_whole().await;
    let inner = good_model();
    e.t.stub.st().script = Some(Arc::new(move |system: &str, user: &str| {
        if task_of(user).contains("TASK RECOGNISE") {
            Reply::Text("I cannot tell.".to_string())
        } else {
            inner(system, user)
        }
    }));
    let id = upload_ok(&e, &e.user, "Kitchen notes.docx", docx_of(&six())).await;
    let j = wait_for(&e.t, &e.user, id, Some("confirm"), &["failed", "cancelled"]).await;
    assert_eq!(j["phase"], "confirm", "{j}");
    assert_eq!(
        (j["total"].clone(), j["taken"].clone()),
        (json!(6), json!(6))
    );
    let rec = whole_calls(&e.t)
        .iter()
        .filter(|(_, u)| task_of(u).contains("TASK RECOGNISE"))
        .count();
    assert_eq!(rec, 2, "one retry");
    assert!(windowed_calls(&e.t) >= 1, "the windows ran");
}

#[tokio::test]
async fn a_file_that_does_not_fit_the_model_takes_the_windowed_path() {
    let e = env().await;
    // a model that names a context of 300 tokens: nothing of a real file fits
    e.t.app.0.knobs.import_context_tokens.store(300, SeqCst);
    let id = upload_ok(&e, &e.user, "Kitchen notes.docx", docx_of(&six())).await;
    let j = wait_for(&e.t, &e.user, id, Some("confirm"), &["failed", "cancelled"]).await;
    assert_eq!(j["phase"], "confirm", "{j}");
    assert_eq!(
        (j["total"].clone(), j["taken"].clone(), j["approx"].clone()),
        (json!(6), json!(6), json!(false))
    );
    assert!(whole_calls(&e.t).is_empty(), "no whole-file call");
    assert!(windowed_calls(&e.t) >= 1);
    let (s, v) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200, "{v}");
    let done = wait_for(&e.t, &e.user, id, None, &["done", "failed"]).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["made"], json!(6));
}

#[tokio::test]
async fn a_material_file_writes_from_line_ranges_after_the_same_prefix() {
    let e = env_whole().await;
    let id = upload_ok(&e, &e.user, "handbook.docx", docx_of(&material_lines(40))).await;
    let j = wait_for(&e.t, &e.user, id, Some("confirm"), &["failed", "cancelled"]).await;
    assert_eq!(j["mode"], "material", "{j}");
    assert_eq!(j["read"], json!({"done": 1, "total": 1}));
    let (s, v) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200, "{v}");
    let done = wait_for(&e.t, &e.user, id, None, &["done", "failed"]).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["made"], json!(20));
    let calls = whole_calls(&e.t);
    let first = prefix_of(&calls[0].1).to_string();
    let writes: Vec<&(String, String)> = calls
        .iter()
        .filter(|(_, u)| task_of(u).contains("TASK WRITE"))
        .collect();
    assert!(!writes.is_empty());
    for (_, u) in &writes {
        assert_eq!(prefix_of(u), first);
        assert!(task_of(u).contains("\"lines\":\"L"));
        assert!(task_of(u).contains("ALREADY WRITTEN"));
        assert!(
            !task_of(u).contains("probe reads hot"),
            "the excerpt text stays in the prefix"
        );
    }
}

// —— sprint 13 fixes ———————————————————————————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn sixty_numbered_questions_on_the_windowed_path_give_total_sixty_and_not_about() {
    let e = env().await;
    e.t.post(
        "/api/ai/admin/limits",
        &e.admin,
        json!({"importMaxQuestions": 50}),
    )
    .await;
    let id = upload_ok(&e, &e.user, "sixty.docx", docx_of(&questions_lines(60))).await;
    let j = wait_for(&e.t, &e.user, id, Some("confirm"), &["failed", "cancelled"]).await;
    assert_eq!(j["phase"], "confirm", "{j}");
    assert_eq!(
        (j["total"].clone(), j["goal"].clone(), j["approx"].clone()),
        (json!(60), json!(50), json!(false)),
        "{j}"
    );
}

#[tokio::test]
async fn whole_file_give_up_per_slot_with_unlimited_off_and_on() {
    for unlimited in [0, 1] {
        let e = env_whole().await;
        e.t.post(
            "/api/ai/admin/limits",
            &e.admin,
            json!({"importUnlimited": unlimited}),
        )
        .await;
        never_delivers_whole(&e, &[3]);
        let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
        wait_for(&e.t, &e.user, id, Some("confirm"), &["failed"]).await;
        let (s, _) = confirm(&e, &e.user, id).await;
        assert_eq!(s, 200);
        let done = wait_for(&e.t, &e.user, id, None, &["done", "failed"]).await;
        assert_eq!(done["state"], "done", "unlimited {unlimited}: {done}");
        let goal = done["goal"].as_i64().expect("goal");
        assert!(goal >= 4, "unlimited {unlimited}: {done}");
        assert_eq!(
            (done["made"].clone(), done["skipped"].clone()),
            (json!(goal - 1), json!([3])),
            "unlimited {unlimited}: {done}"
        );
        assert_eq!(windowed_calls(&e.t), 0, "the whole-file path ran");
        let body =
            e.t.backend
                .st()
                .gen
                .created
                .first()
                .cloned()
                .expect("create");
        assert_eq!(body["skipped"], json!([3]));
        assert_eq!(
            body["questions"].as_array().map(|v| v.len() as i64),
            Some(goal - 1)
        );
    }
}

#[tokio::test]
async fn whole_file_cancel_and_keep_makes_the_partial_bank_and_usage_is_live_while_writing() {
    let e = env_whole().await;
    e.t.post(
        "/api/ai/admin/limits",
        &e.admin,
        json!({"importUnlimited": 1}),
    )
    .await;
    e.t.stub.st().cached_tokens = Some(5);
    e.t.stub.st().cache_write_tokens = Some(9);
    never_delivers_whole(&e, &[3, 4, 5, 6]);
    let id = upload_ok(&e, &e.user, "course.docx", docx_of(&six())).await;
    wait_for(&e.t, &e.user, id, Some("confirm"), &["failed"]).await;
    e.t.stub.st().head_delay_ms = 300;
    let (s, _) = confirm(&e, &e.user, id).await;
    assert_eq!(s, 200);
    let mut live = Value::Null;
    for _ in 0..400 {
        let j = job_of(&e.t, &e.user, id).await;
        if j["made"].as_i64().unwrap_or(0) >= 2 {
            live = j["usage"].clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    // the figures are there while the job is still writing
    assert!(live["calls"].as_i64().unwrap_or(0) >= 2, "{live}");
    assert!(
        live["tokensCacheWrite"].as_i64().unwrap_or(0) >= 18,
        "{live}"
    );
    let (s, v) =
        e.t.post(
            "/api/ai/gen/cancel",
            &e.user,
            json!({"id": id, "keep": true}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let done = wait_for(&e.t, &e.user, id, None, &["done", "failed", "cancelled"]).await;
    e.t.stub.st().head_delay_ms = 0;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["made"], 2);
    assert_eq!(done["skipped"], json!([3, 4]));
    let st = e.t.backend.st();
    let body = st.gen.created.first().expect("the bank was created");
    assert_eq!(body["questions"].as_array().map(Vec::len), Some(2));
}
