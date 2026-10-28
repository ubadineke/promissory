//! Low-level Ironwood transaction construction: explicit notes, anchor, target height and
//! expiry. Installments, NU7 copies and the cancel sweep are all built here, never through the
//! proposal APIs, so we control every field an observer can see.

use anyhow::{anyhow, bail, Result};
use pczt::roles::{creator::Creator, io_finalizer::IoFinalizer};
use rand::{rand_core::UnwrapErr, rngs::SysRng};
use zcash_client_backend::{data_api::WalletCommitmentTrees, wallet::ReceivedNote};
use zcash_keys::{address::Address, keys::UnifiedSpendingKey};
use zcash_primitives::transaction::{
    builder::{BuildConfig, Builder, BundlePadding},
    fees::zip317,
};
use zcash_protocol::{
    consensus::{BlockHeight, BranchId, Parameters},
    memo::MemoBytes,
    value::Zatoshis,
    TxId,
};

use crate::{authorize, network::Params, wallet::Db};

pub struct Output {
    pub to: orchard::Address,
    pub value: Zatoshis,
    pub memo: MemoBytes,
}

pub struct Built {
    pub txid: TxId,
    pub raw: Vec<u8>,
    pub branch: BranchId,
    pub target_height: BlockHeight,
    pub expiry_height: BlockHeight,
    pub anchor_height: BlockHeight,
    pub fee: Zatoshis,
}

/// The Orchard receiver of a unified address. Since NU6.3 a payment to it lands in the Ironwood
/// pool. Anything without one is rejected: paying another pool would cross pools and reveal the
/// amount on chain.
pub fn orchard_receiver<P: Parameters>(params: &P, addr: &str) -> Result<orchard::Address> {
    match Address::decode(params, addr) {
        Some(Address::Unified(ua)) => ua
            .orchard()
            .copied()
            .ok_or_else(|| anyhow!("address has no Orchard/Ironwood receiver")),
        Some(_) => bail!("not a unified address with an Orchard/Ironwood receiver"),
        None => bail!("not a valid address for this network"),
    }
}

/// ZIP 317 fee for a transaction whose only bundle is Ironwood, padded to at least 2 actions.
pub fn ironwood_fee(spends: usize, outputs: usize) -> Zatoshis {
    let actions = spends.max(outputs).max(2) as u64;
    let grace = zip317::GRACE_ACTIONS as u64;
    Zatoshis::const_from_u64(zip317::MARGINAL_FEE.into_u64() * actions.max(grace))
}

/// Builds and authorizes a transaction spending exactly `notes` (all from the account `usk`
/// controls) to `outputs`, anchored at `anchor_height`, for inclusion at `target_height`.
/// The consensus branch follows from `target_height`.
///
/// The sum of `notes` must equal the sum of `outputs` plus the ZIP 317 fee: no change output
/// is ever added.
#[allow(clippy::too_many_arguments)]
pub fn build<N, P: Params>(
    db: &mut Db<P>,
    usk: &UnifiedSpendingKey,
    notes: &[ReceivedNote<N, orchard::Note>],
    outputs: &[Output],
    ovk: Option<orchard::keys::OutgoingViewingKey>,
    target_height: BlockHeight,
    anchor_height: BlockHeight,
    expiry_height: BlockHeight,
) -> Result<Built> {
    build_multi(db, &[(usk, notes)], outputs, ovk, target_height, anchor_height, expiry_height)
}

