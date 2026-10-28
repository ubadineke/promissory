//! The MCP server: tools/list and tools/call over the tools in [`crate::tools`], answered from
//! promissory-app's API. JSON-RPC framing, initialize and ping come from `rmcp`.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rmcp::{
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
    },
    service::RequestContext,
    ErrorData, RoleServer, ServerHandler,
};
use serde_json::{json, Value};

use crate::{
    app::App,
    tools::{self, AllowanceChangeArgs, AllowanceStatusArgs, NoArgs, OneoffArgs, RequestStatusArgs},
};

/// At most this many requests (one-offs and cap changes) per [`RATE_WINDOW`] from one session.
/// promissory-app separately refuses more than 10 pending requests.
pub const RATE_LIMIT: usize = 5;
pub const RATE_WINDOW: Duration = Duration::from_secs(60);

/// A sliding-window limit on request creation.
#[derive(Default)]
pub struct RateLimiter {
    recent: VecDeque<Instant>,
}

impl RateLimiter {
    /// Records an attempt at `now` if the window has room.
    pub fn try_acquire(&mut self, now: Instant) -> bool {
        while self.recent.front().is_some_and(|t| now.duration_since(*t) >= RATE_WINDOW) {
            self.recent.pop_front();
        }
        if self.recent.len() >= RATE_LIMIT {
            return false;
        }
        self.recent.push_back(now);
        true
    }
}

#[derive(Clone)]
pub struct SoMcp {
    app: App,
    limiter: Arc<Mutex<RateLimiter>>,
}

pub const INSTRUCTIONS: &str = "Promissory: shielded Zcash allowances and payments that the payer controls. \
Use allowance_status to see how much of your allowance is left this period and when the next chip arrives in \
your wallet. create_oneoff and request_allowance_change only create pending requests: tell the payer the \
approve_at link and poll request_status. You can never move funds, cancel orders or approve your own requests.";

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn ok(v: Value) -> CallToolResult {
    CallToolResult::structured(v)
}

fn fail(msg: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(msg.into())])
}

