# Evidence

Every transaction we create, with its purpose and height.

## Kill gate: regtest (2026-10-25)

Local chain from `regtest/regtest.sh`: Zebra 7.0.0 and lightwalletd v0.6.0. All upgrades through NU6.3 (Ironwood) are active from block 1, and NU7 is at 5000, so this chain is on the same branch mainnet runs today.

| Step | Height | Result |
|---|---|---|
| Coinbase to payer wallet | 4 | 6.25 ZEC, one Ironwood note |
| Coinbase maturity | 5–114 | Note spendable at 114 |
| Pre-sign send-max to payee, `expiry 0`, not broadcast | tip 114 | `6a7f428d02a3138e0ba1b989dc248ee80878a9df0717a0c575ce9c9917b9ea87`; branch NU6.3; 9,166 B; fee 10,000 zat |
| Early broadcast attempt | 114 | Refused by the CLI's hold guard |
| Hold | 115–149 | Chain never saw the tx (`GetTransaction` → unknown) |
| Broadcast after 35 blocks held | 149 | Accepted into the mempool |
| Mined | **150** | v6, `expiryheight 0`, `locktime 0`, 0 transparent in/out, 2 Ironwood actions, value balance 10,000 zat (the fee) |
| Payee balance | 150 | 6.2499 ZEC Ironwood |
| Payer balance | 150 | 0 |

Conclusion: a fully shielded Ironwood transaction can be pre-signed with no expiry, held unbroadcast for 35+ blocks with its anchor ageing the whole time, and then mined. Zebra checked neither the anchor's age nor the expiry.

## Engine gate: regtest (2026-10-25)

The same local chain, auto-mining every 15 s. `promissory-keeper` runs on 127.0.0.1:8737; quicknet is live drand.
Order 2: 4 × 0.01 ZEC to the payee wallet, every 180 s. Watcher: `regtest/gate-engine.sh`.

| Step | Time / height | Result |
|---|---|---|
| Split: treasury → 4 notes of 0.0101 ZEC in the order account | — | `e93e6973…1cce` |
| Arm: Builder-signed fallback copies (NU6.3, expiry 69,120), tlocked to rounds 33388117 / 177 / 237 / 297, uploaded | 23:28 | 4 blobs locked |
| Payer app closed; nothing payer-side runs | 23:28–23:42 | — |
| #1 released by keeper | **mined at 224** | `2f2b3e2b…b808` |
| #2 released by keeper | **mined at 235** | `3340544f…5796` |
| #3 released by keeper | **mined at 247** | `5917bfbc…7c75` |
| Payer cancels (`--margin 60 --keep-blobs`): one sweep of #4's note back to the treasury | 23:42 | `df8cfe2e…d0f2` |
| #4 released by keeper at its round | 23:44 | **Rejected by Zebra**: `ironwood double-spend: duplicate nullifier`; keeper state `cancelled` |
| Payee balance | 258 | 0.03 ZEC (exactly 3 installments) |

Also: order 1 was split but never armed. Its cancel swept all 4 notes in one Builder transaction (`9bcece73…25b0`), returning 0.0402 ZEC (0.0404 minus the 4-action fee).

## Payer API: link to completed order, no manual steps (regtest, 2026-10-26)

`promissory-app --network regtest` on 127.0.0.1:8738, same chain and keeper.

| Step | Time / height | Result |
|---|---|---|
| `POST /api/v1/orders/from-link` with `#v=1&net=regtest&to=…&amt=1000000&every=180&n=3&memo=Link%20test` | 10:42 | Order 3, funding 0.03045 ZEC (3 × 0.0101 + 0.00015 split fee) |
| Automatic progression: the treasury already covered the funding, so promissory-app split it | next pass | split `abc6a40e…` |
| Automatic progression: armed once the split confirmed | 10:44 | 3 blobs locked |
| #1, #2, #3 released by the keeper | **680, 692, 703** | Order `completed`, `paid_zat` 3,000,000 |

## NU7 gate: regtest (2026-10-26)

