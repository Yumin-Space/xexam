//! The link to the main backend (spec 3.6, 5.4): the internal token and the calls to `/internal/ai/*`.
//!
//! The token is `XEXAM_INTERNAL_TOKEN` (at least 32 printable characters) or the file `<XEXAM_LINK_DIR>/internal.token`
//! written by the backend (read only here). It authenticates the calls to the backend (`X-Xexam-Internal-Token`, a
//! sensitive header) and is the HMAC key of the identity statements. Every answer must match its schema exactly
//! (5.4.6); anything else is a failure, and a 404 means the token was refused.
use crate::config::Config;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// File name of the token in the link directory.
pub const TOKEN_FILE: &str = "internal.token";
/// Largest answer body read from the backend.
pub const MAX_ANSWER: usize = 8 << 20;
/// Most ids per `/internal/ai/users` call. The backend accepts up to 10,000 ids but at most 64 KiB of body: 3,000 ids
/// of any size (up to 19 digits and a comma each) always fit.
pub const MAX_IDS: usize = 3_000;
/// A report to the main backend older than this means "not connected" (the backend applies the same rule).
pub const REPORT_STALE: Duration = Duration::from_secs(60);

/// One line for the main System log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub level: &'static str,
    pub message: String,
}

/// A user as the backend describes it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendUser {
    pub id: i64,
    pub stamp: String,
    pub username: String,
    pub role: String,
    pub disabled: bool,
    pub ai: bool,
    #[serde(rename = "aiAllowed")]
    pub ai_allowed: bool,
}

/// The user of a still valid session (`/internal/ai/introspect`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionUser {
    pub id: i64,
    pub stamp: String,
    pub username: String,
    pub role: String,
    #[serde(rename = "aiAllowed")]
    pub ai_allowed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IntrospectAnswer {
    ok: bool,
    user: Option<SessionUser>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsersAnswer {
    users: Vec<BackendUser>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OkAnswer {
    ok: bool,
}

/// Why a call to the backend failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkError {
    /// No token loaded yet
    NoToken,
    /// The backend answered 404: the token was refused
    Rejected,
    /// Unreachable, timeout, other status, or an answer that does not match its schema
    Failed,
}

#[derive(Default)]
struct LinkState {
    last_reread: Option<Instant>,
    refused: bool,
    connected: bool,
    waiting_logged: bool,
    /// When the backend last accepted a heartbeat
    last_report: Option<Instant>,
}

/// The link. No `Debug` (it holds the token).
pub struct Link {
    base: String,
    client: reqwest::Client,
    env_token: Option<Zeroizing<String>>,
    /// `XEXAM_INTERNAL_TOKEN` was set but is not a valid token: the link stays off (the file is not used either),
    /// the same rule as the main backend's
    off: bool,
    file: PathBuf,
    token: RwLock<Option<Zeroizing<String>>>,
    state: Mutex<LinkState>,
}

/// The shortest internal token accepted (the same as the main backend's).
pub const MIN_TOKEN_CHARS: usize = 32;

fn valid_token(t: &str) -> bool {
    token_problem(t).is_none()
}

/// Why a (trimmed) value is not a valid token, as the log line about `XEXAM_INTERNAL_TOKEN`: the same checks, in the
/// same order, and the same texts as the main backend's `InternalToken::new` (other characters before the length).
fn token_problem(t: &str) -> Option<&'static str> {
    if !t.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        Some("XEXAM_INTERNAL_TOKEN may contain only printable ASCII characters without spaces; AI link off")
    } else if t.len() < MIN_TOKEN_CHARS {
        Some("XEXAM_INTERNAL_TOKEN is too short (at least 32 characters); AI link off")
    } else {
        None
    }
}

impl Link {
    /// Builds the link from the configuration (takes the token out of it).
    pub fn new(cfg: &mut Config) -> Result<Link, String> {
        if !cfg.backend_url.starts_with("http://") {
            return Err(
                "XEXAM_AI_BACKEND_URL must be a plain http:// address of the main backend"
                    .to_string(),
            );
        }
        // A set but invalid XEXAM_INTERNAL_TOKEN turns the link off, like in the main backend: falling back to the file
        // would make the two services disagree about the token (and the line below would not be true). Both services
        // read it the same way: a blank value is unset (the configuration drops it), whitespace around the value is
        // removed (`str::trim`), and the rest is checked with the backend's rules and log texts.
        let (env_token, off) = match cfg.internal_token.take() {
            Some(t) => match token_problem(t.trim()) {
                None => (Some(Zeroizing::new(t.trim().to_string())), false),
                Some(line) => {
                    tracing::warn!("{line}");
                    (None, true)
                }
            },
            None => (None, false),
        };
        let link = Link {
            base: cfg.backend_url.clone(),
            client: crate::provider::backend_client(&cfg.version, &cfg.timeouts)?,
            env_token,
            off,
            file: cfg.link_dir.join(TOKEN_FILE),
            token: RwLock::new(None),
            state: Mutex::new(LinkState::default()),
        };
        link.load();
        Ok(link)
    }

