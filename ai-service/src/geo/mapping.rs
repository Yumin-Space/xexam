//! Reading a provider's JSON answer (spec 5.3): dot paths, the optional success / error checks and the place fields.
use super::{Failure, FailureCode, GeoPlace};
use serde_json::Value;

/// The JSON field mapping of a provider. Every path is "" (not used) or a dot path of 1 to 8 segments.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Mapping {
    pub country: String,
    pub country_code: String,
    pub region: String,
    pub city: String,
    /// Optional success check: the value at `ok_path` (as text) must equal `ok_value`
    pub ok_path: String,
    pub ok_value: String,
    /// Optional error flag: a truthy value at `error_path` means the provider reported a failure
    pub error_path: String,
    /// Where the provider's failure text is
    pub message_path: String,
}

/// The names of the eight mapping fields in the request body, in order (also the `mapping.<name>` of error fields).
pub const FIELD_NAMES: [&str; 8] = [
    "country",
    "countryCode",
    "region",
    "city",
    "okPath",
    "okValue",
    "errorPath",
    "messagePath",
];

/// Longest place text kept (characters).
pub const PLACE_MAX_CHARS: usize = 64;
/// Longest provider failure text returned to the administrator (characters).
pub const MESSAGE_MAX_CHARS: usize = 120;

impl Mapping {
    /// The field named `name` (see [`FIELD_NAMES`]).
    pub fn get(&self, name: &str) -> Option<&str> {
        Some(match name {
            "country" => &self.country,
            "countryCode" => &self.country_code,
            "region" => &self.region,
            "city" => &self.city,
            "okPath" => &self.ok_path,
            "okValue" => &self.ok_value,
            "errorPath" => &self.error_path,
            "messagePath" => &self.message_path,
            _ => return None,
        })
    }

    /// Sets the field named `name`; `false` for an unknown name.
    pub fn set(&mut self, name: &str, value: String) -> bool {
        let slot = match name {
            "country" => &mut self.country,
            "countryCode" => &mut self.country_code,
            "region" => &mut self.region,
            "city" => &mut self.city,
            "okPath" => &mut self.ok_path,
            "okValue" => &mut self.ok_value,
            "errorPath" => &mut self.error_path,
            "messagePath" => &mut self.message_path,
            _ => return false,
        };
        *slot = value;
        true
    }

    /// The mapping as the JSON object of the API (camelCase keys, all eight present).
    pub fn to_json(&self) -> Value {
        let mut m = serde_json::Map::new();
        for name in FIELD_NAMES {
            m.insert(
                name.to_string(),
                Value::from(self.get(name).unwrap_or_default()),
            );
        }
        Value::Object(m)
    }
}

/// A dot path is valid: 1 to 8 segments, each 1 to 64 characters of `A-Z a-z 0-9 _ -`.
pub fn valid_path(path: &str) -> bool {
    let mut n = 0usize;
    for seg in path.split('.') {
        n += 1;
        if n > 8
            || seg.is_empty()
            || seg.len() > 64
            || !seg
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return false;
        }
    }
    n >= 1
}

/// The value at a dot path: an object key per segment, or an array index for an all-digit segment on an array.
pub fn value_at<'a>(json: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = json;
    for seg in path.split('.') {
        cur = match cur {
            Value::Object(m) => m.get(seg)?,
            Value::Array(a) => {
                if seg.is_empty() || !seg.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                a.get(seg.parse::<usize>().ok()?)?
            }
            _ => return None,
        };
    }
    Some(cur)
}

/// The text of a scalar value: a string as it is, a number as its decimal text, a bool as `true` / `false`; null, an
/// object and an array have none.
pub fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Truthy: `true`, a non-empty string other than `false` / `0`, a non-zero number, a non-empty object or array.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty() && s != "false" && s != "0",
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::Object(m) => !m.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Null => false,
    }
}

/// A place text: control characters removed, trimmed, cut to 64 characters; empty is `None`.
pub fn clean_place_text(s: &str) -> Option<String> {
    let stripped: String = s.chars().filter(|c| !c.is_control()).collect();
    let t: String = stripped.trim().chars().take(PLACE_MAX_CHARS).collect();
    (!t.is_empty()).then_some(t)
}

/// The text of a provider failure message: control characters removed, every secret replaced by `***`, trimmed, cut to
/// 120 characters; empty is `None`.
fn clean_message(s: &str, secrets: &[&str]) -> Option<String> {
    let stripped: String = s.chars().filter(|c| !c.is_control()).collect();
    let redacted = redact(&stripped, secrets);
    let t: String = redacted.trim().chars().take(MESSAGE_MAX_CHARS).collect();
    (!t.is_empty()).then_some(t)
}

