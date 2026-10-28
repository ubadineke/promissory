//! Agent allowances: "up to X per period" for an AI agent's own wallet (docs/allowances.md).
//!
//! An allowance is one ordinary standing order to the agent's address, created through the
//! same engine path as `POST /orders`. The period's cap is cut into `chips` equal chips and the
//! order releases one chip every `period / chips` seconds (a drip):
//!
//! - `amount_zat` = cap / chips, `every_secs` = period / chips, `count` = chips × periods;
//! - `window` = chips, so only one period's worth of chips is funded at a time. The engine
//!   splits each refill from the treasury as earlier chips are paid.
//!
//! A chip released to the agent is the agent's money; nothing is ever swept back from the
//! agent's wallet. "Unused" means not yet released: stopping the allowance cancels the order,
//! which sweeps every chip still reserved back to the treasury.
//!
//! Allowances live in `so.db` beside the orders, in their own table `allowances`, which links
//! each one to its order. Pending agent requests live in `agent_requests` ([`requests`]).

pub mod requests;

use std::path::Path as FsPath;

use anyhow::{anyhow, bail, Result};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use promissory_core::{link::MEMO_MAX_BYTES, order::now_unix};

use crate::{
    engine::{CancelView, Engine, Funding, NewOrder, OrderView},
    ApiError, ApiResult, Shared,
};

/// Most chips one period may be cut into.
pub const MAX_CHIPS: u32 = 30;
/// Smallest chip: each chip pays its own 10,000 zat network fee, so smaller chips waste funds.
pub const MIN_CHIP_ZAT: u64 = 100_000;
/// A chip's spacing (`period / chips`) must be at least this, like any installment spacing.
pub const MIN_SPACING_SECS: u64 = promissory_core::link::MIN_EVERY_SECS;
/// Every chip of every period is one installment of the order.
pub const MAX_TOTAL_CHIPS: u32 = promissory_core::link::MAX_INSTALLMENTS;
pub const LABEL_MAX_CHARS: usize = 100;
const MAX_MONEY_ZAT: u64 = 21_000_000 * 100_000_000;
/// Memo of an allowance without a label. The order appends ` (k/n)`.
pub const DEFAULT_MEMO: &str = "Agent allowance";
/// A replacement allowance starts no sooner than this, so its chips can be funded and armed.
pub const MIN_LEAD_SECS: u64 = 300;

/// How an allowance maps onto a standing order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub chip_zat: u64,
    /// Seconds between chips: `period / chips`.
    pub every_secs: u64,
    /// Installments of the order: `chips × periods`.
    pub count: u32,
    /// Rolling window of the order: one period's chips.
    pub window: u32,
}

/// Validates an allowance's shape and maps it onto an order.
pub fn plan(cap_zat: u64, period_secs: u64, chips: u32, periods: u32) -> Result<Plan> {
    if !(1..=MAX_CHIPS).contains(&chips) {
        bail!("chips must be between 1 and {MAX_CHIPS}");
    }
    if periods == 0 {
        bail!("periods must be at least 1");
    }
    let count = chips
        .checked_mul(periods)
        .filter(|c| *c <= MAX_TOTAL_CHIPS)
        .ok_or_else(|| anyhow!("chips × periods must be at most {MAX_TOTAL_CHIPS}"))?;
    if cap_zat == 0 || cap_zat > MAX_MONEY_ZAT {
        bail!("cap_zat must be between 1 and {MAX_MONEY_ZAT}");
    }
    if !cap_zat.is_multiple_of(u64::from(chips)) {
        bail!("cap_zat must be a multiple of chips ({chips}), so every chip is the same size");
    }
    let chip_zat = cap_zat / u64::from(chips);
    if chip_zat < MIN_CHIP_ZAT {
        bail!("each chip (cap_zat / chips = {chip_zat} zat) must be at least {MIN_CHIP_ZAT} zat");
    }
    if !period_secs.is_multiple_of(u64::from(chips)) {
        bail!("period_secs must be a multiple of chips ({chips}), so chips are evenly spaced");
    }
    let every_secs = period_secs / u64::from(chips);
    if every_secs < MIN_SPACING_SECS {
        bail!("period_secs / chips must be at least {MIN_SPACING_SECS} seconds");
    }
    Ok(Plan {
        chip_zat,
        every_secs,
        count,
        window: chips,
    })
}

