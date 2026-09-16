//! Public Steam inventory → marketable holdings (DESIGN.md §6). No login: reads
//! `/inventory/<steamid>/3678970/2`. A private inventory returns HTTP 403 → `Private`.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::market::{MarketError, SteamClient, APPID};

/// One marketable line in the user's inventory, with the total count across stacked assets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryHolding {
    pub market_hash_name: String,
    pub name: String,
    pub icon_url: String,
    pub count: u64,
}

#[derive(Debug)]
pub enum InventoryError {
    Private,
    RateLimited,
    Http(String),
}

impl std::fmt::Display for InventoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InventoryError::Private => write!(f, "inventory is private"),
            InventoryError::RateLimited => write!(f, "rate limited by Steam"),
            InventoryError::Http(e) => write!(f, "network error: {e}"),
        }
    }
}

impl From<MarketError> for InventoryError {
    fn from(e: MarketError) -> Self {
        match e {
            MarketError::RateLimited => InventoryError::RateLimited,
            MarketError::Http(s) => InventoryError::Http(s),
            MarketError::NotListed => InventoryError::Http("unexpected response".into()),
        }
    }
}

/// Reads a public Steam inventory through the shared governed client.
pub struct SteamInventory {
    steam: Arc<SteamClient>,
}

impl SteamInventory {
    pub fn new(steam: Arc<SteamClient>) -> Self {
        Self { steam }
    }

    pub async fn holdings(&self, steam_id: &str) -> Result<Vec<InventoryHolding>, InventoryError> {
        let url = format!("https://steamcommunity.com/inventory/{steam_id}/{APPID}/2");
        let req = self.steam.get(&url).query(&[("l", "english"), ("count", "2000")]);
        let resp = self.steam.send(req).await?;
        match resp.status().as_u16() {
            200 => {}
            403 => return Err(InventoryError::Private),
            other => return Err(InventoryError::Http(format!("status {other}"))),
        }
        let raw: InventoryResponse = resp
            .json()
            .await
            .map_err(|e| InventoryError::Http(e.to_string()))?;
        Ok(aggregate(raw))
    }
}

#[derive(Deserialize)]
struct InventoryResponse {
    #[serde(default)]
    assets: Vec<Asset>,
    #[serde(default)]
    descriptions: Vec<Description>,
}

#[derive(Deserialize)]
struct Asset {
    classid: String,
    instanceid: String,
    amount: String,
}

#[derive(Deserialize)]
struct Description {
    classid: String,
    instanceid: String,
    market_hash_name: String,
    name: String,
    icon_url: String,
    marketable: i32,
}

/// Aggregate stacked assets into per-item counts, keeping only marketable items (only those carry a
/// market price). Sorted by count desc, then name.
fn aggregate(raw: InventoryResponse) -> Vec<InventoryHolding> {
    let descs: HashMap<(&str, &str), &Description> = raw
        .descriptions
        .iter()
        .map(|d| ((d.classid.as_str(), d.instanceid.as_str()), d))
        .collect();

    let mut by_item: HashMap<&str, InventoryHolding> = HashMap::new();
    for asset in &raw.assets {
        let Some(desc) = descs.get(&(asset.classid.as_str(), asset.instanceid.as_str())) else {
            continue;
        };
        if desc.marketable != 1 {
            continue;
        }
        let amount: u64 = asset.amount.parse().unwrap_or(0);
        by_item
            .entry(desc.market_hash_name.as_str())
            .or_insert_with(|| InventoryHolding {
                market_hash_name: desc.market_hash_name.clone(),
                name: desc.name.clone(),
                icon_url: desc.icon_url.clone(),
                count: 0,
            })
            .count += amount;
    }

    let mut holdings: Vec<InventoryHolding> = by_item.into_values().collect();
    holdings.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    holdings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregates_marketable_holdings_from_a_recorded_inventory() {
        let raw: InventoryResponse =
            serde_json::from_str(include_str!("../../fixtures/inventory-sample.json"))
                .expect("fixture should parse");
        let holdings = aggregate(raw);

        // Bound Trinket is non-marketable → excluded.
        assert_eq!(holdings.len(), 2);
        let soul = holdings
            .iter()
            .find(|h| h.market_hash_name == "Soulstone - Torment")
            .expect("soulstone present");
        assert_eq!(soul.count, 8); // 5 + 3 stacked
        assert!(holdings
            .iter()
            .any(|h| h.market_hash_name == "Phantom Emerald" && h.count == 1));
        assert!(holdings.iter().all(|h| h.market_hash_name != "Bound Trinket"));
    }
}
