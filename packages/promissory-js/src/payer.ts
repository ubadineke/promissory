/** Client for the payer API of `promissory-app` (docs/api.md §3). */

import { HttpClient, type ClientOptions } from "./http.js";
import type { Network } from "./address.js";
import type { BlobState } from "./keeper.js";
import type { MadeReceipt, VerifiedReceipt } from "./receipt.js";

export type OrderStatus = "awaiting_funds" | "splitting" | "active" | "completed" | "cancelling" | "cancelled";

export type InstallmentState = "reserved" | "armed" | "paid" | "cancelled";

/** `inheritance` is a dead-man's switch: one payment to an heir unless renewed first. */
export type OrderKind = "standing" | "inheritance";

export interface Funding {
  /** Total to send to the treasury: installments, their fees and the split fee. */
  zat: number;
  address: string;
  /** ZIP-321 payment URI for a QR code. */
  uri: string;
  /** How many installments this request funds (counting what the treasury already holds). */
  installments?: number;
}

export interface Installment {
  /** 0-based; display as k + 1. */
  k: number;
  due_unix: number;
  /** drand quicknet round the transaction is locked to. */
  round: number;
  /** When that round is published. */
  release_unix: number;
  state: InstallmentState;
  keeper_state: BlobState | null;
  /** One per branch copy. */
  txids: string[];
  mined_height: number | null;
  blob_id: string | null;
  /** A fresh copy signed while online shortly before release; it releases 5 minutes first. */
  fresh_release_unix?: number | null;
  fresh_txid?: string | null;
  fresh_keeper_state?: BlobState | null;
  /** Expiry height of the pre-signed copy; 0 means it never expires (switches only). */
  expiry_height?: number | null;
  /** False once the keeper has it out or it is inside the race window. */
  cancellable: boolean;
}

export interface Order {
  id: number;
  status: OrderStatus;
  payee: string;
  amount_zat: number;
  memo: string;
  every_secs: number;
  count: number;
  first_due: number;
  keeper: string;
  split_txid: string | null;
  created_at: number;
  /** What to pay to start the order; `null` once it is funded. */
  funding: Funding | null;
  /** Rolling window: the next chunk to pay, when the treasury can't cover it. */
  topup?: Funding | null;
  /** Installments kept funded ahead; 0 means funded in full up front. */
  window?: number;
  /** The payroll batch this order belongs to, if any. */
  batch?: { id: number; name: string } | null;
  /** Absent from older servers: treat as `standing`. */
  kind?: OrderKind;
  /** Switches: the reminder lead time before the date (`switch_due` notice). */
  remind_secs?: number | null;
  /** Switches: the switch this one renewed. */
  renewed_from?: number | null;
  /** Switches: the switch that replaced this one when it was renewed. */
  renewed_to?: number | null;
  /** The cancel sweep, once the payer has cancelled. */
  sweep_txid: string | null;
  cancelled_at: number | null;
  /** Sum of installments paid so far. */
  paid_zat: number;
  /** Present on `getOrder` only. */
  installments?: Installment[];
}

export interface OrderDetail extends Order {
  installments: Installment[];
}

export interface Info {
  network: Network;
  tip_height: number;
  synced_height: number;
  treasury_address: string;
  treasury_zat: number;
  treasury_spendable_zat: number;
  default_keeper: string;
  cancel_margin_secs: number;
}

export interface CreateOrderRequest {
  payee: string;
  amount_zat: number;
  every_secs: number;
  count: number;
  memo?: string;
  /** Defaults to now + 600 s. */
  first_due?: number;
  jitter_max_secs?: number;
  keeper?: string;
  /** Keep only this many installments funded ahead (rolling window). */
  window?: number;
}

export interface CreateFromLinkRequest {
  /** A full Subscribe link. */
  link: string;
  first_due?: number;
  keeper?: string;
  /** Keep only this many installments funded ahead (rolling window). */
  window?: number;
  /** Attach the link's payee webhook to uploads. Off by default: the keeper learns the endpoint. */
  accept_hook?: boolean;
}

