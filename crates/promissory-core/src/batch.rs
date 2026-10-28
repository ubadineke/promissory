//! Payroll batches: one CSV of payees becomes one standing order per row, funded together.
//!
//! CSV format, one row per payee, with an optional header row:
//!
//! ```text
//! name,ua,amount,cadence,count
//! Ada,u1…,0.25,monthly,12
//! ```
//!
//! - `amount`: ZEC per installment as a decimal string, at most 8 decimals.
//! - `cadence`: `weekly`, `biweekly`, `monthly` (30 days) or a number of seconds (≥ 60).
//! - `count`: number of installments, 1–120.
//!
//! Blank lines and lines starting with `#` are ignored. Fields may be double-quoted, with `""`
//! for a literal quote. Errors carry the 1-based line number of the file.
//!
//! Batches live in `so.db` beside the orders, in their own tables: `batches` and
//! `batch_orders` (one row per order, with the payee's name). The order rows themselves are
//! unchanged, so every other part of the engine treats batch orders like any other.

use std::path::Path;

use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use zcash_protocol::consensus::Parameters;

use crate::{
    link::{MAX_INSTALLMENTS, MEMO_MAX_BYTES, MIN_EVERY_SECS},
    order, txbuild,
};

pub const WEEK_SECS: u64 = 604_800;
pub const BIWEEK_SECS: u64 = 1_209_600;
/// `monthly` is a fixed 30 days: installments are timelocked to absolute rounds.
pub const MONTH_SECS: u64 = 2_592_000;
/// Most rows one batch may hold.
pub const MAX_ROWS: usize = 500;
pub const NAME_MAX_CHARS: usize = 100;
const MAX_MONEY_ZAT: u64 = 21_000_000 * 100_000_000;

/// One validated CSV row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Row {
    /// 1-based line number in the input.
    pub line: usize,
    pub name: String,
    pub ua: String,
    pub amount_zat: u64,
    pub every_secs: u64,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RowError {
    /// 1-based line number in the input; 0 for errors about the whole file.
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for RowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line == 0 {
            write!(f, "{}", self.message)
        } else {
            write!(f, "row {}: {}", self.line, self.message)
        }
    }
}

/// Parses a ZEC decimal string ("0.25") into zatoshis. Zero is rejected.
pub fn parse_zec(s: &str) -> Result<u64> {
    let s = s.trim();
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    if (whole.is_empty() && frac.is_empty())
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        bail!("amount {s:?} is not a ZEC amount like 0.25");
    }
    if frac.len() > 8 {
        bail!("amount has more than 8 decimal places");
    }
    let whole: u64 = if whole.is_empty() { 0 } else { whole.parse().map_err(|_| anyhow!("amount too large"))? };
    let frac: u64 = format!("{frac:0<8}").parse()?;
    let zat = whole
        .checked_mul(100_000_000)
        .and_then(|w| w.checked_add(frac))
        .filter(|z| *z <= MAX_MONEY_ZAT)
        .ok_or_else(|| anyhow!("amount exceeds the 21 million ZEC supply"))?;
    if zat == 0 {
        bail!("amount must be greater than zero");
    }
    Ok(zat)
}

/// `weekly`, `biweekly`, `monthly` (30 days) or whole seconds ≥ 60.
pub fn parse_cadence(s: &str) -> Result<u64> {
    let s = s.trim();
    let secs = match s.to_ascii_lowercase().as_str() {
        "weekly" => WEEK_SECS,
        "biweekly" => BIWEEK_SECS,
        "monthly" => MONTH_SECS,
        other if !other.is_empty() && other.bytes().all(|b| b.is_ascii_digit()) => other
            .parse()
            .map_err(|_| anyhow!("cadence {s:?} is too large"))?,
        _ => bail!("cadence {s:?} must be weekly, biweekly, monthly or a number of seconds"),
    };
    if secs < MIN_EVERY_SECS {
        bail!("cadence must be at least {MIN_EVERY_SECS} seconds");
    }
    Ok(secs)
}

