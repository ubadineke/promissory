# Threat model

What each party can do if it misbehaves, what that costs the payer or payee, and why the design limits it.

## Assets
- The payer's reserved funds.
- The payee's expected payments.
- Privacy: who paid whom, how much, and when.

## Adversaries and outcomes

| Adversary | Can | Cannot | Why |
|---|---|---|---|
| **Keeper (malicious or offline)** | Delay a payment by not broadcasting it. Learn a transaction's txid once its round is out. | Steal or redirect funds. Release early. Change amount or recipient. Stop a cancel. | It holds fully signed transactions that pay only the payee, and can't decrypt them before the round. Payers can upload to several keepers; anyone can run one. |
| **drand (stalled)** | Delay every release until it resumes. | Steal. | Encryption is to a future round; no beacon, no decryption, but funds stay in the payer's reserved notes, which the payer can still sweep. |
| **drand threshold (colluding)** | Compute future beacons and release payments early. | Redirect funds. | The transactions still pay only the payee, and this would expose drand's network to everyone. |
| **lightwalletd (malicious)** | Refuse to relay. Watch IPs. Lie about chain state to the payer app. | Forge transactions or steal. | Zebra validates everything; a lying server can at worst make the app wait or retry. Use several endpoints, or Tor. |
| **Payee** | Know when payments stop. | Pull more than authorized, or change the schedule. | Only the pre-signed amounts exist. |
| **Payer** | Cancel at any time before a payment is broadcast. | Cancel a payment already broadcast; claw back a mined payment. | Cancel is a double-spend race won by whoever reaches the mempool first. The app enforces a 10-minute margin and shows "may still pay". |
| **Observer of the chain** | Spot that a payment was pre-signed, if the payer was offline before it (old anchor, rolling expiry). Link a cancel sweep's inputs together. | Learn sender, recipient or amount. Link two payments by anchor. | One signature per anchor height; fresh copies while online. See [`privacy-review.md`](privacy-review.md). |
| **Network upgrade (NU7)** | Invalidate signatures made for the old branch. | Break orders signed with dual copies. | Copies for both branches share one note; the keeper picks the one valid for the next block. Re-sign if NU7's branch ID changes before activation. |
| **Malicious payer-app user on the same machine** | Read `seed.hex` and spend everything. | — | Same as any hot wallet: promissory-app binds to localhost and needs the `X-Promissory-App` header on writes, but the device must be trusted. |
| **Agent using the MCP server** | Spend within its allowance; request more. | Create payments, raise its cap, or cancel orders without the payer's approval. | The MCP tools only create pending requests; promissory-app needs the payer to approve them in the UI. |

## Failure modes found by testing, and their fixes
See "Incidents" in [`evidence.md`](evidence.md):
- the keeper stalled on a drand relay with no timeout;
- promissory-app deadlocked on a lock guard;
- network calls had no timeouts;
- fresh copies expired under fast blocks.

## Out of scope
- A compromised payer device (as for every wallet).
- Denial of service against public infrastructure (lightwalletd, drand). Payments are delayed, never lost.
