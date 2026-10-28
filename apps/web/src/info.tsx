import { createContext, useContext, type ReactNode } from "react";
import type { Info } from "@promissory/client";
import { api } from "./api";
import { usePoll, type Poll } from "./hooks";

const InfoContext = createContext<Poll<Info> | null>(null);

/** Shares `GET /info` (network, balances, sync height) with every page; refreshed every 5 s. */
export function InfoProvider({ children }: { children: ReactNode }) {
  const poll = usePoll(() => api.info(), 5_000);
  return <InfoContext.Provider value={poll}>{children}</InfoContext.Provider>;
}

export function useInfo(): Poll<Info> {
  const ctx = useContext(InfoContext);
  if (!ctx) throw new Error("useInfo outside InfoProvider");
  return ctx;
}
