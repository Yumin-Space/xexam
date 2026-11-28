//! v3.1 web push routes: `GET /api/push/config`, `POST /api/push/subscribe`, `POST /api/push/test` (contracts B.8).
//!
//! The browser's subscription is validated here (key id, endpoint allow-list, the two keys) and stored by
//! [`crate::database::store::PushSubscribe`]; the notification texts, the quiet hours and the encryption are applied later,
//! when the courier leases a message (`api/internal.rs`).

use super::auth::Authed;
use super::json::PyVal;
use super::response::{
    json_resp, MSG_BAD_REQUEST, MSG_PUSH_BAD_ENDPOINT, MSG_PUSH_BAD_KEYS, MSG_PUSH_NO_DEVICE,
    MSG_PUSH_OFF, MSG_PUSH_STALE_KEY, MSG_PUSH_TEST_BUSY,
};
use super::{blocking, AppState};
use crate::activity::{Actor, Entry};
use crate::database::store::{PushSubscribe, PushSubscribed, PushTestOutcome};
use crate::error::AppError;
use crate::push::{allow, b64_lenient, ece};
use crate::security::base64url_nopad;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::json;

/// The largest `expirationTime` accepted (milliseconds): the largest JavaScript date.
const MAX_EXPIRATION_MS: i128 = 8_640_000_000_000_000;

fn bad_request() -> AppError {
    AppError::BadRequest(MSG_BAD_REQUEST.to_string())
}

fn coded(status: u16, msg: &'static str, code: &'static str) -> AppError {
    AppError::Coded { status, msg, code }
}

/// `GET /api/push/config` → `{"publicKey", "keyId", "courier"}`; the first two are `null` while push is off (no usable key).
pub(super) async fn config(st: &AppState, _a: &Authed) -> Result<Response, AppError> {
    let courier = st.0.push.courier_connected(st.0.store.unix_now());
    let body = match st.0.push.key() {
        Some(key) => {
            json!({"publicKey": key.public_b64(), "keyId": key.key_id(), "courier": courier})
        }
        None => json!({"publicKey": null, "keyId": null, "courier": courier}),
    };
    Ok(json_resp(StatusCode::OK, &body, false))
}

/// `POST /api/push/subscribe` `{"endpoint", "keys": {"p256dh", "auth"}, "expirationTime", "keyId"}` → `{"device": DeviceJson}`.
pub(super) async fn subscribe(
    st: &AppState,
    a: &Authed,
    data: &PyVal,
) -> Result<Response, AppError> {
    // The app always says hello first, which gives the session its device
    let Some(device) = a.device_id else {
        return Err(bad_request());
    };
    if !data.is_dict() {
        return Err(bad_request());
    }
    // 1. shapes
    let Some(PyVal::Str(endpoint)) = data.get("endpoint") else {
        return Err(bad_request());
    };
    let Some(PyVal::Str(key_id)) = data.get("keyId") else {
        return Err(bad_request());
    };
    let Some(keys @ PyVal::Dict(_)) = data.get("keys") else {
        return Err(bad_request());
    };
    let (Some(PyVal::Str(p256dh)), Some(PyVal::Str(auth))) = (keys.get("p256dh"), keys.get("auth"))
    else {
        return Err(bad_request());
    };
    let expires_at: Option<i64> = match data.get("expirationTime") {
        None | Some(PyVal::Null) => None,
        Some(PyVal::Int(ms)) if (0..=MAX_EXPIRATION_MS).contains(ms) => Some((*ms / 1000) as i64),
        Some(PyVal::Float(ms))
            if ms.is_finite() && *ms >= 0.0 && *ms <= MAX_EXPIRATION_MS as f64 =>
        {
            Some((*ms / 1000.0).floor() as i64)
        }
        _ => return Err(bad_request()),
    };
    // 2. the server's key may have changed since the app read /api/push/config
    match st.0.push.key() {
        Some(k) if k.key_id() == *key_id => {}
        _ => return Err(coded(409, MSG_PUSH_STALE_KEY, "push.staleKey")),
    }
    // 3. the endpoint
    if !allow::endpoint_allowed(endpoint) {
        return Err(coded(400, MSG_PUSH_BAD_ENDPOINT, "push.badEndpoint"));
    }
    // 4. the keys (padding and the standard alphabet are tolerated; the stored form is canonical)
    let bad_keys = || coded(400, MSG_PUSH_BAD_KEYS, "push.badKeys");
    let p = b64_lenient(p256dh).ok_or_else(bad_keys)?;
    let au = b64_lenient(auth).ok_or_else(bad_keys)?;
    if p.len() != 65 || p[0] != 0x04 || au.len() != 16 || !ece::check_public_key(&p) {
        return Err(bad_keys());
    }
    let req = PushSubscribe {
        endpoint: endpoint.clone(),
        p256dh: base64url_nopad(&p),
        auth: base64url_nopad(&au),
        key_id: key_id.clone(),
        expires_at,
    };
    let store = st.0.store.clone();
    let user = a.user.id;
    let now = store.unix_now();
    let outcome = blocking(move || store.push_subscribe(user, device, &req, now)).await?;
    match outcome {
        PushSubscribed::Off => return Err(coded(409, MSG_PUSH_OFF, "push.off")),
        PushSubscribed::NoDevice => return Err(bad_request()),
        PushSubscribed::New => {
            // A brand-new subscription for this device (not a re-confirm of the same endpoint)
            st.0.activity
                .record(Entry::new(Actor::of(&a.user), "settings.pushOn").client(&a.client));
        }
        PushSubscribed::Unchanged | PushSubscribed::Confirmed => {}
    }
    let store = st.0.store.clone();
    let device_json = blocking(move || store.device_json(user, device, Some(device))).await?;
    Ok(json_resp(
        StatusCode::OK,
        &json!({ "device": device_json }),
        false,
    ))
}

/// `POST /api/push/test` → `{"ok": true, "courier": bool}`: one test notification for this device (no quiet hours).
pub(super) async fn test(st: &AppState, a: &Authed) -> Result<Response, AppError> {
    let Some(device) = a.device_id else {
        return Err(bad_request());
    };
    let store = st.0.store.clone();
    let push = st.0.push.clone();
    let user = a.user.id;
    let now = store.unix_now();
    let outcome =
        blocking(move || store.push_test(user, device, now, || push.test_allowed(device, now)))
            .await?;
    match outcome {
        PushTestOutcome::Off => Err(coded(409, MSG_PUSH_OFF, "push.off")),
        PushTestOutcome::NoDevice => Err(coded(409, MSG_PUSH_NO_DEVICE, "push.noDevice")),
        PushTestOutcome::Busy => Err(coded(429, MSG_PUSH_TEST_BUSY, "push.testBusy")),
        PushTestOutcome::Queued => {
            st.0.push.wake();
            Ok(json_resp(
                StatusCode::OK,
                &json!({"ok": true, "courier": st.0.push.courier_connected(now)}),
                false,
            ))
        }
    }
}
