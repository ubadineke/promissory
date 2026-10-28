/**
 * Agent allowances and agent requests on the payer API of `promissory-app` (docs/allowances.md).
 *
 * An allowance is a standing order to an agent's address: each period's cap is cut into equal
 * chips released one every `period / chips` seconds, with one period funded at a time.
 */

import { HttpClient, type ClientOptions } from "./http.js";
import type { CancelResult, Funding, Order } from "./payer.js";

/** Limits, as `promissory-app` enforces them. */
export const ALLOWANCE_MAX_CHIPS = 30;
export const ALLOWANCE_MIN_CHIP_ZAT = 100_000;
export const ALLOWANCE_MIN_SPACING_SECS = 60;
export const ALLOWANCE_MAX_TOTAL_CHIPS = 120;
export const ALLOWANCE_LABEL_MAX_CHARS = 100;
/** At most this many agent requests wait for the payer at once. */
export const MAX_PENDING_REQUESTS = 10;
const MAX_MONEY_ZAT = 21_000_000 * 100_000_000;

export type AllowanceStatus =
  | "awaiting_funds"
  | "starting"
  | "active"
  | "completed"
  | "stopping"
  | "stopped"
  | "replacing"
  | "replaced";

export interface Allowance {
  id: number;
  /** The standing order that releases the chips. */
  order_id: number;
  status: AllowanceStatus;
  /** The agent's receiving address. */
  agent_ua: string;
  label: string;
  cap_zat: number;
  period_secs: number;
  chips: number;
  chip_zat: number;
  /** Seconds between chips: `period_secs / chips`. */
  every_secs: number;
  periods: number;
  first_due: number;
  created_at: number;
  stopped_at: number | null;
  /** The allowance that replaced this one after a cap change. */
  replaced_by: number | null;
  /** 1-based. */
  current_period: number;
  period_start: number;
  period_end: number;
  /** Released to the agent this period. What the agent does with it afterwards is its own. */
  spent_this_period_zat: number;
  /** Chips of this period not released yet. */
  remaining_this_period_zat: number;
  released_total_zat: number;
  next_release_unix: number | null;
  /** What to pay to start the allowance; null once funded. */
  funding: Funding | null;
  /** The next period's chips, when the treasury can't cover them. */
  topup: Funding | null;
  keeper: string;
}

export interface CreateAllowanceRequest {
  agent_ua: string;
  /** Cap per period; a multiple of `chips`. */
  cap_zat: number;
  /** A multiple of `chips`, with `period_secs / chips` ≥ 60. */
  period_secs: number;
  chips: number;
  periods: number;
  /** Defaults to now + 600 s. */
  first_due?: number;
  label?: string;
  keeper?: string;
}

export interface StopAllowanceRequest {
  /** Default: the app's cancel margin (600 s). */
  margin_secs?: number;
}

export interface StopAllowanceResult extends CancelResult {
  allowance: Allowance;
}

export type AgentRequestStatus = "pending" | "approved" | "rejected" | "expired";

interface AgentRequestBase {
  id: number;
  status: AgentRequestStatus;
  created_at: number;
  decided_at: number | null;
  /** The order an approved one-off created. */
  order_id: number | null;
  /** The allowance an approved cap change created. */
  allowance_id_new: number | null;
  /** Why the last approval attempt failed; the request stays pending. */
  error: string | null;
  /** Why the agent asks. */
  reason?: string;
}

export interface OneoffRequest extends AgentRequestBase {
  kind: "oneoff";
  payee: string;
  amount_zat: number;
  due?: number;
  memo?: string;
}

export interface AllowanceChangeRequest extends AgentRequestBase {
  kind: "allowance_change";
  allowance_id: number;
  new_cap_zat: number;
}

export type AgentRequest = OneoffRequest | AllowanceChangeRequest;

export type NewAgentRequest =
  | { kind: "oneoff"; payee: string; amount_zat: number; due?: number; memo?: string; reason?: string }
  | { kind: "allowance_change"; allowance_id: number; new_cap_zat: number; reason?: string };

export interface ApprovedRequest {
  request: AgentRequest;
  /** The one-off's order (awaiting funds until the treasury covers it). */
  order?: Order;
  /** The replacement allowance of a cap change. */
  allowance?: Allowance;
}

export interface AllowancePlan {
  chipZat: number;
  everySecs: number;
  /** Installments of the order: chips × periods. */
  count: number;
  /** Rolling window: one period's chips. */
  window: number;
}