/// Splits one CSV line into fields, honouring double quotes.
fn split_line(line: &str) -> Result<Vec<String>> {
    let mut fields = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                chars.next();
                cur.push('"');
            }
            ('"', true) => quoted = false,
            ('"', false) if cur.trim().is_empty() => {
                cur.clear();
                quoted = true;
            }
            (',', false) => fields.push(std::mem::take(&mut cur).trim().to_string()),
            _ => cur.push(c),
        }
    }
    if quoted {
        bail!("unclosed quote");
    }
    fields.push(cur.trim().to_string());
    Ok(fields)
}

const HEADER: [&str; 5] = ["name", "ua", "amount", "cadence", "count"];

/// Parses and validates the whole CSV for `params`' network. Either every row is valid, or
/// every error is returned, each with its line number; nothing is partially accepted.
pub fn parse_csv<P: Parameters>(params: &P, csv: &str) -> std::result::Result<Vec<Row>, Vec<RowError>> {
    let mut rows = vec![];
    let mut errors = vec![];
    let mut seen_data = false;
    for (i, raw) in csv.lines().enumerate() {
        let line = i + 1;
        let text = raw.trim().trim_start_matches('\u{feff}');
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        let fields = match split_line(text) {
            Ok(f) => f,
            Err(e) => {
                errors.push(RowError { line, message: e.to_string() });
                continue;
            }
        };
        if !seen_data
            && fields.len() == HEADER.len()
            && fields.iter().zip(HEADER).all(|(f, h)| f.eq_ignore_ascii_case(h))
        {
            seen_data = true;
            continue;
        }
        seen_data = true;
        match parse_row(params, line, &fields) {
            Ok(r) => rows.push(r),
            Err(msgs) => errors.extend(msgs.into_iter().map(|message| RowError { line, message })),
        }
    }
    if rows.is_empty() && errors.is_empty() {
        errors.push(RowError { line: 0, message: "no rows: expected name,ua,amount,cadence,count".into() });
    }
    if rows.len() > MAX_ROWS {
        errors.push(RowError { line: 0, message: format!("at most {MAX_ROWS} rows per batch") });
    }
    if errors.is_empty() {
        Ok(rows)
    } else {
        Err(errors)
    }
}

fn parse_row<P: Parameters>(params: &P, line: usize, f: &[String]) -> std::result::Result<Row, Vec<String>> {
    if f.len() != 5 {
        return Err(vec![format!("expected 5 fields (name,ua,amount,cadence,count), found {}", f.len())]);
    }
    let mut errs = vec![];
    let name = f[0].clone();
    if name.is_empty() {
        errs.push("name is empty".to_string());
    } else if name.chars().count() > NAME_MAX_CHARS {
        errs.push(format!("name is longer than {NAME_MAX_CHARS} characters"));
    }
    let ua = f[1].clone();
    if ua.is_empty() {
        errs.push("ua is empty".to_string());
    } else if let Err(e) = txbuild::orchard_receiver(params, &ua) {
        errs.push(format!("ua: {e:#}"));
    }
    let amount = parse_zec(&f[2]).map_err(|e| errs.push(e.to_string())).ok();
    let every = parse_cadence(&f[3]).map_err(|e| errs.push(e.to_string())).ok();
    let count = match f[4].parse::<u32>() {
        Ok(n) if (1..=MAX_INSTALLMENTS).contains(&n) => Some(n),
        _ => {
            errs.push(format!("count {:?} must be a whole number from 1 to {MAX_INSTALLMENTS}", f[4]));
            None
        }
    };
    match (amount, every, count) {
        (Some(amount_zat), Some(every_secs), Some(count)) if errs.is_empty() => Ok(Row {
            line,
            name,
            ua,
            amount_zat,
            every_secs,
            count,
        }),
        _ => Err(errs),
    }
}

/// What one order of `count` × `amount_zat` needs from the treasury when it shares a funding
/// note with other orders: its own funding plus the fee of the change output its split adds
/// (each split but the last returns change to the treasury).
pub fn shared_funding_zat(amount_zat: u64, count: u32) -> u64 {
    let amount = zcash_protocol::value::Zatoshis::const_from_u64(amount_zat);
    let change_fee = txbuild::ironwood_fee(1, count as usize + 1).into_u64()
        - txbuild::ironwood_fee(1, count as usize).into_u64();
    order::funding_needed(amount, count) + change_fee
}

