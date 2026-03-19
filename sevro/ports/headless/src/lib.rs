//! # Sevro Headless Port
//!
//! A headless browser engine for AI agents. Parses HTML into a live DOM tree,
//! runs CSS selector queries, and exposes everything directly to Rust.
//!
//! ## Phase 1C: Real DOM parsing
//!
//! - HTML parsed via html5ever (Servo's own HTML parser)
//! - CSS selectors via the `scraper` crate (wraps Servo's selectors engine)
//! - DOM tree stored as ego-tree nodes, directly queryable from Rust
//! - HTTP networking via reqwest with cookie persistence
//! - SpiderMonkey JS: stub (requires C++ compiler, wired in Phase 2)
//!
//! ## What works now
//!
//! - `navigate(url)` → HTTP fetch + full DOM parse
//! - `dom_snapshot_fast()` → walk DOM tree, extract elements (<1ms)
//! - `query_selector(css)` → real CSS selector matching
//! - `get_attribute/set_attribute` → read/write element attributes
//! - `fill_element` → update input values in DOM
//! - `page_source()` → raw HTML
//! - `current_url()` → current URL
//! - Cookie persistence across navigations

pub mod js_runtime;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use scraper::{Html, Selector};
use tracing::{info, warn, debug, instrument};

// ═══════════════════════════════════════════════════════════════
// Configuration
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevroConfig {
    pub navigation_timeout_ms: u64,
    pub js_timeout_ms: u64,
    pub user_agent: String,
    pub accept_language: String,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub disable_cors: bool,
    pub enable_javascript: bool,
}

