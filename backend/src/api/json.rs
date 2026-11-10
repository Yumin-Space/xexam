//! JSON in and out, both following the behavior of Python's `json` module.
//!
//! - Out: `py_json` is byte-identical to `json.dumps(obj, ensure_ascii=False)` (separators `", "` / `": "`, non-ASCII kept as raw UTF-8,
//!   control characters escaped: `\n \r \t \b \f` in short form, the rest as `\u00xx` with lowercase hex, `/` not escaped).
//! - In: `parse_py_json` accepts forms that Python `json.loads` accepts but serde_json rejects: `NaN` / `Infinity` / `-Infinity`,
//!   UTF-8 BOM, huge integers, duplicate keys (the later one wins); depth overflow / invalid UTF-8 / other syntax errors all return `Err` (→ 400 bad json).
use serde::Serialize;
use serde_json::ser::Formatter;
use serde_json::{Serializer, Value};
use std::io;

pub struct PyFormatter;

impl Formatter for PyFormatter {
    fn begin_array_value<W>(&mut self, w: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        if first {
            Ok(())
        } else {
            w.write_all(b", ")
        }
    }
    fn begin_object_key<W>(&mut self, w: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        if first {
            Ok(())
        } else {
            w.write_all(b", ")
        }
    }
    fn begin_object_value<W>(&mut self, w: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        w.write_all(b": ")
    }
}

/// Byte-identical to Python `json.dumps(v, ensure_ascii=False).encode('utf-8')` (floats excluded).
pub fn py_json(v: &Value) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);
    let mut ser = Serializer::with_formatter(&mut buf, PyFormatter);
    if v.serialize(&mut ser).is_err() {
        return b"null".to_vec();
    }
    buf
}

// ---------------------------------------------------------------- Parsing

/// Python values (only the types that `clean_attempt` etc. need to distinguish are kept).
#[derive(Debug, Clone, PartialEq)]
pub enum PyVal {
    Null,
    Bool(bool),
    /// Saturates within the i128 range (Python has arbitrary precision; this is only used to clamp the range)
    Int(i128),
    Float(f64),
    Str(String),
    List(Vec<PyVal>),
    Dict(Vec<(String, PyVal)>),
}

