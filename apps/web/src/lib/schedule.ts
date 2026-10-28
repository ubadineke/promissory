/** Form state shared by the new-order form and the subscribe link builder, validated with promissory-js. */

import {
  COUNT_MAX,
  EVERY_MIN_SECS,
  FormatError,
  MEMO_MAX_BYTES,
  parseZec,
  subscribeParamErrors,
  type Network,
  type SubscribeParams,
} from "@promissory/client";

export interface CadenceValue {
  preset: number | "custom";
  custom: string;
}

export function cadenceSecs(v: CadenceValue): number {
  if (v.preset !== "custom") return v.preset;
  return /^\d+$/.test(v.custom.trim()) ? Number(v.custom.trim()) : Number.NaN;
}

export interface ScheduleForm {
  payee: string;
  amount: string;
  cadence: CadenceValue;
  count: string;
  memo: string;
  keeper: string;
}

export type ScheduleField = "payee" | "amount" | "every" | "count" | "memo" | "keeper";

export interface ScheduleCheck {
  errors: Partial<Record<ScheduleField, string>>;
  /** Present when there are no errors. */
  params?: SubscribeParams;
}

const capitalize = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** Validates the form for `network` and turns it into Subscribe parameters. */
export function checkSchedule(f: ScheduleForm, network: Network): ScheduleCheck {
  const errors: ScheduleCheck["errors"] = {};

  let amountZat = Number.NaN;
  try {
    amountZat = parseZec(f.amount);
  } catch (e) {
    errors.amount = e instanceof FormatError ? capitalize(e.message) : "Enter an amount like 0.01";
  }
  const count = /^\d+$/.test(f.count.trim()) ? Number(f.count.trim()) : Number.NaN;
  const params: Partial<SubscribeParams> = {
    network,
    payee: f.payee.trim(),
    amountZat,
    everySecs: cadenceSecs(f.cadence),
    count,
  };
  if (f.memo !== "") params.memo = f.memo;
  if (f.keeper.trim() !== "") params.keeper = f.keeper.trim();

  const linkErrors = subscribeParamErrors(params);
  if (linkErrors.to) {
    errors.payee =
      f.payee.trim() === ""
        ? "Paste the payee’s unified address"
        : capitalize(linkErrors.to.replace(/^to: /, ""));
  }
  if (linkErrors.amt && !errors.amount) errors.amount = "Enter an amount greater than 0";
  if (linkErrors.every) errors.every = `Use a whole number of seconds, at least ${EVERY_MIN_SECS}`;
  if (linkErrors.n) errors.count = `Enter a whole number from 1 to ${COUNT_MAX}`;
  if (linkErrors.memo) errors.memo = `Keep the memo within ${MEMO_MAX_BYTES} bytes`;
  if (linkErrors.keeper) errors.keeper = "Enter an http:// or https:// URL";

  return Object.keys(errors).length === 0 ? { errors, params: params as SubscribeParams } : { errors };
}

export function emptySchedule(): ScheduleForm {
  return { payee: "", amount: "", cadence: { preset: 604_800, custom: "" }, count: "12", memo: "", keeper: "" };
}

/** Errors worth showing now: all of them after a submit attempt, else only for fields with input. */
export function visibleErrors(f: ScheduleForm, errors: ScheduleCheck["errors"], submitted: boolean): ScheduleCheck["errors"] {
  if (submitted) return errors;
  const dirty: Record<ScheduleField, boolean> = {
    payee: f.payee !== "",
    amount: f.amount !== "",
    every: f.cadence.preset !== "custom" || f.cadence.custom !== "",
    count: f.count !== "",
    memo: true,
    keeper: f.keeper !== "",
  };
  return Object.fromEntries(Object.entries(errors).filter(([k]) => dirty[k as ScheduleField]));
}
