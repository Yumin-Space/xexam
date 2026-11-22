//! v1.3 routes of the activity log: `GET /api/activity`, `GET` / `POST /api/activity/settings`, `POST /api/activity/visit` and
//! `POST /api/users/signin-info` and `POST /api/users/signin-info/clear`. The entries themselves are written by [`crate::activity`]; this file only reads them and
//! changes the three switches (retention days, visits of plain users, sign-in IP / location).
use super::auth::Authed;
use super::json::PyVal;
use super::response::json_resp;
use super::{bad_request, blocking, query_once};
use crate::activity::{with_note, Actor, Entry};
use crate::database::{ActivityFilter, UserFilter, MAX_RETENTION_DAYS, MIN_RETENTION_DAYS};
use crate::error::AppError;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{json, Value};

/// The pages `POST /api/activity/visit` accepts (the route names of the frontend; no ids, no bank names).
pub const PAGES: &[&str] = &[
    "home",
    "study",
    "banks",
    "users",
    "logs",
    "records",
    "record",
    "wrong",
    "settings",
    "profile",
    "about",
    "quiz",
    "result",
    "ai",
    "aiSettings",
];

const GROUPS: &[&str] = &[
    "auth", "users", "banks", "study", "settings", "visit", "ai", "system",
];

/// A non-negative id in a query: 1 to 18 ASCII digits.
fn query_num(q: Option<&str>, key: &str) -> Result<Option<i64>, AppError> {
    match query_once(q, key)? {
        None => Ok(None),
        Some(s) if (1..=18).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) => {
            s.parse().map(Some).map_err(|_| bad_request())
        }
        Some(_) => Err(bad_request()),
    }
}

/// The filter of `GET /api/activity` from its query; any malformed value (or a repeated key) is a 400.
fn filter_of(q: Option<&str>) -> Result<ActivityFilter, AppError> {
    let limit = match query_once(q, "limit")? {
        None => None,
        Some(s) => match s.parse::<i64>() {
            Ok(n) if (1..=200).contains(&n) && s.bytes().all(|b| b.is_ascii_digit()) => Some(n),
            _ => return Err(bad_request()),
        },
    };
    let user = match query_once(q, "user")?.as_deref() {
        None => None,
        Some("system") => Some(UserFilter::System),
        Some("anon") => Some(UserFilter::Anon),
        Some(s) if (1..=18).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) => {
            Some(UserFilter::Id(s.parse().map_err(|_| bad_request())?))
        }
        Some(_) => return Err(bad_request()),
    };
    let group = match query_once(q, "group")? {
        None => None,
        Some(g) if GROUPS.contains(&g.as_str()) => Some(g),
        Some(_) => return Err(bad_request()),
    };
    let action = match query_once(q, "action")? {
        None => None,
        Some(a)
            if (1..=80).contains(&a.len())
                && a.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.') =>
        {
            Some(a)
        }
        Some(_) => return Err(bad_request()),
    };
    let ok = match query_once(q, "result")?.as_deref() {
        None => None,
        Some("ok") => Some(true),
        Some("fail") => Some(false),
        Some(_) => return Err(bad_request()),
    };
    let facets = match query_once(q, "facets")?.as_deref() {
        None | Some("0") => false,
        Some("1") => true,
        Some(_) => return Err(bad_request()),
    };
    Ok(ActivityFilter {
        before: query_num(q, "before")?,
        after: query_num(q, "after")?,
        limit,
        user,
        group,
        action,
        ok,
        facets,
    })
}

/// `{"retentionDays", "userVisits", "signinInfo"}`
fn settings_json(st: &crate::database::Store) -> Result<Value, crate::error::DbError> {
    Ok(json!({
        "retentionDays": st.activity_retention_days()?,
        "userVisits": st.activity_user_visits()?,
        "signinInfo": st.signin_info_enabled()?,
    }))
}

/// `GET /api/activity` (administrators; `activity: false`, not logged).
pub(super) async fn list(st: &super::AppState, query: Option<&str>) -> Result<Value, AppError> {
    let f = filter_of(query)?;
    let store = st.0.store.clone();
    blocking(move || {
        let page = store.activity_page(&f)?;
        let mut out = page.to_json();
        if let Some(o) = out.as_object_mut() {
            o.insert("settings".to_string(), settings_json(&store)?);
        }
        Ok(out)
    })
    .await
}

