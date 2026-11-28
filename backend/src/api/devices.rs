//! v3.1 devices: `GET /api/devices`, `POST /api/devices/{hello,signout,signout-others}`, `GET /api/users/devices` and
//! `POST /api/users/devices/signout` (contracts B.5). The device list of the account (all states), the app's hello at every
//! start, and the sign-out of one device, of every other device, or (administrators) of a user's devices.
//!
//! Every answer that lists devices is `{"devices": [DeviceJson], "signinInfo": bool}` (the administrator's variants add
//! `"user"`); IP and place fields are `null` while the sign-in info eye is closed. The device a request belongs to comes from
//! its session row ([`Authed::device_id`]), never from the cookie; a session that has no device yet is linked by the session
//! lookup of a POST that carries an Origin (the app's hello), see `api::auth`.

use super::auth::Authed;
use super::json::PyVal;
use super::response::{
    json_resp, MSG_BAD_REQUEST, MSG_DEVICE_CURRENT, MSG_DEVICE_NOT_FOUND, MSG_USER_NOT_FOUND,
};
use super::{bad_request, blocking, body_id, query_once, AppState};
use crate::activity::{with_note, Actor, Entry};
use crate::database::store::DeviceChange;
use crate::error::AppError;
use crate::models::tz_name_ok;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{json, Value};

fn not_found() -> AppError {
    AppError::Coded {
        status: 404,
        msg: MSG_DEVICE_NOT_FOUND,
        code: "devices.notFound",
    }
}

fn current() -> AppError {
    AppError::Coded {
        status: 409,
        msg: MSG_DEVICE_CURRENT,
        code: "devices.current",
    }
}

fn ok(v: &Value) -> Response {
    json_resp(StatusCode::OK, v, false)
}

/// The id of a device in a request body: an integer (a float, a string or a missing key is a bad request).
fn device_field(data: &PyVal, key: &str) -> Result<i64, AppError> {
    body_id(data.get(key))?.ok_or_else(bad_request)
}

/// The own list of `user` with `current` marked, as the body of the answers.
async fn own_body(st: &AppState, user: i64, current: Option<i64>) -> Result<Value, AppError> {
    let store = st.0.store.clone();
    blocking(move || store.devices_json(user, current)).await
}

/// `GET /api/devices` → `{"devices": [...], "signinInfo": bool}`: the caller's own devices in every state, the current one
/// first, then the most recently active.
pub(super) async fn list(st: &AppState, a: &Authed) -> Result<Response, AppError> {
    Ok(ok(&own_body(st, a.user.id, a.device_id).await?))
}

/// `POST /api/devices/hello` `{"tz": string|null, "app": bool, "permission": "granted"|"denied"|"default"|"unsupported"}`
/// → `{"device": DeviceJson}`. Stores the device's zone and installed-app flag; a permission other than `granted` removes
/// the device's push subscription (and records `settings.pushOff`). A session without a device was linked by the lookup
/// of this very request; when it still has none (no Origin, so no cookie could be made) the answer is 400.
pub(super) async fn hello(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    let tz = match data.get("tz") {
        Some(PyVal::Null) => None,
        Some(PyVal::Str(s)) => Some(s.as_str()),
        _ => return Err(bad_request()),
    };
    let app = match data.get("app") {
        Some(PyVal::Bool(b)) => *b,
        _ => return Err(bad_request()),
    };
    let permission = match data.get("permission") {
        Some(PyVal::Str(s))
            if matches!(s.as_str(), "granted" | "denied" | "default" | "unsupported") =>
        {
            s.clone()
        }
        _ => return Err(bad_request()),
    };
    let Some(device) = a.device_id else {
        return Err(AppError::BadRequest(MSG_BAD_REQUEST.to_string()));
    };
    // An invalid zone is stored as NULL, never refused: the app reports what the browser says
    let tz = tz.filter(|s| tz_name_ok(s)).map(str::to_string);
    let store = st.0.store.clone();
    let user = a.user.id;
    let remove = permission != "granted";
    let (removed, device_json) =
        blocking(
            move || match store.device_hello(user, device, tz.as_deref(), app, remove)? {
                DeviceChange::NotFound => Ok(None),
                DeviceChange::Done(removed) => Ok(Some((
                    removed,
                    store.device_json(user, device, Some(device))?,
                ))),
            },
        )
        .await?
        .ok_or_else(not_found)?;
    if removed {
        st.0.activity.record(
            Entry::new(Actor::of(&a.user), "settings.pushOff")
                .detail(json!({ "reason": permission }))
                .client(&a.client),
        );
    }
    Ok(ok(&json!({ "device": device_json })))
}

