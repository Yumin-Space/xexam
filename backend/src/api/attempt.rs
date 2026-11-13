//! Request sanitization for `/api/attempt` (server.py `clean_attempt`, v0.2 form).
//! Differences from the legacy version (inputs that used to drop the connection now give a deterministic result): `seconds` of Infinity/NaN/garbage -> 0; `results` not a list -> 400.
//!
//! v0.5: each result may carry an optional `answer` (see [`clean_answer`]); a malformed answer only drops the answer, never the result.
//!
//! v0.6.5: an optional `"kind"`: `"exam"` (the default when it is missing or null) or `"practice"`. Anything else is refused
//! (400) rather than silently saved as an exam: a client that sends a kind means it.
//!
//! v1.1: two more optional keys, never refused. `"limit"` (an integer of seconds, 60 ..= 36000) is the time limit of a timed
//! exam; `"end"` (`"submit"` or `"timeUp"`) says how an exam ended. For every exam the cleaned attempt carries `end` (`Some`;
//! `"timeUp"` only together with a limit, else `Submit`), which makes the store record it under the not-answered rule; a
//! practice session carries neither (`limit` and `end` are `None`).
use super::json::PyVal;
pub use crate::models::is_empty_answer;
use crate::models::{Attempt, AttemptAnswers, AttemptEnd, AttemptKind, AttemptResult, SessionTag};
use crate::security::{py_int_str, truncate_chars};
use serde_json::Value;

pub const MSG_BAD_RESULTS: &str = "成绩的格式不对";
pub const MSG_NO_RESULTS: &str = "没有成绩可保存";
const MAX_RESULTS: usize = 2000;
const MAX_SHEETS: usize = 50;
const MAX_SECONDS: i128 = 7 * 86400;
/// The shortest and the longest time limit of a timed exam, in seconds (`attempts.time_limit`'s CHECK).
const MIN_LIMIT: i128 = 60;
const MAX_LIMIT: i128 = 36_000;
/// Limits of one stored answer: at most this many elements, each a string of at most `MAX_ANSWER_CHARS` characters, an integer
/// `0..=MAX_ANSWER_INT`, or null.
const MAX_ANSWER_ITEMS: usize = 64;
const MAX_ANSWER_CHARS: usize = 8;
const MAX_ANSWER_INT: i128 = 999;

/// The `answer` of one result as it will be stored: `["C"]`, `["A", "D"]`, `[0, 1, null]`. Anything else (not a list, too long,
/// an element of another type or out of range) → `None` (stored as unanswered).
pub fn clean_answer(v: Option<&PyVal>) -> Option<Value> {
    let PyVal::List(items) = v? else {
        return None;
    };
    if items.len() > MAX_ANSWER_ITEMS {
        return None;
    }
    items
        .iter()
        .map(|x| match x {
            PyVal::Null => Some(Value::Null),
            PyVal::Str(s) if s.chars().count() <= MAX_ANSWER_CHARS => Some(Value::from(s.as_str())),
            PyVal::Int(i) if (0..=MAX_ANSWER_INT).contains(i) => {
                i64::try_from(*i).ok().map(Value::from)
            }
            _ => None,
        })
        .collect::<Option<Vec<Value>>>()
        .map(Value::Array)
}

/// Python `max(0, min(int(x or 0), 7*86400))`; 0 when `int()` raises TypeError/ValueError/OverflowError.
fn clean_seconds(v: Option<&PyVal>) -> i64 {
    let n: Option<i128> = match v {
        None => Some(0),
        Some(x) if !x.truthy() => Some(0),
        Some(PyVal::Bool(b)) => Some(*b as i128),
        Some(PyVal::Int(i)) => Some(*i),
        Some(PyVal::Float(f)) if f.is_finite() => Some(f.trunc() as i128),
        Some(PyVal::Str(s)) => py_int_str(s),
        _ => None,
    };
    n.map(|n| n.clamp(0, MAX_SECONDS) as i64).unwrap_or(0)
}

