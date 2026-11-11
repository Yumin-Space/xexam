//! Error codes of `/api/ai/*` and their Chinese texts (spec 5.1, with the numbers of 14.1 as parameters). The codes are
//! stable; the frontend translates by code. Every Chinese text of the AI service is in this file.
use crate::provider::UpCode;
use crate::secret::SecretName;
use axum::http::StatusCode;
use serde_json::{Map, Value};

/// An error answer: status, code, Chinese text, and the optional `field` / `limit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub text: String,
    pub field: Option<&'static str>,
    pub limit: Option<i64>,
}

impl ApiError {
    fn new(status: u16, code: &'static str, text: &str) -> ApiError {
        ApiError {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            code,
            text: text.to_string(),
            field: None,
            limit: None,
        }
    }

    fn with_field(mut self, field: &'static str) -> ApiError {
        self.field = Some(field);
        self
    }

    fn with_limit(mut self, n: i64) -> ApiError {
        self.limit = Some(n);
        self
    }

    /// `{"error": …, "code": …, "field"?: …, "limit"?: …}`
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("error".into(), Value::from(self.text.clone()));
        m.insert("code".into(), Value::from(self.code));
        if let Some(f) = self.field {
            m.insert("field".into(), Value::from(f));
        }
        if let Some(n) = self.limit {
            m.insert("limit".into(), Value::from(n));
        }
        Value::Object(m)
    }

    pub fn backend_down() -> ApiError {
        ApiError::new(
            503,
            "ai.backendDown",
            "AI 暂时无法使用，请稍后再试或联系管理员。",
        )
    }
    pub fn server_busy() -> ApiError {
        ApiError::new(503, "ai.serverBusy", "AI 现在太忙，请稍后再试。")
    }
    pub fn internal() -> ApiError {
        ApiError::new(500, "ai.internal", "AI 内部出错了，请稍后再试。")
    }
    pub fn disabled() -> ApiError {
        ApiError::new(403, "ai.disabled", "AI+ 功能暂时不可用。")
    }
    /// The caller's AI+ feature was paused by an administrator (AI settings)
    pub fn suspended() -> ApiError {
        ApiError::new(403, "ai.suspended", "管理员已暂停你的 AI+ 功能。")
    }
    /// Pausing an administrator's AI+ feature
    pub fn admin_always() -> ApiError {
        ApiError::new(409, "ai.adminAlways", "管理员始终可以使用 AI，不能暂停。")
    }
    pub fn not_allowed() -> ApiError {
        ApiError::new(403, "ai.notAllowed", "你的账号没有开通 AI+ 功能。")
    }
    pub fn admin_only() -> ApiError {
        ApiError::new(403, "ai.adminOnly", "只有管理员可以这样做。")
    }
    pub fn not_found() -> ApiError {
        ApiError::new(404, "ai.notFound", "找不到了，可能已被删除。")
    }
    pub fn busy() -> ApiError {
        ApiError::new(409, "ai.busy", "上一个回答还在生成，请等它结束或按停止。")
    }
    pub fn too_long(n: i64) -> ApiError {
        ApiError::new(
            400,
            "ai.tooLong",
            &format!("消息太长了（最多 {n} 个字符）。"),
        )
        .with_limit(n)
    }
    pub fn message_limit(n: i64) -> ApiError {
        ApiError::new(
            409,
            "ai.messageLimit",
            &format!("这个对话已经很长了（最多 {n} 条消息），请开始一个新对话。"),
        )
        .with_limit(n)
    }
    pub fn chat_limit(n: i64) -> ApiError {
        ApiError::new(
            409,
            "ai.chatLimit",
            &format!("对话太多了（最多 {n} 个），请先删除一些旧对话。"),
        )
        .with_limit(n)
    }
    pub fn not_retryable() -> ApiError {
        ApiError::new(409, "ai.notRetryable", "这个回答不能重试。")
    }
    pub fn bad_request() -> ApiError {
        ApiError::new(400, "ai.badRequest", "请求无效。")
    }
    pub fn too_large() -> ApiError {
        ApiError::new(413, "ai.badRequest", "请求无效。")
    }
    pub fn bad_media_type() -> ApiError {
        ApiError::new(415, "ai.badRequest", "请求无效。")
    }
    pub fn method_not_allowed() -> ApiError {
        ApiError::new(405, "ai.badRequest", "请求无效。")
    }
    pub fn memory_limit(n: i64) -> ApiError {
        ApiError::new(
            409,
            "ai.memoryLimit",
            &format!("记忆已满（最多 {n} 条），请先删除一条。"),
        )
        .with_limit(n)
    }
    pub fn memory_too_long(n: i64) -> ApiError {
        ApiError::new(
            400,
            "ai.memoryTooLong",
            &format!("一条记忆最多 {n} 个字符。"),
        )
        .with_limit(n)
    }
    pub fn not_verified() -> ApiError {
        ApiError::new(409, "ai.notVerified", "请先保存设置并测试成功，再开启 AI。")
    }
    pub fn key_problem() -> ApiError {
        ApiError::new(
            409,
            "ai.keyProblem",
            "已保存的密钥无法解密，请在 AI 设置里重新输入。",
        )
    }
    pub fn bad_url() -> ApiError {
        ApiError::new(
            400,
            "ai.badUrl",
            "地址无效：请输入完整的 http:// 或 https:// 地址，不带用户名、查询参数或 #。",
        )
    }
    pub fn bad_key(f: SecretName) -> ApiError {
        ApiError::new(
            400,
            "ai.badKey",
            "密钥格式不对：只能是 1–400 个可见的英文字符，不能有空格。",
        )
        .with_field(f.json())
    }
    pub fn header_clash() -> ApiError {
        ApiError::new(
            400,
            "ai.headerClash",
            "这种认证方式下，网关密钥和服务商密钥会用到同一个请求头。请改用 cf-aig-authorization，或删除已保存的服务商密钥（勾选“删除这个密钥”）。",
        )
        .with_field(SecretName::ApiKey.json())
    }
    pub fn key_missing(f: SecretName) -> ApiError {
        ApiError::new(400, "ai.keyMissing", "请先填写密钥。").with_field(f.json())
    }
    pub fn key_reenter(f: SecretName) -> ApiError {
        ApiError::new(400, "ai.keyReenter", "地址或服务商变了，请重新输入密钥。")
            .with_field(f.json())
    }
    pub fn model_missing() -> ApiError {
        ApiError::new(400, "ai.modelMissing", "请先选择模型。")
    }
    pub fn bad_retention() -> ApiError {
        ApiError::new(400, "ai.badRetention", "保留天数必须在 1 到 3650 之间。")
    }
    pub fn limit_range(key: &'static str, min: i64, max: i64) -> ApiError {
        ApiError::new(
            400,
            "ai.limitRange",
            &format!("这个上限必须是 {min} 到 {max} 之间的整数。"),
        )
        .with_field(key)
    }
    /// An upstream failure as a JSON error (502, or 504 for a timeout).
    pub fn upstream(code: UpCode) -> ApiError {
        let status = if code == UpCode::Timeout { 504 } else { 502 };
        ApiError::new(status, code.code(), text_of(code.code()))
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> ApiError {
        tracing::error!("AI database error: {e}");
        ApiError::internal()
    }
}

