//! Pending requests from an AI agent (via `promissory-mcp`): a one-off payment or an allowance cap
//! change. Creating a request moves nothing; only the payer's approval in the web UI creates
//! the order. Kept in `so.db`, table `agent_requests`.

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

use super::{AllowanceView, MIN_LEAD_SECS};
use crate::{
    engine::{Engine, NewOrder, OrderView},
    ApiError, ApiResult, Shared,
};

/// Most requests that may wait for the payer at once; more are refused with 429.
pub const MAX_PENDING: usize = 10;
/// A request the payer has not decided on within this long expires.
pub const EXPIRE_SECS: u64 = 7 * 86_400;
/// A one-off may be requested at most this far ahead.
pub const MAX_DUE_AHEAD_SECS: u64 = 400 * 86_400;
pub const REASON_MAX_CHARS: usize = 200;
const MAX_MONEY_ZAT: u64 = 21_000_000 * 100_000_000;
/// Memo of a one-off without one.
pub const ONEOFF_MEMO: &str = "Payment";

/// What the agent asks for. The tag is `kind`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Ask {
    /// One payment from the payer's treasury.
    Oneoff {
        payee: String,
        amount_zat: u64,
        /// When it should pay; default when approved + 10 min.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        due: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        memo: Option<String>,
        /// Why the agent asks; shown to the payer.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// A new cap for an allowance, from its next period on.
    AllowanceChange {
        allowance_id: i64,
        new_cap_zat: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

fn check_text(name: &str, s: &Option<String>, max_chars: usize, max_bytes: usize) -> Result<()> {
    if let Some(s) = s {
        if s.chars().count() > max_chars || s.len() > max_bytes {
            bail!("{name} is too long (at most {max_chars} characters)");
        }
        if s.chars().any(char::is_control) {
            bail!("{name} must not contain control characters");
        }
    }
    Ok(())
}

/// Checks everything that does not need the wallet or the allowance table.
pub fn validate(ask: &Ask, now: u64) -> Result<()> {
    match ask {
        Ask::Oneoff {
            payee,
            amount_zat,
            due,
            memo,
            reason,
        } => {
            if payee.is_empty() || payee.len() > 1000 {
                bail!("payee must be a unified address");
            }
            if *amount_zat == 0 || *amount_zat > MAX_MONEY_ZAT {
                bail!("amount_zat must be between 1 and {MAX_MONEY_ZAT}");
            }
            if let Some(due) = due {
                if *due < now + 60 {
                    bail!("due must be at least a minute from now");
                }
                if *due > now + MAX_DUE_AHEAD_SECS {
                    bail!("due must be within 400 days");
                }
            }
            check_text("memo", memo, MEMO_MAX_BYTES, MEMO_MAX_BYTES)?;
            check_text("reason", reason, REASON_MAX_CHARS, 4 * REASON_MAX_CHARS)?;
        }
        Ask::AllowanceChange {
            allowance_id,
            new_cap_zat,
            reason,
        } => {
            if *allowance_id <= 0 {
                bail!("allowance_id must be a positive integer");
            }
            if *new_cap_zat == 0 || *new_cap_zat > MAX_MONEY_ZAT {
                bail!("new_cap_zat must be between 1 and {MAX_MONEY_ZAT}");
            }
            check_text("reason", reason, REASON_MAX_CHARS, 4 * REASON_MAX_CHARS)?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
pub struct Request {
    pub id: i64,
    #[serde(flatten)]
    pub ask: Ask,
    /// `pending`, `approved`, `rejected` or `expired`.
    pub status: String,
    pub created_at: u64,
    pub decided_at: Option<u64>,
    /// The order an approved one-off created.
    pub order_id: Option<i64>,
    /// The allowance an approved cap change created.
    pub allowance_id_new: Option<i64>,
    /// Why the last approval attempt failed; the request stays pending.
    pub error: Option<String>,
}

pub struct Requests {
    conn: Connection,
}

const COLUMNS: &str = "id, ask, status, created_at, decided_at, order_id, allowance_id_new, error";

/// A row as stored: `ask` is JSON.
type RawRow = (i64, String, String, u64, Option<u64>, Option<i64>, Option<i64>, Option<String>);

fn row(r: &rusqlite::Row) -> rusqlite::Result<RawRow> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))
}

impl Requests {
    pub fn open(path: impl AsRef<FsPath>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS agent_requests (
                id INTEGER PRIMARY KEY,
                kind TEXT NOT NULL,
                ask TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                decided_at INTEGER,
                order_id INTEGER,
                allowance_id_new INTEGER,
                error TEXT
            );",
        )?;
        Ok(Self { conn })
    }

    fn decode((id, ask, status, created_at, decided_at, order_id, allowance_id_new, error): RawRow) -> Result<Request> {
        Ok(Request {
            id,
            ask: serde_json::from_str(&ask)?,
            status,
            created_at,
            decided_at,
            order_id,
            allowance_id_new,
            error,
        })
    }

    /// Expires requests left pending for longer than [`EXPIRE_SECS`].
    pub fn expire(&self, now: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE agent_requests SET status = 'expired', decided_at = ?1
             WHERE status = 'pending' AND created_at + ?2 < ?1",
            params![now, EXPIRE_SECS],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub fn pending_count(&self) -> Result<usize> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM agent_requests WHERE status = 'pending'", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    /// Records a pending request, unless [`MAX_PENDING`] are already waiting. The count and the
    /// insert run in one transaction.
    pub fn insert(&mut self, ask: &Ask, now: u64) -> Result<Option<i64>> {
        let kind = match ask {
            Ask::Oneoff { .. } => "oneoff",
            Ask::AllowanceChange { .. } => "allowance_change",
        };
        let tx = self.conn.transaction()?;
        let n: i64 = tx.query_row("SELECT COUNT(*) FROM agent_requests WHERE status = 'pending'", [], |r| r.get(0))?;
        if n as usize >= MAX_PENDING {
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO agent_requests (kind, ask, status, created_at) VALUES (?1, ?2, 'pending', ?3)",
            params![kind, serde_json::to_string(ask)?, now],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(Some(id))
    }

    pub fn get(&self, id: i64) -> Result<Option<Request>> {
        self.conn
            .query_row(&format!("SELECT {COLUMNS} FROM agent_requests WHERE id = ?1"), [id], row)
            .optional()?
            .map(Self::decode)
            .transpose()
    }

    /// Newest first.
    pub fn list(&self) -> Result<Vec<Request>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {COLUMNS} FROM agent_requests ORDER BY id DESC"))?;
        let rows = stmt.query_map([], row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(Self::decode).collect()
    }

    /// Moves a pending request to `status`. Returns false if it was no longer pending.
    pub fn decide(&self, id: i64, status: &str, order_id: Option<i64>, allowance_id_new: Option<i64>) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE agent_requests SET status = ?2, decided_at = ?3, order_id = ?4,
                    allowance_id_new = ?5, error = NULL
             WHERE id = ?1 AND status = 'pending'",
            params![id, status, now_unix(), order_id, allowance_id_new],
        )?;
        Ok(n == 1)
    }

    pub fn set_error(&self, id: i64, error: &str) -> Result<()> {
        self.conn
            .execute("UPDATE agent_requests SET error = ?2 WHERE id = ?1", params![id, error])?;
        Ok(())
    }
}

fn open(e: &Engine) -> Result<Requests> {
    let r = Requests::open(e.paths.dir.join("so.db"))?;
    r.expire(now_unix())?;
    Ok(r)
}

fn get_request(e: &Engine, id: i64) -> Result<Request, ApiError> {
    open(e)?
        .get(id)?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, format!("no request {id}")))
}

