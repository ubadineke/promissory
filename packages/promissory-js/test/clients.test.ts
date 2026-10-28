import { describe, expect, it, vi } from "vitest";
import { ApiError, NetworkError, type FetchLike } from "../src/http.js";
import { KeeperClient } from "../src/keeper.js";
import { PayerClient, type Order } from "../src/payer.js";

interface Call {
  url: string;
  method: string;
  headers: Record<string, string>;
  body: unknown;
}

function mockFetch(respond: (call: Call) => Response): { fetch: FetchLike; calls: Call[] } {
  const calls: Call[] = [];
  const fetch: FetchLike = async (url, init = {}) => {
    const call: Call = {
      url,
      method: init.method ?? "GET",
      headers: Object.fromEntries(new Headers(init.headers).entries()),
      body: typeof init.body === "string" ? JSON.parse(init.body) : undefined,
    };
    calls.push(call);
    return respond(call);
  };
  return { fetch, calls };
}

const json = (data: unknown, status = 200) =>
  new Response(JSON.stringify(data), { status, headers: { "Content-Type": "application/json" } });

const order: Order = {
  id: 2,
  status: "awaiting_funds",
  payee: "u1payee",
  amount_zat: 1_000_000,
  memo: "SO gate",
  every_secs: 180,
  count: 4,
  first_due: 1_792_967_720,
  keeper: "http://127.0.0.1:8737",
  split_txid: null,
  created_at: 1_792_967_120,
  sweep_txid: null,
  cancelled_at: null,
  funding: { zat: 4_060_000, address: "u1treasury", uri: "zcash:u1treasury?amount=0.0406" },
  paid_zat: 0,
};

describe("KeeperClient", () => {
  it("uploads raw bytes as base64", async () => {
    const m = mockFetch(() => json({ blob_id: "c43b", delete_token: "tok", round: 33388117 }, 201));
    const k = new KeeperClient("http://keeper.example/", { fetch: m.fetch });
    const up = await k.upload(Uint8Array.from([0, 1, 2, 250, 255]), { webhook: "https://h.example" });
    expect(up).toEqual({ blob_id: "c43b", delete_token: "tok", round: 33388117 });
    expect(m.calls[0]).toMatchObject({
      url: "http://keeper.example/v1/blobs",
      method: "POST",
      body: { ciphertext: "AAEC+v8=", webhook: "https://h.example" },
    });
    expect(m.calls[0]!.headers["content-type"]).toBe("application/json");
  });

  it("passes base64 strings through and omits an absent webhook", async () => {
    const m = mockFetch(() => json({ blob_id: "a", delete_token: "b", round: 1 }, 201));
    await new KeeperClient("http://k", { fetch: m.fetch }).upload("YWdl");
    expect(m.calls[0]!.body).toEqual({ ciphertext: "YWdl" });
  });

  it("encodes a large payload without overflowing the call stack", async () => {
    const m = mockFetch(() => json({ blob_id: "a", delete_token: "b", round: 1 }, 201));
    await new KeeperClient("http://k", { fetch: m.fetch }).upload(new Uint8Array(60_000));
    expect((m.calls[0]!.body as { ciphertext: string }).ciphertext).toHaveLength(80_000);
  });

  it("gets status and health", async () => {
    const status = { blob_id: "c 1", round: 5, state: "mined", txid: "2f2b", mined_height: 224, updated_at: 9 };
    const m = mockFetch((c) => (c.url.endsWith("/health") ? json({ ok: true, network: "regtest" }) : json(status)));
    const k = new KeeperClient("http://k", { fetch: m.fetch });
    expect(await k.status("c 1")).toEqual(status);
    expect(m.calls[0]!.url).toBe("http://k/v1/blobs/c%201");
    expect((await k.health()).network).toBe("regtest");
  });

  it("deletes with a bearer token", async () => {
    const m = mockFetch(() => new Response(null, { status: 204 }));
    await expect(new KeeperClient("http://k", { fetch: m.fetch }).delete("c43b", "secret")).resolves.toBeUndefined();
    expect(m.calls[0]).toMatchObject({ method: "DELETE", url: "http://k/v1/blobs/c43b" });
    expect(m.calls[0]!.headers.authorization).toBe("Bearer secret");
  });

  it("surfaces keeper errors as ApiError", async () => {
    const m = mockFetch(() => json({ error: "blob is broadcast; it can no longer be deleted" }, 409));
    const err = await new KeeperClient("http://k", { fetch: m.fetch }).delete("x", "t").catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ status: 409, message: "blob is broadcast; it can no longer be deleted" });
  });
});

