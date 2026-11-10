//! 工作簿层：zip -> 共享字符串 / 工作表 -> 行与单元格 -> 题目。逐步对应旧版 `read_workbook` / `_read_sheet` / `parse_workbook`。
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

/// 共享字符串条目上限、单元格总数上限（旧版没有，规格要求）。
const MAX_SHARED_STRINGS: usize = 1_000_000;
const MAX_CELLS: usize = 2_000_000;
const MAX_SHEETS: usize = 1_000;
/// 一个工作簿里所有 A/B/C 单元格文字的字节数上限（旧版没有）。共享字符串被很多单元格引用时只存一份（`Rc<str>`），
/// 所以这个预算按「每个单元格引用的字节数」计，哪怕只存了一份也要计入：它限制的是解析阶段要处理的总量。
const MAX_CELL_BYTES: usize = 64 * 1024 * 1024;

const HEADER_WORDS: [&str; 5] = ["题目", "題目", "问题", "問題", "Question"];

const MSG_NOT_XLSX: &str = "这个文件不是有效的 .xlsx（可能是 .xls、.csv，或者文件已损坏）。请在 Excel 里另存为「Excel 工作簿(.xlsx)」再上传。";
const MSG_TOO_COMPLEX: &str = "这个文件的内容太多或层级太深，已拒绝。";

/// 一个工作表里的一行：行号、A/B/C/D 四列（旧的三列格式只用前三列）、这一行是否有过任何非空白单元格（任何列）。
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

/// 协作式截止时间：循环里每隔一批检查一次；到了就返回 `ImportError::Timeout`。
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

/// `_col_index`：引用开头的字母串 -> 0 基列号。没有字母前缀返回 None。数值饱和（只有 0/1/2/3 列有意义）。
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
                        // 负索引在旧版会按 Python 下标回绕；这里视为畸形并丢弃该单元格（已登记的差异）
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

/// 工作表是不是四列格式（A 英语、B 中文、C 日语、D 答案…）？只看内容，不看表头。
///
/// 必须同时满足（用来避免把别的版式误判成四列 —— 旧版读不出来的文件不能因此突然读出来）：
/// 1. D 列里有答案标题（「【答案】」之类）的行，比 C 列里的多；
/// 2. 至少有一行，C 列**含日语假名**（C 是日语题目）、而 B 列**不含**（B 是中文），A 列（英语）若有内容则不含假名。
///    A 列可以是空的（这道题没写英语）。反过来，「整张表右移一列」的文件是 A 空、B 日语、C 中文、D 答案：
///    C 列没有假名，不满足第 2 条，仍按旧规则处理（旧版读不出来，新版也读不出来，行为不变）。
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

/// 同 `parse_workbook`，但在 `deadline` 到了之后中止并返回 `ImportError::Timeout`。
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
    // 每个基础 id 下一个要试的后缀：重复很多次时不必每次从 -2 重新探测（否则是平方级）。生成的 id 与旧版相同。
    let mut next_suffix: HashMap<String, u64> = HashMap::new();
    for sheet in raw {
        let mut info = ParsedSheet {
            name: sheet.name.clone(),
            total: 0,
            usable: 0,
            skipped: Vec::new(),
        };
        // 这个工作表是四列（A 英语、B 中文、C 日语、D 答案…）还是旧的三列（A 日语、B 中文、C 答案…）？
        let four_columns = detect_four_columns(&sheet.rows);
        for (row_no, row) in sheet.rows.iter().enumerate() {
            if row_no.is_multiple_of(256) {
                check_deadline(deadline)?;
            }
            let col = |i: usize| row.cols.get(i).and_then(|c| c.as_deref()).unwrap_or("");
            // 题目所在的列（日语）：四列格式是 C，三列格式是 A
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
                continue; // 备注之类的短文字
            }
            if a_len <= 8 && (classify_heading(a).is_some() || HEADER_WORDS.contains(&a_stripped)) {
                continue; // 标题行
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
