//! Receipts: a per-payment proof the payer can show anyone. "I paid X ZEC to address Y in
//! transaction T", without revealing any other payment.
//!
//! # How it works
//!
//! Every installment is built with the order account's outgoing viewing key (OVK), so each
//! output carries an `out_ciphertext` that the sender can open. Opening it does not use the OVK
//! directly: Orchard derives a one-output key, the outgoing cipher key,
//!
//! ```text
//! ock = PRF^ock_Orchard(ovk, cv_net, cmx, epk)        (protocol spec § 4.20.3, 5.4.2)
//! ```
//!
//! from the OVK and that action's public value commitment, note commitment and ephemeral key.
//! The receipt carries that `ock` and nothing else secret. With it, a verifier:
//!
//! 1. decrypts the action's `out_ciphertext` to `(pk_d, esk)`,
//! 2. decrypts `enc_ciphertext` with the shared secret from `esk` and `pk_d`,
//! 3. checks `epk = [esk] g_d` and that the recovered note's commitment equals the on-chain
//!    `cmx` ([`zcash_note_encryption::try_output_recovery_with_ock`] does 1 to 3),
//! 4. checks the recovered recipient equals the claimed payee's Orchard receiver and the value
//!    equals the claimed value.
//!
//! The note commitment binds value, recipient and randomness, so a receipt cannot be made to
//! open to a different payee or amount. The OCK is a PRF output: it reveals nothing about the
//! OVK, and it opens only this one output (every other output has its own `cv_net`, `cmx` and
//! `epk`). The OVK itself would open every payment the account ever made, so it is never put
//! in a receipt.
//!
//! What a receipt does not prove: who sent the funds. Only the sender's wallet can derive the
//! OCK (the recipient learns the note but not `esk`), so producing one shows sender-side
//! knowledge; but a receipt is a bearer string, and anyone it was shown to can show it again.
//! ZIP 311 (payment disclosures) adds a spend-authorization signature over a message for that;
//! see `docs/integrating.md`.
//!
//! # Format
//!
//! `promissory-receipt:v1:<base64url(json)>`, no padding. The JSON is [`Receipt`].

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64URL, Engine};
use orchard::{
    keys::OutgoingViewingKey,
    note_encryption::{IronwoodDomain, OrchardDomain},
    Action,
};
use serde::{Deserialize, Serialize};
use zcash_client_backend::proto::service::TxFilter;
use zcash_note_encryption::{try_output_recovery_with_ock, Domain, EphemeralKeyBytes, OutgoingCipherKey};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::{
    consensus::{BlockHeight, BranchId, NetworkType, Parameters},
    memo::{Memo, MemoBytes},
    TxId,
};

use crate::{
    net::{self, Lwd},
    network::Params,
    store::Store,
    txbuild,
    wallet::{self, Db, KeyStore},
};

/// Prefix of the shareable string.
pub const PREFIX: &str = "promissory-receipt:v1:";

/// The decoded contents of a receipt.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Receipt {
    /// Format version; always 1.
    pub v: u32,
    /// `main`, `test` or `regtest`.
    pub net: String,
    /// Transaction ID, in the usual (byte-reversed) hex.
    pub txid: String,
    /// `ironwood` (installments) or `orchard`.
    pub pool: String,
    /// Index of the action in that pool's bundle.
    pub action_index: u32,
    /// Outgoing cipher key for that one action, hex.
    pub ock: String,
    /// Claimed payee: a unified address with an Orchard receiver.
    pub payee: String,
    /// Claimed value in zatoshis.
    pub value_zat: u64,
}

/// What a verified receipt shows.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Verified {
    pub txid: String,
    pub pool: String,
    pub action_index: u32,
    pub value_zat: u64,
    pub payee: String,
    /// The memo as text, if it is text.
    pub memo: Option<String>,
    /// Height of the block that mined the transaction.
    pub height: u32,
}

fn net_name(n: NetworkType) -> &'static str {
    match n {
        NetworkType::Main => "main",
        NetworkType::Test => "test",
        NetworkType::Regtest => "regtest",
    }
}

