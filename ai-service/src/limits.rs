//! Safety limits that administrators can adjust (spec 14.1). They protect the server; they are not usage limits.
//!
//! Stored in the single-row table `limits` (not part of the settings fingerprint, so changing them never switches AI
//! off). Lowering a limit never deletes or truncates existing data; it only refuses new items beyond it.
use serde_json::{Map, Value};

/// The adjustable limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub per_user_running: i64,
    pub global_running: i64,
    pub chats_per_user: i64,
    pub messages_per_chat: i64,
    pub memories_per_user: i64,
    pub memory_chars: i64,
    pub message_chars: i64,
}

/// One limit: JSON key, database column, default, minimum, maximum.
pub struct LimitDef {
    pub key: &'static str,
    pub column: &'static str,
    pub default: i64,
    pub min: i64,
    pub max: i64,
}

/// The seven limits, in the order of the JSON answers.
pub const DEFS: [LimitDef; 7] = [
    LimitDef {
        key: "perUserRunning",
        column: "per_user_running",
        default: 1,
        min: 1,
        max: 4,
    },
    LimitDef {
        key: "globalRunning",
        column: "global_running",
        default: 16,
        min: 1,
        max: 64,
    },
    LimitDef {
        key: "chatsPerUser",
        column: "chats_per_user",
        default: 1000,
        min: 10,
        max: 100_000,
    },
    LimitDef {
        key: "messagesPerChat",
        column: "messages_per_chat",
        default: 400,
        min: 20,
        max: 5000,
    },
    LimitDef {
        key: "memoriesPerUser",
        column: "memories_per_user",
        default: 50,
        min: 1,
        max: 1000,
    },
    LimitDef {
        key: "memoryChars",
        column: "memory_chars",
        default: 300,
        min: 50,
        max: 2000,
    },
    LimitDef {
        key: "messageChars",
        column: "message_chars",
        default: 16000,
        min: 1000,
        max: 32000,
    },
];

/// Characters of memories put into the system prompt (whole notes only).
pub const MEMORY_PROMPT_CHARS: usize = 4000;
/// History budget floor (4.5); the budget is `max(24000, messageChars + 8000)`.
pub const HISTORY_FLOOR: usize = 24_000;
/// Longest streamed answer (characters); then the upstream request is dropped (`ai.up.tooLong`).
pub const MAX_ANSWER_CHARS: usize = 100_000;
/// Request bodies read by hand, at most this many bytes.
pub const MAX_BODY: usize = 128 * 1024;
/// A hint is cut after this many characters (the guard enforces it; the run ends as `done`, truncated).
pub const HINT_MAX_CHARS: usize = 6_000;
/// A summary prompt never carries more than this many characters of items (not adjustable).
pub const SUMMARY_PROMPT_CHARS: usize = 60_000;

impl Default for Limits {
    fn default() -> Self {
        let mut l = Limits {
            per_user_running: 0,
            global_running: 0,
            chats_per_user: 0,
            messages_per_chat: 0,
            memories_per_user: 0,
            memory_chars: 0,
            message_chars: 0,
        };
        for d in &DEFS {
            l.set(d.key, d.default);
        }
        l
    }
}

/// Why a limits update was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitError {
    /// Not a JSON object, or an unknown key (400 `ai.badRequest`)
    BadRequest,
    /// Out of range or not an integer (400 `ai.limitRange` with `field`)
    Range(&'static str),
}

impl Limits {
    pub fn get(&self, key: &str) -> i64 {
        match key {
            "perUserRunning" => self.per_user_running,
            "globalRunning" => self.global_running,
            "chatsPerUser" => self.chats_per_user,
            "messagesPerChat" => self.messages_per_chat,
            "memoriesPerUser" => self.memories_per_user,
            "memoryChars" => self.memory_chars,
            "messageChars" => self.message_chars,
            _ => 0,
        }
    }

    fn set(&mut self, key: &str, v: i64) {
        match key {
            "perUserRunning" => self.per_user_running = v,
            "globalRunning" => self.global_running = v,
            "chatsPerUser" => self.chats_per_user = v,
            "messagesPerChat" => self.messages_per_chat = v,
            "memoriesPerUser" => self.memories_per_user = v,
            "memoryChars" => self.memory_chars = v,
            "messageChars" => self.message_chars = v,
            _ => {}
        }
    }

