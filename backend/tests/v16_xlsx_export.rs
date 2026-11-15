//! v1.6: the Excel download of a bank. The file is in the six-column IMPORT layout, so the acceptance test is the round
//! trip: build a source workbook, parse it, export the questions, parse the export again and compare. Also the worksheet
//! names, the file name helpers and the `Content-Disposition` header.
//! Made-up sample questions only.
use std::io::{Cursor, Write};

use serde_json::Value;
use xexam_backend::export::xlsx::{question_cells, sheet_names, write_xlsx, HEADER};
use xexam_backend::export::{
    content_disposition, file_name, file_stem, ExportBank, ExportFormat, ExportSheet,
};
use xexam_backend::models::{ParsedWorkbook, Question};
use xexam_backend::parser::{check_xlsx, parse_workbook};

// ---------------------------------------------------------------- source workbook helpers

const RELS_HEAD: &str =
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#;

/// Like the XML escape of a writer that drops the characters XML 1.0 does not allow.
fn esc(s: &str) -> String {
    s.chars()
        .filter(|c| !(*c < ' ' && !matches!(c, '\t' | '\n' | '\r')))
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

type Row = Vec<Option<String>>;

fn six(cells: [&str; 6]) -> Row {
    cells
        .iter()
        .map(|c| (!c.is_empty()).then(|| c.to_string()))
        .collect()
}

fn header_row() -> Row {
    six(HEADER)
}

fn sheet_xml(rows: &[Row]) -> String {
    let mut sheet = String::new();
    for (i, cols) in rows.iter().enumerate() {
        let r = i + 1;
        sheet.push_str(&format!(r#"<row r="{r}">"#));
        for (k, text) in cols.iter().enumerate() {
            if let Some(t) = text {
                let letter = (b'A' + u8::try_from(k).expect("col")) as char;
                sheet.push_str(&format!(
                    r#"<c r="{letter}{r}" t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#,
                    esc(t)
                ));
            }
        }
        sheet.push_str("</row>");
    }
    format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{sheet}</sheetData></worksheet>"#
    )
}

/// A workbook with the given sheets (name, rows).
pub fn workbook(sheets: &[(&str, Vec<Row>)]) -> Vec<u8> {
    let mut wb = String::from(
        r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>"#,
    );
    let mut rels = String::from(RELS_HEAD);
    for (i, (name, _)) in sheets.iter().enumerate() {
        let n = i + 1;
        wb.push_str(&format!(
            r#"<sheet name="{}" sheetId="{n}" r:id="rId{n}"/>"#,
            esc(name)
        ));
        rels.push_str(&format!(
            r#"<Relationship Id="rId{n}" Target="worksheets/s{n}.xml"/>"#
        ));
    }
    wb.push_str("</sheets></workbook>");
    rels.push_str("</Relationships>");
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let mut put = |name: &str, body: &str| {
        w.start_file(name, opts).expect("start");
        w.write_all(body.as_bytes()).expect("write");
    };
    put("xl/workbook.xml", &wb);
    put("xl/_rels/workbook.xml.rels", &rels);
    for (i, (_, rows)) in sheets.iter().enumerate() {
        put(&format!("xl/worksheets/s{}.xml", i + 1), &sheet_xml(rows));
    }
    let mut out = w.finish().expect("finish").into_inner();
    if out.len() < 100 {
        out.resize(100, 0);
    }
    check_xlsx(&out).expect("valid source xlsx");
    out
}

/// The questions of a parsed workbook grouped by sheet, in the order the store would export them.
pub fn bank_of(name: &str, pw: &ParsedWorkbook) -> ExportBank {
    let sheets = pw
        .sheets
        .iter()
        .filter_map(|s| {
            let questions: Vec<Question> = pw
                .questions
                .iter()
                .filter(|q| q.sheet == s.name)
                .cloned()
                .map(|mut q| {
                    q.id = String::new();
                    q
                })
                .collect();
            (!questions.is_empty()).then(|| ExportSheet {
                name: s.name.clone(),
                questions,
            })
        })
        .collect();
    ExportBank {
        id: 7,
        name: name.to_string(),
        sheets,
    }
}

fn clean(q: &Question) -> Value {
    let mut v = serde_json::to_value(q).expect("json");
    if let Some(o) = v.as_object_mut() {
        for k in ["id", "sheet", "row"] {
            o.remove(k);
        }
    }
    v
}

fn assert_clean_parse(pw: &ParsedWorkbook, what: &str) {
    for s in &pw.sheets {
        assert!(
            s.skipped.is_empty(),
            "{what}: skipped rows in {}: {:?}",
            s.name,
            s.skipped
        );
        assert!(
            s.warnings.is_empty(),
            "{what}: warnings in {}: {:?}",
            s.name,
            s.warnings
        );
    }
    assert!(!pw.has_old_layout(), "{what}: old layout");
}

// ---------------------------------------------------------------- sample questions

fn sample_rows_colours() -> Vec<Row> {
    vec![
        header_row(),
        // single choice, mnemonic only in Japanese
        six([
            "Which colour is the sky on a clear day?\nA. Blue\nB. Green\nC. Red",
            "【中文翻译】\n晴天时天空是什么颜色？\nA. 蓝色\nB. 绿色\nC. 红色",
            "晴れた日の空は何色ですか。\nA. 青\nB. 緑\nC. 赤",
            "【Answer】A\n【Explanation】Sunlight scatters in the air.",
            "【答案】A\n【解析】阳光在空气中散射。",
            "【正解】A\n【解説】日光が空気中で散乱します。\n【覚え方】青空は青い",
        ]),
        // multi choice, a stem of several lines, an option of two lines, notes in D, E and F
        six([
            "Which of these are primary colours in paint?\n(Choose two.)\nA. Red\nB. Green\nC. Yellow\n(a warm colour)\nD. Purple\nE. Blue",
            "【中文翻译】\n下列哪些是颜料的原色？\n（选两项。）\nA. 红\nB. 绿\nC. 黄\n（暖色）\nD. 紫\nE. 蓝",
            "絵の具の原色はどれですか。\n（2つ選んでください。）\nA. 赤\nB. 緑\nC. 黄\n（暖色）\nD. 紫\nE. 青",
            "【Answer】A, E (confidence: high)\n【Explanation】Red and blue are primaries.\nYellow is one too in some models.\n【Memory aid】Red, yellow, blue",
            "【答案】A, E（把握度：高）\n【解析】红和蓝是原色。\n【记忆口诀】红黄蓝",
            "【正解】A, E（確信度：高）\n【解説】赤と青は原色です。",
        ]),
        // hotspot with Yes / No in every language (ja.choices present)
        six([
            "HOTSPOT\nDecide whether each statement is true.\n1. Water is wet.\tYes / No\n2. Fire is cold.\tYes / No",
            "【中文翻译】\n判断每个说法是否正确。\n1. 水是湿的。\u{3000}是 / 否\n2. 火是冷的。\u{3000}是 / 否",
            "各文が正しいか判断してください。\n1. 水は濡れている。\u{3000}はい / いいえ\n2. 火は冷たい。\u{3000}はい / いいえ",
            "【Answer】1. Yes; 2. No\n【Explanation】Water is wet, fire is hot.",
            "【答案】1. 是；2. 否\n【解析】水是湿的，火是热的。",
            "【正解】1. はい；2. いいえ\n【解説】水は濡れていて、火は熱いです。",
        ]),
        // hotspot with multi-word choices (TAB) and an item of two lines (not the first), no Chinese or Japanese
        six([
            "HOTSPOT\nMatch each tool with its kind.\n1. A hammer\tHand tool / Power tool\nA drill\n2. that plugs into a socket\tHand tool / Power tool",
            "",
            "",
            "【Answer】1. Hand tool; 2. Power tool",
            "",
            "",
        ]),
        // hotspot whose Japanese text has no choices
        six([
            "HOTSPOT\nStatements about the weather.\n1. Snow is cold.\tYes / No\n2. Rain is dry.\tYes / No",
            "",
            "天気についての文です。\n1. 雪は冷たい。\n2. 雨は乾いている。",
            "【Answer】1. Yes; 2. No (easy)",
            "",
            "【正解】1. Yes; 2. No（簡単）",
        ]),
        // an AI-shaped question: the Chinese text is the whole translation, nothing else
        six([
            "Which animal barks?\nA. Dog\nB. Cat",
            "哪种动物会叫？\nA. 狗\nB. 猫",
            "",
            "【Answer】A\n【Explanation】Dogs bark.",
            "",
            "",
        ]),
        // missing Chinese
        six([
            "Which fruit is yellow?\nA. Banana\nB. Plum",
            "",
            "黄色い果物はどれですか。\nA. バナナ\nB. すもも",
            "【Answer】A",
            "",
            "【正解】A\n【解説】バナナは黄色です。",
        ]),
        // special characters (XML escaping)
        six([
            "Is 1 < 2 & 3 > 2, \"as quoted\" and 'as single'?\nA. Yes\nB. No",
            "1 < 2 且 3 > 2 吗？\nA. 是\nB. 否",
            "",
            "【Answer】A\n【Explanation】Both < and > are fine & \"safe\".",
            "【答案】A",
            "",
        ]),
    ]
}

fn sample_rows_numbers() -> Vec<Row> {
    // No row has Japanese text: only the header row lets the six-column layout be detected
    vec![
        header_row(),
        six([
            "What is two plus two?\nA. Three\nB. Four",
            "二加二等于几？\nA. 三\nB. 四",
            "",
            "【Answer】B\n【Explanation】Basic sums.",
            "【答案】B\n【解析】基础加法。",
            "",
        ]),
        six([
            "What is the first number?\nA. One\nB. Zero",
            "",
            "",
            "【Answer】A",
            "",
            "",
        ]),
    ]
}

fn source_workbook() -> Vec<u8> {
    workbook(&[
        ("Colours", sample_rows_colours()),
        ("Numbers", sample_rows_numbers()),
    ])
}

// ---------------------------------------------------------------- the round trip

#[test]
fn round_trip_of_every_kind_of_question_reproduces_the_questions() {
    let source = parse_workbook(&source_workbook()).expect("parse source");
    assert_clean_parse(&source, "source");
    assert_eq!(source.sheets.len(), 2);
    assert_eq!(source.questions.len(), 10);
    // The sample covers what the test claims to cover
    assert!(source.questions.iter().any(|q| q.ja.is_none()));
    assert!(source.questions.iter().any(|q| q
        .ja
        .as_ref()
        .is_some_and(|j| j.choices.is_none() && j.items.is_some())));
    assert!(source
        .questions
        .iter()
        .any(|q| q.ja.as_ref().is_some_and(|j| j.choices.is_some())));
    assert!(source
        .questions
        .iter()
        .any(|q| q.items.iter().any(|i| i.text.contains('\n'))));

    let bank = bank_of("Sample bank", &source);
    assert_eq!(bank.question_count(), 10);
    let bytes = write_xlsx(&bank).expect("export");
    check_xlsx(&bytes).expect("the export passes the upload check");
    let again = parse_workbook(&bytes).expect("parse export");

    assert_clean_parse(&again, "export");
    let names = |p: &ParsedWorkbook| p.sheets.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&again), names(&source));
    assert_eq!(again.questions.len(), source.questions.len());
    for (a, b) in source.questions.iter().zip(&again.questions) {
        assert_eq!(clean(a), clean(b), "question {:?}", a.stem);
        assert_eq!(a.sheet, b.sheet);
        // The sheet names are kept, so the ids (the key of study records) are kept too
        assert_eq!(a.id, b.id);
    }
    // And a second export of the export is identical
    let second = write_xlsx(&bank_of("Sample bank", &again)).expect("export again");
    assert_eq!(
        parse_workbook(&second).expect("parse").questions,
        again.questions
    );
}

