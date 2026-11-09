//! 工作簿层：zip -> 共享字符串 / 工作表 -> 行与单元格 -> 题目。逐步对应旧版 `read_workbook` / `_read_sheet` / `parse_workbook`。
use std::collections::{HashMap, HashSet};

use crate::error::ImportError;
use crate::models::{ParsedSheet, ParsedWorkbook, Question, SkippedRow};

use super::py_space::{char_len, py_int, py_strip};
use super::question::{classify_heading, parse_question, QuestionOutcome};
use super::xml::{self, Element, XmlError};
use super::zip_guard::{OpenError, Package, ReadError};

const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const RNS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// 共享字符串条目上限、单元格总数上限（旧版没有，规格要求）。
const MAX_SHARED_STRINGS: usize = 1_000_000;
const MAX_CELLS: usize = 2_000_000;
const MAX_SHEETS: usize = 1_000;

const HEADER_WORDS: [&str; 5] = ["题目", "題目", "问题", "問題", "Question"];

const MSG_NOT_XLSX: &str = "这个文件不是有效的 .xlsx（可能是 .xls、.csv，或者文件已损坏）。请在 Excel 里另存为「Excel 工作簿(.xlsx)」再上传。";
const MSG_TOO_COMPLEX: &str = "这个文件的内容太多或层级太深，已拒绝。";

/// 一个工作表里的一行：行号、A/B/C 三列、这一行是否有过任何非空白单元格（任何列）。
struct SheetRow {
    number: i64,
    cols: [Option<String>; 3],
}

