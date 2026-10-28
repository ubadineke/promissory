/** The one `PayerClient` the app uses: relative to the page, or the mock in mock mode. */

import { AllowanceClient, NoticesClient, PayerClient, ReservationClient, type Network } from "@promissory/client";
import { createMockFetch } from "./lib/mock";
import { readSession, writeSession } from "./lib/storage";

const MOCK_KEY = "promissory-mock";

function mockNetwork(): Network | null {
  const q = new URLSearchParams(window.location.search).get("mock");
  if (q === "0" || q === "off") {
    writeSession(MOCK_KEY, null);
    return null;
  }
  if (q !== null) {
    const net: Network = q === "main" ? "main" : "regtest";
    writeSession(MOCK_KEY, net);
    return net;
  }
  const stored = readSession(MOCK_KEY);
  if (stored === "main" || stored === "regtest") return stored;
  return import.meta.env.VITE_MOCK === "1" ? "regtest" : null;
}

const mock = mockNetwork();

export const isMock = mock !== null;

const clientOpts = mock ? { fetch: createMockFetch(mock) } : {};

export const api = new PayerClient("", clientOpts);

/** Proof of reservation: export (payer) and verify (payee). */
export const reservations = new ReservationClient("", clientOpts);

/** Payer-side banners. */
export const notices = new NoticesClient("", clientOpts);

/** Agent allowances and the requests agents send through promissory-mcp. */
export const agents = new AllowanceClient("", clientOpts);
