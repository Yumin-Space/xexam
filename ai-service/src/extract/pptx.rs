//! PPTX (IX-3): slides in `sldIdLst` order, each paragraph a block, table rows one block, then the notes slide as one
//! block prefixed `[Notes] ` (slide numbers, slide images, headers, footers and dates left out). One section per slide.
use super::xmlx::{walk, Ev, XmlBudget};
use super::zipx::{read_rels, ZipPkg};
use super::{Collector, Doc, ExtractError, FileKind, Limits};

/// The paragraphs of a slide part as blocks (tables: one block per row). `notes`: leaves out the placeholders of the
/// notes page that are not the notes themselves.
fn slide_blocks(
    bytes: &[u8],
    notes: bool,
    limits: &Limits,
    budget: &mut XmlBudget,
) -> Result<Vec<String>, ExtractError> {
    let mut out: Vec<String> = Vec::new();
    let mut para = String::new();
    let mut in_p = false;
    let mut in_t = false;
    // shape stack: true = a skipped placeholder (notes only)
    let mut shapes: Vec<bool> = Vec::new();
    let mut table_depth = 0usize;
    let mut row: Vec<String> = Vec::new();
    let mut cell: Vec<String> = Vec::new();
    walk(bytes, limits, budget, |e| {
        match e {
            Ev::Start(t) => match t.name.as_str() {
                "sp" => shapes.push(false),
                "ph" => {
                    if notes
                        && matches!(
                            t.attr("type"),
                            Some("sldNum" | "sldImg" | "hdr" | "ftr" | "dt")
                        )
                    {
                        if let Some(top) = shapes.last_mut() {
                            *top = true;
                        }
                    }
                }
                "tbl" => table_depth += 1,
                "tr" if table_depth == 1 => row.clear(),
                "tc" if table_depth == 1 => cell.clear(),
                "p" => {
                    in_p = true;
                    para.clear();
                }
                "t" if in_p => in_t = true,
                "br" if in_p => para.push('\n'),
                _ => {}
            },
            Ev::End(n) => match n {
                "t" => in_t = false,
                "p" => {
                    in_p = false;
                    let text = para.trim().to_string();
                    para.clear();
                    if text.is_empty() || shapes.last() == Some(&true) {
                        return Ok(());
                    }
                    if table_depth > 0 {
                        cell.push(text);
                    } else {
                        out.push(text);
                    }
                }
                "tc" if table_depth == 1 => row.push(cell.join(" / ")),
                "tr" if table_depth == 1 => {
                    let cells: Vec<&str> = row
                        .iter()
                        .map(|c| c.trim())
                        .filter(|c| !c.is_empty())
                        .collect();
                    if !cells.is_empty() {
                        out.push(cells.join(" | "));
                    }
                    row.clear();
                }
                "tbl" => table_depth = table_depth.saturating_sub(1),
                "sp" => {
                    shapes.pop();
                }
                _ => {}
            },
            Ev::Text(s) => {
                if in_t {
                    para.push_str(s);
                }
            }
        }
        Ok(())
    })?;
    Ok(out)
}

pub fn extract(bytes: &[u8], limits: &Limits) -> Result<Doc, ExtractError> {
    let mut pkg = ZipPkg::open(bytes, limits)?;
    let mut budget = XmlBudget::default();
    let pres = pkg
        .read("ppt/presentation.xml")?
        .ok_or(ExtractError::Corrupt)?;
    let mut ids: Vec<String> = Vec::new();
    walk(&pres, limits, &mut budget, |e| {
        if let Ev::Start(t) = e {
            if t.name == "sldId" {
                if ids.len() >= limits.pptx_slides {
                    return Err(ExtractError::TooComplex);
                }
                ids.push(t.attr("id").unwrap_or("").to_string());
            }
        }
        Ok(())
    })?;
    let rels = read_rels(&mut pkg, "ppt/presentation.xml", limits, &mut budget)?;
    let mut col = Collector::new(limits.max_chars);
    let mut pages = 0usize;
    for rid in &ids {
        let Some(target) = rels
            .iter()
            .find(|r| &r.id == rid)
            .and_then(|r| r.target.clone())
        else {
            continue;
        };
        let Some(slide) = pkg.read_once(&target)? else {
            continue;
        };
        pages += 1;
        col.begin(&format!("Slide {pages}"));
        for b in slide_blocks(&slide, false, limits, &mut budget)? {
            col.add(&b)?;
        }
        let srels = read_rels(&mut pkg, &target, limits, &mut budget)?;
        if let Some(notes_part) = srels
            .iter()
            .find(|r| r.kind.ends_with("/notesSlide"))
            .and_then(|r| r.target.clone())
        {
            if let Some(nb) = pkg.read_once(&notes_part)? {
                let paras = slide_blocks(&nb, true, limits, &mut budget)?;
                if !paras.is_empty() {
                    col.add(&format!("[Notes] {}", paras.join("\n")))?;
                }
            }
        }
    }
    Ok(col.finish(FileKind::Pptx, pages, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::testgen::{pptx, PptxSpec};

    fn lim() -> Limits {
        Limits {
            min_meaningful: 0,
            ..Limits::default()
        }
    }

    #[test]
    fn slides_follow_the_slide_list_not_the_file_order() {
        let doc = extract(&pptx(&PptxSpec::simple()), &lim()).unwrap();
        assert_eq!(doc.pages, 2);
        assert_eq!(doc.sections[0].label, "Slide 1");
        assert_eq!(doc.sections[0].blocks[0], "First slide title");
        assert_eq!(doc.sections[1].label, "Slide 2");
        assert_eq!(doc.sections[1].blocks[0], "Second slide body");
    }

    #[test]
    fn tables_and_notes_without_the_slide_number() {
        let doc = extract(&pptx(&PptxSpec::simple()), &lim()).unwrap();
        let first = &doc.sections[0].blocks;
        assert!(first.contains(&"Row A | Row B".to_string()), "{first:?}");
        let notes = first
            .iter()
            .find(|b| b.starts_with("[Notes] "))
            .expect("notes");
        assert_eq!(notes, "[Notes] Remember the simmer question");
        assert!(!first.iter().any(|b| b.contains("slide number")));
    }

    #[test]
    fn the_slide_cap_applies() {
        let mut spec = PptxSpec::simple();
        spec.slides = (0..5).map(|i| format!("Slide text {i}")).collect();
        spec.order = Vec::new();
        assert_eq!(
            extract(&pptx(&spec), &Limits::small()),
            Err(ExtractError::TooComplex)
        );
    }

    #[test]
    fn a_missing_presentation_is_corrupt() {
        let z = crate::extract::testgen::zip_of(&[("ppt/x.xml", b"<a/>")]);
        assert_eq!(extract(&z, &Limits::default()), Err(ExtractError::Corrupt));
    }
}
