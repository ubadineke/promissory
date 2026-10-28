//! Thin lightwalletd (gRPC) client.
//!
//! With the `tor` feature, [`connect_tor`] reaches lightwalletd through Tor, and [`connect`]
//! does too once a client is installed with [`crate::tor::install`].

use std::fmt;

use anyhow::{anyhow, Context, Result};
use tonic::transport::{Channel, ClientTlsConfig};
use zcash_client_backend::proto::service::{
    compact_tx_streamer_client::CompactTxStreamerClient, BlockId, ChainSpec, Empty, LightdInfo,
    RawTransaction, TreeState, TxFilter,
};
use zcash_protocol::{consensus::BlockHeight, TxId};

pub type Lwd = CompactTxStreamerClient<Channel>;

/// Connects to lightwalletd: over Tor if a process-wide client is installed (feature `tor`),
/// otherwise directly.
pub async fn connect(url: &str) -> Result<Lwd> {
    #[cfg(feature = "tor")]
    if let Some(tor) = crate::tor::installed() {
        return connect_tor(tor, url).await;
    }
    connect_direct(url).await
}

/// Connects to lightwalletd directly, ignoring any installed Tor client.
pub async fn connect_direct(url: &str) -> Result<Lwd> {
    // Without timeouts a server that stops answering would hang the caller (and any lock it
    // holds) indefinitely. Syncs stream many short calls, so a per-call limit is safe.
    let mut endpoint = Channel::from_shared(url.to_string())
        .context("bad lightwalletd url")?
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(120));
    if url.starts_with("https://") {
        endpoint = endpoint.tls_config(ClientTlsConfig::new().with_webpki_roots())?;
    }
    let channel = endpoint
        .connect()
        .await
        .with_context(|| format!("connecting to {url}"))?;
    Ok(CompactTxStreamerClient::new(channel).max_decoding_message_size(64 << 20))
}

/// Connects to lightwalletd through Tor. The host name is resolved by the Tor exit. `.onion`
/// endpoints are allowed. Pass `tor.isolated_client()` to keep this connection off the circuits
/// of every other one.
#[cfg(feature = "tor")]
pub async fn connect_tor(tor: &crate::tor::Client, url: &str) -> Result<Lwd> {
    let uri: tonic::transport::Uri = url.parse().context("bad lightwalletd url")?;
    let onion = uri.host().is_some_and(|h| h.ends_with(".onion"));
    let client = tor
        .connect_to_lightwalletd(uri, onion)
        .await
        .map_err(|e| anyhow!("connecting to {url} over Tor: {e}"))?;
    Ok(client.max_decoding_message_size(64 << 20))
}

pub async fn info(lwd: &mut Lwd) -> Result<LightdInfo> {
    Ok(lwd.get_lightd_info(Empty {}).await?.into_inner())
}

pub async fn tip_height(lwd: &mut Lwd) -> Result<BlockHeight> {
    let block = lwd.get_latest_block(ChainSpec {}).await?.into_inner();
    Ok(BlockHeight::from_u32(u32::try_from(block.height)?))
}

pub async fn tree_state(lwd: &mut Lwd, height: BlockHeight) -> Result<TreeState> {
    let id = BlockId {
        height: u64::from(u32::from(height)),
        hash: vec![],
    };
    Ok(lwd.get_tree_state(id).await?.into_inner())
}

/// Outcome of a `SendTransaction` call, with Zebra's rejection reason kept verbatim.
///
/// Failures to reach the node are not a `SendOutcome`: [`send`] returns them as an error
/// holding a [`Transport`], so callers can tell "the node said no" from "we never got there".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Accepted,
    Rejected { code: i32, message: String },
}

/// The request never got an answer from the node (connection refused or reset, timeout,
/// cancelled). Retrying is safe. Find it with `err.downcast_ref::<Transport>()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transport(pub String);

impl fmt::Display for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "lightwalletd unreachable: {}", self.0)
    }
}

impl std::error::Error for Transport {}

/// Whether `e` is (or wraps) a [`Transport`] failure.
pub fn is_transport(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.is::<Transport>())
}

/// gRPC statuses produced by the client side or the connection rather than by the server's
/// handler: tonic gives them a source error, or uses one of these codes.
fn status_is_transport(s: &tonic::Status) -> bool {
    use std::error::Error;
    matches!(
        s.code(),
        tonic::Code::Unavailable | tonic::Code::DeadlineExceeded | tonic::Code::Cancelled
    ) || (s.code() == tonic::Code::Unknown && s.source().is_some())
}

/// Sends a raw transaction. `Ok` is the node's verdict; a [`Transport`] error means it was
/// never reached.
pub async fn send(lwd: &mut Lwd, raw: &[u8]) -> Result<SendOutcome> {
    let resp = lwd
        .send_transaction(RawTransaction {
            data: raw.to_vec(),
            height: 0,
        })
        .await;
    match resp {
        Ok(r) => {
            let r = r.into_inner();
            if r.error_code == 0 {
                Ok(SendOutcome::Accepted)
            } else {
                Ok(SendOutcome::Rejected {
                    code: r.error_code,
                    message: r.error_message,
                })
            }
        }
        Err(status) if status_is_transport(&status) => Err(Transport(format!(
            "{:?}: {}",
            status.code(),
            status.message()
        ))
        .into()),
        // Some servers report rejections as gRPC errors rather than in the response.
        Err(status) => Ok(SendOutcome::Rejected {
            code: status.code() as i32,
            message: status.message().to_string(),
        }),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxStatus {
    Unknown,
    Mempool,
    Mined(BlockHeight),
    Orphaned,
}

pub async fn tx_status(lwd: &mut Lwd, txid: &TxId) -> Result<TxStatus> {
    let filter = TxFilter {
        block: None,
        index: 0,
        hash: txid.as_ref().to_vec(),
    };
    match lwd.get_transaction(filter).await {
        Ok(r) => Ok(match r.into_inner().height {
            0 => TxStatus::Mempool,
            u64::MAX => TxStatus::Orphaned,
            h => TxStatus::Mined(BlockHeight::from_u32(
                u32::try_from(h).map_err(|_| anyhow!("bad height {h}"))?,
            )),
        }),
        Err(s) if s.code() == tonic::Code::NotFound => Ok(TxStatus::Unknown),
        // lightwalletd reports an unknown txid as a generic error from the node.
        Err(s) if s.message().contains("No such mempool or") => Ok(TxStatus::Unknown),
        Err(s) => Err(anyhow!("GetTransaction: {s}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_statuses() {
        assert!(status_is_transport(&tonic::Status::unavailable("conn refused")));
        assert!(status_is_transport(&tonic::Status::deadline_exceeded("slow")));
        assert!(!status_is_transport(&tonic::Status::unknown("duplicate nullifier")));
        assert!(!status_is_transport(&tonic::Status::invalid_argument("bad tx")));
        let e: anyhow::Error = Transport("x".into()).into();
        assert!(is_transport(&e.context("SendTransaction")));
        assert!(!is_transport(&anyhow!("other")));
    }
}
