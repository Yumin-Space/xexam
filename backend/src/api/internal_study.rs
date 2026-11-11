//! v0.9: the three token routes the AI service calls to read study data, `POST /internal/ai/question`,
//! `POST /internal/ai/record` and `POST /internal/ai/record-info` (the projections themselves are in
//! `database/store/study.rs`).
//!
//! [`super::internal::dispatch`] has already checked the internal token, the method, the content type and the body size
//! (at most 64 KiB) and parsed the body as a JSON object; this module validates the keys, binds the call to the caller's
//! live session and answers.
//!
//! - Validation: exactly the listed keys (an unknown or missing key, a wrong type or an out-of-range value is 400
//!   `MSG_BAD_REQUEST`). `sid` is 64 lower-case hex digits, `id` / `recordId` positive integers, `questionId`
//!   `^[0-9A-Za-z_-]{1,64}$` (so it is safe in a URL query without encoding), `qlang` / `elang` one of `en`, `zh`, `ja`, `max`
//!   1..=70, `record` (optional, `null` counts as absent) exactly `{"id": n, "kind": "practice" | "exam"}`.
//! - Session binding: `store.session_user(sid, now, false)` (no refresh, like `introspect`). No valid session, another user's
//!   session or a pending password change answers `{"ok": false, "reason": "session"}`; a user without the AI+ feature
//!   (administrators always have it) answers `{"ok": false, "reason": "notAllowed"}` (not on `record-info`: it only names the
//!   caller's own record, for the AI service's read-only `saved` routes, which stay readable when AI is off).
//! - Not found (a missing question, a question of a bank the caller cannot see, a record that is not the caller's, a kind
//!   mismatch, a question that is not in that record) is one answer, `{"ok": false, "reason": "notFound"}`, always HTTP 200: the
//!   AI client reads an HTTP 404 from `/internal/*` as "token refused".
//! - Nothing is written and nothing is logged by a call.
use super::response::MSG_BAD_REQUEST;
use super::{blocking, AppState};
use crate::database::store::{QuestionAsk, StudyLang, MAX_SAMPLE};
use crate::error::AppError;
use crate::models::AttemptKind;
use serde_json::{json, Map, Value};

fn bad_request() -> AppError {
    AppError::BadRequest(MSG_BAD_REQUEST.to_string())
}

fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `^[0-9A-Za-z_-]{1,64}$`
fn is_question_id(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `data` must have exactly the keys in `required`, plus any of `optional`.
fn check_keys(
    data: &Map<String, Value>,
    required: &[&str],
    optional: &[&str],
) -> Result<(), AppError> {
    let known = |k: &String| required.contains(&k.as_str()) || optional.contains(&k.as_str());
    if !data.keys().all(known) || !required.iter().all(|k| data.contains_key(*k)) {
        return Err(bad_request());
    }
    Ok(())
}

fn positive(data: &Map<String, Value>, key: &str) -> Result<i64, AppError> {
    data.get(key)
        .and_then(Value::as_i64)
        .filter(|i| *i > 0)
        .ok_or_else(bad_request)
}

fn lang(data: &Map<String, Value>, key: &str) -> Result<StudyLang, AppError> {
    data.get(key)
        .and_then(Value::as_str)
        .and_then(StudyLang::parse)
        .ok_or_else(bad_request)
}

fn sid(data: &Map<String, Value>) -> Result<String, AppError> {
    match data.get("sid") {
        Some(Value::String(s)) if is_lower_hex(s, 64) => Ok(s.clone()),
        _ => Err(bad_request()),
    }
}

/// `{"ok": false, "reason": ...}` (HTTP 200).
fn refused(reason: &str) -> Value {
    json!({ "ok": false, "reason": reason })
}

/// Binds the call to the caller: `Ok(Ok(user id))`, or `Ok(Err(answer))` with the refusal to send as it is.
async fn bind_session(
    st: &AppState,
    sid: String,
    id: i64,
    needs_ai: bool,
) -> Result<Result<i64, Value>, AppError> {
    let store = st.0.store.clone();
    let now = crate::auth::unix_now();
    let found = blocking(move || store.session_user(&sid, now, false)).await?;
    Ok(match found {
        Some((u, _)) if u.id == id && !u.must_change_password => {
            if u.ai_allowed || !needs_ai {
                Ok(u.id)
            } else {
                Err(refused("notAllowed"))
            }
        }
        _ => Err(refused("session")),
    })
}

/// The `record` of a question request, if any.
fn record_ref(data: &Map<String, Value>) -> Result<Option<(i64, AttemptKind)>, AppError> {
    match data.get("record") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(m)) => {
            check_keys(m, &["id", "kind"], &[])?;
            let kind = m
                .get("kind")
                .and_then(Value::as_str)
                .and_then(AttemptKind::parse)
                .ok_or_else(bad_request)?;
            Ok(Some((positive(m, "id")?, kind)))
        }
        Some(_) => Err(bad_request()),
    }
}