export interface CancelRequest {
  margin_secs?: number;
  keep_blobs?: boolean;
  /** Where the swept notes go. Defaults to the treasury. */
  to?: string;
}

export interface MayStillPay {
  k: number;
  /** `released`: the keeper already sent it. `race_window`: its round is too close to cancel. */
  code: "released" | "race_window";
  /** Human-readable form of `code`. */
  reason: string;
}

export interface CancelResult {
  /** Null when nothing was left to sweep. */
  sweep_txid: string | null;
  cancelled: number[];
  may_still_pay: MayStillPay[];
}

export interface CreateSwitchRequest {
  /** Heir's unified address (Orchard/Ironwood receiver). */
  heir: string;
  amount_zat: number;
  /** Unix seconds when it pays, unless renewed first. At least 10 minutes out. */
  at: number;
  memo?: string;
  /** Remind this long before `at`. Defaults to 7 days. */
  remind_secs?: number;
  keeper?: string;
}

export interface RenewSwitchRequest {
  /** New date, unix seconds; later than the current one. */
  at: number;
  /** Refuse if the old copy is due within this many seconds. Defaults to 600. */
  margin_secs?: number;
  /** Leave the old blob at the keeper; the chain then rejects it as a double-spend. */
  keep_blob?: boolean;
}

export interface RenewResult {
  old_id: number;
  new_id: number;
  /** The one transaction that moved the reservation; null if the old switch was never funded. */
  txid: string | null;
  withdrew_blob: boolean;
  /** The new switch. */
  order: OrderDetail;
}

/** A batch order: an `Order` plus the payee's name from the CSV (kept locally, never on chain). */
export interface BatchOrder extends Order {
  name: string;
}

export type BatchStatus = OrderStatus;

export interface Batch {
  id: number;
  name: string;
  created_at: number;
  first_due: number;
  /** `awaiting_funds` while any order waits for funds; `completed` or `cancelled` once all are done. */
  status: BatchStatus;
  order_count: number;
  /** Every installment of every order. */
  total_zat: number;
  paid_zat: number;
  /** What the treasury still needs for the orders awaiting funds; null once all are funded. */
  funding: Funding | null;
  /** In CSV order. `installments` are present on `getBatch` only. */
  orders: BatchOrder[];
}

export interface CreateBatchRequest {
  /** CSV text: `name,ua,amount,cadence,count`, optional header row. */
  csv: string;
  /** Batch name; also the memo every payee sees. */
  name?: string;
  /** First installment of every order. Defaults to now + 600 s. */
  first_due?: number;
  keeper?: string;
}

export interface CreatedBatch {
  batch_id: number;
  name: string;
  orders: BatchOrder[];
  /** One payment to the treasury that funds every order. */
  funding: Funding;
}

export interface BatchCancelOne extends Partial<CancelResult> {
  order_id: number;
  name: string;
  /** Present when this order's cancel failed; the others still went ahead. */
  error?: string;
}

export interface BatchCancelResult {
  batch_id: number;
  /** Orders already completed or cancelled are skipped. */
  results: BatchCancelOne[];
}

/** A display-only exchange rate. Amounts always stay fixed in ZEC. */
export interface Rates {
  /** Lower-case currency code, e.g. "ngn". */
  vs: string;
  /** Price of 1 ZEC in `vs`. Also present as `zec_<vs>`, e.g. `zec_ngn`. */
  rate: number;
  zec_ngn?: number;
  fetched_at: number;
  source: string;
}

/** Header every non-GET request must carry (the CSRF guard). */
export const SO_APP_HEADER = "X-Promissory-App";

/** Default local address of `promissory-app`. */
export const PAYER_DEFAULT_URL = "http://127.0.0.1:8738";