impl Receipt {
    /// `promissory-receipt:v1:<base64url(json)>`.
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("receipt serializes");
        format!("{PREFIX}{}", B64URL.encode(json))
    }

    /// Parses the shareable string. Surrounding whitespace is ignored. Checks the shape only;
    /// [`verify`] checks it against the chain.
    pub fn decode(s: &str) -> Result<Self> {
        let body = s
            .trim()
            .strip_prefix(PREFIX)
            .ok_or_else(|| anyhow!("not a receipt (expected it to start with {PREFIX})"))?;
        let json = B64URL
            .decode(body.trim_end_matches('='))
            .map_err(|_| anyhow!("receipt is not valid base64url"))?;
        let r: Receipt = serde_json::from_slice(&json).context("receipt JSON")?;
        if r.v != 1 {
            bail!("unsupported receipt version {}", r.v);
        }
        if !matches!(r.pool.as_str(), "ironwood" | "orchard") {
            bail!("unsupported receipt pool {}", r.pool);
        }
        parse_txid(&r.txid)?;
        parse_ock(&r.ock)?;
        Ok(r)
    }
}

fn parse_txid(s: &str) -> Result<TxId> {
    let mut bytes: [u8; 32] = hex::decode(s)
        .map_err(|_| anyhow!("receipt txid is not hex"))?
        .try_into()
        .map_err(|_| anyhow!("receipt txid must be 32 bytes"))?;
    bytes.reverse();
    Ok(TxId::from_bytes(bytes))
}

fn parse_ock(s: &str) -> Result<OutgoingCipherKey> {
    let bytes: [u8; 32] = hex::decode(s)
        .map_err(|_| anyhow!("receipt key is not hex"))?
        .try_into()
        .map_err(|_| anyhow!("receipt key must be 32 bytes"))?;
    Ok(OutgoingCipherKey(bytes))
}

/// An opened output: `(value, recipient, memo)`.
pub struct Opened {
    pub value_zat: u64,
    pub recipient: orchard::Address,
    pub memo: [u8; 512],
}

/// The outgoing cipher key of `action` under `ovk`.
pub fn ock_for<T>(ovk: &OutgoingViewingKey, action: &Action<T>) -> OutgoingCipherKey {
    // The PRF is the same for both pools; the domain only selects the note plaintext version.
    IronwoodDomain::derive_ock(
        ovk,
        action.cv_net(),
        &action.cmx().to_bytes(),
        &EphemeralKeyBytes(action.encrypted_note().epk_bytes),
    )
}

/// Opens `action` with an outgoing cipher key, checking the recovered note against the
/// action's note commitment. `None` if the key does not open it.
pub fn open<T>(pool: &str, action: &Action<T>, ock: &OutgoingCipherKey) -> Option<Opened> {
    let out = &action.encrypted_note().out_ciphertext;
    let (note, recipient, memo) = match pool {
        "ironwood" => try_output_recovery_with_ock(&IronwoodDomain::for_action(action), ock, action, out)?,
        "orchard" => try_output_recovery_with_ock(&OrchardDomain::for_action(action), ock, action, out)?,
        _ => return None,
    };
    Some(Opened {
        value_zat: note.value().inner(),
        recipient,
        memo,
    })
}

/// The first action in `actions` that `ovk` opens to `payee` for `value_zat`, with its key.
pub fn find<'a, T: 'a>(
    pool: &str,
    actions: impl IntoIterator<Item = &'a Action<T>>,
    ovk: &OutgoingViewingKey,
    payee: &orchard::Address,
    value_zat: u64,
) -> Option<(u32, OutgoingCipherKey, Opened)> {
    actions.into_iter().enumerate().find_map(|(i, action)| {
        let ock = ock_for(ovk, action);
        let opened = open(pool, action, &ock)?;
        (opened.recipient.to_raw_address_bytes() == payee.to_raw_address_bytes() && opened.value_zat == value_zat)
            .then_some((i as u32, ock, opened))
    })
}

