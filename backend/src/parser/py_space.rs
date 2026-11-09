//! Python 3 `str` 的空白与十进制数字语义（旧版解析器依赖它们，id 也依赖它们）。
use super::digits_table::ZERO_STARTS;

/// Python `\s`（对 `str`）= `str.isspace()` 的 29 个码点。比 Unicode `White_Space`（25 个）多 U+001C..U+001F。
pub fn is_py_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}'..='\u{d}'
            | '\u{1c}'..='\u{1f}'
            | ' '
            | '\u{85}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

/// Python `str.strip()`
pub fn py_strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

/// Python `len(s)`（码点数）
pub fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// Python `s[:n]`（按码点）
pub fn take_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// 删除全部 Python 空白（`re.sub(r'\s+', '', s)`）
pub fn remove_py_space(s: &str) -> String {
    s.chars().filter(|c| !is_py_space(*c)).collect()
}

/// Unicode Nd 数字的数值（Python 3.10 / Unicode 13 的表）。Python `\d` 与 `int()` 接受的正是这些码点。
pub fn digit_value(c: char) -> Option<u8> {
    let cp = c as u32;
    let idx = match ZERO_STARTS.binary_search(&cp) {
        Ok(i) => i,
        Err(0) => return None,
        Err(i) => i - 1,
    };
    let start = *ZERO_STARTS.get(idx)?;
    let d = cp - start;
    if d < 10 {
        u8::try_from(d).ok()
    } else {
        None
    }
}

/// 供正则使用：全部 Nd 码点的区间（不含外层方括号）。
pub fn digit_class_inner() -> String {
    let mut out = String::new();
    for s in ZERO_STARTS {
        out.push_str(&format!("\\x{{{:X}}}-\\x{{{:X}}}", s, s + 9));
    }
    out
}

/// Python `int()` 能解析的十进制整数（规范化后的形式）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PyInt {
    pub negative: bool,
    /// ASCII 数字，无前导零（零自己是 "0"）
    pub digits: String,
}

impl PyInt {
    pub fn to_i64(&self) -> Option<i64> {
        if self.negative {
            let s = format!("-{}", self.digits);
            s.parse::<i64>().ok()
        } else {
            self.digits.parse::<i64>().ok()
        }
    }
}

/// Python 的 `int()` 对数字个数的上限（超过抛 ValueError）
pub const MAX_INT_DIGITS: usize = 4300;

/// 一段纯 Nd 数字 -> ASCII 数字串（去前导零）。超过 4300 位返回 `Err(())`；含非数字返回 `Ok(None)`。
#[allow(clippy::result_unit_err)]
pub fn digits_to_canonical(s: &str) -> Result<Option<String>, ()> {
    let mut out = String::new();
    let mut count = 0usize;
    for ch in s.chars() {
        let Some(v) = digit_value(ch) else {
            return Ok(None);
        };
        count += 1;
        if count > MAX_INT_DIGITS {
            return Err(());
        }
        if out.is_empty() && v == 0 {
            continue;
        }
        out.push(char::from(b'0' + v));
    }
    if out.is_empty() {
        out.push('0');
    }
    Ok(Some(out))
}

/// Python `int(str)`：首尾空白、正负号、数字间单个下划线、Unicode 十进制数字。失败（含超过 4300 位）返回 None。
pub fn py_int(s: &str) -> Option<PyInt> {
    let s = py_strip(s);
    let (negative, body) = match s.chars().next() {
        Some('+') => (false, s.get(1..)?),
        Some('-') => (true, s.get(1..)?),
        _ => (false, s),
    };
    let rest: Vec<char> = body.chars().collect();
    let first = rest.first()?;
    let last = rest.last()?;
    digit_value(*first)?;
    digit_value(*last)?;
    let mut digits = String::new();
    let mut prev_us = false;
    for c in &rest {
        if *c == '_' {
            if prev_us {
                return None;
            }
            prev_us = true;
        } else {
            digit_value(*c)?;
            prev_us = false;
            digits.push(*c);
        }
    }
    let canon = digits_to_canonical(&digits).ok()??;
    Some(PyInt {
        negative,
        digits: canon,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_29_code_points() {
        let all: Vec<u32> = (0..=0x10FFFFu32)
            .filter(|&cp| char::from_u32(cp).is_some_and(is_py_space))
            .collect();
        assert_eq!(all.len(), 29);
        assert_eq!(all[0], 9);
        assert!(all.contains(&0x1c) && all.contains(&0x85) && all.contains(&0x3000));
        for cp in [
            0x200B, 0xFEFF, 0x180E, 0x2800, 0x3164, 0xFFA0, 0xAD, 0x200C, 0x200D, 0x2060,
        ] {
            assert!(!is_py_space(char::from_u32(cp).unwrap()), "{cp:x}");
        }
    }

    #[test]
    fn nd_table_has_650_code_points() {
        let n = (0..=0x10FFFFu32)
            .filter(|&cp| char::from_u32(cp).is_some_and(|c| digit_value(c).is_some()))
            .count();
        assert_eq!(n, 650);
        assert_eq!(digit_value('７'), Some(7));
        assert_eq!(digit_value('٣'), Some(3));
        assert_eq!(digit_value('a'), None);
        assert_eq!(digit_value('0'), Some(0));
    }

    #[test]
    fn py_int_matches_python() {
        let v = |s: &str| py_int(s).and_then(|p| p.to_i64());
        assert_eq!(v("5"), Some(5));
        assert_eq!(v(" 5 "), Some(5));
        assert_eq!(v("+5"), Some(5));
        assert_eq!(v("-5"), Some(-5));
        assert_eq!(v("1_0"), Some(10));
        assert_eq!(v("１２"), Some(12));
        assert_eq!(v("٣"), Some(3));
        assert_eq!(v("007"), Some(7));
        for bad in [
            "", " ", "+", "-", "1__0", "_1", "1_", "1.0", "1e0", "+ 5", "5 5", "a", "０x",
        ] {
            assert_eq!(py_int(bad), None, "{bad:?}");
        }
        assert_eq!(v("9223372036854775807"), Some(i64::MAX));
        assert_eq!(v("-9223372036854775808"), Some(i64::MIN));
        assert_eq!(v("9223372036854775808"), None);
        assert_eq!(py_int(&"9".repeat(4301)), None);
        assert!(py_int(&"9".repeat(4300)).is_some());
    }

    #[test]
    fn helpers() {
        assert_eq!(py_strip("\u{1c} x\u{3000}"), "x");
        assert_eq!(take_chars("😀😀😀", 2), "😀😀");
        assert_eq!(remove_py_space(" a\u{3000}b\tc\n"), "abc");
        assert_eq!(char_len("e\u{301}"), 2);
    }
}
