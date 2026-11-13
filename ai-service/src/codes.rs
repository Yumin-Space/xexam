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
    /// `ai.callLimit`: which cap and when one more call is possible (boxed: keeps `ApiError` small)
    pub call: Option<Box<CallInfo>>,
    /// `ai.busy` of the study routes: what is running (`null` when nothing could be found); serialised last
    pub running: Option<Box<Value>>,
    /// More keys of the answer and a `Retry-After` header (generation / import refusals), boxed to keep `ApiError` small
    pub more: Option<Box<More>>,
}

/// The rarely used parts of an error answer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct More {
    /// More keys (`job`, `feature`, `used`, ...), serialised after the others
    pub extra: Vec<(&'static str, Value)>,
    /// A `Retry-After` header in seconds (`ai.gen.quota`)
    pub retry_header: Option<i64>,
}

/// The extra fields of `ai.callLimit`: the cap (`minute` / `hour` / `day`), when one more call is possible (unix s) and in
/// how many whole seconds (at least 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallInfo {
    pub cap: &'static str,
    pub retry_at: i64,
    pub retry_after: i64,
}

impl ApiError {
    fn new(status: u16, code: &'static str, text: &str) -> ApiError {
        ApiError {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            code,
            text: text.to_string(),
            field: None,
            limit: None,
            call: None,
            running: None,
            more: None,
        }
    }

    /// Adds one more key to the answer.
    pub fn with_extra(mut self, key: &'static str, value: Value) -> ApiError {
        self.more
            .get_or_insert_with(Default::default)
            .extra
            .push((key, value));
        self
    }

    /// The `Retry-After` header of this answer, if it has one.
    pub fn retry_header(&self) -> Option<i64> {
        self.more.as_ref().and_then(|m| m.retry_header)
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
        if let Some(c) = &self.call {
            m.insert("cap".into(), Value::from(c.cap));
        }
        if let Some(n) = self.limit {
            m.insert("limit".into(), Value::from(n));
        }
        if let Some(c) = &self.call {
            m.insert("retryAt".into(), Value::from(c.retry_at));
            m.insert("retryAfter".into(), Value::from(c.retry_after));
        }
        if let Some(r) = &self.running {
            m.insert("running".into(), (**r).clone());
        }
        if let Some(more) = &self.more {
            for (k, v) in &more.extra {
                m.insert((*k).into(), v.clone());
            }
        }
        Value::Object(m)
    }

