//! The payer-side engine behind the API: one wallet, one order store, one lock.

use anyhow::{bail, Result};
use serde::Serialize;
use promissory_core::{
    keeper::BlobStatus,
    link::Subscribe,
    net::{self, Lwd},
    network::SoNetwork,
    order::{self, now_unix, Schedule},
    store::{Installment, Order, Store},
    tlock::{Chain, Drand},
    wallet::{self, Db, Paths},
};
use zcash_protocol::value::Zatoshis;

pub struct Engine {
    pub params: SoNetwork,
    pub paths: Paths,
    pub db: Db,
    pub store: Store,
    pub lwd_url: String,
    pub default_keeper: String,
    chain: Option<Chain>,
}

#[derive(Serialize)]
pub struct Info {
    pub network: &'static str,
    pub tip_height: u32,
    pub synced_height: u32,
    pub treasury_address: String,
    pub treasury_zat: u64,
    pub treasury_spendable_zat: u64,
    pub default_keeper: String,
    pub cancel_margin_secs: u64,
}

#[derive(Clone, Serialize)]
pub struct Funding {
    pub zat: u64,
    pub address: String,
    pub uri: String,
    /// How many installments this request funds (with what the treasury already holds).
    pub installments: u32,
}

#[derive(Clone, Serialize)]
pub struct OrderView {
    pub id: i64,
    pub status: String,
    pub payee: String,
    pub amount_zat: u64,
    pub memo: String,
    pub every_secs: u64,
    pub count: u32,
    pub first_due: u64,
    pub keeper: String,
    pub split_txid: Option<String>,
    pub created_at: u64,
    /// Present only while the order awaits its first funding.
    pub funding: Option<Funding>,
    /// Present when a rolling-window order needs its next chunk and the treasury can't cover it.
    pub topup: Option<Funding>,
    /// Installments kept funded ahead; 0 means funded in full up front.
    pub window: u32,
    pub sweep_txid: Option<String>,
    pub cancelled_at: Option<u64>,
    pub paid_zat: u64,
    /// The payroll batch this order belongs to, if any.
    pub batch: Option<BatchRef>,
    /// `standing`, or `inheritance` for a dead-man's switch.
    pub kind: String,
    /// Inheritance: the reminder lead time before the date.
    pub remind_secs: Option<u64>,
    /// Inheritance: the switch this one renewed.
    pub renewed_from: Option<i64>,
    /// Inheritance: the switch that replaced this one when it was renewed.
    pub renewed_to: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installments: Option<Vec<InstallmentView>>,
}

#[derive(Clone, Serialize)]
pub struct BatchRef {
    pub id: i64,
    /// The payee's name from the CSV row.
    pub name: String,
}

#[derive(Clone, Serialize)]
pub struct InstallmentView {
    pub k: u32,
    pub due_unix: u64,
    pub round: u64,
    pub release_unix: u64,
    pub state: String,
    pub keeper_state: Option<String>,
    pub txids: Vec<String>,
    pub mined_height: Option<u32>,
    pub blob_id: Option<String>,
    /// A fresh copy signed while online shortly before release; it releases first.
    pub fresh_release_unix: Option<u64>,
    pub fresh_txid: Option<String>,
    pub fresh_keeper_state: Option<String>,
    /// Expiry height of the pre-signed (fallback) copy; 0 means it never expires.
    pub expiry_height: Option<u32>,
    pub cancellable: bool,
}

#[derive(Serialize)]
pub struct MayStillPay {
    pub k: u32,
    /// `released` or `race_window`
    pub code: &'static str,
    pub reason: &'static str,
}

#[derive(Serialize)]
pub struct CancelView {
    pub sweep_txid: Option<String>,
    pub cancelled: Vec<u32>,
    pub may_still_pay: Vec<MayStillPay>,
}