/// `POST /api/devices/signout` `{"id": int}` → `{"devices": [...], "signinInfo": bool}`: signs one of the caller's other
/// devices out (its sessions, push subscription and queued pushes go; the row stays as `signedOut`). On a device whose
/// login already expired this is the "Stop" of the list. 404 when the id is not the caller's, 409 for the current device.
pub(super) async fn signout(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    let id = device_field(data, "id")?;
    if a.device_id == Some(id) {
        return Err(current());
    }
    let store = st.0.store.clone();
    let user = a.user.id;
    match blocking(move || store.device_signout(user, id)).await? {
        DeviceChange::NotFound => return Err(not_found()),
        DeviceChange::Done(_) => {}
    }
    let body = own_body(st, user, a.device_id).await?;
    Ok(with_note(ok(&body), None, Some(json!({ "device": id }))))
}

/// `POST /api/devices/signout-others` `{}` → `{"devices": [...], "signinInfo": bool, "count": int}`: signs out every device
/// except the current one; `count` = the devices that had a session or a subscription.
pub(super) async fn signout_others(st: &AppState, a: &Authed) -> Result<Response, AppError> {
    let store = st.0.store.clone();
    let user = a.user.id;
    let keep_device = a.device_id;
    let keep_token = a.token_hash.clone();
    let count =
        blocking(move || store.devices_signout_user(user, keep_device, Some(&keep_token))).await?;
    let mut body = own_body(st, user, a.device_id).await?;
    body["count"] = json!(count);
    Ok(with_note(ok(&body), None, Some(json!({ "count": count }))))
}

/// The id in `?user=<id>`: 1 to 18 ASCII digits, given once.
fn user_query(query: Option<&str>) -> Result<i64, AppError> {
    match query_once(query, "user")? {
        Some(s) if (1..=18).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) => {
            s.parse().map_err(|_| bad_request())
        }
        _ => Err(bad_request()),
    }
}

/// `{"user": {...}, "devices": [...], "signinInfo": bool}` of `user` (404 `MSG_USER_NOT_FOUND`), as the administrator `a`
/// sees it: only their own account's list marks a current device. Also returns the username (the log object).
async fn user_body(st: &AppState, a: &Authed, user: i64) -> Result<(Value, String), AppError> {
    let store = st.0.store.clone();
    let current = (user == a.user.id).then_some(a.device_id).flatten();
    let found = blocking(move || {
        let Some(u) = store.user_by_id(user)? else {
            return Ok(None);
        };
        Ok(Some((u, store.devices_json(user, current)?)))
    })
    .await?;
    let Some((u, list)) = found else {
        return Err(AppError::Status(404, MSG_USER_NOT_FOUND));
    };
    let body = json!({
        "user": { "id": u.id, "username": u.username, "displayName": u.display_name },
        "devices": list["devices"],
        "signinInfo": list["signinInfo"],
    });
    Ok((body, u.username))
}

/// `GET /api/users/devices?user=<id>` (Admin): the devices of any account. Records nothing (a GET).
pub(super) async fn admin_list(
    st: &AppState,
    a: &Authed,
    query: Option<&str>,
) -> Result<Response, AppError> {
    let user = user_query(query)?;
    let (body, _) = user_body(st, a, user).await?;
    Ok(ok(&body))
}

/// `POST /api/users/devices/signout` `{"user": int, "id": int|null}` (Admin) → the body of the GET for that user. `id` = a
/// device of that user; `null` = every device of the user (the caller's own account keeps the current one). The caller's
/// current device can never be signed out here (409).
pub(super) async fn admin_signout(
    st: &AppState,
    a: &Authed,
    data: &PyVal,
) -> Result<Response, AppError> {
    let user = device_field(data, "user")?;
    let id = match data.get("id") {
        Some(PyVal::Null) => None,
        Some(PyVal::Int(_)) => Some(device_field(data, "id")?),
        _ => return Err(bad_request()),
    };
    let own = user == a.user.id;
    if own && id.is_some() && id == a.device_id {
        return Err(current());
    }
    let store = st.0.store.clone();
    let count = match id {
        Some(device) => {
            let found = blocking(move || {
                if store.user_by_id(user)?.is_none() {
                    return Ok(None);
                }
                Ok(Some(store.device_signout(user, device)?))
            })
            .await?;
            match found {
                None => return Err(AppError::Status(404, MSG_USER_NOT_FOUND)),
                Some(DeviceChange::NotFound) => return Err(not_found()),
                Some(DeviceChange::Done(_)) => 1,
            }
        }
        None => {
            let keep_device = if own { a.device_id } else { None };
            let keep_token = own.then(|| a.token_hash.clone());
            let found = blocking(move || {
                if store.user_by_id(user)?.is_none() {
                    return Ok(None);
                }
                Ok(Some(store.devices_signout_user(
                    user,
                    keep_device,
                    keep_token.as_deref(),
                )?))
            })
            .await?;
            match found {
                Some(n) => n,
                None => return Err(AppError::Status(404, MSG_USER_NOT_FOUND)),
            }
        }
    };
    let (body, username) = user_body(st, a, user).await?;
    Ok(with_note(
        ok(&body),
        Some(username),
        Some(json!({ "all": id.is_none(), "count": count })),
    ))
}
