import { useMemo, useState, type ReactNode } from "react";
import { CircleAlert, EyeOff, ShieldCheck, TriangleAlert } from "lucide-react";
import {
  decodeReservationProof,
  formatCadence,
  formatCountdown,
  formatZec,
  type KeeperBlobView,
  type ReservationProof,
  type ReservationReport,
  type ReservedNoteState,
} from "@promissory/client";
import { reservations } from "../api";
import { Address, Button, Card, CardHeader, Chip, cx, Dial, Field, inputClass, Notice, PageHeader, Zec } from "../components/ui";
import { useNow } from "../hooks";
import { formatDateTime, hostOf } from "../lib/time";

type Mode = "proof" | "keeper";

const NOTE_CHIP: Record<ReservedNoteState, { tone: "ok" | "accent" | "danger" | "neutral"; label: string }> = {
  reserved: { tone: "accent", label: "Reserved" },
  paid: { tone: "ok", label: "Paid to you" },
  swept: { tone: "danger", label: "Spent, not to you" },
  spent_unknown: { tone: "neutral", label: "Spent" },
};

const KEEPER_TONE: Record<string, "ok" | "accent" | "warn" | "danger" | "neutral"> = {
  locked: "accent",
  released: "warn",
  broadcast: "warn",
  mined: "ok",
  cancelled: "danger",
  expired: "danger",
  failed: "danger",
  deleted: "neutral",
};

function Stat({ label, children, hint }: { label: string; children: ReactNode; hint?: ReactNode }) {
  return (
    <div className="min-w-0">
      <dt className="text-sm font-medium text-muted">{label}</dt>
      <dd className="mt-1 truncate text-lg font-bold">{children}</dd>
      {hint && <dd className="mt-0.5 text-xs text-muted">{hint}</dd>}
    </div>
  );
}