#[derive(Serialize)]
pub struct RenewView {
    pub old_id: i64,
    pub new_id: i64,
    /// The one transaction that moved the reservation; null if the old switch was never funded.
    pub txid: Option<String>,
    pub withdrew_blob: bool,
    /// The new switch.
    pub order: OrderView,
}

pub struct NewSwitch {
    pub heir: String,
    pub amount_zat: u64,
    pub at: u64,
    pub memo: Option<String>,
    pub remind_secs: Option<u64>,
    pub keeper: Option<String>,
}

pub struct NewOrder {
    pub payee: String,
    pub amount_zat: u64,
    pub every_secs: u64,
    pub count: u32,
    pub memo: Option<String>,
    pub first_due: Option<u64>,
    pub jitter_max_secs: Option<u64>,
    pub keeper: Option<String>,
}

impl Engine {
    pub fn open(params: SoNetwork, dir: &std::path::Path, lwd_url: String, default_keeper: String) -> Result<Self> {
        let paths = Paths::new(dir);
        let db = wallet::open(&paths, params)?;
        wallet::account(&db)?;
        let store = Store::open(dir.join("so.db"))?;
        Ok(Self {
            params,
            paths,
            db,
            store,
            lwd_url,
            default_keeper,
            chain: None,
        })
    }

