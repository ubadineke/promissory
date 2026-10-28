/** Payer-side notices from `promissory-app` (docs/api.md, "Notices"). */

import { HttpClient, type ClientOptions } from "./http.js";

export type NoticeKind =
  | "renewal_paid"
  | "payment_failed_cancelled"
  | "keeper_unreachable"
  | "funds_low"
  /** A dead-man's switch is inside its reminder window: renew it or it pays the heir. */
  | "switch_due";

export interface AppNotice {
  /** Stable ID, e.g. `paid:3:0`. */
  id: string;
  kind: NoticeKind;
  tone: "ok" | "warn" | "danger";
  title: string;
  message: string;
  order_id: number | null;
  k: number | null;
  at: number;
  /** `keeper_unreachable` is not: it clears when the keeper answers again. */
  dismissible: boolean;
}

export class NoticesClient {
  private readonly http: HttpClient;

  constructor(baseUrl: string, opts: ClientOptions = {}) {
    this.http = new HttpClient(baseUrl, opts);
  }

  /** Current notices, most urgent first. */
  list(): Promise<AppNotice[]> {
    return this.http.request("GET", "/api/v1/notices");
  }

  async dismiss(id: string): Promise<void> {
    await this.http.request("POST", `/api/v1/notices/${encodeURIComponent(id)}/dismiss`, {}, { "X-Promissory-App": "1" });
  }
}