A second, fresh chain from `REGTEST_PROFILE=nu7 regtest/regtest.sh` (Zebra RPC 18332, lightwalletd 9167): everything through NU6.3 at block 1, **NU7 (`0x77190ad9`) at 300**. Mined on demand only. A separate `promissory-keeper --nu7-height 300` runs on 127.0.0.1:8747.
Order 1: 2 × 0.01 ZEC to the payee wallet, every 240 s, first due 30 min after create. Raw copies were read back with `promissory dev open-blob` from ciphertexts copied out of our keeper's db at arm time. Rejections are Zebra's `sendrawtransaction` replies, verbatim.

| Step | Time / height | Result |
|---|---|---|
| Split: treasury → 2 notes of 0.0101 ZEC | 248 | `57e31f74…3234` |
| Arm at tip 278 (target 279 = H−21, anchor 276). Round 1 was 1752 s away, so the predicted height at 75 s/block was 302 ≥ 300, and both installments got two copies. | 10:48, tip 278 | #1: NU6.3 `1ae6a2b4…df27` (target 299), NU7 `9c3454e5…0097`. #2: NU6.3 `63454ed2…054e` (target 299), NU7 `85522039…fbfe`. Both blobs uploaded. |
| #1's blob withdrawn from the keeper (`DELETE`, 204), kept for manual tests | 10:48 | — |
| #1 NU7 copy submitted **before** activation | 11:18, tip 298 (next block 299, NU6.3) | **Rejected**: `failed to validate tx: WtxId("private"), error: transaction did not pass consensus validation: transaction uses an incorrect consensus branch id` (code −25) |
| #1 NU6.3 copy submitted at the boundary | tip 299 (`nextblock` `77190ad9`) | **Rejected**, same error. There is no grace window: the mempool validates for the next block's branch. |
| Mine past H | 300–303 | NU7 `active`, `chaintip` `77190ad9` |
| #1 NU6.3 copy submitted **after** activation | tip 303 | **Rejected**: `… transaction uses an incorrect consensus branch id` |
| #1 NU7 copy submitted after activation | tip 303 | Accepted; **mined at 304**: v6, `versiongroupid d884b698`, `expiryheight 207360`, `locktime 0`, 9,166 B |
| #1 NU6.3 copy again, with its NU7 sibling mined | tip 304 | Still `… incorrect consensus branch id`: the branch check comes before nullifier checks. Resubmitting the NU7 copy gives `transaction was committed to the best chain`. |
| #2 released by the keeper at its round | 11:21:52, tip 304 | Keeper picked the NU7 copy for block 305: `85522039…fbfe` into the mempool |
| #2 NU6.3 copy submitted, with the NU7 copy in the mempool | 11:22, tip 304 | **Rejected**: `failed to validate tx: WtxId("private"), error: transaction did not pass consensus validation: transaction uses an incorrect consensus branch id` |
| #2 NU7 copy mined | **305** | v6, `expiryheight 207360`; keeper state `mined` at 305 |
| #2 NU6.3 copy after #2 was mined | tip 305 | **Rejected**, same branch-id error |
| Payee balance | 315 | 0.02 ZEC (both installments, each paid by its NU7 copy) |

Conclusion: both copies were signed 21 blocks before NU7. Zebra refused each copy on the wrong side of the boundary with the same error. The keeper's `copy_for` picked the right copy on its own, and the installment was mined after activation. The default chain (`so-zebra`, NU7 at 5000) was not touched.

## Proof of reservation: regtest (2026-10-26)

Same chain and keeper (127.0.0.1:8737). A second `promissory-app` on 127.0.0.1:8758 with its **own fresh seed** (`promissory-data-regtest-reserve`), funded with 0.5 ZEC by the main app's order 4 (mined at 928). The payee is a separate fresh wallet. Verification runs in a third directory, `promissory-payee-view-regtest`, which holds only an imported view-only account.
Order 1: 3 × 0.01 ZEC every 600 s, memo "Reserve proof test". The proof (1,009 chars, `promissory-reserve:v1:…`) carries a revision-0 UFVK with a single Orchard item.