/// Checks a request against the wallet's network and the allowance table.
fn check_against(e: &Engine, ask: &Ask) -> Result<()> {
    match ask {
        Ask::Oneoff { payee, .. } => {
            promissory_core::txbuild::orchard_receiver(&e.params, payee).map_err(|x| anyhow!("payee: {x}"))?;
        }
        Ask::AllowanceChange {
            allowance_id,
            new_cap_zat,
            ..
        } => {
            let a = super::open(e)?
                .get(*allowance_id)?
                .ok_or_else(|| anyhow!("no allowance {allowance_id}"))?;
            if a.stopped_at.is_some() || a.replaced_by.is_some() {
                bail!("allowance {allowance_id} is stopped");
            }
            if *new_cap_zat == a.cap_zat {
                bail!("new_cap_zat equals the current cap");
            }
            super::plan(*new_cap_zat, a.period_secs, a.chips, 1)?;
        }
    }
    Ok(())
}

/// `POST /requests`: records a pending request. Moves no funds.
pub async fn create(State(e): State<Shared>, Json(ask): Json<Ask>) -> ApiResult<Request> {
    validate(&ask, now_unix())?;
    let e = e.lock().await;
    check_against(&e, &ask)?;
    let mut store = open(&e)?;
    let Some(id) = store.insert(&ask, now_unix())? else {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            format!("{MAX_PENDING} requests are already waiting for the payer; wait until they are decided"),
        ));
    };
    tracing::info!(request = id, "agent request recorded, awaiting the payer");
    Ok(Json(store.get(id)?.ok_or_else(|| anyhow!("request {id} vanished"))?))
}

