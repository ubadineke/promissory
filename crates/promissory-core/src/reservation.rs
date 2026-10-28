//! Proof of reservation: the payer shares a viewing key scoped to one order's account, and the
//! payee checks from the chain that the order's installments are funded, which of them have
//! paid, and whether the payer has swept the rest (a cancel).
//!
//! # Format
//!
//! `promissory-reserve:v1:<base64url(json)>`, no padding. The JSON is [`Proof`]. Its `ufvk` is a
//! ZIP 316 unified full viewing key with exactly one item, the Orchard FVK of the order's
//! account (Ironwood shares Orchard keys since NU6.3). It covers that account only: nothing
//! about the treasury or any other order.
//!
//! # What the payee learns
//!
//! - The notes the split created in the order account (count and value).
//! - Which of them are still unspent: the installments still reserved.
//! - For each spent note, whether the spend paid the payee. Installments are built with the
//!   order account's outgoing viewing key, so their output to the payee can be recovered.
//! - Cancel sweeps carry no outgoing viewing key (`spend::sweep`), so for those the payee sees
//!   only that the notes were spent, never where the funds went.
//!
//! Verification runs against a separate, view-only wallet database (never the payer's).

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64URL, Engine};
use serde::{Deserialize, Serialize};
use zcash_address::unified::{self, Container as _, Encoding as _};
use zcash_client_backend::{
    data_api::{Account as _, AccountBirthday, AccountPurpose, WalletRead, WalletWrite},
    decrypt_transaction,
    proto::service::TxFilter,
    TransferType,
};
use zcash_client_sqlite::AccountUuid;
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::{
    consensus::{BlockHeight, BranchId, Parameters},
    memo::{Memo, MemoBytes},
    TxId,
};

use crate::{
    keeper::{BlobStatus, Keeper},
    net::{self, Lwd},
    network::SoNetwork,
    order,
    store::Store,
    tlock::{Chain, Drand},
    txbuild,
    wallet::{self, Db, Paths},
};

/// Prefix of the shareable string.
pub const PREFIX: &str = "promissory-reserve:v1:";

/// One installment's blob at the keeper.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofBlob {
    /// 0-based installment index.
    pub k: u32,
    /// drand quicknet round the installment is locked to.
    pub round: u64,
    /// `None` until the installment is armed.
    pub blob_id: Option<String>,
}

/// The decoded contents of a proof of reservation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Proof {
    /// Format version; always 1.
    pub v: u32,
    /// `main` or `regtest`.
    pub net: String,
    /// Orchard-only unified full viewing key of the order's account.
    pub ufvk: String,
    /// Birthday height of the order's account: scanning starts here.
    pub birthday: u32,
    pub payee: String,
    pub amount_zat: u64,
    /// Value of each reserved note: `amount_zat` plus the installment's own fee.
    pub note_zat: u64,
    pub count: u32,
    pub every_secs: u64,
    pub first_due: u64,
    pub memo: String,
    pub keeper: String,
    pub blobs: Vec<ProofBlob>,
}

fn net_name(params: &SoNetwork) -> &'static str {
    match params {
        SoNetwork::Main => "main",
        SoNetwork::Regtest(_) => "regtest",
    }
}

impl Proof {
    /// `promissory-reserve:v1:<base64url(json)>`.
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("proof serializes");
        format!("{PREFIX}{}", B64URL.encode(json))
    }

    /// Parses the shareable string. Surrounding whitespace is ignored.
    pub fn decode(s: &str) -> Result<Self> {
        let body = s
            .trim()
            .strip_prefix(PREFIX)
            .ok_or_else(|| anyhow!("not a proof of reservation (expected it to start with {PREFIX})"))?;
        let json = B64URL
            .decode(body.trim_end_matches('='))
            .map_err(|_| anyhow!("proof of reservation is not valid base64url"))?;
        let p: Proof = serde_json::from_slice(&json).context("proof of reservation JSON")?;
        if p.v != 1 {
            bail!("unsupported proof of reservation version {}", p.v);
        }
        if p.count == 0 || p.blobs.len() > p.count as usize {
            bail!("proof of reservation has inconsistent installment counts");
        }
        Ok(p)
    }
}

