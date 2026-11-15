//! JSON Schemas of the model's structured answers (v1.6): the writing calls (`{"questions":[...]}`), the per-chunk plan
//! call and the global plan call. They stay inside the subset both Anthropic and OpenAI strict mode accept: `type`,
//! `properties`, `required` listing every property, `additionalProperties: false`, `items` and `enum`; no length, count,
//! pattern or format keywords (the checks in `gen::parse`, `gen::check` and `gen::whole` still enforce those).

use serde_json::{json, Value};
use std::sync::LazyLock;

fn tri() -> Value {
    json!({
        "type": "object",
        "properties": {
            "en": {"type": "string"},
            "zh": {"type": "string"},
            "ja": {"type": "string"}
        },
        "required": ["en", "zh", "ja"],
        "additionalProperties": false
    })
}

fn option() -> Value {
    json!({
        "type": "object",
        "properties": {
            "en": {"type": "string"},
            "zh": {"type": "string"},
            "ja": {"type": "string"}
        },
        "required": ["en", "zh", "ja"],
        "additionalProperties": false
    })
}

static QUESTIONS: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "type": "object",
        "properties": {
            "questions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "slot": {"type": "integer"},
                        "stem": tri(),
                        "options": {"type": "array", "items": option()},
                        "answer": {"type": "array", "items": {"type": "string"}},
                        "explanation": tri()
                    },
                    "required": ["slot", "stem", "options", "answer", "explanation"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["questions"],
        "additionalProperties": false
    })
});

static GEN_PLAN: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "type": "object",
        "properties": {
            "topics": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "t": {"type": "integer"},
                        "name": {"type": "string"},
                        "refs": {"type": "array", "items": {"type": "string"}}
                    },
                    "required": ["t", "name", "refs"],
                    "additionalProperties": false
                }
            },
            "slots": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "slot": {"type": "integer"},
                        "topic": {"type": "integer"},
                        "refs": {"type": "array", "items": {"type": "string"}},
                        "angle": {"type": "string"},
                        "diff": {"type": "string", "enum": ["easy", "medium", "hard"]}
                    },
                    "required": ["slot", "topic", "refs", "angle", "diff"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["topics", "slots"],
        "additionalProperties": false
    })
});

static GLOBAL_PLAN: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "type": "object",
        "properties": {
            "chunks": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "chunk": {"type": "integer"},
                        "topics": {"type": "array", "items": {"type": "string"}},
                        "avoid": {"type": "array", "items": {"type": "string"}}
                    },
                    "required": ["chunk", "topics", "avoid"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["chunks"],
        "additionalProperties": false
    })
});

/// The answer of a writing call: `{"questions":[{"slot","stem","options","answer","explanation"}]}`.
pub fn questions() -> &'static Value {
    &QUESTIONS
}

/// The answer of a per-chunk plan call (`TASK PLAN`).
pub fn gen_plan() -> &'static Value {
    &GEN_PLAN
}

/// The answer of the global plan call (`TASK GLOBAL`).
pub fn global_plan() -> &'static Value {
    &GLOBAL_PLAN
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every object node: `additionalProperties` false and `required` = all property keys; no forbidden keyword.
    fn strict(node: &Value) {
        let Some(o) = node.as_object() else { return };
        for bad in [
            "minLength",
            "maxLength",
            "minItems",
            "maxItems",
            "pattern",
            "format",
        ] {
            assert!(!o.contains_key(bad), "{bad}");
        }
        if o.get("type") == Some(&json!("object")) {
            assert_eq!(o.get("additionalProperties"), Some(&json!(false)));
            let props = o["properties"].as_object().unwrap();
            let mut keys: Vec<&str> = props.keys().map(String::as_str).collect();
            let mut req: Vec<&str> = o["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            keys.sort_unstable();
            req.sort_unstable();
            assert_eq!(keys, req);
            props.values().for_each(strict);
        }
        if let Some(items) = o.get("items") {
            strict(items);
        }
    }

    /// A tiny structural validator for the subset above.
    fn valid(schema: &Value, v: &Value) -> bool {
        let ty = schema["type"].as_str().unwrap();
        let ok = match ty {
            "string" => v.is_string(),
            "boolean" => v.is_boolean(),
            "integer" => v.is_i64() || v.is_u64(),
            "array" => v
                .as_array()
                .is_some_and(|a| a.iter().all(|x| valid(&schema["items"], x))),
            "object" => v.as_object().is_some_and(|o| {
                let props = schema["properties"].as_object().unwrap();
                o.len() == props.len()
                    && props
                        .iter()
                        .all(|(k, s)| o.get(k).is_some_and(|x| valid(s, x)))
            }),
            _ => false,
        };
        ok && schema
            .get("enum")
            .and_then(Value::as_array)
            .is_none_or(|e| e.contains(v))
    }

    #[test]
    fn every_schema_is_strict() {
        for s in [questions(), gen_plan(), global_plan()] {
            assert_eq!(s["type"], "object");
            strict(s);
        }
    }

    /// The `{"questions":...}` lines of a prompt (the shape line and the example).
    fn question_lines(prompt: &str) -> Vec<Value> {
        prompt
            .lines()
            .filter(|l| l.starts_with("{\"questions\":"))
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn the_prompt_examples_fit_the_questions_schema() {
        for p in [
            crate::gen::whole::GEN_WHOLE_SYSTEM_PROMPT,
            crate::import::convert::CONVERT_SYSTEM_PROMPT,
            crate::import::material::MATERIAL_SYSTEM_PROMPT,
        ] {
            let lines = question_lines(p);
            assert!(lines.len() >= 2);
            for v in lines {
                assert!(valid(questions(), &v));
            }
        }
        // a missing key and a wrong type are refused
        let bad = json!({"questions":[{"slot":1,"stem":{"en":"a","zh":"b"},"options":[],"explanation":{"en":"a","zh":"b","ja":"c"}}]});
        assert!(!valid(questions(), &bad));
        let bad = json!({"questions":[{"slot":"1"}]});
        assert!(!valid(questions(), &bad));
    }

    #[test]
    fn plan_answers_fit_their_schemas() {
        let plan = json!({"topics":[{"t":1,"name":"x","refs":["Q3","Q17"]}],"slots":[{"slot":1,"topic":1,"refs":["Q3"],"angle":"a","diff":"easy"}]});
        assert!(valid(gen_plan(), &plan));
        let plan =
            json!({"topics":[],"slots":[{"slot":1,"topic":1,"refs":[],"angle":"a","diff":"mid"}]});
        assert!(!valid(gen_plan(), &plan));
        let global = json!({"chunks":[{"chunk":1,"topics":["a"],"avoid":[]}]});
        assert!(valid(global_plan(), &global));
        assert!(!valid(
            global_plan(),
            &json!({"chunks":[{"chunk":1,"topics":["a"]}]})
        ));
    }
}
