//! The plaintext inside a tlock blob: one installment's raw transactions, one per consensus
//! branch. All copies spend the same note, so at most one can ever be mined; the keeper
//! broadcasts the copy whose branch matches the next block.

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use zcash_protocol::consensus::{BlockHeight, BranchId, Parameters};

pub const VERSION: u8 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Copy {
    /// Consensus branch ID the copy was signed for, e.g. `0x37a5165b` (NU6.3).
    pub branch_id: u32,
    /// Raw transaction bytes, hex-encoded.
    pub tx: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u8,
    pub copies: Vec<Copy>,
}

impl Envelope {
    pub fn new(copies: Vec<(BranchId, Vec<u8>)>) -> Self {
        Self {
            v: VERSION,
            copies: copies
                .into_iter()
                .map(|(branch, raw)| Copy {
                    branch_id: u32::from(branch),
                    tx: hex::encode(raw),
                })
                .collect(),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("envelope serializes")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let env: Self = serde_json::from_slice(bytes).map_err(|e| anyhow!("envelope: {e}"))?;
        if env.v != VERSION {
            bail!("unsupported envelope version {}", env.v);
        }
        if env.copies.is_empty() {
            bail!("envelope has no copies");
        }
        Ok(env)
    }

    /// The branch for a block at `next_height` and the raw transaction signed for it, if the
    /// envelope has one. Never returns a wrong-branch copy: Zebra penalises those outside the
    /// upgrade window.
    pub fn copy_for<P: Parameters>(
        &self,
        params: &P,
        next_height: BlockHeight,
    ) -> Result<Option<(BranchId, Vec<u8>)>> {
        let branch = BranchId::for_height(params, next_height);
        let want = u32::from(branch);
        self.copies
            .iter()
            .find(|c| c.branch_id == want)
            .map(|c| {
                hex::decode(&c.tx)
                    .map(|raw| (branch, raw))
                    .map_err(|e| anyhow!("copy hex: {e}"))
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::SoNetwork;

    #[test]
    fn picks_copy_by_branch() {
        let params = SoNetwork::regtest();
        let env = Envelope::new(vec![
            (BranchId::Nu6_3, vec![1, 2, 3]),
            (BranchId::Nu7, vec![4, 5, 6]),
        ]);
        let env = Envelope::from_bytes(&env.to_bytes()).unwrap();
        assert_eq!(
            env.copy_for(&params, BlockHeight::from_u32(4999)).unwrap(),
            Some((BranchId::Nu6_3, vec![1, 2, 3]))
        );
        assert_eq!(
            env.copy_for(&params, BlockHeight::from_u32(5000)).unwrap(),
            Some((BranchId::Nu7, vec![4, 5, 6]))
        );
    }

    #[test]
    fn no_wrong_branch_copy() {
        let params = SoNetwork::regtest();
        let env = Envelope::new(vec![(BranchId::Nu6_3, vec![1])]);
        assert_eq!(env.copy_for(&params, BlockHeight::from_u32(6000)).unwrap(), None);
    }
}