/// `POST /internal/ai/question {"sid", "id", "questionId", "qlang", "elang", "record"?}`: one question the caller may see
/// with its answer side, and what they answered in `record` (see `Store::study_question`).
pub(super) async fn question(st: &AppState, data: &Map<String, Value>) -> Result<Value, AppError> {
    check_keys(
        data,
        &["sid", "id", "questionId", "qlang", "elang"],
        &["record"],
    )?;
    let sid = sid(data)?;
    let id = positive(data, "id")?;
    let question_id = match data.get("questionId") {
        Some(Value::String(s)) if is_question_id(s) => s.clone(),
        _ => return Err(bad_request()),
    };
    let ask = QuestionAsk {
        question_id,
        qlang: lang(data, "qlang")?,
        elang: lang(data, "elang")?,
        record: record_ref(data)?,
    };
    let user = match bind_session(st, sid, id, true).await? {
        Ok(user) => user,
        Err(answer) => return Ok(answer),
    };
    let store = st.0.store.clone();
    let found = blocking(move || store.study_question(user, &ask)).await?;
    Ok(found.unwrap_or_else(|| refused("notFound")))
}

/// `POST /internal/ai/record {"sid", "id", "recordId", "max", "elang"}`: the caller's own record with at most `max` (1..=70)
/// of the questions they answered wrong, sampled at random (see `Store::study_record`).
pub(super) async fn record(st: &AppState, data: &Map<String, Value>) -> Result<Value, AppError> {
    check_keys(data, &["sid", "id", "recordId", "max", "elang"], &[])?;
    let sid = sid(data)?;
    let id = positive(data, "id")?;
    let record_id = positive(data, "recordId")?;
    let max = usize::try_from(positive(data, "max")?)
        .ok()
        .filter(|m| *m <= MAX_SAMPLE)
        .ok_or_else(bad_request)?;
    let elang = lang(data, "elang")?;
    let user = match bind_session(st, sid, id, true).await? {
        Ok(user) => user,
        Err(answer) => return Ok(answer),
    };
    let store = st.0.store.clone();
    let found = blocking(move || store.study_record(user, record_id, max, elang)).await?;
    Ok(found.unwrap_or_else(|| refused("notFound")))
}

/// `POST /internal/ai/record-info {"sid", "id", "recordId"}`: the identity of the caller's own record, `{"ok": true, "uiLang",
/// "record": {"id", "at", "kind", "key", "startedAt"}}` (see `Store::study_record_info`). Does not need the AI+ feature.
pub(super) async fn record_info(
    st: &AppState,
    data: &Map<String, Value>,
) -> Result<Value, AppError> {
    check_keys(data, &["sid", "id", "recordId"], &[])?;
    let sid = sid(data)?;
    let id = positive(data, "id")?;
    let record_id = positive(data, "recordId")?;
    let user = match bind_session(st, sid, id, false).await? {
        Ok(user) => user,
        Err(answer) => return Ok(answer),
    };
    let store = st.0.store.clone();
    let found = blocking(move || store.study_record_info(user, record_id)).await?;
    Ok(found.unwrap_or_else(|| refused("notFound")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().expect("object")
    }

    #[test]
    fn question_ids_are_url_safe_and_at_most_64_characters() {
        for ok in ["a", "0123456789ab", "A_b-9", &"x".repeat(64)] {
            assert!(is_question_id(ok), "{ok}");
        }
        for bad in [
            "",
            &"x".repeat(65),
            "a%20b",
            "a&b",
            "a b",
            "a/b",
            "é",
            "a.b",
            "a=b",
        ] {
            assert!(!is_question_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn keys_are_exact() {
        let m = obj(json!({"a": 1, "b": 2}));
        assert!(check_keys(&m, &["a", "b"], &[]).is_ok());
        assert!(check_keys(&m, &["a"], &["b"]).is_ok());
        assert!(check_keys(&m, &["a"], &[]).is_err(), "unknown key");
        assert!(
            check_keys(&m, &["a", "b", "c"], &[]).is_err(),
            "missing key"
        );
    }

    #[test]
    fn record_refs_are_strict() {
        let r = |v: Value| record_ref(&obj(json!({ "record": v })));
        assert_eq!(r(Value::Null).ok(), Some(None));
        assert_eq!(record_ref(&Map::new()).ok(), Some(None));
        assert_eq!(
            r(json!({"id": 3, "kind": "exam"})).ok(),
            Some(Some((3, AttemptKind::Exam)))
        );
        assert_eq!(
            r(json!({"id": 3, "kind": "practice"})).ok(),
            Some(Some((3, AttemptKind::Practice)))
        );
        for bad in [
            json!({"id": 0, "kind": "exam"}),
            json!({"id": -1, "kind": "exam"}),
            json!({"id": 1.5, "kind": "exam"}),
            json!({"id": "1", "kind": "exam"}),
            json!({"id": 1, "kind": "quiz"}),
            json!({"id": 1}),
            json!({"kind": "exam"}),
            json!({"id": 1, "kind": "exam", "x": 1}),
            json!("exam"),
            json!(5),
        ] {
            assert!(r(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn hex_session_ids_are_64_lower_case_digits() {
        let ok = "a".repeat(64);
        assert!(sid(&obj(json!({ "sid": ok }))).is_ok());
        for bad in [
            json!("A".repeat(64)),
            json!("a".repeat(63)),
            json!("g".repeat(64)),
            json!(5),
        ] {
            assert!(sid(&obj(json!({ "sid": bad }))).is_err());
        }
        assert!(sid(&Map::new()).is_err());
    }
}
