//! 与 Python `json.dumps(obj, ensure_ascii=False)` 一致的 JSON 写法：分隔符 `", "` / `": "`，非 ASCII 原样输出。
//! 用来写 `questions.data`、`attempts.sheets`，让 Rust 写出的库与 Python 写出的库字节一致。
use serde::Serialize;
use serde_json::ser::{Formatter, Serializer};
use serde_json::{Map, Value};
use std::io;

struct PyFormatter;

impl Formatter for PyFormatter {
    fn begin_array_value<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }
    fn begin_object_key<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }
    fn begin_object_value<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        writer.write_all(b": ")
    }
}

/// `json.dumps(v, ensure_ascii=False)`
pub fn to_string<T: Serialize + ?Sized>(v: &T) -> Result<String, serde_json::Error> {
    let mut buf = Vec::new();
    {
        let mut ser = Serializer::with_formatter(&mut buf, PyFormatter);
        v.serialize(&mut ser)?;
    }
    String::from_utf8(buf).map_err(|_| {
        <serde_json::Error as serde::ser::Error>::custom("invalid utf-8 in json output")
    })
}

/// 递归地把对象的键排序（按码点，与 Python `sort_keys=True` 一致）。
pub fn sort_keys(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for k in keys {
                if let Some(x) = m.get(k) {
                    out.insert(k.clone(), sort_keys(x));
                }
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(sort_keys).collect()),
        other => other.clone(),
    }
}

/// `json.dumps(v, sort_keys=True, ensure_ascii=False)`，即旧版 `_q_hash` 的规范形式。
pub fn canonical(v: &Value) -> Result<String, serde_json::Error> {
    to_string(&sort_keys(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn python_separators_and_unicode() {
        let v = json!({"b": [1, 2, {"x": "日本"}], "a": "é\n\"q\"", "e": [], "o": {}});
        assert_eq!(
            to_string(&v).unwrap(),
            r#"{"b": [1, 2, {"x": "日本"}], "a": "é\n\"q\"", "e": [], "o": {}}"#
        );
    }

    #[test]
    fn canonical_sorts_keys_recursively() {
        let v = json!({"b": {"z": 1, "y": [ {"d": 1, "c": 2} ]}, "a": null});
        assert_eq!(
            canonical(&v).unwrap(),
            r#"{"a": null, "b": {"y": [{"c": 2, "d": 1}], "z": 1}}"#
        );
    }

    #[test]
    fn control_characters_use_python_escapes() {
        let v = json!("a\u{1}\u{8}\t\u{7f}");
        assert_eq!(to_string(&v).unwrap(), "\"a\\u0001\\b\\t\u{7f}\"");
    }
}
