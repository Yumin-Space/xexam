//! Activity events of the administrators' AI actions (`ai.*`), delivered to the main backend's activity log.
//!
//! An event is one JSON line in the audit outbox (no schema change): `App::activity` writes the line in a short
//! transaction and never waits for the backend; the delivery sends the lines that parse as an event to
//! `POST /internal/ai/activity` (batches of at most 50) and older plain lines to `/internal/ai/log`.
//! `App::activity_now` is for the one event that must be delivered before the action happens (an administrator reading
//! a user's chats).
//!
//! What an event may carry is an allow-list ([`DETAIL_KEYS`]): names, hosts, counts and codes, never a key, a hint, a
//! password, a token, a cookie, or any question, answer, chat or plan text.
use crate::identity::{ClientInfo, Identity};
use crate::util::{cut_chars, hex, unix_now};
use serde_json::{json, Map, Value};

/// Event lines older than this are not sent any more (the backend refuses `at` older than 7 days).
pub const MAX_AGE_SECS: i64 = 6 * 86_400;
/// Most events in one delivery batch (the backend's limit).
pub const BATCH: usize = 50;
/// The keys an event's `detail` may have (rust-contracts 5.2). Anything else is dropped before the line is written.
pub const DETAIL_KEYS: [&str; 22] = [
    "provider",
    "host",
    "changed",
    "keys",
    "models",
    "count",
    "secret",
    "change",
    "oldHost",
    "newHost",
    "slots",
    "connection",
    "model",
    "cleared",
    "on",
    "off",
    "suspended",
    "free",
    "messages",
    "stored",
    "code",
    "questions",
];

/// One activity event, before the id and the time are added.
#[derive(Debug, Clone)]
pub struct Event {
    /// The acting user; `None` = the system (a connection turning broken, AI pausing)
    pub who: Option<Identity>,
    /// `ai.<group>.<name>`
    pub action: &'static str,
    /// Connection name, slot, model id, a username (the target of an admin action), a scope; never content
    pub object: Option<String>,
    pub detail: Value,
    pub ok: bool,
    /// An up-code or a refusal code when `ok` is false (or the result of a test)
    pub reason: Option<String>,
    pub client: ClientInfo,
}

impl Event {
    /// A successful event with no detail.
    pub fn new(who: Option<&Identity>, action: &'static str, client: &ClientInfo) -> Event {
        Event {
            who: who.cloned(),
            action,
            object: None,
            detail: Value::Null,
            ok: true,
            reason: None,
            client: client.clone(),
        }
    }

    pub fn object(mut self, object: impl Into<String>) -> Event {
        self.object = Some(object.into());
        self
    }

    pub fn detail(mut self, detail: Value) -> Event {
        self.detail = detail;
        self
    }

    /// Marks the event as failed with a code.
    pub fn failed(mut self, reason: &str) -> Event {
        self.ok = false;
        self.reason = Some(reason.to_string());
        self
    }
}

/// `ai.` followed by 1 to 60 letters, digits or dots (the backend's rule).
pub fn valid_action(a: &str) -> bool {
    a.strip_prefix("ai.").is_some_and(|rest| {
        (1..=60).contains(&rest.len())
            && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.')
    })
}

/// The detail with only the allowed keys; a string value is cleaned and cut at 200 characters, a list keeps at most
/// 20 such strings. A detail that is not an object, or has nothing left, is `null`.
pub fn clean_detail(detail: &Value) -> Value {
    fn scalar(v: &Value) -> Option<Value> {
        match v {
            Value::String(s) => {
                let clean: String = s.chars().filter(|c| !c.is_control()).collect();
                Some(Value::String(cut_chars(&clean, 200)))
            }
            Value::Number(_) | Value::Bool(_) => Some(v.clone()),
            _ => None,
        }
    }
    let Some(obj) = detail.as_object() else {
        return Value::Null;
    };
    let mut out = Map::new();
    for key in DETAIL_KEYS {
        let Some(v) = obj.get(key) else { continue };
        let kept = match v {
            Value::Array(a) => Some(Value::Array(a.iter().take(20).filter_map(scalar).collect())),
            other => scalar(other),
        };
        if let Some(k) = kept {
            out.insert(key.to_string(), k);
        }
    }
    if out.is_empty() {
        Value::Null
    } else {
        Value::Object(out)
    }
}

