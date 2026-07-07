# tbwatcher ↔ backend API contract

The interface between the tbwatcher desktop client and the `personal-website` app-backend
(`hasankaraca.com.tr`). This is the shared source of truth: the client (this repo) is built against it, and
the backend (`karacahasanppm/personal-website`) implements it. App slug: **`tbwatcher`**.

All endpoints below are **public, read-only, versioned (`/api/v1`), throttled, no auth token** — they carry
no user-specific or sensitive data (premium/tier auth arrives later with the Merchant-of-Record). They live
in a **separate public route group**, distinct from the backend's Sanctum-guarded `/api/v1` routes.

Base URL (at deploy): `https://hasankaraca.com.tr/api/v1`. Until then the client keeps the backend **off**
and uses its local/Steam sources (graceful fallback on any backend error too).

---

## `GET /api/v1/apps/tbwatcher/version`

Latest released version, for an in-app "update available" note (not an auto-updater).

```json
{ "version": "0.2.0", "channel": "stable", "release_notes": "optional changelog", "download_url": "https://.../download", "published_at": "..." }
```
(This is the backend's existing `AppVersionResource`.) The client uses `version`, `release_notes`,
`download_url`; it compares `version` against its own `CARGO_PKG_VERSION` and, if newer, shows the note.

---

## `GET /api/v1/apps/tbwatcher/config`

The operational config + per-tier limits. Mirrors the client's `config::Config` and `entitlement::Limits`
so it maps 1:1. All fields required.

```json
{
  "config": {
    "steam_call_spacing_ms": 1000,
    "quote_retry_backoff_ms": 2000,
    "quote_max_retries": 3,
    "quote_cache_max_age_ms": 1800000,
    "portfolio_cooldown_secs": 600,
    "rate_limit_backoff_secs": 900,
    "stash_rescan_cooldown_secs": 10,
    "sell_slots": 4,
    "chart_window_ms": 432000000,
    "chart_points": 40,
    "history_min_gap_ms": 300000,
    "history_max_age_ms": 2592000000,
    "history_max_points": 1500,
    "poll_ms": 180000,
    "sync_ms": 10000
  },
  "limits": {
    "free":    { "watchlist_max": 5,   "alerts_max": 3,   "stash_max": 10 },
    "premium": { "watchlist_max": 200, "alerts_max": 100, "stash_max": 1000 }
  }
}
```
The client resolves this once at startup; on any failure it falls back to its built-in `LocalConfig` +
`DefaultLimits`. (The unlimited/full build ignores `limits` and stays uncapped.)

---

## `POST /api/v1/apps/tbwatcher/prices`

Batch price lookup from the backend's Steam-price **cache** — this is the fix for Steam's 429s: one backend
call prices a whole stash, and the backend (not each client) is the single, paced consumer of Steam.

Request:
```json
{ "items": ["Long Sword (Immortal) A", "Minor Ruby", "..."] }
```
Response (only items the cache has; missing ones are simply absent → client falls back to direct Steam for
those):
```json
{ "prices": {
  "Long Sword (Immortal) A": { "lowest_price": "$12.43", "median_price": "$13.10", "volume": 1284, "fetched_at_ms": 1782900000000 },
  "Minor Ruby":              { "lowest_price": "$0.74",  "median_price": "$0.79",  "volume": 14233, "fetched_at_ms": 1782900000000 }
} }
```
`lowest_price`/`median_price` are Steam's formatted strings (currency included), `volume` an integer,
`fetched_at_ms` epoch millis — matching `market::MarketQuote`.

---

## `POST /api/v1/apps/tbwatcher/history`

Batch price **history** (a rolling series per item) — the baseline the watchlist chart shows immediately,
even for a just-added item; the client merges its own finer, self-accumulated points on top. The backend
accumulates one point per scheduled fetch and prunes to a short window (5 days).

Request (`since_ms` optional; defaults to the last 5 days):
```json
{ "items": ["Long Sword (Immortal) A"], "since_ms": 1782500000000 }
```
Response (oldest → newest per item; items with no history are absent):
```json
{ "history": {
  "Long Sword (Immortal) A": [
    { "fetched_at_ms": 1782500000000, "lowest_price": "$12.10" },
    { "fetched_at_ms": 1782520000000, "lowest_price": "$12.43" }
  ]
} }
```

---

## `GET /api/v1/apps/tbwatcher/movers`

Top price movers over a window — the biggest % change from the oldest recorded price to the current one,
ranked by absolute move. Computed server-side from the accumulated price history over the whole market.
(Flip arbitrage isn't derivable here: Steam's recent-sale sits below the lowest ask for ~every item, and
buy-order data isn't in the cheap search feed — so we surface movement, which the history does support.)

Request: optional `?limit=N` (1–200, default 50) and `?window_ms=N` (default 24h).

Response (ranked by absolute change; `volume` is the current listing count):
```json
{ "movers": [
  { "market_hash_name": "Minor Ruby", "old_price": "$0.03", "lowest_price": "$0.05", "volume": 28676, "change_pct": 66.7 }
] }
```

---

## `GET /api/v1/apps/tbwatcher/health`

Price-cache freshness, for external uptime monitoring (point UptimeRobot etc. here). Returns **HTTP 503**
when the newest cached price is older than ~3 missed sweeps (45 min), so a silent cron death is caught.

```json
{ "ok": true, "cache_age_secs": 312, "count": 742 }
```

---

## Notes for the backend implementation (owner-owned logic, §7)

- **Prices integration:** a scheduled job pages Steam's Market **`search/render`** (`norender=1`) for the
  whole listed set — each page carries a batch of currently-listed items with their lowest price in one
  request, so a full sweep is far fewer requests than a per-item `priceoverview` grind (and avoids that
  endpoint's hard per-IP throttle). It upserts `cached_prices`; the endpoint just reads the cache. Items with
  no active listing simply don't appear — no per-item polling, no "inactive" pool.
  - **Pagination MUST be stably sorted** (`sort_column=name&sort_dir=asc`). `search/render` returns ~10
    results per page regardless of `count`, so a full sweep is ~75 paged requests; with the default
    (popularity) sort the result set re-orders between requests and paging by `start` skips ~8% of items and
    duplicates others each run (a distinct item like a low-popularity coin randomly falls into the gap).
    Alphabetical sort is stable across requests → every item is covered exactly once.
- **Config source:** admin-editable (App JSON column or `app_configs`), served verbatim; changing a value in
  the admin retunes every client on its next startup fetch.
- **Per-app DB:** operational data (`cached_prices`) belongs in the app's tenant DB (`app_tbwatcher`);
  config/limit definitions can live in the main DB as admin content.
