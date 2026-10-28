//! Observed block spacing, for predicting heights from wall-clock times.

use zcash_protocol::consensus::BlockHeight;

use crate::net::{self, Lwd};

/// The target spacing before NU7, used when the chain can't be measured.
pub const DEFAULT_SECS: u64 = 75;

/// Mean seconds per block over the last `n` blocks, from the block times lightwalletd reports.
/// Falls back to [`DEFAULT_SECS`] if the chain is too short or the server can't say. On a live
/// chain this tracks hash-rate swings; on a local chain it tracks whatever the miner is doing.
pub async fn recent_block_secs(lwd: &mut Lwd, tip: BlockHeight, n: u32) -> u64 {
    if u32::from(tip) <= n {
        return DEFAULT_SECS;
    }
    let (Ok(end), Ok(start)) = (net::tree_state(lwd, tip).await, net::tree_state(lwd, tip - n).await) else {
        return DEFAULT_SECS;
    };
    match end.time.checked_sub(start.time) {
        Some(elapsed) if elapsed > 0 => (u64::from(elapsed) / u64::from(n)).clamp(1, 600),
        _ => DEFAULT_SECS,
    }
}
