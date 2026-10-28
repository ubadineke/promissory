//! The Promissory app's own wallet: seed, account, sync and balance.

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use rand::{rand_core::UnwrapErr, rngs::SysRng, Rng};
use secrecy::{ExposeSecret, SecretVec};
use zcash_client_backend::{
    data_api::{
        wallet::ConfirmationsPolicy, Account as _, AccountBirthday, WalletRead, WalletWrite,
    },
    sync,
    util::SystemClock,
};
pub use zcash_client_sqlite::AccountUuid;
use zcash_client_sqlite::{wallet::init::init_wallet_db, WalletDb};
use zcash_keys::keys::{UnifiedAddressRequest, UnifiedSpendingKey};
use zcash_protocol::consensus::BlockHeight;

use crate::{
    cache::MemBlockCache,
    net,
    network::{Params, SoNetwork},
};

pub type Rand = UnwrapErr<SysRng>;
/// A `zcash_client_sqlite` wallet. A wallet embedding Promissory opens its own wallet file
/// with [`open`] and its own consensus parameters.
pub type Db<P = SoNetwork> = WalletDb<rusqlite::Connection, P, SystemClock, Rand>;

/// Where the engine gets the seed for deriving order accounts and their spending keys. The
/// app keeps it in `seed.hex` ([`Paths`]); a wallet implements this over its own key storage.
pub trait KeyStore {
    fn seed(&self) -> Result<SecretVec<u8>>;
}

impl KeyStore for Paths {
    fn seed(&self) -> Result<SecretVec<u8>> {
        read_seed(self)
    }
}

/// On-disk layout of one Promissory wallet directory.
pub struct Paths {
    pub dir: PathBuf,
}

impl Paths {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_path_buf(),
        }
    }
    pub fn db(&self) -> PathBuf {
        self.dir.join("wallet.db")
    }
    pub fn seed(&self) -> PathBuf {
        self.dir.join("seed.hex")
    }
}

pub fn open<P: Params>(paths: &Paths, params: P) -> Result<Db<P>> {
    let mut db = WalletDb::for_path(paths.db(), params, SystemClock, UnwrapErr(SysRng))?;
    init_wallet_db(&mut db, None).map_err(|e| anyhow!("wallet db migration: {e}"))?;
    Ok(db)
}

/// Opens an existing wallet without migrating it: for reads (e.g. making a receipt) while
/// another process owns the wallet. Never sync or spend through it.
pub fn open_unmigrated<P: Params>(paths: &Paths, params: P) -> Result<Db<P>> {
    if !paths.db().exists() {
        bail!("no wallet at {}", paths.db().display());
    }
    Ok(WalletDb::for_path(paths.db(), params, SystemClock, UnwrapErr(SysRng))?)
}

fn read_seed(paths: &Paths) -> Result<SecretVec<u8>> {
    let hex_seed = fs::read_to_string(paths.seed()).context("reading seed")?;
    Ok(SecretVec::new(hex::decode(hex_seed.trim())?))
}

/// Creates the wallet directory, a fresh random seed (never printed), and one account whose
/// birthday is the current chain tip.
pub async fn init<P: Params>(
    paths: &Paths,
    params: P,
    lwd: &mut net::Lwd,
) -> Result<(AccountUuid, BlockHeight)> {
    if paths.seed().exists() {
        bail!("{} already exists; refusing to overwrite a seed", paths.seed().display());
    }
    fs::create_dir_all(&paths.dir)?;

    let mut seed = [0u8; 32];
    UnwrapErr(SysRng).fill_bytes(&mut seed);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(paths.seed())?;
    f.write_all(hex::encode(seed).as_bytes())?;
    let seed = SecretVec::new(seed.to_vec());

    let mut db = open(paths, params)?;
    let (account, birthday) = create_account(&mut db, &seed, lwd, TREASURY).await?;
    Ok((account, birthday))
}

/// Name of the account that receives funding from the payer's wallet.
pub const TREASURY: &str = "promissory";

/// The treasury's name in wallets created before the product was renamed.
const LEGACY_TREASURY: &str = "standing-order";