/// The Chinese text of a code (also used in the SSE `error` event and for stored codes).
pub fn text_of(code: &str) -> &'static str {
    match code {
        "ai.backendDown" => "AI 暂时无法使用，请稍后再试或联系管理员。",
        "ai.serverBusy" => "AI 现在太忙，请稍后再试。",
        "ai.internal" => "AI 内部出错了，请稍后再试。",
        "ai.disabled" => "AI+ 功能暂时不可用。",
        "ai.suspended" => "管理员已暂停你的 AI+ 功能。",
        "ai.adminAlways" => "管理员始终可以使用 AI，不能暂停。",
        "ai.notAllowed" => "你的账号没有开通 AI+ 功能。",
        "ai.notFound" => "找不到了，可能已被删除。",
        "ai.interrupted" => "服务重启，回答被中断了。",
        "ai.keyProblem" => "已保存的密钥无法解密，请在 AI 设置里重新输入。",
        "ai.up.auth" => "AI 服务商拒绝了这个密钥。",
        "ai.up.quota" => "AI 服务商账号的额度或余额不足。",
        "ai.up.notFound" => "AI 服务商找不到这个模型或地址。",
        "ai.up.rateLimit" => "AI 服务商太忙（触发限流），请稍后再试。",
        "ai.up.overloaded" => "AI 服务商过载，请稍后再试。",
        "ai.up.server" => "AI 服务商内部出错了。",
        "ai.up.badRequest" => "AI 服务商拒绝了这个请求。",
        "ai.up.timeout" => "AI 服务商没有及时回应。",
        "ai.up.connect" => "连不上 AI 服务商。",
        "ai.up.protocol" => "AI 服务商的回应无法读取。",
        "ai.up.refused" => "AI 拒绝回答这个问题。",
        "ai.up.tooLong" => "回答太长，已被截断。",
        _ => "AI 内部出错了，请稍后再试。",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_shape() {
        let e = ApiError::too_long(16000);
        assert_eq!(
            serde_json::to_string(&e.to_json()).expect("json"),
            r#"{"error":"消息太长了（最多 16000 个字符）。","code":"ai.tooLong","limit":16000}"#
        );
        let e = ApiError::key_reenter(SecretName::GatewayKey);
        assert_eq!(e.status, StatusCode::BAD_REQUEST);
        assert_eq!(e.field, Some("gatewayKey"));
        assert_eq!(ApiError::upstream(UpCode::Timeout).status.as_u16(), 504);
        assert_eq!(ApiError::upstream(UpCode::Auth).status.as_u16(), 502);
        assert_eq!(text_of("ai.up.tooLong"), "回答太长，已被截断。");
        for e in [
            ApiError::disabled(),
            ApiError::suspended(),
            ApiError::admin_always(),
        ] {
            assert_eq!(text_of(e.code), e.text, "{}", e.code);
        }
        assert_eq!(ApiError::suspended().status.as_u16(), 403);
        assert_eq!(ApiError::admin_always().status.as_u16(), 409);
    }
}
