# Promissory: build plan

Product and engineering context: `../../standing-order.md` and `../../standing-order-engineering.md`.
This file supersedes the engineering spec wherever the two disagree.

Phases are ordered by dependency, not by date. Nothing in this plan is cut for time; work runs in parallel instead.

## Phases

### 1. Kill gate: done on regtest, mainnet pending funds
- tlock round trip on drand quicknet
- A pre-signed Ironwood transaction with expiry 0, held 30+ blocks, then mined. See `evidence.md`.

### 2. Engine
- **Funding:** one ZIP-321 URI pays a treasury account. One split transaction then creates one note per installment, `amount + fee` each, in the order's reserved account.
- **Signing:** every installment copy is built with the low-level `Builder`, not the proposal APIs.
  - Exact note selection, with the anchor current at signing time.
  - Fallback copy: expiry = the rolling expiry at the predicted mining height.
  - The real spend's action index comes from `ironwood_meta`.
- **Locking:** tlock each envelope (`promissory-core::envelope`) to its round R_k on quicknet and upload it to the keeper(s).
- **Keeper (`promissory-keeper`):** described below.
- **Cancel:** one multi-spend sweep transaction (Builder, all reserved notes, OVK discarded). It must land before the earliest round of any copy minus 10 minutes. UI state "may still pay: inside the race window". Once the keeper has broadcast, cancel is impossible.
- **Surfaces:** Subscribe link page; payer app showing create, fund, per-installment status, cancel.
- **Gate:** three installments released by the keeper with the payer app closed, then a fourth rejected after cancel. Run on regtest first, then mainnet.

### 3. Hardening and features
- **NU7 copies:** height prediction switches from 75 s to 25 s blocks at activation. Re-sign everything if NU7's branch ID changes.
- **NU7 gate (regtest):** NU7 at height H; build the NU7 copy at H−20; mine it after H; show the NU6.3 copy rejected.
- **Fresh copy:** while online in the window [R_k − 45 min, R_k − 10 min], sign a copy with the current anchor and the normal expiry delta (40 blocks before NU7, 120 after). Lock it to R_k − 5 min so it releases before the fallback. R_k never changes.
- **Payroll** (CSV), **remittance** (naira display), **proof of reservation** (viewing key), **notices** (opt-in webhooks), **payee status view**.
- **Tor** for keeper broadcasts and app uploads.
- **Mainnet:** installments due on or after Nov 6, signed now, recorded as evidence.
- **Privacy review** against the fingerprint list.
- **Gate:** NU7 regtest test passes; a fresh copy is mined while its fallback is rejected.

### 4. Further features
Inheritance (dead-man's switch), agent allowances plus an MCP server, receipts, developer kit (Rust crate plus JS client).

### 5. Ship
README, threat model, `who-sees-what.md`, evidence txids, demo video (3-minute cadence, jitter 0, pre-recorded fallback), submission.

## Fixed decisions
- **Accounts:** one reserved account per order; never run proposals or auto-consolidation on it except the sweep.
- **Payee address:** must have an Orchard receiver. Since NU6.3, Ironwood shares the Orchard receiver. Hard-reject anything else.
- **Funding pool:** fund from Ironwood. Warn if the funding comes from Orchard, because the pool crossing reveals the amount.
- **Keys:** the Ironwood spend authorizing key is `SpendAuthorizingKey::from(usk.orchard())`.
- **Fees:** ZIP 317 (`FeeRule::standard()`). A 2-action Ironwood transaction pays 10,000 zat. Demo installments are at least 0.01 ZEC.
- **ZIP 318:** a draft for migration traffic, not a standard. We use its expiry only for fallback copies.
- **Privacy claim:** "Sender, recipient and amount are always hidden. If your device was offline in the hour before a payment, the chain may also tell that payment was pre-signed."

## Keeper contract (`crates/promissory-keeper`)
- `POST /v1/blobs`, body `{"ciphertext": "<base64 tlock-age binary>", "webhook": "<url>"?}` (webhooks off unless enabled in keeper config)
  - returns `{"blob_id", "delete_token", "round"}`
  - the round is read from the age header, never trusted from the client
  - body limit 64 KiB
  - refused with 400 if the round is more than `--max-horizon-days` (default 400) past the latest round, or more than a day in the past
  - rate limited before the body is read: 429 `{"error"}` with `Retry-After` when a token bucket is empty. Per client: `--client-rate` 60/min, `--client-burst` 300. Global: `--global-rate` 1200/min, `--global-burst` 3000. A burst of 0 disables that bucket.
  - `--rate-limit-key ip|none` (default `ip`). With `ip`, buckets are keyed by SHA-256(salt ‖ address), held in memory only, never logged or persisted. The salt and all buckets are replaced at each UTC day. IPv6 counts per /64. Use `none` behind Tor or a reverse proxy, where every client shares one address; the global bucket is then the only limit.
  - storage quota: 507 `{"error"}` if storing would exceed `--max-stored-mib` of held ciphertext (default 1024) or `--max-locked-blobs` locked blobs (default 100,000). 0 disables a cap. Check and insert are atomic.
- `GET /v1/blobs/:id` returns `{"blob_id", "round", "state", "txid"?, "mined_height"?, "last_error"?, "updated_at"}`
- `DELETE /v1/blobs/:id` with `Authorization: Bearer <delete_token>` returns 204. Only a hash of the token is stored, and it is compared in constant time.
  - `POST /v1/blobs/:id/delete` (same header, any body) does the same, for clients limited to GET and POST, such as the Tor HTTP client.
- `GET /v1/health` also reports `lightwalletd_via` and `drand_via` (`direct` or `tor`), `storage` usage against the quota, `max_horizon_days` and `rate_limit_key`.
- **States:** `locked → released → broadcast → mined`; terminal states `cancelled`, `expired`, `failed`, `deleted`.
- **Loop**, every 3 s: get the latest drand round. For each locked blob with round ≤ latest:
  1. Fetch the signature and decrypt.
  2. Parse the envelope and pick the copy for the next block (`Envelope::copy_for`).
  3. Broadcast via lightwalletd.
  4. Classify the result with Zebra's strings:
     - **success:** "already exists in mempool", "already in state"
     - **cancelled:** "duplicate nullifier", "has already spent some of its inputs", "nullifiers were revealed"
     - **expired:** "expired" / `ExpiredTransaction`
     - **no copy for this branch, or `WrongConsensusBranchId`:** retry next block
     - **lightwalletd unreachable** (`net::Transport`: connect failure, reset, timeout): retry next tick; doesn't count as an attempt
     - **anything else:** back off and retry, then `failed`
  5. Track mining with `GetTransaction`.
- **After release:** keep only the txid and status; drop the ciphertext and plaintext. No IP logs.
- **Tor (`--tor`, `--tor-dir`, default `tor` next to `--db`):** lightwalletd, drand and webhooks all go through arti.
  - The keeper refuses to start if Tor can't bootstrap. It never falls back to clearnet.
  - Polling (tip, `GetTransaction`) shares one connection on its own isolated circuit. Every broadcast opens a fresh connection on a newly isolated circuit, so broadcasts can't be linked to each other or to the polling by circuit.
  - drand beacons over Tor are verified against quicknet's public key.
  - Over Tor a tick takes about 5–10 s instead of 3 s.
  - A Tor exit won't connect to private addresses, so `--tor` needs a public lightwalletd (not regtest on 127.0.0.1).
