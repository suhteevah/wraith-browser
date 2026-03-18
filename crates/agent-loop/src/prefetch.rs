//! # Predictive Pre-Fetching
//!
//! Parses the agent's task plan and page context to predict which URLs
//! will be visited next, then pre-fetches and pre-extracts them in the
//! background. When the agent actually navigates, the content is already
//! in the KnowledgeStore — making navigation effectively instant.
//!
//! ## Strategy
//!
//! 1. **Link Analysis** — extract links from current page, score by relevance to task
//! 2. **Task Plan Parsing** — if the agent mentioned URLs in its reasoning, prefetch those
//! 3. **History Patterns** — if the agent has navigated A→B→C before, predict D
//! 4. **Search Result Pre-fetch** — when search results come in, prefetch top 3

use std::collections::HashSet;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

/// Configuration for prefetching behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefetchConfig {
    /// Maximum number of URLs to prefetch at once
    pub max_concurrent: usize,
    /// Maximum number of URLs to prefetch per step
    pub max_per_step: usize,
    /// Minimum relevance score (0.0-1.0) to trigger prefetch
    pub min_relevance: f64,
    /// Whether to prefetch search result URLs
    pub prefetch_search_results: bool,
    /// Whether to extract content (not just cache HTML)
    pub extract_on_prefetch: bool,
}

impl Default for PrefetchConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 3,
            max_per_step: 5,
            min_relevance: 0.3,
            prefetch_search_results: true,
            extract_on_prefetch: true,
        }
    }
}

/// A URL predicted for prefetching with its relevance score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefetchCandidate {
    /// The URL to prefetch
    pub url: String,
    /// Relevance score (0.0 - 1.0)
    pub relevance: f64,
    /// Why this URL was selected
    pub reason: PrefetchReason,
}

/// Why a URL was selected for prefetching.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PrefetchReason {
    /// URL appeared in the agent's reasoning text
    MentionedInPlan,
    /// High-relevance link on the current page
    RelevantPageLink,
    /// Top search result
    SearchResult,
    /// Follows a navigation pattern (e.g., pagination)
    PatternPrediction,
    /// Same-domain link likely related to task
    SameDomainLink,
}

/// The prefetch predictor — analyzes context to predict next URLs.
pub struct PrefetchPredictor {
    pub config: PrefetchConfig,
    /// URLs already fetched or in-flight (avoid duplicates)
    visited: HashSet<String>,
    /// History of navigated URLs for pattern detection
    nav_history: Vec<String>,
}

impl PrefetchPredictor {
    pub fn new(config: PrefetchConfig) -> Self {
        Self {
            config,
            visited: HashSet::new(),
            nav_history: Vec::new(),
        }
    }

    /// Record that a URL has been visited (don't prefetch it again).
    pub fn record_visit(&mut self, url: &str) {
        self.visited.insert(url.to_string());
        self.nav_history.push(url.to_string());
    }

    /// Predict which URLs to prefetch based on current context.
    ///
    /// - `task_description`: the user's task
    /// - `current_url`: where the agent is now
    /// - `llm_response`: the LLM's latest reasoning (may mention URLs)
    /// - `page_links`: links found on the current page: (text, url)
    /// - `search_results`: recent search result URLs
    #[instrument(skip(self, llm_response, page_links, search_results), fields(current_url = %current_url))]
    pub fn predict(
        &self,
        task_description: &str,
        current_url: &str,
        llm_response: &str,
        page_links: &[(String, String)],
        search_results: &[String],
    ) -> Vec<PrefetchCandidate> {
        let mut candidates = Vec::new();
        let task_lower = task_description.to_lowercase();
        let task_words: Vec<&str> = task_lower.split_whitespace().collect();

        // 1. Extract URLs mentioned in LLM reasoning
        candidates.extend(self.extract_urls_from_text(llm_response));

        // 2. Score page links by relevance to task
        candidates.extend(self.score_page_links(page_links, &task_words, current_url));

        // 3. Search results (top N)
        if self.config.prefetch_search_results {
            for (i, url) in search_results.iter().take(3).enumerate() {
                if !self.visited.contains(url) {
                    candidates.push(PrefetchCandidate {
                        url: url.clone(),
                        relevance: 0.8 - (i as f64 * 0.15), // 0.8, 0.65, 0.5
                        reason: PrefetchReason::SearchResult,
                    });
                }
            }
        }

        // 4. Pattern detection (pagination, next page)
        candidates.extend(self.detect_patterns(page_links));

        // Deduplicate, filter visited, sort by relevance, limit
        self.filter_and_rank(candidates)
    }

