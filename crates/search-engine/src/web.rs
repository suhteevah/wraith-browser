//! Web metasearch — aggregates results from multiple search providers.
//!
//! DuckDuckGo HTML scraping (no API key needed) is the primary provider.
//! Brave Search API is the secondary provider (needs API key).

use crate::{SearchResult, SearchSource, error::SearchError};
use scraper::{Html, Selector};
use tracing::{info, debug, warn, instrument};

/// Fan out to available providers and merge results.
#[instrument(skip_all, fields(query = %query))]
pub async fn metasearch(query: &str, max_results: usize) -> Result<Vec<SearchResult>, SearchError> {
    info!(query = %query, max_results, "Running web metasearch");

    let mut all_results = Vec::new();

    // DuckDuckGo HTML — always available, no API key
    match duckduckgo_search(query, max_results).await {
        Ok(results) => {
            debug!(count = results.len(), "DuckDuckGo results received");
            all_results.extend(results);
        }
        Err(e) => {
            warn!(error = %e, "DuckDuckGo search failed, continuing with other providers");
        }
    }

    // Brave Search — if API key is available
    if let Ok(api_key) = std::env::var("BRAVE_SEARCH_API_KEY") {
        match brave_search(query, max_results, &api_key).await {
            Ok(results) => {
                debug!(count = results.len(), "Brave Search results received");
                all_results.extend(results);
            }
            Err(e) => {
                warn!(error = %e, "Brave Search failed");
            }
        }
    }

    // JSearch (RapidAPI) — if API key is available
    if let Ok(rapidapi_key) = std::env::var("RAPIDAPI_KEY") {
        match jsearch(query, max_results, &rapidapi_key).await {
            Ok(results) => {
                debug!(count = results.len(), "JSearch results received");
                all_results.extend(results);
            }
            Err(e) => {
                warn!(error = %e, "JSearch failed");
            }
        }
    }

    // Adzuna — if app ID and key are available
    if let (Ok(app_id), Ok(app_key)) = (std::env::var("ADZUNA_APP_ID"), std::env::var("ADZUNA_APP_KEY")) {
        match adzuna_search(query, max_results, &app_id, &app_key).await {
            Ok(results) => {
                debug!(count = results.len(), "Adzuna results received");
                all_results.extend(results);
            }
            Err(e) => {
                warn!(error = %e, "Adzuna search failed");
            }
        }
    }

    // SearXNG — always available, no auth needed, aggregates multiple engines
    match searxng_search(query, max_results).await {
        Ok(results) => {
            debug!(count = results.len(), "SearXNG results received");
            all_results.extend(results);
        }
        Err(e) => {
            warn!(error = %e, "SearXNG search failed, continuing");
        }
    }

    // Remotive — always available, no auth needed (remote jobs)
    match remotive_search(query, max_results).await {
        Ok(results) => {
            debug!(count = results.len(), "Remotive results received");
            all_results.extend(results);
        }
        Err(e) => {
            warn!(error = %e, "Remotive search failed, continuing");
        }
    }

    // Deduplicate by URL
    all_results.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap_or(std::cmp::Ordering::Equal));
    all_results.dedup_by(|a, b| a.url == b.url);
    all_results.truncate(max_results);

    info!(query = %query, results = all_results.len(), "Metasearch complete");
    Ok(all_results)
}

// ═══════════════════════════════════════════════════════════════
// DuckDuckGo HTML scraping
// ═══════════════════════════════════════════════════════════════

/// Search DuckDuckGo by scraping the HTML results page.
/// No API key required. Respects DDG's terms by not hammering.
#[instrument(skip_all, fields(query = %query))]
async fn duckduckgo_search(query: &str, max_results: usize) -> Result<Vec<SearchResult>, SearchError> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| SearchError::ProviderFailed {
            provider: "DuckDuckGo".to_string(),
            reason: format!("Client build failed: {e}"),
        })?;

    let url = format!("https://html.duckduckgo.com/html/?q={}", urlencoding(query));

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| SearchError::ProviderFailed {
            provider: "DuckDuckGo".to_string(),
            reason: format!("Request failed: {e}"),
        })?;

    let html = response.text().await.map_err(|e| SearchError::ProviderFailed {
        provider: "DuckDuckGo".to_string(),
        reason: format!("Response read failed: {e}"),
    })?;

    parse_duckduckgo_html(&html, max_results)
}

