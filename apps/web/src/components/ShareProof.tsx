import { useEffect, useRef, useState } from "react";
import { Eye, EyeOff, FileCheck2 } from "lucide-react";
import type { OrderDetail } from "@promissory/client";
import { reservations } from "../api";
import { useCopy } from "../hooks";
import { Button, Card, Notice } from "./ui";

const LEARNS = [
  "That this order's payments are funded: how many notes are reserved, and their value.",
  "Which payments have paid, and which are still reserved.",
  "If you cancel: that the remaining notes were spent without paying them.",
  "The keeper's state for each payment.",
];

const NEVER = [
  "Your treasury balance, or any other order.",
  "Where a cancel sends the funds: the sweep carries no outgoing viewing key.",
  "Anything that lets them spend. The key only views this one order's account.",
];

/** Opt-in: export a viewing key scoped to this order for the payee. */
export function ShareProof({ order }: { order: OrderDetail }) {
  const ref = useRef<HTMLDialogElement>(null);
  const [open, setOpen] = useState(false);
  const [proof, setProof] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [copied, copy] = useCopy();
  const armed = order.installments.some((i) => i.blob_id);

  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) d.showModal();
    else if (!open && d.open) d.close();
  }, [open]);

  async function create() {
    setBusy(true);
    setError(undefined);
    try {
      setProof(await reservations.exportProof(order.id));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <Card>
        <div className="space-y-3 px-5 py-5 sm:px-6">
          <h2 className="text-base font-bold">Proof of reservation</h2>
          <p className="text-sm text-muted text-pretty">
            Optional. Lets the payee check from the chain that the next payments are funded, like a salary guarantee. Nothing is
            shared unless you copy it.
          </p>
          <Button
            className="w-full"
            onClick={() => setOpen(true)}
            disabled={!armed}
            title={armed ? undefined : "Available once the order is armed"}
          >
            <FileCheck2 className="size-4" aria-hidden />
            Share proof…
          </Button>
        </div>
      </Card>

      <dialog
        ref={ref}
        aria-labelledby="proof-title"
        onCancel={() => setOpen(false)}
        onClose={() => setOpen(false)}
        className="m-auto w-[calc(100%-2rem)] max-w-lg rounded-3xl border border-border bg-surface p-0 text-fg shadow-2xl"
      >
        <div className="anim-in px-6 py-6">
          <h2 id="proof-title" className="font-display text-xl font-extrabold tracking-tight">
            Share proof of reservation for order #{order.id}?
          </h2>
          <p className="mt-2 text-sm text-muted text-pretty">
            The proof is a viewing key for this order's reserved account only, with the order's terms and keeper blob IDs. Anyone
            holding it can watch this order until it ends, so send it only to the payee.
          </p>

          <div className="mt-4 grid gap-3 sm:grid-cols-2">
            <div className="rounded-2xl border border-border px-4 py-3.5">
              <p className="flex items-center gap-1.5 text-sm font-medium">
                <Eye className="size-4 text-accent" aria-hidden />
                The payee will see
              </p>
              <ul className="mt-2 list-disc space-y-1.5 pl-4 text-sm text-muted marker:text-border-strong">
                {LEARNS.map((t) => (
                  <li key={t}>{t}</li>
                ))}
              </ul>
            </div>
            <div className="rounded-2xl border border-border px-4 py-3.5">
              <p className="flex items-center gap-1.5 text-sm font-medium">
                <EyeOff className="size-4 text-muted" aria-hidden />
                They will never see
              </p>
              <ul className="mt-2 list-disc space-y-1.5 pl-4 text-sm text-muted marker:text-border-strong">
                {NEVER.map((t) => (
                  <li key={t}>{t}</li>
                ))}
              </ul>
            </div>
          </div>

          {proof && (
            <div className="mt-4 space-y-2">
              <label htmlFor="proof-text" className="text-sm font-medium">
                Proof for the payee
              </label>
              <textarea
                id="proof-text"
                readOnly
                rows={4}
                value={proof}
                onFocus={(e) => e.currentTarget.select()}
                className="block w-full resize-none rounded-xl border border-border-strong bg-surface-2 px-3.5 py-2.5 font-mono text-xs break-all text-fg"
              />
              <p className="text-xs text-muted">
                They paste it on the <strong className="font-medium text-fg">Payee status</strong> page of their own Promissory app.
              </p>
            </div>
          )}
          {error && (
            <div className="mt-3">
              <Notice tone="danger">
                <strong>Could not create the proof.</strong> {error}
              </Notice>
            </div>
          )}

          <div className="mt-5 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
            <Button onClick={() => setOpen(false)} autoFocus>
              {proof ? "Done" : "Not now"}
            </Button>
            {proof ? (
              <Button variant="primary" onClick={() => void copy(proof)}>
                {copied ? "Copied" : "Copy proof"}
              </Button>
            ) : (
              <Button variant="primary" onClick={() => void create()} disabled={busy}>
                {busy ? "Creating…" : "Create proof"}
              </Button>
            )}
          </div>
        </div>
      </dialog>
    </>
  );
}