pub async fn list(State(e): State<Shared>) -> ApiResult<Vec<Request>> {
    let e = e.lock().await;
    Ok(Json(open(&e)?.list()?))
}

pub async fn get(State(e): State<Shared>, Path(id): Path<i64>) -> ApiResult<Request> {
    let e = e.lock().await;
    Ok(Json(get_request(&e, id)?))
}

#[derive(Serialize)]
pub struct Approved {
    request: Request,
    /// The one-off's order, awaiting funds until the treasury covers it.
    #[serde(skip_serializing_if = "Option::is_none")]
    order: Option<OrderView>,
    /// The replacement allowance of a cap change.
    #[serde(skip_serializing_if = "Option::is_none")]
    allowance: Option<AllowanceView>,
}

fn not_pending(r: &Request) -> ApiError {
    ApiError(StatusCode::CONFLICT, format!("request {} is {}, not pending", r.id, r.status))
}

/// When an approved one-off pays: as asked, but never sooner than the funding lead time.
pub fn oneoff_due(due: Option<u64>, now: u64) -> u64 {
    due.unwrap_or(now + 600).max(now + MIN_LEAD_SECS)
}

/// `POST /requests/{id}/approve`: the payer's decision. Only this creates an order.
pub async fn approve(State(e): State<Shared>, Path(id): Path<i64>) -> ApiResult<Approved> {
    let mut e = e.lock().await;
    let r = get_request(&e, id)?;
    if r.status != "pending" {
        return Err(not_pending(&r));
    }
    let outcome = match &r.ask {
        Ask::Oneoff {
            payee, amount_zat, due, memo, ..
        } => e
            .create(NewOrder {
                payee: payee.clone(),
                amount_zat: *amount_zat,
                every_secs: promissory_core::link::MIN_EVERY_SECS,
                count: 1,
                memo: Some(memo.clone().filter(|m| !m.trim().is_empty()).unwrap_or_else(|| ONEOFF_MEMO.into())),
                first_due: Some(oneoff_due(*due, now_unix())),
                jitter_max_secs: None,
                keeper: None,
            })
            .await
            .map(|o| (Some(o), None)),
        Ask::AllowanceChange {
            allowance_id,
            new_cap_zat,
            ..
        } => match super::open(&e)?.get(*allowance_id)? {
            None => Err(anyhow!("no allowance {allowance_id}")),
            Some(a) => super::change_cap(&mut e, &a, *new_cap_zat).await.map(|v| (None, Some(v))),
        },
    };
    let store = open(&e)?;
    match outcome {
        Ok((order, allowance)) => {
            store.decide(
                id,
                "approved",
                order.as_ref().map(|o| o.id),
                allowance.as_ref().map(|a| a.id),
            )?;
            tracing::info!(request = id, "agent request approved");
            Ok(Json(Approved {
                request: get_request(&e, id)?,
                order,
                allowance,
            }))
        }
        Err(err) => {
            let msg = crate::engine::message(&err);
            store.set_error(id, &msg)?;
            Err(ApiError(StatusCode::BAD_REQUEST, msg))
        }
    }
}

