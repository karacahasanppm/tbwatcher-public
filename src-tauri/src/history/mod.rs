//! Self-accumulated local price history (PLAN.md Phase 5). We sample the quotes the watchlist already
//! polls and keep a small rolling series per item, so each watchlist item gets a price-fluctuation chart
//! without a login, a backend, or scanning the catalog. **Not** Steam `pricehistory` (that's login-gated
//! — the heavier Phase-7 backend job). A series therefore starts when the item is added and fills in over
//! time.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Don't fatten the series during flat stretches: an unchanged price within this gap is dropped, so a
/// long steady period costs one point per gap, while any price *change* is always recorded promptly.
const MIN_GAP_MS: u64 = 5 * 60 * 1000;
/// Retention window — older points are pruned so the store stays bounded regardless of uptime.
const MAX_AGE_MS: u64 = 30 * 24 * 60 * 60 * 1000;
/// Hard cap per item as a second guard (a very volatile item could otherwise grow unbounded inside the
/// age window). Oldest points drop first.
const MAX_POINTS: usize = 1500;

/// One observed price sample: epoch millis (from the quote's `fetched_at_ms`) + price in USD cents.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct PricePoint {
    pub t_ms: u64,
    pub cents: u64,
}

/// The whole store: market_hash_name → its rolling series. Persisted as JSON.
pub type Series = HashMap<String, Vec<PricePoint>>;

/// Append a sample to one item's series and prune it to the retention window. Pure (no I/O), so the
/// retention rules are unit-tested directly.
pub fn record(series: &mut Vec<PricePoint>, point: PricePoint) {
    if let Some(last) = series.last() {
        let recent = point.t_ms.saturating_sub(last.t_ms) < MIN_GAP_MS;
        if recent && last.cents == point.cents {
            return; // unchanged and recent — noise, not signal
        }
    }
    series.push(point);
    prune(series, point.t_ms);
}

fn prune(series: &mut Vec<PricePoint>, now_ms: u64) {
    let cutoff = now_ms.saturating_sub(MAX_AGE_MS);
    series.retain(|p| p.t_ms >= cutoff);
    if series.len() > MAX_POINTS {
        series.drain(0..series.len() - MAX_POINTS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t_ms: u64, cents: u64) -> PricePoint {
        PricePoint { t_ms, cents }
    }

    #[test]
    fn records_into_an_empty_series() {
        let mut s = Vec::new();
        record(&mut s, at(1_000, 6));
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].cents, 6);
    }

    #[test]
    fn drops_an_unchanged_recent_point_but_keeps_a_change() {
        let mut s = Vec::new();
        record(&mut s, at(0, 6));
        record(&mut s, at(60_000, 6)); // same price, 1 min later → dropped
        assert_eq!(s.len(), 1);
        record(&mut s, at(120_000, 7)); // price moved → kept even though recent
        assert_eq!(s.len(), 2);
        record(&mut s, at(120_000 + MIN_GAP_MS, 7)); // unchanged but past the gap → kept as a heartbeat
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn prunes_points_older_than_the_retention_window() {
        let mut s = vec![at(0, 5), at(1_000, 6)];
        record(&mut s, at(MAX_AGE_MS + 2_000, 9));
        assert_eq!(s.len(), 1); // the two old points fell outside the window
        assert_eq!(s[0].cents, 9);
    }

    #[test]
    fn caps_the_series_length_oldest_first() {
        let mut s: Vec<PricePoint> = (0..MAX_POINTS as u64)
            .map(|i| at(i * MIN_GAP_MS, i)) // every point a change, spaced past the gap
            .collect();
        let next_t = MAX_POINTS as u64 * MIN_GAP_MS;
        record(&mut s, at(next_t, 99_999));
        assert_eq!(s.len(), MAX_POINTS);
        assert_eq!(s.last().unwrap().cents, 99_999);
        assert_eq!(s[0].cents, 1); // the very first point was dropped
    }
}