/// Checks an opened output against a receipt's claims.
pub fn check_claims(opened: &Opened, payee: &orchard::Address, value_zat: u64) -> Result<()> {
    if opened.recipient.to_raw_address_bytes() != payee.to_raw_address_bytes() {
        bail!("the payment went to a different address than the receipt claims");
    }
    if opened.value_zat != value_zat {
        bail!(
            "the payment was {} zat, not the {value_zat} zat the receipt claims",
            opened.value_zat
        );
    }
    Ok(())
}

fn memo_text(memo: &[u8; 512]) -> Option<String> {
    let bytes = MemoBytes::from_bytes(memo).ok()?;
    match Memo::try_from(bytes).ok()? {
        Memo::Text(t) => Some(t.to_string()),
        _ => None,
    }
}

/// Fetches a transaction in full, with its mined height (`None` in the mempool).
async fn fetch_tx<P: Parameters>(params: &P, lwd: &mut Lwd, txid: &TxId) -> Result<Option<(Transaction, Option<BlockHeight>)>> {
    let raw = match lwd
        .get_transaction(TxFilter {
            block: None,
            index: 0,
            hash: txid.as_ref().to_vec(),
        })
        .await
    {
        Ok(r) => r.into_inner(),
        Err(s) if s.code() == tonic::Code::NotFound || s.message().contains("No such mempool or") => return Ok(None),
        Err(s) => bail!("GetTransaction {txid}: {}", s.message()),
    };
    let mined = match raw.height {
        0 | u64::MAX => None,
        h => Some(BlockHeight::from_u32(u32::try_from(h)?)),
    };
    let tip = net::tip_height(lwd).await?;
    let branch = BranchId::for_height(params, mined.unwrap_or(tip + 1));
    let tx = Transaction::read(&raw.data[..], branch).context("parsing transaction")?;
    Ok(Some((tx, mined)))
}

type TxBundle = orchard::Bundle<orchard::bundle::Authorized, zcash_protocol::value::ZatBalance>;

fn bundle_of<'a>(tx: &'a Transaction, pool: &str) -> Option<&'a TxBundle> {
    match pool {
        "ironwood" => tx.ironwood_bundle(),
        "orchard" => tx.orchard_bundle(),
        _ => None,
    }
}

/// What [`make`] needs from the wallet, read up front so no wallet handle is held across
/// network calls.
pub struct Pending {
    net: &'static str,
    order_id: i64,
    k: u32,
    payee_ua: String,
    payee: orchard::Address,
    value_zat: u64,
    ovk: OutgoingViewingKey,
    /// The mined copy is one of these: the fresh copy first, then the fallback copies.
    candidates: Vec<String>,
}

/// Reads what a receipt for installment `k` of `order_id` needs: the order's terms and its
/// account's outgoing viewing key. No sync and no writes.
pub fn prepare<P: Params>(keys: &impl KeyStore, db: &Db<P>, store: &Store, order_id: i64, k: u32) -> Result<Pending> {
    let params = *db.params();
    let order = store.order(order_id)?;
    let inst = store
        .installments(order_id)?
        .into_iter()
        .find(|i| i.k == k)
        .ok_or_else(|| anyhow!("order {order_id} has no payment {}", k + 1))?;
    if inst.state != "paid" && inst.keeper_state.as_deref() != Some("mined") {
        bail!("payment {} of order {order_id} has not been paid (it is {})", k + 1, inst.state);
    }
    let payee = txbuild::orchard_receiver(&params, &order.payee)?;
    let account = wallet::account_by_uuid(&order.account)?;
    let ovk = wallet::usk(keys, db, account)?
        .to_unified_full_viewing_key()
        .orchard()
        .ok_or_else(|| anyhow!("order account has no Orchard-protocol key"))?
        .to_ovk(orchard::keys::Scope::External);
    let mut candidates: Vec<String> = inst.fresh_txid.iter().cloned().collect();
    candidates.extend(
        inst.txids
            .as_deref()
            .unwrap_or("")
            .split(',')
            .filter_map(|c| c.split_once(':').map(|(_, t)| t.to_string())),
    );
    Ok(Pending {
        net: net_name(params.network_type()),
        order_id,
        k,
        payee_ua: order.payee,
        payee,
        value_zat: order.amount_zat,
        ovk,
        candidates,
    })
}

