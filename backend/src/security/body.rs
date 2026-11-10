//! 请求体的流式读取：不信任 `Content-Length`，真实字节计数；超限时先读掉（有上限）再返回，
//! 否则服务端提前回应后连接被重置，浏览器看到的是网络错误而不是「文件太大」。
use axum::body::Body;
use bytes::Bytes;
use http_body_util::BodyExt;
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub enum BodyError {
    /// 超过上限（或 Content-Length 为负）
    TooLarge,
    /// 连接中途断开 / 声明的长度比实际的长
    Broken,
}

/// 排空多余数据的总时限。
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// 读掉并丢弃最多 `cap` 字节（已经读到的 `already` 也算在内）。出错、结束或超时就停。
pub async fn drain(body: &mut Body, already: usize, cap: usize) {
    let _ = tokio::time::timeout(DRAIN_TIMEOUT, async {
        let mut seen = already;
        while seen < cap {
            match body.frame().await {
                Some(Ok(frame)) => {
                    if let Some(d) = frame.data_ref() {
                        seen = seen.saturating_add(d.len());
                    }
                }
                _ => break,
            }
        }
    })
    .await;
}

/// 读取请求体。`declared` 是请求头里的 Content-Length（已解析，可能为负）；
/// 声明值超限 / 为负 -> 先排空再 `TooLarge`；否则按实际收到的字节计数，超过 `limit`（`>`，不是 `>=`）就停。
pub async fn read_limited(
    mut body: Body,
    declared: Option<i128>,
    limit: usize,
) -> Result<Bytes, BodyError> {
    let cap = limit.saturating_mul(4);
    if let Some(n) = declared {
        if n < 0 {
            return Err(BodyError::TooLarge);
        }
        if n > limit as i128 {
            let left = usize::try_from(n).unwrap_or(usize::MAX).min(cap);
            drain(&mut body, 0, left).await;
            return Err(BodyError::TooLarge);
        }
    }
    let mut buf: Vec<u8> = Vec::with_capacity(
        declared
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0)
            .min(64 * 1024),
    );
    loop {
        match body.frame().await {
            None => break,
            Some(Err(_)) => return Err(BodyError::Broken),
            Some(Ok(frame)) => {
                if let Some(d) = frame.data_ref() {
                    if buf.len().saturating_add(d.len()) > limit {
                        let seen = buf.len().saturating_add(d.len());
                        drain(&mut body, seen, cap).await;
                        return Err(BodyError::TooLarge);
                    }
                    buf.extend_from_slice(d);
                }
            }
        }
    }
    Ok(Bytes::from(buf))
}