export class PayerClient {
  private readonly http: HttpClient;

  /**
   * `baseUrl` is the `promissory-app` root, without `/api/v1`. Pass `""` from a page that
   * `promissory-app` serves, so requests use relative paths.
   */
  constructor(baseUrl: string = PAYER_DEFAULT_URL, opts: ClientOptions = {}) {
    this.http = new HttpClient(baseUrl, opts);
  }

  private get<T>(path: string): Promise<T> {
    return this.http.request("GET", `/api/v1${path}`);
  }

  private post<T>(path: string, body?: unknown): Promise<T> {
    return this.http.request("POST", `/api/v1${path}`, body ?? {}, { [SO_APP_HEADER]: "1" });
  }

  info(): Promise<Info> {
    return this.get("/info");
  }

  /** Syncs the wallet, then returns `info`. */
  sync(): Promise<Info> {
    return this.post("/sync");
  }

  listOrders(): Promise<Order[]> {
    return this.get("/orders");
  }

  /** Creates an order awaiting funds. Pay `funding.uri` to start it. */
  createOrder(req: CreateOrderRequest): Promise<Order> {
    return this.post("/orders", req);
  }

  createOrderFromLink(req: CreateFromLinkRequest): Promise<Order> {
    return this.post("/orders/from-link", req);
  }

  /** The order with its installments; the server refreshes keeper state first. */
  getOrder(id: number): Promise<OrderDetail> {
    return this.get(`/orders/${encodeURIComponent(String(id))}`);
  }

  cancelOrder(id: number, req: CancelRequest = {}): Promise<CancelResult> {
    return this.post(`/orders/${encodeURIComponent(String(id))}/cancel`, req);
  }

  /**
   * Creates one order per CSV row and returns one combined funding request. The server
   * validates every row first; a 400 `ApiError` has `body.rows` with `{line, message}` items.
   */
  createBatch(req: CreateBatchRequest): Promise<CreatedBatch> {
    return this.post("/batches", req);
  }

  /** Newest first; orders without installments. */
  listBatches(): Promise<Batch[]> {
    return this.get("/batches");
  }

  /** The batch with every order's installments; keeper state refreshed. */
  getBatch(id: number): Promise<Batch> {
    return this.get(`/batches/${encodeURIComponent(String(id))}`);
  }

  /** Cancels every order of the batch that can still pay. */
  cancelBatch(id: number, req: Omit<CancelRequest, "to"> = {}): Promise<BatchCancelResult> {
    return this.post(`/batches/${encodeURIComponent(String(id))}/cancel`, req);
  }

  /** Creates a dead-man's switch awaiting funds. Pay `funding.uri` to arm it. */
  createSwitch(req: CreateSwitchRequest): Promise<OrderDetail> {
    return this.post("/switches", req);
  }

  /** "I'm still here": moves the reservation to a new switch with a later date, in one transaction. */
  renewSwitch(id: number, req: RenewSwitchRequest): Promise<RenewResult> {
    return this.post(`/switches/${encodeURIComponent(String(id))}/renew`, req);
  }

  /** A receipt for paid installment `k` (0-based) of an order: it opens that one output, nothing else. */
  getReceipt(orderId: number, k: number): Promise<MadeReceipt> {
    return this.get(`/orders/${encodeURIComponent(String(orderId))}/installments/${encodeURIComponent(String(k))}/receipt`);
  }

  /** Checks a receipt against the chain. Uses no keys; anyone's promissory-app can do it. */
  verifyReceipt(receipt: string): Promise<VerifiedReceipt> {
    return this.post("/receipts/verify", { receipt: receipt.trim() });
  }

  /** ZEC exchange rate for display, fetched and cached (5 min) by promissory-app. Default `vs`: "ngn". */
  rates(vs = "ngn"): Promise<Rates> {
    return this.get(`/rates?vs=${encodeURIComponent(vs)}`);
  }
}
