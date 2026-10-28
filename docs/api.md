# Promissory: integration contracts

Three surfaces other software can build on:

1. **Subscribe link**: a URL a payee creates and a payer opens.
2. **Keeper API**: blob upload, status and withdrawal. See "Keeper contract" in `plan.md`; served by `promissory-keeper`.
3. **Payer API**: the local HTTP API of `promissory-app`, the payer-side engine that holds the keys.

All amounts are integer zatoshis (1 ZEC = 100,000,000 zat). All times are unix seconds. JSON fields use snake_case.

## 1. Subscribe link

```
https://<any-host>/s#v=1&net=main&to=<UA>&amt=<zat>&every=<secs>&n=<count>[&memo=<text>][&start=<unix>][&keeper=<url>][&hook=<url>]
```

The parameters go in the URL **fragment**, so the host serving the page never receives them.
Encode values with `encodeURIComponent`. Readers decode a raw `+` as a space, as `URLSearchParams` writes it. Parameter order does not matter.

| Param | Required | Meaning |
|---|---|---|
| `v` | yes | Format version; always `1` |
| `net` | yes | `main` or `regtest` |
| `to` | yes | Payee unified address. Must contain an Orchard receiver; Ironwood shares it since NU6.3. Reject anything else. |
| `amt` | yes | Zatoshis per installment, integer > 0 |
| `every` | yes | Seconds between installments, integer ≥ 60 |
| `n` | yes | Number of installments, 1–120 |
| `memo` | no | Memo text, at most 400 bytes UTF-8. The payer app appends ` (k/n)`. |
| `start` | no | Unix time of the first installment. Default: when the payer arms the order. |
| `keeper` | no | Suggested keeper base URL. The payer may override it. |
| `hook` | no | Webhook for notices. Only honoured by keepers started with `--allow-webhooks`. |

Readers must ignore unknown parameters. A link with a missing or invalid required parameter is rejected with a message naming the parameter.

Presets for `every`: day 86400, week 604800, 30 days 2592000.

## 2. Keeper API

`plan.md`, "Keeper contract". Base URL, for example `http://127.0.0.1:8737`:
- `POST /v1/blobs`
- `GET /v1/blobs/{id}`
- `DELETE /v1/blobs/{id}`
- `GET /v1/health`

## 3. Payer API (`promissory-app`)

- **Binding:** `127.0.0.1:8738` by default; never exposed publicly.
- **CSRF guard:** every non-GET request must send `X-Promissory-App: 1`. A cross-site page cannot set that header without a CORS preflight, and `promissory-app` does not answer preflights.
- **Errors:** a JSON body `{"error": "<message>"}` with a 4xx/5xx status. An unknown order is 404; anything the engine refuses is 400.
- **Routing:** any non-API path serves the web UI's `index.html`, so client-side routes (`/new`, `/orders/3`, `/s#…`) load directly.

### Objects

