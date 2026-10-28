# Integrating Promissory

Promissory turns "pay X every P, N times" into N pre-signed shielded transactions that a keeper releases on schedule. No smart contract, no custodian. This guide covers the four ways to build on it.

| You are… | Use | Code needed |
|---|---|---|
| A **merchant or payee** (subscriptions, payroll, donations) | Subscribe links + keeper status | None, or a few lines with `@promissory/client` |
| A **wallet** (Zodl, Zafe, Vizor, …) | `promissory-core` (Rust) inside the wallet | A "Schedule" button and a screen |
| An **app or service** that sets up payments for a user | The payer API of `promissory-app` | HTTP calls |
| A **keeper operator** | `promissory-keeper` | Run a binary |

Contracts for every surface are in [`api.md`](api.md).

---

## 1. Merchants and payees: Subscribe links

You need a unified address with an Orchard receiver; any current Zodl address has one. Build a link and put it behind a "Subscribe with Zcash" button or QR code:

```ts
import { encodeSubscribeLink } from "@promissory/client";

const link = encodeSubscribeLink("https://pay.example.com/s", {
  net: "main",
  to: "u1…",           // your address
  amt: 5_000_000,      // 0.05 ZEC per period, in zatoshis
  every: 2_592_000,    // 30 days
  n: 12,
  memo: "Pro plan",
});
```

- **The page at `/s` can be static.** The parameters live in the URL fragment, so your server never sees who subscribes. The payer's Promissory app opens the link, shows a summary, and asks them to fund it once.
- **You receive ordinary shielded payments in your wallet.** There's nothing to install. To watch status without keys, poll the keeper with the blob IDs the payer shares, or use webhooks on a keeper that enables them (`--allow-webhooks`).
- **Ask for a proof of reservation** if you need a guarantee (salary, rent). The payer exports a `promissory-reserve:v1:…` string from the order page; paste it on the **Payee status** page of your own `promissory-app`, or run `promissory verify-reservation <proof>`. You see how many payments are funded, which have paid you, and whether the payer cancelled. You don't see where a cancel sent the funds, or anything else of the payer's. See `api.md`, "Proof of reservation".
- **Webhooks are off by default.** A keeper only accepts the `hook` from your link when started with `--allow-webhooks`, because a webhook tells the keeper your endpoint and ties each payment to you. Prefer polling the keeper or a proof of reservation unless you run or trust the keeper.

## 2. Wallets: embed `promissory-core`

Most Zcash wallets are built on `zcash_client_backend` / `zcash_client_sqlite` (librustzcash 2026-10-21 release), which `promissory-core` also uses. A wallet can offer standing orders with its own keys and no second app.

`promissory-core`'s order functions are generic over two things a wallet already has:

- **Consensus parameters.** Any `zcash_protocol::consensus::Parameters + Copy` (`MainNetwork`, `Network`, …): the wallet opens its own wallet file with `promissory_core::wallet::open(paths, params)`, which yields a `Db<P>` (a `zcash_client_sqlite::WalletDb`).
- **Key storage.** Implement `promissory_core::wallet::KeyStore` (one method: `seed()`) over the wallet's own secret storage. Order accounts are derived from that seed at the next ZIP 32 index.

`order.rs` has a compile-time test (`embedding`) that runs the whole lifecycle with `Db<MainNetwork>` and a custom `KeyStore`.

The minimal flow, with the wallet's database, its seed, and a lightwalletd connection:

