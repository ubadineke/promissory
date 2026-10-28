//! Runs the `promissory-mcp` binary against a mocked promissory-app on an ephemeral port and drives every tool
//! over stdio JSON-RPC, as an MCP client would.

use std::{
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    extract::{Path, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
};

const AGENT: &str = "uregtest1ynahn0vg27j478s8azkuzthxhfagywduxzgx3ymx5armlml42rtxeeamhlywpj5p4fp469a5zck46tc7rfd79rd0nmps3lhauugm2y9q";
const PAYEE: &str = "uregtest13543hcxsg0xcd7he0nsux3kmz2rqdg905n69x86mlyth67pesmxjpkfg6mwesu9chlpggf6clg58n7zth7uhd4ud9n655l8hkykuwwxj";

#[derive(Default)]
struct Mock {
    /// Every request the mock saw: "METHOD /path".
    seen: Vec<String>,
    /// Bodies of `POST /requests`.
    created: Vec<Value>,
}

type Shared = Arc<Mutex<Mock>>;

fn allowance(id: i64) -> Value {
    json!({
        "id": id, "order_id": 7, "status": "active", "agent_ua": AGENT, "label": "Research agent",
        "cap_zat": 3_000_000, "period_secs": 600, "chips": 3, "chip_zat": 1_000_000, "every_secs": 200,
        "periods": 2, "first_due": 1_000, "current_period": 1, "period_start": 1_000, "period_end": 1_600,
        "spent_this_period_zat": 1_000_000, "remaining_this_period_zat": 2_000_000,
        "released_total_zat": 1_000_000, "next_release_unix": 4_000_000_000u64, "replaced_by": null,
    })
}

async fn record(State(m): State<Shared>, method: Method, uri: Uri, req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    m.lock().unwrap().seen.push(format!("{method} {}", uri.path()));
    next.run(req).await
}

async fn create_request(State(m): State<Shared>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if headers.get("x-promissory-app").and_then(|v| v.to_str().ok()) != Some("1") {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "missing X-Promissory-App header"}))).into_response();
    }
    let mut m = m.lock().unwrap();
    if body["amount_zat"] == 999 {
        return (StatusCode::TOO_MANY_REQUESTS, Json(json!({"error": "10 requests are already waiting"}))).into_response();
    }
    m.created.push(body.clone());
    let mut r = body;
    r["id"] = json!(40 + m.created.len());
    r["status"] = json!("pending");
    Json(r).into_response()
}

async fn mock_app() -> (String, Shared) {
    let state: Shared = Arc::default();
    let api = Router::new()
        .route("/allowances", get(|| async { Json(json!([allowance(2), allowance(1)])) }))
        .route(
            "/allowances/{id}",
            get(|Path(id): Path<i64>| async move {
                if id == 2 {
                    Json(allowance(2)).into_response()
                } else {
                    (StatusCode::NOT_FOUND, Json(json!({"error": format!("no allowance {id}")}))).into_response()
                }
            }),
        )
        .route(
            "/orders",
            get(|| async {
                Json(json!([{"id": 7, "status": "active", "payee": AGENT, "amount_zat": 1_000_000, "every_secs": 200,
                             "count": 6, "paid_zat": 1_000_000, "first_due": 1_000, "memo": "Research agent",
                             "keeper": "http://127.0.0.1:8737", "installments": []}]))
            }),
        )
        .route(
            "/orders/{id}",
            get(|Path(id): Path<i64>| async move {
                Json(json!({"id": id, "status": "awaiting_funds", "payee": PAYEE, "amount_zat": 2_500_000,
                            "every_secs": 60, "count": 1, "paid_zat": 0, "first_due": 5_000, "memo": "Invoice 7"}))
            }),
        )
        .route("/requests", post(create_request))
        .route(
            "/requests/{id}",
            get(|Path(id): Path<i64>| async move {
                if id == 41 {
                    Json(json!({"id": 41, "kind": "oneoff", "status": "approved", "created_at": 1, "decided_at": 2,
                                "order_id": 12, "allowance_id_new": null, "error": null}))
                    .into_response()
                } else {
                    Json(json!({"id": id, "kind": "allowance_change", "status": "pending", "created_at": 1,
                                "decided_at": null, "order_id": null, "allowance_id_new": null, "error": null}))
                    .into_response()
                }
            }),
        );
    let router = Router::new()
        .nest("/api/v1", api)
        .layer(axum::middleware::from_fn_with_state(state.clone(), record))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{addr}"), state)
}

