import { describe, expect, it } from "vitest";
import { encodeSubscribeLink, PayerClient } from "@promissory/client";
import { createMockFetch } from "./mock";

const REGTEST_UA =
  "uregtest1ynahn0vg27j478s8azkuzthxhfagywduxzgx3ymx5armlml42rtxeeamhlywpj5p4fp469a5zck46tc7rfd79rd0nmps3lhauugm2y9q";

describe("mock promissory-app", () => {
  it("serves every order state", async () => {
    const api = new PayerClient("", { fetch: createMockFetch() });
    const statuses = (await api.listOrders()).map((o) => o.status);
    for (const s of ["awaiting_funds", "splitting", "active", "cancelling", "cancelled", "completed"]) {
      expect(statuses).toContain(s);
    }
    const live = await api.getOrder(2);
    const kinds = new Set(live.installments.map((i) => i.state));
    expect(kinds.has("paid") && kinds.has("armed")).toBe(true);
  });

  it("cancels with per-installment results", async () => {
    const api = new PayerClient("", { fetch: createMockFetch() });
    const r = await api.cancelOrder(2);
    expect(r.sweep_txid).toMatch(/^[0-9a-f]{64}$/);
    expect(r.cancelled.length).toBeGreaterThan(0);
    expect(["cancelling", "cancelled"]).toContain((await api.getOrder(2)).status);
    await expect(api.cancelOrder(2)).rejects.toMatchObject({ status: 409 });
  });

  it("creates orders from links and enforces the CSRF header", async () => {
    const fetch = createMockFetch();
    const api = new PayerClient("", { fetch });
    const link = encodeSubscribeLink("http://x/s", { network: "regtest", payee: REGTEST_UA, amountZat: 1_000_000, everySecs: 604_800, count: 12 });
    const o = await api.createOrderFromLink({ link });
    expect(o).toMatchObject({ status: "awaiting_funds", count: 12, funding: { zat: 12 * 1_010_000 + 60_000 } });
    expect(o.funding?.uri).toMatch(/^zcash:uregtest1.*amount=0\.1218/);
    const res = await fetch("/api/v1/sync", { method: "POST" });
    expect(res.status).toBe(403);
  });
});
