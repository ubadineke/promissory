import { useMemo, useState } from "react";
import { Link } from "react-router";
import { encodeSubscribeLink, formatCadence, formatZec, type Network } from "@promissory/client";
import { QrCode } from "../components/QrCode";
import { ScheduleFields } from "../components/ScheduleFields";
import { Card, CardHeader, CopyButton, cx, describedBy, Field, inputClass, PageHeader } from "../components/ui";
import { useInfo } from "../info";
import { checkSchedule, emptySchedule, visibleErrors, type ScheduleForm } from "../lib/schedule";

export function SubscribeBuilder() {
  const info = useInfo();
  const [form, setForm] = useState<ScheduleForm>(emptySchedule);
  const [chosenNet, setChosenNet] = useState<Network>();
  const network: Network = chosenNet ?? info.data?.network ?? "main";

  const checked = useMemo(() => checkSchedule(form, network), [form, network]);
  const errors = visibleErrors(form, checked.errors, false);
  const link = checked.params ? encodeSubscribeLink(`${window.location.origin}/s`, checked.params) : null;
  const preview = link ? link.slice(link.indexOf("#")) : null;

  return (
    <div>
      <PageHeader
        title="Subscribe link"
        subtitle="For payees: get paid on a schedule. Share the link or QR; payers set it up in their own Promissory app."
      />
      <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_20rem]">
        <Card className="space-y-6 px-5 py-5 sm:px-6 sm:py-6">
          <fieldset>
            <legend className="mb-1.5 text-sm font-semibold">Network</legend>
            <div className="inline-grid grid-cols-2 gap-1.5">
              {(["main", "regtest"] as const).map((n) => (
                <label
                  key={n}
                  className={cx(
                    "flex h-11 cursor-pointer items-center justify-center rounded-xl border px-4 text-sm transition-colors",
                    "has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring has-[:focus-visible]:ring-offset-2 has-[:focus-visible]:ring-offset-bg",
                    network === n ? "border-accent bg-accent-soft font-medium text-accent" : "border-border-strong bg-surface hover:bg-surface-2",
                  )}
                >
                  <input type="radio" name="sub-net" className="sr-only" checked={network === n} onChange={() => setChosenNet(n)} />
                  {n === "main" ? "Mainnet" : "Regtest"}
                </label>
              ))}
            </div>
          </fieldset>

          <ScheduleFields
            form={form}
            set={(patch) => setForm((f) => ({ ...f, ...patch }))}
            errors={errors}
            idPrefix="sub"
            payeeLabel="Your address"
          />

          <Field
            id="sub-keeper"
            label={
              <>
                Suggested keeper <span className="font-normal text-muted">(optional)</span>
              </>
            }
            hint="The payer can use a different one."
            error={errors.keeper}
          >
            <input
              id="sub-keeper"
              type="url"
              className={cx(inputClass, "h-11 font-mono")}
              placeholder="https://keeper.example"
              autoComplete="url"
              spellCheck={false}
              value={form.keeper}
              onChange={(e) => setForm((f) => ({ ...f, keeper: e.target.value }))}
              {...describedBy("sub-keeper", errors.keeper)}
            />
          </Field>
        </Card>

        <aside className="lg:sticky lg:top-6 lg:self-start">
          <Card>
            <CardHeader title="Your link" />
            {link && checked.params ? (
              <div className="space-y-4 px-5 py-5 sm:px-6">
                <div className="mx-auto w-fit rounded-2xl border border-border bg-white p-2.5 shadow-sm">
                  <QrCode value={link} label="QR code of the subscribe link" size={232} />
                </div>
                <p className="text-sm">
                  {formatZec(checked.params.amountZat, { unit: true })} {formatCadence(checked.params.everySecs)},{" "}
                  {checked.params.count} times
                </p>
                <div className="flex items-start gap-1 rounded-2xl border border-border bg-surface-2 py-1 pr-1 pl-3">
                  <code className="min-w-0 flex-1 py-1.5 font-mono text-xs break-all text-muted">{link}</code>
                  <CopyButton text={link} label="Copy subscribe link" />
                </div>
                <Link
                  to={`/s${preview}`}
                  className="inline-flex h-9 items-center rounded-xl text-sm font-medium text-dawn-ink underline-offset-2 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  Preview what payers see
                </Link>
                <p className="text-xs text-muted text-pretty">
                  Everything after “#” stays in the browser: the server hosting the page never receives your address or amounts.
                </p>
              </div>
            ) : (
              <p className="px-5 py-6 text-sm text-muted sm:px-6">Fill in your address, amount and schedule to get a link.</p>
            )}
          </Card>
        </aside>
      </div>
    </div>
  );
}
