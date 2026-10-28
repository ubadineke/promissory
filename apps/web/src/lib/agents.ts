/** Display rules for agent allowances and agent requests. Pure and tested. */

import { formatZec, type AgentRequest, type AgentRequestStatus, type Allowance, type AllowanceStatus } from "@promissory/client";

type Tone = "neutral" | "accent" | "ok" | "warn" | "danger";

export const ALLOWANCE_STATUS: Record<AllowanceStatus, { label: string; tone: Tone; live: boolean }> = {
  awaiting_funds: { label: "Awaiting funds", tone: "warn", live: false },
  starting: { label: "Starting", tone: "accent", live: true },
  active: { label: "Active", tone: "ok", live: true },
  completed: { label: "Completed", tone: "neutral", live: false },
  stopping: { label: "Stopping", tone: "danger", live: true },
  stopped: { label: "Stopped", tone: "neutral", live: false },
  replacing: { label: "Replacing", tone: "accent", live: true },
  replaced: { label: "Replaced", tone: "neutral", live: false },
};

export const REQUEST_STATUS: Record<AgentRequestStatus, { label: string; tone: Tone }> = {
  pending: { label: "Waiting for you", tone: "warn" },
  approved: { label: "Approved", tone: "ok" },
  rejected: { label: "Rejected", tone: "neutral" },
  expired: { label: "Expired", tone: "neutral" },
};

/** Whether the payer can still stop it. */
export function canStop(a: Allowance): boolean {
  return a.status === "awaiting_funds" || a.status === "starting" || a.status === "active" || a.status === "stopping";
}

/** Share of this period's cap released to the agent, 0–100. */
export function periodPct(a: Allowance): number {
  return a.cap_zat > 0 ? Math.min(100, (a.spent_this_period_zat / a.cap_zat) * 100) : 0;
}

/** Network fees for funding `n` notes of one order: one per chip plus the split. */
export function fundingZat(chipZat: number, n: number): number {
  return n * (chipZat + 10_000) + 5_000 * Math.max(2, n);
}

/** One line saying what the agent asks for. */
export function requestTitle(r: AgentRequest): string {
  return r.kind === "oneoff"
    ? `Pay ${formatZec(r.amount_zat)} ZEC once`
    : `Change allowance #${r.allowance_id} to ${formatZec(r.new_cap_zat)} ZEC per period`;
}

/** Pending first, then newest first. */
export function sortRequests(rs: AgentRequest[]): AgentRequest[] {
  return [...rs].sort((a, b) => Number(b.status === "pending") - Number(a.status === "pending") || b.id - a.id);
}
