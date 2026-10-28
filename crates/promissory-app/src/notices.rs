//! Payer-side notices (docs/api.md, "Notices"): banners derived from order state.
//!
//! - `renewal_paid`: an installment was mined (shown for a day after it was due).
//! - `payment_failed_cancelled`: an order's remaining installments can no longer pay because
//!   its reserved notes were swept.
//! - `keeper_unreachable`: an order with installments at a keeper that does not answer
//!   `/v1/health`. Not dismissible; it clears when the keeper answers again.
//! - `funds_low`: reserved for rolling-window top-ups; see [`funds_low`].
//! - `switch_due`: a dead-man's switch is within its reminder window ("renew it or it pays
//!   the heir"). It clears when the switch is renewed (the new one has a later date).
//!
//! Dismissals are kept in `<dir>/notices.json`, beside the order store.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    time::Duration,
};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use promissory_core::{
    notices,
    order::{now_unix, zec_decimal},
    store::{Installment, Order},
};

use crate::{ApiError, ApiResult, Shared};

/// How long a "renewal paid" notice stays up after the installment was due.
const PAID_NOTICE_SECS: u64 = 86_400;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Notice {
    /// Stable ID, e.g. `paid:3:0`; pass it to `dismiss`.
    pub id: String,
    /// `renewal_paid` | `payment_failed_cancelled` | `keeper_unreachable` | `funds_low` |
    /// `switch_due`
    pub kind: &'static str,
    /// `ok` | `warn` | `danger`
    pub tone: &'static str,
    pub title: String,
    pub message: String,
    pub order_id: Option<i64>,
    pub k: Option<u32>,
    pub at: u64,
    pub dismissible: bool,
}

fn short(addr: &str) -> String {
    if addr.len() <= 20 {
        addr.to_string()
    } else {
        format!("{}…{}", &addr[..10], &addr[addr.len() - 6..])
    }
}

/// The parts of an order notices look at. Kept apart from `store::Order`, which is still
/// growing, so this logic and its tests don't depend on its exact shape.
pub struct OrderFacts {
    pub id: i64,
    pub status: String,
    pub payee: String,
    pub amount_zat: u64,
    pub count: u32,
    pub keeper: String,
    pub cancelled_at: Option<u64>,
    /// `standing` or `inheritance`.
    pub kind: String,
    pub first_due: u64,
    /// Inheritance: remind this long before `first_due`.
    pub remind_secs: Option<u64>,
    /// Inheritance: a later switch replaced this one.
    pub renewed: bool,
}

impl OrderFacts {
    /// Adds what `store::Order` does not carry: the kind and renewal links.
    pub fn with_meta(mut self, meta: &promissory_core::store::OrderMeta) -> Self {
        self.kind = meta.kind.clone();
        self.remind_secs = meta.remind_secs;
        self.renewed = meta.renewed_to.is_some();
        self
    }
}

pub struct InstFacts {
    pub k: u32,
    pub state: String,
    pub due_unix: u64,
    pub keeper_state: Option<String>,
    pub mined_height: Option<u32>,
    pub armed_blob: bool,
}

impl From<&Order> for OrderFacts {
    fn from(o: &Order) -> Self {
        Self {
            id: o.id,
            status: o.status.clone(),
            payee: o.payee.clone(),
            amount_zat: o.amount_zat,
            count: o.count,
            keeper: o.keeper.clone(),
            cancelled_at: o.cancelled_at,
            kind: promissory_core::order::KIND_STANDING.into(),
            first_due: o.first_due,
            remind_secs: None,
            renewed: false,
        }
    }
}

/// `2026-11-02 14:05 UTC`.
fn utc(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", secs / 3_600, secs % 3_600 / 60)
}

/// A switch inside its reminder window: renew it, or it pays the heir at its date.
fn switch_due(o: &OrderFacts, insts: &[InstFacts], now: u64) -> Option<Notice> {
    if o.kind != promissory_core::order::KIND_INHERITANCE
        || o.renewed
        || !matches!(o.status.as_str(), "awaiting_funds" | "splitting" | "active")
        || insts.iter().any(|i| i.state == "paid" || i.state == "cancelled")
    {
        return None;
    }
    let remind = o.remind_secs.unwrap_or(promissory_core::order::SWITCH_REMIND_SECS);
    if now >= o.first_due || now + remind < o.first_due {
        return None;
    }
    let left = o.first_due - now;
    Some(Notice {
        id: format!("switch_due:{}", o.id),
        kind: "switch_due",
        tone: if left <= 3_600 { "danger" } else { "warn" },
        title: format!("Dead-man's switch #{}: check in", o.id),
        message: format!(
            "Renew your switch or it pays {} ZEC to {} on {}.",
            zec_decimal(o.amount_zat),
            short(&o.payee),
            utc(o.first_due)
        ),
        order_id: Some(o.id),
        k: None,
        at: o.first_due.saturating_sub(remind),
        dismissible: true,
    })
}

