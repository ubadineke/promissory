//! drand quicknet timelock encryption of raw transactions.
//!
//! [`Drand`] is a blocking client over clearnet relays. With the `tor` feature, [`TorDrand`]
//! fetches the same beacons through Tor and verifies each signature against quicknet's
//! public key, and [`Drand`] itself uses Tor once a client is installed with
//! [`crate::tor::install`].

use anyhow::{anyhow, Result};
use drand_core::HttpClient;

/// drand quicknet (`bls-unchained-g1-rfc9380`, 3 s period).
pub const QUICKNET_HASH: &str = "52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971";
/// quicknet's group public key (G2), as served by `/info`.
pub const QUICKNET_PUBLIC_KEY: &str = "83cf0f2896adee7eb8b5f01fcad3912212c437e0073e911fb90022d3e760183c8c4b450b6a0a6c3ac6a5776a2d1064510d1fec758c921cc22b0e17e63aaf4bcb5ed66304de9cf809bd274ca73bab4af5a6e9c76a4bc09e76eae8991ef5ece45a";
/// Longest we wait on one blocking drand call. `drand_core`'s HTTP client sets no timeout, so a
/// relay that stops answering mid-request would otherwise hang its caller for good.
pub const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Runs a blocking drand call on the blocking pool and gives up after [`CALL_TIMEOUT`]. A call
/// that times out keeps its thread until the socket dies, but no caller waits on it.
pub async fn bounded<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    bounded_for(CALL_TIMEOUT, f).await
}

async fn bounded_for<T: Send + 'static>(
    limit: std::time::Duration,
    f: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    match tokio::time::timeout(limit, tokio::task::spawn_blocking(f)).await {
        Ok(joined) => joined.map_err(|e| anyhow!("drand call panicked: {e}"))?,
        Err(_) => Err(anyhow!("drand relay did not answer within {}s", limit.as_secs_f32())),
    }
}

pub const RELAYS: &[&str] = &[
    "https://api.drand.sh",
    "https://api2.drand.sh",
    "https://api3.drand.sh",
    "https://drand.cloudflare.com",
];

/// The parameters needed to encrypt to quicknet, as served by `/info`.
#[derive(Clone, Debug)]
pub struct Chain {
    pub hash: Vec<u8>,
    pub public_key: Vec<u8>,
    pub genesis_time: u64,
    pub period: u64,
}

impl Chain {
    /// The first round whose beacon is published at or after `unix`.
    ///
    /// Same as drand-client's `roundAt`: `floor((t - genesis) / period) + 1`. We do not use
    /// `drand_core::HttpClient::get_by_unix_time`, which omits the `+ 1`.
    pub fn round_at(&self, unix: u64) -> u64 {
        if unix < self.genesis_time {
            return 1;
        }
        (unix - self.genesis_time) / self.period + 1
    }

    /// The unix time at which `round` is published.
    pub fn round_time(&self, round: u64) -> u64 {
        self.genesis_time + round.saturating_sub(1) * self.period
    }
}

/// A drand client that falls back across relays.
///
/// With the `tor` feature and a client installed by [`crate::tor::install`], every call goes
/// through Tor instead (see [`TorDrand`]); the methods then need a multi-thread tokio runtime.
pub struct Drand {
    clients: Vec<HttpClient>,
    #[cfg(feature = "tor")]
    tor: Option<TorDrand>,
}

impl Drand {
    pub fn quicknet() -> Result<Self> {
        let clients = RELAYS
            .iter()
            .filter_map(|relay| HttpClient::new(&format!("{relay}/{QUICKNET_HASH}"), None).ok())
            .collect::<Vec<_>>();
        if clients.is_empty() {
            return Err(anyhow!("no drand relay could be configured"));
        }
        Ok(Self {
            clients,
            #[cfg(feature = "tor")]
            tor: crate::tor::installed().map(TorDrand::new),
        })
    }

    fn first_ok<T>(&self, f: impl Fn(&HttpClient) -> anyhow::Result<T>) -> Result<T> {
        let mut last = None;
        for c in &self.clients {
            match f(c) {
                Ok(v) => return Ok(v),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("no drand relays")))
    }

