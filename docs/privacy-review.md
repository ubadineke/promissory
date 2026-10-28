# Privacy review

Every way a Promissory payment could be told apart from an ordinary shielded payment, or linked to its payer or to other payments, and what the code does about it. Checked against the code as of 2026-10-26. "Residual" is what remains, stated plainly.

**Claim we make:** sender, recipient and amount are always hidden. If your device was offline in the hour before a payment, the chain may also tell that payment was pre-signed.

## On chain

| # | Fingerprint | What an observer learns | Mitigation in code | Residual |
|---|---|---|---|---|
| 1 | **Anchor age.** The Ironwood anchor is public. A pre-signed fallback copy is mined long after its anchor. | That the transaction was built well before it was mined, i.e. probably pre-signed. | A **fresh copy** is signed while the payer is online, 10–45 min before release, against the anchor current at signing (`order::freshen`), and releases 5 min before the fallback. | Offline payer: the fallback's anchor is as old as the time since arming. Online payer: the anchor is up to ~45 min old, against ~4 min for a wallet that sends immediately. Weak, but present. |
| 2 | **Shared anchor.** Copies signed in one session share an anchor. | Payments mined days apart with the same anchor link to one signer; across orders, to one payer. | **One signature per anchor height, wallet-wide** (`store::anchor_in_use`). `arm` and `freshen` sign at most one installment per block and pick up the rest in later passes. | None for anchors. Arming N installments takes N blocks (~75 s each on mainnet). |
| 3 | **Expiry height.** | A pre-signed copy's expiry gives away when it was built. | **Fallback:** the ZIP 318 rolling expiry for the *predicted mining height*, a pure function of the height shared by everything in that window. **Fresh copy:** the ordinary delta (40 blocks, or 120 after NU7) from the predicted mining height, the same shape as a send made then. Heights are predicted from the measured block spacing. | The fallback's rolling expiry on a non-migration Ironwood→Ironwood transaction is itself uncommon. The fresh copy is the default path while online. |
| 4 | **Expiry 0** (inheritance switch only). | That the transaction never expires: a strong "pre-signed" marker. | Used only for `kind = inheritance`, and labelled in the UI. | Accepted by design: a switch must fire with the payer gone. |
| 5 | **Pool crossing.** Orchard ↔ Ironwood moves value through the value balance. | The amount. | Payee addresses must have an Orchard receiver, which pays into Ironwood (`txbuild::orchard_receiver` rejects others). Installments spend Ironwood only. The funding page warns to pay from Ironwood. | If the payer's wallet funds the treasury from Orchard, that one funding amount is visible. Not under our control; we warn. |
| 6 | **Transaction shape.** | Unusual action counts single transactions out. | Installments are 1 spend → 1 output, padded to 2 actions: the commonest shape. Fees are standard ZIP 317. | The split (1 → N notes) and the cancel sweep (N → 1) have larger shapes, like any batch payment or consolidation. |
| 7 | **Dual NU7 copies.** | Nothing extra: only one copy is ever mined. | Both copies are v6, the same size, and differ only in branch ID. | None. |
| 8 | **Release timing.** | Payments landing exactly on drand round times or due times. | Per-installment **jitter**, uniform random up to the maximum (default 3 h on mainnet, `SoNetwork::default_jitter_secs`). The fresh copy shifts by 5 min. | The payee sees the timing anyway. |
| 9 | **Cancel sweep.** | Where the swept funds went. | One sweep transaction with **no OVK** (`spend::sweep` passes `None`). | The sweep spends the order's notes in one transaction, so the cancelled installments are linked to each other, though not to the payer's other orders. |

## Parties

| Party | Sees | Never sees | Notes |
|---|---|---|---|
| **Keeper** | Ciphertext until the round. Then the transaction, which is about to be public anyway. Blob IDs and status queries. | Keys, amounts, recipients, the payer's identity. | No IP logs. Rate-limit buckets live in memory, keyed by a salted hash that rotates daily (`limits.rs`). It can't release early; drand's threshold would have to collude, and even then it could only pay early, never redirect. |
| **Keeper + webhook** | The payee's notice endpoint and which blobs pay it. | Amounts. | **Opt-in twice:** the keeper needs `--allow-webhooks`, and the payer must tick "send notices" (`accept_hook`). Off by default. |
| **lightwalletd** | The payer's IP while syncing, splitting and sweeping. The keeper's IP when broadcasting. | Contents. | `--tor` on the CLI, promissory-app and keeper puts lightwalletd, the keeper, drand and the rate source behind Tor, on isolated circuits. Without Tor, a lightwalletd operator can tag the keeper's broadcasts as scheduled payments. |
| **drand** | Whoever polls for beacons. | Anything about payments: encryption to a round is offline. | Polls go through Tor with `--tor` (`TorDrand`, which verifies every beacon). |
| **Payee** | Amount, schedule, memo `(k/n)`, whether a payment came. With proof of reservation: the order account's funding and spends. | The payer's other funds and orders, and where a cancel sent the money. | The proof of reservation key is Orchard-only and covers that order's account only. Sweeps carry no OVK, so the payee sees "spent without paying me", never the destination. |
| **Rate source** (CoinGecko) | IP plus "wants ZEC/NGN". | Anything about payments. | Only fetched after the payer turns on Send home. Through Tor with `--tor`. |
| **promissory-app** (payer's own machine) | Everything: it holds the keys. | — | Binds to localhost only. Writes need the CSRF header `X-Promissory-App`. |

## Fixed in this review
- **Anchor shared across a session.** `arm` used to sign every installment against one anchor. It now signs one per anchor height, wallet-wide, and `freshen` does the same.
- **No jitter by default.** The default was 0 everywhere. Mainnet now defaults to up to 3 h; regtest stays at 0 for tests.
- **Clearnet calls under `--tor`.** Rate fetches and webhook-upload health checks used to go direct even with `--tor`. All of them now use the Tor client when one is installed.

## Open
- **Anchor age for offline payers (#1).** This is inherent: a payment the payer didn't sign close to release can only carry an old anchor. A wallet that runs a background task (as the payer app does) narrows it to under an hour.
- **Keeper broadcasting many payers' transactions.** Without Tor, a lightwalletd operator can group them. Run the keeper with `--tor`, or have payers upload to several keepers.
