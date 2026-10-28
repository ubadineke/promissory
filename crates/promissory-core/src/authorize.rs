//! Turns a PCZT into a fully authorized transaction: sign every Orchard/Ironwood spend this
//! key controls, create the proofs, and extract.

use std::sync::OnceLock;

use anyhow::{anyhow, bail, Result};
use orchard::circuit::{OrchardCircuitVersion, VerifyingKey};
use pczt::{
    roles::{prover::Prover, signer::Signer, tx_extractor::TransactionExtractor},
    Pczt,
};
use rand::{rand_core::UnwrapErr, rngs::SysRng};
use zcash_keys::keys::UnifiedSpendingKey;
use zcash_primitives::transaction::{
    builder::cached_orchard_proving_key, components::orchard::bundle_version_for_branch,
    Transaction,
};
use zcash_protocol::consensus::BranchId;

pub fn branch_of(pczt: &Pczt) -> Result<BranchId> {
    BranchId::try_from(*pczt.global().consensus_branch_id())
        .map_err(|e| anyhow!("pczt branch: {e}"))
}

/// Signs every spend the keys control and creates the proofs. `expected_spends` is the number
/// of real spends the caller put in the PCZT; dummy actions were already signed by the IO
/// finalizer with throwaway keys, so our keys fail on them and they are skipped.
pub fn sign_and_prove(pczt: Pczt, usks: &[&UnifiedSpendingKey], expected_spends: usize) -> Result<Pczt> {
    let branch = branch_of(&pczt)?;
    let n_orchard = pczt.orchard().actions().len();
    let n_ironwood = pczt.ironwood().actions().len();

    let asks: Vec<_> = usks
        .iter()
        .map(|usk| orchard::keys::SpendAuthorizingKey::from(usk.orchard()))
        .collect();
    let mut signer = Signer::new(pczt).map_err(|e| anyhow!("signer: {e:?}"))?;
    let mut signed = 0;
    for index in 0..n_ironwood {
        if asks
            .iter()
            .any(|ask| signer.sign_ironwood(UnwrapErr(SysRng), index, ask).is_ok())
        {
            signed += 1;
        }
    }
    for index in 0..n_orchard {
        if asks
            .iter()
            .any(|ask| signer.sign_orchard(UnwrapErr(SysRng), index, ask).is_ok())
        {
            signed += 1;
        }
    }
    if signed != expected_spends {
        bail!("signed {signed} spends, expected {expected_spends}");
    }

    let mut prover = Prover::new(signer.finish());
    for pool in [orchard::ValuePool::Ironwood, orchard::ValuePool::Orchard] {
        let needed = match pool {
            orchard::ValuePool::Ironwood => prover.requires_ironwood_proof(),
            orchard::ValuePool::Orchard => prover.requires_orchard_proof(),
        };
        if !needed {
            continue;
        }
        let circuit = bundle_version_for_branch(branch, pool)
            .ok_or_else(|| anyhow!("branch {branch:?} has no {pool:?} pool"))?
            .circuit_version();
        let pk = cached_orchard_proving_key(circuit);
        prover = match pool {
            orchard::ValuePool::Ironwood => prover
                .create_ironwood_proof(UnwrapErr(SysRng), pk)
                .map_err(|e| anyhow!("ironwood proof: {e:?}"))?,
            orchard::ValuePool::Orchard => prover
                .create_orchard_proof(UnwrapErr(SysRng), pk)
                .map_err(|e| anyhow!("orchard proof: {e:?}"))?,
        };
    }
    Ok(prover.finish())
}

/// Signs, proves and extracts.
pub fn authorize(pczt: Pczt, usks: &[&UnifiedSpendingKey], expected_spends: usize) -> Result<Transaction> {
    let pczt = sign_and_prove(pczt, usks, expected_spends)?;
    let vk = verifying_key(branch_of(&pczt)?)?;
    TransactionExtractor::new(pczt)
        .with_orchard(vk)
        .extract(UnwrapErr(SysRng))
        .map_err(|e| anyhow!("extract: {e:?}"))
}

/// The Orchard-protocol verifying key for `branch`. Both pools on a branch share one circuit
/// generation, so one key covers them.
pub fn verifying_key(branch: BranchId) -> Result<&'static VerifyingKey> {
    let circuit = bundle_version_for_branch(branch, orchard::ValuePool::Ironwood)
        .or_else(|| bundle_version_for_branch(branch, orchard::ValuePool::Orchard))
        .ok_or_else(|| anyhow!("branch {branch:?} has no Orchard-protocol pool"))?
        .circuit_version();
    Ok(cached_verifying_key(circuit))
}

/// Mirrors `zcash_primitives`' `cached_orchard_proving_key`: building a key takes seconds.
fn cached_verifying_key(circuit: OrchardCircuitVersion) -> &'static VerifyingKey {
    static INSECURE_PRE_NU6_2: OnceLock<VerifyingKey> = OnceLock::new();
    static FIXED_POST_NU6_2: OnceLock<VerifyingKey> = OnceLock::new();
    static POST_NU6_3: OnceLock<VerifyingKey> = OnceLock::new();
    let cell = match circuit {
        OrchardCircuitVersion::InsecurePreNu6_2 => &INSECURE_PRE_NU6_2,
        OrchardCircuitVersion::FixedPostNu6_2 => &FIXED_POST_NU6_2,
        OrchardCircuitVersion::PostNu6_3 => &POST_NU6_3,
    };
    cell.get_or_init(|| VerifyingKey::build(circuit))
}

pub fn serialize(tx: &Transaction) -> Result<Vec<u8>> {
    let mut raw = vec![];
    tx.write(&mut raw)?;
    Ok(raw)
}
