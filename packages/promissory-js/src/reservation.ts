/**
 * Proof of reservation (docs/api.md, "Proof of reservation"): the payer's opt-in export of a
 * viewing key scoped to one order, and the payee-side check that promissory-app runs against the chain.
 */

import { HttpClient, type ClientOptions } from "./http.js";
import type { Network } from "./address.js";
import type { BlobState } from "./keeper.js";

/** Prefix of every proof string. */
export const RESERVATION_PREFIX = "promissory-reserve:v1:";

export interface ReservationBlob {
  /** 0-based installment index. */
  k: number;
  /** drand quicknet round the installment is locked to. */
  round: number;
  /** `null` until the order is armed. */
  blob_id: string | null;
}

/** The decoded JSON inside `promissory-reserve:v1:<base64url(json)>`. */
export interface ReservationProof {
  v: 1;
  net: Network;
  /** ZIP 316 UFVK with only an Orchard item: the order's account, nothing else. */
  ufvk: string;
  /** Birthday height of the order's account. */
  birthday: number;
  payee: string;
  amount_zat: number;
  /** Each reserved note: `amount_zat` plus that installment's fee. */
  note_zat: number;
  count: number;
  every_secs: number;
  first_due: number;
  memo: string;
  keeper: string;
  blobs: ReservationBlob[];
}

export class ReservationError extends Error {
  override name = "ReservationError";
}

function base64UrlDecode(s: string): string {
  if (!/^[A-Za-z0-9_-]*=*$/.test(s)) throw new ReservationError("proof is not valid base64url");
  const b64 = s.replace(/=+$/, "").replace(/-/g, "+").replace(/_/g, "/");
  const bin = atob(b64 + "=".repeat((4 - (b64.length % 4)) % 4));
  return new TextDecoder().decode(Uint8Array.from(bin, (c) => c.charCodeAt(0)));
}

/**
 * Parses a proof string locally, without contacting anything. Use it to preview the terms;
 * only `verifyReservation` checks them against the chain.
 */
export function decodeReservationProof(text: string): ReservationProof {
  const s = text.trim();
  if (!s.startsWith(RESERVATION_PREFIX)) {
    throw new ReservationError(`not a proof of reservation (expected it to start with ${RESERVATION_PREFIX})`);
  }
  let p: ReservationProof;
  try {
    p = JSON.parse(base64UrlDecode(s.slice(RESERVATION_PREFIX.length))) as ReservationProof;
  } catch (e) {
    if (e instanceof ReservationError) throw e;
    throw new ReservationError("proof of reservation is not valid JSON");
  }
  if (p.v !== 1) throw new ReservationError(`unsupported proof of reservation version ${String(p.v)}`);
  if (typeof p.ufvk !== "string" || typeof p.payee !== "string" || !Array.isArray(p.blobs)) {
    throw new ReservationError("proof of reservation is missing fields");
  }
  return p;
}

/** Inverse of `decodeReservationProof` (for tests and tools; promissory-app builds real proofs). */
export function encodeReservationProof(p: ReservationProof): string {
  const bytes = new TextEncoder().encode(JSON.stringify(p));
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return RESERVATION_PREFIX + btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export type ReservedNoteState = "reserved" | "paid" | "swept" | "spent_unknown";

export interface ReservedNote {
  /** The transaction that created the note (the split). */
  txid: string;
  action_index: number;
  value_zat: number;
  received_height: number | null;
  /**
   * `reserved`: unspent. `paid`: spent to the payee. `swept`: spent without paying the payee
   * (the payer's cancel; where it went is not visible). `spent_unknown`: spending tx unavailable.
   */
  state: ReservedNoteState;
  spent_txid: string | null;
  spent_height: number | null;
  paid_zat: number | null;
  paid_memo: string | null;
}

export interface KeeperBlobView {
  k: number | null;
  blob_id: string;
  round: number | null;
  release_unix: number | null;
  state: BlobState | null;
  txid: string | null;
  mined_height: number | null;
  /** Set when the keeper could not be asked. */
  error: string | null;
}

export interface ReservationReport {
  net: Network;
  terms: {
    payee: string;
    amount_zat: number;
    note_zat: number;
    count: number;
    every_secs: number;
    first_due: number;
    memo: string;
    keeper: string;
  };
  tip_height: number;
  synced_height: number;
  funded: number;
  funded_zat: number;
  /** `funded === count` and every note is worth `note_zat`. */
  matches_terms: boolean;
  reserved: number;
  reserved_zat: number;
  paid: number;
  paid_zat: number;
  swept: number;
  swept_zat: number;
  spent_unknown: number;
  /** Some reserved notes were spent without paying the payee. */
  cancelled_by_payer: boolean;
  next_release_unix: number | null;
  notes: ReservedNote[];
  keeper: KeeperBlobView[];
  warnings: string[];
}

const SO_APP_HEADER = "X-Promissory-App";

/** The reservation endpoints of `promissory-app`. Same base URL as `PayerClient`. */
export class ReservationClient {
  private readonly http: HttpClient;

  constructor(baseUrl: string, opts: ClientOptions = {}) {
    this.http = new HttpClient(baseUrl, opts);
  }

  /** Payer side: the proof string for one order. Sharing it is the payer's choice. */
  async exportProof(orderId: number): Promise<string> {
    const r = await this.http.request<{ proof: string }>(
      "GET",
      `/api/v1/orders/${encodeURIComponent(String(orderId))}/reservation`,
    );
    return r.proof;
  }

  /** Payee side: scans the chain with the proof's viewing key in a separate view-only wallet. */
  verify(proof: string): Promise<ReservationReport> {
    return this.http.request("POST", "/api/v1/reservations/verify", { proof: proof.trim() }, { [SO_APP_HEADER]: "1" });
  }

  /** Keeper state for blob IDs, no keys involved. promissory-app fetches it (keepers send no CORS headers). */
  keeperStatus(keeper: string, blobIds: string[]): Promise<KeeperBlobView[]> {
    return this.http.request(
      "POST",
      "/api/v1/reservations/keeper-status",
      { keeper, blob_ids: blobIds },
      { [SO_APP_HEADER]: "1" },
    );
  }
}
