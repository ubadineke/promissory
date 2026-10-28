//! Standing orders: create → fund → split → arm → (keeper releases) → cancel.

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Result};
use rand::{rand_core::UnwrapErr, rngs::SysRng, RngExt};
use zcash_address::ZcashAddress;
use zcash_client_backend::{
    data_api::{
        wallet::{
            create_pczt_from_proposal, extract_and_store_transaction_from_pczt,
            input_selection::{GreedyInputSelector, SpendPolicy},
            propose_transfer, ConfirmationsPolicy,
        },
        WalletRead,
    },
    fees::{zip317::SingleOutputChangeStrategy, DustOutputPolicy, StandardFeeRule},
    util::SystemClock,
    wallet::OvkPolicy,
};
use zcash_client_sqlite::ReceivedNoteId;
use zcash_primitives::transaction::builder::{default_tx_expiry_delta, BundlePadding};
use zcash_protocol::{
    consensus::{BlockHeight, BranchId, NetworkUpgrade, Parameters},
    memo::MemoBytes,
    value::Zatoshis,
    zip318::{self, BlockSpacing},
    ShieldedPool, TxId,
};
use zip321::{Payment, TransactionRequest};

use crate::{
    authorize,
    envelope::Envelope,
    keeper::{BlobStatus, Keeper},
    net::{self, Lwd, SendOutcome},
    spend::{self, IronwoodNote},
    store::{Installment, Order, Store},
    tlock::{self, Drand},
    txbuild::{self, Output},
    network::{predict_height_spaced, Params},
    spacing,
    wallet::{self, Db, KeyStore},
};

/// Every installment is a 1-in, 1-out Ironwood transaction (2 actions after padding).
pub fn installment_fee() -> Zatoshis {
    txbuild::ironwood_fee(1, 1)
}

/// By default, cancelling must land this long before the earliest round of any copy.
pub const CANCEL_MARGIN_SECS: u64 = 600;

/// An ordinary standing order (`orders.kind`).
pub const KIND_STANDING: &str = "standing";
/// A dead-man's switch: one payment to an heir at a date, renewed while the payer is alive.
/// Its pre-signed copy never expires (expiry height 0), the one place Promissory uses that.
pub const KIND_INHERITANCE: &str = "inheritance";
/// Default reminder before a switch fires: a week.
pub const SWITCH_REMIND_SECS: u64 = 7 * 86_400;

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after 1970")
        .as_secs()
}

