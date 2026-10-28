import { describe, expect, it } from "vitest";
import type { Installment, OrderDetail } from "@promissory/client";
import { cancelOutcome, canCancel, hasFreshCopy, installmentPhase, isUnlocked, keeperState, orderSteps, releaseAt } from "./phase";

const inst = (over: Partial<Installment> = {}): Installment => ({
  k: 0,
  due_unix: 1_000,
  round: 1,
  release_unix: 1_001,
  state: "armed",
  keeper_state: "locked",
  txids: ["aa"],
  mined_height: null,
  blob_id: "b",
  cancellable: true,
  ...over,
});

describe("installmentPhase", () => {
  it("follows the keeper lifecycle", () => {
    expect(installmentPhase(inst(), "active", 900).kind).toBe("locked");
    expect(installmentPhase(inst(), "active", 1_001).kind).toBe("releasing");
    expect(installmentPhase(inst({ keeper_state: "released" }), "active", 1_005).kind).toBe("released");
    expect(installmentPhase(inst({ keeper_state: "broadcast" }), "active", 1_010).kind).toBe("broadcast");
    expect(installmentPhase(inst({ state: "paid", keeper_state: "mined", mined_height: 224 }), "active", 1_100)).toEqual({
      kind: "paid",
      height: 224,
    });
    expect(installmentPhase(inst({ state: "reserved", keeper_state: null, blob_id: null }), "splitting", 0).kind).toBe("reserved");
  });

  it("marks what cancel could not stop", () => {
    expect(installmentPhase(inst({ cancellable: false }), "cancelling", 990)).toEqual({ kind: "may_still_pay", why: "race_window" });
    expect(installmentPhase(inst({ keeper_state: "released" }), "cancelled", 1_002)).toEqual({ kind: "may_still_pay", why: "released" });
    expect(installmentPhase(inst({ state: "cancelled", keeper_state: "deleted" }), "cancelled", 0).kind).toBe("cancelled");
    expect(installmentPhase(inst({ keeper_state: "cancelled" }), "cancelled", 2_000).kind).toBe("cancelled");
    expect(installmentPhase(inst({ keeper_state: "deleted" }), "active", 0).kind).toBe("withdrawn");
  });

  it("opens the padlock once released", () => {
    expect(isUnlocked({ kind: "locked" })).toBe(false);
    expect(isUnlocked({ kind: "releasing" })).toBe(true);
    expect(isUnlocked({ kind: "paid", height: 1 })).toBe(true);
    expect(isUnlocked({ kind: "may_still_pay", why: "race_window" })).toBe(false);
  });
});

describe("fresh copies", () => {
  const fresh = (over: Partial<Installment> = {}) =>
    inst({ fresh_txid: "ff", fresh_keeper_state: "locked", fresh_release_unix: 701, ...over });

  it("counts down to the earlier copy", () => {
    expect(releaseAt(fresh())).toBe(701);
    expect(releaseAt(inst())).toBe(1_001);
    expect(installmentPhase(fresh(), "active", 800).kind).toBe("releasing");
  });

  it("follows whichever copy is further along", () => {
    expect(keeperState(fresh({ fresh_keeper_state: "broadcast" }))).toBe("broadcast");
    // The fallback's double-spend rejection does not hide a mined fresh copy.
    expect(keeperState(fresh({ keeper_state: "cancelled", fresh_keeper_state: "mined" }))).toBe("mined");
    expect(installmentPhase(fresh({ keeper_state: "cancelled", fresh_keeper_state: "mined" }), "active", 900).kind).toBe("paid");
    // An expired fresh copy leaves the fallback in charge.
    expect(keeperState(fresh({ fresh_keeper_state: "expired" }))).toBe("locked");
  });

  it("flags a pending fresh copy only", () => {
    expect(hasFreshCopy(fresh())).toBe(true);
    expect(hasFreshCopy(fresh({ state: "paid" }))).toBe(false);
    expect(hasFreshCopy(inst())).toBe(false);
  });
});

describe("cancelOutcome", () => {
  it("classifies each installment", () => {
    expect(cancelOutcome(inst())).toBe("will_cancel");
    expect(cancelOutcome(inst({ cancellable: false }))).toBe("race_window");
    expect(cancelOutcome(inst({ keeper_state: "broadcast", cancellable: false }))).toBe("released");
    expect(cancelOutcome(inst({ state: "paid", keeper_state: "mined" }))).toBe("final");
    expect(cancelOutcome(inst({ state: "reserved", keeper_state: null }))).toBe("will_cancel");
  });
});

describe("orderSteps", () => {
  const order = (over: Partial<OrderDetail>): OrderDetail => ({
    id: 1,
    status: "awaiting_funds",
    payee: "u1",
    amount_zat: 1,
    memo: "",
    every_secs: 60,
    count: 1,
    first_due: 0,
    keeper: "http://k",
    split_txid: null,
    created_at: 0,
    sweep_txid: null,
    cancelled_at: null,
    funding: { zat: 1, address: "u1", uri: "zcash:u1" },
    paid_zat: 0,
    installments: [inst({ state: "reserved", keeper_state: null, blob_id: null })],
    ...over,
  });

  it("tracks funded, split, armed", () => {
    expect(orderSteps(order({}))).toEqual({ funded: "current", split: "todo", armed: "todo" });
    expect(orderSteps(order({ status: "splitting", split_txid: "e9" }))).toEqual({ funded: "done", split: "current", armed: "todo" });
    expect(orderSteps(order({ status: "active", split_txid: "e9", installments: [inst()] }))).toEqual({
      funded: "done",
      split: "done",
      armed: "done",
    });
    expect(orderSteps(order({ status: "cancelled" }))).toEqual({ funded: "done", split: "todo", armed: "todo" });
  });

  it("allows cancel only while the order is live", () => {
    expect(canCancel(order({ status: "active" }))).toBe(true);
    expect(canCancel(order({ status: "awaiting_funds" }))).toBe(true);
    expect(canCancel(order({ status: "cancelling" }))).toBe(false);
    expect(canCancel(order({ status: "completed" }))).toBe(false);
  });
});
