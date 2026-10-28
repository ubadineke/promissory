import { useMemo, useRef, useState, type ChangeEvent, type FormEvent } from "react";
import { Link, useNavigate } from "react-router";
import { ChevronRight, CircleAlert, FileUp } from "lucide-react";
import {
  ApiError,
  formatCadence,
  formatZec,
  parsePayrollCsv,
  truncateMiddle,
  type Batch,
  type PayrollRowError,
} from "@promissory/client";
import { api } from "../api";
import { ErrorPanel } from "../components/ErrorPanel";
import { StatusChip } from "../components/StatusChip";
import { Button, Card, CardHeader, cx, describedBy, Field, inputClass, Notice, PageHeader, Skeleton, Zec } from "../components/ui";
import { usePoll } from "../hooks";
import { useInfo } from "../info";
import { Naira, RateNote } from "../lib/fiat";
import { formatDateTime, fromLocalInput, toLocalInput } from "../lib/time";

const SAMPLE_HEADER = "name,ua,amount,cadence,count";

function BatchRow({ batch }: { batch: Batch }) {
  const pct = batch.total_zat > 0 ? Math.min(100, (batch.paid_zat / batch.total_zat) * 100) : 0;
  return (
    <li>
      <Link
        to={`/payroll/${batch.id}`}
        className="group flex items-center gap-4 px-5 py-4 transition-colors hover:bg-surface-2/60 focus-visible:bg-surface-2 focus-visible:outline-none sm:px-6"
      >
        <div className="min-w-0 flex-1 space-y-1.5">
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <span className="font-medium">{batch.name || `Batch #${batch.id}`}</span>
            <StatusChip status={batch.status} />
          </div>
          <p className="truncate text-sm text-muted">
            #{batch.id} · {batch.order_count} {batch.order_count === 1 ? "payee" : "payees"} · first {formatDateTime(batch.first_due)}
          </p>
          <div className="flex items-center gap-3 text-xs text-muted">
            <div className="h-1.5 w-24 overflow-hidden rounded-full bg-surface-2" aria-hidden>
              <div className="h-full rounded-full bg-accent" style={{ width: `${pct}%` }} />
            </div>
            <span className="tabular-nums">
              {formatZec(batch.paid_zat)} of {formatZec(batch.total_zat)} ZEC paid
            </span>
          </div>
        </div>
        <ChevronRight className="size-5 shrink-0 text-muted transition-transform group-hover:translate-x-0.5" aria-hidden />
      </Link>
    </li>
  );
}

function Preview({ csv, errors, rows }: { csv: string; errors: PayrollRowError[]; rows: ReturnType<typeof parsePayrollCsv>["rows"] }) {
  const lines = csv.split(/\r\n|\n|\r/);
  const byLine = new Map<number, string[]>();
  for (const e of errors) byLine.set(e.line, [...(byLine.get(e.line) ?? []), e.message]);
  const entries = [
    ...rows.map((r) => ({ line: r.line, row: r, errs: undefined as string[] | undefined })),
    ...[...byLine.entries()].filter(([l]) => l > 0).map(([line, errs]) => ({ line, row: undefined, errs })),
  ].sort((a, b) => a.line - b.line);
  const fileErrors = byLine.get(0);

  return (
    <div className="overflow-x-auto">
      {fileErrors && (
        <div className="px-5 pt-4 sm:px-6">
          <Notice tone="danger">{fileErrors.join(" ")}</Notice>
        </div>
      )}
      {entries.length > 0 && (
        <table className="w-full min-w-[34rem] text-left text-sm">
          <thead className="text-sm text-muted">
            <tr className="border-b border-border">
              <th scope="col" className="py-2.5 pl-5 pr-2 font-medium sm:pl-6">Row</th>
              <th scope="col" className="px-2 py-2.5 font-medium">Name</th>
              <th scope="col" className="px-2 py-2.5 font-medium">Address</th>
              <th scope="col" className="px-2 py-2.5 text-right font-medium">Each, ZEC</th>
              <th scope="col" className="px-2 py-2.5 font-medium">Schedule</th>
              <th scope="col" className="py-2.5 pl-2 pr-5 text-right font-medium sm:pr-6">Total, ZEC</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border">
            {entries.map(({ line, row, errs }) =>
              row ? (
                <tr key={line}>
                  <td className="py-2.5 pl-5 pr-2 tabular-nums text-muted sm:pl-6">{line}</td>
                  <td className="max-w-40 truncate px-2 py-2.5 font-medium">{row.name}</td>
                  <td className="px-2 py-2.5 font-mono text-[0.92em]" title={row.ua}>
                    {truncateMiddle(row.ua, 9, 4)}
                  </td>
                  <td className="px-2 py-2.5 text-right">
                    <Zec zat={row.amountZat} unit={false} />
                  </td>
                  <td className="whitespace-nowrap px-2 py-2.5 text-muted">
                    {row.count} × {formatCadence(row.everySecs).replace(/^every /, "")}
                  </td>
                  <td className="py-2.5 pl-2 pr-5 text-right sm:pr-6">
                    <Zec zat={row.amountZat * row.count} unit={false} />
                  </td>
                </tr>
              ) : (
                <tr key={line} className="bg-danger-soft/40">
                  <td className="py-2.5 pl-5 pr-2 align-top tabular-nums text-danger sm:pl-6">{line}</td>
                  <td colSpan={5} className="max-w-0 py-2.5 pl-2 pr-5 sm:pr-6">
                    <p className="truncate font-mono text-xs text-muted" title={lines[line - 1]}>
                      {lines[line - 1]}
                    </p>
                    <ul className="mt-1 space-y-0.5 text-danger">
                      {errs!.map((m, i) => (
                        <li key={i} className="flex items-start gap-1.5">
                          <CircleAlert className="mt-0.5 size-3.5 shrink-0" aria-hidden />
                          {m}
                        </li>
                      ))}
                    </ul>
                  </td>
                </tr>
              ),
            )}
          </tbody>
        </table>
      )}
    </div>
  );
}

