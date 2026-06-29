mod catalog;
mod entitlement;
mod gate;
mod history;
mod inventory;
mod market;
mod money;
mod save;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use catalog::{CatalogItem, SteamCatalog};
use entitlement::{Limits, LimitsSource, Tier};
use inventory::{InventoryError, InventoryHolding, SteamInventory};
use market::{MarketDataSource, MarketError, MarketQuote, SteamClient, SteamMarketSource};
use save::SaveReader;
use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, State,
};
use tauri_plugin_notification::NotificationExt;

#[derive(Default, Serialize, Deserialize, Clone)]
struct AppConfig {
    steam_id: Option<String>,
    #[serde(default)]
    alerts: Vec<Alert>,
    #[serde(default)]
    tier: Tier,
}

/// A price-threshold alert. Fires once on the rising edge (condition false→true) and re-arms when the
/// condition clears, so a held condition doesn't spam notifications.
#[derive(Serialize, Deserialize, Clone)]
struct Alert {
    market_hash_name: String,
    name: String,
    target_cents: u64,
    direction: String, // "below" | "above"
    #[serde(default)]
    triggered: bool,
}

struct AppState {
    market: Arc<dyn MarketDataSource>,
    catalog: SteamCatalog,
    inventory: SteamInventory,
    watchlist: Mutex<Vec<CatalogItem>>,
    quotes: Mutex<HashMap<String, MarketQuote>>,
    config: Mutex<AppConfig>,
    history: Mutex<history::Series>,
    limits: Box<dyn LimitsSource>,
    stash_cache: Mutex<gate::ReloadCache<SellAdvisor>>,
    portfolio_cache: Mutex<gate::ReloadCache<Portfolio>>,
    watchlist_path: PathBuf,
    config_path: PathBuf,
    history_path: PathBuf,
    stash_cache_path: PathBuf,
    portfolio_cache_path: PathBuf,
}

// ---- Entitlement & metered limits (Phase 6) ----

/// The result of an action subject to a metered limit. `ok=false` with a `limit` is an *expected* outcome
/// (the UI shows a respectful upsell), not an error — so the command still returns `Ok`.
#[derive(Serialize)]
struct AddResult {
    ok: bool,
    limit: Option<LimitHit>,
}

/// Surfaced when a metered limit blocks an action — enough for the UI to explain exactly what was hit.
/// `Deserialize`/`Clone` because the Sell Advisor carries one in its persisted, cloneable scan cache.
#[derive(Serialize, Deserialize, Clone)]
struct LimitHit {
    kind: String,
    limit: u32,
    message: String,
}

/// The user's tier, its limits, and current usage — so the UI can show "N / max" and a once-at-limit upsell.
#[derive(Serialize)]
struct Entitlement {
    tier: Tier,
    limits: Limits,
    watchlist_used: u32,
    alerts_used: u32,
}

#[tauri::command]
fn entitlement_get(state: State<'_, AppState>) -> Entitlement {
    let (tier, alerts_used) = {
        let config = state.config.lock().unwrap();
        (config.tier, config.alerts.len() as u32)
    };
    let watchlist_used = state.watchlist.lock().unwrap().len() as u32;
    Entitlement {
        tier,
        limits: state.limits.limits(tier),
        watchlist_used,
        alerts_used,
    }
}

/// Set the tier. Today this is how gating is tested; later the admin panel / payment webhook drives it.
#[tauri::command]
fn entitlement_set_tier(state: State<'_, AppState>, tier: Tier) -> Result<(), String> {
    state.config.lock().unwrap().tier = tier;
    persist_config(&state)
}

// ---- Market Watch (Pillar A) ----

/// A watchlist row sent to the UI: the item plus its latest known quote (if any).
#[derive(Serialize)]
struct WatchEntry {
    item: CatalogItem,
    quote: Option<MarketQuote>,
}

