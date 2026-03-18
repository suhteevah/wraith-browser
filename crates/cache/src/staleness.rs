//! Staleness policy engine.
//!
//! The cache doesn't use dumb fixed TTLs. It learns how fast content changes
//! per domain and adjusts automatically. If a domain's content barely changes
//! (e.g., docs.rust-lang.org), the TTL extends. If it changes constantly
//! (e.g., twitter.com), the TTL shrinks.
//!
//! Priority chain:
//! 1. Pinned pages → never stale
//! 2. Domain override TTL (user/agent set) → use that
//! 3. Domain computed TTL (learned from change frequency) → use that
//! 4. Content type default TTL → fallback

use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use crate::schema::ContentType;

/// Global staleness policy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StalenessPolicy {
    /// Minimum TTL — never consider content stale faster than this (seconds)
    pub min_ttl_secs: u64,

    /// Maximum TTL — always re-validate after this long (seconds)
    pub max_ttl_secs: u64,

    /// Factor for computing TTL from change frequency.
    /// If a page changes every N seconds, TTL = N * change_factor.
    /// E.g., changes every 3600s (1h), factor 0.5 → TTL = 1800s (30min)
    pub change_factor: f64,

    /// Number of observations needed before using computed TTL
    pub min_observations: u64,

    /// Whether to use conditional requests (If-None-Match, If-Modified-Since)
    /// when available — validates cache without re-downloading
    pub use_conditional_requests: bool,

    /// Maximum age for search result cache (seconds)
    pub search_max_age_secs: u64,

    /// Whether to prefetch pages that are about to go stale
    pub prefetch_near_stale: bool,

    /// Prefetch threshold: when remaining TTL is below this fraction, prefetch
    pub prefetch_threshold: f64,
}

impl Default for StalenessPolicy {
    fn default() -> Self {
        Self {
            min_ttl_secs: 60,                        // 1 minute minimum
            max_ttl_secs: 30 * 24 * 3600,            // 30 days maximum
            change_factor: 0.5,                       // TTL = half the change interval
            min_observations: 3,                      // Need 3 fetches before adaptive TTL kicks in
            use_conditional_requests: true,            // Always try ETag/If-Modified-Since
            search_max_age_secs: 6 * 3600,            // Search results: 6 hours
            prefetch_near_stale: false,                // Disabled by default (saves bandwidth)
            prefetch_threshold: 0.1,                   // Prefetch when <10% TTL remaining
        }
    }
}

impl StalenessPolicy {
    /// Compute adaptive TTL from observed change intervals.
    ///
    /// Given a list of intervals between content changes (in seconds),
    /// compute a reasonable TTL.
    #[instrument(skip(self, change_intervals), fields(domain = %domain, observations = change_intervals.len()))]
    pub fn compute_ttl(
        &self,
        domain: &str,
        content_type: ContentType,
        change_intervals: &[u64],
    ) -> u64 {
        // Not enough data → use content type default
        if (change_intervals.len() as u64) < self.min_observations {
            let default = content_type.default_ttl_secs();
            debug!(
                domain = %domain,
                observations = change_intervals.len(),
                min_needed = self.min_observations,
                ttl = default,
                "Insufficient data, using content type default TTL"
            );
            return default;
        }

        // Compute median change interval (more robust than mean)
        let mut sorted = change_intervals.to_vec();
        sorted.sort();
        let median = sorted[sorted.len() / 2];

        // Apply change factor
        let computed = (median as f64 * self.change_factor) as u64;

        // Clamp to min/max
        let clamped = computed.clamp(self.min_ttl_secs, self.max_ttl_secs);

        debug!(
            domain = %domain,
            median_change_interval = median,
            change_factor = self.change_factor,
            computed_ttl = computed,
            clamped_ttl = clamped,
            "Computed adaptive TTL"
        );

        clamped
    }

    /// Check if a cached entry should be prefetched (near-stale).
    pub fn should_prefetch(&self, age_secs: u64, ttl_secs: u64) -> bool {
        if !self.prefetch_near_stale {
            return false;
        }
        let remaining_fraction = 1.0 - (age_secs as f64 / ttl_secs as f64);
        remaining_fraction < self.prefetch_threshold
    }
}
