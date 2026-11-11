//! Anthropic Messages format: request body and stream handling (4.4, 4.6).
use super::errors::{from_type, UpCode};
use super::{Item, Turn};
use serde_json::{json, Value};

/// `{"model", "max_tokens", "system", "messages", "stream": true}`. Consecutive messages of the same role are merged
/// with a blank line (the API requires alternation); no sampling parameters, no `thinking`. Prompt caching: `system` is
/// one text block with a `cache_control` breakpoint, and so is the last message before the final one (the history up to
/// the new question): two breakpoints, the incremental cache of the conversation.
pub fn body(model: &str, system: &str, turns: &[Turn], max_tokens: i64) -> Value {
    let mut merged: Vec<(&str, String)> = Vec::new();
    for t in turns {
        match merged.last_mut() {
            Some((role, text)) if *role == t.role.as_str() => {
                text.push_str("\n\n");
                text.push_str(&t.content);
            }
            _ => merged.push((t.role.as_str(), t.content.clone())),
        }
    }
    let last_history = merged.len().checked_sub(2);
    let messages: Vec<Value> = merged
        .into_iter()
        .enumerate()
        .map(|(i, (role, content))| {
            if Some(i) == last_history {
                json!({"role": role, "content": [{"type": "text", "text": content, "cache_control": {"type": "ephemeral"}}]})
            } else {
                json!({"role": role, "content": content})
            }
        })
        .collect();
    json!({
        "model": model,
        "max_tokens": max_tokens,
        "system": [{"type": "text", "text": system, "cache_control": {"type": "ephemeral"}}],
        "messages": messages,
        "stream": true,
    })
}

#[derive(Default)]
pub struct State {
    text: bool,
    refused: bool,
    /// `message_stop` arrived: the only normal end of an Anthropic stream (4.6)
    stopped: bool,
}

fn int(v: Option<&Value>) -> Option<i64> {
    v.and_then(Value::as_i64)
}