    /// A per-user cap on AI model calls is reached (429 `ai.callLimit`): names the cap, its limit and when the user can
    /// call again. The day cap ends with the owner's "明天 0 点后可以再次使用。".
    pub fn call_limit(hit: &crate::quota::Hit, now: i64) -> ApiError {
        let (name, tail) = match hit.cap {
            crate::quota::Cap::Minute => ("每分钟", ""),
            crate::quota::Cap::Hour => ("每小时", ""),
            crate::quota::Cap::Day => ("每天", "明天 0 点后可以再次使用。"),
        };
        let text = format!(
            "你的 AI 请求额度已用完（{name} {} 次），请联系管理员设置 AI 请求额度上限。{tail}",
            hit.limit
        );
        let mut e = ApiError::new(429, "ai.callLimit", &text).with_limit(hit.limit);
        e.call = Some(Box::new(CallInfo {
            cap: hit.cap.key(),
            retry_at: hit.retry_at,
            retry_after: (hit.retry_at - now).max(1),
        }));
        e
    }
    /// The call caps are not in order (minute <= hour <= day): 400 `ai.limitOrder` naming the larger window's field
    pub fn limit_order(field: &'static str) -> ApiError {
        ApiError::new(
            400,
            "ai.limitOrder",
            "每小时的次数不能少于每分钟的次数，每天的次数不能少于每小时的次数。",
        )
        .with_field(field)
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
    /// A-OWNER-21: free mode is an administrator state; an ordinary account cannot get it
    pub fn free_admin_only() -> ApiError {
        ApiError::new(409, "ai.freeAdminOnly", "只有管理员账号可以使用自由模式。")
    }
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
    /// `ai.busy` with the answer that is running (`RunningInfo`, or `Value::Null`)
    pub fn busy_with(running: Value) -> ApiError {
        let mut e = ApiError::busy();
        e.running = Some(Box::new(running));
        e
    }
    /// A summary of a record without any wrong answer
    pub fn no_wrong() -> ApiError {
        ApiError::new(409, "ai.noWrong", text_of("ai.noWrong"))
    }
    /// A summary of a record whose wrong answers can no longer be used
    pub fn no_usable() -> ApiError {
        ApiError::new(409, "ai.noUsable", text_of("ai.noUsable"))
    }
    /// A summary of a record without per-question records
    pub fn no_detail() -> ApiError {
        ApiError::new(409, "ai.noDetail", text_of("ai.noDetail"))
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
    // —— AI features, generation and import (v1.1) ——————————————————————————————————————————————————————————————————————

    /// The caller's AI+ feature is switched off for this user (FP-2)
    pub fn feature_off(f: crate::features::Feature) -> ApiError {
        ApiError::new(403, "ai.featureOff", text_of("ai.featureOff"))
            .with_extra("feature", Value::from(f.key()))
    }
    /// The composition counts of a mode do not add up to 1 ..= 100
    pub fn gen_count(field: &'static str) -> ApiError {
        ApiError::new(400, "ai.genCount", text_of("ai.genCount")).with_field(field)
    }
    pub fn gen_bad_name(max: i64) -> ApiError {
        ApiError::new(400, "ai.gen.badName", text_of("ai.gen.badName"))
            .with_extra("max", Value::from(max))
    }
    /// The bank name typed at the import confirm step is empty after trimming
    pub fn import_name_empty() -> ApiError {
        ApiError::new(400, "ai.import.nameEmpty", text_of("ai.import.nameEmpty"))
    }
    /// A bank with that name exists already; nothing was created
    pub fn import_name_taken() -> ApiError {
        ApiError::new(409, "ai.import.nameTaken", text_of("ai.import.nameTaken"))
    }
    pub fn gen_forbidden() -> ApiError {
        ApiError::new(403, "ai.gen.forbidden", text_of("ai.gen.forbidden"))
    }
    pub fn gen_not_found() -> ApiError {
        ApiError::new(404, "ai.gen.notFound", text_of("ai.gen.notFound"))
    }
    /// A job of this user is queued or running; `job` is its Job object (`null` while an upload is being taken)
    pub fn gen_busy(job: Value) -> ApiError {
        ApiError::new(409, "ai.gen.busy", text_of("ai.gen.busy")).with_extra("job", job)
    }
    pub fn gen_ai_source() -> ApiError {
        ApiError::new(422, "ai.gen.aiSource", text_of("ai.gen.aiSource"))
    }
    pub fn gen_too_few(have: i64, need: i64, what: &'static str) -> ApiError {
        ApiError::new(
            422,
            "ai.gen.tooFew",
            &format!("题目太少，无法生成（至少需要 {need} 道，现有 {have} 道）。"),
        )
        .with_extra("have", Value::from(have))
        .with_extra("need", Value::from(need))
        .with_extra("what", Value::from(what))
    }
    /// Today's AI bank units are used up (generation and import share them)
    pub fn gen_quota(used: i64, limit: i64, reset_at: i64, now: i64) -> ApiError {
        let mut e = ApiError::new(
            429,
            "ai.gen.quota",
            &format!("今天的 {limit} 次 AI 题库机会（生成和导入共用）已经用完，0 点后可以再用。"),
        )
        .with_extra("used", Value::from(used))
        .with_extra("limit", Value::from(limit))
        .with_extra("resetAt", Value::from(reset_at))
        .with_extra("retryAfter", Value::from((reset_at - now).max(1)));
        e.more.get_or_insert_with(Default::default).retry_header = Some((reset_at - now).max(1));
        e
    }
    pub fn gen_too_late(job: Value) -> ApiError {
        ApiError::new(409, "ai.gen.tooLate", text_of("ai.gen.tooLate")).with_extra("job", job)
    }
    pub fn import_not_waiting(job: Value) -> ApiError {
        ApiError::new(409, "ai.import.notWaiting", text_of("ai.import.notWaiting"))
            .with_extra("job", job)
    }
    /// An upload refused by its file type (`ai.import.type`, `.legacy`, `.image`; 415)
    pub fn import_type(code: &'static str, ext: &str) -> ApiError {
        ApiError::new(415, code, text_of(code)).with_extra("ext", Value::from(ext))
    }
    pub fn import_empty() -> ApiError {
        ApiError::new(400, "ai.import.empty", text_of("ai.import.empty"))
    }
    pub fn import_too_large(max_mb: i64) -> ApiError {
        ApiError::new(
            413,
            "ai.import.tooLarge",
            &format!("文件太大了（最大 {max_mb} MB）。"),
        )
        .with_extra("maxMb", Value::from(max_mb))
    }
    pub fn length_required() -> ApiError {
        ApiError::new(411, "ai.badRequest", "请求无效。")
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
            &format!("一条记忆最多 {n} 个 Token。"),
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
        "ai.hintLeak" => "这个提示透露了答案，已被隐藏。请重新生成。",
        "ai.noWrong" => "这次没有答错的题（未作答的题不计入 AI 总结）。",
        "ai.noUsable" => "这些答错的题已经不可用（题库已删除或没有访问权限），无法总结。",
        "ai.noDetail" => "这条记录没有逐题记录，无法总结。",
        "ai.featureOff" => "你没有权限使用这个功能。",
        "ai.genCount" => "每次生成的题数（单选 + 多选）必须在 1 到 100 之间。",
        "ai.gen.badName" => "名称不能为空，不能太长，也不能含有控制字符。",
        "ai.gen.forbidden" => "你没有题库管理权限，不能使用这个功能。",
        "ai.gen.notFound" => "找不到这个题库或工作表。",
        "ai.gen.busy" => "你已经有一个 AI 题库任务在进行，请等它结束。",
        "ai.gen.aiSource" => "勾选里包含 AI+题集，不能用来生成题目，请取消勾选 AI+题集。",
        "ai.gen.tooFew" => "题目太少，无法生成。",
        "ai.gen.quota" => "今天的 AI 题库机会（生成和导入共用）已经用完，0 点后可以再用。",
        "ai.gen.tooLate" => "已经来不及取消了。",
        "ai.gen.quality" => "AI 没能写出足够多合格的题目，请稍后再试。",
        "ai.gen.stopped" => "AI 功能被关闭或暂停，任务已停止。",
        "ai.gen.timeout" => "任务用时太长，已停止。",
        "ai.gen.interrupted" => "服务重启，任务被中断了。",
        "ai.gen.save" => "题库没能保存，请稍后再试。",
        "ai.import.type" => "不支持这种文件类型。",
        "ai.import.legacy" => "不支持旧版 Office 或加密的文件，请另存为新格式。",
        "ai.import.image" => "不支持图片，请使用有文字的文件。",
        "ai.import.empty" => "这个文件是空的。",
        "ai.import.tooLarge" => "文件太大了。",
        "ai.import.notWaiting" => "这个导入现在不需要确认。",
        "ai.import.nameEmpty" => "请输入题库名称。",
        "ai.import.nameTaken" => "已经有同名题库，请换一个名称。",
        "ai.import.noText" => "读不到文字：文件可能是扫描件或图片。",
        "ai.import.encoding" => "无法识别这个文本文件的编码。",
        "ai.import.protected" => "文件受密码保护，无法读取。",
        "ai.import.corrupt" => "文件已损坏或格式不对，无法读取。",
        "ai.import.tooComplex" => "文件太复杂，无法读取。",
        "ai.import.tooMuchText" => "文件的文字太多，请拆分后再导入。",
        "ai.import.readTimeout" => "读取文件用时太长，已停止。",
        "ai.import.noContent" => "文件里没有可以学习的内容。",
        "ai.import.unclear" => "AI 没能看懂这个文件，请稍后再试。",
        "ai.import.tooMany" => "文件里的题目太多，请拆分后再导入。",
        "ai.import.tooLongQuestion" => "文件里有一道题太长，无法导入。",
        "ai.import.callLimit" => "AI 请求次数已达上限，请稍后再试。",
        "ai.import.expired" => "超过 30 分钟没有确认，导入已取消。",
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
        for e in [
            ApiError::no_wrong(),
            ApiError::no_usable(),
            ApiError::no_detail(),
        ] {
            assert_eq!(text_of(e.code), e.text, "{}", e.code);
            assert_eq!(e.status.as_u16(), 409);
        }
        assert_eq!(
            text_of("ai.hintLeak"),
            "这个提示透露了答案，已被隐藏。请重新生成。"
        );
        // `running` comes last and only when set (`null` included)
        let b = ApiError::busy_with(serde_json::Value::Null);
        assert!(serde_json::to_string(&b.to_json())
            .expect("json")
            .ends_with(r#""code":"ai.busy","running":null}"#));
        let b = ApiError::busy_with(serde_json::json!({"chatId": 3}));
        assert_eq!(b.to_json()["running"]["chatId"], 3);
        assert!(ApiError::busy().to_json().get("running").is_none());
        assert_eq!(ApiError::suspended().status.as_u16(), 403);
        assert_eq!(ApiError::admin_always().status.as_u16(), 409);
    }

    #[test]
    fn call_limit_shape() {
        use crate::quota::{Cap, Hit};
        let hour = Hit {
            cap: Cap::Hour,
            limit: 20,
            retry_at: 1_762_950_300,
        };
        let e = ApiError::call_limit(&hour, 1_762_949_588);
        assert_eq!(e.status.as_u16(), 429);
        assert_eq!(
            serde_json::to_string(&e.to_json()).expect("json"),
            r#"{"error":"你的 AI 请求额度已用完（每小时 20 次），请联系管理员设置 AI 请求额度上限。","code":"ai.callLimit","cap":"hour","limit":20,"retryAt":1762950300,"retryAfter":712}"#
        );
        let day = Hit {
            cap: Cap::Day,
            limit: 100,
            retry_at: 1_000,
        };
        let e = ApiError::call_limit(&day, 5_000);
        assert!(e.text.ends_with("明天 0 点后可以再次使用。"), "{}", e.text);
        assert_eq!(
            e.call.as_ref().map(|c| c.retry_after),
            Some(1),
            "never below one second"
        );
        let o = ApiError::limit_order("callsPerHour");
        assert_eq!(o.field, Some("callsPerHour"));
        assert_eq!(o.status.as_u16(), 400);
    }

    #[test]
    fn generation_and_import_answers() {
        use crate::features::Feature;
        let e = ApiError::feature_off(Feature::Import);
        assert_eq!(
            serde_json::to_string(&e.to_json()).expect("json"),
            r#"{"error":"你没有权限使用这个功能。","code":"ai.featureOff","feature":"import"}"#
        );
        assert_eq!(e.status.as_u16(), 403);
        let q = ApiError::gen_quota(3, 3, 2_000, 1_900);
        assert_eq!(q.status.as_u16(), 429);
        let keys: Vec<String> = q
            .to_json()
            .as_object()
            .expect("obj")
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            keys,
            ["error", "code", "used", "limit", "resetAt", "retryAfter"]
        );
        assert_eq!(q.retry_header(), Some(100));
        assert_eq!(q.to_json()["retryAfter"], 100);
        let b = ApiError::gen_busy(Value::Null);
        assert_eq!(b.to_json()["job"], Value::Null);
        assert_eq!(b.status.as_u16(), 409);
        let t = ApiError::gen_too_few(18, 30, "source");
        assert_eq!(
            (
                t.status.as_u16(),
                t.to_json()["have"].clone(),
                t.to_json()["need"].clone(),
                t.to_json()["what"].clone()
            ),
            (
                422,
                serde_json::json!(18),
                serde_json::json!(30),
                serde_json::json!("source")
            )
        );
        let i = ApiError::import_type("ai.import.legacy", "doc");
        assert_eq!(
            (i.status.as_u16(), i.to_json()["ext"].clone()),
            (415, serde_json::json!("doc"))
        );
        let l = ApiError::import_too_large(20);
        assert_eq!(
            (l.status.as_u16(), l.to_json()["maxMb"].clone()),
            (413, serde_json::json!(20))
        );
        assert_eq!(ApiError::gen_ai_source().status.as_u16(), 422);
        assert_eq!(ApiError::gen_bad_name(60).to_json()["max"], 60);
        assert_eq!(
            ApiError::gen_count("genBankMulti").field,
            Some("genBankMulti")
        );
        assert_eq!(ApiError::length_required().status.as_u16(), 411);
    }

    #[test]
    fn every_job_code_has_its_own_text() {
        let generic = text_of("nonsense");
        for code in [
            "ai.featureOff",
            "ai.genCount",
            "ai.gen.badName",
            "ai.gen.forbidden",
            "ai.gen.notFound",
            "ai.gen.busy",
            "ai.gen.aiSource",
            "ai.gen.tooFew",
            "ai.gen.quota",
            "ai.gen.tooLate",
            "ai.gen.quality",
            "ai.gen.stopped",
            "ai.gen.timeout",
            "ai.gen.interrupted",
            "ai.gen.save",
            "ai.import.type",
            "ai.import.legacy",
            "ai.import.image",
            "ai.import.empty",
            "ai.import.tooLarge",
            "ai.import.notWaiting",
            "ai.import.nameEmpty",
            "ai.import.nameTaken",
            "ai.import.noText",
            "ai.import.encoding",
            "ai.import.protected",
            "ai.import.corrupt",
            "ai.import.tooComplex",
            "ai.import.tooMuchText",
            "ai.import.readTimeout",
            "ai.import.noContent",
            "ai.import.unclear",
            "ai.import.tooMany",
            "ai.import.tooLongQuestion",
            "ai.import.callLimit",
            "ai.import.expired",
        ] {
            assert_ne!(text_of(code), generic, "{code}");
        }
        // every extraction error maps to a code that has a text
        for e in crate::extract::ALL_ERRORS {
            let code = format!("ai.import.{}", e.code());
            assert_ne!(text_of(&code), generic, "{code}");
        }
    }
}