impl From<&Installment> for InstFacts {
    fn from(i: &Installment) -> Self {
        Self {
            k: i.k,
            state: i.state.clone(),
            due_unix: i.due_unix,
            keeper_state: i.keeper_state.clone(),
            mined_height: i.mined_height,
            armed_blob: i.blob_id.is_some(),
        }
    }
}

/// A rolling-window order whose next top-up the treasury can't cover: the payer must fund it
/// (the order page shows the request) or later installments will not be armed in time.
pub fn funds_low(order: &Order, installments: &[Installment], treasury_spendable_zat: u64) -> Option<Notice> {
    if order.status != "active" {
        return None;
    }
    let (n, need) = promissory_core::order::next_funding_from(order, installments)?;
    if treasury_spendable_zat >= need {
        return None;
    }
    Some(Notice {
        id: format!("funds_low:{}:{}", order.id, order.funded_count.unwrap_or(0)),
        kind: "funds_low",
        tone: "warn",
        title: format!("Order #{}: funds low", order.id),
        message: format!(
            "Top up {} ZEC to fund the next {n} payment{}; the order page has the request.",
            zec_decimal(need - treasury_spendable_zat),
            if n == 1 { "" } else { "s" }
        ),
        order_id: Some(order.id),
        k: None,
        at: now_unix(),
        dismissible: false,
    })
}

/// Notices that follow from the store alone (no network).
pub fn from_state(orders: &[(OrderFacts, Vec<InstFacts>)], now: u64) -> Vec<Notice> {
    let mut out = vec![];
    for (o, insts) in orders {
        for i in insts {
            if i.state == "paid" && i.due_unix + PAID_NOTICE_SECS >= now {
                out.push(Notice {
                    id: format!("paid:{}:{}", o.id, i.k),
                    kind: "renewal_paid",
                    tone: "ok",
                    title: format!("Payment {} of {} paid", i.k + 1, o.count),
                    message: format!(
                        "Order #{}: {} ZEC to {}{}.",
                        o.id,
                        zec_decimal(o.amount_zat),
                        short(&o.payee),
                        i.mined_height.map(|h| format!(", mined at height {h}")).unwrap_or_default()
                    ),
                    order_id: Some(o.id),
                    k: Some(i.k),
                    at: i.due_unix,
                    dismissible: true,
                });
            }
        }
        out.extend(switch_due(o, insts, now));
        let cancelled = insts.iter().filter(|i| i.state == "cancelled").count();
        // A renewed switch was "cancelled" only to be replaced: no failure to report.
        if cancelled > 0 && !o.renewed && matches!(o.status.as_str(), "cancelling" | "cancelled") {
            let rejected = insts.iter().filter(|i| i.keeper_state.as_deref() == Some("cancelled")).count();
            out.push(Notice {
                id: format!("cancelled:{}", o.id),
                kind: "payment_failed_cancelled",
                tone: "warn",
                title: format!("Order #{}: payment failed, cancelled", o.id),
                message: format!(
                    "{cancelled} of {} payments will not pay: the reserved funds were swept back to you.{}",
                    o.count,
                    if rejected > 0 {
                        format!(" The chain rejected {rejected} released installment{} as a double-spend.", if rejected == 1 { "" } else { "s" })
                    } else {
                        String::new()
                    }
                ),
                order_id: Some(o.id),
                k: None,
                at: o.cancelled_at.unwrap_or(now),
                dismissible: true,
            });
        }
    }
    out
}

/// Orders whose keeper still holds installments that matter: armed and not yet out.
fn keeper_dependent(orders: &[(OrderFacts, Vec<InstFacts>)]) -> BTreeMap<String, Vec<i64>> {
    let mut by_keeper: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for (o, insts) in orders {
        if !matches!(o.status.as_str(), "active" | "cancelling") {
            continue;
        }
        if insts.iter().any(|i| i.state == "armed" && i.armed_blob) {
            by_keeper.entry(o.keeper.clone()).or_default().push(o.id);
        }
    }
    by_keeper
}

fn dismissed_path(dir: &std::path::Path) -> PathBuf {
    dir.join("notices.json")
}

