//! 黄金响应比对：把 `tests/baseline/golden/*.json`（172 项，旧版 server.py 的真实响应）逐项按顺序重放到新路由上。
//! 解析器和数据库用假实现，由每一项黄金响应里的内容喂给假实现（所以这里验证的是 API 层：守卫、状态码、头、
//! 字节级 JSON 格式、键顺序、预览/确认的状态机、attempt 清洗的错误路径），不是解析器或数据库本身。
//! 运行 `cargo test --test api_golden -- --nocapture` 可看到汇总表。
#[path = "api_common/mod.rs"]
mod common;
use axum::body::Body;
use common::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use xexam_backend::api::json::py_json;
use xexam_backend::error::ImportError;
use xexam_backend::models::ImportRecord;

const GOLDEN: &str = "C:/Users/Min/Desktop/Claude/xexam/tests/baseline/golden";
const VOLATILE_KEYS: [&str; 5] = ["token", "mtime", "at", "version", "arch"];

fn normalize(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, x)| {
                    let nv = if VOLATILE_KEYS.contains(&k.as_str())
                        && x.as_str().is_some_and(|s| !s.is_empty())
                    {
                        Value::from("<VOLATILE>")
                    } else {
                        normalize(x)
                    };
                    (k.clone(), nv)
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(normalize).collect()),
        other => other.clone(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Matched,
    MatchedEngineFaked,
    Skipped,
    Intended,
    Differs,
}

struct Row {
    num: String,
    name: String,
    outcome: Outcome,
    note: String,
}

/// 把黄金文件里的请求还原成字节。大请求体（`<N bytes>`）按 capture.py 的构造方式重建。
fn request_body(name: &str, body: &Option<String>) -> Option<Vec<u8>> {
    let b = body.as_ref()?;
    if name == "json_invalid_utf8" {
        return Some(b"{\"a\":\"\xff\"}".to_vec());
    }
    if let Some(n) = b
        .strip_prefix('<')
        .and_then(|r| r.strip_suffix(" bytes>"))
        .and_then(|n| n.parse::<usize>().ok())
    {
        return Some(match name {
            "json_too_large_1000001" => {
                let mut v = b"{\"pad\":\"".to_vec();
                v.extend(std::iter::repeat_n(b'a', 999_990));
                v.extend_from_slice(b"\"}");
                v.extend(std::iter::repeat_n(b' ', 10));
                v
            }
            "json_deeply_nested" => [vec![b'['; 100_000], vec![b']'; 100_000]].concat(),
            _ => vec![0u8; n],
        });
    }
    Some(b.as_bytes().to_vec())
}

fn load() -> Vec<(String, Value)> {
    let mut files: Vec<_> = std::fs::read_dir(GOLDEN)
        .expect("golden dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|p| {
            let fname = p.file_name()?.to_string_lossy().to_string();
            if fname.len() > 3
                && fname[..3].chars().all(|c| c.is_ascii_digit())
                && fname.ends_with(".json")
            {
                let v: Value = serde_json::from_slice(&std::fs::read(&p).ok()?).ok()?;
                Some((fname[..3].to_string(), v))
            } else {
                None
            }
        })
        .collect()
}

/// 按黄金响应里的内容配置假引擎。返回 true 表示这一项的引擎行为是被喂进去的。
fn prime(engine: &FakeEngine, g: &Value) -> bool {
    let method = g["request"]["method"].as_str().unwrap_or("");
    let path = g["request"]["path"].as_str().unwrap_or("");
    let path = path.split(['?', '#']).next().unwrap_or("");
    let status = g["status"].as_u64().unwrap_or(0);
    let j = &g["json"];
    engine.with(|f| {
        f.parse = None;
        f.apply = None;
    });
    let mut faked = false;
    match (method, path) {
        ("GET" | "HEAD", "/api/info") if status == 200 => {
            if let Some(n) = j["bankCount"].as_i64() {
                engine.with(|f| f.count = n);
                faked = true;
            }
        }
        ("GET", "/api/bank") if status == 200 => {
            engine.with(|f| f.bank = j.clone());
            faked = true;
        }
        ("GET", "/api/progress") if status == 200 => {
            engine.with(|f| f.progress = j.clone());
            faked = true;
        }
        ("GET", "/api/imports") if status == 200 => {
            let recs: Vec<ImportRecord> =
                serde_json::from_value(j["imports"].clone()).unwrap_or_default();
            engine.with(|f| f.imports = recs);
            faked = true;
        }
        ("POST", "/api/upload") => {
            if status == 200 {
                let usable = j["usable"].as_u64().unwrap_or(0) as usize;
                let mut plan = j.clone();
                if let Value::Object(m) = &mut plan {
                    m.remove("filename");
                    m.remove("token");
                    if let Some(Value::Array(ss)) = m.get_mut("sheets") {
                        for s in ss {
                            if let Value::Object(so) = s {
                                so.remove("skippedMore");
                                so.remove("skippedCount");
                            }
                        }
                    }
                }
                engine.with(|f| {
                    f.parse = Some(Ok(workbook(usable)));
                    f.plan = plan;
                });
                faked = true;
            } else if status == 400 {
                let msg = g["json"]["error"].as_str().unwrap_or("").to_string();
                engine.with(|f| f.parse = Some(Err(ImportError::Message(msg))));
                faked = true;
            }
        }
        ("POST", "/api/import/confirm") if status == 200 => {
            engine.with(|f| {
                f.apply = Some(Ok(j["result"].clone()));
                f.bank = j["bank"].clone();
            });
            faked = true;
        }
        ("POST", "/api/attempt" | "/api/reset") if status == 200 => {
            engine.with(|f| {
                f.attempt_result = j.clone();
                f.progress = j.clone();
            });
            faked = true;
        }
        _ => {}
    }
    faked
}

