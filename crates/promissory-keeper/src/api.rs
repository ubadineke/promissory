//! HTTP API (see the keeper contract in docs/plan.md). Every error is JSON `{"error": "..."}`.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::{
    extract::{
        rejection::JsonRejection, ConnectInfo, DefaultBodyLimit, FromRequest, Path, Request, State,
    },
    http::{header::AUTHORIZATION, header::RETRY_AFTER, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::json;
use promissory_core::tlock;

use crate::db::{Quota, State as BlobState};
use crate::keeper::{Deleted, Keeper, MAX_PAST_ROUNDS};
use crate::limits::{Limits, RateLimiter};
use crate::{unix_now, webhook};

pub const BODY_LIMIT: usize = 64 * 1024;

/// The API with default [`Limits`].
pub fn router(keeper: Arc<Keeper>) -> Router {
    router_with(keeper, Limits::default())
}

/// The API with the given upload limits.
///
/// Per-client rate limits need the peer address: serve with
/// `into_make_service_with_connect_info::<SocketAddr>()`. Without it only the global bucket
/// applies.
pub fn router_with(keeper: Arc<Keeper>, limits: Limits) -> Router {
    let limiter = Arc::new(RateLimiter::new(limits.clone()));
    Router::new()
        .route("/v1/health", get(health))
        .route(
            "/v1/blobs",
            post(upload).route_layer(middleware::from_fn_with_state(limiter, rate_limit)),
        )
        .route("/v1/blobs/{id}", get(status).delete(delete))
        // For HTTP clients that only speak GET and POST (the Tor client among them).
        .route("/v1/blobs/{id}/delete", post(delete))
        .fallback(|| async { ApiError(StatusCode::NOT_FOUND, "not found".into()) })
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(Extension(Arc::new(limits)))
        .with_state(keeper)
}

/// Runs before the body is read. The address is used for the bucket key only, never logged.
async fn rate_limit(State(rl): State<Arc<RateLimiter>>, req: Request, next: Next) -> Response {
    let ip = if rl.keys_by_ip() {
        req.extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip())
    } else {
        None
    };
    match rl.check(ip, Instant::now(), unix_now()) {
        Ok(()) => next.run(req).await,
        Err(wait) => {
            let secs = wait.as_secs() + 1;
            let mut resp = ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                format!("too many uploads; retry in {secs}s"),
            )
            .into_response();
            resp.headers_mut().insert(RETRY_AFTER, secs.into());
            resp
        }
    }
}

pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

fn internal(e: anyhow::Error) -> ApiError {
    tracing::error!("api: {e:#}");
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
}

/// `Json` whose rejections are JSON too. Oversized bodies keep 413; malformed ones are 400.
pub struct ApiJson<T>(T);

