//! Entitlement & metered limits (PLAN.md Phase 6). The freemium boundary is structural and **data-driven**:
//! limits are values from a *swappable source* — a local default today, the admin panel / backend later
//! (DESIGN §4 seam, like `IMarketDataSource`) — never hardcoded into a feature. Until premium is
//! purchasable (a Merchant-of-Record is wired in), the free tier ships as a **bounded trial**: every
//! feature works, just capped, so the metering is real and the value is felt before the ask. Nothing is
//! ever fully locked. The real free/premium numbers are tuned later with evidence (INTENT §5.2) and
//! ultimately served by the admin panel.

use serde::{Deserialize, Serialize};

/// Free vs premium. Read from local config today; resolved by the admin panel / backend later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    #[default]
    Free,
    Premium,
}

/// The metered volume limits for a tier — the contract the admin panel will eventually serve. Grown one
/// dimension at a time as each gets a real enforcement point (no speculative fields).
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Limits {
    pub watchlist_max: u32,
    pub alerts_max: u32,
    /// How many stash items the Sell Advisor values per scan (the trial shows the first N; premium, all).
    pub stash_max: u32,
}

/// Where limits come from. Local defaults today; a remote/admin source swaps in later without touching
/// any feature code.
pub trait LimitsSource: Send + Sync {
    fn limits(&self, tier: Tier) -> Limits;
}

/// Built-in defaults — placeholders until the admin panel serves them. The free tier is a **bounded trial**
/// (small but genuinely usable caps); premium lifts them well past any normal use.
pub struct DefaultLimits;

impl LimitsSource for DefaultLimits {
    fn limits(&self, tier: Tier) -> Limits {
        match tier {
            // Trial: enough to feel the value (watch a handful of items, set a few alerts), bounded enough
            // that a heavy trader hits the wall and sees the upsell.
            Tier::Free => Limits {
                watchlist_max: 5,
                alerts_max: 3,
                stash_max: 10,
            },
            Tier::Premium => Limits {
                watchlist_max: 200,
                alerts_max: 100,
                stash_max: 1000,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_defaults_to_free() {
        assert_eq!(Tier::default(), Tier::Free);
    }

    #[test]
    fn free_is_a_bounded_trial_below_premium() {
        let s = DefaultLimits;
        let free = s.limits(Tier::Free);
        let premium = s.limits(Tier::Premium);
        // Trial is bounded but never fully locked (a feature with a zero cap would be "locked away").
        assert!(
            free.watchlist_max > 0 && free.alerts_max > 0 && free.stash_max > 0,
            "nothing is fully locked"
        );
        assert!(premium.watchlist_max > free.watchlist_max);
        assert!(premium.alerts_max > free.alerts_max);
        assert!(premium.stash_max > free.stash_max);
    }
}