| Step | Time / height | Result (`promissory verify-reservation`) |
|---|---|---|
| Proof exported before the split, verified over the API | 952 | `funded: 0`, warning "only 0 of 3 installments are funded" |
| Split: treasury → 3 notes of 0.0101 ZEC in the order account | 954 | `5918bbf2…82dd` |
| Armed; proof exported and verified from the payee-view dir | 970 | **`funded: 3 of 3 notes, 0.0303 ZEC (matches the terms)`**, `reserved: 3`, keeper #1–#3 `locked` |
| #1 released by the keeper | **mined at 975** | `756395a3…8124` |
| Verified again | 975 | **`paid: 1 (0.01 ZEC to the payee)`**, `reserved: 2`. The note was classified as paid by recovering the installment's output with the account's OVK and matching the payee's Orchard receiver; memo "Reserve proof test (1/3)". |
| Payer cancels (`margin_secs` 60): one sweep of #2's and #3's notes; their blobs withdrawn | mined by 979 | sweep `97629318…da61` |
| Verified again | 979 | **`CANCELLED BY PAYER: 2 note(s), 0.0202 ZEC, spent without paying the payee`**; both notes `swept`, spent in `97629318…`, with no destination (the sweep has no OVK); keeper #2, #3 `deleted` |

The payer app's `GET /notices` returned "Payment 1 of 3 paid" after 975, then also "Order #1: payment failed, cancelled" after the sweep. Screenshots: `apps/web/screenshots/*-payee-17-funded.png`, `*-payee-18-one-paid.png`, `*-payee-19-cancelled.png`, `*-16[abc]-*share-proof*.png`, `*-20-payee-keeper-only.png`, `*-21-notices.png`, `*-22-notices-cancelled.png`.

## Fresh copy: regtest (2026-10-26)

Order 8: 2 × 0.01 ZEC every 300 s. promissory-app was online, so its progression pass signed a **fresh copy** of each installment:
- current anchor;
- targeted at the predicted mining height, using the measured block spacing;
- normal expiry delta;
- tlocked 100 rounds (5 min) before the pre-signed fallback.

| Installment | Fresh copy | Fallback (pre-signed at arm) |
|---|---|---|
| #1 | `299cc65a…51a4`, **mined at 1081** | `0150abf2…07df`, rejected as a double-spend → keeper `cancelled` |
| #2 | `5276b210…bc56`, **mined at 1081** | `8a0b3548…c762`, rejected as a double-spend → keeper `cancelled` |

Both installments show `paid`; the order is `completed`. Releases were late because of the keeper stall below. Once the keeper recovered, both copies of each installment were due in the same tick: the chain took the fresh copy and refused the fallback, as designed.

The first attempt (order 5) showed why the target must be the predicted mining height. Built at tip + 1 with the normal 40-block delta, the fresh copies expired before release, because the local miner runs at 15 s per block rather than 75 s. The fallbacks paid instead (986, 987). Heights are now predicted from the observed spacing (`spacing::recent_block_secs`).

## Rolling window: regtest (2026-10-26)

Order 7: 4 × 0.01 ZEC every 180 s, `window` 2.
- **First funding request:** 0.0203 ZEC, covering 2 installments, not 0.0406.
- **#1 and #2:** mined at 986 and 987.
- **Top-up:** promissory-app saw no funded installments left ahead, split the next 2 notes from the treasury on its own, and armed them.
- **#3 and #4:** mined at 1007; the order is `completed`.

## Incidents found while testing (2026-10-26)

- **Keeper loop hung for 24 min** at tip 1037: a thread sample showed `drand_core::HttpClient::latest` blocked in `ureq` with no timeout, on a relay connection that never answered.
  - **Fixed:** every blocking drand call now goes through `tlock::bounded` (30 s limit, tested), in the keeper, the order engine, reservation checks and promissory-app.
- **promissory-app deadlock**, introduced while shortening lock hold times: `match ticker.lock().await.begin_pass().await { … }` keeps the guard for the whole `match`, so the per-order `lock()` inside waited on itself. Found within one pass: the API stopped answering, a thread sample showed 0% CPU and no sockets.
  - **Fixed:** the result is bound to a variable first. API latency during a pass is now ~40 ms.
- **No network timeouts** in promissory-app's lightwalletd and keeper clients, the same class of bug as the drand hang.
  - **Fixed:** lightwalletd gets a 20 s connect / 120 s call timeout; the keeper gets 15 s / 60 s direct and 120 s over Tor.
- **Host overload** (load average ~200 on 14 cores, from unrelated processes) starved Zebra; `generate` calls took up to 30 min.
  - **No code change:** the keeper's broadcasts sat in the mempool and were mined once blocks resumed.