impl Pending {
    /// Finds the mined copy and the action that pays the payee, and derives its key.
    pub async fn finish<P: Parameters>(self, params: &P, lwd: &mut Lwd) -> Result<Receipt> {
        for txid in &self.candidates {
            let Some((tx, Some(_))) = fetch_tx(params, lwd, &parse_txid(txid)?).await? else {
                continue;
            };
            for pool in ["ironwood", "orchard"] {
                let Some(bundle) = bundle_of(&tx, pool) else { continue };
                if let Some((i, ock, _)) = find(pool, bundle.actions().iter(), &self.ovk, &self.payee, self.value_zat) {
                    return Ok(Receipt {
                        v: 1,
                        net: self.net.into(),
                        txid: txid.clone(),
                        pool: pool.into(),
                        action_index: i,
                        ock: hex::encode(ock.0),
                        payee: self.payee_ua.clone(),
                        value_zat: self.value_zat,
                    });
                }
            }
        }
        bail!("could not find payment {} of order {} on chain", self.k + 1, self.order_id)
    }
}

/// Makes a receipt for installment `k` of order `order_id`, which must have paid. Reads the
/// store and the wallet's keys only (no sync, no writes), and fetches the mined transaction
/// from lightwalletd. See [`prepare`] to release the wallet before the network calls.
pub async fn make<P: Params>(
    keys: &impl KeyStore,
    db: &Db<P>,
    store: &Store,
    lwd: &mut Lwd,
    order_id: i64,
    k: u32,
) -> Result<Receipt> {
    let params = *db.params();
    prepare(keys, db, store, order_id, k)?.finish(&params, lwd).await
}

