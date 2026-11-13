//! Text extraction for the AI file import (spec IX-1 .. IX-6). Pure and synchronous: bytes in, sections of text out. No
//! AI, no network, no database, nothing written to disk. It runs inside a killed-after-30-s child process of
//! ai-service (`child`), so a malformed file can never crash or hang the service itself.
//!
//! Every parser is bounded: zip entries and member sizes (a declared size is never trusted), XML depth / attributes /
//! events (a DOCTYPE is refused, entities are never expanded), pages, sheets, cells, slides, fields and the extracted
//! text. The numbers are in [`Limits`] and match the table of IX-6.
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub mod child;
pub mod clean;
pub mod csv;
pub mod docx;
pub mod pdf;
pub mod pptx;
pub mod sniff;
#[doc(hidden)]
pub mod testgen;
pub mod textenc;
pub mod xlsx;
pub mod xmlx;
pub mod zipx;

/// The seven supported kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Pdf,
    Docx,
    Xlsx,
    Pptx,
    Md,
    Txt,
    Csv,
}

impl FileKind {
    /// The wire word (also the Job's `file.type`).
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::Pdf => "pdf",
            FileKind::Docx => "docx",
            FileKind::Xlsx => "xlsx",
            FileKind::Pptx => "pptx",
            FileKind::Md => "md",
            FileKind::Txt => "txt",
            FileKind::Csv => "csv",
        }
    }

    /// The kind of a lower-case extension (`markdown` is `md`).
    pub fn from_ext(ext: &str) -> Option<FileKind> {
        match ext {
            "pdf" => Some(FileKind::Pdf),
            "docx" => Some(FileKind::Docx),
            "xlsx" => Some(FileKind::Xlsx),
            "pptx" => Some(FileKind::Pptx),
            "md" | "markdown" => Some(FileKind::Md),
            "txt" => Some(FileKind::Txt),
            "csv" => Some(FileKind::Csv),
            _ => None,
        }
    }
}

/// A part of the file: a PDF page, a sheet, a slide, or the whole of a text file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub label: String,
    pub blocks: Vec<String>,
}

/// The extracted text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Doc {
    pub kind: FileKind,
    pub sections: Vec<Section>,
    /// Unicode scalar values of all blocks
    pub chars: usize,
    /// Characters with `char::is_alphanumeric`
    pub meaningful: usize,
    /// PDF pages, slides, sheets; 0 for text files
    pub pages: usize,
    pub encoding: Option<String>,
}

/// Why a file cannot be read. `code()` is the suffix of the `ai.import.*` job codes (IM-9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtractError {
    Empty,
    TooLarge,
    Unsupported,
    Legacy,
    Image,
    Protected,
    Corrupt,
    TooComplex,
    NoText,
    TooMuchText,
    BadEncoding,
    Timeout,
}

/// Every variant, for the code round trip.
pub const ALL_ERRORS: [ExtractError; 12] = [
    ExtractError::Empty,
    ExtractError::TooLarge,
    ExtractError::Unsupported,
    ExtractError::Legacy,
    ExtractError::Image,
    ExtractError::Protected,
    ExtractError::Corrupt,
    ExtractError::TooComplex,
    ExtractError::NoText,
    ExtractError::TooMuchText,
    ExtractError::BadEncoding,
    ExtractError::Timeout,
];

impl ExtractError {
    pub fn code(self) -> &'static str {
        match self {
            ExtractError::Empty => "empty",
            ExtractError::TooLarge => "tooLarge",
            ExtractError::Unsupported => "type",
            ExtractError::Legacy => "legacy",
            ExtractError::Image => "image",
            ExtractError::Protected => "protected",
            ExtractError::Corrupt => "corrupt",
            ExtractError::TooComplex => "tooComplex",
            ExtractError::NoText => "noText",
            ExtractError::TooMuchText => "tooMuchText",
            ExtractError::BadEncoding => "encoding",
            ExtractError::Timeout => "readTimeout",
        }
    }

    pub fn from_code(code: &str) -> Option<ExtractError> {
        ALL_ERRORS.into_iter().find(|e| e.code() == code)
    }
}

/// The UI language hint that orders the guesses for a text file without a byte order mark (IX-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LangHint {
    Zh,
    En,
    Ja,
}

impl LangHint {
    pub fn parse(s: &str) -> Option<LangHint> {
        match s {
            "zh" | "zh-CN" => Some(LangHint::Zh),
            "en" => Some(LangHint::En),
            "ja" => Some(LangHint::Ja),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            LangHint::Zh => "zh",
            LangHint::En => "en",
            LangHint::Ja => "ja",
        }
    }
}

