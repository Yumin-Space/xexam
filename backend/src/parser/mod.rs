//! XLSX parsing. **Interface contract (do not change the signatures)**:
//!
//! - `check_xlsx(&[u8]) -> Result<(), ImportError>`      upload content check (corresponds to the legacy `server.check_xlsx`)
//! - `parse_workbook(&[u8]) -> Result<ParsedWorkbook, ImportError>`   corresponds to the legacy `xlsx_parser.parse_workbook`
//! - `make_id(sheet, stem, first) -> String`             question id (the key of study records, **must be byte-for-byte identical to the legacy version**)
//! - `py_space::is_py_space(char) -> bool`               the 29 code points of Python `\s`
//!
//! Behavior is based on: `docs/audit/xlsx-parser.md`, `tests/baseline/parser_golden/` (legacy output of 50 fixtures),
//! `tests/baseline/golden/id_vectors.json` (85 id assertions).
//!
//! Internal layers: `zip_guard` (zip layer and limits) -> `xml` (quick-xml events -> a small tree equivalent to ElementTree) ->
//! `workbook` (workbook / shared strings / cells) -> `question` (question parsing on plain strings).
use crate::error::ImportError;
use crate::models::ParsedWorkbook;

mod digits_table;
mod id;
pub mod py_space;
mod question;
mod workbook;
mod xml;
mod zip_guard;

/// Upload content check: must look like a normal .xlsx, with no compression bomb and no DOCTYPE / entity declarations.
pub fn check_xlsx(data: &[u8]) -> Result<(), ImportError> {
    zip_guard::check_xlsx(data)
}

/// Parses the whole workbook (`sheets` + `questions`).
pub fn parse_workbook(data: &[u8]) -> Result<ParsedWorkbook, ImportError> {
    workbook::parse_workbook(data)
}

/// Same as `parse_workbook`, but the parser stops by itself once `deadline` has passed (returning `ImportError::Timeout`).
/// The upload endpoint uses it to implement `XEXAM_PARSE_TIMEOUT_SECS`, so that after a timeout the background thread does not keep holding CPU and memory.
pub fn parse_workbook_until(
    data: &[u8],
    deadline: Option<std::time::Instant>,
) -> Result<ParsedWorkbook, ImportError> {
    workbook::parse_workbook_until(data, deadline)
}

/// Question id: the first 12 hex digits of `sha1(sheet + "\n" + strip_ws(stem)[:300] + "|" + strip_ws(first)[:200])`.
pub fn make_id(sheet: &str, stem: &str, first: &str) -> String {
    id::make_id(sheet, stem, first)
}
