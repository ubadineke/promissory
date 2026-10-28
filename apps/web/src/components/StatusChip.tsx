import type { OrderStatus } from "@promissory/client";
import { STATUS_LABEL } from "../lib/phase";
import { Chip } from "./ui";

const TONE = {
  awaiting_funds: "warn",
  splitting: "accent",
  active: "ok",
  completed: "neutral",
  cancelling: "danger",
  cancelled: "neutral",
} as const;

/** `renewed`: a switch whose funds moved to a newer switch (its status is `cancelled`). */
export function StatusChip({ status, renewed = false }: { status: OrderStatus; renewed?: boolean }) {
  if (renewed) {
    return (
      <Chip tone="neutral">
        <span aria-hidden className="size-1.5 rounded-full border border-current" />
        Renewed
      </Chip>
    );
  }
  return (
    <Chip tone={TONE[status]}>
      <span
        aria-hidden
        className={
          status === "active" || status === "splitting" || status === "cancelling"
            ? "size-1.5 rounded-full bg-current"
            : "size-1.5 rounded-full border border-current"
        }
      />
      {STATUS_LABEL[status]}
    </Chip>
  );
}