```jsonc
// Order
{
  "id": 2,
  "status": "awaiting_funds" | "splitting" | "active" | "completed" | "cancelling" | "cancelled",
  "payee": "u1…",
  "amount_zat": 1000000,
  "memo": "SO gate",
  "every_secs": 180,
  "count": 4,
  "first_due": 1792967720,
  "keeper": "http://127.0.0.1:8737",
  "split_txid": "e93e…" | null,
  "created_at": 1792967120,
  "funding": { "zat": 4060000, "address": "u1…", "uri": "zcash:u1…?amount=0.0406", "installments": 4 } | null, // only while awaiting_funds
  "topup": Funding | null,        // rolling window: the shortfall for the next `installments`, when the treasury can't cover them
  "window": 2,                   // installments kept funded ahead; 0 = funded in full up front
  "batch": {"id": 1, "name": "Ada"} | null, // payroll batch and the CSV row's name
  "sweep_txid": "df8c…" | null,  // the cancel sweep, once cancelled
  "cancelled_at": 1792968120 | null,
  "paid_zat": 3000000,           // sum of installments paid so far
  "kind": "standing" | "inheritance", // inheritance = dead-man's switch (below)
  "remind_secs": 604800 | null,  // switches: reminder lead time before the date
  "renewed_from": 4 | null,      // switches: the switch this one renewed
  "renewed_to": 9 | null,        // switches: the switch that replaced this one
  "installments": [Installment]  // present on GET /orders/{id} only
}

// Installment
{
  "k": 0,                          // 0-based; display as k+1
  "due_unix": 1792967720,
  "round": 33388117,
  "release_unix": 1792967721,      // when the drand round is published
  "state": "reserved" | "armed" | "paid" | "cancelled",
  "keeper_state": "locked" | "released" | "broadcast" | "mined" | "cancelled" | "expired" | "failed" | "deleted" | null,
  "txids": ["2f2b…"],              // one per branch copy
  "mined_height": 224 | null,
  "blob_id": "c43b…" | null,
  "fresh_release_unix": 1792967421 | null, // fresh copy (signed while online), releases 5 min first
  "fresh_txid": "9a1c…" | null,
  "fresh_keeper_state": "locked" | null,
  "expiry_height": 1260 | 0 | null, // the pre-signed copy's expiry; 0 = never expires (switches only)
  "cancellable": true              // false once the keeper has it out or it is inside the race window
}
```

### Endpoints

| Method and path | Body | Returns |
|---|---|---|
| `GET /api/v1/info` | — | `{network: "main" \| "regtest", tip_height, synced_height, treasury_address, treasury_zat, treasury_spendable_zat, default_keeper, cancel_margin_secs}` |
| `POST /api/v1/sync` | — | same as `info`, after syncing |
| `GET /api/v1/orders` | — | `[Order]` without `installments` |
| `POST /api/v1/orders` | `{payee, amount_zat, every_secs, count, memo?, first_due?, jitter_max_secs?, keeper?, window?}` | `Order` with `funding`. `first_due` defaults to now + 600. |
| `POST /api/v1/orders/from-link` | `{link, first_due?, keeper?, window?, accept_hook?}` | Same as `POST /orders`, with fields taken from a Subscribe link. The link's `hook` is attached to uploads only with `accept_hook: true` (default off: it tells the keeper the payee's endpoint); if the keeper refuses webhooks the blob is uploaded without it. |
| `GET /api/v1/orders/{id}` | — | `Order` with `installments` (keeper state refreshed) |
| `POST /api/v1/orders/{id}/cancel` | `{margin_secs?, keep_blobs?, to?}` | `{sweep_txid: string \| null, cancelled: [k], may_still_pay: [{k, code: "released" \| "race_window", reason}]}` |

**Automatic progression:** a background task, every 15 s, syncs the wallet and moves orders along without any user step:
- `awaiting_funds` → split, once the treasury's spendable balance covers the funding;
- `splitting` → arm, once the split's notes are spendable;
- `active`:
  - fund the next rolling-window chunk from the treasury when it can (otherwise `topup` is shown);
  - arm newly funded installments;
  - sign fresh copies for installments released 10–45 min from now;
  - refresh keeper status.

A payer therefore only creates the order, pays the QR, and can then close the app.

### Payroll batches

A batch is many standing orders created from one CSV and funded with one payment. Each row becomes an ordinary order, created through the same engine path as `POST /orders`. The batch is only a grouping kept in `so.db` (tables `batches` and `batch_orders`). Payee names stay local and never go on chain.

**CSV format:** one row per payee, `name,ua,amount,cadence,count`.

| Column | Meaning |
|---|---|
| `name` | Payee label, 1–100 characters. Quote it if it contains a comma: `"Obi, Ada"` (`""` is a literal quote). |
| `ua` | Unified address with an Orchard receiver, for the app's network |
| `amount` | ZEC per installment as a decimal string, > 0, at most 8 decimals: `0.25` |
| `cadence` | `weekly` (604800 s), `biweekly` (1209600), `monthly` (a fixed 30 days, 2592000), or whole seconds ≥ 60 |
| `count` | Installments, 1–120 |