    pub fn chain(&self) -> Result<Chain> {
        #[cfg(feature = "tor")]
        if let Some(t) = &self.tor {
            return crate::tor::block_on(t.chain())?;
        }
        let info = self.first_ok(|c| Ok(c.chain_info()?))?;
        let chain = Chain {
            hash: info.hash(),
            public_key: info.public_key(),
            genesis_time: info.genesis_time(),
            period: info.period(),
        };
        if hex::encode(&chain.hash) != QUICKNET_HASH {
            return Err(anyhow!("relay served an unexpected chain"));
        }
        Ok(chain)
    }

    pub fn latest_round(&self) -> Result<u64> {
        #[cfg(feature = "tor")]
        if let Some(t) = &self.tor {
            return crate::tor::block_on(t.latest_round())?;
        }
        self.first_ok(|c| Ok(c.latest()?.round()))
    }

    /// The beacon signature for `round`, or `None` if it has not been published yet.
    ///
    /// One request when the round is out. Only if every relay fails is the latest round
    /// fetched, to tell "not yet" from a real error.
    pub fn signature(&self, round: u64) -> Result<Option<Vec<u8>>> {
        #[cfg(feature = "tor")]
        if let Some(t) = &self.tor {
            return crate::tor::block_on(t.signature(round))?;
        }
        match self.first_ok(|c| Ok(c.get(round)?.signature())) {
            Ok(sig) => Ok(Some(sig)),
            Err(e) => match self.latest_round() {
                Ok(latest) if latest < round => Ok(None),
                _ => Err(e),
            },
        }
    }
}

/// drand quicknet over Tor, through the relays in [`RELAYS`]. Every beacon is verified
/// against [`QUICKNET_PUBLIC_KEY`], so a relay or exit can withhold a beacon but not forge one.
#[cfg(feature = "tor")]
pub struct TorDrand {
    tor: crate::tor::Client,
    info: tokio::sync::OnceCell<drand_core::chain::ChainInfo>,
}

#[cfg(feature = "tor")]
impl TorDrand {
    /// Uses its own isolated circuits, unlinkable to other users of `tor`.
    pub fn new(tor: &crate::tor::Client) -> Self {
        Self {
            tor: tor.isolated_client(),
            info: tokio::sync::OnceCell::new(),
        }
    }

    /// GETs `path` under the quicknet base of each relay in turn; returns the first response
    /// that arrives (any status).
    async fn get(&self, path: &str) -> Result<crate::tor::Reply> {
        let mut last = None;
        for relay in RELAYS {
            let url = format!("{relay}/{QUICKNET_HASH}/{path}");
            match crate::tor::http(&self.tor, &url, None, None).await {
                Ok(r) if r.status < 500 => return Ok(r),
                Ok(r) => last = Some(anyhow!("drand {relay}: HTTP {}", r.status)),
                Err(e) => last = Some(e.context(format!("drand {relay}"))),
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("no drand relays")))
    }

    async fn info(&self) -> Result<&drand_core::chain::ChainInfo> {
        self.info
            .get_or_try_init(|| async {
                let r = self.get("info").await?;
                if r.status != 200 {
                    return Err(anyhow!("drand info: HTTP {}", r.status));
                }
                let info: drand_core::chain::ChainInfo =
                    serde_json::from_slice(&r.body).map_err(|e| anyhow!("drand info: {e}"))?;
                if hex::encode(info.hash()) != QUICKNET_HASH
                    || hex::encode(info.public_key()) != QUICKNET_PUBLIC_KEY
                {
                    return Err(anyhow!("relay served an unexpected chain"));
                }
                Ok(info)
            })
            .await
    }

    /// Fetches and verifies a beacon; `None` if the relay says it does not exist yet.
    async fn beacon(&self, which: &str) -> Result<Option<drand_core::beacon::ApiBeacon>> {
        let info = self.info().await?.clone();
        let r = self.get(&format!("public/{which}")).await?;
        if matches!(r.status, 404 | 425) {
            return Ok(None);
        }
        if r.status != 200 {
            return Err(anyhow!("drand beacon {which}: HTTP {}", r.status));
        }
        let b: drand_core::beacon::ApiBeacon =
            serde_json::from_slice(&r.body).map_err(|e| anyhow!("drand beacon: {e}"))?;
        if !b.verify(info).map_err(|e| anyhow!("drand verify: {e}"))? {
            return Err(anyhow!("drand beacon {which} failed verification"));
        }
        Ok(Some(b))
    }