/// Every number of IX-1 .. IX-6. `Default` is the binding set; tests lower them.
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_chars: usize,
    pub min_meaningful: usize,
    pub zip_entries: usize,
    pub zip_member: u64,
    pub zip_total: u64,
    pub xml_depth: usize,
    pub xml_attrs: usize,
    pub xml_events_per_part: u64,
    pub xml_events_total: u64,
    pub pdf_pages: usize,
    pub xlsx_sheets: usize,
    pub xlsx_cells: usize,
    pub xlsx_shared: usize,
    pub pptx_slides: usize,
    pub csv_fields: usize,
    pub csv_field_chars: usize,
    /// Cooperative deadline, checked every 4 096 XML events, per PDF page and per 1 000 rows / records
    pub deadline: Option<Instant>,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_chars: 400_000,
            min_meaningful: 20,
            zip_entries: 10_000,
            zip_member: 32 << 20,
            zip_total: 128 << 20,
            xml_depth: 256,
            xml_attrs: 256,
            xml_events_per_part: 4_000_000,
            xml_events_total: 16_000_000,
            pdf_pages: 2_000,
            xlsx_sheets: 200,
            xlsx_cells: 500_000,
            xlsx_shared: 500_000,
            pptx_slides: 1_000,
            csv_fields: 1_000,
            csv_field_chars: 100_000,
            deadline: None,
        }
    }
}

impl Limits {
    /// Small numbers for tests (a made-up file hits every cap without being large).
    pub fn small() -> Limits {
        Limits {
            max_chars: 2_000,
            min_meaningful: 20,
            zip_entries: 50,
            zip_member: 64 << 10,
            zip_total: 256 << 10,
            xml_depth: 16,
            xml_attrs: 16,
            xml_events_per_part: 5_000,
            xml_events_total: 20_000,
            pdf_pages: 20,
            xlsx_sheets: 4,
            xlsx_cells: 50,
            xlsx_shared: 50,
            pptx_slides: 4,
            csv_fields: 8,
            csv_field_chars: 200,
            deadline: None,
        }
    }

    /// `Err(Timeout)` once the cooperative deadline has passed.
    pub fn check_deadline(&self) -> Result<(), ExtractError> {
        match self.deadline {
            Some(d) if Instant::now() >= d => Err(ExtractError::Timeout),
            _ => Ok(()),
        }
    }
}

/// Full detection (IX-2), including zip members.
pub fn detect(bytes: &[u8], ext: &str) -> Result<FileKind, ExtractError> {
    sniff::detect(bytes, ext)
}

/// Detection by the extension and the first bytes only, no zip parsing (the upload route's early refusal).
pub fn sniff_magic(bytes: &[u8], ext: &str) -> Result<FileKind, ExtractError> {
    sniff::sniff_magic(bytes, ext)
}

/// Collects blocks into sections while counting characters (stops at `max_chars`).
pub(crate) struct Collector {
    pub sections: Vec<Section>,
    chars: usize,
    meaningful: usize,
    max_chars: usize,
}

impl Collector {
    pub fn new(max_chars: usize) -> Collector {
        Collector {
            sections: Vec::new(),
            chars: 0,
            meaningful: 0,
            max_chars,
        }
    }

    pub fn begin(&mut self, label: &str) {
        self.sections.push(Section {
            label: label.to_string(),
            blocks: Vec::new(),
        });
    }

    /// Cleans and adds a block to the current section; `Err(TooMuchText)` as soon as the total passes the cap.
    pub fn add(&mut self, raw: &str) -> Result<(), ExtractError> {
        let text = clean::clean_text(raw);
        if text.is_empty() {
            return Ok(());
        }
        for c in text.chars() {
            self.chars += 1;
            if c.is_alphanumeric() {
                self.meaningful += 1;
            }
        }
        if self.chars > self.max_chars {
            return Err(ExtractError::TooMuchText);
        }
        if self.sections.is_empty() {
            self.begin("");
        }
        if let Some(s) = self.sections.last_mut() {
            s.blocks.push(text);
        }
        Ok(())
    }

    pub fn finish(self, kind: FileKind, pages: usize, encoding: Option<String>) -> Doc {
        Doc {
            kind,
            sections: self.sections,
            chars: self.chars,
            meaningful: self.meaningful,
            pages,
            encoding,
        }
    }
}

/// Reads the text of a file (IX-3). The kind is decided by [`detect`] (content wins over the extension for binary
/// kinds); an empty input is `Empty`. `NoText` when fewer than `min_meaningful` meaningful characters remain.
pub fn extract(
    bytes: &[u8],
    ext: &str,
    hint: LangHint,
    limits: &Limits,
) -> Result<Doc, ExtractError> {
    if bytes.is_empty() {
        return Err(ExtractError::Empty);
    }
    let ext = ext.to_ascii_lowercase();
    let kind = sniff::detect_with(bytes, &ext, limits)?;
    let doc = match kind {
        FileKind::Pdf => pdf::extract(bytes, limits)?,
        FileKind::Docx => docx::extract(bytes, limits)?,
        FileKind::Xlsx => xlsx::extract(bytes, limits)?,
        FileKind::Pptx => pptx::extract(bytes, limits)?,
        FileKind::Md | FileKind::Txt => {
            let (text, enc) = textenc::decode(bytes, hint)?;
            let mut col = Collector::new(limits.max_chars);
            col.begin("");
            for (n, line) in text.split('\n').enumerate() {
                if n % 1000 == 0 {
                    limits.check_deadline()?;
                }
                col.add(line)?;
            }
            col.finish(kind, 0, Some(enc.to_string()))
        }
        FileKind::Csv => csv::extract(bytes, hint, limits)?,
    };
    if doc.meaningful < limits.min_meaningful {
        return Err(ExtractError::NoText);
    }
    Ok(doc)
}
