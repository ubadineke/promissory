//! End-to-end keeper tests: the real API and release loop against a fake beacon and chain.
//!
//! Ciphertexts are real tlock encryptions to quicknet round 1000, and the fake beacon serves
//! quicknet's real round-1000 signature (the tlock-rs test vector), so decryption is genuine.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use promissory_core::{
    envelope::Envelope,
    net::{SendOutcome, TxStatus},
    network::SoNetwork,
    tlock,
};
use promissory_keeper::{
    api,
    backend::{Beacon, Chain},
    db::{Db, State},
    keeper::{Config, Keeper, MAX_PAST_ROUNDS},
    limits::{Limits, RateKey},
    webhook::Notifier,
};
use tower::ServiceExt;
use zcash_primitives::transaction::{Authorized, Transaction, TransactionData, TxVersion};
use zcash_protocol::{
    consensus::{BlockHeight, BranchId},
    TxId,
};

const ROUND: u64 = 1000;
const QUICKNET_PK: &str = "83cf0f2896adee7eb8b5f01fcad3912212c437e0073e911fb90022d3e760183c8c4b450b6a0a6c3ac6a5776a2d1064510d1fec758c921cc22b0e17e63aaf4bcb5ed66304de9cf809bd274ca73bab4af5a6e9c76a4bc09e76eae8991ef5ece45a";
const ROUND_1000_SIG: &str = "b44679b9a59af2ec876b1a6b1ad52ea9b1615fc3982b19576350f93447cb1125e342b73a8dd2bacbe47e4b6b63ed5e39";
const T0: u64 = 1_800_000_000;

struct FakeBeacon {
    latest: AtomicU64,
}