/// `GET /api/activity/settings`
pub(super) async fn settings(st: &super::AppState) -> Result<Value, AppError> {
    let store = st.0.store.clone();
    blocking(move || settings_json(&store)).await
}

/// `POST /api/activity/settings` `{"retentionDays": 7..3650, "userVisits": bool}` (either key optional) → the settings
/// object. Entry `logs.settings` with the resulting `retentionDays` and `userVisits`.
pub(super) async fn update_settings(
    st: &super::AppState,
    a: &Authed,
    data: &PyVal,
) -> Result<Response, AppError> {
    let days = match data.get("retentionDays") {
        None => None,
        Some(PyVal::Int(i)) => match i64::try_from(*i) {
            Ok(n) if (MIN_RETENTION_DAYS..=MAX_RETENTION_DAYS).contains(&n) => Some(n),
            _ => return Err(bad_request()),
        },
        Some(_) => return Err(bad_request()),
    };
    let visits = match data.get("userVisits") {
        None => None,
        Some(PyVal::Bool(b)) => Some(*b),
        Some(_) => return Err(bad_request()),
    };
    let store = st.0.store.clone();
    let admin = a.user.id;
    let out = blocking(move || {
        if let Some(d) = days {
            store.set_system_setting(
                crate::database::ACTIVITY_RETENTION_DAYS,
                &d.to_string(),
                Some(admin),
            )?;
        }
        if let Some(v) = visits {
            store.set_system_setting(
                crate::database::ACTIVITY_USER_VISITS,
                if v { "1" } else { "0" },
                Some(admin),
            )?;
        }
        settings_json(&store)
    })
    .await?;
    st.0.activity.reload_settings(&st.0.store);
    let detail = json!({ "retentionDays": out["retentionDays"], "userVisits": out["userVisits"] });
    Ok(with_note(
        json_resp(StatusCode::OK, &out, false),
        None,
        Some(detail),
    ))
}

/// `POST /api/activity/visit` `{"page": "home"}` → `{"ok": true}`. An unknown page is a 400. An administrator's visit is
/// recorded; a plain user's only while `activity.user_visits` is on; otherwise nothing is written. At most 30 writes per user
/// per minute (the rest is ignored); the same user and page within 30 minutes merge into one row (`count + 1`).
pub(super) async fn visit(
    st: &super::AppState,
    a: &Authed,
    data: &PyVal,
) -> Result<Response, AppError> {
    let page = match data.get("page") {
        Some(PyVal::Str(p)) if PAGES.contains(&p.as_str()) => p.clone(),
        _ => return Err(bad_request()),
    };
    let admin = a.user.role == crate::auth::Role::Admin;
    let sink = &st.0.activity;
    if (admin || sink.user_visits()) && sink.visit_allowed(a.user.id, crate::auth::unix_now()) {
        sink.record(Entry::new(Actor::of(&a.user), "page.view").object(page));
    }
    Ok(json_resp(StatusCode::OK, &json!({ "ok": true }), false))
}

/// `POST /api/users/signin-info` `{"enabled": bool}` → the users answer (as `GET /api/users`). Closed: nothing new is
/// recorded and stored addresses / locations are hidden; open: they show again and recording resumes. Nothing stored is
/// changed here. Entry `users.signinInfo`.
pub(super) async fn signin_info(
    st: &super::AppState,
    a: &Authed,
    data: &PyVal,
) -> Result<Response, AppError> {
    let enabled = match data.get("enabled") {
        Some(PyVal::Bool(b)) => *b,
        _ => return Err(bad_request()),
    };
    let store = st.0.store.clone();
    let admin = a.user.id;
    blocking(move || store.set_signin_info(enabled, Some(admin))).await?;
    st.0.activity.reload_settings(&st.0.store);
    let users = super::auth::list(st).await?;
    Ok(with_note(
        json_resp(StatusCode::OK, &users, false),
        None,
        Some(json!({ "enabled": enabled })),
    ))
}

