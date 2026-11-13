//! The AI file import routes (spec IJ-2, API-U6, API-U7): the raw upload and the confirm. The file is read in memory
//! only; the extraction and every AI step run in `import::job`.
use super::{body_id, json_response, only_keys, require_ai_user, Answer, Ctx};
use crate::app::AppState;
use crate::codes::ApiError;
use crate::extract::sniff::{check_ext, ext_of};
use crate::extract::{sniff_magic, ExtractError, LangHint};
use crate::features::Feature;
use crate::gen::runner::{self, gen_status};
use crate::gen::wire::{self, Reason, WireError};
use crate::gen::{bank_mode_name, check_bank_name, clean_file_name, Composition, NameError};
use crate::identity::Identity;
use crate::import::job::{spawn_import, ImportJob};
use crate::store::{data, jobs};
use crate::util::{local_second, local_stamp, unix_now};
use axum::extract::Request;
use axum::http::{header, HeaderMap, StatusCode};
use serde_json::{json, Value};

/// Percent-decodes a header value (`encodeURIComponent`); `None` when the result is not valid UTF-8.
pub fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = b.get(i + 1).and_then(|c| (*c as char).to_digit(16))?;
            let l = b.get(i + 2).and_then(|c| (*c as char).to_digit(16))?;
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Removes the user from the set of uploads in progress when dropped (SEC-7).
struct UploadGuard {
    app: AppState,
    user: i64,
}

impl Drop for UploadGuard {
    fn drop(&mut self) {
        self.app
            .0
            .uploading
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.user);
    }
}

fn octet_stream(h: &HeaderMap) -> bool {
    h.get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/octet-stream"))
}

fn type_error(e: ExtractError, ext: &str) -> Option<ApiError> {
    match e {
        ExtractError::Legacy => Some(ApiError::import_type("ai.import.legacy", ext)),
        ExtractError::Image => Some(ApiError::import_type("ai.import.image", ext)),
        ExtractError::Unsupported => Some(ApiError::import_type("ai.import.type", ext)),
        _ => None,
    }
}

