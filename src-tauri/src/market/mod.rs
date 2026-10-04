//! Market data layer — the swappable seam (DESIGN.md §4). Every Steam HTTP call goes through one
//! shared `SteamClient` whose rate-limit governor paces it (the tolerated-tool boundary). v1 is
//! client-direct; a backend proxy can later satisfy `MarketDataSource`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

pub const APPID: &str = "3678970";

/// A point-in-time market quote for one item. Prices stay as Steam's formatted strings (currency
/// symbol included) and the frontend shows them verbatim; numeric parsing for fee math arrives with
/// the Flip pillar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketQuote {
    pub market_hash_name: String,
    pub lowest_price: Option<String>,
    pub median_price: Option<String>,
    pub volume: Option<u64>,
    pub fetched_at_ms: u64,
}

#[derive(Debug)]
pub enum MarketError {
    Http(String),
    RateLimited,
    NotListed,
}

impl std::fmt::Display for MarketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarketError::Http(e) => write!(f, "network error: {e}"),
            MarketError::RateLimited => write!(f, "rate limited by Steam"),
            MarketError::NotListed => write!(f, "no market listing"),
        }
    }
}

/// One HTTP client + one governor shared by every Steam call (quotes, search, …), so all traffic is
/// paced globally.
pub struct SteamClient {
    http: reqwest::Client,
    governor: RateLimiter,
}

impl SteamClient {
    /// `spacing_ms` is the minimum gap between Steam calls (quotes/search/inventory) — the rate-limit
    /// governor, supplied by the runtime `Config` (backend-tunable).
    pub fn new(spacing_ms: u64) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("tbwatcher/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("failed to build HTTP client");
        Self {
            http,
            governor: RateLimiter::new(Duration::from_millis(spacing_ms)),
        }
    }

    pub fn get(&self, url: &str) -> reqwest::RequestBuilder {
        self.http.get(url)
    }

    /// Pace and send. 429 surfaces as `RateLimited`; other statuses are returned for the caller to
    /// inspect (e.g. the inventory endpoint's 403 = private).
    pub async fn send(&self, req: reqwest::RequestBuilder) -> Result<reqwest::Response, MarketError> {
        self.governor.acquire().await;
        let resp = req.send().await.map_err(|e| MarketError::Http(e.to_string()))?;
        if resp.status().as_u16() == 429 {
            return Err(MarketError::RateLimited);
        }
        Ok(resp)
    }

    /// Pace, send, and deserialize JSON.
    pub async fn send_json<T: DeserializeOwned>(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<T, MarketError> {
        let resp = self.send(req).await?;
        resp.json().await.map_err(|e| MarketError::Http(e.to_string()))
    }
}

/// Serializes outbound Steam calls with a minimum spacing. Holding the lock across the sleep enforces
/// the gap globally (DESIGN.md §4).
pub struct RateLimiter {
    min_interval: Duration,
    last: Mutex<Option<Instant>>,
}

impl RateLimiter {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last: Mutex::new(None),
        }
    }

    pub async fn acquire(&self) {
        let mut last = self.last.lock().await;
        if let Some(prev) = *last {
            let elapsed = prev.elapsed();
            if elapsed < self.min_interval {
                tokio::time::sleep(self.min_interval - elapsed).await;
            }
        }
        *last = Some(Instant::now());
    }
}

/// The seam: any market data source (client-direct today, backend later) implements this.
#[async_trait::async_trait]
pub trait MarketDataSource: Send + Sync {
    async fn quote(&self, market_hash_name: &str) -> Result<MarketQuote, MarketError>;

    /// Price many hashes in one shot. The default fans out to `quote` per item — correct for a source with
    /// no native batch (direct Steam, each call still governed). A cache-backed source (the backend) should
    /// override this with a single request; that's what turns a 200-item stash scan into one call instead of
    /// tripping our own rate limit. Hashes the source can't price are simply absent from the map.
    async fn quote_batch(&self, hashes: &[String]) -> HashMap<String, MarketQuote> {
        let mut out = HashMap::with_capacity(hashes.len());
        for hash in hashes {
            if let Ok(quote) = self.quote(hash).await {
                out.insert(hash.clone(), quote);
            }
        }
        out
    }
}