- A header row (exactly these five names) is optional.
- Blank lines and lines starting with `#` are skipped.
- At most 500 rows.
- Every row is validated before anything is created. Errors name the 1-based line number in the file; line 0 means the whole file.

```csv
name,ua,amount,cadence,count
Ada Obi,u1…,0.25,monthly,12
Tunde Bello,u1…,0.1,weekly,52
```

**Funding:** one ZIP-321 request to the treasury for the sum over orders of `funding_needed(amount, count)`, plus one change-output fee (5,000 zat once `count` ≥ 2; 0 for a 1-installment order). Each order's split returns change to the treasury, which costs an extra action. Automatic progression splits awaiting orders in id order, each once the treasury's spendable balance covers it. With one funding note, the orders split one after another as each split's change confirms: about one block apart on regtest.

```jsonc
// Batch
{
  "id": 1,
  "name": "October payroll",            // also each order's memo ("Payroll" when empty)
  "created_at": 1793012550,
  "first_due": 1793013383,              // shared by every order
  "status": "awaiting_funds" | "splitting" | "active" | "cancelling" | "completed" | "cancelled",
  "order_count": 3,
  "total_zat": 11100000,                // every installment of every order
  "paid_zat": 3700000,
  "funding": { "zat", "address", "uri" } | null, // what the orders still awaiting funds need
  "orders": [Order & { "name": "Ada Obi" }]       // CSV order; installments only on GET /batches/{id}
}
```

`status` summarizes the orders' statuses: `cancelling` if any order is cancelling; otherwise `awaiting_funds`, `splitting`, then `active` if any order is in that state; otherwise `cancelled` if any order was cancelled, else `completed`.

| Method and path | Body | Returns |
|---|---|---|
| `POST /api/v1/batches` | `{csv, name?, first_due?, keeper?}` | `{batch_id, name, orders: [Order & {name}], funding: {zat, address, uri}}`. `first_due` defaults to now + 600 and must be at least 60 s ahead. An invalid CSV is a 400 `{error, rows: [{line, message}]}`, and nothing is created. If creating an order fails part way, the orders already made are cancelled (nothing was funded) and the batch is removed. |
| `GET /api/v1/batches` | — | `[Batch]`, newest first, orders without installments |
| `GET /api/v1/batches/{id}` | — | `Batch` with each order's installments (keeper state refreshed). Unknown id: 404. |
| `POST /api/v1/batches/{id}/cancel` | `{margin_secs?, keep_blobs?}` | `{batch_id, results: [{order_id, name, sweep_txid, cancelled, may_still_pay} \| {order_id, name, error}]}`. Cancels every order not yet `completed` or `cancelled`, each as `POST /orders/{id}/cancel` does. One order failing does not stop the others. |

CLI: `promissory payroll import <file.csv> [--first-in <secs>] [--name <name>] [--keeper <url>]`, `promissory payroll status <id>`, `promissory payroll list`.

### Exchange rate (display only)

| Method and path | Returns |
|---|---|
| `GET /api/v1/rates?vs=ngn` | `{vs: "ngn", rate, zec_ngn, fetched_at, source: "coingecko"}`. `zec_<vs>` repeats `rate` (price of 1 ZEC). `vs` defaults to `ngn`. On failure: 502 `{error}`. |

`promissory-app` fetches the rate itself (CoinGecko simple price), so the app keeps one network path that can move to Tor, and caches it per currency for 5 minutes. The rate is for display only: amounts are always fixed in ZEC, and no rate ever reaches a signed transaction. The web UI only asks for a rate after the payer turns on naira display (the "Send home" flow does this).

### Proof of reservation

Opt-in. The payer exports, for one order, a viewing key that covers only that order's reserved account; the payee checks it against the chain. Nothing about the treasury or any other order is in it.