    pub async fn chain(&self) -> Result<Chain> {
        let info = self.info().await?;
        Ok(Chain {
            hash: info.hash(),
            public_key: info.public_key(),
            genesis_time: info.genesis_time(),
            period: info.period(),
        })
    }

    pub async fn latest_round(&self) -> Result<u64> {
        self.beacon("latest")
            .await?
            .map(|b| b.round())
            .ok_or_else(|| anyhow!("drand has no latest beacon"))
    }

    /// The verified signature for `round`, or `None` if it has not been published yet.
    pub async fn signature(&self, round: u64) -> Result<Option<Vec<u8>>> {
        match self.beacon(&round.to_string()).await? {
            Some(b) if b.round() == round => Ok(Some(b.signature())),
            Some(b) => Err(anyhow!("asked for round {round}, relay sent {}", b.round())),
            None => Ok(None),
        }
    }
}

pub fn encrypt(chain: &Chain, round: u64, plaintext: &[u8]) -> Result<Vec<u8>> {
    let mut out = vec![];
    tlock_age::encrypt(&mut out, plaintext, &chain.hash, &chain.public_key, round)
        .map_err(|e| anyhow!("tlock encrypt: {e}"))?;
    Ok(out)
}

pub fn decrypt(chain_hash: &[u8], signature: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    let mut out = vec![];
    tlock_age::decrypt(&mut out, ciphertext, chain_hash, signature)
        .map_err(|e| anyhow!("tlock decrypt: {e}"))?;
    Ok(out)
}

/// The round a ciphertext is locked to, read from its age header.
pub fn locked_round(ciphertext: &[u8]) -> Result<u64> {
    Ok(locked_header(ciphertext)?.0)
}

/// The round and drand chain hash a ciphertext is locked to, read from its age header.
pub fn locked_header(ciphertext: &[u8]) -> Result<(u64, Vec<u8>)> {
    let header = tlock_age::decrypt_header(ciphertext).map_err(|e| anyhow!("tlock header: {e}"))?;
    Ok((header.round(), header.hash()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_hung_call_is_abandoned() {
        let limit = std::time::Duration::from_millis(50);
        let hung = bounded_for(limit, || {
            std::thread::sleep(std::time::Duration::from_secs(2));
            Ok(())
        });
        let started = std::time::Instant::now();
        assert!(hung.await.unwrap_err().to_string().contains("did not answer"));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert_eq!(bounded_for(limit, || Ok(7)).await.unwrap(), 7);
    }

    fn quicknet() -> Chain {
        Chain {
            hash: hex::decode(QUICKNET_HASH).unwrap(),
            public_key: vec![],
            genesis_time: 1692803367,
            period: 3,
        }
    }

    #[test]
    fn round_at_matches_drand_client() {
        let c = quicknet();
        assert_eq!(c.round_at(0), 1);
        assert_eq!(c.round_at(c.genesis_time), 1);
        assert_eq!(c.round_at(c.genesis_time + 2), 1);
        assert_eq!(c.round_at(c.genesis_time + 3), 2);
    }

    #[test]
    fn header_of_a_quicknet_ciphertext() {
        let mut c = quicknet();
        c.public_key = hex::decode(QUICKNET_PUBLIC_KEY).unwrap();
        let ct = encrypt(&c, 1234, b"hi").unwrap();
        assert_eq!(locked_header(&ct).unwrap(), (1234, hex::decode(QUICKNET_HASH).unwrap()));
        assert_eq!(locked_round(&ct).unwrap(), 1234);
        assert!(locked_header(b"nope").is_err());
    }

    #[test]
    fn round_time_inverts_round_at() {
        let c = quicknet();
        for round in [1u64, 2, 1000, 33_381_386] {
            assert_eq!(c.round_at(c.round_time(round)), round);
        }
    }
}
