// Shared entitlement types (Phase 6). Limits are data the backend resolves from a swappable source
// (local defaults today, the admin panel later), so the UI never hardcodes them.

export type Tier = "free" | "premium";

/** Returned when a metered limit blocks an action — enough to explain exactly what was hit. */
export type LimitHit = { kind: string; limit: number; message: string };

/** Result of an add action: `limit` is set (and `ok` false) when a metered limit blocked it. */
export type AddResult = { ok: boolean; limit: LimitHit | null };

export type Entitlement = {
  tier: Tier;
  limits: { watchlist_max: number; alerts_max: number; stash_max: number };
  watchlist_used: number;
  alerts_used: number;
};
