import { ShieldCheck } from "lucide-react";
import { Card, PageHeader } from "../components/ui";

const PARTIES = [
  { party: "The chain", sees: "Ordinary shielded transactions; at most a hint that a payment was scheduled." },
  { party: "The payee", sees: "Amounts, schedule, memo and cancellation: what receiving the payments shows anyway." },
  { party: "drand", sees: "Nothing. It publishes the same randomness for everyone, on a fixed clock." },
  { party: "The keeper", sees: "Ciphertext until the due date, then a transaction that is about to be public anyway." },
  { party: "Light-wallet server", sees: "Your IP address and sync activity, as for any wallet. Tor is optional." },
];

export function Privacy() {
  return (
    <div className="max-w-3xl">
      <PageHeader title="Who sees what" subtitle="Every payment settles as an ordinary shielded Zcash transaction." />

      <Card className="mb-6 px-5 py-5 sm:px-6">
        <div className="flex gap-3">
          <ShieldCheck className="mt-0.5 size-5 shrink-0 text-accent" aria-hidden />
          <p className="text-pretty">
            <strong className="font-semibold">Sender, recipient and amount are always hidden.</strong> If your device was offline
            in the hour before a payment, the chain may also tell that payment was pre-signed.
          </p>
        </div>
      </Card>

      <Card className="mb-8 overflow-hidden">
        <table className="w-full text-left text-sm">
          <caption className="sr-only">What each party can see</caption>
          <thead className="border-b border-border bg-surface-2/60 text-sm text-muted">
            <tr>
              <th scope="col" className="px-5 py-3 font-medium sm:px-6">
                Party
              </th>
              <th scope="col" className="px-5 py-3 font-medium sm:px-6">
                Sees
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border">
            {PARTIES.map((r) => (
              <tr key={r.party} className="align-top">
                <th scope="row" className="w-[34%] px-5 py-3.5 font-medium sm:px-6">
                  {r.party}
                </th>
                <td className="px-5 py-3.5 text-muted text-pretty sm:px-6">{r.sees}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>

      <h2 className="mb-3 font-display text-lg font-bold">Why the hour before a payment matters</h2>
      <div className="space-y-3 text-sm text-muted text-pretty">
        <p>
          Each payment is signed ahead of time. When your app is online shortly before a payment is due, it signs a fresh copy that
          looks like any other transaction. If it was offline, the keeper releases a fallback copy signed earlier, whose expiry
          setting can hint that it was prepared in advance. It still reveals nothing about who paid whom, or how much.
        </p>
        <p>
          Cancelling is private too: it is one ordinary shielded transaction that moves the reserved funds back to you. Payments
          it invalidates simply fail to be mined.
        </p>
      </div>
    </div>
  );
}
