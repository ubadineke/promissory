/**
 * Payroll CSV: `name,ua,amount,cadence,count`, one standing order per row. The same rules as
 * `promissory-core::batch`, so a page can preview and validate before calling `createBatch`.
 */

import { checkPayeeAddress, type Network } from "./address.js";
import { FormatError, parseZec } from "./format.js";
import { COUNT_MAX, COUNT_MIN, EVERY_MIN_SECS } from "./link.js";

/** `cadence` keywords. `monthly` is a fixed 30 days. */
export const PAYROLL_CADENCES = {
  weekly: 604_800,
  biweekly: 1_209_600,
  monthly: 2_592_000,
} as const;

export const PAYROLL_HEADER = ["name", "ua", "amount", "cadence", "count"] as const;
export const PAYROLL_MAX_ROWS = 500;
export const PAYROLL_NAME_MAX_CHARS = 100;

/** ZIP 317 marginal fee per action. */
const MARGINAL_FEE = 5_000;
/** An installment is a 1-in, 1-out Ironwood transaction (2 actions). */
const INSTALLMENT_FEE = 10_000;

export interface PayrollRow {
  /** 1-based line number in the CSV. */
  line: number;
  name: string;
  ua: string;
  amountZat: number;
  everySecs: number;
  count: number;
}

export interface PayrollRowError {
  /** 1-based line number; 0 for errors about the whole file. */
  line: number;
  message: string;
}

export interface PayrollParse {
  /** Rows that passed every check. */
  rows: PayrollRow[];
  /** Every problem found, in line order. The batch is only valid when this is empty. */
  errors: PayrollRowError[];
  /** Sum of every installment of every valid row. */
  totalZat: number;
  /** What the treasury must receive for the valid rows, fees included (matches promissory-app). */
  fundingZat: number;
}

/** Parses a cadence: "weekly" | "biweekly" | "monthly" | whole seconds ≥ 60. Throws `FormatError`. */
export function parsePayrollCadence(input: string): number {
  const s = input.trim().toLowerCase();
  const preset = (PAYROLL_CADENCES as Record<string, number>)[s];
  if (preset !== undefined) return preset;
  if (!/^\d+$/.test(s)) throw new FormatError(`cadence "${input.trim()}" must be weekly, biweekly, monthly or a number of seconds`);
  const secs = Number(s);
  if (!Number.isSafeInteger(secs) || secs < EVERY_MIN_SECS) {
    throw new FormatError(`cadence must be at least ${EVERY_MIN_SECS} seconds`);
  }
  return secs;
}

/** Treasury funding one order needs when it shares a funding note with other orders. */
export function payrollFundingZat(amountZat: number, count: number): number {
  const fee = (outputs: number) => MARGINAL_FEE * Math.max(2, outputs);
  // One note per installment covering its own fee, the split fee, and the split's change output.
  return count * (amountZat + INSTALLMENT_FEE) + fee(count) + (fee(count + 1) - fee(count));
}

/** Splits one CSV line into trimmed fields, honouring double quotes (`""` is a literal quote). */
export function splitCsvLine(line: string): string[] {
  const fields: string[] = [];
  let cur = "";
  let quoted = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i]!;
    if (quoted) {
      if (c === '"' && line[i + 1] === '"') {
        cur += '"';
        i++;
      } else if (c === '"') {
        quoted = false;
      } else {
        cur += c;
      }
    } else if (c === '"' && cur.trim() === "") {
      cur = "";
      quoted = true;
    } else if (c === ",") {
      fields.push(cur.trim());
      cur = "";
    } else {
      cur += c;
    }
  }
  if (quoted) throw new FormatError("unclosed quote");
  fields.push(cur.trim());
  return fields;
}

/**
 * Parses and validates a payroll CSV for `network`. A header row is optional; blank lines and
 * lines starting with `#` are skipped. Returns every error with its line number.
 */
export function parsePayrollCsv(csv: string, network: Network): PayrollParse {
  const rows: PayrollRow[] = [];
  const errors: PayrollRowError[] = [];
  let seenData = false;
  csv.split(/\r\n|\n|\r/).forEach((raw, i) => {
    const line = i + 1;
    const text = raw.replace(/^﻿/, "").trim();
    if (text === "" || text.startsWith("#")) return;
    let f: string[];
    try {
      f = splitCsvLine(text);
    } catch (e) {
      errors.push({ line, message: (e as Error).message });
      return;
    }
    if (!seenData && f.length === PAYROLL_HEADER.length && f.every((v, j) => v.toLowerCase() === PAYROLL_HEADER[j])) {
      seenData = true;
      return;
    }
    seenData = true;
    if (f.length !== 5) {
      errors.push({ line, message: `expected 5 fields (name,ua,amount,cadence,count), found ${f.length}` });
      return;
    }
    const [name, ua, amount, cadence, count] = f as [string, string, string, string, string];
    const errs: string[] = [];
    if (name === "") errs.push("name is empty");
    else if ([...name].length > PAYROLL_NAME_MAX_CHARS) errs.push(`name is longer than ${PAYROLL_NAME_MAX_CHARS} characters`);
    if (ua === "") errs.push("ua is empty");
    else {
      try {
        checkPayeeAddress(ua, network);
      } catch (e) {
        errs.push(`ua: ${(e as Error).message}`);
      }
    }
    let amountZat = 0;
    try {
      amountZat = parseZec(amount);
      if (amountZat === 0) errs.push("amount must be greater than zero");
    } catch (e) {
      errs.push(`amount "${amount}": ${(e as Error).message}`);
    }
    let everySecs = 0;
    try {
      everySecs = parsePayrollCadence(cadence);
    } catch (e) {
      errs.push((e as Error).message);
    }
    const n = /^\d+$/.test(count) ? Number(count) : Number.NaN;
    if (!(Number.isSafeInteger(n) && n >= COUNT_MIN && n <= COUNT_MAX)) {
      errs.push(`count "${count}" must be a whole number from ${COUNT_MIN} to ${COUNT_MAX}`);
    }
    if (errs.length > 0) errors.push(...errs.map((message) => ({ line, message })));
    else rows.push({ line, name, ua, amountZat, everySecs, count: n });
  });
  if (rows.length === 0 && errors.length === 0) {
    errors.push({ line: 0, message: "no rows: expected name,ua,amount,cadence,count" });
  }
  if (rows.length > PAYROLL_MAX_ROWS) {
    errors.push({ line: 0, message: `at most ${PAYROLL_MAX_ROWS} rows per batch` });
  }
  return {
    rows,
    errors,
    totalZat: rows.reduce((s, r) => s + r.amountZat * r.count, 0),
    fundingZat: rows.reduce((s, r) => s + payrollFundingZat(r.amountZat, r.count), 0),
  };
}

/** Converts a fiat amount to whole zatoshis at `rate` (fiat per ZEC), rounding to the nearest. */
export function fiatToZat(fiat: number, rate: number): number {
  if (!(Number.isFinite(fiat) && fiat > 0 && Number.isFinite(rate) && rate > 0)) {
    throw new FormatError("enter a positive amount");
  }
  return Math.round((fiat / rate) * 1e8);
}

/** Fiat value of `zat` at `rate` (fiat per ZEC). Display only. */
export function zatToFiat(zat: number, rate: number): number {
  return (zat / 1e8) * rate;
}
