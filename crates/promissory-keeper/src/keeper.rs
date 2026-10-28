//! The release loop: decrypt blobs whose drand round has been published, broadcast the copy
//! for the next block's branch, and follow the transaction until it is buried.
//!
//! The ciphertext stays in the database while the blob is `released`, `broadcast`, or mined
//! but not yet buried, because retries and rebroadcasts need it. Its round has been published
//! by then, so it protects nothing a public beacon doesn't already reveal. It is dropped as soon
//! as the blob is buried or reaches a terminal state.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use rand::{rand_core::UnwrapErr, rngs::SysRng, Rng};
use sha2::{Digest, Sha256};
use promissory_core::{
    envelope::Envelope,
    net::{self, SendOutcome, TxStatus},
    network::SoNetwork,
    tlock,
};
use subtle::ConstantTimeEq;
use tokio::sync::Mutex as AsyncMutex;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BlockHeight, BranchId};

use crate::backend::{parse_txid, Beacon, Chain};
use crate::db::{Blob, Db, NewBlob, Quota, State, Update};
use crate::unix_now;
use crate::webhook::Notifier;

/// One day of quicknet rounds (3 s period). Older ciphertexts are refused at upload.
pub const MAX_PAST_ROUNDS: u64 = 86_400 / 3;

#[derive(Clone, Debug)]
pub struct Config {
    pub tick: Duration,
    /// Failed send attempts (unclassified rejections) before a blob is `failed`.
    pub max_attempts: u32,
    pub backoff_cap: Duration,
    /// How long a broadcast transaction may be unknown to the node before it is rebroadcast.
    pub unknown_grace: Duration,
    /// Depth at which a mined transaction counts as buried and the ciphertext is dropped.
    pub confirmations: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(3),
            max_attempts: 20,
            backoff_cap: Duration::from_secs(300),
            unknown_grace: Duration::from_secs(30),
            confirmations: 10,
        }
    }
}

/// How to treat a rejection from `SendTransaction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The node already has this transaction: treat as broadcast.
    AlreadyKnown,
    /// A note it spends has been spent elsewhere (the payer's cancel sweep).
    Cancelled,
    Expired,
    /// Signed for another consensus branch: try again next block.
    WrongBranch,
    /// Anything else: back off and retry.
    Retry,
}

/// Classifies a Zebra rejection message (as relayed by lightwalletd).
pub fn classify(message: &str) -> Verdict {
    let m = message.to_ascii_lowercase();
    let has = |s: &str| m.contains(s);
    if has("already exists in mempool") || has("already in mempool") || has("already in state") {
        Verdict::AlreadyKnown
    } else if has("duplicate nullifier")
        || (has("duplicate") && has("nullifier"))
        || has("has already spent some of its inputs")
        || has("nullifiers were revealed")
        || has("double-spend")
    {
        Verdict::Cancelled
    } else if has("expired") || has("greater than its expiry") {
        Verdict::Expired
    } else if has("wrongconsensusbranchid") || has("consensus branch id") {
        Verdict::WrongBranch
    } else {
        Verdict::Retry
    }
}

/// What the keeper last saw of the outside world, for `/v1/health`.
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub tip_height: Option<u32>,
    pub latest_round: Option<u64>,
    pub last_tick_at: Option<u64>,
    pub last_error: Option<String>,
}

pub struct Uploaded {
    pub blob_id: String,
    pub delete_token: String,
    pub round: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Deleted {
    Ok,
    NotFound,
    BadToken,
    TooLate(State),
}

pub struct Keeper {
    pub db: Db,
    beacon: Arc<dyn Beacon>,
    chain: Arc<dyn Chain>,
    params: SoNetwork,
    cfg: Config,
    notifier: Option<Notifier>,
    status: Mutex<Status>,
    /// Held across "check state, send" and across deletes, so a deleted blob is never sent.
    send_lock: AsyncMutex<()>,
}

fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    UnwrapErr(SysRng).fill_bytes(&mut b);
    hex::encode(b)
}

pub fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

impl Keeper {
    pub fn new(
        db: Db,
        beacon: Arc<dyn Beacon>,
        chain: Arc<dyn Chain>,
        params: SoNetwork,
        cfg: Config,
        notifier: Option<Notifier>,
    ) -> Self {
        Self {
            db,
            beacon,
            chain,
            params,
            cfg,
            notifier,
            status: Mutex::new(Status::default()),
            send_lock: AsyncMutex::new(()),
        }
    }

    pub fn webhooks_enabled(&self) -> bool {
        self.notifier.is_some()
    }