    /// Path of the token file.
    pub fn token_path(&self) -> &std::path::Path {
        &self.file
    }

    /// Whether the token comes from `XEXAM_INTERNAL_TOKEN`.
    pub fn from_env(&self) -> bool {
        self.env_token.is_some()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, LinkState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn read_source(&self) -> Option<Zeroizing<String>> {
        if self.off {
            return None;
        }
        if let Some(t) = &self.env_token {
            return Some(t.clone());
        }
        let raw = Zeroizing::new(std::fs::read(&self.file).ok()?);
        // Whitespace around the content is ignored, as the main backend does (`str::trim`)
        let text = std::str::from_utf8(&raw).ok()?.trim();
        valid_token(text).then(|| Zeroizing::new(text.to_string()))
    }

    /// (Re)reads the token source; `true` when a token is loaded.
    pub fn load(&self) -> bool {
        let t = self.read_source();
        let present = t.is_some();
        if let Some(t) = t {
            *self.token.write().unwrap_or_else(|e| e.into_inner()) = Some(t);
        }
        present || self.has_token()
    }

    pub fn has_token(&self) -> bool {
        self.token
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    /// A copy of the current token.
    pub fn token(&self) -> Option<Zeroizing<String>> {
        self.token.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Writes the "waiting" line once (when the token is still missing).
    pub fn note_waiting(&self) {
        if self.off {
            return;
        }
        let mut st = self.state();
        if !st.waiting_logged {
            st.waiting_logged = true;
            tracing::info!(
                "AI link: waiting for the internal token at {}",
                self.file.display()
            );
        }
    }

    /// The token was refused (a 404 from `/internal/*`, or a statement whose HMAC does not verify): re-reads the source
    /// at most once per 5 s (30 s once the refusal was reported). Returns `true` when that produced a different token.
    pub fn note_rejected(&self) -> bool {
        let mut st = self.state();
        let now = Instant::now();
        let every = if st.refused {
            Duration::from_secs(30)
        } else {
            Duration::from_secs(5)
        };
        if st
            .last_reread
            .is_some_and(|t| now.saturating_duration_since(t) < every)
        {
            return false;
        }
        st.last_reread = Some(now);
        st.connected = false;
        let fresh = self.read_source();
        let mut cur = self.token.write().unwrap_or_else(|e| e.into_inner());
        let changed = match (&fresh, cur.as_ref()) {
            (Some(f), Some(c)) => f.as_str() != c.as_str(),
            (Some(_), None) => true,
            _ => false,
        };
        if changed {
            *cur = fresh;
            return true;
        }
        if !st.refused {
            st.refused = true;
            tracing::warn!(
                "AI link: the main service refused the internal token (XEXAM_INTERNAL_TOKEN must be the same for both services)"
            );
        }
        false
    }

    /// A call or a statement succeeded with the current token.
    pub fn note_ok(&self) {
        let mut st = self.state();
        st.refused = false;
        if !st.connected {
            st.connected = true;
            tracing::info!("AI link: connected to the main service");
        }
    }

    /// A statement verified with the current token. A purely local check: it says nothing about whether the main
    /// service can be reached, so it never sets "connected" (only real calls do).
    pub fn note_verified(&self) {
        self.state().refused = false;
    }

    /// `true` after a call to the main service succeeded and none failed since.
    pub fn is_connected(&self) -> bool {
        self.state().connected
    }

    fn note_failed(&self) {
        let mut st = self.state();
        if st.connected {
            st.connected = false;
            tracing::info!("AI link: lost the main service");
        }
    }

    async fn call(&self, path: &str, body: serde_json::Value) -> Result<Vec<u8>, LinkError> {
        self.send(path, body, false).await
    }

    /// A call of the study data routes (`/internal/ai/question`, `record`, `record-info`). The same request as
    /// [`Link::call`], but a 404 only fails this call (`LinkError::Rejected`): it does not re-read the token and does not
    /// mark the link as lost, because an ai-service newer than the backend (a data route the backend does not have yet)
    /// must not switch AI off for everyone. Transport errors, other statuses and oversize answers still count as a
    /// failed link; a good answer calls neither `note_ok` nor anything else (no "connected" line). The caller parses the
    /// answer and never touches the link state for a parse failure.
    pub async fn data_call(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<Vec<u8>, LinkError> {
        self.send(path, body, true).await
    }

    async fn send(
        &self,
        path: &str,
        body: serde_json::Value,
        data: bool,
    ) -> Result<Vec<u8>, LinkError> {
        let Some(token) = self.token() else {
            return Err(LinkError::NoToken);
        };
        let mut value =
            reqwest::header::HeaderValue::from_str(&token).map_err(|_| LinkError::NoToken)?;
        value.set_sensitive(true);
        drop(token);
        let bytes = serde_json::to_vec(&body).map_err(|_| LinkError::Failed)?;
        let sent = self
            .client
            .post(format!("{}{path}", self.base))
            .header("X-Xexam-Internal-Token", value)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(bytes)
            .send()
            .await;
        let mut resp = match sent {
            Ok(r) => r,
            Err(_) => {
                self.note_failed();
                return Err(LinkError::Failed);
            }
        };
        match resp.status().as_u16() {
            200 => {}
            404 => {
                if !data {
                    self.note_rejected();
                }
                return Err(LinkError::Rejected);
            }
            _ => {
                self.note_failed();
                return Err(LinkError::Failed);
            }
        }
        let mut buf = Vec::new();
        loop {
            match resp.chunk().await {
                Ok(Some(b)) => {
                    if buf.len() + b.len() > MAX_ANSWER {
                        self.note_failed();
                        return Err(LinkError::Failed);
                    }
                    buf.extend_from_slice(&b);
                }
                Ok(None) => break,
                Err(_) => {
                    self.note_failed();
                    return Err(LinkError::Failed);
                }
            }
        }
        Ok(buf)
    }

    fn parsed<T: for<'de> Deserialize<'de>>(&self, body: &[u8]) -> Result<T, LinkError> {
        match serde_json::from_slice::<T>(body) {
            Ok(v) => Ok(v),
            Err(_) => {
                self.note_failed();
                Err(LinkError::Failed)
            }
        }
    }

    /// Re-check of a session while an answer runs: `Some(user)` when it is still valid, `None` for `ok:false`.
    pub async fn introspect(&self, sid: &str, id: i64) -> Result<Option<SessionUser>, LinkError> {
        let body = self
            .call(
                "/internal/ai/introspect",
                serde_json::json!({"sid": sid, "id": id}),
            )
            .await?;
        let a: IntrospectAnswer = self.parsed(&body)?;
        let out = match (a.ok, a.user) {
            (true, Some(u)) if valid_role(&u.role) && u.id > 0 => Some(u),
            (false, None) => None,
            _ => {
                self.note_failed();
                return Err(LinkError::Failed);
            }
        };
        self.note_ok();
        Ok(out)
    }

    /// The users with the given ids that still exist, plus (with `holders`) every administrator and every user with AI+.
    pub async fn users(&self, ids: &[i64], holders: bool) -> Result<Vec<BackendUser>, LinkError> {
        let mut out: Vec<BackendUser> = Vec::new();
        let mut chunks: Vec<&[i64]> = ids.chunks(MAX_IDS).collect();
        if chunks.is_empty() {
            chunks.push(&[]);
        }
        for (i, chunk) in chunks.iter().enumerate() {
            let body = self
                .call(
                    "/internal/ai/users",
                    serde_json::json!({"ids": chunk, "holders": holders && i == 0}),
                )
                .await?;
            let a: UsersAnswer = self.parsed(&body)?;
            if a.users.iter().any(|u| !valid_role(&u.role) || u.id <= 0) {
                self.note_failed();
                return Err(LinkError::Failed);
            }
            for u in a.users {
                if !out.iter().any(|x| x.id == u.id) {
                    out.push(u);
                }
            }
        }
        out.sort_by_key(|u| u.id);
        self.note_ok();
        Ok(out)
    }

    /// Appends lines to the main System log.
    pub async fn log(&self, lines: &[LogLine]) -> Result<(), LinkError> {
        let items: Vec<serde_json::Value> = lines
            .iter()
            .map(|l| serde_json::json!({"level": l.level, "message": l.message}))
            .collect();
        let body = self
            .call("/internal/ai/log", serde_json::json!({ "lines": items }))
            .await?;
        let a: OkAnswer = self.parsed(&body)?;
        if !a.ok {
            self.note_failed();
            return Err(LinkError::Failed);
        }
        self.note_ok();
        Ok(())
    }

    /// Sends one state report (`POST /internal/ai/heartbeat`, contracts 2.2). The answer must be exactly
    /// `{"ok": true}`; an accepted report is remembered for [`Link::report_fresh`]. Never logged.
    pub async fn heartbeat(&self, report: serde_json::Value) -> Result<(), LinkError> {
        let body = self.call("/internal/ai/heartbeat", report).await?;
        let a: OkAnswer = self.parsed(&body)?;
        if !a.ok {
            self.note_failed();
            return Err(LinkError::Failed);
        }
        self.state().last_report = Some(Instant::now());
        self.note_ok();
        Ok(())
    }

    /// `true` while the backend accepted a report less than [`REPORT_STALE`] ago.
    pub fn report_fresh(&self) -> bool {
        self.state()
            .last_report
            .is_some_and(|t| t.elapsed() < REPORT_STALE)
    }

    /// Tests: makes the last accepted report look `by` older (never used by the service itself).
    #[doc(hidden)]
    pub fn age_report_for_tests(&self, by: Duration) {
        let mut st = self.state();
        st.last_report = st.last_report.and_then(|t| t.checked_sub(by));
    }

    /// Keeps trying to load a missing token every 5 s (logging the waiting line once).
    pub async fn wait_for_token(&self, mut stop: tokio::sync::watch::Receiver<bool>) {
        if self.off {
            return;
        }
        loop {
            if self.load() {
                return;
            }
            self.note_waiting();
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                _ = stop.changed() => return,
            }
        }
    }
}

fn valid_role(r: &str) -> bool {
    r == "admin" || r == "user"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const NOT_PRINTABLE: &str =
        "XEXAM_INTERNAL_TOKEN may contain only printable ASCII characters without spaces; AI link off";
    const TOO_SHORT: &str =
        "XEXAM_INTERNAL_TOKEN is too short (at least 32 characters); AI link off";

    /// A link whose `XEXAM_INTERNAL_TOKEN` is `value` (the link directory does not exist: no file fallback).
    fn link_with_env_token(value: &str) -> Link {
        let dir = std::env::temp_dir().join("xexam-ai-service-link-unit-test-no-such-dir");
        let map: HashMap<&str, String> = [
            ("XEXAM_INTERNAL_TOKEN", value.to_string()),
            ("XEXAM_LINK_DIR", dir.display().to_string()),
        ]
        .into_iter()
        .collect();
        let mut cfg = Config::from_env_with(&|k| map.get(k).cloned());
        Link::new(&mut cfg).expect("link")
    }

    #[test]
    fn env_token_rules_and_texts_match_the_backend() {
        let ok = "a".repeat(43);
        assert_eq!(token_problem(&ok), None);
        assert_eq!(token_problem(&"a".repeat(32)), None);
        // 40 characters with a space inside: the reason is the space, not the length
        let inner = format!("{} {}", "a".repeat(20), "b".repeat(19));
        assert_eq!(inner.len(), 40);
        assert_eq!(token_problem(&inner), Some(NOT_PRINTABLE));
        // a short value with another character: the character is named first, as in the backend
        assert_eq!(token_problem("short\u{e9}"), Some(NOT_PRINTABLE));
        assert_eq!(token_problem("too-short"), Some(TOO_SHORT));
        assert_eq!(token_problem(&"a".repeat(31)), Some(TOO_SHORT));

        // invalid (also once trimmed): the link is off, no file fallback
        for bad in [
            inner,
            "too-short".to_string(),
            format!(" {} ", "a".repeat(31)),
        ] {
            let link = link_with_env_token(&bad);
            assert!(!link.has_token(), "{bad:?}");
            assert!(!link.from_env(), "{bad:?}");
            assert!(!link.load(), "{bad:?}");
        }
        // whitespace around the value is removed, as the backend does: both services use the same token
        for padded in [
            ok.clone(),
            format!("{ok} "),
            format!("{ok}\r"),
            format!("{ok}\r\n"),
            format!("\t {ok}"),
        ] {
            let link = link_with_env_token(&padded);
            assert!(link.has_token(), "{padded:?}");
            assert!(link.from_env(), "{padded:?}");
            assert_eq!(
                link.token().as_deref().map(String::as_str),
                Some(ok.as_str()),
                "{padded:?}"
            );
        }
    }

    #[test]
    fn a_verified_statement_does_not_mean_connected() {
        let link = link_with_env_token(&"a".repeat(43));
        link.note_verified();
        link.note_verified();
        assert!(!link.is_connected());
        link.note_ok();
        assert!(link.is_connected());
        link.note_verified();
        assert!(link.is_connected());
        link.note_failed();
        assert!(!link.is_connected());
        link.note_verified();
        assert!(!link.is_connected());
    }
}
