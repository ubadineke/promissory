/** Shared HTTP plumbing for the keeper and payer clients. */

export type FetchLike = (input: string, init?: RequestInit) => Promise<Response>;

export interface ClientOptions {
  /** Custom `fetch`, e.g. for tests, proxies or Tor. Defaults to the global one. */
  fetch?: FetchLike;
  /** Extra headers sent with every request. */
  headers?: Record<string, string>;
}

/** The server answered with a 4xx/5xx status. `message` is its `{"error"}` text when present. */
export class ApiError extends Error {
  override name = "ApiError";
  constructor(
    readonly status: number,
    message: string,
    readonly body?: unknown,
  ) {
    super(message);
  }
}

/** The request never got an HTTP answer: connection refused, DNS failure, CORS, abort. */
export class NetworkError extends Error {
  override name = "NetworkError";
  constructor(
    message: string,
    override readonly cause?: unknown,
  ) {
    super(message);
  }
}

export class HttpClient {
  private readonly fetchImpl: FetchLike;
  private readonly headers: Record<string, string>;

  constructor(
    readonly baseUrl: string,
    opts: ClientOptions = {},
  ) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    // Wrapped so the global is never called with a foreign `this`.
    this.fetchImpl = opts.fetch ?? ((input, init) => globalThis.fetch(input, init));
    this.headers = opts.headers ?? {};
  }

  async request<T>(method: string, path: string, body?: unknown, headers: Record<string, string> = {}): Promise<T> {
    const init: RequestInit = {
      method,
      headers: {
        Accept: "application/json",
        ...(body !== undefined ? { "Content-Type": "application/json" } : {}),
        ...this.headers,
        ...headers,
      },
    };
    if (body !== undefined) init.body = JSON.stringify(body);

    let res: Response;
    try {
      res = await this.fetchImpl(this.baseUrl + path, init);
    } catch (e) {
      throw new NetworkError(`${method} ${path}: ${e instanceof Error ? e.message : String(e)}`, e);
    }

    const text = await res.text();
    let data: unknown = undefined;
    if (text !== "") {
      try {
        data = JSON.parse(text);
      } catch {
        data = text;
      }
    }
    if (!res.ok) {
      const msg =
        data && typeof data === "object" && typeof (data as { error?: unknown }).error === "string"
          ? (data as { error: string }).error
          : `${res.status} ${res.statusText || "error"}`.trim();
      throw new ApiError(res.status, msg, data);
    }
    return data as T;
  }
}
