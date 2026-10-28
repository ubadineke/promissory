# promissory-js API reference

`@promissory/client` (`packages/promissory-js`). Contracts it implements: `api.md`. Usage examples: `packages/promissory-js/README.md`.

Amounts are integer zatoshis (`number`; the 21M ZEC cap is below 2^53). Times are unix seconds.

## Subscribe links (`link.ts`)

| Export | Description |
|---|---|
| `encodeSubscribeLink(base, params)` | Builds `base#v=1&net&to&amt&every&n[&memo][&start][&keeper][&hook]`. Replaces any fragment on `base`. Throws `LinkError`. |
| `parseSubscribeLink(urlOrFragment)` | Accepts a URL, `#…` or the bare fragment. Ignores unknown parameters; rejects duplicates and bad percent-encoding. Throws `LinkError`. |
| `validateSubscribeParams(p)` | Throws a `LinkError` for the first problem, in link order. |
| `subscribeParamErrors(p)` | Every problem, keyed by link parameter. `{}` when valid. |
| `check.<field>(value)` | Single-field checks returning a message or `null`. |
| `subscribeTotalZat(p)` | `amountZat × count`, before fees. |
| `LinkError` | `.param`: `"v" \| "net" \| "to" \| "amt" \| "every" \| "n" \| "memo" \| "start" \| "keeper" \| "hook"`. |
| Constants | `LINK_VERSION` 1, `MEMO_MAX_BYTES` 400, `EVERY_MIN_SECS` 60, `COUNT_MIN` 1, `COUNT_MAX` 120, `EVERY_PRESETS` `{day, week, days30}`. |

`SubscribeParams`: `{ network, payee, amountZat, everySecs, count, memo?, start?, keeper?, hook? }`, mapping to `net, to, amt, every, n, memo, start, keeper, hook`.

Rules: `v` = 1; `net` ∈ {main, regtest}; `to` decodes as a unified address for `net` with an Orchard receiver; `amt` integer > 0; `every` integer ≥ 60; `n` integer 1–120; `memo` ≤ 400 UTF-8 bytes (empty means none); `start` integer > 0; `keeper`, `hook` http(s) URLs. Integers must be plain digits (no sign, decimals or exponent).

## Addresses (`address.ts`)

| Export | Description |
|---|---|
| `decodeUnifiedAddress(addr)` | `{ network: "main" \| "test" \| "regtest", receivers: [{typecode, data}] }`. Bech32m (no length limit), F4Jumble⁻¹, HRP padding, TLV items. Rejects bad checksums, mixed case, unknown prefixes, wrong receiver lengths, duplicate typecodes, P2PKH+P2SH, transparent-only. Throws `AddressError`. |
| `checkPayeeAddress(addr, network)` | Also requires the network to match and an Orchard receiver (typecode 3). |
| `isValidPayeeAddress(addr, network)` | Boolean form. |

Revision 2 addresses (`zu…`, `tu…`) are not accepted.

## Formatting (`format.ts`)

| Export | Example |
|---|---|
| `formatZec(zat, {minDecimals=2, grouping=true, unit=false})` | `4_060_000` → `"0.0406"`; `2.1e15` → `"21,000,000.00"` |
| `parseZec(text)` | `"0.01"` → `1_000_000`; at most 8 decimals; optional trailing `ZEC`. Throws `FormatError`. |
| `formatDuration(secs)` | `180` → `"3 min"`, `604800` → `"1 week"`, `2592000` → `"30 days"` |
| `formatCadence(secs)` | `604800` → `"every week"`, `60` → `"every minute"`, `0` → `"once"` (a switch) |
| `formatCountdown(secs)` | `65` → `"01:05"`, `90061` → `"1d 01:01:01"` |
| `truncateMiddle(s, head=8, tail=6)` | `"u1abcdef…uvwxyz"` |

## HTTP (`http.ts`)

Both clients take `(baseUrl, { fetch?, headers? })`. Errors: `ApiError` (`status`, `message` from `{"error"}`, raw `body`) and `NetworkError` (no HTTP response).

## Keeper (`keeper.ts`)

`new KeeperClient(baseUrl)`:

| Method | Request |
|---|---|
| `upload(ciphertext: Uint8Array \| base64, {webhook?})` → `UploadedBlob` | `POST /v1/blobs` |
| `status(blobId)` → `BlobStatus` | `GET /v1/blobs/{id}` |
| `delete(blobId, deleteToken)` | `DELETE /v1/blobs/{id}`, `Authorization: Bearer …` |
| `health()` → `KeeperHealth` | `GET /v1/health` |

`isOutOfPayerHands(state)`: true for `released`, `broadcast`, `mined`.

## Payer (`payer.ts`)

`new PayerClient(baseUrl = "http://127.0.0.1:8738")`; pass `""` for relative paths. Non-GET requests send `X-Promissory-App: 1`.

