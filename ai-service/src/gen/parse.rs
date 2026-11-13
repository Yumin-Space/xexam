//! Parsing the model's answer (GP-3). The answer is data: it is parsed leniently around the JSON, every element is
//! validated field by field, and every string is sanitised at once (control and invisible format characters dropped)
//! so the checks, the dedup and the saved bank only ever see the sanitised text.

use serde_json::Value;

use crate::gen::{is_hidden_format, Tri};

#[derive(Debug, Clone, PartialEq)]
pub struct RawOption {
    pub text: Tri,
    pub correct: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawQuestion {
    pub slot: usize,
    pub stem: Tri,
    pub options: Vec<RawOption>,
    pub explanation: Tri,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// No braces, or the text between them is not JSON.
    NoJson,
    /// JSON without a non-empty `questions` array.
    NoQuestions,
}

/// Drops control characters (keeping `\n` when `keep_newline`) and the invisible format characters.
pub fn sanitize(s: &str, keep_newline: bool) -> String {
    s.chars()
        .filter(|&c| !is_hidden_format(c) && (!c.is_control() || (keep_newline && c == '\n')))
        .collect()
}

fn tri_of(v: &Value, keep_newline: bool) -> Option<Tri> {
    let o = v.as_object()?;
    let get = |k: &str| o.get(k)?.as_str().map(|s| sanitize(s, keep_newline));
    Some(Tri {
        en: get("en")?,
        zh: get("zh")?,
        ja: get("ja")?,
    })
}

fn element(v: &Value) -> Result<RawQuestion, usize> {
    let slot = v
        .get("slot")
        .and_then(Value::as_u64)
        .filter(|&n| n > 0)
        .map(|n| n as usize);
    let unusable = slot.unwrap_or(0);
    let slot = slot.ok_or(0usize)?;
    let stem = v
        .get("stem")
        .and_then(|s| tri_of(s, true))
        .ok_or(unusable)?;
    let explanation = v
        .get("explanation")
        .and_then(|s| tri_of(s, true))
        .ok_or(unusable)?;
    let list = v.get("options").and_then(Value::as_array).ok_or(unusable)?;
    let mut options = Vec::with_capacity(list.len());
    for o in list {
        let text = tri_of(o, false).ok_or(unusable)?;
        let correct = o.get("correct").and_then(Value::as_bool).ok_or(unusable)?;
        options.push(RawOption { text, correct });
    }
    Ok(RawQuestion {
        slot,
        stem,
        options,
        explanation,
    })
}

/// Parses the model's answer. Fences and prose around the JSON are tolerated. An element that does not fit is
/// `Err(slot)` (0 when the slot itself is unusable).
pub fn parse_answer(text: &str) -> Result<Vec<Result<RawQuestion, usize>>, ParseError> {
    let trimmed = text.trim();
    let unfenced: String;
    let mut body = trimmed;
    if trimmed.starts_with("```") {
        let mut lines: Vec<&str> = trimmed.lines().collect();
        lines.remove(0);
        if lines.last().map(|l| l.trim() == "```").unwrap_or(false) {
            lines.pop();
        }
        unfenced = lines.join("\n");
        body = &unfenced;
    }
    let start = body.find('{').ok_or(ParseError::NoJson)?;
    let end = body.rfind('}').ok_or(ParseError::NoJson)?;
    if end < start {
        return Err(ParseError::NoJson);
    }
    let value: Value = serde_json::from_str(&body[start..=end]).map_err(|_| ParseError::NoJson)?;
    let list = value
        .get("questions")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or(ParseError::NoQuestions)?;
    Ok(list.iter().map(element).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = r#"{"questions":[{"slot":2,"stem":{"en":"E","zh":"Z","ja":"J"},"options":[{"en":"a","zh":"b","ja":"c","correct":true},{"en":"d","zh":"e","ja":"f","correct":false}],"explanation":{"en":"x","zh":"y","ja":"z"}}]}"#;

    #[test]
    fn plain_json() {
        let v = parse_answer(ONE).unwrap();
        assert_eq!(v.len(), 1);
        let q = v[0].as_ref().unwrap();
        assert_eq!(q.slot, 2);
        assert_eq!(q.stem.zh, "Z");
        assert_eq!(q.options.len(), 2);
        assert!(q.options[0].correct && !q.options[1].correct);
        assert_eq!(q.explanation.ja, "z");
    }

    #[test]
    fn fenced_json_with_and_without_a_closing_fence() {
        let fenced = format!("```json\n{ONE}\n```");
        assert!(parse_answer(&fenced).unwrap()[0].is_ok());
        let open = format!("```\n{ONE}");
        assert!(parse_answer(&open).unwrap()[0].is_ok());
    }

    #[test]
    fn prose_around_the_json() {
        let t = format!("Sure, here you go:\n{ONE}\nHope this helps!");
        assert!(parse_answer(&t).unwrap()[0].is_ok());
    }

    #[test]
    fn no_json_and_no_questions() {
        assert_eq!(parse_answer("I cannot do that."), Err(ParseError::NoJson));
        assert_eq!(parse_answer("{not json}"), Err(ParseError::NoJson));
        assert_eq!(parse_answer("} {"), Err(ParseError::NoJson));
        assert_eq!(
            parse_answer(r#"{"items":[]}"#),
            Err(ParseError::NoQuestions)
        );
        assert_eq!(
            parse_answer(r#"{"questions":[]}"#),
            Err(ParseError::NoQuestions)
        );
        assert_eq!(
            parse_answer(r#"{"questions":"x"}"#),
            Err(ParseError::NoQuestions)
        );
        assert_eq!(parse_answer(""), Err(ParseError::NoJson));
    }

    #[test]
    fn bad_elements_keep_their_slot_number() {
        let t = r#"{"questions":[
            {"slot":1,"stem":{"en":"E","zh":"Z"},"options":[],"explanation":{"en":"x","zh":"y","ja":"z"}},
            {"slot":3,"stem":{"en":"E","zh":"Z","ja":"J"},"options":[{"en":"a","zh":"b","ja":"c"}],"explanation":{"en":"x","zh":"y","ja":"z"}},
            {"slot":"4"},
            {"slot":0},
            "junk",
            {"slot":5,"stem":{"en":"E","zh":"Z","ja":"J"},"options":[{"en":"a","zh":"b","ja":"c","correct":true}],"explanation":{"en":"x","zh":"y","ja":"z"},"extra":1}
        ]}"#;
        let v = parse_answer(t).unwrap();
        assert_eq!(v.len(), 6);
        assert_eq!(v[0], Err(1));
        assert_eq!(v[1], Err(3));
        assert_eq!(v[2], Err(0));
        assert_eq!(v[3], Err(0));
        assert_eq!(v[4], Err(0));
        assert!(v[5].is_ok());
    }

    #[test]
    fn strings_are_sanitised() {
        let t = serde_json::json!({"questions": [{
            "slot": 1,
            "stem": {"en": "Hi\u{202E} there\u{200B}\nline\u{0007}two\r", "zh": "Z", "ja": "J"},
            "options": [{"en": "a\nb\u{200E}", "zh": "b", "ja": "c", "correct": true}],
            "explanation": {"en": "x\ny", "zh": "y", "ja": "z"}
        }]})
        .to_string();
        let q = parse_answer(&t).unwrap().remove(0).unwrap();
        assert_eq!(q.stem.en, "Hi there\nlinetwo");
        assert_eq!(q.options[0].text.en, "ab");
        assert_eq!(q.explanation.en, "x\ny");
    }

    #[test]
    fn sanitize_keeps_zero_width_joiner() {
        assert_eq!(sanitize("a\u{200D}b\u{FEFF}", false), "a\u{200D}b");
    }
}
