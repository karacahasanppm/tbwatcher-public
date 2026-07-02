//! Item catalog via Steam's live market search (DESIGN.md §10). v1 uses Steam's `query` index for
//! discovery rather than enumerating every listing: `search/render` caps at 10 results/page, so the
//! ~743 tradeable items would be ~75 governed requests. Live search is lighter and politer; full
//! offline enumeration is deferred. The seam stays so a cached/enumerated provider can replace it.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::market::{MarketError, SteamClient, APPID};

/// One marketable item, as the UI and the persisted watchlist need it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogItem {
    pub market_hash_name: String,
    pub name: String,
    pub icon_url: String,
    pub item_type: String,
    pub name_color: Option<String>,
}

pub struct SteamCatalog {
    steam: Arc<SteamClient>,
}

impl SteamCatalog {
    pub fn new(steam: Arc<SteamClient>) -> Self {
        Self { steam }
    }

    pub async fn search(&self, query: &str, count: u32) -> Result<Vec<CatalogItem>, MarketError> {
        let count = count.to_string();
        let req = self
            .steam
            .get("https://steamcommunity.com/market/search/render/")
            .query(&[
                ("appid", APPID),
                ("norender", "1"),
                ("count", count.as_str()),
                ("query", query),
            ]);
        let resp: SearchResponse = self.steam.send_json(req).await?;
        Ok(resp.results.into_iter().map(CatalogItem::from).collect())
    }
}

#[derive(Deserialize)]
struct SearchResponse {
    results: Vec<SearchResult>,
}

#[derive(Deserialize)]
struct SearchResult {
    name: String,
    asset_description: AssetDescription,
}

#[derive(Deserialize)]
struct AssetDescription {
    icon_url: String,
    #[serde(rename = "type")]
    item_type: String,
    name_color: Option<String>,
    market_hash_name: String,
}

impl From<SearchResult> for CatalogItem {
    fn from(r: SearchResult) -> Self {
        CatalogItem {
            market_hash_name: r.asset_description.market_hash_name,
            name: r.name,
            icon_url: r.asset_description.icon_url,
            item_type: r.asset_description.item_type,
            name_color: r.asset_description.name_color,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_recorded_search_into_catalog_items() {
        let resp: SearchResponse =
            serde_json::from_str(include_str!("../../fixtures/search-soulstone.json"))
                .expect("fixture should parse");
        let items: Vec<CatalogItem> = resp.results.into_iter().map(CatalogItem::from).collect();

        assert_eq!(items.len(), 4);
        assert!(items
            .iter()
            .any(|i| i.market_hash_name == "Soulstone - Torment"));
        assert_eq!(items[0].item_type, "Soulstone");
    }
}
