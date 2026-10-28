/** Dead-man's switch: what the Switch page shows, derived from orders. Pure and tested. */

import type { Order } from "@promissory/client";

export const DAY = 86_400;

/** Renewal presets offered next to the custom date. */
export const RENEW_PRESETS = [
  { label: "30 days", secs: 30 * DAY },
  { label: "90 days", secs: 90 * DAY },
  { label: "1 year", secs: 365 * DAY },
] as const;

export function isSwitch(o: Order): boolean {
  return o.kind === "inheritance";
}

/** Switches still guarding something: not renewed into another one, not paid, not cancelled. */
export function liveSwitches(orders: Order[]): Order[] {
  return orders
    .filter((o) => isSwitch(o) && o.renewed_to == null && !["completed", "cancelled", "cancelling"].includes(o.status))
    .sort((a, b) => a.first_due - b.first_due);
}

/** Switches that ended: paid to the heir, or cancelled outright (renewals are not history). */
export function pastSwitches(orders: Order[]): Order[] {
  return orders.filter((o) => isSwitch(o) && o.renewed_to == null && ["completed", "cancelled", "cancelling"].includes(o.status));
}

export type SwitchPhase = "unfunded" | "arming" | "armed" | "due_soon" | "firing";

/** Where a live switch stands, for its status line. `remind` defaults to 7 days. */
export function switchPhase(o: Order, now: number): SwitchPhase {
  if (o.status === "awaiting_funds") return "unfunded";
  if (o.status === "splitting") return "arming";
  if (now >= o.first_due) return "firing";
  if (o.first_due - now <= (o.remind_secs ?? 7 * DAY)) return "due_soon";
  return "armed";
}

/** The new date a preset gives: from the later of now and the current date, so renewing never shortens it. */
export function renewTarget(o: Order, now: number, secs: number): number {
  return Math.max(now, o.first_due) + secs;
}

/** Why a renewal date is refused locally, or null. */
/**
 * Inside the last `marginSecs` before it fires, a switch can no longer be renewed: the keeper may
 * already be broadcasting it, so a renewal could race the payment. Mirrors the server's margin.
 */
export function tooLateToRenew(o: Order, now: number, marginSecs = 600): boolean {
  return o.first_due - now < marginSecs;
}

export function renewError(o: Order, at: number | null, now: number, marginSecs = 600): string | null {
  if (tooLateToRenew(o, now, marginSecs)) {
    return `Too late to renew: it pays in under ${Math.round(marginSecs / 60)} minutes`;
  }
  if (at === null) return "Choose a date";
  if (at <= o.first_due) return "The new date must be later than the current one";
  if (at < now + marginSecs) return `Choose a date at least ${Math.round(marginSecs / 60)} minutes from now`;
  return null;
}
