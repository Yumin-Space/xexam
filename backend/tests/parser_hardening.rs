//! 加固与差异：在内存里造 zip，验证有意与旧版不同的行为。
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
    // check_xlsx 要求至少 100 字节
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
    // Python 会把 -1 当作最后一个条目；新版视为畸形并丢弃该单元格
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
