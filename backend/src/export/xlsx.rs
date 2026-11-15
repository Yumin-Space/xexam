//! The Excel download of a bank: the six-column IMPORT layout (A English question, B Chinese question, C Japanese
//! question, D English answer + explanation, E Chinese, F Japanese), one worksheet per sheet of the bank, a header row on
//! top of each. The file imports again as it is: the cells are the inverse of the six-column parser
//! (`parser::question::parse_six_columns`), and a sheet name Excel accepts is kept exactly, so the question ids stay.
//!
//! What cannot round-trip is accepted: option or item text that itself looks like an option or number line, a hotspot
//! choice that contains `/ ( ) ; ,`, a note that contains parentheses, text that holds a recognised `【…】` heading, and a
//! sheet name that had to be changed for Excel (its questions then get new ids on a re-import).
use super::{xml_text, ExportBank, ExportError};
use crate::models::{Answer, Kind, Question, Zh};
use std::collections::HashSet;
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// The header row written at the top of every sheet. It makes the six-column detection work on every sheet, also on a
/// sheet whose questions have no Japanese text (C1 contains kana, E1/F1 contain an answer heading), and the parser
/// ignores it as a question (A1 is one line, D1 has no 【】 heading).
pub const HEADER: [&str; 6] = [
    "Question (English)",
    "中文题目",
    "日本語の問題",
    "Answer (English)",
    "【答案】和解析",
    "【正解】と解説",
];

/// Excel's limit of one cell, in UTF-16 code units.
const MAX_CELL_UNITS: usize = 32_767;
/// Excel's limit of a worksheet name, in UTF-16 code units.
const MAX_NAME_UNITS: usize = 31;

/// UTF-16 length of `s`.
fn units(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `s` cut at a character boundary to at most `max` UTF-16 code units.
fn cut_units(s: &str, max: usize) -> &str {
    let mut used = 0;
    for (i, c) in s.char_indices() {
        used += c.len_utf16();
        if used > max {
            return s.get(..i).unwrap_or("");
        }
    }
    s
}

// ------------------------------------------------------------------ Cells

/// The English answer text shared by columns D, E and F: the letters joined `", "` for a choice question, and
/// `1. {choice}; 2. {choice}` (the English choice at the stored index of each item) for a hotspot. A bad index gives `?`.
fn answer_text(q: &Question) -> String {
    match &q.answer {
        Answer::Letters(letters) => letters.join(", "),
        Answer::Indexes(idx) => q
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let choice = idx
                    .get(i)
                    .and_then(|&j| usize::try_from(j).ok())
                    .and_then(|j| it.choices.get(j))
                    .map_or("?", String::as_str);
                format!("{}. {choice}", i + 1)
            })
            .collect::<Vec<_>>()
            .join("; "),
    }
}

fn is_hotspot(q: &Question) -> bool {
    q.kind == Kind::Hotspot || (q.options.is_empty() && !q.items.is_empty())
}

/// Column A.
fn english_cell(q: &Question) -> String {
    if is_hotspot(q) {
        let mut a = format!("HOTSPOT\n{}", q.stem);
        for (i, it) in q.items.iter().enumerate() {
            let lines: Vec<&str> = it.text.split('\n').collect();
            let (last, extra) = lines.split_last().map_or(("", &[][..]), |(l, e)| (*l, e));
            for line in extra {
                a.push('\n');
                a.push_str(line);
            }
            a.push_str(&format!("\n{}. {last}", i + 1));
            if !it.choices.is_empty() {
                a.push('\t');
                a.push_str(&it.choices.join(" / "));
            }
        }
        a
    } else {
        let mut a = q.stem.clone();
        for o in &q.options {
            a.push_str(&format!("\n{}. {}", o.key, o.text));
        }
        a
    }
}

/// Column C: the Japanese question (empty without one).
fn japanese_cell(q: &Question) -> String {
    let Some(ja) = q.ja.as_ref() else {
        return String::new();
    };
    let mut c = ja.stem.clone();
    localized_lines(q, ja, &mut c);
    c
}