/// Parse DuckDuckGo HTML results page.
fn parse_duckduckgo_html(html: &str, max_results: usize) -> Result<Vec<SearchResult>, SearchError> {
    let document = Html::parse_document(html);

    // DuckDuckGo HTML results are in .result elements
    let result_sel = Selector::parse(".result")
        .map_err(|e| SearchError::ProviderFailed {
            provider: "DuckDuckGo".to_string(),
            reason: format!("Selector parse failed: {e}"),
        })?;

    let link_sel = Selector::parse(".result__a")
        .map_err(|_| SearchError::ProviderFailed {
            provider: "DuckDuckGo".to_string(),
            reason: "link selector failed".to_string(),
        })?;

    let snippet_sel = Selector::parse(".result__snippet")
        .map_err(|_| SearchError::ProviderFailed {
            provider: "DuckDuckGo".to_string(),
            reason: "snippet selector failed".to_string(),
        })?;

    let mut results = Vec::new();
    let total = document.select(&result_sel).count();

    for (i, result_el) in document.select(&result_sel).enumerate() {
        if results.len() >= max_results {
            break;
        }

        // Extract link and title
        let (title, url) = if let Some(link) = result_el.select(&link_sel).next() {
            let title: String = link.text().collect::<String>().trim().to_string();
            let href = link.value().attr("href").unwrap_or("").to_string();

            // DDG wraps URLs in a redirect — extract the actual URL
            let actual_url = extract_ddg_url(&href).unwrap_or(href);

            if title.is_empty() || actual_url.is_empty() {
                continue;
            }

            (title, actual_url)
        } else {
            continue;
        };

        // Extract snippet
        let snippet = result_el
            .select(&snippet_sel)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        // Compute relevance score (higher rank = higher score)
        let relevance = 1.0 - (i as f32 / total.max(1) as f32);

        results.push(SearchResult {
            title,
            url,
            snippet,
            source: SearchSource::DuckDuckGo,
            relevance_score: relevance,
        });
    }

    debug!(count = results.len(), "Parsed DuckDuckGo results");
    Ok(results)
}

/// Extract the actual URL from a DuckDuckGo redirect URL.
/// DDG links look like: //duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com&rut=...
fn extract_ddg_url(href: &str) -> Option<String> {
    if let Some(uddg_start) = href.find("uddg=") {
        let encoded = &href[uddg_start + 5..];
        let encoded = encoded.split('&').next().unwrap_or(encoded);
        let decoded = percent_decode(encoded);
        if decoded.is_empty() {
            None
        } else {
            Some(decoded)
        }
    } else if href.starts_with("http://") || href.starts_with("https://") {
        Some(href.to_string())
    } else {
        None
    }
}

/// Simple percent-decoding for URL query values.
fn percent_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            // Collect all consecutive percent-encoded bytes into a buffer
            // so multi-byte UTF-8 sequences (e.g. %C3%A9 for 'é') are decoded correctly.
            let mut bytes = Vec::new();
            let hex: String = chars.by_ref().take(2).collect();
            if hex.len() == 2 {
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    bytes.push(byte);
                } else {
                    result.push('%');
                    result.push_str(&hex);
                    continue;
                }
            } else {
                result.push('%');
                result.push_str(&hex);
                continue;
            }
            // Keep consuming consecutive %XX sequences
            // We need to peek ahead, so we use a loop with a temporary collection
            // Since chars is by-ref, we need to be careful
            loop {
                // Try to peek at the next char to see if it's '%'
                // We'll clone the iterator state by collecting remaining into a string
                // Actually, we can just check the next char
                let mut peeker = chars.clone();
                if let Some('%') = peeker.next() {
                    let hex2: String = peeker.by_ref().take(2).collect();
                    if hex2.len() == 2 {
                        if let Ok(byte) = u8::from_str_radix(&hex2, 16) {
                            bytes.push(byte);
                            // Actually advance the real iterator past '%XX'
                            chars.next(); // skip '%'
                            chars.next(); // skip first hex char
                            chars.next(); // skip second hex char
                            continue;
                        }
                    }
                }
                break;
            }
            result.push_str(&String::from_utf8_lossy(&bytes));
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