    /// Extract URLs that appear in the LLM's response text.
    fn extract_urls_from_text(&self, text: &str) -> Vec<PrefetchCandidate> {
        let mut candidates = Vec::new();

        for word in text.split_whitespace() {
            let trimmed = word.trim_matches(|c: char| !c.is_alphanumeric() && c != ':' && c != '/' && c != '.');
            if (trimmed.starts_with("http://") || trimmed.starts_with("https://"))
                && !self.visited.contains(trimmed)
            {
                candidates.push(PrefetchCandidate {
                    url: trimmed.to_string(),
                    relevance: 0.9, // High — the agent explicitly mentioned it
                    reason: PrefetchReason::MentionedInPlan,
                });
            }
        }

        candidates
    }

    /// Score page links by relevance to the task description.
    fn score_page_links(
        &self,
        links: &[(String, String)],
        task_words: &[&str],
        current_url: &str,
    ) -> Vec<PrefetchCandidate> {
        let current_domain = url::Url::parse(current_url)
            .map(|u| u.host_str().unwrap_or("").to_string())
            .unwrap_or_default();

        let mut candidates = Vec::new();
        for (text, url) in links {
            if self.visited.contains(url) || url.is_empty() {
                continue;
            }

            let text_lower = text.to_lowercase();
            let url_lower = url.to_lowercase();

            // Score based on word overlap with task
            let word_matches = task_words.iter()
                .filter(|w| w.len() > 3) // Skip short words
                .filter(|w| text_lower.contains(**w) || url_lower.contains(**w))
                .count();

            let mut relevance = if task_words.is_empty() {
                0.0
            } else {
                (word_matches as f64) / (task_words.len() as f64).max(1.0)
            };

            // Boost same-domain links
            let link_domain = url::Url::parse(url)
                .map(|u| u.host_str().unwrap_or("").to_string())
                .unwrap_or_default();
            if link_domain == current_domain {
                relevance += 0.1;
            }

            // Boost links with action words
            let action_words = ["detail", "view", "show", "product", "result", "page", "article"];
            if action_words.iter().any(|w| url_lower.contains(w) || text_lower.contains(w)) {
                relevance += 0.05;
            }

            if relevance >= self.config.min_relevance {
                let reason = if link_domain == current_domain {
                    PrefetchReason::SameDomainLink
                } else {
                    PrefetchReason::RelevantPageLink
                };

                candidates.push(PrefetchCandidate {
                    url: url.clone(),
                    relevance: relevance.min(1.0),
                    reason,
                });
            }
        }

        candidates
    }

    /// Detect navigation patterns (pagination, next/prev).
    fn detect_patterns(&self, links: &[(String, String)]) -> Vec<PrefetchCandidate> {
        let mut candidates = Vec::new();

        for (text, url) in links {
            if self.visited.contains(url) {
                continue;
            }

            let text_lower = text.to_lowercase();

            // Pagination patterns
            if text_lower == "next" || text_lower == "next page" || text_lower == "next →"
                || text_lower == ">" || text_lower == ">>"
                || text_lower.contains("page 2") || text_lower.contains("next results")
            {
                candidates.push(PrefetchCandidate {
                    url: url.clone(),
                    relevance: 0.7,
                    reason: PrefetchReason::PatternPrediction,
                });
            }

            // "Load more" / "Show more"
            if text_lower.contains("load more") || text_lower.contains("show more")
                || text_lower.contains("view all")
            {
                candidates.push(PrefetchCandidate {
                    url: url.clone(),
                    relevance: 0.6,
                    reason: PrefetchReason::PatternPrediction,
                });
            }
        }

        candidates
    }

