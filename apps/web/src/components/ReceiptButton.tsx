import { useEffect, useRef, useState } from "react";
import { Link } from "react-router";
import { ReceiptText } from "lucide-react";
import type { MadeReceipt } from "@promissory/client";
import { api } from "../api";
import { useCopy } from "../hooks";
import { Button, Notice } from "./ui";

/** "Get receipt" for a paid installment: a string that proves this one payment and nothing else. */
export function ReceiptButton({ orderId, k }: { orderId: number; k: number }) {
  const ref = useRef<HTMLDialogElement>(null);
  const [open, setOpen] = useState(false);
  const [made, setMade] = useState<MadeReceipt>();
  const [error, setError] = useState<string>();
  const [copied, copy] = useCopy();

  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) d.showModal();
    else if (!open && d.open) d.close();
  }, [open]);

  async function get() {
    setOpen(true);
    if (made) return;
    setError(undefined);
    try {
      setMade(await api.getReceipt(orderId, k));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  const link = made ? `${window.location.origin}/receipt#${made.receipt}` : "";
  return (
    <>
      <Button size="sm" variant="ghost" className="-ml-3 h-8" onClick={() => void get()}>
        <ReceiptText className="size-4" aria-hidden />
        Get receipt
      </Button>
      <dialog
        ref={ref}
        aria-labelledby={`receipt-title-${k}`}
        onCancel={() => setOpen(false)}
        onClose={() => setOpen(false)}
        className="m-auto w-[calc(100%-2rem)] max-w-lg rounded-3xl border border-border bg-surface p-0 text-fg shadow-2xl"
      >
        <div className="anim-in px-6 py-6">
          <h2 id={`receipt-title-${k}`} className="font-display text-xl font-extrabold tracking-tight">
            Receipt for payment {k + 1}
          </h2>
          <p className="mt-2 text-sm text-muted text-pretty">
            Proves you paid this amount to this address in this transaction. It opens this one payment and nothing else:
            not your other payments, not your balance. Anyone can check it on the chain.
          </p>
          {made ? (
            <div className="mt-4 space-y-2">
              <label htmlFor={`receipt-text-${k}`} className="text-sm font-medium">
                Receipt
              </label>
              <textarea
                id={`receipt-text-${k}`}
                readOnly
                rows={4}
                value={made.receipt}
                onFocus={(e) => e.currentTarget.select()}
                className="block w-full resize-none rounded-xl border border-border-strong bg-surface-2 px-3.5 py-2.5 font-mono text-xs break-all text-fg"
              />
              <p className="text-xs text-muted">
                Check it on the{" "}
                <Link to={`/receipt#${made.receipt}`} className="font-medium text-fg underline underline-offset-2" onClick={() => setOpen(false)}>
                  receipt page
                </Link>{" "}
                of any Promissory app. Whoever holds it can show it on, so share it only with who needs it.
              </p>
            </div>
          ) : error ? (
            <div className="mt-4">
              <Notice tone="danger">
                <strong>Could not make the receipt.</strong> {error}
              </Notice>
            </div>
          ) : (
            <p className="mt-4 text-sm text-muted" aria-live="polite">
              Finding the payment on chain…
            </p>
          )}
          <div className="mt-5 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
            <Button onClick={() => setOpen(false)} autoFocus>
              Done
            </Button>
            {made && (
              <>
                <Button onClick={() => void copy(link)}>Copy link</Button>
                <Button variant="primary" onClick={() => void copy(made.receipt)}>
                  {copied ? "Copied" : "Copy receipt"}
                </Button>
              </>
            )}
          </div>
        </div>
      </dialog>
    </>
  );
}