/// ZEC amount as a ZIP 321 decimal string (no trailing zeros).
pub fn zec_decimal(zat: u64) -> String {
    let s = format!("{}.{:08}", zat / 100_000_000, zat % 100_000_000);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

pub struct Schedule {
    pub amount: Zatoshis,
    pub memo: String,
    pub every_secs: u64,
    pub count: u32,
    pub first_due: u64,
    /// Each release is delayed by a uniform random 0..=jitter_max_secs.
    pub jitter_max_secs: u64,
}

pub struct Created {
    pub id: i64,
    pub funding_zat: u64,
    pub treasury_address: String,
}

impl Created {
    pub fn funding_uri(&self) -> String {
        format!(
            "zcash:{}?amount={}",
            self.treasury_address,
            zec_decimal(self.funding_zat)
        )
    }
}

/// The amount the treasury must receive to fund `count` installments of `amount`: one note per
/// installment covering its own fee, plus the split transaction's fee (one funding note in,
/// `count` notes out, no change).
pub fn funding_needed(amount: Zatoshis, count: u32) -> u64 {
    let per_note = amount.into_u64() + installment_fee().into_u64();
    u64::from(count) * per_note + txbuild::ironwood_fee(1, count as usize).into_u64()
}

/// Uploads a blob, with the order's webhook if the payer opted in to one. A keeper that refuses
/// webhooks still gets the blob: the payment matters more than the notice.
async fn upload(keeper: &Keeper, order: &Order, ciphertext: &[u8]) -> Result<crate::keeper::Uploaded> {
    if let Some(hook) = &order.hook {
        match crate::notices::upload_with_webhook(&order.keeper, ciphertext, hook).await {
            Ok(up) => return Ok(up),
            Err(e) => tracing::warn!(order = order.id, "uploading without the webhook: {e:#}"),
        }
    }
    keeper.upload(ciphertext).await
}

/// If the store couldn't record an uploaded blob, withdraw it from the keeper before failing.
/// Otherwise the next pass would sign the installment again and leave two copies at the keeper
/// that the payer's records only know one of. (Both pay the same note to the same payee, so this
/// guards the records, not the funds.)
async fn withdraw_unrecorded(keeper: &Keeper, uploaded: &crate::keeper::Uploaded, recorded: Result<()>) -> Result<()> {
    let Err(e) = recorded else { return Ok(()) };
    match keeper.delete(&uploaded.blob_id, &uploaded.delete_token).await {
        Ok(()) => Err(e.context("recording the upload failed; the blob was withdrawn")),
        Err(d) => Err(e.context(format!(
            "recording the upload failed and withdrawing blob {} failed too ({d:#})",
            uploaded.blob_id
        ))),
    }
}

async fn quicknet() -> Result<tlock::Chain> {
    tlock::bounded(|| Drand::quicknet()?.chain()).await
}

/// Records a new order funded in full up front. See [`create_with_window`].
pub async fn create<P: Params>(
    keys: &impl KeyStore,
    db: &mut Db<P>,
    store: &mut Store,
    lwd: &mut Lwd,
    payee: &str,
    schedule: &Schedule,
    keeper: &str,
) -> Result<Created> {
    create_with_window(keys, db, store, lwd, payee, schedule, keeper, 0).await
}

/// Records a new order, reserves an account for it, and returns the funding request.
///
/// With `window > 0` only that many installments are funded ahead: the first request covers
/// `window` installments, and later ones are split from the treasury as earlier ones are paid
/// ([`next_funding`]), so little is locked up at once.
#[allow(clippy::too_many_arguments)]
pub async fn create_with_window<P: Params>(
    keys: &impl KeyStore,
    db: &mut Db<P>,
    store: &mut Store,
    lwd: &mut Lwd,
    payee: &str,
    schedule: &Schedule,
    keeper: &str,
    window: u32,
) -> Result<Created> {
    let params = *db.params();
    txbuild::orchard_receiver(&params, payee)?;
    if schedule.count == 0 {
        bail!("an order needs at least one installment");
    }
    if schedule.first_due < now_unix() + CANCEL_MARGIN_SECS / 10 {
        bail!("the first installment must be due in the future");
    }

    let chain = quicknet().await?;
    let mut rng = UnwrapErr(SysRng);
    let rounds: Vec<u64> = (0..schedule.count)
        .map(|k| {
            let due = schedule.first_due + schedule.every_secs * u64::from(k);
            let jitter = if schedule.jitter_max_secs == 0 {
                0
            } else {
                rng.random_range(0..=schedule.jitter_max_secs)
            };
            chain.round_at(due + jitter)
        })
        .collect();

    let account = wallet::create_order_account(keys, db, lwd, "order").await?;
    let treasury = wallet::account(db)?;
    let treasury_address = wallet::next_address(db, treasury)?;
    let id = store.insert_order(
        &account.expose_uuid().to_string(),
        payee,
        schedule.amount.into_u64(),
        &schedule.memo,
        schedule.every_secs,
        schedule.count,
        schedule.first_due,
        keeper,
        &treasury_address,
        &rounds,
    )?;
    store.set_window(id, window)?;
    store.set_funded_count(id, 0)?;
    let first = if window == 0 { schedule.count } else { window.min(schedule.count) };
    Ok(Created {
        id,
        funding_zat: funding_needed(schedule.amount, first),
        treasury_address,
    })
}

/// Installments split into notes so far.
fn funded(order: &Order) -> u32 {
    match order.funded_count {
        Some(n) => n,
        // Orders from before rolling windows were funded in full by their one split.
        None if order.status == "awaiting_funds" => 0,
        None => order.count,
    }
}

/// The next chunk of installments to fund and what it costs, if the order needs one now: the
/// whole order up front, or with a rolling window, enough to keep `window` unpaid
/// installments funded.
pub fn next_funding(store: &Store, order: &Order) -> Result<Option<(u32, u64)>> {
    Ok(next_funding_from(order, &store.installments(order.id)?))
}

/// [`next_funding`] given the order's installments.
pub fn next_funding_from(order: &Order, installments: &[Installment]) -> Option<(u32, u64)> {
    if matches!(order.status.as_str(), "cancelled" | "cancelling" | "completed") {
        return None;
    }
    let funded = funded(order);
    let remaining = order.count.saturating_sub(funded);
    let n = if order.window == 0 {
        remaining
    } else {
        let settled = installments
            .iter()
            .filter(|i| i.state == "paid" || i.state == "cancelled")
            .count() as u32;
        let ahead = funded.saturating_sub(settled);
        order.window.saturating_sub(ahead).min(remaining)
    };
    (n > 0).then(|| (n, funding_needed(Zatoshis::const_from_u64(order.amount_zat), n)))
}

/// Splits treasury funds into one exact note per installment of the next funding chunk
/// ([`next_funding`]), in the order's account.
pub async fn split<P: Params>(keys: &impl KeyStore, db: &mut Db<P>, store: &mut Store, lwd: &mut Lwd, id: i64) -> Result<TxId> {
    split_many(keys, db, store, lwd, &[id]).await
}

/// [`split`] for several orders in one transaction: one treasury spend funds every order's next
/// chunk. A payroll batch funded by one payment is split at once, for one fee, and the chain
/// sees one transaction instead of one per employee.
pub async fn split_many<P: Params>(
    keys: &impl KeyStore,
    db: &mut Db<P>,
    store: &mut Store,
    lwd: &mut Lwd,
    ids: &[i64],
) -> Result<TxId> {
    let mut plan = vec![];
    for &id in ids {
        let order = store.order(id)?;
        let Some((chunk, _)) = next_funding(store, &order)? else {
            bail!("order {id} is {} and needs no funding now", order.status);
        };
        plan.push((order, chunk));
    }
    if plan.is_empty() {
        bail!("no orders to split");
    }
    let params = *db.params();
    wallet::sync(db, lwd).await?;
    let treasury = wallet::account(db)?;

    let mut payments = vec![];
    for (order, chunk) in &plan {
        let order_account = wallet::account_by_uuid(&order.account)?;
        let per_note = Zatoshis::from_u64(order.amount_zat + installment_fee().into_u64())?;
        for _ in 0..*chunk {
            let addr: ZcashAddress = wallet::next_address(db, order_account)?
                .parse()
                .map_err(|e| anyhow!("order address: {e}"))?;
            payments.push(
                Payment::new(addr, Some(per_note), None, None, None, vec![])
                    .map_err(|e| anyhow!("payment: {e:?}"))?,
            );
        }
    }
    let request = TransactionRequest::new(payments).map_err(|e| anyhow!("request: {e:?}"))?;

    let proposal = propose_transfer::<_, _, _, _, std::convert::Infallible>(
        db,
        &params,
        treasury,
        &GreedyInputSelector::new(),
        &SingleOutputChangeStrategy::new(
            StandardFeeRule::Zip317,
            None,
            ShieldedPool::Ironwood,
            DustOutputPolicy::default(),
        ),
        request,
        ConfirmationsPolicy::default(),
        &SpendPolicy::default(),
        None,
        None,
    )
    .map_err(|e| anyhow!("split proposal (is the treasury funded and confirmed?): {e}"))?;
    let spends = proposal
        .steps()
        .first()
        .shielded_inputs()
        .map(|i| i.notes().len())
        .unwrap_or(0);

    let mut rng = UnwrapErr(SysRng);
    let pczt = create_pczt_from_proposal::<_, _, std::convert::Infallible, _, std::convert::Infallible, _>(
        db,
        &params,
        &SystemClock,
        &mut rng,
        treasury,
        OvkPolicy::Sender,
        &proposal,
        None,
        BundlePadding::DEFAULT,
    )
    .map_err(|e| anyhow!("split pczt: {e}"))?;
    let usk = wallet::usk(keys, db, treasury)?;
    let pczt = authorize::sign_and_prove(pczt, &[&usk], spends)?;
    let vk = authorize::verifying_key(authorize::branch_of(&pczt)?)?;
    let txid = extract_and_store_transaction_from_pczt::<_, ReceivedNoteId>(
        db,
        &SystemClock,
        &mut rng,
        pczt,
        None,
        Some(vk),
    )
    .map_err(|e| anyhow!("split extract: {e}"))?;
    let tx = db
        .get_transaction(txid)?
        .ok_or_else(|| anyhow!("split transaction not stored"))?;
    match net::send(lwd, &authorize::serialize(&tx)?).await? {
        SendOutcome::Accepted => {}
        SendOutcome::Rejected { code, message } => bail!("split rejected ({code}): {message}"),
    }
    for (order, chunk) in &plan {
        store.set_funded_count(order.id, funded(order) + chunk)?;
        if order.status == "awaiting_funds" {
            store.set_split(order.id, &txid.to_string())?;
        }
    }
    Ok(txid)
}

/// The consensus branches an installment needs copies for, each with the target height to
/// build it at: the branch in effect now, and the branch at the predicted mining height if an
/// upgrade activates in between.
fn copy_targets<P: Parameters>(params: &P, target_now: BlockHeight, predicted: BlockHeight) -> Vec<BlockHeight> {
    let now_branch = BranchId::for_height(params, target_now);
    if BranchId::for_height(params, predicted) == now_branch {
        return vec![predicted];
    }
    // The current-branch copy targets the last height before the next upgrade.
    let next_upgrade = [NetworkUpgrade::Nu7]
        .into_iter()
        .filter_map(|nu| params.activation_height(nu))
        .find(|h| *h > target_now && *h <= predicted)
        .expect("branches differ, so an upgrade activates in between");
    vec![next_upgrade - 1, predicted]
}

/// The expiry of a fallback copy built for `target`: the ZIP 318 rolling expiry, or 0 (never)
/// for a dead-man's switch.
pub fn fallback_expiry(inheritance: bool, spacing: &BlockSpacing, target: BlockHeight) -> BlockHeight {
    if inheritance {
        BlockHeight::from_u32(0)
    } else {
        zip318::expiry_height(spacing, target)
    }
}

pub struct Armed {
    pub k: u32,
    pub round: u64,
    pub blob_id: String,
    pub copies: Vec<(BranchId, TxId)>,
    pub expiry: BlockHeight,
}

/// Pre-signs the next reserved installment as a fallback copy, tlocks it to its round, and
/// uploads it to the keeper. One installment per anchor height: call again after the next block
/// for the next one (promissory-app's progression pass does this), so no two installments share an anchor.
pub async fn arm<P: Params>(keys: &impl KeyStore, db: &mut Db<P>, store: &mut Store, lwd: &mut Lwd, id: i64) -> Result<Vec<Armed>> {
    let order = store.order(id)?;
    let inheritance = store.kind(id)? == KIND_INHERITANCE;
    let params = *db.params();
    wallet::sync(db, lwd).await?;
    let order_account = wallet::account_by_uuid(&order.account)?;
    let usk = wallet::usk(keys, db, order_account)?;
    let ovk = usk
        .to_unified_full_viewing_key()
        .orchard()
        .ok_or_else(|| anyhow!("order account has no Orchard-protocol key"))?
        .to_ovk(orchard::keys::Scope::External);
    let payee = txbuild::orchard_receiver(&params, &order.payee)?;
    let per_note = order.amount_zat + installment_fee().into_u64();

    let installments = store.installments(id)?;
    let used: HashSet<(String, u32)> = installments
        .iter()
        .filter_map(|i| Some((i.note_txid.clone()?, i.note_index?)))
        .collect();
    let mut free: Vec<IronwoodNote> = spend::ironwood_notes(db, order_account)?
        .into_iter()
        .filter(|n| n.note().value().inner() == per_note)
        .filter(|n| !used.contains(&(n.txid().to_string(), u32::from(n.output_index()))))
        .collect();
    // Arm in schedule order, as many as there are spendable notes: the rest wait for their
    // split (or top-up) to confirm.
    let pending: Vec<&Installment> = installments
        .iter()
        .filter(|i| i.state == "reserved")
        .take(free.len())
        .collect();
    if pending.is_empty() {
        return Ok(vec![]);
    }

    let chain = quicknet().await?;
    let keeper = Keeper::new(&order.keeper);
    let (target_now, anchor) = spend::heights_now(db)?;
    let tip = target_now - 1;
    let now = now_unix();
    let block_secs = spacing::recent_block_secs(lwd, tip, 30).await;
    let spacing = BlockSpacing::for_network(&params);
    let mut armed = vec![];
    // Each signature gets its own anchor: anchors are public, and two payments mined days apart
    // with one identical anchor would link to the same signer (and two orders to one payer). So
    // at most one installment is signed per anchor height, wallet-wide; the rest wait for later
    // blocks (the next pass).
    let pending: Vec<&Installment> = if store.anchor_in_use(u32::from(anchor))? {
        vec![]
    } else {
        pending.into_iter().take(1).collect()
    };
    for inst in pending {
        let note = free.remove(0);
        let release_at = chain.round_time(inst.round);
        let predicted = predict_height_spaced(&params, tip, now, release_at, block_secs);
        let memo = MemoBytes::from_bytes(
            format!("{} ({}/{})", order.memo, inst.k + 1, order.count).as_bytes(),
        )
        .map_err(|e| anyhow!("memo: {e:?}"))?;

        let mut copies = vec![];
        let mut expiry = BlockHeight::from_u32(0);
        for target in copy_targets(&params, target_now, predicted) {
            // Fallback copies take the ZIP 318 rolling expiry for their expected mining
            // height, so they still look like migration-era traffic when they land. A
            // dead-man's switch may fire months out with the payer gone, so its copy never
            // expires (expiry 0): a known fingerprint, shown as such in the UI.
            expiry = fallback_expiry(inheritance, &spacing, target);
            let built = txbuild::build(
                db,
                &usk,
                std::slice::from_ref(&note),
                &[Output {
                    to: payee,
                    value: Zatoshis::from_u64(order.amount_zat)?,
                    memo: memo.clone(),
                }],
                Some(ovk.clone()),
                target,
                anchor,
                expiry,
            )?;
            copies.push((built.branch, built.txid, built.raw));
        }

        let envelope = Envelope::new(copies.iter().map(|(b, _, raw)| (*b, raw.clone())).collect());
        let ciphertext = tlock::encrypt(&chain, inst.round, &envelope.to_bytes())?;
        let uploaded = upload(&keeper, &order, &ciphertext).await?;
        if uploaded.round != inst.round {
            bail!("keeper locked blob to round {}, expected {}", uploaded.round, inst.round);
        }
        let txids = copies
            .iter()
            .map(|(b, t, _)| format!("{:08x}:{t}", u32::from(*b)))
            .collect::<Vec<_>>()
            .join(",");
        let recorded = store.set_armed(
            id,
            inst.k,
            &note.txid().to_string(),
            u32::from(note.output_index()),
            &txids,
            &uploaded.blob_id,
            &uploaded.delete_token,
            u32::from(anchor),
            u32::from(expiry),
        );
        withdraw_unrecorded(&keeper, &uploaded, recorded).await?;
        armed.push(Armed {
            k: inst.k,
            round: inst.round,
            blob_id: uploaded.blob_id,
            copies: copies.iter().map(|(b, t, _)| (*b, *t)).collect(),
            expiry,
        });
    }
    store.set_order_status(id, "active")?;
    Ok(armed)
}

/// How far along a keeper blob is; higher wins when an installment has two copies.
fn progress(state: &str) -> u8 {
    match state {
        "mined" => 5,
        "broadcast" => 4,
        "released" => 3,
        "locked" => 2,
        _ => 1, // cancelled, expired, failed, deleted
    }
}

/// Pulls each installment's keeper state into the store and returns it. An installment with a
/// fresh copy has two blobs spending the same note; the one furthest along speaks for it, and
/// once either is mined the installment is paid and the other blob is withdrawn.
pub async fn refresh(store: &mut Store, id: i64) -> Result<Vec<(Installment, Option<BlobStatus>)>> {
    let order = store.order(id)?;
    let keeper = Keeper::new(&order.keeper);
    let mut out = vec![];
    for mut inst in store.installments(id)? {
        if inst.state == "cancelled" {
            out.push((inst, None));
            continue;
        }
        let fallback = match &inst.blob_id {
            Some(blob) => keeper.status(blob).await.ok(),
            None => None,
        };
        let fresh = match &inst.fresh_blob_id {
            Some(blob) => keeper.status(blob).await.ok(),
            None => None,
        };
        if let Some(s) = &fallback {
            store.set_keeper_view(id, inst.k, &s.state, s.mined_height)?;
            inst.keeper_state = Some(s.state.clone());
            inst.mined_height = s.mined_height;
        }
        if let Some(s) = &fresh {
            store.set_fresh_keeper_state(id, inst.k, &s.state)?;
            inst.fresh_keeper_state = Some(s.state.clone());
        }
        let status = match (fallback, fresh) {
            (Some(a), Some(b)) => Some(if progress(&b.state) >= progress(&a.state) { b } else { a }),
            (a, b) => a.or(b),
        };
        if let Some(s) = &status {
            match s.state.as_str() {
                "mined" => {
                    inst.state = "paid".into();
                    inst.mined_height = s.mined_height;
                    store.set_keeper_view(id, inst.k, inst.keeper_state.as_deref().unwrap_or("mined"), s.mined_height)?;
                    // The other copy can only fail now; withdrawing it spares the keeper a
                    // doomed broadcast that would tie the two together.
                    for (blob, token, state) in [
                        (&inst.blob_id, &inst.delete_token, &inst.keeper_state),
                        (&inst.fresh_blob_id, &inst.fresh_delete_token, &inst.fresh_keeper_state),
                    ] {
                        if let (Some(blob), Some(token), Some("locked")) = (blob, token, state.as_deref()) {
                            let _ = keeper.delete(blob, token).await;
                        }
                    }
                }
                // Every copy failed: the payer's sweep spent the note first.
                "cancelled" => inst.state = "cancelled".into(),
                _ => {}
            }
            store.set_installment_state(id, inst.k, &inst.state)?;
        }
        out.push((inst, status));
    }
    if out.iter().all(|(i, _)| i.state == "paid" || i.state == "cancelled") {
        let status = if out.iter().any(|(i, _)| i.state == "cancelled") {
            "cancelled"
        } else {
            "completed"
        };
        store.set_order_status(id, status)?;
    }
    Ok(out)
}

/// A fresh copy is signed when the payer is online within this long before the release...
pub const FRESH_WINDOW_SECS: u64 = 45 * 60;
/// ...but no later than this before it, so it can still be mined within its expiry delta.
pub const FRESH_MIN_LEAD_SECS: u64 = 10 * 60;
/// The fresh copy releases this many drand rounds (5 minutes) before the fallback.
pub const FRESH_LEAD_ROUNDS: u64 = 100;

/// How far ahead of its fallback a fresh copy releases: `FRESH_LEAD_ROUNDS`, but never more than
/// half the cadence, so on a short schedule it can't overtake the payment before it.
pub fn fresh_lead_rounds(chain: &tlock::Chain, every_secs: u64) -> u64 {
    if every_secs == 0 {
        return FRESH_LEAD_ROUNDS;
    }
    FRESH_LEAD_ROUNDS.min((every_secs / 2 / chain.period.max(1)).max(1))
}

/// The release time of an installment's earliest copy.
pub fn earliest_release(chain: &tlock::Chain, inst: &Installment) -> u64 {
    chain.round_time(inst.fresh_round.unwrap_or(inst.round).min(inst.round))
}

/// Signs a fresh copy of the next armed installment whose release is between
/// `FRESH_MIN_LEAD_SECS` and `FRESH_WINDOW_SECS` away: the current anchor, built for its
/// predicted mining height with the ordinary expiry delta, like a normal send made then. It is
/// tlocked [`fresh_lead_rounds`] before the fallback, so it is mined first and the pre-signed
/// fallback fails as a double-spend. One signature per anchor height, wallet-wide; call again
/// after the next block for the next one. Returns the installments freshened.
pub async fn freshen<P: Params>(keys: &impl KeyStore, db: &mut Db<P>, store: &mut Store, lwd: &mut Lwd, id: i64) -> Result<Vec<u32>> {
    let order = store.order(id)?;
    // A fresh copy has an ordinary expiry and needs the payer online near the date. A switch
    // exists to pay when the payer is not there, so it keeps its never-expiring copy.
    if store.kind(id)? == KIND_INHERITANCE {
        return Ok(vec![]);
    }
    let now = now_unix();
    let chain = quicknet().await?;
    let due: Vec<Installment> = store
        .installments(id)?
        .into_iter()
        .filter(|i| i.state == "armed" && i.fresh_blob_id.is_none())
        .filter(|i| i.keeper_state.as_deref() == Some("locked"))
        .filter(|i| {
            let release = chain.round_time(i.round);
            release > now + FRESH_MIN_LEAD_SECS && release <= now + FRESH_WINDOW_SECS
        })
        .collect();
    if due.is_empty() {
        return Ok(vec![]);
    }

    let params = *db.params();
    wallet::sync(db, lwd).await?;
    let order_account = wallet::account_by_uuid(&order.account)?;
    let usk = wallet::usk(keys, db, order_account)?;
    let ovk = usk
        .to_unified_full_viewing_key()
        .orchard()
        .ok_or_else(|| anyhow!("order account has no Orchard-protocol key"))?
        .to_ovk(orchard::keys::Scope::External);
    let payee = txbuild::orchard_receiver(&params, &order.payee)?;
    let notes = spend::ironwood_notes(db, order_account)?;
    let (target_now, anchor) = spend::heights_now(db)?;
    let tip = target_now - 1;
    let block_secs = spacing::recent_block_secs(lwd, tip, 30).await;
    let keeper = Keeper::new(&order.keeper);

    let mut done = vec![];
    if store.anchor_in_use(u32::from(anchor))? {
        return Ok(done);
    }
    let lead = fresh_lead_rounds(&chain, order.every_secs);
    for inst in due.into_iter().take(1) {
        // Built for the height it will be mined at, with the ordinary expiry delta from there:
        // the same shape as a payment made at that moment. The branch follows that height, so
        // a fresh copy due after an upgrade is signed for the new branch.
        let release = chain.round_time(inst.round.saturating_sub(lead));
        let target = predict_height_spaced(&params, tip, now, release, block_secs);
        let expiry = target + default_tx_expiry_delta(BranchId::for_height(&params, target));
        let Some(note) = notes.iter().find(|n| {
            Some(n.txid().to_string()) == inst.note_txid
                && Some(u32::from(n.output_index())) == inst.note_index
        }) else {
            continue;
        };
        let memo = MemoBytes::from_bytes(
            format!("{} ({}/{})", order.memo, inst.k + 1, order.count).as_bytes(),
        )
        .map_err(|e| anyhow!("memo: {e:?}"))?;
        let built = txbuild::build(
            db,
            &usk,
            std::slice::from_ref(note),
            &[Output {
                to: payee,
                value: Zatoshis::from_u64(order.amount_zat)?,
                memo,
            }],
            Some(ovk.clone()),
            target,
            anchor,
            expiry,
        )?;
        let round = inst.round.saturating_sub(lead);
        let envelope = Envelope::new(vec![(built.branch, built.raw)]);
        let ciphertext = tlock::encrypt(&chain, round, &envelope.to_bytes())?;
        let uploaded = upload(&keeper, &order, &ciphertext).await?;
        let recorded = store.set_fresh(
            id,
            inst.k,
            round,
            &built.txid.to_string(),
            &uploaded.blob_id,
            &uploaded.delete_token,
            u32::from(anchor),
        );
        withdraw_unrecorded(&keeper, &uploaded, recorded).await?;
        done.push(inst.k);
    }
    Ok(done)
}

/// Why a cancel could not stop an installment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StillPays {
    /// The keeper has already released or broadcast it.
    Released,
    /// Its round is closer than the cancel margin.
    RaceWindow,
}

