//! File type detection (IX-2): the extension lists, then the content. Content wins over the extension for the binary
//! kinds (a PDF named `.docx` is read as a PDF).
use super::{zipx, ExtractError, FileKind, Limits};

/// Old binary Office and similar formats.
pub const LEGACY_EXTS: [&str; 7] = ["doc", "xls", "ppt", "rtf", "wps", "et", "dps"];
/// Pictures.
pub const IMAGE_EXTS: [&str; 10] = [
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif",
];
/// What an import accepts.
pub const IMPORT_EXTS: [&str; 8] = [
    "pdf", "docx", "xlsx", "pptx", "md", "markdown", "txt", "csv",
];

const OLE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// The lower-case extension after the last `.` ("" when none).
pub fn ext_of(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((_, e)) => e.to_ascii_lowercase(),
        None => String::new(),
    }
}

/// IX-2 step 1: the extension lists.
fn by_ext(ext: &str) -> Result<(), ExtractError> {
    if LEGACY_EXTS.contains(&ext) {
        Err(ExtractError::Legacy)
    } else if IMAGE_EXTS.contains(&ext) {
        Err(ExtractError::Image)
    } else if !IMPORT_EXTS.contains(&ext) {
        Err(ExtractError::Unsupported)
    } else {
        Ok(())
    }
}

/// The extension rules alone (IX-2 step 1): `Legacy`, `Image` or `Unsupported`.
pub fn check_ext(ext: &str) -> Result<(), ExtractError> {
    by_ext(&ext.to_ascii_lowercase())
}

