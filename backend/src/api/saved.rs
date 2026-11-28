//! v3.1 saved-session marker: `POST /api/study/saved` and `POST /api/study/saved/discard` (contracts B.9).
//!
//! The app reports an unfinished practice / exam; the server keeps one marker per session (the key of the attempt tag) and
//! the reminder tick of the push scheduler works from it. A finished session (`POST /api/attempt`) clears its marker.

use super::auth::Authed;
use super::json::PyVal;
use super::response::{json_resp, MSG_BAD_REQUEST};
use super::{blocking, AppState};
use crate::database::store::SavedReport;
use crate::error::AppError;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::json;

/// Largest number of questions in one session.
const MAX_TOTAL: i128 = 100_000;
/// `startedAt` (the browser clock, unix seconds) is below this.
const MAX_STARTED_AT: i128 = 1 << 41;
/// Largest `timeLeft` (seconds): the longest exam limit.
const MAX_TIME_LEFT: i128 = 36_000;

fn bad_request() -> AppError {
    AppError::BadRequest(MSG_BAD_REQUEST.to_string())
}

/// A session key: 32 lower-case hex digits.
fn key_of(data: &PyVal) -> Result<String, AppError> {
    match data.get("key") {
        Some(PyVal::Str(s))
            if s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) =>
        {
            Ok(s.clone())
        }
        _ => Err(bad_request()),
    }
}

fn int_in(v: Option<&PyVal>, lo: i128, hi: i128) -> Result<i64, AppError> {
    match v {
        Some(PyVal::Int(i)) if (lo..=hi).contains(i) => Ok(*i as i64),
        _ => Err(bad_request()),
    }
}

/// `POST /api/study/saved` `{"key", "kind", "answered", "total", "startedAt", "timeLeft"}` → `{"ok": true}`.
pub(super) async fn report(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    let Some(device) = a.device_id else {
        return Err(bad_request());
    };
    if !data.is_dict() {
        return Err(bad_request());
    }
    let key = key_of(data)?;
    let kind = match data.get("kind") {
        Some(PyVal::Str(s)) if s == "exam" || s == "practice" => s.clone(),
        _ => return Err(bad_request()),
    };
    let total = int_in(data.get("total"), 1, MAX_TOTAL)?;
    let answered = int_in(data.get("answered"), 0, i128::from(total))?;
    let started_at = int_in(data.get("startedAt"), 1, MAX_STARTED_AT - 1)?;
    let time_left = match data.get("timeLeft") {
        None | Some(PyVal::Null) => None,
        // Only a timed exam has a time left
        v @ Some(_) if kind == "exam" => Some(int_in(v, 0, MAX_TIME_LEFT)?),
        Some(_) => return Err(bad_request()),
    };
    let report = SavedReport {
        key,
        kind,
        answered,
        total,
        started_at,
        time_left,
    };
    let store = st.0.store.clone();
    let user = a.user.id;
    let now = store.unix_now();
    // A disabled account answers 401 through `blocking`; a device that is not the user's cannot happen for a session row
    if !blocking(move || store.saved_report(user, device, &report, now)).await? {
        return Err(bad_request());
    }
    Ok(json_resp(StatusCode::OK, &json!({ "ok": true }), false))
}

/// `POST /api/study/saved/discard` `{"key"}` → `{"ok": true}` (no marker is not an error).
pub(super) async fn discard(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    if !data.is_dict() {
        return Err(bad_request());
    }
    let key = key_of(data)?;
    let store = st.0.store.clone();
    let user = a.user.id;
    blocking(move || store.saved_discard(user, &key)).await?;
    Ok(json_resp(StatusCode::OK, &json!({ "ok": true }), false))
}