/// Appends the option / item lines of `t` (a language of the question) to `out`.
fn localized_lines(q: &Question, t: &Zh, out: &mut String) {
    if is_hotspot(q) {
        let Some(items) = t.items.as_ref() else {
            return;
        };
        for (i, text) in items.iter().enumerate() {
            out.push_str(&format!("\n{}. {text}", i + 1));
            if let Some(ch) = t.choices.as_ref().and_then(|c| c.get(i)) {
                if !ch.is_empty() {
                    out.push('\u{3000}');
                    out.push_str(&ch.join(" / "));
                }
            }
        }
    } else if let Some(options) = t.options.as_ref() {
        for (o, text) in q.options.iter().zip(options) {
            out.push_str(&format!("\n{}. {text}", o.key));
        }
    }
}

/// One answer column: `{answer heading}{ans}{note}` then the explanation and memory aid sections, each only when
/// non-empty. `parens` is the pair of brackets around the note.
fn answer_cell(
    headings: [&str; 3],
    parens: (&str, &str),
    ans: &str,
    note: &str,
    explanation: &str,
    mnemonic: &str,
) -> String {
    let mut s = format!("{}{ans}", headings[0]);
    if !note.is_empty() {
        s.push_str(&format!("{}{note}{}", parens.0, parens.1));
    }
    if !explanation.is_empty() {
        s.push_str(&format!("\n{}{explanation}", headings[1]));
    }
    if !mnemonic.is_empty() {
        s.push_str(&format!("\n{}{mnemonic}", headings[2]));
    }
    s
}

/// The six cells A-F of one question (the inverse of the six-column parser).
pub fn question_cells(q: &Question) -> [String; 6] {
    let ans = answer_text(q);
    let explain = q.explain.as_ref();
    let empty = String::new();
    let pick = |t: Option<&crate::models::ExplainText>| match t {
        Some(t) => (t.note.clone(), t.explanation.clone(), t.mnemonic.clone()),
        None => (empty.clone(), empty.clone(), empty.clone()),
    };
    let (en_note, mut en_expl, mut en_mn) = pick(explain.and_then(|e| e.en.as_ref()));
    if explain.is_none() {
        // Defensive: a question without per-language text keeps its compatibility copies in the English column
        en_expl = q.explanation.clone();
        en_mn = q.mnemonic.clone();
    }
    let (zh_note, zh_expl, zh_mn) = pick(explain.and_then(|e| e.zh.as_ref()));
    let (ja_note, ja_expl, ja_mn) = pick(explain.and_then(|e| e.ja.as_ref()));
    [
        english_cell(q),
        q.translation.clone(),
        japanese_cell(q),
        answer_cell(
            ["【Answer】", "【Explanation】", "【Memory aid】"],
            (" (", ")"),
            &ans,
            &en_note,
            &en_expl,
            &en_mn,
        ),
        answer_cell(
            ["【答案】", "【解析】", "【记忆口诀】"],
            ("（", "）"),
            &ans,
            &zh_note,
            &zh_expl,
            &zh_mn,
        ),
        answer_cell(
            ["【正解】", "【解説】", "【覚え方】"],
            ("（", "）"),
            &ans,
            &ja_note,
            &ja_expl,
            &ja_mn,
        ),
    ]
}

// ------------------------------------------------------------------ Sheet names

/// Whether Excel accepts `name` as a worksheet name as it is.
fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && units(name) <= MAX_NAME_UNITS
        && !name.starts_with('\'')
        && !name.ends_with('\'')
        && !name
            .chars()
            .any(|c| c.is_control() || matches!(c, ':' | '\\' | '/' | '?' | '*' | '[' | ']'))
}

/// `name` made acceptable to Excel (see [`sheet_names`]).
fn clean_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, ':' | '\\' | '/' | '?' | '*' | '[' | ']') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let t = replaced.trim_matches('\'');
    let t = cut_units(t, MAX_NAME_UNITS).trim_matches('\'');
    if t.is_empty() {
        "Sheet".to_string()
    } else {
        t.to_string()
    }
}

