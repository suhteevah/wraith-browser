//! Website Capability Fingerprinting — on first visit, map a site's
//! capabilities (login forms, search boxes, APIs, navigation structure) and
//! cache the result per domain.
//!
//! The fingerprint drives strategy selection: should the agent use a browser,
//! call a discovered API directly, or rely on cached content?

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument};

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Cached capability fingerprint for a single domain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteCapability {
    /// The domain this fingerprint applies to (e.g., `"example.com"`).
    pub domain: String,
    /// Whether a login / sign-in form was detected.
    pub has_login: bool,
    /// Whether a search input was detected.
    pub has_search: bool,
    /// Whether API endpoints were detected.
    pub has_api: bool,
    /// URL of the detected login page, if any.
    pub login_url: Option<String>,
    /// CSS selector for the detected search input, if any.
    pub search_selector: Option<String>,
    /// Base URL for a detected API, if any.
    pub api_base_url: Option<String>,
    /// Navigation links extracted from the page.
    pub nav_links: Vec<NavLink>,
    /// Detected front-end / CMS technologies.
    pub detected_tech: Vec<String>,
    /// Recommended browsing strategy based on capabilities.
    pub optimal_strategy: BrowsingStrategy,
    /// When this fingerprint was created.
    pub fingerprinted_at: DateTime<Utc>,
}

/// A single navigation link found on the page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavLink {
    /// Visible link text.
    pub text: String,
    /// Target URL (absolute or relative).
    pub url: String,
    /// Whether this link is part of the primary navigation (inside `<nav>` or
    /// `<header>`).
    pub is_primary: bool,
}

/// Strategy the agent should use when interacting with this site.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BrowsingStrategy {
    /// Normal browser interaction required.
    BrowserUI,
    /// Skip the browser entirely — use a discovered REST/GraphQL API.
    DirectAPI,
    /// Authenticate via the browser, then switch to API calls.
    Hybrid,
    /// Content changes rarely — prefer cached results.
    CachedContent,
}

// ---------------------------------------------------------------------------
// Fingerprinting logic
// ---------------------------------------------------------------------------

