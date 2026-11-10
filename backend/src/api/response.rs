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
        AppError::PendingGone => error_json(StatusCode::CONFLICT, MSG_PENDING_GONE, head_only),
        AppError::ShutdownDisabled => {
            error_json(StatusCode::FORBIDDEN, MSG_SHUTDOWN_DISABLED, head_only)
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
