# Agent allowances: design

"Up to X per period" for an AI agent's own wallet. Built entirely from the existing engine: an allowance is one standing order, created, funded, armed, released and cancelled like any other. Nothing in promissory-core changed.

## Shape

An allowance is `{agent_ua, cap_zat, period_secs, chips, periods, first_due}`. It becomes one order to `agent_ua`:

| Order field | Value |
|---|---|
| `amount_zat` | `cap_zat / chips`: one chip |
| `every_secs` | `period_secs / chips` |
| `count` | `chips × periods` |
| `window` | `chips`: one period funded at a time |
| `memo` | the label, or "Agent allowance"; the order appends ` (k/n)` |
| `jitter_max_secs` | 0 |

Example: 0.03 ZEC per 10 minutes in 3 chips for 2 periods is an order of 6 × 0.01 ZEC, one every 200 s, window 3.

Rules (`promissory-app` and `@promissory/client` both enforce them):
- `chips` 1–30; `periods` ≥ 1; `chips × periods` ≤ 120 (the order's installment limit).
- `cap_zat` and `period_secs` are multiples of `chips`, so every chip and every gap is equal and periods line up with chips.
- A chip is at least 0.001 ZEC (100,000 zat): each chip pays its own 10,000 zat fee.
- `period_secs / chips` ≥ 60, like any installment spacing.

**Rolling window.** With `window = chips`, the first funding request covers one period (`chips` notes plus the split fee). As chips are paid, the engine splits the next ones from the treasury on its own; if the treasury can't cover them, the allowance shows a `topup`. At most one period's cap sits in the order's reserved account at any time.

## Chips: released one at a time (drip), not all at the period start

A chip released to the agent is the agent's money. There is nothing to sweep back from the agent's wallet, so "unused chips swept back at the end of the period" can only mean chips **not yet released**. Two ways to release:

| | Drip: one chip every `period / chips` (chosen) | Burst: all chips at the period start |
|---|---|---|
| What the agent holds | Grows through the period, one chip at a time | The whole cap at the start |
| What a stop saves | Every chip not yet released (except one inside the 10 min race window) | Nothing in the current period: it's all out already |
| On chain | One 2-action shielded transaction every `period / chips`; shaped like every other installment | `chips` identical transactions in the same block, every period: a visible burst, the same size each time |
| Fees | `chips × 10,000` zat per period | The same |
| Benefit over a single installment of `cap` | Real: the cap is delivered gradually and stopping cuts it off | None: the agent gets the full cap at once, with `chips` times the fees and a stronger fingerprint |

The burst gives chips no purpose (a single installment of `cap` per period would do the same with one fee) and makes a recognizable pattern. The drip is what makes chips useful: the agent can never hold more of this period's allowance than has been released so far, and the payer's stop takes effect within one chip. The engine already supports it without any change, because chips follow `first_due + k × every`.

**Privacy.** Sender, recipient and amount are hidden as for every order. Each chip is an ordinary 1-in, 1-out Ironwood transaction released at its drand round, like any subscription installment. A short period with many chips produces frequent transactions from the same keeper; prefer fewer, larger chips for long periods (for example 4 per day).

**Fees.** `chips` × 10,000 zat per period, plus one split per refill (5,000 zat per note, at least 10,000). More chips cost more but expose less at once.

## Status

`GET /allowances/{id}` derives everything from the order's installments:
- `current_period` (1-based): the period now running, the first before it starts, the last after it ends;
- `spent_this_period_zat`: chips of this period that paid. That is what the allowance has delivered. What the agent does with its own money afterwards is not visible to the payer, by design;
- `remaining_this_period_zat`: chips of this period still reserved or armed;
- `next_release_unix`: the earliest chip still reserved or armed;
- `released_total_zat`.

Statuses map from the order's: `awaiting_funds`, `starting` (splitting), `active`, `completed`, `stopping` / `stopped` (cancelling / cancelled), and `replacing` / `replaced` after a cap change.

## Pause, stop and cap changes

- **Stop** (`POST /allowances/{id}/stop`) is the order's cancel: one sweep returns every unreleased chip to the treasury. Chips inside the race window (`margin_secs`, default 600 s) or already out at the keeper may still pay, exactly as for any order. There is no pause: stop, then create a new allowance to resume.
- **Lower or raise the cap** is cancel and re-create. The order's amount and rounds are fixed at signing time, so a change can't be applied in place. The replacement starts at the next period boundary (never sooner than 5 minutes, so it can be funded and armed), with the periods that remain. The old allowance is stopped, which also sweeps the chips left in the current period (except any inside the race window). Its `replaced_by` points to the new one.

Agents can't do either. Through `promissory-mcp` they can only ask (`request_allowance_change`); the payer approves in the web UI (`/requests`). See `mcp.md`.

## Storage

Table `allowances` in `so.db` (created with `CREATE TABLE IF NOT EXISTS`), one row per allowance, linked to its order by `order_id` (unique). Agent requests are in `agent_requests`. Order and installment rows are untouched, so the rest of the engine (automatic progression, notices, reservation proofs, cancel) treats an allowance's order like any other.

## promissory-core changes that would help (not made)

1. **Explicit rounds per installment.** For a burst or any custom schedule, `order::Schedule` would carry `dues: Option<Vec<u64>>`. `create_with_window` would compute `rounds` from `dues[k] + jitter` instead of `first_due + every_secs × k`, and allow `every_secs = 0` when `dues` is given. `Store::insert_order` already takes `rounds: &[u64]`, but writes `due_unix = first_due + every_secs × k`; it would take `dues: &[u64]` too. Nothing else in `arm`, `freshen`, `refresh` or `cancel` depends on even spacing.
2. **Partial cancel.** `order::cancel` sweeps every note the order holds. A cap change could keep the current period running if cancel took `from_k: u32` and swept only installments `k ≥ from_k` (leaving earlier notes and blobs alone, and the order `active` until they pay). The replacement would then start exactly at the next boundary without cutting the current period short.