export type AllowanceField = "cap_zat" | "period_secs" | "chips" | "periods";

/** Every problem with an allowance's shape, by field, as `promissory-app` would report them. */
export function allowanceErrors(cap: number, periodSecs: number, chips: number, periods: number): Partial<Record<AllowanceField, string>> {
  const e: Partial<Record<AllowanceField, string>> = {};
  const int = (n: number) => Number.isSafeInteger(n);
  if (!int(chips) || chips < 1 || chips > ALLOWANCE_MAX_CHIPS) e.chips = `Chips must be 1–${ALLOWANCE_MAX_CHIPS}`;
  if (!int(periods) || periods < 1) e.periods = "At least 1 period";
  else if (!e.chips && chips * periods > ALLOWANCE_MAX_TOTAL_CHIPS)
    e.periods = `Chips × periods must be at most ${ALLOWANCE_MAX_TOTAL_CHIPS}`;
  if (!int(cap) || cap < 1 || cap > MAX_MONEY_ZAT) e.cap_zat = "Enter a cap greater than zero";
  else if (!e.chips && cap % chips !== 0) e.cap_zat = `The cap must split into ${chips} equal chips`;
  else if (!e.chips && cap / chips < ALLOWANCE_MIN_CHIP_ZAT) e.cap_zat = "Each chip must be at least 0.001 ZEC";
  if (!int(periodSecs) || periodSecs < 1) e.period_secs = "Enter a period";
  else if (!e.chips && periodSecs % chips !== 0) e.period_secs = `The period must split into ${chips} equal gaps`;
  else if (!e.chips && periodSecs / chips < ALLOWANCE_MIN_SPACING_SECS)
    e.period_secs = `Chips must be at least ${ALLOWANCE_MIN_SPACING_SECS} s apart`;
  return e;
}

/** How an allowance maps onto a standing order, or null if its shape is invalid. */
export function planAllowance(cap: number, periodSecs: number, chips: number, periods: number): AllowancePlan | null {
  if (Object.keys(allowanceErrors(cap, periodSecs, chips, periods)).length > 0) return null;
  return { chipZat: cap / chips, everySecs: periodSecs / chips, count: chips * periods, window: chips };
}

const path = (n: number) => encodeURIComponent(String(n));

export class AllowanceClient {
  private readonly http: HttpClient;

  /** `baseUrl` is the `promissory-app` root; `""` from a page `promissory-app` serves. */
  constructor(baseUrl: string, opts: ClientOptions = {}) {
    this.http = new HttpClient(baseUrl, opts);
  }

  private read<T>(p: string): Promise<T> {
    return this.http.request("GET", `/api/v1${p}`);
  }

  private write<T>(p: string, body: unknown = {}): Promise<T> {
    return this.http.request("POST", `/api/v1${p}`, body, { "X-Promissory-App": "1" });
  }

  /** Creates the allowance's order, awaiting funds: pay `funding.uri` to start it. */
  create(req: CreateAllowanceRequest): Promise<Allowance> {
    return this.write("/allowances", req);
  }

  /** Newest first, from local state. */
  list(): Promise<Allowance[]> {
    return this.read("/allowances");
  }

  /** Keeper state refreshed. */
  get(id: number): Promise<Allowance> {
    return this.read(`/allowances/${path(id)}`);
  }

  /** Cancels the order: every chip not yet released goes back to the treasury. */
  stop(id: number, req: StopAllowanceRequest = {}): Promise<StopAllowanceResult> {
    return this.write(`/allowances/${path(id)}/stop`, req);
  }

  /** Records a pending request; moves nothing. 429 when 10 are already waiting. */
  createRequest(req: NewAgentRequest): Promise<AgentRequest> {
    return this.write("/requests", req);
  }

  /** Newest first. */
  listRequests(): Promise<AgentRequest[]> {
    return this.read("/requests");
  }

  getRequest(id: number): Promise<AgentRequest> {
    return this.read(`/requests/${path(id)}`);
  }

  /** The payer's approval: only this creates the order or the new allowance. */
  approveRequest(id: number): Promise<ApprovedRequest> {
    return this.write(`/requests/${path(id)}/approve`);
  }

  rejectRequest(id: number): Promise<AgentRequest> {
    return this.write(`/requests/${path(id)}/reject`);
  }
}