#[async_trait]
impl Beacon for FakeBeacon {
    async fn latest_round(&self) -> Result<u64> {
        Ok(self.latest.load(Ordering::SeqCst))
    }
    async fn signature(&self, round: u64) -> Result<Option<Vec<u8>>> {
        assert_eq!(round, ROUND, "the fake only knows round 1000");
        Ok((self.latest.load(Ordering::SeqCst) >= round).then(|| hex::decode(ROUND_1000_SIG).unwrap()))
    }
    fn chain_hash(&self) -> Vec<u8> {
        hex::decode(tlock::QUICKNET_HASH).unwrap()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Fake {
    Unknown,
    Mempool,
    Mined(u32),
    Orphaned,
}

#[derive(Default)]
struct FakeChain {
    tip: AtomicU32,
    sent: Mutex<Vec<Vec<u8>>>,
    /// Rejection messages for the next sends; empty means accept.
    rejections: Mutex<VecDeque<String>>,
    /// Sends that fail before reaching the node.
    transport_failures: AtomicU32,
    status: Mutex<HashMap<TxId, Fake>>,
    /// Status a txid switches to after its next poll.
    then: Mutex<HashMap<TxId, Fake>>,
}

impl FakeChain {
    fn set(&self, txid: TxId, s: Fake) {
        self.status.lock().unwrap().insert(txid, s);
    }
    fn reject_next(&self, msg: &str) {
        self.rejections.lock().unwrap().push_back(msg.to_string());
    }
    fn sent(&self) -> Vec<Vec<u8>> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl Chain for FakeChain {
    async fn tip_height(&self) -> Result<BlockHeight> {
        Ok(BlockHeight::from_u32(self.tip.load(Ordering::SeqCst)))
    }
    async fn send(&self, raw: &[u8]) -> Result<SendOutcome> {
        if self
            .transport_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(promissory_core::net::Transport("Unavailable: connection reset".into()).into());
        }
        self.sent.lock().unwrap().push(raw.to_vec());
        if let Some(message) = self.rejections.lock().unwrap().pop_front() {
            return Ok(SendOutcome::Rejected { code: -26, message });
        }
        self.set(txid_of(raw), Fake::Mempool);
        Ok(SendOutcome::Accepted)
    }
    async fn tx_status(&self, txid: &TxId) -> Result<TxStatus> {
        let now = self.status.lock().unwrap().get(txid).copied().unwrap_or(Fake::Unknown);
        if let Some(later) = self.then.lock().unwrap().remove(txid) {
            self.set(*txid, later);
        }
        Ok(match now {
            Fake::Unknown => TxStatus::Unknown,
            Fake::Mempool => TxStatus::Mempool,
            Fake::Mined(h) => TxStatus::Mined(BlockHeight::from_u32(h)),
            Fake::Orphaned => TxStatus::Orphaned,
        })
    }
}

/// A minimal valid v6 transaction (no bundles); `lock_time` makes each one distinct.
fn raw_tx(branch: BranchId, lock_time: u32) -> Vec<u8> {
    let tx = TransactionData::<Authorized>::from_parts(
        TxVersion::V6,
        branch,
        lock_time,
        BlockHeight::from_u32(0),
        None,
        None,
        None,
        None,
    )
    .freeze()
    .unwrap();
    let mut raw = vec![];
    tx.write(&mut raw).unwrap();
    raw
}

fn txid_of(raw: &[u8]) -> TxId {
    Transaction::read(raw, BranchId::Nu6_3).unwrap().txid()
}

fn encrypt(env: &Envelope) -> Vec<u8> {
    let chain = tlock::Chain {
        hash: hex::decode(tlock::QUICKNET_HASH).unwrap(),
        public_key: hex::decode(QUICKNET_PK).unwrap(),
        genesis_time: 1692803367,
        period: 3,
    };
    tlock::encrypt(&chain, ROUND, &env.to_bytes()).unwrap()
}

struct Harness {
    keeper: Arc<Keeper>,
    app: Router,
    beacon: Arc<FakeBeacon>,
    chain: Arc<FakeChain>,
}

fn test_config() -> Config {
    Config {
        tick: Duration::from_secs(3),
        max_attempts: 3,
        backoff_cap: Duration::from_secs(300),
        unknown_grace: Duration::from_secs(0),
        confirmations: 10,
    }
}

fn harness_with(db: Db, notifier: Option<Notifier>) -> Harness {
    let beacon = Arc::new(FakeBeacon {
        latest: AtomicU64::new(ROUND - 1),
    });
    let chain = Arc::new(FakeChain::default());
    chain.tip.store(100, Ordering::SeqCst);
    let keeper = Arc::new(Keeper::new(
        db,
        beacon.clone(),
        chain.clone(),
        SoNetwork::regtest(),
        test_config(),
        notifier,
    ));
    Harness {
        app: api::router(keeper.clone()),
        keeper,
        beacon,
        chain,
    }
}

fn harness() -> Harness {
    harness_with(Db::in_memory().unwrap(), None)
}

fn harness_limits(limits: Limits) -> Harness {
    let mut h = harness();
    h.app = api::router_with(h.keeper.clone(), limits);
    h
}

impl Harness {
    async fn call(&self, req: Request<Body>) -> (StatusCode, Value) {
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let v = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).expect("JSON body")
        };
        (status, v)
    }

