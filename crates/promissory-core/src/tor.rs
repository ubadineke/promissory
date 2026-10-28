//! Tor transport (feature `tor`): an arti client from `zcash_client_backend::tor`.
//!
//! Two ways to use it:
//!
//! - **Explicitly:** [`bootstrap`] a [`Client`], then pass it to [`crate::net::connect_tor`],
//!   [`crate::keeper::Keeper::with_tor`] or [`crate::tlock::TorDrand::new`].
//! - **Process-wide:** [`install`] a client once at startup. From then on
//!   [`crate::net::connect`], [`crate::keeper::Keeper::new`] and [`crate::tlock::Drand`] go
//!   over Tor, so code that only calls those (the order engine) needs no change. There is no
//!   fallback: if Tor fails, the call fails.
//!
//! Host names are sent to the Tor exit, which resolves them; nothing is resolved locally.
//! Connections to private or loopback addresses are refused by arti, so a regtest node on
//! `127.0.0.1` cannot be reached with Tor on.

use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};

pub use zcash_client_backend::tor::Client;

/// How long [`bootstrap`] waits for arti before giving up.
pub const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(180);

static INSTALLED: OnceLock<Client> = OnceLock::new();

/// Creates `dir` (mode 0700) if needed and bootstraps an arti client whose state and cache
/// live in it. Keeping the directory makes later bootstraps much faster.
pub async fn bootstrap(dir: &Path) -> Result<Client> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating Tor dir {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("restricting {}", dir.display()))?;
    }
    let started = Instant::now();
    let client = tokio::time::timeout(BOOTSTRAP_TIMEOUT, Client::create(dir, |_| {}))
        .await
        .map_err(|_| anyhow!("Tor bootstrap timed out after {}s", BOOTSTRAP_TIMEOUT.as_secs()))?
        .map_err(|e| anyhow!("Tor bootstrap failed: {e}"))?;
    let _ = FIRST_BOOTSTRAP.set(started.elapsed());
    Ok(client)
}

static FIRST_BOOTSTRAP: OnceLock<Duration> = OnceLock::new();

/// How long the first [`bootstrap`] in this process took.
pub fn bootstrap_time() -> Option<Duration> {
    FIRST_BOOTSTRAP.get().copied()
}

/// Routes all of promissory-core's network traffic in this process over `client`. Can be called once.
pub fn install(client: Client) -> Result<()> {
    if INSTALLED.set(client).is_err() {
        bail!("a Tor client is already installed");
    }
    Ok(())
}

/// The process-wide client, if [`install`] was called.
pub fn installed() -> Option<&'static Client> {
    INSTALLED.get()
}

/// Bootstraps a client in `dir` and [`install`]s it.
pub async fn bootstrap_and_install(dir: &Path) -> Result<()> {
    install(bootstrap(dir).await?)
}

/// Runs a future to completion from synchronous code (the blocking drand client). Works on
/// a multi-thread runtime's worker and on its blocking pool.
pub(crate) fn block_on<F: std::future::Future>(f: F) -> Result<F::Output> {
    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| anyhow!("Tor needs a tokio runtime"))?;
    Ok(tokio::task::block_in_place(|| handle.block_on(f)))
}

/// One HTTP response, read in full.
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

/// GET or POST (with a JSON body) over `client`. The tor HTTP client supports only these two
/// methods. No retries: callers decide, since a retried POST may be applied twice.
pub async fn http(
    client: &Client,
    url: &str,
    json_body: Option<Vec<u8>>,
    bearer: Option<&str>,
) -> Result<Reply> {
    use http_body_util::BodyExt;
    use zcash_client_backend::tor::http::HttpError;

    let uri: tonic::transport::Uri = url.parse().with_context(|| format!("bad url {url}"))?;
    let auth = bearer.map(|t| format!("Bearer {t}"));
    let headers = |b: hyper::http::request::Builder| {
        let b = b.header("accept", "application/json");
        match &auth {
            Some(a) => b.header("authorization", a.as_str()),
            None => b,
        }
    };
    let read = |body: hyper::body::Incoming| async move {
        Ok(body.collect().await.map_err(HttpError::from)?.to_bytes().to_vec())
    };
    let resp = match json_body {
        None => client.http_get(uri, headers, read, 0, |_| None).await,
        Some(json) => {
            let body = http_body_util::Full::new(bytes::Bytes::from(json));
            client
                .http_post(
                    uri,
                    |b| headers(b).header("content-type", "application/json"),
                    body,
                    read,
                    0,
                    |_| None,
                )
                .await
        }
    }
    .map_err(|e| anyhow!("over Tor: {e}"))?;
    Ok(Reply {
        status: resp.status().as_u16(),
        body: resp.into_body(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live: bootstraps arti and POSTs JSON through a Tor exit, as keeper uploads do.
    /// `cargo test -p promissory-core --features tor -- --ignored tor_post`
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the Tor network"]
    async fn tor_post_round_trip() {
        let dir = std::env::temp_dir().join("promissory-core-tor-test");
        let tor = bootstrap(&dir).await.unwrap();
        let r = http(&tor, "https://httpbin.org/post", Some(br#"{"so":1}"#.to_vec()), Some("t0k"))
            .await
            .unwrap();
        assert_eq!(r.status, 200);
        let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v["json"]["so"], 1);
        assert_eq!(v["headers"]["Authorization"], "Bearer t0k");
    }
}
