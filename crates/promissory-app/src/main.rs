//! `promissory-app`: the payer-side Promissory engine as a local HTTP API (docs/api.md §3), plus
//! the web UI. It holds the keys for reserved funds, so it binds to localhost only.

mod allowance;
mod batch;
mod engine;
mod notices;
mod rates;
mod receipt;
mod reservation;
mod switch;

use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::Result;
use axum::{
    extract::{Path, Request, State},
    http::{Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use serde::Deserialize;
use promissory_core::network::SoNetwork;
use tokio::sync::Mutex;
use tower_http::services::{ServeDir, ServeFile};

use engine::{Engine, NewOrder};

#[derive(Parser)]
#[command(name = "promissory-app", about = "Promissory payer app (local API + web UI)")]
struct Args {
    /// `main` or `regtest`
    #[arg(long, default_value = "main")]
    network: String,
    /// NU7 activation height of the local chain (regtest only)
    #[arg(long, default_value_t = SoNetwork::REGTEST_NU7)]
    nu7_height: u32,
    /// Wallet directory created by `promissory init` [default: promissory-data / promissory-data-regtest]
    #[arg(long)]
    dir: Option<PathBuf>,
    /// lightwalletd endpoint [default: zec.rocks on main, 127.0.0.1:9067 on regtest]
    #[arg(long)]
    lwd: Option<String>,
    /// Keeper used for new orders unless a link or request names another
    #[arg(long, default_value = "http://127.0.0.1:8737")]
    keeper: String,
    #[arg(long, default_value = "127.0.0.1:8738")]
    listen: SocketAddr,
    /// Built web UI to serve at `/`
    #[arg(long, default_value = "apps/web/dist")]
    web: PathBuf,
    /// Seconds between automatic progression passes
    #[arg(long, default_value_t = 15)]
    tick_secs: u64,
    /// Reach lightwalletd, keepers and drand only through Tor (never falls back to clearnet).
    /// Needs a build with `--features tor`.
    #[arg(long)]
    tor: bool,
    /// Tor (arti) state directory [default: <dir>/tor]
    #[arg(long)]
    tor_dir: Option<PathBuf>,
}

type Shared = Arc<Mutex<Engine>>;

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::BAD_REQUEST, engine::message(&e))
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

/// A cross-site page cannot add this header without a CORS preflight, which we never answer.
async fn csrf_guard(req: Request, next: Next) -> Response {
    if req.method() != Method::GET
        && req.method() != Method::HEAD
        && req.headers().get("x-promissory-app").and_then(|v| v.to_str().ok()) != Some("1")
    {
        return ApiError(StatusCode::FORBIDDEN, "missing X-Promissory-App header".into()).into_response();
    }
    next.run(req).await
}

/// Runs every write to completion even if the client goes away. axum drops a handler's future
/// when the connection closes, so a cancel cut off there could be lost while the page said
/// "Sending sweep…", or broadcast its sweep without recording it.
async fn detach_writes(req: Request, next: Next) -> Response {
    if req.method() == Method::GET || req.method() == Method::HEAD {
        return next.run(req).await;
    }
    match tokio::spawn(next.run(req)).await {
        Ok(r) => r,
        Err(e) => ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("request failed: {e}")).into_response(),
    }
}

async fn info(State(e): State<Shared>) -> ApiResult<engine::Info> {
    Ok(Json(e.lock().await.info().await?))
}

async fn sync(State(e): State<Shared>) -> ApiResult<engine::Info> {
    let mut e = e.lock().await;
    e.sync().await?;
    Ok(Json(e.info().await?))
}

async fn orders(State(e): State<Shared>) -> ApiResult<Vec<engine::OrderView>> {
    Ok(Json(e.lock().await.orders()?))
}