**Format:** `promissory-reserve:v1:<base64url(json)>`, base64url without padding (readers accept padding and ignore surrounding whitespace). The JSON:

```jsonc
{
  "v": 1,
  "net": "main" | "regtest",
  "ufvk": "uview1…",        // ZIP 316 UFVK, revision 0, with exactly ONE item: the Orchard FVK of the order's account
  "birthday": 952,          // birthday height of that account; scanning starts here
  "payee": "u1…",
  "amount_zat": 1000000,
  "note_zat": 1010000,      // each reserved note: amount plus that installment's fee
  "count": 3,
  "every_secs": 600,
  "first_due": 1793012788,
  "memo": "Pro plan",
  "keeper": "https://keeper.example",
  "blobs": [{ "k": 0, "round": 33403141, "blob_id": "44d8…" | null }]   // null until armed
}
```

Verifiers must reject a `ufvk` with any item other than a single Orchard FVK. Ironwood shares Orchard keys since NU6.3, so that one item sees the account's Ironwood notes.

**What the payee learns, and what not.**
- The notes the split created in the order account (count and value), and which are still unspent: the installments still reserved.
- For each spent note, whether the spend paid the payee. Installments are built with the account's outgoing viewing key (`OvkPolicy::Sender`), so their output to the payee's address can be recovered and matched.
- A cancel sweep carries **no** outgoing viewing key (`spend::sweep`). The payee sees only that those notes were spent, never where the funds went. That is intended: the proof must not lead to the payer's treasury.
- The keeper state of each blob (no keys needed).

Anyone holding the string can watch the order until it ends. It cannot spend.

| Method and path | Body | Returns |
|---|---|---|
| `GET /api/v1/orders/{id}/reservation` | — | `{order_id, proof}`. 404 for an unknown order. |
| `POST /api/v1/reservations/verify` | `{proof}` | `Report` (below). Uses a view-only wallet in `<dir>/payee-views/`, never the payer's wallet db. The first call for a proof scans from its birthday. |
| `POST /api/v1/reservations/keeper-status` | `{keeper, blob_ids: [string]}` | `[BlobView]`: keeper state only, no keys. promissory-app fetches it because keepers send no CORS headers. 1–120 IDs. |

```jsonc
// Report
{
  "net": "regtest",
  "terms": { "payee", "amount_zat", "note_zat", "count", "every_secs", "first_due", "memo", "keeper" },
  "tip_height": 979, "synced_height": 979,
  "funded": 3, "funded_zat": 3030000,
  "matches_terms": true,          // funded == count and every note == note_zat
  "reserved": 0, "reserved_zat": 0,
  "paid": 1, "paid_zat": 1000000,
  "swept": 2, "swept_zat": 2020000,  // spent without paying the payee: the payer's cancel
  "spent_unknown": 0,             // spent, but the spending transaction could not be fetched
  "cancelled_by_payer": true,     // swept > 0
  "next_release_unix": 1793013387 | null,  // earliest round of a blob still `locked`
  "notes": [{ "txid", "action_index", "value_zat", "received_height",
              "state": "reserved" | "paid" | "swept" | "spent_unknown",
              "spent_txid", "spent_height", "paid_zat", "paid_memo" }],
  "keeper": [BlobView],
  "warnings": ["2 reserved notes were spent without paying you: the payer cancelled"]
}

// BlobView
{ "k": 0 | null, "blob_id", "round", "release_unix", "state": BlobState | null, "txid", "mined_height", "error": string | null }
```

CLI: `promissory order reserve-proof <id>` prints the string. `promissory verify-reservation <proof> [--dir <payee-view-dir>]` prints the report; `--dir` defaults to `promissory-payee-view` / `promissory-payee-view-regtest` and must not be a spending wallet (one with `seed.hex` is refused).

### Notices

Banners for the payer, derived from order state.

| Method and path | Returns |
|---|---|
| `GET /api/v1/notices` | `[Notice]`, most urgent first |
| `POST /api/v1/notices/{id}/dismiss` | `{id}`. Kept in `<dir>/notices.json`. `keeper:*` notices cannot be dismissed. |

