import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import { BadgeCheck, CircleX, ReceiptText } from "lucide-react";
import { decodeReceipt, type Receipt, type VerifiedReceipt } from "@promissory/client";
import { api } from "../api";
import { Address, Button, Card, cx, Notice, PageHeader, Zec } from "../components/ui";

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid gap-1 sm:grid-cols-[9rem_minmax(0,1fr)] sm:gap-4">
      <dt className="text-sm text-fg/80">{label}</dt>
      <dd className="min-w-0 text-sm">{children}</dd>
    </div>
  );
}

/** Public: check a payment receipt against the chain. Uses no keys. */
export function ReceiptVerify() {
  const [text, setText] = useState(() => decodeURIComponent(window.location.hash.slice(1)));
  const [claim, setClaim] = useState<Receipt>();
  const [result, setResult] = useState<VerifiedReceipt>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);

  async function verify(e?: FormEvent) {
    e?.preventDefault();
    setResult(undefined);
    setError(undefined);
    let r: Receipt;
    try {
      r = decodeReceipt(text);
    } catch (err) {
      setClaim(undefined);
      setError(err instanceof Error ? err.message : String(err));
      return;
    }
    setClaim(r);
    setBusy(true);
    try {
      setResult(await api.verifyReceipt(text));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  // A shared link carries the receipt in the fragment, which never reaches a server.
  useEffect(() => {
    if (text.startsWith("promissory-receipt:")) void verify();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div>
      <PageHeader
        title="Check a payment receipt"
        subtitle="A receipt proves one shielded payment: how much, to which address, in which transaction. It reveals nothing else."
      />
      <Card className="mb-6">
        <form className="space-y-3 px-5 py-5 sm:px-6" onSubmit={(e) => void verify(e)}>
          <label htmlFor="receipt" className="text-sm font-semibold">
            Receipt
          </label>
          <textarea
            id="receipt"
            rows={4}
            value={text}
            onChange={(e) => setText(e.currentTarget.value)}
            placeholder="promissory-receipt:v1:…"
            spellCheck={false}
            className={cx(
              "block w-full resize-y rounded-xl border border-border-strong bg-surface px-3.5 py-2.5 font-mono text-xs break-all text-fg",
              "focus:border-accent focus:outline-none focus:ring-2 focus:ring-ring/30",
            )}
          />
          <div className="flex justify-end">
            <Button type="submit" variant="primary" disabled={busy || text.trim() === ""}>
              <ReceiptText className="size-4" aria-hidden />
              {busy ? "Checking the chain…" : "Check receipt"}
            </Button>
          </div>
        </form>
      </Card>

      {result && (
        <section className="mb-6 rounded-[28px] p-6 sm:p-8" style={{ background: "var(--hero)" }}>
          <h2 className="flex items-center gap-2 text-base font-bold text-ok">
            <BadgeCheck className="size-5" aria-hidden />
            Valid: this payment is on chain
          </h2>
          <dl className="mt-4 space-y-3" aria-live="polite">
            <div>
              <dt className="text-sm text-fg/80">Paid</dt>
              <dd className="font-display text-4xl font-extrabold tracking-tight sm:text-5xl">
                <Zec zat={result.value_zat} />
              </dd>
            </div>
            <Row label="To">
              <Address value={result.payee} head={14} tail={10} label="Copy payee address" />
            </Row>
            <Row label="Transaction">
              <Address value={result.txid} head={12} tail={10} label="Copy transaction ID" />
              <span className="block text-xs text-fg/80">
                {result.pool} action {result.action_index}, mined at block {result.height.toLocaleString()}
              </span>
            </Row>
            {result.memo && (
              <Row label="Memo">
                <span className="break-words">“{result.memo}”</span>
              </Row>
            )}
          </dl>
          <p className="mt-5 border-t border-fg/10 pt-4 text-xs text-fg/80 text-pretty">
            Checked by decrypting only this output with the key in the receipt and matching it against the note commitment
            on chain. It shows the payment was made; a receipt is a bearer string, so it does not prove who is showing it.
          </p>
        </section>
      )}
      {error && (
        <Notice tone="danger" icon={<CircleX className="size-4" aria-hidden />}>
          <strong>Not valid.</strong> {error}
          {claim && (
            <span className="mt-1 block text-xs">
              It claims {claim.value_zat.toLocaleString()} zat to {claim.payee.slice(0, 16)}… in {claim.txid.slice(0, 12)}….
            </span>
          )}
        </Notice>
      )}
    </div>
  );
}
