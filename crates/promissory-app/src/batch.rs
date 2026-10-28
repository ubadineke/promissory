//! Payroll batches over the payer API: one CSV, one standing order per row, one combined
//! funding request (docs/api.md, "Payroll batches"). Each row goes through the same engine
//! path as `POST /orders`; automatic progression then splits each order once the treasury's
//! spendable balance covers it.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use promissory_core::{
    batch::{self, Batch, Batches},
    order::{self, now_unix},
    wallet,
};

use crate::{
    engine::{self, Engine, Funding, NewOrder, OrderView},
    ApiError, Shared,
};

type BatchResult<T> = Result<Json<T>, Response>;

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    ApiError(status, msg.into()).into_response()
}

fn engine_err(e: anyhow::Error) -> Response {
    ApiError::from(e).into_response()
}

fn open(e: &Engine) -> Result<Batches, Response> {
    Batches::open(e.paths.dir.join("so.db")).map_err(engine_err)
}

fn uri(address: &str, zat: u64) -> String {
    format!("zcash:{address}?amount={}", order::zec_decimal(zat))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBody {
    name: Option<String>,
    csv: String,
    first_due: Option<u64>,
    keeper: Option<String>,
}

#[derive(Serialize)]
pub struct Created {
    batch_id: i64,
    name: String,
    orders: Vec<Member>,
    /// One ZIP-321 request to the treasury for every order together.
    funding: Funding,
}

#[derive(Serialize)]
pub struct Member {
    /// The payee's name from the CSV; kept locally, never sent on chain.
    name: String,
    #[serde(flatten)]
    order: OrderView,
}

#[derive(Serialize)]
pub struct BatchView {
    id: i64,
    name: String,
    created_at: u64,
    first_due: u64,
    /// Summary of the orders' statuses: `awaiting_funds` while any order still waits for
    /// funds, then `splitting`, `active`, `cancelling`, and finally `completed` or `cancelled`.
    status: &'static str,
    order_count: usize,
    /// Sum of every installment of every order.
    total_zat: u64,
    paid_zat: u64,
    /// What the treasury still needs for the orders awaiting funds; null once all are funded.
    funding: Option<Funding>,
    orders: Vec<Member>,
}

fn status_of(orders: &[Member]) -> &'static str {
    let has = |s: &str| orders.iter().any(|m| m.order.status == s);
    if has("cancelling") {
        "cancelling"
    } else if has("awaiting_funds") {
        "awaiting_funds"
    } else if has("splitting") {
        "splitting"
    } else if has("active") {
        "active"
    } else if has("cancelled") {
        "cancelled"
    } else {
        "completed"
    }
}

fn view(e: &mut Engine, b: Batch, mut views: Vec<OrderView>) -> Result<BatchView, Response> {
    let orders: Vec<Member> = b
        .members
        .into_iter()
        .filter_map(|m| {
            let i = views.iter().position(|o| o.id == m.order_id)?;
            Some(Member {
                name: m.name,
                order: views.swap_remove(i),
            })
        })
        .collect();
    let awaiting: u64 = orders
        .iter()
        .filter(|m| m.order.status == "awaiting_funds")
        .map(|m| batch::shared_funding_zat(m.order.amount_zat, m.order.count))
        .sum();
    let funding = if awaiting > 0 {
        let treasury = wallet::account(&e.db).map_err(engine_err)?;
        let address = wallet::current_address(&mut e.db, treasury).map_err(engine_err)?;
        Some(Funding {
            uri: uri(&address, awaiting),
            zat: awaiting,
            address,
            installments: orders
                .iter()
                .filter(|m| m.order.status == "awaiting_funds")
                .map(|m| m.order.count)
                .sum(),
        })
    } else {
        None
    };
    Ok(BatchView {
        id: b.id,
        name: b.name,
        created_at: b.created_at,
        first_due: b.first_due,
        status: status_of(&orders),
        order_count: orders.len(),
        total_zat: orders.iter().map(|m| m.order.amount_zat * u64::from(m.order.count)).sum(),
        paid_zat: orders.iter().map(|m| m.order.paid_zat).sum(),
        funding,
        orders,
    })
}

