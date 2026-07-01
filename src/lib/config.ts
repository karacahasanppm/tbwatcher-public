// Runtime operational config, fetched once from the backend-swappable source (the `config_get` command)
// so the frontend never hardcodes its cadences. Mirrors Rust `config::Config`. Falls back to the shipped
// defaults when there's no Tauri backend (dev/browser) or the call fails.

import { invoke } from "@tauri-apps/api/core";

export type Config = {
  steam_call_spacing_ms: number;
  quote_retry_backoff_ms: number;
  quote_max_retries: number;
  quote_cache_max_age_ms: number;
  portfolio_cooldown_secs: number;
  rate_limit_backoff_secs: number;
  stash_rescan_cooldown_secs: number;
  sell_slots: number;
  chart_window_ms: number;
  chart_points: number;
  history_min_gap_ms: number;
  history_max_age_ms: number;
  history_max_points: number;
  poll_ms: number;
  sync_ms: number;
};

const DEFAULTS: Config = {
  steam_call_spacing_ms: 1000,
  quote_retry_backoff_ms: 2000,
  quote_max_retries: 3,
  quote_cache_max_age_ms: 30 * 60 * 1000,
  portfolio_cooldown_secs: 600,
  rate_limit_backoff_secs: 900,
  stash_rescan_cooldown_secs: 600,
  sell_slots: 4,
  chart_window_ms: 5 * 24 * 60 * 60 * 1000,
  chart_points: 40,
  history_min_gap_ms: 5 * 60 * 1000,
  history_max_age_ms: 30 * 24 * 60 * 60 * 1000,
  history_max_points: 1500,
  poll_ms: 180_000,
  sync_ms: 10_000,
};

let cached: Promise<Config> | null = null;

/** The runtime config, resolved once and memoized. Never rejects — falls back to shipped defaults. */
export function getConfig(): Promise<Config> {
  if (!cached) {
    cached = invoke<Config>("config_get").catch(() => DEFAULTS);
  }
  return cached;
}
