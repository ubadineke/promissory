//! Which chain we run against: mainnet, or the local regtest chain in `regtest/`.

use zcash_protocol::{
    consensus::{BlockHeight, MainNetwork, NetworkType, NetworkUpgrade, Parameters},
    local_consensus::LocalNetwork,
};

/// Consensus parameters the engine can run with: our [`SoNetwork`], or a wallet's own
/// (`zcash_protocol::consensus::Network`, `MainNetwork`, …).
pub trait Params: Parameters + Copy + Send + Sync + 'static {}

impl<P: Parameters + Copy + Send + Sync + 'static> Params for P {}

/// The height we expect to be mining at unix time `at`, given the tip at `now`, at the target
/// spacing: 75 s per block, dropping to 25 s at NU7 activation (ZIP 218).
pub fn predict_height<P: Parameters>(params: &P, tip: BlockHeight, now: u64, at: u64) -> BlockHeight {
    predict_height_spaced(params, tip, now, at, 75)
}

/// Like [`predict_height`], with `pre_nu7_secs` observed seconds per block before NU7 (see
/// `spacing::recent_block_secs`); after activation blocks come three times as fast.
pub fn predict_height_spaced<P: Parameters>(
    params: &P,
    tip: BlockHeight,
    now: u64,
    at: u64,
    pre_nu7_secs: u64,
) -> BlockHeight {
    let pre = pre_nu7_secs.max(1);
    let post = (pre / 3).max(1);
    let mut secs = at.saturating_sub(now);
    let mut h = u32::from(tip);
    if let Some(nu7) = params.activation_height(NetworkUpgrade::Nu7).map(u32::from) {
        if h < nu7 {
            let pre_secs = u64::from(nu7 - h) * pre;
            if secs <= pre_secs {
                return BlockHeight::from_u32(h + secs.div_ceil(pre) as u32).max(tip + 1);
            }
            secs -= pre_secs;
            h = nu7;
        }
    }
    BlockHeight::from_u32(h + secs.div_ceil(post) as u32).max(tip + 1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoNetwork {
    Main,
    Regtest(LocalNetwork),
}

impl SoNetwork {
    /// NU7 activation height of the default local chain (`regtest/zebrad.toml.tmpl`).
    pub const REGTEST_NU7: u32 = 5000;

    /// Must match `regtest/zebrad.toml.tmpl`: everything through NU6.3 at height 1, NU7 at 5000.
    pub fn regtest() -> Self {
        Self::regtest_with_nu7(Self::REGTEST_NU7)
    }

    /// A local chain with everything through NU6.3 at height 1 and NU7 at `nu7`. Must match the
    /// node's configured activation heights.
    pub fn regtest_with_nu7(nu7: u32) -> Self {
        let one = Some(BlockHeight::from_u32(1));
        SoNetwork::Regtest(LocalNetwork {
            overwinter: one,
            sapling: one,
            blossom: one,
            heartwood: one,
            canopy: one,
            nu5: one,
            nu6: one,
            nu6_1: one,
            nu6_2: one,
            nu6_3: one,
            nu7: Some(BlockHeight::from_u32(nu7)),
        })
    }

    /// Default maximum random release delay per installment. On mainnet, up to 3 hours, so a
    /// payment doesn't land exactly on its round's time; on a local chain none, so tests are quick.
    pub fn default_jitter_secs(&self) -> u64 {
        match self {
            SoNetwork::Main => 3 * 3600,
            SoNetwork::Regtest(_) => 0,
        }
    }

    /// Mean block interval in seconds at `height`.
    pub fn block_secs(&self, height: BlockHeight) -> u64 {
        if self.is_nu_active(NetworkUpgrade::Nu7, height) {
            25
        } else {
            75
        }
    }

    /// See [`predict_height`].
    pub fn predict_height(&self, tip: BlockHeight, now: u64, at: u64) -> BlockHeight {
        predict_height(self, tip, now, at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predicts_across_nu7() {
        let net = SoNetwork::regtest(); // NU7 at 5000
        let tip = BlockHeight::from_u32(4000);
        assert_eq!(net.predict_height(tip, 0, 0), BlockHeight::from_u32(4001));
        assert_eq!(net.predict_height(tip, 0, 750), BlockHeight::from_u32(4010));
        // 1000 blocks at 75 s reach activation; 250 s more is 10 blocks at 25 s.
        assert_eq!(net.predict_height(tip, 0, 75_000 + 250), BlockHeight::from_u32(5010));
    }

    #[test]
    fn measured_spacing() {
        let net = SoNetwork::regtest(); // NU7 at 5000
        let tip = BlockHeight::from_u32(100);
        // A local miner at 15 s: ten minutes is 40 blocks, not 8.
        assert_eq!(predict_height_spaced(&net, tip, 0, 600, 15), BlockHeight::from_u32(140));
        assert_eq!(predict_height_spaced(&net, tip, 0, 600, 75), net.predict_height(tip, 0, 600));
    }

    #[test]
    fn mainnet_nu7_is_scheduled() {
        assert_eq!(
            SoNetwork::Main.activation_height(NetworkUpgrade::Nu7),
            Some(BlockHeight::from_u32(3_543_405))
        );
    }
}

impl Parameters for SoNetwork {
    fn network_type(&self) -> NetworkType {
        match self {
            SoNetwork::Main => NetworkType::Main,
            SoNetwork::Regtest(_) => NetworkType::Regtest,
        }
    }

    fn activation_height(&self, nu: NetworkUpgrade) -> Option<BlockHeight> {
        match self {
            SoNetwork::Main => MainNetwork.activation_height(nu),
            SoNetwork::Regtest(local) => local.activation_height(nu),
        }
    }
}
