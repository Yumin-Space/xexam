//! OpenAI-compatible format: request body and stream handling (4.4, 4.6).
use super::errors::{from_error_value, UpCode};
use super::{Item, Turn};
use serde_json::{json, Map, Value};

/// `{"model", "messages": [system, …], "stream": true, "stream_options"?}`. Nothing else (maximum compatibility).
pub fn body(model: &str, system: &str, turns: &[Turn], usage_option: bool) -> Value {
    let mut messages = vec![json!({"role": "system", "content": system})];
    for t in turns {
        messages.push(json!({"role": t.role.as_str(), "content": t.content}));
    }
    let mut m = Map::new();
    m.insert("model".into(), Value::from(model));
    m.insert("messages".into(), Value::Array(messages));
    m.insert("stream".into(), Value::Bool(true));
    if usage_option {
        m.insert("stream_options".into(), json!({"include_usage": true}));
    }
    Value::Object(m)
}

/// Stream state.
#[derive(Default)]
pub struct State {
    chunks: usize,
    text: bool,
    refusal: bool,
    filtered: bool,
}

impl State {
    /// Handles the `data` of one event.
    pub fn on_data(&mut self, data: &str, out: &mut Vec<Item>) {
        if data.trim() == "[DONE]" {
            out.push(self.finish());
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            out.push(Item::Fail(UpCode::Protocol));
            return;
        };
        if !v.is_object() {
            out.push(Item::Fail(UpCode::Protocol));
            return;
        }
        if v.get("error").is_some_and(|e| !e.is_null()) {
            out.push(Item::Fail(from_error_value(&v).unwrap_or(UpCode::Server)));
            return;
        }
        self.chunks += 1;
        if let Some(choice) = v.get("choices").and_then(|c| c.get(0)) {
            let delta = choice.get("delta");
            // Reasoning models (DeepSeek, vLLM: `reasoning_content`; OpenRouter, others: `reasoning`): only the fact is
            // kept, the text is dropped here
            let reasoning = ["reasoning_content", "reasoning"].iter().any(|k| {
                delta
                    .and_then(|d| d.get(*k))
                    .and_then(Value::as_str)
                    .is_some_and(|r| !r.is_empty())
            });
            if reasoning {
                out.push(Item::Thinking);
            }
            if let Some(t) = delta.and_then(|d| d.get("content")).and_then(Value::as_str) {
                if !t.is_empty() {
                    self.text = true;
                    out.push(Item::Text(t.to_string()));
                }
            }
            if delta
                .and_then(|d| d.get("refusal"))
                .and_then(Value::as_str)
                .is_some_and(|r| !r.is_empty())
            {
                self.refusal = true;
            }
            match choice.get("finish_reason").and_then(Value::as_str) {
                Some("content_filter") => self.filtered = true,
                Some("length") => out.push(Item::Truncated),
                _ => {}
            }
        }
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            let input = u.get("prompt_tokens").and_then(Value::as_i64);
            let output = u.get("completion_tokens").and_then(Value::as_i64);
            if input.is_some() || output.is_some() {
                out.push(Item::Usage { input, output });
            }
            if let Some(n) = u
                .get("prompt_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
                .and_then(Value::as_i64)
            {
                out.push(Item::Cached(n));
            }
        }
    }