impl StillPays {
    pub fn code(&self) -> &'static str {
        match self {
            StillPays::Released => "released",
            StillPays::RaceWindow => "race_window",
        }
    }

    pub fn reason(&self) -> &'static str {
        match self {
            StillPays::Released => "already released by the keeper",
            StillPays::RaceWindow => "inside the race window",
        }
    }
}

pub struct Cancelled {
    pub sweep: Option<TxId>,
    pub cancelled: Vec<u32>,
    /// Installments that may still pay: the keeper already has them out, or their round is
    /// inside the race window.
    pub may_still_pay: Vec<(u32, StillPays)>,
}

/// Sweeps every note the order still controls back to `return_to` (the treasury by default)
/// in one transaction. Installments whose round is within `margin_secs` are left alone: the
/// keeper may already be broadcasting them. With `withdraw_blobs`, the swept installments'
/// blobs are deleted from the keeper; without it the keeper still releases them at their round
/// and the chain rejects them as double-spends, which shows the cancel is chain-enforced.
#[allow(clippy::too_many_arguments)]
pub async fn cancel<P: Params>(
    keys: &impl KeyStore,
    db: &mut Db<P>,
    store: &mut Store,
    lwd: &mut Lwd,
    id: i64,
    return_to: Option<&str>,
    margin_secs: u64,
    withdraw_blobs: bool,
) -> Result<Cancelled> {
    let order: Order = store.order(id)?;
    if order.status == "completed" {
        bail!("order {id} is completed: every installment has paid, so there is nothing to cancel");
    }
    wallet::sync(db, lwd).await?;
    let statuses = refresh(store, id).await?;
    let chain = quicknet().await?;
    let now = now_unix();

    let mut keep_notes: HashSet<(String, u32)> = HashSet::new();
    let mut cancelled = vec![];
    let mut may_still_pay = vec![];
    for (inst, status) in &statuses {
        if inst.state == "paid" || inst.state == "cancelled" {
            continue;
        }
        let reason = if status.as_ref().is_some_and(|s| s.is_out_of_our_hands()) {
            Some(StillPays::Released)
        } else if inst.blob_id.is_some() && earliest_release(&chain, inst) <= now + margin_secs {
            Some(StillPays::RaceWindow)
        } else {
            None
        };
        match reason {
            Some(reason) => {
                if let (Some(t), Some(i)) = (&inst.note_txid, inst.note_index) {
                    keep_notes.insert((t.clone(), i));
                }
                may_still_pay.push((inst.k, reason));
            }
            None => cancelled.push(inst.k),
        }
    }

    let order_account = wallet::account_by_uuid(&order.account)?;
    let notes: Vec<IronwoodNote> = spend::ironwood_notes(db, order_account)?
        .into_iter()
        .filter(|n| !keep_notes.contains(&(n.txid().to_string(), u32::from(n.output_index()))))
        .collect();

    let mut sweep = None;
    if !notes.is_empty() {
        let to = match return_to {
            Some(addr) => addr.to_string(),
            None => {
                let treasury = wallet::account(db)?;
                wallet::next_address(db, treasury)?
            }
        };
        let usk = wallet::usk(keys, db, order_account)?;
        let built = spend::sweep(db, &usk, &notes, &to, MemoBytes::empty())?;
        match net::send(lwd, &built.raw).await? {
            SendOutcome::Accepted => sweep = Some(built.txid),
            SendOutcome::Rejected { code, message } => bail!(
                "cancel sweep rejected ({code}): {message}. If the keeper released an installment \
                 just now, run cancel again to sweep the rest."
            ),
        }
    }

    let keeper = Keeper::new(&order.keeper);
    for (inst, _) in &statuses {
        if cancelled.contains(&inst.k) {
            if withdraw_blobs {
                // The chain enforces the cancel; withdrawing the blobs only spares the keeper a
                // doomed broadcast that would tie the sweep to the order.
                for (blob, token) in [
                    (&inst.blob_id, &inst.delete_token),
                    (&inst.fresh_blob_id, &inst.fresh_delete_token),
                ] {
                    if let (Some(blob), Some(token)) = (blob, token) {
                        let _ = keeper.delete(blob, token).await;
                    }
                }
            }
            store.set_installment_state(id, inst.k, "cancelled")?;
        }
    }
    store.set_cancelled(id, sweep.map(|t| t.to_string()).as_deref())?;
    store.set_order_status(id, if may_still_pay.is_empty() { "cancelled" } else { "cancelling" })?;
    Ok(Cancelled {
        sweep,
        cancelled,
        may_still_pay,
    })
}

