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
    /// Longest memory note, in tokens (estimated by [`estimate_tokens`])
    pub memory_tokens: i64,
    /// AI model calls per user: rolling 60 s window
    pub calls_per_minute: i64,
    /// AI model calls per user: rolling 60 min window
    pub calls_per_hour: i64,
    /// AI model calls per user: since today's 0:00 in the server's time zone
    pub calls_per_day: i64,
    /// Longest normal answer (chat, hint, explanation, summary) in tokens; sent to the model as `max_tokens`
    pub reply_max_tokens: i64,
    /// AI bank generation and import: units per user per day (shared; 0:00 reset)
    pub gen_per_day: i64,
    /// 1 = administrators are not limited by `gen_per_day`
    pub gen_admins_exempt: i64,
    /// Fewest single / multiple-choice questions a bank needs before it can be a generation source
    pub gen_min_source: i64,
    /// Fewest wrong-answer-book questions for the wrong-answer mode
    pub gen_min_wrong: i64,
    pub gen_bank_single: i64,
    pub gen_bank_multi: i64,
    pub gen_wrong_single: i64,
    pub gen_wrong_multi: i64,
    /// Longest answer of one generation / import call, in tokens
    pub gen_call_tokens: i64,
    /// Most model calls one generation job may use, for everyone incl. administrators (A-OWNER-36)
    pub gen_max_calls: i64,
    /// Most model calls one file-import job may use, for everyone incl. administrators (A-OWNER-36b)
    pub import_max_calls: i64,
    /// Most model calls the recognising step of one file import may use, before the confirm (A-OWNER-38)
    pub recognize_max_calls: i64,
    /// 1 = AI bank generation has no call ceiling, no per-slot retry cap and no deadline (A-OWNER-45); the 3-errors-in-a-row stop and cancel stay
    pub gen_unlimited: i64,
    /// 1 = the same for file import, recognition included (A-OWNER-45)
    pub import_unlimited: i64,
    /// Generation / import jobs running at once, server-wide
    pub gen_running: i64,
    /// Largest uploaded file of an AI import, in MiB
    pub import_max_mb: i64,
    /// Most questions one import may convert
    pub import_max_questions: i64,
}

/// One limit: JSON key, database column, default, minimum, maximum.
pub struct LimitDef {
    pub key: &'static str,
    pub column: &'static str,
    pub default: i64,
    pub min: i64,
    pub max: i64,
}

/// The 27 limits, in the order of the JSON answers (the ten of v1.0, then the generation settings, then the import ones).
/// The retired `limits.message_chars` column is not listed: nothing reads or writes it any more.
pub const DEFS: [LimitDef; 27] = [
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
        key: "memoryTokens",
        column: "memory_tokens",
        default: 1000,
        min: 50,
        max: 250_000,
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
        key: "replyMaxTokens",
        column: "reply_max_tokens",
        default: 3000,
        min: 1,
        max: 250_000,
    },
    LimitDef {
        key: "genPerDay",
        column: "gen_per_day",
        default: 3,
        min: 0,
        max: 100,
    },
    LimitDef {
        key: "genAdminsExempt",
        column: "gen_admins_exempt",
        default: 1,
        min: 0,
        max: 1,
    },
    LimitDef {
        key: "genMinSource",
        column: "gen_min_source",
        default: 30,
        min: 5,
        max: 2000,
    },
    LimitDef {
        key: "genMinWrong",
        column: "gen_min_wrong",
        default: 10,
        min: 1,
        max: 500,
    },
    LimitDef {
        key: "genBankSingle",
        column: "gen_bank_single",
        default: 14,
        min: 0,
        max: 100,
    },
    LimitDef {
        key: "genBankMulti",
        column: "gen_bank_multi",
        default: 6,
        min: 0,
        max: 100,
    },
    LimitDef {
        key: "genWrongSingle",
        column: "gen_wrong_single",
        default: 14,
        min: 0,
        max: 100,
    },
    LimitDef {
        key: "genWrongMulti",
        column: "gen_wrong_multi",
        default: 6,
        min: 0,
        max: 100,
    },
    LimitDef {
        key: "genCallTokens",
        column: "gen_call_tokens",
        default: 20_000,
        min: 1000,
        max: 250_000,
    },
    LimitDef {
        key: "genMaxCalls",
        column: "gen_max_calls",
        default: 15,
        min: 8,
        max: 40,
    },
    LimitDef {
        key: "importMaxCalls",
        column: "import_max_calls",
        default: 25,
        min: 8,
        max: 40,
    },
    LimitDef {
        key: "recognizeMaxCalls",
        column: "recognize_max_calls",
        default: 20,
        min: 8,
        max: 40,
    },
    LimitDef {
        key: "genUnlimited",
        column: "gen_unlimited",
        default: 1,
        min: 0,
        max: 1,
    },
    LimitDef {
        key: "importUnlimited",
        column: "import_unlimited",
        default: 1,
        min: 0,
        max: 1,
    },
    LimitDef {
        key: "genRunning",
        column: "gen_running",
        default: 2,
        min: 1,
        max: 8,
    },
    LimitDef {
        key: "importMaxMb",
        column: "import_max_mb",
        default: 20,
        min: 1,
        max: 20,
    },
    LimitDef {
        key: "importMaxQuestions",
        column: "import_max_questions",
        default: 50,
        min: 1,
        max: 50,
    },
];

