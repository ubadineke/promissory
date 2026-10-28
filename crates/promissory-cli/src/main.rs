use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use promissory_core::{
    envelope::Envelope,
    net::{self, SendOutcome, TxStatus},
    batch,
    network::SoNetwork,
    order, presign, receipt, reservation, spend,
    store::Store,
    tlock,
    wallet::{self, Paths},
};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::{
    consensus::{BlockHeight, BranchId},
    memo::MemoBytes,
    value::Zatoshis,
    TxId,
};

#[derive(Parser)]
#[command(name = "promissory", about = "Promissory CLI")]
struct Cli {
    /// Chain to use: `main`, or `regtest` (the local chain from regtest/regtest.sh)
    #[arg(long, global = true, default_value = "main")]
    network: NetworkArg,
    /// lightwalletd endpoint [default: zec.rocks on main, 127.0.0.1:9067 on regtest]
    #[arg(long, global = true)]
    lwd: Option<String>,
    /// Wallet directory (seed, wallet db, pre-signed transactions)
    /// [default: promissory-data on main, promissory-data-regtest on regtest]
    #[arg(long, global = true)]
    dir: Option<PathBuf>,
    /// NU7 activation height of the regtest chain [default: 5000, as in regtest/zebrad.toml.tmpl]
    #[arg(long, global = true)]
    nu7_height: Option<u32>,
    /// Send all lightwalletd, keeper and drand traffic through Tor; fail rather than fall back
    /// to clearnet. Needs a build with `--features tor`.
    #[arg(long, global = true)]
    tor: bool,
    /// Tor (arti) state directory [default: <dir>/tor]
    #[arg(long, global = true)]
    tor_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum NetworkArg {
    Main,
    Regtest,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show lightwalletd chain, height, version and next upgrade.
    LwdInfo,
    /// Encrypt a blob to a quicknet round `secs` from now, wait, then decrypt it.
    TlockRoundtrip {
        #[arg(long, default_value_t = 9000)]
        bytes: usize,
        #[arg(long, default_value_t = 9)]
        secs: u64,
    },
    /// Create the wallet: random seed (written to <dir>/seed.hex, never printed) and one account.
    Init,
    /// Print a fresh receiving address and a ZIP-321 funding URI.
    Address {
        /// Amount in ZEC for the funding URI
        #[arg(long)]
        amount: Option<String>,
    },
    /// Sync the wallet with the chain.
    Sync,
    /// Show balances.
    Balance,
    /// Pre-sign a send-max of the whole Ironwood balance to PAYEE. Nothing is broadcast and
    /// nothing is recorded as spent in the wallet.
    Presign {
        payee: String,
        /// Expiry height; 0 disables expiry
        #[arg(long, default_value_t = 0)]
        expiry: u32,
        #[arg(long)]
        memo: Option<String>,
    },
    /// Broadcast a pre-signed transaction once it has been held for MIN_HOLD blocks.
    Broadcast {
        txid: String,
        #[arg(long, default_value_t = 30)]
        min_hold: u32,
    },
    /// Show the chain status of a pre-signed transaction.
    Status { txid: String },
    /// Spend every Ironwood note in the wallet to TO in one Builder-made transaction, and
    /// broadcast it. This is the cancel primitive.
    Sweep { to: String },
    /// Payee side: verify a proof of reservation from the chain, in a separate view-only wallet
    /// (the global --dir, default promissory-payee-view / promissory-payee-view-regtest; never the payer's).
    VerifyReservation {
        /// The `promissory-reserve:v1:…` string from `promissory order reserve-proof`
        proof: String,
    },
    /// Check a payment receipt (`promissory-receipt:v1:…`) against the chain: decrypts that one output
    /// and checks it pays the claimed address the claimed amount. Needs no wallet.
    VerifyReceipt {
        receipt: String,
    },
    /// Dead-man's switch: one payment to an heir at a date, unless you renew it first.
    #[command(subcommand)]
    Switch(SwitchCmd),
    /// Standing orders.
    #[command(subcommand)]
    Order(OrderCmd),
    /// Payroll: many standing orders from one CSV, funded together.
    #[command(subcommand)]
    Payroll(PayrollCmd),
    /// Test aids. Never needed to run an order.
    #[command(subcommand)]
    Dev(DevCmd),
}

#[derive(Subcommand)]
enum DevCmd {
    /// Decrypt a keeper ciphertext once its round is published and write each branch copy to
    /// <out>/<txid>.hex. The files are spendable transactions: test chains only.
    OpenBlob {
        /// tlock-age ciphertext, e.g. copied out of a keeper database
        ciphertext: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Subcommand)]
enum SwitchCmd {
    /// Create a switch and print the funding request. Its pre-signed copy never expires
    /// (expiry height 0), which makes it recognisable on chain as pre-signed.
    Create {
        /// Heir's unified address (must have an Orchard/Ironwood receiver)
        #[arg(long)]
        heir: String,
        /// Amount in ZEC
        #[arg(long)]
        amount: String,
        /// When it pays: unix seconds, or `+SECS` from now
        #[arg(long)]
        at: String,
        /// Remind this many days before (promissory-app's `switch_due` notice)
        #[arg(long, default_value_t = 7)]
        remind_days: u64,
        #[arg(long, default_value = "Inheritance")]
        memo: String,
        #[arg(long, default_value = "http://127.0.0.1:8737")]
        keeper: String,
    },
    /// "I'm still here": move the reservation to a new switch with a later date, in one
    /// transaction. The old pre-signed copy can then never pay.
    Renew {
        id: i64,
        /// New date: unix seconds, or `+SECS` from now
        #[arg(long)]
        at: String,
        /// Refuse if the old copy is due within this many seconds
        #[arg(long, default_value_t = order::CANCEL_MARGIN_SECS)]
        margin: u64,
        /// Leave the old blob at the keeper: it is released at its round and the chain
        /// rejects it as a double-spend
        #[arg(long)]
        keep_blob: bool,
    },
}

#[derive(Subcommand)]
enum PayrollCmd {
    /// Create one order per CSV row (name,ua,amount,cadence,count) and print the combined
    /// funding request. Every row is validated before anything is created.
    Import {
        file: PathBuf,
        /// Seconds from now until every order's first installment
        #[arg(long, default_value_t = 900)]
        first_in: u64,
        /// Batch name; also the memo each payee sees
        #[arg(long, default_value = "")]
        name: String,
        #[arg(long, default_value = "http://127.0.0.1:8737")]
        keeper: String,
    },
    /// Show a batch: each payee's order, status and amount paid.
    Status { id: i64 },
    /// List batches.
    List,
}

#[derive(Subcommand)]
enum OrderCmd {
    /// Create an order and print the funding request for the treasury.
    Create {
        /// Payee unified address (must have an Orchard/Ironwood receiver)
        #[arg(long)]
        payee: String,
        /// Amount per installment in ZEC
        #[arg(long)]
        amount: String,
        /// Seconds between installments
        #[arg(long)]
        every: u64,
        #[arg(long)]
        count: u32,
        /// Seconds from now until the first installment
        #[arg(long, default_value_t = 900)]
        first_in: u64,
        /// Maximum random release delay per installment, in seconds [default: 3 h on main, 0 on regtest]
        #[arg(long)]
        jitter: Option<u64>,
        #[arg(long, default_value = "Promissory")]
        memo: String,
        #[arg(long, default_value = "http://127.0.0.1:8737")]
        keeper: String,
        /// Keep only this many installments funded ahead (rolling window); 0 funds all now
        #[arg(long, default_value_t = 0)]
        window: u32,
    },
    /// Split the funded treasury into one note per installment.
    Split { id: i64 },
    /// Pre-sign, tlock and upload every reserved installment.
    Arm { id: i64 },
    /// Sign fresh copies of installments due within the next 45 minutes (needs to be online).
    Freshen { id: i64 },
    /// Show an order's installments and their keeper state.
    Status { id: i64 },
    /// Print a proof of reservation for one order: a viewing key for that order's account only,
    /// plus its terms and keeper blobs. Share it with the payee only if you want them to see it.
    ReserveProof { id: i64 },
    /// List orders.
    List,
    /// Print a receipt for one paid installment (K is 0-based: payment #1 is 0). It opens
    /// that one output and nothing else. Reads the wallet without changing it.
    Receipt { id: i64, k: u32 },
    /// Cancel: sweep every note the order still controls back to the treasury (or --to).
    Cancel {
        id: i64,
        #[arg(long)]
        to: Option<String>,
        /// Leave installments due within this many seconds alone (race window)
        #[arg(long, default_value_t = order::CANCEL_MARGIN_SECS)]
        margin: u64,
        /// Keep the cancelled blobs at the keeper, so it still releases them and the chain
        /// visibly rejects them
        #[arg(long)]
        keep_blobs: bool,
    },
}

/// A pre-signed transaction held on disk. `raw_hex` is a spendable transaction: keep the
/// file private and never log it.
#[derive(Serialize, Deserialize)]
struct Held {
    txid: String,
    branch: String,
    signed_at_tip: u32,
    target_height: u32,
    expiry_height: u32,
    value_zat: u64,
    fee_zat: u64,
    payee: String,
    raw_hex: String,
    broadcast_at_tip: Option<u32>,
    mined_height: Option<u32>,
}

fn held_path(dir: &PathBuf, txid: &str) -> PathBuf {
    dir.join("held").join(format!("{txid}.json"))
}

fn load_held(dir: &PathBuf, txid: &str) -> Result<Held> {
    let s = fs::read_to_string(held_path(dir, txid)).context("no such pre-signed transaction")?;
    Ok(serde_json::from_str(&s)?)
}

fn save_held(dir: &PathBuf, h: &Held) -> Result<()> {
    fs::create_dir_all(dir.join("held"))?;
    fs::write(held_path(dir, &h.txid), serde_json::to_string_pretty(h)?)?;
    Ok(())
}

fn parse_zec(s: &str) -> Result<Zatoshis> {
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    if frac.len() > 8 {
        bail!("at most 8 decimal places");
    }
    let zat = whole.parse::<u64>()? * 100_000_000 + format!("{frac:0<8}").parse::<u64>()?;
    Ok(Zatoshis::from_u64(zat)?)
}

fn zec(zat: u64) -> String {
    format!("{}.{:08}", zat / 100_000_000, zat % 100_000_000)
}

/// `--at`: unix seconds, or `+SECS` from now.
fn parse_at(s: &str) -> Result<u64> {
    match s.strip_prefix('+') {
        Some(rel) => Ok(order::now_unix() + rel.parse::<u64>().context("--at +SECS")?),
        None => s.parse::<u64>().context("--at must be unix seconds or +SECS"),
    }
}

fn parse_txid(s: &str) -> Result<TxId> {
    let mut bytes: [u8; 32] = hex::decode(s)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("txid must be 32 bytes"))?;
    // txids are displayed byte-reversed
    bytes.reverse();
    Ok(TxId::from_bytes(bytes))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let (params, default_lwd, default_dir) = match cli.network {
        NetworkArg::Main => (SoNetwork::Main, "https://zec.rocks:443", "promissory-data"),
        NetworkArg::Regtest => (
            cli.nu7_height.map_or_else(SoNetwork::regtest, SoNetwork::regtest_with_nu7),
            "http://127.0.0.1:9067",
            "promissory-data-regtest",
        ),
    };
    let lwd_url = cli.lwd.clone().unwrap_or_else(|| default_lwd.to_string());
    let dir = cli.dir.clone().unwrap_or_else(|| PathBuf::from(default_dir));
    let paths = Paths::new(&dir);
    if cli.tor {
        enable_tor(&cli.tor_dir.clone().unwrap_or_else(|| dir.join("tor"))).await?;
    }
    match cli.cmd {
        Cmd::LwdInfo => {
            let mut lwd = net::connect(&lwd_url).await?;
            let i = net::info(&mut lwd).await?;
            println!("chain:        {}", i.chain_name);
            println!("height:       {}", i.block_height);
            println!("branch:       {}", i.consensus_branch_id);
            println!("server:       {} {}", i.vendor, i.version);
            println!("node build:   {}", i.zcashd_build);
            println!(
                "next upgrade: {}",
                if i.upgrade_name.is_empty() {
                    "none scheduled".to_string()
                } else {
                    format!("{} at {}", i.upgrade_name, i.upgrade_height)
                }
            );
        }
        Cmd::TlockRoundtrip { bytes, secs } => {
            let drand = tlock::Drand::quicknet()?;
            let chain = drand.chain()?;
            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            let round = chain.round_at(now + secs);
            let plaintext: Vec<u8> = (0..bytes).map(|i| (i * 31 % 251) as u8).collect();
            let ct = tlock::encrypt(&chain, round, &plaintext)?;
            println!(
                "encrypted {bytes} B -> {} B, locked to round {round} (header says {})",
                ct.len(),
                tlock::locked_round(&ct)?
            );
            if drand.signature(round)?.is_some() {
                bail!("round {round} already published; encryption target was in the past");
            }
            println!("round {round} not yet published; decryption is impossible until it is");
            let sig = loop {
                if let Some(sig) = drand.signature(round)? {
                    break sig;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            };
            let pt = tlock::decrypt(&chain.hash, &sig, &ct)?;
            if pt != plaintext {
                bail!("round trip mismatch");
            }
            println!("round {round} published; decrypted {} B, matches", pt.len());
        }
        Cmd::Init => {
            let mut lwd = net::connect(&lwd_url).await?;
            let (account, birthday) = wallet::init(&paths, params, &mut lwd).await?;
            println!("created account {account:?} with birthday {birthday}");
            println!("seed written to {} (mode 0600)", paths.seed().display());
        }
        Cmd::Address { amount } => {
            let mut db = wallet::open(&paths, params)?;
            let account = wallet::account(&db)?;
            let ua = wallet::next_address(&mut db, account)?;
            println!("address: {ua}");
            if let Some(amount) = amount {
                println!("funding URI: zcash:{ua}?amount={amount}");
            }
        }
        Cmd::Sync => {
            let mut lwd = net::connect(&lwd_url).await?;
            let mut db = wallet::open(&paths, params)?;
            wallet::sync(&mut db, &mut lwd).await?;
            let b = wallet::balances(&db, wallet::account(&db)?)?;
            println!(
                "synced to {} (tip {}); ironwood {} ZEC ({} spendable), orchard {} ZEC",
                b.fully_scanned,
                b.chain_tip,
                zec(b.ironwood_total),
                zec(b.ironwood_spendable),
                zec(b.orchard_total)
            );
        }
        Cmd::Balance => {
            let db = wallet::open(&paths, params)?;
            let b = wallet::balances(&db, wallet::account(&db)?)?;
            println!("{b:#?}");
        }
        Cmd::Presign {
            payee,
            expiry,
            memo,
        } => {
            let mut lwd = net::connect(&lwd_url).await?;
            let mut db = wallet::open(&paths, params)?;
            wallet::sync(&mut db, &mut lwd).await?;
            let account = wallet::account(&db)?;
            let usk = wallet::usk(&paths, &db, account)?;
            let memo = memo
                .map(|m| MemoBytes::from_bytes(m.as_bytes()))
                .transpose()
                .map_err(|e| anyhow::anyhow!("memo: {e:?}"))?;
            let tip = net::tip_height(&mut lwd).await?;
            println!("building and proving (first run builds the proving key, ~10 s)...");
            let p = presign::presign_send_max(
                &mut db,
                account,
                &usk,
                &payee,
                memo,
                Some(BlockHeight::from_u32(expiry)),
            )?;
            let held = Held {
                txid: p.txid.to_string(),
                branch: format!("{:?}", p.branch),
                signed_at_tip: tip.into(),
                target_height: p.target_height.into(),
                expiry_height: p.expiry_height.into(),
                value_zat: p.value_zat,
                fee_zat: p.fee_zat,
                payee,
                raw_hex: hex::encode(&p.raw),
                broadcast_at_tip: None,
                mined_height: None,
            };
            save_held(&dir, &held)?;
            println!("pre-signed {} (NOT broadcast)", held.txid);
            println!(
                "  branch {} | signed at tip {} | target {} | expiry {} | {} B",
                held.branch,
                held.signed_at_tip,
                held.target_height,
                held.expiry_height,
                p.raw.len()
            );
            println!("  pays {} ZEC, fee {} ZEC", zec(p.value_zat), zec(p.fee_zat));
        }
        Cmd::Broadcast { txid, min_hold } => {
            let mut held = load_held(&dir, &txid)?;
            let mut lwd = net::connect(&lwd_url).await?;
            let tip: u32 = net::tip_height(&mut lwd).await?.into();
            let held_for = tip.saturating_sub(held.signed_at_tip);
            if held_for < min_hold {
                bail!(
                    "held for {held_for} blocks; need {min_hold} (about {} more minutes)",
                    (min_hold - held_for) * 75 / 60
                );
            }
            println!("held for {held_for} blocks; broadcasting {txid}");
            match net::send(&mut lwd, &hex::decode(&held.raw_hex)?).await? {
                SendOutcome::Accepted => println!("accepted into the mempool"),
                SendOutcome::Rejected { code, message } => {
                    bail!("rejected ({code}): {message}")
                }
            }
            held.broadcast_at_tip = Some(tip);
            save_held(&dir, &held)?;
            let id = parse_txid(&txid)?;
            loop {
                match net::tx_status(&mut lwd, &id).await? {
                    TxStatus::Mined(h) => {
                        held.mined_height = Some(h.into());
                        save_held(&dir, &held)?;
                        println!("mined at height {h}");
                        break;
                    }
                    s => println!("  {s:?}; waiting"),
                }
                tokio::time::sleep(Duration::from_secs(20)).await;
            }
        }
        Cmd::Status { txid } => {
            let held = load_held(&dir, &txid)?;
            let mut lwd = net::connect(&lwd_url).await?;
            let tip: u32 = net::tip_height(&mut lwd).await?.into();
            println!(
                "{txid}: signed at {}, tip {tip} (held {} blocks), expiry {}, broadcast {:?}, mined {:?}",
                held.signed_at_tip,
                tip.saturating_sub(held.signed_at_tip),
                held.expiry_height,
                held.broadcast_at_tip,
                held.mined_height
            );
            println!("chain: {:?}", net::tx_status(&mut lwd, &parse_txid(&txid)?).await?);
        }
        Cmd::VerifyReceipt { receipt } => {
            let r = receipt::Receipt::decode(&receipt)?;
            let mut lwd = net::connect(&lwd_url).await?;
            let v = receipt::verify(&mut lwd, &params, &r).await?;
            println!("valid receipt");
            println!("  paid    {} ZEC", zec(v.value_zat));
            println!("  to      {}", v.payee);
            println!("  in tx   {} ({} action {}), mined at height {}", v.txid, v.pool, v.action_index, v.height);
            if let Some(m) = v.memo {
                println!("  memo    {m}");
            }
        }
        Cmd::Order(OrderCmd::Receipt { id, k }) => {
            // Read-only: no migration, no sync, so it is safe beside a running promissory-app.
            let store = Store::open_read_only(dir.join("so.db"))?;
            let db = wallet::open_unmigrated(&paths, params)?;
            let mut lwd = net::connect(&lwd_url).await?;
            println!("{}", receipt::make(&paths, &db, &store, &mut lwd, id, k).await?.encode());
        }
        Cmd::Switch(cmd) => {
            let mut store = Store::open(dir.join("so.db"))?;
            let mut lwd = net::connect(&lwd_url).await?;
            let mut db = wallet::open(&paths, params)?;
            match cmd {
                SwitchCmd::Create {
                    heir,
                    amount,
                    at,
                    remind_days,
                    memo,
                    keeper,
                } => {
                    let at = parse_at(&at)?;
                    let created = order::create_inheritance(
                        &paths,
                        &mut db,
                        &mut store,
                        &mut lwd,
                        &heir,
                        parse_zec(&amount)?,
                        &memo,
                        at,
                        remind_days * 86_400,
                        &keeper,
                    )
                    .await?;
                    println!("switch {} created: {amount} ZEC to the heir at {at} unless renewed", created.id);
                    println!("its pre-signed copy never expires (expiry 0): it can be recognised on chain as pre-signed");
                    println!("fund the treasury with {} ZEC:", zec(created.funding_zat));
                    println!("  {}", created.funding_uri());
                    println!("then: promissory order split {} && promissory order arm {}", created.id, created.id);
                }
                SwitchCmd::Renew {
                    id,
                    at,
                    margin,
                    keep_blob,
                } => {
                    let r = order::renew(&paths, &mut db, &mut store, &mut lwd, id, parse_at(&at)?, margin, !keep_blob).await?;
                    match r.txid {
                        Some(t) => println!("renewed: switch {} -> {} in one transaction {t}", r.old_id, r.new_id),
                        None => println!("renewed: switch {} -> {} (it was never funded)", r.old_id, r.new_id),
                    }
                    if r.withdrew_blob {
                        println!("the old copy was withdrawn from the keeper");
                    } else if r.txid.is_some() {
                        println!("the old copy stays at the keeper and will fail as a double-spend");
                    }
                    println!("then: promissory order arm {} once the renewal confirms", r.new_id);
                }
            }
        }
        Cmd::Order(cmd) => {
            let mut store = Store::open(dir.join("so.db"))?;
            match cmd {
                OrderCmd::Create {
                    payee,
                    amount,
                    every,
                    count,
                    first_in,
                    jitter,
                    memo,
                    keeper,
                    window,
                } => {
                    let mut lwd = net::connect(&lwd_url).await?;
                    let mut db = wallet::open(&paths, params)?;
                    let schedule = order::Schedule {
                        amount: parse_zec(&amount)?,
                        memo,
                        every_secs: every,
                        count,
                        first_due: order::now_unix() + first_in,
                        jitter_max_secs: jitter.unwrap_or_else(|| params.default_jitter_secs()),
                    };
                    let created =
                        order::create_with_window(
                            &paths, &mut db, &mut store, &mut lwd, &payee, &schedule, &keeper, window,
                        )
                        .await?;
                    println!("order {} created: {count} × {amount} ZEC every {every} s", created.id);
                    println!("fund the treasury with {} ZEC:", zec(created.funding_zat));
                    println!("  {}", created.funding_uri());
                    println!("then: promissory order split {} && promissory order arm {}", created.id, created.id);
                }
                OrderCmd::Split { id } => {
                    let mut lwd = net::connect(&lwd_url).await?;
                    let mut db = wallet::open(&paths, params)?;
                    let txid = order::split(&paths, &mut db, &mut store, &mut lwd, id).await?;
                    println!("split {txid} broadcast; arm once it has confirmed");
                }
                OrderCmd::Arm { id } => {
                    let mut lwd = net::connect(&lwd_url).await?;
                    let mut db = wallet::open(&paths, params)?;
                    let armed = order::arm(&paths, &mut db, &mut store, &mut lwd, id).await?;
                    if armed.is_empty() {
                        println!("nothing to arm yet: wait for the split to confirm");
                    }
                    for a in armed {
                        let copies = a
                            .copies
                            .iter()
                            .map(|(b, t)| format!("{b:?} {t}"))
                            .collect::<Vec<_>>()
                            .join(", ");
                        println!(
                            "#{} armed: round {}, blob {}, expiry {}, copies [{copies}]",
                            a.k + 1,
                            a.round,
                            a.blob_id,
                            a.expiry
                        );
                    }
                }
                OrderCmd::Freshen { id } => {
                    let mut lwd = net::connect(&lwd_url).await?;
                    let mut db = wallet::open(&paths, params)?;
                    let ks = order::freshen(&paths, &mut db, &mut store, &mut lwd, id).await?;
                    if ks.is_empty() {
                        println!("no installment is inside the fresh-copy window");
                    }
                    for k in ks {
                        println!("#{} fresh copy uploaded", k + 1);
                    }
                }
                OrderCmd::Status { id } => {
                    let o = store.order(id)?;
                    let chain = promissory_core::tlock::Drand::quicknet()?.chain()?;
                    let now = order::now_unix();
                    let rows = order::refresh(&mut store, id).await?;
                    let o = store.order(o.id)?;
                    println!(
                        "order {} [{}]: {} × {} ZEC to {}…",
                        o.id,
                        o.status,
                        o.count,
                        zec(o.amount_zat),
                        &o.payee[..20.min(o.payee.len())]
                    );
                    for (inst, status) in rows {
                        let release = chain.round_time(inst.round);
                        let when = if release > now {
                            format!("in {} s", release - now)
                        } else {
                            format!("{} s ago", now - release)
                        };
                        let keeper = status
                            .map(|s| {
                                let mut v = s.state;
                                if let Some(h) = s.mined_height {
                                    v.push_str(&format!(" at {h}"));
                                }
                                if let Some(e) = s.last_error {
                                    v.push_str(&format!(" ({e})"));
                                }
                                v
                            })
                            .unwrap_or_else(|| "-".into());
                        println!(
                            "  #{:<2} round {} ({when:>12}) | {:<9} | keeper: {keeper}",
                            inst.k + 1,
                            inst.round,
                            inst.state
                        );
                    }
                }
                OrderCmd::ReserveProof { id } => {
                    let db = wallet::open(&paths, params)?;
                    println!("{}", reservation::export(&params, &db, &store, id)?.encode());
                }
                OrderCmd::Receipt { .. } => unreachable!("handled above"),
                OrderCmd::List => {
                    for o in store.orders()? {
                        println!(
                            "{:>3} [{}] {} × {} ZEC every {} s",
                            o.id,
                            o.status,
                            o.count,
                            zec(o.amount_zat),
                            o.every_secs
                        );
                    }
                }
                OrderCmd::Cancel {
                    id,
                    to,
                    margin,
                    keep_blobs,
                } => {
                    let mut lwd = net::connect(&lwd_url).await?;
                    let mut db = wallet::open(&paths, params)?;
                    let c = order::cancel(
                        &paths,
                        &mut db,
                        &mut store,
                        &mut lwd,
                        id,
                        to.as_deref(),
                        margin,
                        !keep_blobs,
                    )
                        .await?;
                    match c.sweep {
                        Some(t) => println!("cancel sweep {t} broadcast"),
                        None => println!("nothing left to sweep"),
                    }
                    for k in c.cancelled {
                        println!("  #{} cancelled", k + 1);
                    }
                    for (k, why) in c.may_still_pay {
                        println!("  #{} may still pay: {}", k + 1, why.reason());
                    }
                }
            }
        }
        Cmd::Payroll(cmd) => {
            let mut store = Store::open(dir.join("so.db"))?;
            let batches = batch::Batches::open(dir.join("so.db"))?;
            match cmd {
                PayrollCmd::Import {
                    file,
                    first_in,
                    name,
                    keeper,
                } => {
                    let csv = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
                    let rows = match batch::parse_csv(&params, &csv) {
                        Ok(rows) => rows,
                        Err(errors) => {
                            for e in &errors {
                                eprintln!("{}: {e}", file.display());
                            }
                            bail!("{} invalid row(s); nothing was created", errors.len());
                        }
                    };
                    let mut lwd = net::connect(&lwd_url).await?;
                    let mut db = wallet::open(&paths, params)?;
                    let first_due = order::now_unix() + first_in;
                    let id = batches.create(name.trim(), first_due)?;
                    let memo = batch::memo_for(&name);
                    let mut address = String::new();
                    for row in &rows {
                        let schedule = order::Schedule {
                            amount: Zatoshis::from_u64(row.amount_zat)?,
                            memo: memo.clone(),
                            every_secs: row.every_secs,
                            count: row.count,
                            first_due,
                            jitter_max_secs: params.default_jitter_secs(),
                        };
                        let created =
                            order::create(&paths, &mut db, &mut store, &mut lwd, &row.ua, &schedule, &keeper)
                                .await
                                .with_context(|| format!("row {} ({})", row.line, row.name))?;
                        batches.add_order(id, created.id, &row.name, row.line)?;
                        address = created.treasury_address;
                        println!(
                            "  order {:>3}: {} × {} ZEC every {} s to {}",
                            created.id,
                            row.count,
                            zec(row.amount_zat),
                            row.every_secs,
                            row.name
                        );
                    }
                    let total = batch::total_funding_zat(&rows);
                    println!("batch {id} created with {} orders", rows.len());
                    println!("fund the treasury once with {} ZEC:", zec(total));
                    println!("  zcash:{address}?amount={}", order::zec_decimal(total));
                    println!("promissory-app splits and arms each order as the funds arrive; or run `promissory order split` / `promissory order arm` per order");
                }
                PayrollCmd::Status { id } => {
                    let b = batches.get(id)?;
                    println!(
                        "batch {} {:?}: {} orders, first due {}",
                        b.id,
                        b.name,
                        b.members.len(),
                        b.first_due
                    );
                    let (mut total, mut paid_total) = (0, 0);
                    for m in b.members {
                        let rows = order::refresh(&mut store, m.order_id).await?;
                        let o = store.order(m.order_id)?;
                        let paid = rows.iter().filter(|(i, _)| i.state == "paid").count() as u64;
                        total += o.amount_zat * u64::from(o.count);
                        paid_total += paid * o.amount_zat;
                        println!(
                            "  {:<20} order {:>3} [{:<14}] {}/{} paid, {} of {} ZEC",
                            m.name,
                            o.id,
                            o.status,
                            paid,
                            o.count,
                            zec(paid * o.amount_zat),
                            zec(o.amount_zat * u64::from(o.count))
                        );
                    }
                    println!("paid {} of {} ZEC", zec(paid_total), zec(total));
                }
                PayrollCmd::List => {
                    for b in batches.list()? {
                        println!("{:>3} {:?}: {} orders, first due {}", b.id, b.name, b.members.len(), b.first_due);
                    }
                }
            }
        }
        Cmd::VerifyReservation { proof } => {
            let view_dir = cli.dir.clone().unwrap_or_else(|| {
                PathBuf::from(match cli.network {
                    NetworkArg::Main => "promissory-payee-view",
                    NetworkArg::Regtest => "promissory-payee-view-regtest",
                })
            });
            let proof = reservation::Proof::decode(&proof)?;
            let mut lwd = net::connect(&lwd_url).await?;
            let r = reservation::verify(params, &view_dir, &mut lwd, &proof).await?;
            print_reservation(&r);
        }
        Cmd::Dev(DevCmd::OpenBlob { ciphertext, out }) => {
            let ct = fs::read(&ciphertext)?;
            let round = tlock::locked_round(&ct)?;
            let drand = tlock::Drand::quicknet()?;
            let sig = drand
                .signature(round)?
                .with_context(|| format!("round {round} not published yet"))?;
            let env = Envelope::from_bytes(&tlock::decrypt(&drand.chain()?.hash, &sig, &ct)?)?;
            fs::create_dir_all(&out)?;
            for c in env.copies {
                let branch = BranchId::try_from(c.branch_id).map_err(|e| anyhow::anyhow!("{e}"))?;
                let txid = Transaction::read(&hex::decode(&c.tx)?[..], branch)?.txid();
                fs::write(out.join(format!("{txid}.hex")), &c.tx)?;
                println!("round {round}: {branch:?} (0x{:08x}) {txid}, {} B", c.branch_id, c.tx.len() / 2);
            }
        }
        Cmd::Sweep { to } => {
            let mut lwd = net::connect(&lwd_url).await?;
            let mut db = wallet::open(&paths, params)?;
            wallet::sync(&mut db, &mut lwd).await?;
            let account = wallet::account(&db)?;
            let usk = wallet::usk(&paths, &db, account)?;
            let notes = spend::ironwood_notes(&db, account)?;
            let built = spend::sweep(&mut db, &usk, &notes, &to, MemoBytes::empty())?;
            println!(
                "sweep {}: {} notes, fee {} ZEC, branch {:?}, target {}, anchor {}, expiry {}, {} B",
                built.txid,
                notes.len(),
                zec(built.fee.into_u64()),
                built.branch,
                built.target_height,
                built.anchor_height,
                built.expiry_height,
                built.raw.len()
            );
            match net::send(&mut lwd, &built.raw).await? {
                SendOutcome::Accepted => println!("accepted into the mempool"),
                SendOutcome::Rejected { code, message } => bail!("rejected ({code}): {message}"),
            }
        }
    }
    Ok(())
}

fn print_reservation(r: &reservation::Report) {
    let t = &r.terms;
    println!(
        "order: {} × {} ZEC every {} s to {}…, first due {}",
        t.count,
        zec(t.amount_zat),
        t.every_secs,
        &t.payee[..24.min(t.payee.len())],
        t.first_due
    );
    println!("scanned to {} (tip {})", r.synced_height, r.tip_height);
    println!(
        "funded:   {} of {} notes, {} ZEC{}",
        r.funded,
        t.count,
        zec(r.funded_zat),
        if r.matches_terms { " (matches the terms)" } else { "" }
    );
    println!("reserved: {} ({} ZEC still unspent)", r.reserved, zec(r.reserved_zat));
    println!("paid:     {} ({} ZEC to the payee)", r.paid, zec(r.paid_zat));
    if r.swept > 0 {
        println!(
            "CANCELLED BY PAYER: {} note(s), {} ZEC, spent without paying the payee. Where they went is not visible: cancel sweeps carry no outgoing viewing key.",
            r.swept,
            zec(r.swept_zat)
        );
    }
    for n in &r.notes {
        let detail = match n.state {
            "paid" => format!("paid {} ZEC in {}", zec(n.paid_zat.unwrap_or(0)), n.spent_txid.as_deref().unwrap_or("")),
            "swept" | "spent_unknown" => format!("spent in {}", n.spent_txid.as_deref().unwrap_or("")),
            _ => String::new(),
        };
        println!("  note {}:{} {} ZEC  {:<8} {detail}", &n.txid[..12], n.action_index, zec(n.value_zat), n.state);
    }
    for b in &r.keeper {
        println!(
            "  keeper #{} blob {}…: {}{}",
            b.k.map(|k| k + 1).unwrap_or(0),
            &b.blob_id[..12.min(b.blob_id.len())],
            b.state.as_deref().or(b.error.as_deref()).unwrap_or("-"),
            b.mined_height.map(|h| format!(" at {h}")).unwrap_or_default()
        );
    }
    if let Some(at) = r.next_release_unix {
        println!("next release: {at} (in {} s)", at.saturating_sub(order::now_unix()));
    }
    for w in &r.warnings {
        println!("warning: {w}");
    }
}

/// Installs a process-wide Tor client: from here on promissory-core reaches lightwalletd, keepers and
/// drand only through Tor.
#[cfg(feature = "tor")]
async fn enable_tor(dir: &std::path::Path) -> Result<()> {
    eprintln!("bootstrapping Tor in {} …", dir.display());
    promissory_core::tor::bootstrap_and_install(dir).await?;
    if let Some(d) = promissory_core::tor::bootstrap_time() {
        eprintln!("Tor ready in {:.1}s", d.as_secs_f32());
    }
    Ok(())
}

#[cfg(not(feature = "tor"))]
async fn enable_tor(_: &std::path::Path) -> Result<()> {
    bail!("this build has no Tor support; rebuild with `--features tor`")
}
