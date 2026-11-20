//! v1.6 bank download: one question bank as an Excel workbook in the import format ([`xlsx`], imports again as it is) or as
//! a Word document for reading and printing ([`docx`]). `GET /api/banks/download` reads the bank with
//! `Store::export_bank_as` and builds the file with [`export`] in a blocking task.
//!
//! v1.9: the download can be limited to some of the three languages ([`Langs`], query `langs=en,zh,ja`). Excel keeps all
//! six columns in place and leaves the cells of unticked languages empty; Word holds only the ticked languages. A subset
//! adds `_<codes>` to the file name.
use crate::models::Question;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};

pub mod docx;
pub mod xlsx;

/// One bank as the download exports it: its active questions, grouped by sheet, sheets in the order of their first
/// question (`seq`), questions in `seq` order. Each `Question` has `sheet` and `row` set; `id` is empty (never exported).
#[derive(Debug, Clone, PartialEq)]
pub struct ExportBank {
    pub id: i64,
    pub name: String,
    pub sheets: Vec<ExportSheet>,
}

/// One sheet of an [`ExportBank`] (never empty: a sheet without active questions is left out).
#[derive(Debug, Clone, PartialEq)]
pub struct ExportSheet {
    pub name: String,
    pub questions: Vec<Question>,
}

impl ExportBank {
    /// Number of questions over all sheets.
    pub fn question_count(&self) -> usize {
        self.sheets.iter().map(|s| s.questions.len()).sum()
    }
}

/// The two download formats (`format=xlsx|docx`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Xlsx,
    Docx,
}

impl ExportFormat {
    /// `"xlsx"` / `"docx"` (exact, lower case); anything else is `None`.
    pub fn parse(s: &str) -> Option<ExportFormat> {
        match s {
            "xlsx" => Some(ExportFormat::Xlsx),
            "docx" => Some(ExportFormat::Docx),
            _ => None,
        }
    }

    /// The query value, file extension and activity `detail.format`.
    pub fn as_str(self) -> &'static str {
        match self {
            ExportFormat::Xlsx => "xlsx",
            ExportFormat::Docx => "docx",
        }
    }

    /// The `Content-Type` of the download.
    pub fn mime(self) -> &'static str {
        match self {
            ExportFormat::Xlsx => {
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            }
            ExportFormat::Docx => {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            }
        }
    }
}

/// Building a file failed (zip or I/O error). The route answers 500 and logs the text.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ExportError(pub String);

impl From<std::io::Error> for ExportError {
    fn from(e: std::io::Error) -> Self {
        ExportError(e.to_string())
    }
}

impl From<zip::result::ZipError> for ExportError {
    fn from(e: zip::result::ZipError) -> Self {
        ExportError(e.to_string())
    }
}

/// The languages a download holds (`langs=en,zh,ja`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Langs {
    pub en: bool,
    pub zh: bool,
    pub ja: bool,
}

impl Langs {
    /// All three languages (the v1.8 download).
    pub const ALL: Langs = Langs {
        en: true,
        zh: true,
        ja: true,
    };

    /// The `langs` query value: comma-separated, each token exactly `en`, `zh` or `ja`, 1-3 tokens, no empty token and no
    /// duplicate. Anything else (empty value, other code, upper case, repeated code) is `None`.
    pub fn parse(s: &str) -> Option<Langs> {
        let mut l = Langs {
            en: false,
            zh: false,
            ja: false,
        };
        for tok in s.split(',') {
            let slot = match tok {
                "en" => &mut l.en,
                "zh" => &mut l.zh,
                "ja" => &mut l.ja,
                _ => return None,
            };
            if *slot {
                return None;
            }
            *slot = true;
        }
        Some(l)
    }

    /// True when all three languages are set.
    pub fn is_all(self) -> bool {
        self.en && self.zh && self.ja
    }

    /// The set codes in `en`, `zh`, `ja` order.
    pub fn codes(self) -> Vec<&'static str> {
        let mut v = Vec::with_capacity(3);
        if self.en {
            v.push("en");
        }
        if self.zh {
            v.push("zh");
        }
        if self.ja {
            v.push("ja");
        }
        v
    }

    /// The file name suffix (`en-zh`); `None` when all three are set.
    pub fn suffix(self) -> Option<String> {
        if self.is_all() {
            None
        } else {
            Some(self.codes().join("-"))
        }
    }
}

/// Builds the file of `bank` in `format` with all three languages. `now` is the creation time written into the document
/// properties.
pub fn export(
    bank: &ExportBank,
    format: ExportFormat,
    now: jiff::Timestamp,
) -> Result<Vec<u8>, ExportError> {
    export_with(bank, format, Langs::ALL, now)
}

/// Builds the file of `bank` in `format` holding only `langs`.
pub fn export_with(
    bank: &ExportBank,
    format: ExportFormat,
    langs: Langs,
    now: jiff::Timestamp,
) -> Result<Vec<u8>, ExportError> {
    match format {
        ExportFormat::Xlsx => xlsx::write_xlsx_langs(bank, langs),
        ExportFormat::Docx => docx::write_docx_langs(bank, langs, now),
    }
}

/// Text for an XML element or attribute value: `& < > " '` escaped. Characters XML 1.0 does not allow (C0 controls other
/// than tab, LF and CR, U+FFFE, U+FFFF) are dropped.
pub fn xml_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(ch),
            c if c < ' ' || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}

/// The bank name as a file name without extension: control characters and `/ \ : * ? " < > |` become `_`, at most 120
/// characters, leading and trailing white space and dots removed; `"bank"` when nothing is left.
pub fn file_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .take(120)
        .collect();
    let t = cleaned.trim_matches(|c: char| c.is_whitespace() || c == '.');
    if t.is_empty() {
        "bank".to_string()
    } else {
        t.to_string()
    }
}

/// `<file_stem(name)>.<ext>`, or `<file_stem(name)>_<codes>.<ext>` when `langs` is a subset.
pub fn file_name(bank: &ExportBank, format: ExportFormat, langs: Langs) -> String {
    match langs.suffix() {
        None => format!("{}.{}", file_stem(&bank.name), format.as_str()),
        Some(sfx) => format!("{}_{sfx}.{}", file_stem(&bank.name), format.as_str()),
    }
}

/// RFC 5987 `attr-char`: everything else is percent-encoded in `filename*`.
const ATTR_CHAR: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'!')
    .remove(b'#')
    .remove(b'$')
    .remove(b'&')
    .remove(b'+')
    .remove(b'-')
    .remove(b'.')
    .remove(b'^')
    .remove(b'_')
    .remove(b'`')
    .remove(b'|')
    .remove(b'~');

/// `attachment; filename="<ascii>"; filename*=UTF-8''<percent-encoded file_name>` (RFC 6266 / RFC 5987). The ASCII
/// fallback is the file name itself when it is printable ASCII, else `xexam-bank-<id>[_<codes>].<ext>`.
pub fn content_disposition(bank: &ExportBank, format: ExportFormat, langs: Langs) -> String {
    let name = file_name(bank, format, langs);
    let ascii = if name.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        name.clone()
    } else {
        match langs.suffix() {
            None => format!("xexam-bank-{}.{}", bank.id, format.as_str()),
            Some(sfx) => format!("xexam-bank-{}_{sfx}.{}", bank.id, format.as_str()),
        }
    };
    format!(
        "attachment; filename=\"{ascii}\"; filename*=UTF-8''{}",
        utf8_percent_encode(&name, ATTR_CHAR)
    )
}