impl State {
    /// Handles one named event (the name falls back to the `type` in the data).
    pub fn on_event(&mut self, name: &str, data: &str, out: &mut Vec<Item>) {
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            out.push(Item::Fail(UpCode::Protocol));
            return;
        };
        let name = if name == "message" {
            v.get("type").and_then(Value::as_str).unwrap_or(name)
        } else {
            name
        };
        match name {
            "message_start" => {
                let u = v.get("message").and_then(|m| m.get("usage"));
                let base = int(u.and_then(|u| u.get("input_tokens")));
                let c1 = int(u.and_then(|u| u.get("cache_creation_input_tokens")));
                let c2 = int(u.and_then(|u| u.get("cache_read_input_tokens")));
                if let Some(b) = base {
                    out.push(Item::Usage {
                        input: Some(b + c1.unwrap_or(0) + c2.unwrap_or(0)),
                        output: None,
                    });
                }
                if let Some(n) = c2 {
                    out.push(Item::Cached(n));
                }
            }
            // Extended thinking (a model or gateway may enable it on its own): only the fact is kept, the thinking
            // text, its signature and redacted blocks are dropped here
            "content_block_start" => {
                let t = v
                    .get("content_block")
                    .and_then(|b| b.get("type"))
                    .and_then(Value::as_str);
                if matches!(t, Some("thinking" | "redacted_thinking")) {
                    out.push(Item::Thinking);
                }
            }
            "content_block_delta" => {
                let d = v.get("delta");
                match d.and_then(|d| d.get("type")).and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(t) = d.and_then(|d| d.get("text")).and_then(Value::as_str) {
                            if !t.is_empty() {
                                self.text = true;
                                out.push(Item::Text(t.to_string()));
                            }
                        }
                    }
                    Some("thinking_delta") => out.push(Item::Thinking),
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(n) = int(v
                    .get("usage")
                    .and_then(|u| u.get("cache_read_input_tokens")))
                {
                    out.push(Item::Cached(n));
                }
                if let Some(o) = int(v.get("usage").and_then(|u| u.get("output_tokens"))) {
                    out.push(Item::Usage {
                        input: None,
                        output: Some(o),
                    });
                }
                match v
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    Some("refusal") => self.refused = true,
                    Some("max_tokens") => out.push(Item::Truncated),
                    _ => {}
                }
            }
            "message_stop" => {
                self.stopped = true;
                out.push(self.end());
            }
            "error" => {
                let t = v
                    .get("error")
                    .and_then(|e| e.get("type"))
                    .and_then(Value::as_str)
                    .and_then(from_type)
                    .unwrap_or(UpCode::Server);
                out.push(Item::Fail(t));
            }
            _ => {}
        }
    }

    /// The answer after `message_stop`.
    fn end(&self) -> Item {
        if self.refused && !self.text {
            Item::Fail(UpCode::Refused)
        } else {
            Item::Done
        }
    }

    /// The end of the body. Unlike the OpenAI format, a body that ends before `message_stop` was cut short:
    /// `ai.up.protocol` (the text so far is kept, the answer is saved as an error and can be retried).
    pub fn finish(&mut self) -> Item {
        if self.stopped {
            self.end()
        } else {
            Item::Fail(UpCode::Protocol)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Role;
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

    #[test]
    fn body_merges_roles() {
        let t = [
            Turn {
                role: Role::User,
                content: "a".into(),
            },
            Turn {
                role: Role::User,
                content: "b".into(),
            },
            Turn {
                role: Role::Assistant,
                content: "c".into(),
            },
            Turn {
                role: Role::User,
                content: "d".into(),
            },
        ];
        let b = body("m", "sys", &t, 8192);
        assert_eq!(
            serde_json::to_string(&b).expect("json"),
            r#"{"model":"m","max_tokens":8192,"system":[{"type":"text","text":"sys","cache_control":{"type":"ephemeral"}}],"messages":[{"role":"user","content":"a\n\nb"},{"role":"assistant","content":[{"type":"text","text":"c","cache_control":{"type":"ephemeral"}}]},{"role":"user","content":"d"}],"stream":true}"#
        );
    }

    #[test]
    fn events() {
        let out = run(&[
            (
                "message_start",
                r#"{"type":"message_start","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":2}}}"#,
            ),
            ("ping", r#"{"type":"ping"}"#),
            ("content_block_start", r#"{"type":"content_block_start"}"#),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hi"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"x"}}"#,
            ),
            (
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":3}}"#,
            ),
            ("message_stop", r#"{"type":"message_stop"}"#),
        ]);
        assert_eq!(
            out,
            vec![
                Item::Usage {
                    input: Some(12),
                    output: None
                },
                Item::Cached(2),
                Item::Text("Hi".into()),
                Item::Thinking,
                Item::Usage {
                    input: None,
                    output: Some(3)
                },
                Item::Truncated,
                Item::Done
            ]
        );
        let out = run(&[
            ("message_start", r#"{"message":{}}"#),
            ("message_delta", r#"{"delta":{"stop_reason":"refusal"}}"#),
            ("message_stop", "{}"),
        ]);
        assert_eq!(out, vec![Item::Fail(UpCode::Refused)]);
        let out = run(&[(
            "error",
            r#"{"type":"error","error":{"type":"overloaded_error","message":"m"}}"#,
        )]);
        assert_eq!(out, vec![Item::Fail(UpCode::Overloaded)]);
        // the name may come from the data's type
        let out = run(&[("message", r#"{"type":"message_stop"}"#)]);
        assert_eq!(out, vec![Item::Done]);
        assert_eq!(run(&[]), vec![Item::Fail(UpCode::Protocol)]);
        assert_eq!(run(&[("x", "nope")]), vec![Item::Fail(UpCode::Protocol)]);
    }

    #[test]
    fn thinking_blocks_are_reported_without_their_text() {
        let out = run(&[
            (
                "message_start",
                r#"{"message":{"usage":{"input_tokens":3}}}"#,
            ),
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"made-up idea"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"c2lnbmF0dXJl"}}"#,
            ),
            (
                "content_block_stop",
                r#"{"type":"content_block_stop","index":0}"#,
            ),
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"redacted_thinking","data":"bWFkZS11cA=="}}"#,
            ),
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":2,"content_block":{"type":"text","text":""}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":2,"delta":{"type":"text_delta","text":"Answer"}}"#,
            ),
            ("message_stop", r#"{"type":"message_stop"}"#),
        ]);
        assert_eq!(
            out,
            vec![
                Item::Usage {
                    input: Some(3),
                    output: None
                },
                Item::Thinking,
                Item::Thinking,
                Item::Thinking,
                Item::Text("Answer".into()),
                Item::Done
            ]
        );
        assert!(!format!("{out:?}").contains("made-up"));
    }

    #[test]
    fn a_body_that_ends_before_message_stop_is_cut_short() {
        let out = run(&[
            (
                "message_start",
                r#"{"type":"message_start","message":{"usage":{"input_tokens":4}}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"Half an ans"}}"#,
            ),
        ]);
        assert_eq!(
            out,
            vec![
                Item::Usage {
                    input: Some(4),
                    output: None
                },
                Item::Text("Half an ans".into()),
                Item::Fail(UpCode::Protocol)
            ]
        );
        // also after message_delta (stop reason and usage, but no message_stop)
        let out = run(&[
            ("message_start", r#"{"message":{}}"#),
            (
                "content_block_delta",
                r#"{"delta":{"type":"text_delta","text":"a"}}"#,
            ),
            (
                "message_delta",
                r#"{"delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}"#,
            ),
        ]);
        assert_eq!(out.last(), Some(&Item::Fail(UpCode::Protocol)));
        // only pings: no message at all
        assert_eq!(
            run(&[("ping", r#"{"type":"ping"}"#)]),
            vec![Item::Fail(UpCode::Protocol)]
        );
        // finish() after message_stop keeps the normal end
        let mut s = State::default();
        let mut out = Vec::new();
        s.on_event("message_stop", "{}", &mut out);
        assert_eq!(out, vec![Item::Done]);
        assert_eq!(s.finish(), Item::Done);
    }
}