fn load_dismissed(dir: &std::path::Path) -> BTreeSet<String> {
    fs::read(dismissed_path(dir))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub async fn list(State(e): State<Shared>) -> ApiResult<Vec<Notice>> {
    let (orders, low, dir) = {
        let e = e.lock().await;
        let spendable = promissory_core::wallet::account(&e.db)
            .and_then(|t| promissory_core::wallet::balances(&e.db, t))
            .map(|b| b.ironwood_spendable)
            .unwrap_or(0);
        let mut orders = vec![];
        let mut low = vec![];
        for o in e.store.orders()? {
            let insts = e.store.installments(o.id)?;
            low.extend(funds_low(&o, &insts, spendable));
            let facts = OrderFacts::from(&o).with_meta(&e.store.meta(o.id).unwrap_or_default());
            orders.push((facts, insts.iter().map(InstFacts::from).collect::<Vec<_>>()));
        }
        (orders, low, e.paths.dir.clone())
    };
    let now = now_unix();
    let dismissed = load_dismissed(&dir);
    let mut out: Vec<Notice> = from_state(&orders, now)
        .into_iter()
        .chain(low)
        .filter(|n| !dismissed.contains(&n.id))
        .collect();
    for (keeper, ids) in keeper_dependent(&orders) {
        if notices::health(&keeper, Duration::from_secs(3)).await.is_err() {
            let list = ids.iter().map(|i| format!("#{i}")).collect::<Vec<_>>().join(", ");
            out.push(Notice {
                id: format!("keeper:{keeper}"),
                kind: "keeper_unreachable",
                tone: "danger",
                title: "Keeper unreachable".into(),
                message: format!(
                    "{keeper} is not answering. Order{} {list} {} on it to release payments; they are delayed, not lost, until it is back.",
                    if ids.len() == 1 { "" } else { "s" },
                    if ids.len() == 1 { "relies" } else { "rely" }
                ),
                order_id: (ids.len() == 1).then(|| ids[0]),
                k: None,
                at: now,
                dismissible: false,
            });
        }
    }
    // Most urgent first, then newest.
    let rank = |t: &str| match t {
        "danger" => 0,
        "warn" => 1,
        _ => 2,
    };
    out.sort_by(|a, b| rank(a.tone).cmp(&rank(b.tone)).then(b.at.cmp(&a.at)));
    Ok(Json(out))
}

#[derive(Serialize)]
pub struct Dismissed {
    id: String,
}

pub async fn dismiss(State(e): State<Shared>, Path(id): Path<String>) -> ApiResult<Dismissed> {
    if id.is_empty() || id.len() > 2100 || id.starts_with("keeper:") {
        return Err(ApiError(StatusCode::BAD_REQUEST, "this notice cannot be dismissed".into()));
    }
    let dir = e.lock().await.paths.dir.clone();
    let mut set = load_dismissed(&dir);
    set.insert(id.clone());
    fs::write(dismissed_path(&dir), serde_json::to_vec(&set).map_err(anyhow::Error::from)?)
        .map_err(anyhow::Error::from)?;
    Ok(Json(Dismissed { id }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_order(window: u32, funded: u32) -> Order {
        Order {
            id: 7,
            account: String::new(),
            payee: String::new(),
            amount_zat: 1_000_000,
            memo: String::new(),
            every_secs: 180,
            count: 4,
            first_due: 0,
            keeper: String::new(),
            status: "active".into(),
            split_txid: None,
            created_at: 0,
            funding_address: None,
            sweep_txid: None,
            cancelled_at: None,
            window,
            funded_count: Some(funded),
            hook: None,
        }
    }

    fn store_inst(k: u32, state: &str) -> Installment {
        Installment {
            order_id: 7,
            k,
            due_unix: 0,
            round: 0,
            state: state.into(),
            note_txid: None,
            note_index: None,
            txids: None,
            blob_id: None,
            delete_token: None,
            anchor_height: None,
            expiry_height: None,
            keeper_state: None,
            mined_height: None,
            fresh_round: None,
            fresh_txid: None,
            fresh_blob_id: None,
            fresh_delete_token: None,
            fresh_keeper_state: None,
        }
    }

    #[test]
    fn funds_low_when_the_next_window_is_unaffordable() {
        // Window 2, both funded installments paid: the next two need 2 × 0.0101 + 0.0001.
        let o = store_order(2, 2);
        let insts = [store_inst(0, "paid"), store_inst(1, "paid"), store_inst(2, "reserved"), store_inst(3, "reserved")];
        let n = funds_low(&o, &insts, 1_000_000).expect("treasury short by 1,030,000 zat");
        assert_eq!(n.kind, "funds_low");
        assert!(n.message.contains("0.0103"), "{}", n.message);
        assert!(funds_low(&o, &insts, 2_030_000).is_none());
        // Still one funded installment ahead: one more is needed.
        let insts = [store_inst(0, "paid"), store_inst(1, "armed"), store_inst(2, "reserved"), store_inst(3, "reserved")];
        assert!(funds_low(&o, &insts, 0).unwrap().message.contains("next 1 payment;"));
        // Fully funded up front: never low.
        assert!(funds_low(&store_order(0, 4), &insts, 0).is_none());
    }

    fn order(id: i64, status: &str) -> OrderFacts {
        OrderFacts {
            id,
            status: status.into(),
            payee: "uregtest1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq".into(),
            amount_zat: 1_000_000,
            count: 3,
            keeper: "http://127.0.0.1:1".into(),
            cancelled_at: Some(2_000),
            kind: "standing".into(),
            first_due: 1_000,
            remind_secs: None,
            renewed: false,
        }
    }

    fn switch(id: i64, first_due: u64, renewed: bool) -> OrderFacts {
        OrderFacts {
            kind: "inheritance".into(),
            first_due,
            remind_secs: Some(7 * 86_400),
            renewed,
            count: 1,
            ..order(id, "active")
        }
    }

    #[test]
    fn utc_dates() {
        assert_eq!(utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc(1_793_000_000), "2026-10-26 07:33 UTC");
        assert_eq!(utc(951_825_600), "2000-02-29 12:00 UTC");
    }

    #[test]
    fn switch_due_inside_the_reminder_window() {
        let due = 2_000_000_000;
        let armed = || vec![inst(0, "armed", Some("locked"))];
        // Eight days out: no reminder yet; six days out: renew or it pays.
        assert!(from_state(&[(switch(4, due, false), armed())], due - 8 * 86_400).is_empty());
        let n = from_state(&[(switch(4, due, false), armed())], due - 6 * 86_400);
        assert_eq!(n.len(), 1);
        assert_eq!((n[0].kind, n[0].tone, n[0].id.as_str()), ("switch_due", "warn", "switch_due:4"));
        assert!(n[0].message.starts_with("Renew your switch or it pays 0.01 ZEC to uregtest1q"), "{}", n[0].message);
        assert!(n[0].message.ends_with("on 2033-05-18 03:33 UTC."), "{}", n[0].message);
        // The last hour is urgent.
        assert_eq!(from_state(&[(switch(4, due, false), armed())], due - 600)[0].tone, "danger");
        // Past the date, renewed, paid, or a standing order: nothing.
        assert!(from_state(&[(switch(4, due, false), armed())], due + 1).is_empty());
        assert!(from_state(&[(switch(4, due, true), armed())], due - 600).is_empty());
        assert!(from_state(&[(switch(4, due, false), vec![inst(0, "paid", Some("mined"))])], due - 600)
            .iter()
            .all(|n| n.kind != "switch_due"));
        let mut standing = order(4, "active");
        standing.first_due = due;
        assert!(from_state(&[(standing, armed())], due - 600).is_empty());
    }

    #[test]
    fn renewed_switch_is_not_a_failed_payment() {
        let mut old = switch(4, 5_000, true);
        old.status = "cancelled".into();
        assert!(from_state(&[(old, vec![inst(0, "cancelled", None)])], 2_000).is_empty());
    }

    fn inst(k: u32, state: &str, keeper_state: Option<&str>) -> InstFacts {
        InstFacts {
            k,
            state: state.into(),
            due_unix: 1_000 + 180 * u64::from(k),
            keeper_state: keeper_state.map(Into::into),
            mined_height: Some(10),
            armed_blob: true,
        }
    }

    #[test]
    fn paid_and_cancelled() {
        let orders = vec![(
            order(1, "cancelled"),
            vec![inst(0, "paid", Some("mined")), inst(1, "cancelled", Some("cancelled")), inst(2, "cancelled", None)],
        )];
        let n = from_state(&orders, 2_000);
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].kind, "renewal_paid");
        assert_eq!(n[0].id, "paid:1:0");
        assert_eq!(n[1].kind, "payment_failed_cancelled");
        assert!(n[1].message.starts_with("2 of 3 payments"));
        assert!(n[1].message.contains("1 released installment as a double-spend"));
        // A day after it was due, the paid notice is gone.
        assert_eq!(from_state(&orders, 1_000 + PAID_NOTICE_SECS + 1).len(), 1);
    }

    #[test]
    fn keeper_only_for_live_orders() {
        let orders = vec![
            (order(1, "active"), vec![inst(0, "armed", Some("locked"))]),
            (order(2, "completed"), vec![inst(0, "paid", Some("mined"))]),
        ];
        let k = keeper_dependent(&orders);
        assert_eq!(k.get("http://127.0.0.1:1"), Some(&vec![1]));
    }
}
