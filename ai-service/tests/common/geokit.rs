//! Test doubles of the external IP-location lookup (v1.4): the fake backend's `/internal/ai/geo/*` routes and a local
//! stub provider on 127.0.0.1. Nothing here reaches another host; every place is made up ("Example City").
#![allow(dead_code)]

use super::{json_answer, text_answer, BackendState};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::response::Response;
use axum::Router;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How the fake backend answers `geo/next`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NextSwitch {
    #[default]
    Normal,
    /// An older backend without the route
    Http404,
    Http500,
}

/// The geo side of the fake backend.
#[derive(Default)]
pub struct GeoBackend {
    /// Jobs waiting to be handed out: (id, ip)
    pub queue: Vec<(String, String)>,
    /// Jobs handed out so far
    pub handed: Vec<(String, String)>,
    /// Bodies of the `geo/next` calls
    pub next_bodies: Vec<Value>,
    /// Every result object received, in order
    pub results: Vec<Value>,
    /// Size of every `geo/result` batch
    pub batches: Vec<usize>,
    /// The location source ("" = cloudflare)
    pub source: String,
    /// Bodies of the `geo/source` calls
    pub source_calls: Vec<Value>,
    /// Number of `geo/source` reads (empty body)
    pub source_reads: u32,
    pub next_switch: NextSwitch,
    /// Answer `geo/source` with HTTP 500
    pub source_down: bool,
}

impl GeoBackend {
    /// The source now in effect.
    pub fn source_name(&self) -> &str {
        if self.source == "api" {
            "api"
        } else {
            "cloudflare"
        }
    }

    /// Queues a job for the next `geo/next`.
    pub fn push_job(&mut self, id: &str, ip: &str) {
        self.queue.push((id.to_string(), ip.to_string()));
    }
}

/// `POST /internal/ai/geo/result` and `/internal/ai/geo/source` of the fake backend.
pub fn answer(s: &mut BackendState, path: &str, body: &Value) -> Result<Value, u16> {
    match path {
        "/internal/ai/geo/result" => {
            let list = body
                .get("results")
                .and_then(Value::as_array)
                .ok_or(400u16)?;
            if list.is_empty() || list.len() > 10 {
                return Err(400);
            }
            s.geo.batches.push(list.len());
            s.geo.results.extend(list.iter().cloned());
            Ok(json!({"ok": true}))
        }
        "/internal/ai/geo/source" => {
            if s.geo.source_down {
                return Err(500);
            }
            // a read ({}) is not a switch
            if body.get("source").is_some() {
                s.geo.source_calls.push(body.clone());
            }
            s.geo.source_reads += u32::from(body.get("source").is_none());
            if let Some(src) = body.get("source").and_then(Value::as_str) {
                if src != "api" && src != "cloudflare" {
                    return Err(400);
                }
                s.geo.source = src.to_string();
            }
            Ok(json!({"source": s.geo.source_name()}))
        }
        _ => Err(404),
    }
}

