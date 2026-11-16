//! OpenAI Responses format: request body and stream handling.
//!
//! The answer text comes only from `response.output_text.delta`: the same text is repeated in `output_text.done`,
//! `content_part.done`, `output_item.done` and `response.completed`, and none of those is emitted again (it would double
//! the answer). Only a stream that completes without a single delta is salvaged from `response.completed`.
use super::errors::{from_error_value, from_type, UpCode};
use super::{Item, TokenLimit, Turn};
use serde_json::{json, Map, Value};

/// `{"model", "instructions"?, "input": [{role, content}…], "stream": true, "store": false, "max_output_tokens"?}`.
/// Nothing else: `instructions` is the system text (left out when empty); the token field is absent for
/// `TokenField::Omit`; the thinking strength and the structured output field are added by `put_effort` / `put_schema`.
pub fn body(model: &str, system: &str, turns: &[Turn], token: TokenLimit) -> Value {
    let input: Vec<Value> = turns
        .iter()
        .map(|t| json!({"role": t.role.as_str(), "content": t.content}))
        .collect();
    let mut m = Map::new();
    m.insert("model".into(), Value::from(model));
    if !system.is_empty() {
        m.insert("instructions".into(), Value::from(system));
    }
    m.insert("input".into(), Value::Array(input));
    m.insert("stream".into(), Value::Bool(true));
    m.insert("store".into(), Value::Bool(false));
    if let Some(key) = token.field.key() {
        m.insert(key.into(), Value::from(token.value));
    }
    Value::Object(m)
}

/// Which reasoning text stream was seen first (a gateway that sends both must not double it).
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReasoningSrc {
    Summary,
    Raw,
}

/// Stream state.
#[derive(Default)]
pub struct State {
    /// A text delta came
    text: bool,
    refused: bool,
    filtered: bool,
    /// `response.completed` or `response.incomplete` arrived
    ended: bool,
    reasoning: Option<ReasoningSrc>,
}

fn int(v: Option<&Value>) -> Option<i64> {
    v.and_then(Value::as_i64)
}