/// Creates the next ZIP 32 account on the wallet's seed, born at the current tip.
async fn create_account<P: Params>(
    db: &mut Db<P>,
    seed: &SecretVec<u8>,
    lwd: &mut net::Lwd,
    name: &str,
) -> Result<(AccountUuid, BlockHeight)> {
    let tip = net::tip_height(lwd).await?;
    // The birthday is the block after the tree state we start from.
    let treestate = net::tree_state(lwd, tip - 1).await?;
    let birthday = AccountBirthday::from_treestate(treestate, None)
        .map_err(|e| anyhow!("birthday: {e:?}"))?;
    let (account, _usk) = db.create_account(name, seed, &birthday, None)?;
    Ok((account, birthday.height()))
}

/// A fresh account reserved for one standing order. Nothing but that order's installments and
/// its cancel sweep ever spends from it.
pub async fn create_order_account<P: Params>(
    keys: &impl KeyStore,
    db: &mut Db<P>,
    lwd: &mut net::Lwd,
    name: &str,
) -> Result<AccountUuid> {
    let seed = keys.seed()?;
    Ok(create_account(db, &seed, lwd, name).await?.0)
}

/// The treasury account.
pub fn account<P: Params>(db: &Db<P>) -> Result<AccountUuid> {
    for id in db.get_account_ids()? {
        if let Some(acct) = db.get_account(id)? {
            if matches!(acct.name(), Some(TREASURY | LEGACY_TREASURY)) {
                return Ok(id);
            }
        }
    }
    Err(anyhow!("wallet has no treasury account; run init"))
}

pub fn account_by_uuid(s: &str) -> Result<AccountUuid> {
    Ok(AccountUuid::from_uuid(s.parse().map_err(|e| anyhow!("account uuid: {e}"))?))
}

pub fn usk<P: Params>(keys: &impl KeyStore, db: &Db<P>, account: AccountUuid) -> Result<UnifiedSpendingKey> {
    let seed = keys.seed()?;
    let acct = db
        .get_account(account)?
        .ok_or_else(|| anyhow!("unknown account"))?;
    let derivation = acct
        .source()
        .key_derivation()
        .ok_or_else(|| anyhow!("account is not derived from a seed"))?;
    UnifiedSpendingKey::from_seed(db.params(), seed.expose_secret(), derivation.account_index())
        .map_err(|e| anyhow!("deriving spending key: {e:?}"))
}

/// A fresh Orchard-receiver address for this account. Since NU6.3, a payment to an Orchard
/// receiver lands in the Ironwood pool.
pub fn next_address<P: Params>(db: &mut Db<P>, account: AccountUuid) -> Result<String> {
    let (ua, _) = db
        .get_next_available_address(account, UnifiedAddressRequest::ORCHARD)?
        .ok_or_else(|| anyhow!("unknown account"))?;
    Ok(ua.encode(db.params()))
}

/// The account's most recently generated Orchard-receiver address, or a fresh one if none.
pub fn current_address<P: Params>(db: &mut Db<P>, account: AccountUuid) -> Result<String> {
    match db.get_last_generated_address_matching(account, UnifiedAddressRequest::ORCHARD)? {
        Some(ua) => Ok(ua.encode(db.params())),
        None => next_address(db, account),
    }
}

pub async fn sync<P: Params>(db: &mut Db<P>, lwd: &mut net::Lwd) -> Result<()> {
    let cache = MemBlockCache::default();
    let params = *db.params();
    sync::run(lwd, &params, &cache, db, 1000)
        .await
        .map_err(|e| anyhow!("sync: {e}"))
}

#[derive(Debug)]
pub struct Balances {
    pub chain_tip: BlockHeight,
    pub fully_scanned: BlockHeight,
    pub ironwood_spendable: u64,
    pub ironwood_total: u64,
    pub orchard_total: u64,
}

pub fn balances<P: Params>(db: &Db<P>, account: AccountUuid) -> Result<Balances> {
    let summary = db
        .get_wallet_summary(ConfirmationsPolicy::default())?
        .ok_or_else(|| anyhow!("wallet not synced"))?;
    let bal = summary
        .account_balances()
        .get(&account)
        .ok_or_else(|| anyhow!("no balance for account"))?;
    Ok(Balances {
        chain_tip: summary.chain_tip_height(),
        fully_scanned: summary.fully_scanned_height(),
        ironwood_spendable: bal.ironwood_balance().spendable_value().into_u64(),
        ironwood_total: bal.ironwood_balance().total().into_u64(),
        orchard_total: bal.orchard_balance().total().into_u64(),
    })
}
