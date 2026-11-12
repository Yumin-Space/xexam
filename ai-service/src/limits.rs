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
    /// AI model calls per user: rolling 60 s window
    pub calls_per_minute: i64,
    /// AI model calls per user: rolling 60 min window
    pub calls_per_hour: i64,
    /// AI model calls per user: since today's 0:00 in the server's time zone
    pub calls_per_day: i64,
    /// Longest answer in characters (zh / ja characters; in English 4 letters count as 1); sent to the model as
    /// `max_tokens` (see [`reply_tokens`])
    pub reply_max_chars: i64,
}

/// One limit: JSON key, database column, default, minimum, maximum.
pub struct LimitDef {
    pub key: &'static str,
    pub column: &'static str,
    pub default: i64,
    pub min: i64,
    pub max: i64,
}

/// The ten limits, in the order of the JSON answers (the per-user call caps, then the answer length, come last).
/// The retired `limits.message_chars` column is not listed: nothing reads or writes it any more.
pub const DEFS: [LimitDef; 10] = [
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
        default: 200,
        min: 50,
        max: 2000,
    },
    LimitDef {
        key: "callsPerMinute",
        column: "calls_per_minute",
        default: 5,
        min: 1,
        max: 100,
    },
    LimitDef {
        key: "callsPerHour",
        column: "calls_per_hour",
        default: 20,
        min: 1,
        max: 2000,
    },
    LimitDef {
        key: "callsPerDay",
        column: "calls_per_day",
        default: 100,
        min: 1,
        max: 20_000,
    },
    LimitDef {
        key: "replyMaxChars",
        column: "reply_max_chars",
        default: 1000,
        min: 1,
        max: 16_000,
    },
];

/// The `max_tokens` the model is given for an answer of at most `chars` characters (the answer-length limit
/// `replyMaxChars`), by the reply language: Chinese and Japanese 1 character = 1.5 tokens, rounded up, so that an
/// answer can really reach the character count; English 1 character = 4 letters = about 1 token. Computed per call and
/// never stored. A call without a known language uses the Chinese / Japanese factor. The frontend has the same
/// function (`replyTokens`) with the same test vectors.
pub fn reply_tokens(chars: i64, lang: Option<crate::prompt::Lang>) -> i64 {
    let chars = chars.max(0);
    match lang {
        Some(crate::prompt::Lang::En) => chars,
        _ => chars.saturating_mul(3).saturating_add(1) / 2,
    }
}

