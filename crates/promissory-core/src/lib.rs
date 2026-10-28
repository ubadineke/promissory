//! Promissory core: pre-signed, timelocked, cancellable shielded payments on Zcash.
//!
//! A payer authorizes `n` payments once. Each installment is a fully signed Ironwood
//! transaction, tlock-encrypted to the drand round at its due date and held by a keeper. When
//! the round is published, the keeper decrypts and broadcasts it; the payer's wallet can be
//! closed. To cancel, the payer spends the reserved notes; any later installment then fails on
//! chain as a double-spend. There is no contract and no custodian.
//!
//! # Lifecycle
//!
//! 1. [`order::create`]: reserve an account for the order and compute the funding request
//!    ([`order::Created::funding_uri`], a ZIP 321 URI for any wallet).
//! 2. [`order::split`]: split the funding into one exact note per installment.
//! 3. [`order::arm`]: build each installment with [`txbuild::build`] (explicit note, anchor,
//!    target height and expiry), wrap the branch copies in an [`envelope::Envelope`], encrypt
//!    with [`tlock::encrypt`], and upload with [`keeper::Keeper::upload`].
//! 4. The keeper (`promissory-keeper`) releases each envelope at its round.
//! 5. [`order::cancel`]: one [`spend::sweep`] of every note the order still controls.
//!
//! # Embedding in a wallet
//!
//! A wallet that already uses `zcash_client_sqlite` can drive these steps directly with its own
//! database and seed; see `docs/integrating.md`. The lower-level pieces are usable on their own:
//!
//! - [`txbuild::build`]: build an Ironwood transaction spending exact notes, with no change.
//! - [`authorize::authorize`]: sign, prove and extract a PCZT.
//! - [`tlock`]: drand quicknet round arithmetic and encryption.
//! - [`envelope`]: the plaintext format keepers understand.
//! - [`link`]: parse and build Subscribe links.
//! - [`keeper`]: the keeper HTTP client.
//!
//! ```no_run
//! # async fn demo() -> anyhow::Result<()> {
//! use promissory_core::{link::Subscribe, tlock::Drand};
//!
//! let s = Subscribe::parse("https://so.example/s#v=1&net=main&to=u1…&amt=1000000&every=604800&n=12")?;
//! let chain = Drand::quicknet()?.chain()?;
//! let first_round = chain.round_at(s.start.unwrap_or(1_800_000_000));
//! # let _ = first_round; Ok(()) }
//! ```

pub mod authorize;
pub mod batch;
pub mod cache;
pub mod envelope;
pub mod keeper;
pub mod link;
pub mod net;
pub mod network;
pub mod notices;
pub mod order;
pub mod presign;
pub mod receipt;
pub mod reservation;
pub mod spacing;
pub mod spend;
pub mod store;
pub mod tlock;
#[cfg(feature = "tor")]
pub mod tor;
pub mod txbuild;
pub mod wallet;
