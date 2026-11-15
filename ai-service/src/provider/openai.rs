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
    /// Inline reasoning span handling for the content stream
    think: Think,
    /// Undecided start of the content, or the tail of an open reasoning span
    pending: String,
    /// The close tag of the open span (set on entering `Inside`)
    close: &'static str,
}

/// Where the content stream is relative to an inline reasoning span at its start.
#[derive(Default, PartialEq, Clone, Copy)]
enum Think {
    /// Nothing decided yet: the text so far could still become `<think>`
    #[default]
    Start,
    /// Inside the span: everything is reasoning (reported as `Reasoning`)
    Inside,
    /// Just after the span: leading whitespace is skipped
    After,
    /// Ordinary answer text
    Text,
}

/// Leading reasoning spans: (open, close). Case-sensitive, lower case only.
const SPANS: [(&str, &str); 3] = [
    ("<think>", "</think>"),
    ("<thinking>", "</thinking>"),
    ("<thought>", "</thought>"),
];

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
            // Reasoning models (DeepSeek, vLLM: `reasoning_content`; OpenRouter, others: `reasoning`): the first
            // non-empty string field wins, so gateways that send both never double the text
            let reasoning = ["reasoning_content", "reasoning"].iter().find_map(|k| {
                delta
                    .and_then(|d| d.get(*k))
                    .and_then(Value::as_str)
                    .filter(|r| !r.is_empty())
            });
            if let Some(r) = reasoning {
                out.push(Item::Reasoning(r.to_string()));
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

    /// Routes one content piece: a `<think>`, `<thinking>` or `<thought>` span at the start of the content (servers
    /// without a reasoning parser put the model's reasoning there) is reported as `Reasoning`, never as answer text;
    /// it often names the answer, so only AI Chat keeps it.
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
                let span = SPANS.iter().find(|(open, _)| head.starts_with(open));
                if let Some((open, close)) = span {
                    let rest = head[open.len()..].to_string();
                    self.close = close;
                    self.pending.clear();
                    out.push(Item::Thinking);
                    self.think = Think::Inside;
                    self.inside(&rest, out);
                } else if head.is_empty()
                    || SPANS
                        .iter()
                        .any(|(open, _)| head.len() < open.len() && open.starts_with(head))
                {
                    // could still become an opening tag
                } else {
                    self.think = Think::Text;
                    let all = std::mem::take(&mut self.pending);
                    self.emit(all, out);
                }
            }
            Think::Inside => self.inside(t, out),
        }
    }

    /// Inside the span: look for its own closing tag; the text before it is reasoning, only a possible partial tag at
    /// the end is held back.
    fn inside(&mut self, t: &str, out: &mut Vec<Item>) {
        self.pending.push_str(t);
        let close = self.close;
        if let Some(i) = self.pending.find(close) {
            if i > 0 {
                out.push(Item::Reasoning(self.pending[..i].to_string()));
            }
            let rest = self.pending[i + close.len()..].trim_start().to_string();
            self.pending.clear();
            self.think = Think::After;
            if !rest.is_empty() {
                self.think = Think::Text;
                self.emit(rest, out);
            }
        } else {
            let mut cut = self.pending.len().saturating_sub(close.len() - 1);
            while !self.pending.is_char_boundary(cut) {
                cut += 1;
            }
            if cut > 0 {
                let head: String = self.pending.drain(..cut).collect();
                out.push(Item::Reasoning(head));
            }
        }
    }

    fn emit(&mut self, t: String, out: &mut Vec<Item>) {
        self.text = true;
        out.push(Item::Text(t));
    }

    /// Before the end: releases an undecided start that never became a span; an unterminated span is reasoning.
    pub fn flush(&mut self, out: &mut Vec<Item>) {
        if self.think == Think::Start && !self.pending.is_empty() {
            self.think = Think::Text;
            let all = std::mem::take(&mut self.pending);
            self.emit(all, out);
        } else if self.think == Think::Inside && !self.pending.is_empty() {
            let rest = std::mem::take(&mut self.pending);
            out.push(Item::Reasoning(rest));
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
                Item::Reasoning("x".into()),
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
    fn reasoning_fields_become_reasoning_items() {
        let r = |s: &str| Item::Reasoning(s.into());
        let out = run(&[
            r#"{"choices":[{"delta":{"role":"assistant","content":"","reasoning_content":"made-up step one"}}]}"#,
            r#"{"choices":[{"delta":{"reasoning":"made-up step two"}}]}"#,
            r#"{"choices":[{"delta":{"reasoning_content":""}}]}"#,
            r#"{"choices":[{"delta":{"reasoning":""}}]}"#,
            r#"{"choices":[{"delta":{"reasoning":null}}]}"#,
            r#"{"choices":[{"delta":{"reasoning_content":5}}]}"#,
            r#"{"choices":[{"delta":{"reasoning":{}}}]}"#,
            r#"{"choices":[{"delta":{"reasoning":[]}}]}"#,
            r#"{"choices":[{"delta":{"reasoning":" "}}]}"#,
            r#"{"choices":[{"delta":{"reasoning_content":"A","reasoning":"B"}}]}"#,
            r#"{"choices":[{"delta":{"reasoning_content":"","reasoning":"C"}}]}"#,
            r#"{"choices":[{"delta":{"reasoning":"r","content":"t"}}]}"#,
            "[DONE]",
        ]);
        assert_eq!(
            out,
            vec![
                r("made-up step one"),
                r("made-up step two"),
                r(" "),
                r("A"),
                r("C"),
                r("r"),
                Item::Text("t".into()),
                Item::Done
            ]
        );
        // reasoning only, never any text: an empty answer
        let out = run(&[r#"{"choices":[{"delta":{"reasoning":"x"}}]}"#, "[DONE]"]);
        assert_eq!(out, vec![r("x"), Item::Done]);
    }

    fn reasonings(out: &[Item]) -> String {
        out.iter()
            .filter_map(|i| {
                if let Item::Reasoning(t) = i {
                    Some(t.as_str())
                } else {
                    None
                }
            })
            .collect()
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
        // reasoning alone does not count as an answer
        let out = run(&[
            r#"{"choices":[{"delta":{"reasoning_content":"r"}}]}"#,
            r#"{"choices":[{"delta":{"refusal":"no"}}]}"#,
            "[DONE]",
        ]);
        assert_eq!(
            out,
            vec![Item::Reasoning("r".into()), Item::Fail(UpCode::Refused)]
        );
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

    fn chunked(full: &str, n: usize) -> Vec<Item> {
        let chars: Vec<char> = full.chars().collect();
        let events: Vec<String> = chars
            .chunks(n)
            .map(|c| delta(&c.iter().collect::<String>()))
            .collect();
        let refs: Vec<&str> = events.iter().map(String::as_str).collect();
        run(&refs)
    }

    #[test]
    fn inline_spans_become_reasoning_for_every_chunking() {
        let inner = "The user wants a hint. \u{e9} \u{601d} The first option is the right one.";
        for (open, close) in SPANS {
            let full = format!("{open}{inner}{close}\n\nThink about how often the data is read.");
            for n in [1usize, 2, 3, 5, 7, 16, 400] {
                let out = chunked(&full, n);
                assert_eq!(
                    texts(&out),
                    "Think about how often the data is read.",
                    "{open} chunk {n}"
                );
                assert_eq!(reasonings(&out), inner, "{open} chunk {n}");
                let facts = out.iter().filter(|i| **i == Item::Thinking).count();
                assert_eq!(facts, 1, "{open} chunk {n}");
                assert_eq!(out.last(), Some(&Item::Done));
            }
        }
    }

    #[test]
    fn unterminated_span_becomes_reasoning() {
        let out = run(&[&delta("<thought>the answer is A"), &delta(" and more")]);
        assert_eq!(texts(&out), "");
        assert_eq!(reasonings(&out), "the answer is A and more");
        assert_eq!(out.last(), Some(&Item::Done));
        let out = run(&[&delta("<think>abc</thi")]);
        assert_eq!(texts(&out), "");
        assert_eq!(reasonings(&out), "abc</thi");
    }

    #[test]
    fn prefixes_and_late_tags_stay_text() {
        let same = |evs: &[&str], want: &str| {
            let d: Vec<String> = evs.iter().map(|e| delta(e)).collect();
            let refs: Vec<&str> = d.iter().map(String::as_str).collect();
            let out = run(&refs);
            assert_eq!(texts(&out), want);
            assert_eq!(reasonings(&out), "", "{want}");
        };
        same(&["<thi", "s is not a tag"], "<this is not a tag");
        same(&["<th"], "<th");
        same(&["<thinker>x"], "<thinker>x");
        same(&["<thought-x>y"], "<thought-x>y");
        same(&["<THINK>x</THINK>"], "<THINK>x</THINK>");
        same(
            &["Hello <thought>x</thought>"],
            "Hello <thought>x</thought>",
        );
        same(&["Hello <think>x</think>"], "Hello <think>x</think>");
        let out = run(&[&delta("  \n<think>r</think>  answer")]);
        assert_eq!(reasonings(&out), "r");
        assert_eq!(texts(&out), "answer");
    }

    #[test]
    fn close_tag_split_and_whitespace_after_it() {
        let out = run(&[
            &delta("<thought>abc</tho"),
            &delta("ught>"),
            &delta("\n\n"),
            &delta("  Ans"),
        ]);
        assert_eq!(reasonings(&out), "abc");
        assert_eq!(texts(&out), "Ans");
        let out = run(&[&delta("<thought>a</think>b</thought>c")]);
        assert_eq!(reasonings(&out), "a</think>b");
        assert_eq!(texts(&out), "c");
    }
}
