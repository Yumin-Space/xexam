//! Workbook layer: zip -> shared strings / worksheets -> rows and cells -> questions. Maps step by step to the legacy `read_workbook` / `_read_sheet` / `parse_workbook`.
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;

use crate::error::ImportError;
use crate::models::{ParsedSheet, ParsedWorkbook, Question, SkippedRow};

use super::py_space::{char_len, py_int, py_strip};
use super::question::{classify_heading, parse_question_with_english, QuestionOutcome};
use super::xml::{self, Element, XmlError};
use super::zip_guard::{OpenError, Package, ReadError};

const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const RNS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// Upper limits on shared-string entries and total cell count (not in the legacy version; required by the spec).
const MAX_SHARED_STRINGS: usize = 1_000_000;
const MAX_CELLS: usize = 2_000_000;
const MAX_SHEETS: usize = 1_000;
/// Upper limit on the byte count of all A/B/C cell text in one workbook (not in the legacy version). A shared string referenced by many cells is stored only once (`Rc<str>`),
/// so this budget counts "bytes referenced by each cell" even when only one copy is stored: it bounds the total amount the parse stage has to process.
const MAX_CELL_BYTES: usize = 64 * 1024 * 1024;

const HEADER_WORDS: [&str; 5] = ["题目", "題目", "问题", "問題", "Question"];

const MSG_NOT_XLSX: &str = "这个文件不是有效的 .xlsx（可能是 .xls、.csv，或者文件已损坏）。请在 Excel 里另存为「Excel 工作簿(.xlsx)」再上传。";
const MSG_TOO_COMPLEX: &str = "这个文件的内容太多或层级太深，已拒绝。";

/// One row in a worksheet: row number, the four columns A/B/C/D (the old three-column format uses only the first three), and whether the row ever had a non-blank cell (in any column).
struct SheetRow {
    number: i64,
    cols: [Option<Rc<str>>; 4],
}

fn xml_err(e: XmlError) -> ImportError {
    match e {
        XmlError::Malformed => ImportError::Message("文件里的 XML 读不了：格式不正确".to_string()),
        XmlError::Doctype => ImportError::EntityDeclaration,
        XmlError::TooComplex => ImportError::Message(MSG_TOO_COMPLEX.to_string()),
        XmlError::Timeout => ImportError::Timeout,
    }
}

/// Cooperative deadline: checked once per batch inside loops; returns `ImportError::Timeout` when it is reached.
fn check_deadline(deadline: Option<Instant>) -> Result<(), ImportError> {
    if deadline.is_some_and(|d| Instant::now() >= d) {
        Err(ImportError::Timeout)
    } else {
        Ok(())
    }
}

fn read_err(name: &str, e: ReadError) -> ImportError {
    match e {
        ReadError::Missing => ImportError::Message(format!(
            "这个文件缺少 .xlsx 必需的内容（\"There is no item named '{name}' in the archive\"），可能已损坏。"
        )),
        ReadError::TooBig => ImportError::TooBigUncompressed,
        ReadError::Corrupt => ImportError::Corrupt,
    }
}

fn parse_part(
    pkg: &mut Package<'_>,
    name: &str,
    deadline: Option<Instant>,
) -> Result<Element, ImportError> {
    check_deadline(deadline)?;
    let bytes = pkg.read(name).map_err(|e| read_err(name, e))?;
    xml::parse_until(&bytes, deadline).map_err(xml_err)
}

/// `_si_text`: collects only `<t>` and the `<t>` inside `<r>`; ignores `<rPh>`.
fn si_text(si: &Element) -> String {
    let mut out = String::new();
    for child in &si.children {
        if child.is(NS, "t") {
            out.push_str(child.text.as_deref().unwrap_or(""));
        } else if child.is(NS, "r") {
            for t in &child.children {
                if t.is(NS, "t") {
                    out.push_str(t.text.as_deref().unwrap_or(""));
                }
            }
        }
    }
    out
}

