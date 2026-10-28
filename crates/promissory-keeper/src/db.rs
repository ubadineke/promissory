//! SQLite persistence for blobs. All writes are single statements, so a crash leaves every
//! row in a state the release loop can resume from.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS blobs (
    blob_id         TEXT PRIMARY KEY,
    round           INTEGER NOT NULL,
    ciphertext      BLOB,
    token_hash      BLOB NOT NULL,
    webhook         TEXT,
    state           TEXT NOT NULL,
    txid            TEXT,
    mined_height    INTEGER,
    last_error      TEXT,
    attempts        INTEGER NOT NULL DEFAULT 0,
    next_attempt_at INTEGER NOT NULL DEFAULT 0,
    unknown_since   INTEGER,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS blobs_state_round ON blobs (state, round);
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Locked,
    Released,
    Broadcast,
    Mined,
    Cancelled,
    Expired,
    Failed,
    Deleted,
}

impl State {
    pub const ALL: [State; 8] = [
        State::Locked,
        State::Released,
        State::Broadcast,
        State::Mined,
        State::Cancelled,
        State::Expired,
        State::Failed,
        State::Deleted,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            State::Locked => "locked",
            State::Released => "released",
            State::Broadcast => "broadcast",
            State::Mined => "mined",
            State::Cancelled => "cancelled",
            State::Expired => "expired",
            State::Failed => "failed",
            State::Deleted => "deleted",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        State::ALL
            .into_iter()
            .find(|st| st.as_str() == s)
            .ok_or_else(|| anyhow!("unknown state {s}"))
    }

    /// No further transitions, and the ciphertext has been dropped.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            State::Cancelled | State::Expired | State::Failed | State::Deleted
        )
    }
}

#[derive(Clone, Debug)]
pub struct Blob {
    pub blob_id: String,
    pub round: u64,
    pub ciphertext: Option<Vec<u8>>,
    pub token_hash: Vec<u8>,
    pub webhook: Option<String>,
    pub state: State,
    /// Display (byte-reversed) hex, as block explorers show it.
    pub txid: Option<String>,
    pub mined_height: Option<u32>,
    pub last_error: Option<String>,
    pub attempts: u32,
    pub next_attempt_at: u64,
    pub unknown_since: Option<u64>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl Blob {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        let state: String = r.get("state")?;
        Ok(Blob {
            blob_id: r.get("blob_id")?,
            round: r.get::<_, i64>("round")? as u64,
            ciphertext: r.get("ciphertext")?,
            token_hash: r.get("token_hash")?,
            webhook: r.get("webhook")?,
            state: State::parse(&state).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
            })?,
            txid: r.get("txid")?,
            mined_height: r.get("mined_height")?,
            last_error: r.get("last_error")?,
            attempts: r.get("attempts")?,
            next_attempt_at: r.get::<_, i64>("next_attempt_at")? as u64,
            unknown_since: r.get::<_, Option<i64>>("unknown_since")?.map(|t| t as u64),
            created_at: r.get::<_, i64>("created_at")? as u64,
            updated_at: r.get::<_, i64>("updated_at")? as u64,
        })
    }
}

/// A new blob as accepted by the API.
pub struct NewBlob<'a> {
    pub blob_id: &'a str,
    pub round: u64,
    pub ciphertext: &'a [u8],
    pub token_hash: &'a [u8],
    pub webhook: Option<&'a str>,
}

/// Fields the release loop changes. `None` leaves a column as it is, except where noted.
#[derive(Default)]
pub struct Update {
    pub state: Option<State>,
    pub txid: Option<String>,
    pub mined_height: Option<Option<u32>>,
    /// `Some(None)` clears the error.
    pub last_error: Option<Option<String>>,
    pub attempts: Option<u32>,
    pub next_attempt_at: Option<u64>,
    pub unknown_since: Option<Option<u64>>,
    /// Set the ciphertext to NULL.
    pub drop_ciphertext: bool,
}

pub struct Db {
    conn: Mutex<Connection>,
}

/// Storage held right now, for the upload quota.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Ciphertext bytes still stored (locked, released, or in flight).
    pub stored_bytes: u64,
    pub locked_blobs: u64,
}