/// Characters of memories put into the system prompt (whole notes only).
pub const MEMORY_PROMPT_CHARS: usize = 4000;
/// History budget floor (4.5); the budget is `max(24000, MESSAGE_CHARS + 8000)`, which is this floor.
pub const HISTORY_FLOOR: usize = 24_000;
/// The fixed internal cap of one user message (characters). Not an administrator's setting: it only protects the
/// server (v1.0: the per-message limit is expressed by the answer length, not by an admin field).
pub const MESSAGE_CHARS: usize = 16_000;
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
            calls_per_minute: 0,
            calls_per_hour: 0,
            calls_per_day: 0,
            reply_max_chars: 0,
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
            "callsPerMinute" => self.calls_per_minute,
            "callsPerHour" => self.calls_per_hour,
            "callsPerDay" => self.calls_per_day,
            "replyMaxChars" => self.reply_max_chars,
            _ => 0,
        }
    }

    pub(crate) fn set(&mut self, key: &str, v: i64) {
        match key {
            "perUserRunning" => self.per_user_running = v,
            "globalRunning" => self.global_running = v,
            "chatsPerUser" => self.chats_per_user = v,
            "messagesPerChat" => self.messages_per_chat = v,
            "memoriesPerUser" => self.memories_per_user = v,
            "memoryChars" => self.memory_chars = v,
            "callsPerMinute" => self.calls_per_minute = v,
            "callsPerHour" => self.calls_per_hour = v,
            "callsPerDay" => self.calls_per_day = v,
            "replyMaxChars" => self.reply_max_chars = v,
            _ => {}
        }
    }

    /// The history budget in characters: `max(24000, MESSAGE_CHARS + 8000)`, a constant since the per-message limit is
    /// fixed.
    pub fn history_budget(&self) -> usize {
        HISTORY_FLOOR.max(MESSAGE_CHARS.saturating_add(8000))
    }

    /// The three call caps as the pure counting function wants them.
    pub fn caps(&self) -> crate::quota::Caps {
        crate::quota::Caps {
            minute: self.calls_per_minute,
            hour: self.calls_per_hour,
            day: self.calls_per_day,
        }
    }

    /// The order rule of the call caps (`callsPerMinute <= callsPerHour <= callsPerDay`; equal is allowed). Returns the
    /// field to blame: the larger window whose value is too small (`callsPerHour` first, then `callsPerDay`), or `None`.
    /// Checked by the update route after `apply_update` (400 `ai.limitOrder` with `field`).
    pub fn order_violation(&self) -> Option<&'static str> {
        if self.calls_per_hour < self.calls_per_minute {
            Some("callsPerHour")
        } else if self.calls_per_day < self.calls_per_hour {
            Some("callsPerDay")
        } else {
            None
        }
    }

    /// Applies an update (any subset of the ten keys, integers only), all or nothing.
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

    /// `"chatsPerUser 1000 -> 2000, memoryChars 200 -> 300"` for the keys that differ, or `None`.
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
    use crate::prompt::Lang;
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
        assert_eq!(l.memory_chars, 200);
        assert_eq!(l.reply_max_chars, 1000);
        assert_eq!(
            (l.calls_per_minute, l.calls_per_hour, l.calls_per_day),
            (5, 20, 100)
        );
        assert_eq!(l.history_budget(), 24000);
        assert_eq!(MESSAGE_CHARS, 16_000);
    }

    #[test]
    fn reply_tokens_vectors() {
        // The same vectors as the frontend's replyTokens
        let v = [
            (1000, Some(Lang::Zh), 1500),
            (1000, Some(Lang::Ja), 1500),
            (1000, Some(Lang::En), 1000),
            (1, Some(Lang::Zh), 2),
            (1, Some(Lang::En), 1),
            (333, Some(Lang::Ja), 500),
            (16000, Some(Lang::Zh), 24000),
            (16000, Some(Lang::En), 16000),
            (300, Some(Lang::Zh), 450),
            (1000, None, 1500),
        ];
        for (chars, lang, want) in v {
            assert_eq!(reply_tokens(chars, lang), want, "{chars} {lang:?}");
        }
    }

    #[test]
    fn validation_is_all_or_nothing() {
        let l = Limits::default();
        assert_eq!(
            l.apply_update(&obj(json!({"chatsPerUser": 2000, "replyMaxChars": 0}))),
            Err(LimitError::Range("replyMaxChars"))
        );
        assert_eq!(
            l.apply_update(&obj(json!({"replyMaxChars": 16001}))),
            Err(LimitError::Range("replyMaxChars"))
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
        // messageChars left the adjustable limits: an unknown key now
        assert_eq!(
            l.apply_update(&obj(json!({"messageChars": 16000}))),
            Err(LimitError::BadRequest)
        );
        let n = l
            .apply_update(&obj(json!({"chatsPerUser": 2000, "replyMaxChars": 16000})))
            .expect("ok");
        assert_eq!(
            l.describe_change(&n).as_deref(),
            Some("chatsPerUser 1000 -> 2000, replyMaxChars 1000 -> 16000")
        );
        assert_eq!(l.describe_change(&l), None);
    }

    #[test]
    fn answer_shape() {
        let a = Limits::default().answer();
        let s = serde_json::to_string(&a).expect("json");
        assert!(s.starts_with("{\"limits\":{\"perUserRunning\":1,\"globalRunning\":16,"));
        assert!(s.contains("\"ranges\":{\"perUserRunning\":[1,4]"));
        // ten keys: the call caps, then the answer length last
        let keys: Vec<&String> = a["limits"].as_object().expect("object").keys().collect();
        assert_eq!(keys.len(), 10);
        assert_eq!(
            &keys[5..],
            &[
                "memoryChars",
                "callsPerMinute",
                "callsPerHour",
                "callsPerDay",
                "replyMaxChars"
            ]
        );
        assert_eq!(a["defaults"], a["limits"]);
        assert_eq!(a["limits"]["replyMaxChars"], json!(1000));
        assert_eq!(a["limits"]["memoryChars"], json!(200));
        assert_eq!(a["ranges"]["replyMaxChars"], json!([1, 16000]));
        assert_eq!(a["ranges"]["callsPerMinute"], json!([1, 100]));
        assert_eq!(a["ranges"]["callsPerHour"], json!([1, 2000]));
        assert_eq!(a["ranges"]["callsPerDay"], json!([1, 20000]));
        assert!(a["limits"].get("messageChars").is_none());
    }

    #[test]
    fn call_caps_are_validated_and_ordered() {
        let l = Limits::default();
        assert_eq!(
            l.apply_update(&obj(json!({"callsPerMinute": 0}))),
            Err(LimitError::Range("callsPerMinute"))
        );
        assert_eq!(
            l.apply_update(&obj(json!({"callsPerMinute": 101}))),
            Err(LimitError::Range("callsPerMinute"))
        );
        assert_eq!(
            l.apply_update(&obj(json!({"callsPerHour": 2001}))),
            Err(LimitError::Range("callsPerHour"))
        );
        assert_eq!(
            l.apply_update(&obj(json!({"callsPerDay": 20001}))),
            Err(LimitError::Range("callsPerDay"))
        );
        let ok = l
            .apply_update(&obj(
                json!({"callsPerMinute": 10, "callsPerHour": 10, "callsPerDay": 10}),
            ))
            .expect("equal is allowed");
        assert_eq!(ok.order_violation(), None);
        assert_eq!(Limits::default().order_violation(), None);
        // minute > hour blames the hour field
        let bad = l
            .apply_update(&obj(json!({"callsPerMinute": 30})))
            .expect("in range");
        assert_eq!(bad.order_violation(), Some("callsPerHour"));
        // hour > day blames the day field
        let bad = l
            .apply_update(&obj(json!({"callsPerHour": 200})))
            .expect("in range");
        assert_eq!(bad.order_violation(), Some("callsPerDay"));
        let caps = ok.caps();
        assert_eq!((caps.minute, caps.hour, caps.day), (10, 10, 10));
    }

    #[test]
    fn describe_change_names_the_call_caps() {
        let l = Limits::default();
        let n = l
            .apply_update(&obj(json!({"callsPerHour": 30})))
            .expect("ok");
        assert_eq!(
            l.describe_change(&n).as_deref(),
            Some("callsPerHour 20 -> 30")
        );
    }
}
