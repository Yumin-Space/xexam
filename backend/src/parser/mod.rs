//! XLSX 解析。**接口契约（不要改签名）**：
//!
//! - `check_xlsx(&[u8]) -> Result<(), ImportError>`      上传内容检查（对应旧版 `server.check_xlsx`）
//! - `parse_workbook(&[u8]) -> Result<ParsedWorkbook, ImportError>`   对应旧版 `xlsx_parser.parse_workbook`
//! - `make_id(sheet, stem, first) -> String`             题目 id（学习记录的键，**必须与旧版逐字节一致**）
//! - `py_space::is_py_space(char) -> bool`               Python `\s` 的 29 个码点
//!
//! 行为依据：`docs/audit/xlsx-parser.md`、`tests/baseline/parser_golden/`（50 个夹具的旧版输出）、
//! `tests/baseline/golden/id_vectors.json`（85 条 id 断言）。
use crate::error::ImportError;
use crate::models::ParsedWorkbook;

pub fn check_xlsx(_data: &[u8]) -> Result<(), ImportError> {
    unimplemented!("parser::check_xlsx")
}

pub fn parse_workbook(_data: &[u8]) -> Result<ParsedWorkbook, ImportError> {
    unimplemented!("parser::parse_workbook")
}

pub fn make_id(_sheet: &str, _stem: &str, _first: &str) -> String {
    unimplemented!("parser::make_id")
}
