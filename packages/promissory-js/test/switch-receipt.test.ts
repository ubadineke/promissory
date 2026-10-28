import { describe, expect, it } from "vitest";
import type { FetchLike } from "../src/http.js";
import { PayerClient } from "../src/payer.js";
import { decodeReceipt, encodeReceipt, RECEIPT_PREFIX, ReceiptError, type Receipt } from "../src/receipt.js";

interface Call {
  url: string;
  method: string;
  headers: Record<string, string>;
  body: unknown;
}

function recorder(reply: unknown): { fetch: FetchLike; calls: Call[] } {
  const calls: Call[] = [];
  const fetch: FetchLike = async (url, init = {}) => {
    calls.push({
      url,
      method: init.method ?? "GET",
      headers: Object.fromEntries(new Headers(init.headers).entries()),
      body: typeof init.body === "string" ? JSON.parse(init.body) : undefined,
    });
    return new Response(JSON.stringify(reply), { status: 200, headers: { "Content-Type": "application/json" } });
  };
  return { fetch, calls };
}

const sample: Receipt = {
  v: 1,
  net: "regtest",
  txid: "ab".repeat(32),
  pool: "ironwood",
  action_index: 1,
  ock: "cd".repeat(32),
  payee: "uregtest1payee",
  value_zat: 1_000_000,
};

describe("receipts", () => {
  it("round-trips without padding", () => {
    const s = encodeReceipt(sample);
    expect(s.startsWith(RECEIPT_PREFIX)).toBe(true);
    expect(s).not.toMatch(/[=+/]/);
    expect(decodeReceipt(`  ${s}\n`)).toEqual(sample);
  });

  it("matches the Rust encoding", () => {
    // serde_json field order is the struct's; JSON.stringify keeps insertion order.
    const json = '{"v":1,"net":"regtest","txid":"' + "ab".repeat(32) + '","pool":"ironwood","action_index":1,"ock":"' + "cd".repeat(32) + '","payee":"uregtest1payee","value_zat":1000000}';
    expect(encodeReceipt(sample)).toBe(RECEIPT_PREFIX + Buffer.from(json).toString("base64url"));
  });

  it("rejects malformed receipts", () => {
    expect(() => decodeReceipt("promissory-reserve:v1:abc")).toThrow(ReceiptError);
    expect(() => decodeReceipt("promissory-receipt:v1:!!!")).toThrow(/base64url/);
    expect(() => decodeReceipt(encodeReceipt({ ...sample, v: 2 as 1 }))).toThrow(/version/);
    expect(() => decodeReceipt(encodeReceipt({ ...sample, pool: "sapling" as "orchard" }))).toThrow(/pool/);
    expect(() => decodeReceipt(encodeReceipt({ ...sample, ock: "zz" }))).toThrow(/key/);
    expect(() => decodeReceipt(encodeReceipt({ ...sample, txid: "AB".repeat(32) }))).toThrow(/txid/);
  });
});

describe("PayerClient switches and receipts", () => {
  it("calls the switch and receipt endpoints", async () => {
    const m = recorder({});
    const p = new PayerClient("", { fetch: m.fetch });
    await p.createSwitch({ heir: "u1heir", amount_zat: 1_000_000, at: 1_800_000_000, remind_secs: 86_400 });
    await p.renewSwitch(4, { at: 1_800_086_400, keep_blob: true });
    await p.getReceipt(7, 2);
    await p.verifyReceipt(`  ${encodeReceipt(sample)} `);
    expect(m.calls.map((c) => `${c.method} ${c.url}`)).toEqual([
      "POST /api/v1/switches",
      "POST /api/v1/switches/4/renew",
      "GET /api/v1/orders/7/installments/2/receipt",
      "POST /api/v1/receipts/verify",
    ]);
    expect(m.calls[0]!.body).toEqual({ heir: "u1heir", amount_zat: 1_000_000, at: 1_800_000_000, remind_secs: 86_400 });
    expect(m.calls[1]!.body).toEqual({ at: 1_800_086_400, keep_blob: true });
    expect(m.calls[3]!.body).toEqual({ receipt: encodeReceipt(sample) });
    for (const c of m.calls) expect(c.headers["x-promissory-app"]).toBe(c.method === "GET" ? undefined : "1");
  });
});
