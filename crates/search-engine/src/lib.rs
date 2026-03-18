//! # openclaw-search
//!
//! Dual-mode search engine for OpenClaw Browser:
//!
//! 1. **Web Metasearch** — Aggregates results from DuckDuckGo (HTML scraping,
//!    no API key) and Brave Search (API key via `BRAVE_SEARCH_API_KEY`).
//!
//! 2. **Local Index** — Tantivy-powered full-text index of browsed pages,
//!    extracted content, and agent session history. Enables "search my
//!    browsing history" and RAG-style context retrieval.

pub mod web;
pub mod local;
pub mod error;

use serde::{Deserialize, Serialize};
use tracing::{info, debug};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub source: SearchSource,
    pub relevance_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SearchSource {
    Google,
    Bing,
    DuckDuckGo,
    Brave,
    JSearch,
    Adzuna,
    Remotive,
    SearXNG,
    LocalIndex,
}

/// Unified search — combines web metasearch with local index results.
pub async fn search(query: &str, max_results: usize) -> Result<Vec<SearchResult>, error::SearchError> {
    info!(query = %query, max_results, "Executing unified search");

    let mut all_results = Vec::new();

    // Web metasearch
    match web::metasearch(query, max_results).await {
        Ok(results) => {
            debug!(web_results = results.len(), "Web search complete");
            all_results.extend(results);
        }
        Err(e) => {
            tracing::warn!(error = %e, "Web metasearch failed, using local results only");
        }
    }

    // Local index
    match local::search_local(query, max_results) {
        Ok(results) => {
            debug!(local_results = results.len(), "Local search complete");
            all_results.extend(results);
        }
        Err(e) => {
            tracing::warn!(error = %e, "Local search failed");
        }
    }

    // Merge, deduplicate, rank
    all_results.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    all_results.dedup_by(|a, b| a.url == b.url);
    all_results.truncate(max_results);

    info!(query = %query, total_results = all_results.len(), "Unified search complete");
    Ok(all_results)
}
