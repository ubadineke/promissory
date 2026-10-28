//! `GET /rates?vs=ngn`: a ZEC exchange rate for display only (the remittance view shows
//! "≈ ₦…" next to ZEC amounts). Amounts are always fixed in ZEC; nothing is ever converted
//! for signing. Fetched server-side so the app keeps one network path (which can move to Tor),
//! and cached for five minutes so a page polling it does not hit the rate source each time.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use axum::{extract::Query, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use promissory_core::order::now_unix;

use crate::ApiError;

pub const CACHE_SECS: u64 = 300;
pub const SOURCE: &str = "coingecko";
const URL: &str = "https://api.coingecko.com/api/v3/simple/price?ids=zcash&vs_currencies=";

#[derive(Clone, Serialize)]
pub struct Rate {
    /// Lower-case currency code, e.g. `ngn`.
    pub vs: String,
    /// Price of 1 ZEC in `vs`. Also sent as `zec_<vs>`, e.g. `zec_ngn`.
    pub rate: f64,
    pub fetched_at: u64,
    pub source: &'static str,
}

#[derive(Deserialize)]
pub struct RatesQuery {
    vs: Option<String>,
}

fn cache() -> &'static Mutex<HashMap<String, Rate>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Rate>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// One GET: through Tor when `--tor` installed a client (on its own circuit), else direct.
async fn get(url: &str) -> anyhow::Result<(u16, Vec<u8>)> {
    #[cfg(feature = "tor")]
    if let Some(tor) = promissory_core::tor::installed() {
        let r = promissory_core::tor::http(&tor.isolated_client(), url, None, None).await?;
        return Ok((r.status, r.body));
    }
    let r = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("promissory")
        .build()?
        .get(url)
        .send()
        .await?;
    Ok((r.status().as_u16(), r.bytes().await?.to_vec()))
}

async fn fetch(vs: &str) -> anyhow::Result<f64> {
    let mut last = anyhow::anyhow!("no attempt");
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(500 * attempt)).await;
        }
        match get(&format!("{URL}{vs}")).await {
            Ok((200..=299, body)) => {
                let body: serde_json::Value = serde_json::from_slice(&body)?;
                return body["zcash"][vs]
                    .as_f64()
                    .filter(|r| r.is_finite() && *r > 0.0)
                    .ok_or_else(|| anyhow::anyhow!("rate source has no ZEC/{} price", vs.to_uppercase()));
            }
            Ok((status, _)) => last = anyhow::anyhow!("rate source answered {status}"),
            Err(e) => last = anyhow::anyhow!("rate source unreachable: {e}"),
        }
    }
    Err(last)
}

/// A fresh or cached rate. Returns `{zec_<vs>, vs, rate, fetched_at, source}`, or 502 with
/// `{error}` when the source cannot be reached and nothing fresh is cached.
pub async fn rates(Query(q): Query<RatesQuery>) -> Result<Json<serde_json::Value>, ApiError> {
    let vs = q.vs.unwrap_or_else(|| "ngn".into()).to_ascii_lowercase();
    if !(3..=5).contains(&vs.len()) || !vs.bytes().all(|b| b.is_ascii_lowercase()) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "vs must be a currency code like ngn".into()));
    }
    let now = now_unix();
    let cached = cache().lock().expect("rate cache").get(&vs).cloned();
    let rate = match cached {
        Some(r) if now.saturating_sub(r.fetched_at) < CACHE_SECS => r,
        _ => {
            let rate = fetch(&vs)
                .await
                .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
            let r = Rate {
                vs: vs.clone(),
                rate,
                fetched_at: now,
                source: SOURCE,
            };
            cache().lock().expect("rate cache").insert(vs.clone(), r.clone());
            r
        }
    };
    let mut body = serde_json::to_value(&rate).expect("rate serializes");
    body[format!("zec_{vs}")] = serde_json::json!(rate.rate);
    Ok(Json(body))
}