/// Excel-safe, unique worksheet names for `names` (same length, same order). A name Excel accepts is kept EXACTLY (so a
/// re-import keeps the question ids). Otherwise:
/// - `: \ / ? * [ ]` and control characters become `_`;
/// - a leading or trailing `'` is removed;
/// - the name is cut to 31 UTF-16 code units;
/// - an empty result becomes `Sheet`.
///
/// A name that equals an earlier one case-insensitively gets ` (2)`, ` (3)`, … (cut first so the result stays at most
/// 31 units).
pub fn sheet_names(names: &[String]) -> Vec<String> {
    let mut taken: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let base = if name_ok(name) {
            name.clone()
        } else {
            clean_name(name)
        };
        let mut chosen = base.clone();
        let mut n = 2;
        while taken.contains(&chosen.to_lowercase()) {
            let suffix = format!(" ({n})");
            let room = MAX_NAME_UNITS.saturating_sub(units(&suffix));
            chosen = format!("{}{suffix}", cut_units(&base, room));
            n += 1;
        }
        taken.insert(chosen.to_lowercase());
        out.push(chosen);
    }
    out
}

// ------------------------------------------------------------------ Workbook parts

const XML_DECL: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
const NS_MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const NS_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// The column letter of a zero-based index below 26.
fn col_letter(i: usize) -> char {
    char::from(b'A' + u8::try_from(i.min(25)).unwrap_or(25))
}

/// One cell, cut to Excel's limit (`cut` is set when a cell had to be shortened). Empty cells are left out.
fn cell_xml(col: usize, row: usize, text: &str, cut: &mut bool) -> String {
    if text.is_empty() {
        return String::new();
    }
    let text = if units(text) > MAX_CELL_UNITS {
        *cut = true;
        cut_units(text, MAX_CELL_UNITS)
    } else {
        text
    };
    format!(
        r#"<c r="{}{row}" t="inlineStr" s="1"><is><t xml:space="preserve">{}</t></is></c>"#,
        col_letter(col),
        xml_text(text)
    )
}