fn is_image_magic(b: &[u8]) -> bool {
    b.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
        || b.starts_with(&[0xFF, 0xD8, 0xFF])
        || b.starts_with(b"GIF87a")
        || b.starts_with(b"GIF89a")
        || (b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP")
        || (b.starts_with(b"BM") && b.len() > 14 && b[6..10] == [0, 0, 0, 0])
        || b.starts_with(&[0x49, 0x49, 0x2A, 0x00])
        || b.starts_with(&[0x4D, 0x4D, 0x00, 0x2A])
        || (b.len() >= 12
            && &b[4..8] == b"ftyp"
            && matches!(&b[8..12], b"heic" | b"heix" | b"mif1"))
}

fn text_kind(ext: &str) -> Option<FileKind> {
    match ext {
        "md" | "markdown" => Some(FileKind::Md),
        "txt" => Some(FileKind::Txt),
        "csv" => Some(FileKind::Csv),
        _ => None,
    }
}

/// A NUL byte in the first 8 KiB after the byte order mark.
fn looks_binary(b: &[u8]) -> bool {
    let b = b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(b);
    if b.starts_with(&[0xFF, 0xFE]) || b.starts_with(&[0xFE, 0xFF]) {
        return false; // UTF-16 text has NULs; the decoder checks it
    }
    b[..b.len().min(8192)].contains(&0)
}

enum Magic {
    Pdf,
    Zip,
    Legacy,
    Image,
    None,
}

fn magic(bytes: &[u8]) -> Magic {
    let head = &bytes[..bytes.len().min(1024)];
    if head.windows(5).any(|w| w == b"%PDF-") {
        Magic::Pdf
    } else if head.starts_with(&[b'P', b'K', 3, 4]) {
        Magic::Zip
    } else if head.starts_with(&OLE) || head.starts_with(b"{\\rtf") {
        Magic::Legacy
    } else if is_image_magic(head) {
        Magic::Image
    } else {
        Magic::None
    }
}

/// Steps 1 and 2 without zip parsing: for a zip the extension decides (`docx`, `xlsx`, `pptx`), any other extension
/// gives `Docx` as an undecided placeholder; the child's [`detect`] reads the members.
pub fn sniff_magic(bytes: &[u8], ext: &str) -> Result<FileKind, ExtractError> {
    let ext = ext.to_ascii_lowercase();
    by_ext(&ext)?;
    match magic(bytes) {
        Magic::Pdf => Ok(FileKind::Pdf),
        Magic::Zip => Ok(match ext.as_str() {
            "xlsx" => FileKind::Xlsx,
            "pptx" => FileKind::Pptx,
            _ => FileKind::Docx,
        }),
        Magic::Legacy => Err(ExtractError::Legacy),
        Magic::Image => Err(ExtractError::Image),
        Magic::None => match text_kind(&ext) {
            Some(k) if !looks_binary(bytes) => Ok(k),
            Some(_) => Err(ExtractError::Unsupported),
            // a binary extension without its magic bytes
            None => Err(ExtractError::Corrupt),
        },
    }
}

/// Full detection with the binding limits: [`sniff_magic`] plus the members of a zip.
pub fn detect(bytes: &[u8], ext: &str) -> Result<FileKind, ExtractError> {
    detect_with(bytes, ext, &Limits::default())
}

/// Full detection (`word/document.xml` → DOCX, `xl/workbook.xml` → XLSX, `ppt/presentation.xml` → PPTX, none →
/// `Corrupt`).
pub fn detect_with(bytes: &[u8], ext: &str, limits: &Limits) -> Result<FileKind, ExtractError> {
    let ext = ext.to_ascii_lowercase();
    by_ext(&ext)?;
    if let Magic::Zip = magic(bytes) {
        let pkg = zipx::ZipPkg::open(bytes, limits)?;
        return if pkg.has("word/document.xml") {
            Ok(FileKind::Docx)
        } else if pkg.has("xl/workbook.xml") {
            Ok(FileKind::Xlsx)
        } else if pkg.has("ppt/presentation.xml") {
            Ok(FileKind::Pptx)
        } else {
            Err(ExtractError::Corrupt)
        };
    }
    sniff_magic(bytes, &ext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::testgen;

    #[test]
    fn extension_lists_come_first() {
        for e in LEGACY_EXTS {
            assert_eq!(sniff_magic(b"x", e), Err(ExtractError::Legacy), "{e}");
        }
        for e in IMAGE_EXTS {
            assert_eq!(sniff_magic(b"x", e), Err(ExtractError::Image), "{e}");
        }
        assert_eq!(sniff_magic(b"x", "exe"), Err(ExtractError::Unsupported));
        assert_eq!(sniff_magic(b"x", ""), Err(ExtractError::Unsupported));
        assert_eq!(sniff_magic(b"hello", "TXT"), Ok(FileKind::Txt));
        assert_eq!(ext_of("A.b.PDF"), "pdf");
        assert_eq!(ext_of("noext"), "");
    }

    #[test]
    fn content_beats_the_extension_for_binary_kinds() {
        let pdf = b"%PDF-1.4\n1 0 obj";
        assert_eq!(sniff_magic(pdf, "docx"), Ok(FileKind::Pdf));
        assert_eq!(sniff_magic(pdf, "txt"), Ok(FileKind::Pdf));
        // the marker may sit up to 1 024 bytes in
        let mut late = vec![b' '; 1000];
        late.extend_from_slice(b"%PDF-1.7");
        assert_eq!(sniff_magic(&late, "pdf"), Ok(FileKind::Pdf));
        assert_eq!(sniff_magic(b"PK\x03\x04rest", "xlsx"), Ok(FileKind::Xlsx));
        assert_eq!(sniff_magic(b"PK\x03\x04rest", "pptx"), Ok(FileKind::Pptx));
        assert_eq!(sniff_magic(b"PK\x03\x04rest", "txt"), Ok(FileKind::Docx));
    }

    #[test]
    fn ole_rtf_and_images_are_refused_by_content() {
        let mut ole = OLE.to_vec();
        ole.extend_from_slice(&[0; 20]);
        assert_eq!(sniff_magic(&ole, "docx"), Err(ExtractError::Legacy));
        assert_eq!(
            sniff_magic(b"{\\rtf1 hi}", "txt"),
            Err(ExtractError::Legacy)
        );
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];
        assert_eq!(sniff_magic(&png, "pdf"), Err(ExtractError::Image));
        assert_eq!(
            sniff_magic(&[0xFF, 0xD8, 0xFF, 0xE0], "txt"),
            Err(ExtractError::Image)
        );
        assert_eq!(sniff_magic(b"GIF89a....", "txt"), Err(ExtractError::Image));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
        webp.extend_from_slice(&[0; 8]);
        assert_eq!(sniff_magic(&webp, "txt"), Err(ExtractError::Image));
        let mut heic = vec![0, 0, 0, 0x18];
        heic.extend_from_slice(b"ftypheic");
        heic.extend_from_slice(&[0; 8]);
        assert_eq!(sniff_magic(&heic, "md"), Err(ExtractError::Image));
    }

    #[test]
    fn text_files_and_binary_junk() {
        assert_eq!(sniff_magic(b"a,b\n1,2", "csv"), Ok(FileKind::Csv));
        assert_eq!(sniff_magic(b"# t", "markdown"), Ok(FileKind::Md));
        assert_eq!(
            sniff_magic(b"ab\0cd", "txt"),
            Err(ExtractError::Unsupported)
        );
        // UTF-16 text has NUL bytes and is fine
        assert_eq!(
            sniff_magic(&[0xFF, 0xFE, b'a', 0, b'b', 0], "txt"),
            Ok(FileKind::Txt)
        );
        // a binary extension without its magic bytes
        assert_eq!(
            sniff_magic(b"plain text", "pdf"),
            Err(ExtractError::Corrupt)
        );
        assert_eq!(
            sniff_magic(b"plain text", "docx"),
            Err(ExtractError::Corrupt)
        );
    }

    #[test]
    fn a_zip_is_decided_by_its_members_whatever_the_extension() {
        let docx = testgen::docx(&testgen::DocxSpec::paragraphs(&[
            "Which tool keeps a pot at a steady simmer?",
        ]));
        assert_eq!(detect(&docx, "txt"), Ok(FileKind::Docx));
        assert_eq!(detect(&docx, "xlsx"), Ok(FileKind::Docx));
        let xlsx = testgen::xlsx(&testgen::XlsxSpec::simple());
        assert_eq!(detect(&xlsx, "docx"), Ok(FileKind::Xlsx));
        let pptx = testgen::pptx(&testgen::PptxSpec::simple());
        assert_eq!(detect(&pptx, "pptx"), Ok(FileKind::Pptx));
        let plain_zip = testgen::zip_of(&[("readme.txt", b"hello")]);
        assert_eq!(detect(&plain_zip, "docx"), Err(ExtractError::Corrupt));
        // a PDF named .docx is read as a PDF
        assert_eq!(detect(b"%PDF-1.4 x", "docx"), Ok(FileKind::Pdf));
    }
}
