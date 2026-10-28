//! Payment receipts over the payer API (docs/api.md, "Receipts").
//!
//! - `GET /orders/{id}/installments/{k}/receipt`: the payer's receipt for one paid installment.
//! - `POST /receipts/verify`: anyone's check of a receipt against the chain. Uses no keys.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use promissory_core::{
    net,
    receipt::{self, Receipt, Verified},
};

use crate::{ApiError, ApiResult, Shared};

#[derive(Serialize)]
pub struct Made {
    order_id: i64,
    k: u32,
    txid: String,
    action_index: u32,
    /// `promissory-receipt:v1:<base64url(json)>`
    receipt: String,
}

pub async fn make(State(e): State<Shared>, Path((id, k)): Path<(i64, u32)>) -> ApiResult<Made> {
    // Read what the receipt needs, then release the engine for the network calls.
    let (pending, params, lwd) = {
        let e = e.lock().await;
        if e.store.order(id).is_err() {
            return Err(ApiError(StatusCode::NOT_FOUND, format!("no order {id}")));
        }
        (receipt::prepare(&e.paths, &e.db, &e.store, id, k)?, e.params, e.lwd())
    };
    let mut lwd = lwd.await?;
    let r = pending.finish(&params, &mut lwd).await?;
    Ok(Json(Made {
        order_id: id,
        k,
        txid: r.txid.clone(),
        action_index: r.action_index,
        receipt: r.encode(),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyBody {
    receipt: String,
}

pub async fn verify(State(e): State<Shared>, Json(b): Json<VerifyBody>) -> ApiResult<Verified> {
    let r = Receipt::decode(&b.receipt)?;
    let (params, lwd_url) = {
        let e = e.lock().await;
        (e.params, e.lwd_url.clone())
    };
    let mut lwd = net::connect(&lwd_url).await?;
    Ok(Json(receipt::verify(&mut lwd, &params, &r).await?))
}
