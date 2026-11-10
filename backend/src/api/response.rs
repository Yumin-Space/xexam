//! Unified response construction: every reply carries Content-Type, Content-Length, `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`,
//! and no `Server` header. A HEAD reply has exactly the same headers as GET (including the real Content-Length), just without a body.
use super::json::py_json;
use crate::error::AppError;
use axum::body::Body;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};

pub const JSON_CT: &str = "application/json; charset=utf-8";
pub const TEXT_CT: &str = "text/plain; charset=utf-8";

pub const MSG_UPLOAD_TOO_LARGE: &str =
    "文件太大了（最大 20 MB）。题库通常只有 1–2 MB，请确认选对了文件。";
pub const MSG_PENDING_GONE: &str = "这次上传的预览已经过期或已被处理，请重新上传。";
pub const MSG_SHUTDOWN_DISABLED: &str = "在 Docker 里请用 docker compose stop 停止容器。";
pub const MSG_INTERNAL: &str = "服务器内部出错，请稍后再试。";
pub const MSG_UPLOAD_INCOMPLETE: &str = "文件是空的，或者没有上传完整。";
// v0.5
pub const MSG_BANK_NOT_FOUND: &str = "这个题库不存在（可能已被删除），请刷新页面后再试。";
pub const MSG_RECORD_NOT_FOUND: &str = "这条考试记录不存在（可能已被删除），请刷新页面后再试。";
pub const MSG_BAD_TARGET: &str = "上传目标不对，请刷新页面后再试。";
pub const MSG_BAD_REQUEST: &str = "请求的格式不对。";
pub const MSG_BAD_SETTINGS: &str = "设置的值不对。";
// v0.6
pub const MSG_SIGN_IN_REQUIRED: &str = "请先登录。";
pub const MSG_ADMIN_ONLY: &str = "只有管理员可以进行这个操作。";
pub const MSG_BAD_LOGIN: &str = "用户名或密码不对。";
pub const MSG_TOO_MANY_LOGINS: &str = "登录失败次数太多，请稍后再试。";
pub const MSG_PASSWORD_CHANGE_REQUIRED: &str = "请先设置新密码。";
pub const MSG_SAME_PASSWORD: &str = "新密码不能和原来的一样。";
pub const MSG_BAD_USERNAME: &str = "用户名不对：1–40 个字符，只能用字母、数字和 . _ - @。";
pub const MSG_BAD_PASSWORD: &str = "密码需要 8–128 个字符。";
pub const MSG_USERNAME_TAKEN: &str = "这个用户名已经有人用了。";
pub const MSG_USER_NOT_FOUND: &str = "这个用户不存在（可能已被删除），请刷新页面后再试。";
pub const MSG_LAST_ADMIN: &str = "至少要保留一个启用的管理员。";
pub const MSG_NOT_YOURSELF: &str = "不能对自己的账户做这个操作。";
pub const MSG_WRONG_PASSWORD: &str = "当前密码不对。";

pub fn make(status: StatusCode, ctype: &'static str, body: Vec<u8>, head_only: bool) -> Response {
    let len = body.len();
    let mut resp = Response::new(if head_only {
        Body::empty()
    } else {
        Body::from(body)
    });
    *resp.status_mut() = status;
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ctype));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    resp
}

pub fn json_resp(status: StatusCode, v: &Value, head_only: bool) -> Response {
    make(status, JSON_CT, py_json(v), head_only)
}

pub fn text_resp(status: StatusCode, text: &str, head_only: bool) -> Response {
    make(status, TEXT_CT, text.as_bytes().to_vec(), head_only)
}

pub fn error_json(status: StatusCode, msg: &str, head_only: bool) -> Response {
    json_resp(status, &json!({ "error": msg }), head_only)
}

/// Plain text returned when a GET is rejected by the Host guard (verbatim from server.py).
pub fn forbidden_host_text(seen: &str) -> String {
    format!(
        "Forbidden host: {seen}。通过别的地址访问（局域网 IP、你的域名）时，请把它加进环境变量 XEXAM_ALLOWED_HOSTS，或设 XEXAM_ALLOW_ANY_HOST=1（只在前面有身份验证时）。"
    )
}

/// `AppError` → response. Internal details (SQLite errors, paths) go only to the log, never into the response.
pub fn error_response(e: AppError, head_only: bool) -> Response {
    match e {
        AppError::ForbiddenHost { seen } => text_resp(
            StatusCode::FORBIDDEN,
            &forbidden_host_text(&seen),
            head_only,
        ),
        AppError::Denied(reason) => error_json(StatusCode::FORBIDDEN, &reason, head_only),
        AppError::UnsupportedMediaType => error_json(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "bad content type",
            head_only,
        ),
        AppError::JsonTooLarge => error_json(StatusCode::PAYLOAD_TOO_LARGE, "too large", head_only),
        AppError::UploadTooLarge { .. } => error_json(
            StatusCode::PAYLOAD_TOO_LARGE,
            MSG_UPLOAD_TOO_LARGE,
            head_only,
        ),
        AppError::BadJson => error_json(StatusCode::BAD_REQUEST, "bad json", head_only),
        AppError::BadRequest(m) => error_json(StatusCode::BAD_REQUEST, &m, head_only),
        AppError::NotFound => error_json(StatusCode::NOT_FOUND, "not found", head_only),
        AppError::NotFoundMsg(m) => error_json(StatusCode::NOT_FOUND, m, head_only),
        AppError::PendingGone => error_json(StatusCode::CONFLICT, MSG_PENDING_GONE, head_only),
        AppError::ShutdownDisabled => {
            error_json(StatusCode::FORBIDDEN, MSG_SHUTDOWN_DISABLED, head_only)
        }
        AppError::Status(code, m) => error_json(
            StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            m,
            head_only,
        ),
        AppError::TooManyLogins { retry_after } => {
            let mut r = json_resp(
                StatusCode::TOO_MANY_REQUESTS,
                &json!({ "error": MSG_TOO_MANY_LOGINS, "retryAfter": retry_after }),
                head_only,
            );
            r.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(retry_after));
            r
        }
        AppError::Db(crate::error::DbError::NoUser) => {
            error_json(StatusCode::UNAUTHORIZED, MSG_SIGN_IN_REQUIRED, head_only)
        }
        AppError::Db(_) | AppError::Internal => {
            error_json(StatusCode::INTERNAL_SERVER_ERROR, MSG_INTERNAL, head_only)
        }
    }
}

/// 405: `Allow` lists the methods supported by this path.
pub fn method_not_allowed(allow: &'static str) -> Response {
    let mut r = error_json(StatusCode::METHOD_NOT_ALLOWED, "method not allowed", false);
    r.headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static(allow));
    r
}