fn zec(zat: u64) -> String {
    let s = format!("{}.{:08}", zat / 100_000_000, zat % 100_000_000);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// What an agent needs from an allowance view.
pub fn allowance_summary(a: &Value, now: u64) -> Value {
    let next = a["next_release_unix"].as_u64();
    json!({
        "id": a["id"],
        "status": a["status"],
        "label": a["label"],
        "receiving_address": a["agent_ua"],
        "cap_zat": a["cap_zat"],
        "cap_zec": a["cap_zat"].as_u64().map(zec),
        "period_secs": a["period_secs"],
        "chips_per_period": a["chips"],
        "chip_zat": a["chip_zat"],
        "current_period": a["current_period"],
        "periods": a["periods"],
        "period_start": a["period_start"],
        "period_end": a["period_end"],
        "spent_this_period_zat": a["spent_this_period_zat"],
        "remaining_this_period_zat": a["remaining_this_period_zat"],
        "released_total_zat": a["released_total_zat"],
        "next_release_unix": next,
        "next_release_in_secs": next.map(|t| t.saturating_sub(now)),
        "replaced_by": a["replaced_by"],
    })
}

pub fn order_summary(o: &Value) -> Value {
    json!({
        "id": o["id"],
        "status": o["status"],
        "payee": o["payee"],
        "amount_zat": o["amount_zat"],
        "every_secs": o["every_secs"],
        "count": o["count"],
        "paid_zat": o["paid_zat"],
        "first_due": o["first_due"],
        "memo": o["memo"],
    })
}

impl SoMcp {
    pub fn new(app: App) -> Self {
        Self {
            app,
            limiter: Arc::new(Mutex::new(RateLimiter::default())),
        }
    }

    fn acquire(&self) -> bool {
        self.limiter.lock().map(|mut l| l.try_acquire(Instant::now())).unwrap_or(false)
    }

    pub fn tool_list() -> Vec<Tool> {
        tools::all()
            .into_iter()
            .map(|t| {
                let Value::Object(schema) = t.schema else { unreachable!("schemas are objects") };
                let mut tool = Tool::new(t.name, t.description, schema);
                tool.title = Some(t.title.to_string());
                tool.annotations = Some(
                    ToolAnnotations::with_title(t.title)
                        .read_only(t.read_only)
                        .destructive(false)
                        .open_world(false),
                );
                tool
            })
            .collect()
    }

    /// Runs one tool. Invalid input and promissory-app refusals come back as tool errors (`isError`),
    /// so the agent can read them and correct itself.
    pub async fn call(&self, name: &str, args: Option<serde_json::Map<String, Value>>) -> Result<CallToolResult, ErrorData> {
        let result = match name {
            tools::ALLOWANCE_STATUS => self.allowance_status(args).await,
            tools::LIST_ORDERS => self.list_orders(args).await,
            tools::CREATE_ONEOFF => self.create_oneoff(args).await,
            tools::REQUEST_STATUS => self.request_status(args).await,
            tools::REQUEST_ALLOWANCE_CHANGE => self.request_allowance_change(args).await,
            other => return Err(ErrorData::invalid_params(format!("unknown tool: {other}"), None)),
        };
        Ok(result.unwrap_or_else(fail))
    }

    async fn allowance_status(&self, args: Option<serde_json::Map<String, Value>>) -> Result<CallToolResult, String> {
        let a: AllowanceStatusArgs = tools::parse(args)?;
        let now = now_unix();
        match a.id {
            Some(id) => {
                tools::check_id("id", id)?;
                let v = self.app.get(&format!("/allowances/{id}")).await.map_err(|e| e.0)?;
                Ok(ok(allowance_summary(&v, now)))
            }
            None => {
                let v = self.app.get("/allowances").await.map_err(|e| e.0)?;
                let all: Vec<Value> = v.as_array().into_iter().flatten().map(|a| allowance_summary(a, now)).collect();
                Ok(ok(json!({ "allowances": all })))
            }
        }
    }

    async fn list_orders(&self, args: Option<serde_json::Map<String, Value>>) -> Result<CallToolResult, String> {
        let _: NoArgs = tools::parse(args)?;
        let v = self.app.get("/orders").await.map_err(|e| e.0)?;
        let orders: Vec<Value> = v.as_array().into_iter().flatten().map(order_summary).collect();
        Ok(ok(json!({ "orders": orders })))
    }

    async fn create_oneoff(&self, args: Option<serde_json::Map<String, Value>>) -> Result<CallToolResult, String> {
        let a: OneoffArgs = tools::parse(args)?;
        a.validate(now_unix())?;
        if !self.acquire() {
            return Err(format!(
                "rate limited: at most {RATE_LIMIT} requests per minute; wait and try again"
            ));
        }
        let mut body = json!({ "kind": "oneoff", "payee": a.payee, "amount_zat": a.amount_zat });
        if let Some(due) = a.due {
            body["due"] = due.into();
        }
        if let Some(memo) = a.memo {
            body["memo"] = memo.into();
        }
        if let Some(reason) = a.reason {
            body["reason"] = reason.into();
        }
        self.pending(&body).await
    }

    async fn request_allowance_change(&self, args: Option<serde_json::Map<String, Value>>) -> Result<CallToolResult, String> {
        let a: AllowanceChangeArgs = tools::parse(args)?;
        a.validate()?;
        if !self.acquire() {
            return Err(format!(
                "rate limited: at most {RATE_LIMIT} requests per minute; wait and try again"
            ));
        }
        let mut body = json!({ "kind": "allowance_change", "allowance_id": a.id, "new_cap_zat": a.new_cap_zat });
        if let Some(reason) = a.reason {
            body["reason"] = reason.into();
        }
        self.pending(&body).await
    }

    /// Records the request in promissory-app and returns where the payer decides on it.
    async fn pending(&self, body: &Value) -> Result<CallToolResult, String> {
        let r = self.app.create_request(body).await.map_err(|e| e.0)?;
        let id = r["id"].as_i64().ok_or("promissory-app returned no request id")?;
        Ok(ok(json!({
            "request_id": id,
            "status": "pending",
            "approve_at": self.app.approve_url(id),
            "note": "Nothing has been paid or changed. The payer must approve this request; poll request_status.",
        })))
    }

    async fn request_status(&self, args: Option<serde_json::Map<String, Value>>) -> Result<CallToolResult, String> {
        let a: RequestStatusArgs = tools::parse(args)?;
        tools::check_id("request_id", a.request_id)?;
        let r = self.app.get(&format!("/requests/{}", a.request_id)).await.map_err(|e| e.0)?;
        let mut out = json!({
            "request_id": r["id"],
            "kind": r["kind"],
            "status": r["status"],
            "created_at": r["created_at"],
            "decided_at": r["decided_at"],
            "order_id": r["order_id"],
            "allowance_id": r["allowance_id_new"],
            "error": r["error"],
        });
        if r["status"] == "pending" {
            out["approve_at"] = self.app.approve_url(a.request_id as i64).into();
        }
        if let Some(order_id) = r["order_id"].as_i64() {
            if let Ok(o) = self.app.get(&format!("/orders/{order_id}")).await {
                out["order"] = order_summary(&o);
            }
        }
        Ok(ok(out))
    }
}

impl ServerHandler for SoMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("promissory", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(Self::tool_list()))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        Self::tool_list().into_iter().find(|t| t.name == name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.call(&request.name, request.arguments).await.map(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::ServiceExt;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    #[test]
    fn rate_limiter_slides() {
        let mut l = RateLimiter::default();
        let t0 = Instant::now();
        for _ in 0..RATE_LIMIT {
            assert!(l.try_acquire(t0));
        }
        assert!(!l.try_acquire(t0 + Duration::from_secs(59)));
        assert!(l.try_acquire(t0 + RATE_WINDOW));
    }

    #[test]
    fn summaries() {
        let a = json!({"id": 1, "status": "active", "agent_ua": "uregtest1a", "cap_zat": 3_000_000,
                       "chips": 3, "chip_zat": 1_000_000, "next_release_unix": 1_100, "label": "",
                       "spent_this_period_zat": 1_000_000, "remaining_this_period_zat": 2_000_000});
        let s = allowance_summary(&a, 1_000);
        assert_eq!(s["receiving_address"], "uregtest1a");
        assert_eq!(s["cap_zec"], "0.03");
        assert_eq!(s["next_release_in_secs"], 100);
        assert_eq!(s["remaining_this_period_zat"], 2_000_000);
        assert_eq!(zec(100_000_000), "1");
        assert_eq!(zec(1_050_000), "0.0105");
    }

    /// Runs the server over an in-memory pipe and exchanges raw JSON-RPC lines with it.
    struct Session {
        write: tokio::io::WriteHalf<tokio::io::DuplexStream>,
        lines: tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
    }

    impl Session {
        async fn start() -> Self {
            // Points at a closed port: these tests never reach promissory-app.
            let server = SoMcp::new(App::new("http://127.0.0.1:9").unwrap());
            let (client, server_io) = tokio::io::duplex(1 << 16);
            tokio::spawn(async move {
                if let Ok(running) = server.serve(server_io).await {
                    let _ = running.waiting().await;
                }
            });
            let (read, write) = tokio::io::split(client);
            let mut s = Session {
                write,
                lines: BufReader::new(read).lines(),
            };
            let init = s
                .request(json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
                    "protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": {"name": "test", "version": "1"}}}))
                .await;
            assert_eq!(init["result"]["serverInfo"]["name"], "promissory");
            assert!(init["result"]["capabilities"]["tools"].is_object());
            assert!(init["result"]["instructions"].as_str().unwrap().contains("never move funds"));
            s.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
            s
        }

        async fn send(&mut self, v: Value) {
            self.write.write_all(format!("{v}\n").as_bytes()).await.unwrap();
        }

        async fn request(&mut self, v: Value) -> Value {
            let id = v["id"].clone();
            self.send(v).await;
            loop {
                let line = tokio::time::timeout(Duration::from_secs(10), self.lines.next_line())
                    .await
                    .expect("response in time")
                    .unwrap()
                    .expect("server still running");
                let msg: Value = serde_json::from_str(&line).unwrap();
                if msg["id"] == id {
                    return msg;
                }
            }
        }

        async fn call(&mut self, id: u64, name: &str, args: Value) -> Value {
            self.request(json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
                                "params": {"name": name, "arguments": args}}))
                .await
        }
    }

    #[tokio::test]
    async fn json_rpc_handling() {
        let mut s = Session::start().await;

        let pong = s.request(json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})).await;
        assert_eq!(pong["result"], json!({}));

        let list = s.request(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).await;
        let tools = list["result"]["tools"].as_array().unwrap();
        let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            ["allowance_status", "list_orders", "create_oneoff", "request_status", "request_allowance_change"]
        );
        for t in tools {
            assert!(t["description"].as_str().unwrap().contains("Confirmation rule"));
            assert_eq!(t["inputSchema"]["additionalProperties"], false);
        }

        let unknown = s.request(json!({"jsonrpc": "2.0", "id": 3, "method": "orders/cancel"})).await;
        assert_eq!(unknown["error"]["code"], -32601);

        let no_tool = s.call(4, "cancel_order", json!({"id": 1})).await;
        assert!(no_tool["error"].is_object(), "{no_tool}");

        // Invalid input is a tool error the agent can read, and never reaches promissory-app.
        let bad = s.call(5, "create_oneoff", json!({"payee": "t1nope", "amount_zat": 5})).await;
        assert_eq!(bad["result"]["isError"], true, "{bad}");
        assert!(bad["result"]["content"][0]["text"].as_str().unwrap().contains("unified address"));

        let bad = s.call(6, "request_allowance_change", json!({"id": 1, "new_cap_zat": "lots"})).await;
        assert_eq!(bad["result"]["isError"], true, "{bad}");

        let bad = s.call(7, "request_status", json!({"request_id": 0})).await;
        assert_eq!(bad["result"]["isError"], true, "{bad}");

        // promissory-app unreachable: a readable tool error, not a crash.
        let down = s.call(8, "list_orders", json!({})).await;
        assert_eq!(down["result"]["isError"], true, "{down}");
        assert!(down["result"]["content"][0]["text"].as_str().unwrap().contains("cannot reach promissory-app"));

        // Still serving after all of that.
        let pong = s.request(json!({"jsonrpc": "2.0", "id": 9, "method": "ping"})).await;
        assert_eq!(pong["result"], json!({}));
    }
}