    /// Owns its copy of the URL, so the future doesn't borrow the (non-`Sync`) engine.
    pub fn lwd(&self) -> impl std::future::Future<Output = Result<Lwd>> + Send + 'static {
        let url = self.lwd_url.clone();
        async move { net::connect(&url).await }
    }

    async fn chain(&mut self) -> Result<Chain> {
        if let Some(c) = &self.chain {
            return Ok(c.clone());
        }
        let chain = promissory_core::tlock::bounded(|| Drand::quicknet()?.chain()).await?;
        self.chain = Some(chain.clone());
        Ok(chain)
    }

    pub async fn sync(&mut self) -> Result<()> {
        let mut lwd = self.lwd().await?;
        wallet::sync(&mut self.db, &mut lwd).await
    }

    pub async fn info(&mut self) -> Result<Info> {
        let mut lwd = self.lwd().await?;
        let tip = net::tip_height(&mut lwd).await?;
        let treasury = wallet::account(&self.db)?;
        let (synced, total, spendable) = match wallet::balances(&self.db, treasury) {
            Ok(b) => (u32::from(b.fully_scanned), b.ironwood_total, b.ironwood_spendable),
            Err(_) => (0, 0, 0),
        };
        Ok(Info {
            network: match self.params {
                SoNetwork::Main => "main",
                SoNetwork::Regtest(_) => "regtest",
            },
            tip_height: tip.into(),
            synced_height: synced,
            treasury_address: wallet::current_address(&mut self.db, treasury)?,
            treasury_zat: total,
            treasury_spendable_zat: spendable,
            default_keeper: self.default_keeper.clone(),
            cancel_margin_secs: order::CANCEL_MARGIN_SECS,
        })
    }

    fn view(&self, o: &Order, chain: Option<&Chain>, rows: Option<Vec<(Installment, Option<BlobStatus>)>>) -> OrderView {
        let installments = self.store.installments(o.id).unwrap_or_default();
        let paid = installments.iter().filter(|i| i.state == "paid").count() as u64;
        let next = order::next_funding(&self.store, o).ok().flatten();
        let request = |zat: u64, installments: u32| {
            let address = o.funding_address.clone().unwrap_or_default();
            Funding {
                uri: format!("zcash:{address}?amount={}", order::zec_decimal(zat)),
                zat,
                address,
                installments,
            }
        };
        let funding = next
            .filter(|_| o.status == "awaiting_funds")
            .map(|(n, zat)| request(zat, n));
        // A running order whose next chunk the treasury can't cover yet: the payer tops up the
        // shortfall (the treasury's spendable balance counts towards it).
        let spendable = wallet::account(&self.db)
            .and_then(|t| wallet::balances(&self.db, t))
            .map(|b| b.ironwood_spendable)
            .unwrap_or(0);
        let topup = next
            .filter(|(_, zat)| o.status == "active" && spendable < *zat)
            .map(|(n, zat)| request(zat - spendable, n));
        let now = now_unix();
        let meta = self.store.meta(o.id).unwrap_or_default();
        let installments = rows.map(|rows| {
            rows.into_iter()
                .map(|(i, _)| {
                    let release_unix = chain.map(|c| c.round_time(i.round)).unwrap_or(i.due_unix);
                    let fresh_release_unix = i.fresh_round.and_then(|r| chain.map(|c| c.round_time(r)));
                    let earliest = fresh_release_unix.unwrap_or(release_unix).min(release_unix);
                    let out_of_hands = matches!(
                        i.keeper_state.as_deref(),
                        Some("released" | "broadcast" | "mined")
                    );
                    InstallmentView {
                        k: i.k,
                        due_unix: i.due_unix,
                        round: i.round,
                        release_unix,
                        cancellable: matches!(i.state.as_str(), "reserved" | "armed")
                            && !out_of_hands
                            && (i.blob_id.is_none() || earliest > now + order::CANCEL_MARGIN_SECS),
                        txids: i
                            .txids
                            .as_deref()
                            .unwrap_or("")
                            .split(',')
                            .filter_map(|c| c.split_once(':').map(|(_, t)| t.to_string()))
                            .collect(),
                        state: i.state,
                        keeper_state: i.keeper_state,
                        mined_height: i.mined_height,
                        blob_id: i.blob_id,
                        fresh_release_unix,
                        fresh_txid: i.fresh_txid,
                        fresh_keeper_state: i.fresh_keeper_state,
                        expiry_height: i.expiry_height,
                    }
                })
                .collect()
        });
        OrderView {
            id: o.id,
            status: o.status.clone(),
            payee: o.payee.clone(),
            amount_zat: o.amount_zat,
            memo: o.memo.clone(),
            every_secs: o.every_secs,
            count: o.count,
            first_due: o.first_due,
            keeper: o.keeper.clone(),
            split_txid: o.split_txid.clone(),
            created_at: o.created_at,
            funding,
            topup,
            window: o.window,
            sweep_txid: o.sweep_txid.clone(),
            cancelled_at: o.cancelled_at,
            paid_zat: paid * o.amount_zat,
            batch: promissory_core::batch::Batches::open(self.paths.dir.join("so.db"))
                .ok()
                .and_then(|b| b.of_order(o.id).ok().flatten())
                .map(|(id, name)| BatchRef { id, name }),
            kind: meta.kind,
            remind_secs: meta.remind_secs,
            renewed_from: meta.renewed_from,
            renewed_to: meta.renewed_to,
            installments,
        }
    }

    pub fn orders(&self) -> Result<Vec<OrderView>> {
        Ok(self
            .store
            .orders()?
            .iter()
            .rev()
            .map(|o| self.view(o, None, None))
            .collect())
    }

    pub async fn order(&mut self, id: i64) -> Result<OrderView> {
        let chain = self.chain().await.ok();
        let rows = order::refresh(&mut self.store, id).await?;
        let o = self.store.order(id)?;
        Ok(self.view(&o, chain.as_ref(), Some(rows)))
    }

    pub async fn create(&mut self, req: NewOrder) -> Result<OrderView> {
        self.create_windowed(req, 0).await
    }

    /// Like [`Self::create`], keeping only `window` installments funded ahead (0 = all).
    pub async fn create_windowed(&mut self, req: NewOrder, window: u32) -> Result<OrderView> {
        if req.amount_zat == 0 {
            bail!("amount must be greater than zero");
        }
        if req.every_secs < promissory_core::link::MIN_EVERY_SECS {
            bail!("installments must be at least {} seconds apart", promissory_core::link::MIN_EVERY_SECS);
        }
        if !(1..=promissory_core::link::MAX_INSTALLMENTS).contains(&req.count) {
            bail!("count must be between 1 and {}", promissory_core::link::MAX_INSTALLMENTS);
        }
        let schedule = Schedule {
            amount: Zatoshis::from_u64(req.amount_zat)?,
            memo: req.memo.unwrap_or_else(|| "Promissory".into()),
            every_secs: req.every_secs,
            count: req.count,
            first_due: req.first_due.unwrap_or_else(|| now_unix() + 600),
            jitter_max_secs: req.jitter_max_secs.unwrap_or_else(|| self.params.default_jitter_secs()),
        };
        let keeper = req.keeper.unwrap_or_else(|| self.default_keeper.clone());
        let mut lwd = self.lwd().await?;
        let created = order::create_with_window(
            &self.paths,
            &mut self.db,
            &mut self.store,
            &mut lwd,
            &req.payee,
            &schedule,
            &keeper,
            window,
        )
        .await?;
        self.order(created.id).await
    }

    /// Creates an order from a Subscribe link. The link's `hook` is kept only with
    /// `accept_hook`: attaching it tells the keeper the payee's endpoint.
    pub async fn create_from_link(
        &mut self,
        link: &str,
        first_due: Option<u64>,
        keeper: Option<String>,
        accept_hook: bool,
        window: u32,
    ) -> Result<OrderView> {
        let s = Subscribe::parse(link)?;
        if !s.net.matches(&self.params) {
            bail!("this link is for {}, but this app runs on a different network", s.net.as_str());
        }
        let hook = s.hook.clone().filter(|_| accept_hook);
        if let Some(h) = &hook {
            if !promissory_core::notices::valid_hook(h) {
                bail!("parameter `hook` must be an http(s) URL");
            }
        }
        let view = self.create_windowed(NewOrder {
            payee: s.to,
            amount_zat: s.amount_zat,
            every_secs: s.every_secs,
            count: s.count,
            memo: s.memo,
            first_due: first_due.or(s.start),
            jitter_max_secs: None,
            keeper: keeper.or(s.keeper),
        }, window)
        .await?;
        match hook {
            Some(h) => {
                self.store.set_hook(view.id, &h)?;
                self.order(view.id).await
            }
            None => Ok(view),
        }
    }

    pub async fn cancel(&mut self, id: i64, margin_secs: Option<u64>, keep_blobs: bool, to: Option<String>) -> Result<CancelView> {
        let mut lwd = self.lwd().await?;
        let c = order::cancel(
            &self.paths,
            &mut self.db,
            &mut self.store,
            &mut lwd,
            id,
            to.as_deref(),
            margin_secs.unwrap_or(order::CANCEL_MARGIN_SECS),
            !keep_blobs,
        )
        .await?;
        Ok(CancelView {
            sweep_txid: c.sweep.map(|t| t.to_string()),
            cancelled: c.cancelled,
            may_still_pay: c
                .may_still_pay
                .into_iter()
                .map(|(k, why)| MayStillPay {
                    k,
                    code: why.code(),
                    reason: why.reason(),
                })
                .collect(),
        })
    }

    /// Creates a dead-man's switch awaiting funds.
    pub async fn create_switch(&mut self, req: NewSwitch) -> Result<OrderView> {
        if req.amount_zat == 0 {
            bail!("amount must be greater than zero");
        }
        let keeper = req.keeper.unwrap_or_else(|| self.default_keeper.clone());
        let mut lwd = self.lwd().await?;
        let created = order::create_inheritance(
            &self.paths,
            &mut self.db,
            &mut self.store,
            &mut lwd,
            &req.heir,
            Zatoshis::from_u64(req.amount_zat)?,
            req.memo.as_deref().unwrap_or("Inheritance"),
            req.at,
            req.remind_secs.unwrap_or(order::SWITCH_REMIND_SECS),
            &keeper,
        )
        .await?;
        self.order(created.id).await
    }

    /// Renews a switch to a later date in one transaction; see `order::renew`.
    pub async fn renew_switch(&mut self, id: i64, at: u64, margin_secs: Option<u64>, keep_blob: bool) -> Result<RenewView> {
        let mut lwd = self.lwd().await?;
        let r = order::renew(
            &self.paths,
            &mut self.db,
            &mut self.store,
            &mut lwd,
            id,
            at,
            margin_secs.unwrap_or(order::CANCEL_MARGIN_SECS),
            !keep_blob,
        )
        .await?;
        Ok(RenewView {
            old_id: r.old_id,
            new_id: r.new_id,
            txid: r.txid.map(|t| t.to_string()),
            withdrew_blob: r.withdrew_blob,
            order: self.order(r.new_id).await?,
        })
    }

    /// First half of a progression pass: sync, then fund every order the treasury covers.
    /// Returns the orders to step through with [`Self::step_order`]. Each half is short, so the
    /// caller can release the engine lock between them and API requests are not starved.
    pub async fn begin_pass(&mut self) -> Result<Vec<i64>> {
        self.sync().await?;
        let treasury = wallet::account(&self.db)?;
        if let Err(e) = self.fund_covered(treasury).await {
            tracing::warn!("funding pass failed: {e:#}");
        }
        Ok(self.store.orders()?.into_iter().map(|o| o.id).collect())
    }

    /// One order's share of a progression pass: arm split orders; for active ones also sign
    /// fresh copies and refresh keeper state.
    pub async fn step_order(&mut self, id: i64) -> Result<()> {
        {
            let o = self.store.order(id)?;
            let result = match o.status.as_str() {
                "awaiting_funds" => Ok(()),
                "splitting" => self.arm(o.id).await,
                "active" => {
                    let armed = self.arm(o.id).await;
                    let mut lwd = self.lwd().await?;
                    match order::freshen(&self.paths, &mut self.db, &mut self.store, &mut lwd, o.id).await {
                        Ok(ks) if !ks.is_empty() => {
                            tracing::info!(order = o.id, installments = ?ks, "fresh copies signed")
                        }
                        Ok(_) => {}
                        Err(e) => tracing::warn!(order = o.id, "freshen failed: {e:#}"),
                    }
                    armed.and(order::refresh(&mut self.store, o.id).await.map(|_| ()))
                }
                "cancelling" => order::refresh(&mut self.store, o.id).await.map(|_| ()),
                _ => Ok(()),
            };
            if let Err(e) = result {
                tracing::warn!(order = o.id, status = %o.status, "step failed: {e:#}");
            }
        }
        Ok(())
    }
}