/// A trimmed label, checked against the length limits (it is also the on-chain memo).
pub fn clean_label(label: Option<String>) -> Result<String> {
    let label = label.unwrap_or_default().trim().to_string();
    if label.chars().count() > LABEL_MAX_CHARS {
        bail!("label is longer than {LABEL_MAX_CHARS} characters");
    }
    if label.len() > MEMO_MAX_BYTES {
        bail!("label is longer than {MEMO_MAX_BYTES} bytes");
    }
    if label.chars().any(char::is_control) {
        bail!("label must not contain control characters");
    }
    Ok(label)
}

// ---------------------------------------------------------------------------------------------
// Storage

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Allowance {
    pub id: i64,
    pub order_id: i64,
    pub agent_ua: String,
    pub cap_zat: u64,
    pub period_secs: u64,
    pub chips: u32,
    pub periods: u32,
    pub first_due: u64,
    pub label: String,
    pub created_at: u64,
    pub stopped_at: Option<u64>,
    /// Set when a cap change replaced this allowance with a new one.
    pub replaced_by: Option<i64>,
}

pub struct Allowances {
    conn: Connection,
}

const COLUMNS: &str = "id, order_id, agent_ua, cap_zat, period_secs, chips, periods, first_due, label, \
                       created_at, stopped_at, replaced_by";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Allowance> {
    Ok(Allowance {
        id: r.get(0)?,
        order_id: r.get(1)?,
        agent_ua: r.get(2)?,
        cap_zat: r.get(3)?,
        period_secs: r.get(4)?,
        chips: r.get(5)?,
        periods: r.get(6)?,
        first_due: r.get(7)?,
        label: r.get(8)?,
        created_at: r.get(9)?,
        stopped_at: r.get(10)?,
        replaced_by: r.get(11)?,
    })
}