function Segmented({ mode, onChange }: { mode: Mode; onChange: (m: Mode) => void }) {
  const opts: { value: Mode; label: string }[] = [
    { value: "proof", label: "Proof of reservation" },
    { value: "keeper", label: "Keeper only" },
  ];
  return (
    <div role="tablist" aria-label="How to check" className="mb-6 inline-flex rounded-2xl border border-border bg-surface-2 p-1">
      {opts.map((o) => (
        <button
          key={o.value}
          role="tab"
          type="button"
          aria-selected={mode === o.value}
          onClick={() => onChange(o.value)}
          className={cx(
            "h-9 rounded-xl px-3.5 text-sm whitespace-nowrap transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            mode === o.value ? "bg-surface font-medium text-fg shadow-sm" : "text-muted hover:text-fg",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function KeeperTable({ blobs, now }: { blobs: KeeperBlobView[]; now: number }) {
  if (blobs.length === 0) return <p className="px-5 py-4 text-sm text-muted sm:px-6">No blobs at the keeper yet.</p>;
  return (
    <ul className="divide-y divide-border">
      {blobs.map((b, i) => (
        <li key={b.blob_id} className="flex flex-wrap items-center gap-x-4 gap-y-1 px-5 py-3 text-sm sm:px-6">
          <span className="w-8 font-medium tabular-nums">#{(b.k ?? i) + 1}</span>
          <span className="min-w-0 flex-1">
            <Address value={b.blob_id} head={8} tail={6} label="Copy blob ID" className="text-muted" />
          </span>
          <span className="text-muted tabular-nums">
            {b.mined_height
              ? `mined at ${b.mined_height}`
              : b.release_unix && (b.state === "locked" || b.state === null)
                ? b.release_unix > now
                  ? `opens in ${formatCountdown(b.release_unix - now)}`
                  : `opened ${formatDateTime(b.release_unix)}`
                : ""}
          </span>
          {b.state ? <Chip tone={KEEPER_TONE[b.state] ?? "neutral"}>{b.state}</Chip> : <Chip tone="danger">unreachable</Chip>}
        </li>
      ))}
    </ul>
  );
}

function Terms({
  p,
  className = "text-sm text-muted",
}: {
  p: Pick<ReservationProof, "amount_zat" | "every_secs" | "count" | "payee" | "first_due" | "memo" | "keeper">;
  className?: string;
}) {
  return (
    <p className={cx("flex flex-wrap items-center gap-x-1.5", className)}>
      <span>
        <Zec zat={p.amount_zat} className="font-medium text-fg" /> {formatCadence(p.every_secs)}, {p.count} payments from{" "}
        {formatDateTime(p.first_due)}, to
      </span>
      <Address value={p.payee} label="Copy payee address" className="text-fg" />
      {p.memo && <span>· “{p.memo}”</span>}
      <span>· keeper {hostOf(p.keeper)}</span>
    </p>
  );
}

function ReportView({ r, now }: { r: ReservationReport; now: number }) {
  const t = r.terms;
  const next = r.next_release_unix && !r.cancelled_by_payer ? r.next_release_unix : null;
  return (
    <div className="space-y-6">
      <section className="flex flex-wrap items-center gap-6 rounded-[28px] p-6 sm:p-8" style={{ background: "var(--hero)" }}>
        <div className="min-w-0 flex-[1_1_20rem]">
          <h2 className="font-display text-[1.9rem] leading-[1.05] font-extrabold tracking-[-0.03em] text-balance sm:text-[2.3rem]">
            {r.funded} of {t.count} funded, {r.paid} paid
          </h2>
          <Terms p={t} className="mt-3 text-sm text-fg/80" />
        </div>
        {next ? (
          <Dial
            label="Next release in"
            value={formatCountdown(Math.max(0, next - now))}
            progress={1 - (next - now) / Math.max(1, t.every_secs)}
            dateTime={new Date(next * 1000).toISOString()}
          />
        ) : r.paid === t.count ? (
          <Dial label="All paid" value={`${r.paid} of ${t.count}`} progress={1} />
        ) : null}
      </section>

      {r.cancelled_by_payer ? (
        <Notice tone="danger" icon={<TriangleAlert className="size-4" aria-hidden />}>
          <strong>Cancelled by payer.</strong> {r.swept} reserved payment{r.swept === 1 ? " was" : "s were"} spent without paying
          you (<Zec zat={r.swept_zat} />). Where the funds went is not visible to you: a cancel sweep carries no outgoing viewing
          key, by design, so this proof never reveals the payer's other funds.
        </Notice>
      ) : r.funded === t.count && r.matches_terms ? (
        <Notice tone="ok" icon={<ShieldCheck className="size-4" aria-hidden />}>
          <strong>Funded as promised.</strong> All {t.count} payments were reserved on chain, each worth{" "}
          {formatZec(t.amount_zat)} ZEC plus its network fee.
        </Notice>
      ) : null}

      <Card>
        <dl className="grid grid-cols-2 gap-4 px-5 py-5 sm:grid-cols-3 sm:px-6">
          <Stat label="Funded" hint={<Zec zat={r.funded_zat} />}>
            <span className="tabular-nums">
              {r.funded} <span className="text-sm font-normal text-muted">of {t.count}</span>
            </span>
          </Stat>
          <Stat label="Paid to you" hint={<Zec zat={r.paid_zat} />}>
            <span className="tabular-nums">{r.paid}</span>
          </Stat>
          <Stat label="Still reserved" hint={<Zec zat={r.reserved_zat} />}>
            <span className="tabular-nums">{r.reserved}</span>
          </Stat>
        </dl>
      </Card>

      {r.warnings.filter((w) => !/cancelled/.test(w)).length > 0 && (
        <Notice tone="warn" icon={<CircleAlert className="size-4" aria-hidden />}>
          {r.warnings
            .filter((w) => !/cancelled/.test(w))
            .map((w) => (
              <p key={w}>{w}</p>
            ))}
        </Notice>
      )}

      <Card>
        <CardHeader title="Reserved notes">One note per payment, in the order's own account. Scanned to height {r.synced_height}.</CardHeader>
        {r.notes.length === 0 ? (
          <p className="px-5 py-4 text-sm text-muted sm:px-6">No notes yet. The order may not be funded.</p>
        ) : (
          <ul className="divide-y divide-border">
            {r.notes.map((n) => (
              <li key={`${n.txid}:${n.action_index}`} className="flex flex-wrap items-center gap-x-4 gap-y-1 px-5 py-3 text-sm sm:px-6">
                <Zec zat={n.value_zat} className="w-28 font-medium" />
                <span className="min-w-0 flex-1 text-muted">
                  {n.state === "paid" && n.spent_txid ? (
                    <>
                      Paid <Zec zat={n.paid_zat ?? 0} /> in <Address value={n.spent_txid} head={8} tail={6} label="Copy transaction ID" />
                      {n.paid_memo && <span className="ml-1">“{n.paid_memo}”</span>}
                    </>
                  ) : n.spent_txid ? (
                    <>
                      Spent in <Address value={n.spent_txid} head={8} tail={6} label="Copy transaction ID" />
                      {n.spent_height && <span className="ml-1 tabular-nums">at {n.spent_height}</span>}
                    </>
                  ) : (
                    <>Unspent since height {n.received_height ?? "—"}</>
                  )}
                </span>
                <Chip tone={NOTE_CHIP[n.state].tone}>{NOTE_CHIP[n.state].label}</Chip>
              </li>
            ))}
          </ul>
        )}
      </Card>

      <Card>
        <CardHeader title="Keeper">State of each payment at {hostOf(t.keeper)}. No keys needed.</CardHeader>
        <KeeperTable blobs={r.keeper} now={now} />
      </Card>
    </div>
  );
}

function ProofCheck({ now }: { now: number }) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [report, setReport] = useState<ReservationReport>();

  const preview = useMemo(() => {
    if (!text.trim()) return { p: undefined, err: undefined };
    try {
      return { p: decodeReservationProof(text), err: undefined };
    } catch (e) {
      return { p: undefined, err: e instanceof Error ? e.message : String(e) };
    }
  }, [text]);

  async function verify() {
    setBusy(true);
    setError(undefined);
    try {
      setReport(await reservations.verify(text));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-6">
      <Card className="px-5 py-5 sm:px-6">
        <form
          className="space-y-4"
          onSubmit={(e) => {
            e.preventDefault();
            if (preview.p) void verify();
          }}
        >
          <Field
            id="proof"
            label="Proof of reservation"
            error={preview.err}
            hint="The promissory-reserve:v1:… text the payer sent you. It only lets this app view that one order."
          >
            <textarea
              id="proof"
              rows={3}
              spellCheck={false}
              value={text}
              onChange={(e) => {
                setText(e.target.value);
                setReport(undefined);
              }}
              placeholder="promissory-reserve:v1:…"
              aria-invalid={preview.err ? true : undefined}
              aria-describedby={preview.err ? "proof-error" : "proof-hint"}
              className={cx(inputClass, "resize-y py-2 font-mono text-xs break-all")}
            />
          </Field>
          {preview.p && !report && <Terms p={preview.p} />}
          <div className="flex flex-wrap items-center gap-3">
            <Button type="submit" variant="primary" disabled={!preview.p || busy}>
              {busy ? "Scanning the chain…" : report ? "Check again" : "Verify"}
            </Button>
            {busy && <span className="text-sm text-muted">The first check scans from the order's start; later ones are quick.</span>}
          </div>
        </form>
      </Card>
      {error && (
        <Notice tone="danger" icon={<CircleAlert className="size-4" aria-hidden />}>
          <strong>Could not verify.</strong> {error}
        </Notice>
      )}
      {report && <ReportView r={report} now={now} />}
      <Card className="px-5 py-5 sm:px-6">
        <div className="flex gap-3">
          <EyeOff className="mt-0.5 size-5 shrink-0 text-muted" aria-hidden />
          <div className="space-y-2 text-sm text-muted text-pretty">
            <p>
              <strong className="font-semibold text-fg">What this check can and can't see.</strong> The key covers only this order's
              reserved account. You see each reserved note, whether it is still unspent, and whether a spend paid you: payments are
              sent with an outgoing viewing key, so their output to your address can be recognised.
            </p>
            <p>
              A cancel is different. The payer's sweep carries no outgoing viewing key, so you see that the notes were spent, never
              where they went. That keeps the payer's treasury and other orders private.
            </p>
            <p>The check runs in a separate view-only wallet inside this app. It never touches your spending keys.</p>
          </div>
        </div>
      </Card>
    </div>
  );
}

function KeeperCheck({ now }: { now: number }) {
  const [keeper, setKeeper] = useState("");
  const [ids, setIds] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [blobs, setBlobs] = useState<KeeperBlobView[]>();
  const list = ids
    .split(/[\s,]+/)
    .map((s) => s.trim())
    .filter(Boolean);

  async function check() {
    setBusy(true);
    setError(undefined);
    try {
      setBlobs(await reservations.keeperStatus(keeper.trim(), list));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  const mined = blobs?.filter((b) => b.state === "mined").length ?? 0;
  const failed = blobs?.filter((b) => b.state === "cancelled" || b.state === "failed" || b.state === "expired").length ?? 0;
  const next = blobs
    ?.filter((b) => b.state === "locked" && b.release_unix)
    .map((b) => b.release_unix as number)
    .sort((a, b) => a - b)[0];

  return (
    <div className="space-y-6">
      <Card className="px-5 py-5 sm:px-6">
        <form
          className="space-y-4"
          onSubmit={(e) => {
            e.preventDefault();
            void check();
          }}
        >
          <Field id="keeper" label="Keeper URL" hint="From the payer, or from your Subscribe link.">
            <input
              id="keeper"
              type="url"
              value={keeper}
              onChange={(e) => setKeeper(e.target.value)}
              placeholder="https://keeper.example"
              className={cx(inputClass, "h-11")}
              aria-describedby="keeper-hint"
            />
          </Field>
          <Field id="blobs" label="Blob IDs" hint="One per line, or separated by commas.">
            <textarea
              id="blobs"
              rows={3}
              spellCheck={false}
              value={ids}
              onChange={(e) => setIds(e.target.value)}
              className={cx(inputClass, "resize-y py-2 font-mono text-xs")}
              aria-describedby="blobs-hint"
            />
          </Field>
          <Button type="submit" variant="primary" disabled={busy || !keeper.trim() || list.length === 0}>
            {busy ? "Checking…" : "Check keeper"}
          </Button>
        </form>
      </Card>
      {error && (
        <Notice tone="danger" icon={<CircleAlert className="size-4" aria-hidden />}>
          <strong>Could not check.</strong> {error}
        </Notice>
      )}
      {blobs && (
        <Card>
          <CardHeader title={`${mined} of ${blobs.length} paid${failed ? `, ${failed} failed` : ""}`}>
            {next ? `Next release in ${formatCountdown(Math.max(0, next - now))}.` : "Nothing is waiting to release."} The keeper can't
            tell whether locked payments are still funded; a proof of reservation can.
          </CardHeader>
          <KeeperTable blobs={blobs} now={now} />
        </Card>
      )}
    </div>
  );
}

export function PayeeStatus() {
  const [mode, setMode] = useState<Mode>("proof");
  const now = useNow();
  return (
    <div className="max-w-3xl">
      <PageHeader
        title="Payee status"
        subtitle="Check a standing order that pays you: whether it is funded, what has paid, and whether the payer cancelled."
      />
      <Segmented mode={mode} onChange={setMode} />
      {mode === "proof" ? <ProofCheck now={now} /> : <KeeperCheck now={now} />}
    </div>
  );
}