#[tauri::command]
async fn catalog_search(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<CatalogItem>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    state.catalog.search(query, 20).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn watchlist_get(state: State<'_, AppState>) -> Vec<WatchEntry> {
    let watchlist = state.watchlist.lock().unwrap();
    let quotes = state.quotes.lock().unwrap();
    watchlist
        .iter()
        .cloned()
        .map(|item| {
            let quote = quotes.get(&item.market_hash_name).cloned();
            WatchEntry { item, quote }
        })
        .collect()
}

#[tauri::command]
fn watchlist_add(state: State<'_, AppState>, item: CatalogItem) -> Result<AddResult, String> {
    // Read the limit without holding the watchlist lock, to keep lock acquisition single-at-a-time.
    let max = {
        let tier = state.config.lock().unwrap().tier;
        state.limits.limits(tier).watchlist_max
    };
    {
        let mut watchlist = state.watchlist.lock().unwrap();
        if watchlist
            .iter()
            .any(|i| i.market_hash_name == item.market_hash_name)
        {
            return Ok(AddResult { ok: true, limit: None }); // already watched — no-op
        }
        if watchlist.len() as u32 >= max {
            return Ok(AddResult {
                ok: false,
                limit: Some(LimitHit {
                    kind: "watchlist".into(),
                    limit: max,
                    message: format!("You've reached your plan's limit of {max} watched items."),
                }),
            });
        }
        watchlist.push(item);
    }
    persist_watchlist(&state)?;
    Ok(AddResult { ok: true, limit: None })
}

#[tauri::command]
fn watchlist_remove(state: State<'_, AppState>, market_hash_name: String) -> Result<(), String> {
    {
        let mut watchlist = state.watchlist.lock().unwrap();
        watchlist.retain(|i| i.market_hash_name != market_hash_name);
    }
    // Drop the item's price history too — otherwise an unwatched series never gets a new point, so its
    // age-based prune never runs and it lingers forever.
    if state.history.lock().unwrap().remove(&market_hash_name).is_some() {
        persist_history(&state).ok();
    }
    persist_watchlist(&state)
}

#[tauri::command]
async fn watchlist_refresh(state: State<'_, AppState>) -> Result<Vec<WatchEntry>, String> {
    let items: Vec<CatalogItem> = state.watchlist.lock().unwrap().clone();
    let mut entries = Vec::with_capacity(items.len());
    let mut recorded = false;
    for item in items {
        // Each quote is governed (paced); a failed item degrades to no quote, never breaks the batch.
        let quote = state.market.quote(&item.market_hash_name).await.ok();
        if let Some(q) = &quote {
            state
                .quotes
                .lock()
                .unwrap()
                .insert(item.market_hash_name.clone(), q.clone());
            // Self-accumulate the price series off the poll we already make (no extra Steam load).
            if let Some(cents) = q.lowest_price.as_deref().and_then(money::parse_usd_cents) {
                let mut history = state.history.lock().unwrap();
                let series = history.entry(item.market_hash_name.clone()).or_default();
                history::record(series, history::PricePoint { t_ms: q.fetched_at_ms, cents });
                recorded = true;
            }
        }
        entries.push(WatchEntry { item, quote });
    }
    if recorded {
        persist_history(&state)?;
    }
    Ok(entries)
}

/// The accumulated price series for one item, oldest → newest, for the watchlist chart.
#[tauri::command]
fn price_history(state: State<'_, AppState>, market_hash_name: String) -> Vec<history::PricePoint> {
    state
        .history
        .lock()
        .unwrap()
        .get(&market_hash_name)
        .cloned()
        .unwrap_or_default()
}

// ---- Portfolio (Pillar D) ----

#[derive(Clone, Serialize, Deserialize)]
struct PortfolioEntry {
    holding: InventoryHolding,
    lowest_price: Option<String>,
    value_text: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Portfolio {
    steam_id: Option<String>,
    entries: Vec<PortfolioEntry>,
    total_value_text: Option<String>,
    error: Option<String>,
    /// Wall-clock ms the shown holdings were fetched ("data as of …"); 0 when there's nothing to show.
    as_of_ms: u64,
    /// Seconds until a reload is allowed (persisted cooldown); the UI counts it down and locks the button.
    next_reload_secs: u64,
    /// True when we're showing saved data because a fresh fetch failed (the `error` says why).
    stale: bool,
}

#[tauri::command]
fn steamid_get(state: State<'_, AppState>) -> Option<String> {
    state.config.lock().unwrap().steam_id.clone()
}

/// Portfolio reload cooldown after a *successful* fetch — saved data is served within this window so we
/// don't re-hit the (strict) inventory endpoint. Persisted (wall-clock), so a restart can't reset it.
const PORTFOLIO_COOLDOWN_SECS: u64 = 600;
/// A longer back-off after Steam rate-limits us, so a 429 isn't provoked again right away.
const RATE_LIMIT_BACKOFF_SECS: u64 = 900;

#[tauri::command]
fn steamid_set(state: State<'_, AppState>, steam_id: String) -> Result<(), String> {
    let changed = {
        let mut config = state.config.lock().unwrap();
        let trimmed = steam_id.trim();
        let next = (!trimmed.is_empty()).then(|| trimmed.to_string());
        let changed = config.steam_id != next;
        config.steam_id = next;
        changed
    };
    // Only a *different* account invalidates the cached portfolio (and its cooldown). Re-submitting the
    // same id must not — otherwise it would be a one-click cooldown bypass.
    if changed {
        *state.portfolio_cache.lock().unwrap() = gate::ReloadCache::default();
        persist_portfolio_cache(&state).ok();
    }
    persist_config(&state)
}

#[tauri::command]
async fn portfolio_get(state: State<'_, AppState>) -> Result<Portfolio, String> {
    let steam_id = state.config.lock().unwrap().steam_id.clone();
    let Some(steam_id) = steam_id else {
        return Ok(Portfolio {
            steam_id: None,
            entries: Vec::new(),
            total_value_text: None,
            error: Some("Set your SteamID to load your inventory.".into()),
            as_of_ms: 0,
            next_reload_secs: 0,
            stale: false,
        });
    };

    // Snapshot the gate once: the cooldown deadline (always present) and the saved payload (only for the
    // same account). The cooldown is keyed on `locked_until_ms` ALONE — independent of whether we have a
    // payload — so a rate-limit back-off with nothing cached still suppresses fetches (otherwise every tab
    // revisit would re-hit Steam and re-arm the cooldown, making the countdown appear to reset).
    let (locked_until_ms, cached) = {
        let c = state.portfolio_cache.lock().unwrap();
        let payload = c
            .payload
            .as_ref()
            .filter(|p| p.steam_id.as_deref() == Some(steam_id.as_str()))
            .map(|p| (c.fetched_at_ms, p.clone()));
        (c.locked_until_ms, payload)
    };

    // Within the persisted cooldown, never touch Steam — serve saved data if we have it, else say why and
    // count down. Restart-proof and revisit-proof.
    let remaining = gate::remaining_secs(locked_until_ms);
    if remaining > 0 {
        return Ok(match cached {
            Some((fetched_at_ms, mut payload)) => {
                payload.as_of_ms = fetched_at_ms;
                payload.next_reload_secs = remaining;
                payload.stale = false;
                payload.error = None;
                payload
            }
            None => Portfolio {
                steam_id: Some(steam_id),
                entries: Vec::new(),
                total_value_text: None,
                error: Some("Steam is rate-limiting us — reload is paused.".into()),
                as_of_ms: 0,
                next_reload_secs: remaining,
                stale: true,
            },
        });
    }

    // Cooldown elapsed → attempt a fresh fetch.
    let now = gate::now_ms();
    let holdings = match state.inventory.holdings(&steam_id).await {
        Ok(h) => h,
        Err(e) => {
            // Re-arm a cooldown even on failure (so it can't be hammered, incl. by reopening the app).
            // A rate-limit gets the longer back-off; other hiccups get the standard window.
            let (message, backoff_secs): (String, u64) = match e {
                InventoryError::Private => (
                    "Inventory is private — make it public to see your portfolio.".into(),
                    PORTFOLIO_COOLDOWN_SECS,
                ),
                InventoryError::RateLimited => {
                    ("Steam is rate-limiting us.".into(), RATE_LIMIT_BACKOFF_SECS)
                }
                InventoryError::Http(detail) => (
                    format!("Couldn't reach Steam ({detail})."),
                    PORTFOLIO_COOLDOWN_SECS,
                ),
            };
            state.portfolio_cache.lock().unwrap().locked_until_ms = now + backoff_secs * 1000;
            persist_portfolio_cache(&state).ok();
            // Keep showing the last saved data if we have it; otherwise just the reason.
            return Ok(match cached {
                Some((fetched_at_ms, mut payload)) => {
                    payload.as_of_ms = fetched_at_ms;
                    payload.next_reload_secs = backoff_secs;
                    payload.stale = true;
                    payload.error = Some(format!("{message} Showing last saved data."));
                    payload
                }
                None => Portfolio {
                    steam_id: Some(steam_id),
                    entries: Vec::new(),
                    total_value_text: None,
                    error: Some(format!("{message} Reload is paused briefly.")),
                    as_of_ms: 0,
                    next_reload_secs: backoff_secs,
                    stale: true,
                },
            });
        }
    };

    let mut entries = Vec::with_capacity(holdings.len());
    let mut total_cents: u64 = 0;
    for holding in holdings {
        let quote = quote_cached_or_fetch(&state, &holding.market_hash_name).await;
        let lowest_price = quote.as_ref().and_then(|q| q.lowest_price.clone());
        let value_cents = lowest_price
            .as_deref()
            .and_then(money::parse_usd_cents)
            .map(|c| c * holding.count);
        if let Some(v) = value_cents {
            total_cents += v;
        }
        entries.push(PortfolioEntry {
            holding,
            lowest_price,
            value_text: value_cents.map(money::format_usd_cents),
        });
    }

    let portfolio = Portfolio {
        steam_id: Some(steam_id),
        entries,
        total_value_text: Some(money::format_usd_cents(total_cents)),
        error: None,
        as_of_ms: now,
        next_reload_secs: PORTFOLIO_COOLDOWN_SECS,
        stale: false,
    };
    *state.portfolio_cache.lock().unwrap() = gate::ReloadCache {
        fetched_at_ms: now,
        locked_until_ms: now + PORTFOLIO_COOLDOWN_SECS * 1000,
        payload: Some(portfolio.clone()),
    };
    persist_portfolio_cache(&state)?;
    Ok(portfolio)
}

/// Reuse the shared quote cache (filled by the watchlist too); only fetch on a miss.
async fn quote_cached_or_fetch(state: &AppState, market_hash_name: &str) -> Option<MarketQuote> {
    if let Some(q) = state.quotes.lock().unwrap().get(market_hash_name).cloned() {
        return Some(q);
    }
    let quote = state.market.quote(market_hash_name).await.ok();
    if let Some(q) = &quote {
        state
            .quotes
            .lock()
            .unwrap()
            .insert(market_hash_name.to_string(), q.clone());
    }
    quote
}

// ---- Alerts (Pillar D) ----

#[tauri::command]
fn alerts_list(state: State<'_, AppState>) -> Vec<Alert> {
    state.config.lock().unwrap().alerts.clone()
}

#[tauri::command]
fn alerts_set(
    state: State<'_, AppState>,
    market_hash_name: String,
    name: String,
    target_price: String,
    direction: String,
) -> Result<AddResult, String> {
    let target_cents = money::parse_price_input(&target_price).ok_or("invalid target price")?;
    {
        let mut config = state.config.lock().unwrap();
        match config
            .alerts
            .iter_mut()
            .find(|a| a.market_hash_name == market_hash_name && a.direction == direction)
        {
            // Editing an existing alert isn't metered (it doesn't grow the count).
            Some(existing) => {
                existing.target_cents = target_cents;
                existing.name = name;
                existing.triggered = false;
            }
            None => {
                let max = state.limits.limits(config.tier).alerts_max;
                if config.alerts.len() as u32 >= max {
                    return Ok(AddResult {
                        ok: false,
                        limit: Some(LimitHit {
                            kind: "alerts".into(),
                            limit: max,
                            message: format!("You've reached your plan's limit of {max} alerts."),
                        }),
                    });
                }
                config.alerts.push(Alert {
                    market_hash_name,
                    name,
                    target_cents,
                    direction,
                    triggered: false,
                });
            }
        }
    }
    persist_config(&state)?;
    Ok(AddResult { ok: true, limit: None })
}

#[tauri::command]
fn alerts_remove(
    state: State<'_, AppState>,
    market_hash_name: String,
    direction: String,
) -> Result<(), String> {
    state
        .config
        .lock()
        .unwrap()
        .alerts
        .retain(|a| !(a.market_hash_name == market_hash_name && a.direction == direction));
    persist_config(&state)
}

/// Check every alert against the latest (cached-or-fetched) price; fire an OS notification on each
/// rising edge. Returns the names that fired, for in-app feedback.
#[tauri::command]
async fn alerts_check(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let alerts: Vec<Alert> = state.config.lock().unwrap().alerts.clone();
    let mut fired = Vec::new();
    let mut changed = false;

    for alert in alerts {
        let Some(quote) = quote_cached_or_fetch(&state, &alert.market_hash_name).await else {
            continue;
        };
        let Some(price_cents) = quote.lowest_price.as_deref().and_then(money::parse_usd_cents) else {
            continue;
        };
        let hit = match alert.direction.as_str() {
            "below" => price_cents <= alert.target_cents,
            "above" => price_cents >= alert.target_cents,
            _ => false,
        };

        let mut config = state.config.lock().unwrap();
        let Some(stored) = config
            .alerts
            .iter_mut()
            .find(|a| a.market_hash_name == alert.market_hash_name && a.direction == alert.direction)
        else {
            continue;
        };
        if hit && !stored.triggered {
            stored.triggered = true;
            changed = true;
            let arrow = if alert.direction == "below" { "≤" } else { "≥" };
            let body = format!(
                "{} {} {} (now {})",
                alert.name,
                arrow,
                money::format_usd_cents(alert.target_cents),
                quote.lowest_price.as_deref().unwrap_or("?")
            );
            let _ = app
                .notification()
                .builder()
                .title("tbwatcher alert")
                .body(body)
                .show();
            fired.push(alert.name.clone());
        } else if !hit && stored.triggered {
            stored.triggered = false;
            changed = true;
        }
    }

    if changed {
        persist_config(&state)?;
    }
    Ok(fired)
}

// ---- Stash & Sell Advisor (Pillar E) ----

/// TBH lists at most 4 items on the Market at once (verified game rule: 4 concurrent listings, 8h
/// relist). With slots this scarce the decision is *which item earns a slot* — so ranking is by unit
/// value, not by line total; the top `slot_limit` priced rows are the recommendation.
const SELL_SLOTS: usize = 4;

/// Valuing the stash hits Steam once per item, so a rescan is throttled to once per window; calls in
/// between are served from the last result (persisted, so a restart can't bypass it). The expensive part
/// is the prices (the save read is cheap), and this section will later anchor the subscription metering.
const RESCAN_COOLDOWN_SECS: u64 = 600;

/// One rankable stash line: an item the player can list, with its current unit and line value.
/// Prices are gross (pre-fee); the post-fee seller take arrives with the Flip pillar.
#[derive(Clone, Serialize, Deserialize)]
struct SellEntry {
    market_hash_name: String,
    count: u64,
    lowest_price: Option<String>,
    line_value_text: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct SellAdvisor {
    entries: Vec<SellEntry>,
    slot_limit: usize,
    total_value_text: Option<String>,
    error: Option<String>,
    /// Wall-clock ms the shown scan was taken ("data as of …"); 0 when there's nothing to show.
    as_of_ms: u64,
    /// Seconds until a fresh rescan is allowed; the UI counts this down and locks the button.
    next_scan_secs: u64,
    /// Set when the tier's `stash_max` capped the scan — the UI shows the upsell (trial values the first N).
    limit: Option<LimitHit>,
}

/// Progress ticks streamed while valuing the stash, so the UI shows live progress instead of a freeze
/// during the (necessarily paced) per-item Steam quotes.
#[derive(Clone, Serialize)]
struct StashProgress {
    done: usize,
    total: usize,
}

/// Read the local save (read-only), value every market-tradeable item in the full in-game stash, and
/// rank them so the player can spend their scarce listing slots on the most valuable items. Emits a
/// progress tick per item over `on_progress` (each quote is paced, so a full stash takes a while).
#[tauri::command]
async fn stash_advisor(
    state: State<'_, AppState>,
    on_progress: tauri::ipc::Channel<StashProgress>,
) -> Result<SellAdvisor, String> {
    // Within the persisted cooldown, serve the last scan untouched — no save read, no Steam quotes.
    {
        let cache = state.stash_cache.lock().unwrap();
        let remaining = gate::remaining_secs(cache.locked_until_ms);
        if remaining > 0 {
            if let Some(mut result) = cache.payload.clone() {
                result.next_scan_secs = remaining;
                result.as_of_ms = cache.fetched_at_ms;
                return Ok(result);
            }
        }
    }

    let stash = match SaveReader::new().read_stash() {
        Ok(s) => s,
        Err(e) => {
            // A missing/changed save degrades to a clear message, never a crash or misreport.
            return Ok(SellAdvisor {
                entries: Vec::new(),
                slot_limit: SELL_SLOTS,
                total_value_text: None,
                error: Some(e.to_string()),
                as_of_ms: 0,
                next_scan_secs: 0,
                limit: None,
            });
        }
    };

    // Resolve the marketable items up front so we can report an accurate total to value.
    let mut jobs: Vec<(Vec<String>, u64)> = stash
        .into_iter()
        .filter_map(|sc| {
            let candidates = save::market_hashes(sc.item_key);
            // Skip non-tradeable items (gear below Legendary, non-listable types, unknown ids).
            (!candidates.is_empty()).then_some((candidates, sc.count))
        })
        .collect();

    // Trial meters stash depth: value only the first `stash_max` items, lifted by premium. Capping here
    // (before quoting) also keeps the trial polite to Steam — fewer paced quotes per scan.
    let tier = state.config.lock().unwrap().tier;
    let cap = state.limits.limits(tier).stash_max as usize;
    let limit = (jobs.len() > cap).then(|| {
        let found = jobs.len();
        LimitHit {
            kind: "stash".into(),
            limit: cap as u32,
            message: format!("Showing the first {cap} of {found} market-tradeable stash items."),
        }
    });
    jobs.truncate(cap);

    let total = jobs.len();
    let _ = on_progress.send(StashProgress { done: 0, total });

    // Distinct item ids can resolve to one market hash (variant ids of the same gear) — group them.
    struct Agg {
        count: u64,
        lowest_price: Option<String>,
    }
    let mut by_hash: HashMap<String, Agg> = HashMap::new();
    for (done, (candidates, count)) in jobs.into_iter().enumerate() {
        let quote = quote_best(&state, &candidates).await;
        let (display, price) = match quote {
            Some(q) => (q.market_hash_name, q.lowest_price),
            None => (candidates.into_iter().next().unwrap(), None),
        };
        let agg = by_hash.entry(display).or_insert(Agg {
            count: 0,
            lowest_price: None,
        });
        agg.count += count;
        if agg.lowest_price.is_none() {
            agg.lowest_price = price;
        }
        let _ = on_progress.send(StashProgress {
            done: done + 1,
            total,
        });
    }

    let mut rows: Vec<(SellEntry, Option<u64>)> = by_hash
        .into_iter()
        .map(|(market_hash_name, agg)| {
            let unit_cents = agg.lowest_price.as_deref().and_then(money::parse_usd_cents);
            let line_value_text = unit_cents.map(|c| money::format_usd_cents(c * agg.count));
            (
                SellEntry {
                    market_hash_name,
                    count: agg.count,
                    lowest_price: agg.lowest_price,
                    line_value_text,
                },
                unit_cents,
            )
        })
        .collect();

    // Highest unit value first; unlisted (no price) last; then by count, then name for stable order.
    rows.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| b.0.count.cmp(&a.0.count))
            .then_with(|| a.0.market_hash_name.cmp(&b.0.market_hash_name))
    });

    let total_cents: u64 = rows
        .iter()
        .filter_map(|(e, unit)| unit.map(|c| c * e.count))
        .sum();
    let entries = rows.into_iter().map(|(e, _)| e).collect();

    let now = gate::now_ms();
    let result = SellAdvisor {
        entries,
        slot_limit: SELL_SLOTS,
        total_value_text: Some(money::format_usd_cents(total_cents)),
        error: None,
        as_of_ms: now,
        next_scan_secs: RESCAN_COOLDOWN_SECS,
        limit,
    };
    *state.stash_cache.lock().unwrap() = gate::ReloadCache {
        fetched_at_ms: now,
        locked_until_ms: now + RESCAN_COOLDOWN_SECS * 1000,
        payload: Some(result.clone()),
    };
    persist_stash_cache(&state)?;
    Ok(result)
}