/// [`build`] spending notes of several accounts in one transaction: each group of notes is
/// spent with its own account's key. Used to renew an inheritance switch in one transaction
/// (the old reservation plus a treasury note for the fee, into the new reservation and change).
#[allow(clippy::too_many_arguments)]
pub fn build_multi<N, P: Params>(
    db: &mut Db<P>,
    spends: &[(&UnifiedSpendingKey, &[ReceivedNote<N, orchard::Note>])],
    outputs: &[Output],
    ovk: Option<orchard::keys::OutgoingViewingKey>,
    target_height: BlockHeight,
    anchor_height: BlockHeight,
    expiry_height: BlockHeight,
) -> Result<Built> {
    let n_spends: usize = spends.iter().map(|(_, n)| n.len()).sum();
    if n_spends == 0 {
        bail!("no notes to spend");
    }
    let params = *db.params();
    let fee = ironwood_fee(n_spends, outputs.len());
    let input: u64 = spends
        .iter()
        .flat_map(|(_, notes)| notes.iter())
        .map(|n| n.note().value().inner())
        .sum();
    let output: u64 = outputs.iter().map(|o| o.value.into_u64()).sum();
    if input != output + fee.into_u64() {
        bail!(
            "inputs {input} != outputs {output} + fee {}; Promissory transactions have no change",
            fee.into_u64()
        );
    }

    let all: Vec<&ReceivedNote<N, orchard::Note>> = spends.iter().flat_map(|(_, notes)| notes.iter()).collect();
    let (anchor, paths) = db
        .with_ironwood_tree_mut::<_, _, anyhow::Error>(|tree| {
            let anchor = tree
                .root_at_checkpoint_id(&anchor_height)?
                .ok_or_else(|| anyhow!("no Ironwood checkpoint at anchor height {anchor_height}"))?;
            let paths = all
                .iter()
                .map(|n| {
                    tree.witness_at_checkpoint_id_caching(
                        n.note_commitment_tree_position(),
                        &anchor_height,
                    )?
                    .ok_or_else(|| anyhow!("note witness unavailable at {anchor_height}"))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((anchor, paths))
        })?
        .ok_or_else(|| anyhow!("wallet has no Ironwood tree"))?;

    let mut builder = Builder::new(
        params,
        target_height,
        BuildConfig::Standard {
            sapling_anchor: None,
            orchard_anchor: None,
            ironwood_anchor: Some(anchor.into()),
            orchard_padding: BundlePadding::DEFAULT,
            ironwood_padding: BundlePadding::DEFAULT,
        },
    )
    .with_expiry_height(expiry_height);
    let mut paths = paths.into_iter();
    for (usk, notes) in spends {
        let fvk = usk
            .to_unified_full_viewing_key()
            .orchard()
            .cloned()
            .ok_or_else(|| anyhow!("account has no Orchard-protocol key"))?;
        for note in notes.iter() {
            let path = paths.next().expect("one path per note");
            builder
                .add_ironwood_spend::<std::convert::Infallible>(fvk.clone(), *note.note(), path.into())
                .map_err(|e| anyhow!("add spend: {e}"))?;
        }
    }
    for o in outputs {
        builder
            .add_ironwood_output::<std::convert::Infallible>(ovk.clone(), o.to, o.value, o.memo.clone())
            .map_err(|e| anyhow!("add output: {e}"))?;
    }
    let builder_fee = builder
        .get_fee(&zip317::FeeRule::standard())
        .map_err(|e| anyhow!("fee: {e:?}"))?;
    if builder_fee != fee {
        bail!("builder fee {builder_fee:?} differs from computed fee {fee:?}");
    }

    let result = builder
        .build_for_pczt(UnwrapErr(SysRng), &zip317::FeeRule::standard())
        .map_err(|e| anyhow!("build: {e}"))?;
    let pczt = Creator::build_from_parts(result.pczt_parts)
        .ok_or_else(|| anyhow!("transaction version not representable as a PCZT"))?;
    let pczt = IoFinalizer::new(pczt)
        .finalize_io(UnwrapErr(SysRng))
        .map_err(|e| anyhow!("io finalize: {e:?}"))?;
    let branch = authorize::branch_of(&pczt)?;
    let usks: Vec<&UnifiedSpendingKey> = spends.iter().map(|(usk, _)| *usk).collect();
    let tx = authorize::authorize(pczt, &usks, n_spends)?;

    Ok(Built {
        txid: tx.txid(),
        raw: authorize::serialize(&tx)?,
        branch,
        target_height,
        expiry_height,
        anchor_height,
        fee,
    })
}