/// Records a dead-man's switch: one payment of `amount` to `heir` at `at` (unix seconds), unless
/// the payer renews it first ([`renew`]). Funded and armed like any order; its pre-signed copy
/// has expiry height 0 so it can pay whenever it fires, and it never gets a fresh copy. The
/// payer is reminded `remind_secs` before the date (promissory-app's `switch_due` notice).
#[allow(clippy::too_many_arguments)]
pub async fn create_inheritance<P: Params>(
    keys: &impl KeyStore,
    db: &mut Db<P>,
    store: &mut Store,
    lwd: &mut Lwd,
    heir: &str,
    amount: Zatoshis,
    memo: &str,
    at: u64,
    remind_secs: u64,
    keeper: &str,
) -> Result<Created> {
    if at < now_unix() + CANCEL_MARGIN_SECS {
        bail!(
            "a switch must be at least {} minutes out, so there is time to renew it",
            CANCEL_MARGIN_SECS / 60
        );
    }
    let schedule = Schedule {
        amount,
        memo: memo.to_string(),
        every_secs: 0,
        count: 1,
        first_due: at,
        jitter_max_secs: 0,
    };
    let created = create_with_window(keys, db, store, lwd, heir, &schedule, keeper, 0).await?;
    store.set_kind(created.id, KIND_INHERITANCE)?;
    store.set_remind_secs(created.id, remind_secs)?;
    Ok(created)
}