    /// Filter visited URLs, deduplicate, sort by relevance, limit.
    fn filter_and_rank(&self, candidates: Vec<PrefetchCandidate>) -> Vec<PrefetchCandidate> {
        let mut seen = HashSet::new();
        let mut filtered: Vec<PrefetchCandidate> = candidates
            .into_iter()
            .filter(|c| !self.visited.contains(&c.url))
            .filter(|c| c.relevance >= self.config.min_relevance)
            .filter(|c| seen.insert(c.url.clone()))
            .collect();

        filtered.sort_by(|a, b| {
            b.relevance.partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        filtered.truncate(self.config.max_per_step);

        debug!(
            count = filtered.len(),
            top_url = filtered.first().map(|c| c.url.as_str()).unwrap_or("none"),
            "Prefetch candidates selected"
        );

        filtered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn predictor() -> PrefetchPredictor {
        PrefetchPredictor::new(PrefetchConfig::default())
    }

    #[test]
    fn extracts_urls_from_llm_response() {
        let p = predictor();
        let text = "I should check https://example.com/products and then navigate to https://example.com/cart";
        let candidates = p.extract_urls_from_text(text);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].reason, PrefetchReason::MentionedInPlan);
    }

    #[test]
    fn skips_visited_urls() {
        let mut p = predictor();
        p.record_visit("https://example.com/visited");
        let text = "Check https://example.com/visited and https://example.com/new";
        let candidates = p.extract_urls_from_text(text);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].url, "https://example.com/new");
    }

    #[test]
    fn scores_page_links_by_task_relevance() {
        let p = predictor();
        let links = vec![
            ("Buy Rust Book".to_string(), "https://shop.com/rust-book".to_string()),
            ("About Us".to_string(), "https://shop.com/about".to_string()),
            ("Privacy Policy".to_string(), "https://shop.com/privacy".to_string()),
        ];
        let task_words: Vec<&str> = "find rust programming book".split_whitespace().collect();
        let candidates = p.score_page_links(&links, &task_words, "https://shop.com");

        // "Buy Rust Book" should score highest
        assert!(!candidates.is_empty());
        let rust_candidate = candidates.iter().find(|c| c.url.contains("rust-book"));
        assert!(rust_candidate.is_some());
    }

    #[test]
    fn detects_pagination_patterns() {
        let p = predictor();
        let links = vec![
            ("Next".to_string(), "https://example.com/page/2".to_string()),
            ("Previous".to_string(), "https://example.com/page/0".to_string()),
            ("Home".to_string(), "https://example.com/".to_string()),
        ];
        let candidates = p.detect_patterns(&links);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].url, "https://example.com/page/2");
        assert_eq!(candidates[0].reason, PrefetchReason::PatternPrediction);
    }

    #[test]
    fn predict_combines_all_strategies() {
        let p = predictor();
        let candidates = p.predict(
            "find cheap flights to Paris",
            "https://flights.com/search",
            "I see results. Let me check https://flights.com/deal/123",
            &[
                ("Paris Flights $299".to_string(), "https://flights.com/paris".to_string()),
                ("Next Page".to_string(), "https://flights.com/search?page=2".to_string()),
            ],
            &["https://flights.com/compare".to_string()],
        );

        // Should have: mentioned URL, relevant link, pagination, search result
        assert!(candidates.len() >= 2);
        // Mentioned URL should rank highest
        assert_eq!(candidates[0].reason, PrefetchReason::MentionedInPlan);
    }

    #[test]
    fn respects_max_per_step_limit() {
        let mut config = PrefetchConfig::default();
        config.max_per_step = 2;
        config.min_relevance = 0.0;
        let p = PrefetchPredictor::new(config);

        let links: Vec<(String, String)> = (0..10)
            .map(|i| (format!("Link {i}"), format!("https://example.com/{i}")))
            .collect();

        let candidates = p.predict(
            "test task with many words to match links",
            "https://example.com",
            "",
            &links,
            &[],
        );

        assert!(candidates.len() <= 2);
    }

    #[test]
    fn search_results_get_high_relevance() {
        let p = predictor();
        let candidates = p.predict(
            "find information",
            "https://search.com",
            "",
            &[],
            &[
                "https://result1.com".to_string(),
                "https://result2.com".to_string(),
                "https://result3.com".to_string(),
            ],
        );

        assert_eq!(candidates.len(), 3);
        assert!(candidates[0].relevance > candidates[1].relevance);
        assert!(candidates[1].relevance > candidates[2].relevance);
    }

    #[test]
    fn record_visit_prevents_prefetch() {
        let mut p = predictor();
        p.record_visit("https://example.com/page1");
        p.record_visit("https://example.com/page2");

        let candidates = p.predict(
            "test",
            "https://example.com",
            "check https://example.com/page1 and https://example.com/page3",
            &[],
            &[],
        );

        // page1 is visited, only page3 should appear
        assert!(candidates.iter().all(|c| c.url != "https://example.com/page1"));
        assert!(candidates.iter().any(|c| c.url == "https://example.com/page3"));
    }
}
