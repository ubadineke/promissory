//! Kill-gate pre-signing via the proposal path: send-max from the account to a payee,
//! authorized into raw transaction bytes, without storing anything as sent in the wallet.

use std::convert::Infallible;

use anyhow::{anyhow, Result};
use rand::{rand_core::UnwrapErr, rngs::SysRng};
use zcash_address::ZcashAddress;
use zcash_client_backend::{
    data_api::{
        wallet::{
            create_pczt_from_proposal, input_selection::LockedInputPolicy,
            propose_send_max_transfer, ConfirmationsPolicy,
        },
        MaxSpendMode,
    },
    util::SystemClock,
    wallet::OvkPolicy,
};
use zcash_client_sqlite::AccountUuid;
use zcash_primitives::transaction::{builder::BundlePadding, fees::zip317};
use zcash_protocol::{
    consensus::{BlockHeight, BranchId},
    memo::MemoBytes,
    ShieldedPool, TxId,
};
use zcash_keys::keys::UnifiedSpendingKey;

use crate::{authorize, network::Params, wallet::Db};

pub struct Presigned {
    pub txid: TxId,
    pub raw: Vec<u8>,
    pub branch: BranchId,
    pub target_height: BlockHeight,
    pub expiry_height: BlockHeight,
    pub fee_zat: u64,
    pub value_zat: u64,
}

pub fn presign_send_max<P: Params>(
    db: &mut Db<P>,
    account: AccountUuid,
    usk: &UnifiedSpendingKey,
    payee: &str,
    memo: Option<MemoBytes>,
    expiry: Option<BlockHeight>,
) -> Result<Presigned> {
    let params = *db.params();
    let recipient = ZcashAddress::try_from_encoded(payee).map_err(|e| anyhow!("payee: {e}"))?;

    let proposal = propose_send_max_transfer::<_, _, _, Infallible>(
        db,
        &params,
        account,
        &[ShieldedPool::Ironwood],
        &zip317::FeeRule::standard(),
        recipient,
        memo,
        MaxSpendMode::Everything,
        ConfirmationsPolicy::default(),
        &LockedInputPolicy::Exclude,
        None,
    )
    .map_err(|e| anyhow!("proposal: {e}"))?;

    let step = proposal.steps().first();
    let fee_zat = step.balance().fee_required().into_u64();
    let value_zat = step
        .transaction_request()
        .payments()
        .values()
        .map(|p| p.amount().map(|a| a.into_u64()).unwrap_or(0))
        .sum();
    let target_height = BlockHeight::from(proposal.min_target_height());

    let mut rng = UnwrapErr(SysRng);
    let pczt = create_pczt_from_proposal::<_, _, Infallible, _, Infallible, _>(
        db,
        &params,
        &SystemClock,
        &mut rng,
        account,
        OvkPolicy::Sender,
        &proposal,
        expiry,
        BundlePadding::DEFAULT,
    )
    .map_err(|e| anyhow!("create pczt: {e}"))?;

    let branch = authorize::branch_of(&pczt)?;
    let expiry_height = BlockHeight::from(*pczt.global().expiry_height());
    let spends = proposal
        .steps()
        .first()
        .shielded_inputs()
        .map(|i| i.notes().len())
        .unwrap_or(0);
    let tx = authorize::authorize(pczt, &[usk], spends)?;
    let raw = authorize::serialize(&tx)?;
    Ok(Presigned {
        txid: tx.txid(),
        raw,
        branch,
        target_height,
        expiry_height,
        fee_zat,
        value_zat,
    })
}
