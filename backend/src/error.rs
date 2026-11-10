//! Error types. The externally visible (HTTP) shape matches the old version:
//! - GET rejected by the Host guard: 403 `text/plain` (`Forbidden host: …`)
//! - POST rejected: 403 JSON `{"error": "<Chinese reason>"}`
//! - Other errors: JSON `{"error": "<message>"}`, status code see `status()`
//!
//! Internal details (exception text, paths, SQLite errors) go only to the log, never into the response.

use thiserror::Error;

/// Errors while parsing an XLSX. The `Display` of each variant is the Chinese message of the old `ValueError` (shown directly by the frontend), **kept verbatim**.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ImportError {
    #[error("文件是空的，或者没有上传完整。")]
    Empty,
    #[error("这是旧版 .xls 格式，或者是设置了密码的 Excel 文件。请在 Excel 里去掉密码，并另存为「Excel 工作簿(.xlsx)」再上传。")]
    LegacyXls,
    #[error("这个文件不是 .xlsx（可能是 .csv、.pdf 或别的格式）。请在 Excel 里另存为「Excel 工作簿(.xlsx)」再上传。")]
    NotXlsx,
    #[error("这个 .xlsx 文件已损坏，打不开。请用 Excel 重新保存一份再上传。")]
    Corrupt,
    #[error("这个文件里没有 Excel 工作簿的内容，不是有效的 .xlsx。")]
    NoWorkbook,
    #[error("文件解压后太大，已拒绝。")]
    TooBigUncompressed,
    #[error("文件里含有不被允许的内容（XML 实体声明），已拒绝。正常的 Excel 文件不会有这个。")]
    EntityDeclaration,
    /// Parsing exceeded the time limit (`XEXAM_PARSE_TIMEOUT_SECS`). The parser is cooperative: it stops itself when it sees the deadline.
    #[error("解析这个文件用的时间太长，已经停止。请确认选对了文件，或者换一份小一点的再试。")]
    Timeout,
    /// Other parse-stage failures that have an explicit Chinese message (e.g. a malformed workbook structure).
    #[error("{0}")]
    Message(String),
    /// Content the parser did not anticipate. Externally shown as the "error while reading this Excel file: …" message; details go only to the log.
    #[error("读取这个 Excel 时出错")]
    Unexpected,
}

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("数据库任务异常结束")]
    Join,
}

/// Errors at the HTTP layer. `IntoResponse` is implemented in `api::error_response`.
#[derive(Debug, Error)]
pub enum AppError {
    /// GET rejected by the Host guard
    #[error("forbidden host")]
    ForbiddenHost { seen: String },
    /// POST rejected by the Host / Origin guard, with an actionable Chinese reason
    #[error("denied")]
    Denied(String),
    #[error("bad content type")]
    UnsupportedMediaType,
    #[error("too large")]
    JsonTooLarge,
    /// Upload exceeds 20 MB
    #[error("upload too large")]
    UploadTooLarge { max_mb: u64 },
    #[error("bad json")]
    BadJson,
    /// 400 + Chinese reason (xlsx validation, no results to save, no usable questions, ...)
    #[error("{0}")]
    BadRequest(String),
    #[error("not found")]
    NotFound,
    /// 409: the preview has expired or was already handled
    #[error("pending gone")]
    PendingGone,
    /// 403: the app cannot be shut down from the page when running in Docker
    #[error("shutdown disabled")]
    ShutdownDisabled,
    #[error("db error")]
    Db(#[from] DbError),
    #[error("internal")]
    Internal,
}

impl From<ImportError> for AppError {
    fn from(e: ImportError) -> Self {
        match e {
            ImportError::Unexpected => {
                AppError::BadRequest("读取这个 Excel 时出错：文件内容不符合预期".to_string())
            }
            other => AppError::BadRequest(other.to_string()),
        }
    }
}