/// `POST /requests/{id}/reject`.
pub async fn reject(State(e): State<Shared>, Path(id): Path<i64>) -> ApiResult<Request> {
    let e = e.lock().await;
    let r = get_request(&e, id)?;
    if !open(&e)?.decide(id, "rejected", None, None)? {
        return Err(not_pending(&r));
    }
    tracing::info!(request = id, "agent request rejected");
    Ok(Json(get_request(&e, id)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oneoff() -> Ask {
        Ask::Oneoff {
            payee: "uregtest1x".into(),
            amount_zat: 1_000_000,
            due: None,
            memo: Some("Invoice 7".into()),
            reason: None,
        }
    }

    #[test]
    fn asks_parse_strictly() {
        let ask: Ask = serde_json::from_str(r#"{"kind":"oneoff","payee":"u1x","amount_zat":5}"#).unwrap();
        assert!(matches!(ask, Ask::Oneoff { amount_zat: 5, .. }));
        let ask: Ask = serde_json::from_str(r#"{"kind":"allowance_change","allowance_id":2,"new_cap_zat":9}"#).unwrap();
        assert_eq!(
            ask,
            Ask::AllowanceChange {
                allowance_id: 2,
                new_cap_zat: 9,
                reason: None
            }
        );
        for bad in [
            r#"{"kind":"oneoff","payee":"u1x","amount_zat":5,"extra":1}"#,
            r#"{"kind":"oneoff","payee":"u1x","amount_zat":-5}"#,
            r#"{"kind":"oneoff","payee":"u1x","amount_zat":1.5}"#,
            r#"{"kind":"oneoff","payee":"u1x","amount_zat":"5"}"#,
            r#"{"kind":"cancel_order","order_id":1}"#,
            r#"{"payee":"u1x","amount_zat":5}"#,
        ] {
            assert!(serde_json::from_str::<Ask>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn validation() {
        let now = 1_000_000;
        assert!(validate(&oneoff(), now).is_ok());
        let with = |f: fn(&mut Ask)| {
            let mut a = oneoff();
            f(&mut a);
            validate(&a, now).unwrap_err().to_string()
        };
        assert!(with(|a| if let Ask::Oneoff { amount_zat, .. } = a { *amount_zat = 0 }).contains("amount_zat"));
        assert!(with(|a| if let Ask::Oneoff { due, .. } = a { *due = Some(1_000_010) }).contains("minute from now"));
        assert!(with(|a| if let Ask::Oneoff { due, .. } = a { *due = Some(1_000_000 + 401 * 86_400) }).contains("400 days"));
        assert!(with(|a| if let Ask::Oneoff { memo, .. } = a { *memo = Some("x".repeat(401)) }).contains("memo is too long"));
        assert!(with(|a| if let Ask::Oneoff { reason, .. } = a { *reason = Some("a\nb".into()) }).contains("control"));
        assert!(with(|a| if let Ask::Oneoff { payee, .. } = a { payee.clear() }).contains("payee"));
        let change = |id, cap| Ask::AllowanceChange {
            allowance_id: id,
            new_cap_zat: cap,
            reason: None,
        };
        assert!(validate(&change(1, 5), now).is_ok());
        assert!(validate(&change(0, 5), now).is_err());
        assert!(validate(&change(1, 0), now).is_err());
    }

    #[test]
    fn pending_requests_are_capped_and_decided_once() {
        let mut s = Requests::open(":memory:").unwrap();
        let ids: Vec<i64> = (0..MAX_PENDING).map(|_| s.insert(&oneoff(), 100).unwrap().unwrap()).collect();
        assert_eq!(s.insert(&oneoff(), 100).unwrap(), None, "the 11th pending request is refused");
        assert!(s.decide(ids[0], "rejected", None, None).unwrap());
        assert!(!s.decide(ids[0], "approved", Some(3), None).unwrap(), "decided only once");
        let id = s.insert(&oneoff(), 100).unwrap().expect("room again after a decision");
        let r = s.get(id).unwrap().unwrap();
        assert_eq!((r.status.as_str(), r.ask.clone()), ("pending", oneoff()));
        assert_eq!(s.list().unwrap()[0].id, id);
        assert_eq!(s.get(ids[0]).unwrap().unwrap().status, "rejected");

        s.set_error(id, "boom").unwrap();
        assert_eq!(s.get(id).unwrap().unwrap().error.as_deref(), Some("boom"));
        assert!(s.decide(id, "approved", Some(42), None).unwrap());
        let r = s.get(id).unwrap().unwrap();
        assert_eq!((r.order_id, r.error), (Some(42), None));
    }

    #[test]
    fn stale_requests_expire() {
        let mut s = Requests::open(":memory:").unwrap();
        let old = s.insert(&oneoff(), 100).unwrap().unwrap();
        let new = s.insert(&oneoff(), 100 + EXPIRE_SECS).unwrap().unwrap();
        s.expire(101 + EXPIRE_SECS).unwrap();
        assert_eq!(s.get(old).unwrap().unwrap().status, "expired");
        assert_eq!(s.get(new).unwrap().unwrap().status, "pending");
        assert_eq!(s.pending_count().unwrap(), 1);
    }

    #[test]
    fn oneoff_due_keeps_a_lead_time() {
        assert_eq!(oneoff_due(None, 1_000), 1_600);
        assert_eq!(oneoff_due(Some(1_100), 1_000), 1_300);
        assert_eq!(oneoff_due(Some(9_000), 1_000), 9_000);
    }

    #[test]
    fn request_json_is_flat() {
        let r = Request {
            id: 3,
            ask: oneoff(),
            status: "pending".into(),
            created_at: 1,
            decided_at: None,
            order_id: None,
            allowance_id_new: None,
            error: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["kind"], "oneoff");
        assert_eq!(v["amount_zat"], 1_000_000);
        assert_eq!(v["memo"], "Invoice 7");
        assert!(v.get("due").is_none());
    }
}