// ═══════════════════════════════════════════════════════════════
// Brave Search API
// ═══════════════════════════════════════════════════════════════

/// Search using the Brave Search API.
/// Requires BRAVE_SEARCH_API_KEY environment variable.
#[instrument(skip_all, fields(query = %query))]
async fn brave_search(query: &str, max_results: usize, api_key: &str) -> Result<Vec<SearchResult>, SearchError> {
    let client = reqwest::Client::new();

    let response = client
        .get("https://api.search.brave.com/res/v1/web/search")
        .header("X-Subscription-Token", api_key)
        .header("Accept", "application/json")
        .query(&[
            ("q", query),
            ("count", &max_results.to_string()),
        ])
        .send()
        .await
        .map_err(|e| SearchError::ProviderFailed {
            provider: "Brave".to_string(),
            reason: format!("Request failed: {e}"),
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(SearchError::ProviderFailed {
            provider: "Brave".to_string(),
            reason: format!("API error {status}: {body}"),
        });
    }

    let body: serde_json::Value = response.json().await.map_err(|e| SearchError::ProviderFailed {
        provider: "Brave".to_string(),
        reason: format!("JSON parse failed: {e}"),
    })?;

    let mut results = Vec::new();
    if let Some(web_results) = body.get("web").and_then(|w| w.get("results")).and_then(|r| r.as_array()) {
        for (i, item) in web_results.iter().enumerate() {
            let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
            let url = item.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
            let snippet = item.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string();

            if title.is_empty() || url.is_empty() {
                continue;
            }

            let relevance = 1.0 - (i as f32 / web_results.len().max(1) as f32);
            results.push(SearchResult {
                title,
                url,
                snippet,
                source: SearchSource::Brave,
                relevance_score: relevance,
            });
        }
    }

    debug!(count = results.len(), "Brave Search results parsed");
    Ok(results)
}

// ═══════════════════════════════════════════════════════════════
// JSearch (RapidAPI)
// ═══════════════════════════════════════════════════════════════

/// Search using JSearch via RapidAPI.
/// Requires RAPIDAPI_KEY environment variable.
#[instrument(skip_all, fields(query = %query))]
async fn jsearch(query: &str, max_results: usize, api_key: &str) -> Result<Vec<SearchResult>, SearchError> {
    let client = reqwest::Client::new();

    let response = client
        .get("https://jsearch.p.rapidapi.com/search")
        .header("X-RapidAPI-Key", api_key)
        .header("X-RapidAPI-Host", "jsearch.p.rapidapi.com")
        .query(&[
            ("query", query),
            ("num_pages", "1"),
        ])
        .send()
        .await
        .map_err(|e| SearchError::ProviderFailed {
            provider: "JSearch".to_string(),
            reason: format!("Request failed: {e}"),
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(SearchError::ProviderFailed {
            provider: "JSearch".to_string(),
            reason: format!("API error {status}: {body}"),
        });
    }

    let body: serde_json::Value = response.json().await.map_err(|e| SearchError::ProviderFailed {
        provider: "JSearch".to_string(),
        reason: format!("JSON parse failed: {e}"),
    })?;

    let mut results = Vec::new();
    if let Some(data) = body.get("data").and_then(|d| d.as_array()) {
        for (i, item) in data.iter().enumerate() {
            if results.len() >= max_results {
                break;
            }

            let title = item.get("job_title").and_then(|t| t.as_str()).unwrap_or("").to_string();
            let employer = item.get("employer_name").and_then(|e| e.as_str()).unwrap_or("");
            let url = item.get("job_apply_link").and_then(|u| u.as_str()).unwrap_or("").to_string();
            let snippet = item.get("job_description").and_then(|d| d.as_str()).unwrap_or("").to_string();

            if title.is_empty() || url.is_empty() {
                continue;
            }

            let display_title = if employer.is_empty() {
                title
            } else {
                format!("{title} — {employer}")
            };

            let relevance = 1.0 - (i as f32 / data.len().max(1) as f32);
            results.push(SearchResult {
                title: display_title,
                url,
                snippet,
                source: SearchSource::JSearch,
                relevance_score: relevance,
            });
        }
    }

    debug!(count = results.len(), "JSearch results parsed");
    Ok(results)
}

// ═══════════════════════════════════════════════════════════════
// Adzuna API
// ═══════════════════════════════════════════════════════════════

/// Search using the Adzuna job search API.
/// Requires ADZUNA_APP_ID and ADZUNA_APP_KEY environment variables.
#[instrument(skip_all, fields(query = %query))]
async fn adzuna_search(query: &str, max_results: usize, app_id: &str, app_key: &str) -> Result<Vec<SearchResult>, SearchError> {
    let client = reqwest::Client::new();

    let response = client
        .get("https://api.adzuna.com/v1/api/jobs/us/search/1")
        .query(&[
            ("app_id", app_id),
            ("app_key", app_key),
            ("what", query),
            ("results_per_page", &max_results.to_string()),
        ])
        .send()
        .await
        .map_err(|e| SearchError::ProviderFailed {
            provider: "Adzuna".to_string(),
            reason: format!("Request failed: {e}"),
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(SearchError::ProviderFailed {
            provider: "Adzuna".to_string(),
            reason: format!("API error {status}: {body}"),
        });
    }

    let body: serde_json::Value = response.json().await.map_err(|e| SearchError::ProviderFailed {
        provider: "Adzuna".to_string(),
        reason: format!("JSON parse failed: {e}"),
    })?;

    let mut results = Vec::new();
    if let Some(items) = body.get("results").and_then(|r| r.as_array()) {
        for (i, item) in items.iter().enumerate() {
            if results.len() >= max_results {
                break;
            }

            let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
            let url = item.get("redirect_url").and_then(|u| u.as_str()).unwrap_or("").to_string();
            let snippet = item.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string();

            if title.is_empty() || url.is_empty() {
                continue;
            }

            let relevance = 1.0 - (i as f32 / items.len().max(1) as f32);
            results.push(SearchResult {
                title,
                url,
                snippet,
                source: SearchSource::Adzuna,
                relevance_score: relevance,
            });
        }
    }

    debug!(count = results.len(), "Adzuna results parsed");
    Ok(results)
}

// ═══════════════════════════════════════════════════════════════
// Remotive API
// ═══════════════════════════════════════════════════════════════

/// Search remote jobs using the Remotive API.
/// No authentication required.
#[instrument(skip_all, fields(query = %query))]
async fn remotive_search(query: &str, max_results: usize) -> Result<Vec<SearchResult>, SearchError> {
    let client = reqwest::Client::new();

    let response = client
        .get("https://remotive.com/api/remote-jobs")
        .query(&[
            ("search", query),
            ("limit", &max_results.to_string()),
        ])
        .send()
        .await
        .map_err(|e| SearchError::ProviderFailed {
            provider: "Remotive".to_string(),
            reason: format!("Request failed: {e}"),
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(SearchError::ProviderFailed {
            provider: "Remotive".to_string(),
            reason: format!("API error {status}: {body}"),
        });
    }

    let body: serde_json::Value = response.json().await.map_err(|e| SearchError::ProviderFailed {
        provider: "Remotive".to_string(),
        reason: format!("JSON parse failed: {e}"),
    })?;

    let mut results = Vec::new();
    if let Some(jobs) = body.get("jobs").and_then(|j| j.as_array()) {
        for (i, item) in jobs.iter().enumerate() {
            if results.len() >= max_results {
                break;
            }

            let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
            let company = item.get("company_name").and_then(|c| c.as_str()).unwrap_or("");
            let url = item.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
            let snippet = item.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string();

            if title.is_empty() || url.is_empty() {
                continue;
            }

            let display_title = if company.is_empty() {
                title
            } else {
                format!("{title} — {company}")
            };

            let relevance = 1.0 - (i as f32 / jobs.len().max(1) as f32);
            results.push(SearchResult {
                title: display_title,
                url,
                snippet,
                source: SearchSource::Remotive,
                relevance_score: relevance,
            });
        }
    }

    debug!(count = results.len(), "Remotive results parsed");
    Ok(results)
}

// ═══════════════════════════════════════════════════════════════
// SearXNG (public instances, no auth)
// ═══════════════════════════════════════════════════════════════

/// Public SearXNG instances that support JSON output.
const SEARXNG_INSTANCES: &[&str] = &[
    "https://search.ononoki.org",
    "https://search.sapti.me",
    "https://searx.be",
    "https://search.bus-hit.me",
    "https://priv.au",
];

/// Search using a public SearXNG instance.
/// Falls through instances on failure — no auth required.
#[instrument(skip_all, fields(query = %query))]
async fn searxng_search(query: &str, max_results: usize) -> Result<Vec<SearchResult>, SearchError> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| SearchError::ProviderFailed {
            provider: "SearXNG".to_string(),
            reason: format!("Client build failed: {e}"),
        })?;

    // Also check for a custom instance URL
    let custom_instance = std::env::var("SEARXNG_URL").ok();
    let instances: Vec<&str> = if let Some(ref custom) = custom_instance {
        std::iter::once(custom.as_str()).chain(SEARXNG_INSTANCES.iter().copied()).collect()
    } else {
        SEARXNG_INSTANCES.to_vec()
    };

    for instance in &instances {
        let url = format!("{instance}/search");
        let result = client
            .get(&url)
            .query(&[
                ("q", query),
                ("format", "json"),
                ("engines", "google,bing,duckduckgo,brave,wikipedia"),
                ("safesearch", "0"),
            ])
            .header("Accept", "application/json")
            .send()
            .await;

        let response = match result {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                debug!(instance = %instance, status = %r.status(), "SearXNG instance returned error, trying next");
                continue;
            }
            Err(e) => {
                debug!(instance = %instance, error = %e, "SearXNG instance unreachable, trying next");
                continue;
            }
        };

        let body: serde_json::Value = match response.json().await {
            Ok(v) => v,
            Err(e) => {
                debug!(instance = %instance, error = %e, "SearXNG JSON parse failed, trying next");
                continue;
            }
        };

        let mut results = Vec::new();
        if let Some(search_results) = body.get("results").and_then(|r| r.as_array()) {
            for (i, item) in search_results.iter().enumerate() {
                if results.len() >= max_results {
                    break;
                }

                let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                let url = item.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
                let snippet = item.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();

                if title.is_empty() || url.is_empty() {
                    continue;
                }

                let relevance = 1.0 - (i as f32 / search_results.len().max(1) as f32);
                results.push(SearchResult {
                    title,
                    url,
                    snippet,
                    source: SearchSource::SearXNG,
                    relevance_score: relevance,
                });
            }
        }

        debug!(instance = %instance, count = results.len(), "SearXNG results parsed");
        return Ok(results);
    }

    Err(SearchError::ProviderFailed {
        provider: "SearXNG".to_string(),
        reason: "All SearXNG instances failed".to_string(),
    })
}

/// Simple URL encoding for query strings.
fn urlencoding(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => result.push(c),
            ' ' => result.push('+'),
            _ => {
                // Encode each UTF-8 byte, not the Unicode code point
                for byte in c.to_string().as_bytes() {
                    result.push_str(&format!("%{:02X}", byte));
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urlencoding() {
        assert_eq!(urlencoding("hello world"), "hello+world");
        assert_eq!(urlencoding("rust web browser"), "rust+web+browser");
        assert_eq!(urlencoding("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn test_extract_ddg_url() {
        let href = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage&rut=abc";
        assert_eq!(extract_ddg_url(href), Some("https://example.com/page".to_string()));

        let direct = "https://example.com";
        assert_eq!(extract_ddg_url(direct), Some("https://example.com".to_string()));
    }

    #[test]
    fn test_parse_empty_html() {
        let results = parse_duckduckgo_html("<html><body></body></html>", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_ddg_results() {
        let html = r#"
        <html><body>
            <div class="result">
                <a class="result__a" href="https://example.com">Example Site</a>
                <a class="result__snippet">This is the snippet text.</a>
            </div>
            <div class="result">
                <a class="result__a" href="https://other.com">Other Site</a>
                <a class="result__snippet">Another snippet.</a>
            </div>
        </body></html>
        "#;
        let results = parse_duckduckgo_html(html, 10).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Example Site");
        assert_eq!(results[0].url, "https://example.com");
        assert!(results[0].relevance_score > results[1].relevance_score);
    }
}
