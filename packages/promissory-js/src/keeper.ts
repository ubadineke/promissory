/** Client for a `promissory-keeper` service (keeper contract in docs/plan.md). */

import { HttpClient, type ClientOptions } from "./http.js";
import type { Network } from "./address.js";

export type BlobState =
  | "locked"
  | "released"
  | "broadcast"
  | "mined"
  | "cancelled"
  | "expired"
  | "failed"
  | "deleted";

export const BLOB_STATES: readonly BlobState[] = [
  "locked",
  "released",
  "broadcast",
  "mined",
  "cancelled",
  "expired",
  "failed",
  "deleted",
];

export interface UploadedBlob {
  blob_id: string;
  /** Bearer token for `delete`. The keeper keeps only its hash; store it safely. */
  delete_token: string;
  /** drand quicknet round, read by the keeper from the age header. */
  round: number;
}

export interface BlobStatus {
  blob_id: string;
  round: number;
  state: BlobState;
  txid?: string;
  mined_height?: number;
  last_error?: string;
  updated_at: number;
}

export interface KeeperHealth {
  ok: boolean;
  network: Network;
  tip_height: number | null;
  latest_round: number | null;
  last_tick_at: number | null;
  last_error: string | null;
  webhooks: boolean;
  blobs: Record<BlobState, number>;
}

export interface UploadOptions {
  /** Notice URL; rejected unless the keeper runs with webhooks enabled. */
  webhook?: string;
}

/** Released or later: the keeper has the transaction out, so cancel can no longer stop it. */
export function isOutOfPayerHands(state: BlobState): boolean {
  return state === "released" || state === "broadcast" || state === "mined";
}

function toBase64(bytes: Uint8Array): string {
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(bin);
}

export class KeeperClient {
  private readonly http: HttpClient;

  /** `baseUrl` is the keeper root, e.g. `http://127.0.0.1:8737`. */
  constructor(baseUrl: string, opts: ClientOptions = {}) {
    this.http = new HttpClient(baseUrl, opts);
  }

  get baseUrl(): string {
    return this.http.baseUrl;
  }

  /** Uploads a tlock-age ciphertext (raw bytes, or already base64). Body limit 64 KiB. */
  upload(ciphertext: Uint8Array | string, opts: UploadOptions = {}): Promise<UploadedBlob> {
    const body: { ciphertext: string; webhook?: string } = {
      ciphertext: typeof ciphertext === "string" ? ciphertext : toBase64(ciphertext),
    };
    if (opts.webhook !== undefined) body.webhook = opts.webhook;
    return this.http.request("POST", "/v1/blobs", body);
  }

  status(blobId: string): Promise<BlobStatus> {
    return this.http.request("GET", `/v1/blobs/${encodeURIComponent(blobId)}`);
  }

  /** Withdraws a still-locked blob. Fails with 409 once it has been released. */
  async delete(blobId: string, deleteToken: string): Promise<void> {
    await this.http.request("DELETE", `/v1/blobs/${encodeURIComponent(blobId)}`, undefined, {
      Authorization: `Bearer ${deleteToken}`,
    });
  }

  health(): Promise<KeeperHealth> {
    return this.http.request("GET", "/v1/health");
  }
}