pub struct Renewed {
    /// The switch that was renewed; now `cancelled`, its note spent.
    pub old_id: i64,
    /// The same switch with the later date, already funded (status `splitting` until its
    /// note confirms, then armed by the next progression pass or `promissory order arm`).
    pub new_id: i64,
    /// The one transaction that spent the old reservation (plus a treasury note for the fee)
    /// into the new reservation. `None` if the old switch was never funded.
    pub txid: Option<TxId>,
    /// The old blob was deleted from the keeper. Without that the keeper still releases it at
    /// its round and the chain rejects it as a double-spend.
    pub withdrew_blob: bool,
}

/// Renews a dead-man's switch: "I am still here". One transaction spends the old switch's
/// reserved note and one treasury note (for the fee), and outputs the new switch's reservation
/// plus change to the treasury. The old pre-signed copy can then never pay: its note is spent.
/// The new switch has the same heir, amount, memo, reminder and keeper, due at `at`.
///
/// Refused when the old copy is already out of the payer's hands, or due within `margin_secs`
/// (it might be mined before the renewal).
#[allow(clippy::too_many_arguments)]
pub async fn renew<P: Params>(
    keys: &impl KeyStore,
    db: &mut Db<P>,
    store: &mut Store,
    lwd: &mut Lwd,
    id: i64,
    at: u64,
    margin_secs: u64,
    withdraw_blob: bool,
) -> Result<Renewed> {
    let order = store.order(id)?;
    let meta = store.meta(id)?;
    if meta.kind != KIND_INHERITANCE {
        bail!("order {id} is not a dead-man's switch");
    }
    if let Some(next) = meta.renewed_to {
        bail!("switch {id} was already renewed as #{next}; renew that one");
    }
    if matches!(order.status.as_str(), "completed" | "cancelled" | "cancelling") {
        bail!("switch {id} is {}; there is nothing to renew", order.status);
    }
    if at <= order.first_due {
        bail!("the new date must be later than the current one");
    }
    let remind = meta.remind_secs.unwrap_or(SWITCH_REMIND_SECS);
    let params = *db.params();

    wallet::sync(db, lwd).await?;
    let statuses = refresh(store, id).await?;
    let chain = quicknet().await?;
    let now = now_unix();
    let (inst, status) = statuses.first().ok_or_else(|| anyhow!("switch {id} has no installment"))?;
    if inst.state == "paid" {
        bail!("switch {id} has already paid the heir");
    }
    if status.as_ref().is_some_and(|s| s.is_out_of_our_hands()) {
        bail!("the keeper has already released switch {id}; it will pay");
    }
    if inst.blob_id.is_some() && earliest_release(&chain, inst) <= now + margin_secs {
        bail!("switch {id} is due too soon to renew safely (inside the race window)");
    }

    let order_account = wallet::account_by_uuid(&order.account)?;
    let old_notes = spend::ironwood_notes(db, order_account)?;
    if old_notes.is_empty() {
        if order.status != "awaiting_funds" {
            bail!("switch {id}'s reserved note is not spendable yet; try again after the next block");
        }
        // Never funded: nothing to move, just replace the record.
        let created = create_inheritance(
            keys,
            db,
            store,
            lwd,
            &order.payee,
            Zatoshis::from_u64(order.amount_zat)?,
            &order.memo,
            at,
            remind,
            &order.keeper,
        )
        .await?;
        store.set_renewed_from(created.id, id)?;
        store.set_installment_state(id, 0, "cancelled")?;
        store.set_cancelled(id, None)?;
        store.set_order_status(id, "cancelled")?;
        return Ok(Renewed {
            old_id: id,
            new_id: created.id,
            txid: None,
            withdrew_blob: false,
        });
    }

    // The renewal's fee comes from one treasury note; the rest of it returns as change.
    let fee = txbuild::ironwood_fee(old_notes.len() + 1, 2).into_u64();
    let treasury = wallet::account(db)?;
    let mut treasury_notes = spend::ironwood_notes(db, treasury)?;
    treasury_notes.sort_by_key(|n| n.note().value().inner());
    let fee_note = treasury_notes
        .into_iter()
        .find(|n| n.note().value().inner() >= fee)
        .ok_or_else(|| {
            anyhow!(
                "renewing needs {} ZEC spendable in the treasury for the network fee; top it up",
                zec_decimal(fee)
            )
        })?;
    let per_note = order.amount_zat + installment_fee().into_u64();
    let input: u64 = old_notes.iter().map(|n| n.note().value().inner()).sum::<u64>() + fee_note.note().value().inner();
    let change = input
        .checked_sub(per_note + fee)
        .ok_or_else(|| anyhow!("switch {id}'s reserved notes ({input} zat) do not cover a new reservation"))?;

    let created = create_inheritance(
        keys,
        db,
        store,
        lwd,
        &order.payee,
        Zatoshis::from_u64(order.amount_zat)?,
        &order.memo,
        at,
        remind,
        &order.keeper,
    )
    .await?;
    let new_id = created.id;
    // Built synchronously: the store and wallet are not `Sync`, so they must not be held
    // across the broadcast's await.
    let built = (|| -> Result<txbuild::Built> {
        let new_account = wallet::account_by_uuid(&store.order(new_id)?.account)?;
        let reserve_to = txbuild::orchard_receiver(&params, &wallet::next_address(db, new_account)?)?;
        let mut outputs = vec![Output {
            to: reserve_to,
            value: Zatoshis::from_u64(per_note)?,
            memo: MemoBytes::empty(),
        }];
        if change > 0 {
            let back = wallet::next_address(db, treasury)?;
            outputs.push(Output {
                to: txbuild::orchard_receiver(&params, &back)?,
                value: Zatoshis::from_u64(change)?,
                memo: MemoBytes::empty(),
            });
        }
        let (target, mut anchor) = spend::heights_now(db)?;
        // Keep the wallet's one-signature-per-anchor rule: step back past anchors that a
        // pre-signed installment already uses.
        for _ in 0..10 {
            if !store.anchor_in_use(u32::from(anchor))? {
                break;
            }
            anchor = anchor - 1;
        }
        let expiry = target + default_tx_expiry_delta(BranchId::for_height(&params, target));
        let old_usk = wallet::usk(keys, db, order_account)?;
        let treasury_usk = wallet::usk(keys, db, treasury)?;
        // No outgoing viewing key, like the cancel sweep: a proof of reservation shared for
        // the old switch must not show where its funds went.
        txbuild::build_multi(
            db,
            &[
                (&old_usk, &old_notes[..]),
                (&treasury_usk, std::slice::from_ref(&fee_note)),
            ],
            &outputs,
            None,
            target,
            anchor,
            expiry,
        )
    })();
    let built = match built {
        Ok(b) => match net::send(lwd, &b.raw).await {
            Ok(SendOutcome::Accepted) => Ok(b.txid),
            Ok(SendOutcome::Rejected { code, message }) => Err(anyhow!("renewal rejected ({code}): {message}")),
            Err(e) => Err(e),
        },
        Err(e) => Err(e),
    };
    let txid = match built {
        Ok(t) => t,
        Err(e) => {
            // The new switch was never funded: retire it so nothing tries to fund it.
            store.set_order_status(new_id, "cancelled")?;
            return Err(e);
        }
    };

    store.set_renewed_from(new_id, id)?;
    store.set_funded_count(new_id, 1)?;
    store.set_split(new_id, &txid.to_string())?;

    let mut withdrew_blob = false;
    if withdraw_blob {
        let keeper = Keeper::new(&order.keeper);
        for (blob, token) in [
            (&inst.blob_id, &inst.delete_token),
            (&inst.fresh_blob_id, &inst.fresh_delete_token),
        ] {
            if let (Some(blob), Some(token)) = (blob, token) {
                withdrew_blob |= keeper.delete(blob, token).await.is_ok();
            }
        }
    }
    store.set_installment_state(id, inst.k, "cancelled")?;
    store.set_cancelled(id, Some(&txid.to_string()))?;
    store.set_order_status(id, "cancelled")?;
    Ok(Renewed {
        old_id: id,
        new_id,
        txid: Some(txid),
        withdrew_blob,
    })
}

