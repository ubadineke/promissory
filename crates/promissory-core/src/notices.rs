//! Notices: payee webhooks at the keeper, and the keeper reachability check behind the payer's
//! "keeper unreachable" banner.
//!
//! # Webhooks are off by default
//!
//! A Subscribe link may carry a payee-supplied `hook` URL. If the payer attaches it to an
//! upload, the keeper POSTs `{blob_id, txid, status}` to it on every state change. That tells the
//! keeper the payee's endpoint, and links each blob to that payee, which the keeper otherwise
//! never learns. So keepers accept webhooks only when started with `--allow-webhooks`, and the
//! payer app attaches one only when the payer opts in.
//!
//! [`upload_with_webhook`] checks the keeper accepts webhooks, then uploads with one.
//! [`crate::order::arm`] uses it for orders whose payer opted in to the link's `hook`.

use std::time::Duration;

use anyhow::{anyhow, bail, Result};

use crate::keeper::{Keeper, Uploaded};

pub use crate::keeper::Health;

/// `GET /v1/health` with a short timeout, over Tor when a client is installed. `Err` means the
/// keeper is unreachable.
pub async fn health(keeper: &str, timeout: Duration) -> Result<Health> {
    tokio::time::timeout(timeout, Keeper::new(keeper).health())
        .await
        .map_err(|_| anyhow!("keeper health: timed out"))?
}

/// Only plain http(s) URLs of sane length, as the keeper itself requires.
pub fn valid_hook(url: &str) -> bool {
    url.len() <= 2048
        && reqwest::Url::parse(url)
            .map(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
            .unwrap_or(false)
}

/// `POST /v1/blobs` with a webhook. Fails before uploading if the keeper has webhooks disabled,
/// so the caller can fall back to a plain upload (and tell the payer) instead.
pub async fn upload_with_webhook(keeper: &str, ciphertext: &[u8], hook: &str) -> Result<Uploaded> {
    if !valid_hook(hook) {
        bail!("webhook must be an http(s) URL");
    }
    if !health(keeper, Duration::from_secs(30)).await?.webhooks {
        bail!("this keeper does not accept webhooks (it runs without --allow-webhooks)");
    }
    Keeper::new(keeper).upload_with(ciphertext, Some(hook)).await
}

#[cfg(test)]
mod tests {
    use super::valid_hook;

    #[test]
    fn hooks() {
        assert!(valid_hook("https://shop.example/hooks/so"));
        assert!(valid_hook("http://127.0.0.1:9000/n"));
        assert!(!valid_hook("ftp://x.example/"));
        assert!(!valid_hook("not a url"));
        assert!(!valid_hook(&format!("https://x.example/{}", "a".repeat(2100))));
    }
}