```jsonc
// Notice
{
  "id": "paid:1:0",            // paid:<order>:<k> | cancelled:<order> | keeper:<url> | funds-low (reserved)
  "kind": "renewal_paid" | "payment_failed_cancelled" | "keeper_unreachable" | "funds_low" | "switch_due",
  "tone": "ok" | "warn" | "danger",
  "title": "Payment 1 of 3 paid",
  "message": "Order #1: 0.01 ZEC to uregtest1m…m7j6xj, mined at height 975.",
  "order_id": 1 | null, "k": 0 | null, "at": 1793012788, "dismissible": true
}
```

- `renewal_paid`: an installment is mined; shown for 24 h after it was due.
- `payment_failed_cancelled`: the order is cancelling or cancelled and some installments can no longer pay; says how many the chain rejected as double-spends.
- `switch_due` (id `switch_due:<order>`): a dead-man's switch is inside its reminder window (`remind_secs` before its date, default 7 days): "Renew your switch or it pays <heir> on <date>". `danger` in the last hour. It goes away when the switch is renewed (the renewal has a later date), pays, or is cancelled. A renewed switch never raises `payment_failed_cancelled`.
- `keeper_unreachable`: an active order has armed installments at a keeper whose `/v1/health` does not answer within 3 s. Payments are delayed, not lost.
- `funds_low`: not emitted yet. Orders are fully reserved up front; the hook (`notices::funds_low`) is for rolling-window top-ups.

### Inheritance switch

A dead-man's switch is an order of one payment to an heir at a date, with `kind: "inheritance"`. While alive, the payer renews it to a later date; if they stop, the keeper releases it at the date and the heir is paid with nobody's help. It is funded, split and armed like any order and listed by `GET /orders`.

- **Expiry 0.** Its pre-signed copy has expiry height 0 (never expires), because it may fire months later with the payer gone. This is the one place Promissory uses expiry 0. It is a known fingerprint: the transaction can be recognised on chain as pre-signed. Sender, heir and amount stay hidden. Ordinary orders keep the ZIP 318 expiry for fallback copies.
- **No fresh copy.** `freshen` skips switches: a fresh copy has a normal expiry and needs the payer online near the date.
- **Renew** is one transaction: it spends the old switch's reserved note plus one treasury note (for the 0.0001 ZEC fee) and outputs the new switch's reservation plus change to the treasury. No outgoing viewing key, like a cancel sweep. The old pre-signed copy can then never pay: its note is spent. By default the old blob is withdrawn from the keeper; with `keep_blob` the keeper releases it at its round and the chain rejects it as a double-spend. A switch that was never funded is re-dated without a transaction. Renewing is refused once the keeper has the old copy out, or within `margin_secs` (default 600) of its round.

| Method and path | Body | Returns |
|---|---|---|
| `POST /api/v1/switches` | `{heir, amount_zat, at, memo?, remind_secs?, keeper?}` | `Order` with `funding` and `kind: "inheritance"`. `at` (unix seconds) must be at least 10 minutes out. `remind_secs` defaults to 7 days; `memo` to "Inheritance". |
| `POST /api/v1/switches/{id}/renew` | `{at, margin_secs?, keep_blob?}` | `{old_id, new_id, txid: string \| null, withdrew_blob, order: Order}`. `at` must be later than the current date. The new switch is `splitting` until the renewal confirms, then armed by the progression pass. The old one becomes `cancelled` with `renewed_to`. 404 if `{id}` is not a switch. |

CLI: `promissory switch create --heir <ua> --amount <zec> --at <unix|+secs> [--remind-days 7] [--memo] [--keeper]`, then `promissory order split/arm` as for any order; `promissory switch renew <id> --at <unix|+secs> [--margin 600] [--keep-blob]`.

### Receipts

A receipt proves one payment: "X ZEC went to address Y in transaction T". It opens that one output and nothing else.