/// The event as the backend wants it, keys in this order: `id`, `at`, `userId`, `stamp`, `username`, `action`, `object`,
/// `detail`, `result`, `reason`, `ip`, `ua`. `id` is 32 lower-case hex digits.
pub fn payload(ev: &Event, id: &str, at: i64) -> Value {
    let (user_id, stamp, username) = match &ev.who {
        Some(w) => (
            Value::from(w.id),
            Value::from(w.stamp.clone()),
            Value::from(w.username.clone()),
        ),
        None => (Value::Null, Value::Null, Value::Null),
    };
    let object = ev.object.as_deref().map(|o| {
        let clean: String = o.chars().filter(|c| !c.is_control()).collect();
        cut_chars(&clean, 200)
    });
    json!({
        "id": id,
        "at": at,
        "userId": user_id,
        "stamp": stamp,
        "username": username,
        "action": ev.action,
        "object": object,
        "detail": clean_detail(&ev.detail),
        "result": if ev.ok { "ok" } else { "fail" },
        "reason": ev.reason.as_deref().map(|r| cut_chars(r, 100)),
        "ip": ev.client.ip,
        "ua": ev.client.ua,
    })
}

/// A fresh event id: 16 random bytes as 32 hex digits.
pub fn new_id() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("the operating system's random source failed");
    hex(&b)
}

/// The event an outbox line holds, if it is one (a JSON object with a valid `ai.` action and a 32-hex id).
pub fn parse_line(line: &str) -> Option<Value> {
    if !line.starts_with('{') {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let o = v.as_object()?;
    let action_ok = o
        .get("action")
        .and_then(Value::as_str)
        .is_some_and(valid_action);
    let id_ok = o
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|i| crate::util::is_lower_hex(i, 32));
    (action_ok && id_ok && o.get("at").is_some_and(Value::is_i64)).then_some(v)
}

/// The event line for the outbox: the payload with a new id at `at`, as compact JSON.
pub fn line_of(ev: &Event, at: i64) -> String {
    payload(ev, &new_id(), at).to_string()
}

/// Starts the task that delivers the outbox soon after an event was written (the 30-second loop of the retention task
/// stays the safety net). Call once from `main`.
pub fn spawn_delivery(app: crate::app::AppState) -> tokio::task::JoinHandle<()> {
    let signal = app.0.outbox_signal();
    tokio::spawn(async move {
        let mut down = app.0.shutdown.clone();
        loop {
            tokio::select! {
                _ = signal.notified() => {}
                _ = down.wait_for(|d| *d) => return,
            }
            // Let a burst of events (one admin action can emit several) arrive, then send them together
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            app.0.deliver_outbox().await;
        }
    })
}

/// `true` when the event's `at` is still deliverable.
pub fn fresh_enough(at: i64, now: i64) -> bool {
    at >= now - MAX_AGE_SECS
}

