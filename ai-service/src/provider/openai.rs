//! OpenAI-compatible format: request body and stream handling (4.4, 4.6).
use super::errors::{from_error_value, UpCode};
use super::{Item, TokenLimit, Turn};
use serde_json::{json, Map, Value};

/// `{"model", "messages": [system, …], "stream": true, "stream_options"?, "max_tokens"? | "max_completion_tokens"?}`.
/// Nothing else (maximum compatibility); the token field is absent for `TokenField::Omit`.
pub fn body(
    model: &str,
    system: &str,
    turns: &[Turn],
    usage_option: bool,
    token: TokenLimit,
) -> Value {
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
    if let Some(key) = token.field.key() {
        m.insert(key.into(), Value::from(token.value));
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
    /// Inline `<think>` span handling for the content stream
    think: Think,
    /// Undecided start of the content, or the tail of an open `<think>` span
    pending: String,
}

/// Where the content stream is relative to an inline `<think>...</think>` span at its start.
#[derive(Default, PartialEq, Clone, Copy)]
enum Think {
    /// Nothing decided yet: the text so far could still become `<think>`
    #[default]
    Start,
    /// Inside the span: everything is dropped (it is reasoning, reported as `Thinking`)
    Inside,
    /// Just after the span: leading whitespace is skipped
    After,
    /// Ordinary answer text
    Text,
}

const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

impl State {
    /// Handles the `data` of one event.
    pub fn on_data(&mut self, data: &str, out: &mut Vec<Item>) {
        if data.trim() == "[DONE]" {
            self.flush(out);
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
                    self.content(t, out);
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
            // Gateways differ: a details field (two spellings) or a top-level Anthropic-style figure
            let details = u.get("prompt_tokens_details");
            let write = details
                .and_then(|d| d.get("cache_creation_tokens"))
                .or_else(|| details.and_then(|d| d.get("cache_write_tokens")))
                .or_else(|| u.get("cache_creation_input_tokens"))
                .and_then(Value::as_i64);
            if let Some(n) = write {
                out.push(Item::CacheWrite(n));
            }
        }
    }

    /// Routes one content piece: a `<think>...</think>` span at the start of the content (servers without a
    /// reasoning parser put the model's reasoning there) is dropped and reported as `Thinking`; it often names the
    /// answer, so it must never reach the guard, the popover or the stored chat.
    fn content(&mut self, t: &str, out: &mut Vec<Item>) {
        match self.think {
            Think::Text => self.emit(t.to_string(), out),
            Think::After => {
                let t = t.trim_start();
                if !t.is_empty() {
                    self.think = Think::Text;
                    self.emit(t.to_string(), out);
                }
            }
            Think::Start => {
                self.pending.push_str(t);
                let head = self.pending.trim_start();
                if let Some(after) = head.strip_prefix(THINK_OPEN) {
                    let rest = after.to_string();
                    self.pending.clear();
                    self.think = Think::Inside;
                    self.inside(&rest, out);
                } else if head.is_empty()
                    || (head.len() < THINK_OPEN.len() && THINK_OPEN.starts_with(head))
                {
                    // could still become the opening tag
                } else {
                    self.think = Think::Text;
                    let all = std::mem::take(&mut self.pending);
                    self.emit(all, out);
                }
            }
            Think::Inside => self.inside(t, out),
        }
    }

    /// Inside the span: look for the closing tag; keep only a possible partial tag at the end.
    fn inside(&mut self, t: &str, out: &mut Vec<Item>) {
        out.push(Item::Thinking);
        self.pending.push_str(t);
        if let Some(i) = self.pending.find(THINK_CLOSE) {
            let rest = self.pending[i + THINK_CLOSE.len()..]
                .trim_start()
                .to_string();
            self.pending.clear();
            self.think = Think::After;
            if !rest.is_empty() {
                self.think = Think::Text;
                self.emit(rest, out);
            }
        } else {
            let mut cut = self.pending.len().saturating_sub(THINK_CLOSE.len() - 1);
            while !self.pending.is_char_boundary(cut) {
                cut += 1;
            }
            self.pending.drain(..cut);
        }
    }

    fn emit(&mut self, t: String, out: &mut Vec<Item>) {
        self.text = true;
        out.push(Item::Text(t));
    }

    /// Before the end: releases an undecided start that never became `<think>`; an unterminated span is dropped.
    pub fn flush(&mut self, out: &mut Vec<Item>) {
        if self.think == Think::Start && !self.pending.is_empty() {
            self.think = Think::Text;
            let all = std::mem::take(&mut self.pending);
            self.emit(all, out);
        } else if self.think == Think::Inside {
            self.pending.clear();
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
    use crate::provider::TokenField;

    fn run(events: &[&str]) -> Vec<Item> {
        let mut s = State::default();
        let mut out = Vec::new();
        for e in events {
            s.on_data(e, &mut out);
            if matches!(out.last(), Some(Item::Done | Item::Fail(_))) {
                return out;
            }
        }
        s.flush(&mut out);
        out.push(s.finish());
        out
    }

    #[test]
    fn cache_write_tokens_in_each_gateway_spelling() {
        for usage in [
            r#"{"prompt_tokens":20,"completion_tokens":2,"prompt_tokens_details":{"cache_creation_tokens":9}}"#,
            r#"{"prompt_tokens":20,"completion_tokens":2,"prompt_tokens_details":{"cache_write_tokens":9}}"#,
            r#"{"prompt_tokens":20,"completion_tokens":2,"cache_creation_input_tokens":9}"#,
        ] {
            let ev = format!(r#"{{"choices":[],"usage":{usage}}}"#);
            let out = run(&[ev.as_str()]);
            assert!(out.contains(&Item::CacheWrite(9)), "{usage}");
        }
        let out = run(&[r#"{"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":2}}"#]);
        assert!(!out.iter().any(|i| matches!(i, Item::CacheWrite(_))));
    }

    #[test]
    fn body_is_minimal() {
        let t = [Turn {
            role: Role::User,
            content: "hi".into(),
        }];
        let b = body("m", "sys", &t, true, TokenLimit::omit());
        assert_eq!(
            serde_json::to_string(&b).expect("json"),
            r#"{"model":"m","messages":[{"role":"system","content":"sys"},{"role":"user","content":"hi"}],"stream":true,"stream_options":{"include_usage":true}}"#
        );
        assert!(body("m", "sys", &t, false, TokenLimit::omit())
            .get("stream_options")
            .is_none());
        let with = |field| body("m", "sys", &t, true, TokenLimit { field, value: 1500 });
        assert_eq!(
            serde_json::to_string(&with(TokenField::MaxTokens)).expect("json"),
            r#"{"model":"m","messages":[{"role":"system","content":"sys"},{"role":"user","content":"hi"}],"stream":true,"stream_options":{"include_usage":true},"max_tokens":1500}"#
        );
        let c = with(TokenField::MaxCompletionTokens);
        assert_eq!(c["max_completion_tokens"], json!(1500));
        assert!(c.get("max_tokens").is_none());
        // everything but the token field is identical
        let mut plain = body("m", "sys", &t, true, TokenLimit::omit());
        plain["max_completion_tokens"] = json!(1500);
        assert_eq!(plain, c);
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

    fn delta(c: &str) -> String {
        json!({"choices":[{"delta":{"content":c}}]}).to_string()
    }

    fn texts(out: &[Item]) -> String {
        out.iter()
            .filter_map(|i| {
                if let Item::Text(t) = i {
                    Some(t.as_str())
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn inline_think_span_is_dropped_for_every_chunking() {
        let full = "<think>The user wants a hint. The first option is the right one.</think>

Think about how often the data is read.";
        for n in [1usize, 2, 3, 5, 7, 16, 400] {
            let chars: Vec<char> = full.chars().collect();
            let events: Vec<String> = chars
                .chunks(n)
                .map(|c| delta(&c.iter().collect::<String>()))
                .collect();
            let refs: Vec<&str> = events.iter().map(String::as_str).collect();
            let out = run(&refs);
            assert_eq!(
                texts(&out),
                "Think about how often the data is read.",
                "chunk {n}"
            );
            assert!(out.contains(&Item::Thinking));
            assert_eq!(out.last(), Some(&Item::Done));
        }
    }

    #[test]
    fn unterminated_think_drops_everything_and_plain_text_passes() {
        let out = run(&[&delta("<think>the answer is A"), &delta(" and more")]);
        assert_eq!(texts(&out), "");
        // a start that only looks like the tag is released unchanged
        let out = run(&[&delta("<thi"), &delta("s is not a tag")]);
        assert_eq!(texts(&out), "<this is not a tag");
        let out = run(&[&delta("<th")]);
        assert_eq!(texts(&out), "<th");
        // a <think> later in the text is not a leading span and stays text
        let out = run(&[&delta("Hello <think>x</think>")]);
        assert_eq!(texts(&out), "Hello <think>x</think>");
    }
}
