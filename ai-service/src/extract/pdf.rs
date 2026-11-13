//! PDF (IX-3): the text layer only, page by page (`pdf-extract` over `lopdf`). A scan without a text layer is `NoText`;
//! an encrypted file opens only when the empty password does (`Protected` otherwise); a malformed file that makes the
//! library panic is `Corrupt` (the child process also catches panics, this is the second line).
use super::clean::drop_cjk_spaces;
use super::{Collector, Doc, ExtractError, FileKind, Limits};
use std::panic::{catch_unwind, AssertUnwindSafe};

pub fn extract(bytes: &[u8], limits: &Limits) -> Result<Doc, ExtractError> {
    match catch_unwind(AssertUnwindSafe(|| extract_inner(bytes, limits))) {
        Ok(r) => r,
        Err(_) => Err(ExtractError::Corrupt),
    }
}

fn extract_inner(bytes: &[u8], limits: &Limits) -> Result<Doc, ExtractError> {
    let mut doc = pdf_extract::Document::load_mem(bytes).map_err(|_| ExtractError::Corrupt)?;
    if doc.is_encrypted() {
        doc.decrypt("").map_err(|_| ExtractError::Protected)?;
    }
    let pages = doc.get_pages();
    if pages.len() > limits.pdf_pages {
        return Err(ExtractError::TooComplex);
    }
    let mut col = Collector::new(limits.max_chars);
    let mut meaningful_total = 0usize;
    for (n, (&page_no, _)) in pages.iter().enumerate() {
        limits.check_deadline()?;
        let mut text = String::new();
        // a page error leaves that page empty
        let page_ok = {
            let mut out = pdf_extract::PlainTextOutput::new(&mut text);
            pdf_extract::output_doc_page(&doc, &mut out, page_no).is_ok()
        };
        col.begin(&format!("p. {}", n + 1));
        if page_ok {
            let text = drop_cjk_spaces(&text);
            meaningful_total += text.chars().filter(|c| c.is_alphanumeric()).count();
            for line in text.split('\n') {
                col.add(line)?;
            }
        }
    }
    // a scan with only page numbers: fewer than 8 meaningful characters per page
    if meaningful_total < 8 * pages.len() {
        return Err(ExtractError::NoText);
    }
    Ok(col.finish(FileKind::Pdf, pages.len(), None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::testgen;

    const EN_1: &str = "Which tool keeps a stock pot at a steady simmer";
    const EN_2: &str = "A second page about knife storage and safety";
    const ZH: &str = "请问哪种工具可以让汤锅保持稳定的小火炖煮";
    const JA: &str = "ストックポットを一定の弱火で保つ道具はどれですか";

    fn text_of(doc: &Doc) -> String {
        doc.sections
            .iter()
            .flat_map(|s| s.blocks.iter())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn two_latin_pages_become_two_sections() {
        let pdf = testgen::pdf_latin(&[EN_1, EN_2]);
        let doc = extract(&pdf, &Limits::default()).unwrap();
        assert_eq!(doc.pages, 2);
        assert_eq!(doc.sections.len(), 2);
        assert_eq!(doc.sections[0].label, "p. 1");
        assert!(text_of(&doc).contains(EN_1), "{}", text_of(&doc));
        assert!(doc.sections[1].blocks.join(" ").contains("knife storage"));
    }

    #[test]
    fn chinese_and_japanese_text_comes_back_without_spaces_between_characters() {
        for s in [ZH, JA] {
            let pdf = testgen::pdf_cjk(s);
            let doc = extract(&pdf, &Limits::default()).unwrap();
            let t = text_of(&doc).replace('\n', "");
            assert!(t.contains(s), "{t:?} should contain {s:?}");
        }
    }

    #[test]
    fn pages_without_text_are_no_text() {
        assert_eq!(
            extract(&testgen::pdf_empty(3), &Limits::default()),
            Err(ExtractError::NoText)
        );
        // page numbers only
        let nums: Vec<String> = (1..=3).map(|n| n.to_string()).collect();
        let refs: Vec<&str> = nums.iter().map(String::as_str).collect();
        assert_eq!(
            extract(&testgen::pdf_latin(&refs), &Limits::default()),
            Err(ExtractError::NoText)
        );
    }

    #[test]
    fn an_encrypted_file_that_needs_a_password_is_protected() {
        assert_eq!(
            extract(&testgen::pdf_encrypted_junk(), &Limits::default()),
            Err(ExtractError::Protected)
        );
    }

    #[test]
    fn too_many_pages_garbage_and_truncation() {
        assert_eq!(
            extract(&testgen::pdf_pages(21), &Limits::small()),
            Err(ExtractError::TooComplex)
        );
        assert_eq!(
            extract(
                b"%PDF-1.4\nthis is not a pdf body at all",
                &Limits::default()
            ),
            Err(ExtractError::Corrupt)
        );
        let full = testgen::pdf_latin(&[EN_1]);
        let cut = &full[..full.len() / 2];
        assert!(matches!(
            extract(cut, &Limits::default()),
            Err(ExtractError::Corrupt) | Err(ExtractError::NoText)
        ));
    }

    #[test]
    fn a_library_panic_is_corrupt() {
        assert_eq!(
            extract(&testgen::pdf_bad_tounicode(), &Limits::default()),
            Err(ExtractError::Corrupt)
        );
    }
}