impl Allowances {
    pub fn open(path: impl AsRef<FsPath>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS allowances (
                id INTEGER PRIMARY KEY,
                order_id INTEGER NOT NULL UNIQUE,
                agent_ua TEXT NOT NULL,
                cap_zat INTEGER NOT NULL,
                period_secs INTEGER NOT NULL,
                chips INTEGER NOT NULL,
                periods INTEGER NOT NULL,
                first_due INTEGER NOT NULL,
                label TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                stopped_at INTEGER,
                replaced_by INTEGER
            );",
        )?;
        Ok(Self { conn })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert(
        &self,
        order_id: i64,
        agent_ua: &str,
        cap_zat: u64,
        period_secs: u64,
        chips: u32,
        periods: u32,
        first_due: u64,
        label: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO allowances (order_id, agent_ua, cap_zat, period_secs, chips, periods,
                                     first_due, label, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![order_id, agent_ua, cap_zat, period_secs, chips, periods, first_due, label, now_unix()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get(&self, id: i64) -> Result<Option<Allowance>> {
        Ok(self
            .conn
            .query_row(&format!("SELECT {COLUMNS} FROM allowances WHERE id = ?1"), [id], row)
            .optional()?)
    }

    /// Newest first.
    pub fn list(&self) -> Result<Vec<Allowance>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {COLUMNS} FROM allowances ORDER BY id DESC"))?;
        let rows = stmt.query_map([], row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_stopped(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE allowances SET stopped_at = COALESCE(stopped_at, ?2) WHERE id = ?1",
            params![id, now_unix()],
        )?;
        Ok(())
    }

    pub fn set_replaced_by(&self, id: i64, new_id: i64) -> Result<()> {
        self.conn
            .execute("UPDATE allowances SET replaced_by = ?2 WHERE id = ?1", params![id, new_id])?;
        Ok(())
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM allowances WHERE id = ?1", [id])?;
        Ok(())
    }
}

pub fn open(e: &Engine) -> Result<Allowances> {
    Allowances::open(e.paths.dir.join("so.db"))
}

// ---------------------------------------------------------------------------------------------
// Status

/// One chip as the status needs it.
#[derive(Clone, Debug)]
pub struct Chip {
    pub k: u32,
    pub release_unix: u64,
    /// The installment state: `reserved`, `armed`, `paid` or `cancelled`.
    pub state: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Progress {
    /// 1-based; the period now running, the first one before it starts, the last one after.
    pub current_period: u32,
    pub period_start: u64,
    pub period_end: u64,
    /// Released to the agent this period: spent from the allowance. What the agent then does
    /// with its own money is not visible to the payer.
    pub spent_this_period_zat: u64,
    /// Chips of this period not yet released (and not cancelled).
    pub remaining_this_period_zat: u64,
    pub released_total_zat: u64,
    /// Release time of the next chip still reserved or armed.
    pub next_release_unix: Option<u64>,
}

pub fn progress(a: &Allowance, chips: &[Chip], now: u64) -> Progress {
    let chip_zat = a.cap_zat / u64::from(a.chips.max(1));
    let elapsed = now.saturating_sub(a.first_due) / a.period_secs.max(1);
    let p = (elapsed.min(u64::from(a.periods.max(1) - 1))) as u32;
    let in_period = |c: &&Chip| c.k / a.chips.max(1) == p;
    let pending = |c: &Chip| c.state == "reserved" || c.state == "armed";
    let period_start = a.first_due + u64::from(p) * a.period_secs;
    Progress {
        current_period: p + 1,
        period_start,
        period_end: period_start + a.period_secs,
        spent_this_period_zat: chips.iter().filter(in_period).filter(|c| c.state == "paid").count() as u64 * chip_zat,
        remaining_this_period_zat: chips.iter().filter(in_period).filter(|c| pending(c)).count() as u64 * chip_zat,
        released_total_zat: chips.iter().filter(|c| c.state == "paid").count() as u64 * chip_zat,
        next_release_unix: chips.iter().filter(|c| pending(c)).map(|c| c.release_unix).min(),
    }
}

/// Allowance status from its order's status.
pub fn status_of(order_status: &str, replaced: bool) -> &'static str {
    match order_status {
        "awaiting_funds" => "awaiting_funds",
        "splitting" => "starting",
        "active" => "active",
        "completed" => "completed",
        "cancelling" if replaced => "replacing",
        "cancelling" => "stopping",
        _ if replaced => "replaced",
        _ => "stopped",
    }
}

#[derive(Clone, Serialize)]
pub struct AllowanceView {
    pub id: i64,
    pub order_id: i64,
    /// `awaiting_funds`, `starting`, `active`, `completed`, `stopping`, `stopped`,
    /// `replacing` or `replaced`.
    pub status: &'static str,
    pub agent_ua: String,
    pub label: String,
    pub cap_zat: u64,
    pub period_secs: u64,
    pub chips: u32,
    pub chip_zat: u64,
    /// Seconds between chips.
    pub every_secs: u64,
    pub periods: u32,
    pub first_due: u64,
    pub created_at: u64,
    pub stopped_at: Option<u64>,
    pub replaced_by: Option<i64>,
    #[serde(flatten)]
    pub progress: Progress,
    /// What to pay to start the allowance; null once funded.
    pub funding: Option<Funding>,
    /// The next period's chips, when the treasury can't cover them.
    pub topup: Option<Funding>,
    pub keeper: String,
}

pub fn view(a: &Allowance, o: &OrderView, chips: &[Chip], now: u64) -> AllowanceView {
    AllowanceView {
        id: a.id,
        order_id: a.order_id,
        status: status_of(&o.status, a.replaced_by.is_some()),
        agent_ua: a.agent_ua.clone(),
        label: a.label.clone(),
        cap_zat: a.cap_zat,
        period_secs: a.period_secs,
        chips: a.chips,
        chip_zat: a.cap_zat / u64::from(a.chips.max(1)),
        every_secs: o.every_secs,
        periods: a.periods,
        first_due: a.first_due,
        created_at: a.created_at,
        stopped_at: a.stopped_at,
        replaced_by: a.replaced_by,
        progress: progress(a, chips, now),
        funding: o.funding.clone(),
        topup: o.topup.clone(),
        keeper: o.keeper.clone(),
    }
}

/// The allowance's view with keeper state refreshed.
pub async fn view_fresh(e: &mut Engine, a: &Allowance) -> Result<AllowanceView> {
    let o = e.order(a.order_id).await?;
    let chips: Vec<Chip> = o
        .installments
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|i| Chip {
            k: i.k,
            release_unix: i.release_unix,
            state: i.state.clone(),
        })
        .collect();
    Ok(view(a, &o, &chips, now_unix()))
}

/// The allowance's view from local state only (no network). A chip's release time is its due
/// time: allowances have no jitter, so the round is published within 3 s of it.
fn view_local(e: &Engine, a: &Allowance, o: &OrderView) -> Result<AllowanceView> {
    let chips: Vec<Chip> = e
        .store
        .installments(a.order_id)?
        .into_iter()
        .map(|i| Chip {
            k: i.k,
            release_unix: i.due_unix,
            state: i.state,
        })
        .collect();
    Ok(view(a, o, &chips, now_unix()))
}

// ---------------------------------------------------------------------------------------------
// Engine operations, shared with the request approvals

pub struct NewAllowance {
    pub agent_ua: String,
    pub cap_zat: u64,
    pub period_secs: u64,
    pub chips: u32,
    pub periods: u32,
    pub first_due: Option<u64>,
    pub label: Option<String>,
    pub keeper: Option<String>,
}

/// Creates the allowance's order (awaiting funds) and records the allowance.
pub async fn create_allowance(e: &mut Engine, req: NewAllowance) -> Result<AllowanceView> {
    let p = plan(req.cap_zat, req.period_secs, req.chips, req.periods)?;
    let label = clean_label(req.label)?;
    let now = now_unix();
    let first_due = req.first_due.unwrap_or(now + 600);
    if first_due < now + 60 {
        bail!("first_due must be at least a minute from now");
    }
    let order = e
        .create_windowed(
            NewOrder {
                payee: req.agent_ua.clone(),
                amount_zat: p.chip_zat,
                every_secs: p.every_secs,
                count: p.count,
                memo: Some(if label.is_empty() { DEFAULT_MEMO.to_string() } else { label.clone() }),
                first_due: Some(first_due),
                jitter_max_secs: None,
                keeper: req.keeper,
            },
            p.window,
        )
        .await?;
    let store = open(e)?;
    let id = match store.insert(
        order.id,
        &req.agent_ua,
        req.cap_zat,
        req.period_secs,
        req.chips,
        req.periods,
        first_due,
        &label,
    ) {
        Ok(id) => id,
        Err(err) => {
            // Nothing was funded yet: cancelling only marks the order cancelled.
            let _ = e.cancel(order.id, None, false, None).await;
            return Err(err);
        }
    };
    let a = store.get(id)?.ok_or_else(|| anyhow!("allowance {id} vanished"))?;
    view_fresh(e, &a).await
}

/// Stops an allowance: cancels its order, sweeping every chip not yet released back to the
/// treasury. Chips inside the race window, or already out at the keeper, may still pay.
pub async fn stop_allowance(e: &mut Engine, a: &Allowance, margin_secs: Option<u64>) -> Result<CancelView> {
    let status = e.store.order(a.order_id)?.status;
    match status.as_str() {
        "completed" => bail!("allowance {} is completed: every chip was released", a.id),
        "cancelled" => bail!("allowance {} is already stopped", a.id),
        _ => {}
    }
    let c = e.cancel(a.order_id, margin_secs, false, None).await?;
    open(e)?.set_stopped(a.id)?;
    Ok(c)
}

/// When a replacement for `a` starts, and how many of `a`'s periods it still covers: the next
/// period boundary (or `a`'s start if it has not begun), never sooner than [`MIN_LEAD_SECS`].
pub fn replacement_schedule(a: &Allowance, now: u64) -> Result<(u64, u32)> {
    if now + MIN_LEAD_SECS <= a.first_due {
        return Ok((a.first_due, a.periods));
    }
    let next = (now.saturating_sub(a.first_due) / a.period_secs.max(1) + 1) as u32;
    if now < a.first_due {
        // Starts too soon to replace in place: the replacement begins one lead time later.
        return Ok((now + MIN_LEAD_SECS, a.periods));
    }
    if next >= a.periods {
        bail!("allowance {} is in its last period; there is no later period to change", a.id);
    }
    let start = a.first_due + u64::from(next) * a.period_secs;
    Ok((start.max(now + MIN_LEAD_SECS), a.periods - next))
}

/// Replaces `a` with an allowance capped at `new_cap_zat` from the next period on, then stops
/// `a`. The order is fixed at creation, so a cap change is a cancel and a new order.
pub async fn change_cap(e: &mut Engine, a: &Allowance, new_cap_zat: u64) -> Result<AllowanceView> {
    if a.stopped_at.is_some() || a.replaced_by.is_some() {
        bail!("allowance {} is stopped", a.id);
    }
    if new_cap_zat == a.cap_zat {
        bail!("the new cap equals the current cap");
    }
    plan(new_cap_zat, a.period_secs, a.chips, 1)?;
    let (first_due, periods) = replacement_schedule(a, now_unix())?;
    let keeper = e.store.order(a.order_id)?.keeper;
    let new = create_allowance(
        e,
        NewAllowance {
            agent_ua: a.agent_ua.clone(),
            cap_zat: new_cap_zat,
            period_secs: a.period_secs,
            chips: a.chips,
            periods,
            first_due: Some(first_due),
            label: Some(a.label.clone()),
            keeper: Some(keeper),
        },
    )
    .await?;
    if let Err(err) = stop_allowance(e, a, None).await {
        // Keep one allowance running, not two: undo the unfunded replacement.
        let _ = e.cancel(new.order_id, None, false, None).await;
        let _ = open(e)?.delete(new.id);
        return Err(err.context(format!("could not stop allowance {}", a.id)));
    }
    open(e)?.set_replaced_by(a.id, new.id)?;
    Ok(new)
}

// ---------------------------------------------------------------------------------------------
// HTTP handlers

fn not_found(id: i64) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, format!("no allowance {id}"))
}

