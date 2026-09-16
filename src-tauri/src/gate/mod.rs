//! Persisted reload gates for the expensive Steam-backed reloads (portfolio, stash). The cooldown is a
//! **wall-clock** "locked until" timestamp written to disk, so closing and reopening the app cannot bypass
//! it. The last successful payload is cached alongside, so within the window — or after a failed refresh —
//! the UI serves saved data (clearly dated) instead of re-hitting Steam.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Whole seconds remaining until `until_ms`, or 0 once it has passed.
pub fn remaining_secs(until_ms: u64) -> u64 {
    let now = now_ms();
    if until_ms > now {
        (until_ms - now).div_ceil(1000)
    } else {
        0
    }
}

/// A persisted reload gate: the last successful payload plus when it was fetched and when the next Steam
/// fetch becomes allowed. Defaults to empty/expired, so a first load always fetches.
#[derive(Clone, Serialize, Deserialize)]
pub struct ReloadCache<T> {
    /// Wall-clock ms when `payload` was fetched; shown to the user as "data as of …". 0 = never fetched.
    pub fetched_at_ms: u64,
    /// A new Steam fetch is suppressed until this wall-clock ms (persisted → survives restarts).
    pub locked_until_ms: u64,
    pub payload: Option<T>,
}

// Hand-written so the empty default needs no `T: Default` (the payload is simply `None`).
impl<T> Default for ReloadCache<T> {
    fn default() -> Self {
        Self {
            fetched_at_ms: 0,
            locked_until_ms: 0,
            payload: None,
        }
    }
}
