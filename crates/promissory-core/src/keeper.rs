//! Client for a `promissory-keeper` service (contract in docs/plan.md).

use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct Uploaded {
    pub blob_id: String,
    pub delete_token: String,
    pub round: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BlobStatus {
    pub blob_id: String,
    pub round: u64,
    pub state: String,
    pub txid: Option<String>,
    pub mined_height: Option<u32>,
    pub last_error: Option<String>,
}

impl BlobStatus {
    /// The keeper has sent (or is about to send) the transaction: cancelling can no longer
    /// stop it.
    pub fn is_out_of_our_hands(&self) -> bool {
        matches!(self.state.as_str(), "released" | "broadcast" | "mined")
    }
}

/// The parts of `GET /v1/health` payers use.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Health {
    pub ok: bool,
    /// Whether the keeper accepts webhooks (`--allow-webhooks`).
    #[serde(default)]
    pub webhooks: bool,
    pub last_error: Option<String>,
}

pub struct Keeper {
    base: String,
    http: Http,
}

enum Http {
    Direct(reqwest::Client),
    #[cfg(feature = "tor")]
    Tor(crate::tor::Client),
}

/// A response read in full.
struct Reply {
    status: u16,
    body: Vec<u8>,
}

impl Reply {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).map_err(|e| anyhow!("keeper reply: {e}"))
    }
}

enum Method<'a> {
    Get,
    Post(Vec<u8>),
    /// `DELETE` directly; over Tor, whose HTTP client has only GET and POST, the keeper's
    /// `POST <path>/delete` alias.
    Delete(&'a str),
}

impl Keeper {
    /// A client for the keeper at `base`. Goes over Tor if a process-wide client is installed
    /// (feature `tor`, [`crate::tor::install`]); otherwise direct.
    pub fn new(base: &str) -> Self {
        #[cfg(feature = "tor")]
        if let Some(tor) = crate::tor::installed() {
            return Self::with_tor(base, tor);
        }
        Self::direct(base)
    }

    /// A direct (clearnet) client, ignoring any installed Tor client.
    pub fn direct(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            http: Http::Direct(
                reqwest::Client::builder()
                    .connect_timeout(std::time::Duration::from_secs(15))
                    .timeout(std::time::Duration::from_secs(60))
                    .build()
                    .expect("static reqwest config"),
            ),
        }
    }

    /// A client that reaches the keeper through Tor, on circuits isolated from every other
    /// user of `tor`, so two keepers can't link a payer by circuit. A clearnet keeper sees a
    /// Tor exit's address; `.onion` keepers are not supported by the Tor HTTP client.
    #[cfg(feature = "tor")]
    pub fn with_tor(base: &str, tor: &crate::tor::Client) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            http: Http::Tor(tor.isolated_client()),
        }
    }

    /// Whether requests go through Tor.
    pub fn is_tor(&self) -> bool {
        !matches!(self.http, Http::Direct(_))
    }

    async fn request(&self, path: &str, method: Method<'_>) -> Result<Reply> {
        let url = format!("{}{path}", self.base);
        match &self.http {
            Http::Direct(http) => {
                let req = match method {
                    Method::Get => http.get(&url),
                    Method::Post(json) => http
                        .post(&url)
                        .header(reqwest::header::CONTENT_TYPE, "application/json")
                        .body(json),
                    Method::Delete(token) => http.delete(&url).bearer_auth(token),
                };
                let resp = req.send().await?;
                let status = resp.status().as_u16();
                Ok(Reply {
                    status,
                    body: resp.bytes().await?.to_vec(),
                })
            }
            #[cfg(feature = "tor")]
            Http::Tor(tor) => {
                let call = async {
                    match method {
                        Method::Get => crate::tor::http(tor, &url, None, None).await,
                        Method::Post(json) => crate::tor::http(tor, &url, Some(json), None).await,
                        Method::Delete(token) => {
                            let url = format!("{url}/delete");
                            crate::tor::http(tor, &url, Some(b"{}".to_vec()), Some(token)).await
                        }
                    }
                };
                // Circuits are slower than clearnet, but a stuck one must not hang the caller.
                let r = tokio::time::timeout(std::time::Duration::from_secs(120), call)
                    .await
                    .map_err(|_| anyhow!("keeper over Tor did not answer within 120s"))??;
                Ok(Reply {
                    status: r.status,
                    body: r.body,
                })
            }
        }
    }

    pub async fn upload(&self, ciphertext: &[u8]) -> Result<Uploaded> {
        self.upload_with(ciphertext, None).await
    }

    /// Uploads with an optional webhook for notices. A keeper started without
    /// `--allow-webhooks` rejects a webhook (and the payee's endpoint is then never stored).
    pub async fn upload_with(&self, ciphertext: &[u8], webhook: Option<&str>) -> Result<Uploaded> {
        #[derive(Serialize)]
        struct Body<'a> {
            ciphertext: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            webhook: Option<&'a str>,
        }
        let body = serde_json::to_vec(&Body {
            ciphertext: B64.encode(ciphertext),
            webhook,
        })?;
        let r = self.request("/v1/blobs", Method::Post(body)).await?;
        if !r.ok() {
            bail!("keeper upload: {} {}", r.status, r.text());
        }
        r.json()
    }

    /// `GET /v1/health`.
    pub async fn health(&self) -> Result<Health> {
        let r = self.request("/v1/health", Method::Get).await?;
        if !r.ok() {
            bail!("keeper health: {}", r.status);
        }
        r.json()
    }

    pub async fn status(&self, blob_id: &str) -> Result<BlobStatus> {
        let r = self.request(&format!("/v1/blobs/{blob_id}"), Method::Get).await?;
        if !r.ok() {
            bail!("keeper status: {}", r.status);
        }
        r.json()
    }

    pub async fn delete(&self, blob_id: &str, delete_token: &str) -> Result<()> {
        let r = self
            .request(&format!("/v1/blobs/{blob_id}"), Method::Delete(delete_token))
            .await?;
        if r.ok() {
            return Ok(());
        }
        if r.status == 404 {
            // An unknown blob is gone already. A keeper without the POST alias answers its
            // router's generic 404 instead, which must not pass for a delete.
            let unknown_route = self.is_tor()
                && r.json::<serde_json::Value>()
                    .ok()
                    .and_then(|v| v["error"].as_str().map(|e| e != "no such blob"))
                    .unwrap_or(true);
            if !unknown_route {
                return Ok(());
            }
            bail!("keeper delete: this keeper has no POST delete route; upgrade it");
        }
        Err(anyhow!("keeper delete: {} {}", r.status, r.text()))
    }
}
