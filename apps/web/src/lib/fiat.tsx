/**
 * Naira display for remittances. Display only: amounts stay fixed in ZEC, and the rate never
 * reaches anything that is signed. Off until the payer turns it on (the "Send home" flow does),
 * because fetching a rate is a network request promissory-app makes on the payer's behalf.
 */

import { useSyncExternalStore } from "react";
import { zatToFiat, type Rates } from "@promissory/client";
import { api } from "../api";

const KEY = "promissory-fiat";
const REFRESH_MS = 60_000;

interface State {
  enabled: boolean;
  rate: Rates | undefined;
  error: string | undefined;
}

function readEnabled(): boolean {
  try {
    return localStorage.getItem(KEY) === "ngn";
  } catch {
    return false;
  }
}

let state: State = { enabled: readEnabled(), rate: undefined, error: undefined };
const listeners = new Set<() => void>();
let timer: number | undefined;
let inflight = false;

function set(patch: Partial<State>) {
  state = { ...state, ...patch };
  listeners.forEach((l) => l());
}

async function load() {
  if (!state.enabled || inflight) return;
  inflight = true;
  try {
    set({ rate: await api.rates("ngn"), error: undefined });
  } catch (e) {
    set({ error: e instanceof Error ? e.message : String(e) });
  } finally {
    inflight = false;
  }
}

function schedule() {
  window.clearInterval(timer);
  timer = undefined;
  if (state.enabled && listeners.size > 0) {
    void load();
    timer = window.setInterval(() => document.visibilityState === "visible" && void load(), REFRESH_MS);
  }
}

function subscribe(l: () => void) {
  listeners.add(l);
  if (listeners.size === 1) schedule();
  return () => {
    listeners.delete(l);
    if (listeners.size === 0) schedule();
  };
}

export function setNairaEnabled(enabled: boolean) {
  try {
    if (enabled) localStorage.setItem(KEY, "ngn");
    else localStorage.removeItem(KEY);
  } catch {
    // Storage unavailable: the choice lasts for this page view.
  }
  set({ enabled, ...(enabled ? {} : { rate: undefined, error: undefined }) });
  schedule();
}

/** The current ZEC→NGN rate when naira display is on and the rate is known. */
export function useNaira(): State & { ngnPerZec: number | undefined } {
  const s = useSyncExternalStore(subscribe, () => state);
  return { ...s, ngnPerZec: s.enabled ? (s.rate?.zec_ngn ?? s.rate?.rate) : undefined };
}

const ngn = new Intl.NumberFormat("en-NG", { style: "currency", currency: "NGN", maximumFractionDigits: 0 });
const time = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });

export function formatNaira(value: number): string {
  return ngn.format(value);
}

/** "≈ ₦…" for `zat`, or nothing when the rate is unknown or naira display is off. */
export function Naira({ zat, className }: { zat: number; className?: string }) {
  const { ngnPerZec } = useNaira();
  if (ngnPerZec === undefined || zat === 0) return null;
  return (
    <span className={className ?? "font-normal text-muted tabular-nums"} title="Display only; the amount is fixed in ZEC">
      {" "}
      ≈ {formatNaira(zatToFiat(zat, ngnPerZec))}
    </span>
  );
}

/** "Rate as of 14:05, display only." when a naira value is on screen. */
export function RateNote({ className }: { className?: string }) {
  const { rate, enabled } = useNaira();
  if (!enabled || !rate) return null;
  return (
    <p className={className ?? "text-xs text-muted"}>
      ₦ rate as of {time.format(rate.fetched_at * 1000)} ({rate.source}), display only. Amounts are fixed in ZEC.
    </p>
  );
}