/// Checks a receipt against the chain: the transaction is mined, the key opens the named
/// action, and the note it holds pays the claimed payee the claimed value.
pub async fn verify<P: Parameters>(lwd: &mut Lwd, params: &P, receipt: &Receipt) -> Result<Verified> {
    let net = net_name(params.network_type());
    if receipt.net != net {
        bail!("this receipt is for {}, but this app runs on {net}", receipt.net);
    }
    let payee = txbuild::orchard_receiver(params, &receipt.payee).context("receipt payee")?;
    let txid = parse_txid(&receipt.txid)?;
    let ock = parse_ock(&receipt.ock)?;
    let (tx, mined) = fetch_tx(params, lwd, &txid)
        .await?
        .ok_or_else(|| anyhow!("transaction {} is not on chain", receipt.txid))?;
    let height = mined.ok_or_else(|| anyhow!("transaction {} is not mined yet", receipt.txid))?;
    let bundle = bundle_of(&tx, &receipt.pool)
        .ok_or_else(|| anyhow!("transaction {} has no {} bundle", receipt.txid, receipt.pool))?;
    let action = bundle
        .actions()
        .get(receipt.action_index as usize)
        .ok_or_else(|| anyhow!("transaction {} has no action {}", receipt.txid, receipt.action_index))?;
    let opened = open(&receipt.pool, action, &ock)
        .ok_or_else(|| anyhow!("the receipt's key does not open action {} of {}", receipt.action_index, receipt.txid))?;
    check_claims(&opened, &payee, receipt.value_zat)?;
    Ok(Verified {
        txid: receipt.txid.clone(),
        pool: receipt.pool.clone(),
        action_index: receipt.action_index,
        value_zat: opened.value_zat,
        payee: receipt.payee.clone(),
        memo: memo_text(&opened.memo),
        height: u32::from(height),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchard::{
        builder::{Builder, BundleType},
        bundle::BundleVersion,
        keys::{FullViewingKey, Scope, SpendingKey},
        value::NoteValue,
        Anchor,
    };
    use rand::{rand_core::UnwrapErr, rngs::SysRng};

    fn key(b: u8) -> FullViewingKey {
        FullViewingKey::from(&SpendingKey::from_bytes([b; 32]).unwrap())
    }

    /// An Ironwood bundle (unproven; encryption is what matters) with two outputs from the
    /// sender: one to the payee, one to someone else.
    fn bundle() -> (Vec<Action<()>>, OutgoingViewingKey, orchard::Address, orchard::Address) {
        let sender = key(1);
        let ovk = sender.to_ovk(Scope::External);
        let payee = key(2).address_at(0u32, Scope::External);
        let other = key(3).address_at(0u32, Scope::External);
        let version = BundleVersion::ironwood_v3();
        let mut b = Builder::new(BundleType::DEFAULT, version, version.default_flags(), Anchor::empty_tree()).unwrap();
        let mut memo = [0u8; 512];
        memo[..9].copy_from_slice(b"Rent (1/1");
        b.add_output(Some(ovk.clone()), payee, NoteValue::from_raw(1_000_000), memo).unwrap();
        b.add_output(Some(ovk.clone()), other, NoteValue::from_raw(7_000_000), [0xf6; 512]).unwrap();
        let (bundle, _) = b.build::<i64>(UnwrapErr(SysRng)).unwrap().unwrap();
        let actions = bundle.actions().iter().map(|a| a.clone().map(|_| ())).collect();
        (actions, ovk, payee, other)
    }

    #[test]
    fn opens_only_its_own_output() {
        let (actions, ovk, payee, other) = bundle();
        let (i, ock, opened) = find("ironwood", &actions, &ovk, &payee, 1_000_000).expect("payee output");
        assert_eq!(opened.value_zat, 1_000_000);
        assert_eq!(memo_text(&opened.memo).as_deref(), Some("Rent (1/1"));
        check_claims(&opened, &payee, 1_000_000).unwrap();
        // The same key opens no other action: not the other payment, not the padding.
        for (j, a) in actions.iter().enumerate() {
            assert_eq!(open("ironwood", a, &ock).is_some(), j as u32 == i, "action {j}");
        }
        // The other output has its own key.
        let (j, ock2, _) = find("ironwood", &actions, &ovk, &other, 7_000_000).unwrap();
        assert_ne!(i, j);
        assert_ne!(ock.0, ock2.0);
        // Wrong pool domain (note plaintext version) does not open it either.
        assert!(open("orchard", &actions[i as usize], &ock).is_none());
    }

    #[test]
    fn tampered_claims_fail() {
        let (actions, ovk, payee, other) = bundle();
        let (i, mut ock, opened) = find("ironwood", &actions, &ovk, &payee, 1_000_000).unwrap();
        assert!(check_claims(&opened, &other, 1_000_000).unwrap_err().to_string().contains("different address"));
        assert!(check_claims(&opened, &payee, 2_000_000).unwrap_err().to_string().contains("not the 2000000"));
        ock.0[0] ^= 1;
        assert!(open("ironwood", &actions[i as usize], &ock).is_none(), "a flipped key bit opens nothing");
    }

    fn sample() -> Receipt {
        Receipt {
            v: 1,
            net: "regtest".into(),
            txid: "ab".repeat(32),
            pool: "ironwood".into(),
            action_index: 1,
            ock: "cd".repeat(32),
            payee: "uregtest1…".into(),
            value_zat: 1_000_000,
        }
    }

    #[test]
    fn round_trip() {
        let r = sample();
        let s = r.encode();
        assert!(s.starts_with(PREFIX));
        assert!(!s.contains('=') && !s.contains('+') && !s.contains('/'));
        assert_eq!(Receipt::decode(&format!(" {s}\n")).unwrap(), r);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(Receipt::decode("promissory-receipt:v2:abc").is_err());
        assert!(Receipt::decode("promissory-receipt:v1:!!!").is_err());
        for f in [
            |r: &mut Receipt| r.v = 2,
            |r: &mut Receipt| r.pool = "sapling".into(),
            |r: &mut Receipt| r.txid = "ab".into(),
            |r: &mut Receipt| r.ock = "zz".repeat(32),
        ] {
            let mut r = sample();
            f(&mut r);
            assert!(Receipt::decode(&r.encode()).is_err());
        }
    }
}