/// `POST /batches`: validates every row first; creates nothing if any row is invalid.
pub async fn create(State(e): State<Shared>, Json(b): Json<CreateBody>) -> BatchResult<Created> {
    let name = b.name.unwrap_or_default().trim().to_string();
    if name.chars().count() > batch::NAME_MAX_CHARS {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("name is longer than {} characters", batch::NAME_MAX_CHARS),
        ));
    }
    let now = now_unix();
    let first_due = b.first_due.unwrap_or(now + 600);
    if first_due < now + 60 {
        return Err(err(StatusCode::BAD_REQUEST, "first_due must be at least a minute from now"));
    }

    let mut e = e.lock().await;
    let rows = match batch::parse_csv(&e.params, &b.csv) {
        Ok(rows) => rows,
        Err(errors) => {
            let summary = match errors.as_slice() {
                [one] => one.to_string(),
                _ => format!("{} errors; first: {}", errors.len(), errors[0]),
            };
            let body = serde_json::json!({ "error": summary, "rows": errors });
            return Err((StatusCode::BAD_REQUEST, Json(body)).into_response());
        }
    };

    let batches = open(&e)?;
    let batch_id = batches.create(&name, first_due).map_err(engine_err)?;
    let memo = batch::memo_for(&name);
    let mut orders: Vec<Member> = vec![];
    for row in &rows {
        let created = e
            .create(NewOrder {
                payee: row.ua.clone(),
                amount_zat: row.amount_zat,
                every_secs: row.every_secs,
                count: row.count,
                memo: Some(memo.clone()),
                first_due: Some(first_due),
                jitter_max_secs: None,
                keeper: b.keeper.clone(),
            })
            .await
            .and_then(|o| {
                batches.add_order(batch_id, o.id, &row.name, row.line)?;
                Ok(o)
            });
        match created {
            Ok(order) => orders.push(Member {
                name: row.name.clone(),
                order,
            }),
            Err(failure) => {
                // Undo: nothing was funded yet, so cancelling only marks the orders cancelled.
                for m in &orders {
                    if let Err(c) = e.cancel(m.order.id, None, false, None).await {
                        tracing::warn!(order = m.order.id, "rollback cancel failed: {c:#}");
                    }
                }
                let _ = batches.delete(batch_id);
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    format!("row {}: {}", row.line, engine::message(&failure)),
                ));
            }
        }
    }

    let zat = batch::total_funding_zat(&rows);
    let address = orders
        .last()
        .and_then(|m| m.order.funding.as_ref())
        .map(|f| f.address.clone())
        .unwrap_or_default();
    tracing::info!(batch = batch_id, orders = orders.len(), zat, "batch created");
    let installments = orders.iter().map(|m| m.order.count).sum();
    Ok(Json(Created {
        batch_id,
        name,
        orders,
        funding: Funding {
            uri: uri(&address, zat),
            zat,
            address,
            installments,
        },
    }))
}

/// `GET /batches`: newest first, orders without installments.
pub async fn list(State(e): State<Shared>) -> BatchResult<Vec<BatchView>> {
    let mut e = e.lock().await;
    let all = open(&e)?.list().map_err(engine_err)?;
    let mut views = e.orders().map_err(engine_err)?;
    let mut out = vec![];
    for b in all {
        // An order belongs to at most one batch.
        let (mine, rest): (Vec<_>, Vec<_>) = views
            .into_iter()
            .partition(|o| b.members.iter().any(|m| m.order_id == o.id));
        views = rest;
        out.push(view(&mut e, b, mine)?);
    }
    Ok(Json(out))
}

fn get_batch(e: &Engine, id: i64) -> Result<Batch, Response> {
    open(e)?
        .get(id)
        .map_err(|_| err(StatusCode::NOT_FOUND, format!("no batch {id}")))
}

/// `GET /batches/{id}`: each order with its installments, keeper state refreshed.
pub async fn get(State(e): State<Shared>, Path(id): Path<i64>) -> BatchResult<BatchView> {
    let mut e = e.lock().await;
    let b = get_batch(&e, id)?;
    let mut views = vec![];
    for m in &b.members {
        views.push(e.order(m.order_id).await.map_err(engine_err)?);
    }
    Ok(Json(view(&mut e, b, views)?))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CancelBody {
    margin_secs: Option<u64>,
    #[serde(default)]
    keep_blobs: bool,
}

#[derive(Serialize)]
pub struct CancelOne {
    order_id: i64,
    name: String,
    #[serde(flatten)]
    result: Option<engine::CancelView>,
    /// Set when this order's cancel failed; the others still went ahead.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
pub struct CancelBatch {
    batch_id: i64,
    /// Orders already completed or cancelled are skipped.
    results: Vec<CancelOne>,
}

/// `POST /batches/{id}/cancel`: cancels every order of the batch that can still pay.
pub async fn cancel(
    State(e): State<Shared>,
    Path(id): Path<i64>,
    body: Option<Json<CancelBody>>,
) -> BatchResult<CancelBatch> {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let mut e = e.lock().await;
    let batch = get_batch(&e, id)?;
    let mut results = vec![];
    for m in batch.members {
        let status = e.store.order(m.order_id).map_err(engine_err)?.status;
        if matches!(status.as_str(), "completed" | "cancelled") {
            continue;
        }
        let (result, error) = match e.cancel(m.order_id, b.margin_secs, b.keep_blobs, None).await {
            Ok(v) => (Some(v), None),
            Err(x) => (None, Some(engine::message(&x))),
        };
        results.push(CancelOne {
            order_id: m.order_id,
            name: m.name,
            result,
            error,
        });
    }
    tracing::info!(batch = id, orders = results.len(), "batch cancelled");
    Ok(Json(CancelBatch { batch_id: id, results }))
}