    async fn post(&self, body: Value) -> (StatusCode, Value) {
        self.call(
            Request::post("/v1/blobs")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
    }

    /// POST /v1/blobs as if from `peer`.
    async fn post_from(&self, peer: &str, body: Value) -> (StatusCode, Value) {
        let addr: std::net::SocketAddr = peer.parse().unwrap();
        self.call(
            Request::post("/v1/blobs")
                .header("content-type", "application/json")
                .extension(axum::extract::ConnectInfo(addr))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
    }

    /// Uploads `env` and returns (blob_id, delete_token).
    async fn upload(&self, env: &Envelope) -> (String, String) {
        let (status, v) = self.post(json!({ "ciphertext": STANDARD.encode(encrypt(env)) })).await;
        assert_eq!(status, StatusCode::CREATED, "{v}");
        assert_eq!(v["round"], ROUND);
        (
            v["blob_id"].as_str().unwrap().to_string(),
            v["delete_token"].as_str().unwrap().to_string(),
        )
    }

    async fn get(&self, id: &str) -> Value {
        let (status, v) = self
            .call(Request::get(format!("/v1/blobs/{id}")).body(Body::empty()).unwrap())
            .await;
        assert_eq!(status, StatusCode::OK);
        v
    }

    async fn delete(&self, id: &str, auth: Option<&str>) -> (StatusCode, Value) {
        let mut req = Request::delete(format!("/v1/blobs/{id}"));
        if let Some(a) = auth {
            req = req.header("authorization", a);
        }
        self.call(req.body(Body::empty()).unwrap()).await
    }

    fn publish(&self) {
        self.beacon.latest.store(ROUND, Ordering::SeqCst);
    }

    fn ciphertext_present(&self, id: &str) -> bool {
        self.keeper.db.get(id).unwrap().unwrap().ciphertext.is_some()
    }
}

fn one_copy(lock_time: u32) -> (Envelope, Vec<u8>) {
    let raw = raw_tx(BranchId::Nu6_3, lock_time);
    (Envelope::new(vec![(BranchId::Nu6_3, raw.clone())]), raw)
}

#[tokio::test]
async fn upload_is_locked() {
    let h = harness();
    let (id, token) = h.upload(&one_copy(1).0).await;
    assert_eq!(id.len(), 32);
    assert_eq!(token.len(), 64);
    let v = h.get(&id).await;
    assert_eq!(v["state"], "locked");
    assert_eq!(v["round"], ROUND);
    assert!(v.get("txid").is_none());
    let stored = h.keeper.db.get(&id).unwrap().unwrap();
    assert_ne!(stored.token_hash, token.as_bytes(), "only a hash of the token is stored");
}

#[tokio::test]
async fn nothing_sent_before_round() {
    let h = harness();
    let (id, _) = h.upload(&one_copy(1).0).await;
    h.keeper.tick(T0).await.unwrap();
    h.keeper.tick(T0 + 3).await.unwrap();
    assert!(h.chain.sent().is_empty());
    assert_eq!(h.get(&id).await["state"], "locked");
}

#[tokio::test]
async fn releases_right_branch_then_mined_then_drops_ciphertext() {
    let h = harness();
    let nu63 = raw_tx(BranchId::Nu6_3, 1);
    let nu7 = raw_tx(BranchId::Nu7, 2);
    let env = Envelope::new(vec![(BranchId::Nu7, nu7), (BranchId::Nu6_3, nu63.clone())]);
    let (id, _) = h.upload(&env).await;
    h.publish();

    h.keeper.tick(T0).await.unwrap();
    assert_eq!(h.chain.sent(), vec![nu63.clone()]);
    let txid = txid_of(&nu63);
    let v = h.get(&id).await;
    assert_eq!(v["state"], "broadcast");
    assert_eq!(v["txid"], txid.to_string());

    h.keeper.tick(T0 + 3).await.unwrap();
    assert_eq!(h.get(&id).await["state"], "broadcast");

    h.chain.set(txid, Fake::Mined(101));
    h.chain.tip.store(101, Ordering::SeqCst);
    h.keeper.tick(T0 + 6).await.unwrap();
    let v = h.get(&id).await;
    assert_eq!(v["state"], "mined");
    assert_eq!(v["mined_height"], 101);
    assert!(h.ciphertext_present(&id), "kept until buried, for reorgs");

    h.chain.tip.store(110, Ordering::SeqCst);
    h.keeper.tick(T0 + 9).await.unwrap();
    assert_eq!(h.get(&id).await["state"], "mined");
    assert!(!h.ciphertext_present(&id), "ciphertext dropped once buried");
    assert_eq!(h.chain.sent().len(), 1, "sent exactly once");
}

#[tokio::test]
async fn double_spend_is_cancelled() {
    let h = harness();
    let (id, _) = h.upload(&one_copy(1).0).await;
    h.publish();
    h.chain.reject_next(
        "transaction rejected because another transaction in the mempool has already spent some of its inputs",
    );
    h.keeper.tick(T0).await.unwrap();
    let v = h.get(&id).await;
    assert_eq!(v["state"], "cancelled");
    assert!(v["last_error"].as_str().unwrap().contains("already spent"));
    assert!(!h.ciphertext_present(&id));
    h.keeper.tick(T0 + 3).await.unwrap();
    assert_eq!(h.chain.sent().len(), 1);
}

#[tokio::test]
async fn duplicate_nullifier_is_cancelled() {
    let h = harness();
    let (id, _) = h.upload(&one_copy(1).0).await;
    h.publish();
    h.chain.reject_next("orchard double-spend: duplicate nullifier: Nullifier(..)");
    h.keeper.tick(T0).await.unwrap();
    assert_eq!(h.get(&id).await["state"], "cancelled");
}

#[tokio::test]
async fn expired_is_expired() {
    let h = harness();
    let (id, _) = h.upload(&one_copy(1).0).await;
    h.publish();
    h.chain.reject_next("transaction did not pass consensus validation: ExpiredTransaction { expiry_height: 90 }");
    h.keeper.tick(T0).await.unwrap();
    assert_eq!(h.get(&id).await["state"], "expired");
    assert!(!h.ciphertext_present(&id));
}

#[tokio::test]
async fn transient_error_backs_off_then_succeeds() {
    let h = harness();
    let (id, _) = h.upload(&one_copy(1).0).await;
    h.publish();
    h.chain.reject_next("mempool is full");
    h.chain.reject_next("mempool is full");

    h.keeper.tick(T0).await.unwrap();
    let v = h.get(&id).await;
    assert_eq!(v["state"], "released");
    assert!(v["last_error"].as_str().unwrap().contains("mempool is full"));

    // First back-off is one tick (3 s), the second two ticks (6 s).
    h.keeper.tick(T0 + 1).await.unwrap();
    assert_eq!(h.chain.sent().len(), 1);
    h.keeper.tick(T0 + 3).await.unwrap();
    assert_eq!(h.chain.sent().len(), 2);
    h.keeper.tick(T0 + 8).await.unwrap();
    assert_eq!(h.chain.sent().len(), 2);
    h.keeper.tick(T0 + 9).await.unwrap();
    assert_eq!(h.chain.sent().len(), 3);
    let v = h.get(&id).await;
    assert_eq!(v["state"], "broadcast");
    assert!(v.get("last_error").is_none());
}

#[tokio::test]
async fn gives_up_after_max_attempts() {
    let h = harness();
    let (id, _) = h.upload(&one_copy(1).0).await;
    h.publish();
    for _ in 0..3 {
        h.chain.reject_next("something odd");
    }
    for t in [0, 3, 9, 21] {
        h.keeper.tick(T0 + t).await.unwrap();
    }
    assert_eq!(h.chain.sent().len(), 3);
    assert_eq!(h.get(&id).await["state"], "failed");
    assert!(!h.ciphertext_present(&id));
}

#[tokio::test]
async fn no_copy_for_branch_waits_for_upgrade() {
    let h = harness();
    let nu7 = raw_tx(BranchId::Nu7, 7);
    let (id, _) = h.upload(&Envelope::new(vec![(BranchId::Nu7, nu7.clone())])).await;
    h.publish();
    h.keeper.tick(T0).await.unwrap();
    assert!(h.chain.sent().is_empty(), "never sends a wrong-branch copy");
    let v = h.get(&id).await;
    assert_eq!(v["state"], "released");
    assert!(v["last_error"].as_str().unwrap().contains("no copy"));

    // Next block is NU7's activation height (5000 on regtest).
    h.chain.tip.store(4999, Ordering::SeqCst);
    h.keeper.tick(T0 + 3).await.unwrap();
    assert_eq!(h.chain.sent(), vec![nu7]);
    assert_eq!(h.get(&id).await["state"], "broadcast");
}

#[tokio::test]
async fn wrong_branch_rejection_retries_next_tick() {
    let h = harness();
    let (id, _) = h.upload(&one_copy(1).0).await;
    h.publish();
    h.chain.reject_next("transaction uses an incorrect consensus branch id: WrongConsensusBranchId");
    h.keeper.tick(T0).await.unwrap();
    assert_eq!(h.get(&id).await["state"], "released");
    assert_eq!(h.keeper.db.get(&id).unwrap().unwrap().attempts, 0);
    h.keeper.tick(T0 + 3).await.unwrap();
    assert_eq!(h.chain.sent().len(), 2);
    assert_eq!(h.get(&id).await["state"], "broadcast");
}

#[tokio::test]
async fn delete_needs_the_token_and_prevents_broadcast() {
    let h = harness();
    let (id, token) = h.upload(&one_copy(1).0).await;

    let (s, v) = h.delete(&id, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(v["error"].is_string());
    let (s, _) = h.delete(&id, Some("Bearer nope")).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = h.delete("00", Some(&format!("Bearer {token}"))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(h.get(&id).await["state"], "locked");

    let (s, v) = h.delete(&id, Some(&format!("Bearer {token}"))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(v, Value::Null);
    assert_eq!(h.get(&id).await["state"], "deleted");
    assert!(!h.ciphertext_present(&id));

    h.publish();
    h.keeper.tick(T0).await.unwrap();
    h.keeper.tick(T0 + 3).await.unwrap();
    assert!(h.chain.sent().is_empty());
    assert_eq!(h.get(&id).await["state"], "deleted");
}

#[tokio::test]
async fn delete_after_broadcast_is_refused() {
    let h = harness();
    let (id, token) = h.upload(&one_copy(1).0).await;
    h.publish();
    h.keeper.tick(T0).await.unwrap();
    let (s, v) = h.delete(&id, Some(&format!("Bearer {token}"))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(v["error"].as_str().unwrap().contains("broadcast"));
}

#[tokio::test]
async fn unknown_after_broadcast_rebroadcasts_then_classifies() {
    let h = harness();
    let (id, _) = h.upload(&one_copy(1).0).await;
    h.publish();
    h.keeper.tick(T0).await.unwrap();
    let txid = txid_of(&h.chain.sent()[0]);

    // The payer's sweep spent the note; the node drops our transaction.
    h.chain.set(txid, Fake::Unknown);
    h.keeper.tick(T0 + 3).await.unwrap();
    assert_eq!(h.chain.sent().len(), 1, "first sighting only starts the grace period");
    h.chain.reject_next("duplicate nullifier");
    h.keeper.tick(T0 + 6).await.unwrap();
    assert_eq!(h.chain.sent().len(), 2);
    assert_eq!(h.get(&id).await["state"], "cancelled");
}

#[tokio::test]
async fn orphaned_is_rebroadcast() {
    let h = harness();
    let (id, _) = h.upload(&one_copy(1).0).await;
    h.publish();
    h.keeper.tick(T0).await.unwrap();
    let txid = txid_of(&h.chain.sent()[0]);
    h.chain.set(txid, Fake::Mined(101));
    h.chain.tip.store(101, Ordering::SeqCst);
    h.keeper.tick(T0 + 3).await.unwrap();
    assert_eq!(h.get(&id).await["state"], "mined");

    h.chain.set(txid, Fake::Orphaned);
    h.chain.reject_next("transaction already exists in mempool");
    h.keeper.tick(T0 + 6).await.unwrap();
    assert_eq!(h.chain.sent().len(), 2);
    let v = h.get(&id).await;
    assert_eq!(v["state"], "broadcast");
    assert!(v.get("mined_height").is_none());
}

#[tokio::test]
async fn own_copy_mined_is_not_mistaken_for_cancel() {
    let h = harness();
    let (id, _) = h.upload(&one_copy(1).0).await;
    h.publish();
    h.keeper.tick(T0).await.unwrap();
    let txid = txid_of(&h.chain.sent()[0]);
    // Orphaned when polled; mined by the time the rebroadcast is rejected as a double spend.
    h.chain.set(txid, Fake::Orphaned);
    h.chain.then.lock().unwrap().insert(txid, Fake::Mined(103));
    h.chain.reject_next("duplicate nullifier");
    h.chain.tip.store(103, Ordering::SeqCst);
    h.keeper.tick(T0 + 3).await.unwrap();
    let v = h.get(&id).await;
    assert_eq!(v["state"], "mined", "{v}");
    assert_eq!(v["mined_height"], 103);
}

#[tokio::test]
async fn bad_requests_are_json_errors() {
    let h = harness();

    let big = "A".repeat(api::BODY_LIMIT + 1);
    let (s, v) = h.post(json!({ "ciphertext": big })).await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(v["error"].is_string());

    let (s, v) = h.post(json!({ "ciphertext": "not base64!" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("base64"));

    let (s, v) = h.post(json!({ "ciphertext": STANDARD.encode(b"age? no") })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("tlock"));

    let (s, _) = h.post(json!({ "nope": 1 })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, v) = h
        .call(
            Request::post("/v1/blobs")
                .header("content-type", "application/json")
                .body(Body::from("{"))
                .unwrap(),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].is_string());

    let ct = STANDARD.encode(encrypt(&one_copy(1).0));
    let (s, v) = h.post(json!({ "ciphertext": ct, "webhook": "https://example.com/h" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("webhooks are disabled"));

    let (s, v) = h
        .call(Request::get("/v1/blobs/ffff").body(Body::empty()).unwrap())
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(v["error"].is_string());
}

#[tokio::test]
async fn rejects_rounds_more_than_a_day_old() {
    let h = harness();
    h.beacon.latest.store(ROUND + MAX_PAST_ROUNDS + 1, Ordering::SeqCst);
    let ct = STANDARD.encode(encrypt(&one_copy(1).0));
    let (s, v) = h.post(json!({ "ciphertext": ct })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("in the past"));

    let h = harness();
    h.beacon.latest.store(ROUND + MAX_PAST_ROUNDS, Ordering::SeqCst);
    h.upload(&one_copy(1).0).await;
}

#[tokio::test]
async fn health_reports_tip_and_round() {
    let h = harness();
    h.keeper.tick(T0).await.unwrap();
    let (s, v) = h
        .call(Request::get("/v1/health").body(Body::empty()).unwrap())
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["ok"], true);
    assert_eq!(v["tip_height"], 100);
    assert_eq!(v["latest_round"], ROUND - 1);
    assert_eq!(v["network"], "regtest");
    assert_eq!(v["blobs"]["locked"], 0);
}

#[tokio::test]
async fn survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keeper.db");
    let (id, raw) = {
        let h = harness_with(Db::open(&path).unwrap(), None);
        let (env, raw) = one_copy(5);
        (h.upload(&env).await.0, raw)
    };
    let h = harness_with(Db::open(&path).unwrap(), None);
    assert_eq!(h.get(&id).await["state"], "locked");
    h.publish();
    h.keeper.tick(T0).await.unwrap();
    assert_eq!(h.chain.sent(), vec![raw]);
    assert_eq!(h.get(&id).await["state"], "broadcast");
}

#[tokio::test]
async fn webhooks_post_each_state_change() {
    use axum::{routing::post, Json};
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let sink = seen.clone();
    let app = Router::new().route(
        "/hook",
        post(move |Json(v): Json<Value>| {
            let sink = sink.clone();
            async move {
                sink.lock().unwrap().push(v);
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let h = harness_with(Db::in_memory().unwrap(), Some(Notifier::new().unwrap()));
    let (env, raw) = one_copy(1);
    let ct = STANDARD.encode(encrypt(&env));
    let (s, v) = h
        .post(json!({ "ciphertext": ct, "webhook": format!("http://{addr}/hook") }))
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let id = v["blob_id"].as_str().unwrap().to_string();
    h.publish();
    h.keeper.tick(T0).await.unwrap();

    let want = txid_of(&raw).to_string();
    for _ in 0..100 {
        if seen.lock().unwrap().len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut got = seen.lock().unwrap().clone();
    got.sort_by_key(|v| v["status"].as_str().unwrap().to_string());
    assert_eq!(
        got,
        vec![
            json!({ "blob_id": id, "txid": want, "status": "broadcast" }),
            json!({ "blob_id": id, "txid": null, "status": "released" }),
        ]
    );
}

#[test]
fn state_names_round_trip() {
    for s in State::ALL {
        assert_eq!(State::parse(s.as_str()).unwrap(), s);
    }
}

fn ct_json(lock_time: u32) -> Value {
    json!({ "ciphertext": STANDARD.encode(encrypt(&one_copy(lock_time).0)) })
}

#[tokio::test]
async fn rejects_rounds_beyond_the_horizon() {
    // The fixture's round is 1000, so test with a zero-day horizon: nothing past `latest`.
    let limits = Limits {
        max_horizon_days: 0,
        ..Limits::default()
    };
    let h = harness_limits(limits.clone());
    h.beacon.latest.store(ROUND - 1, Ordering::SeqCst);
    let (s, v) = h.post(ct_json(1)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("0 days ahead"), "{v}");

    let h = harness_limits(limits);
    h.beacon.latest.store(ROUND, Ordering::SeqCst);
    let (s, _) = h.post(ct_json(1)).await;
    assert_eq!(s, StatusCode::CREATED, "exactly at the horizon is fine");
}

#[tokio::test]
async fn per_client_rate_limit_is_429_json() {
    let h = harness_limits(Limits {
        client_per_min: 1,
        client_burst: 2,
        ..Limits::default()
    });
    for _ in 0..2 {
        assert_eq!(h.post_from("192.0.2.7:1000", ct_json(1)).await.0, StatusCode::CREATED);
    }
    let (s, v) = h.post_from("192.0.2.7:1001", ct_json(1)).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert!(v["error"].as_str().unwrap().contains("too many uploads"), "{v}");
    assert_eq!(h.post_from("192.0.2.8:1000", ct_json(1)).await.0, StatusCode::CREATED);
    // Status reads are never limited.
    let (s, _) = h.call(Request::get("/v1/health").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn global_rate_limit_applies_without_client_keys() {
    let h = harness_limits(Limits {
        rate_key: RateKey::None,
        client_per_min: 1,
        client_burst: 1,
        global_per_min: 1,
        global_burst: 3,
        ..Limits::default()
    });
    for peer in ["192.0.2.1:1", "192.0.2.1:2", "192.0.2.2:1"] {
        assert_eq!(h.post_from(peer, ct_json(1)).await.0, StatusCode::CREATED, "key none: one client may burst");
    }
    let (s, v) = h.post_from("192.0.2.3:1", ct_json(1)).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert!(v["error"].is_string());
}

#[tokio::test]
async fn storage_quota_is_507_json() {
    // tlock ciphertexts vary by a few bytes; room for two and a half.
    let one = encrypt(&one_copy(1).0).len() as u64;
    let h = harness_limits(Limits {
        max_stored_bytes: one * 5 / 2,
        ..Limits::default()
    });
    let (id, token) = h.upload(&one_copy(1).0).await;
    h.upload(&one_copy(1).0).await;
    let (s, v) = h.post(ct_json(1)).await;
    assert_eq!(s, StatusCode::INSUFFICIENT_STORAGE);
    assert!(v["error"].as_str().unwrap().contains("stored ciphertext bytes"), "{v}");
    // Deleting a blob drops its ciphertext and frees the space.
    assert_eq!(h.delete(&id, Some(&format!("Bearer {token}"))).await.0, StatusCode::NO_CONTENT);
    h.upload(&one_copy(1).0).await;

    let h = harness_limits(Limits {
        max_locked_blobs: 1,
        ..Limits::default()
    });
    h.upload(&one_copy(1).0).await;
    let (s, v) = h.post(ct_json(1)).await;
    assert_eq!(s, StatusCode::INSUFFICIENT_STORAGE);
    assert!(v["error"].as_str().unwrap().contains("locked blobs"), "{v}");
    let (_, v) = h.call(Request::get("/v1/health").body(Body::empty()).unwrap()).await;
    assert_eq!(v["storage"]["locked_blobs"], 1);
    assert_eq!(v["storage"]["max_locked_blobs"], 1);
}

#[tokio::test]
async fn transport_failure_retries_next_tick_without_backoff() {
    let h = harness();
    let (id, _) = h.upload(&one_copy(1).0).await;
    h.publish();
    h.chain.transport_failures.store(2, Ordering::SeqCst);
    h.keeper.tick(T0).await.unwrap();
    let v = h.get(&id).await;
    assert_eq!(v["state"], "released");
    assert!(v["last_error"].as_str().unwrap().contains("unreachable"), "{v}");
    h.keeper.tick(T0 + 3).await.unwrap();
    assert_eq!(h.keeper.db.get(&id).unwrap().unwrap().attempts, 0, "not counted as a rejection");
    h.keeper.tick(T0 + 6).await.unwrap();
    assert_eq!(h.chain.sent().len(), 1);
    assert_eq!(h.get(&id).await["state"], "broadcast");
}

#[tokio::test]
async fn post_delete_alias_for_get_post_only_clients() {
    let h = harness();
    let (id, token) = h.upload(&one_copy(1).0).await;
    let req = |auth: &str| {
        Request::post(format!("/v1/blobs/{id}/delete"))
            .header("authorization", auth)
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap()
    };
    assert_eq!(h.call(req("Bearer nope")).await.0, StatusCode::FORBIDDEN);
    assert_eq!(h.call(req(&format!("Bearer {token}"))).await.0, StatusCode::NO_CONTENT);
    assert_eq!(h.get(&id).await["state"], "deleted");
}

#[tokio::test]
async fn health_reports_transports_and_limits() {
    let h = harness();
    let (_, v) = h.call(Request::get("/v1/health").body(Body::empty()).unwrap()).await;
    assert_eq!(v["lightwalletd_via"], "direct");
    assert_eq!(v["drand_via"], "direct");
    assert_eq!(v["max_horizon_days"], 400);
    assert_eq!(v["rate_limit_key"], "ip");
}