#[tokio::test]
async fn replay_all_golden_captures() {
    let goldens = load();
    assert_eq!(goldens.len(), 172, "expected the 172 golden captures");
    let normal = app_with(config_with(&[]), None);
    let mut shutdown_app = app_with(
        {
            let mut c = default_config();
            c.allow_shutdown = true;
            c
        },
        None,
    );
    let mut shutdown_rx = shutdown_app.shutdown_rx.take_for_test();
    let mut token_map: HashMap<String, String> = HashMap::new();
    let mut rows: Vec<Row> = Vec::new();

    for (num, g) in &goldens {
        let name = g["name"].as_str().unwrap_or("").to_string();
        let req = &g["request"];
        let method = req["method"].as_str().unwrap_or("GET").to_string();
        let path = req["path"].as_str().unwrap_or("/").to_string();
        let host_header = &req["host_header"];
        let is_shutdown_inst = host_header.as_str().is_some_and(|h| h.ends_with(":8703"));
        let mut push = |outcome: Outcome, note: &str| {
            rows.push(Row {
                num: num.clone(),
                name: name.clone(),
                outcome,
                note: note.to_string(),
            })
        };

        // 不属于 API 层的项
        if g.get("no_response").is_some() || g["status"].is_null() {
            push(
                Outcome::Skipped,
                "no HTTP response was captured (server dropped / refused the connection)",
            );
            continue;
        }
        let status = g["status"].as_u64().unwrap_or(0) as u16;
        let static_get = method != "POST" && !path.starts_with("/api/") && status == 200;
        if static_get {
            push(
                Outcome::Skipped,
                "static file: served by Nginx, not by the backend",
            );
            continue;
        }
        if status == 501 {
            // 旧版 501 是标准库的偶然产物；迁移计划 Q13：改为 405 + Allow
            let app = &normal;
            let r = send(
                &app.router,
                build(&method, &path, host_header.as_str(), &[], Body::empty()),
            )
            .await;
            let ok = r.status == 405 && r.header("allow").is_some();
            push(
                if ok {
                    Outcome::Intended
                } else {
                    Outcome::Differs
                },
                &format!(
                    "old 501 (stdlib HTML) -> new {} + Allow (migration-plan Q13)",
                    r.status
                ),
            );
            continue;
        }

        let a = if is_shutdown_inst {
            &shutdown_app
        } else {
            &normal
        };
        let faked = prime(&a.engine, g);

        // 重建请求
        let mut hs: Vec<(String, Vec<u8>)> = vec![];
        let mut has_cl = false;
        if let Some(m) = req["headers"].as_object() {
            for (k, v) in m {
                let mut val = v.as_str().unwrap_or("").to_string();
                if k.eq_ignore_ascii_case("content-length") {
                    has_cl = true;
                }
                for (gt, lt) in &token_map {
                    val = val.replace(gt, lt);
                }
                hs.push((k.clone(), val.into_bytes()));
            }
        }
        let mut body = request_body(&name, &req["body"].as_str().map(|s| s.to_string()));
        if let Some(b) = &mut body {
            if b.len() < 1000 {
                if let Ok(text) = std::str::from_utf8(b) {
                    let mut s = text.to_string();
                    for (gt, lt) in &token_map {
                        s = s.replace(gt, lt);
                    }
                    *b = s.into_bytes();
                }
            }
        }
        if let Some(b) = &body {
            if !has_cl {
                hs.push((
                    "content-length".to_string(),
                    b.len().to_string().into_bytes(),
                ));
            }
        }
        let hrefs: Vec<(&str, &[u8])> =
            hs.iter().map(|(k, v)| (k.as_str(), v.as_slice())).collect();
        let r = send(
            &a.router,
            build(
                &method,
                &path,
                host_header.as_str(),
                &hrefs,
                body.map(Body::from).unwrap_or_else(Body::empty),
            ),
        )
        .await;

        // 比对
        let mut diffs: Vec<String> = vec![];
        if r.status.as_u16() != status {
            diffs.push(format!("status {} != {}", r.status, status));
        }
        let gh = g["headers_stable"].as_object().cloned().unwrap_or_default();
        for k in ["content-type", "cache-control", "x-content-type-options"] {
            let want = gh.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
            let got = r.header(k);
            if want != got {
                diffs.push(format!("header {k}: {got:?} != {want:?}"));
            }
        }
        if r.headers.get("server").is_some() {
            diffs.push("unexpected Server header".into());
        }
        // 我们自己的 Content-Length 必须等于真实长度（HEAD 例外：长度是 GET 的长度）
        if method != "HEAD" && r.header("content-length") != Some(r.body.len().to_string()) {
            diffs.push("Content-Length != body length".into());
        }
        let golden_norm = &g["normalized"];
        if g["json"].is_null() {
            // 纯文本（或 HEAD 的空正文）
            if method == "HEAD" {
                if !r.body.is_empty() {
                    diffs.push("HEAD with a body".into());
                }
            } else if let Some(raw) = g["raw_body"].as_str() {
                if r.text() != raw {
                    diffs.push(format!("text body {:?} != {:?}", r.text(), raw));
                }
            } else if g["body_len"].as_u64() != Some(r.body.len() as u64) {
                // 静态文件类的 404：黄金只记了长度和 md5（"not found" = 9 字节）
                diffs.push(format!(
                    "body length {} != {:?}",
                    r.body.len(),
                    g["body_len"]
                ));
            }
        } else {
            let ours = r.json();
            let ours_norm = normalize(&ours);
            // 键顺序：用同一个格式化器重新序列化后逐字节比较
            if py_json(&ours_norm) != py_json(golden_norm) {
                diffs.push(format!(
                    "normalized body differs:\n    ours  : {}\n    golden: {}",
                    String::from_utf8_lossy(&py_json(&ours_norm))
                        .chars()
                        .take(300)
                        .collect::<String>(),
                    String::from_utf8_lossy(&py_json(golden_norm))
                        .chars()
                        .take(300)
                        .collect::<String>()
                ));
            }
            // 没有易变值的响应：原始字节必须完全一致（含 Content-Length）
            let has_volatile = py_json(&ours_norm) != py_json(&ours);
            if !has_volatile {
                if r.body != g["raw_body"].as_str().unwrap_or("").as_bytes() {
                    diffs.push("raw body bytes differ from golden".into());
                }
                if let Some(cl) = gh.get("content-length").and_then(|v| v.as_str()) {
                    if method != "HEAD" && r.header("content-length").as_deref() != Some(cl) {
                        diffs.push(format!(
                            "content-length {:?} != {cl}",
                            r.header("content-length")
                        ));
                    }
                }
            }
        }
        // 记住令牌：黄金里的令牌 -> 这次真实发出的令牌
        if path == "/api/upload" && status == 200 {
            if let (Some(gt), Some(lt)) = (g["json"]["token"].as_str(), r.json()["token"].as_str())
            {
                token_map.insert(gt.to_string(), lt.to_string());
            }
        }
        if num == "171" {
            // shutdown 开启：应当触发了停机信号
            let got = shutdown_rx.try_recv().is_ok();
            if !got {
                diffs.push("shutdown signal was not sent".into());
            }
        }
        if diffs.is_empty() {
            push(
                if faked {
                    Outcome::MatchedEngineFaked
                } else {
                    Outcome::Matched
                },
                "",
            );
        } else {
            push(Outcome::Differs, &diffs.join("; "));
        }
    }

    // ---- 汇总表 ----
    let count = |o: Outcome| rows.iter().filter(|r| r.outcome == o).count();
    println!("\n=== golden replay: API layer vs 172 captures of server.py v0.2 ===");
    println!("captured                      : {}", rows.len());
    println!(
        "matched (API layer only)      : {}",
        count(Outcome::Matched)
    );
    println!(
        "matched (engine fed by golden): {}",
        count(Outcome::MatchedEngineFaked)
    );
    println!(
        "intended differences          : {}",
        count(Outcome::Intended)
    );
    println!(
        "skipped                       : {}",
        count(Outcome::Skipped)
    );
    println!(
        "DIFFERING                     : {}",
        count(Outcome::Differs)
    );
    for r in rows.iter().filter(|r| {
        matches!(
            r.outcome,
            Outcome::Intended | Outcome::Skipped | Outcome::Differs
        )
    }) {
        println!("  [{:?}] {} {} -- {}", r.outcome, r.num, r.name, r.note);
    }
    let bad: Vec<String> = rows
        .iter()
        .filter(|r| r.outcome == Outcome::Differs)
        .map(|r| format!("{} {}: {}", r.num, r.name, r.note))
        .collect();
    assert!(bad.is_empty(), "differing captures:\n{}", bad.join("\n"));
    assert_eq!(rows.len(), 172);
    let _ = json!(null);
}

/// `Option<Receiver>` 里取出接收端（测试里只关心有没有收到信号）。
trait TakeForTest {
    fn take_for_test(&mut self) -> tokio::sync::mpsc::Receiver<()>;
}
impl TakeForTest for Option<tokio::sync::mpsc::Receiver<()>> {
    fn take_for_test(&mut self) -> tokio::sync::mpsc::Receiver<()> {
        self.take().expect("receiver")
    }
}