**Format:** `promissory-receipt:v1:<base64url(json)>`, no padding (readers accept padding and ignore surrounding whitespace):

```jsonc
{
  "v": 1,
  "net": "main" | "test" | "regtest",
  "txid": "34e0…1511",          // display byte order
  "pool": "ironwood" | "orchard",
  "action_index": 0,            // index in that pool's bundle
  "ock": "de26…2d04",           // 32-byte outgoing cipher key for that one action, hex
  "payee": "u1…",               // claimed payee (must have an Orchard receiver)
  "value_zat": 1000000          // claimed value
}
```

**Cryptography.** Installments are built with the order account's outgoing viewing key (`OvkPolicy::Sender`). Orchard and Ironwood derive a per-output key from it, `ock = PRF^ock(ovk, cv_net, cmx, epk)` over that action's public value commitment, note commitment and ephemeral key (protocol spec § 4.20.3, 5.4.2). The payer's app computes it (`orchard::note_encryption::IronwoodDomain::derive_ock`). The verifier decrypts the action's `out_ciphertext` with it to `(pk_d, esk)`, decrypts the note, and checks `epk` and that the note's commitment equals the on-chain `cmx` (`zcash_note_encryption::try_output_recovery_with_ock`). It then checks that the recipient equals the claimed payee's Orchard receiver and that the value equals the claimed value. The commitment binds value and recipient, so a receipt cannot be edited to show another payee or amount. The OCK is a PRF output, so it reveals nothing about the OVK and opens no other output. The OVK itself is never put in a receipt, because it would open every payment from the account.

**What it does not prove:** who is showing it. Only the sender's wallet can derive an OCK, but a receipt is a bearer string that anyone it was shown to can pass on. See ZIP 311 in `integrating.md`.

| Method and path | Body | Returns |
|---|---|---|
| `GET /api/v1/orders/{id}/installments/{k}/receipt` | — | `{order_id, k, txid, action_index, receipt}`. `k` is 0-based. 400 unless that installment is paid. It finds the mined copy (fresh or fallback) through lightwalletd. It reads the wallet but does not sync it. |
| `POST /api/v1/receipts/verify` | `{receipt}` | `{txid, pool, action_index, value_zat, payee, memo: string \| null, height}` if valid. Otherwise 400 `{error}`, e.g. "the payment went to a different address than the receipt claims", "the payment was 1000000 zat, not the 1000001 zat the receipt claims", "the receipt's key does not open action 0 of …", or "not mined yet". Uses no keys. |

CLI: `promissory order receipt <id> <k>` (k 0-based) prints the string. It opens the store read-only and the wallet without migrating or syncing, so it is safe to run beside a running `promissory-app`. `promissory verify-receipt <receipt>` needs no wallet. Web: a "Get receipt" button on each paid installment, and a public `/receipt` page; a link `/receipt#promissory-receipt:v1:…` keeps the receipt in the URL fragment, which is never sent to a server.

### Payee webhooks (off by default)

A Subscribe link may carry `hook`, a payee URL. The keeper POSTs `{blob_id, txid, status}` there on each state change, **only** if it runs with `--allow-webhooks`; otherwise it rejects uploads that carry one. They are off by default because a webhook tells the keeper the payee's endpoint and links every blob to that payee, which the keeper otherwise never learns. A payee that wants status without that can poll the keeper with blob IDs or use a proof of reservation. `promissory_core::notices::upload_with_webhook` uploads with a webhook after checking the keeper's `/v1/health` reports `webhooks: true`.

### Agent allowances

"Up to X per period" for an AI agent's own wallet. Each allowance is one ordinary standing order to the agent's address: the period's cap is cut into `chips` equal chips, released one every `period_secs / chips`, with `window = chips` so one period is funded at a time. Design and trade-offs: `allowances.md`. Kept in `so.db`, table `allowances`, linked to the order.