/// The Orchard-only UFVK for `account`: the one item a payee needs, and nothing else.
pub fn orchard_only_ufvk(params: &SoNetwork, db: &Db, account: AccountUuid) -> Result<String> {
    let acct = db
        .get_account(account)?
        .ok_or_else(|| anyhow!("unknown order account"))?;
    let fvk = acct
        .ufvk()
        .and_then(|k| k.orchard())
        .ok_or_else(|| anyhow!("order account has no Orchard-protocol viewing key"))?;
    let ufvk = unified::Ufvk::try_from_items(
        unified::Revision::R0,
        vec![unified::Uitem::Data(unified::Fvk::Orchard(fvk.to_bytes()))],
    )
    .map_err(|e| anyhow!("building viewing key: {e}"))?;
    // Round-trip through zcash_keys so we only ever hand out a key it accepts.
    let encoded = ufvk.encode(&params.network_type());
    UnifiedFullViewingKey::decode(params, &encoded).map_err(|e| anyhow!("viewing key: {e}"))?;
    Ok(encoded)
}

/// Builds the proof of reservation for order `id`.
pub fn export(params: &SoNetwork, db: &Db, store: &Store, id: i64) -> Result<Proof> {
    let o = store.order(id)?;
    let account = wallet::account_by_uuid(&o.account)?;
    if account == wallet::account(db)? {
        bail!("order {id} has no reserved account of its own");
    }
    let birthday = db.get_account_birthday(account)?;
    let blobs = store
        .installments(id)?
        .into_iter()
        .map(|i| ProofBlob {
            k: i.k,
            round: i.round,
            blob_id: i.blob_id,
        })
        .collect();
    Ok(Proof {
        v: 1,
        net: net_name(params).into(),
        ufvk: orchard_only_ufvk(params, db, account)?,
        birthday: birthday.into(),
        payee: o.payee,
        amount_zat: o.amount_zat,
        note_zat: o.amount_zat + order::installment_fee().into_u64(),
        count: o.count,
        every_secs: o.every_secs,
        first_due: o.first_due,
        memo: o.memo,
        keeper: o.keeper,
        blobs,
    })
}