/// `POST /api/ai/import/upload` (IJ-2): the order of the steps is the order of the refusals. Called after the
/// identity check; the body is read here, by its own reader, with the limit of the setting.
pub async fn upload(app: AppState, who: Identity, req: Request) -> Answer {
    // the guard order after the identity: Content-Type exactly application/octet-stream
    if !octet_stream(req.headers()) {
        return Err(ApiError::bad_media_type());
    }
    // 1. AI usable for this user, 2. the feature
    let snap = require_ai_user(&app, &who)?;
    drop(snap);
    if !app.0.feature_allowed(&who, Feature::Import) {
        return Err(ApiError::feature_off(Feature::Import));
    }
    // 3. headers
    let l = app.0.limits();
    let raw_name = req
        .headers()
        .get("x-filename")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(ApiError::bad_request)?;
    let decoded = percent_decode(raw_name).ok_or_else(ApiError::bad_request)?;
    let name = clean_file_name(&decoded);
    let n = name.chars().count();
    if !(1..=200).contains(&n) {
        return Err(ApiError::bad_request());
    }
    let ext = ext_of(&name);
    if let Err(e) = check_ext(&ext) {
        if let Some(err) = type_error(e, &ext) {
            return Err(err);
        }
    }
    let max_mb = l.import_max_mb;
    let limit = usize::try_from(max_mb).unwrap_or(20) << 20;
    let declared = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .ok_or_else(ApiError::length_required)?;
    if declared > limit as u64 {
        return Err(ApiError::import_too_large(max_mb));
    }
    if declared == 0 {
        return Err(ApiError::import_empty());
    }
    let lang = req
        .headers()
        .get("x-ui-lang")
        .and_then(|v| v.to_str().ok())
        .and_then(LangHint::parse)
        .unwrap_or(LangHint::Zh);
    // 4. busy: one upload at a time per user (SEC-7), and one AI bank job per user
    let guard = {
        let mut set = app.0.uploading.lock().unwrap_or_else(|e| e.into_inner());
        if !set.insert(who.id) {
            return Err(ApiError::gen_busy(Value::Null));
        }
        UploadGuard {
            app: app.clone(),
            user: who.id,
        }
    };
    let now = unix_now();
    if let Some(active) = app.0.store.read(|c| jobs::active_for_user(c, who.id))? {
        return Err(ApiError::gen_busy(active.to_json()));
    }
    // 5. quota
    let q = app
        .0
        .store
        .read(|c| runner::quota_of(c, &app.0, &who, now))?;
    if q.exhausted() {
        return Err(ApiError::gen_quota(q.used, q.limit, q.day.next, now));
    }
    // 6. the backend's permission check (API-I5)
    let bytes = app
        .0
        .link
        .data_call("/internal/ai/import/check", json!({"sid": who.sid}))
        .await
        .map_err(|_| ApiError::backend_down())?;
    let user = wire::parse_import_check(&bytes).map_err(|e| match e {
        WireError::Refused(Reason::NotAllowed) => ApiError::gen_forbidden(),
        WireError::Refused(_) => ApiError::not_allowed(),
        WireError::Unreadable => ApiError::backend_down(),
    })?;
    if user.id != who.id {
        return Err(ApiError::not_allowed());
    }
    // 7. a place for the bytes in memory
    let pending = app
        .0
        .import_pending
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::server_busy())?;
    // 8. the body (at most limit + 1 bytes), then the magic bytes
    let body = axum::body::to_bytes(req.into_body(), limit + 1)
        .await
        .map_err(|_| ApiError::bad_request())?;
    if body.len() > limit {
        return Err(ApiError::import_too_large(max_mb));
    }
    if body.is_empty() {
        return Err(ApiError::import_empty());
    }
    let kind = match sniff_magic(&body, &ext) {
        Ok(k) => Some(k),
        Err(e) => {
            if let Some(err) = type_error(e, &ext) {
                return Err(err);
            }
            None
        }
    };
    let type_word = kind
        .or_else(|| crate::extract::FileKind::from_ext(&ext))
        .map_or("txt", |k| k.as_str())
        .to_string();
    // 9. one write transaction: the user's row, the quota again, the job (not charged yet)
    let key = crate::util::hex(&runner::random_bytes::<16>());
    let stem = match name.rsplit_once('.') {
        Some((s, _)) if !s.trim().is_empty() => s.to_string(),
        _ => "file".to_string(),
    };
    let bank_name = bank_mode_name(&stem, &who.username, &local_stamp(now));
    let comp = Composition {
        single: usize::try_from(l.gen_bank_single).unwrap_or(0),
        multi: usize::try_from(l.gen_bank_multi).unwrap_or(0),
    };
    let payload = json!({
        "source": "file",
        "name": bank_name,
        "file": {"name": name, "type": type_word, "size": body.len()},
        "mode": null,
        "read": {"done": 0, "total": 0},
        "confirmBy": null,
        "errorArg": null,
        "comp": {"single": comp.single, "multi": comp.multi},
        "callTokens": l.gen_call_tokens,
        "maxQuestions": l.import_max_questions,
    });
    let ins = app
        .0
        .store
        .write(|tx| -> Result<Result<i64, bool>, rusqlite::Error> {
            data::ensure_user(tx, &who, now)?;
            let q = runner::quota_of(tx, &app.0, &who, now)?;
            if q.exhausted() {
                return Ok(Err(true));
            }
            match jobs::insert(
                tx,
                &jobs::NewJob {
                    user_id: who.id,
                    job_key: &key,
                    goal: 0,
                    payload: &payload,
                    charged: false,
                    phase: Some("reading"),
                    now,
                },
            ) {
                Ok(id) => Ok(Ok(id)),
                Err(jobs::InsertError::Busy) => Ok(Err(false)),
                Err(jobs::InsertError::Db(e)) => Err(e),
            }
        })?;
    let task = match ins {
        Ok(id) => id,
        Err(true) => {
            let q = app
                .0
                .store
                .read(|c| runner::quota_of(c, &app.0, &who, now))?;
            return Err(ApiError::gen_quota(q.used, q.limit, q.day.next, now));
        }
        Err(false) => {
            let active = app.0.store.read(|c| jobs::active_for_user(c, who.id))?;
            return Err(ApiError::gen_busy(
                active.map_or(Value::Null, |j| j.to_json()),
            ));
        }
    };
    spawn_import(
        &app,
        ImportJob {
            task,
            key,
            who: who.clone(),
            name: bank_name,
            file_name: name,
            file_type: type_word,
            ext,
            lang,
            bytes: body.to_vec(),
            comp,
            call_chars: crate::limits::Limits::usize_of(crate::limits::tokens_to_chars(
                l.import_tokens(),
            )),
            max_questions: usize::try_from(l.import_max_questions).unwrap_or(50),
            at: local_second(now),
            pending: Some(pending),
        },
    );
    drop(guard);
    let row = app
        .0
        .store
        .read(|c| jobs::get(c, task))?
        .ok_or_else(ApiError::internal)?;
    let gen = gen_status(&app, &who, now)?;
    Ok(json_response(
        StatusCode::ACCEPTED,
        &json!({"job": row.to_json(), "gen": gen}),
    ))
}