function Import() {
  const info = useInfo();
  const navigate = useNavigate();
  const fileRef = useRef<HTMLInputElement>(null);
  const [csv, setCsv] = useState("");
  const [name, setName] = useState("");
  const [first, setFirst] = useState(() => toLocalInput(Math.floor(Date.now() / 1000) + 900));
  const [busy, setBusy] = useState(false);
  const [apiError, setApiError] = useState<string>();
  const [serverRows, setServerRows] = useState<PayrollRowError[]>();

  const network = info.data?.network;
  const parsed = useMemo(() => (network && csv.trim() !== "" ? parsePayrollCsv(csv, network) : null), [csv, network]);
  const firstDue = fromLocalInput(first);
  const firstError =
    firstDue === null ? "Pick a date and time" : firstDue <= Math.floor(Date.now() / 1000) + 60 ? "Pick a time at least a minute from now" : undefined;
  const ok = parsed !== null && parsed.errors.length === 0 && !firstError;

  async function onFile(e: ChangeEvent<HTMLInputElement>) {
    const f = e.target.files?.[0];
    if (f) setCsv(await f.text());
    e.target.value = "";
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!ok || firstDue === null) return;
    setBusy(true);
    setApiError(undefined);
    setServerRows(undefined);
    try {
      const created = await api.createBatch({ csv, first_due: firstDue, ...(name.trim() ? { name: name.trim() } : {}) });
      navigate(`/payroll/${created.batch_id}`);
    } catch (err) {
      setApiError(err instanceof Error ? err.message : String(err));
      const rows = err instanceof ApiError ? (err.body as { rows?: PayrollRowError[] } | undefined)?.rows : undefined;
      if (rows) setServerRows(rows);
      setBusy(false);
    }
  }

  return (
    <form onSubmit={(e) => void submit(e)} noValidate className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_18rem]">
      <div className="min-w-0 space-y-6">
        <Card className="space-y-5 px-5 py-5 sm:px-6 sm:py-6">
          <div className="grid gap-5 sm:grid-cols-2">
            <Field id="pay-name" label={<>Batch name <span className="font-normal text-muted">(optional)</span></>} hint="Also the memo each payee sees.">
              <input
                id="pay-name"
                className={cx(inputClass, "h-11")}
                autoComplete="off"
                placeholder="Salaries"
                maxLength={100}
                value={name}
                onChange={(e) => setName(e.target.value)}
                {...describedBy("pay-name", undefined)}
              />
            </Field>
            <Field id="pay-first" label="First payment" hint="Every order starts here. Your local time." error={firstError}>
              <input
                id="pay-first"
                type="datetime-local"
                className={cx(inputClass, "h-11 tabular-nums")}
                value={first}
                onChange={(e) => setFirst(e.target.value)}
                {...describedBy("pay-first", firstError)}
              />
            </Field>
          </div>
          <Field
            id="pay-csv"
            label="Payees (CSV)"
            hint={
              <>
                One row per payee: <code className="font-mono text-fg">{SAMPLE_HEADER}</code>. Amount in ZEC; cadence{" "}
                <code className="font-mono">weekly</code>, <code className="font-mono">biweekly</code>, <code className="font-mono">monthly</code>{" "}
                (30 days) or seconds. Header row optional.
              </>
            }
            aside={
              <>
                <input ref={fileRef} type="file" accept=".csv,text/csv,text/plain" className="sr-only" onChange={(e) => void onFile(e)} tabIndex={-1} />
                <Button size="sm" variant="ghost" className="-my-1.5" onClick={() => fileRef.current?.click()}>
                  <FileUp className="size-4" aria-hidden />
                  Upload file
                </Button>
              </>
            }
          >
            <textarea
              id="pay-csv"
              rows={7}
              className={cx(inputClass, "py-2 font-mono text-xs leading-5")}
              placeholder={`${SAMPLE_HEADER}\nAda,u1…,0.25,monthly,12\nTunde,u1…,0.1,weekly,52`}
              autoComplete="off"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              value={csv}
              onChange={(e) => {
                setCsv(e.target.value);
                setServerRows(undefined);
              }}
              {...describedBy("pay-csv", undefined)}
            />
          </Field>
        </Card>

        {parsed && (
          <Card className="overflow-hidden">
            <CardHeader
              title="Preview"
              action={
                parsed.errors.length > 0 ? (
                  <span className="text-sm font-medium text-danger">
                    {parsed.errors.length} {parsed.errors.length === 1 ? "problem" : "problems"}
                  </span>
                ) : (
                  <span className="text-sm text-ok">All rows valid</span>
                )
              }
            >
              {parsed.rows.length} valid {parsed.rows.length === 1 ? "row" : "rows"}. Nothing is created until every row is valid.
            </CardHeader>
            <Preview csv={csv} rows={parsed.rows} errors={serverRows ?? parsed.errors} />
          </Card>
        )}
      </div>

      <aside className="space-y-4 lg:sticky lg:top-6 lg:self-start">
        <Card className="px-5 py-5 sm:px-6">
          <h2 className="text-base font-bold">Summary</h2>
          {parsed && parsed.rows.length > 0 ? (
            <dl className="mt-3 space-y-2.5 text-sm">
              <div className="flex justify-between gap-3">
                <dt className="text-muted">Payees</dt>
                <dd className="tabular-nums">{parsed.rows.length}</dd>
              </div>
              <div className="flex justify-between gap-3">
                <dt className="text-muted">Payments</dt>
                <dd className="tabular-nums">{parsed.rows.reduce((s, r) => s + r.count, 0)}</dd>
              </div>
              <div className="flex justify-between gap-3">
                <dt className="text-muted">All payments</dt>
                <dd className="text-right">
                  <Zec zat={parsed.totalZat} />
                  <Naira zat={parsed.totalZat} className="block text-xs text-muted" />
                </dd>
              </div>
              <div className="flex justify-between gap-3 border-t border-border pt-2.5 font-medium">
                <dt>Fund once</dt>
                <dd className="tabular-nums">
                  <Zec zat={parsed.fundingZat} />
                </dd>
              </div>
              <p className="text-xs text-muted">Includes network fees for every payment and split.</p>
              <RateNote />
            </dl>
          ) : (
            <p className="mt-2 text-sm text-muted">Paste or upload a CSV to preview the batch.</p>
          )}
        </Card>
        {apiError && (
          <Notice tone="danger">
            <strong>Couldn’t create the batch.</strong> {apiError}
          </Notice>
        )}
        <Button type="submit" variant="primary" className="w-full" disabled={!ok || busy}>
          {busy ? "Creating orders…" : parsed && parsed.rows.length > 0 ? `Create ${parsed.rows.length} standing orders` : "Create batch"}
        </Button>
        <p className="text-xs text-muted text-pretty">
          Nothing is signed or sent yet. Next you get one QR code that funds every order; each one starts as soon as the funds arrive.
        </p>
      </aside>
    </form>
  );
}

export function Payroll() {
  const info = useInfo();
  const batches = usePoll(() => api.listBatches(), 10_000);

  return (
    <div>
      <PageHeader title="Payroll" subtitle="One CSV, one standing order per payee, funded with a single payment." />
      {batches.data && batches.data.length > 0 && (
        <>
          <h2 className="mb-3 font-display text-lg font-bold">Batches</h2>
          <Card className="mb-8">
            <ul className="divide-y divide-border">
              {batches.data.map((b) => (
                <BatchRow key={b.id} batch={b} />
              ))}
            </ul>
          </Card>
        </>
      )}
      <h2 className="mb-3 font-display text-lg font-bold">New batch</h2>
      {info.data ? (
        <Import />
      ) : info.error ? (
        <ErrorPanel error={info.error} onRetry={() => void info.refresh()} />
      ) : (
        <div className="space-y-4" aria-busy="true">
          <Skeleton className="h-64 w-full" />
        </div>
      )}
    </div>
  );
}