/// Quote the first candidate hash that's actually listed, reusing the shared quote cache. Gear has two
/// hash spellings ("… A" then plain); a `NotListed` candidate falls through to the next, while a
/// rate-limit/network error stops this item (treated as no quote rather than a wrong one).
async fn quote_best(state: &AppState, candidates: &[String]) -> Option<MarketQuote> {
    {
        let cache = state.quotes.lock().unwrap();
        for hash in candidates {
            if let Some(q) = cache.get(hash) {
                return Some(q.clone());
            }
        }
    }
    for hash in candidates {
        match state.market.quote(hash).await {
            Ok(q) => {
                state.quotes.lock().unwrap().insert(hash.clone(), q.clone());
                return Some(q);
            }
            Err(MarketError::NotListed) => continue,
            Err(_) => return None,
        }
    }
    None
}

// ---- persistence ----

/// Write a file atomically (temp + rename) so a crash or a kill mid-write can't leave a half-written,
/// corrupt JSON behind — which `load_json` would silently discard, losing the user's watchlist/config.
fn write_atomic(path: &PathBuf, contents: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn persist_watchlist(state: &AppState) -> Result<(), String> {
    let watchlist = state.watchlist.lock().unwrap();
    let json = serde_json::to_string_pretty(&*watchlist).map_err(|e| e.to_string())?;
    write_atomic(&state.watchlist_path, &json)
}

fn persist_config(state: &AppState) -> Result<(), String> {
    let config = state.config.lock().unwrap();
    let json = serde_json::to_string_pretty(&*config).map_err(|e| e.to_string())?;
    write_atomic(&state.config_path, &json)
}

fn persist_history(state: &AppState) -> Result<(), String> {
    let history = state.history.lock().unwrap();
    let json = serde_json::to_string(&*history).map_err(|e| e.to_string())?;
    write_atomic(&state.history_path, &json)
}

fn persist_stash_cache(state: &AppState) -> Result<(), String> {
    let cache = state.stash_cache.lock().unwrap();
    let json = serde_json::to_string(&*cache).map_err(|e| e.to_string())?;
    write_atomic(&state.stash_cache_path, &json)
}

fn persist_portfolio_cache(state: &AppState) -> Result<(), String> {
    let cache = state.portfolio_cache.lock().unwrap();
    let json = serde_json::to_string(&*cache).map_err(|e| e.to_string())?;
    write_atomic(&state.portfolio_cache_path, &json)
}

fn load_json<T: serde::de::DeserializeOwned + Default>(path: &PathBuf) -> T {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let steam = Arc::new(SteamClient::new());

            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir).ok();
            let watchlist_path = data_dir.join("watchlist.json");
            let config_path = data_dir.join("config.json");
            let history_path = data_dir.join("history.json");
            let stash_cache_path = data_dir.join("stash_cache.json");
            let portfolio_cache_path = data_dir.join("portfolio_cache.json");

            app.manage(AppState {
                market: Arc::new(SteamMarketSource::new(steam.clone())),
                catalog: SteamCatalog::new(steam.clone()),
                inventory: SteamInventory::new(steam.clone()),
                watchlist: Mutex::new(load_json(&watchlist_path)),
                quotes: Mutex::new(HashMap::new()),
                config: Mutex::new(load_json(&config_path)),
                history: Mutex::new(load_json(&history_path)),
                limits: Box::new(entitlement::DefaultLimits),
                stash_cache: Mutex::new(load_json(&stash_cache_path)),
                portfolio_cache: Mutex::new(load_json(&portfolio_cache_path)),
                watchlist_path,
                config_path,
                history_path,
                stash_cache_path,
                portfolio_cache_path,
            });

            let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("tbwatcher")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            catalog_search,
            watchlist_get,
            watchlist_add,
            watchlist_remove,
            watchlist_refresh,
            price_history,
            steamid_get,
            steamid_set,
            portfolio_get,
            alerts_list,
            alerts_set,
            alerts_remove,
            alerts_check,
            stash_advisor,
            entitlement_get,
            entitlement_set_tier
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