fn row_xml(row: usize, cells: &[String; 6], cut: &mut bool) -> String {
    let mut s = format!(r#"<row r="{row}">"#);
    for (i, text) in cells.iter().enumerate() {
        s.push_str(&cell_xml(i, row, text, cut));
    }
    s.push_str("</row>");
    s
}

fn sheet_xml(questions: &[Question], first: bool, cut: &mut bool) -> String {
    let mut s = String::new();
    s.push_str(XML_DECL);
    s.push_str(&format!(r#"<worksheet xmlns="{NS_MAIN}">"#));
    s.push_str(&format!(
        r#"<sheetViews><sheetView{} workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/><selection pane="bottomLeft"/></sheetView></sheetViews>"#,
        if first { r#" tabSelected="1""# } else { "" }
    ));
    s.push_str(
        r#"<cols><col min="1" max="3" width="60" customWidth="1"/><col min="4" max="6" width="50" customWidth="1"/></cols>"#,
    );
    s.push_str("<sheetData>");
    let header: [String; 6] = HEADER.map(str::to_string);
    s.push_str(&row_xml(1, &header, cut));
    for (i, q) in questions.iter().enumerate() {
        s.push_str(&row_xml(i + 2, &question_cells(q), cut));
    }
    s.push_str("</sheetData></worksheet>");
    s
}

const STYLES_XML: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
    r#"<fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>"#,
    r#"<fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>"#,
    r#"<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>"#,
    r#"<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>"#,
    r#"<cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>"#,
    r#"<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0" applyAlignment="1"><alignment wrapText="1" vertical="top"/></xf></cellXfs>"#,
    r#"<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>"#,
    r#"</styleSheet>"#
);

fn content_types(sheet_count: usize) -> String {
    let mut s = String::new();
    s.push_str(XML_DECL);
    s.push_str(r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#);
    s.push_str(r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#);
    s.push_str(r#"<Default Extension="xml" ContentType="application/xml"/>"#);
    s.push_str(r#"<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#);
    for n in 1..=sheet_count {
        s.push_str(&format!(
            r#"<Override PartName="/xl/worksheets/sheet{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
        ));
    }
    s.push_str(r#"<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#);
    s.push_str(r#"<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>"#);
    s.push_str(r#"<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>"#);
    s.push_str("</Types>");
    s
}

fn root_rels() -> String {
    format!(
        r#"{XML_DECL}<Relationships xmlns="{NS_PKG_REL}"><Relationship Id="rId1" Type="{NS_REL}/officeDocument" Target="xl/workbook.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId3" Type="{NS_REL}/extended-properties" Target="docProps/app.xml"/></Relationships>"#
    )
}

fn core_xml(title: &str) -> String {
    format!(
        r#"{XML_DECL}<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>{}</dc:title><dc:creator>Xexam</dc:creator></cp:coreProperties>"#,
        xml_text(title)
    )
}

const APP_XML: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Xexam</Application></Properties>"#
);

fn workbook_xml(names: &[String]) -> String {
    let mut s = format!(
        r#"{XML_DECL}<workbook xmlns="{NS_MAIN}" xmlns:r="{NS_REL}"><bookViews><workbookView/></bookViews><sheets>"#
    );
    for (i, name) in names.iter().enumerate() {
        let n = i + 1;
        s.push_str(&format!(
            r#"<sheet name="{}" sheetId="{n}" r:id="rId{n}"/>"#,
            xml_text(name)
        ));
    }
    s.push_str("</sheets></workbook>");
    s
}

fn workbook_rels(sheet_count: usize) -> String {
    let mut s = format!(r#"{XML_DECL}<Relationships xmlns="{NS_PKG_REL}">"#);
    for n in 1..=sheet_count {
        s.push_str(&format!(
            r#"<Relationship Id="rId{n}" Type="{NS_REL}/worksheet" Target="worksheets/sheet{n}.xml"/>"#
        ));
    }
    s.push_str(&format!(
        r#"<Relationship Id="rId{}" Type="{NS_REL}/styles" Target="styles.xml"/></Relationships>"#,
        sheet_count + 1
    ));
    s
}

/// The whole workbook.
pub fn write_xlsx(bank: &ExportBank) -> Result<Vec<u8>, ExportError> {
    let mut cut = false;
    // A bank without sheets still gets one worksheet (the header row only)
    let (names, sheets): (Vec<String>, Vec<&[Question]>) = if bank.sheets.is_empty() {
        (vec!["Sheet1".to_string()], vec![&[][..]])
    } else {
        (
            sheet_names(
                &bank
                    .sheets
                    .iter()
                    .map(|s| s.name.clone())
                    .collect::<Vec<_>>(),
            ),
            bank.sheets.iter().map(|s| s.questions.as_slice()).collect(),
        )
    };
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut put = |name: &str, body: &str| -> Result<(), ExportError> {
        zip.start_file(name, opts)?;
        zip.write_all(body.as_bytes())?;
        Ok(())
    };
    put("[Content_Types].xml", &content_types(names.len()))?;
    put("_rels/.rels", &root_rels())?;
    put("docProps/core.xml", &core_xml(&bank.name))?;
    put("docProps/app.xml", APP_XML)?;
    put("xl/workbook.xml", &workbook_xml(&names))?;
    put("xl/_rels/workbook.xml.rels", &workbook_rels(names.len()))?;
    put("xl/styles.xml", STYLES_XML)?;
    for (i, questions) in sheets.iter().enumerate() {
        put(
            &format!("xl/worksheets/sheet{}.xml", i + 1),
            &sheet_xml(questions, i == 0, &mut cut),
        )?;
    }
    if cut {
        tracing::warn!(
            "bank {:?}: a cell is longer than {MAX_CELL_UNITS} characters and was cut in the Excel download",
            bank.name
        );
    }
    Ok(zip.finish()?.into_inner())
}