/// Which quota an insert would exceed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quota {
    StoredBytes,
    LockedBlobs,
}

fn usage(conn: &Connection) -> Result<Usage> {
    let (bytes, locked) = conn.query_row(
        "SELECT COALESCE(SUM(LENGTH(ciphertext)), 0),
                (SELECT COUNT(*) FROM blobs WHERE state = 'locked')
         FROM blobs WHERE ciphertext IS NOT NULL",
        [],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
    )?;
    Ok(Usage {
        stored_bytes: bytes as u64,
        locked_blobs: locked as u64,
    })
}

impl Db {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn insert(&self, b: &NewBlob, now: u64) -> Result<()> {
        self.conn().execute(
            "INSERT INTO blobs (blob_id, round, ciphertext, token_hash, webhook, state,
                                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'locked', ?6, ?6)",
            params![b.blob_id, b.round as i64, b.ciphertext, b.token_hash, b.webhook, now as i64],
        )?;
        Ok(())
    }

    /// Inserts `b` unless that would take storage past `max_bytes` or the locked count past
    /// `max_locked` (0 means no cap). Check and insert hold one lock, so concurrent uploads
    /// can't overshoot together.
    pub fn insert_within(
        &self,
        b: &NewBlob,
        now: u64,
        max_bytes: u64,
        max_locked: u64,
    ) -> Result<Result<(), Quota>> {
        let conn = self.conn();
        let u = usage(&conn)?;
        if max_bytes > 0 && u.stored_bytes + b.ciphertext.len() as u64 > max_bytes {
            return Ok(Err(Quota::StoredBytes));
        }
        if max_locked > 0 && u.locked_blobs + 1 > max_locked {
            return Ok(Err(Quota::LockedBlobs));
        }
        conn.execute(
            "INSERT INTO blobs (blob_id, round, ciphertext, token_hash, webhook, state,
                                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'locked', ?6, ?6)",
            params![b.blob_id, b.round as i64, b.ciphertext, b.token_hash, b.webhook, now as i64],
        )?;
        Ok(Ok(()))
    }

    pub fn usage(&self) -> Result<Usage> {
        usage(&self.conn())
    }

    pub fn get(&self, blob_id: &str) -> Result<Option<Blob>> {
        Ok(self
            .conn()
            .query_row("SELECT * FROM blobs WHERE blob_id = ?1", [blob_id], Blob::from_row)
            .optional()?)
    }

    /// Locked or released blobs whose round has been published and whose back-off has passed.
    pub fn due(&self, latest_round: u64, now: u64) -> Result<Vec<Blob>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT * FROM blobs
             WHERE state IN ('locked', 'released') AND round <= ?1 AND next_attempt_at <= ?2
             ORDER BY round",
        )?;
        let rows = stmt.query_map(params![latest_round as i64, now as i64], Blob::from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Blobs whose transaction is in flight: broadcast, or mined but not yet buried.
    pub fn in_flight(&self) -> Result<Vec<Blob>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT * FROM blobs
             WHERE state = 'broadcast' OR (state = 'mined' AND ciphertext IS NOT NULL)
             ORDER BY round",
        )?;
        let rows = stmt.query_map([], Blob::from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Applies `u` only if the blob is still in state `from`. Returns whether it applied, so a
    /// concurrent delete always wins over a stale loop update.
    pub fn update(&self, blob_id: &str, from: State, u: &Update, now: u64) -> Result<bool> {
        let n = self.conn().execute(
            "UPDATE blobs SET
                state           = COALESCE(?3, state),
                txid            = COALESCE(?4, txid),
                mined_height    = CASE WHEN ?5 THEN ?6 ELSE mined_height END,
                last_error      = CASE WHEN ?7 THEN ?8 ELSE last_error END,
                attempts        = COALESCE(?9, attempts),
                next_attempt_at = COALESCE(?10, next_attempt_at),
                unknown_since   = CASE WHEN ?11 THEN ?12 ELSE unknown_since END,
                ciphertext      = CASE WHEN ?13 THEN NULL ELSE ciphertext END,
                updated_at      = ?14
             WHERE blob_id = ?1 AND state = ?2",
            params![
                blob_id,
                from.as_str(),
                u.state.map(State::as_str),
                u.txid,
                u.mined_height.is_some(),
                u.mined_height.flatten(),
                u.last_error.is_some(),
                u.last_error.clone().flatten(),
                u.attempts,
                u.next_attempt_at.map(|t| t as i64),
                u.unknown_since.is_some(),
                u.unknown_since.flatten().map(|t| t as i64),
                u.drop_ciphertext,
                now as i64,
            ],
        )?;
        Ok(n == 1)
    }

    pub fn counts(&self) -> Result<Vec<(State, u64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT state, COUNT(*) FROM blobs GROUP BY state")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        rows.map(|r| {
            let (s, n) = r?;
            Ok((State::parse(&s)?, n as u64))
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(db: &Db, id: &str, round: u64) {
        db.insert(
            &NewBlob {
                blob_id: id,
                round,
                ciphertext: b"ct",
                token_hash: b"h",
                webhook: None,
            },
            100,
        )
        .unwrap();
    }

    #[test]
    fn due_respects_round_and_backoff() {
        let db = Db::in_memory().unwrap();
        insert(&db, "a", 10);
        insert(&db, "b", 20);
        assert_eq!(db.due(15, 100).unwrap().len(), 1);
        let u = Update {
            next_attempt_at: Some(200),
            ..Default::default()
        };
        assert!(db.update("a", State::Locked, &u, 100).unwrap());
        assert!(db.due(15, 199).unwrap().is_empty());
        assert_eq!(db.due(15, 200).unwrap().len(), 1);
    }

    #[test]
    fn update_is_conditional_on_state() {
        let db = Db::in_memory().unwrap();
        insert(&db, "a", 10);
        let u = Update {
            state: Some(State::Deleted),
            drop_ciphertext: true,
            ..Default::default()
        };
        assert!(db.update("a", State::Locked, &u, 101).unwrap());
        assert!(!db.update("a", State::Locked, &u, 102).unwrap());
        let b = db.get("a").unwrap().unwrap();
        assert_eq!(b.state, State::Deleted);
        assert!(b.ciphertext.is_none());
        assert_eq!(b.updated_at, 101);
    }

    #[test]
    fn quota_counts_held_ciphertext_and_locked_blobs() {
        let db = Db::in_memory().unwrap();
        let nb = |id: &'static str| NewBlob {
            blob_id: id,
            round: 1,
            ciphertext: b"0123456789",
            token_hash: b"h",
            webhook: None,
        };
        assert_eq!(db.insert_within(&nb("a"), 1, 25, 0).unwrap(), Ok(()));
        assert_eq!(db.insert_within(&nb("b"), 1, 25, 0).unwrap(), Ok(()));
        assert_eq!(db.insert_within(&nb("c"), 1, 25, 0).unwrap(), Err(Quota::StoredBytes));
        assert_eq!(db.usage().unwrap(), Usage { stored_bytes: 20, locked_blobs: 2 });
        assert_eq!(db.insert_within(&nb("c"), 1, 0, 2).unwrap(), Err(Quota::LockedBlobs));

        // Dropped ciphertext frees bytes; leaving `locked` frees a slot.
        let u = Update {
            state: Some(State::Deleted),
            drop_ciphertext: true,
            ..Default::default()
        };
        assert!(db.update("a", State::Locked, &u, 2).unwrap());
        assert_eq!(db.usage().unwrap(), Usage { stored_bytes: 10, locked_blobs: 1 });
        assert_eq!(db.insert_within(&nb("c"), 1, 25, 2).unwrap(), Ok(()));
    }

    #[test]
    fn survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k.db");
        insert(&Db::open(&path).unwrap(), "a", 10);
        let b = Db::open(&path).unwrap().get("a").unwrap().unwrap();
        assert_eq!(b.state, State::Locked);
        assert_eq!(b.ciphertext.as_deref(), Some(&b"ct"[..]));
    }
}