/// `POST /internal/ai/geo/next`: hands out up to 10 queued jobs; with none queued it waits (at most 300 ms of the
/// asked `waitMs`) for one to arrive.
pub async fn next_answer(st: &Arc<Mutex<BackendState>>, body: &Value) -> Response {
    let wait = body
        .get("waitMs")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(300);
    let deadline = Instant::now() + Duration::from_millis(wait);
    let mut recorded = false;
    loop {
        {
            let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
            if !recorded {
                recorded = true;
                s.geo.next_bodies.push(body.clone());
            }
            match s.geo.next_switch {
                NextSwitch::Http404 => return text_answer(404, "not found"),
                NextSwitch::Http500 => return text_answer(500, "boom"),
                NextSwitch::Normal => {}
            }
            let take = s.geo.queue.len().min(10);
            if take > 0 {
                let jobs: Vec<(String, String)> = s.geo.queue.drain(..take).collect();
                s.geo.handed.extend(jobs.iter().cloned());
                let lookups: Vec<Value> = jobs
                    .iter()
                    .map(|(id, ip)| json!({"id": id, "ip": ip}))
                    .collect();
                return json_answer(200, &json!({ "lookups": lookups }));
            }
            if Instant::now() >= deadline {
                return json_answer(200, &json!({"lookups": []}));
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// —— stub provider ————————————————————————————————————————————————————————————————————————————————————————————————————

/// One request the stub provider received.
#[derive(Debug, Clone)]
pub struct GeoReq {
    pub path: String,
    pub query: Option<String>,
    pub headers: Vec<(String, String)>,
}

impl GeoReq {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The path and query as sent.
    pub fn target(&self) -> String {
        match &self.query {
            Some(q) => format!("{}?{q}", self.path),
            None => self.path.clone(),
        }
    }
}

/// A local provider. The first path segment picks the behaviour, the last one is the address:
/// `ok` (a place), `ipapi` (ip-api shape), `ipapi-fail`, `ipapico-err`, `slow` (answers after 2 s), `s500`, `s404`,
/// `text` (not JSON), `array` (a JSON array), `empty` (no mapped fields), `redir/<port>` (302 to another stub),
/// `big` (more than 64 KiB), `echo` (a failure whose message repeats the key it was sent).
pub struct GeoStub {
    pub url: String,
    pub port: u16,
    pub seen: Arc<Mutex<Vec<GeoReq>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for GeoStub {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl GeoStub {
    pub fn requests(&self) -> Vec<GeoReq> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn count(&self) -> usize {
        self.requests().len()
    }

    /// A URL template for a behaviour: `http://127.0.0.1:<port>/<kind>/{ip}`.
    pub fn template(&self, kind: &str) -> String {
        format!("{}/{kind}/{{ip}}", self.url)
    }
}

fn place_ok(ip: &str) -> Value {
    json!({"ip": ip, "city": "Example City", "region": "Example Region", "country": "ZZ"})
}

async fn stub_handler(State(seen): State<Arc<Mutex<Vec<GeoReq>>>>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(str::to_string);
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    seen.lock().unwrap_or_else(|e| e.into_inner()).push(GeoReq {
        path: path.clone(),
        query: query.clone(),
        headers: headers.clone(),
    });
    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let kind = segs.first().copied().unwrap_or("");
    let ip = segs.last().copied().unwrap_or("");
    match kind {
        "ok" => json_answer(200, &place_ok(ip)),
        "ipapi" => json_answer(
            200,
            &json!({"status": "success", "country": "Exampleland", "countryCode": "ZZ",
                    "regionName": "Example Region", "city": "Example City"}),
        ),
        "ipapi-fail" => json_answer(200, &json!({"status": "fail", "message": "reserved range"})),
        "ipapico-err" => json_answer(200, &json!({"error": true, "reason": "RateLimited"})),
        "slow" => {
            tokio::time::sleep(Duration::from_secs(2)).await;
            json_answer(200, &place_ok(ip))
        }
        "s500" => text_answer(500, "boom"),
        "s404" => text_answer(404, "nothing here"),
        "text" => text_answer(200, "<html>not json</html>"),
        "array" => json_answer(200, &json!([1, 2, 3])),
        "empty" => json_answer(200, &json!({"ip": ip, "other": 1})),
        "redir" => {
            let port = segs.get(1).copied().unwrap_or("1");
            let mut r = Response::new(Body::empty());
            *r.status_mut() = axum::http::StatusCode::FOUND;
            r.headers_mut().insert(
                "location",
                format!("http://127.0.0.1:{port}/ok/{ip}")
                    .parse()
                    .expect("location"),
            );
            r
        }
        "big" => json_answer(
            200,
            &json!({"city": "Example City", "pad": "x".repeat(70 * 1024)}),
        ),
        "echo" => {
            // the key as the provider received it: the `token` parameter or the Authorization header
            let from_query = query
                .as_deref()
                .and_then(|q| q.split('&').find_map(|p| p.strip_prefix("token=")))
                .unwrap_or("");
            let from_header = headers
                .iter()
                .find(|(k, _)| k == "authorization")
                .map(|(_, v)| v.as_str())
                .unwrap_or("");
            json_answer(
                200,
                &json!({"error": true, "message": format!("denied for {from_query}{from_header}")}),
            )
        }
        _ => text_answer(404, "no such behaviour"),
    }
}

/// Starts a stub provider on 127.0.0.1.
pub async fn geo_stub() -> GeoStub {
    let seen: Arc<Mutex<Vec<GeoReq>>> = Arc::new(Mutex::new(Vec::new()));
    let router = Router::new()
        .fallback(stub_handler)
        .with_state(seen.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = l.local_addr().expect("addr");
    let task = tokio::spawn(async move {
        let _ = axum::serve(l, router).await;
    });
    GeoStub {
        url: format!("http://{addr}"),
        port: addr.port(),
        seen,
        task,
    }
}