/// Usage of a response: `input_tokens` already includes the cached tokens and `output_tokens` the reasoning tokens, so
/// nothing is added.
fn usage(u: Option<&Value>, out: &mut Vec<Item>) {
    let Some(u) = u.filter(|u| u.is_object()) else {
        return;
    };
    let input = int(u.get("input_tokens").or_else(|| u.get("prompt_tokens")));
    let output = int(u
        .get("output_tokens")
        .or_else(|| u.get("completion_tokens")));
    if input.is_some() || output.is_some() {
        out.push(Item::Usage { input, output });
    }
    let details = u.get("input_tokens_details");
    if let Some(n) = int(details.and_then(|d| d.get("cached_tokens"))) {
        out.push(Item::Cached(n));
    }
    let write = int(details.and_then(|d| d.get("cache_creation_tokens")))
        .or_else(|| int(details.and_then(|d| d.get("cache_write_tokens"))))
        .or_else(|| int(u.get("cache_creation_input_tokens")));
    if let Some(n) = write {
        out.push(Item::CacheWrite(n));
    }
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

impl State {
    /// Handles one event. The kind is the `type` in the data, else the SSE event name (a gateway may send no `event:`
    /// line, which the parser reports as `message`).
    pub fn on_event(&mut self, name: &str, data: &str, out: &mut Vec<Item>) {
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
        let kind = v.get("type").and_then(Value::as_str).unwrap_or(name);
        // an error object in any other event (a gateway's in-band error)
        if kind != "error" && v.get("error").is_some_and(|e| !e.is_null()) {
            out.push(Item::Fail(from_error_value(&v).unwrap_or(UpCode::Server)));
            return;
        }
        match kind {
            "response.output_text.delta" => {
                if let Some(d) = str_of(&v, "delta") {
                    self.text = true;
                    out.push(Item::Text(d.to_string()));
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let src = if kind == "response.reasoning_text.delta" {
                    ReasoningSrc::Raw
                } else {
                    ReasoningSrc::Summary
                };
                if let Some(d) = str_of(&v, "delta") {
                    if self.reasoning.is_none() || self.reasoning == Some(src) {
                        self.reasoning = Some(src);
                        out.push(Item::Reasoning(d.to_string()));
                    }
                }
            }
            "response.output_item.added" => {
                if v.get("item")
                    .and_then(|i| i.get("type"))
                    .and_then(Value::as_str)
                    == Some("reasoning")
                {
                    out.push(Item::Thinking);
                }
            }
            "response.refusal.delta" => {
                if str_of(&v, "delta").is_some() {
                    self.refused = true;
                }
            }
            "response.completed" | "response.incomplete" => {
                let r = v.get("response").unwrap_or(&Value::Null);
                self.terminal(r, out);
            }
            "response.failed" => {
                let r = v.get("response").unwrap_or(&Value::Null);
                usage(r.get("usage"), out);
                out.push(Item::Fail(from_error_value(r).unwrap_or(UpCode::Server)));
            }
            "error" => {
                let code = v
                    .get("code")
                    .and_then(Value::as_str)
                    .and_then(from_type)
                    .or_else(|| from_error_value(&v))
                    .unwrap_or(UpCode::Server);
                out.push(Item::Fail(code));
            }
            // created, in_progress, output_item.done, content_part.*, *.done, annotations, rate limits, …
            _ => {}
        }
    }

    /// `response.completed` / `response.incomplete`: usage, a cut or filtered answer, the salvage, then the end.
    fn terminal(&mut self, r: &Value, out: &mut Vec<Item>) {
        self.ended = true;
        usage(r.get("usage"), out);
        // a gateway may send `response.completed` with an incomplete status, so both events are read the same way
        match r
            .get("incomplete_details")
            .and_then(|d| d.get("reason"))
            .and_then(Value::as_str)
        {
            Some("content_filter") => self.filtered = true,
            Some(_) => out.push(Item::Truncated),
            None => {}
        }
        if !self.text {
            // no delta ever came (a cached replay, a gateway that only sends the final response): take the output
            let parts = r
                .get("output")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|i| i.get("type").and_then(Value::as_str) == Some("message"))
                .filter_map(|i| i.get("content").and_then(Value::as_array))
                .flatten();
            for p in parts {
                match p.get("type").and_then(Value::as_str) {
                    Some("output_text") => {
                        if let Some(t) = str_of(p, "text") {
                            self.text = true;
                            out.push(Item::Text(t.to_string()));
                        }
                    }
                    Some("refusal") => self.refused = true,
                    _ => {}
                }
            }
        }
        out.push(self.end());
    }

    /// The answer after the terminal event: refused (or filtered) without any text fails, else done.
    fn end(&self) -> Item {
        if !self.text && (self.refused || self.filtered) {
            Item::Fail(UpCode::Refused)
        } else {
            Item::Done
        }
    }

    /// The end of the body. A body that ends before `response.completed` / `response.incomplete` was cut short:
    /// `ai.up.protocol` (the strict rule of the Anthropic format).
    pub fn finish(&mut self) -> Item {
        if self.ended {
            self.end()
        } else {
            Item::Fail(UpCode::Protocol)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Role, TokenField};
    use super::*;

    fn run(events: &[(&str, &str)]) -> Vec<Item> {
        let mut s = State::default();
        let mut out = Vec::new();
        for (n, d) in events {
            s.on_event(n, d, &mut out);
            if matches!(out.last(), Some(Item::Done | Item::Fail(_))) {
                return out;
            }
        }
        out.push(s.finish());
        out
    }

    fn delta(t: &str) -> String {
        json!({"type": "response.output_text.delta", "delta": t}).to_string()
    }

    fn turn(role: Role, c: &str) -> Turn {
        Turn {
            role,
            content: c.into(),
        }
    }

    fn limit(field: TokenField) -> TokenLimit {
        TokenLimit { field, value: 1500 }
    }

    #[test]
    fn body_keys() {
        let t = [
            turn(Role::User, "hi"),
            turn(Role::Assistant, "yo"),
            turn(Role::User, "again"),
        ];
        let b = body("m", "sys", &t, limit(TokenField::MaxOutputTokens));
        assert_eq!(b.as_object().expect("object").len(), 6);
        assert_eq!(
            serde_json::to_string(&b).expect("json"),
            r#"{"model":"m","instructions":"sys","input":[{"role":"user","content":"hi"},{"role":"assistant","content":"yo"},{"role":"user","content":"again"}],"stream":true,"store":false,"max_output_tokens":1500}"#
        );
        let b = body("m", "sys", &t, TokenLimit::omit());
        assert_eq!(b.as_object().expect("object").len(), 5);
        assert!(b.get("max_output_tokens").is_none());
        for k in ["max_tokens", "max_completion_tokens", "messages", "system"] {
            assert!(b.get(k).is_none(), "{k}");
        }
        // no system text: no instructions
        let b = body("m", "", &t, TokenLimit::omit());
        assert!(b.get("instructions").is_none());
        assert_eq!(b.as_object().expect("object").len(), 4);
        assert_eq!(b["input"][0]["content"], json!("hi"));
    }

    #[test]
    fn text_comes_only_from_deltas() {
        let (d1, d2) = (delta("Hel"), delta("lo"));
        let out = run(&[
            (
                "response.created",
                r#"{"type":"response.created","response":{}}"#,
            ),
            (
                "response.output_item.added",
                r#"{"type":"response.output_item.added","item":{"type":"message"}}"#,
            ),
            ("response.output_text.delta", &d1),
            ("response.output_text.delta", &d2),
            (
                "response.output_text.done",
                r#"{"type":"response.output_text.done","text":"Hello"}"#,
            ),
            (
                "response.content_part.done",
                r#"{"type":"response.content_part.done","part":{"type":"output_text","text":"Hello"}}"#,
            ),
            (
                "response.output_item.done",
                r#"{"type":"response.output_item.done","item":{"type":"message","content":[{"type":"output_text","text":"Hello"}]}}"#,
            ),
            (
                "response.completed",
                r#"{"type":"response.completed","response":{"output":[{"type":"message","content":[{"type":"output_text","text":"Hello"}]}],"usage":{"input_tokens":30,"output_tokens":4,"input_tokens_details":{"cached_tokens":12},"output_tokens_details":{"reasoning_tokens":1}}}}"#,
            ),
        ]);
        assert_eq!(
            out,
            vec![
                Item::Text("Hel".into()),
                Item::Text("lo".into()),
                Item::Usage {
                    input: Some(30),
                    output: Some(4)
                },
                Item::Cached(12),
                Item::Done
            ]
        );
    }

    #[test]
    fn events_without_an_event_name() {
        let d = delta("x");
        let out = run(&[
            ("message", &d),
            (
                "message",
                r#"{"type":"response.completed","response":{"usage":{"input_tokens":1,"output_tokens":2}}}"#,
            ),
        ]);
        assert_eq!(
            out,
            vec![
                Item::Text("x".into()),
                Item::Usage {
                    input: Some(1),
                    output: Some(2)
                },
                Item::Done
            ]
        );
        // the name alone is enough when the data has no type
        let out = run(&[
            ("response.output_text.delta", r#"{"delta":"y"}"#),
            ("response.completed", r#"{"response":{}}"#),
        ]);
        assert_eq!(out, vec![Item::Text("y".into()), Item::Done]);
    }

    #[test]
    fn salvage_from_completed_when_no_delta_came() {
        let out = run(&[(
            "response.completed",
            r#"{"type":"response.completed","response":{"output":[{"type":"reasoning","id":"r1"},{"type":"message","content":[{"type":"output_text","text":"A"},{"type":"output_text","text":"B"}]}],"usage":{"input_tokens":5,"output_tokens":2}}}"#,
        )]);
        assert_eq!(
            out,
            vec![
                Item::Usage {
                    input: Some(5),
                    output: Some(2)
                },
                Item::Text("A".into()),
                Item::Text("B".into()),
                Item::Done
            ]
        );
        // a refusal part without text
        let out = run(&[(
            "response.completed",
            r#"{"type":"response.completed","response":{"output":[{"type":"message","content":[{"type":"refusal","refusal":"no"}]}]}}"#,
        )]);
        assert_eq!(out, vec![Item::Fail(UpCode::Refused)]);
    }

    #[test]
    fn cache_write_in_each_spelling() {
        for u in [
            r#"{"input_tokens":20,"output_tokens":2,"input_tokens_details":{"cache_creation_tokens":9}}"#,
            r#"{"input_tokens":20,"output_tokens":2,"input_tokens_details":{"cache_write_tokens":9}}"#,
            r#"{"input_tokens":20,"output_tokens":2,"cache_creation_input_tokens":9}"#,
        ] {
            let ev = format!(r#"{{"type":"response.completed","response":{{"usage":{u}}}}}"#);
            let out = run(&[("response.completed", ev.as_str())]);
            assert!(out.contains(&Item::CacheWrite(9)), "{u}");
        }
        let out = run(&[(
            "response.completed",
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":20,"output_tokens":2}}}"#,
        )]);
        assert!(!out
            .iter()
            .any(|i| matches!(i, Item::CacheWrite(_) | Item::Cached(_))));
    }

    #[test]
    fn incomplete_max_output_tokens_is_truncated() {
        let d = delta("part");
        let out = run(&[
            ("response.output_text.delta", &d),
            (
                "response.incomplete",
                r#"{"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":9,"output_tokens":16}}}"#,
            ),
        ]);
        assert_eq!(
            out,
            vec![
                Item::Text("part".into()),
                Item::Usage {
                    input: Some(9),
                    output: Some(16)
                },
                Item::Truncated,
                Item::Done
            ]
        );
        // an unknown reason is a cut too; a completed event with an incomplete reason as well
        let out = run(&[(
            "response.completed",
            r#"{"type":"response.completed","response":{"incomplete_details":{"reason":"other"}}}"#,
        )]);
        assert_eq!(out, vec![Item::Truncated, Item::Done]);
    }

    #[test]
    fn content_filter_and_refusal() {
        let filter = r#"{"type":"response.incomplete","response":{"incomplete_details":{"reason":"content_filter"}}}"#;
        let out = run(&[("response.incomplete", filter)]);
        assert_eq!(out, vec![Item::Fail(UpCode::Refused)]);
        let d = delta("some");
        let out = run(&[
            ("response.output_text.delta", &d),
            ("response.incomplete", filter),
        ]);
        assert_eq!(out, vec![Item::Text("some".into()), Item::Done]);
        // a refusal stream with no text
        let out = run(&[
            (
                "response.refusal.delta",
                r#"{"type":"response.refusal.delta","delta":"I cannot"}"#,
            ),
            (
                "response.completed",
                r#"{"type":"response.completed","response":{}}"#,
            ),
        ]);
        assert_eq!(out, vec![Item::Fail(UpCode::Refused)]);
    }

    #[test]
    fn failed_and_error_events() {
        let out = run(&[(
            "response.failed",
            r#"{"type":"response.failed","response":{"error":{"code":"server_error","message":"m"},"usage":{"input_tokens":3,"output_tokens":0}}}"#,
        )]);
        assert_eq!(
            out,
            vec![
                Item::Usage {
                    input: Some(3),
                    output: Some(0)
                },
                Item::Fail(UpCode::Server)
            ]
        );
        let out = run(&[(
            "response.failed",
            r#"{"type":"response.failed","response":{"error":{"code":"rate_limit_exceeded"}}}"#,
        )]);
        assert_eq!(out, vec![Item::Fail(UpCode::RateLimit)]);
        let out = run(&[(
            "response.failed",
            r#"{"type":"response.failed","response":{"status":"failed"}}"#,
        )]);
        assert_eq!(out, vec![Item::Fail(UpCode::Server)]);
        // the error event: code at the top level, or nested
        let out = run(&[(
            "error",
            r#"{"type":"error","code":"rate_limit_exceeded","message":"m"}"#,
        )]);
        assert_eq!(out, vec![Item::Fail(UpCode::RateLimit)]);
        let out = run(&[(
            "error",
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"m"}}"#,
        )]);
        assert_eq!(out, vec![Item::Fail(UpCode::BadRequest)]);
        let out = run(&[("error", r#"{"type":"error","message":"m"}"#)]);
        assert_eq!(out, vec![Item::Fail(UpCode::Server)]);
        // an in-band error object in another event
        let d = delta("a");
        let out = run(&[
            ("response.output_text.delta", &d),
            ("message", r#"{"error":{"code":"insufficient_quota"}}"#),
        ]);
        assert_eq!(out, vec![Item::Text("a".into()), Item::Fail(UpCode::Quota)]);
    }

    #[test]
    fn a_missing_terminal_event_is_a_protocol_failure() {
        let d = delta("a");
        let out = run(&[("response.output_text.delta", &d)]);
        assert_eq!(
            out,
            vec![Item::Text("a".into()), Item::Fail(UpCode::Protocol)]
        );
        let out = run(&[("response.output_text.delta", &d), ("message", "[DONE]")]);
        assert_eq!(
            out,
            vec![Item::Text("a".into()), Item::Fail(UpCode::Protocol)]
        );
        assert_eq!(run(&[]), vec![Item::Fail(UpCode::Protocol)]);
        assert_eq!(
            run(&[("message", "not json")]),
            vec![Item::Fail(UpCode::Protocol)]
        );
        assert_eq!(
            run(&[("message", "[1,2]")]),
            vec![Item::Fail(UpCode::Protocol)]
        );
        // [DONE] after the terminal event is harmless (the loop has stopped at Done already; finish agrees)
        let mut s = State::default();
        let mut out = Vec::new();
        s.on_event(
            "response.completed",
            r#"{"type":"response.completed","response":{}}"#,
            &mut out,
        );
        assert_eq!(s.finish(), Item::Done);
    }

    #[test]
    fn reasoning_text_and_items() {
        let r = |s: &str| Item::Reasoning(s.into());
        let out = run(&[
            (
                "response.output_item.added",
                r#"{"type":"response.output_item.added","item":{"type":"reasoning","id":"rs_1","encrypted_content":"SECRET-BLOB"}}"#,
            ),
            (
                "response.reasoning_summary_text.delta",
                r#"{"type":"response.reasoning_summary_text.delta","delta":"step one"}"#,
            ),
            (
                "response.reasoning_summary_text.done",
                r#"{"type":"response.reasoning_summary_text.done","text":"step one"}"#,
            ),
            (
                "response.reasoning_text.delta",
                r#"{"type":"response.reasoning_text.delta","delta":"raw"}"#,
            ),
            (
                "response.reasoning_summary_text.delta",
                r#"{"type":"response.reasoning_summary_text.delta","delta":""}"#,
            ),
            (
                "response.reasoning_summary_text.delta",
                r#"{"type":"response.reasoning_summary_text.delta","delta":" two"}"#,
            ),
            (
                "response.output_item.added",
                r#"{"type":"response.output_item.added","item":{"type":"message"}}"#,
            ),
            (
                "response.completed",
                r#"{"type":"response.completed","response":{}}"#,
            ),
        ]);
        assert_eq!(
            out,
            vec![Item::Thinking, r("step one"), r(" two"), Item::Done]
        );
        assert!(!format!("{out:?}").contains("SECRET-BLOB"));
        // the raw kind alone works too
        let out = run(&[
            (
                "response.reasoning_text.delta",
                r#"{"type":"response.reasoning_text.delta","delta":"raw"}"#,
            ),
            (
                "response.completed",
                r#"{"type":"response.completed","response":{}}"#,
            ),
        ]);
        assert_eq!(out, vec![r("raw"), Item::Done]);
    }

    #[test]
    fn unknown_events_are_ignored() {
        let d = delta("ok");
        let out = run(&[
            ("response.created", r#"{"type":"response.created"}"#),
            ("response.in_progress", r#"{"type":"response.in_progress"}"#),
            (
                "response.output_text.annotation.added",
                r#"{"type":"response.output_text.annotation.added","annotation":{"url":"https://example.com"}}"#,
            ),
            (
                "rate_limits.updated",
                r#"{"type":"rate_limits.updated","rate_limits":[]}"#,
            ),
            ("something.new", r#"{"type":"something.new","delta":"no"}"#),
            ("response.output_text.delta", &d),
            (
                "response.output_text.delta",
                r#"{"type":"response.output_text.delta","delta":""}"#,
            ),
            (
                "response.completed",
                r#"{"type":"response.completed","response":{}}"#,
            ),
        ]);
        assert_eq!(out, vec![Item::Text("ok".into()), Item::Done]);
    }
}
