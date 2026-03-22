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
pub mod scoring;

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
/// Handles OR queries by splitting into sub-queries and merging results.
pub async fn search(query: &str, max_results: usize) -> Result<Vec<SearchResult>, error::SearchError> {
    // Split OR queries into sub-queries for providers that don't support OR natively
    let sub_queries = split_or_query(query);

    if sub_queries.len() > 1 {
        info!(query = %query, sub_queries = sub_queries.len(), "OR query detected — splitting into {} sub-queries", sub_queries.len());
        return search_multi(sub_queries, max_results).await;
    }

    search_single(query, max_results).await
}

/// Split a query containing OR operators into individual sub-queries.
/// "site:greenhouse.io QA engineer OR SDET remote" becomes:
/// ["site:greenhouse.io QA engineer remote", "site:greenhouse.io SDET remote"]
fn split_or_query(query: &str) -> Vec<String> {
    // Find OR-separated terms (case-insensitive)
    let parts: Vec<&str> = query.split(" OR ").collect();
    if parts.len() <= 1 {
        // Also try " or " (lowercase)
        let parts_lower: Vec<&str> = query.split(" or ").collect();
        if parts_lower.len() <= 1 {
            return vec![query.to_string()];
        }
        return build_or_variants(&parts_lower);
    }
    build_or_variants(&parts)
}

/// Given ["site:greenhouse.io QA engineer", "SDET remote"],
/// extract the common prefix/suffix and build full queries.
fn build_or_variants(parts: &[&str]) -> Vec<String> {
    if parts.len() <= 1 {
        return parts.iter().map(|s| s.to_string()).collect();
    }

    // Strategy: the first part has the prefix context, the last part has the suffix context.
    // Middle parts are the OR alternatives.
    // Example: "site:greenhouse.io QA engineer OR SDET OR DevOps remote 2026"
    // parts = ["site:greenhouse.io QA engineer", "SDET", "DevOps remote 2026"]

    let first = parts[0].trim();
    let last = parts[parts.len() - 1].trim();

    // Find words in the first part that look like prefix context (site:, quoted strings, years)
    let first_words: Vec<&str> = first.split_whitespace().collect();
    let last_words: Vec<&str> = last.split_whitespace().collect();

    // The last word(s) of the first part are the first OR alternative
    // The first word(s) of the last part are the last OR alternative
    // Common suffix: words in the last part that appear after the alternative

    // Simple heuristic: treat each part as a complete alternative,
    // but carry forward any site: prefix and trailing context words
    let prefix: Vec<&str> = first_words.iter()
        .take_while(|w| w.starts_with("site:") || w.starts_with("\""))
        .copied()
        .collect();

    // Suffix: common trailing context (years, "remote", location terms)
    let suffix_words: Vec<&str> = last_words.iter()
        .rev()
        .take_while(|w| {
            let lower = w.to_lowercase();
            lower == "remote" || lower == "hybrid" || lower == "onsite"
                || lower.parse::<u32>().is_ok() // years like "2026"
                || lower.starts_with("\"")
        })
        .copied()
        .collect::<Vec<_>>()
        .into_iter().rev().collect();

    let prefix_str = if prefix.is_empty() { String::new() } else { prefix.join(" ") + " " };
    let suffix_str = if suffix_words.is_empty() { String::new() } else { " ".to_string() + &suffix_words.join(" ") };

    // Build one query per OR variant
    let mut queries = Vec::new();

    // First part: remove prefix (already extracted), use as first alternative
    let first_alt = first_words[prefix.len()..].join(" ");
    if !first_alt.is_empty() {
        queries.push(format!("{}{}{}", prefix_str, first_alt.trim(), suffix_str));
    }

    // Middle parts: standalone alternatives
    for part in &parts[1..parts.len()-1] {
        let trimmed = part.trim();
        if !trimmed.is_empty() {
            queries.push(format!("{}{}{}", prefix_str, trimmed, suffix_str));
        }
    }

    // Last part: remove suffix (already extracted), use as last alternative
    if parts.len() > 1 {
        let last_alt_words: Vec<&str> = last_words[..last_words.len().saturating_sub(suffix_words.len())].to_vec();
        let last_alt = last_alt_words.join(" ");
        if !last_alt.is_empty() {
            queries.push(format!("{}{}{}", prefix_str, last_alt.trim(), suffix_str));
        }
    }

    if queries.is_empty() {
        queries.push(parts.join(" "));
    }

    queries
}

/// Run multiple sub-queries and merge results.
async fn search_multi(queries: Vec<String>, max_results: usize) -> Result<Vec<SearchResult>, error::SearchError> {
    let per_query = (max_results / queries.len()).max(5);
    let mut all_results = Vec::new();

    for q in &queries {
        debug!(sub_query = %q, "Executing OR sub-query");
        match search_single(q, per_query).await {
            Ok(results) => {
                debug!(count = results.len(), query = %q, "Sub-query results");
                all_results.extend(results);
            }
            Err(e) => {
                tracing::warn!(error = %e, query = %q, "Sub-query failed");
            }
        }
    }

    // Deduplicate and rank
    all_results.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    all_results.dedup_by(|a, b| a.url == b.url);
    all_results.truncate(max_results);

    info!(total = all_results.len(), queries = queries.len(), "OR search complete");
    Ok(all_results)
}

/// Single query search — the original implementation.
async fn search_single(query: &str, max_results: usize) -> Result<Vec<SearchResult>, error::SearchError> {
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

    Ok(all_results)
}