fn get_allowance(e: &Engine, id: i64) -> Result<Allowance, ApiError> {
    open(e)?.get(id)?.ok_or_else(|| not_found(id))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBody {
    agent_ua: String,
    cap_zat: u64,
    period_secs: u64,
    chips: u32,
    periods: u32,
    first_due: Option<u64>,
    label: Option<String>,
    keeper: Option<String>,
}

/// `POST /allowances`: creates the allowance's order, awaiting funds.
pub async fn create(State(e): State<Shared>, Json(b): Json<CreateBody>) -> ApiResult<AllowanceView> {
    let mut e = e.lock().await;
    let view = create_allowance(
        &mut e,
        NewAllowance {
            agent_ua: b.agent_ua,
            cap_zat: b.cap_zat,
            period_secs: b.period_secs,
            chips: b.chips,
            periods: b.periods,
            first_due: b.first_due,
            label: b.label,
            keeper: b.keeper,
        },
    )
    .await?;
    tracing::info!(allowance = view.id, order = view.order_id, "allowance created");
    Ok(Json(view))
}

/// `GET /allowances`: newest first, from local state (no keeper refresh).
pub async fn list(State(e): State<Shared>) -> ApiResult<Vec<AllowanceView>> {
    let e = e.lock().await;
    let all = open(&e)?.list()?;
    let orders = e.orders()?;
    let mut out = vec![];
    for a in &all {
        if let Some(o) = orders.iter().find(|o| o.id == a.order_id) {
            out.push(view_local(&e, a, o)?);
        }
    }
    Ok(Json(out))
}

/// `GET /allowances/{id}`: keeper state refreshed.
pub async fn get(State(e): State<Shared>, Path(id): Path<i64>) -> ApiResult<AllowanceView> {
    let mut e = e.lock().await;
    let a = get_allowance(&e, id)?;
    Ok(Json(view_fresh(&mut e, &a).await?))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct StopBody {
    margin_secs: Option<u64>,
}

#[derive(Serialize)]
pub struct Stopped {
    allowance: AllowanceView,
    #[serde(flatten)]
    cancel: CancelView,
}

/// `POST /allowances/{id}/stop`: cancels the order; unreleased chips go back to the treasury.
pub async fn stop(State(e): State<Shared>, Path(id): Path<i64>, body: Option<Json<StopBody>>) -> ApiResult<Stopped> {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let mut e = e.lock().await;
    let a = get_allowance(&e, id)?;
    let cancel = stop_allowance(&mut e, &a, b.margin_secs).await?;
    let a = get_allowance(&e, id)?;
    tracing::info!(allowance = id, order = a.order_id, "allowance stopped");
    Ok(Json(Stopped {
        allowance: view_fresh(&mut e, &a).await?,
        cancel,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowance() -> Allowance {
        Allowance {
            id: 1,
            order_id: 9,
            agent_ua: "uregtest1agent".into(),
            cap_zat: 3_000_000,
            period_secs: 600,
            chips: 3,
            periods: 2,
            first_due: 10_000,
            label: String::new(),
            created_at: 9_000,
            stopped_at: None,
            replaced_by: None,
        }
    }

    fn chips(states: &[&str]) -> Vec<Chip> {
        states
            .iter()
            .enumerate()
            .map(|(k, s)| Chip {
                k: k as u32,
                release_unix: 10_000 + 200 * k as u64,
                state: s.to_string(),
            })
            .collect()
    }

    #[test]
    fn plan_maps_onto_an_order() {
        let p = plan(3_000_000, 600, 3, 2).unwrap();
        assert_eq!(
            p,
            Plan {
                chip_zat: 1_000_000,
                every_secs: 200,
                count: 6,
                window: 3
            }
        );
        assert_eq!(plan(500_000, 86_400, 1, 120).unwrap().count, 120);
    }

    #[test]
    fn plan_rejects_bad_shapes() {
        let err = |r: Result<Plan>| r.unwrap_err().to_string();
        assert!(err(plan(3_000_000, 600, 0, 2)).contains("chips must be between"));
        assert!(err(plan(3_000_000, 600, 31, 2)).contains("chips must be between"));
        assert!(err(plan(3_000_000, 600, 3, 0)).contains("periods must be at least 1"));
        assert!(err(plan(3_000_000, 6_000, 3, 41)).contains("at most 120"));
        assert!(err(plan(0, 600, 3, 2)).contains("cap_zat must be between"));
        assert!(err(plan(3_000_001, 600, 3, 2)).contains("multiple of chips"));
        assert!(err(plan(300_000, 600, 6, 2)).contains("at least 100000 zat"));
        assert!(err(plan(3_000_000, 601, 3, 2)).contains("period_secs must be a multiple"));
        assert!(err(plan(3_000_000, 150, 3, 2)).contains("at least 60 seconds"));
        assert!(err(plan(u64::MAX - 1, 600, 2, 1)).contains("cap_zat must be between"));
        assert!(err(plan(3_000_000, 600, u32::MAX, u32::MAX)).contains("chips must be between"));
        assert!(err(plan(3_000_000, 600, 30, u32::MAX)).contains("at most 120"));
    }

    #[test]
    fn labels_are_checked() {
        assert_eq!(clean_label(None).unwrap(), "");
        assert_eq!(clean_label(Some("  Research agent ".into())).unwrap(), "Research agent");
        assert!(clean_label(Some("x".repeat(101))).is_err());
        assert!(clean_label(Some("a\u{0}b".into())).is_err());
    }

    #[test]
    fn progress_before_during_and_after() {
        let a = allowance();
        // Before the first period: period 1, nothing spent, the whole cap remaining.
        let p = progress(&a, &chips(&["armed", "armed", "armed", "reserved", "reserved", "reserved"]), 9_500);
        assert_eq!((p.current_period, p.period_start, p.period_end), (1, 10_000, 10_600));
        assert_eq!((p.spent_this_period_zat, p.remaining_this_period_zat), (0, 3_000_000));
        assert_eq!(p.next_release_unix, Some(10_000));

        // Inside period 1, two chips paid.
        let p = progress(&a, &chips(&["paid", "paid", "armed", "reserved", "reserved", "reserved"]), 10_450);
        assert_eq!((p.spent_this_period_zat, p.remaining_this_period_zat), (2_000_000, 1_000_000));
        assert_eq!(p.next_release_unix, Some(10_400));

        // Period 2, after a stop that cancelled the last two chips.
        let p = progress(&a, &chips(&["paid", "paid", "paid", "paid", "cancelled", "cancelled"]), 10_700);
        assert_eq!((p.current_period, p.period_start), (2, 10_600));
        assert_eq!((p.spent_this_period_zat, p.remaining_this_period_zat), (1_000_000, 0));
        assert_eq!(p.released_total_zat, 4_000_000);
        assert_eq!(p.next_release_unix, None);

        // Long after the end: stays on the last period.
        let p = progress(&a, &chips(&["paid"; 6]), 99_999);
        assert_eq!(p.current_period, 2);
        assert_eq!(p.released_total_zat, 6_000_000);
    }

    #[test]
    fn statuses() {
        assert_eq!(status_of("splitting", false), "starting");
        assert_eq!(status_of("cancelling", false), "stopping");
        assert_eq!(status_of("cancelled", false), "stopped");
        assert_eq!(status_of("cancelled", true), "replaced");
        assert_eq!(status_of("cancelling", true), "replacing");
        assert_eq!(status_of("completed", false), "completed");
    }

    #[test]
    fn replacement_starts_at_the_next_period() {
        let a = allowance();
        // Not started and far enough ahead: replaces in place.
        assert_eq!(replacement_schedule(&a, 9_000).unwrap(), (10_000, 2));
        // Starts too soon: shifted by the lead time.
        assert_eq!(replacement_schedule(&a, 9_900).unwrap(), (10_200, 2));
        // In period 1: from period 2, which is far enough ahead.
        assert_eq!(replacement_schedule(&a, 10_100).unwrap(), (10_600, 1));
        // In period 1 but close to its end: no sooner than the lead time.
        assert_eq!(replacement_schedule(&a, 10_500).unwrap(), (10_800, 1));
        // In the last period: nothing left to change.
        assert!(replacement_schedule(&a, 10_700).is_err());
    }

    #[test]
    fn store_round_trip() {
        let s = Allowances::open(":memory:").unwrap();
        let id = s.insert(4, "uregtest1agent", 3_000_000, 600, 3, 2, 10_000, "Bot").unwrap();
        let id2 = s.insert(5, "uregtest1agent", 6_000_000, 600, 3, 1, 10_600, "").unwrap();
        let a = s.get(id).unwrap().unwrap();
        assert_eq!((a.order_id, a.cap_zat, a.chips, a.label.as_str()), (4, 3_000_000, 3, "Bot"));
        assert_eq!(a.stopped_at, None);
        s.set_stopped(id).unwrap();
        s.set_replaced_by(id, id2).unwrap();
        let a = s.get(id).unwrap().unwrap();
        assert!(a.stopped_at.is_some());
        assert_eq!(a.replaced_by, Some(id2));
        assert_eq!(s.list().unwrap().iter().map(|a| a.id).collect::<Vec<_>>(), vec![id2, id]);
        // One allowance per order.
        assert!(s.insert(4, "uregtest1agent", 3_000_000, 600, 3, 2, 10_000, "").is_err());
        s.delete(id2).unwrap();
        assert!(s.get(id2).unwrap().is_none());
    }
}
