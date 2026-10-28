//! The payer app's own records: orders and their installments (`so.db` beside the wallet).
//! Delete tokens are stored here; keep the file private.

use std::path::Path;

use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

pub struct Store {
    conn: Connection,
}

#[derive(Clone, Debug, Serialize)]
pub struct Order {
    pub id: i64,
    pub account: String,
    pub payee: String,
    pub amount_zat: u64,
    pub memo: String,
    pub every_secs: u64,
    pub count: u32,
    pub first_due: u64,
    pub keeper: String,
    pub status: String,
    pub split_txid: Option<String>,
    pub created_at: u64,
    pub funding_address: Option<String>,
    pub sweep_txid: Option<String>,
    pub cancelled_at: Option<u64>,
    /// Installments kept funded ahead of time; 0 funds the whole order up front.
    pub window: u32,
    /// Installments funded so far (split into notes). `None` on orders from before rolling
    /// windows, which were funded in full by their single split.
    pub funded_count: Option<u32>,
    /// Payee webhook from a Subscribe link, attached to uploads only if the payer opted in.
    pub hook: Option<String>,
}

/// What kind of order this is, and how it relates to other orders. Kept apart from [`Order`]
/// so code that builds an `Order` by hand keeps compiling.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct OrderMeta {
    /// `standing` (the default) or `inheritance` (a dead-man's switch).
    pub kind: String,
    /// Inheritance only: remind the payer this long before the date.
    pub remind_secs: Option<u64>,
    /// The switch this order renewed, if it is a renewal.
    pub renewed_from: Option<i64>,
    /// The order that renewed this one, if any.
    pub renewed_to: Option<i64>,
}