#[cfg(test)]
mod switch_tests {
    use super::*;
    use crate::network::SoNetwork;

    #[test]
    fn only_switches_never_expire() {
        let params = SoNetwork::regtest();
        let spacing = BlockSpacing::for_network(&params);
        let target = BlockHeight::from_u32(1_300);
        assert_eq!(fallback_expiry(true, &spacing, target), BlockHeight::from_u32(0));
        let normal = fallback_expiry(false, &spacing, target);
        assert!(normal > target, "a standing order's fallback copy keeps a real expiry: {normal}");
        assert_eq!(normal, zip318::expiry_height(&spacing, target));
    }
}

#[cfg(test)]
mod fresh_tests {
    use super::*;

    #[test]
    fn fresh_copy_never_overtakes_the_payment_before_it() {
        // quicknet: a round every 3 seconds.
        let chain = tlock::Chain { hash: vec![], public_key: vec![], genesis_time: 0, period: 3 };
        // Long cadences keep the usual 5-minute lead.
        assert_eq!(fresh_lead_rounds(&chain, 604_800), FRESH_LEAD_ROUNDS);
        assert_eq!(fresh_lead_rounds(&chain, 600), FRESH_LEAD_ROUNDS);
        // Every 3 minutes: at most 90 s early (30 rounds of 3 s), behind the previous due time.
        assert_eq!(fresh_lead_rounds(&chain, 180), 30);
        for every in [60, 90, 180, 300, 599] {
            assert!(fresh_lead_rounds(&chain, every) * chain.period < every, "every {every}s");
        }
    }
}