/// The latest start time `"startedAt"` may name (unix seconds, the year 2100); anything else is stored as NULL.
const MAX_STARTED_AT: i128 = 4_102_444_800;

/// v0.9: the optional `"key"` (32 lower-case hex digits) and `"startedAt"` (an integer of unix seconds, 1 to the year 2100)
/// of a request: the session the record is saved from. Each is looked at on its own and an invalid or missing one is `None`
/// (stored as NULL): a request is never refused because of them, so older clients keep working.
pub fn clean_session_tag(d: &PyVal) -> SessionTag {
    let key = match d.get("key") {
        Some(PyVal::Str(s))
            if s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) =>
        {
            Some(s.clone())
        }
        _ => None,
    };
    let started_at = match d.get("startedAt") {
        Some(PyVal::Int(i)) if (1..=MAX_STARTED_AT).contains(i) => i64::try_from(*i).ok(),
        _ => None,
    };
    SessionTag { key, started_at }
}

/// v1.1: the time limit of a request: an integer of seconds in `MIN_LIMIT ..= MAX_LIMIT`, else `None` (missing, null, out of
/// range, a float, a string, a bool). Never refuses a request.
fn clean_limit(d: &PyVal) -> Option<i64> {
    match d.get("limit") {
        Some(PyVal::Int(i)) if (MIN_LIMIT..=MAX_LIMIT).contains(i) => i64::try_from(*i).ok(),
        _ => None,
    }
}

/// v1.1: `(limit, end)` of an attempt of `kind`. A practice session has neither. An exam always has an `end`: `TimeUp` when
/// the request says `"timeUp"` and a valid limit is stored, `Submit` in every other case (`"submit"`, `"timeUp"` without a
/// limit, a missing or any other value). Records written by v1.1 thus always follow the not-answered rule.
fn clean_limit_and_end(d: &PyVal, kind: AttemptKind) -> (Option<i64>, Option<AttemptEnd>) {
    if kind != AttemptKind::Exam {
        return (None, None);
    }
    let limit = clean_limit(d);
    let time_up = matches!(d.get("end"), Some(PyVal::Str(s)) if s == "timeUp");
    let end = if time_up && limit.is_some() {
        AttemptEnd::TimeUp
    } else {
        AttemptEnd::Submit
    };
    (limit, Some(end))
}

pub fn clean_attempt(d: &PyVal) -> Result<Attempt, String> {
    clean_attempt_with_answers(d).map(|(attempt, _)| attempt)
}