## Payroll batch: regtest (2026-10-26)

A separate `promissory-app` built from this tree (`target-payroll`) on 127.0.0.1:8748, with its **own fresh wallet** `promissory-data-regtest-payroll`. It was not a copy: a copy shares the seed, so both apps would derive the same order accounts. The new wallet was funded through the main app: order #6 on 8738 paid 0.5 ZEC to its treasury in one installment, mined at 934. Same chain, same keeper (8737).

CSV (3 rows; two payees are the payee wallet, one the second regtest address; one name contains a quoted comma):

```csv
name,ua,amount,cadence,count
Ada Obi,uregtest13543…kuwwxj,0.01,900,3
Tunde Bello,uregtest1ynahn…gm2y9q,0.012,900,3
"Chidi, N.",uregtest13543…kuwwxj,0.015,900,3
```

| Step | Time / height | Result |
|---|---|---|
| `POST /api/v1/batches` with 2 bad rows (count 0; bad UA and cadence `daily`) | 12:01 | **400** `{error, rows: [{line: 2, …}, {line: 3, …}, {line: 3, …}]}`. Nothing created: `GET /batches` and `GET /orders` both `[]`. |
| `POST /api/v1/batches` (`name` "Regtest payroll", `first_due` now + 840) | 12:02:30 | Batch 1 with orders 1–3, `awaiting_funds`. One funding request: **0.1125 ZEC** (11,250,000 zat = 3,045,000 + 3,645,000 + 4,545,000 + 3 × 5,000 change-output fee). |
| Combined funding | — | The treasury already held the 0.5 ZEC funding note (self-funded, as on regtest), which covers 0.1125. No separate payment was needed. |
| Automatic progression: splits one after another from the single treasury note, each waiting for the previous split's change to confirm | 12:02:37, 12:03:38, 12:04:25 | `ced5a8d8…b84d`, `a953a9a4…f934`, `9ebe0133…26fa` |
| Armed (+ fresh copies) | 12:05:29, 12:06:12, 12:07:12 | 9 blobs locked at the keeper, rounds 33403339 / 33403639 / 33403939 |
| Keeper releases. The host slept from about 12:12 to 15:11, so every round was overdue at wake-up. | **987–991** | All 9 installments paid; batch `completed`, `paid_zat` 11,100,000. #1 of orders 2 and 3 mined at 989 (`7527782d…c0b5`, `754fe65e…16a1`). |

Batch 1 completed before it could be cancelled, so a second batch tested cancel: the same CSV with cadence 1800 s.

