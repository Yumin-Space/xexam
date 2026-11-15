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

/// One entry of `answer`: a letter A, B, C ... (case-insensitive) or a 0-based integer, as an option index.
fn answer_index(v: &Value) -> Option<usize> {
    if let Some(n) = v.as_u64() {
        return usize::try_from(n).ok();
    }
    let mut chars = v.as_str()?.trim().chars();
    let c = chars.next()?.to_ascii_uppercase();
    if chars.next().is_some() || !c.is_ascii_uppercase() {
        return None;
    }
    Some((c as u8 - b'A') as usize)
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
        options.push(RawOption {
            text,
            correct: false,
        });
    }
    match v.get("answer") {
        // v1.6 shape: one `answer` array of option letters (or 0-based positions) decides every flag.
        Some(answer) => {
            let picks = answer.as_array().ok_or(unusable)?;
            if picks.is_empty() {
                return Err(unusable);
            }
            for p in picks {
                let idx = answer_index(p).ok_or(unusable)?;
                options.get_mut(idx).ok_or(unusable)?.correct = true;
            }
        }
        // Old shape: per-option `correct` booleans. A missing one counts as false when another option is true;
        // with no true option at all the element stays unusable.
        None => {
            let mut any_true = false;
            let mut any_missing = false;
            for (o, raw) in list.iter().zip(options.iter_mut()) {
                raw.correct = match o.get("correct") {
                    None => {
                        any_missing = true;
                        false
                    }
                    Some(c) => c.as_bool().ok_or(unusable)?,
                };
                any_true |= raw.correct;
            }
            if any_missing && !any_true {
                return Err(unusable);
            }
        }
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

    fn with_options(opts: &str) -> String {
        format!(
            r#"{{"questions":[{{"slot":4,"stem":{{"en":"E","zh":"Z","ja":"J"}},"options":[{opts}],"explanation":{{"en":"x","zh":"y","ja":"z"}}}}]}}"#
        )
    }

    #[test]
    fn a_missing_correct_is_false_when_another_option_is_true() {
        let t = with_options(
            r#"{"en":"a","zh":"b","ja":"c","correct":true},{"en":"d","zh":"e","ja":"f"},{"en":"g","zh":"h","ja":"i"}"#,
        );
        let q = parse_answer(&t).unwrap().remove(0).unwrap();
        let flags: Vec<bool> = q.options.iter().map(|o| o.correct).collect();
        assert_eq!(flags, vec![true, false, false]);
    }

    #[test]
    fn all_correct_keys_missing_stays_unusable() {
        let t = with_options(r#"{"en":"a","zh":"b","ja":"c"},{"en":"d","zh":"e","ja":"f"}"#);
        assert_eq!(parse_answer(&t).unwrap()[0], Err(4));
    }

    #[test]
    fn a_non_boolean_correct_stays_unusable() {
        let t = with_options(
            r#"{"en":"a","zh":"b","ja":"c","correct":true},{"en":"d","zh":"e","ja":"f","correct":"yes"}"#,
        );
        assert_eq!(parse_answer(&t).unwrap()[0], Err(4));
    }

    fn with_answer(opts: &str, answer: &str) -> String {
        with_options(opts).replace(
            r#""options":["#,
            &format!(r#""answer":{answer},"options":["#),
        )
    }

    const THREE: &str =
        r#"{"en":"a","zh":"b","ja":"c"},{"en":"d","zh":"e","ja":"f"},{"en":"g","zh":"h","ja":"i"}"#;

    fn flags(t: &str) -> Vec<bool> {
        let q = parse_answer(t).unwrap().remove(0).unwrap();
        q.options.iter().map(|o| o.correct).collect()
    }

    #[test]
    fn answer_letters_set_the_flags() {
        assert_eq!(flags(&with_answer(THREE, r#"["B"]"#)), [false, true, false]);
        assert_eq!(
            flags(&with_answer(THREE, r#"["A","C"]"#)),
            [true, false, true]
        );
        assert_eq!(
            flags(&with_answer(THREE, r#"["C","C"]"#)),
            [false, false, true]
        );
    }

    #[test]
    fn answer_accepts_lower_case_and_integers() {
        assert_eq!(flags(&with_answer(THREE, r#"["b"]"#)), [false, true, false]);
        assert_eq!(flags(&with_answer(THREE, r#"[0,2]"#)), [true, false, true]);
    }

    #[test]
    fn a_bad_answer_is_unusable() {
        for a in [
            r#"["D"]"#,
            "[3]",
            "[]",
            r#"["AB"]"#,
            r#"[""]"#,
            r#""B""#,
            "[true]",
            "[null]",
        ] {
            assert_eq!(
                parse_answer(&with_answer(THREE, a)).unwrap()[0],
                Err(4),
                "{a}"
            );
        }
        // no answer and no `correct` flags at all
        assert_eq!(parse_answer(&with_options(THREE)).unwrap()[0], Err(4));
    }

    #[test]
    fn answer_wins_over_old_correct_flags() {
        let t = with_answer(
            r#"{"en":"a","zh":"b","ja":"c","correct":true},{"en":"d","zh":"e","ja":"f","correct":false}"#,
            r#"["B"]"#,
        );
        assert_eq!(flags(&t), [false, true]);
    }

    #[test]
    fn old_form_without_answer_still_reads() {
        let t = with_options(
            r#"{"en":"a","zh":"b","ja":"c","correct":false},{"en":"d","zh":"e","ja":"f","correct":true}"#,
        );
        assert_eq!(flags(&t), [false, true]);
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