/// Tokens of one Chinese / Japanese character, as 3 / 2: the factor the generation planner uses to turn a call's token
/// budget into the amount of text it can hold. An estimate for planning only; no setting uses it any more.
pub fn chars_to_tokens(chars: i64) -> i64 {
    chars.max(0).saturating_mul(3).saturating_add(1) / 2
}

/// The `want` that stands for "no answer length of our own" (A-OWNER-50): a job with its "unlimited" switch on asks for
/// this many tokens; the provider layer then sends no token field (OpenAI-compatible) or the largest value the model
/// accepts (Anthropic, where the API requires one).
pub const UNLIMITED_TOKENS: i64 = 250_000;

/// The inverse of [`chars_to_tokens`]: characters of Chinese / Japanese text that fit in `tokens` (rounded down).
pub fn tokens_to_chars(tokens: i64) -> i64 {
    tokens.max(0).saturating_mul(2) / 3
}

/// Estimated tokens of a text: ASCII characters 1/4 token each, every other character 3/2 token, rounded up. Used only
/// where the service has to enforce a token setting itself (the memory length).
pub fn estimate_tokens(s: &str) -> i64 {
    let quarters: i64 = s.chars().map(|c| if c.is_ascii() { 1 } else { 6 }).sum();
    (quarters + 3) / 4
}

/// The longest prefix of `s` whose [`estimate_tokens`] is at most `max`.
pub fn cut_to_tokens(s: &str, max: i64) -> String {
    let budget = max.max(0).saturating_mul(4);
    let mut used = 0i64;
    let mut out = String::new();
    for c in s.chars() {
        used += if c.is_ascii() { 1 } else { 6 };
        if used > budget {
            break;
        }
        out.push(c);
    }
    out
}