fn xml_err(e: XmlError) -> ImportError {
    match e {
        XmlError::Malformed => ImportError::Message("文件里的 XML 读不了：格式不正确".to_string()),
        XmlError::Doctype => ImportError::EntityDeclaration,
        XmlError::TooComplex => ImportError::Message(MSG_TOO_COMPLEX.to_string()),
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

fn parse_part(pkg: &mut Package<'_>, name: &str) -> Result<Element, ImportError> {
    let bytes = pkg.read(name).map_err(|e| read_err(name, e))?;
    xml::parse(&bytes).map_err(xml_err)
}

/// `_si_text`：只收集 `<t>` 和 `<r>` 里的 `<t>`，忽略 `<rPh>`。
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

/// `_col_index`：引用开头的字母串 -> 0 基列号。没有字母前缀返回 None。数值饱和（只有 0/1/2 列有意义）。
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

/// `_part_path`：以 `/` 开头去掉所有前导斜杠；否则 `posixpath.normpath('xl/' + target)`。
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
    shared: &[String],
    cell_budget: &mut usize,
) -> Result<(Vec<SheetRow>, usize), ImportError> {
    let root = parse_part(pkg, member)?;
    let mut rows: Vec<SheetRow> = Vec::new();
    let mut nonempty_rows = 0usize;
    for row in root.iter_tag(NS, "row") {
        let Some(number) = row
            .attr(None, "r")
            .and_then(py_int)
            .and_then(|p| p.to_i64())
        else {
            continue;
        };
        let mut cols: [Option<String>; 3] = [None, None, None];
        let mut any = false;
        for c in row.find_all(NS, "c") {
            *cell_budget = cell_budget
                .checked_sub(1)
                .ok_or_else(|| ImportError::Message(MSG_TOO_COMPLEX.to_string()))?;
            let Some(col) = c.attr(None, "r").and_then(col_index) else {
                continue;
            };
            let t = c.attr(None, "t");
            let val: String = if t == Some("inlineStr") {
                c.find(NS, "is").map(si_text).unwrap_or_default()
            } else {
                let Some(text) = c.find(NS, "v").and_then(|v| v.text.as_deref()) else {
                    continue;
                };
                match t {
                    Some("s") => {
                        // 负索引在旧版会按 Python 下标回绕；这里视为畸形并丢弃该单元格（已登记的差异）
                        let idx = py_int(text)
                            .filter(|p| !p.negative)
                            .and_then(|p| p.digits.parse::<usize>().ok());
                        match idx.and_then(|i| shared.get(i)) {
                            Some(s) => s.clone(),
                            None => continue,
                        }
                    }
                    Some("b") => (if text == "1" { "TRUE" } else { "FALSE" }).to_string(),
                    _ => text.to_string(),
                }
            };
            if !py_strip(&val).is_empty() {
                any = true;
                if let Some(slot) = usize::try_from(col).ok().and_then(|i| cols.get_mut(i)) {
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

fn read_workbook(pkg: &mut Package<'_>) -> Result<Vec<RawSheet>, ImportError> {
    let mut shared: Vec<String> = Vec::new();
    if pkg.contains("xl/sharedStrings.xml") {
        let sst = parse_part(pkg, "xl/sharedStrings.xml")?;
        for si in sst.find_all(NS, "si") {
            if shared.len() >= MAX_SHARED_STRINGS {
                return Err(ImportError::Message(MSG_TOO_COMPLEX.to_string()));
            }
            shared.push(si_text(si));
        }
    }
    let wb = parse_part(pkg, "xl/workbook.xml")?;
    let rels = parse_part(pkg, "xl/_rels/workbook.xml.rels")?;
    let mut target: HashMap<Option<String>, Option<String>> = HashMap::new();
    for r in rels.find_all(PKG, "Relationship") {
        target.insert(
            r.attr(None, "Id").map(str::to_string),
            r.attr(None, "Target").map(str::to_string),
        );
    }
    let mut out = Vec::new();
    let mut cell_budget = MAX_CELLS;
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
        let (rows, nonempty_rows) = read_sheet(pkg, &member, &shared, &mut cell_budget)?;
        out.push(RawSheet {
            name: s.attr(None, "name").unwrap_or("").to_string(),
            rows,
            nonempty_rows,
        });
    }
    Ok(out)
}

pub(super) fn parse_workbook(data: &[u8]) -> Result<ParsedWorkbook, ImportError> {
    let mut pkg = Package::open(data).map_err(|e| match e {
        OpenError::NotZip => ImportError::Message(MSG_NOT_XLSX.to_string()),
        OpenError::Duplicate => ImportError::Corrupt,
        OpenError::TooManyEntries => ImportError::TooBigUncompressed,
    })?;
    let raw = read_workbook(&mut pkg)?;

    let mut sheets: Vec<ParsedSheet> = Vec::new();
    let mut questions: Vec<Question> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for sheet in raw {
        let mut info = ParsedSheet {
            name: sheet.name.clone(),
            total: 0,
            usable: 0,
            skipped: Vec::new(),
        };
        for row in &sheet.rows {
            let Some(a) = row.cols.first().and_then(|c| c.as_deref()) else {
                continue;
            };
            let a_stripped = py_strip(a);
            if a_stripped.is_empty() {
                continue;
            }
            let b = row.cols.get(1).and_then(|c| c.as_deref()).unwrap_or("");
            let c = row.cols.get(2).and_then(|c| c.as_deref()).unwrap_or("");
            let a_len = char_len(a_stripped);
            if a_len < 15 && py_strip(b).is_empty() && py_strip(c).is_empty() {
                continue; // 备注之类的短文字
            }
            if a_len <= 8 && (classify_heading(a).is_some() || HEADER_WORDS.contains(&a_stripped)) {
                continue; // 标题行
            }
            info.total += 1;
            match parse_question(&sheet.name, row.number, a, b, c) {
                QuestionOutcome::Fatal(e) => return Err(e),
                QuestionOutcome::Skip(reason) => info.skipped.push(SkippedRow {
                    row: row.number,
                    reason,
                }),
                QuestionOutcome::Ok(mut q) => {
                    let base = q.id.clone();
                    let mut n = 2u64;
                    while seen.contains(&q.id) {
                        q.id = format!("{base}-{n}");
                        n += 1;
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
