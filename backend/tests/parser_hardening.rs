//! Hardening and differences: build zips in memory and verify behavior that intentionally differs from the old version.
use std::io::{Cursor, Write};
use xexam_backend::error::ImportError;
use xexam_backend::parser::{check_xlsx, parse_workbook};

const WB: &str = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
const RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Target="worksheets/a.xml"/></Relationships>"#;

fn zip_of(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (n, b) in parts {
        w.start_file(*n, opts).expect("start");
        w.write_all(b).expect("write");
    }
    let mut out = w.finish().expect("finish").into_inner();
    // check_xlsx requires at least 100 bytes
    if out.len() < 100 {
        out.resize(100, 0);
    }
    out
}

fn sheet(cell_a: &str) -> String {
    format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1">{cell_a}<c r="C1" t="inlineStr"><is><t>【答案】B</t></is></c></row></sheetData></worksheet>"#
    )
}

fn pkg(sheet_xml: &str, shared: Option<&str>) -> Vec<u8> {
    let mut parts: Vec<(&str, &[u8])> = vec![
        ("xl/workbook.xml", WB.as_bytes()),
        ("xl/_rels/workbook.xml.rels", RELS.as_bytes()),
        ("xl/worksheets/a.xml", sheet_xml.as_bytes()),
    ];
    if let Some(s) = shared {
        parts.push(("xl/sharedStrings.xml", s.as_bytes()));
    }
    zip_of(&parts)
}

const QA: &str = r#"<c r="A1" t="inlineStr"><is><t>これは十分に長い問題文です。
A. 甲
B. 乙</t></is></c>"#;

#[test]
fn a_minimal_workbook_parses() {
    let data = pkg(&sheet(QA), None);
    check_xlsx(&data).expect("check");
    let pw = parse_workbook(&data).expect("parse");
    assert_eq!(pw.questions.len(), 1);
    assert_eq!(pw.sheets.first().map(|s| (s.total, s.usable)), Some((1, 1)));
}

