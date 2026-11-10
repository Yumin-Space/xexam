//! Streaming read of the request body: does not trust `Content-Length`, counts real bytes; when over the limit, it drains the rest (bounded) before returning,
//! otherwise the server responds early, the connection is reset, and the browser sees a network error instead of "file too large".
use axum::body::Body;
use bytes::Bytes;
use http_body_util::BodyExt;
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub enum BodyError {
    /// Over the limit (or Content-Length is negative)
    TooLarge,
    /// Connection dropped midway / the declared length is longer than the actual body
    Broken,
}

/// Total time limit for draining excess data.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// Reads and discards up to `cap` bytes (the `already` bytes read so far count too). Stops on error, end of stream or timeout.
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

/// Reads the request body. `declared` is the Content-Length from the request headers (already parsed, may be negative);
/// a declared value over the limit / negative -> drain first, then `TooLarge`; otherwise count the bytes actually received and stop once over `limit` (`>`, not `>=`).
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