/// A note in the order account, as the payee sees it.
#[derive(Clone, Debug, Serialize)]
pub struct NoteView {
    /// The transaction that created the note (the split).
    pub txid: String,
    pub action_index: u32,
    pub value_zat: u64,
    pub received_height: Option<u32>,
    /// `reserved` (unspent), `paid` (spent to the payee), `swept` (spent without paying the
    /// payee: a cancel) or `spent_unknown` (spent, but the transaction could not be fetched).
    pub state: &'static str,
    pub spent_txid: Option<String>,
    pub spent_height: Option<u32>,
    /// What the spend paid the payee, when it did.
    pub paid_zat: Option<u64>,
    /// The memo of the payment to the payee, when it did.
    pub paid_memo: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BlobView {
    pub k: Option<u32>,
    pub blob_id: String,
    pub round: Option<u64>,
    pub release_unix: Option<u64>,
    pub state: Option<String>,
    pub txid: Option<String>,
    pub mined_height: Option<u32>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Terms {
    pub payee: String,
    pub amount_zat: u64,
    pub note_zat: u64,
    pub count: u32,
    pub every_secs: u64,
    pub first_due: u64,
    pub memo: String,
    pub keeper: String,
}

/// What a payee learns from a proof of reservation.
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub net: String,
    pub terms: Terms,
    pub tip_height: u32,
    pub synced_height: u32,
    /// Notes the order account ever received.
    pub funded: u32,
    pub funded_zat: u64,
    /// `funded == count` and every note is worth exactly `note_zat`.
    pub matches_terms: bool,
    /// Unspent notes: installments still reserved.
    pub reserved: u32,
    pub reserved_zat: u64,
    pub paid: u32,
    pub paid_zat: u64,
    /// Notes spent without paying the payee: the payer's cancel sweep.
    pub swept: u32,
    pub swept_zat: u64,
    pub spent_unknown: u32,
    /// `swept > 0`.
    pub cancelled_by_payer: bool,
    /// Earliest release among installments the keeper still holds locked.
    pub next_release_unix: Option<u64>,
    pub notes: Vec<NoteView>,
    pub keeper: Vec<BlobView>,
    pub warnings: Vec<String>,
}

async fn quicknet() -> Option<Chain> {
    crate::tlock::bounded(|| Drand::quicknet()?.chain()).await.ok()
}

/// Keeper status of each blob; needs no keys. `rounds` gives `(k, round)` per blob when known.
pub async fn keeper_status(keeper_url: &str, blobs: &[(Option<u32>, Option<u64>, String)]) -> Vec<BlobView> {
    let keeper = Keeper::new(keeper_url);
    let chain = quicknet().await;
    let mut out = vec![];
    for (k, round, blob_id) in blobs {
        let (status, error): (Option<BlobStatus>, Option<String>) = match keeper.status(blob_id).await {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let round = status.as_ref().map(|s| s.round).or(*round);
        out.push(BlobView {
            k: *k,
            blob_id: blob_id.clone(),
            round,
            release_unix: round.and_then(|r| chain.as_ref().map(|c| c.round_time(r))),
            state: status.as_ref().map(|s| s.state.clone()),
            txid: status.as_ref().and_then(|s| s.txid.clone()),
            mined_height: status.as_ref().and_then(|s| s.mined_height),
            error,
        });
    }
    out
}

/// Fetches a mined (or mempool) transaction in full.
async fn fetch_tx(params: &SoNetwork, lwd: &mut Lwd, txid: &TxId, tip: BlockHeight) -> Result<(Transaction, Option<BlockHeight>)> {
    let raw = lwd
        .get_transaction(TxFilter {
            block: None,
            index: 0,
            hash: txid.as_ref().to_vec(),
        })
        .await
        .map_err(|s| anyhow!("GetTransaction {txid}: {}", s.message()))?
        .into_inner();
    let mined = match raw.height {
        0 | u64::MAX => None,
        h => Some(BlockHeight::from_u32(u32::try_from(h)?)),
    };
    let branch = BranchId::for_height(params, mined.unwrap_or(tip + 1));
    let tx = Transaction::read(&raw.data[..], branch).context("parsing transaction")?;
    Ok((tx, mined))
}

/// What a spend of the order account paid the payee: `(zat, memo)` of the Ironwood outputs
/// to `payee` that the account's outgoing viewing key recovers.
fn paid_to_payee(
    params: &SoNetwork,
    tx: &Transaction,
    mined: Option<BlockHeight>,
    ufvk: &UnifiedFullViewingKey,
    payee: &orchard::Address,
) -> Option<(u64, Option<String>)> {
    let ufvks = HashMap::from([((), ufvk.clone())]);
    let d = decrypt_transaction(params, mined, None, tx, &ufvks);
    let payee = payee.to_raw_address_bytes();
    let mut total = 0;
    let mut memo = None;
    for out in d.ironwood_outputs().iter().chain(d.orchard_outputs()) {
        if out.transfer_type() == TransferType::Outgoing && out.note().0.recipient().to_raw_address_bytes() == payee {
            total += out.note().0.value().inner();
            if memo.is_none() {
                memo = memo_text(out.memo());
            }
        }
    }
    (total > 0).then_some((total, memo))
}

fn memo_text(m: &MemoBytes) -> Option<String> {
    match Memo::try_from(m.clone()) {
        Ok(Memo::Text(t)) => Some(t.to_string()),
        _ => None,
    }
}

struct RawNote {
    txid: TxId,
    action_index: u32,
    value: u64,
    received: Option<u32>,
    spends: Vec<(TxId, Option<u32>)>,
}

/// Every Ironwood note the view-only `account` has received, with the transactions that spend
/// it. Read straight from the payee-side wallet database (zcash_client_sqlite schema).
fn received_notes(db_path: &Path, account: AccountUuid) -> Result<Vec<RawNote>> {
    let conn = rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(
        "SELECT n.id, t.txid, n.action_index, n.value, t.mined_height, st.txid, st.mined_height
         FROM ironwood_received_notes n
         JOIN accounts a ON a.id = n.account_id
         JOIN transactions t ON t.id_tx = n.transaction_id
         LEFT JOIN ironwood_received_note_spends s ON s.ironwood_received_note_id = n.id
         LEFT JOIN transactions st ON st.id_tx = s.transaction_id
         WHERE a.uuid = ?1
         ORDER BY n.commitment_tree_position, n.id",
    )?;
    let txid = |b: Vec<u8>| -> rusqlite::Result<TxId> {
        let bytes: [u8; 32] = b
            .try_into()
            .map_err(|_| rusqlite::Error::InvalidColumnType(0, "txid".into(), rusqlite::types::Type::Blob))?;
        Ok(TxId::from_bytes(bytes))
    };
    let mut notes: Vec<(i64, RawNote)> = vec![];
    let mut rows = stmt.query([account.expose_uuid().as_bytes().to_vec()])?;
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let spend = match r.get::<_, Option<Vec<u8>>>(5)? {
            Some(b) => Some((txid(b)?, r.get::<_, Option<u32>>(6)?)),
            None => None,
        };
        if let Some((last_id, n)) = notes.last_mut() {
            if *last_id == id {
                n.spends.extend(spend);
                continue;
            }
        }
        notes.push((
            id,
            RawNote {
                txid: txid(r.get(1)?)?,
                action_index: r.get(2)?,
                value: r.get::<_, i64>(3)? as u64,
                received: r.get(4)?,
                spends: spend.into_iter().collect(),
            },
        ));
    }
    Ok(notes.into_iter().map(|(_, n)| n).collect())
}

/// Verifies `proof` against the chain, using (and creating if needed) a view-only wallet in
/// `view_dir`. `view_dir` must never be the payer's wallet directory.
pub async fn verify(params: SoNetwork, view_dir: &Path, lwd: &mut Lwd, proof: &Proof) -> Result<Report> {
    if proof.net != net_name(&params) {
        bail!("this proof is for {}, but this app runs on {}", proof.net, net_name(&params));
    }
    let (_, _, container) = unified::Ufvk::decode(&proof.ufvk).map_err(|e| anyhow!("viewing key: {e}"))?;
    if !matches!(container.items_as_parsed(), [unified::Uitem::Data(unified::Fvk::Orchard(_))]) {
        bail!("a proof of reservation must carry an Orchard-only viewing key");
    }
    let ufvk = UnifiedFullViewingKey::decode(&params, &proof.ufvk).map_err(|e| anyhow!("viewing key: {e}"))?;
    let payee = txbuild::orchard_receiver(&params, &proof.payee)?;

    let paths = Paths::new(view_dir);
    if paths.seed().exists() {
        bail!("{} holds a spending wallet; use a separate payee-view directory", view_dir.display());
    }
    fs::create_dir_all(view_dir)?;
    let mut db = wallet::open(&paths, params)?;
    let account = match db.get_account_for_ufvk(&ufvk)? {
        Some(a) => a.id(),
        None => {
            let h = BlockHeight::from_u32(proof.birthday.max(1));
            let treestate = net::tree_state(lwd, h - 1).await?;
            let birthday = AccountBirthday::from_treestate(treestate, None)
                .map_err(|e| anyhow!("birthday: {e:?}"))?;
            db.import_account_ufvk("reservation", &ufvk, &birthday, AccountPurpose::ViewOnly, None)?
                .id()
        }
    };
    wallet::sync(&mut db, lwd).await?;
    let tip = net::tip_height(lwd).await?;
    let synced = db
        .block_fully_scanned()?
        .map(|m| u32::from(m.block_height()))
        .unwrap_or(0);

    let raw = received_notes(&paths.db(), account)?;
    let mut cache: HashMap<TxId, Result<Option<(u64, Option<String>)>, String>> = HashMap::new();
    let mut notes = vec![];
    for n in raw {
        // A note can have several recorded spends (e.g. one mined, one expired); prefer mined.
        let spend = n
            .spends
            .iter()
            .find(|(_, h)| h.is_some())
            .or(n.spends.first())
            .cloned();
        let mut view = NoteView {
            txid: n.txid.to_string(),
            action_index: n.action_index,
            value_zat: n.value,
            received_height: n.received,
            state: "reserved",
            spent_txid: None,
            spent_height: None,
            paid_zat: None,
            paid_memo: None,
        };
        if let Some((stxid, sheight)) = spend {
            if !cache.contains_key(&stxid) {
                let r = match fetch_tx(&params, lwd, &stxid, tip).await {
                    Ok((tx, mined)) => Ok(paid_to_payee(&params, &tx, mined.or(sheight.map(BlockHeight::from_u32)), &ufvk, &payee)),
                    Err(e) => Err(format!("{e:#}")),
                };
                cache.insert(stxid, r);
            }
            view.spent_txid = Some(stxid.to_string());
            view.spent_height = sheight;
            match &cache[&stxid] {
                Ok(Some((zat, memo))) => {
                    view.state = "paid";
                    view.paid_zat = Some(*zat);
                    view.paid_memo = memo.clone();
                }
                Ok(None) => view.state = "swept",
                Err(_) => view.state = "spent_unknown",
            }
        }
        notes.push(view);
    }

    let sum = |state: &str| -> (u32, u64) {
        notes
            .iter()
            .filter(|n| n.state == state)
            .fold((0, 0), |(c, z), n| (c + 1, z + n.value_zat))
    };
    let (reserved, reserved_zat) = sum("reserved");
    let (paid, _) = sum("paid");
    let paid_zat = notes.iter().filter_map(|n| n.paid_zat).sum();
    let (swept, swept_zat) = sum("swept");
    let (spent_unknown, _) = sum("spent_unknown");
    let funded = notes.len() as u32;
    let funded_zat = notes.iter().map(|n| n.value_zat).sum();
    let matches_terms = funded == proof.count && notes.iter().all(|n| n.value_zat == proof.note_zat);

    let mut warnings = vec![];
    if synced < u32::from(tip) {
        warnings.push(format!("scanned to {synced} of {tip}; results may be incomplete"));
    }
    if funded < proof.count {
        warnings.push(format!("only {funded} of {} installments are funded", proof.count));
    }
    if notes.iter().any(|n| n.value_zat != proof.note_zat) {
        warnings.push(format!("some notes are not worth {} zat, the installment amount plus its fee", proof.note_zat));
    }
    if swept > 0 {
        warnings.push(format!(
            "{swept} reserved note{} spent without paying you: the payer cancelled",
            if swept == 1 { " was" } else { "s were" }
        ));
    }

    let blobs: Vec<(Option<u32>, Option<u64>, String)> = proof
        .blobs
        .iter()
        .filter_map(|b| b.blob_id.clone().map(|id| (Some(b.k), Some(b.round), id)))
        .collect();
    let keeper = keeper_status(&proof.keeper, &blobs).await;
    let next_release_unix = keeper
        .iter()
        .filter(|b| b.state.as_deref() == Some("locked"))
        .filter_map(|b| b.release_unix)
        .min();

    Ok(Report {
        net: proof.net.clone(),
        terms: Terms {
            payee: proof.payee.clone(),
            amount_zat: proof.amount_zat,
            note_zat: proof.note_zat,
            count: proof.count,
            every_secs: proof.every_secs,
            first_due: proof.first_due,
            memo: proof.memo.clone(),
            keeper: proof.keeper.clone(),
        },
        tip_height: tip.into(),
        synced_height: synced,
        funded,
        funded_zat,
        matches_terms,
        reserved,
        reserved_zat,
        paid,
        paid_zat,
        swept,
        swept_zat,
        spent_unknown,
        cancelled_by_payer: swept > 0,
        next_release_unix,
        notes,
        keeper,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Proof {
        Proof {
            v: 1,
            net: "regtest".into(),
            ufvk: "uviewregtest1…".into(),
            birthday: 900,
            payee: "uregtest1…".into(),
            amount_zat: 1_000_000,
            note_zat: 1_010_000,
            count: 2,
            every_secs: 180,
            first_due: 1_793_000_000,
            memo: "Pro plan".into(),
            keeper: "http://127.0.0.1:8737".into(),
            blobs: vec![
                ProofBlob { k: 0, round: 1, blob_id: Some("ab".into()) },
                ProofBlob { k: 1, round: 2, blob_id: None },
            ],
        }
    }

    #[test]
    fn round_trip() {
        let p = sample();
        let s = p.encode();
        assert!(s.starts_with(PREFIX));
        assert!(!s.contains('=') && !s.contains('+') && !s.contains('/'));
        assert_eq!(Proof::decode(&format!("  {s}\n")).unwrap(), p);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(Proof::decode("promissory-reserve:v2:abc").is_err());
        assert!(Proof::decode("promissory-reserve:v1:!!!").is_err());
        let mut p = sample();
        p.v = 2;
        assert!(Proof::decode(&p.encode()).is_err());
        let mut p = sample();
        p.count = 1;
        assert!(Proof::decode(&p.encode()).is_err());
    }

    #[test]
    fn orchard_only_key() {
        use zcash_keys::keys::UnifiedSpendingKey;
        let params = SoNetwork::regtest();
        let usk = UnifiedSpendingKey::from_seed(&params, &[7; 32], zip32::AccountId::ZERO).unwrap();
        let full = usk.to_unified_full_viewing_key();
        let fvk = full.orchard().unwrap();
        let ufvk = unified::Ufvk::try_from_items(
            unified::Revision::R0,
            vec![unified::Uitem::Data(unified::Fvk::Orchard(fvk.to_bytes()))],
        )
        .unwrap();
        let k = UnifiedFullViewingKey::decode(&params, &ufvk.encode(&params.network_type())).unwrap();
        assert!(k.sapling().is_none());
        assert_eq!(k.orchard().unwrap().to_bytes(), fvk.to_bytes());
    }
}