#[test]
fn stripped_control_characters_and_missing_japanese_on_every_row() {
    let source = parse_workbook(&source_workbook()).expect("parse source");
    let mut bank = bank_of("Sample bank", &source);
    // A control character in the data is dropped by the writer, nothing else changes
    let q = &mut bank.sheets[0].questions[0];
    let expected = q.stem.clone();
    q.stem.push('\u{1}');
    // Only the second sheet: no Japanese anywhere on it
    let numbers = bank.sheets.remove(1);
    assert!(numbers.questions.iter().all(|q| q.ja.is_none()));
    bank.sheets.push(numbers);
    let again = parse_workbook(&write_xlsx(&bank).expect("export")).expect("parse");
    assert_clean_parse(&again, "export");
    assert_eq!(again.questions[0].stem, expected);
    assert_eq!(again.questions.len(), bank.question_count());
}

#[test]
fn cells_follow_the_import_layout() {
    let source = parse_workbook(&source_workbook()).expect("parse source");
    let single = source
        .questions
        .iter()
        .find(|q| q.stem.starts_with("Which colour"))
        .expect("single");
    let c = question_cells(single);
    assert_eq!(
        c[0],
        "Which colour is the sky on a clear day?\nA. Blue\nB. Green\nC. Red"
    );
    assert_eq!(
        c[3],
        "【Answer】A\n【Explanation】Sunlight scatters in the air."
    );
    assert_eq!(c[4], "【答案】A\n【解析】阳光在空气中散射。");
    assert!(c[5].starts_with("【正解】A\n【解説】"));
    assert!(c[5].ends_with("\n【覚え方】青空は青い"));
    let hot = source
        .questions
        .iter()
        .find(|q| q.stem.starts_with("Decide whether"))
        .expect("hotspot");
    let c = question_cells(hot);
    assert!(c[0].starts_with(
        "HOTSPOT\nDecide whether each statement is true.\n1. Water is wet.\tYes / No"
    ));
    assert!(c[2].contains("1. 水は濡れている。\u{3000}はい / いいえ"));
    assert!(c[3].starts_with("【Answer】1. Yes; 2. No"));
}

