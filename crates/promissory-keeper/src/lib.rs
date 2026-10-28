//! Promissory keeper: holds tlock-encrypted installments and broadcasts each one when its
//! drand round is published.

pub mod api;
pub mod backend;
pub mod db;
pub mod keeper;
pub mod limits;
pub mod webhook;

use std::time::{SystemTime, UNIX_EPOCH};

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