| Step | Time / height | Result |
|---|---|---|
| `POST /api/v1/batches` ("Regtest payroll 2", first due 16:32:03) | 16:22:08 | Batch 2 with orders 4–6, funding 0.1125 ZEC. Split and armed automatically by 16:26. |
| #1 of each order released by the keeper | **1036** | `6211dab5…12c0`, `2441c080…fdaf`, `758b39a7…93c1`: one payment per payee |
| `POST /api/v1/batches/2/cancel` | 16:49:08 (#2 due 17:02, inside the 600 s race window) | One sweep per order: `31768ccb…4cc4`, `ac7bb60f…9b96`, `17b37097…35a2`. Each `cancelled: [2]` (#3) with `may_still_pay: [{k: 1, code: "race_window"}]` (#2). |
| #2 released at its round | **1090** | Paid, as `may_still_pay` warned. All orders `cancelled`; batch `cancelled`, `paid_zat` 7,400,000 of 11,100,000. #3 of each order never paid. |
| `promissory payroll status 2` | 17:03 | `2/3 paid` for each payee; `paid 0.07400000 of 0.11100000 ZEC` |

`promissory payroll import` with an invalid file prints every bad row (`row 5: …`, `row 6: …`) and exits 1 before it opens the wallet.

Rates: `GET /api/v1/rates?vs=ngn` returned `{zec_ngn: 1654743, source: "coingecko"}`. It was fetched by `promissory-app` and cached for 5 minutes. Screenshots: `apps/web/screenshots/payroll-*.png` and `remit-*.png`.

## Inheritance switch: regtest (2026-10-26)

A separate `promissory-app` built from this tree (`target-switch`), on 127.0.0.1:8778, with its **own fresh wallet** `promissory-data-regtest-switch` (`promissory init`, not a copy). It was funded through the main app: order #11 on 8738 paid 0.5 ZEC to its treasury. Same chain, same keeper (8737). Heir: the payee wallet `uregtest13543…kuwwxj`. Times are WAT (UTC+1).

| Step | Time / height | Result |
|---|---|---|
| `POST /api/v1/switches` `{heir, amount_zat: 1000000, at: now + 720}` | 17:47:01 | Switch #1, `kind: "inheritance"`, funding 0.0102 ZEC, due 17:59:00 (round 33410192). `GET /notices` at once showed `switch_due:1` ("Renew your switch or it pays 0.01 ZEC to uregtest13…kuwwxj on 2026-10-26 16:59 UTC"): the 7-day reminder window covers the whole 12 minutes. |
| Split from the treasury (progression pass) | 17:47:05, mined **1261** | `87b4eb96…8c86` |
| Armed | 17:49:38 | Copy `8c617587…112d`, blob `1971cd69…3d68`, **`expiry_height: 0`** |
| `POST /api/v1/switches/1/renew` `{at: now + 1080, margin_secs: 120, keep_blob: true}` | 17:50:12 (old copy due in 528 s) | `{old_id: 1, new_id: 2, txid: "0fb2d210…47d3", withdrew_blob: false}`. One transaction, mined at **1273**: 2 Ironwood actions, the old reservation and one treasury note in, the new reservation and change out, fee 0.0001 ZEC, ordinary expiry 1313 (height + 40). Switch #1 `cancelled` with `renewed_to: 2`. Switch #2 `splitting`, due 18:08:12. |
| Switch #2 armed | 17:52:56 | Copy `cdb648a4…e76d`, `expiry_height: 0` |
| Old copy released by the keeper at its round | 17:59:00 | Keeper state **`cancelled`**: `rejected (-25): … ironwood double-spend: duplicate nullifier: Nullifier(0x1dcfa0f1…1a12)`. `getrawtransaction 8c617587…` on Zebra: "Transaction not found in mempool or best chain". The old switch can no longer pay. |
| Switch #2 fires | 18:08:12, mined **1341** | Keeper `broadcast` → `mined`. Order `completed`; the heir was paid 0.01 ZEC, memo "Inheritance test (1/1)". |
| `getrawtransaction cdb648a4…e76d 1` (Zebra RPC 127.0.0.1:18232) | — | `"height": 1341, "expiryheight": 0, "version": 6`, 2 Ironwood actions |
| Receipt for it: `GET /orders/2/installments/0/receipt` → `POST /receipts/verify` | 18:09 | Valid: 0.01 ZEC to `uregtest13543…kuwwxj`, Ironwood action 1, height 1341. Without `X-Promissory-App`, verify is refused (403). `GET /orders/1/installments/0/receipt` → 400 "payment 1 of order 1 has not been paid (it is cancelled)". |

The renewal tx (`0fb2d210…`) carries the normal expiry. Only the pre-signed switch copies have expiry 0. Screenshots: `apps/web/screenshots/switch-*.png` and `receipt-*.png`.

## Receipts: regtest (2026-10-26)

Receipts made from the main wallet (`promissory-data-regtest`) **read-only** while its `promissory-app` kept running: `promissory order receipt` opens `so.db` with `SQLITE_OPEN_READ_ONLY` and the wallet without migrating or syncing it. `so.db` had the same MD5 before and after. Every receipt was checked with `promissory verify-receipt`, which uses no wallet. Payee in all four is `uregtest13543…kuwwxj`.

| Order, payment | Mined copy | Action | Height | `verify-receipt` |
|---|---|---|---|---|
| #2, 1 of 4 | `2f2b3e2b…68b808` (fallback) | 0 | 224 | valid: 0.01 ZEC, memo "SO gate (1/4)" |
| #3, 2 of 3 | `45d3812f…ca1a7` (fallback) | 0 | 692 | valid: 0.01 ZEC, memo "Link test (2/3)" |
| #7, 4 of 4 | `34e0a110…cec1511` (fallback) | 0 | 1007 | valid: 0.01 ZEC, memo "Rolling (4/4)" |
| #8, 1 of 2 | `299cc65a…5951a4` (**fresh copy**) | **1** | 1081 | valid: 0.01 ZEC, memo "Fresh gate 2 (1/2)" |

For order 8 the receipt found the fresh copy, the one actually mined, and its payee output at action 1: the builder shuffles actions, so the index is searched, not assumed.

Tampering with order 7's receipt (re-encoded with one field changed):

| Changed | Result |
|---|---|
| `payee` → the main treasury's address | `the payment went to a different address than the receipt claims` |
| `value_zat` 1000000 → 1000001 | `the payment was 1000000 zat, not the 1000001 zat the receipt claims` |
| one hex digit of `ock` | `the receipt's key does not open action 0 of 34e0a110…` |
| `action_index` 0 → 1 | `the receipt's key does not open action 1 of 34e0a110…` |

The last row shows that a receipt's key opens its own output and nothing else, even in the same transaction. The unit test `receipt::tests::opens_only_its_own_output` checks the same thing on a two-payment bundle: one key per output, and neither opens the other.

## Agent allowances + MCP: regtest (2026-10-26)

A separate payer promissory-app on 127.0.0.1:8788 with a fresh wallet (`promissory-data-regtest-allow`, funded with 0.5 ZEC from the main app: order 12, mined at 1250). The agent's own wallet is `promissory-data-regtest-agent`. Design: [`allowances.md`](allowances.md).

**Allowance 1 ("Research agent"):** cap 0.03 ZEC per 600 s, 3 chips of 0.01, 2 periods.
- **Shape:** one order to the agent, 6 installments 200 s apart, `window` 3 (one period funded at a time).
- **Funding:** the first request was 0.03045 ZEC; top-ups were split from the treasury automatically as chips paid (16:52, 16:55, 16:59).

| Step | Result |
|---|---|
| MCP over stdio: `initialize`, `tools/list` | 5 tools; server instructions state that the agent can never move funds or approve its own requests |
| MCP `allowance_status {id:1}` | cap 0.03 ZEC, the agent's receiving address, 0.03 remaining, next release in 281 s |
| MCP `create_oneoff` 0.015 ZEC ("Invoice 7: GPU hours") | `request_id 1`, **pending**, `approve_at …/requests/1`; no order existed |
| Payer approves request 1 | order 2 created; a second approve returns 409. The one-off **paid at 1300** |
| MCP `request_allowance_change` | request 2 created and rejected by the payer; a cap that isn't a multiple of 3 chips is refused (400) |
| Chips #1–#4 | **paid at 1279, 1292, 1305, 1317** |
| Payer stops the allowance in period 2 (`margin_secs` 60) | one sweep `c746ce44…baeb` takes back chips #5 and #6; `may_still_pay` empty; allowance `stopped` |
| Agent wallet | 0.04 ZEC: chips #1–#4 |

## Incident: duplicate blob after a failed store write (2026-10-26)

During the allowance run the disk filled up, and the test promissory-app logged `unable to open database file` at 16:49.
- `arm` had already uploaded chip #1's blob when recording it failed. The next pass signed chip #1 again (same note, same payee) and uploaded a second blob.
- The keeper held two blobs for round 33410050: the unrecorded one was **mined at 1279**, and the recorded one then failed as a double-spend. The app therefore showed chip #1 as `cancelled` although it had paid.
- No funds were at risk: both copies spend one note to the same payee.
- **Fixed:** if recording an upload fails, `arm` and `freshen` now withdraw that blob from the keeper before returning the error (`order::withdraw_unrecorded`), so a retry can never leave an untracked copy behind.

## Web UI end to end: regtest (2026-10-26)

After the Daybreak redesign, every flow was driven through the web UI in a real browser (Playwright on Chromium), against the main promissory-app on 127.0.0.1:8738 and the keeper on 127.0.0.1:8737. Each form was filled and each button clicked as a person would. Payees were separate wallets (`promissory-data-regtest-payee`, `-agent`).

| Flow | What was done in the UI | Result |
|---|---|---|
| Standing order | New standing order, 0.001 ZEC × 3 every 60 s (order 13) | Funded from the treasury, armed; the timeline showed Locked → Unlocking → Broadcast → Paid for each. **Paid at 2121, 2124, 2128**; `completed` |
| External funding | Order of 0.6 ZEC × 2, more than the treasury held (order 22). Its funding request was paid from a second wallet (`promissory-data-regtest-payroll`, sweep `e7067e9a…850c`) | The page left "Awaiting funds" 174 s later without a reload, and the funding card went away. Active after 349 s; **paid at 2151, 2158** |
| Subscribe link | Built a link on Subscribe link, opened it as the payer ("Preview what payers see"), "Set up this standing order" (order 15) | **Paid at 2126** and after; `completed` |
| Payroll | Pasted a 3-row CSV with one bad address: the row was flagged and the create button stayed disabled. With 2 valid rows: batch 1 (orders 16, 17) | **Paid at 2136 and 2147**; batch `completed` |
| Send home | ₦500 priced from the live rate (0.00030443 ZEC), 1 payment (order 21) | Order page in naira with the rate source; **paid at 2147** |
| Agent request | The agent called `create_oneoff` twice over MCP. In the UI the payer approved request 1 and rejected request 2 | Request 1 → order 19, **paid at 2140**; request 2 `rejected` |
| Allowance | Created in the UI: 0.003 ZEC per 10 min, 3 chips (allowance 1, order 20) | Chips **paid at 2152, 2158, 2165**; `completed` |
| Proof of reservation | "Share proof…" on order 13, pasted on Payee status | "3 of 3 funded, 0 paid", each reserved note and keeper blob listed, in 3 s |
| Receipts | "Get receipt" on a paid payment, then "receipt page" | `Valid: this payment is on chain`. The same receipt with `value_zat` × 10: `Not valid. the payment was 1000000 zat, not the 10000000 zat the receipt claims` |
| Switch | Created a switch 25 min out, then "I'm still here: renew for 30 days" | Switch 18 replaced by 23 in one transaction; 18 `cancelled` with `renewed_to` 23 |
| Cancel | Order 24, 3 payments 15 min apart, first one 5.5 min out. Cancel in the dialog, **browser closed 150 ms after confirming** | The server still finished: sweep `a8f9d1ac…b537`, #2 and #3 `cancelled` and deleted at the keeper, #1 kept as inside the race window and **paid at 2174** |
| Batch cancel | Batch 2 (orders 26, 27), first payments 30 min out, "Cancel batch…" | Both `cancelled`: sweeps `6044c9bd…`, `3100ecbf…`; uploaded blobs deleted |
| Allowance stop | Allowance 2 (order 28), "Stop allowance" → "Stop and sweep" | `stopped`; sweep `447492c8…` |

No page errors in any run.

### Found and fixed

- **A cancel could be lost if the page closed.** On the first cancel attempt (order 14) the script closed the browser while the dialog said "Sending sweep…". The request was waiting for the engine lock; axum drops a handler's future when its connection closes, so the cancel never ran and nothing was logged. Had it been dropped after broadcasting the sweep but before recording it, the order would have been left inconsistent. **Fixed:** every non-GET request now runs in its own task (`detach_writes` in promissory-app), so it finishes whether or not the client is still there. Test: `writes_survive_a_dropped_client`, which fails without the fix. Order 24 above is the live check.
- **Payments paid out of order on short cadences.** With payments 3 minutes apart, order 14's #3 was mined (2125) before #2 (2132): #3's fresh copy releases 5 minutes before its due time, which is earlier than #2's due time. The same happened on orders 16, 17 and 20. **Fixed:** a fresh copy now leads by at most half the cadence (`order::fresh_lead_rounds`), so on a 2-minute schedule it leads by 60 s. The 5-minute lead is unchanged for cadences of 10 minutes or more. Test: `fresh_copy_never_overtakes_the_payment_before_it`. Live check, order 29 (3 payments every 2 min, first 20 min out): all three paid through fresh copies with a 60 s lead, **mined in order at 2389, 2396, 2404**.
- **The cancel result claimed success when nothing was stopped.** On order 14 every remaining payment was inside the race window, so nothing was swept and all of them paid, but the dialog said "Cancel sent". It now says "Too late to stop these payments" when there was no sweep and nothing was cancelled.
- Visual: the QR frame on funding cards stretched to the card's height.

## Kill gate: mainnet

Not run yet. It needs a funded wallet; see `promissory --network main address`.
