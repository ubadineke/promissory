# @promissory/client

TypeScript client for [Promissory](../../docs/api.md): pre-signed, timelocked, cancellable recurring payments on shielded Zcash.

- **Subscribe links**: build and parse `…/s#v=1&net=…` links, with strict validation.
- **Unified address check**: a full ZIP 316 decode (Bech32m, F4Jumble, padding, receivers) that requires an Orchard receiver.
- **Keeper client**: upload, status, withdraw and health for `promissory-keeper`.
- **Payer client**: every endpoint of the local `promissory-app` API.
- **Formatting**: zatoshi ↔ ZEC without floating point, plus duration and cadence labels.

ESM with type declarations. No runtime dependencies: it uses `fetch`, `TextEncoder` and `URL`, so it runs in browsers, Node 18+, Deno and Bun.

## Merchants: offer a subscription

```ts
import { encodeSubscribeLink, EVERY_PRESETS, parseZec } from "@promissory/client";

const link = encodeSubscribeLink("https://pay.example.com/s", {
  network: "main",
  payee: "u1…", // must contain an Orchard receiver
  amountZat: parseZec("0.01"),
  everySecs: EVERY_PRESETS.week,
  count: 12,
  memo: "Newsletter",
});
// https://pay.example.com/s#v=1&net=main&to=u1…&amt=1000000&every=604800&n=12&memo=Newsletter
```

Parameters live in the fragment, so the server hosting `/s` never sees them. Invalid input throws `LinkError` naming the parameter (`err.param === "amt"`).

## Wallets: open a link

```ts
import { formatCadence, formatZec, LinkError, parseSubscribeLink, subscribeTotalZat } from "@promissory/client";

try {
  const p = parseSubscribeLink(location.href);
  console.log(
    `Pay ${formatZec(p.amountZat, { unit: true })} ${formatCadence(p.everySecs)}, ${p.count} times; ` +
      `total ${formatZec(subscribeTotalZat(p), { unit: true })} + fees`,
  );
} catch (e) {
  if (e instanceof LinkError) showError(e.param, e.message);
}
```

Unknown parameters are ignored, duplicates are rejected. For forms that show every problem at once, use `subscribeParamErrors(params)`.

## Check a payee address

```ts
import { checkPayeeAddress, decodeUnifiedAddress, isValidPayeeAddress } from "@promissory/client";

isValidPayeeAddress("u1…", "main");     // true / false
checkPayeeAddress("u1…", "main");       // throws AddressError("address has no Orchard receiver"), etc.
decodeUnifiedAddress("u1…").receivers;  // [{ typecode: 3, data: Uint8Array(43) }, …]
```

Revision 0 unified addresses (`u`, `utest`, `uregtest`) are decoded in full and checked against the ZIP 316 rules. `promissory-app` remains the authoritative check, since it is what builds the transactions.

## Drive the payer app

```ts
import { PayerClient } from "@promissory/client";

const app = new PayerClient(); // http://127.0.0.1:8738; pass "" from a page promissory-app serves
const order = await app.createOrderFromLink({ link });
showQr(order.funding!.uri); // ZIP-321; pay once, then the app splits and arms by itself

const detail = await app.getOrder(order.id);
const result = await app.cancelOrder(order.id);
// { sweep_txid, cancelled: [3, 4], may_still_pay: [{ k: 2, reason: "…" }] }
```

Every non-GET request carries `X-Promissory-App: 1`. Failures throw `ApiError` (`status`, server `message`) or `NetworkError` when `promissory-app` is unreachable.

## Talk to a keeper

```ts
import { KeeperClient } from "@promissory/client";

const keeper = new KeeperClient("https://keeper.example");
const { blob_id, delete_token, round } = await keeper.upload(tlockCiphertextBytes);
await keeper.status(blob_id);           // { state: "locked" | "released" | "broadcast" | "mined" | … }
await keeper.delete(blob_id, delete_token); // 409 once released
await keeper.health();
```

## Formatting

```ts
formatZec(4_060_000);              // "0.0406"
formatZec(100_000_000, { unit: true }); // "1.00 ZEC"
parseZec("0.3");                   // 30_000_000 (exact)
formatDuration(180);               // "3 min"
formatCadence(604_800);            // "every week"
formatCountdown(3_725);            // "1:02:05"
truncateMiddle("u1abcdef…", 5, 4); // "u1abc…wxyz"
```

## Development

```sh
pnpm build   # tsc → dist/
pnpm test    # typecheck + vitest
```

API reference: [docs/promissory-js.md](../../docs/promissory-js.md).