/// [`clean_attempt`] plus the cleaned answers, index-aligned with the kept results.
pub fn clean_attempt_with_answers(d: &PyVal) -> Result<(Attempt, AttemptAnswers), String> {
    let empty = PyVal::List(Vec::new());
    let raw_results = match d.get("results") {
        Some(v) if v.truthy() => v,
        _ => &empty,
    };
    let PyVal::List(items) = raw_results else {
        return Err(MSG_BAD_RESULTS.to_string());
    };
    let mut results = Vec::new();
    let mut answers = Vec::new();
    for r in items.iter().take(MAX_RESULTS) {
        if let (Some(PyVal::Str(id)), Some(PyVal::Bool(correct))) = (r.get("id"), r.get("correct"))
        {
            results.push(AttemptResult {
                id: truncate_chars(id, 64),
                correct: *correct,
            });
            answers.push(clean_answer(r.get("answer")));
        }
    }
    if results.is_empty() {
        return Err(MSG_NO_RESULTS.to_string());
    }
    let sheets: Vec<String> = match d.get("sheets") {
        Some(PyVal::List(l)) => l
            .iter()
            .filter_map(|s| {
                if let PyVal::Str(s) = s {
                    Some(truncate_chars(s, 80))
                } else {
                    None
                }
            })
            .take(MAX_SHEETS)
            .collect(),
        _ => Vec::new(),
    };
    let mode = match d.get("mode") {
        Some(PyVal::Str(m)) if matches!(m.as_str(), "all" | "weak" | "retry") => m.clone(),
        _ => "all".to_string(),
    };
    let kind = match d.get("kind") {
        None | Some(PyVal::Null) => AttemptKind::Exam,
        Some(PyVal::Str(k)) => AttemptKind::parse(k).ok_or_else(|| MSG_BAD_RESULTS.to_string())?,
        Some(_) => return Err(MSG_BAD_RESULTS.to_string()),
    };
    let (limit, end) = clean_limit_and_end(d, kind);
    Ok((
        Attempt {
            results,
            sheets,
            mode,
            seconds: clean_seconds(d.get("seconds")),
            kind,
            limit,
            end,
        },
        answers,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::json::parse_py_json;
    use serde_json::json;

    fn clean(s: &str) -> Result<Attempt, String> {
        clean_attempt(&parse_py_json(s.as_bytes()).unwrap())
    }

    #[test]
    fn session_tags_are_optional_and_never_refuse_a_request() {
        let tag = |s: &str| clean_session_tag(&parse_py_json(s.as_bytes()).unwrap());
        let key = "0123456789abcdef0123456789abcdef";
        assert_eq!(tag("{}"), SessionTag::default());
        assert_eq!(
            tag(&format!(r#"{{"key":"{key}","startedAt":1780000000}}"#)),
            SessionTag {
                key: Some(key.to_string()),
                started_at: Some(1_780_000_000)
            }
        );
        // Each one on its own; anything invalid is dropped
        for bad in [
            r#"{"key":"0123456789ABCDEF0123456789abcdef"}"#,
            r#"{"key":"short"}"#,
            r#"{"key":5}"#,
            r#"{"key":null}"#,
            r#"{"key":"0123456789abcdef0123456789abcdef0"}"#,
        ] {
            assert_eq!(tag(bad).key, None, "{bad}");
        }
        for bad in [
            r#"{"startedAt":0}"#,
            r#"{"startedAt":-5}"#,
            r#"{"startedAt":4102444801}"#,
            r#"{"startedAt":1.5}"#,
            r#"{"startedAt":"1780000000"}"#,
            r#"{"startedAt":null}"#,
            r#"{"startedAt":true}"#,
        ] {
            assert_eq!(tag(bad).started_at, None, "{bad}");
        }
        assert_eq!(tag(r#"{"startedAt":1}"#).started_at, Some(1));
        assert_eq!(
            tag(r#"{"startedAt":4102444800}"#).started_at,
            Some(4_102_444_800)
        );
        assert_eq!(
            tag(&format!(r#"{{"key":"{key}","startedAt":"x"}}"#))
                .key
                .as_deref(),
            Some(key)
        );
    }

    fn clean_with_answers(s: &str) -> (Attempt, AttemptAnswers) {
        clean_attempt_with_answers(&parse_py_json(s.as_bytes()).unwrap()).unwrap()
    }

    #[test]
    fn basic_and_defaults() {
        let a =
            clean(r#"{"results":[{"id":"a","correct":true},{"id":"b","correct":false}]}"#).unwrap();
        assert_eq!(a.results.len(), 2);
        assert_eq!((a.mode.as_str(), a.seconds, a.sheets.len()), ("all", 0, 0));
        assert!(a.results[0].correct && !a.results[1].correct);
    }

    #[test]
    fn kind_defaults_to_exam_and_accepts_practice_only() {
        let r = r#""results":[{"id":"a","correct":true}]"#;
        assert_eq!(clean(&format!("{{{r}}}")).unwrap().kind, AttemptKind::Exam);
        for (kind, want) in [
            ("null", AttemptKind::Exam),
            (r#""exam""#, AttemptKind::Exam),
            (r#""practice""#, AttemptKind::Practice),
        ] {
            let a = clean(&format!(r#"{{{r},"kind":{kind}}}"#)).unwrap();
            assert_eq!(a.kind, want, "{kind}");
        }
        for bad in [r#""Practice""#, r#""quiz""#, "1", "true", "[]", r#""""#] {
            assert_eq!(
                clean(&format!(r#"{{{r},"kind":{bad}}}"#)).unwrap_err(),
                MSG_BAD_RESULTS,
                "{bad}"
            );
        }
    }

    #[test]
    fn results_validation() {
        assert_eq!(clean("{}").unwrap_err(), MSG_NO_RESULTS);
        assert_eq!(clean(r#"{"results": []}"#).unwrap_err(), MSG_NO_RESULTS);
        assert_eq!(clean(r#"{"results": null}"#).unwrap_err(), MSG_NO_RESULTS);
        assert_eq!(clean(r#"{"results": 0}"#).unwrap_err(), MSG_NO_RESULTS);
        for bad in [r#""abc""#, "5", r#"{"a": 1}"#, "true"] {
            assert_eq!(
                clean(&format!(r#"{{"results": {bad}}}"#)).unwrap_err(),
                MSG_BAD_RESULTS,
                "{bad}"
            );
        }
        // correct must be a real bool; id must be a str
        assert_eq!(clean(r#"{"results":[{"id":"x","correct":1},"x",{"id":5,"correct":true},{"correct":true},null]}"#).unwrap_err(), MSG_NO_RESULTS);
        let a = clean(r#"{"results":[{"id":"x","correct":1},{"id":"ok","correct":true,"zzz":1}]}"#)
            .unwrap();
        assert_eq!(a.results.len(), 1);
    }

    #[test]
    fn id_truncated_by_chars_and_results_capped() {
        let long = "令".repeat(100);
        let a = clean(&format!(
            r#"{{"results":[{{"id":"{long}","correct":true}}]}}"#
        ))
        .unwrap();
        assert_eq!(a.results[0].id.chars().count(), 64);
        let many = vec![r#"{"id":"a","correct":true}"#; 2500].join(",");
        let a = clean(&format!(r#"{{"results":[{many}]}}"#)).unwrap();
        assert_eq!(a.results.len(), 2000);
        // Only the first 2000 are taken: valid items after that do not fill in
        let mut parts = vec!["1"; 2000];
        parts.push(r#"{"id":"late","correct":true}"#);
        assert_eq!(
            clean(&format!(r#"{{"results":[{}]}}"#, parts.join(","))).unwrap_err(),
            MSG_NO_RESULTS
        );
    }

    #[test]
    fn sheets_rules() {
        let a = clean(r#"{"results":[{"id":"a","correct":true}],"sheets":["s1",5,null,"s2"]}"#)
            .unwrap();
        assert_eq!(a.sheets, vec!["s1", "s2"]);
        for v in ["\"abc\"", "5", "{\"a\":1}", "null", "true"] {
            let a = clean(&format!(
                r#"{{"results":[{{"id":"a","correct":true}}],"sheets":{v}}}"#
            ))
            .unwrap();
            assert!(a.sheets.is_empty(), "{v}");
        }
        let many = (0..60)
            .map(|i| format!("\"s{i}\""))
            .collect::<Vec<_>>()
            .join(",");
        let a = clean(&format!(
            r#"{{"results":[{{"id":"a","correct":true}}],"sheets":[{many}]}}"#
        ))
        .unwrap();
        assert_eq!(a.sheets.len(), 50);
        let long = "あ".repeat(100);
        let a = clean(&format!(
            r#"{{"results":[{{"id":"a","correct":true}}],"sheets":["{long}"]}}"#
        ))
        .unwrap();
        assert_eq!(a.sheets[0].chars().count(), 80);
    }

    #[test]
    fn mode_rules() {
        for (m, want) in [
            ("\"weak\"", "weak"),
            ("\"retry\"", "retry"),
            ("\"all\"", "all"),
            ("\"WEAK\"", "all"),
            ("5", "all"),
            ("null", "all"),
            ("[]", "all"),
        ] {
            let a = clean(&format!(
                r#"{{"results":[{{"id":"a","correct":true}}],"mode":{m}}}"#
            ))
            .unwrap();
            assert_eq!(a.mode, want, "{m}");
        }
    }

    #[test]
    fn answers_are_cleaned_per_result() {
        let (a, ans) = clean_with_answers(
            r#"{"results":[
                {"id":"s","correct":true,"answer":["C"]},
                {"id":"m","correct":false,"answer":["A","D"]},
                {"id":"h","correct":false,"answer":[0,1,null]},
                {"id":"none","correct":true},
                {"id":"null","correct":true,"answer":null},
                {"id":"empty","correct":true,"answer":[]},
                {"id":"dropped","correct":1,"answer":["A"]},
                {"id":"bad","correct":true,"answer":"C"}
            ]}"#,
        );
        assert_eq!(
            a.results.len(),
            7,
            "the result with a non-bool correct is dropped with its answer"
        );
        assert_eq!(
            ans,
            vec![
                Some(json!(["C"])),
                Some(json!(["A", "D"])),
                Some(json!([0, 1, null])),
                None,
                None,
                Some(json!([])),
                None
            ]
        );
        assert_eq!(
            a.results[6].id, "bad",
            "a malformed answer keeps its result"
        );
    }

    #[test]
    fn answer_limits() {
        let one = |answer: &str| {
            clean_with_answers(&format!(
                r#"{{"results":[{{"id":"a","correct":true,"answer":{answer}}}]}}"#
            ))
            .1
            .remove(0)
        };
        assert_eq!(one(r#"["12345678"]"#), Some(json!(["12345678"])));
        assert_eq!(one(r#"["123456789"]"#), None, "9 characters");
        assert_eq!(
            one(r#"["日本語の答え八字"]"#),
            Some(json!(["日本語の答え八字"])),
            "counted in characters"
        );
        assert_eq!(one("[0, 999]"), Some(json!([0, 999])));
        for bad in [
            "[1000]", "[-1]", "[1.0]", "[1.5]", "[true]", "[[0]]", "[{}]", "[NaN]", "{}", "5",
            r#""A""#, "true",
        ] {
            assert_eq!(one(bad), None, "{bad}");
        }
        let max = vec!["0"; 64].join(",");
        assert_eq!(
            one(&format!("[{max}]")).map(|v| v.as_array().map(Vec::len)),
            Some(Some(64))
        );
        let over = vec!["0"; 65].join(",");
        assert_eq!(one(&format!("[{over}]")), None, "65 elements");
        // duplicate keys: the later one wins (same as the rest of the request)
        assert_eq!(one(r#"["A"], "answer": ["B"]"#), Some(json!(["B"])));
    }

    #[test]
    fn limit_and_end_are_cleaned_for_exams_only_and_never_refuse() {
        let r = r#""results":[{"id":"a","correct":true}]"#;
        let one = |extra: &str| clean(&format!("{{{r}{extra}}}")).unwrap();
        // an exam always has an end; the default is a hand-in without a limit
        let a = one("");
        assert_eq!((a.limit, a.end), (None, Some(AttemptEnd::Submit)));
        for (extra, limit, end) in [
            (r#","limit":3600"#, Some(3600), AttemptEnd::Submit),
            (
                r#","limit":3600,"end":"submit""#,
                Some(3600),
                AttemptEnd::Submit,
            ),
            (
                r#","limit":3600,"end":"timeUp""#,
                Some(3600),
                AttemptEnd::TimeUp,
            ),
            (
                r#","limit":60,"end":"timeUp""#,
                Some(60),
                AttemptEnd::TimeUp,
            ),
            (
                r#","limit":36000,"end":"timeUp""#,
                Some(36000),
                AttemptEnd::TimeUp,
            ),
            // time-up without a stored limit is a hand-in
            (r#","end":"timeUp""#, None, AttemptEnd::Submit),
            (r#","limit":59,"end":"timeUp""#, None, AttemptEnd::Submit),
            (r#","limit":36001,"end":"timeUp""#, None, AttemptEnd::Submit),
            (r#","limit":1.5,"end":"timeUp""#, None, AttemptEnd::Submit),
            (
                r#","limit":3600.0,"end":"timeUp""#,
                None,
                AttemptEnd::Submit,
            ),
            (
                r#","limit":"3600","end":"timeUp""#,
                None,
                AttemptEnd::Submit,
            ),
            (r#","limit":true,"end":"timeUp""#, None, AttemptEnd::Submit),
            (r#","limit":null,"end":"timeUp""#, None, AttemptEnd::Submit),
            (r#","limit":-5"#, None, AttemptEnd::Submit),
            // anything but "timeUp" is a hand-in
            (
                r#","limit":3600,"end":"TIMEUP""#,
                Some(3600),
                AttemptEnd::Submit,
            ),
            (r#","limit":3600,"end":5"#, Some(3600), AttemptEnd::Submit),
            (
                r#","limit":3600,"end":null"#,
                Some(3600),
                AttemptEnd::Submit,
            ),
            (
                r#","limit":3600,"end":["timeUp"]"#,
                Some(3600),
                AttemptEnd::Submit,
            ),
            (
                r#","kind":"exam","limit":600,"end":"timeUp""#,
                Some(600),
                AttemptEnd::TimeUp,
            ),
        ] {
            let a = one(extra);
            assert_eq!((a.limit, a.end), (limit, Some(end)), "{extra}");
        }
        // a practice session has neither, whatever is sent
        for extra in [
            r#","kind":"practice""#,
            r#","kind":"practice","limit":3600"#,
            r#","kind":"practice","limit":3600,"end":"timeUp""#,
            r#","kind":"practice","end":"submit""#,
        ] {
            let a = one(extra);
            assert_eq!(
                (a.kind, a.limit, a.end),
                (AttemptKind::Practice, None, None),
                "{extra}"
            );
        }
    }

    #[test]
    fn empty_answers_are_nothing_chosen() {
        for empty in [
            None,
            Some(json!(null)),
            Some(json!([])),
            Some(json!([null])),
            Some(json!([null, null, null])),
        ] {
            assert!(is_empty_answer(empty.as_ref()), "{empty:?}");
        }
        for answered in [
            json!(["C"]),
            json!(["A", "D"]),
            json!([0, 1, null]),
            json!([null, 0]),
            json!([0]),
            json!([""]),
        ] {
            assert!(!is_empty_answer(Some(&answered)), "{answered}");
        }
        // what the cleaning keeps as Some: `[]` and `[null, ...]` reach this check
        let (_, ans) = clean_with_answers(
            r#"{"results":[
                {"id":"a","correct":false,"answer":[]},
                {"id":"b","correct":false,"answer":[null,null]},
                {"id":"c","correct":false,"answer":"A"},
                {"id":"d","correct":false}
            ]}"#,
        );
        assert!(ans.iter().all(|a| is_empty_answer(a.as_ref())), "{ans:?}");
    }

    #[test]
    fn seconds_rules() {
        let secs = |v: &str| {
            clean(&format!(
                r#"{{"results":[{{"id":"a","correct":true}}],"seconds":{v}}}"#
            ))
            .unwrap()
            .seconds
        };
        assert_eq!(secs("60"), 60);
        assert_eq!(secs("99999999999999999999999999"), 604800);
        assert_eq!(secs("-5"), 0);
        assert_eq!(secs("12.7"), 12);
        assert_eq!(secs("-12.7"), 0);
        assert_eq!(secs("1e30"), 604800);
        assert_eq!(secs("\"77\""), 77);
        assert_eq!(secs("\" 77 \""), 77);
        assert_eq!(secs("\"abc\""), 0);
        assert_eq!(secs("\"12.7\""), 0);
        assert_eq!(secs("Infinity"), 0);
        assert_eq!(secs("-Infinity"), 0);
        assert_eq!(secs("NaN"), 0);
        assert_eq!(secs("null"), 0);
        assert_eq!(secs("true"), 1);
        assert_eq!(secs("false"), 0);
        assert_eq!(secs("[1]"), 0);
        assert_eq!(secs("{}"), 0);
        assert_eq!(secs("604800"), 604800);
        assert_eq!(secs("604801"), 604800);
    }
}
