//! 旧版 `progress.json` 的读取与转换。语义对齐 Python 的 `int()` / 真值判断 / `json.dumps`，
//! 但**任何**畸形输入都只返回 `Err(说明)`，绝不 panic。
use super::pyjson;
use rusqlite::types::Value as SqlValue;
use serde_json::{Map, Value};
use std::path::Path;

/// 旧文件的大小上限（真实文件只有几十 KB）。
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct StatRow {
    pub id: String,
    pub attempts: i64,
    pub correct: i64,
    pub wrong: i64,
    pub streak: i64,
    pub last: Option<i64>,
    pub at: SqlValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttemptRow {
    pub at: SqlValue,
    pub sheets: String,
    pub mode: SqlValue,
    pub total: i64,
    pub correct: i64,
    pub percent: i64,
    pub seconds: i64,
}

/// 读文件（接受 UTF-8 BOM），要求顶层是含 `questions`（对象）和 `history`（数组）的对象。
pub fn load(path: &Path) -> Result<(Map<String, Value>, Vec<Value>), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("无法读取文件（{}）", e.kind()))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err("文件太大".to_string());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("无法读取文件（{}）", e.kind()))?;
    let body = bytes
        .strip_prefix(&[0xEF, 0xBB, 0xBF][..])
        .unwrap_or(&bytes);
    let text = std::str::from_utf8(body).map_err(|_| "文件不是有效的 UTF-8".to_string())?;
    let doc: Value = serde_json::from_str(text).map_err(|e| format!("不是有效的 JSON（{e}）"))?;
    let Value::Object(mut top) = doc else {
        return Err("最外层不是 JSON 对象".to_string());
    };
    let qs = top
        .remove("questions")
        .ok_or_else(|| "缺少 questions".to_string())?;
    let hist = top
        .remove("history")
        .ok_or_else(|| "缺少 history".to_string())?;
    match (qs, hist) {
        (Value::Object(q), Value::Array(h)) => Ok((q, h)),
        _ => Err("questions 必须是对象、history 必须是数组".to_string()),
    }
}

/// Python 的真值判断。
pub fn py_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_none_or(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

fn py_int_from_str(s: &str) -> Result<i64, String> {
    let t = s.trim_matches(is_py_space);
    let (neg, body) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let norm: String = body
        .chars()
        .map(|c| {
            if ('\u{ff10}'..='\u{ff19}').contains(&c) {
                char::from(b'0' + (c as u32 - 0xff10) as u8)
            } else {
                c
            }
        })
        .collect();
    let first_ok = norm.chars().next().is_some_and(|c| c.is_ascii_digit());
    let last_ok = norm.chars().next_back().is_some_and(|c| c.is_ascii_digit());
    if !first_ok
        || !last_ok
        || norm.contains("__")
        || !norm.chars().all(|c| c.is_ascii_digit() || c == '_')
    {
        return Err(format!("不是整数：{s:?}"));
    }
    let digits: String = norm.chars().filter(|c| *c != '_').collect();
    let signed = if neg { format!("-{digits}") } else { digits };
    signed.parse::<i64>().map_err(|_| "整数太大".to_string())
}

/// Python 的 `int(x)`（落在 SQLite 的 64 位整数范围内才算成功）。
pub fn py_int(v: &Value) -> Result<i64, String> {
    match v {
        Value::Bool(b) => Ok(i64::from(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i)
            } else if n.as_u64().is_some() {
                Err("整数太大".to_string())
            } else {
                match n.as_f64() {
                    Some(f) if f.is_finite() && f.abs() < 9.2e18 => Ok(f.trunc() as i64),
                    _ => Err("数值超出范围".to_string()),
                }
            }
        }
        Value::String(s) => py_int_from_str(s),
        Value::Null => Err("int() 的参数不能是 null".to_string()),
        Value::Array(_) | Value::Object(_) => Err("int() 的参数不能是数组或对象".to_string()),
    }
}

