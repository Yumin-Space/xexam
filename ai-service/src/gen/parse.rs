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

/// Index just after the string literal that opens at `i` (`b[i] == b'"'`); the text length when it never closes.
fn skip_string(b: &[u8], i: usize) -> usize {
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'"' => return j + 1,
            _ => j += 1,
        }
    }
    b.len()
}

/// Index of the `}` that closes the object opening at `i` (string-aware), or None when it never closes.
fn match_brace(b: &[u8], i: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut j = i;
    while j < b.len() {
        match b[j] {
            b'"' => {
                j = skip_string(b, j);
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// `true` when the string literal spanning `text[i..end]` is exactly `"key"`.
fn is_key(text: &str, i: usize, end: usize, key: &str) -> bool {
    end <= text.len()
        && text.get(i + 1..end.saturating_sub(1)) == Some(key)
        && end - i == key.len() + 2
}

/// Repair for one known slip: a `"stem":{en,zh,ja}` object followed straight by `,{` (the model left out the
/// `"options":[` key before the first option). Inserts the key. None when nothing was changed.
fn repair_missing_options(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len() + 16);
    let (mut last, mut i, mut changed) = (0usize, 0usize, false);
    while i < b.len() {
        if b[i] != b'"' {
            i += 1;
            continue;
        }
        let end = skip_string(b, i);
        let stem = is_key(text, i, end, "stem");
        i = end;
        if !stem {
            continue;
        }
        let colon = skip_ws(b, i);
        if b.get(colon) != Some(&b':') {
            continue;
        }
        let open = skip_ws(b, colon + 1);
        if b.get(open) != Some(&b'{') {
            continue;
        }
        let Some(close) = match_brace(b, open) else {
            continue;
        };
        let is_tri = serde_json::from_str::<Value>(&text[open..=close])
            .ok()
            .and_then(|v| tri_of(&v, true))
            .is_some();
        let comma = skip_ws(b, close + 1);
        if !is_tri || b.get(comma) != Some(&b',') {
            continue;
        }
        let next = skip_ws(b, comma + 1);
        if b.get(next) != Some(&b'{') {
            continue;
        }
        out.push_str(&text[last..=comma]);
        out.push_str("\"options\":[");
        last = comma + 1;
        changed = true;
        i = close + 1;
    }
    if !changed {
        return None;
    }
    out.push_str(&text[last..]);
    Some(out)
}

/// Parses `text` as JSON, with the missing-`options` repair as the second try.
fn parse_value(text: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        return Some(v);
    }
    serde_json::from_str::<Value>(&repair_missing_options(text)?).ok()
}

/// The `slot` number written in one question object's text (0 when none can be read).
fn slot_in(text: &str) -> usize {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'"' {
            i += 1;
            continue;
        }
        let end = skip_string(b, i);
        if is_key(text, i, end, "slot") {
            let colon = skip_ws(b, end);
            if b.get(colon) == Some(&b':') {
                let d = skip_ws(b, colon + 1);
                let digits: String = text[d..].chars().take_while(char::is_ascii_digit).collect();
                if let Ok(n) = digits.parse::<usize>() {
                    return n;
                }
            }
        }
        i = end;
    }
    0
}

/// Per-question salvage: splits the `questions` array text into its top-level objects (string-aware; an object that
/// never closes is dropped) and reads each on its own. A broken object with a readable slot is `Err(slot)`; one
/// without a slot is skipped (its slot is redone as missing).
fn salvage(text: &str) -> Vec<Result<RawQuestion, usize>> {
    let b = text.as_bytes();
    let mut i = 0;
    let mut at = None;
    while i < b.len() {
        if b[i] != b'"' {
            i += 1;
            continue;
        }
        let end = skip_string(b, i);
        if is_key(text, i, end, "questions") {
            let colon = skip_ws(b, end);
            if b.get(colon) == Some(&b':') {
                let open = skip_ws(b, colon + 1);
                if b.get(open) == Some(&b'[') {
                    at = Some(open + 1);
                    break;
                }
            }
        }
        i = end;
    }
    let mut out = Vec::new();
    let Some(mut i) = at else { return out };
    while i < b.len() {
        match b[i] {
            b'"' => i = skip_string(b, i),
            b']' => break,
            b'{' => {
                let Some(close) = match_brace(b, i) else {
                    break;
                };
                let obj = &text[i..=close];
                match parse_value(obj) {
                    Some(v) => out.push(element(&v)),
                    None => {
                        let slot = slot_in(obj);
                        if slot > 0 {
                            out.push(Err(slot));
                        }
                    }
                }
                i = close + 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// Parses the model's answer. Fences and prose around the JSON are tolerated. An element that does not fit is
/// `Err(slot)` (0 when the slot itself is unusable). A known slip is repaired, and when the whole text is still not
/// JSON the questions that are readable on their own are kept.
pub fn parse_answer(text: &str) -> Result<Vec<Result<RawQuestion, usize>>, ParseError> {
    parse_answer_flagged(text).map(|(list, _)| list)
}

/// Like `parse_answer`, and also says whether the list came from per-question salvage (the whole text is not JSON);
/// the caller still decides from the text whether the answer was cut.
pub fn parse_answer_flagged(
    text: &str,
) -> Result<(Vec<Result<RawQuestion, usize>>, bool), ParseError> {
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
    let Some(value) = parse_value(&body[start..=end]) else {
        let list = salvage(&body[start..]);
        // nothing with a usable slot (only stray objects such as a lone option): the answer stays unreadable
        if list.iter().all(|r| matches!(r, Err(0))) {
            return Err(ParseError::NoJson);
        }
        return Ok((list, true));
    };
    let list = value
        .get("questions")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or(ParseError::NoQuestions)?;
    Ok((list.iter().map(element).collect(), false))
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

    // ---- v1.7: repair and salvage (self-made samples)

    /// One question; `tail` follows the stem (`,` = the missing-options slip, `,"options":[` = the right form).
    fn q(slot: usize, tail: &str) -> String {
        format!(
            r#"{{"slot":{slot},"stem":{{"en":"E{slot}","zh":"Z","ja":"J"}}{tail}{{"en":"a","zh":"b","ja":"c"}},{{"en":"d","zh":"e","ja":"f"}}],"answer":["B"],"explanation":{{"en":"x","zh":"y","ja":"z"}}}}"#
        )
    }
    const OPT: &str = r#","options":["#;

    #[test]
    fn the_missing_options_key_is_repaired() {
        let t = format!(
            r#"{{"questions":[{},{},{}]}}"#,
            q(1, ","),
            q(2, OPT),
            q(3, ",")
        );
        let (list, salvaged) = parse_answer_flagged(&t).unwrap();
        assert!(!salvaged, "whole-text repair, not salvage");
        assert_eq!(list.len(), 3);
        for (i, r) in list.iter().enumerate() {
            let r = r.as_ref().unwrap();
            assert_eq!(r.slot, i + 1);
            assert_eq!(r.options.len(), 2);
            assert!(!r.options[0].correct && r.options[1].correct);
        }
    }

    #[test]
    fn one_garbage_question_costs_only_its_own_slot() {
        let bad = r#"{"slot":3,"stem":{"en":"E","zh":"Z" "ja":}, ???}"#;
        let t = format!(
            r#"{{"questions":[{},{},{bad},{},{}]}}"#,
            q(1, OPT),
            q(2, ","),
            q(4, OPT),
            q(5, ",")
        );
        let (list, salvaged) = parse_answer_flagged(&t).unwrap();
        assert!(salvaged);
        assert_eq!(list.len(), 5);
        assert_eq!(list.iter().filter(|r| r.is_ok()).count(), 4);
        assert_eq!(list[2], Err(3));
        assert_eq!(list[4].as_ref().unwrap().slot, 5);
    }

    #[test]
    fn a_broken_object_without_a_slot_is_skipped() {
        let t = format!(
            r#"{{"questions":[{},{{"stem":oops}},{}]}}"#,
            q(1, OPT),
            q(2, OPT)
        );
        let list = parse_answer(&t).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|r| r.is_ok()));
    }

    #[test]
    fn braces_and_escaped_quotes_inside_strings_do_not_confuse_the_matcher() {
        let tricky = r#"{"slot":1,"stem":{"en":"say \"} {\" now","zh":"Z","ja":"J"},{"en":"a \"{\"","zh":"b","ja":"c"},{"en":"d}","zh":"e","ja":"f"}],"answer":["A"],"explanation":{"en":"x","zh":"y","ja":"z"}}"#;
        let t = format!(r#"{{"questions":[{tricky},{{"slot":2,"stem":nope}}]}}"#);
        let list = parse_answer(&t).unwrap();
        assert_eq!(list.len(), 2);
        let first = list[0].as_ref().unwrap();
        assert_eq!(first.stem.en, "say \"} {\" now");
        assert_eq!(first.options[0].text.en, "a \"{\"");
        assert_eq!(first.options[1].text.en, "d}");
        assert_eq!(list[1], Err(2));
    }

    #[test]
    fn a_cut_answer_keeps_its_complete_questions() {
        let full = format!(
            r#"{{"questions":[{},{},{}]}}"#,
            q(1, OPT),
            q(2, ","),
            q(3, OPT)
        );
        let cut = &full[..full.len() - 60];
        assert!(!cut.ends_with('}'));
        let (list, salvaged) = parse_answer_flagged(cut).unwrap();
        assert!(salvaged);
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|r| r.is_ok()));
        // nothing complete at all: the old error
        assert_eq!(
            parse_answer(r#"{"questions":[{"slot":1,"stem":{"en":"cut"#),
            Err(ParseError::NoJson)
        );
        assert_eq!(
            parse_answer(r#"{"questions":[{"slot":1,"stem":{"en":"a","zh":"b","ja":"c"}"#),
            Err(ParseError::NoJson)
        );
    }

    #[test]
    fn salvage_handles_cjk_text_nested_arrays_and_a_trailing_fence() {
        let cjk = r#"{"slot":1,"stem":{"en":"Pick {one}","zh":"选择「}」和【{】中的一个","ja":"どれ？\"}\""},{"en":"a","zh":"甲}","ja":"あ{"},{"en":"b","zh":"乙","ja":"い"}],"answer":["A"],"explanation":{"en":"[x]","zh":"因为]","ja":"理由"}}"#;
        let t = format!(
            "```json\n{{\"questions\":[{cjk},{{\"slot\":2,\"stem\":[1,[2,{{}}]],\"bad\":}},{}]}}\n```",
            q(3, OPT)
        );
        let (list, salvaged) = parse_answer_flagged(&t).unwrap();
        assert!(salvaged);
        assert_eq!(list.len(), 3);
        let first = list[0].as_ref().unwrap();
        assert_eq!(first.stem.zh, "选择「}」和【{】中的一个");
        assert_eq!(first.stem.ja, "どれ？\"}\"");
        assert_eq!(first.options[0].text.zh, "甲}");
        assert!(first.options[0].correct);
        assert_eq!(list[1], Err(2));
        assert_eq!(list[2].as_ref().unwrap().slot, 3);
        // only stray objects without a slot (a lone option after a broken question): still unreadable
        assert_eq!(
            parse_answer(r#"{"questions":[{"slot":x},{"en":"a","zh":"b","ja":"c"}]"#),
            Err(ParseError::NoJson)
        );
    }

    #[test]
    fn the_repair_leaves_normal_answers_and_other_objects_alone() {
        assert!(repair_missing_options(ONE).is_none());
        // a stem that is not a full en / zh / ja object is not touched
        assert!(repair_missing_options(r#"{"stem":{"en":"a"},{"x":1}}"#).is_none());
        // the word "stem" inside a string is not a key
        assert!(repair_missing_options(r#"["\"stem\":{}",{"a":1}]"#).is_none());
    }

    #[test]
    fn sanitize_keeps_zero_width_joiner() {
        assert_eq!(sanitize("a\u{200D}b\u{FEFF}", false), "a\u{200D}b");
    }
}