/// Client-direct source over Steam's public `priceoverview` (no login).
pub struct SteamMarketSource {
    steam: Arc<SteamClient>,
}

impl SteamMarketSource {
    pub fn new(steam: Arc<SteamClient>) -> Self {
        Self { steam }
    }
}

#[async_trait::async_trait]
impl MarketDataSource for SteamMarketSource {
    async fn quote(&self, market_hash_name: &str) -> Result<MarketQuote, MarketError> {
        let req = self
            .steam
            .get("https://steamcommunity.com/market/priceoverview/")
            .query(&[
                ("appid", APPID),
                ("currency", "1"), // USD; made configurable later
                ("market_hash_name", market_hash_name),
            ]);
        let raw: PriceOverview = self.steam.send_json(req).await?;
        raw.into_quote(market_hash_name).ok_or(MarketError::NotListed)
    }
}

/// Raw `priceoverview` response. Price/volume fields are absent when the item has no live listings.
#[derive(Debug, Deserialize)]
struct PriceOverview {
    success: bool,
    lowest_price: Option<String>,
    median_price: Option<String>,
    volume: Option<String>,
}

impl PriceOverview {
    fn into_quote(self, market_hash_name: &str) -> Option<MarketQuote> {
        if !self.success {
            return None;
        }
        Some(MarketQuote {
            market_hash_name: market_hash_name.to_string(),
            lowest_price: self.lowest_price,
            median_price: self.median_price,
            volume: self.volume.as_deref().and_then(parse_volume),
            fetched_at_ms: now_ms(),
        })
    }
}

/// Steam writes volume as a grouped string, e.g. "122,907".
fn parse_volume(raw: &str) -> Option<u64> {
    raw.replace(',', "").parse().ok()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_grouped_volume() {
        assert_eq!(parse_volume("122,907"), Some(122_907));
        assert_eq!(parse_volume("9"), Some(9));
        assert_eq!(parse_volume("nope"), None);
    }

    #[test]
    fn maps_a_recorded_priceoverview_into_a_quote() {
        let raw: PriceOverview =
            serde_json::from_str(include_str!("../../fixtures/priceoverview-soulstone.json"))
                .expect("fixture should parse");
        let quote = raw
            .into_quote("Soulstone - Torment")
            .expect("a successful response becomes a quote");
        assert_eq!(quote.lowest_price.as_deref(), Some("$0.06"));
        assert_eq!(quote.median_price.as_deref(), Some("$0.07"));
        assert_eq!(quote.volume, Some(122_907));
    }

    #[test]
    fn unsuccessful_response_is_not_a_quote() {
        let raw = PriceOverview {
            success: false,
            lowest_price: None,
            median_price: None,
            volume: None,
        };
        assert!(raw.into_quote("Whatever").is_none());
    }

    /// A source that prices only the hashes it knows, erroring on the rest — to check the default
    /// `quote_batch` fan-out returns exactly the priced ones and drops the misses.
    struct FakeSource;

    #[async_trait::async_trait]
    impl MarketDataSource for FakeSource {
        async fn quote(&self, hash: &str) -> Result<MarketQuote, MarketError> {
            if hash == "Known" {
                Ok(MarketQuote {
                    market_hash_name: hash.to_string(),
                    lowest_price: Some("$1.00".into()),
                    median_price: None,
                    volume: None,
                    fetched_at_ms: 0,
                })
            } else {
                Err(MarketError::NotListed)
            }
        }
    }

    #[tokio::test]
    async fn default_quote_batch_keeps_priced_and_drops_misses() {
        let out = FakeSource
            .quote_batch(&["Known".to_string(), "Missing".to_string()])
            .await;
        assert_eq!(out.len(), 1);
        assert_eq!(out["Known"].lowest_price.as_deref(), Some("$1.00"));
        assert!(!out.contains_key("Missing"));
    }
}
