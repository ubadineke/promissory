import { useState } from "react";
import { Globe } from "lucide-react";
import { fiatToZat, formatZec, parseZec } from "@promissory/client";
import { formatNaira, RateNote, useNaira } from "../lib/fiat";
import { cx, describedBy, Field, inputClass, Notice } from "./ui";

/** "123,456.50" → 123456.5; NaN when it is not a plain amount. */
function parseNaira(s: string): number {
  const t = s.replace(/[₦,\s]/g, "");
  return /^\d+(\.\d{0,2})?$/.test(t) ? Number(t) : Number.NaN;
}

function zatOf(amount: string): number | null {
  try {
    return parseZec(amount);
  } catch {
    return null;
  }
}

/**
 * Remittance amount: type naira, get ZEC rounded to whole zatoshis. The order itself is fixed in
 * ZEC; the naira value of each payment moves with the rate.
 */
export function SendHomeAmount({ amount, setAmount, idPrefix }: { amount: string; setAmount: (zec: string) => void; idPrefix: string }) {
  const { ngnPerZec, rate, error } = useNaira();
  const [ngn, setNgn] = useState("");
  const [lastSet, setLastSet] = useState<string>();
  const id = `${idPrefix}-ngn`;
  const shownNgn = lastSet === amount ? ngn : "";
  const naira = parseNaira(shownNgn);
  const ngnError = shownNgn !== "" && !(naira > 0) ? "Enter an amount in naira, like 50,000" : undefined;
  const zat = zatOf(amount);

  function onNgn(v: string) {
    setNgn(v);
    const n = parseNaira(v);
    if (ngnPerZec !== undefined && n > 0) {
      const z = fiatToZat(n, ngnPerZec);
      const zec = formatZec(Math.max(1, z), { minDecimals: 0, grouping: false });
      setLastSet(zec);
      setAmount(zec);
    } else {
      setLastSet(amount);
    }
  }

  return (
    <div className="space-y-3 rounded-2xl border border-dawn/40 bg-dawn-soft px-4 py-4">
      <p className="flex items-center gap-2 text-sm font-medium">
        <Globe className="size-4 text-dawn-ink" aria-hidden />
        Send home: weekly, priced in naira
      </p>
      {ngnPerZec === undefined ? (
        error ? (
          <Notice tone="warn">
            <strong>No naira rate right now.</strong> Enter the amount in ZEC below. ({error})
          </Notice>
        ) : (
          <p className="text-sm text-muted">Fetching the ZEC/NGN rate…</p>
        )
      ) : (
        <Field
          id={id}
          label="Amount per payment in naira"
          hint={
            zat !== null
              ? `Converted at ${formatNaira(ngnPerZec)} per ZEC and rounded to whole zatoshis: ${formatZec(zat, { minDecimals: 0 })} ZEC ≈ ${formatNaira((zat / 1e8) * ngnPerZec)}.`
              : `1 ZEC ≈ ${formatNaira(ngnPerZec)}.`
          }
          error={ngnError}
        >
          <div className="relative max-w-64">
            <span className="pointer-events-none absolute inset-y-0 left-3 flex items-center text-sm text-muted">₦</span>
            <input
              id={id}
              className={cx(inputClass, "h-11 pl-7 tabular-nums")}
              inputMode="decimal"
              autoComplete="off"
              placeholder="50,000"
              value={shownNgn}
              onChange={(e) => onNgn(e.target.value)}
              {...describedBy(id, ngnError)}
            />
          </div>
        </Field>
      )}
      <p className="text-xs text-muted text-pretty">
        The order pays a fixed amount of ZEC each week. Its naira value moves with the market.
      </p>
      {rate && <RateNote />}
    </div>
  );
}
