/** What the UI shows for orders and installments, derived from API state. Pure and tested. */

import { isOutOfPayerHands, type BlobState, type Installment, type Order, type OrderStatus } from "@promissory/client";

export type InstallmentPhase =
  /** Note reserved; not yet signed and uploaded. */
  | { kind: "reserved" }
  /** At the keeper, timelocked. */
  | { kind: "locked" }
  /** Its drand round is out but the keeper has not reported yet. */
  | { kind: "releasing" }
  | { kind: "released" }
  | { kind: "broadcast" }
  | { kind: "paid"; height: number | null }
  | { kind: "cancelled" }
  /** The order was cancelled but this one was outside the sweep's reach. */
  | { kind: "may_still_pay"; why: "race_window" | "released" }
  | { kind: "expired" }
  | { kind: "failed" }
  | { kind: "withdrawn" };

const CANCELLING: readonly OrderStatus[] = ["cancelling", "cancelled"];

export function isCancelState(status: OrderStatus): boolean {
  return CANCELLING.includes(status);
}

const PROGRESS: Partial<Record<BlobState, number>> = { mined: 5, broadcast: 4, released: 3, locked: 2 };

/**
 * The keeper state that speaks for an installment. With a fresh copy there are two blobs
 * spending one note; whichever is further along wins (a mined fresh copy makes the fallback's
 * double-spend rejection irrelevant).
 */
export function keeperState(inst: Installment): BlobState | null {
  const fallback = inst.keeper_state;
  const fresh = inst.fresh_keeper_state ?? null;
  if (!fresh) return fallback;
  if (!fallback) return fresh;
  return (PROGRESS[fresh] ?? 1) >= (PROGRESS[fallback] ?? 1) ? fresh : fallback;
}

/** When the installment's earliest copy releases: the fresh copy, if any, goes first. */
export function releaseAt(inst: Installment): number {
  return Math.min(inst.release_unix, inst.fresh_release_unix ?? Number.POSITIVE_INFINITY);
}

/** A fresh copy is waiting at the keeper: the payment will look like a normal send. */
export function hasFreshCopy(inst: Installment): boolean {
  return !!inst.fresh_txid && inst.state !== "paid" && inst.state !== "cancelled";
}

export function installmentPhase(inst: Installment, status: OrderStatus, now: number): InstallmentPhase {
  const ks = keeperState(inst);
  if (inst.state === "paid" || ks === "mined") return { kind: "paid", height: inst.mined_height };
  if (inst.state === "cancelled" || ks === "cancelled") return { kind: "cancelled" };
  if (ks === "expired") return { kind: "expired" };
  if (ks === "failed") return { kind: "failed" };
  if (ks === "deleted") return isCancelState(status) ? { kind: "cancelled" } : { kind: "withdrawn" };
  if (ks === "broadcast") return { kind: "broadcast" };
  if (ks === "released") {
    return isCancelState(status) ? { kind: "may_still_pay", why: "released" } : { kind: "released" };
  }
  if (isCancelState(status)) return { kind: "may_still_pay", why: "race_window" };
  if (inst.state === "reserved") return { kind: "reserved" };
  return now >= releaseAt(inst) ? { kind: "releasing" } : { kind: "locked" };
}

/** Whether the padlock is drawn open. */
export function isUnlocked(p: InstallmentPhase): boolean {
  return (
    p.kind === "releasing" ||
    p.kind === "released" ||
    p.kind === "broadcast" ||
    p.kind === "paid" ||
    (p.kind === "may_still_pay" && p.why === "released")
  );
}

export type CancelOutcome = "final" | "will_cancel" | "race_window" | "released";

/** What a cancel issued now would do to this installment. */
export function cancelOutcome(inst: Installment): CancelOutcome {
  if (inst.state === "paid" || inst.state === "cancelled") return "final";
  if (inst.keeper_state && inst.keeper_state !== "locked") {
    return isOutOfPayerHands(inst.keeper_state) ? "released" : "final";
  }
  return inst.cancellable ? "will_cancel" : "race_window";
}

export function canCancel(order: Order): boolean {
  return !isCancelState(order.status) && order.status !== "completed";
}

export type StepState = "done" | "current" | "todo";

/** Funded → split → armed. */
export function orderSteps(order: Order): { funded: StepState; split: StepState; armed: StepState } {
  const insts = order.installments ?? [];
  const funded = order.status !== "awaiting_funds";
  const armedAny = insts.some((i) => i.blob_id !== null || i.state === "armed" || i.state === "paid");
  const armed = order.status === "active" || order.status === "completed" || armedAny;
  const split = armed || (order.split_txid !== null && order.status !== "splitting");
  const live = !isCancelState(order.status);
  const at = (done: boolean, prevDone: boolean): StepState => (done ? "done" : prevDone && live ? "current" : "todo");
  return {
    funded: at(funded, true),
    split: at(split, funded),
    armed: at(armed, split),
  };
}

/** One-line description of an order status for chips. */
export const STATUS_LABEL: Record<OrderStatus, string> = {
  awaiting_funds: "Awaiting funds",
  splitting: "Reserving",
  active: "Active",
  completed: "Completed",
  cancelling: "Cancelling",
  cancelled: "Cancelled",
};