/// Replaces every occurrence of each secret (non-empty) by `***`.
pub fn redact(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_string();
    for s in secrets {
        if !s.is_empty() {
            out = out.replace(s, "***");
        }
    }
    out
}

fn provider_fail(json: &Value, m: &Mapping, secrets: &[&str]) -> Failure {
    let message = (!m.message_path.is_empty())
        .then(|| value_at(json, &m.message_path))
        .flatten()
        .and_then(text_of)
        .and_then(|t| clean_message(&t, secrets));
    Failure {
        code: FailureCode::ProviderFail,
        status: None,
        message,
    }
}

/// Reads a provider answer (a JSON object): the error flag, the success check, then the four place fields. `secrets` are
/// the texts (the key, its percent-encoded form) that must never appear in a failure message.
pub fn interpret(json: &Value, m: &Mapping, secrets: &[&str]) -> Result<GeoPlace, Failure> {
    if !m.error_path.is_empty() && value_at(json, &m.error_path).is_some_and(truthy) {
        return Err(provider_fail(json, m, secrets));
    }
    if !m.ok_path.is_empty() {
        let got = value_at(json, &m.ok_path).and_then(text_of);
        if got.as_deref().map(str::trim) != Some(m.ok_value.trim()) {
            return Err(provider_fail(json, m, secrets));
        }
    }
    let pick = |path: &str| -> Option<String> {
        if path.is_empty() {
            return None;
        }
        value_at(json, path)
            .and_then(text_of)
            .and_then(|t| clean_place_text(&t))
    };
    let place = GeoPlace {
        country: pick(&m.country),
        country_code: pick(&m.country_code),
        region: pick(&m.region),
        city: pick(&m.city),
    };
    if place.is_empty() {
        return Err(Failure::code(FailureCode::NoFields));
    }
    Ok(place)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(country: &str, code: &str, region: &str, city: &str) -> Mapping {
        Mapping {
            country: country.into(),
            country_code: code.into(),
            region: region.into(),
            city: city.into(),
            ..Mapping::default()
        }
    }

    #[test]
    fn path_syntax() {
        for ok in ["a", "a.b", "a-b_c.D9", "0", "a.0.b", "a.b.c.d.e.f.g.h"] {
            assert!(valid_path(ok), "{ok}");
        }
        assert!(valid_path(&"x".repeat(64)));
        for bad in [
            "",
            ".",
            "a.",
            ".a",
            "a..b",
            "a b",
            "a/b",
            "a[0]",
            "a$",
            "é",
            "a.b.c.d.e.f.g.h.i",
        ] {
            assert!(!valid_path(bad), "{bad:?}");
        }
        assert!(!valid_path(&"x".repeat(65)));
    }

    #[test]
    fn value_at_walks_objects_and_arrays() {
        let j = json!({"a": {"b": {"c": "deep"}}, "list": [{"x": "zero"}, {"x": "one"}], "n": 7, "digits": {"0": "key"}});
        assert_eq!(value_at(&j, "a.b.c"), Some(&json!("deep")));
        assert_eq!(value_at(&j, "list.1.x"), Some(&json!("one")));
        assert_eq!(value_at(&j, "list.2.x"), None);
        assert_eq!(value_at(&j, "list.x"), None);
        // an all-digit segment on an object is a key
        assert_eq!(value_at(&j, "digits.0"), Some(&json!("key")));
        assert_eq!(value_at(&j, "a.missing"), None);
        assert_eq!(value_at(&j, "n.x"), None);
        assert_eq!(value_at(&j, "a.b.c.d"), None);
    }

    #[test]
    fn text_and_truthy_tables() {
        assert_eq!(text_of(&json!("x")).as_deref(), Some("x"));
        assert_eq!(text_of(&json!(12)).as_deref(), Some("12"));
        assert_eq!(text_of(&json!(1.5)).as_deref(), Some("1.5"));
        assert_eq!(text_of(&json!(true)).as_deref(), Some("true"));
        assert_eq!(text_of(&json!(false)).as_deref(), Some("false"));
        for none in [json!(null), json!({"a": 1}), json!([1])] {
            assert_eq!(text_of(&none), None, "{none}");
        }
        for (v, want) in [
            (json!(true), true),
            (json!(false), false),
            (json!("yes"), true),
            (json!(""), false),
            (json!("false"), false),
            (json!("0"), false),
            (json!("00"), true),
            (json!(1), true),
            (json!(-3), true),
            (json!(0), false),
            (json!(0.0), false),
            (json!({"a": 1}), true),
            (json!({}), false),
            (json!([0]), true),
            (json!([]), false),
            (json!(null), false),
        ] {
            assert_eq!(truthy(&v), want, "{v}");
        }
    }

    #[test]
    fn place_text_is_cleaned_and_cut() {
        assert_eq!(
            clean_place_text("  Example City \u{7}\n").as_deref(),
            Some("Example City")
        );
        assert_eq!(clean_place_text(" \t\n"), None);
        assert_eq!(clean_place_text(""), None);
        let long = "é".repeat(80);
        let cut = clean_place_text(&long).expect("text");
        assert_eq!(cut.chars().count(), 64);
    }

    #[test]
    fn a_plain_answer_maps_to_a_place() {
        let j = json!({"country": "ZZ", "region": "Example Region", "city": "Example City", "extra": 1});
        let p = interpret(&j, &map("", "country", "region", "city"), &[]).expect("place");
        assert_eq!(p.country, None);
        assert_eq!(p.country_code.as_deref(), Some("ZZ"));
        assert_eq!(p.region.as_deref(), Some("Example Region"));
        assert_eq!(p.city.as_deref(), Some("Example City"));
    }

    #[test]
    fn a_missing_or_non_scalar_field_is_none_and_all_none_is_no_fields() {
        let j = json!({"city": {"name": "x"}, "region": null, "country": ""});
        let e =
            interpret(&j, &map("country", "countryCode", "region", "city"), &[]).expect_err("none");
        assert_eq!(e.code, FailureCode::NoFields);
        let j = json!({"city": "Example City"});
        let p = interpret(&j, &map("country", "countryCode", "region", "city"), &[])
            .expect("one field is enough");
        assert_eq!(p.city.as_deref(), Some("Example City"));
        assert_eq!(p.country, None);
    }

    #[test]
    fn the_success_check_compares_trimmed_text() {
        let mut m = map("country", "", "regionName", "city");
        m.ok_path = "status".into();
        m.ok_value = "success".into();
        m.message_path = "message".into();
        let good = json!({"status": "success", "country": "Exampleland", "city": "Example City"});
        assert!(interpret(&good, &m, &[]).is_ok());
        let fail = json!({"status": "fail", "message": "reserved range"});
        let e = interpret(&fail, &m, &[]).expect_err("fail");
        assert_eq!(e.code, FailureCode::ProviderFail);
        assert_eq!(e.message.as_deref(), Some("reserved range"));
        // case-sensitive, and a missing field counts as different
        assert!(interpret(&json!({"status": "Success", "city": "x"}), &m, &[]).is_err());
        assert!(interpret(&json!({"city": "x"}), &m, &[]).is_err());
        // a boolean success flag
        let mut m2 = map("country", "", "region", "city");
        m2.ok_path = "success".into();
        m2.ok_value = "true".into();
        assert!(interpret(&json!({"success": true, "city": "Example City"}), &m2, &[]).is_ok());
        assert!(interpret(
            &json!({"success": false, "message": "x", "city": "Example City"}),
            &m2,
            &[]
        )
        .is_err());
    }

    #[test]
    fn the_error_flag_wins_and_takes_its_message() {
        let mut m = map("country_name", "country_code", "region", "city");
        m.error_path = "error".into();
        m.message_path = "reason".into();
        let e =
            interpret(&json!({"error": true, "reason": "Rate limited"}), &m, &[]).expect_err("err");
        assert_eq!(e.code, FailureCode::ProviderFail);
        assert_eq!(e.message.as_deref(), Some("Rate limited"));
        // an error object is truthy; the message can be nested
        let mut m = map("", "country", "region", "city");
        m.error_path = "error".into();
        m.message_path = "error.message".into();
        let e = interpret(
            &json!({"error": {"title": "Wrong", "message": "Invalid token"}}),
            &m,
            &[],
        )
        .expect_err("err");
        assert_eq!(e.message.as_deref(), Some("Invalid token"));
        // a falsy flag does not fail
        assert!(interpret(&json!({"error": false, "country": "ZZ"}), &m, &[]).is_ok());
        // no message path: no message
        m.message_path.clear();
        let e = interpret(&json!({"error": "x", "country": "ZZ"}), &m, &[]).expect_err("err");
        assert_eq!(e.message, None);
    }

    #[test]
    fn a_failure_message_is_cleaned_cut_and_never_holds_the_key() {
        let mut m = map("", "country", "", "");
        m.error_path = "error".into();
        m.message_path = "message".into();
        let j =
            json!({"error": 1, "message": "bad token TESTKEY-geo-0123456789abcdef\u{1b}[31m here"});
        let e = interpret(&j, &m, &["TESTKEY-geo-0123456789abcdef"]).expect_err("fail");
        let msg = e.message.expect("message");
        assert!(!msg.contains("TESTKEY"), "{msg}");
        assert!(msg.contains("***"), "{msg}");
        assert!(!msg.chars().any(char::is_control));
        let long = json!({"error": 1, "message": "x".repeat(300)});
        let e = interpret(&long, &m, &[]).expect_err("fail");
        assert_eq!(e.message.expect("message").chars().count(), 120);
    }
}