| Method | Request |
|---|---|
| `info()` | `GET /api/v1/info` |
| `sync()` | `POST /api/v1/sync` |
| `listOrders()` | `GET /api/v1/orders` |
| `createOrder(req)` | `POST /api/v1/orders` |
| `createOrderFromLink({link, first_due?, keeper?})` | `POST /api/v1/orders/from-link` |
| `getOrder(id)` → `OrderDetail` | `GET /api/v1/orders/{id}` |
| `cancelOrder(id, {margin_secs?, keep_blobs?, to?})` → `CancelResult` | `POST /api/v1/orders/{id}/cancel` |
| `createBatch({csv, name?, first_due?, keeper?})` → `CreatedBatch` | `POST /api/v1/batches`. A 400 `ApiError` has `body.rows`: `[{line, message}]`. |
| `listBatches()` → `Batch[]` | `GET /api/v1/batches` |
| `getBatch(id)` → `Batch` | `GET /api/v1/batches/{id}` |
| `cancelBatch(id, {margin_secs?, keep_blobs?})` → `BatchCancelResult` | `POST /api/v1/batches/{id}/cancel` |
| `rates(vs = "ngn")` → `Rates` | `GET /api/v1/rates?vs=…`. Display only. |
| `createSwitch({heir, amount_zat, at, memo?, remind_secs?, keeper?})` → `OrderDetail` | `POST /api/v1/switches` (dead-man's switch, `kind: "inheritance"`) |
| `renewSwitch(id, {at, margin_secs?, keep_blob?})` → `RenewResult` | `POST /api/v1/switches/{id}/renew` |
| `getReceipt(orderId, k)` → `MadeReceipt` | `GET /api/v1/orders/{id}/installments/{k}/receipt` |
| `verifyReceipt(receipt)` → `VerifiedReceipt` | `POST /api/v1/receipts/verify` |

Types mirror `api.md` in snake_case: `Info`, `Order`, `OrderDetail`, `Installment`, `Funding`, `CancelResult`, `MayStillPay`, `OrderStatus`, `InstallmentState`. `Order.funding` is `null` once the order is funded, as the live `promissory-app` returns it.

## Proof of reservation and notices (`reservation.ts`, `notices.ts`)

| Export | Description |
|---|---|
| `decodeReservationProof(text)` | Parses `promissory-reserve:v1:<base64url(json)>` locally into a `ReservationProof` (no network). Throws `ReservationError`. Use it to preview terms; only `verify` checks the chain. |
| `encodeReservationProof(proof)` | Inverse, for tests and tools. |
| `ReservationClient(baseUrl, opts)` | `exportProof(orderId)` → string; `verify(proof)` → `ReservationReport`; `keeperStatus(keeper, blobIds)` → `KeeperBlobView[]`. Same base URL as `PayerClient`. |
| `NoticesClient(baseUrl, opts)` | `list()` → `AppNotice[]`; `dismiss(id)`. `NoticeKind` includes `switch_due`. |
| `decodeReceipt(text)` / `encodeReceipt(r)` (`receipt.ts`) | Parse or build `promissory-receipt:v1:<base64url(json)>` locally; throws `ReceiptError`. Only `verifyReceipt` checks the chain. |

Report and notice shapes: `api.md`, "Proof of reservation" and "Notices".

## Payroll (`payroll.ts`)

| Export | Description |
|---|---|
| `parsePayrollCsv(csv, network)` | `{rows, errors, totalZat, fundingZat}` for `name,ua,amount,cadence,count` (rules in `api.md`, "Payroll batches"). `errors`: `[{line, message}]`, line 0 for the whole file. Same checks as promissory-app, for a preview before `createBatch`. |
| `parsePayrollCadence(s)` | `weekly` 604800, `biweekly` 1209600, `monthly` 2592000, or seconds ≥ 60. Throws `FormatError`. |
| `payrollFundingZat(amountZat, count)` | One order's share of the combined funding, including its split's change-output fee. |
| `splitCsvLine(line)` | Fields with double-quote handling. |
| `fiatToZat(fiat, rate)` / `zatToFiat(zat, rate)` | Display conversions at `rate` (fiat per ZEC); `fiatToZat` rounds to whole zatoshis. |

## Agent allowances and requests (`allowance.ts`)

| Export | Description |
|---|---|
| `new AllowanceClient(baseUrl, opts?)` | `create(req)`, `list()`, `get(id)`, `stop(id, {margin_secs?})` for allowances; `createRequest(ask)`, `listRequests()`, `getRequest(id)`, `approveRequest(id)`, `rejectRequest(id)` for agent requests. Writes send `X-Promissory-App: 1`. Types: `Allowance`, `AgentRequest` (`OneoffRequest` \| `AllowanceChangeRequest`), `ApprovedRequest`, `StopAllowanceResult`. Rules in `api.md`, "Agent allowances". |
| `allowanceErrors(capZat, periodSecs, chips, periods)` | Problems by field (`cap_zat`, `period_secs`, `chips`, `periods`), the same checks as promissory-app, for a form. |
| `planAllowance(capZat, periodSecs, chips, periods)` | `{chipZat, everySecs, count, window}`: the order an allowance becomes, or `null` if invalid. |
| `ALLOWANCE_MAX_CHIPS`, `ALLOWANCE_MIN_CHIP_ZAT`, `ALLOWANCE_MIN_SPACING_SECS`, `ALLOWANCE_MAX_TOTAL_CHIPS`, `MAX_PENDING_REQUESTS` | 30, 100,000, 60, 120, 10. |
