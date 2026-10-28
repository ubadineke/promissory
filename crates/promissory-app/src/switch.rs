//! Dead-man's switch over the payer API (docs/api.md, "Inheritance switch").
//!
//! - `POST /switches`: one payment to an heir at a date, unless renewed first.
//! - `POST /switches/{id}/renew`: move the reservation to a new switch with a later date.
//!
//! Switches are orders with `kind: "inheritance"`; `GET /orders` lists them with the rest.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use promissory_core::order::KIND_INHERITANCE;

use crate::{
    engine::{NewSwitch, OrderView, RenewView},
    ApiError, ApiResult, Shared,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBody {
    heir: String,
    amount_zat: u64,
    /// Unix seconds when it pays.
    at: u64,
    memo: Option<String>,
    /// Remind this long before `at`; default 7 days.
    remind_secs: Option<u64>,
    keeper: Option<String>,
}

pub async fn create(State(e): State<Shared>, Json(b): Json<CreateBody>) -> ApiResult<OrderView> {
    let req = NewSwitch {
        heir: b.heir,
        amount_zat: b.amount_zat,
        at: b.at,
        memo: b.memo,
        remind_secs: b.remind_secs,
        keeper: b.keeper,
    };
    Ok(Json(e.lock().await.create_switch(req).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenewBody {
    /// New date, unix seconds; must be later than the current one.
    at: u64,
    margin_secs: Option<u64>,
    /// Leave the old blob at the keeper (the chain then rejects it as a double-spend).
    #[serde(default)]
    keep_blob: bool,
}

pub async fn renew(State(e): State<Shared>, Path(id): Path<i64>, Json(b): Json<RenewBody>) -> ApiResult<RenewView> {
    let mut e = e.lock().await;
    match e.store.meta(id) {
        Ok(m) if m.kind == KIND_INHERITANCE => {}
        _ => return Err(ApiError(StatusCode::NOT_FOUND, format!("no switch {id}"))),
    }
    Ok(Json(e.renew_switch(id, b.at, b.margin_secs, b.keep_blob).await?))
}
