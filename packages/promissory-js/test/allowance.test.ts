import { describe, expect, it } from "vitest";
import { AllowanceClient, allowanceErrors, planAllowance } from "../src/allowance.js";
import { ApiError, type FetchLike } from "../src/http.js";

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

describe("planAllowance", () => {
  it("maps an allowance onto a rolling-window order", () => {
    expect(planAllowance(3_000_000, 600, 3, 2)).toEqual({ chipZat: 1_000_000, everySecs: 200, count: 6, window: 3 });
    expect(planAllowance(500_000, 86_400, 1, 120)?.count).toBe(120);
  });

  it("names every invalid field", () => {
    expect(allowanceErrors(3_000_000, 600, 3, 2)).toEqual({});
    expect(allowanceErrors(3_000_001, 601, 3, 2)).toEqual({
      cap_zat: "The cap must split into 3 equal chips",
      period_secs: "The period must split into 3 equal gaps",
    });
    expect(allowanceErrors(300_000, 150, 6, 21)).toEqual({
      cap_zat: "Each chip must be at least 0.001 ZEC",
      period_secs: "Chips must be at least 60 s apart",
      periods: "Chips × periods must be at most 120",
    });
    expect(Object.keys(allowanceErrors(0, 0, 0, 0)).sort()).toEqual(["cap_zat", "chips", "period_secs", "periods"]);
    expect(allowanceErrors(3_000_000, 600, 1.5, 2).chips).toBeDefined();
    expect(planAllowance(3_000_000, 600, 31, 1)).toBeNull();
  });
});

describe("AllowanceClient", () => {
  it("creates, lists, gets and stops allowances", async () => {
    const m = mockFetch((c) => json(c.url.endsWith("/allowances") && c.method === "GET" ? [] : { id: 1 }));
    const a = new AllowanceClient("http://127.0.0.1:8738/", { fetch: m.fetch });
    const req = { agent_ua: "uregtest1a", cap_zat: 3_000_000, period_secs: 600, chips: 3, periods: 2, label: "Bot" };
    await a.create(req);
    await a.list();
    await a.get(1);
    await a.stop(1, { margin_secs: 60 });
    expect(m.calls.map((c) => `${c.method} ${c.url}`)).toEqual([
      "POST http://127.0.0.1:8738/api/v1/allowances",
      "GET http://127.0.0.1:8738/api/v1/allowances",
      "GET http://127.0.0.1:8738/api/v1/allowances/1",
      "POST http://127.0.0.1:8738/api/v1/allowances/1/stop",
    ]);
    expect(m.calls[0]!.body).toEqual(req);
    expect(m.calls[3]!.body).toEqual({ margin_secs: 60 });
    for (const c of m.calls.filter((c) => c.method === "POST")) expect(c.headers["x-promissory-app"]).toBe("1");
  });

  it("creates, approves and rejects requests", async () => {
    const m = mockFetch(() => json({ id: 4, status: "pending" }));
    const a = new AllowanceClient("", { fetch: m.fetch });
    await a.createRequest({ kind: "oneoff", payee: "uregtest1p", amount_zat: 5_000_000, memo: "Invoice" });
    await a.listRequests();
    await a.getRequest(4);
    await a.approveRequest(4);
    await a.rejectRequest(5);
    expect(m.calls.map((c) => `${c.method} ${c.url}`)).toEqual([
      "POST /api/v1/requests",
      "GET /api/v1/requests",
      "GET /api/v1/requests/4",
      "POST /api/v1/requests/4/approve",
      "POST /api/v1/requests/5/reject",
    ]);
    expect(m.calls[0]!.body).toEqual({ kind: "oneoff", payee: "uregtest1p", amount_zat: 5_000_000, memo: "Invoice" });
    expect(m.calls[3]!.body).toEqual({});
  });

  it("surfaces the pending cap as an ApiError", async () => {
    const m = mockFetch(() => json({ error: "10 requests are already waiting for the payer" }, 429));
    const a = new AllowanceClient("", { fetch: m.fetch });
    const err = await a
      .createRequest({ kind: "allowance_change", allowance_id: 1, new_cap_zat: 6_000_000 })
      .catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).status).toBe(429);
    expect((err as ApiError).message).toContain("already waiting");
  });
});
