//! Opt-in state-change notices. Best effort: one POST per transition, no retries.

use std::time::Duration;

use serde::Serialize;

pub const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Serialize)]
struct Notice<'a> {
    blob_id: &'a str,
    txid: Option<&'a str>,
    status: &'a str,
}

#[derive(Clone)]
pub struct Notifier {
    client: Client,
}

#[derive(Clone)]
enum Client {
    Direct(reqwest::Client),
    #[cfg(feature = "tor")]
    Tor(promissory_core::tor::Client),
}

impl Notifier {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            client: Client::Direct(
                reqwest::Client::builder()
                    .timeout(TIMEOUT)
                    .redirect(reqwest::redirect::Policy::none())
                    .build()?,
            ),
        })
    }

    /// Notices go through Tor (on their own isolated circuits), so receivers don't learn the
    /// keeper's address.
    #[cfg(feature = "tor")]
    pub fn tor(tor: &promissory_core::tor::Client) -> Self {
        Self {
            client: Client::Tor(tor.isolated_client()),
        }
    }

    /// Sends the notice in the background; failures are logged without the URL.
    pub fn notify(&self, url: &str, blob_id: &str, txid: Option<&str>, status: &str) {
        let body = match serde_json::to_vec(&Notice {
            blob_id,
            txid,
            status,
        }) {
            Ok(b) => b,
            Err(_) => return,
        };
        let blob_id = blob_id.to_string();
        match &self.client {
            Client::Direct(c) => {
                let req = c
                    .post(url)
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body);
                tokio::spawn(async move {
                    match req.send().await {
                        Ok(r) if r.status().is_success() => {}
                        Ok(r) => tracing::debug!(%blob_id, status = %r.status(), "webhook refused"),
                        Err(_) => tracing::debug!(%blob_id, "webhook failed"),
                    }
                });
            }
            #[cfg(feature = "tor")]
            Client::Tor(tor) => {
                let (tor, url) = (tor.clone(), url.to_string());
                tokio::spawn(async move {
                    let send = promissory_core::tor::http(&tor, &url, Some(body), None);
                    match tokio::time::timeout(TIMEOUT * 6, send).await {
                        Ok(Ok(r)) if (200..300).contains(&r.status) => {}
                        Ok(Ok(r)) => tracing::debug!(%blob_id, status = r.status, "webhook refused"),
                        _ => tracing::debug!(%blob_id, "webhook failed"),
                    }
                });
            }
        }
    }
}

/// Only plain http(s) URLs, of sane length.
pub fn valid_url(url: &str) -> bool {
    url.len() <= 2048
        && reqwest::Url::parse(url)
            .map(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
            .unwrap_or(false)
}
