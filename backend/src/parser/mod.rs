//! XLSX 解析。**接口契约（不要改签名）**：
//!
//! - `check_xlsx(&[u8]) -> Result<(), ImportError>`      上传内容检查（对应旧版 `server.check_xlsx`）
//! - `parse_workbook(&[u8]) -> Result<ParsedWorkbook, ImportError>`   对应旧版 `xlsx_parser.parse_workbook`
//! - `make_id(sheet, stem, first) -> String`             题目 id（学习记录的键，**必须与旧版逐字节一致**）
//! - `py_space::is_py_space(char) -> bool`               Python `\s` 的 29 个码点
//!
//! 行为依据：`docs/audit/xlsx-parser.md`、`tests/baseline/parser_golden/`（50 个夹具的旧版输出）、
//! `tests/baseline/golden/id_vectors.json`（85 条 id 断言）。
//!
//! 内部分层：`zip_guard`（zip 层与上限）→ `xml`（quick-xml 事件 → 与 ElementTree 等价的小树）→
//! `workbook`（工作簿 / 共享字符串 / 单元格）→ `question`（纯字符串的题目解析）。
use crate::error::ImportError;
use crate::models::ParsedWorkbook;

mod digits_table;
mod id;
pub mod py_space;
mod question;
mod workbook;
mod xml;
mod zip_guard;

/// 上传内容检查：必须像一个正常的 .xlsx，没有压缩炸弹，没有 DOCTYPE / 实体声明。
pub fn check_xlsx(data: &[u8]) -> Result<(), ImportError> {
    zip_guard::check_xlsx(data)
}

/// 解析整份工作簿（`sheets` + `questions`）。
pub fn parse_workbook(data: &[u8]) -> Result<ParsedWorkbook, ImportError> {
    workbook::parse_workbook(data)
}

/// 同 `parse_workbook`，但解析器会在 `deadline` 到了之后自己停下（返回 `ImportError::Timeout`）。
/// 上传接口用它实现 `XEXAM_PARSE_TIMEOUT_SECS`，这样超时之后后台线程不会继续占着 CPU 和内存。
pub fn parse_workbook_until(
    data: &[u8],
    deadline: Option<std::time::Instant>,
) -> Result<ParsedWorkbook, ImportError> {
    workbook::parse_workbook_until(data, deadline)
}

/// 题目 id：`sha1(sheet + "\n" + 去空白(stem)[:300] + "|" + 去空白(first)[:200])` 的前 12 位十六进制。
pub fn make_id(sheet: &str, stem: &str, first: &str) -> String {
    id::make_id(sheet, stem, first)
}