/// `POST /api/users/geo-source` `{"source": "cloudflare"}` → the users answer (as `GET /api/users`). Only the way back to the
/// Cloudflare request headers goes through here (it works without ai-service); switching to the external lookup is done by
/// ai-service after it checked that tested settings are saved (`POST /api/ai/admin/geo/enable`). Stored places stay as
/// they were. Entry `users.geoSource` with `{"source": "cloudflare"}`.
pub(super) async fn geo_source(
    st: &super::AppState,
    a: &Authed,
    data: &PyVal,
) -> Result<Response, AppError> {
    match data {
        PyVal::Dict(items)
            if items.len() == 1
                && matches!(data.get("source"), Some(PyVal::Str(s)) if s == "cloudflare") => {}
        _ => return Err(bad_request()),
    }
    let store = st.0.store.clone();
    let admin = a.user.id;
    blocking(move || store.set_geo_source(false, Some(admin))).await?;
    st.0.activity.reload_settings(&st.0.store);
    st.0.activity.geo().drop_pending();
    let users = super::auth::list(st).await?;
    Ok(with_note(
        json_resp(StatusCode::OK, &users, false),
        None,
        Some(json!({ "source": "cloudflare" })),
    ))
}

/// Rows of the activity log cleared per statement by `POST /api/users/signin-info/clear`.
const CLEAR_BATCH: i64 = 5000;

/// `POST /api/users/signin-info/clear` (no body fields) → the users answer (as `GET /api/users`). Clears every stored
/// sign-in address and location: `users.last_login_ip` / `_country` / `_region` / `_city` and `sessions.ip` in one
/// transaction, then `ip` / `country` / `region` / `city` of the activity log in batches (the lock is released between
/// them). Works whether the switch is on or off and cannot be undone; what is recorded afterwards stays. Entry
/// `users.signinInfoClear` with `{"rows": <rows changed>}`.
pub(super) async fn signin_info_clear(
    st: &super::AppState,
    _a: &Authed,
) -> Result<Response, AppError> {
    // The lookup cache, queue and jobs in flight go first: a result that arrives later is dropped (v1.4)
    st.0.activity.geo().clear();
    // Rows still in the queue are written first, so they are cleared too
    st.0.activity.flush().await;
    let store = st.0.store.clone();
    let rows = blocking(move || {
        let mut rows = store.clear_signin_info()?;
        loop {
            let n = store.clear_activity_ip(CLEAR_BATCH)?;
            if n == 0 {
                break;
            }
            rows = rows.saturating_add(n);
        }
        Ok(rows)
    })
    .await?;
    let users = super::auth::list(st).await?;
    Ok(with_note(
        json_resp(StatusCode::OK, &users, false),
        None,
        Some(json!({ "rows": rows })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_filters_parse_and_refuse() {
        let f = filter_of(Some(
            "before=9&after=2&limit=50&user=3&group=ai&action=ai.settings.key&result=fail&facets=1",
        ))
        .expect("ok");
        assert_eq!((f.before, f.after, f.limit), (Some(9), Some(2), Some(50)));
        assert_eq!(f.user, Some(UserFilter::Id(3)));
        assert_eq!(f.group.as_deref(), Some("ai"));
        assert_eq!(f.ok, Some(false));
        assert!(f.facets);
        let none = filter_of(None).expect("empty");
        assert!(none.limit.is_none() && none.user.is_none() && !none.facets);
        assert_eq!(
            filter_of(Some("user=system")).expect("s").user,
            Some(UserFilter::System)
        );
        assert_eq!(
            filter_of(Some("user=anon")).expect("a").user,
            Some(UserFilter::Anon)
        );
        for bad in [
            "limit=0",
            "limit=201",
            "limit=x",
            "limit=-5",
            "limit=1&limit=2",
            "user=bob",
            "user=-1",
            "group=nope",
            "action=a%20b",
            "action=",
            "result=maybe",
            "facets=2",
            "before=abc",
            "after=",
        ] {
            assert!(filter_of(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn pages_are_the_contract_list() {
        assert_eq!(PAGES.len(), 15);
        assert!(PAGES.contains(&"profile"));
        assert!(PAGES.contains(&"aiSettings") && PAGES.contains(&"quiz"));
        assert!(!PAGES.contains(&"login"));
    }
}