impl PyVal {
    /// Python truthiness test (`x or default`).
    pub fn truthy(&self) -> bool {
        match self {
            PyVal::Null => false,
            PyVal::Bool(b) => *b,
            PyVal::Int(i) => *i != 0,
            PyVal::Float(f) => *f != 0.0,
            PyVal::Str(s) => !s.is_empty(),
            PyVal::List(l) => !l.is_empty(),
            PyVal::Dict(d) => !d.is_empty(),
        }
    }
    /// `dict.get(key)`: for duplicate keys the last one wins. Returns None if not a dict.
    pub fn get(&self, key: &str) -> Option<&PyVal> {
        match self {
            PyVal::Dict(d) => d.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn is_dict(&self) -> bool {
        matches!(self, PyVal::Dict(_))
    }
}

const MAX_DEPTH: usize = 512;
const MAX_INT_DIGITS: usize = 4300; // Python 3.10.7+ limit on integer string conversion

/// Parses the request body. Every `Err(())` corresponds to the legacy 400 `bad json`.
#[allow(clippy::result_unit_err)]
pub fn parse_py_json(bytes: &[u8]) -> Result<PyVal, ()> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| ())?;
    let mut p = Parser {
        s: text.as_bytes(),
        i: 0,
        depth: 0,
    };
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(());
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }
    fn lit(&mut self, word: &str) -> bool {
        let w = word.as_bytes();
        if self.s.get(self.i..self.i + w.len()) == Some(w) {
            self.i += w.len();
            true
        } else {
            false
        }
    }
    fn value(&mut self) -> Result<PyVal, ()> {
        self.ws();
        match self.peek().ok_or(())? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => Ok(PyVal::Str(self.string()?)),
            b'n' if self.lit("null") => Ok(PyVal::Null),
            b't' if self.lit("true") => Ok(PyVal::Bool(true)),
            b'f' if self.lit("false") => Ok(PyVal::Bool(false)),
            b'N' if self.lit("NaN") => Ok(PyVal::Float(f64::NAN)),
            b'I' if self.lit("Infinity") => Ok(PyVal::Float(f64::INFINITY)),
            b'-' if self.s.get(self.i..self.i + 9) == Some(b"-Infinity") => {
                self.i += 9;
                Ok(PyVal::Float(f64::NEG_INFINITY))
            }
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(()),
        }
    }
    fn digits(&mut self) -> usize {
        let start = self.i;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.i += 1;
        }
        self.i - start
    }
    fn number(&mut self) -> Result<PyVal, ()> {
        let start = self.i;
        let neg = self.peek() == Some(b'-');
        if neg {
            self.i += 1;
        }
        let int_digits = match self.peek() {
            Some(b'0') => {
                self.i += 1;
                1
            }
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err(()),
        };
        let mut is_float = false;
        if self.peek() == Some(b'.') && matches!(self.s.get(self.i + 1), Some(b'0'..=b'9')) {
            is_float = true;
            self.i += 1;
            self.digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let save = self.i;
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if self.digits() == 0 {
                self.i = save; // no digits after 'e': the number ends here, and the rest is reported as an error by the outer layer
            } else {
                is_float = true;
            }
        }
        let lexeme = std::str::from_utf8(self.s.get(start..self.i).ok_or(())?).map_err(|_| ())?;
        if is_float {
            return lexeme.parse::<f64>().map(PyVal::Float).map_err(|_| ());
        }
        if int_digits > MAX_INT_DIGITS {
            return Err(());
        }
        const CAP: i128 = 1_000_000_000_000_000_000_000_000_000_000;
        let mut v: i128 = 0;
        for b in lexeme.bytes().filter(u8::is_ascii_digit) {
            v = v
                .saturating_mul(10)
                .saturating_add((b - b'0') as i128)
                .min(CAP);
        }
        Ok(PyVal::Int(if neg { -v } else { v }))
    }
    fn hex4(&mut self) -> Result<u32, ()> {
        let h = self.s.get(self.i..self.i + 4).ok_or(())?;
        let mut v = 0u32;
        for &b in h {
            v = v * 16 + (b as char).to_digit(16).ok_or(())?;
        }
        self.i += 4;
        Ok(v)
    }
    fn string(&mut self) -> Result<String, ()> {
        self.i += 1; // the opening quote
        let mut out: Vec<u8> = Vec::new();
        loop {
            let b = self.peek().ok_or(())?;
            self.i += 1;
            match b {
                b'"' => break,
                b'\\' => {
                    let e = self.peek().ok_or(())?;
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp)
                                && self.s.get(self.i..self.i + 2) == Some(b"\\u")
                            {
                                let save = self.i;
                                self.i += 2;
                                match self.hex4() {
                                    Ok(lo) if (0xDC00..0xE000).contains(&lo) => {
                                        cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00)
                                    }
                                    _ => self.i = save,
                                }
                            }
                            let ch = char::from_u32(cp).unwrap_or('\u{FFFD}'); // lone surrogate: a Rust String cannot hold it, so replace it with U+FFFD
                            let mut tmp = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
                        }
                        _ => return Err(()),
                    }
                }
                0..=0x1f => return Err(()), // raw control characters are not allowed in a string
                _ => out.push(b),
            }
        }
        String::from_utf8(out).map_err(|_| ())
    }
    fn enter(&mut self) -> Result<(), ()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(());
        }
        Ok(())
    }
    fn array(&mut self) -> Result<PyVal, ()> {
        self.enter()?;
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
        } else {
            loop {
                items.push(self.value()?);
                self.ws();
                match self.peek() {
                    Some(b',') => self.i += 1,
                    Some(b']') => {
                        self.i += 1;
                        break;
                    }
                    _ => return Err(()),
                }
            }
        }
        self.depth -= 1;
        Ok(PyVal::List(items))
    }
    fn object(&mut self) -> Result<PyVal, ()> {
        self.enter()?;
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
        } else {
            loop {
                self.ws();
                if self.peek() != Some(b'"') {
                    return Err(());
                }
                let k = self.string()?;
                self.ws();
                if self.peek() != Some(b':') {
                    return Err(());
                }
                self.i += 1;
                let v = self.value()?;
                items.push((k, v));
                self.ws();
                match self.peek() {
                    Some(b',') => self.i += 1,
                    Some(b'}') => {
                        self.i += 1;
                        break;
                    }
                    _ => return Err(()),
                }
            }
        }
        self.depth -= 1;
        Ok(PyVal::Dict(items))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formatter_matches_python_spacing() {
        let v = json!({"a": 1, "b": [1, 2, {"c": null}], "d": "x", "e": {}, "f": []});
        assert_eq!(
            String::from_utf8(py_json(&v)).unwrap(),
            r#"{"a": 1, "b": [1, 2, {"c": null}], "d": "x", "e": {}, "f": []}"#
        );
    }

    #[test]
    fn escapes_like_python() {
        let v = json!("a\"b\\c/d\n\r\t\u{8}\u{c}\u{1}\u{1f}\u{7f}é中😀\u{2028}");
        let s = String::from_utf8(py_json(&v)).unwrap();
        assert_eq!(
            s,
            "\"a\\\"b\\\\c/d\\n\\r\\t\\b\\f\\u0001\\u001f\u{7f}é中😀\u{2028}\""
        );
    }

    #[test]
    fn parses_python_extensions() {
        assert!(matches!(
            parse_py_json(b"\xEF\xBB\xBF{}"),
            Ok(PyVal::Dict(_))
        ));
        let v = parse_py_json(br#"{"x": NaN, "y": Infinity, "z": -Infinity, "n": 12345678901234567890123456789012345}"#).unwrap();
        assert!(matches!(v.get("x"), Some(PyVal::Float(f)) if f.is_nan()));
        assert_eq!(v.get("y"), Some(&PyVal::Float(f64::INFINITY)));
        assert!(matches!(v.get("n"), Some(PyVal::Int(_))));
        // duplicate keys: the later one wins
        let d = parse_py_json(br#"{"a": 1, "a": 2}"#).unwrap();
        assert_eq!(d.get("a"), Some(&PyVal::Int(2)));
    }

    #[test]
    fn rejects_what_python_rejects() {
        for bad in [
            &b"{not json"[..],
            b"",
            b" ",
            b"[1,]",
            b"{\"a\":1,}",
            b"01",
            b"1 2",
            b"\"a\nb\"",
            b"{'a':1}",
            b"\xff\xfe",
            b"nul",
            b"-",
            b"1.",
            b"[1 2]",
        ] {
            assert!(
                parse_py_json(bad).is_err(),
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
        let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
        assert!(parse_py_json(deep.as_bytes()).is_err());
        let ok_depth = format!("{}{}", "[".repeat(100), "]".repeat(100));
        assert!(parse_py_json(ok_depth.as_bytes()).is_ok());
    }

    #[test]
    fn strings_and_escapes() {
        let v = parse_py_json(r#"["é😀\n\/", "令牌"]"#.as_bytes()).unwrap();
        assert_eq!(
            v,
            PyVal::List(vec![
                PyVal::Str("é😀\n/".to_string()),
                PyVal::Str("令牌".to_string())
            ])
        );
        // a lone surrogate does not crash
        assert!(parse_py_json(br#""\ud800""#).is_ok());
    }

    #[test]
    fn truthiness_matches_python() {
        for (v, t) in [
            (PyVal::Null, false),
            (PyVal::Bool(false), false),
            (PyVal::Int(0), false),
            (PyVal::Float(0.0), false),
            (PyVal::Str(String::new()), false),
            (PyVal::List(vec![]), false),
            (PyVal::Dict(vec![]), false),
            (PyVal::Float(f64::NAN), true),
            (PyVal::Int(-1), true),
            (PyVal::Str("0".into()), true),
        ] {
            assert_eq!(v.truthy(), t, "{v:?}");
        }
    }
}