```jsonc
// Allowance
{
  "id": 1, "order_id": 14,
  "status": "awaiting_funds" | "starting" | "active" | "completed" | "stopping" | "stopped" | "replacing" | "replaced",
  "agent_ua": "u1…", "label": "Research agent",
  "cap_zat": 3000000, "period_secs": 600, "chips": 3, "chip_zat": 1000000, "every_secs": 200,
  "periods": 2, "first_due": 1793033000, "created_at": 1793032400,
  "stopped_at": null, "replaced_by": null,      // replaced_by: the allowance a cap change created
  "current_period": 1, "period_start": 1793033000, "period_end": 1793033600,
  "spent_this_period_zat": 1000000,             // released to the agent this period
  "remaining_this_period_zat": 2000000,         // this period's chips not released yet
  "released_total_zat": 1000000,
  "next_release_unix": 1793033200 | null,
  "funding": Funding | null, "topup": Funding | null, "keeper": "http://127.0.0.1:8737"
}
```

| Method and path | Body | Returns |
|---|---|---|
| `POST /api/v1/allowances` | `{agent_ua, cap_zat, period_secs, chips, periods, first_due?, label?, keeper?}` | `Allowance` with `funding`. `chips` 1–30, `periods` ≥ 1, `chips × periods` ≤ 120; `cap_zat` and `period_secs` multiples of `chips`; a chip ≥ 100,000 zat; `period_secs / chips` ≥ 60; `first_due` default now + 600. |
| `GET /api/v1/allowances` | — | `[Allowance]`, newest first, from local state |
| `GET /api/v1/allowances/{id}` | — | `Allowance`, keeper state refreshed. Unknown id: 404. |
| `POST /api/v1/allowances/{id}/stop` | `{margin_secs?}` | `{allowance, sweep_txid, cancelled, may_still_pay}`: the order's cancel. Every unreleased chip returns to the treasury; chips inside the race window may still pay. |

### Agent requests

What an agent asks for through `promissory-mcp` (`mcp.md`). Creating one moves nothing; only the payer's approval creates an order. Kept in `so.db`, table `agent_requests`.

```jsonc
// Request: the ask's fields are inline, tagged by `kind`
{
  "id": 3, "kind": "oneoff", "payee": "u1…", "amount_zat": 2500000, "due": 1793040000, "memo": "Invoice 7",
  // or: "kind": "allowance_change", "allowance_id": 1, "new_cap_zat": 6000000,
  "reason": "why the agent asks" | absent,
  "status": "pending" | "approved" | "rejected" | "expired",
  "created_at": 1793032400, "decided_at": null,
  "order_id": null,          // the one-off's order, once approved
  "allowance_id_new": null,  // the replacement allowance of an approved cap change
  "error": null              // why the last approval attempt failed; the request stays pending
}
```

| Method and path | Body | Returns |
|---|---|---|
| `POST /api/v1/requests` | `{kind: "oneoff", payee, amount_zat, due?, memo?, reason?}` or `{kind: "allowance_change", allowance_id, new_cap_zat, reason?}` | `Request` (`pending`). Strict: unknown fields are refused. `due` ≥ now + 60 s and within 400 days; `memo` ≤ 400 bytes; `reason` ≤ 200 characters. The payee is checked for the app's network and an Orchard receiver; a cap change against the allowance. **429** while 10 requests are pending. |
| `GET /api/v1/requests` | — | `[Request]`, newest first. Pending requests older than 7 days become `expired`. |
| `GET /api/v1/requests/{id}` | — | `Request`. Unknown id: 404. |
| `POST /api/v1/requests/{id}/approve` | — | `{request, order?, allowance?}`. A one-off becomes a 1-installment order due at `max(due or now + 600, now + 300)`; a cap change replaces the allowance from its next period. Not pending: **409**. If the engine fails, 400 and the request stays pending with `error`. |
| `POST /api/v1/requests/{id}/reject` | — | `Request` (`rejected`). Not pending: 409. |

The web UI shows these at `/allowances` and `/requests` (approve link: `/requests/{id}`).