struct Client {
    _child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl Client {
    fn spawn(app: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_promissory-mcp"))
            .args(["--app", app])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        Client {
            stdin: child.stdin.take().unwrap(),
            lines: BufReader::new(child.stdout.take().unwrap()).lines(),
            _child: child,
            next_id: 1,
        }
    }

    async fn send(&mut self, v: Value) {
        self.stdin.write_all(format!("{v}\n").as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).await;
        loop {
            let line = tokio::time::timeout(Duration::from_secs(20), self.lines.next_line())
                .await
                .expect("promissory-mcp answered in time")
                .unwrap()
                .expect("promissory-mcp is still running");
            let msg: Value = serde_json::from_str(&line).expect("stdout carries only JSON-RPC");
            if msg["id"] == id {
                return msg;
            }
        }
    }

    /// Calls a tool and returns its structured result, asserting it is not an error.
    async fn tool(&mut self, name: &str, args: Value) -> Value {
        let r = self.request("tools/call", json!({"name": name, "arguments": args})).await;
        assert_eq!(r["result"]["isError"], false, "{name}: {r}");
        r["result"]["structuredContent"].clone()
    }

    async fn tool_error(&mut self, name: &str, args: Value) -> String {
        let r = self.request("tools/call", json!({"name": name, "arguments": args})).await;
        assert_eq!(r["result"]["isError"], true, "{name}: {r}");
        r["result"]["content"][0]["text"].as_str().unwrap().to_string()
    }
}

#[tokio::test]
async fn every_tool_against_a_mocked_promissory_app() {
    let (app, state) = mock_app().await;
    let mut c = Client::spawn(&app);

    let init = c
        .request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "it", "version": "1"}}),
        )
        .await;
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "promissory");
    c.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;

    // tools/list
    let list = c.request("tools/list", json!({})).await;
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 5);
    for t in tools {
        let d = t["description"].as_str().unwrap();
        assert!(d.contains("never moves funds") && d.contains("payer approves"), "{}", t["name"]);
    }

    // allowance_status, one and all
    let a = c.tool("allowance_status", json!({"id": 2})).await;
    assert_eq!(a["receiving_address"], AGENT);
    assert_eq!(a["cap_zat"], 3_000_000);
    assert_eq!(a["cap_zec"], "0.03");
    assert_eq!(a["spent_this_period_zat"], 1_000_000);
    assert_eq!(a["remaining_this_period_zat"], 2_000_000);
    assert_eq!(a["next_release_unix"], 4_000_000_000u64);
    assert!(a["next_release_in_secs"].as_u64().unwrap() > 0);
    let all = c.tool("allowance_status", json!({})).await;
    assert_eq!(all["allowances"].as_array().unwrap().len(), 2);
    let missing = c.tool_error("allowance_status", json!({"id": 3})).await;
    assert!(missing.contains("404") && missing.contains("no allowance 3"), "{missing}");

    // list_orders
    let orders = c.tool("list_orders", json!({})).await;
    assert_eq!(orders["orders"][0]["id"], 7);
    assert!(orders["orders"][0].get("installments").is_none());

    // create_oneoff: a pending request, with where to approve it
    let r = c
        .tool("create_oneoff", json!({"payee": PAYEE, "amount_zat": 2_500_000, "memo": "Invoice 7", "reason": "API credits"}))
        .await;
    assert_eq!(r["request_id"], 41);
    assert_eq!(r["status"], "pending");
    assert_eq!(r["approve_at"], format!("{app}/requests/41"));

    // request_status on the approved one-off includes its order
    let s = c.tool("request_status", json!({"request_id": 41})).await;
    assert_eq!((s["status"].as_str(), s["order_id"].as_i64()), (Some("approved"), Some(12)));
    assert_eq!(s["order"]["status"], "awaiting_funds");

    // request_allowance_change
    let r = c.tool("request_allowance_change", json!({"id": 2, "new_cap_zat": 6_000_000})).await;
    assert_eq!((r["request_id"].as_i64(), r["status"].as_str()), (Some(42), Some("pending")));
    let s = c.tool("request_status", json!({"request_id": 42})).await;
    assert_eq!(s["status"], "pending");
    assert_eq!(s["approve_at"], format!("{app}/requests/42"));

    // promissory-app's pending cap comes back as a readable error
    let full = c.tool_error("create_oneoff", json!({"payee": PAYEE, "amount_zat": 999})).await;
    assert!(full.contains("429"), "{full}");

    // Invalid input never reaches promissory-app
    let before = state.lock().unwrap().seen.len();
    c.tool_error("create_oneoff", json!({"payee": PAYEE, "amount_zat": 0})).await;
    c.tool_error("create_oneoff", json!({"payee": PAYEE, "amount_zat": 5, "to_order": 7})).await;
    c.tool_error("request_allowance_change", json!({"id": -1, "new_cap_zat": 5})).await;
    assert_eq!(state.lock().unwrap().seen.len(), before);

    // The session's own rate limit: 3 requests made so far (2 created + 1 refused), 2 left.
    c.tool("create_oneoff", json!({"payee": PAYEE, "amount_zat": 1_000})).await;
    c.tool("create_oneoff", json!({"payee": PAYEE, "amount_zat": 1_001})).await;
    let limited = c.tool_error("create_oneoff", json!({"payee": PAYEE, "amount_zat": 1_002})).await;
    assert!(limited.contains("rate limited"), "{limited}");

    // The only write promissory-mcp ever made is POST /requests; it never approves, cancels or creates orders.
    let m = state.lock().unwrap();
    assert_eq!(m.created.len(), 4);
    assert_eq!(m.created[0], json!({"kind": "oneoff", "payee": PAYEE, "amount_zat": 2_500_000, "memo": "Invoice 7", "reason": "API credits"}));
    assert_eq!(m.created[1], json!({"kind": "allowance_change", "allowance_id": 2, "new_cap_zat": 6_000_000}));
    for s in &m.seen {
        assert!(s.starts_with("GET ") || s == "POST /api/v1/requests", "unexpected call {s}");
    }
}

#[tokio::test]
async fn refuses_a_non_local_app() {
    let out = Command::new(env!("CARGO_BIN_EXE_promissory-mcp"))
        .args(["--app", "http://example.com:8738"])
        .output()
        .await
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("this machine"));
}