/// `_col_index`: the leading letter string of a reference -> 0-based column index. Returns None if there is no letter prefix. Values saturate (only columns 0/1/2/3 are meaningful).
fn col_index(reference: &str) -> Option<u64> {
    let mut n: u64 = 0;
    let mut any = false;
    for ch in reference.chars() {
        if !ch.is_ascii_alphabetic() {
            break;
        }
        any = true;
        n = n
            .saturating_mul(26)
            .saturating_add(u64::from(ch.to_ascii_uppercase() as u8 - b'A' + 1));
    }
    any.then(|| n.saturating_sub(1))
}

/// `_part_path`: if the target starts with `/`, strip all leading slashes; otherwise `posixpath.normpath('xl/' + target)`.
fn part_path(target: &str) -> String {
    if target.starts_with('/') {
        return target.trim_start_matches('/').to_string();
    }
    let mut stack: Vec<&str> = vec!["xl"];
    for comp in target.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                if stack.last().is_some_and(|l| *l != "..") {
                    stack.pop();
                } else {
                    stack.push("..");
                }
            }
            other => stack.push(other),
        }
    }
    if stack.is_empty() {
        ".".to_string()
    } else {
        stack.join("/")
    }
}

fn read_sheet(
    pkg: &mut Package<'_>,
    member: &str,
    shared: &[Rc<str>],
    cell_budget: &mut usize,
    byte_budget: &mut usize,
    deadline: Option<Instant>,
) -> Result<(Vec<SheetRow>, usize), ImportError> {
    let root = parse_part(pkg, member, deadline)?;
    let mut rows: Vec<SheetRow> = Vec::new();
    let mut nonempty_rows = 0usize;
    for (row_no, row) in root.iter_tag(NS, "row").into_iter().enumerate() {
        if row_no.is_multiple_of(256) {
            check_deadline(deadline)?;
        }
        let Some(number) = row
            .attr(None, "r")
            .and_then(py_int)
            .and_then(|p| p.to_i64())
        else {
            continue;
        };
        let mut cols: [Option<Rc<str>>; 4] = [None, None, None, None];
        let mut any = false;
        for c in row.find_all(NS, "c") {
            *cell_budget = cell_budget
                .checked_sub(1)
                .ok_or_else(|| ImportError::Message(MSG_TOO_COMPLEX.to_string()))?;
            let Some(col) = c.attr(None, "r").and_then(col_index) else {
                continue;
            };
            let t = c.attr(None, "t");
            let val: Rc<str> = if t == Some("inlineStr") {
                Rc::from(c.find(NS, "is").map(si_text).unwrap_or_default())
            } else {
                let Some(text) = c.find(NS, "v").and_then(|v| v.text.as_deref()) else {
                    continue;
                };
                match t {
                    Some("s") => {
                        // In the legacy version a negative index wrapped around like a Python subscript; here it is treated as malformed and the cell is dropped (a registered difference)
                        let idx = py_int(text)
                            .filter(|p| !p.negative)
                            .and_then(|p| p.digits.parse::<usize>().ok());
                        match idx.and_then(|i| shared.get(i)) {
                            Some(s) => Rc::clone(s),
                            None => continue,
                        }
                    }
                    Some("b") => Rc::from(if text == "1" { "TRUE" } else { "FALSE" }),
                    _ => Rc::from(text),
                }
            };
            if !py_strip(&val).is_empty() {
                any = true;
                if let Some(slot) = usize::try_from(col).ok().and_then(|i| cols.get_mut(i)) {
                    *byte_budget = byte_budget
                        .checked_sub(val.len())
                        .ok_or_else(|| ImportError::Message(MSG_TOO_COMPLEX.to_string()))?;
                    *slot = Some(val);
                }
            }
        }
        if any {
            nonempty_rows += 1;
            rows.push(SheetRow { number, cols });
        }
    }
    Ok((rows, nonempty_rows))
}

struct RawSheet {
    name: String,
    rows: Vec<SheetRow>,
    nonempty_rows: usize,
}