/// The combined treasury funding for every row of a batch.
pub fn total_funding_zat(rows: &[Row]) -> u64 {
    rows.iter().map(|r| shared_funding_zat(r.amount_zat, r.count)).sum()
}

/// The memo each order of a batch carries (the app appends " (k/n)").
pub fn memo_for(batch_name: &str) -> String {
    let m = if batch_name.trim().is_empty() { "Payroll" } else { batch_name.trim() };
    let mut s = m.to_string();
    while s.len() > MEMO_MAX_BYTES - 16 {
        s.pop();
    }
    s
}

#[derive(Clone, Debug, Serialize)]
pub struct Member {
    pub order_id: i64,
    pub name: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Batch {
    pub id: i64,
    pub name: String,
    pub created_at: u64,
    pub first_due: u64,
    pub members: Vec<Member>,
}

/// Batch records in `so.db`. Opens its own connection; the order store is untouched.
pub struct Batches {
    conn: Connection,
}

impl Batches {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS batches (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                first_due INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS batch_orders (
                batch_id INTEGER NOT NULL REFERENCES batches(id),
                order_id INTEGER NOT NULL PRIMARY KEY,
                name TEXT NOT NULL,
                line INTEGER NOT NULL
            );",
        )?;
        Ok(Self { conn })
    }

    pub fn create(&self, name: &str, first_due: u64) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO batches (name, created_at, first_due) VALUES (?1, ?2, ?3)",
            params![name, order::now_unix(), first_due],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn add_order(&self, batch_id: i64, order_id: i64, name: &str, line: usize) -> Result<()> {
        self.conn.execute(
            "INSERT INTO batch_orders (batch_id, order_id, name, line) VALUES (?1, ?2, ?3, ?4)",
            params![batch_id, order_id, name, line as i64],
        )?;
        Ok(())
    }

    /// Removes a batch record (its orders stay). Used when creating a batch fails part way.
    pub fn delete(&self, batch_id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM batch_orders WHERE batch_id = ?1", [batch_id])?;
        self.conn.execute("DELETE FROM batches WHERE id = ?1", [batch_id])?;
        Ok(())
    }

    pub fn get(&self, id: i64) -> Result<Batch> {
        let (name, created_at, first_due) = self
            .conn
            .query_row(
                "SELECT name, created_at, first_due FROM batches WHERE id = ?1",
                [id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?, r.get::<_, u64>(2)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow!("no batch {id}"))?;
        let members = self
            .conn
            .prepare("SELECT order_id, name FROM batch_orders WHERE batch_id = ?1 ORDER BY line, order_id")?
            .query_map([id], |r| Ok(Member { order_id: r.get(0)?, name: r.get(1)? }))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Batch {
            id,
            name,
            created_at,
            first_due,
            members,
        })
    }

    /// Newest first.
    pub fn list(&self) -> Result<Vec<Batch>> {
        let ids: Vec<i64> = self
            .conn
            .prepare("SELECT id FROM batches ORDER BY id DESC")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        ids.into_iter().map(|id| self.get(id)).collect()
    }