async fn order(State(e): State<Shared>, Path(id): Path<i64>) -> ApiResult<engine::OrderView> {
    let mut e = e.lock().await;
    if e.store.order(id).is_err() {
        return Err(ApiError(StatusCode::NOT_FOUND, format!("no order {id}")));
    }
    Ok(Json(e.order(id).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBody {
    payee: String,
    amount_zat: u64,
    every_secs: u64,
    count: u32,
    memo: Option<String>,
    first_due: Option<u64>,
    jitter_max_secs: Option<u64>,
    keeper: Option<String>,
    /// Keep only this many installments funded ahead (rolling window); 0 or absent = all.
    window: Option<u32>,
}

async fn create(State(e): State<Shared>, Json(b): Json<CreateBody>) -> ApiResult<engine::OrderView> {
    let window = b.window.unwrap_or(0);
    let req = NewOrder {
        payee: b.payee,
        amount_zat: b.amount_zat,
        every_secs: b.every_secs,
        count: b.count,
        memo: b.memo,
        first_due: b.first_due,
        jitter_max_secs: b.jitter_max_secs,
        keeper: b.keeper,
    };
    Ok(Json(e.lock().await.create_windowed(req, window).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FromLinkBody {
    link: String,
    first_due: Option<u64>,
    keeper: Option<String>,
    /// Attach the link's payee webhook to uploads (off by default; see docs/api.md).
    #[serde(default)]
    accept_hook: bool,
    window: Option<u32>,
}

async fn create_from_link(State(e): State<Shared>, Json(b): Json<FromLinkBody>) -> ApiResult<engine::OrderView> {
    let window = b.window.unwrap_or(0);
    Ok(Json(
        e.lock()
            .await
            .create_from_link(&b.link, b.first_due, b.keeper, b.accept_hook, window)
            .await?,
    ))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct CancelBody {
    margin_secs: Option<u64>,
    #[serde(default)]
    keep_blobs: bool,
    to: Option<String>,
}

async fn cancel(
    State(e): State<Shared>,
    Path(id): Path<i64>,
    body: Option<Json<CancelBody>>,
) -> ApiResult<engine::CancelView> {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let mut e = e.lock().await;
    if e.store.order(id).is_err() {
        return Err(ApiError(StatusCode::NOT_FOUND, format!("no order {id}")));
    }
    Ok(Json(e.cancel(id, b.margin_secs, b.keep_blobs, b.to).await?))
}

async fn not_found() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "no such endpoint".into())
}

fn router(engine: Shared, web: PathBuf) -> Router {
    let api = Router::new()
        .route("/info", get(info))
        .route("/sync", post(sync))
        .route("/orders", get(orders).post(create))
        .route("/orders/from-link", post(create_from_link))
        .route("/orders/{id}", get(order))
        .route("/orders/{id}/cancel", post(cancel))
        .route("/batches", get(batch::list).post(batch::create))
        .route("/batches/{id}", get(batch::get))
        .route("/batches/{id}/cancel", post(batch::cancel))
        .route("/rates", get(rates::rates))
        .route("/orders/{id}/reservation", get(reservation::export))
        .route("/reservations/verify", post(reservation::verify))
        .route("/reservations/keeper-status", post(reservation::keeper_status))
        .route("/notices", get(notices::list))
        .route("/notices/{id}/dismiss", post(notices::dismiss))
        .route("/switches", post(switch::create))
        .route("/switches/{id}/renew", post(switch::renew))
        .route("/orders/{id}/installments/{k}/receipt", get(receipt::make))
        .route("/receipts/verify", post(receipt::verify))
        .route("/allowances", get(allowance::list).post(allowance::create))
        .route("/allowances/{id}", get(allowance::get))
        .route("/allowances/{id}/stop", post(allowance::stop))
        .route("/requests", get(allowance::requests::list).post(allowance::requests::create))
        .route("/requests/{id}", get(allowance::requests::get))
        .route("/requests/{id}/approve", post(allowance::requests::approve))
        .route("/requests/{id}/reject", post(allowance::requests::reject))
        .fallback(not_found)
        .layer(middleware::from_fn(detach_writes))
        .layer(middleware::from_fn(csrf_guard))
        .with_state(engine);
    // Client-side routes (/new, /orders/1, /s, …) fall back to the app shell; a missing build
    // asset is a plain 404.
    let ui = ServeDir::new(&web).fallback(ServeFile::new(web.join("index.html")));
    Router::new()
        .nest("/api/v1", api)
        .nest_service("/assets", ServeDir::new(web.join("assets")))
        .fallback_service(ui)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "promissory_app=info".into()),
        )
        .init();
    let args = Args::parse();
    let (params, default_lwd, default_dir) = match args.network.as_str() {
        "main" => (SoNetwork::Main, "https://zec.rocks:443", "promissory-data"),
        "regtest" => (
            SoNetwork::regtest_with_nu7(args.nu7_height),
            "http://127.0.0.1:9067",
            "promissory-data-regtest",
        ),
        other => anyhow::bail!("unknown network {other}"),
    };
    let dir = args.dir.unwrap_or_else(|| PathBuf::from(default_dir));
    if args.tor {
        enable_tor(&args.tor_dir.clone().unwrap_or_else(|| dir.join("tor"))).await?;
    }
    let lwd = args.lwd.unwrap_or_else(|| default_lwd.to_string());
    let engine = Arc::new(Mutex::new(Engine::open(params, &dir, lwd, args.keeper)?));

    let ticker = engine.clone();
    tokio::spawn(async move {
        loop {
            // Lock per step, not per pass, so API requests interleave with slow network calls.
            // Bind the result first: a guard in a `match` scrutinee lives to the end of the match,
            // and the per-order locks below would wait on it forever.
            let begun = ticker.lock().await.begin_pass().await;
            match begun {
                Ok(ids) => {
                    for id in ids {
                        let _ = ticker.lock().await.step_order(id).await;
                    }
                }
                Err(e) => tracing::warn!("progression pass failed: {e:#}"),
            }
            tokio::time::sleep(Duration::from_secs(args.tick_secs)).await;
        }
    });

    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!("promissory-app on http://{} ({}), wallet {}", args.listen, args.network, dir.display());
    axum::serve(listener, router(engine, args.web))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

/// Installs a process-wide Tor client before the engine opens: promissory-core's `net::connect`,
/// `Keeper::new` and `Drand` then go through Tor, so the engine needs no change.
#[cfg(feature = "tor")]
async fn enable_tor(dir: &std::path::Path) -> Result<()> {
    tracing::info!("bootstrapping Tor in {}", dir.display());
    promissory_core::tor::bootstrap_and_install(dir).await?;
    tracing::info!(secs = promissory_core::tor::bootstrap_time().map(|d| d.as_secs_f32()), "Tor ready");
    Ok(())
}

#[cfg(not(feature = "tor"))]
async fn enable_tor(_: &std::path::Path) -> Result<()> {
    anyhow::bail!("this build has no Tor support; rebuild with `--features tor`")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use axum::{body::Body, routing::post};
    use tower::ServiceExt;

    use super::*;

    /// A write whose client hangs up early must still finish: the cancel that was lost while the
    /// dialog said "Sending sweep…".
    #[tokio::test]
    async fn writes_survive_a_dropped_client() {
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let app = Router::new()
            .route(
                "/w",
                post(move || async move {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    flag.store(true, Ordering::SeqCst);
                }),
            )
            .layer(middleware::from_fn(detach_writes));
        let req = Request::post("/w").body(Body::empty()).unwrap();
        // The client gives up after 10 ms, dropping the request future.
        assert!(tokio::time::timeout(Duration::from_millis(10), app.oneshot(req)).await.is_err());
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(done.load(Ordering::SeqCst), "the write was dropped with its client");
    }
}