/// 绑定到 SQLite 的值（Python `sqlite3` 能绑定的类型）。
pub fn to_sql(v: &Value) -> Result<SqlValue, String> {
    match v {
        Value::Null => Ok(SqlValue::Null),
        Value::Bool(b) => Ok(SqlValue::Integer(i64::from(*b))),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(SqlValue::Integer(i))
            } else if let Some(f) = n.as_f64().filter(|_| n.as_u64().is_none()) {
                Ok(SqlValue::Real(f))
            } else {
                Err("整数太大".to_string())
            }
        }
        Value::String(s) => Ok(SqlValue::Text(s.clone())),
        Value::Array(_) | Value::Object(_) => Err("数组或对象不能当作文本存入".to_string()),
    }
}

fn as_object<'a>(v: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    v.as_object().ok_or_else(|| format!("{what} 不是对象"))
}

/// 校验并转换。成功后才会写库，所以失败时库里一个字都不会变。
pub fn convert(
    qs: &Map<String, Value>,
    hist: &[Value],
) -> Result<(Vec<StatRow>, Vec<AttemptRow>), String> {
    let mut stats = Vec::with_capacity(qs.len());
    for (qid, r) in qs {
        let r = as_object(r, &format!("questions[{qid:?}]"))?;
        let num = |k: &str| -> Result<i64, String> {
            match r.get(k) {
                None => Ok(0),
                Some(v) => py_int(v).map_err(|e| format!("questions[{qid:?}].{k}：{e}")),
            }
        };
        let at = match r.get("at") {
            None => SqlValue::Null,
            Some(v) => to_sql(v).map_err(|e| format!("questions[{qid:?}].at：{e}"))?,
        };
        stats.push(StatRow {
            id: qid.clone(),
            attempts: num("attempts")?,
            correct: num("correct")?,
            wrong: num("wrong")?,
            streak: num("streak")?,
            last: r.get("last").map(|v| i64::from(py_truthy(v))),
            at,
        });
    }
    // 旧文件最新的在前；这里按"从旧到新"的顺序排好，写入时 id 递增就和时间一致
    let mut attempts = Vec::with_capacity(hist.len());
    for (i, h) in hist.iter().enumerate().rev() {
        let h = as_object(h, &format!("history[{i}]"))?;
        let need = |k: &str| -> Result<i64, String> {
            match h.get(k) {
                None => Err(format!("history[{i}] 缺少 {k}")),
                Some(v) => py_int(v).map_err(|e| format!("history[{i}].{k}：{e}")),
            }
        };
        let at = match h.get("at") {
            None => SqlValue::Text(String::new()),
            Some(Value::Null) => return Err(format!("history[{i}].at 不能是 null")),
            Some(v) => to_sql(v).map_err(|e| format!("history[{i}].at：{e}"))?,
        };
        let mode = match h.get("mode") {
            None => SqlValue::Text("all".to_string()),
            Some(Value::Null) => return Err(format!("history[{i}].mode 不能是 null")),
            Some(v) => to_sql(v).map_err(|e| format!("history[{i}].mode：{e}"))?,
        };
        let sheets = match h.get("sheets") {
            None => "[]".to_string(),
            Some(v) => pyjson::to_string(v).map_err(|e| format!("history[{i}].sheets：{e}"))?,
        };
        let seconds = match h.get("seconds") {
            None => 0,
            Some(v) => py_int(v).map_err(|e| format!("history[{i}].seconds：{e}"))?,
        };
        attempts.push(AttemptRow {
            at,
            sheets,
            mode,
            total: need("total")?,
            correct: need("correct")?,
            percent: need("percent")?,
            seconds,
        });
    }
    Ok((stats, attempts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn int_semantics_match_python() {
        assert_eq!(py_int(&json!(5)).unwrap(), 5);
        assert_eq!(py_int(&json!(3.7)).unwrap(), 3);
        assert_eq!(py_int(&json!(-3.7)).unwrap(), -3);
        assert_eq!(py_int(&json!(true)).unwrap(), 1);
        assert_eq!(py_int(&json!(" 12 ")).unwrap(), 12);
        assert_eq!(py_int(&json!("+1_000")).unwrap(), 1000);
        assert_eq!(py_int(&json!("１２")).unwrap(), 12);
        for bad in [
            json!(null),
            json!([]),
            json!({}),
            json!(""),
            json!("1.5"),
            json!("1__0"),
            json!("_1"),
            json!("1_"),
            json!("abc"),
            json!(1e30),
            json!(u64::MAX),
        ] {
            assert!(py_int(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn truthiness_matches_python() {
        for f in [
            json!(null),
            json!(false),
            json!(0),
            json!(0.0),
            json!(""),
            json!([]),
            json!({}),
        ] {
            assert!(!py_truthy(&f), "{f}");
        }
        for t in [
            json!(true),
            json!(1),
            json!(-1),
            json!(0.5),
            json!("0"),
            json!([0]),
            json!({"a": 1}),
        ] {
            assert!(py_truthy(&t), "{t}");
        }
    }

    #[test]
    fn convert_orders_history_oldest_first_and_defaults() {
        let doc = json!({
            "questions": {
                "a": {"attempts": 2, "correct": 1, "wrong": 1, "streak": 0, "last": false, "at": "2026-01-01 00:00"},
                "b": {}
            },
            "history": [
                {"at": "new", "sheets": ["x"], "total": 2, "correct": 1, "percent": 50},
                {"at": "old", "mode": "weak", "total": "3", "correct": 3, "percent": 100, "seconds": 9}
            ]
        });
        let (qs, hist) = (
            doc["questions"].as_object().unwrap(),
            doc["history"].as_array().unwrap(),
        );
        let (stats, attempts) = convert(qs, hist).unwrap();
        assert_eq!(stats.len(), 2);
        assert_eq!(stats[0].last, Some(0));
        assert_eq!(stats[1].last, None);
        assert_eq!(stats[1].at, SqlValue::Null);
        assert_eq!(attempts[0].at, SqlValue::Text("old".into()));
        assert_eq!(attempts[0].total, 3);
        assert_eq!(attempts[1].mode, SqlValue::Text("all".into()));
        assert_eq!(attempts[1].sheets, "[\"x\"]");
        assert_eq!(attempts[1].seconds, 0);
    }

    #[test]
    fn convert_rejects_malformed_shapes() {
        let bad_cases = [
            json!({"questions": {"a": 5}, "history": []}),
            json!({"questions": {"a": []}, "history": []}),
            json!({"questions": {"a": {"attempts": "x"}}, "history": []}),
            json!({"questions": {"a": {"attempts": null}}, "history": []}),
            json!({"questions": {"a": {"at": [1]}}, "history": []}),
            json!({"questions": {}, "history": [5]}),
            json!({"questions": {}, "history": [{"correct": 1, "percent": 1}]}),
            json!({"questions": {}, "history": [{"total": 1, "correct": 1, "percent": 1, "at": null}]}),
            json!({"questions": {}, "history": [{"total": 1, "correct": 1, "percent": 1, "mode": null}]}),
            json!({"questions": {}, "history": [{"total": 1e30, "correct": 1, "percent": 1}]}),
        ];
        for d in bad_cases {
            let (qs, hist) = (
                d["questions"].as_object().unwrap(),
                d["history"].as_array().unwrap(),
            );
            assert!(convert(qs, hist).is_err(), "{d}");
        }
    }

    #[test]
    fn load_strips_bom_and_checks_shape() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("p.json");
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(br#"{"questions": {}, "history": []}"#);
        std::fs::write(&p, &bytes).unwrap();
        assert!(load(&p).is_ok());
        let cases: [(&str, &[u8]); 9] = [
            ("a", b"\xff\xfe"),
            ("b", b"not json"),
            ("c", b"[]"),
            ("d", b"{\"questions\": {}}"),
            ("e", b"{\"history\": []}"),
            ("f", b"{\"questions\": [], \"history\": []}"),
            ("g", b"{\"questions\": {}, \"history\": {}}"),
            ("h", b""),
            ("i", b"null"),
        ];
        for (name, body) in cases {
            let q = dir.path().join(name);
            std::fs::write(&q, body).unwrap();
            assert!(load(&q).is_err(), "{name}");
        }
    }
}