fn read_workbook(
    pkg: &mut Package<'_>,
    deadline: Option<Instant>,
) -> Result<Vec<RawSheet>, ImportError> {
    let mut shared: Vec<Rc<str>> = Vec::new();
    if pkg.contains("xl/sharedStrings.xml") {
        let sst = parse_part(pkg, "xl/sharedStrings.xml", deadline)?;
        for si in sst.find_all(NS, "si") {
            if shared.len() >= MAX_SHARED_STRINGS {
                return Err(ImportError::Message(MSG_TOO_COMPLEX.to_string()));
            }
            if shared.len().is_multiple_of(1024) {
                check_deadline(deadline)?;
            }
            shared.push(Rc::from(si_text(si)));
        }
    }
    let wb = parse_part(pkg, "xl/workbook.xml", deadline)?;
    let rels = parse_part(pkg, "xl/_rels/workbook.xml.rels", deadline)?;
    let mut target: HashMap<Option<String>, Option<String>> = HashMap::new();
    for r in rels.find_all(PKG, "Relationship") {
        target.insert(
            r.attr(None, "Id").map(str::to_string),
            r.attr(None, "Target").map(str::to_string),
        );
    }
    let mut out = Vec::new();
    let mut cell_budget = MAX_CELLS;
    let mut byte_budget = MAX_CELL_BYTES;
    let Some(sheets) = wb.find(NS, "sheets") else {
        return Ok(out);
    };
    for (n, s) in sheets.find_all(NS, "sheet").enumerate() {
        if n >= MAX_SHEETS {
            return Err(ImportError::Message(MSG_TOO_COMPLEX.to_string()));
        }
        if matches!(s.attr(None, "state"), Some("hidden" | "veryHidden")) {
            continue;
        }
        let rid = s.attr(Some(RNS), "id").map(str::to_string);
        let Some(Some(tgt)) = target.get(&rid) else {
            continue;
        };
        if tgt.is_empty() {
            continue;
        }
        let member = part_path(tgt);
        if !pkg.contains(&member) {
            continue;
        }
        let (rows, nonempty_rows) = read_sheet(
            pkg,
            &member,
            &shared,
            &mut cell_budget,
            &mut byte_budget,
            deadline,
        )?;
        out.push(RawSheet {
            name: s.attr(None, "name").unwrap_or("").to_string(),
            rows,
            nonempty_rows,
        });
    }
    Ok(out)
}

/// Is the worksheet in the four-column format (A English, B Chinese, C Japanese, D answer...)? Looks at content only, not the header.
///
/// All of the following must hold (to avoid misdetecting other layouts as four-column -- files the legacy version could not read must not suddenly become readable):
/// 1. More rows have an answer title (such as "【答案】") in column D than in column C;
/// 2. At least one row has Japanese kana in column C (C is the Japanese question) and none in column B (B is Chinese), and column A (English), if non-empty, has no kana.
///    Column A may be empty (this question has no English). Conversely, a file "shifted one column right" has A empty, B Japanese, C Chinese, D answer:
///    C has no kana, so condition 2 fails and the old rules still apply (the legacy version could not read it, and neither can the new one; behavior unchanged).
fn detect_four_columns(rows: &[SheetRow]) -> bool {
    fn text(row: &SheetRow, i: usize) -> &str {
        row.cols.get(i).and_then(|c| c.as_deref()).unwrap_or("")
    }
    let has_answer =
        |t: &str| t.contains("【答案】") || t.contains("【正解】") || t.contains("【答え】");
    let is_kana = |c: char| ('\u{3040}'..='\u{30ff}').contains(&c);
    let (mut in_c, mut in_d, mut shaped) = (0usize, 0usize, 0usize);
    for row in rows {
        if has_answer(text(row, 2)) {
            in_c += 1;
        }
        if has_answer(text(row, 3)) {
            in_d += 1;
            let (a, b, c) = (text(row, 0), text(row, 1), text(row, 2));
            if c.chars().any(is_kana) && !b.chars().any(is_kana) && !a.chars().any(is_kana) {
                shaped += 1;
            }
        }
    }
    in_d > 0 && in_d > in_c && shaped > 0
}

pub(super) fn parse_workbook(data: &[u8]) -> Result<ParsedWorkbook, ImportError> {
    parse_workbook_until(data, None)
}