    /// The batch an order belongs to, if any.
    pub fn of_order(&self, order_id: i64) -> Result<Option<(i64, String)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT batch_id, name FROM batch_orders WHERE order_id = ?1",
                [order_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::SoNetwork;

    const UA1: &str = "uregtest13543hcxsg0xcd7he0nsux3kmz2rqdg905n69x86mlyth67pesmxjpkfg6mwesu9chlpggf6clg58n7zth7uhd4ud9n655l8hkykuwwxj";
    const UA2: &str = "uregtest1ynahn0vg27j478s8azkuzthxhfagywduxzgx3ymx5armlml42rtxeeamhlywpj5p4fp469a5zck46tc7rfd79rd0nmps3lhauugm2y9q";

    fn net() -> SoNetwork {
        SoNetwork::regtest()
    }

    #[test]
    fn amounts() {
        assert_eq!(parse_zec("0.25").unwrap(), 25_000_000);
        assert_eq!(parse_zec("1").unwrap(), 100_000_000);
        assert_eq!(parse_zec(".5").unwrap(), 50_000_000);
        assert_eq!(parse_zec("0.00000001").unwrap(), 1);
        assert!(parse_zec("0").is_err());
        assert!(parse_zec("0.000000001").is_err());
        assert!(parse_zec("1,5").is_err());
        assert!(parse_zec("-1").is_err());
        assert!(parse_zec("").is_err());
        assert!(parse_zec("21000001").is_err());
    }

    #[test]
    fn cadences() {
        assert_eq!(parse_cadence("weekly").unwrap(), WEEK_SECS);
        assert_eq!(parse_cadence("BiWeekly").unwrap(), BIWEEK_SECS);
        assert_eq!(parse_cadence("monthly").unwrap(), 2_592_000);
        assert_eq!(parse_cadence("180").unwrap(), 180);
        assert!(parse_cadence("59").is_err());
        assert!(parse_cadence("daily").is_err());
    }

    #[test]
    fn parses_with_header_quotes_and_comments() {
        let csv = format!(
            "name,ua,amount,cadence,count\n# staff\n\nAda,{UA1},0.25,monthly,12\n\"Lovelace, Bo\",{UA2},0.01,180,3\n"
        );
        let rows = parse_csv(&net(), &csv).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].line, 4);
        assert_eq!(rows[0].amount_zat, 25_000_000);
        assert_eq!(rows[1].name, "Lovelace, Bo");
        assert_eq!(rows[1].every_secs, 180);
        assert_eq!(rows[1].count, 3);
    }

    #[test]
    fn headerless() {
        let rows = parse_csv(&net(), &format!("Ada,{UA1},0.01,weekly,1")).unwrap();
        assert_eq!(rows[0].line, 1);
    }

    #[test]
    fn reports_every_error_by_line() {
        let csv = format!("name,ua,amount,cadence,count\nAda,{UA1},0.01,weekly,0\n,u1nope,abc,daily,3\nBo,{UA2},0.01\n");
        let errs = parse_csv(&net(), &csv).unwrap_err();
        let lines: Vec<usize> = errs.iter().map(|e| e.line).collect();
        assert!(lines.contains(&2) && lines.contains(&3) && lines.contains(&4), "{errs:?}");
        assert_eq!(errs.iter().filter(|e| e.line == 3).count(), 4, "{errs:?}");
        assert!(errs[0].to_string().starts_with("row 2: count"));
    }

    #[test]
    fn rejects_other_network() {
        let main_ua = "u10gsry8xrghdry83eg87jfjepxfft8jefk5wg6yjkwavzk9x8ksvhz0xagf7tk33r840qye5zqvpup0xhten3de9wj6pje3cmru93g4mv";
        let errs = parse_csv(&net(), &format!("Ada,{main_ua},0.01,weekly,1")).unwrap_err();
        assert_eq!(errs[0].line, 1);
        assert!(errs[0].message.starts_with("ua:"));
    }

    #[test]
    fn empty_is_an_error() {
        assert_eq!(parse_csv(&net(), "name,ua,amount,cadence,count\n").unwrap_err()[0].line, 0);
    }

    #[test]
    fn funding_covers_change_fee() {
        let one = order::funding_needed(zcash_protocol::value::Zatoshis::const_from_u64(1_000_000), 3);
        assert_eq!(shared_funding_zat(1_000_000, 3), one + 5_000);
        let rows = parse_csv(&net(), &format!("A,{UA1},0.01,180,3\nB,{UA2},0.01,180,3")).unwrap();
        assert_eq!(total_funding_zat(&rows), 2 * (one + 5_000));
    }

    #[test]
    fn store_round_trip() {
        let dir = std::env::temp_dir().join(format!("promissory-batch-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("so.db");
        let _ = std::fs::remove_file(&path);
        let b = Batches::open(&path).unwrap();
        let id = b.create("October", 1_800_000_000).unwrap();
        b.add_order(id, 7, "Bo", 3).unwrap();
        b.add_order(id, 6, "Ada", 2).unwrap();
        let got = b.get(id).unwrap();
        assert_eq!(got.name, "October");
        assert_eq!(got.members.iter().map(|m| m.order_id).collect::<Vec<_>>(), vec![6, 7]);
        assert_eq!(b.of_order(7).unwrap(), Some((id, "Bo".into())));
        assert_eq!(b.list().unwrap().len(), 1);
        b.delete(id).unwrap();
        assert!(b.get(id).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
