//! Upload limits: how far ahead a round may be, token-bucket rate limits on `POST /v1/blobs`,
//! and a storage quota.
//!
//! Privacy: client addresses are never logged or written to disk. With `RateKey::Ip`, a
//! client's bucket is keyed by SHA-256(salt ‖ address) in memory only; the salt is random and
//! replaced, with every bucket, at each UTC day boundary. IPv6 addresses count per /64. Behind
//! Tor or a proxy every client shares one address, so use `RateKey::None` there and rely on
//! the global bucket.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rand::{rand_core::UnwrapErr, rngs::SysRng, Rng};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum RateKey {
    /// No per-client buckets; only the global one.
    None,
    /// One bucket per client address (salted hash, memory only).
    Ip,
}

impl RateKey {
    pub fn as_str(self) -> &'static str {
        match self {
            RateKey::None => "none",
            RateKey::Ip => "ip",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Limits {
    /// Uploads locked to a round more than this many days ahead are refused.
    pub max_horizon_days: u64,
    pub rate_key: RateKey,
    /// Per-client bucket: refill per minute and capacity. A capacity of 0 disables it.
    pub client_per_min: u32,
    pub client_burst: u32,
    /// Global bucket, shared by everyone. A capacity of 0 disables it.
    pub global_per_min: u32,
    pub global_burst: u32,
    /// Cap on the ciphertext bytes held (locked, released or in flight). 0 means no cap.
    pub max_stored_bytes: u64,
    /// Cap on the number of locked blobs. 0 means no cap.
    pub max_locked_blobs: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_horizon_days: 400,
            rate_key: RateKey::Ip,
            // Arming a 120-installment order uploads 120 blobs at once.
            client_per_min: 60,
            client_burst: 300,
            global_per_min: 1200,
            global_burst: 3000,
            max_stored_bytes: 1 << 30,
            max_locked_blobs: 100_000,
        }
    }
}

impl Limits {
    /// The furthest round ahead of `latest` an upload may be locked to (quicknet: 3 s rounds).
    pub fn max_round(&self, latest: u64) -> u64 {
        latest.saturating_add(self.max_horizon_days.saturating_mul(86_400) / 3)
    }
}

#[derive(Clone, Copy, Debug)]
struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn full(burst: u32, now: Instant) -> Self {
        Self {
            tokens: f64::from(burst),
            at: now,
        }
    }

    fn refill(&mut self, per_min: u32, burst: u32, now: Instant) {
        let dt = now.saturating_duration_since(self.at).as_secs_f64();
        self.tokens = (self.tokens + dt * f64::from(per_min) / 60.0).min(f64::from(burst));
        self.at = now;
    }

    /// Time until one token is available.
    fn wait(&self, per_min: u32) -> Duration {
        if per_min == 0 {
            return Duration::from_secs(3600);
        }
        Duration::from_secs_f64(((1.0 - self.tokens) * 60.0 / f64::from(per_min)).max(0.0))
    }
}

/// Above this many tracked clients, refilled buckets are forgotten (and all of them if that
/// is not enough), so a flood of addresses can't grow memory without bound.
const MAX_CLIENTS: usize = 65_536;

struct Clients {
    day: u64,
    salt: [u8; 32],
    buckets: HashMap<[u8; 32], Bucket>,
}

pub struct RateLimiter {
    limits: Limits,
    global: Mutex<Bucket>,
    clients: Mutex<Clients>,
}

fn fresh_salt() -> [u8; 32] {
    let mut s = [0u8; 32];
    UnwrapErr(SysRng).fill_bytes(&mut s);
    s
}

/// IPv4 (including v4-mapped v6) as is; IPv6 truncated to its /64.
fn client_bytes(ip: IpAddr) -> Vec<u8> {
    match ip {
        IpAddr::V4(v4) => v4.octets().to_vec(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.octets().to_vec(),
            None => v6.octets()[..8].to_vec(),
        },
    }
}

impl RateLimiter {
    pub fn new(limits: Limits) -> Self {
        let now = Instant::now();
        Self {
            global: Mutex::new(Bucket::full(limits.global_burst, now)),
            clients: Mutex::new(Clients {
                day: 0,
                salt: fresh_salt(),
                buckets: HashMap::new(),
            }),
            limits,
        }
    }

    /// Whether `check` uses the client's address at all.
    pub fn keys_by_ip(&self) -> bool {
        self.limits.rate_key == RateKey::Ip && self.limits.client_burst > 0
    }

    /// Number of client buckets currently held (for tests).
    pub fn tracked_clients(&self) -> usize {
        self.clients.lock().unwrap_or_else(|e| e.into_inner()).buckets.len()
    }