    /// The history budget in characters.
    pub fn history_budget(&self) -> usize {
        let m = usize::try_from(self.message_chars).unwrap_or(0);
        HISTORY_FLOOR.max(m.saturating_add(8000))
    }

    /// Applies an update (any subset of the seven keys, integers only), all or nothing.
    pub fn apply_update(&self, body: &Map<String, Value>) -> Result<Limits, LimitError> {
        for k in body.keys() {
            if !DEFS.iter().any(|d| d.key == k) {
                return Err(LimitError::BadRequest);
            }
        }
        let mut next = *self;
        for d in &DEFS {
            if let Some(v) = body.get(d.key) {
                let n = match v {
                    Value::Number(n) => n.as_i64(),
                    _ => None,
                };
                match n {
                    Some(n) if (d.min..=d.max).contains(&n) => next.set(d.key, n),
                    _ => return Err(LimitError::Range(d.key)),
                }
            }
        }
        Ok(next)
    }

    /// `"chatsPerUser 1000 -> 2000, messageChars 16000 -> 20000"` for the keys that differ, or `None`.
    pub fn describe_change(&self, next: &Limits) -> Option<String> {
        let parts: Vec<String> = DEFS
            .iter()
            .filter(|d| self.get(d.key) != next.get(d.key))
            .map(|d| format!("{} {} -> {}", d.key, self.get(d.key), next.get(d.key)))
            .collect();
        (!parts.is_empty()).then(|| parts.join(", "))
    }

    /// `{"<key>": n, …}` in the order of [`DEFS`].
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        for d in &DEFS {
            m.insert(d.key.to_string(), Value::from(self.get(d.key)));
        }
        Value::Object(m)
    }

    /// The answer of `GET /api/ai/admin/limits`.
    pub fn answer(&self) -> Value {
        let mut ranges = Map::new();
        for d in &DEFS {
            ranges.insert(
                d.key.to_string(),
                Value::Array(vec![Value::from(d.min), Value::from(d.max)]),
            );
        }
        let mut m = Map::new();
        m.insert("limits".into(), self.to_json());
        m.insert("defaults".into(), Limits::default().to_json());
        m.insert("ranges".into(), Value::Object(ranges));
        Value::Object(m)
    }

    pub fn usize_of(v: i64) -> usize {
        usize::try_from(v).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn defaults_and_budget() {
        let l = Limits::default();
        assert_eq!(l.per_user_running, 1);
        assert_eq!(l.global_running, 16);
        assert_eq!(l.chats_per_user, 1000);
        assert_eq!(l.messages_per_chat, 400);
        assert_eq!(l.memories_per_user, 50);
        assert_eq!(l.memory_chars, 300);
        assert_eq!(l.message_chars, 16000);
        assert_eq!(l.history_budget(), 24000);
        let big = l
            .apply_update(&obj(json!({"messageChars": 32000})))
            .expect("ok");
        assert_eq!(big.history_budget(), 40000);
    }

    #[test]
    fn validation_is_all_or_nothing() {
        let l = Limits::default();
        assert_eq!(
            l.apply_update(&obj(json!({"chatsPerUser": 2000, "messageChars": 999}))),
            Err(LimitError::Range("messageChars"))
        );
        assert_eq!(
            l.apply_update(&obj(json!({"perUserRunning": 2.5}))),
            Err(LimitError::Range("perUserRunning"))
        );
        assert_eq!(
            l.apply_update(&obj(json!({"perUserRunning": "2"}))),
            Err(LimitError::Range("perUserRunning"))
        );
        assert_eq!(
            l.apply_update(&obj(json!({"bogus": 1}))),
            Err(LimitError::BadRequest)
        );
        let n = l
            .apply_update(&obj(json!({"chatsPerUser": 2000, "messageChars": 20000})))
            .expect("ok");
        assert_eq!(
            l.describe_change(&n).as_deref(),
            Some("chatsPerUser 1000 -> 2000, messageChars 16000 -> 20000")
        );
        assert_eq!(l.describe_change(&l), None);
    }

    #[test]
    fn answer_shape() {
        let a = Limits::default().answer();
        let s = serde_json::to_string(&a).expect("json");
        assert!(s.starts_with("{\"limits\":{\"perUserRunning\":1,\"globalRunning\":16,"));
        assert!(s.contains("\"ranges\":{\"perUserRunning\":[1,4]"));
    }
}