describe("PayerClient", () => {
  const routes = (c: Call): Response => {
    const path = new URL(c.url, "http://x").pathname;
    if (path === "/api/v1/info" || path === "/api/v1/sync") return json({ network: "regtest", tip_height: 10 });
    if (path === "/api/v1/orders" && c.method === "GET") return json([order]);
    if (path.startsWith("/api/v1/orders") && path.endsWith("/cancel")) {
      return json({ sweep_txid: "df8c", cancelled: [3], may_still_pay: [{ k: 2, reason: "inside the race window" }] });
    }
    if (path === "/api/v1/orders/2") return json({ ...order, installments: [] });
    return json(order, 201);
  };

  it("uses relative paths with an empty base", async () => {
    const m = mockFetch(routes);
    const p = new PayerClient("", { fetch: m.fetch });
    await p.info();
    expect(m.calls[0]!.url).toBe("/api/v1/info");
  });

  it("defaults to the local promissory-app address", async () => {
    const m = mockFetch(routes);
    await new PayerClient(undefined, { fetch: m.fetch }).listOrders();
    expect(m.calls[0]!.url).toBe("http://127.0.0.1:8738/api/v1/orders");
  });

  it("sends X-Promissory-App on every non-GET request, and only there", async () => {
    const m = mockFetch(routes);
    const p = new PayerClient("", { fetch: m.fetch });
    await p.info();
    await p.listOrders();
    await p.getOrder(2);
    await p.sync();
    await p.createOrder({ payee: "u1payee", amount_zat: 1_000_000, every_secs: 180, count: 4, memo: "m" });
    await p.createOrderFromLink({ link: "https://x/s#v=1", keeper: "http://k" });
    await p.cancelOrder(2, { margin_secs: 60, keep_blobs: true });
    await p.cancelOrder(2);
    for (const c of m.calls) {
      expect(c.headers["x-promissory-app"], `${c.method} ${c.url}`).toBe(c.method === "GET" ? undefined : "1");
    }
    expect(m.calls.map((c) => `${c.method} ${c.url}`)).toEqual([
      "GET /api/v1/info",
      "GET /api/v1/orders",
      "GET /api/v1/orders/2",
      "POST /api/v1/sync",
      "POST /api/v1/orders",
      "POST /api/v1/orders/from-link",
      "POST /api/v1/orders/2/cancel",
      "POST /api/v1/orders/2/cancel",
    ]);
    expect(m.calls[4]!.body).toEqual({ payee: "u1payee", amount_zat: 1_000_000, every_secs: 180, count: 4, memo: "m" });
    expect(m.calls[5]!.body).toEqual({ link: "https://x/s#v=1", keeper: "http://k" });
    expect(m.calls[6]!.body).toEqual({ margin_secs: 60, keep_blobs: true });
    expect(m.calls[7]!.body).toEqual({});
  });

  it("returns typed bodies", async () => {
    const p = new PayerClient("", { fetch: mockFetch(routes).fetch });
    expect((await p.listOrders())[0]!.funding?.zat).toBe(4_060_000);
    expect((await p.getOrder(2)).installments).toEqual([]);
    const r = await p.cancelOrder(2);
    expect(r.may_still_pay[0]!.reason).toBe("inside the race window");
  });

  it("raises ApiError with the server message", async () => {
    const m = mockFetch(() => json({ error: "payee has no Orchard receiver" }, 400));
    const err = await new PayerClient("", { fetch: m.fetch }).createOrder(
      { payee: "x", amount_zat: 1, every_secs: 60, count: 1 },
    ).catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ status: 400, message: "payee has no Orchard receiver" });
  });

  it("raises ApiError for non-JSON error bodies", async () => {
    const m = mockFetch(() => new Response("Bad Gateway", { status: 502, statusText: "Bad Gateway" }));
    const err = await new PayerClient("", { fetch: m.fetch }).info().catch((e) => e);
    expect(err).toMatchObject({ name: "ApiError", status: 502, message: "502 Bad Gateway", body: "Bad Gateway" });
  });

  it("raises NetworkError when promissory-app is unreachable", async () => {
    const fetch = vi.fn<FetchLike>().mockRejectedValue(new TypeError("fetch failed"));
    const err = await new PayerClient("", { fetch }).info().catch((e) => e);
    expect(err).toBeInstanceOf(NetworkError);
    expect(err.message).toBe("GET /api/v1/info: fetch failed");
  });
});
