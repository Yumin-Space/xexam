//! CSV (RFC 4180: comma, `"` quoting with `""`, CRLF / LF, a quoted field may span lines) → one block per record (IX-3).
use super::textenc;
use super::{Collector, Doc, ExtractError, FileKind, LangHint, Limits};

/// Parses `text` into records of fields. Fields per record and characters per field are capped.
pub fn records(text: &str, limits: &Limits) -> Result<Vec<Vec<String>>, ExtractError> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut rec: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut field_chars = 0usize;
    let mut any = false;
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        any = true;
        if quoted {
            if c == '"' {
                if it.peek() == Some(&'"') {
                    it.next();
                    field.push('"');
                    field_chars += 1;
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
                field_chars += 1;
            }
            if field_chars > limits.csv_field_chars {
                return Err(ExtractError::TooComplex);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            ',' => {
                rec.push(std::mem::take(&mut field));
                field_chars = 0;
                if rec.len() > limits.csv_fields {
                    return Err(ExtractError::TooComplex);
                }
            }
            '\r' | '\n' => {
                if c == '\r' && it.peek() == Some(&'\n') {
                    it.next();
                }
                rec.push(std::mem::take(&mut field));
                field_chars = 0;
                out.push(std::mem::take(&mut rec));
                if out.len().is_multiple_of(1000) {
                    limits.check_deadline()?;
                }
                any = false;
            }
            c => {
                field.push(c);
                field_chars += 1;
                if field_chars > limits.csv_field_chars {
                    return Err(ExtractError::TooComplex);
                }
            }
        }
    }
    if any {
        rec.push(field);
        out.push(rec);
    }
    Ok(out)
}

pub fn extract(bytes: &[u8], hint: LangHint, limits: &Limits) -> Result<Doc, ExtractError> {
    let (text, enc) = textenc::decode(bytes, hint)?;
    let recs = records(&text, limits)?;
    let mut col = Collector::new(limits.max_chars);
    col.begin("");
    for r in recs {
        let fields: Vec<&str> = r
            .iter()
            .map(|f| f.trim())
            .filter(|f| !f.is_empty())
            .collect();
        if !fields.is_empty() {
            col.add(&fields.join(" | "))?;
        }
    }
    Ok(col.finish(FileKind::Csv, 0, Some(enc.to_string())))
}

#[cfg(test)]
#[allow(clippy::useless_vec)]
mod tests {
    use super::*;
    use crate::extract::testgen;

    #[test]
    fn quoting_doubled_quotes_and_newlines_in_fields() {
        let recs = records(&testgen::csv_tricky(), &Limits::default()).unwrap();
        assert_eq!(recs[0], vec!["id", "question", "note"]);
        assert_eq!(recs[1][1], "Which tool, if any, keeps a \"steady\" simmer?");
        assert_eq!(recs[2][2], "line one\nline two");
        assert_eq!(recs.len(), 3);
    }

    #[test]
    fn crlf_and_a_missing_final_newline() {
        let r = records("a,b\r\nc,d", &Limits::default()).unwrap();
        assert_eq!(r, vec![vec!["a", "b"], vec!["c", "d"]]);
        assert!(records("", &Limits::default()).unwrap().is_empty());
    }

    #[test]
    fn empty_fields_are_dropped_from_the_join() {
        let limits = Limits {
            min_meaningful: 0,
            ..Limits::default()
        };
        let doc = extract(b"alpha,,gamma\n,,\none,two,three", LangHint::En, &limits).unwrap();
        assert_eq!(
            doc.sections[0].blocks,
            vec!["alpha | gamma", "one | two | three"]
        );
    }

    #[test]
    fn field_caps() {
        let l = Limits::small();
        let wide = vec!["x"; 10].join(",");
        assert_eq!(records(&wide, &l), Err(ExtractError::TooComplex));
        let long = format!("\"{}\"", "y".repeat(201));
        assert_eq!(records(&long, &l), Err(ExtractError::TooComplex));
        assert!(records(&vec!["x"; 8].join(","), &l).is_ok());
    }
}
