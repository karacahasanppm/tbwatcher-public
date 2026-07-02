mod backend;
mod catalog;
mod config;
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
use config::ConfigSource;
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
    /// Backend-tunable operational config (pacing, cooldowns, cadences, caps) — distinct from the user's
    /// persisted `AppConfig` above. Resolved from a `ConfigSource` at startup (local today, remote later).
    settings: config::Config,
    /// The app-backend client (None when the backend is off) — used to fetch price-history baselines.
    backend: Option<Arc<backend::BackendClient>>,
    stash_cache: Mutex<gate::ReloadCache<SellAdvisor>>,
    portfolio_cache: Mutex<gate::ReloadCache<Portfolio>>,
    watchlist_path: PathBuf,
    config_path: PathBuf,
    history_path: PathBuf,
    stash_cache_path: PathBuf,
    portfolio_cache_path: PathBuf,
    quotes_path: PathBuf,
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
fn watchlist_add(state: State<'_, AppState>, item: CatalogItem) -> Result<(), String> {
    {
        let mut watchlist = state.watchlist.lock().unwrap();
        if watchlist
            .iter()
            .any(|i| i.market_hash_name == item.market_hash_name)
        {
            return Ok(()); // already watched — no-op
        }
        watchlist.push(item);
    }
    persist_watchlist(&state)
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
                history::record(series, history::PricePoint { t_ms: q.fetched_at_ms, cents }, &retention(&state));
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

/// The price-history retention bounds from the runtime config, as the `history` module wants them.
fn retention(state: &AppState) -> history::Retention {
    history::Retention {
        min_gap_ms: state.settings.history_min_gap_ms,
        max_age_ms: state.settings.history_max_age_ms,
        max_points: state.settings.history_max_points as usize,
    }
}

/// The watchlist chart's series: the backend's price-history baseline (immediate, even for a just-added
/// item) merged with the client's own finer self-accumulated points, over the `chart_window_ms` window,
/// downsampled to `chart_points` for readability. Backend off ⇒ just the local points.
#[tauri::command]
async fn price_history(
    state: State<'_, AppState>,
    market_hash_name: String,
) -> Result<Vec<history::PricePoint>, String> {
    let since = gate::now_ms().saturating_sub(state.settings.chart_window_ms);

    // Local self-accumulated points within the window (lock dropped before any await).
    let mut points: Vec<history::PricePoint> = {
        let history = state.history.lock().unwrap();
        history
            .get(&market_hash_name)
            .map(|s| s.iter().copied().filter(|p| p.t_ms >= since).collect())
            .unwrap_or_default()
    };

    // Backend baseline merged in, if the backend is on.
    if let Some(client) = &state.backend {
        if let Ok(map) = client.post_history(std::slice::from_ref(&market_hash_name), since).await {
            if let Some(series) = map.get(&market_hash_name) {
                for bp in series {
                    if let Some(cents) = bp.lowest_price.as_deref().and_then(money::parse_usd_cents) {
                        points.push(history::PricePoint { t_ms: bp.fetched_at_ms, cents });
                    }
                }
            }
        }
    }

    points.sort_by_key(|p| p.t_ms);
    Ok(downsample(points, state.settings.chart_points as usize))
}

/// Reduce a time-sorted series to at most `target` points by keeping the last point in each of `target`
/// even time-buckets — so a dense recent stretch and a sparse baseline both read as one clean line.
fn downsample(points: Vec<history::PricePoint>, target: usize) -> Vec<history::PricePoint> {
    if target == 0 || points.len() <= target {
        return points;
    }
    let (min_t, max_t) = (points.first().unwrap().t_ms, points.last().unwrap().t_ms);
    let span = max_t.saturating_sub(min_t).max(1);
    let mut out: Vec<history::PricePoint> = Vec::with_capacity(target);
    let mut last_bucket = usize::MAX;
    for p in points {
        let bucket = (((p.t_ms - min_t) as u128 * target as u128) / span as u128).min(target as u128 - 1) as usize;
        if bucket == last_bucket {
            *out.last_mut().unwrap() = p;
        } else {
            out.push(p);
            last_bucket = bucket;
        }
    }
    out
}

/// The runtime operational config, so the frontend reads its cadences (poll/sync) from the same swappable
/// source as the backend — never hardcoding them.
#[tauri::command]
fn config_get(state: State<'_, AppState>) -> config::Config {
    state.settings.clone()
}

/// An available update, for a respectful in-app "update available" note (no auto-updater).
#[derive(Serialize)]
struct UpdateInfo {
    version: String,
    notes: Option<String>,
    url: Option<String>,
}

/// True when dotted-numeric `candidate` is a higher version than `current` (e.g. "0.2.0" > "0.1.6").
fn is_newer(candidate: &str, current: &str) -> bool {
    let parts = |s: &str| -> Vec<u32> { s.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect() };
    parts(candidate) > parts(current)
}

/// Ask the backend for the latest release; return it only if it's newer than this build. `None` when the
/// backend is off/unreachable or we're already current — the note simply doesn't show.
#[tauri::command]
async fn backend_version_check() -> Option<UpdateInfo> {
    let client = backend::BackendClient::new(backend::base_url()?);
    let latest = client.get_version().await.ok()?;
    is_newer(&latest.version, env!("CARGO_PKG_VERSION")).then_some(UpdateInfo {
        version: latest.version,
        notes: latest.release_notes,
        url: latest.download_url,
    })
}

/// Top price movers from the backend (ranked over the whole market's history). Empty when the backend is
/// off or unreachable — the Movers view just shows nothing then.
#[tauri::command]
async fn movers_get(state: State<'_, AppState>) -> Result<Vec<backend::Mover>, String> {
    match state.backend.clone() {
        Some(client) => Ok(client.get_movers(50).await.unwrap_or_default()),
        None => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod version_tests {
    use super::is_newer;

    #[test]
    fn compares_dotted_numeric_versions() {
        assert!(is_newer("0.2.0", "0.1.6"));
        assert!(is_newer("0.10.0", "0.9.0")); // numeric, not lexical
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.6", "0.1.6")); // same → no update
        assert!(!is_newer("0.1.5", "0.1.6")); // older
    }
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

// Portfolio reload windows come from the runtime config: `portfolio_cooldown_secs` serves saved data after
// a success (so we don't re-hit the strict inventory endpoint); `rate_limit_backoff_secs` is the longer
// back-off after a 429. Both are persisted as wall-clock deadlines, so a restart can't reset them.

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
                    state.settings.portfolio_cooldown_secs,
                ),
                InventoryError::RateLimited => (
                    "Steam is rate-limiting us.".into(),
                    state.settings.rate_limit_backoff_secs,
                ),
                InventoryError::Http(detail) => (
                    format!("Couldn't reach Steam ({detail})."),
                    state.settings.portfolio_cooldown_secs,
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
        next_reload_secs: state.settings.portfolio_cooldown_secs,
        stale: false,
    };
    *state.portfolio_cache.lock().unwrap() = gate::ReloadCache {
        fetched_at_ms: now,
        locked_until_ms: now + state.settings.portfolio_cooldown_secs * 1000,
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
) -> Result<(), String> {
    let target_cents = money::parse_price_input(&target_price).ok_or("invalid target price")?;
    {
        let mut config = state.config.lock().unwrap();
        match config
            .alerts
            .iter_mut()
            .find(|a| a.market_hash_name == market_hash_name && a.direction == direction)
        {
            Some(existing) => {
                existing.target_cents = target_cents;
                existing.name = name;
                existing.triggered = false;
            }
            None => {
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
    persist_config(&state)
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

// Stash tunables come from the runtime config: `sell_slots` is how many Market listing slots the advisor
// ranks toward (TBH lists at most 4 at once — 4 concurrent listings, 8h relist — so the decision is *which
// item earns a slot*, ranked by unit value); `stash_rescan_cooldown_secs` throttles the (per-item-quoted)
// rescan, serving the persisted last result in between. This section will later anchor subscription metering.

/// One rankable stash line: an item the player can list, with its current unit and line value.
/// Prices are gross (pre-fee); the post-fee seller take arrives with the Flip pillar.
#[derive(Clone, Serialize, Deserialize)]
struct SellEntry {
    market_hash_name: String,
    count: u64,
    lowest_price: Option<String>,
    line_value_text: Option<String>,
    /// Stash-filter class — "Wearable" | "Decoration" | "Engraving" | "Inscription" (save::category).
    category: String,
    /// The game's own sprite path ("/game/…") for the visual stash grid (served from static/game/).
    icon: String,
    /// Rarity grade (Common…Cosmic) — the visual stash grid tints the slot by it, game-style.
    grade: String,
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
                slot_limit: state.settings.sell_slots as usize,
                total_value_text: None,
                error: Some(e.to_string()),
                as_of_ms: 0,
                next_scan_secs: 0,
            });
        }
    };

    // Resolve the marketable items up front so we can report an accurate total to value. Each job also
    // carries its stash-filter category and game sprite, resolved from the same item_key (both Some for
    // everything bridged).
    let jobs: Vec<(Vec<String>, u64, &'static str, &'static str, &'static str)> = stash
        .into_iter()
        .filter_map(|sc| {
            let candidates = save::market_hashes(sc.item_key);
            // Skip non-tradeable items (gear below Legendary, non-listable types, unknown ids).
            if candidates.is_empty() {
                return None;
            }
            let category = save::category(sc.item_key).unwrap_or("Wearable");
            let icon = save::icon(sc.item_key).unwrap_or("");
            let grade = save::grade(sc.item_key).unwrap_or("");
            Some((candidates, sc.count, category, icon, grade))
        })
        .collect();

    let total = jobs.len();
    let _ = on_progress.send(StashProgress { done: 0, total });

    // Distinct item ids can resolve to one market hash (variant ids of the same gear) — group them.
    // Variants of one item share a category, so the first-seen one is authoritative for the group.
    struct Agg {
        count: u64,
        lowest_price: Option<String>,
        category: &'static str,
        icon: &'static str,
        grade: &'static str,
    }
    let mut by_hash: HashMap<String, Agg> = HashMap::new();
    for (done, (candidates, count, category, icon, grade)) in jobs.into_iter().enumerate() {
        let quote = quote_best(&state, &candidates).await;
        let (display, price) = match quote {
            Some(q) => (q.market_hash_name, q.lowest_price),
            None => (candidates.into_iter().next().unwrap(), None),
        };
        let agg = by_hash.entry(display).or_insert(Agg {
            count: 0,
            lowest_price: None,
            category,
            icon,
            grade,
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
                    category: agg.category.to_string(),
                    icon: agg.icon.to_string(),
                    grade: agg.grade.to_string(),
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
        slot_limit: state.settings.sell_slots as usize,
        total_value_text: Some(money::format_usd_cents(total_cents)),
        error: None,
        as_of_ms: now,
        next_scan_secs: state.settings.stash_rescan_cooldown_secs,
    };
    *state.stash_cache.lock().unwrap() = gate::ReloadCache {
        fetched_at_ms: now,
        locked_until_ms: now + state.settings.stash_rescan_cooldown_secs * 1000,
        payload: Some(result.clone()),
    };
    persist_stash_cache(&state)?;
    // Persist the freshly-filled quote cache so last-known prices survive a restart — that's what the next
    // scan falls back to when Steam throttles an item.
    persist_quotes(&state).ok();
    Ok(result)
}

/// Quote the first candidate hash that's actually listed. Reuses a *recent* cached price (dedups a scan +
/// cuts Steam calls); on a stale/missing entry it fetches fresh, pausing and retrying a rate-limit. If a
/// fetch is still throttled after retries, it falls back to the last known price we've stored — so a 429
/// shows the previous value, not a blank. Gear has two spellings ("… A" then plain); `NotListed` (or a
/// success-but-unlisted spelling) falls through to the next candidate.
async fn quote_best(state: &AppState, candidates: &[String]) -> Option<MarketQuote> {
    let now = gate::now_ms();
    let max_age = state.settings.quote_cache_max_age_ms;
    // 1. Reuse a fresh, priced cache entry.
    {
        let cache = state.quotes.lock().unwrap();
        for hash in candidates {
            if let Some(q) = cache.get(hash) {
                if q.lowest_price.is_some() && now.saturating_sub(q.fetched_at_ms) < max_age {
                    return Some(q.clone());
                }
            }
        }
    }
    // 2. Fetch fresh; a rate-limit is paused-and-retried rather than dropped. Only a *priced* quote updates
    //    the cache, so a transient "unlisted" reply can't erase a good last-known price.
    for hash in candidates {
        match quote_with_retry(state, hash).await {
            Ok(q) if q.lowest_price.is_some() => {
                state.quotes.lock().unwrap().insert(hash.clone(), q.clone());
                return Some(q);
            }
            Ok(_) | Err(MarketError::NotListed) => continue, // unlisted spelling — try the next
            Err(_) => break,                                 // gave up after retries → fall back below
        }
    }
    // 3. Fall back to the last known price (possibly stale) so a throttled item isn't shown blank.
    let cache = state.quotes.lock().unwrap();
    candidates
        .iter()
        .find_map(|hash| cache.get(hash).filter(|q| q.lowest_price.is_some()).cloned())
}

/// One governed quote, pausing and retrying if Steam rate-limits us (config `quote_max_retries` times,
/// `quote_retry_backoff_ms` apart). A non-429 result (success, NotListed, network) returns immediately.
async fn quote_with_retry(state: &AppState, hash: &str) -> Result<MarketQuote, MarketError> {
    let mut attempt = 0;
    loop {
        match state.market.quote(hash).await {
            Err(MarketError::RateLimited) if attempt < state.settings.quote_max_retries => {
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_millis(
                    state.settings.quote_retry_backoff_ms,
                ))
                .await;
            }
            other => return other,
        }
    }
}

/// Show (and focus) the large game-themed stash grid window. The window is defined in tauri.conf.json
/// (label "stash", hidden at startup) so it loads the app through the same proven path as the main window
/// — programmatically-built windows came up blank. Closing it only hides it (see the window-event handler),
/// so this always finds it. Showing/focusing triggers the grid's scan (it stays idle while hidden).
#[tauri::command]
fn open_stash_window(app: tauri::AppHandle) -> Result<(), String> {
    let w = app
        .get_webview_window("stash")
        .ok_or("stash window not found")?;
    w.show().map_err(|e| e.to_string())?;
    w.set_focus().map_err(|e| e.to_string())?;
    Ok(())
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

fn persist_quotes(state: &AppState) -> Result<(), String> {
    let quotes = state.quotes.lock().unwrap();
    let json = serde_json::to_string(&*quotes).map_err(|e| e.to_string())?;
    write_atomic(&state.quotes_path, &json)
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
        // Closing the stash window only hides it, so it persists (reopen = show) and never has to be
        // rebuilt programmatically (which came up blank). The main window closes normally → app exits.
        .on_window_event(|window, event| {
            if window.label() == "stash" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            // Optional app-backend (see backend/mod.rs + docs/BACKEND_API.md). When enabled, config/prices/
            // history come from the backend, each falling back to the local/Steam source on any failure.
            let backend = backend::base_url().map(|url| Arc::new(backend::BackendClient::new(url)));
            let resolved = backend
                .as_ref()
                .and_then(|c| tauri::async_runtime::block_on(c.get_config()).ok());
            let settings = resolved
                .map(|r| r.config)
                .unwrap_or_else(|| config::LocalConfig.config());

            let steam = Arc::new(SteamClient::new(settings.steam_call_spacing_ms));
            let steam_source: Arc<dyn MarketDataSource> = Arc::new(SteamMarketSource::new(steam.clone()));
            let market: Arc<dyn MarketDataSource> = match &backend {
                Some(c) => Arc::new(backend::RemoteMarketSource::new(c.clone(), steam_source)),
                None => steam_source,
            };

            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir).ok();
            let watchlist_path = data_dir.join("watchlist.json");
            let config_path = data_dir.join("config.json");
            let history_path = data_dir.join("history.json");
            let stash_cache_path = data_dir.join("stash_cache.json");
            let portfolio_cache_path = data_dir.join("portfolio_cache.json");
            let quotes_path = data_dir.join("price_cache.json");

            app.manage(AppState {
                market,
                catalog: SteamCatalog::new(steam.clone()),
                inventory: SteamInventory::new(steam.clone()),
                watchlist: Mutex::new(load_json(&watchlist_path)),
                quotes: Mutex::new(load_json(&quotes_path)),
                config: Mutex::new(load_json(&config_path)),
                history: Mutex::new(load_json(&history_path)),
                settings,
                backend,
                stash_cache: Mutex::new(load_json(&stash_cache_path)),
                portfolio_cache: Mutex::new(load_json(&portfolio_cache_path)),
                watchlist_path,
                config_path,
                history_path,
                stash_cache_path,
                portfolio_cache_path,
                quotes_path,
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
            config_get,
            backend_version_check,
            movers_get,
            steamid_get,
            steamid_set,
            portfolio_get,
            alerts_list,
            alerts_set,
            alerts_remove,
            alerts_check,
            stash_advisor,
            open_stash_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