/// Analyze a page's HTML to build a [`SiteCapability`] fingerprint.
///
/// * `domain` — the site's domain (e.g., `"example.com"`).
/// * `html` — the full HTML source of the page.
/// * `url` — the URL the HTML was fetched from (used to resolve relative
///   links and detect login pages).
#[instrument(skip(html))]
pub fn fingerprint_site(domain: &str, html: &str, url: &str) -> SiteCapability {
    let lower = html.to_lowercase();

    let has_login = detect_login(&lower);
    let has_search = detect_search(&lower);
    let has_api = detect_api(&lower);
    let nav_links = extract_nav_links(html);
    let detected_tech = detect_technology(html);

    let login_url = if has_login {
        Some(url.to_string())
    } else {
        None
    };

    let search_selector = if has_search {
        determine_search_selector(&lower)
    } else {
        None
    };

    let api_base_url = if has_api {
        extract_api_base(&lower, url)
    } else {
        None
    };

    let optimal_strategy = determine_strategy(has_login, has_search, has_api);

    info!(
        domain,
        has_login,
        has_search,
        has_api,
        tech_count = detected_tech.len(),
        nav_count = nav_links.len(),
        strategy = ?optimal_strategy,
        "Site fingerprinted"
    );

    SiteCapability {
        domain: domain.to_string(),
        has_login,
        has_search,
        has_api,
        login_url,
        search_selector,
        api_base_url,
        nav_links,
        detected_tech,
        optimal_strategy,
        fingerprinted_at: Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// Detect login / sign-in forms.
fn detect_login(lower_html: &str) -> bool {
    lower_html.contains("type=\"password\"")
        || lower_html.contains("type='password'")
        || lower_html.contains("sign in")
        || lower_html.contains("log in")
        || lower_html.contains("signin")
        || lower_html.contains("login")
}

/// Detect search inputs.
fn detect_search(lower_html: &str) -> bool {
    lower_html.contains("type=\"search\"")
        || lower_html.contains("type='search'")
        || lower_html.contains("name=\"q\"")
        || lower_html.contains("name=\"query\"")
        || lower_html.contains("name=\"search\"")
        || lower_html.contains("action=\"/search")
}

/// Detect API endpoint indicators.
fn detect_api(lower_html: &str) -> bool {
    lower_html.contains("/api/")
        || lower_html.contains("graphql")
        || lower_html.contains("application/json")
        || lower_html.contains("rest api")
        || lower_html.contains("swagger")
        || lower_html.contains("openapi")
}

/// Try to determine a CSS selector for the search input.
fn determine_search_selector(lower_html: &str) -> Option<String> {
    if lower_html.contains("type=\"search\"") || lower_html.contains("type='search'") {
        Some("input[type=search]".to_string())
    } else if lower_html.contains("name=\"q\"") {
        Some("input[name=q]".to_string())
    } else if lower_html.contains("name=\"query\"") {
        Some("input[name=query]".to_string())
    } else if lower_html.contains("name=\"search\"") {
        Some("input[name=search]".to_string())
    } else {
        None
    }
}

/// Try to extract a base URL for the API from the HTML.
fn extract_api_base(lower_html: &str, page_url: &str) -> Option<String> {
    // Look for /api/ path references — simplistic but effective.
    let re = Regex::new(r#"["'](/api/[^"']*?)["']"#).ok()?;
    if let Some(cap) = re.captures(lower_html) {
        let _path = cap.get(1)?.as_str();
        // Combine with page origin.
        if let Ok(parsed) = url::Url::parse(page_url) {
            let base = format!("{}://{}/api/", parsed.scheme(), parsed.host_str()?);
            return Some(base);
        }
    }
    None
}

/// Extract navigation links from the HTML.
///
/// Links inside `<nav>` or `<header>` elements are marked as primary.
fn extract_nav_links(html: &str) -> Vec<NavLink> {
    let mut links = Vec::new();

    // Regex to find <a> tags with href and text.
    let link_re = Regex::new(r#"<a\s[^>]*href=["']([^"']+)["'][^>]*>([^<]+)</a>"#).unwrap();

    // Determine regions covered by <nav> or <header>.
    let nav_re = Regex::new(r"(?is)<nav\b[^>]*>.*?</nav>").unwrap();
    let header_re = Regex::new(r"(?is)<header\b[^>]*>.*?</header>").unwrap();
    let mut primary_ranges: Vec<(usize, usize)> = Vec::new();
    for m in nav_re.find_iter(html) {
        primary_ranges.push((m.start(), m.end()));
    }
    for m in header_re.find_iter(html) {
        primary_ranges.push((m.start(), m.end()));
    }

    for cap in link_re.captures_iter(html) {
        let url = cap[1].trim().to_string();
        let text = cap[2].trim().to_string();

        if text.is_empty() || url.starts_with('#') || url.starts_with("javascript:") {
            continue;
        }

        let match_start = cap.get(0).map(|m| m.start()).unwrap_or(0);
        let is_primary = primary_ranges
            .iter()
            .any(|(start, end)| match_start >= *start && match_start < *end);

        links.push(NavLink {
            text,
            url,
            is_primary,
        });
    }

    debug!(count = links.len(), "Nav links extracted");
    links
}

/// Scan HTML for technology indicators and return a list of detected
/// technology names.
#[instrument(skip(html))]
pub fn detect_technology(html: &str) -> Vec<String> {
    let lower = html.to_lowercase();
    let mut tech = Vec::new();

    let patterns: &[(&[&str], &str)] = &[
        (&["wp-content", "wordpress"], "WordPress"),
        (&["__next_data__", "/_next/"], "React/Next.js"),
        (&["shopify", "cdn.shopify"], "Shopify"),
        (&["__vue__"], "Vue.js"),
        (&["angular"], "Angular"),
        (&["_nuxt"], "Nuxt.js"),
        (&["gatsby"], "Gatsby"),
        (&["wix.com"], "Wix"),
    ];

    // Special-case: "react" alone (without also matching Angular's "reactive")
    // is handled by checking for common React markers.
    let react_re = Regex::new(r"react[.\-/]|reactdom|__react").unwrap();
    if react_re.is_match(&lower) && !tech.contains(&"React/Next.js".to_string()) {
        tech.push("React/Next.js".to_string());
    }

    for (indicators, name) in patterns {
        if tech.contains(&name.to_string()) {
            continue;
        }
        if indicators.iter().any(|ind| lower.contains(ind)) {
            tech.push(name.to_string());
        }
    }

    // Special-case: plain "vue" without false positives.
    let vue_re = Regex::new(r"vue[.\-/]|vue\.js|vuejs").unwrap();
    if vue_re.is_match(&lower) && !tech.contains(&"Vue.js".to_string()) {
        tech.push("Vue.js".to_string());
    }

    debug!(tech = ?tech, "Technology detection complete");
    tech
}

/// Choose the optimal [`BrowsingStrategy`] based on detected capabilities.
fn determine_strategy(has_login: bool, _has_search: bool, has_api: bool) -> BrowsingStrategy {
    match (has_login, has_api) {
        (_, true) if !has_login => BrowsingStrategy::DirectAPI,
        (true, true) => BrowsingStrategy::Hybrid,
        (true, false) => BrowsingStrategy::BrowserUI,
        (false, false) => BrowsingStrategy::BrowserUI,
        _ => BrowsingStrategy::BrowserUI,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const LOGIN_HTML: &str = r#"
        <html>
        <body>
            <form action="/login">
                <input type="text" name="username" />
                <input type="password" name="password" />
                <button>Sign In</button>
            </form>
        </body>
        </html>
    "#;

    const SEARCH_HTML: &str = r#"
        <html>
        <body>
            <form action="/search">
                <input type="search" name="q" placeholder="Search..." />
            </form>
        </body>
        </html>
    "#;

    const API_HTML: &str = r#"
        <html>
        <head><script src="/api/config.js"></script></head>
        <body>
            <p>Explore our REST API at /api/v1/</p>
        </body>
        </html>
    "#;

    const REACT_HTML: &str = r#"
        <html>
        <head>
            <script src="/_next/static/chunks/main.js"></script>
            <script id="__NEXT_DATA__" type="application/json">{"props":{}}</script>
        </head>
        <body><div id="__next"></div></body>
        </html>
    "#;

    const WP_HTML: &str = r#"
        <html>
        <head>
            <link rel="stylesheet" href="/wp-content/themes/flavor/style.css" />
            <meta name="generator" content="WordPress 6.4" />
        </head>
        <body></body>
        </html>
    "#;

    const SHOPIFY_HTML: &str = r#"
        <html>
        <head>
            <link rel="stylesheet" href="https://cdn.shopify.com/s/files/1/theme.css" />
        </head>
        <body></body>
        </html>
    "#;

    const NAV_HTML: &str = r#"
        <html>
        <header>
            <nav>
                <a href="/about">About Us</a>
                <a href="/products">Products</a>
            </nav>
        </header>
        <body>
            <a href="/blog">Blog</a>
            <a href="/contact">Contact</a>
        </body>
        </html>
    "#;

    #[test]
    fn fingerprint_detects_login_forms() {
        let cap = fingerprint_site("example.com", LOGIN_HTML, "https://example.com/login");
        assert!(cap.has_login);
        assert_eq!(cap.login_url, Some("https://example.com/login".to_string()));
    }

    #[test]
    fn fingerprint_detects_search() {
        let cap = fingerprint_site("example.com", SEARCH_HTML, "https://example.com");
        assert!(cap.has_search);
        assert!(cap.search_selector.is_some());
    }

    #[test]
    fn detect_technology_finds_react_nextjs() {
        let tech = detect_technology(REACT_HTML);
        assert!(tech.iter().any(|t| t.contains("React")), "Expected React, got {:?}", tech);
    }

    #[test]
    fn detect_technology_finds_wordpress() {
        let tech = detect_technology(WP_HTML);
        assert!(tech.iter().any(|t| t.contains("WordPress")), "Expected WordPress, got {:?}", tech);
    }

    #[test]
    fn detect_technology_finds_shopify() {
        let tech = detect_technology(SHOPIFY_HTML);
        assert!(tech.iter().any(|t| t.contains("Shopify")), "Expected Shopify, got {:?}", tech);
    }

    #[test]
    fn strategy_direct_api_when_api_detected_no_login() {
        let cap = fingerprint_site("api.example.com", API_HTML, "https://api.example.com");
        assert!(cap.has_api);
        assert!(!cap.has_login);
        assert_eq!(cap.optimal_strategy, BrowsingStrategy::DirectAPI);
    }

    #[test]
    fn strategy_browser_ui_for_login_no_api() {
        let cap = fingerprint_site("example.com", LOGIN_HTML, "https://example.com/login");
        assert!(cap.has_login);
        assert!(!cap.has_api);
        assert_eq!(cap.optimal_strategy, BrowsingStrategy::BrowserUI);
    }

    #[test]
    fn strategy_hybrid_when_login_and_api() {
        let html = r#"
            <html><body>
                <form><input type="password" /></form>
                <a href="/api/v1/data">API</a>
            </body></html>
        "#;
        let cap = fingerprint_site("example.com", html, "https://example.com");
        assert!(cap.has_login);
        assert!(cap.has_api);
        assert_eq!(cap.optimal_strategy, BrowsingStrategy::Hybrid);
    }

    #[test]
    fn nav_link_extraction() {
        let cap = fingerprint_site("example.com", NAV_HTML, "https://example.com");
        assert!(!cap.nav_links.is_empty());

        let primary: Vec<_> = cap.nav_links.iter().filter(|l| l.is_primary).collect();
        let secondary: Vec<_> = cap.nav_links.iter().filter(|l| !l.is_primary).collect();

        assert!(primary.len() >= 2, "Expected at least 2 primary nav links, got {}", primary.len());
        assert!(!secondary.is_empty(), "Expected at least 1 non-primary link");

        let texts: Vec<&str> = cap.nav_links.iter().map(|l| l.text.as_str()).collect();
        assert!(texts.contains(&"About Us"));
        assert!(texts.contains(&"Products"));
    }
}