    /// Takes one token from the client's bucket and the global one, or returns how long to
    /// wait. A refused request takes nothing. `unix` only selects the salt's day.
    pub fn check(&self, ip: Option<IpAddr>, now: Instant, unix: u64) -> Result<(), Duration> {
        let l = &self.limits;
        let mut global = self.global.lock().unwrap_or_else(|e| e.into_inner());
        if l.global_burst > 0 {
            global.refill(l.global_per_min, l.global_burst, now);
            if global.tokens < 1.0 {
                return Err(global.wait(l.global_per_min));
            }
        }
        if let (Some(ip), true) = (ip, self.keys_by_ip()) {
            let mut c = self.clients.lock().unwrap_or_else(|e| e.into_inner());
            let day = unix / 86_400;
            if c.day != day {
                c.day = day;
                c.salt = fresh_salt();
                c.buckets.clear();
            }
            let key: [u8; 32] = Sha256::new()
                .chain_update(c.salt)
                .chain_update(client_bytes(ip))
                .finalize()
                .into();
            if c.buckets.len() >= MAX_CLIENTS && !c.buckets.contains_key(&key) {
                c.buckets.retain(|_, b| {
                    b.refill(l.client_per_min, l.client_burst, now);
                    b.tokens < f64::from(l.client_burst)
                });
                if c.buckets.len() >= MAX_CLIENTS {
                    c.buckets.clear();
                }
            }
            let b = c
                .buckets
                .entry(key)
                .or_insert_with(|| Bucket::full(l.client_burst, now));
            b.refill(l.client_per_min, l.client_burst, now);
            if b.tokens < 1.0 {
                return Err(b.wait(l.client_per_min));
            }
            b.tokens -= 1.0;
        }
        if l.global_burst > 0 {
            global.tokens -= 1.0;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(client: (u32, u32), global: (u32, u32), key: RateKey) -> RateLimiter {
        RateLimiter::new(Limits {
            rate_key: key,
            client_per_min: client.0,
            client_burst: client.1,
            global_per_min: global.0,
            global_burst: global.1,
            ..Limits::default()
        })
    }

    const A: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, 1));
    const B: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, 2));

    #[test]
    fn per_client_bucket_refills() {
        let rl = limiter((60, 2), (6000, 1000), RateKey::Ip);
        let t = Instant::now();
        assert!(rl.check(Some(A), t, 0).is_ok());
        assert!(rl.check(Some(A), t, 0).is_ok());
        let wait = rl.check(Some(A), t, 0).unwrap_err();
        assert!(wait <= Duration::from_secs(1) && wait > Duration::ZERO, "{wait:?}");
        assert!(rl.check(Some(B), t, 0).is_ok(), "other clients are unaffected");
        assert!(rl.check(Some(A), t + Duration::from_millis(1001), 0).is_ok());
    }

    #[test]
    fn global_bucket_is_the_backstop() {
        let rl = limiter((60, 100), (60, 3), RateKey::Ip);
        let t = Instant::now();
        for ip in [A, B, A] {
            assert!(rl.check(Some(ip), t, 0).is_ok());
        }
        assert!(rl.check(Some(B), t, 0).is_err());
        assert!(rl.check(None, t, 0).is_err());
    }

    #[test]
    fn key_none_ignores_addresses() {
        let rl = limiter((60, 1), (6000, 1000), RateKey::None);
        let t = Instant::now();
        for _ in 0..10 {
            assert!(rl.check(Some(A), t, 0).is_ok());
        }
        assert_eq!(rl.tracked_clients(), 0, "no per-address state at all");
    }

    #[test]
    fn refused_requests_take_no_tokens() {
        let rl = limiter((60, 1), (60, 2), RateKey::Ip);
        let t = Instant::now();
        assert!(rl.check(Some(A), t, 0).is_ok());
        assert!(rl.check(Some(A), t, 0).is_err());
        assert!(rl.check(Some(B), t, 0).is_ok(), "A's refusal left the global token");
    }

    #[test]
    fn salt_and_buckets_rotate_daily() {
        let rl = limiter((60, 1), (6000, 1000), RateKey::Ip);
        let t = Instant::now();
        assert!(rl.check(Some(A), t, 86_400 * 5).is_ok());
        let salt = rl.clients.lock().unwrap().salt;
        assert!(rl.check(Some(A), t, 86_400 * 5 + 100).is_err());
        assert!(rl.check(Some(A), t, 86_400 * 6).is_ok(), "new day, new bucket");
        assert_ne!(rl.clients.lock().unwrap().salt, salt);
        assert_eq!(rl.tracked_clients(), 1);
    }

    #[test]
    fn ipv6_counts_per_slash_64() {
        let rl = limiter((60, 1), (6000, 1000), RateKey::Ip);
        let t = Instant::now();
        let a: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2::ffff".parse().unwrap();
        let c: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert!(rl.check(Some(a), t, 0).is_ok());
        assert!(rl.check(Some(b), t, 0).is_err());
        assert!(rl.check(Some(c), t, 0).is_ok());
    }

    #[test]
    fn horizon_in_rounds() {
        let l = Limits::default();
        assert_eq!(l.max_round(100), 100 + 400 * 28_800);
    }
}
