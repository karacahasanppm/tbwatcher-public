//! How fast the in-game inventory is filling, from periodic samples of its used-slot count — so a player about
//! to step away can see roughly when it will be full (after that, new loot is lost).

/// One reading of the inventory's used slots.
#[derive(Clone, Copy)]
pub struct Sample {
    pub t_ms: u64,
    pub used: u32,
}

/// The current filling run must span this long before a rate is shown; shorter is mostly noise.
const MIN_SPAN_MS: u64 = 20 * 60 * 1000;

/// Slots added per hour over the current filling run — the samples since the count last dropped (a
/// "Stash All" or alchemy starts a new run). `None` until that run spans `MIN_SPAN_MS`; 0.0 if nothing landed.
pub fn rate_per_hour(samples: &[Sample]) -> Option<f64> {
    let last = samples.last()?;
    let mut start = samples.len() - 1;
    while start > 0 && samples[start - 1].used <= samples[start].used {
        start -= 1;
    }
    let first = samples[start];
    let span = last.t_ms.saturating_sub(first.t_ms);
    if span < MIN_SPAN_MS {
        return None;
    }
    Some(f64::from(last.used - first.used) / (span as f64 / 3_600_000.0))
}

/// Hours until every open slot is used at `rate` slots/hour; `None` when it isn't filling.
pub fn hours_to_full(used: u32, open: u32, rate: f64) -> Option<f64> {
    (rate > 0.0).then(|| f64::from(open.saturating_sub(used)) / rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;
    fn s(min: u64, used: u32) -> Sample {
        Sample { t_ms: min * MIN, used }
    }

    #[test]
    fn needs_twenty_minutes_of_run_before_estimating() {
        assert_eq!(rate_per_hour(&[]), None);
        assert_eq!(rate_per_hour(&[s(0, 50), s(15, 53)]), None);
    }

    #[test]
    fn rate_is_slots_per_hour_over_the_run() {
        let r = rate_per_hour(&[s(0, 50), s(30, 56), s(60, 62)]).unwrap();
        assert!((r - 12.0).abs() < 1e-9);
    }

    #[test]
    fn a_cleanup_starts_a_new_run() {
        // Filled to 90, then "Stash All" dropped it to 20 — only the run since 20 counts.
        let r = rate_per_hour(&[s(0, 80), s(30, 90), s(40, 20), s(70, 26)]).unwrap();
        assert!((r - 12.0).abs() < 1e-9);
    }

    #[test]
    fn not_filling_means_no_eta() {
        assert_eq!(rate_per_hour(&[s(0, 40), s(45, 40)]), Some(0.0));
        assert_eq!(hours_to_full(59, 104, 0.0), None);
        assert_eq!(hours_to_full(59, 104, 9.0), Some(5.0));
    }
}
