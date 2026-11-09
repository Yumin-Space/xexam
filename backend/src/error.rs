//! 错误类型。对外（HTTP）的形态与旧版一致：
//! - GET 被 Host 守卫拒绝：403 `text/plain`（`Forbidden host: …`）
//! - POST 被拒绝：403 JSON `{"error": "<中文原因>"}`
//! - 其余错误：JSON `{"error": "<文案>"}`，状态码见 `status()`
//!
//! 内部细节（异常文本、路径、SQLite 报错）只进日志，不进响应。

use thiserror::Error;

/// 解析 XLSX 时的错误。变体的 `Display` 就是旧版 `ValueError` 的中文文案（前端直接显示），**逐字保留**。
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
    /// 解析阶段其它的、有明确中文说明的失败（例如工作簿结构不对）。
    #[error("{0}")]
    Message(String),
    /// 解析器没预料到的内容。对外是"读取这个 Excel 时出错：…"，细节只进日志。
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

/// HTTP 层的错误。`IntoResponse` 在 `api::error_response` 里实现。
#[derive(Debug, Error)]
pub enum AppError {
    /// GET 被 Host 守卫拒绝
    #[error("forbidden host")]
    ForbiddenHost { seen: String },
    /// POST 被 Host / Origin 守卫拒绝，带可照着做的中文原因
    #[error("denied")]
    Denied(String),
    #[error("bad content type")]
    UnsupportedMediaType,
    #[error("too large")]
    JsonTooLarge,
    /// 上传超过 20 MB
    #[error("upload too large")]
    UploadTooLarge { max_mb: u64 },
    #[error("bad json")]
    BadJson,
    /// 400 + 中文原因（xlsx 校验、没有成绩可保存、没有可用的题……）
    #[error("{0}")]
    BadRequest(String),
    #[error("not found")]
    NotFound,
    /// 409：预览已过期或已被处理
    #[error("pending gone")]
    PendingGone,
    /// 403：在 Docker 里不能用页面关闭程序
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
