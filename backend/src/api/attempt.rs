//! Request sanitization for `/api/attempt` (server.py `clean_attempt`, v0.2 form).
//! Differences from the legacy version (inputs that used to drop the connection now give a deterministic result): `seconds` of Infinity/NaN/garbage -> 0; `results` not a list -> 400.
use super::json::PyVal;
use crate::models::{Attempt, AttemptResult};
use crate::security::{py_int_str, truncate_chars};

pub const MSG_BAD_RESULTS: &str = "成绩的格式不对";
pub const MSG_NO_RESULTS: &str = "没有成绩可保存";
const MAX_RESULTS: usize = 2000;
const MAX_SHEETS: usize = 50;
const MAX_SECONDS: i128 = 7 * 86400;

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

pub fn clean_attempt(d: &PyVal) -> Result<Attempt, String> {
    let empty = PyVal::List(Vec::new());
    let raw_results = match d.get("results") {
        Some(v) if v.truthy() => v,
        _ => &empty,
    };
    let PyVal::List(items) = raw_results else {
        return Err(MSG_BAD_RESULTS.to_string());
    };
    let mut results = Vec::new();
    for r in items.iter().take(MAX_RESULTS) {
        if let (Some(PyVal::Str(id)), Some(PyVal::Bool(correct))) = (r.get("id"), r.get("correct"))
        {
            results.push(AttemptResult {
                id: truncate_chars(id, 64),
                correct: *correct,
            });
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
    Ok(Attempt {
        results,
        sheets,
        mode,
        seconds: clean_seconds(d.get("seconds")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::json::parse_py_json;

    fn clean(s: &str) -> Result<Attempt, String> {
        clean_attempt(&parse_py_json(s.as_bytes()).unwrap())
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
