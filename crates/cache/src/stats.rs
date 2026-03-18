//! Cache statistics and health monitoring.

use tracing::info;

/// Report cache health and statistics to the agent.
/// This helps the agent decide when to cache vs fetch, and manages storage budget.
pub fn report_stats(store: &crate::KnowledgeStore) {
    if let Ok(stats) = store.stats() {
        info!(
            pages = stats.total_pages,
            searches = stats.total_searches,
            snapshots = stats.total_snapshots,
            domains = stats.total_domains,
            hits = stats.total_cache_hits,
            stale = stats.stale_pages,
            pinned = stats.pinned_pages,
            disk_mb = stats.total_disk_bytes / (1024 * 1024),
            "Cache statistics"
        );
    }
}