#[test]
fn the_workbook_has_one_sheet_per_bank_sheet_and_a_header_row() {
    let source = parse_workbook(&source_workbook()).expect("parse source");
    let bytes = write_xlsx(&bank_of("Sample bank", &source)).expect("export");
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
    let mut read = |name: &str| {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut z.by_name(name).expect(name), &mut s).expect("read");
        s
    };
    for part in [
        "[Content_Types].xml",
        "_rels/.rels",
        "docProps/core.xml",
        "docProps/app.xml",
        "xl/workbook.xml",
        "xl/_rels/workbook.xml.rels",
        "xl/styles.xml",
        "xl/worksheets/sheet1.xml",
        "xl/worksheets/sheet2.xml",
    ] {
        let xml = read(part);
        let mut r = quick_xml::Reader::from_str(&xml);
        loop {
            match r.read_event() {
                Ok(quick_xml::events::Event::Eof) => break,
                Ok(_) => {}
                Err(e) => panic!("{part}: {e}"),
            }
        }
    }
    assert!(read("docProps/core.xml").contains("<dc:title>Sample bank</dc:title>"));
    let wb = read("xl/workbook.xml");
    assert!(wb.contains(r#"<sheet name="Colours" sheetId="1" r:id="rId1"/>"#));
    assert!(wb.contains(r#"<sheet name="Numbers" sheetId="2" r:id="rId2"/>"#));
    let s1 = read("xl/worksheets/sheet1.xml");
    assert!(s1.contains("Question (English)"));
    assert!(s1.contains(r#"state="frozen""#));
    assert!(s1.contains("&lt;") && s1.contains("&amp;") && s1.contains("&quot;"));
}

#[test]
fn a_bank_without_sheets_gives_one_header_only_sheet() {
    let bank = ExportBank {
        id: 1,
        name: "Empty".into(),
        sheets: vec![],
    };
    let bytes = write_xlsx(&bank).expect("export");
    check_xlsx(&bytes).expect("valid");
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
    let mut wb = String::new();
    std::io::Read::read_to_string(&mut z.by_name("xl/workbook.xml").expect("wb"), &mut wb)
        .expect("read");
    assert!(wb.contains(r#"name="Sheet1""#));
    assert!(parse_workbook(&write_xlsx(&bank).expect("export"))
        .expect("parse")
        .questions
        .is_empty());
}

// ---------------------------------------------------------------- names

fn names(v: &[&str]) -> Vec<String> {
    sheet_names(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>())
}

#[test]
fn sheet_names_are_excel_safe_and_unique() {
    assert_eq!(
        names(&["Colours", "第一章 Part 1"]),
        ["Colours", "第一章 Part 1"]
    );
    assert_eq!(names(&["a/b:c?"]), ["a_b_c_"]);
    assert_eq!(names(&["[x]*y\\z"]), ["_x__y_z"]);
    assert_eq!(names(&["'quoted'"]), ["quoted"]);
    let long = "abcdefghij".repeat(4);
    assert_eq!(names(&[&long]), ["abcdefghijabcdefghijabcdefghija"]);
    // Cut by UTF-16 units: an emoji counts as two
    let emoji = "😀".repeat(20);
    let cut = &names(&[&emoji])[0];
    assert_eq!(cut.encode_utf16().count(), 30);
    assert_eq!(names(&["S", "s", "S"]), ["S", "s (2)", "S (3)"]);
    assert_eq!(names(&[""]), ["Sheet"]);
    assert_eq!(names(&["", "///"]), ["Sheet", "___"]);
    assert_eq!(names(&["''", ""]), ["Sheet", "Sheet (2)"]);
    // A duplicate of a 31-unit name stays within 31 units
    let max = "m".repeat(31);
    let out = names(&[&max, &max]);
    assert_eq!(out[0], max);
    assert_eq!(out[1].encode_utf16().count(), 31);
    assert!(out[1].ends_with(" (2)"));
}

// ---------------------------------------------------------------- file names

fn bank_named(name: &str) -> ExportBank {
    ExportBank {
        id: 7,
        name: name.into(),
        sheets: vec![],
    }
}

#[test]
fn file_stem_and_content_disposition() {
    assert_eq!(file_stem("Sample bank"), "Sample bank");
    assert_eq!(file_stem("a/b:c*d?e\"f<g>h|i\\j"), "a_b_c_d_e_f_g_h_i_j");
    assert_eq!(file_stem("..."), "bank");
    assert_eq!(file_stem("  . . "), "bank");
    assert_eq!(file_stem(""), "bank");
    assert_eq!(file_stem(&"x".repeat(200)).chars().count(), 120);
    assert_eq!(
        file_name(&bank_named("Sample"), ExportFormat::Xlsx),
        "Sample.xlsx"
    );

    assert_eq!(
        content_disposition(&bank_named("Sample bank"), ExportFormat::Xlsx),
        "attachment; filename=\"Sample bank.xlsx\"; filename*=UTF-8''Sample%20bank.xlsx"
    );
    assert_eq!(
        content_disposition(&bank_named("示例题库"), ExportFormat::Docx),
        "attachment; filename=\"xexam-bank-7.docx\"; filename*=UTF-8''%E7%A4%BA%E4%BE%8B%E9%A2%98%E5%BA%93.docx"
    );
    let full = content_disposition(&bank_named("/:*?\"<>|"), ExportFormat::Xlsx);
    assert_eq!(
        full,
        "attachment; filename=\"________.xlsx\"; filename*=UTF-8''________.xlsx"
    );
    assert_eq!(
        content_disposition(&bank_named("..."), ExportFormat::Xlsx),
        "attachment; filename=\"bank.xlsx\"; filename*=UTF-8''bank.xlsx"
    );
}

#[test]
fn format_parse_and_mime() {
    assert_eq!(ExportFormat::parse("xlsx"), Some(ExportFormat::Xlsx));
    assert_eq!(ExportFormat::parse("docx"), Some(ExportFormat::Docx));
    for bad in ["pdf", "XLSX", "", " xlsx"] {
        assert_eq!(ExportFormat::parse(bad), None, "{bad:?}");
    }
    assert!(ExportFormat::Xlsx.mime().ends_with("spreadsheetml.sheet"));
    assert!(ExportFormat::Docx
        .mime()
        .ends_with("wordprocessingml.document"));
}
