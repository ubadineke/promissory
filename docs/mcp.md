# MCP server: `promissory-mcp`

`promissory-mcp` lets an AI agent (Claude Code, Claude Desktop, or any Model Context Protocol client) check its allowance and ask the payer for payments. It speaks MCP over stdio (JSON-RPC 2.0, one message per line; built on the official Rust SDK, `rmcp`) and talks only to `promissory-app`'s local API. It holds no keys and cannot move funds.

## Build

```sh
cargo build --release -p promissory-mcp      # target/release/promissory-mcp
```

`--app <url>` is promissory-app's address (default `http://127.0.0.1:8738`). It must be `http://` on this machine (127.0.0.1, ::1 or localhost); anything else is refused at startup. stdout carries only protocol messages; diagnostics go to stderr.

## Register it

**Claude Code:**

```sh
claude mcp add promissory -- /path/to/promissory/target/release/promissory-mcp --app http://127.0.0.1:8738
```

**Claude Desktop:** add to `claude_desktop_config.json` (macOS: `~/Library/Application Support/Claude/`, Windows: `%APPDATA%\Claude\`), then restart the app:

```json
{
  "mcpServers": {
    "promissory": {
      "command": "/path/to/promissory/target/release/promissory-mcp",
      "args": ["--app", "http://127.0.0.1:8738"]
    }
  }
}
```

**By hand**, to check it works:

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"sh","version":"1"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"allowance_status","arguments":{}}}' \
  | target/release/promissory-mcp --app http://127.0.0.1:8738
```

## Tools

Every description carries the confirmation rule: *this server never moves funds and holds no keys; any payment or change to an allowance is only a pending request until the payer approves it in the Promissory app; no tool can cancel orders.* All amounts are integer zatoshis. Arguments are validated strictly: unknown fields, wrong types, negative or fractional numbers and numeric strings are refused. Invalid input and promissory-app refusals come back as tool errors (`isError: true`) with a readable message; an unknown tool is a JSON-RPC error.

| Tool | Arguments | Does | Calls |
|---|---|---|---|
| `allowance_status` | `{id?}` | Read-only. Cap, period, chips, `spent_this_period_zat`, `remaining_this_period_zat`, `next_release_unix` / `next_release_in_secs`, and `receiving_address` (the agent's own address). Without `id`, every allowance. | `GET /allowances[/{id}]` |
| `list_orders` | `{}` | Read-only. A summary of the payer's standing orders: id, status, payee, amount, cadence, count, paid. | `GET /orders` |
| `create_oneoff` | `{payee, amount_zat, due?, memo?, reason?}` | **Pending request only.** Returns `{request_id, status: "pending", approve_at: "<app>/requests/<id>"}`. | `POST /requests` |
| `request_status` | `{request_id}` | Read-only. `pending` / `approved` (with `order_id` and the order's status, or the new `allowance_id`) / `rejected` / `expired`. | `GET /requests/{id}`, `GET /orders/{id}` |
| `request_allowance_change` | `{id, new_cap_zat, reason?}` | **Pending request only.** If approved, the new cap applies from the allowance's next period. | `POST /requests` |

Validation: `payee` must look like a unified address (`u1…`, `utest1…`, `uregtest1…`, bech32 characters; promissory-app then checks the checksum, network and Orchard receiver); `amount_zat` and `new_cap_zat` 1 to 21 million ZEC; `due` at least a minute ahead and within 400 days; `memo` at most 400 bytes; `reason` at most 200 characters; no control characters; ids positive.

## Approval flow

1. The agent calls `create_oneoff`. promissory-app records a pending request in `so.db` (`agent_requests`). Nothing is signed, reserved or paid.
2. The agent tells the payer the `approve_at` link and polls `request_status`.
3. The payer opens **Agent allowances → Agent requests** (`/requests`, or the link) and sees what was asked, the payee, the amount, the memo and the agent's reason. **Approve** creates a one-payment order (due as asked, or 10 minutes after approval; never sooner than 5 minutes, so it can be funded and armed), which the treasury funds automatically if it can. **Reject** tells the agent no.
4. A cap change, once approved, stops the allowance (unreleased chips return to the treasury) and creates a replacement with the new cap from the next period (`allowances.md`).

## Limits

- promissory-app refuses a new request with **429** while 10 are pending. The payer has to decide on some first.
- `promissory-mcp` also allows at most 5 new requests per minute per session.
- Undecided requests expire after 7 days.

## Trust model

- **The agent never sees keys.** `promissory-mcp` holds none and has no endpoint that reaches them. promissory-app keeps the seed; the agent's own wallet is separate, and only receives chips.
- **Every spend needs the payer.** The only write `promissory-mcp` can make is `POST /requests`, which records a request. Approve, reject, create, stop and cancel are not reachable from any tool. The integration test asserts that `promissory-mcp` only ever sent GETs and `POST /requests`.
- **Allowances are bounded by construction.** The keeper releases pre-signed chips on schedule; the agent gets one chip at a time and never more than what was released. The payer can stop it any time.
- **What the agent learns.** `list_orders` shows the payer's orders (payees, amounts, schedule). Don't register `promissory-mcp` with an agent you wouldn't show that to.
- **Boundary.** The confirmation rule holds for the MCP surface. promissory-app's API is local and guarded only against browsers (the `X-Promissory-App` header), so an agent that can also run arbitrary shell commands on the payer's machine could call promissory-app directly. Give such agents a sandbox that blocks promissory-app's port, or run them as another OS user with a firewall rule, and let them reach promissory-app only through `promissory-mcp`.
