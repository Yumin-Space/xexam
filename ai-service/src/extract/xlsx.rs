//! XLSX (IX-3): sheet names in workbook order, shared strings (phonetic runs skipped), one block per row with the
//! non-empty cells joined by " | ", one section per sheet (hidden sheets included). A sheet part is read once even when
//! several sheets point at it.
use super::xmlx::{walk, Ev, XmlBudget};
use super::zipx::{read_rels, ZipPkg};
use super::{Collector, Doc, ExtractError, FileKind, Limits};

fn shared_strings(
    pkg: &mut ZipPkg<'_>,
    part: &str,
    limits: &Limits,
    budget: &mut XmlBudget,
) -> Result<Vec<String>, ExtractError> {
    let Some(bytes) = pkg.read(part)? else {
        return Ok(Vec::new());
    };
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_si = false;
    let mut in_t = false;
    let mut rph = 0usize;
    walk(&bytes, limits, budget, |e| {
        match e {
            Ev::Start(t) => match t.name.as_str() {
                "si" => {
                    in_si = true;
                    cur.clear();
                    if out.len() >= limits.xlsx_shared {
                        return Err(ExtractError::TooComplex);
                    }
                }
                "rPh" => rph += 1,
                "t" if in_si && rph == 0 => in_t = true,
                _ => {}
            },
            Ev::End(n) => match n {
                "si" => {
                    in_si = false;
                    out.push(std::mem::take(&mut cur));
                }
                "rPh" => rph = rph.saturating_sub(1),
                "t" => in_t = false,
                _ => {}
            },
            Ev::Text(s) => {
                if in_t {
                    cur.push_str(s);
                }
            }
        }
        Ok(())
    })?;
    Ok(out)
}

struct Cell {
    kind: String,
    v: String,
    inline: String,
}