/// The current time for a new event line.
pub fn now() -> i64 {
    unix_now()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Role;

    fn who() -> Identity {
        Identity {
            id: 3,
            stamp: "ab".repeat(16),
            username: "alice".into(),
            role: Role::Admin,
            ai_allowed: true,
            sid: "sid".into(),
        }
    }

    fn client() -> ClientInfo {
        ClientInfo {
            ip: Some("203.0.113.7".into()),
            ua: Some("Demo/1.0".into()),
        }
    }

    #[test]
    fn actions_follow_the_backend_rule() {
        for ok in [
            "ai.settings.key",
            "ai.gen.create",
            "ai.a",
            "ai.System.Paused",
        ] {
            assert!(valid_action(ok), "{ok}");
        }
        let long = format!("ai.{}", "a".repeat(61));
        for bad in [
            "",
            "ai",
            "ai.",
            "AI.x",
            "ai.x y",
            "ai.x_y",
            "ai.é",
            "ai.x/y",
            long.as_str(),
        ] {
            assert!(!valid_action(bad), "{bad}");
        }
        assert!(valid_action(&format!("ai.{}", "a".repeat(60))));
    }

    #[test]
    fn the_payload_has_the_contract_keys_in_order() {
        let ev = Event::new(Some(&who()), "ai.settings.keyReuse", &client())
            .object("main")
            .detail(json!({"oldHost": "old.example.com", "newHost": "api.example.com", "slots": ["apiKey"]}));
        let id = "0123456789abcdef0123456789abcdef";
        let v = payload(&ev, id, 1_794_640_837);
        let keys: Vec<&str> = v
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            vec![
                "id", "at", "userId", "stamp", "username", "action", "object", "detail", "result",
                "reason", "ip", "ua"
            ]
        );
        assert_eq!(v["id"], id);
        assert_eq!(v["userId"], 3);
        assert_eq!(v["username"], "alice");
        assert_eq!(v["result"], "ok");
        assert_eq!(v["reason"], Value::Null);
        assert_eq!(v["ip"], "203.0.113.7");
        assert_eq!(v["detail"]["slots"], json!(["apiKey"]));
        // a system event has no user; a failed one names its reason
        let sys = Event::new(None, "ai.system.connection", &ClientInfo::default())
            .object("main")
            .failed("ai.up.auth");
        let v = payload(&sys, id, 5);
        assert_eq!(
            (
                v["userId"].clone(),
                v["stamp"].clone(),
                v["username"].clone()
            ),
            (Value::Null, Value::Null, Value::Null)
        );
        assert_eq!(
            (v["result"].clone(), v["reason"].clone()),
            (json!("fail"), json!("ai.up.auth"))
        );
        assert_eq!(v["ip"], Value::Null);
    }

    #[test]
    fn only_allowed_detail_keys_survive_and_nothing_secret_can_ride_along() {
        let d = json!({
            "provider": "openai", "host": "api.example.com", "count": 2, "secret": "apiKey", "change": "set",
            "apiKey": "sk-marker-secret-123", "hint": "…1234", "password": "pw", "text": "a question",
            "models": 4, "slots": ["apiKey", {"nested": "x"}], "changed": ["name", "baseUrl"], "cleared": true
        });
        let c = clean_detail(&d);
        let s = c.to_string();
        for leaked in ["sk-marker-secret-123", "1234", "pw", "a question", "nested"] {
            assert!(!s.contains(leaked), "{leaked} leaked: {s}");
        }
        assert_eq!(c["secret"], "apiKey");
        assert_eq!(c["slots"], json!(["apiKey"]));
        assert_eq!(c["count"], 2);
        assert_eq!(clean_detail(&json!({"apiKey": "x"})), Value::Null);
        assert_eq!(clean_detail(&json!("string")), Value::Null);
        assert_eq!(clean_detail(&Value::Null), Value::Null);
        // long strings are cut, control characters removed, long lists cut
        let long = clean_detail(&json!({"host": format!("a\nb{}", "h".repeat(400))}));
        let host = long["host"].as_str().expect("host");
        assert_eq!(host.chars().count(), 200);
        assert!(!host.contains('\n'));
        let many: Vec<String> = (0..50).map(|i| format!("s{i}")).collect();
        assert_eq!(
            clean_detail(&json!({"changed": many}))["changed"]
                .as_array()
                .map(Vec::len),
            Some(20)
        );
        // the allow-list is exactly the contract's keys
        assert_eq!(DETAIL_KEYS.len(), 22);
    }

    #[test]
    fn object_and_ids_are_cleaned() {
        let ev = Event::new(Some(&who()), "ai.users.features", &client())
            .object(format!("bo\tb{}", "x".repeat(300)));
        let v = payload(&ev, &new_id(), 1);
        let o = v["object"].as_str().expect("object");
        assert_eq!(o.chars().count(), 200);
        assert!(!o.contains('\t'));
        let id = new_id();
        assert!(crate::util::is_lower_hex(&id, 32));
        assert_ne!(id, new_id());
    }

    #[test]
    fn an_outbox_line_is_an_event_only_when_it_is_a_whole_one() {
        let ev = Event::new(Some(&who()), "ai.settings.enable", &client());
        let line = line_of(&ev, 100);
        let parsed = parse_line(&line).expect("event line");
        assert_eq!(parsed["action"], "ai.settings.enable");
        assert!(parse_line("AI settings were changed by alice").is_none());
        assert!(parse_line("{not json").is_none());
        assert!(parse_line(r#"{"action":"ai.x"}"#).is_none(), "no id");
        assert!(parse_line(r#"{"id":"short","action":"ai.x","at":1}"#).is_none());
        assert!(parse_line(
            r#"{"id":"0123456789abcdef0123456789abcdef","action":"bank.upload","at":1}"#
        )
        .is_none());
        assert!(parse_line(
            r#"{"id":"0123456789abcdef0123456789abcdef","action":"ai.x","at":"1"}"#
        )
        .is_none());
        assert!(parse_line("[1,2]").is_none());
    }

    #[test]
    fn old_events_are_not_deliverable() {
        let now = 10_000_000;
        assert!(fresh_enough(now, now));
        assert!(fresh_enough(now - MAX_AGE_SECS, now));
        assert!(!fresh_enough(now - MAX_AGE_SECS - 1, now));
    }
}
