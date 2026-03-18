//! Query types and results for the knowledge store.

use serde::{Deserialize, Serialize};
use crate::schema::ContentType;

/// A query against the knowledge store.
#[derive(Debug, Clone)]
pub enum CacheQuery {
    /// Look up a specific URL
    Url(String),
    /// Full-text search across all cached content
    Fulltext { query: String, max_results: usize },
    /// Find pages by domain
    Domain { domain: String, max_results: usize },
    /// Find pages by tag
    Tag { tag: String, max_results: usize },
    /// Find pages similar to a given URL
    Similar { url: String, max_results: usize },
    /// Find stale pages that need refresh
    Stale { max_results: usize },
    /// Find recently cached pages
    Recent { max_results: usize },
    /// Find most-accessed pages
    Popular { max_results: usize },
    /// Find pinned pages
    Pinned,
}

/// A search result from the knowledge store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheResult {
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub relevance_score: f32,
    pub is_stale: bool,
    pub age_secs: u64,
    pub hit_count: u64,
    pub content_type: ContentType,
    pub token_count: usize,
    pub source: KnowledgeSource,
}

/// Where the result came from in the knowledge store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KnowledgeSource {
    /// From a cached page
    CachedPage,
    /// From a cached search result
    CachedSearch,
    /// From an agent session snapshot
    SessionSnapshot,
}
