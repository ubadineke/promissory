//! The two outside services the keeper depends on, behind traits so the release loop can be
//! tested against fakes.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use promissory_core::{
    net::{self, Lwd, SendOutcome, TxStatus},
    tlock::{self, Drand},
};
use tokio::sync::Mutex;
use zcash_protocol::{consensus::BlockHeight, TxId};

/// The drand beacon the ciphertexts are locked to.
#[async_trait]
pub trait Beacon: Send + Sync {
    async fn latest_round(&self) -> Result<u64>;
    /// The signature for `round`, or `None` if it has not been published yet.
    async fn signature(&self, round: u64) -> Result<Option<Vec<u8>>>;
    fn chain_hash(&self) -> Vec<u8>;
    /// `"direct"` or `"tor"`, for `/v1/health`.
    fn transport(&self) -> &'static str {
        "direct"
    }
}

/// The Zcash chain, as seen through lightwalletd.
#[async_trait]
pub trait Chain: Send + Sync {
    async fn tip_height(&self) -> Result<BlockHeight>;
    async fn send(&self, raw: &[u8]) -> Result<SendOutcome>;
    async fn tx_status(&self, txid: &TxId) -> Result<TxStatus>;
    /// `"direct"` or `"tor"`, for `/v1/health`.
    fn transport(&self) -> &'static str {
        "direct"
    }
}

/// drand quicknet over HTTP. `tlock::Drand` is blocking, so every call runs on the blocking pool.
pub struct DrandBeacon {
    drand: Arc<Drand>,
}

impl DrandBeacon {
    pub fn quicknet() -> Result<Self> {
        Ok(Self {
            drand: Arc::new(Drand::quicknet()?),
        })
    }
}

#[async_trait]
impl Beacon for DrandBeacon {
    async fn latest_round(&self) -> Result<u64> {
        let d = self.drand.clone();
        tlock::bounded(move || d.latest_round()).await
    }

    async fn signature(&self, round: u64) -> Result<Option<Vec<u8>>> {
        let d = self.drand.clone();
        tlock::bounded(move || d.signature(round)).await
    }

    fn chain_hash(&self) -> Vec<u8> {
        hex::decode(tlock::QUICKNET_HASH).expect("constant hash")
    }
}

/// drand quicknet over Tor, every beacon verified against quicknet's public key.
#[cfg(feature = "tor")]
pub struct TorDrandBeacon {
    drand: tlock::TorDrand,
}

#[cfg(feature = "tor")]
impl TorDrandBeacon {
    pub fn new(tor: &promissory_core::tor::Client) -> Self {
        Self {
            drand: tlock::TorDrand::new(tor),
        }
    }
}

#[cfg(feature = "tor")]
#[async_trait]
impl Beacon for TorDrandBeacon {
    async fn latest_round(&self) -> Result<u64> {
        self.drand.latest_round().await
    }

    async fn signature(&self, round: u64) -> Result<Option<Vec<u8>>> {
        self.drand.signature(round).await
    }

    fn chain_hash(&self) -> Vec<u8> {
        hex::decode(tlock::QUICKNET_HASH).expect("constant hash")
    }

    fn transport(&self) -> &'static str {
        "tor"
    }
}

enum Via {
    Direct,
    #[cfg(feature = "tor")]
    Tor(promissory_core::tor::Client),
}

/// lightwalletd over gRPC. Connects lazily and reconnects after a transport error, so the
/// keeper starts (and keeps serving the API) while lightwalletd is down.
///
/// Over Tor, polling (tip, transaction status) shares one connection on its own isolated
/// circuit, and every broadcast opens a fresh connection on a newly isolated circuit, so no
/// two broadcasts, and no broadcast and the polling, share a circuit or exit.
pub struct LwdChain {
    url: String,
    via: Via,
    client: Mutex<Option<Lwd>>,
}

impl LwdChain {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            via: Via::Direct,
            client: Mutex::new(None),
        }
    }

    /// lightwalletd through Tor. Never falls back to a direct connection.
    #[cfg(feature = "tor")]
    pub fn tor(url: impl Into<String>, tor: &promissory_core::tor::Client) -> Self {
        Self {
            url: url.into(),
            via: Via::Tor(tor.isolated_client()),
            client: Mutex::new(None),
        }
    }

    async fn connect(&self, fresh_circuit: bool) -> Result<Lwd> {
        let _ = fresh_circuit;
        match &self.via {
            Via::Direct => net::connect_direct(&self.url).await,
            #[cfg(feature = "tor")]
            Via::Tor(tor) if fresh_circuit => net::connect_tor(&tor.isolated_client(), &self.url).await,
            #[cfg(feature = "tor")]
            Via::Tor(tor) => net::connect_tor(tor, &self.url).await,
        }
    }

    async fn client(&self) -> Result<Lwd> {
        let mut c = self.client.lock().await;
        if c.is_none() {
            *c = Some(self.connect(false).await?);
        }
        Ok(c.clone().expect("connected"))
    }

    async fn reset_on_err<T>(&self, r: Result<T>) -> Result<T> {
        if r.is_err() {
            *self.client.lock().await = None;
        }
        r
    }
}

#[async_trait]
impl Chain for LwdChain {
    async fn tip_height(&self) -> Result<BlockHeight> {
        let mut c = self.client().await?;
        let r = net::tip_height(&mut c).await;
        self.reset_on_err(r).await
    }

    async fn send(&self, raw: &[u8]) -> Result<SendOutcome> {
        let fresh = !matches!(self.via, Via::Direct);
        let c = if fresh {
            self.connect(true).await
        } else {
            self.client().await
        };
        // Not connecting is a transport failure too: safe to retry next tick.
        let mut c = c.map_err(|e| net::Transport(format!("{e:#}")))?;
        let r = net::send(&mut c, raw).await;
        if fresh {
            r
        } else {
            self.reset_on_err(r).await
        }
    }

    async fn tx_status(&self, txid: &TxId) -> Result<TxStatus> {
        let mut c = self.client().await?;
        let r = net::tx_status(&mut c, txid).await;
        self.reset_on_err(r).await
    }

    fn transport(&self) -> &'static str {
        match self.via {
            Via::Direct => "direct",
            #[cfg(feature = "tor")]
            Via::Tor(_) => "tor",
        }
    }
}

/// Parses a display-order (byte-reversed) txid.
pub fn parse_txid(s: &str) -> Result<TxId> {
    let mut bytes: [u8; 32] = hex::decode(s)?
        .try_into()
        .map_err(|_| anyhow!("txid must be 32 bytes"))?;
    bytes.reverse();
    Ok(TxId::from_bytes(bytes))
}