/// Characters of memories put into the system prompt (whole notes only).
pub const MEMORY_PROMPT_CHARS: usize = 4000;
/// History budget floor (4.5); the budget is `max(24000, MESSAGE_CHARS + 8000)`, which is this floor.
pub const HISTORY_FLOOR: usize = 24_000;
/// The fixed internal cap of one user message (characters). Not an administrator's setting: it only protects the
/// server (v1.0: the per-message limit is expressed by the answer length, not by an admin field).
pub const MESSAGE_CHARS: usize = 16_000;
/// Longest streamed answer (characters; 250,000 tokens of English are about 1,000,000); then the upstream request is dropped (`ai.up.tooLong`).
pub const MAX_ANSWER_CHARS: usize = 1_000_000;
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
            memory_tokens: 0,
            calls_per_minute: 0,
            calls_per_hour: 0,
            calls_per_day: 0,
            reply_max_tokens: 0,
            gen_per_day: 0,
            gen_admins_exempt: 0,
            gen_min_source: 0,
            gen_min_wrong: 0,
            gen_bank_single: 0,
            gen_bank_multi: 0,
            gen_wrong_single: 0,
            gen_wrong_multi: 0,
            gen_call_tokens: 0,
            gen_max_calls: 0,
            import_max_calls: 0,
            recognize_max_calls: 0,
            gen_unlimited: 0,
            import_unlimited: 0,
            gen_running: 0,
            import_max_mb: 0,
            import_max_questions: 0,
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
    /// The per-call answer length of an AI generation, in tokens: the setting, or [`UNLIMITED_TOKENS`] when the
    /// generation's "unlimited" switch is on (A-OWNER-50).
    pub fn gen_tokens(&self) -> i64 {
        if self.gen_unlimited == 1 {
            UNLIMITED_TOKENS
        } else {
            self.gen_call_tokens
        }
    }

    /// The same for an AI file import (its own "unlimited" switch).
    pub fn import_tokens(&self) -> i64 {
        if self.import_unlimited == 1 {
            UNLIMITED_TOKENS
        } else {
            self.gen_call_tokens
        }
    }

    pub fn get(&self, key: &str) -> i64 {
        match key {
            "perUserRunning" => self.per_user_running,
            "globalRunning" => self.global_running,
            "chatsPerUser" => self.chats_per_user,
            "messagesPerChat" => self.messages_per_chat,
            "memoriesPerUser" => self.memories_per_user,
            "memoryTokens" => self.memory_tokens,
            "callsPerMinute" => self.calls_per_minute,
            "callsPerHour" => self.calls_per_hour,
            "callsPerDay" => self.calls_per_day,
            "replyMaxTokens" => self.reply_max_tokens,
            "genPerDay" => self.gen_per_day,
            "genAdminsExempt" => self.gen_admins_exempt,
            "genMinSource" => self.gen_min_source,
            "genMinWrong" => self.gen_min_wrong,
            "genBankSingle" => self.gen_bank_single,
            "genBankMulti" => self.gen_bank_multi,
            "genWrongSingle" => self.gen_wrong_single,
            "genWrongMulti" => self.gen_wrong_multi,
            "genCallTokens" => self.gen_call_tokens,
            "genMaxCalls" => self.gen_max_calls,
            "importMaxCalls" => self.import_max_calls,
            "recognizeMaxCalls" => self.recognize_max_calls,
            "genUnlimited" => self.gen_unlimited,
            "importUnlimited" => self.import_unlimited,
            "genRunning" => self.gen_running,
            "importMaxMb" => self.import_max_mb,
            "importMaxQuestions" => self.import_max_questions,
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
            "memoryTokens" => self.memory_tokens = v,
            "callsPerMinute" => self.calls_per_minute = v,
            "callsPerHour" => self.calls_per_hour = v,
            "callsPerDay" => self.calls_per_day = v,
            "replyMaxTokens" => self.reply_max_tokens = v,
            "genPerDay" => self.gen_per_day = v,
            "genAdminsExempt" => self.gen_admins_exempt = v,
            "genMinSource" => self.gen_min_source = v,
            "genMinWrong" => self.gen_min_wrong = v,
            "genBankSingle" => self.gen_bank_single = v,
            "genBankMulti" => self.gen_bank_multi = v,
            "genWrongSingle" => self.gen_wrong_single = v,
            "genWrongMulti" => self.gen_wrong_multi = v,
            "genCallTokens" => self.gen_call_tokens = v,
            "genMaxCalls" => self.gen_max_calls = v,
            "importMaxCalls" => self.import_max_calls = v,
            "recognizeMaxCalls" => self.recognize_max_calls = v,
            "genUnlimited" => self.gen_unlimited = v,
            "importUnlimited" => self.import_unlimited = v,
            "genRunning" => self.gen_running = v,
            "importMaxMb" => self.import_max_mb = v,
            "importMaxQuestions" => self.import_max_questions = v,
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

    /// The composition rule of AI bank generation (checked after an update is applied): single + multiple must be 1 ..= 100
    /// in each mode. Returns the multi key of the failing mode (bank mode first), or `None`.
    pub fn gen_count_violation(&self) -> Option<&'static str> {
        let ok = |n: i64| (1..=100).contains(&n);
        if !ok(self.gen_bank_single + self.gen_bank_multi) {
            Some("genBankMulti")
        } else if !ok(self.gen_wrong_single + self.gen_wrong_multi) {
            Some("genWrongMulti")
        } else {
            None
        }
    }

    /// Applies an update (any subset of the 27 keys, integers only), all or nothing.
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

    /// `"chatsPerUser 1000 -> 2000, memoryTokens 1000 -> 2000"` for the keys that differ, or `None`.
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
        assert_eq!(l.memory_tokens, 1000);
        assert_eq!(l.reply_max_tokens, 3000);
        assert_eq!(
            (l.calls_per_minute, l.calls_per_hour, l.calls_per_day),
            (5, 20, 100)
        );
        assert_eq!(l.history_budget(), 24000);
        assert_eq!(MESSAGE_CHARS, 16_000);
    }

    #[test]
    fn token_helpers() {
        assert_eq!(chars_to_tokens(1000), 1500);
        assert_eq!(chars_to_tokens(1), 2);
        assert_eq!(tokens_to_chars(1500), 1000);
        assert_eq!(tokens_to_chars(20_000), 13_333);
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("\u{4e2d}\u{6587}"), 3);
        assert_eq!(cut_to_tokens("abcdefgh", 1), "abcd");
        assert_eq!(
            cut_to_tokens("\u{4e2d}\u{6587}\u{5b57}", 3),
            "\u{4e2d}\u{6587}"
        );
        assert_eq!(cut_to_tokens("abc", 0), "");
    }

    #[test]
    fn validation_is_all_or_nothing() {
        let l = Limits::default();
        assert_eq!(
            l.apply_update(&obj(json!({"chatsPerUser": 2000, "replyMaxTokens": 0}))),
            Err(LimitError::Range("replyMaxTokens"))
        );
        assert_eq!(
            l.apply_update(&obj(json!({"replyMaxTokens": 250001}))),
            Err(LimitError::Range("replyMaxTokens"))
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
            .apply_update(&obj(json!({"chatsPerUser": 2000, "replyMaxTokens": 16000})))
            .expect("ok");
        assert_eq!(
            l.describe_change(&n).as_deref(),
            Some("chatsPerUser 1000 -> 2000, replyMaxTokens 3000 -> 16000")
        );
        assert_eq!(l.describe_change(&l), None);
    }

    #[test]
    fn answer_shape() {
        let a = Limits::default().answer();
        let s = serde_json::to_string(&a).expect("json");
        assert!(s.starts_with("{\"limits\":{\"perUserRunning\":1,\"globalRunning\":16,"));
        assert!(s.contains("\"ranges\":{\"perUserRunning\":[1,4]"));
        // 27 keys: v1.0's ten (the call caps, then the answer length), then generation, then import
        let keys: Vec<&String> = a["limits"].as_object().expect("object").keys().collect();
        assert_eq!(keys.len(), 27);
        assert_eq!(
            &keys[5..],
            &[
                "memoryTokens",
                "callsPerMinute",
                "callsPerHour",
                "callsPerDay",
                "replyMaxTokens",
                "genPerDay",
                "genAdminsExempt",
                "genMinSource",
                "genMinWrong",
                "genBankSingle",
                "genBankMulti",
                "genWrongSingle",
                "genWrongMulti",
                "genCallTokens",
                "genMaxCalls",
                "importMaxCalls",
                "recognizeMaxCalls",
                "genUnlimited",
                "importUnlimited",
                "genRunning",
                "importMaxMb",
                "importMaxQuestions"
            ]
        );
        assert_eq!(a["defaults"], a["limits"]);
        assert_eq!(a["limits"]["replyMaxTokens"], json!(3000));
        assert_eq!(a["limits"]["memoryTokens"], json!(1000));
        assert_eq!(a["ranges"]["replyMaxTokens"], json!([1, 250000]));
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

    #[test]
    fn generation_and_import_defaults_and_ranges() {
        let l = Limits::default();
        assert_eq!(
            (
                l.gen_per_day,
                l.gen_admins_exempt,
                l.gen_min_source,
                l.gen_min_wrong
            ),
            (3, 1, 30, 10)
        );
        assert_eq!(
            (
                l.gen_bank_single,
                l.gen_bank_multi,
                l.gen_wrong_single,
                l.gen_wrong_multi
            ),
            (14, 6, 14, 6)
        );
        assert_eq!((l.gen_call_tokens, l.gen_running), (20_000, 2));
        assert_eq!((l.gen_max_calls, l.import_max_calls), (15, 25));
        assert_eq!(l.recognize_max_calls, 20);
        assert_eq!((l.gen_unlimited, l.import_unlimited), (1, 1));
        assert_eq!((l.import_max_mb, l.import_max_questions), (20, 50));
        for (k, bad) in [
            ("genPerDay", 101),
            ("genMinSource", 4),
            ("genMinWrong", 0),
            ("genCallTokens", 999),
            ("genRunning", 9),
            ("genMaxCalls", 7),
            ("genMaxCalls", 41),
            ("importMaxCalls", 7),
            ("importMaxCalls", 41),
            ("recognizeMaxCalls", 7),
            ("recognizeMaxCalls", 41),
            ("genUnlimited", 2),
            ("importUnlimited", -1),
            ("importMaxMb", 21),
            ("importMaxMb", 0),
            ("importMaxQuestions", 51),
        ] {
            let want = DEFS
                .iter()
                .find(|d| d.key == k)
                .map(|d| d.key)
                .unwrap_or("");
            assert_eq!(
                l.apply_update(&obj(json!({ k: bad }))),
                Err(LimitError::Range(want))
            );
        }
    }

    #[test]
    fn the_composition_rule_names_the_failing_mode() {
        let l = Limits::default();
        assert_eq!(l.gen_count_violation(), None);
        let bad = l
            .apply_update(&obj(json!({"genBankSingle": 0, "genBankMulti": 0})))
            .expect("in range");
        assert_eq!(bad.gen_count_violation(), Some("genBankMulti"));
        let bad = l
            .apply_update(&obj(json!({"genWrongSingle": 60, "genWrongMulti": 60})))
            .expect("in range");
        assert_eq!(bad.gen_count_violation(), Some("genWrongMulti"));
        let one = l
            .apply_update(&obj(json!({"genBankSingle": 0, "genBankMulti": 5})))
            .expect("in range");
        assert_eq!(one.gen_count_violation(), None);
    }
}