impl<S: Send + Sync, T: serde::de::DeserializeOwned> FromRequest<S> for ApiJson<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(v)) => Ok(ApiJson(v)),
            Err(e) => {
                let status = match e.status() {
                    StatusCode::PAYLOAD_TOO_LARGE => StatusCode::PAYLOAD_TOO_LARGE,
                    StatusCode::UNSUPPORTED_MEDIA_TYPE => StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    _ => StatusCode::BAD_REQUEST,
                };
                let msg = match &e {
                    JsonRejection::BytesRejection(_) if status == StatusCode::PAYLOAD_TOO_LARGE => {
                        format!("body exceeds {BODY_LIMIT} bytes")
                    }
                    e => e.body_text(),
                };
                Err(ApiError(status, msg))
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadReq {
    ciphertext: String,
    webhook: Option<String>,
}

#[derive(Serialize)]
struct UploadResp {
    blob_id: String,
    delete_token: String,
    round: u64,
}

async fn upload(
    State(k): State<Arc<Keeper>>,
    Extension(limits): Extension<Arc<Limits>>,
    ApiJson(req): ApiJson<UploadReq>,
) -> Result<(StatusCode, Json<UploadResp>), ApiError> {
    if let Some(url) = &req.webhook {
        if !k.webhooks_enabled() {
            return Err(bad("webhooks are disabled on this keeper"));
        }
        if !webhook::valid_url(url) {
            return Err(bad("webhook must be an http(s) URL"));
        }
    }
    let ct = STANDARD
        .decode(req.ciphertext.trim())
        .map_err(|_| bad("ciphertext is not base64"))?;
    let (round, chain_hash) =
        tlock::locked_header(&ct).map_err(|_| bad("ciphertext is not tlock-age"))?;
    if chain_hash != hex::decode(tlock::QUICKNET_HASH).expect("constant hash") {
        return Err(bad("ciphertext is not locked to drand quicknet"));
    }
    let latest = k.latest_round().await.map_err(|e| {
        tracing::warn!("upload: drand: {e:#}");
        ApiError(StatusCode::SERVICE_UNAVAILABLE, "drand unavailable".into())
    })?;
    if round.saturating_add(MAX_PAST_ROUNDS) < latest {
        return Err(bad(format!(
            "round {round} is more than a day in the past (latest {latest})"
        )));
    }
    if round > limits.max_round(latest) {
        return Err(bad(format!(
            "round {round} is more than {} days ahead (latest {latest})",
            limits.max_horizon_days
        )));
    }
    let up = k
        .store_within(
            &ct,
            round,
            req.webhook.as_deref(),
            limits.max_stored_bytes,
            limits.max_locked_blobs,
        )
        .map_err(internal)?
        .map_err(|q| {
            let what = match q {
                Quota::StoredBytes => "stored ciphertext bytes",
                Quota::LockedBlobs => "locked blobs",
            };
            ApiError(
                StatusCode::INSUFFICIENT_STORAGE,
                format!("keeper is full ({what}); try later or another keeper"),
            )
        })?;
    Ok((
        StatusCode::CREATED,
        Json(UploadResp {
            blob_id: up.blob_id,
            delete_token: up.delete_token,
            round: up.round,
        }),
    ))
}

#[derive(Serialize)]
struct StatusResp {
    blob_id: String,
    round: u64,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    txid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mined_height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
    updated_at: u64,
}

async fn status(
    State(k): State<Arc<Keeper>>,
    Path(id): Path<String>,
) -> Result<Json<StatusResp>, ApiError> {
    let b = k
        .db
        .get(&id)
        .map_err(internal)?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such blob".into()))?;
    Ok(Json(StatusResp {
        blob_id: b.blob_id,
        round: b.round,
        state: b.state.as_str(),
        txid: b.txid,
        mined_height: b.mined_height,
        last_error: b.last_error,
        updated_at: b.updated_at,
    }))
}

async fn delete(
    State(k): State<Arc<Keeper>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "missing bearer token".into()))?;
    match k.delete(&id, token).await.map_err(internal)? {
        Deleted::Ok => Ok(StatusCode::NO_CONTENT),
        Deleted::NotFound => Err(ApiError(StatusCode::NOT_FOUND, "no such blob".into())),
        Deleted::BadToken => Err(ApiError(StatusCode::FORBIDDEN, "wrong delete token".into())),
        Deleted::TooLate(s) => Err(ApiError(
            StatusCode::CONFLICT,
            format!("blob is {}; it can no longer be deleted", s.as_str()),
        )),
    }
}

async fn health(
    State(k): State<Arc<Keeper>>,
    Extension(limits): Extension<Arc<Limits>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let s = k.status();
    let mut blobs: BTreeMap<&str, u64> = BlobState::ALL.iter().map(|s| (s.as_str(), 0)).collect();
    for (state, n) in k.db.counts().map_err(internal)? {
        blobs.insert(state.as_str(), n);
    }
    let usage = k.db.usage().map_err(internal)?;
    let (lwd_via, drand_via) = k.transports();
    let network = match k.network() {
        promissory_core::network::SoNetwork::Main => "main",
        promissory_core::network::SoNetwork::Regtest(_) => "regtest",
    };
    Ok(Json(json!({
        "ok": s.last_error.is_none() && s.tip_height.is_some() && s.latest_round.is_some(),
        "network": network,
        "tip_height": s.tip_height,
        "latest_round": s.latest_round,
        "last_tick_at": s.last_tick_at,
        "last_error": s.last_error,
        "webhooks": k.webhooks_enabled(),
        "lightwalletd_via": lwd_via,
        "drand_via": drand_via,
        "blobs": blobs,
        "storage": {
            "stored_bytes": usage.stored_bytes,
            "max_stored_bytes": limits.max_stored_bytes,
            "locked_blobs": usage.locked_blobs,
            "max_locked_blobs": limits.max_locked_blobs,
        },
        "max_horizon_days": limits.max_horizon_days,
        "rate_limit_key": limits.rate_key.as_str(),
    })))
}
