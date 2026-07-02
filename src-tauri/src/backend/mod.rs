//! Client for the personal-website app-backend (hasankaraca.com.tr). See docs/BACKEND_API.md for the
//! contract. Gated OFF by default (no base URL) → the app uses its local/Steam sources exactly as today.
//! When enabled, the Remote* sources fetch from the backend and **fall back to the local/Steam sources on
//! any failure**, so the app degrades gracefully whether the backend is missing, slow, or cold-cached.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::market::{MarketDataSource, MarketError, MarketQuote};

/// The backend base URL. Reads `TBW_BACKEND_URL` first (for staging against a local Laravel instance),
/// else this compile default — the live app-backend. On any backend failure the Remote* sources fall back
/// to direct Steam, so a missing/slow/cold backend never breaks the app.
const DEFAULT_BASE_URL: Option<&str> = Some("https://hasankaraca.com.tr/api/v1");

/// The configured backend base URL, if any. `None` disables every Remote* source.
pub fn base_url() -> Option<String> {
    std::env::var("TBW_BACKEND_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| DEFAULT_BASE_URL.map(String::from))
}

/// HTTP client for the app-backend. Public read-only endpoints, no auth token (see the contract).
pub struct BackendClient {
    base: String,
    http: reqwest::Client,
}

impl BackendClient {
    pub fn new(base: String) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("tbwatcher/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(6))
            .build()
            .expect("failed to build HTTP client");
        Self {
            base: base.trim_end_matches('/').to_string(),
            http,
        }
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ()> {
        self.http
            .get(format!("{}/{path}", self.base))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| ())?
            .json()
            .await
            .map_err(|_| ())
    }

    pub async fn get_version(&self) -> Result<VersionInfo, ()> {
        self.get_json("apps/tbwatcher/version").await
    }

    pub async fn get_config(&self) -> Result<ConfigResponse, ()> {
        self.get_json("apps/tbwatcher/config").await
    }

    /// Top price movers over a window, ranked by absolute % change (backend-computed).
    pub async fn get_movers(&self, limit: u32) -> Result<Vec<Mover>, ()> {
        let resp: MoversResponse = self.get_json(&format!("apps/tbwatcher/movers?limit={limit}")).await?;
        Ok(resp.movers)
    }

    /// Batch price lookup from the backend cache — one call prices a whole stash without touching Steam.
    pub async fn post_prices(&self, items: &[String]) -> Result<HashMap<String, BackendPrice>, ()> {
        let resp: PricesResponse = self
            .http
            .post(format!("{}/apps/tbwatcher/prices", self.base))
            .json(&serde_json::json!({ "items": items }))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| ())?
            .json()
            .await
            .map_err(|_| ())?;
        Ok(resp.prices)
    }

    /// Batch price history per item since `since_ms` — the chart's baseline (the client adds its own
    /// finer points on top).
    pub async fn post_history(
        &self,
        items: &[String],
        since_ms: u64,
    ) -> Result<HashMap<String, Vec<HistoryPoint>>, ()> {
        let resp: HistoryResponse = self
            .http
            .post(format!("{}/apps/tbwatcher/history", self.base))
            .json(&serde_json::json!({ "items": items, "since_ms": since_ms }))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| ())?
            .json()
            .await
            .map_err(|_| ())?;
        Ok(resp.history)
    }
}

#[derive(Deserialize)]
struct HistoryResponse {
    history: HashMap<String, Vec<HistoryPoint>>,
}

#[derive(Deserialize)]
struct MoversResponse {
    movers: Vec<Mover>,
}

/// One ranked price mover (backend-computed over the whole cached market's history).
#[derive(Serialize, Deserialize)]
pub struct Mover {
    pub market_hash_name: String,
    pub old_price: String,
    pub lowest_price: String,
    pub volume: Option<u64>,
    pub change_pct: f64,
}

/// One backend price-history point (the market hash is the map key).
#[derive(Deserialize)]
pub struct HistoryPoint {
    pub fetched_at_ms: u64,
    pub lowest_price: Option<String>,
}

/// Matches the backend's `AppVersionResource` (release_notes / download_url); other fields (channel,
/// published_at) are ignored.
#[derive(Deserialize)]
pub struct VersionInfo {
    pub version: String,
    #[serde(default)]
    pub release_notes: Option<String>,
    #[serde(default)]
    pub download_url: Option<String>,
}

/// The backend serves per-tier limits too, but this build is single-tier (no metering), so we take only
/// `config` and let serde ignore the rest.
#[derive(Deserialize)]
pub struct ConfigResponse {
    pub config: Config,
}

#[derive(Deserialize)]
struct PricesResponse {
    prices: HashMap<String, BackendPrice>,
}

/// One cached price from the backend. The market hash is the map key, so it isn't repeated here.
#[derive(Deserialize)]
pub struct BackendPrice {
    pub lowest_price: Option<String>,
    pub median_price: Option<String>,
    pub volume: Option<u64>,
    pub fetched_at_ms: u64,
}

impl BackendPrice {
    fn into_quote(self, market_hash_name: &str) -> MarketQuote {
        MarketQuote {
            market_hash_name: market_hash_name.to_string(),
            lowest_price: self.lowest_price,
            median_price: self.median_price,
            volume: self.volume,
            fetched_at_ms: self.fetched_at_ms,
        }
    }
}

/// Prices via the backend cache, falling back to direct Steam. Slots in behind `Arc<dyn MarketDataSource>`,
/// so `quote_best`, the persistent price cache, and the retry logic all keep working unchanged.
pub struct RemoteMarketSource {
    client: Arc<BackendClient>,
    fallback: Arc<dyn MarketDataSource>,
}

impl RemoteMarketSource {
    pub fn new(client: Arc<BackendClient>, fallback: Arc<dyn MarketDataSource>) -> Self {
        Self { client, fallback }
    }
}

#[async_trait::async_trait]
impl MarketDataSource for RemoteMarketSource {
    async fn quote(&self, market_hash_name: &str) -> Result<MarketQuote, MarketError> {
        // Single-item batch call; the backend serves from cache, so no Steam rate limit applies.
        if let Ok(mut prices) = self.client.post_prices(&[market_hash_name.to_string()]).await {
            if let Some(price) = prices.remove(market_hash_name) {
                return Ok(price.into_quote(market_hash_name));
            }
        }
        // Backend unreachable or cold-cached for this item → price it directly from Steam.
        self.fallback.quote(market_hash_name).await
    }
}
