import { useEffect, useRef, useState, type ReactNode } from "react";
import { Ban, CircleCheck, LockOpen, TriangleAlert } from "lucide-react";
import { formatDuration, formatZec, type CancelResult, type OrderDetail } from "@promissory/client";
import { api } from "../api";
import { cancelOutcome } from "../lib/phase";
import { Address, Button, Notice } from "./ui";

const list = (ks: number[]) => ks.map((k) => `#${k + 1}`).join(", ");

function Group({ icon, title, ks, children }: { icon: ReactNode; title: string; ks: number[]; children?: ReactNode }) {
  if (ks.length === 0) return null;
  return (
    <li className="flex gap-3 py-2.5">
      <span className="mt-0.5 shrink-0" aria-hidden>
        {icon}
      </span>
      <div className="min-w-0 text-sm">
        <p className="font-medium">
          {title} <span className="font-normal text-muted tabular-nums">{list(ks)}</span>
        </p>
        {children && <p className="mt-0.5 text-muted text-pretty">{children}</p>}
      </div>
    </li>
  );
}

/** Explains exactly what cancel does, sends it, then shows the per-installment outcome. */
export function CancelDialog({
  order,
  marginSecs,
  open,
  onClose,
  onDone,
}: {
  order: OrderDetail;
  marginSecs: number;
  open: boolean;
  onClose: () => void;
  onDone: (r: CancelResult) => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [result, setResult] = useState<CancelResult>();

  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) {
      setError(undefined);
      setResult(undefined);
      d.showModal();
    } else if (!open && d.open) d.close();
  }, [open]);

  const outcomes = order.installments.map((i) => ({ k: i.k, o: cancelOutcome(i) }));
  const pick = (o: string) => outcomes.filter((x) => x.o === o).map((x) => x.k);
  const willCancel = pick("will_cancel");
  const race = pick("race_window");
  const released = pick("released");

  async function confirm() {
    setBusy(true);
    setError(undefined);
    try {
      const r = await api.cancelOrder(order.id);
      setResult(r);
      onDone(r);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <dialog
      ref={ref}
      aria-labelledby="cancel-title"
      onCancel={(e) => {
        if (busy) e.preventDefault();
        else onClose();
      }}
      className="m-auto w-[calc(100%-2rem)] max-w-lg rounded-3xl border border-border bg-surface p-0 text-fg shadow-2xl"
    >
      <div className="anim-in px-6 py-6">
        {!result ? (
          <>
            <h2 id="cancel-title" className="font-display text-xl font-extrabold tracking-tight">
              Cancel standing order #{order.id}?
            </h2>
            <div className="mt-3 space-y-3 text-sm text-muted text-pretty">
              <p>
                The app sends <strong className="text-fg">one transaction</strong> that sweeps the reserved funds of every payment not
                yet released back to your treasury. When their time comes, the keeper’s copies fail on-chain as double-spends, so
                they can never pay.
              </p>
              <p>
                Payments released in the next {formatDuration(Math.max(60, marginSecs))} are inside the{" "}
                <strong className="text-fg">race window</strong>: the sweep may not be mined before they are, so they may still pay.
              </p>
            </div>

            <ul className="mt-4 divide-y divide-border rounded-2xl border border-border px-4">
              <Group icon={<Ban className="size-4 text-muted" />} title="Will be cancelled:" ks={willCancel}>
                About {formatZec(willCancel.length * order.amount_zat, { unit: true })} returns to your treasury, less one network fee.
              </Group>
              <Group icon={<TriangleAlert className="size-4 text-warn" />} title="May still pay: inside the race window:" ks={race} />
              <Group icon={<LockOpen className="size-4 text-warn" />} title="Already released by the keeper:" ks={released}>
                These will pay.
              </Group>
              {willCancel.length + race.length + released.length === 0 && (
                <li className="py-2.5 text-sm text-muted">No payments are left to cancel.</li>
              )}
            </ul>

            <div className="mt-4">
              <Notice tone="warn" icon={<TriangleAlert className="size-4" aria-hidden />}>
                <strong>Once the keeper has broadcast an installment, cancel cannot stop it.</strong>
              </Notice>
            </div>

            {error && (
              <div className="mt-3">
                <Notice tone="danger">
                  <strong>Cancel failed.</strong> {error}
                </Notice>
              </div>
            )}

            <div className="mt-5 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
              <Button onClick={onClose} disabled={busy} autoFocus>
                Keep order
              </Button>
              <Button variant="danger" onClick={() => void confirm()} disabled={busy || willCancel.length + race.length === 0}>
                {busy ? "Sending sweep…" : "Cancel order"}
              </Button>
            </div>
          </>
        ) : (
          <>
            <h2 id="cancel-title" className="flex items-center gap-2 font-display text-xl font-extrabold tracking-tight">
              {result.sweep_txid || result.cancelled.length > 0 ? (
                <>
                  <CircleCheck className="size-5 text-ok" aria-hidden />
                  Cancel sent
                </>
              ) : (
                // Every remaining payment was released or inside the race window: nothing was swept.
                <>
                  <TriangleAlert className="size-5 text-warn" aria-hidden />
                  Too late to stop these payments
                </>
              )}
            </h2>
            <CancelResultView result={result} />
            <div className="mt-5 flex justify-end">
              <Button variant="primary" onClick={onClose} autoFocus>
                Done
              </Button>
            </div>
          </>
        )}
      </div>
    </dialog>
  );
}

/** Per-installment results of a cancel. */
export function CancelResultView({ result }: { result: CancelResult }) {
  return (
    <div className="mt-3 space-y-3">
      {result.sweep_txid && (
        <p className="flex flex-wrap items-center gap-x-2 text-sm text-muted">
          Sweep transaction <Address value={result.sweep_txid} head={10} tail={8} label="Copy sweep transaction ID" />
        </p>
      )}
      <ul className="divide-y divide-border rounded-2xl border border-border px-4">
        <Group icon={<Ban className="size-4 text-muted" />} title="Cancelled:" ks={result.cancelled} />
        {result.may_still_pay.map((m) => (
          <li key={m.k} className="flex gap-3 py-2.5 text-sm">
            <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warn" aria-hidden />
            <p>
              <span className="font-medium">
                <span className="tabular-nums">#{m.k + 1}</span> may still pay:
              </span>{" "}
              {m.reason}
            </p>
          </li>
        ))}
        {result.cancelled.length === 0 && result.may_still_pay.length === 0 && (
          <li className="py-2.5 text-sm text-muted">Nothing was left to cancel.</li>
        )}
      </ul>
      {result.may_still_pay.length > 0 && (
        <p className="text-sm text-muted text-pretty">Once the keeper has broadcast an installment, cancel cannot stop it.</p>
      )}
    </div>
  );
}
