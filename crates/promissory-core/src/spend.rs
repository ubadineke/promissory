//! Note selection and the sweep (cancel) transaction.

use std::num::NonZeroU32;

use anyhow::{anyhow, Result};
use zcash_client_backend::{
    data_api::{
        wallet::{
            input_selection::{LockFilter, LockedInputPolicy},
            ConfirmationsPolicy, TargetHeight,
        },
        InputSource, MaxSpendMode, TargetValue, WalletRead,
    },
    wallet::ReceivedNote,
};
use zcash_client_sqlite::{AccountUuid, ReceivedNoteId};
use zcash_keys::keys::UnifiedSpendingKey;
use zcash_protocol::{
    consensus::{BlockHeight, BranchId},
    memo::MemoBytes,
    value::Zatoshis,
    ShieldedPool,
};
use zcash_primitives::transaction::builder::default_tx_expiry_delta;

use crate::{
    network::Params,
    txbuild::{self, Built, Output},
    wallet::Db,
};

pub type IronwoodNote = ReceivedNote<ReceivedNoteId, orchard::Note>;

/// Confirmations an anchor sits behind the tip; the same default ordinary wallets use, so our
/// anchors look like theirs.
pub const ANCHOR_CONFIRMATIONS: u32 = 3;

/// Target and anchor heights for a transaction built now.
pub fn heights_now<P: Params>(db: &Db<P>) -> Result<(BlockHeight, BlockHeight)> {
    let (target, anchor) = db
        .get_target_and_anchor_heights(NonZeroU32::new(ANCHOR_CONFIRMATIONS).unwrap())?
        .ok_or_else(|| anyhow!("wallet not synced"))?;
    Ok((BlockHeight::from(target), anchor))
}

/// Every spendable Ironwood note in `account`, oldest commitment first.
pub fn ironwood_notes<P: Params>(db: &Db<P>, account: AccountUuid) -> Result<Vec<IronwoodNote>> {
    let (target, _) = heights_now(db)?;
    let notes = db.select_spendable_notes(
        account,
        TargetValue::AllFunds(MaxSpendMode::MaxSpendable),
        &[ShieldedPool::Ironwood],
        TargetHeight::from(target),
        ConfirmationsPolicy::default(),
        &[],
        LockFilter::Policy(&LockedInputPolicy::Exclude),
    )?;
    let mut notes = notes.ironwood().to_vec();
    notes.sort_by_key(|n| u64::from(n.note_commitment_tree_position()));
    Ok(notes)
}

/// Spends `notes` in one transaction to `to`, with the ordinary expiry delta and no outgoing
/// viewing key: a viewing key shared for proof of reservation must not reveal where a cancel
/// sent the funds.
pub fn sweep<P: Params>(
    db: &mut Db<P>,
    usk: &UnifiedSpendingKey,
    notes: &[IronwoodNote],
    to: &str,
    memo: MemoBytes,
) -> Result<Built> {
    let params = *db.params();
    let to = txbuild::orchard_receiver(&params, to)?;
    let (target, anchor) = heights_now(db)?;
    let total: u64 = notes.iter().map(|n| n.note().value().inner()).sum();
    let fee = txbuild::ironwood_fee(notes.len(), 1);
    let value = Zatoshis::from_u64(
        total
            .checked_sub(fee.into_u64())
            .ok_or_else(|| anyhow!("notes ({total} zat) do not cover the fee"))?,
    )?;
    let expiry = target + default_tx_expiry_delta(BranchId::for_height(&params, target));
    txbuild::build(
        db,
        usk,
        notes,
        &[Output { to, value, memo }],
        None,
        target,
        anchor,
        expiry,
    )
}
