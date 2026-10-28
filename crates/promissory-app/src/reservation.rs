//! Proof of reservation over the payer API (docs/api.md, "Proof of reservation").
//!
//! - `GET /orders/{id}/reservation`: the payer's opt-in export for one order.
//! - `POST /reservations/verify`: the payee-side check, against a view-only wallet in
//!   `<dir>/payee-views/`, never the payer's wallet database.
//! - `POST /reservations/keeper-status`: keeper state for a list of blob IDs, no keys needed.
//!   The keeper sends no CORS headers, so the web UI asks promissory-app to fetch it.

use std::path::PathBuf;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use promissory_core::{
    net,
    notices::valid_hook,
    reservation::{self, BlobView, Proof, Report},
};
use tokio::sync::Mutex;

use crate::{ApiError, ApiResult, Shared};

/// One verification at a time: they share the payee-view wallet database.
static VERIFY: Mutex<()> = Mutex::const_new(());

#[derive(Serialize)]
pub struct Exported {
    order_id: i64,
    /// `promissory-reserve:v1:<base64url(json)>`
    proof: String,
}

pub async fn export(State(e): State<Shared>, Path(id): Path<i64>) -> ApiResult<Exported> {
    let e = e.lock().await;
    if e.store.order(id).is_err() {
        return Err(ApiError(StatusCode::NOT_FOUND, format!("no order {id}")));
    }
    let proof = reservation::export(&e.params, &e.db, &e.store, id)?;
    Ok(Json(Exported {
        order_id: id,
        proof: proof.encode(),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyBody {
    proof: String,
}

pub async fn verify(State(e): State<Shared>, Json(b): Json<VerifyBody>) -> ApiResult<Report> {
    let proof = Proof::decode(&b.proof)?;
    // Copy what we need and release the engine: a first scan can take a while.
    let (params, lwd_url, view_dir): (_, _, PathBuf) = {
        let e = e.lock().await;
        (e.params, e.lwd_url.clone(), e.paths.dir.join("payee-views"))
    };
    let _guard = VERIFY.lock().await;
    let mut lwd = net::connect(&lwd_url).await?;
    Ok(Json(reservation::verify(params, &view_dir, &mut lwd, &proof).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeeperStatusBody {
    keeper: String,
    blob_ids: Vec<String>,
}

pub async fn keeper_status(Json(b): Json<KeeperStatusBody>) -> ApiResult<Vec<BlobView>> {
    if !valid_hook(&b.keeper) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "keeper must be an http(s) URL".into()));
    }
    if b.blob_ids.is_empty() || b.blob_ids.len() > promissory_core::link::MAX_INSTALLMENTS as usize {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            format!("give between 1 and {} blob IDs", promissory_core::link::MAX_INSTALLMENTS),
        ));
    }
    let mut blobs = vec![];
    for id in b.blob_ids {
        let id = id.trim().to_string();
        if id.is_empty() || id.len() > 128 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(ApiError(StatusCode::BAD_REQUEST, format!("not a blob ID: {id}")));
        }
        blobs.push((None, None, id));
    }
    Ok(Json(reservation::keeper_status(&b.keeper, &blobs).await))
}
