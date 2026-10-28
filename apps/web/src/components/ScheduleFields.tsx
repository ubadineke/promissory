import { MEMO_MAX_BYTES, utf8Length } from "@promissory/client";
import type { ScheduleField, ScheduleForm } from "../lib/schedule";
import { CadencePicker } from "./CadencePicker";
import { cx, describedBy, Field, inputClass } from "./ui";

/** Payee, amount, cadence, count and memo: the fields every schedule needs. */
export function ScheduleFields({
  form,
  set,
  errors,
  idPrefix,
  payeeLabel = "Pay to",
}: {
  form: ScheduleForm;
  set: (patch: Partial<ScheduleForm>) => void;
  errors: Partial<Record<ScheduleField, string>>;
  idPrefix: string;
  payeeLabel?: string;
}) {
  const id = (s: string) => `${idPrefix}-${s}`;
  const memoBytes = utf8Length(form.memo);
  return (
    <>
      <Field id={id("payee")} label={payeeLabel} hint="A unified address with an Orchard receiver (u1…)." error={errors.payee}>
        <textarea
          id={id("payee")}
          rows={2}
          className={cx(inputClass, "resize-none py-2 font-mono break-all")}
          placeholder="u1…"
          autoComplete="off"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          value={form.payee}
          onChange={(e) => set({ payee: e.target.value.replace(/\s+/g, "") })}
          {...describedBy(id("payee"), errors.payee)}
        />
      </Field>

      <div className="grid gap-5 sm:grid-cols-2">
        <Field id={id("amount")} label="Amount per payment" hint="Up to 8 decimals." error={errors.amount}>
          <div className="relative">
            <input
              id={id("amount")}
              className={cx(inputClass, "h-11 pr-14 tabular-nums")}
              inputMode="decimal"
              autoComplete="off"
              placeholder="0.01"
              value={form.amount}
              onChange={(e) => set({ amount: e.target.value })}
              {...describedBy(id("amount"), errors.amount)}
            />
            <span className="pointer-events-none absolute inset-y-0 right-3 flex items-center text-sm text-muted">ZEC</span>
          </div>
        </Field>
        <Field id={id("count")} label="Number of payments" hint="From 1 to 120." error={errors.count}>
          <input
            id={id("count")}
            type="number"
            min={1}
            max={120}
            step={1}
            className={cx(inputClass, "h-11 tabular-nums")}
            inputMode="numeric"
            autoComplete="off"
            value={form.count}
            onChange={(e) => set({ count: e.target.value })}
            {...describedBy(id("count"), errors.count)}
          />
        </Field>
      </div>

      <CadencePicker id={id("every")} value={form.cadence} onChange={(cadence) => set({ cadence })} error={errors.every} />

      <Field
        id={id("memo")}
        label={
          <>
            Memo <span className="font-normal text-muted">(optional)</span>
          </>
        }
        hint="Shown to the payee. Each payment adds “(k/n)”, like “(3/12)”."
        error={errors.memo}
        aside={
          <span className={cx("text-xs tabular-nums", memoBytes > MEMO_MAX_BYTES ? "text-danger" : "text-muted")}>
            {memoBytes}/{MEMO_MAX_BYTES} bytes
          </span>
        }
      >
        <input
          id={id("memo")}
          className={cx(inputClass, "h-11")}
          autoComplete="off"
          placeholder="Newsletter, weekly"
          value={form.memo}
          onChange={(e) => set({ memo: e.target.value })}
          {...describedBy(id("memo"), errors.memo)}
        />
      </Field>
    </>
  );
}
