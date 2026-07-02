//! Runtime configuration seam (backend-managed later). Every operational tunable — Steam pacing, the
//! cooldown/back-off windows, the sell-slot and chart sizes, price-history retention, and the frontend
//! poll cadences — is a value from a *swappable source* (the DESIGN §4 seam). Today a
//! local default; a `RemoteConfig` fetched from the admin/backend API swaps in here later, without any
//! consumer changing. Resolved once at startup and held in `AppState`; live refresh is a later step.

use serde::{Deserialize, Serialize};

/// All backend-tunable operational values. Serialized to the frontend (`config_get`) and, later,
/// deserialized from the backend API. Grown as real knobs appear — no speculative fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Minimum spacing between Steam calls — the rate-limit governor (ms).
    pub steam_call_spacing_ms: u64,
    /// On a rate-limit (429), how long to pause before retrying that quote (ms) and how many times.
    pub quote_retry_backoff_ms: u64,
    pub quote_max_retries: u32,
    /// A cached quote younger than this is reused instead of re-fetched (dedups a scan + fewer Steam
    /// calls); older ones are re-fetched, falling back to the last known price if the fetch is throttled.
    pub quote_cache_max_age_ms: u64,
    /// Portfolio: serve-saved-data window after a successful fetch; longer back-off after a 429 (secs).
    pub portfolio_cooldown_secs: u64,
    pub rate_limit_backoff_secs: u64,
    /// Stash: rescan throttle (secs) + how many Market listing slots the advisor ranks toward.
    pub stash_rescan_cooldown_secs: u64,
    pub sell_slots: u32,
    /// Watchlist chart: the window it spans (ms) and how many points to draw (the merged backend+local
    /// series is downsampled to this many, so a dense stretch stays readable).
    pub chart_window_ms: u64,
    pub chart_points: u32,
    /// Price-history retention — bounds the local store regardless of uptime.
    pub history_min_gap_ms: u64,
    pub history_max_age_ms: u64,
    pub history_max_points: u32,
    /// Frontend cadences: app-wide price poll (ms) + market-view cache re-read (ms).
    pub poll_ms: u64,
    pub sync_ms: u64,
}

/// Where config comes from. Local defaults today; a remote/admin source swaps in later without touching
/// any consumer.
pub trait ConfigSource: Send + Sync {
    fn config(&self) -> Config;
}

/// Built-in defaults — the values the app shipped with before config became backend-driven.
pub struct LocalConfig;

impl ConfigSource for LocalConfig {
    fn config(&self) -> Config {
        Config {
            steam_call_spacing_ms: 1000,
            quote_retry_backoff_ms: 2000,
            quote_max_retries: 3,
            quote_cache_max_age_ms: 30 * 60 * 1000,
            portfolio_cooldown_secs: 600,
            rate_limit_backoff_secs: 900,
            stash_rescan_cooldown_secs: 60,
            sell_slots: 4,
            chart_window_ms: 5 * 24 * 60 * 60 * 1000,
            chart_points: 40,
            history_min_gap_ms: 5 * 60 * 1000,
            history_max_age_ms: 30 * 24 * 60 * 60 * 1000,
            history_max_points: 1500,
            poll_ms: 180_000,
            sync_ms: 10_000,
        }
    }
}