    /// The end: `[DONE]` or the end of the body.
    pub fn finish(&mut self) -> Item {
        if self.chunks == 0 {
            Item::Fail(UpCode::Protocol)
        } else if !self.text && (self.refusal || self.filtered) {
            Item::Fail(UpCode::Refused)
        } else {
            Item::Done
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Role;
    use super::*;

    fn run(events: &[&str]) -> Vec<Item> {
        let mut s = State::default();
        let mut out = Vec::new();
        for e in events {
            s.on_data(e, &mut out);
            if matches!(out.last(), Some(Item::Done | Item::Fail(_))) {
                return out;
            }
        }
        out.push(s.finish());
        out
    }

    #[test]
    fn body_is_minimal() {
        let t = [Turn {
            role: Role::User,
            content: "hi".into(),
        }];
        let b = body("m", "sys", &t, true);
        assert_eq!(
            serde_json::to_string(&b).expect("json"),
            r#"{"model":"m","messages":[{"role":"system","content":"sys"},{"role":"user","content":"hi"}],"stream":true,"stream_options":{"include_usage":true}}"#
        );
        assert!(body("m", "sys", &t, false).get("stream_options").is_none());
    }

    #[test]
    fn text_usage_and_end() {
        let out = run(&[
            r#"{"choices":[{"delta":{"role":"assistant","content":""}}]}"#,
            r#"{"choices":[{"delta":{"content":"Hel"}}]}"#,
            r#"{"choices":[{"delta":{"content":"lo","reasoning_content":"x"}}]}"#,
            r#"{"choices":[{"delta":{"content":5}}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":7,"completion_tokens":2}}"#,
            "[DONE]",
        ]);
        assert_eq!(
            out,
            vec![
                Item::Text("Hel".into()),
                Item::Thinking,
                Item::Text("lo".into()),
                Item::Usage {
                    input: Some(7),
                    output: Some(2)
                },
                Item::Done
            ]
        );
    }

    #[test]
    fn reasoning_is_reported_without_its_text() {
        let out = run(&[
            r#"{"choices":[{"delta":{"role":"assistant","content":"","reasoning_content":"made-up step one"}}]}"#,
            r#"{"choices":[{"delta":{"reasoning":"made-up step two"}}]}"#,
            r#"{"choices":[{"delta":{"reasoning_content":""}}]}"#,
            r#"{"choices":[{"delta":{"reasoning":null,"content":"Answer"}}]}"#,
            r#"{"choices":[{"delta":{"reasoning_content":5}}]}"#,
            "[DONE]",
        ]);
        assert_eq!(
            out,
            vec![
                Item::Thinking,
                Item::Thinking,
                Item::Text("Answer".into()),
                Item::Done
            ]
        );
        assert!(!format!("{out:?}").contains("made-up"));
        // reasoning only, never any text: an empty answer, as before
        let out = run(&[r#"{"choices":[{"delta":{"reasoning":"x"}}]}"#, "[DONE]"]);
        assert_eq!(out, vec![Item::Thinking, Item::Done]);
    }

    #[test]
    fn length_refusal_filter_and_in_band_errors() {
        let out = run(&[r#"{"choices":[{"delta":{"content":"a"},"finish_reason":"length"}]}"#]);
        assert_eq!(
            out,
            vec![Item::Text("a".into()), Item::Truncated, Item::Done]
        );
        let out = run(&[r#"{"choices":[{"delta":{"refusal":"no"}}]}"#, "[DONE]"]);
        assert_eq!(out, vec![Item::Fail(UpCode::Refused)]);
        let out = run(&[r#"{"choices":[{"delta":{},"finish_reason":"content_filter"}]}"#]);
        assert_eq!(out, vec![Item::Fail(UpCode::Refused)]);
        let out = run(&[r#"{"error":{"message":"m","code":"rate_limit_error"}}"#]);
        assert_eq!(out, vec![Item::Fail(UpCode::RateLimit)]);
        let out = run(&[r#"{"error":{"message":"m"}}"#]);
        assert_eq!(out, vec![Item::Fail(UpCode::Server)]);
        // OpenRouter's shape: the code is an HTTP status as a number
        let out = run(&[
            r#"{"choices":[{"delta":{"content":"a"}}]}"#,
            r#"{"error":{"code":402,"message":"m"}}"#,
        ]);
        assert_eq!(out, vec![Item::Text("a".into()), Item::Fail(UpCode::Quota)]);
        assert_eq!(run(&["not json"]), vec![Item::Fail(UpCode::Protocol)]);
        assert_eq!(run(&[]), vec![Item::Fail(UpCode::Protocol)]);
        assert_eq!(run(&["[DONE]"]), vec![Item::Fail(UpCode::Protocol)]);
    }
}