impl Engine {
    /// Funds, in one split transaction, every order whose next chunk the treasury can cover:
    /// new orders and rolling-window top-ups alike, oldest first.
    async fn fund_covered(&mut self, treasury: promissory_core::wallet::AccountUuid) -> Result<()> {
        let mut spendable = wallet::balances(&self.db, treasury)?.ironwood_spendable;
        let mut ids = vec![];
        for o in self.store.orders()? {
            if let Some((_, need)) = order::next_funding(&self.store, &o)? {
                if need <= spendable {
                    spendable -= need;
                    ids.push(o.id);
                }
            }
        }
        if ids.is_empty() {
            return Ok(());
        }
        let mut lwd = self.lwd().await?;
        let txid = order::split_many(&self.paths, &mut self.db, &mut self.store, &mut lwd, &ids).await?;
        tracing::info!(orders = ?ids, %txid, "split broadcast");
        Ok(())
    }

    /// Arms whatever installments have spendable notes.
    async fn arm(&mut self, id: i64) -> Result<()> {
        let mut lwd = self.lwd().await?;
        let armed = order::arm(&self.paths, &mut self.db, &mut self.store, &mut lwd, id).await?;
        if !armed.is_empty() {
            tracing::info!(order = id, installments = armed.len(), "armed");
        }
        Ok(())
    }
}

/// Maps an engine error to an API error message.
pub fn message(e: &anyhow::Error) -> String {
    format!("{e:#}")
}