/// Same as `parse_workbook`, but aborts and returns `ImportError::Timeout` once `deadline` is reached.
pub(super) fn parse_workbook_until(
    data: &[u8],
    deadline: Option<Instant>,
) -> Result<ParsedWorkbook, ImportError> {
    let mut pkg = Package::open(data).map_err(|e| match e {
        OpenError::NotZip => ImportError::Message(MSG_NOT_XLSX.to_string()),
        OpenError::Duplicate => ImportError::Corrupt,
        OpenError::TooManyEntries => ImportError::TooBigUncompressed,
    })?;
    let raw = read_workbook(&mut pkg, deadline)?;

    let mut sheets: Vec<ParsedSheet> = Vec::new();
    let mut questions: Vec<Question> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    // Next suffix to try for each base id: when an id repeats many times, do not re-probe from -2 each time (otherwise it is quadratic). Generated ids are the same as in the legacy version.
    let mut next_suffix: HashMap<String, u64> = HashMap::new();
    for sheet in raw {
        let mut info = ParsedSheet {
            name: sheet.name.clone(),
            total: 0,
            usable: 0,
            skipped: Vec::new(),
        };
        // Is this worksheet four-column (A English, B Chinese, C Japanese, D answer...) or the old three-column (A Japanese, B Chinese, C answer...)?
        let four_columns = detect_four_columns(&sheet.rows);
        for (row_no, row) in sheet.rows.iter().enumerate() {
            if row_no.is_multiple_of(256) {
                check_deadline(deadline)?;
            }
            let col = |i: usize| row.cols.get(i).and_then(|c| c.as_deref()).unwrap_or("");
            // Column holding the question (Japanese): C in the four-column format, A in the three-column format
            let (english, a, b, c) = if four_columns {
                (col(0), col(2), col(1), col(3))
            } else {
                ("", col(0), col(1), col(2))
            };
            let a_stripped = py_strip(a);
            if a_stripped.is_empty() {
                continue;
            }
            let a_len = char_len(a_stripped);
            if a_len < 15 && py_strip(b).is_empty() && py_strip(c).is_empty() {
                continue; // short text such as notes
            }
            if a_len <= 8 && (classify_heading(a).is_some() || HEADER_WORDS.contains(&a_stripped)) {
                continue; // header row
            }
            info.total += 1;
            match parse_question_with_english(&sheet.name, row.number, a, b, c, english) {
                QuestionOutcome::Fatal(e) => return Err(e),
                QuestionOutcome::Skip(reason) => info.skipped.push(SkippedRow {
                    row: row.number,
                    reason,
                }),
                QuestionOutcome::Ok(mut q) => {
                    if seen.contains(&q.id) {
                        let base = q.id.clone();
                        let n = next_suffix.entry(base.clone()).or_insert(2);
                        loop {
                            q.id = format!("{base}-{n}");
                            *n += 1;
                            if !seen.contains(&q.id) {
                                break;
                            }
                        }
                    }
                    seen.insert(q.id.clone());
                    questions.push(*q);
                    info.usable += 1;
                }
            }
        }
        if info.total > 0 || sheet.nonempty_rows == 0 {
            sheets.push(info);
        }
    }
    Ok(ParsedWorkbook { sheets, questions })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_paths_match_posixpath() {
        assert_eq!(
            part_path("worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(part_path("/xl/worksheets/a.xml"), "xl/worksheets/a.xml");
        assert_eq!(part_path("///a.xml"), "a.xml");
        assert_eq!(part_path("../xl/worksheets/x.xml"), "xl/worksheets/x.xml");
        assert_eq!(part_path("../../x"), "../x");
        assert_eq!(part_path("./a//b/./c.xml"), "xl/a/b/c.xml");
        assert_eq!(part_path(""), "xl");
        assert_eq!(part_path(".."), ".");
    }

    #[test]
    fn column_letters() {
        assert_eq!(col_index("A1"), Some(0));
        assert_eq!(col_index("c3"), Some(2));
        assert_eq!(col_index("AA7"), Some(26));
        assert_eq!(col_index("1A"), None);
        assert_eq!(col_index(""), None);
        assert_eq!(col_index(&"Z".repeat(200)), Some(u64::MAX - 1));
    }
}