#[test]
fn negative_shared_string_index_is_dropped_not_wrapped() {
    let sst = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>これは十分に長い問題文です。
A. 甲
B. 乙</t></si></sst>"#;
    let ok = pkg(&sheet(r#"<c r="A1" t="s"><v>0</v></c>"#), Some(sst));
    assert_eq!(parse_workbook(&ok).expect("ok").questions.len(), 1);
    // Python treats -1 as the last entry; the new version treats it as malformed and drops the cell
    let neg = pkg(&sheet(r#"<c r="A1" t="s"><v>-1</v></c>"#), Some(sst));
    assert_eq!(parse_workbook(&neg).expect("neg").questions.len(), 0);
}

#[test]
fn doctype_is_rejected_whatever_the_extension_case_or_encoding() {
    let dt = br#"<!DOCTYPE x [<!ENTITY e "BOOM">]><worksheet/>"#;
    let utf16: Vec<u8> = std::iter::once(0xFEFFu16)
        .chain(String::from_utf8_lossy(dt).encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect();
    for (name, body) in [
        ("xl/worksheets/a.XML", dt.to_vec()),
        ("xl/worksheets/a.dat", dt.to_vec()),
        ("xl/worksheets/a.xml", utf16.clone()),
        ("customXml/item1", utf16),
    ] {
        let data = zip_of(&[
            ("xl/workbook.xml", WB.as_bytes()),
            ("xl/_rels/workbook.xml.rels", RELS.as_bytes()),
            (name, &body),
        ]);
        assert_eq!(
            check_xlsx(&data),
            Err(ImportError::EntityDeclaration),
            "{name}"
        );
    }
}

#[test]
fn doctype_inside_parse_is_rejected_even_without_check() {
    let dt = r#"<!DOCTYPE x [<!ENTITY e "BOOM">]><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"/>"#;
    let data = pkg(dt, None);
    assert_eq!(parse_workbook(&data), Err(ImportError::EntityDeclaration));
}

#[test]
fn garbage_never_panics() {
    for data in [
        vec![],
        vec![b'P', b'K'],
        vec![0u8; 500],
        b"PK\x03\x04".repeat(100),
    ] {
        let _ = check_xlsx(&data);
        let _ = parse_workbook(&data);
    }
    let broken = pkg("<worksheet", None);
    assert!(
        matches!(parse_workbook(&broken), Err(ImportError::Message(m)) if m.starts_with("文件里的 XML 读不了："))
    );
}

#[test]
fn deeply_nested_xml_is_refused() {
    let deep = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{}{}</worksheet>"#,
        "<a>".repeat(200),
        "</a>".repeat(200)
    );
    assert!(matches!(
        parse_workbook(&pkg(&deep, None)),
        Err(ImportError::Message(_))
    ));
}

// ---------------------------------------------------------------- Regression tests added after review

use std::time::{Duration, Instant};
use xexam_backend::parser::parse_workbook_until;

/// Many cells reference the same very large shared string: the byte budget must stop this (previously each cell deep-copied it, which blew up memory).
#[test]
fn shared_string_fanout_is_bounded_by_the_byte_budget() {
    let big = "あ".repeat(400_000); // about 1.2 MB
    let sst = format!(
        r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>{big}</t></si></sst>"#
    );
    // Cells A and B of every row both reference it: 1.2 MB x 2 x 40 rows = about 96 MB > 64 MB budget
    let mut rows = String::new();
    for r in 1..=40 {
        rows.push_str(&format!(
            r#"<row r="{r}"><c r="A{r}" t="s"><v>0</v></c><c r="B{r}" t="s"><v>0</v></c></row>"#
        ));
    }
    let sheet_xml = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{rows}</sheetData></worksheet>"#
    );
    let data = pkg(&sheet_xml, Some(&sst));
    assert!(
        matches!(parse_workbook(&data), Err(ImportError::Message(m)) if m.contains("内容太多")),
        "must be refused as too complex"
    );
}

/// A normal-size shared string referenced by many cells still parses.
#[test]
fn shared_strings_used_by_several_cells_still_parse() {
    let sst = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>これは十分に長い問題文です。
A. 甲
B. 乙</t></si></sst>"#;
    let rows: String = (1..=3)
        .map(|r| format!(r#"<row r="{r}"><c r="A{r}" t="s"><v>0</v></c><c r="C{r}" t="inlineStr"><is><t>【答案】B</t></is></c></row>"#))
        .collect();
    let sheet_xml = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{rows}</sheetData></worksheet>"#
    );
    let pw = parse_workbook(&pkg(&sheet_xml, Some(sst))).expect("parse");
    assert_eq!(pw.questions.len(), 3);
    // Three identical questions: the ids are base, base-2, base-3 (same as the old version)
    let ids: Vec<&str> = pw.questions.iter().map(|q| q.id.as_str()).collect();
    assert_eq!(ids[1], format!("{}-2", ids[0]));
    assert_eq!(ids[2], format!("{}-3", ids[0]));
}

/// Duplicate-id suffixes are generated linearly: thousands of identical questions must still parse quickly, with suffixes -2, -3 … in order.
#[test]
fn thousands_of_duplicate_questions_are_not_quadratic() {
    let sst = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>これは十分に長い問題文です。
A. 甲
B. 乙</t></si></sst>"#;
    let n = 6000;
    let rows: String = (1..=n)
        .map(|r| format!(r#"<row r="{r}"><c r="A{r}" t="s"><v>0</v></c><c r="C{r}" t="inlineStr"><is><t>【答案】B</t></is></c></row>"#))
        .collect();
    let sheet_xml = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{rows}</sheetData></worksheet>"#
    );
    let data = pkg(&sheet_xml, Some(sst));
    let t = Instant::now();
    let pw = parse_workbook(&data).expect("parse");
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "took {:?}",
        t.elapsed()
    );
    assert_eq!(pw.questions.len(), n);
    let base = pw.questions[0].id.clone();
    assert_eq!(pw.questions[n - 1].id, format!("{base}-{n}"));
    let uniq: std::collections::HashSet<&str> =
        pw.questions.iter().map(|q| q.id.as_str()).collect();
    assert_eq!(uniq.len(), n, "all ids must be distinct");
}

/// An element has an upper limit on its number of attributes.
#[test]
fn too_many_attributes_on_one_element_are_refused() {
    let attrs: String = (0..300).map(|i| format!(r#" a{i}="x""#)).collect();
    let sheet_xml = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"{attrs}/></sheetData></worksheet>"#
    );
    assert!(
        matches!(parse_workbook(&pkg(&sheet_xml, None)), Err(ImportError::Message(m)) if m.contains("内容太多")),
    );
}

/// The deadline has passed: the parser stops by itself and returns `Timeout` (instead of running to completion).
#[test]
fn an_expired_deadline_stops_the_parser_itself() {
    let rows: String = (1..=3000)
        .map(|r| {
            format!(
                r#"<row r="{r}"><c r="A{r}" t="inlineStr"><is><t>これは十分に長い問題文です。{r}
A. 甲
B. 乙</t></is></c><c r="C{r}" t="inlineStr"><is><t>【答案】B</t></is></c></row>"#
            )
        })
        .collect();
    let sheet_xml = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{rows}</sheetData></worksheet>"#
    );
    let data = pkg(&sheet_xml, None);
    let past = Instant::now() - Duration::from_secs(1);
    assert_eq!(
        parse_workbook_until(&data, Some(past)),
        Err(ImportError::Timeout)
    );
    // No deadline / deadline still far away: parse normally
    assert_eq!(
        parse_workbook_until(&data, None)
            .expect("no deadline")
            .questions
            .len(),
        3000
    );
    let far = Instant::now() + Duration::from_secs(60);
    assert_eq!(
        parse_workbook_until(&data, Some(far))
            .expect("far deadline")
            .questions
            .len(),
        3000
    );
}

/// Flip one byte in the data area of the first zip member named `name` (so the CRC no longer matches); leave everything else as is.
fn corrupt_member(zip_bytes: &[u8], name: &str) -> Vec<u8> {
    let mut out = zip_bytes.to_vec();
    let pat = name.as_bytes();
    // Local file header: signature PK\x03\x04, 30-byte fixed header, then file name, extra, data
    let mut i = 0;
    while i + 30 < out.len() {
        if out[i..].starts_with(b"PK\x03\x04") {
            let name_len = u16::from_le_bytes([out[i + 26], out[i + 27]]) as usize;
            let extra_len = u16::from_le_bytes([out[i + 28], out[i + 29]]) as usize;
            let comp_size =
                u32::from_le_bytes([out[i + 18], out[i + 19], out[i + 20], out[i + 21]]) as usize;
            let start = i + 30;
            if out.get(start..start + name_len) == Some(pat) {
                let data = start + name_len + extra_len;
                assert!(comp_size > 0, "member {name} is empty");
                out[data] ^= 0xFF;
                return out;
            }
            i = start + name_len + extra_len + comp_size;
        } else {
            i += 1;
        }
    }
    panic!("member {name} not found");
}

/// The old version only decompressed .xml / .rels to check them; a corrupted **non-XML** member (a thumbnail or the like, CRC mismatch) was accepted by the old version, and the new version must accept it too.
/// A corrupted XML member must be rejected.
#[test]
fn a_corrupt_non_xml_member_is_tolerated_but_a_corrupt_xml_member_is_not() {
    let mut parts: Vec<(&str, &[u8])> = vec![
        ("xl/workbook.xml", WB.as_bytes()),
        ("xl/_rels/workbook.xml.rels", RELS.as_bytes()),
        ("xl/worksheets/a.xml", sheet(QA).leak().as_bytes()),
    ];
    let thumb = vec![7u8; 400];
    parts.push(("docProps/thumbnail.jpeg", &thumb));
    let good = zip_of(&parts);
    check_xlsx(&good).expect("untouched file is fine");

    let bad_thumb = corrupt_member(&good, "docProps/thumbnail.jpeg");
    check_xlsx(&bad_thumb).expect("a corrupt thumbnail must not make the workbook unusable");
    assert_eq!(
        parse_workbook(&bad_thumb).expect("parse").questions.len(),
        1
    );

    let bad_xml = corrupt_member(&good, "xl/worksheets/a.xml");
    assert_eq!(check_xlsx(&bad_xml), Err(ImportError::Corrupt));
}
