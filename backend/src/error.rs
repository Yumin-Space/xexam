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

impl ImportError {
    /// An English reason for the log (the `Display` text is the Chinese message the page shows; the log is English).
    pub fn log_reason(&self) -> &'static str {
        match self {
            ImportError::Empty => "the file is empty or was not uploaded completely",
            ImportError::LegacyXls => "an old .xls file or a password-protected Excel file",
            ImportError::NotXlsx => "not an .xlsx file",
            ImportError::Corrupt => "the .xlsx file is damaged",
            ImportError::NoWorkbook => "the file holds no Excel workbook",
            ImportError::TooBigUncompressed => "too large once decompressed",
            ImportError::EntityDeclaration => "the file contains an XML entity declaration",
            ImportError::Timeout => "reading the file took longer than the time limit",
            ImportError::Message(_) => {
                "the workbook could not be read (not a valid .xlsx, a missing part, malformed or too complex XML, or an overlong answer)"
            }
            ImportError::Unexpected => "content the parser did not expect",
        }
    }
}

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// A blocking database task ended abnormally (panicked or was cancelled). Only ever logged: the response is
    /// `MSG_INTERNAL` or `MSG_APPLY_FAILED`.
    #[error("the database task ended abnormally")]
    Join,
    /// The consistent copy (`VACUUM INTO`) written before a schema migration could not be made; the database was not migrated.
    #[error("backup before the schema migration failed: {0}")]
    Backup(String),
    /// Argon2 hashing failed (only for the default administrator created by the schema-v3 migration).
    #[error("{0}")]
    Password(String),
    /// The engine does not implement this operation (default trait methods of test engines).
    #[error("not supported by this engine")]
    Unsupported,
    /// A per-user write for a user who was deleted or disabled after the request's session was checked; nothing was written.
    /// The API answers 401 `MSG_SIGN_IN_REQUIRED`.
    #[error("the user no longer exists or is disabled")]
    NoUser,
    /// An exam whose questions no longer exist (their bank was deleted after the exam started, or the ids were made up);
    /// nothing was written. The API answers 409 `MSG_EXAM_QUESTIONS_GONE`.
    #[error("none of the exam's questions exist any more")]
    QuestionsGone,
    /// A bank manager tried to update or delete a bank they do not own (checked inside the write transaction); nothing was
    /// written. The API answers 403 `MSG_NOT_BANK_OWNER`.
    #[error("the user may not manage this question bank")]
    NotBankOwner,
    /// A user's bank list names a bank that does not exist; nothing was written. Turned into `UserChange::UnknownBanks`
    /// (400 `MSG_UNKNOWN_BANKS`) by the users store; never leaves it.
    #[error("unknown question bank id")]
    UnknownBanks,
    /// v1.1: an Excel import aimed at an AI-generated bank (checked before parsing and again inside the write
    /// transaction); nothing was written. The API answers 400 `MSG_AI_BANK_IMPORT`.
    #[error("an AI-generated question bank cannot be updated from an Excel file")]
    AiBank,
    /// v1.1 (C-B1): a sheet deletion that names every sheet of the bank; nothing was written. 400 `MSG_SHEETS_ALL`.
    #[error("every sheet of the bank was named")]
    SheetsAll,
    /// v1.1 (C-B1): a sheet deletion aimed at an AI-generated bank (it has one sheet); nothing was written. 400
    /// `MSG_AI_BANK_SHEETS`.
    #[error("an AI-generated question bank has a single sheet")]
    AiBankSheets,
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
    /// 404 + Chinese reason (the bank / exam record does not exist, e.g. deleted in another tab)
    #[error("{0}")]
    NotFoundMsg(&'static str),
    /// 409: the preview has expired or was already handled
    #[error("pending gone")]
    PendingGone,
    /// 403: the app cannot be shut down from the page when running in Docker
    #[error("shutdown disabled")]
    ShutdownDisabled,
    /// v0.6 accounts and sign-in: a status code with a fixed Chinese message (401 sign in, 403 admin only / password change
    /// required / wrong current password, 404 unknown user, 409 username taken / last admin / not yourself, 429 throttled)
    #[error("{1}")]
    Status(u16, &'static str),
    /// 429 `MSG_TOO_MANY_LOGINS` with the seconds to wait (`"retryAfter"` in the body and the `Retry-After` header): a sign-in or
    /// a current-password check refused by the login throttle
    #[error("too many sign-in attempts")]
    TooManyLogins { retry_after: u64 },
    /// v0.8: 409 `{"error": MSG_AI_SERVICE_OFF, "code": "ai.serviceOff"}`: a user's AI+ service switch was sent while AI+
    /// is not available (ai-service not connected, or its AI+ service off); nothing was written
    #[error("AI+ is not available")]
    AiServiceOff,
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