```rust
use promissory_core::{order, store::Store, wallet::Paths};

let mut store = Store::open(data_dir.join("so.db"))?;
let schedule = order::Schedule {
    amount, memo, every_secs, count, first_due,
    jitter_max_secs: 3 * 3600,
};
let keys = MyWalletKeys; // impl promissory_core::wallet::KeyStore

// 1. Reserve an account and get the funding request. Window 2 keeps only the next two
//    installments funded; 0 funds the whole order up front.
let created = order::create_with_window(&keys, &mut db, &mut store, &mut lwd, payee, &schedule, keeper_url, 2).await?;

// 2. The user funds `created.funding_uri()`. When the funds can be spent (and again whenever
//    `order::next_funding` asks for the next rolling-window chunk):
order::split(&keys, &mut db, &mut store, &mut lwd, created.id).await?;

// 3. When the split notes can be spent: pre-sign, tlock and upload the funded installments.
order::arm(&keys, &mut db, &mut store, &mut lwd, created.id).await?;

// 4. While online, shortly before each due date: sign a fresh copy that releases first.
order::freshen(&keys, &mut db, &mut store, &mut lwd, created.id).await?;

// Any time: status from the keeper, or cancel.
order::refresh(&mut store, created.id).await?;
order::cancel(&keys, &mut db, &mut store, &mut lwd, created.id, None, order::CANCEL_MARGIN_SECS, true).await?;
```

Rules a wallet integration must keep:

- **Never spend from an order's account** except through `order::cancel`. Its notes are already committed to signed installments. Exclude these accounts from auto-shielding, consolidation and pool migration.
- **Don't store installments as sent.** They are built with the low-level builder and are not recorded in the wallet database. When the keeper broadcasts one, the wallet sees it through its normal scan.
- **Reject payee addresses without an Orchard receiver.** `txbuild::orchard_receiver` does this. Paying another pool would cross pools and reveal the amount.
- **Fund from Ironwood.** Funding from Orchard crosses pools and reveals the total on chain.
- **Show the race window.** An installment whose round is less than `CANCEL_MARGIN_SECS` away, or that the keeper has already released, cannot be cancelled.

### Dead-man's switch

```rust
// One payment to the heir at `at`, unless renewed. Fund, split and arm it like any order.
let sw = order::create_inheritance(&keys, &mut db, &mut store, &mut lwd, heir, amount, "Inheritance", at, order::SWITCH_REMIND_SECS, keeper_url).await?;

// "I'm still here": one transaction moves the reservation to a new switch with a later date.
let r = order::renew(&keys, &mut db, &mut store, &mut lwd, sw.id, later, order::CANCEL_MARGIN_SECS, true).await?;
// r.new_id is `splitting`; call order::arm on it once r.txid confirms.
```

`store.meta(id)` gives `kind`, `remind_secs`, `renewed_from` and `renewed_to`. A switch's pre-signed copy has expiry height 0, so it can still pay months later. That makes it recognisable on chain as pre-signed, and the wallet should say so. Remind the user before `first_due - remind_secs`.

### Receipts

```rust
// Payer: a receipt for paid installment k (0-based). Reads the wallet, does not sync it.
let r = promissory_core::receipt::make(&keys, &db, &store, &mut lwd, order_id, k).await?;
let shareable = r.encode(); // promissory-receipt:v1:…

// Anyone: check it against the chain. No keys.
let v = promissory_core::receipt::verify(&mut lwd, &params, &promissory_core::receipt::Receipt::decode(&shareable)?).await?;
```

`receipt::prepare` reads what is needed from the wallet up front, and `Pending::finish` does the network part. Use them when the wallet handle must not be held across awaits.

**Relation to ZIP 311 (payment disclosures, draft).** A ZIP 311 disclosure lists `(index, ock)` pairs for the outputs it reveals; the verifier recovers each note with the outgoing cipher key. A receipt is that part, applied to Orchard/Ironwood actions; the draft still has "TODO: Add support for Orchard". `PRF^ock_Orchard` takes `cv_net`, `cmx` and `epk`, and `try_output_recovery_with_ock` checks the recovered note against `cmx`. A receipt leaves out the rest of ZIP 311:
- the spend-authority part: a new spend proof plus a `spendAuthSig` over the disclosure under the randomized `rk`, which proves the presenter controls the spend;
- the optional challenge `msg`, which makes a disclosure interactive and non-replayable.

So a receipt shows that the payment happened. It does not show who is presenting it, and it can be passed on. For "prove *you* paid", ZIP 311's signed form is the upgrade: re-prove an action spend with the payer's key and sign `BLAKE2b-256("ZIP311Signed" || coinType, unsigned disclosure)`. Never hand out the OVK instead: it opens every payment the account made.

