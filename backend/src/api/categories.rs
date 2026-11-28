//! v3.1 bank categories: `POST /api/banks/categories` and `POST /api/banks/category` (contracts B.10, administrators only; the
//! route table already refuses everyone else). Both answer with the body of `GET /api/banks` as the caller sees it.

use super::auth::{self, Authed};
use super::json::PyVal;
use super::response::{
    json_resp, MSG_BAD_REQUEST, MSG_BANK_NOT_FOUND, MSG_CATEGORY_NAME_BAD, MSG_CATEGORY_NOT_FOUND,
    MSG_CATEGORY_ORDER, MSG_CATEGORY_TAKEN,
};
use super::{blocking, AppState};
use crate::database::store::{CategoryError, CategoryOp};
use crate::error::AppError;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::json;

/// The most banks one move may name.
const MAX_MOVE_IDS: usize = 1000;

fn bad_request() -> AppError {
    AppError::BadRequest(MSG_BAD_REQUEST.to_string())
}

/// An integer id in a body (a bool or a float is not one). An integer outside the id range cannot exist: `None`.
fn int_id(v: Option<&PyVal>) -> Result<Option<i64>, AppError> {
    match v {
        Some(PyVal::Int(i)) => Ok(i64::try_from(*i).ok()),
        _ => Err(bad_request()),
    }
}

/// A list of ids. An id outside the range becomes -1 (no row has it), so the set checks refuse it.
fn id_list(v: Option<&PyVal>, max: usize) -> Result<Vec<i64>, AppError> {
    let Some(PyVal::List(items)) = v else {
        return Err(bad_request());
    };
    if items.len() > max {
        return Err(bad_request());
    }
    items
        .iter()
        .map(|item| match item {
            PyVal::Int(i) => Ok(i64::try_from(*i).unwrap_or(-1)),
            _ => Err(bad_request()),
        })
        .collect()
}

fn name_of(data: &PyVal) -> Result<String, AppError> {
    match data.get("name") {
        Some(PyVal::Str(s)) => Ok(s.clone()),
        _ => Err(bad_request()),
    }
}

/// `categoryId` of a body: `null` = uncategorised, an integer = that category (out of range = one no row has).
fn category_of(data: &PyVal) -> Result<Option<i64>, AppError> {
    match data.get("categoryId") {
        Some(PyVal::Null) => Ok(None),
        other => Ok(Some(int_id(other)?.unwrap_or(-1))),
    }
}

fn refusal(e: CategoryError) -> AppError {
    let coded = |status, msg, code| AppError::Coded { status, msg, code };
    match e {
        CategoryError::NameBad => coded(400, MSG_CATEGORY_NAME_BAD, "banks.categoryName"),
        CategoryError::Taken => coded(409, MSG_CATEGORY_TAKEN, "banks.categoryTaken"),
        CategoryError::NotFound => coded(404, MSG_CATEGORY_NOT_FOUND, "banks.categoryNotFound"),
        CategoryError::Order => coded(409, MSG_CATEGORY_ORDER, "banks.categoryOrder"),
        CategoryError::BankNotFound => AppError::NotFoundMsg(MSG_BANK_NOT_FOUND),
        CategoryError::BadRequest => bad_request(),
        CategoryError::Db(e) => AppError::Db(e),
    }
}

/// The fresh `GET /api/banks` body for the caller (no ETag).
async fn banks_answer(st: &AppState, a: &Authed) -> Result<serde_json::Value, AppError> {
    let v = auth::viewer(st, a).await?;
    let store = st.0.store.clone();
    blocking(move || store.banks_as(&v)).await
}

/// `POST /api/banks/categories` `{"op": "create"|"rename"|"delete"|"order"|"bankOrder", ...}` (one operation per call) → the
/// body of `GET /api/banks`. Errors: 400 bad body / `banks.categoryName`, 404 `banks.categoryNotFound`, 409
/// `banks.categoryTaken` / `banks.categoryOrder`.
pub(super) async fn edit(st: &AppState, a: &Authed, data: &PyVal) -> Result<Response, AppError> {
    let Some(PyVal::Str(op_name)) = data.get("op") else {
        return Err(bad_request());
    };
    let op_name = op_name.clone();
    let op = match op_name.as_str() {
        "create" => CategoryOp::Create {
            name: name_of(data)?,
        },
        "rename" => {
            let name = name_of(data)?;
            // An id no row can have is refused like a missing category
            let id = int_id(data.get("id"))?.unwrap_or(-1);
            CategoryOp::Rename { id, name }
        }
        "delete" => CategoryOp::Delete {
            id: int_id(data.get("id"))?.unwrap_or(-1),
        },
        "order" => CategoryOp::Order {
            ids: id_list(data.get("ids"), usize::MAX)?,
        },
        "bankOrder" => CategoryOp::BankOrder {
            category_id: category_of(data)?,
            ids: id_list(data.get("ids"), usize::MAX)?,
        },
        _ => return Err(bad_request()),
    };
    let store = st.0.store.clone();
    let done = blocking(move || Ok(store.category_apply(&op))).await?;
    let name = done.map_err(refusal)?;
    let body = banks_answer(st, a).await?;
    Ok(crate::activity::with_note(
        json_resp(StatusCode::OK, &body, false),
        None,
        Some(json!({"op": op_name, "name": name})),
    ))
}

/// `POST /api/banks/category` `{"ids": [1..=1000 distinct bank ids], "categoryId": int|null}` → the body of `GET /api/banks`.
/// 404 `MSG_BANK_NOT_FOUND` when a bank does not exist, 404 `banks.categoryNotFound`.
pub(super) async fn move_banks(
    st: &AppState,
    a: &Authed,
    data: &PyVal,
) -> Result<Response, AppError> {
    let ids = id_list(data.get("ids"), MAX_MOVE_IDS)?;
    let category_id = category_of(data)?;
    let count = ids.len();
    let store = st.0.store.clone();
    let done = blocking(move || Ok(store.banks_move_to_category(&ids, category_id))).await?;
    done.map_err(refusal)?;
    let body = banks_answer(st, a).await?;
    Ok(crate::activity::with_note(
        json_resp(StatusCode::OK, &body, false),
        None,
        Some(json!({"count": count, "categoryId": category_id})),
    ))
}