fn cell_text(c: &Cell, shared: &[String]) -> Option<String> {
    let text = match c.kind.as_str() {
        "s" => {
            c.v.trim()
                .parse::<usize>()
                .ok()
                .and_then(|i| shared.get(i))
                .cloned()
                .unwrap_or_default()
        }
        "inlineStr" => c.inline.clone(),
        "str" => c.v.clone(),
        "b" => if c.v.trim() == "1" { "TRUE" } else { "FALSE" }.to_string(),
        "e" => return None,
        _ => c.v.clone(),
    };
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn read_sheet(
    bytes: &[u8],
    shared: &[String],
    limits: &Limits,
    budget: &mut XmlBudget,
    cells_seen: &mut usize,
    col: &mut Collector,
) -> Result<(), ExtractError> {
    let mut row: Vec<String> = Vec::new();
    let mut cell: Option<Cell> = None;
    let mut in_v = false;
    let mut in_is = false;
    let mut in_is_t = false;
    let mut rows = 0usize;
    walk(bytes, limits, budget, |e| {
        match e {
            Ev::Start(t) => match t.name.as_str() {
                "row" => row.clear(),
                "c" => {
                    cell = Some(Cell {
                        kind: t.attr("t").unwrap_or("n").to_string(),
                        v: String::new(),
                        inline: String::new(),
                    })
                }
                "v" if cell.is_some() => in_v = true,
                "is" if cell.is_some() => in_is = true,
                "t" if in_is => in_is_t = true,
                _ => {}
            },
            Ev::End(n) => match n {
                "v" => in_v = false,
                "is" => in_is = false,
                "t" => in_is_t = false,
                "c" => {
                    if let Some(c) = cell.take() {
                        if let Some(text) = cell_text(&c, shared) {
                            *cells_seen += 1;
                            if *cells_seen > limits.xlsx_cells {
                                return Err(ExtractError::TooComplex);
                            }
                            row.push(text);
                        }
                    }
                }
                "row" => {
                    rows += 1;
                    if rows.is_multiple_of(1000) {
                        limits.check_deadline()?;
                    }
                    if !row.is_empty() {
                        col.add(&row.join(" | "))?;
                        row.clear();
                    }
                }
                _ => {}
            },
            Ev::Text(s) => {
                if let Some(c) = cell.as_mut() {
                    if in_v {
                        c.v.push_str(s);
                    } else if in_is_t {
                        c.inline.push_str(s);
                    }
                }
            }
        }
        Ok(())
    })
}

pub fn extract(bytes: &[u8], limits: &Limits) -> Result<Doc, ExtractError> {
    let mut pkg = ZipPkg::open(bytes, limits)?;
    let mut budget = XmlBudget::default();
    let wb = pkg.read("xl/workbook.xml")?.ok_or(ExtractError::Corrupt)?;
    // (name, r:id) in workbook order
    let mut sheets: Vec<(String, String)> = Vec::new();
    walk(&wb, limits, &mut budget, |e| {
        if let Ev::Start(t) = e {
            if t.name == "sheet" {
                if sheets.len() >= limits.xlsx_sheets {
                    return Err(ExtractError::TooComplex);
                }
                sheets.push((
                    t.attr("name").unwrap_or("").to_string(),
                    t.attr("id").unwrap_or("").to_string(),
                ));
            }
        }
        Ok(())
    })?;
    let rels = read_rels(&mut pkg, "xl/workbook.xml", limits, &mut budget)?;
    let shared_part = rels
        .iter()
        .find(|r| r.kind.ends_with("/sharedStrings"))
        .and_then(|r| r.target.clone())
        .unwrap_or_else(|| "xl/sharedStrings.xml".to_string());
    let shared = shared_strings(&mut pkg, &shared_part, limits, &mut budget)?;
    let mut col = Collector::new(limits.max_chars);
    let mut cells_seen = 0usize;
    let mut pages = 0usize;
    for (name, rid) in &sheets {
        let Some(target) = rels
            .iter()
            .find(|r| &r.id == rid)
            .and_then(|r| r.target.clone())
        else {
            continue;
        };
        let Some(part) = pkg.read_once(&target)? else {
            continue;
        };
        pages += 1;
        col.begin(name);
        read_sheet(
            &part,
            &shared,
            limits,
            &mut budget,
            &mut cells_seen,
            &mut col,
        )?;
    }
    Ok(col.finish(FileKind::Xlsx, pages, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::testgen::{xlsx, SheetSpec, XlsxSpec};

    fn lim() -> Limits {
        Limits {
            min_meaningful: 0,
            ..Limits::default()
        }
    }

    #[test]
    fn two_sheets_in_workbook_order_with_every_cell_kind() {
        let doc = extract(&xlsx(&XlsxSpec::simple()), &lim()).unwrap();
        assert_eq!(doc.pages, 3);
        assert_eq!(doc.sections[0].label, "Ovens");
        assert_eq!(
            doc.sections[0].blocks,
            vec![
                "Which tool keeps a pot at a steady simmer? | Thermostat | 42 | TRUE | inline note"
            ]
        );
        assert_eq!(doc.sections[1].label, "Knives");
        assert_eq!(doc.sections[1].blocks, vec!["Second sheet row | FALSE"]);
        // the hidden sheet is read too, and an error cell is skipped
        assert_eq!(doc.sections[2].label, "Hidden");
        assert_eq!(doc.sections[2].blocks, vec!["secret row"]);
    }

    #[test]
    fn phonetic_runs_are_skipped_in_shared_strings() {
        let mut spec = XlsxSpec::simple();
        spec.shared = vec![
            "<si><r><t>漢字</t></r><rPh sb=\"0\" eb=\"2\"><t>かんじ</t></rPh></si>".to_string(),
            "<si><t>plain</t></si>".to_string(),
        ];
        spec.sheets = vec![SheetSpec::new(
            "S",
            "<row><c t=\"s\"><v>0</v></c><c t=\"s\"><v>1</v></c></row>",
            false,
        )];
        let doc = extract(&xlsx(&spec), &lim()).unwrap();
        assert_eq!(doc.sections[0].blocks, vec!["漢字 | plain"]);
    }

    #[test]
    fn sheet_cell_and_shared_string_caps() {
        let l = Limits::small();
        let mut spec = XlsxSpec::simple();
        spec.sheets = (0..5)
            .map(|i| {
                SheetSpec::new(
                    &format!("S{i}"),
                    "<row><c t=\"n\"><v>1</v></c></row>",
                    false,
                )
            })
            .collect();
        assert_eq!(extract(&xlsx(&spec), &l), Err(ExtractError::TooComplex));
        let mut spec = XlsxSpec::simple();
        let rows: String = (0..60)
            .map(|i| format!("<row><c t=\"n\"><v>{i}</v></c></row>"))
            .collect();
        spec.sheets = vec![SheetSpec::new("Big", &rows, false)];
        assert_eq!(extract(&xlsx(&spec), &l), Err(ExtractError::TooComplex));
        let mut spec = XlsxSpec::simple();
        spec.shared = (0..60).map(|i| format!("<si><t>s{i}</t></si>")).collect();
        spec.sheets = vec![SheetSpec::new(
            "S",
            "<row><c t=\"n\"><v>1</v></c></row>",
            false,
        )];
        assert_eq!(extract(&xlsx(&spec), &l), Err(ExtractError::TooComplex));
    }

    #[test]
    fn sheets_pointing_at_one_part_read_it_once() {
        let mut spec = XlsxSpec::simple();
        spec.sheets = vec![SheetSpec::new(
            "One",
            "<row><c t=\"n\"><v>7</v></c></row>",
            false,
        )];
        spec.alias_sheets = vec!["Two".into(), "Three".into()];
        let doc = extract(&xlsx(&spec), &lim()).unwrap();
        assert_eq!(doc.pages, 1);
        assert_eq!(doc.sections.len(), 1);
    }

    #[test]
    fn an_external_or_escaping_target_is_skipped() {
        let mut spec = XlsxSpec::simple();
        spec.sheets = vec![SheetSpec::new(
            "Good",
            "<row><c t=\"n\"><v>1</v></c></row>",
            false,
        )];
        spec.extra_rels = vec![
            "<Relationship Id=\"rIdX\" Type=\"t/worksheet\" Target=\"https://example.com/s.xml\" TargetMode=\"External\"/>".into(),
            "<Relationship Id=\"rIdY\" Type=\"t/worksheet\" Target=\"../../../etc/passwd\"/>".into(),
        ];
        spec.extra_sheet_refs = vec![
            ("Evil1".into(), "rIdX".into()),
            ("Evil2".into(), "rIdY".into()),
            ("Gone".into(), "rIdNone".into()),
        ];
        let doc = extract(&xlsx(&spec), &lim()).unwrap();
        assert_eq!(doc.pages, 1);
        assert_eq!(doc.sections[0].label, "Good");
    }

    #[test]
    fn a_missing_workbook_is_corrupt() {
        let z = crate::extract::testgen::zip_of(&[("xl/other.xml", b"<a/>")]);
        assert_eq!(extract(&z, &Limits::default()), Err(ExtractError::Corrupt));
    }
}
