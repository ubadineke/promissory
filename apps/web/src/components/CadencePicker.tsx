import { EVERY_MIN_SECS, EVERY_PRESETS, formatDuration } from "@promissory/client";
import { cadenceSecs, type CadenceValue } from "../lib/schedule";
import { cx, describedBy, inputClass } from "./ui";

export const CADENCE_PRESETS = [
  { secs: 180, label: "3 min", note: "demo" },
  { secs: EVERY_PRESETS.day, label: "Day" },
  { secs: EVERY_PRESETS.week, label: "Week" },
  { secs: EVERY_PRESETS.days30, label: "30 days" },
] as const;

/** Segmented preset picker (a radio group) plus a seconds field for a custom cadence. */
export function CadencePicker({
  id,
  value,
  onChange,
  error,
}: {
  id: string;
  value: CadenceValue;
  onChange: (v: CadenceValue) => void;
  error?: string | undefined;
}) {
  const secs = cadenceSecs(value);
  const options = [...CADENCE_PRESETS.map((p) => ({ key: String(p.secs), ...p })), { key: "custom", label: "Custom", secs: 0 }];
  return (
    <fieldset className="space-y-2" aria-describedby={error ? `${id}-error` : `${id}-hint`}>
      <legend className="mb-1.5 text-sm font-semibold">Repeat every</legend>
      <div className="grid grid-cols-3 gap-1.5 sm:grid-cols-5">
        {options.map((o) => {
          const checked = o.key === "custom" ? value.preset === "custom" : value.preset === o.secs;
          return (
            <label
              key={o.key}
              className={cx(
                "relative flex h-11 cursor-pointer items-center justify-center gap-1 rounded-xl border text-sm transition-colors",
                "has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring has-[:focus-visible]:ring-offset-2 has-[:focus-visible]:ring-offset-bg",
                checked ? "border-accent bg-accent-soft font-medium text-accent" : "border-border-strong bg-surface hover:bg-surface-2",
              )}
            >
              <input
                type="radio"
                name={id}
                className="sr-only"
                checked={checked}
                onChange={() => onChange({ ...value, preset: o.key === "custom" ? "custom" : o.secs })}
              />
              {o.label}
              {"note" in o && <span className="text-xs font-normal text-muted">({o.note})</span>}
            </label>
          );
        })}
      </div>
      {value.preset === "custom" && (
        <div className="flex items-center gap-2">
          <label htmlFor={`${id}-custom`} className="sr-only">
            Custom interval in seconds
          </label>
          <input
            id={`${id}-custom`}
            className={cx(inputClass, "h-11 max-w-40 tabular-nums")}
            inputMode="numeric"
            autoComplete="off"
            placeholder="seconds"
            value={value.custom}
            onChange={(e) => onChange({ ...value, custom: e.target.value })}
            {...describedBy(id, error)}
          />
          <span className="text-sm text-muted">seconds{Number.isSafeInteger(secs) && secs > 0 ? ` = ${formatDuration(secs)}` : ""}</span>
        </div>
      )}
      {error ? (
        <p id={`${id}-error`} className="text-sm text-danger" role="alert">
          {error}
        </p>
      ) : (
        <p id={`${id}-hint`} className="text-sm text-muted">
          At least {EVERY_MIN_SECS} seconds between payments.
        </p>
      )}
    </fieldset>
  );
}