/// `POST /api/ai/import/confirm {"id"}` (API-U7): the recognised file is imported. Refusals leave the job waiting.
pub async fn confirm(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &["id", "name"])?;
    let id = body_id(&ctx.body, "id")?.ok_or_else(ApiError::bad_request)?;
    // A-OWNER-57: the bank name typed at the confirm step is required (trimmed, same rules as any bank name)
    let name = match ctx.body.get("name") {
        None | Some(Value::Null) => return Err(ApiError::import_name_empty()),
        Some(Value::String(raw)) => check_bank_name(raw).map_err(|e| match e {
            NameError::Empty => ApiError::import_name_empty(),
            NameError::TooLong | NameError::Control => {
                ApiError::gen_bad_name(crate::gen::NAME_MAX_BANK as i64)
            }
        })?,
        Some(_) => return Err(ApiError::bad_request()),
    };
    let app = ctx.app.clone();
    let who = ctx.who.clone();
    require_ai_user(&app, &who)?;
    if !app.0.feature_allowed(&who, Feature::Import) {
        return Err(ApiError::feature_off(Feature::Import));
    }
    let row = app
        .0
        .store
        .read(|c| jobs::get_for_user(c, who.id, id))?
        .ok_or_else(ApiError::gen_not_found)?;
    if !row.is_file() || !row.active() || row.phase.as_deref() != Some("confirm") {
        return Err(ApiError::import_not_waiting(row.to_json()));
    }
    // the bank-management permission may have been taken away during the wait (SEC-8)
    let bytes = app
        .0
        .link
        .data_call(
            "/internal/ai/import/check",
            json!({"sid": who.sid, "name": name}),
        )
        .await
        .map_err(|_| ApiError::backend_down())?;
    let user = wire::parse_import_check_named(&bytes).map_err(|e| match e {
        WireError::Refused(Reason::NotAllowed) => ApiError::gen_forbidden(),
        WireError::Refused(_) => ApiError::not_allowed(),
        WireError::Unreadable => ApiError::backend_down(),
    })?;
    let (user, taken) = user;
    if user.id != who.id {
        return Err(ApiError::not_allowed());
    }
    if taken {
        return Err(ApiError::import_name_taken());
    }
    let now = unix_now();
    enum Outcome {
        Quota,
        NotWaiting,
        Done,
    }
    let out = app.0.store.write(|tx| -> rusqlite::Result<Outcome> {
        let q = runner::quota_of(tx, &app.0, &who, now)?;
        if q.exhausted() {
            return Ok(Outcome::Quota);
        }
        // the chosen name replaces the suggested one (the runner reads it back after the confirm)
        if let Some(row) = jobs::get(tx, id)? {
            let mut p = row.payload.clone();
            if let Some(o) = p.as_object_mut() {
                o.insert("name".into(), json!(name));
            }
            jobs::update_payload(tx, id, &p, None, now)?;
        }
        Ok(if jobs::confirm(tx, id, now)? == 0 {
            Outcome::NotWaiting
        } else {
            Outcome::Done
        })
    })?;
    match out {
        Outcome::Quota => {
            let q = app
                .0
                .store
                .read(|c| runner::quota_of(c, &app.0, &who, now))?;
            Err(ApiError::gen_quota(q.used, q.limit, q.day.next, now))
        }
        Outcome::NotWaiting => {
            let row = app
                .0
                .store
                .read(|c| jobs::get_for_user(c, who.id, id))?
                .ok_or_else(ApiError::gen_not_found)?;
            Err(ApiError::import_not_waiting(row.to_json()))
        }
        Outcome::Done => {
            app.0.jobs.notify_confirm(id);
            let row = app
                .0
                .store
                .read(|c| jobs::get(c, id))?
                .ok_or_else(ApiError::internal)?;
            let gen = gen_status(&app, &who, now)?;
            super::ok(json!({"job": row.to_json(), "gen": gen}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decoding_of_file_names() {
        assert_eq!(
            percent_decode("Kitchen%20notes.pdf").as_deref(),
            Some("Kitchen notes.pdf")
        );
        assert_eq!(
            percent_decode("%E6%BC%A2%E5%AD%97.docx").as_deref(),
            Some("漢字.docx")
        );
        assert_eq!(percent_decode("plain.txt").as_deref(), Some("plain.txt"));
        assert_eq!(percent_decode("%FF%FE.txt"), None, "not UTF-8");
        assert_eq!(percent_decode("bad%2"), None);
        assert_eq!(percent_decode("bad%zz"), None);
    }

    #[test]
    fn only_octet_stream_is_accepted() {
        let mut h = HeaderMap::new();
        assert!(!octet_stream(&h));
        for (v, ok) in [
            ("application/octet-stream", true),
            ("Application/Octet-Stream", true),
            ("application/json", false),
            ("multipart/form-data; boundary=x", false),
            ("text/plain", false),
        ] {
            h.insert(header::CONTENT_TYPE, v.parse().expect("hv"));
            assert_eq!(octet_stream(&h), ok, "{v}");
        }
    }
}