If the wallet has its own transaction pipeline, use the lower layers directly: `txbuild::build` for exact-note Ironwood transactions, `envelope::Envelope` + `tlock::encrypt` for the keeper payload, and `keeper::Keeper` for upload.

## 3. Apps and services: the payer API

Run `promissory-app` next to the user's wallet data. Everything the web UI does goes through its local HTTP API, so another program can do the same:

```ts
import { PayerClient } from "@promissory/client";

const payer = new PayerClient("http://127.0.0.1:8738");
const order = await payer.createOrderFromLink({ link }); // funding.uri: show it as a QR
// promissory-app splits and arms automatically once funded.
const detail = await payer.getOrder(order.id);           // per-installment state
await payer.cancelOrder(order.id);

// Dead-man's switch: create, then renew while alive.
const sw = await payer.createSwitch({ heir, amount_zat: 150_000_000, at: Math.floor(Date.now() / 1000) + 90 * 86_400 });
await payer.renewSwitch(sw.id, { at: sw.first_due + 30 * 86_400 });

// Receipts: one paid installment, checked by anyone's promissory-app.
const { receipt } = await payer.getReceipt(order.id, 0);
const shown = await payer.verifyReceipt(receipt); // {value_zat, payee, memo, txid, height}
```

`decodeReceipt` (promissory-js) parses a receipt locally to preview what it claims. Only `verifyReceipt` checks the claim against the chain.

`promissory-app` binds to localhost only and requires the `X-Promissory-App: 1` header on writes. It holds spending keys, so never expose it on a network.

## 4. Keeper operators

```
promissory-keeper --network main --db keeper.db --listen 0.0.0.0:8737
```

- **What a keeper sees:** ciphertext until each round is published, then the transaction (which is about to be public anyway).
- **What it never sees:** keys, or who the payer is.
- **What it can't do:** steal, or release early. If it fails, the payment is only delayed, and payers can upload to several keepers for redundancy.
- **Logging:** IP addresses are not logged. Put the keeper behind Tor or a privacy-respecting proxy, and then run it with `--rate-limit-key none`.
- **Limits:** see "Keeper contract" in `plan.md`: round horizon, per-client and global rate limits, storage quota.

## Tor

Tor support is behind the cargo feature `tor` (arti, through `zcash_client_backend`'s `tor` module). It is off by default in `promissory-core`, `promissory-cli` and `promissory-app`, and on by default in `promissory-keeper`.

- **Keeper:** `promissory-keeper --tor [--tor-dir DIR]`.
  - lightwalletd (with a fresh circuit for every broadcast), drand and webhooks all go over Tor.
  - The keeper refuses to start if Tor won't bootstrap.
- **Payer:** `promissory --tor …` (build with `--features tor`) and `promissory-app --tor` (likewise).
  - These install one process-wide Tor client before anything connects. lightwalletd sync and broadcasts, keeper uploads, status checks and deletes, and drand chain info then go only through Tor. Nothing falls back to clearnet.
  - A first bootstrap takes 20–35 s. With a warm `--tor-dir` it takes about 10 s.
- **Library:** `promissory_core::tor::bootstrap(dir)`, then any of these:
  - `net::connect_tor(&tor, url)`;
  - `keeper::Keeper::with_tor(base, &tor)`, on circuits isolated per keeper;
  - `tlock::TorDrand::new(&tor)`.
  - Or call `promissory_core::tor::install(tor)`, so that `net::connect`, `Keeper::new` and `Drand` use Tor implicitly.
- **DNS:** host names are passed to the Tor exit, which resolves them. Nothing is resolved locally.
- **Limits of the Tor HTTP client:** it does GET and POST only. Deletes use the keeper's `POST /v1/blobs/{id}/delete`, and older keepers without that route return an error, never a silent success. `.onion` keepers aren't supported yet; `.onion` lightwalletd endpoints are.
- **Regtest:** Tor exits refuse private addresses, so `--tor` can't reach a lightwalletd or keeper on `127.0.0.1`.