    pub fn network(&self) -> SoNetwork {
        self.params
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set_status(&self, f: impl FnOnce(&mut Status)) {
        f(&mut self.status.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// The latest round, from the last tick or else straight from the beacon.
    pub async fn latest_round(&self) -> Result<u64> {
        if let Some(r) = self.status().latest_round {
            return Ok(r);
        }
        let r = self.beacon.latest_round().await?;
        self.set_status(|s| s.latest_round = Some(r));
        Ok(r)
    }

    /// Stores a validated ciphertext locked to `round`.
    pub fn store(&self, ciphertext: &[u8], round: u64, webhook: Option<&str>) -> Result<Uploaded> {
        self.store_within(ciphertext, round, webhook, 0, 0)?
            .map_err(|q| anyhow::anyhow!("quota {q:?}"))
    }

    /// Stores a validated ciphertext locked to `round`, unless that would exceed the storage
    /// quota (`0` disables a cap).
    pub fn store_within(
        &self,
        ciphertext: &[u8],
        round: u64,
        webhook: Option<&str>,
        max_bytes: u64,
        max_locked: u64,
    ) -> Result<Result<Uploaded, Quota>> {
        let blob_id = random_hex(16);
        let delete_token = random_hex(32);
        let nb = NewBlob {
            blob_id: &blob_id,
            round,
            ciphertext,
            token_hash: &token_hash(&delete_token),
            webhook,
        };
        if let Err(q) = self.db.insert_within(&nb, unix_now(), max_bytes, max_locked)? {
            tracing::warn!(?q, "storage quota reached; upload refused");
            return Ok(Err(q));
        }
        tracing::info!(%blob_id, round, "stored");
        Ok(Ok(Uploaded {
            blob_id,
            delete_token,
            round,
        }))
    }

    /// How the keeper reaches lightwalletd and drand (`"direct"` or `"tor"`), for health.
    pub fn transports(&self) -> (&'static str, &'static str) {
        (self.chain.transport(), self.beacon.transport())
    }

    /// Deletes a blob that has not been broadcast yet.
    pub async fn delete(&self, blob_id: &str, token: &str) -> Result<Deleted> {
        let _guard = self.send_lock.lock().await;
        let Some(b) = self.db.get(blob_id)? else {
            return Ok(Deleted::NotFound);
        };
        if !bool::from(token_hash(token).ct_eq(&b.token_hash)) {
            return Ok(Deleted::BadToken);
        }
        match b.state {
            State::Deleted => Ok(Deleted::Ok),
            State::Locked | State::Released => {
                let u = Update {
                    state: Some(State::Deleted),
                    drop_ciphertext: true,
                    ..Default::default()
                };
                self.transition(&b, u, unix_now())?;
                Ok(Deleted::Ok)
            }
            s => Ok(Deleted::TooLate(s)),
        }
    }

    pub async fn run(self: Arc<Self>) {
        let mut every = tokio::time::interval(self.cfg.tick);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            every.tick().await;
            if let Err(e) = self.tick(unix_now()).await {
                tracing::warn!("tick: {e:#}");
            }
        }
    }

    /// One pass of the loop at unix time `now`.
    pub async fn tick(&self, now: u64) -> Result<()> {
        let r = self.tick_inner(now).await;
        self.set_status(|s| {
            s.last_tick_at = Some(now);
            s.last_error = r.as_ref().err().map(|e| format!("{e:#}"));
        });
        r
    }

    async fn tick_inner(&self, now: u64) -> Result<()> {
        let tip = self.chain.tip_height().await.context("lightwalletd tip")?;
        self.set_status(|s| s.tip_height = Some(tip.into()));
        for b in self.db.in_flight()? {
            if let Err(e) = self.track(&b, tip, now).await {
                tracing::warn!(blob_id = %b.blob_id, "track: {e:#}");
            }
        }
        let latest = self.beacon.latest_round().await.context("drand latest round")?;
        self.set_status(|s| s.latest_round = Some(latest));
        for b in self.db.due(latest, now)? {
            if let Err(e) = self.send_copy(&b, tip, now).await {
                tracing::warn!(blob_id = %b.blob_id, "release: {e:#}");
            }
        }
        Ok(())
    }

    /// Applies `u` if the blob is still in `b.state`, logging and notifying a state change.
    fn transition(&self, b: &Blob, u: Update, now: u64) -> Result<bool> {
        let applied = self.db.update(&b.blob_id, b.state, &u, now)?;
        if let (true, Some(to)) = (applied, u.state) {
            if to != b.state {
                let txid = u.txid.as_deref().or(b.txid.as_deref());
                tracing::info!(blob_id = %b.blob_id, txid = ?txid, "{} -> {}", b.state.as_str(), to.as_str());
                if let (Some(n), Some(url)) = (&self.notifier, &b.webhook) {
                    n.notify(url, &b.blob_id, txid, to.as_str());
                }
            }
        }
        Ok(applied)
    }

    fn finish(&self, b: &Blob, to: State, why: String, now: u64) -> Result<()> {
        let u = Update {
            state: Some(to),
            last_error: Some(Some(why)),
            drop_ciphertext: true,
            ..Default::default()
        };
        self.transition(b, u, now).map(drop)
    }

    /// Leaves the blob as it is and tries again next tick.
    fn retry_next_tick(&self, b: &Blob, why: String, now: u64) -> Result<()> {
        let u = Update {
            last_error: Some(Some(why)),
            next_attempt_at: Some(now),
            ..Default::default()
        };
        self.transition(b, u, now).map(drop)
    }

    fn back_off(&self, b: &Blob, why: String, now: u64) -> Result<()> {
        let attempts = b.attempts + 1;
        if attempts >= self.cfg.max_attempts {
            tracing::warn!(blob_id = %b.blob_id, attempts, "giving up: {why}");
            return self.finish(b, State::Failed, why, now);
        }
        let base = self.cfg.tick.as_secs().max(1);
        let delay = base
            .saturating_mul(1 << (attempts - 1).min(20))
            .min(self.cfg.backoff_cap.as_secs());
        tracing::info!(blob_id = %b.blob_id, attempts, delay, "rejected, backing off: {why}");
        let u = Update {
            last_error: Some(Some(why)),
            attempts: Some(attempts),
            next_attempt_at: Some(now + delay),
            ..Default::default()
        };
        self.transition(b, u, now).map(drop)
    }

    fn accepted(&self, b: &Blob, txid: String, now: u64) -> Result<()> {
        let u = Update {
            state: Some(State::Broadcast),
            txid: Some(txid),
            mined_height: Some(None),
            last_error: Some(None),
            unknown_since: Some(None),
            next_attempt_at: Some(0),
            ..Default::default()
        };
        self.transition(b, u, now).map(drop)
    }

    fn mined(&self, b: &Blob, h: BlockHeight, tip: BlockHeight, now: u64) -> Result<()> {
        let h = u32::from(h);
        let buried = u32::from(tip) + 1 >= h + self.cfg.confirmations;
        if b.state == State::Mined && b.mined_height == Some(h) && !buried {
            return Ok(());
        }
        let u = Update {
            state: Some(State::Mined),
            mined_height: Some(Some(h)),
            last_error: Some(None),
            unknown_since: Some(None),
            drop_ciphertext: buried,
            ..Default::default()
        };
        self.transition(b, u, now).map(drop)
    }

    /// Decrypts `b`, picks the copy for the next block, sends it and classifies the outcome.
    /// Used for the first release and for rebroadcasts.
    async fn send_copy(&self, b: &Blob, tip: BlockHeight, now: u64) -> Result<()> {
        let _guard = self.send_lock.lock().await;
        // Re-read under the lock: a delete may have landed since the blob was listed.
        let Some(mut b) = self.db.get(&b.blob_id)?.filter(|cur| cur.state == b.state) else {
            return Ok(());
        };
        let Some(ct) = b.ciphertext.as_deref() else {
            return Ok(());
        };
        let Some(sig) = self.beacon.signature(b.round).await? else {
            return Ok(());
        };
        let env = match tlock::decrypt(&self.beacon.chain_hash(), &sig, ct)
            .and_then(|pt| Envelope::from_bytes(&pt))
        {
            Ok(env) => env,
            Err(e) => return self.finish(&b, State::Failed, format!("{e:#}"), now),
        };
        if b.state == State::Locked {
            let u = Update {
                state: Some(State::Released),
                ..Default::default()
            };
            if !self.transition(&b, u, now)? {
                return Ok(());
            }
            b.state = State::Released;
        }

        let next = tip + 1;
        let (branch, raw) = match env.copy_for(&self.params, next) {
            Ok(Some(copy)) => copy,
            Ok(None) => {
                let branch = BranchId::for_height(&self.params, next);
                let why = format!("no copy for branch {branch:?} at height {next}");
                return self.retry_next_tick(&b, why, now);
            }
            Err(e) => return self.finish(&b, State::Failed, format!("{e:#}"), now),
        };
        let txid = match Transaction::read(&raw[..], branch) {
            Ok(tx) => tx.txid().to_string(),
            Err(e) => return self.finish(&b, State::Failed, format!("copy does not parse: {e}"), now),
        };

        let (code, message) = match self.chain.send(&raw).await {
            Ok(SendOutcome::Accepted) => return self.accepted(&b, txid, now),
            Ok(SendOutcome::Rejected { code, message }) => (code, message),
            // Never reached the node: nothing to classify, try again next tick.
            Err(e) if net::is_transport(&e) => return self.retry_next_tick(&b, format!("{e:#}"), now),
            Err(e) => return Err(e.context("SendTransaction")),
        };
        let why = format!("rejected ({code}): {message}");
        match classify(&message) {
            Verdict::AlreadyKnown => self.accepted(&b, txid, now),
            Verdict::Cancelled => {
                // Our own earlier copy being mined also makes this copy a double spend.
                if let Some(prev) = b.txid.as_deref() {
                    if let Ok(TxStatus::Mined(h)) = self.chain.tx_status(&parse_txid(prev)?).await {
                        return self.mined(&b, h, tip, now);
                    }
                }
                self.finish(&b, State::Cancelled, why, now)
            }
            Verdict::Expired => self.finish(&b, State::Expired, why, now),
            Verdict::WrongBranch => self.retry_next_tick(&b, why, now),
            Verdict::Retry => self.back_off(&b, why, now),
        }
    }

    /// Follows a broadcast (or mined, not yet buried) transaction.
    async fn track(&self, b: &Blob, tip: BlockHeight, now: u64) -> Result<()> {
        let Some(txid) = b.txid.as_deref() else {
            return Ok(());
        };
        let status = self.chain.tx_status(&parse_txid(txid)?).await?;
        match status {
            TxStatus::Mined(h) => self.mined(b, h, tip, now),
            TxStatus::Mempool if b.state == State::Mined => {
                // Reorged out and back in the mempool.
                let u = Update {
                    state: Some(State::Broadcast),
                    mined_height: Some(None),
                    ..Default::default()
                };
                self.transition(b, u, now).map(drop)
            }
            TxStatus::Mempool if b.unknown_since.is_some() => {
                let u = Update {
                    unknown_since: Some(None),
                    ..Default::default()
                };
                self.transition(b, u, now).map(drop)
            }
            TxStatus::Mempool => Ok(()),
            TxStatus::Orphaned | TxStatus::Unknown => {
                let since = match (&status, b.unknown_since) {
                    (TxStatus::Unknown, None) => {
                        let u = Update {
                            unknown_since: Some(Some(now)),
                            ..Default::default()
                        };
                        return self.transition(b, u, now).map(drop);
                    }
                    (TxStatus::Unknown, Some(t)) => t,
                    _ => 0,
                };
                if now < since + self.cfg.unknown_grace.as_secs() || now < b.next_attempt_at {
                    return Ok(());
                }
                let mut b = b.clone();
                if b.state == State::Mined {
                    let u = Update {
                        state: Some(State::Broadcast),
                        mined_height: Some(None),
                        ..Default::default()
                    };
                    if !self.transition(&b, u, now)? {
                        return Ok(());
                    }
                    b.state = State::Broadcast;
                }
                tracing::info!(blob_id = %b.blob_id, ?status, "rebroadcasting");
                self.send_copy(&b, tip, now).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_zebra_rejections() {
        use Verdict::*;
        let cases = [
            ("transaction already exists in mempool", AlreadyKnown),
            ("transaction is already in state", AlreadyKnown),
            ("orchard double-spend: duplicate nullifier: Nullifier(..)", Cancelled),
            ("duplicate Ironwood nullifier", Cancelled),
            (
                "transaction rejected because another transaction in the mempool has already spent some of its inputs",
                Cancelled,
            ),
            ("one or more nullifiers were revealed by a mined transaction", Cancelled),
            ("transaction did not pass consensus validation: ExpiredTransaction { expiry_height: 120 }", Expired),
            ("transaction is expired", Expired),
            ("transaction must not be mined at a block 130 greater than its expiry 120", Expired),
            ("WrongConsensusBranchId", WrongBranch),
            ("transaction uses an incorrect consensus branch id", WrongBranch),
            ("mempool is full", Retry),
            ("", Retry),
        ];
        for (msg, want) in cases {
            assert_eq!(classify(msg), want, "{msg}");
        }
    }

    #[test]
    fn transport_codes() {
        // Transport failures arrive as a typed error, not as a rejection code.
        let e: anyhow::Error = net::Transport("Unavailable: connection refused".into()).into();
        assert!(net::is_transport(&e));
        assert!(!net::is_transport(&anyhow::anyhow!("rejected (14): mempool is full")));
    }
}
