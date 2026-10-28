# Promissory

*A shielded promise to pay.*

**Scheduled payments for shielded Zcash: subscriptions, payroll, remittances and allowances that pay on time with your wallet closed, cancel instantly, and reveal nothing on chain. No smart contract, no custodian.**

Zcash users have wanted recurring payments since 2019. ZecWallet tried and dropped them: a wallet can't pay while it's closed. Promissory makes each payment a fully signed shielded transaction that nobody can open until its date, and anyone can broadcast after.

## How it works

1. **Reserve.** The payer funds the order once (one QR code, from Zodl or any wallet). The app splits the funds into one note per payment, in an account reserved for the order.
2. **Sign.** Each payment is a complete Ironwood transaction spending exactly its note.
3. **Lock.** Each transaction is encrypted with drand timelock encryption to the drand round at its due date.
4. **Release.** At that round a keeper (anyone can run one) decrypts the transaction and broadcasts it. The payer's device can be off.
5. **Cancel.** The payer spends the reserved notes. Every later payment then fails on chain as a double-spend: no server decides that.

Every payment settles as an ordinary shielded transaction, so sender, recipient and amount stay private. [`docs/privacy-review.md`](docs/privacy-review.md) lists every fingerprint and what we do about it.

## What's in it

| Feature | Notes |
|---|---|
| **Subscriptions** | A payee builds a "Subscribe with Zcash" link; the payer opens it and funds once. The payee needs no setup. |
| **Scheduled and post-dated payments** | One payment on a chosen date. |
| **Payroll** | A CSV of `name,ua,amount,cadence,count` becomes one order per row, funded with one payment and split in one transaction. |
| **Remittance** | A weekly preset with amounts shown in naira. The rate is for display only; payments are fixed in ZEC. |
| **Rolling window** | Fund only the next N payments; the app tops up from your treasury as they are paid, and asks you to top up when it runs short. |
| **Fresh copies** | While you're online, each payment is re-signed shortly before it's due, so it looks like a normal send. The pre-signed copy then fails harmlessly. |
| **NU7-proof** | Payments due after NU7 (Nov 5 2026) are signed for both branches now; the keeper sends whichever is valid. |
| **Proof of reservation** | Share an order-scoped viewing key; the payee can see the next payments are funded, and is warned if you cancel. |
| **Notices** | Payment paid, payment cancelled, keeper unreachable, funds low. Payee webhooks are opt-in. |
| **Inheritance switch** | Pays an heir on a date unless you renew. |
| **Agent allowances + MCP** | "Up to X per period" for an AI agent, through an MCP server. Every spend beyond the allowance needs your approval. |
| **Receipts** | A per-payment proof you can show without revealing other payments. |
| **Tor** | `--tor` routes lightwalletd, keeper and drand traffic through Tor. |

## Components

| Path | What |
|---|---|
| `crates/promissory-core` | The engine: wallet, low-level transaction building, PCZT signing, tlock, orders, keeper client. Embeddable in wallets ([`docs/integrating.md`](docs/integrating.md)). |
| `crates/promissory-keeper` | The keeper service: blob store, drand-driven release, broadcast, status, limits, Tor. |
| `crates/promissory-app` | The payer app: a local HTTP API ([`docs/api.md`](docs/api.md)) plus the web UI, with automatic progression (split → arm → fresh copies → top-ups). |
| `crates/promissory-cli` | `so`: every engine operation from the command line. |
| `crates/promissory-mcp` | MCP server for agent allowances ([`docs/mcp.md`](docs/mcp.md)). |
| `packages/promissory-js` | `@promissory/client`: Subscribe links, address checks, keeper and payer clients (TypeScript). |
| `apps/web` | The web UI (React). Served by promissory-app. `/` is the landing page and `/orders` the app. |
| `regtest/` | A local Zebra + lightwalletd chain in Docker, with NU6.3 (Ironwood) active and NU7 configurable. |

## Run it locally (regtest)

You need Docker, Rust and pnpm.

```sh
cargo build --release
pnpm install && pnpm -r build

# A local chain, a payer wallet and a payee wallet
regtest/regtest.sh up && regtest/regtest.sh mine 2
./target/release/promissory --network regtest init
./target/release/promissory --network regtest --dir promissory-data-regtest-payee init

# Mine coins to the payer, wait out coinbase maturity, keep blocks coming
regtest/regtest.sh up "$(./target/release/promissory --network regtest address | cut -d' ' -f2)"
regtest/regtest.sh mine 1 && regtest/regtest.sh up && regtest/regtest.sh mine 110
regtest/regtest.sh automine 15 &

# Keeper and payer app
./target/release/promissory-keeper --network regtest &
./target/release/promissory-app --network regtest &
open http://127.0.0.1:8738/orders
```

Create an order in the web UI (or open a Subscribe link), fund it, and close nothing: promissory-app splits and arms it, and the keeper pays each installment at its time. `regtest/gate-engine.sh` replays the engine gate: three payments with the app closed, then a cancel, and the fourth rejected by the chain.

## Evidence

Every gate, with heights, txids and Zebra's exact error strings, is in [`docs/evidence.md`](docs/evidence.md):
- the kill gate
- the engine gate
- NU7 dual copies
- fresh copies
- rolling window
- proof of reservation
- payroll
- the incidents found and fixed along the way

## Limits, stated openly

- **Amounts are fixed in ZEC.** There is no fiat pricing until shielded stablecoins exist.
- **Funds are reserved ahead.** A rolling window keeps that to the next few payments.
- **An offline payer's payments may look pre-signed on chain:** an old anchor and a rolling expiry. Sender, recipient and amount stay hidden either way.
- **It depends on drand.** If drand stalls, payments are delayed, never stolen.
- **The payee has no guarantee** unless you share proof of reservation, the same as card payments.
- **Cancel can't stop a payment the keeper has already broadcast.** The app shows which payments are inside that window.

## Docs

- [`docs/plan.md`](docs/plan.md): build plan, fixed decisions, keeper contract
- [`docs/api.md`](docs/api.md): Subscribe link format, keeper API, payer API
- [`docs/integrating.md`](docs/integrating.md): for merchants, wallets, apps and keeper operators
- [`docs/privacy-review.md`](docs/privacy-review.md): fingerprints, parties, mitigations
- [`docs/who-sees-what.md`](docs/who-sees-what.md): the short version
- [`docs/threat-model.md`](docs/threat-model.md): what can go wrong, and what can't
- [`docs/promissory-js.md`](docs/promissory-js.md): TypeScript client reference
- [`docs/mcp.md`](docs/mcp.md): MCP server for agents
- [`docs/evidence.md`](docs/evidence.md): everything we ran

## License

MIT OR Apache-2.0.