/// Compile-time check that a wallet with its own consensus parameters and key storage can drive
/// the whole order lifecycle.
#[cfg(test)]
#[allow(dead_code)]
mod embedding {
    use super::*;
    use secrecy::SecretVec;
    use zcash_protocol::consensus::MainNetwork;

    struct WalletKeys;
    impl KeyStore for WalletKeys {
        fn seed(&self) -> Result<SecretVec<u8>> {
            unimplemented!("a wallet reads its own seed storage")
        }
    }

    async fn lifecycle(db: &mut Db<MainNetwork>, store: &mut Store, lwd: &mut Lwd, s: &Schedule) -> Result<()> {
        let keys = WalletKeys;
        let created = create_with_window(&keys, db, store, lwd, "u1…", s, "https://keeper", 2).await?;
        split(&keys, db, store, lwd, created.id).await?;
        arm(&keys, db, store, lwd, created.id).await?;
        freshen(&keys, db, store, lwd, created.id).await?;
        refresh(store, created.id).await?;
        cancel(&keys, db, store, lwd, created.id, None, CANCEL_MARGIN_SECS, true).await?;
        let switch = create_inheritance(&keys, db, store, lwd, "u1…", s.amount, "", s.first_due, SWITCH_REMIND_SECS, "https://keeper").await?;
        renew(&keys, db, store, lwd, switch.id, s.first_due + 86_400, CANCEL_MARGIN_SECS, true).await?;
        Ok(())
    }
}