impl Default for OrderMeta {
    fn default() -> Self {
        Self {
            kind: "standing".into(),
            remind_secs: None,
            renewed_from: None,
            renewed_to: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Installment {
    pub order_id: i64,
    pub k: u32,
    pub due_unix: u64,
    pub round: u64,
    pub state: String,
    pub note_txid: Option<String>,
    pub note_index: Option<u32>,
    pub txids: Option<String>,
    pub blob_id: Option<String>,
    #[serde(skip)]
    pub delete_token: Option<String>,
    pub anchor_height: Option<u32>,
    pub expiry_height: Option<u32>,
    pub keeper_state: Option<String>,
    pub mined_height: Option<u32>,
    /// The fresh copy, if one was signed while online shortly before the due date.
    pub fresh_round: Option<u64>,
    pub fresh_txid: Option<String>,
    pub fresh_blob_id: Option<String>,
    #[serde(skip)]
    pub fresh_delete_token: Option<String>,
    pub fresh_keeper_state: Option<String>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after 1970")
        .as_secs()
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS orders (
                id INTEGER PRIMARY KEY,
                account TEXT NOT NULL,
                payee TEXT NOT NULL,
                amount_zat INTEGER NOT NULL,
                memo TEXT NOT NULL,
                every_secs INTEGER NOT NULL,
                count INTEGER NOT NULL,
                first_due INTEGER NOT NULL,
                keeper TEXT NOT NULL,
                status TEXT NOT NULL,
                split_txid TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS installments (
                order_id INTEGER NOT NULL REFERENCES orders(id),
                k INTEGER NOT NULL,
                due_unix INTEGER NOT NULL,
                round INTEGER NOT NULL,
                state TEXT NOT NULL,
                note_txid TEXT,
                note_index INTEGER,
                txids TEXT,
                blob_id TEXT,
                delete_token TEXT,
                anchor_height INTEGER,
                expiry_height INTEGER,
                keeper_state TEXT,
                mined_height INTEGER,
                PRIMARY KEY (order_id, k)
            );",
        )?;
        // Added after the first schema; ignore the error when the column already exists.
        for column in [
            "funding_address TEXT",
            "sweep_txid TEXT",
            "cancelled_at INTEGER",
            "window INTEGER NOT NULL DEFAULT 0",
            "funded_count INTEGER",
            "hook TEXT",
            "kind TEXT DEFAULT 'standing'",
            "remind_secs INTEGER",
            "renewed_from INTEGER",
        ] {
            let _ = conn.execute(&format!("ALTER TABLE orders ADD COLUMN {column}"), []);
        }
        for column in [
            "fresh_round INTEGER",
            "fresh_txid TEXT",
            "fresh_blob_id TEXT",
            "fresh_delete_token TEXT",
            "fresh_keeper_state TEXT",
            "fresh_anchor_height INTEGER",
        ] {
            let _ = conn.execute(&format!("ALTER TABLE installments ADD COLUMN {column}"), []);
        }
        Ok(Self { conn })
    }

    /// Opens an existing store without migrating it, read-only: for reading another process's
    /// orders (e.g. making a receipt while `promissory-app` runs on the same directory).
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Ok(Self { conn })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_order(
        &mut self,
        account: &str,
        payee: &str,
        amount_zat: u64,
        memo: &str,
        every_secs: u64,
        count: u32,
        first_due: u64,
        keeper: &str,
        funding_address: &str,
        rounds: &[u64],
    ) -> Result<i64> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO orders (account, payee, amount_zat, memo, every_secs, count, first_due,
                                 keeper, status, created_at, funding_address)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'awaiting_funds', ?9, ?10)",
            params![
                account,
                payee,
                amount_zat,
                memo,
                every_secs,
                count,
                first_due,
                keeper,
                now(),
                funding_address
            ],
        )?;
        let id = tx.last_insert_rowid();
        for (k, round) in rounds.iter().enumerate() {
            tx.execute(
                "INSERT INTO installments (order_id, k, due_unix, round, state)
                 VALUES (?1, ?2, ?3, ?4, 'reserved')",
                params![id, k as u32, first_due + every_secs * k as u64, round],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    pub fn order(&self, id: i64) -> Result<Order> {
        self.conn
            .query_row(
                "SELECT id, account, payee, amount_zat, memo, every_secs, count, first_due, keeper,
                        status, split_txid, created_at, funding_address, sweep_txid, cancelled_at,
                        window, funded_count, hook
                 FROM orders WHERE id = ?1",
                [id],
                |r| {
                    Ok(Order {
                        id: r.get(0)?,
                        account: r.get(1)?,
                        payee: r.get(2)?,
                        amount_zat: r.get(3)?,
                        memo: r.get(4)?,
                        every_secs: r.get(5)?,
                        count: r.get(6)?,
                        first_due: r.get(7)?,
                        keeper: r.get(8)?,
                        status: r.get(9)?,
                        split_txid: r.get(10)?,
                        created_at: r.get(11)?,
                        funding_address: r.get(12)?,
                        sweep_txid: r.get(13)?,
                        cancelled_at: r.get(14)?,
                        window: r.get(15)?,
                        funded_count: r.get(16)?,
                        hook: r.get(17)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| anyhow!("no order {id}"))
    }

    pub fn orders(&self) -> Result<Vec<Order>> {
        let ids: Vec<i64> = self
            .conn
            .prepare("SELECT id FROM orders ORDER BY id")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        ids.into_iter().map(|id| self.order(id)).collect()
    }

    pub fn set_order_status(&self, id: i64, status: &str) -> Result<()> {
        self.conn
            .execute("UPDATE orders SET status = ?2 WHERE id = ?1", params![id, status])?;
        Ok(())
    }

    pub fn set_hook(&self, id: i64, hook: &str) -> Result<()> {
        self.conn
            .execute("UPDATE orders SET hook = ?2 WHERE id = ?1", params![id, hook])?;
        Ok(())
    }

    /// `standing` or `inheritance` ([`crate::order::KIND_INHERITANCE`]).
    pub fn set_kind(&self, id: i64, kind: &str) -> Result<()> {
        self.conn
            .execute("UPDATE orders SET kind = ?2 WHERE id = ?1", params![id, kind])?;
        Ok(())
    }

    /// How long before an inheritance switch's date the payer is reminded to renew it.
    pub fn set_remind_secs(&self, id: i64, secs: u64) -> Result<()> {
        self.conn
            .execute("UPDATE orders SET remind_secs = ?2 WHERE id = ?1", params![id, secs])?;
        Ok(())
    }

    /// Records that order `id` replaced order `from` (a renewed inheritance switch).
    pub fn set_renewed_from(&self, id: i64, from: i64) -> Result<()> {
        self.conn
            .execute("UPDATE orders SET renewed_from = ?2 WHERE id = ?1", params![id, from])?;
        Ok(())
    }

    /// An order's kind and renewal links. A store from before these columns (opened
    /// read-only, unmigrated) reads as a plain standing order.
    pub fn meta(&self, id: i64) -> Result<OrderMeta> {
        let row = self
            .conn
            .query_row(
                "SELECT COALESCE(kind, 'standing'), remind_secs, renewed_from,
                        (SELECT MAX(n.id) FROM orders n WHERE n.renewed_from = orders.id)
                 FROM orders WHERE id = ?1",
                [id],
                |r| {
                    Ok(OrderMeta {
                        kind: r.get(0)?,
                        remind_secs: r.get(1)?,
                        renewed_from: r.get(2)?,
                        renewed_to: r.get(3)?,
                    })
                },
            )
            .optional();
        match row {
            Ok(Some(m)) => Ok(m),
            Ok(None) => Err(anyhow!("no order {id}")),
            Err(e) if e.to_string().contains("no such column") => {
                self.order(id)?;
                Ok(OrderMeta::default())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// The order's kind: `standing` or `inheritance`.
    pub fn kind(&self, id: i64) -> Result<String> {
        Ok(self.meta(id)?.kind)
    }

    pub fn set_window(&self, id: i64, window: u32) -> Result<()> {
        self.conn
            .execute("UPDATE orders SET window = ?2 WHERE id = ?1", params![id, window])?;
        Ok(())
    }

    pub fn set_funded_count(&self, id: i64, funded: u32) -> Result<()> {
        self.conn.execute(
            "UPDATE orders SET funded_count = ?2 WHERE id = ?1",
            params![id, funded],
        )?;
        Ok(())
    }

    pub fn set_cancelled(&self, id: i64, sweep_txid: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE orders SET sweep_txid = COALESCE(?2, sweep_txid), cancelled_at = ?3 WHERE id = ?1",
            params![id, sweep_txid, now()],
        )?;
        Ok(())
    }

    pub fn set_split(&self, id: i64, txid: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE orders SET split_txid = ?2, status = 'splitting' WHERE id = ?1",
            params![id, txid],
        )?;
        Ok(())
    }

    pub fn installments(&self, order_id: i64) -> Result<Vec<Installment>> {
        let mut stmt = self.conn.prepare(
            "SELECT order_id, k, due_unix, round, state, note_txid, note_index, txids, blob_id,
                    delete_token, anchor_height, expiry_height, keeper_state, mined_height,
                    fresh_round, fresh_txid, fresh_blob_id, fresh_delete_token, fresh_keeper_state
             FROM installments WHERE order_id = ?1 ORDER BY k",
        )?;
        let rows = stmt.query_map([order_id], |r| {
            Ok(Installment {
                order_id: r.get(0)?,
                k: r.get(1)?,
                due_unix: r.get(2)?,
                round: r.get(3)?,
                state: r.get(4)?,
                note_txid: r.get(5)?,
                note_index: r.get(6)?,
                txids: r.get(7)?,
                blob_id: r.get(8)?,
                delete_token: r.get(9)?,
                anchor_height: r.get(10)?,
                expiry_height: r.get(11)?,
                keeper_state: r.get(12)?,
                mined_height: r.get(13)?,
                fresh_round: r.get(14)?,
                fresh_txid: r.get(15)?,
                fresh_blob_id: r.get(16)?,
                fresh_delete_token: r.get(17)?,
                fresh_keeper_state: r.get(18)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_armed(
        &self,
        order_id: i64,
        k: u32,
        note_txid: &str,
        note_index: u32,
        txids: &str,
        blob_id: &str,
        delete_token: &str,
        anchor_height: u32,
        expiry_height: u32,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE installments SET state = 'armed', note_txid = ?3, note_index = ?4, txids = ?5,
                    blob_id = ?6, delete_token = ?7, anchor_height = ?8, expiry_height = ?9,
                    keeper_state = 'locked'
             WHERE order_id = ?1 AND k = ?2",
            params![
                order_id,
                k,
                note_txid,
                note_index,
                txids,
                blob_id,
                delete_token,
                anchor_height,
                expiry_height
            ],
        )?;
        Ok(())
    }

    pub fn set_installment_state(&self, order_id: i64, k: u32, state: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE installments SET state = ?3 WHERE order_id = ?1 AND k = ?2",
            params![order_id, k, state],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_fresh(
        &self,
        order_id: i64,
        k: u32,
        round: u64,
        txid: &str,
        blob_id: &str,
        delete_token: &str,
        anchor_height: u32,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE installments SET fresh_round = ?3, fresh_txid = ?4, fresh_blob_id = ?5,
                    fresh_delete_token = ?6, fresh_keeper_state = 'locked', fresh_anchor_height = ?7
             WHERE order_id = ?1 AND k = ?2",
            params![order_id, k, round, txid, blob_id, delete_token, anchor_height],
        )?;
        Ok(())
    }

    /// Whether any installment of any order was signed against the anchor at `height`. The
    /// engine signs at most one transaction per anchor, so no two payments share one.
    pub fn anchor_in_use(&self, height: u32) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM installments
                           WHERE anchor_height = ?1 OR fresh_anchor_height = ?1)",
            [height],
            |r| r.get(0),
        )?)
    }

    pub fn set_fresh_keeper_state(&self, order_id: i64, k: u32, state: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE installments SET fresh_keeper_state = ?3 WHERE order_id = ?1 AND k = ?2",
            params![order_id, k, state],
        )?;
        Ok(())
    }

    pub fn set_keeper_view(
        &self,
        order_id: i64,
        k: u32,
        keeper_state: &str,
        mined_height: Option<u32>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE installments SET keeper_state = ?3, mined_height = ?4
             WHERE order_id = ?1 AND k = ?2",
            params![order_id, k, keeper_state, mined_height],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_signature_per_anchor() {
        let mut store = Store::open(":memory:").unwrap();
        let id = store
            .insert_order("acct", "u1", 1_000_000, "m", 60, 2, 1_000, "http://k", "u1t", &[10, 30])
            .unwrap();
        assert!(!store.anchor_in_use(100).unwrap());
        store.set_armed(id, 0, "tx", 0, "copies", "blob", "token", 100, 200).unwrap();
        store.set_fresh(id, 0, 9, "ftx", "fblob", "ftoken", 101).unwrap();
        assert!(store.anchor_in_use(100).unwrap(), "fallback anchor");
        assert!(store.anchor_in_use(101).unwrap(), "fresh anchor");
        assert!(!store.anchor_in_use(102).unwrap());
    }

    #[test]
    fn kind_and_renewal_links() {
        let mut store = Store::open(":memory:").unwrap();
        let a = store
            .insert_order("acct", "u1", 1_000_000, "m", 0, 1, 1_000, "http://k", "u1t", &[10])
            .unwrap();
        assert_eq!(store.meta(a).unwrap(), OrderMeta::default());
        assert_eq!(store.kind(a).unwrap(), "standing");
        store.set_kind(a, "inheritance").unwrap();
        store.set_remind_secs(a, 604_800).unwrap();
        let b = store
            .insert_order("acct2", "u1", 1_000_000, "m", 0, 1, 2_000, "http://k", "u1t", &[20])
            .unwrap();
        store.set_kind(b, "inheritance").unwrap();
        store.set_renewed_from(b, a).unwrap();
        let ma = store.meta(a).unwrap();
        assert_eq!((ma.kind.as_str(), ma.remind_secs, ma.renewed_to), ("inheritance", Some(604_800), Some(b)));
        assert_eq!(store.meta(b).unwrap().renewed_from, Some(a));
        assert!(store.meta(99).is_err());
    }

    #[test]
    fn read_only_store_without_new_columns() {
        let dir = std::env::temp_dir().join(format!("promissory-store-ro-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("so.db");
        let _ = std::fs::remove_file(&path);
        {
            let mut store = Store::open(&path).unwrap();
            store
                .insert_order("acct", "u1", 1, "m", 60, 1, 1_000, "http://k", "u1t", &[10])
                .unwrap();
        }
        // Simulate a store from before `kind`: SQLite can drop a column since 3.35.
        Connection::open(&path).unwrap().execute("ALTER TABLE orders DROP COLUMN kind", []).unwrap();
        let ro = Store::open_read_only(&path).unwrap();
        assert_eq!(ro.kind(1).unwrap(), "standing");
        assert!(ro.set_kind(1, "inheritance").is_err(), "read-only");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