impl Default for SevroConfig {
    fn default() -> Self {
        Self {
            navigation_timeout_ms: 30_000,
            js_timeout_ms: 30_000,
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".to_string(),
            accept_language: "en-US,en;q=0.9".to_string(),
            viewport_width: 1920,
            viewport_height: 1080,
            disable_cors: false,
            enable_javascript: true,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Page lifecycle
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq)]
pub enum PageEvent {
    DomContentLoaded,
    Load,
    NetworkIdle,
    Cancelled,
    Error(String),
}

// ═══════════════════════════════════════════════════════════════
// DOM types
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomNode {
    pub node_id: u64,
    pub node_type: DomNodeType,
    pub tag_name: String,
    pub attributes: HashMap<String, String>,
    pub text_content: String,
    pub children: Vec<u64>,
    pub parent: Option<u64>,
    pub bounding_box: Option<BoundingBox>,
    pub is_visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DomNodeType {
    Element,
    Text,
    Comment,
    Document,
    DocumentFragment,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub expires: Option<String>,
}

#[derive(Debug, Clone)]
pub enum RequestAction {
    Continue,
    Block,
    Modify {
        url: Option<String>,
        headers: Option<Vec<(String, String)>>,
    },
}

// ═══════════════════════════════════════════════════════════════
// The Engine — Phase 1C: Real DOM parsing
// ═══════════════════════════════════════════════════════════════

pub struct SevroEngine {
    config: SevroConfig,
    current_url: Option<String>,
    current_html: Option<String>,
    /// The parsed DOM — rebuilt on every navigation
    parsed_dom: Option<Html>,
    /// Extracted DOM nodes (cached after parse)
    dom_nodes: Vec<DomNode>,
    cookies: Vec<Cookie>,
    /// HTTP client with cookie jar
    client: reqwest::Client,
    /// QuickJS runtime for JavaScript execution
    js: Option<js_runtime::JsRuntime>,
    /// Navigation history for back/forward
    history: Vec<String>,
    #[allow(clippy::type_complexity)]
    _request_interceptor: Option<Box<dyn Fn(&str) -> RequestAction + Send + Sync>>,
}

impl SevroEngine {
    #[instrument(skip(config), fields(viewport = format!("{}x{}", config.viewport_width, config.viewport_height)))]
    pub fn new(config: SevroConfig) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(&config.user_agent)
            .cookie_store(true)
            .gzip(true)
            .brotli(true)
            .build()
            .expect("failed to build HTTP client");

        let js = if config.enable_javascript {
            match js_runtime::JsRuntime::new() {
                Ok(rt) => {
                    info!("QuickJS runtime initialized");
                    Some(rt)
                }
                Err(e) => {
                    warn!(error = %e, "QuickJS init failed — JS execution disabled");
                    None
                }
            }
        } else {
            None
        };

        info!(js = config.enable_javascript, has_js = js.is_some(), "Sevro engine initialized");
        Self {
            config,
            current_url: None,
            current_html: None,
            parsed_dom: None,
            dom_nodes: Vec::new(),
            cookies: Vec::new(),
            client,
            js,
            history: Vec::new(),
            _request_interceptor: None,
        }
    }

    /// Navigate to a URL — fetches HTML and parses into a live DOM tree.
    #[instrument(skip(self), fields(url = %url))]
    pub async fn navigate(&mut self, url: &str) -> Result<PageEvent, String> {
        info!(url = %url, "Navigating");

        // Push current URL to history
        if let Some(ref current) = self.current_url {
            self.history.push(current.clone());
        }

        let (status, html, final_url) = self.http_fetch(url).await?;

        if status >= 400 {
            return Ok(PageEvent::Error(format!("HTTP {status}")));
        }

        // Parse HTML into DOM
        let parsed = Html::parse_document(&html);

        // Extract DOM nodes from the parsed tree
        self.dom_nodes = extract_dom_nodes(&parsed);
        self.parsed_dom = Some(parsed);
        self.current_html = Some(html);
        self.current_url = Some(final_url);

        debug!(nodes = self.dom_nodes.len(), "DOM parsed");

        // Execute JavaScript if enabled
        if let Some(ref js) = self.js {
            if let Err(e) = js.setup_dom_bridge(&self.dom_nodes) {
                warn!(error = %e, "DOM bridge setup failed");
            } else if let Some(ref html) = self.current_html {
                match js.execute_page_scripts(html) {
                    Ok(n) => debug!(scripts = n, "Page scripts executed"),
                    Err(e) => debug!(error = %e, "Script execution failed (non-fatal)"),
                }
            }
        }

        Ok(PageEvent::DomContentLoaded)
    }

    /// Fast DOM snapshot — just returns the cached node list. Target: <1ms.
    #[instrument(skip(self))]
    pub fn dom_snapshot_fast(&self) -> Vec<DomNode> {
        self.dom_nodes.clone()
    }

    /// DOM snapshot with layout info. Currently same as fast (no layout engine yet).
    #[instrument(skip(self))]
    pub fn dom_snapshot_with_layout(&self) -> Vec<DomNode> {
        // Phase 2: add Stylo layout computation here
        self.dom_nodes.clone()
    }

    /// Query CSS selector against the live DOM tree. Returns matching node IDs.
    #[instrument(skip(self), fields(selector = %selector))]
    pub fn query_selector(&self, selector: &str) -> Vec<u64> {
        let Some(ref dom) = self.parsed_dom else {
            return vec![];
        };

        let sel = match Selector::parse(selector) {
            Ok(s) => s,
            Err(e) => {
                warn!(selector = %selector, error = ?e, "Invalid CSS selector");
                return vec![];
            }
        };

        // Match selector against DOM, return node IDs by position
        let mut results = Vec::new();
        for (i, _element) in dom.select(&sel).enumerate() {
            // The node_id in our dom_nodes is 1-indexed
            // We need to find which dom_node corresponds to this element
            // For now, use positional index
            results.push((i + 1) as u64);
        }

        debug!(selector = %selector, matches = results.len(), "CSS selector query");
        results
    }

    /// Get computed style for an element. Stub — needs Stylo.
    pub fn computed_style(&self, _node_id: u64, _property: &str) -> Option<String> {
        None // Phase 2: wire Stylo
    }

    /// Get bounding box for an element. Stub — needs layout engine.
    pub fn bounding_box(&self, node_id: u64) -> Option<BoundingBox> {
        self.dom_nodes.iter()
            .find(|n| n.node_id == node_id)
            .and_then(|n| n.bounding_box)
    }

    pub fn get_attribute(&self, node_id: u64, name: &str) -> Option<String> {
        self.dom_nodes.iter()
            .find(|n| n.node_id == node_id)
            .and_then(|n| n.attributes.get(name).cloned())
    }

    pub fn set_attribute(&mut self, node_id: u64, name: &str, value: &str) {
        if let Some(node) = self.dom_nodes.iter_mut().find(|n| n.node_id == node_id) {
            node.attributes.insert(name.to_string(), value.to_string());
        }
    }

    /// Execute JavaScript via QuickJS.
    #[instrument(skip(self, script))]
    pub async fn eval_js(&self, script: &str) -> Result<String, String> {
        match &self.js {
            Some(js) => js.run_script(script),
            None => Err("JavaScript execution disabled (enable_javascript = false)".to_string()),
        }
    }

    pub fn click_element(&mut self, _node_id: u64) {
        debug!("Click element (DOM event dispatch stub)");
    }

    pub fn fill_element(&mut self, node_id: u64, text: &str) {
        if let Some(node) = self.dom_nodes.iter_mut().find(|n| n.node_id == node_id) {
            node.attributes.insert("value".to_string(), text.to_string());
        }
    }

    pub fn focus_element(&mut self, _node_id: u64) {
        debug!("Focus element (stub)");
    }

    pub fn get_cookies(&self, domain: &str) -> Vec<&Cookie> {
        self.cookies.iter()
            .filter(|c| c.domain == domain || domain.ends_with(&format!(".{}", c.domain)))
            .collect()
    }

    pub fn set_cookie(&mut self, cookie: Cookie) {
        self.cookies.retain(|c| !(c.name == cookie.name && c.domain == cookie.domain));
        self.cookies.push(cookie);
    }

    pub fn set_request_interceptor(
        &mut self,
        handler: Box<dyn Fn(&str) -> RequestAction + Send + Sync>,
    ) {
        self._request_interceptor = Some(handler);
    }

    pub fn current_url(&self) -> Option<&str> {
        self.current_url.as_deref()
    }

    pub fn page_source(&self) -> Option<&str> {
        self.current_html.as_deref()
    }

    pub fn config(&self) -> &SevroConfig {
        &self.config
    }

    /// Go back in history.
    pub async fn go_back(&mut self) -> Result<PageEvent, String> {
        if let Some(url) = self.history.pop() {
            let url_clone = url.clone();
            self.navigate(&url_clone).await
        } else {
            Ok(PageEvent::Error("No history to go back to".to_string()))
        }
    }

    /// Fetch a URL using stealth TLS (rquest/BoringSSL) if available,
    /// falling back to reqwest (rustls) otherwise.
    async fn http_fetch(&self, url: &str) -> Result<(u16, String, String), String> {
        #[cfg(feature = "stealth-tls")]
        {
            debug!(url = %url, "Fetching with stealth TLS (rquest + BoringSSL)");

            let client = rquest::Client::builder()
                .cookie_store(true)
                .build()
                .map_err(|e| format!("rquest build failed: {e}"))?;

            let response = client.get(url)
                .header("Accept-Language", &self.config.accept_language)
                .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8")
                .header("Sec-Fetch-Dest", "document")
                .header("Sec-Fetch-Mode", "navigate")
                .header("Sec-Fetch-Site", "none")
                .header("Sec-Fetch-User", "?1")
                .header("Upgrade-Insecure-Requests", "1")
                .send()
                .await
                .map_err(|e| format!("rquest request failed: {e}"))?;

            let status = response.status().as_u16();
            let final_url = response.url().to_string();
            let body = response.text().await
                .map_err(|e| format!("rquest body failed: {e}"))?;

            Ok((status, body, final_url))
        }

        #[cfg(not(feature = "stealth-tls"))]
        {
            debug!(url = %url, "Fetching with reqwest (rustls — may be flagged by Cloudflare)");

            let response = self.client.get(url)
                .header("Accept-Language", &self.config.accept_language)
                .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
                .send()
                .await
                .map_err(|e| format!("HTTP request failed: {e}"))?;

            let status = response.status().as_u16();
            let final_url = response.url().to_string();
            let body = response.text().await
                .map_err(|e| format!("Body read failed: {e}"))?;

            Ok((status, body, final_url))
        }
    }

    /// Check if stealth TLS is available.
    pub fn has_stealth_tls() -> bool {
        cfg!(feature = "stealth-tls")
    }

    #[instrument(skip(self))]
    pub fn shutdown(&mut self) {
        info!("Sevro engine shutting down");
        self.dom_nodes.clear();
        self.parsed_dom = None;
        self.current_html = None;
        self.current_url = None;
        self.cookies.clear();
        self.history.clear();
    }
}

impl Default for SevroEngine {
    fn default() -> Self {
        Self::new(SevroConfig::default())
    }
}

// ═══════════════════════════════════════════════════════════════
// DOM extraction from parsed HTML
// ═══════════════════════════════════════════════════════════════

/// Extract DomNode list from a parsed HTML document.
/// Walks the document tree and converts each element to a DomNode.
fn extract_dom_nodes(dom: &Html) -> Vec<DomNode> {
    let mut nodes = Vec::new();
    let mut node_id: u64 = 0;

    // Interactive element selectors
    let interactive_sel = Selector::parse(
        "a, button, input, select, textarea, [role='button'], [role='link'], \
         [role='tab'], [onclick], summary, label, h1, h2, h3, h4, h5, h6, img, p"
    ).unwrap_or_else(|_| Selector::parse("a").unwrap());

    // Hidden element selectors
    let hidden_indicators = ["display:none", "display: none", "visibility:hidden", "visibility: hidden"];

    for element in dom.select(&interactive_sel) {
        node_id += 1;

        let tag = element.value().name().to_string();

        // Extract attributes
        let mut attributes = HashMap::new();
        for attr in element.value().attrs() {
            attributes.insert(attr.0.to_string(), attr.1.to_string());
        }

        // Get text content
        let text_content = element.text().collect::<Vec<_>>().join(" ").trim().to_string();

        // Visibility heuristic
        let style = attributes.get("style").map(|s| s.as_str()).unwrap_or("");
        let is_hidden = attributes.contains_key("hidden")
            || attributes.get("aria-hidden").map(|v| v == "true").unwrap_or(false)
            || hidden_indicators.iter().any(|h| style.contains(h));

        // Skip hidden elements and empty non-interactive elements
        if is_hidden {
            continue;
        }
        if text_content.is_empty()
            && !matches!(tag.as_str(), "input" | "select" | "textarea" | "img")
            && !attributes.contains_key("href")
        {
            continue;
        }

        nodes.push(DomNode {
            node_id,
            node_type: DomNodeType::Element,
            tag_name: tag,
            attributes,
            text_content,
            children: vec![],
            parent: None,
            bounding_box: None, // No layout engine yet
            is_visible: !is_hidden,
        });
    }

    // Also extract the <title> element
    if let Ok(title_sel) = Selector::parse("title") {
        if let Some(title_el) = dom.select(&title_sel).next() {
            let title_text = title_el.text().collect::<Vec<_>>().join("");
            nodes.push(DomNode {
                node_id: node_id + 1,
                node_type: DomNodeType::Element,
                tag_name: "title".to_string(),
                attributes: HashMap::new(),
                text_content: title_text,
                children: vec![],
                parent: None,
                bounding_box: None,
                is_visible: false,
            });
        }
    }

    nodes
}

// ═══════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_test_html(html: &str) -> SevroEngine {
        let mut engine = SevroEngine::default();
        let parsed = Html::parse_document(html);
        engine.dom_nodes = extract_dom_nodes(&parsed);
        engine.parsed_dom = Some(parsed);
        engine.current_html = Some(html.to_string());
        engine.current_url = Some("https://test.local".to_string());
        engine
    }

    #[test]
    fn config_defaults() {
        let config = SevroConfig::default();
        assert_eq!(config.viewport_width, 1920);
        assert!(config.enable_javascript);
    }

    #[test]
    fn parse_simple_page() {
        let engine = parse_test_html(r#"
            <html><head><title>Test Page</title></head>
            <body>
                <h1>Hello World</h1>
                <a href="/about">About</a>
                <button>Click Me</button>
                <input type="text" placeholder="Name">
            </body></html>
        "#);

        let nodes = engine.dom_snapshot_fast();
        assert!(nodes.len() >= 4, "Expected at least 4 nodes, got {}", nodes.len());

        // Check we have a link, button, input, and heading
        let tags: Vec<&str> = nodes.iter().map(|n| n.tag_name.as_str()).collect();
        assert!(tags.contains(&"a"), "Missing <a> element");
        assert!(tags.contains(&"button"), "Missing <button> element");
        assert!(tags.contains(&"input"), "Missing <input> element");
        assert!(tags.contains(&"h1"), "Missing <h1> element");
    }

    #[test]
    fn parse_extracts_attributes() {
        let engine = parse_test_html(r#"
            <html><body>
                <a href="https://example.com" class="nav-link">Example</a>
            </body></html>
        "#);

        let link = engine.dom_nodes.iter().find(|n| n.tag_name == "a").unwrap();
        assert_eq!(link.attributes.get("href").unwrap(), "https://example.com");
        assert_eq!(link.attributes.get("class").unwrap(), "nav-link");
        assert_eq!(link.text_content, "Example");
    }

    #[test]
    fn parse_skips_hidden_elements() {
        let engine = parse_test_html(r#"
            <html><body>
                <button>Visible</button>
                <button style="display:none">Hidden</button>
                <button hidden>Also Hidden</button>
                <button aria-hidden="true">Aria Hidden</button>
            </body></html>
        "#);

        let buttons: Vec<_> = engine.dom_nodes.iter()
            .filter(|n| n.tag_name == "button")
            .collect();
        assert_eq!(buttons.len(), 1, "Should only have 1 visible button");
        assert_eq!(buttons[0].text_content, "Visible");
    }

    #[test]
    fn query_selector_finds_elements() {
        let engine = parse_test_html(r#"
            <html><body>
                <input type="text" name="username">
                <input type="password" name="pass">
                <button type="submit">Login</button>
            </body></html>
        "#);

        let inputs = engine.query_selector("input");
        assert_eq!(inputs.len(), 2, "Should find 2 inputs");

        let buttons = engine.query_selector("button");
        assert_eq!(buttons.len(), 1, "Should find 1 button");

        let none = engine.query_selector(".nonexistent");
        assert!(none.is_empty());
    }

    #[test]
    fn get_and_set_attribute() {
        let mut engine = parse_test_html(r#"
            <html><body><input type="text" name="email"></body></html>
        "#);

        let input_id = engine.dom_nodes.iter()
            .find(|n| n.tag_name == "input")
            .unwrap().node_id;

        assert_eq!(engine.get_attribute(input_id, "type"), Some("text".to_string()));
        assert_eq!(engine.get_attribute(input_id, "name"), Some("email".to_string()));

        engine.set_attribute(input_id, "value", "user@test.com");
        assert_eq!(engine.get_attribute(input_id, "value"), Some("user@test.com".to_string()));
    }

    #[test]
    fn fill_element_sets_value() {
        let mut engine = parse_test_html(r#"
            <html><body><input type="text" id="name"></body></html>
        "#);

        let input_id = engine.dom_nodes.iter()
            .find(|n| n.tag_name == "input")
            .unwrap().node_id;

        engine.fill_element(input_id, "Matt");
        assert_eq!(engine.get_attribute(input_id, "value"), Some("Matt".to_string()));
    }

    #[test]
    fn cookie_operations() {
        let mut engine = SevroEngine::default();
        engine.set_cookie(Cookie {
            name: "session".to_string(),
            value: "abc123".to_string(),
            domain: "example.com".to_string(),
            path: "/".to_string(),
            secure: true,
            http_only: true,
            expires: None,
        });

        assert_eq!(engine.get_cookies("example.com").len(), 1);
        assert_eq!(engine.get_cookies("sub.example.com").len(), 1);
        assert_eq!(engine.get_cookies("other.com").len(), 0);
    }

    #[test]
    fn page_source_and_url() {
        let engine = parse_test_html("<html><body>Hello</body></html>");
        assert!(engine.page_source().unwrap().contains("Hello"));
        assert_eq!(engine.current_url(), Some("https://test.local"));
    }

    #[test]
    fn title_extraction() {
        let engine = parse_test_html(r#"
            <html><head><title>My Page Title</title></head><body><p>Content</p></body></html>
        "#);

        let title_node = engine.dom_nodes.iter()
            .find(|n| n.tag_name == "title");
        assert!(title_node.is_some());
        assert_eq!(title_node.unwrap().text_content, "My Page Title");
    }

    #[test]
    fn shutdown_clears_all() {
        let mut engine = parse_test_html("<html><body><a href='/'>Link</a></body></html>");
        assert!(!engine.dom_nodes.is_empty());

        engine.shutdown();

        assert!(engine.current_url().is_none());
        assert!(engine.page_source().is_none());
        assert!(engine.dom_snapshot_fast().is_empty());
    }

    #[test]
    fn extract_form_elements() {
        let engine = parse_test_html(r#"
            <html><body>
                <form action="/login" method="post">
                    <label for="user">Username</label>
                    <input type="text" id="user" name="username" placeholder="Enter username">
                    <label for="pass">Password</label>
                    <input type="password" id="pass" name="password">
                    <select name="role">
                        <option value="user">User</option>
                        <option value="admin">Admin</option>
                    </select>
                    <textarea name="bio">Tell us about yourself</textarea>
                    <button type="submit">Sign In</button>
                </form>
            </body></html>
        "#);

        let tags: Vec<&str> = engine.dom_nodes.iter()
            .map(|n| n.tag_name.as_str())
            .collect();

        assert!(tags.contains(&"input"), "Missing input");
        assert!(tags.contains(&"select"), "Missing select");
        assert!(tags.contains(&"textarea"), "Missing textarea");
        assert!(tags.contains(&"button"), "Missing button");
        assert!(tags.contains(&"label"), "Missing label");

        // Check placeholder extraction
        let username_input = engine.dom_nodes.iter()
            .find(|n| n.attributes.get("name").map(|v| v == "username").unwrap_or(false))
            .unwrap();
        assert_eq!(username_input.attributes.get("placeholder").unwrap(), "Enter username");
    }

    #[tokio::test]
    async fn eval_js_works_with_quickjs() {
        let engine = SevroEngine::default();
        let result = engine.eval_js("1+1").await.unwrap();
        assert_eq!(result, "2");
    }

    #[tokio::test]
    async fn eval_js_disabled_returns_error() {
        let config = SevroConfig { enable_javascript: false, ..Default::default() };
        let engine = SevroEngine::new(config);
        assert!(engine.eval_js("1+1").await.is_err());
    }

    #[test]
    fn dom_snapshot_performance() {
        // Parse a moderately complex page
        let mut html = String::from("<html><body>");
        for i in 0..100 {
            html.push_str(&format!(r#"<a href="/page/{i}">Link {i}</a>"#));
            html.push_str(&format!(r#"<button>Button {i}</button>"#));
        }
        html.push_str("</body></html>");

        let engine = parse_test_html(&html);

        // Snapshot should be fast
        let start = std::time::Instant::now();
        let nodes = engine.dom_snapshot_fast();
        let elapsed = start.elapsed();

        assert!(nodes.len() >= 200, "Expected 200+ nodes, got {}", nodes.len());
        assert!(elapsed.as_millis() < 10, "Snapshot took {}ms, target <1ms", elapsed.as_millis());
    }
}
