import { describe, expect, it } from "vitest";
import type { Order } from "@promissory/client";
import { liveSwitches, pastSwitches, renewError, renewTarget, switchPhase, tooLateToRenew } from "./switch";

const base: Order = {
  id: 1,
  status: "active",
  payee: "u1heir",
  amount_zat: 1_000_000,
  memo: "",
  every_secs: 0,
  count: 1,
  first_due: 10_000_000,
  keeper: "http://k",
  split_txid: null,
  created_at: 0,
  funding: null,
  sweep_txid: null,
  cancelled_at: null,
  paid_zat: 0,
  kind: "inheritance",
  remind_secs: 7 * 86_400,
};
const o = (p: Partial<Order>): Order => ({ ...base, ...p });

describe("switch", () => {
  it("separates live switches from renewed and finished ones", () => {
    const orders = [
      o({ id: 1, renewed_to: 2, status: "cancelled" }),
      o({ id: 2, renewed_from: 1, first_due: 20_000_000 }),
      o({ id: 3, first_due: 15_000_000, status: "awaiting_funds" }),
      o({ id: 4, status: "completed" }),
      o({ id: 5, kind: "standing" }),
      o({ id: 6, kind: undefined }),
    ];
    expect(liveSwitches(orders).map((x) => x.id)).toEqual([3, 2]);
    expect(pastSwitches(orders).map((x) => x.id)).toEqual([4]);
  });

  it("phases", () => {
    expect(switchPhase(o({ status: "awaiting_funds" }), 0)).toBe("unfunded");
    expect(switchPhase(o({ status: "splitting" }), 0)).toBe("arming");
    expect(switchPhase(base, base.first_due - 8 * 86_400)).toBe("armed");
    expect(switchPhase(base, base.first_due - 6 * 86_400)).toBe("due_soon");
    expect(switchPhase(o({ remind_secs: 60 }), base.first_due - 120)).toBe("armed");
    expect(switchPhase(base, base.first_due)).toBe("firing");
  });

  it("renewal dates never go backwards", () => {
    expect(renewTarget(base, 0, 100)).toBe(base.first_due + 100);
    expect(renewTarget(base, base.first_due + 5, 100)).toBe(base.first_due + 105);
    expect(renewError(base, null, 0)).toMatch(/Choose/);
    expect(renewError(base, base.first_due, 0)).toMatch(/later/);
    expect(renewError(base, base.first_due + 1, base.first_due)).toMatch(/10 minutes/);
    expect(renewError(base, base.first_due + 3600, 0)).toBeNull();
  });
});

describe("tooLateToRenew", () => {
  it("refuses renewal inside the last ten minutes", () => {
    const o = { first_due: 10_000 } as Parameters<typeof tooLateToRenew>[0];
    expect(tooLateToRenew(o, 10_000 - 601)).toBe(false);
    expect(tooLateToRenew(o, 10_000 - 599)).toBe(true);
    expect(renewError(o, 20_000, 10_000 - 300)).toContain("Too late to renew");
  });
});
