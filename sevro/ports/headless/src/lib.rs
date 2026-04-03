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

#[cfg(feature = "std")]
pub mod js_runtime;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use scraper::{Html, Selector};
#[cfg(feature = "std")]
use std::sync::Arc;
#[cfg(feature = "std")]
use std::time::{Duration, Instant};
use tracing::{info, warn, debug, instrument};
#[cfg(feature = "stealth-tls")]
use rquest_util::Emulation;

#[cfg(feature = "std")]
use wraith_transport::{HttpTransport, TransportRequest, TransportMethod};

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
    /// HTTP/HTTPS/SOCKS5 proxy URL (e.g., "http://user:pass@proxy:8080" or "socks5://127.0.0.1:1080")
    pub proxy_url: Option<String>,
    /// External challenge solver URL for protected page access (e.g., "http://localhost:8191")
    /// Only used as fallback when QuickJS can't solve the challenge.
    pub flaresolverr_url: Option<String>,
    /// Fallback proxy URL used only when an access restriction is detected.
    /// Separate from proxy_url so the primary path stays direct.
    pub fallback_proxy_url: Option<String>,
}

impl Default for SevroConfig {
    fn default() -> Self {
        Self {
            navigation_timeout_ms: 30_000,
            js_timeout_ms: 30_000,
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:136.0) Gecko/20100101 Firefox/136.0".to_string(),
            accept_language: "en-US,en;q=0.5".to_string(),
            viewport_width: 1920,
            viewport_height: 1080,
            disable_cors: false,
            enable_javascript: true,
            proxy_url: None,
            flaresolverr_url: None,
            fallback_proxy_url: None,
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
    /// Can be clicked/filled — visible AND not pointer-events:none
    pub is_interactive: bool,
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
    /// Pluggable HTTP transport (default: ReqwestTransport).
    /// Used by the main fetch paths (http_fetch, http_get).
    #[cfg(feature = "std")]
    transport: Arc<dyn HttpTransport>,
    /// Raw reqwest client — retained for methods that need reqwest-specific
    /// features (cookie jar, .json(), .multipart(), proxy client builders).
    #[cfg(feature = "std")]
    client: reqwest::Client,
    /// QuickJS runtime for JavaScript execution
    #[cfg(feature = "std")]
    js: Option<js_runtime::JsRuntime>,
    /// Navigation history for back/forward
    history: Vec<String>,
    #[allow(clippy::type_complexity)]
    _request_interceptor: Option<Box<dyn Fn(&str) -> RequestAction + Send + Sync>>,
    /// Resolved iframe contents: maps iframe node_id to (src_url, parsed DomNodes)
    iframe_contents: HashMap<u64, (String, Vec<DomNode>)>,
    /// Fingerprint overrides for DOM-level spoofing (Camoufox-style).
    /// Keys are dot-notation property paths (e.g. "navigator.userAgent").
    fingerprint: Option<HashMap<String, serde_json::Value>>,
}

// SAFETY: SevroEngine is always accessed behind Arc<Mutex<...>>, guaranteeing
// single-threaded access. The non-Send/Sync types (rquickjs Rc<Runtime>,
// scraper::Html with tendril::NonAtomic) are never shared across threads —
// the Mutex serializes all access. This is the standard pattern for wrapping
// single-threaded libraries in async Rust (e.g., SQLite, QuickJS).
unsafe impl Send for SevroEngine {}
unsafe impl Sync for SevroEngine {}

impl SevroEngine {
    #[cfg(feature = "std")]
    #[instrument(skip(config), fields(viewport = format!("{}x{}", config.viewport_width, config.viewport_height)))]
    pub fn new(config: SevroConfig) -> Self {
        let mut client_builder = reqwest::Client::builder()
            .user_agent(&config.user_agent)
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::limited(10))
            .gzip(true)
            .brotli(true);

        if let Some(ref proxy_url) = config.proxy_url {
            match reqwest::Proxy::all(proxy_url) {
                Ok(proxy) => {
                    info!(proxy = %proxy_url, "HTTP proxy configured");
                    client_builder = client_builder.proxy(proxy);
                }
                Err(e) => {
                    warn!(error = %e, proxy = %proxy_url, "Failed to configure proxy — continuing without");
                }
            }
        }

        let client = client_builder.build().expect("failed to build HTTP client");

        // Build a transport-layer reqwest client with the same settings.
        // This mirrors the main client's config so the transport behaves identically.
        let mut transport_builder = reqwest::Client::builder()
            .user_agent(&config.user_agent)
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::limited(10))
            .gzip(true)
            .brotli(true);
        if let Some(ref proxy_url) = config.proxy_url {
            if let Ok(proxy) = reqwest::Proxy::all(proxy_url) {
                transport_builder = transport_builder.proxy(proxy);
            }
        }
        let transport_client = transport_builder.build().expect("failed to build transport client");
        let transport: Arc<dyn HttpTransport> = Arc::new(
            wraith_transport::ReqwestTransport::with_client(transport_client),
        );

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
            transport,
            client,
            js,
            history: Vec::new(),
            _request_interceptor: None,
            iframe_contents: HashMap::new(),
            fingerprint: None,
        }
    }

    #[cfg(not(feature = "std"))]
    pub fn new(config: SevroConfig) -> Self {
        info!("Sevro engine initialized (no-std: networking and JS disabled)");
        Self {
            config,
            current_url: None,
            current_html: None,
            parsed_dom: None,
            dom_nodes: Vec::new(),
            cookies: Vec::new(),
            history: Vec::new(),
            _request_interceptor: None,
            iframe_contents: HashMap::new(),
            fingerprint: None,
        }
    }

    /// Set or replace the fingerprint overrides used for DOM-level spoofing.
    /// Keys are dot-notation property paths (e.g. "window.innerWidth", "navigator.userAgent").
    pub fn set_fingerprint(&mut self, fp: HashMap<String, serde_json::Value>) {
        self.fingerprint = Some(fp);
    }

    /// Navigate to a URL — fetches HTML and parses into a live DOM tree.
    ///
    /// ## Fallback chain (each tier only fires if the previous fails):
    ///
    /// 1. **Direct fetch** — compatible TLS + Chrome headers (fastest, ~50ms)
    /// 2. **QuickJS challenge solver** — if challenge page "Just a moment..." detected
    /// 3. **External solver** — if QuickJS can't solve (obfuscated challenge)
    /// 4. **Fallback proxy** — if access restriction detected ("you have been blocked")
    #[cfg(feature = "std")]
    #[instrument(skip(self), fields(url = %url))]
    pub async fn navigate(&mut self, url: &str) -> Result<PageEvent, String> {
        info!(url = %url, "Navigating");

        // Push current URL to history
        if let Some(ref current) = self.current_url {
            self.history.push(current.clone());
        }

        // === Tier 1: Direct fetch ===
        let (status, html, final_url, set_cookies) = self.http_fetch(url).await?;

        // Store Set-Cookie headers from the redirect chain
        self.store_set_cookies(&set_cookies, &final_url);

        if status >= 400 {
            warn!(status, url = %url, body_len = html.len(), "HTTP error status — parsing body anyway");
        }

        // Parse HTML and run inline scripts
        self.load_page(&html, &final_url);

        // Wait for page stability (async rendering, setTimeout callbacks, etc.)
        self.wait_for_stability(500).await;

        // SPA handling: if the page has very few visible elements, try platform-specific APIs
        let is_challenge = Self::is_cloudflare_challenge(&html, status);
        let is_blocked = Self::is_ip_blocked(&html);
        debug!(is_challenge, is_blocked, status, html_len = html.len(), "Challenge/block detection result");
        if !is_challenge && !is_blocked {
            // Known platform API hydration — always try regardless of element count.
            // These platforms serve shell HTML with navigation chrome but load
            // actual content (job listings) via their own APIs.
            if final_url.contains("ashbyhq.com") {
                self.try_ashby_api_hydration(&final_url).await;
            } else if final_url.contains("greenhouse.io/embed/job_board") {
                self.try_greenhouse_embed_hydration(&final_url).await;
            } else if html.contains("talentbrew.com") || html.contains("SearchAsService") {
                self.try_radancy_api_hydration(&final_url, &html).await;
            } else if html.contains("phenompeople.com") || html.contains("phenom.com") {
                self.try_phenom_api_hydration(&final_url, &html).await;
            } else if final_url.contains("myworkdayjobs.com") {
                self.try_workday_api_hydration(&final_url, &html).await;
            } else {
                // Unknown platform — only try generic SPA hydration if page looks empty
                let interactive_count = self.dom_nodes.iter()
                    .filter(|n| n.is_visible && matches!(n.tag_name.as_str(),
                        "a" | "button" | "input" | "select" | "textarea" | "h1" | "h2" | "h3" | "p"))
                    .count();
                if interactive_count < 5 {
                    self.try_spa_hydration(&final_url).await;
                }
            }
            return Ok(PageEvent::DomContentLoaded);
        }

        // === Tier 2: QuickJS challenge solver (for "Just a moment..." pages) ===
        if Self::is_cloudflare_challenge(&html, status) && !Self::is_ip_blocked(&html) {
            info!(url = %url, status, "Challenge page detected — Tier 2: QuickJS solver");

            if let Some(cookies) = self.try_quickjs_solve(url).await {
                let retry = self.http_fetch_with_cookies(url, &cookies).await;
                if let Ok((rs, rh, ru)) = retry {
                    if !Self::is_cloudflare_challenge(&rh, rs) && !Self::is_ip_blocked(&rh) {
                        info!(status = rs, "Tier 2 resolved — QuickJS solved challenge");
                        self.load_page(&rh, &ru);
                        return Ok(PageEvent::DomContentLoaded);
                    }
                }
            }

            // === Tier 3: External solver (for obfuscated challenges) ===
            // Strategy: use the external solver's full page response directly.
            // Cookie replay usually fails because cookies are tied to
            // the solver's browser profile, not ours.
            if self.config.flaresolverr_url.is_some() {
                info!(url = %url, "Tier 3: Escalating to external solver");

                if let Some(page_html) = self.try_flaresolverr_full_page(url).await {
                    if !Self::is_ip_blocked(&page_html) {
                        info!(html_len = page_html.len(), "Tier 3 resolved — external solver returned real page");
                        self.load_page(&page_html, url);
                        return Ok(PageEvent::DomContentLoaded);
                    }
                }
            }
        }

        // === Tier 3.5: External solver for access restrictions too ===
        // The external solver has its own browser + IP — it can resolve both
        // challenges AND access restrictions since it runs on a different machine.
        if Self::is_ip_blocked(&html) && self.config.flaresolverr_url.is_some() {
            info!(url = %url, "Tier 3: Access restricted — external solver has its own browser+IP, trying it");

            if let Some(cookies) = self.try_flaresolverr(url).await {
                let retry = self.http_fetch_with_cookies(url, &cookies).await;
                if let Ok((rs, rh, ru)) = retry {
                    if !Self::is_ip_blocked(&rh) && !Self::is_cloudflare_challenge(&rh, rs) {
                        info!(status = rs, "External solver resolved access restriction + challenge");
                        self.load_page(&rh, &ru);
                        return Ok(PageEvent::DomContentLoaded);
                    }
                }

                // Even if our IP can't use the cookies, the external solver may have
                // returned the actual page content in its response
                if let Some(page_html) = self.try_flaresolverr_full_page(url).await {
                    if !Self::is_ip_blocked(&page_html) && !Self::is_cloudflare_challenge(&page_html, 200) {
                        info!("External solver returned full page content directly");
                        self.load_page(&page_html, url);
                        return Ok(PageEvent::DomContentLoaded);
                    }
                }
            }
        }

        // === Tier 4: Fallback proxy (for access restrictions when no external solver) ===
        if Self::is_ip_blocked(&html) {
            if let Some(ref fallback_proxy) = self.config.fallback_proxy_url.clone() {
                info!(url = %url, proxy = %fallback_proxy, "Tier 4: Access restricted — retrying via fallback proxy");

                if let Ok((ps, ph, pu)) = self.http_fetch_via_proxy(url, fallback_proxy).await {
                    if !Self::is_ip_blocked(&ph) {
                        info!(status = ps, "Tier 4 resolved — proxy resolved access restriction");
                        self.load_page(&ph, &pu);
                        return Ok(PageEvent::DomContentLoaded);
                    }
                }
            } else if self.config.flaresolverr_url.is_none() {
                warn!(url = %url, "Access restricted — configure --flaresolverr or --fallback-proxy to resolve");
            }
        }

        Ok(PageEvent::DomContentLoaded)
    }

    /// Ashby API-native hydration: fetch form definition via GraphQL and build synthetic DOM.
    /// This accesses the API directly — no React, no ES modules, just direct API access.
    #[cfg(feature = "std")]
    async fn try_ashby_api_hydration(&mut self, page_url: &str) {
        // Extract company name and job ID from URL
        // Format: https://jobs.ashbyhq.com/{company}/{job_id}/application
        let parsed = match url::Url::parse(page_url) {
            Ok(u) => u,
            Err(_) => return,
        };
        let segments: Vec<&str> = parsed.path().trim_matches('/').split('/').collect();
        if segments.len() < 2 { return; }
        let company = segments[0];
        let job_id = segments[1];

        info!(company = %company, job_id = %job_id, "Ashby: fetching form via GraphQL API");

        // Query the GraphQL API for form definition
        let query = serde_json::json!({
            "operationName": "ApiJobPostingWithBoard",
            "variables": {
                "organizationHostedJobsPageName": company,
                "jobPostingId": job_id
            },
            "query": "query ApiJobPostingWithBoard($organizationHostedJobsPageName: String!, $jobPostingId: String!) { jobPosting(organizationHostedJobsPageName: $organizationHostedJobsPageName, jobPostingId: $jobPostingId) { id title descriptionPlain applicationForm { sections { title fieldEntries { ... on FormFieldEntry { field descriptionHtml } } } } } }"
        });

        let resp = match self.client
            .post("https://jobs.ashbyhq.com/api/non-user-graphql")
            .header("Content-Type", "application/json")
            .json(&query)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                debug!(error = %e, "Ashby GraphQL request failed");
                return;
            }
        };

        let body = match resp.text().await {
            Ok(t) => t,
            Err(_) => return,
        };

        let data: serde_json::Value = match serde_json::from_str(&body) {
            Ok(d) => d,
            Err(_) => return,
        };

        let posting = match data.get("data").and_then(|d| d.get("jobPosting")) {
            Some(p) => p,
            None => {
                debug!("Ashby: no jobPosting in GraphQL response");
                return;
            }
        };

        let title = posting.get("title").and_then(|t| t.as_str()).unwrap_or("Job Application");
        let description = posting.get("descriptionPlain").and_then(|d| d.as_str()).unwrap_or("");

        // Build synthetic HTML from the form definition
        let mut html = format!(
            r#"<html><head><title>{} @ {}</title></head><body>
<h1>{}</h1>
<p>{}</p>
<form id="ashby-application" action="https://jobs.ashbyhq.com/api/non-user-graphql" method="POST" data-company="{}" data-job-id="{}">
"#,
            title, company, title,
            if description.len() > 500 { &description[..500] } else { description },
            company, job_id
        );

        let sections = posting.get("applicationForm")
            .and_then(|f| f.get("sections"))
            .and_then(|s| s.as_array());

        if let Some(sections) = sections {
            for section in sections {
                let section_title = section.get("title").and_then(|t| t.as_str()).unwrap_or("");
                if !section_title.is_empty() {
                    html.push_str(&format!("<fieldset><legend>{}</legend>\n", section_title));
                }

                if let Some(entries) = section.get("fieldEntries").and_then(|e| e.as_array()) {
                    for entry in entries {
                        let field = match entry.get("field") {
                            Some(f) => f,
                            None => continue,
                        };

                        let field_title = field.get("title").and_then(|t| t.as_str()).unwrap_or("");
                        let field_type = field.get("type").and_then(|t| t.as_str()).unwrap_or("String");
                        let field_path = field.get("path").and_then(|p| p.as_str()).unwrap_or("");
                        let field_id = field.get("id").and_then(|i| i.as_str()).unwrap_or("");
                        let required = field.get("isNullable").and_then(|n| n.as_bool()).map(|n| !n).unwrap_or(true);
                        let req_attr = if required { " required" } else { "" };

                        html.push_str(&format!("<label for=\"{}\">{}</label>\n", field_path, field_title));

                        match field_type {
                            "String" | "Email" | "Phone" | "LongText" => {
                                let input_type = match field_type {
                                    "Email" => "email",
                                    "Phone" => "tel",
                                    "LongText" => "textarea",
                                    _ => "text",
                                };
                                if input_type == "textarea" {
                                    html.push_str(&format!(
                                        "<textarea id=\"{}\" name=\"{}\" data-field-id=\"{}\"{}></textarea>\n",
                                        field_path, field_path, field_id, req_attr
                                    ));
                                } else {
                                    html.push_str(&format!(
                                        "<input type=\"{}\" id=\"{}\" name=\"{}\" data-field-id=\"{}\"{}>\n",
                                        input_type, field_path, field_path, field_id, req_attr
                                    ));
                                }
                            }
                            "ValueSelect" => {
                                html.push_str(&format!(
                                    "<select id=\"{}\" name=\"{}\" data-field-id=\"{}\"{}>\n<option value=\"\">Select...</option>\n",
                                    field_path, field_path, field_id, req_attr
                                ));
                                if let Some(values) = field.get("selectableValues").and_then(|v| v.as_array()) {
                                    for val in values {
                                        let label = val.get("label").and_then(|l| l.as_str()).unwrap_or("");
                                        let value = val.get("value").and_then(|v| v.as_str()).unwrap_or(label);
                                        html.push_str(&format!("<option value=\"{}\">{}</option>\n", value, label));
                                    }
                                }
                                html.push_str("</select>\n");
                            }
                            "File" => {
                                html.push_str(&format!(
                                    "<input type=\"file\" id=\"{}\" name=\"{}\" data-field-id=\"{}\"{}>\n",
                                    field_path, field_path, field_id, req_attr
                                ));
                            }
                            "Boolean" => {
                                html.push_str(&format!(
                                    "<input type=\"checkbox\" id=\"{}\" name=\"{}\" data-field-id=\"{}\">\n",
                                    field_path, field_path, field_id
                                ));
                            }
                            _ => {
                                html.push_str(&format!(
                                    "<input type=\"text\" id=\"{}\" name=\"{}\" data-field-id=\"{}\"{}>\n",
                                    field_path, field_path, field_id, req_attr
                                ));
                            }
                        }
                    }
                }

                if !section_title.is_empty() {
                    html.push_str("</fieldset>\n");
                }
            }
        }

        html.push_str("<button type=\"submit\">Submit Application</button>\n</form>\n</body></html>");

        // Replace the page with the synthetic form HTML
        info!(html_len = html.len(), fields = sections.map(|s| s.len()).unwrap_or(0), "Ashby: built synthetic form from GraphQL API");
        self.load_page(&html, page_url);
    }

    /// Greenhouse embed hydration: fetch job listings via the Greenhouse Boards API
    /// when the URL is a `boards.greenhouse.io/embed/job_board?for=<company>` embed.
    #[cfg(feature = "std")]
    async fn try_greenhouse_embed_hydration(&mut self, page_url: &str) {
        let parsed = match url::Url::parse(page_url) {
            Ok(u) => u,
            Err(_) => return,
        };

        // Extract company name from the `for` query parameter
        let company = match parsed.query_pairs().find(|(k, _)| k == "for").map(|(_, v)| v.to_string()) {
            Some(c) if !c.is_empty() => c,
            _ => {
                debug!("Greenhouse embed: no 'for' query parameter found in URL");
                return;
            }
        };

        info!(company = %company, "Greenhouse embed: fetching jobs via Boards API");

        let api_url = format!(
            "https://boards-api.greenhouse.io/v1/boards/{}/jobs?content=true",
            company
        );

        let resp = match self.client.get(&api_url).send().await {
            Ok(r) => r,
            Err(e) => {
                debug!(error = %e, "Greenhouse Boards API request failed");
                return;
            }
        };

        let body = match resp.text().await {
            Ok(t) => t,
            Err(_) => return,
        };

        let data: serde_json::Value = match serde_json::from_str(&body) {
            Ok(d) => d,
            Err(_) => return,
        };

        let jobs = match data.get("jobs").and_then(|j| j.as_array()) {
            Some(j) => j,
            None => {
                debug!("Greenhouse embed: no jobs array in API response");
                return;
            }
        };

        // Build synthetic HTML from the job listings
        let mut html = format!(
            r#"<html><head><title>{} — Open Positions</title></head><body>
<h1>{} — Open Positions</h1>
<div id="greenhouse-job-list">
"#,
            company, company
        );

        for job in jobs {
            let title = job.get("title").and_then(|t| t.as_str()).unwrap_or("Untitled");
            let job_id = job.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
            let location = job.get("location").and_then(|l| l.get("name")).and_then(|n| n.as_str()).unwrap_or("");
            let abs_url = job.get("absolute_url").and_then(|u| u.as_str()).unwrap_or("#");
            let content = job.get("content").and_then(|c| c.as_str()).unwrap_or("");

            html.push_str(&format!(
                r#"<div class="greenhouse-job" data-job-id="{}">
<h2><a href="{}">{}</a></h2>
<p class="location">{}</p>
<div class="description">{}</div>
</div>
"#,
                job_id, abs_url, title, location,
                if content.len() > 500 { &content[..500] } else { content }
            ));
        }

        html.push_str("</div>\n</body></html>");

        info!(html_len = html.len(), job_count = jobs.len(), "Greenhouse embed: built synthetic listing from Boards API");
        self.load_page(&html, page_url);
    }

    /// Radancy/TalentBrew hydration: fetch job search results via the TalentBrew Search API
    /// when the page contains Radancy meta tags (used by Boeing, L3Harris, Lockheed Martin, etc.).
    #[cfg(feature = "std")]
    async fn try_radancy_api_hydration(&mut self, page_url: &str, html: &str) {
        let parsed = match url::Url::parse(page_url) {
            Ok(u) => u,
            Err(_) => return,
        };

        // Extract org ID from meta tag: <meta name="site-organization-id" content="185">
        let org_id = {
            let marker = "name=\"site-organization-id\"";
            if let Some(pos) = html.find(marker) {
                let after = &html[pos + marker.len()..];
                if let Some(cstart) = after.find("content=\"") {
                    let val_start = cstart + 9;
                    if let Some(cend) = after[val_start..].find('"') {
                        Some(after[val_start..val_start + cend].to_string())
                    } else { None }
                } else { None }
            } else { None }
        };

        let org_id = match org_id {
            Some(id) if !id.is_empty() => id,
            _ => {
                debug!("Radancy: no site-organization-id meta tag found");
                return;
            }
        };

        // Extract search keyword from URL query parameters
        let keyword = parsed.query_pairs()
            .find(|(k, _)| k == "keyword" || k == "SearchText")
            .map(|(_, v)| v.to_string())
            .unwrap_or_default();

        let base_url_str = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""));

        info!(org_id = %org_id, keyword = %keyword, host = %base_url_str, "Radancy: fetching jobs via TalentBrew Search API");

        let api_url = format!(
            "{}/search-jobs/results?ActiveFacetID=0&CurrentPage=1&OrgIds={}&RecordsPerPage=25&SearchResultsModuleName=Search+Results&SearchText={}",
            base_url_str, org_id, keyword
        );

        let resp = match self.client
            .get(&api_url)
            .header("X-Requested-With", "XMLHttpRequest")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                debug!(error = %e, "Radancy TalentBrew API request failed");
                return;
            }
        };

        let body = match resp.text().await {
            Ok(t) => t,
            Err(_) => return,
        };

        let data: serde_json::Value = match serde_json::from_str(&body) {
            Ok(d) => d,
            Err(_) => {
                debug!("Radancy: failed to parse API response as JSON");
                return;
            }
        };

        let results_html = match data.get("results").and_then(|r| r.as_str()) {
            Some(r) if !r.is_empty() => r,
            _ => {
                debug!("Radancy: no results HTML in API response");
                return;
            }
        };

        // Wrap the results HTML in a full page structure
        let host = parsed.host_str().unwrap_or("careers");
        let full_html = format!(
            r#"<html><head><title>{} — Job Search Results</title></head><body>
<h1>Job Search Results</h1>
<div id="radancy-job-list">
{}
</div>
</body></html>"#,
            host, results_html
        );

        info!(html_len = full_html.len(), "Radancy: built job listing from TalentBrew Search API");
        self.load_page(&full_html, page_url);
    }

    /// Phenom People hydration: fetch job search results via the Phenom /widgets API
    /// when the page contains Phenom scripts (used by RTX/Raytheon, MITRE, etc.).
    #[cfg(feature = "std")]
    async fn try_phenom_api_hydration(&mut self, page_url: &str, _html: &str) {
        let parsed = match url::Url::parse(page_url) {
            Ok(u) => u,
            Err(_) => return,
        };

        let host = match parsed.host_str() {
            Some(h) => h.to_string(),
            None => return,
        };

        // Extract lang/country from URL path segments like /us/en/ or /global/en/
        let segments: Vec<&str> = parsed.path().trim_matches('/').split('/').collect();
        let (lang, country) = {
            let mut country_val = "us".to_string();
            let mut lang_code = "en".to_string();
            // Look for patterns like /us/en/ or /global/en/
            if segments.len() >= 2 {
                let first = segments[0].to_lowercase();
                let second = segments[1].to_lowercase();
                // Check if first segment looks like a country/region and second like a language
                if second.len() == 2 && first.len() <= 6 {
                    country_val = first;
                    lang_code = second;
                }
            }
            let lang = format!("{}_{}", lang_code, country_val);
            (lang, country_val)
        };

        // Extract search keywords from query parameter
        let keywords = parsed
            .query_pairs()
            .find(|(k, _)| k == "keywords" || k == "q")
            .map(|(_, v)| v.to_string())
            .unwrap_or_default();

        info!(host = %host, lang = %lang, country = %country, keywords = %keywords, "Phenom: fetching jobs via /widgets API");

        let base_url = format!("{}://{}", parsed.scheme(), host);
        let api_url = format!("{}/widgets", base_url);

        let payload = serde_json::json!({
            "lang": lang,
            "deviceType": "desktop",
            "country": country,
            "pageName": "search-results",
            "ddoKey": "refineSearch",
            "from": 0,
            "s": 1,
            "rk": format!("l-{}-search-results", lang),
            "jobs": true,
            "counts": true,
            "all_fields": ["category", "country", "state", "city", "type", "subCategory"],
            "size": 25,
            "keywords": keywords
        });

        let resp = match self.client
            .post(&api_url)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                debug!(error = %e, "Phenom /widgets API request failed");
                return;
            }
        };

        let body = match resp.text().await {
            Ok(t) => t,
            Err(_) => return,
        };

        let data: serde_json::Value = match serde_json::from_str(&body) {
            Ok(d) => d,
            Err(_) => return,
        };

        let refine = match data.get("refineSearch").and_then(|r| r.get("data")) {
            Some(d) => d,
            None => {
                debug!("Phenom: no refineSearch.data in API response");
                return;
            }
        };

        let jobs = match refine.get("jobs").and_then(|j| j.as_array()) {
            Some(j) => j,
            None => {
                debug!("Phenom: no jobs array in API response");
                return;
            }
        };

        let total_count = refine.get("totalCount").and_then(|t| t.as_u64()).unwrap_or(jobs.len() as u64);

        // Build synthetic HTML from the job listings
        let mut synthetic_html = format!(
            r#"<html><head><title>{} Jobs - Search Results</title></head>
<body>
<h1>Search Results ({} jobs)</h1>
<div class="job-results">
"#,
            total_count, total_count
        );

        for job in jobs {
            let title = job.get("title").and_then(|t| t.as_str()).unwrap_or("Untitled");
            let req_id = job.get("reqId").and_then(|r| r.as_str()).unwrap_or("");
            let city = job.get("city").and_then(|c| c.as_str()).unwrap_or("");
            let state = job.get("state").and_then(|s| s.as_str()).unwrap_or("");
            let job_type = job.get("type").and_then(|t| t.as_str()).unwrap_or("");
            let teaser = job.get("descriptionTeaser").and_then(|d| d.as_str()).unwrap_or("");
            let apply_url = job.get("applyUrl").and_then(|u| u.as_str()).unwrap_or("#");
            let skills = job.get("ml_skills")
                .and_then(|s| s.as_array())
                .map(|arr| arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<&str>>()
                    .join(", "))
                .unwrap_or_default();

            synthetic_html.push_str(&format!(
                r#"<div class="job-card" data-req-id="{}">
<h2><a href="{}">{}</a></h2>
<p class="location">{}, {}</p>
<p class="type">{}</p>
<p class="description">{}</p>
<p class="skills">{}</p>
</div>
"#,
                req_id, apply_url, title, city, state, job_type,
                if teaser.len() > 500 { &teaser[..500] } else { teaser },
                skills
            ));
        }

        synthetic_html.push_str("</div>\n</body></html>");

        info!(html_len = synthetic_html.len(), job_count = jobs.len(), total_count, "Phenom: built synthetic listing from /widgets API");
        self.load_page(&synthetic_html, page_url);
    }

    /// Workday hydration: fetch job listings via the Workday CXS API
    /// when the URL contains `myworkdayjobs.com` (used by Honeywell, Deloitte, etc.).
    /// Requires a PLAY_SESSION cookie that is set by the initial page load.
    #[cfg(feature = "std")]
    async fn try_workday_api_hydration(&mut self, page_url: &str, html: &str) {
        // Confirm this is a Workday page via URL or HTML content
        if !page_url.contains("myworkdayjobs.com") && !html.contains("myworkdayjobs.com") {
            return;
        }

        let parsed = match url::Url::parse(page_url) {
            Ok(u) => u,
            Err(_) => return,
        };

        let host = match parsed.host_str() {
            Some(h) => h.to_string(),
            None => return,
        };

        // Extract company from subdomain: {company}.wd{N}.myworkdayjobs.com
        let subdomain_parts: Vec<&str> = host.split('.').collect();
        if subdomain_parts.len() < 3 {
            debug!("Workday: unexpected host format — {}", host);
            return;
        }
        let company = subdomain_parts[0];

        // Extract site name from path: /en-US/{SiteName}/...
        let segments: Vec<&str> = parsed.path().trim_matches('/').split('/').collect();
        let site_name = if segments.len() >= 2 {
            segments[1]
        } else if segments.len() == 1 && !segments[0].is_empty() {
            segments[0]
        } else {
            debug!("Workday: could not extract site name from path");
            return;
        };

        // Extract search query from URL query parameters
        let search_text = parsed
            .query_pairs()
            .find(|(k, _)| k == "q" || k == "searchText")
            .map(|(_, v)| v.to_string())
            .unwrap_or_default();

        info!(company = %company, site_name = %site_name, search_text = %search_text, host = %host, "Workday: fetching jobs via CXS API");

        // Check for PLAY_SESSION cookie from the initial page fetch
        let cookie_header = match self.cookie_header_for_url(page_url) {
            Some(c) => c,
            None => {
                warn!("Workday: no cookies available for {} — PLAY_SESSION required; use CDP fallback", host);
                return;
            }
        };

        if !cookie_header.contains("PLAY_SESSION") {
            warn!("Workday: PLAY_SESSION cookie not found in stored cookies — API will likely fail; use CDP fallback");
            // Still try — the cookie jar from reqwest may have it even if our manual store doesn't
        }

        let api_url = format!("https://{}/wday/cxs/{}/{}/jobs", host, company, site_name);

        let payload = serde_json::json!({
            "appliedFacets": {},
            "limit": 20,
            "offset": 0,
            "searchText": search_text
        });

        let resp = match self.client
            .post(&api_url)
            .header("Content-Type", "application/json")
            .header("Cookie", &cookie_header)
            .json(&payload)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                debug!(error = %e, "Workday CXS API request failed");
                return;
            }
        };

        let status = resp.status().as_u16();
        if status == 422 || status == 403 || (300..400).contains(&status) {
            warn!(status, "Workday: CXS API returned error status — PLAY_SESSION may be invalid");
            return;
        }

        let body = match resp.text().await {
            Ok(t) => t,
            Err(_) => return,
        };

        let data: serde_json::Value = match serde_json::from_str(&body) {
            Ok(d) => d,
            Err(_) => {
                debug!("Workday: failed to parse CXS API response as JSON");
                return;
            }
        };

        let jobs = match data.get("jobPostings").and_then(|j| j.as_array()) {
            Some(j) => j,
            None => {
                debug!("Workday: no jobPostings array in CXS API response");
                return;
            }
        };

        let total = data.get("total").and_then(|t| t.as_u64()).unwrap_or(jobs.len() as u64);

        // Build synthetic HTML from the job listings
        let mut synthetic_html = format!(
            r#"<html><head><title>{} — Jobs ({} results)</title></head>
<body>
<h1>{} — Job Search Results ({} total)</h1>
<div id="workday-job-list">
"#,
            company, total, company, total
        );

        for job in jobs {
            let title = job.get("title").and_then(|t| t.as_str()).unwrap_or("Untitled");
            let location = job.get("locationsText").and_then(|l| l.as_str()).unwrap_or("");
            let posted_on = job.get("postedOn").and_then(|p| p.as_str()).unwrap_or("");
            let external_path = job.get("externalPath").and_then(|u| u.as_str()).unwrap_or("#");
            let job_url = format!("https://{}{}", host, external_path);

            let bullet_fields = job.get("bulletFields")
                .and_then(|b| b.as_array())
                .map(|arr| arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<&str>>()
                    .join(" · "))
                .unwrap_or_default();

            synthetic_html.push_str(&format!(
                r#"<div class="workday-job" data-external-path="{}">
<h2><a href="{}">{}</a></h2>
<p class="location">{}</p>
<p class="details">{}</p>
<p class="posted">{}</p>
</div>
"#,
                external_path, job_url, title, location, bullet_fields, posted_on
            ));
        }

        synthetic_html.push_str("</div>\n</body></html>");

        info!(html_len = synthetic_html.len(), job_count = jobs.len(), total, "Workday: built synthetic listing from CXS API");
        self.load_page(&synthetic_html, page_url);
    }

    /// SPA hydration: after initial page load, check if inline scripts created dynamic
    /// script elements (SPA pattern) and fetch+execute them.
    #[cfg(feature = "std")]
    async fn try_spa_hydration(&mut self, base_url: &str) {
        let js = match self.js.as_ref() {
            Some(js) => js,
            None => return,
        };

        // Step 0: Fulfill pending fetch() requests from inline scripts.
        // Inline scripts call fetch() which is a stub — the requests are logged.
        // We replay them via Rust HTTP, inject the responses, and let callbacks run.
        let xhr_log = js.run_script("__wraith_get_xhr_log()").ok().unwrap_or_default();
        if let Ok(requests) = serde_json::from_str::<Vec<serde_json::Value>>(&xhr_log) {
            for req in &requests {
                let req_url = req.get("url").and_then(|u| u.as_str()).unwrap_or("");
                if req_url.is_empty() { continue; }

                let full_url = if req_url.starts_with("http") {
                    req_url.to_string()
                } else if req_url.starts_with('/') {
                    if let Ok(base) = url::Url::parse(base_url) {
                        format!("{}://{}{}", base.scheme(), base.host_str().unwrap_or(""), req_url)
                    } else { continue }
                } else { continue };

                debug!(url = %full_url, "SPA hydration: fulfilling pending fetch/XHR");
                if let Ok(resp) = self.client.get(&full_url).send().await {
                    if resp.status().is_success() {
                        if let Ok(text) = resp.text().await {
                            // Inject the response as a JS variable and try to process it
                            let escaped = text.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n").replace('\r', "");
                            let inject = format!(
                                "try {{ var __wraith_fetch_response = JSON.parse('{}'); }} catch(e) {{ var __wraith_fetch_response = '{}'; }}",
                                escaped, escaped
                            );
                            let _ = js.run_script(&inject);
                            debug!(url = %full_url, len = text.len(), "SPA: injected fetch response");
                        }
                    }
                }
            }
            // Clear the XHR log
            let _ = js.run_script("__wraith_xhr_log = []");
        }

        // Step 1: Check for dynamically created script elements
        let dynamic_urls = match js.run_script(
            r#"(() => {
                try {
                    var urls = [];
                    if (typeof __wraith_dynamic_scripts !== 'undefined') {
                        for (var i = 0; i < __wraith_dynamic_scripts.length; i++) {
                            urls.push(__wraith_dynamic_scripts[i]);
                        }
                    }
                    return JSON.stringify(urls);
                } catch(e) { return '[]'; }
            })()"#
        ) {
            Ok(json) => {
                serde_json::from_str::<Vec<String>>(&json).unwrap_or_default()
            }
            Err(_) => Vec::new(),
        };

        if dynamic_urls.is_empty() {
            debug!("SPA hydration: no dynamic scripts found");
            return;
        }

        info!(count = dynamic_urls.len(), "SPA hydration: fetching dynamic scripts");

        // Step 2: Fetch each dynamic script
        let mut fetched = 0;
        let max_size: usize = 5 * 1024 * 1024; // 5MB total budget
        let mut total_size: usize = 0;

        for script_url in &dynamic_urls {
            if total_size >= max_size { break; }

            let full_url = if script_url.starts_with("http") {
                script_url.clone()
            } else if script_url.starts_with("//") {
                format!("https:{}", script_url)
            } else if script_url.starts_with('/') {
                if let Ok(base) = url::Url::parse(base_url) {
                    format!("{}://{}{}", base.scheme(), base.host_str().unwrap_or(""), script_url)
                } else {
                    continue;
                }
            } else {
                continue;
            };

            match self.client.get(&full_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(text) = resp.text().await {
                        if text.len() + total_size <= max_size {
                            total_size += text.len();
                            // Execute the script in QuickJS
                            match js.run_script(&text) {
                                Ok(_) => {
                                    fetched += 1;
                                    debug!(url = %full_url, len = text.len(), "SPA: executed dynamic script");
                                }
                                Err(e) => {
                                    debug!(url = %full_url, error = %e, "SPA: dynamic script failed");
                                }
                            }
                        }
                    }
                }
                _ => {
                    debug!(url = %full_url, "SPA: failed to fetch dynamic script");
                }
            }
        }

        if fetched > 0 {
            // Flush timers after executing dynamic scripts
            let _ = js.run_script("if(typeof __wraith_flush_timers==='function')__wraith_flush_timers()");
            info!(fetched, total_size, "SPA hydration complete — dynamic scripts executed");
        }
    }

    /// Detect hard access restrictions (different from solvable challenges).
    fn is_ip_blocked(html: &str) -> bool {
        html.contains("Sorry, you have been blocked")
            || html.contains("Access to this page has been denied")
            || html.contains("Your IP address has been blocked")
            || html.contains("This request was blocked by the security rules")
    }

    /// Tier 2: Try solving the CF challenge with QuickJS. Returns cookie string if successful.
    #[cfg(feature = "std")]
    async fn try_quickjs_solve(&self, url: &str) -> Option<String> {
        let js = self.js.as_ref()?;

        // Set location for the challenge scripts
        let _ = js.run_script(&format!(
            "__wraith_set_location({})",
            serde_json::to_string(url).unwrap_or_default()
        ));

        // Run the challenge scripts
        if let Some(ref page_html) = self.current_html {
            let _ = js.execute_page_scripts(page_html);
            let _ = js.run_script("__wraith_flush_timers()");
        }

        // Check for CF cookies
        let cookie_json = js.run_script("__wraith_get_cookies()").ok()?;
        if cookie_json.contains("cf_clearance") || cookie_json.contains("__cf_bm") {
            let cookies: std::collections::HashMap<String, String> =
                serde_json::from_str(&cookie_json).ok()?;
            let header = cookies.iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join("; ");
            info!(cookies = cookies.len(), "QuickJS captured CF cookies");
            return Some(header);
        }

        debug!("QuickJS solver did not produce CF cookies");
        None
    }

    /// Tier 3: Call external challenge solver to resolve protected page via real browser.
    /// The solver must be running (e.g., `docker run -p 8191:8191 flaresolverr/flaresolverr`).
    #[cfg(feature = "std")]
    async fn try_flaresolverr(&self, url: &str) -> Option<String> {
        let solver_url = self.config.flaresolverr_url.as_ref()?;
        let endpoint = format!("{}/v1", solver_url);

        info!(url = %url, solver = %solver_url, "Calling external challenge solver");

        let mut payload = serde_json::json!({
            "cmd": "request.get",
            "url": url,
            "maxTimeout": 120000
        });

        // If we have a fallback proxy, tell the solver to use it too
        if let Some(ref proxy) = self.config.fallback_proxy_url {
            payload["proxy"] = serde_json::json!({"url": proxy});
            info!(proxy = %proxy, "External solver using proxy");
        }

        let response = self.client.post(&endpoint)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "External solver request failed");
                e
            })
            .ok()?;

        let body: serde_json::Value = response.json().await.ok()?;

        // Extract cookies from solver response
        let cookies = body["solution"]["cookies"].as_array()?;
        let cookie_header: String = cookies.iter()
            .filter_map(|c| {
                let name = c["name"].as_str()?;
                let value = c["value"].as_str()?;
                Some(format!("{}={}", name, value))
            })
            .collect::<Vec<_>>()
            .join("; ");

        if cookie_header.is_empty() {
            warn!("External solver returned no cookies");
            return None;
        }

        // Also check if the solver returned the actual page content
        if let Some(solution_html) = body["solution"]["response"].as_str() {
            if !solution_html.is_empty() && !Self::is_cloudflare_challenge(solution_html, 200) {
                info!(cookie_count = cookies.len(), "External solver resolved challenge — cookies captured");
            }
        }

        Some(cookie_header)
    }

    /// Tier 3 variant: get the full page HTML from the external solver's response.
    /// The solver returns the rendered page content — we can use it directly
    /// without needing to replay cookies (which may be restricted anyway).
    #[cfg(feature = "std")]
    async fn try_flaresolverr_full_page(&self, url: &str) -> Option<String> {
        let solver_url = self.config.flaresolverr_url.as_ref()?;
        let endpoint = format!("{}/v1", solver_url);

        info!(url = %url, "External solver: requesting full page content");

        let mut payload = serde_json::json!({
            "cmd": "request.get",
            "url": url,
            "maxTimeout": 120000
        });

        if let Some(ref proxy) = self.config.fallback_proxy_url {
            payload["proxy"] = serde_json::json!({"url": proxy});
        }

        let response = self.client.post(&endpoint)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "External solver request failed");
                e
            })
            .ok()?;

        let body: serde_json::Value = response.json().await.ok()?;

        // Check status
        let status = body["solution"]["status"].as_i64().unwrap_or(0);
        if status != 200 {
            warn!(status, "External solver returned non-200 status");
        }

        // Extract the full rendered HTML
        let html = body["solution"]["response"].as_str()?;
        if html.is_empty() {
            return None;
        }

        info!(
            html_len = html.len(),
            status,
            "External solver returned page content"
        );

        Some(html.to_string())
    }

    /// Fetch via a specific proxy (for Tier 4 access restriction fallback).
    #[cfg(feature = "std")]
    async fn http_fetch_via_proxy(&self, url: &str, proxy_url: &str) -> Result<(u16, String, String), String> {
        debug!(url = %url, proxy = %proxy_url, "Fetching via fallback proxy");

        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|e| format!("Invalid proxy URL: {e}"))?;

        let client = reqwest::Client::builder()
            .user_agent(&self.config.user_agent)
            .proxy(proxy)
            .cookie_store(true)
            .gzip(true)
            .brotli(true)
            .build()
            .map_err(|e| format!("Proxy client build failed: {e}"))?;

        let response = client.get(url)
            .header("Upgrade-Insecure-Requests", "1")
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8")
            .header("Priority", "u=0, i")
            .header("Sec-Fetch-Site", "none")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-User", "?1")
            .header("Sec-Fetch-Dest", "document")
            .header("Accept-Encoding", "gzip, deflate, br, zstd")
            .header("Accept-Language", &self.config.accept_language)
            .send()
            .await
            .map_err(|e| format!("Proxy fetch failed: {e}"))?;

        let status = response.status().as_u16();
        let final_url = response.url().to_string();
        let body = response.text().await.map_err(|e| format!("body failed: {e}"))?;
        Ok((status, body, final_url))
    }

    /// Wait for page stability by checking if the visible element count has settled.
    /// This catches pages that render asynchronously (setTimeout callbacks, fetch-then-render).
    #[cfg(feature = "std")]
    async fn wait_for_stability(&mut self, max_wait_ms: u64) {
        let mut last_count = self.dom_nodes.iter().filter(|n| n.is_visible).count();
        let start = Instant::now();
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            // Flush JS timers
            if let Some(ref js) = self.js {
                let _ = js.run_script("__wraith_flush_timers()");
            }
            // Re-extract DOM
            if let Some(ref html) = self.current_html.clone() {
                let parsed = scraper::Html::parse_document(html);
                self.dom_nodes = extract_dom_nodes(&parsed);
            }
            let new_count = self.dom_nodes.iter().filter(|n| n.is_visible).count();
            if new_count == last_count || start.elapsed().as_millis() > max_wait_ms as u128 {
                break;
            }
            last_count = new_count;
        }
    }

    /// Load HTML into the DOM engine and execute scripts.
    fn load_page(&mut self, html: &str, url: &str) {
        self.load_page_with_scripts(html, url, None);
    }

    fn load_page_with_scripts(
        &mut self,
        html: &str,
        url: &str,
        fetched_scripts: Option<&std::collections::HashMap<String, String>>,
    ) {
        let parsed = Html::parse_document(html);
        self.dom_nodes = extract_dom_nodes(&parsed);
        self.parsed_dom = Some(parsed);
        self.current_html = Some(html.to_string());
        self.current_url = Some(url.to_string());

        debug!(nodes = self.dom_nodes.len(), url = %url, "DOM parsed");

        // Set up JS environment and run scripts (with fingerprint spoofing if configured)
        #[cfg(feature = "std")]
        if let Some(ref js) = self.js {
            if let Err(e) = js.setup_dom_bridge_with_fingerprint(&self.dom_nodes, self.fingerprint.as_ref()) {
                warn!(error = %e, "DOM bridge setup failed");
            } else {
                // Set actual page location
                let _ = js.run_script(&format!(
                    "__wraith_set_location({})",
                    serde_json::to_string(url).unwrap_or_default()
                ));

                match js.execute_page_scripts_with_fetcher(html, fetched_scripts) {
                    Ok(n) => debug!(scripts = n, "Page scripts executed"),
                    Err(e) => debug!(error = %e, "Script execution failed (non-fatal)"),
                }
            }
        }
    }

    /// Detect if an HTTP response is a challenge page (not a solved page).
    /// A page with real content that also has challenge remnants is NOT a challenge.
    fn is_cloudflare_challenge(html: &str, _status: u16) -> bool {
        // Definitive challenge signatures — check these regardless of page size.
        // Some sites (e.g. Indeed) serve bloated CAPTCHA pages (>50KB of inline CSS).
        if html.contains("CLOUDFLARE_STATIC_PAGE")
            || html.contains("cf-browser-verification")
            || html.contains("cf_chl_opt")
            || html.contains("Invalid CORS request")
        {
            return true;
        }

        // If the page has substantial content (>50KB), it's probably real content
        // with leftover CF scripts/tags — not an unsolved challenge.
        // Challenge pages are typically small (<20KB).
        if html.len() > 50_000 {
            return false;
        }

        // Weaker challenge page signatures (only trust on small pages)
        html.contains("Checking if the site connection is secure")
            || html.contains("Attention Required! | Cloudflare")
            || html.contains("Just a moment...")
            || html.contains("Authenticating...")
            || html.contains("challenge-platform")
            || html.contains("Security Check")
            || (html.contains("cloudflare") && html.contains("challenge"))
    }

    /// Fetch with explicit cookie header (for challenge resolution retry).
    #[cfg(feature = "std")]
    async fn http_fetch_with_cookies(&self, url: &str, cookies: &str) -> Result<(u16, String, String), String> {
        debug!(url = %url, "Retrying with challenge cookies");

        #[cfg(feature = "stealth-tls")]
        {
            let client = rquest::Client::builder()
                .emulation(Emulation::Firefox136)
                .cookie_store(true)
                .build()
                .map_err(|e| format!("rquest build failed: {e}"))?;

            let response = client.get(url)
                .header("Cookie", cookies)
                .header("Accept-Language", &self.config.accept_language)
                .send()
                .await
                .map_err(|e| format!("retry request failed: {e}"))?;

            let status = response.status().as_u16();
            let final_url = response.url().to_string();
            let body = response.text().await.map_err(|e| format!("body failed: {e}"))?;
            return Ok((status, body, final_url));
        }

        #[cfg(not(feature = "stealth-tls"))]
        {
            let mut headers = std::collections::BTreeMap::new();
            headers.insert("Cookie".to_string(), cookies.to_string());
            headers.insert("sec-ch-ua".to_string(), "\"Google Chrome\";v=\"136\", \"Chromium\";v=\"136\", \"Not_A Brand\";v=\"24\"".to_string());
            headers.insert("sec-ch-ua-mobile".to_string(), "?0".to_string());
            headers.insert("sec-ch-ua-platform".to_string(), "\"Windows\"".to_string());
            headers.insert("Upgrade-Insecure-Requests".to_string(), "1".to_string());
            headers.insert("Accept".to_string(), "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8".to_string());
            headers.insert("Sec-Fetch-Site".to_string(), "none".to_string());
            headers.insert("Sec-Fetch-Mode".to_string(), "navigate".to_string());
            headers.insert("Sec-Fetch-User".to_string(), "?1".to_string());
            headers.insert("Sec-Fetch-Dest".to_string(), "document".to_string());
            headers.insert("Accept-Encoding".to_string(), "gzip, deflate, br, zstd".to_string());
            headers.insert("Accept-Language".to_string(), self.config.accept_language.clone());

            let request = TransportRequest {
                method: TransportMethod::Get,
                url: url.to_string(),
                headers,
                body: None,
            };

            let response = self.transport.execute(request).await
                .map_err(|e| format!("retry request failed: {e}"))?;

            let status = response.status;
            let final_url = response.url;
            let body = String::from_utf8_lossy(&response.body).to_string();
            Ok((status, body, final_url))
        }
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

    /// Get the resolved iframe contents map.
    pub fn iframe_contents(&self) -> &HashMap<u64, (String, Vec<DomNode>)> {
        &self.iframe_contents
    }

    /// Resolve iframes: fetch content for each `<iframe src="...">` in the DOM.
    /// Parses the iframe HTML into DomNodes with prefixed node_ids to avoid collisions.
    /// Called automatically after navigate().
    #[instrument(skip(self))]
    #[cfg(feature = "std")]
    pub async fn resolve_iframes(&mut self) {
        self.iframe_contents.clear();

        let base_url = match &self.current_url {
            Some(u) => u.clone(),
            None => return,
        };

        // Collect iframe src URLs and their node_ids
        let iframes: Vec<(u64, String)> = self.dom_nodes.iter()
            .filter(|n| n.tag_name == "iframe" && n.node_type == DomNodeType::Element)
            .filter_map(|n| {
                n.attributes.get("src").and_then(|src| {
                    if src.is_empty() || src.starts_with("about:") || src.starts_with("javascript:") {
                        return None;
                    }
                    // Resolve relative URLs
                    let absolute = if src.starts_with("http://") || src.starts_with("https://") {
                        src.clone()
                    } else if let Ok(base) = url::Url::parse(&base_url) {
                        base.join(src).map(|u| u.to_string()).unwrap_or_default()
                    } else {
                        return None;
                    };
                    if absolute.is_empty() { return None; }
                    Some((n.node_id, absolute))
                })
            })
            .collect();

        if iframes.is_empty() {
            return;
        }

        info!(count = iframes.len(), "Resolving iframe contents");

        for (iframe_node_id, src_url) in iframes {
            debug!(node_id = iframe_node_id, url = %src_url, "Fetching iframe content");

            // Fetch the iframe HTML
            let fetch_result = self.client.get(&src_url)
                .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
                .header("Accept-Language", &self.config.accept_language)
                .header("Sec-Fetch-Dest", "iframe")
                .header("Sec-Fetch-Mode", "navigate")
                .header("Sec-Fetch-Site", "cross-site")
                .send()
                .await;

            match fetch_result {
                Ok(response) => {
                    if !response.status().is_success() {
                        debug!(status = response.status().as_u16(), url = %src_url, "Iframe fetch failed with HTTP error");
                        continue;
                    }
                    match response.text().await {
                        Ok(iframe_html) => {
                            if iframe_html.is_empty() {
                                continue;
                            }
                            // Parse iframe HTML and extract nodes
                            let iframe_dom = Html::parse_document(&iframe_html);
                            let mut iframe_nodes = extract_dom_nodes(&iframe_dom);

                            // Prefix node_ids to avoid collision with parent DOM
                            // Use iframe_node_id * 10000 + child_id scheme
                            let id_base = iframe_node_id * 10000;
                            for node in &mut iframe_nodes {
                                node.node_id = id_base + node.node_id;
                                node.parent = node.parent.map(|p| id_base + p);
                                node.children = node.children.iter().map(|c| id_base + c).collect();
                            }

                            info!(
                                iframe_node_id = iframe_node_id,
                                url = %src_url,
                                child_nodes = iframe_nodes.len(),
                                "Iframe content resolved"
                            );
                            self.iframe_contents.insert(iframe_node_id, (src_url, iframe_nodes));
                        }
                        Err(e) => {
                            debug!(error = %e, url = %src_url, "Failed to read iframe body");
                        }
                    }
                }
                Err(e) => {
                    debug!(error = %e, url = %src_url, "Failed to fetch iframe");
                }
            }
        }
    }

    /// Enter an iframe context: switches current page state to the iframe's content.
    /// Returns the iframe's DomNodes, or an error if the iframe was not resolved.
    pub fn enter_iframe(&mut self, iframe_node_id: u64) -> Result<Vec<DomNode>, String> {
        let (src_url, iframe_nodes) = self.iframe_contents.get(&iframe_node_id)
            .ok_or_else(|| format!("No resolved iframe content for node_id {}", iframe_node_id))?;

        let src_url = src_url.clone();
        let iframe_nodes = iframe_nodes.clone();

        // Save current URL to history
        if let Some(ref current) = self.current_url {
            self.history.push(current.clone());
        }

        // Replace current page state with iframe content
        self.current_url = Some(src_url);
        self.dom_nodes = iframe_nodes.clone();
        // Clear parsed_dom since it no longer matches dom_nodes
        self.parsed_dom = None;

        info!(iframe_node_id = iframe_node_id, nodes = iframe_nodes.len(), "Entered iframe context");
        Ok(iframe_nodes)
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
    #[cfg(feature = "std")]
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

    /// Build a Cookie header string from stored cookies for a given URL.
    pub fn cookie_header_for_url(&self, url: &str) -> Option<String> {
        let domain = url::Url::parse(url).ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()))?;
        let matching: Vec<String> = self.cookies.iter()
            .filter(|c| domain == c.domain || domain.ends_with(&format!(".{}", c.domain)))
            .map(|c| format!("{}={}", c.name, c.value))
            .collect();
        if matching.is_empty() { None } else { Some(matching.join("; ")) }
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

    /// Fetch all external `<script src="...">` URLs from HTML.
    /// Returns a map of URL -> script content for scripts that were successfully fetched.
    /// Skips analytics, tracking, and non-JS scripts. Limits to 2MB total.
    #[cfg(feature = "std")]
    pub async fn fetch_external_scripts(client: &reqwest::Client, html: &str, base_url: &str) -> std::collections::HashMap<String, String> {
        let mut scripts = std::collections::HashMap::new();
        let mut total_bytes: usize = 0;
        let max_total: usize = 2 * 1024 * 1024; // 2MB limit

        let doc = Html::parse_document(html);
        let sel = match scraper::Selector::parse("script[src]") {
            Ok(s) => s,
            Err(_) => return scripts,
        };

        for el in doc.select(&sel) {
            if total_bytes >= max_total {
                debug!("Script fetch budget exhausted ({}B)", total_bytes);
                break;
            }

            let src = match el.value().attr("src") {
                Some(s) => s,
                None => continue,
            };

            // Skip known analytics/tracking scripts
            if src.contains("google-analytics") || src.contains("gtag")
                || src.contains("facebook") || src.contains("hotjar")
                || src.contains("segment") || src.contains("sentry")
                || src.contains("clarity") || src.contains("intercom")
            {
                debug!(src = %src, "Skipping analytics script");
                continue;
            }

            // Skip non-JS types
            if let Some(t) = el.value().attr("type") {
                if t.contains("json") || t.contains("template") {
                    continue;
                }
            }

            // Resolve relative URLs
            let full_url = if src.starts_with("http") {
                src.to_string()
            } else if src.starts_with("//") {
                format!("https:{}", src)
            } else if src.starts_with('/') {
                if let Ok(base) = url::Url::parse(base_url) {
                    format!("{}://{}{}", base.scheme(), base.host_str().unwrap_or(""), src)
                } else {
                    continue;
                }
            } else {
                continue; // Skip relative paths without base
            };

            // Fetch the script
            match client.get(&full_url)
                .header("Accept", "application/javascript, text/javascript, */*")
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    match resp.text().await {
                        Ok(text) if !text.is_empty() => {
                            let len = text.len();
                            if total_bytes + len <= max_total {
                                debug!(src = %src, len, "Fetched external script");
                                scripts.insert(src.to_string(), text);
                                total_bytes += len;
                            }
                        }
                        _ => {}
                    }
                }
                Ok(resp) => {
                    debug!(src = %src, status = %resp.status(), "Script fetch failed");
                }
                Err(e) => {
                    debug!(src = %src, error = %e, "Script fetch error");
                }
            }
        }

        info!(fetched = scripts.len(), total_bytes, "External scripts fetched");
        scripts
    }

    /// Parse Set-Cookie header strings and store them in self.cookies.
    fn store_set_cookies(&mut self, set_cookie_headers: &[String], response_url: &str) {
        let domain = url::Url::parse(response_url).ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()))
            .unwrap_or_default();

        for header in set_cookie_headers {
            // Parse "name=value; Path=/; Domain=.example.com; ..."
            let parts: Vec<&str> = header.splitn(2, ';').collect();
            let name_value = parts[0].trim();
            let (name, value) = match name_value.split_once('=') {
                Some((n, v)) => (n.trim().to_string(), v.trim().to_string()),
                None => continue,
            };

            let attrs = if parts.len() > 1 { parts[1] } else { "" };
            let attrs_lower = attrs.to_lowercase();

            let cookie_domain = attrs_lower
                .split(';')
                .find_map(|a| {
                    let a = a.trim();
                    if a.starts_with("domain=") {
                        Some(a.trim_start_matches("domain=").trim_start_matches('.').to_string())
                    } else { None }
                })
                .unwrap_or_else(|| domain.clone());

            let path = attrs_lower
                .split(';')
                .find_map(|a| {
                    let a = a.trim();
                    if a.starts_with("path=") {
                        Some(a.trim_start_matches("path=").to_string())
                    } else { None }
                })
                .unwrap_or_else(|| "/".to_string());

            let secure = attrs_lower.contains("secure");
            let http_only = attrs_lower.contains("httponly");

            debug!(name = %name, domain = %cookie_domain, "Storing cookie from Set-Cookie header");
            self.set_cookie(Cookie {
                name,
                value,
                domain: cookie_domain,
                path,
                secure,
                http_only,
                expires: None,
            });
        }
    }

    /// Navigate with full OAuth/auth redirect chain handling.
    ///
    /// Follows all redirects (302 -> 302 -> 302 -> 200), captures Set-Cookie
    /// headers at each hop, detects OAuth callback URLs (`?code=` or `?token=`),
    /// and returns the final page event.
    #[cfg(feature = "std")]
    #[instrument(skip(self), fields(url = %url))]
    pub async fn navigate_with_auth(&mut self, url: &str) -> Result<PageEvent, String> {
        info!(url = %url, "Navigating with auth redirect tracking");

        // Use a no-redirect client so we can manually follow each hop and capture cookies
        let mut client_builder = reqwest::Client::builder()
            .user_agent(&self.config.user_agent)
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::none())
            .gzip(true)
            .brotli(true);

        if let Some(ref proxy_url) = self.config.proxy_url {
            if let Ok(proxy) = reqwest::Proxy::all(proxy_url) {
                client_builder = client_builder.proxy(proxy);
            }
        }

        let redirect_client = client_builder.build()
            .map_err(|e| format!("Failed to build redirect client: {e}"))?;

        let mut current_url = url.to_string();
        let mut hop_count = 0u32;
        let max_hops = 10u32;

        loop {
            if hop_count >= max_hops {
                warn!(hops = hop_count, "OAuth redirect chain exceeded max hops");
                break;
            }

            debug!(url = %current_url, hop = hop_count, "Auth redirect hop");

            let mut req = redirect_client.get(&current_url)
                .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
                .header("Accept-Language", &self.config.accept_language);

            // Attach stored cookies
            if let Some(cookie_header) = self.cookie_header_for_url(&current_url) {
                req = req.header("Cookie", cookie_header);
            }

            let response = req.send().await
                .map_err(|e| format!("Auth redirect fetch failed: {e}"))?;

            let status = response.status().as_u16();

            // Capture Set-Cookie headers from this hop
            let hop_cookies: Vec<String> = response.headers().get_all("set-cookie")
                .iter()
                .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
                .collect();
            self.store_set_cookies(&hop_cookies, &current_url);

            if !hop_cookies.is_empty() {
                info!(hop = hop_count, cookies = hop_cookies.len(), url = %current_url,
                    "Captured cookies at redirect hop");
            }

            // Check for redirect
            if (300..400).contains(&status) {
                if let Some(location) = response.headers().get("location") {
                    let location_str = location.to_str()
                        .map_err(|e| format!("Invalid Location header: {e}"))?;

                    // Resolve relative redirect URLs
                    let next_url = if location_str.starts_with("http") {
                        location_str.to_string()
                    } else {
                        let base = url::Url::parse(&current_url)
                            .map_err(|e| format!("Invalid base URL: {e}"))?;
                        base.join(location_str)
                            .map_err(|e| format!("Failed to resolve redirect: {e}"))?
                            .to_string()
                    };

                    // Detect OAuth callback URLs
                    if next_url.contains("?code=") || next_url.contains("&code=")
                        || next_url.contains("?token=") || next_url.contains("&token=")
                        || next_url.contains("?access_token=") || next_url.contains("&access_token=")
                    {
                        info!(url = %next_url, "OAuth callback URL detected in redirect chain");
                    }

                    current_url = next_url;
                    hop_count += 1;
                    continue;
                }
            }

            // Final response (non-redirect) -- read the body and load the page
            let final_url = current_url.clone();
            let body = response.text().await
                .map_err(|e| format!("Body read failed: {e}"))?;

            info!(
                hops = hop_count,
                status,
                final_url = %final_url,
                cookies_total = self.cookies.len(),
                "Auth redirect chain complete"
            );

            // Push to history and load the page
            if let Some(ref current) = self.current_url {
                self.history.push(current.clone());
            }
            self.load_page(&body, &final_url);
            return Ok(PageEvent::Load);
        }

        // Fallback: if we exhausted hops, do a normal navigate on the last URL
        self.navigate(&current_url).await
    }

    /// Go back in history.
    #[cfg(feature = "std")]
    pub async fn go_back(&mut self) -> Result<PageEvent, String> {
        if let Some(url) = self.history.pop() {
            let url_clone = url.clone();
            self.navigate(&url_clone).await
        } else {
            Ok(PageEvent::Error("No history to go back to".to_string()))
        }
    }

    /// Fetch a URL using compatible TLS (rquest/BoringSSL) if available,
    /// falling back to reqwest (rustls) otherwise.
    /// Returns (status, body, final_url, set_cookie_headers).
    /// Redirect policy follows up to 10 hops; cookie_store preserves cookies
    /// across redirects. set_cookie_headers captures Set-Cookie from the final response.
    #[cfg(feature = "std")]
    async fn http_fetch(&self, url: &str) -> Result<(u16, String, String, Vec<String>), String> {
        #[cfg(feature = "stealth-tls")]
        {
            debug!(url = %url, "Fetching with compatible TLS (rquest + BoringSSL)");

            let client = rquest::Client::builder()
                .emulation(Emulation::Firefox136)
                .cookie_store(true)
                .build()
                .map_err(|e| format!("rquest build failed: {e}"))?;

            let mut req = client.get(url)
                .header("Accept-Language", &self.config.accept_language);

            // Attach stored cookies (including any injected via cookie_set)
            if let Some(cookie_header) = self.cookie_header_for_url(url) {
                debug!(cookies = %cookie_header, "Attaching stored cookies to enhanced request");
                req = req.header("Cookie", cookie_header);
            }

            let response = req.send()
                .await
                .map_err(|e| format!("rquest request failed: {e}"))?;

            let status = response.status().as_u16();
            let final_url = response.url().to_string();
            let set_cookies: Vec<String> = response.headers().get_all("set-cookie")
                .iter()
                .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
                .collect();
            if !set_cookies.is_empty() {
                debug!(count = set_cookies.len(), "Captured Set-Cookie headers from enhanced response");
            }
            let body = response.text().await
                .map_err(|e| format!("rquest body failed: {e}"))?;

            Ok((status, body, final_url, set_cookies))
        }

        #[cfg(not(feature = "stealth-tls"))]
        {
            debug!(url = %url, "Fetching via HttpTransport (reqwest backend)");

            let mut headers = std::collections::BTreeMap::new();
            headers.insert("sec-ch-ua".to_string(), "\"Google Chrome\";v=\"136\", \"Chromium\";v=\"136\", \"Not_A Brand\";v=\"24\"".to_string());
            headers.insert("sec-ch-ua-mobile".to_string(), "?0".to_string());
            headers.insert("sec-ch-ua-platform".to_string(), "\"Windows\"".to_string());
            headers.insert("Upgrade-Insecure-Requests".to_string(), "1".to_string());
            headers.insert("Accept".to_string(), "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8".to_string());
            headers.insert("Sec-Fetch-Site".to_string(), "none".to_string());
            headers.insert("Sec-Fetch-Mode".to_string(), "navigate".to_string());
            headers.insert("Sec-Fetch-User".to_string(), "?1".to_string());
            headers.insert("Sec-Fetch-Dest".to_string(), "document".to_string());
            headers.insert("Accept-Encoding".to_string(), "gzip, deflate, br, zstd".to_string());
            headers.insert("Accept-Language".to_string(), self.config.accept_language.clone());
            headers.insert("Priority".to_string(), "u=0, i".to_string());

            // Attach stored cookies (including any injected via cookie_set)
            if let Some(cookie_header) = self.cookie_header_for_url(url) {
                debug!(cookies = %cookie_header, "Attaching stored cookies to request");
                headers.insert("Cookie".to_string(), cookie_header);
            }

            let request = TransportRequest {
                method: TransportMethod::Get,
                url: url.to_string(),
                headers,
                body: None,
            };

            let response = self.transport.execute(request).await
                .map_err(|e| format!("HTTP request failed: {e}"))?;

            let status = response.status;
            let final_url = response.url;
            let set_cookies = response.set_cookie_headers;
            if !set_cookies.is_empty() {
                debug!(count = set_cookies.len(), "Captured Set-Cookie headers from response");
            }
            let body = String::from_utf8_lossy(&response.body).to_string();

            Ok((status, body, final_url, set_cookies))
        }
    }

    /// Check if compatible TLS is available.
    pub fn has_stealth_tls() -> bool {
        cfg!(feature = "stealth-tls")
    }

    /// Submit form data via HTTP POST. Used as fallback when React form submission
    /// doesn't work (because React scripts aren't loaded in QuickJS).
    #[cfg(feature = "std")]
    pub async fn submit_form_data(&self, url: &str, json_body: &str) -> Result<String, String> {
        self.submit_form_data_with_content_type(url, json_body, "application/json").await
    }

    /// Submit form data with a specific content type.
    /// Handles JSON, multipart/form-data (for Greenhouse), and URL-encoded (for Lever).
    #[cfg(feature = "std")]
    pub async fn submit_form_data_with_content_type(
        &self, url: &str, json_body: &str, content_type: &str
    ) -> Result<String, String> {
        info!(url = %url, content_type = %content_type, body_len = json_body.len(), "Submitting form via direct HTTP POST");

        let origin = self.current_url.as_deref().and_then(|u| {
            url::Url::parse(u).ok().map(|u| format!("{}://{}", u.scheme(), u.host_str().unwrap_or("")))
        }).unwrap_or_default();
        let referer = self.current_url.as_deref().unwrap_or("").to_string();

        // Send the request based on content type
        let send_result: Result<(u16, String), String> = if content_type.contains("multipart") {
            // Greenhouse API expects multipart/form-data
            let fields: std::collections::HashMap<String, String> =
                serde_json::from_str(json_body).unwrap_or_default();

            let mut form = reqwest::multipart::Form::new();
            for (key, value) in &fields {
                form = form.text(key.clone(), value.clone());
            }

            match self.client.post(url)
                .header("Accept", "application/json, text/html, */*")
                .header("Origin", &origin)
                .header("Referer", &referer)
                .multipart(form)
                .send().await
            {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let body = resp.text().await.unwrap_or_default();
                    Ok((status, body))
                }
                Err(e) => Err(format!("HTTP POST (multipart) failed: {}", e))
            }
        } else if content_type.contains("x-www-form-urlencoded") {
            // Lever expects URL-encoded form data
            let fields: std::collections::HashMap<String, String> =
                serde_json::from_str(json_body).unwrap_or_default();

            match self.client.post(url)
                .header("Accept", "application/json, text/html, */*")
                .header("Origin", &origin)
                .header("Referer", &referer)
                .form(&fields)
                .send().await
            {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let body = resp.text().await.unwrap_or_default();
                    Ok((status, body))
                }
                Err(e) => Err(format!("HTTP POST (form) failed: {}", e))
            }
        } else {
            // Default: JSON body
            match self.client.post(url)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json, text/html, */*")
                .header("Origin", &origin)
                .header("Referer", &referer)
                .body(json_body.to_string())
                .send().await
            {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let body = resp.text().await.unwrap_or_default();
                    Ok((status, body))
                }
                Err(e) => Err(format!("HTTP POST (json) failed: {}", e))
            }
        };

        match send_result {
            Ok((status, body)) => {
                let preview = if body.len() > 500 { &body[..500] } else { &body };
                if status < 400 {
                    Ok(format!("HTTP {} — {}", status, preview))
                } else {
                    Err(format!("HTTP {} — {}", status, preview))
                }
            }
            Err(e) => Err(e)
        }
    }

    /// Perform a simple HTTP GET and return (status_code, body).
    #[cfg(feature = "std")]
    pub async fn http_get(&self, url: &str) -> Result<(u16, String), String> {
        let mut headers = std::collections::BTreeMap::new();
        headers.insert(
            "User-Agent".to_string(),
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36".to_string(),
        );

        let request = TransportRequest {
            method: TransportMethod::Get,
            url: url.to_string(),
            headers,
            body: None,
        };

        let response = self.transport.execute(request).await
            .map_err(|e| format!("HTTP GET failed: {e}"))?;

        let status = response.status;
        let body = String::from_utf8_lossy(&response.body).to_string();
        Ok((status, body))
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
// CSS visibility detection
// ═══════════════════════════════════════════════════════════════

/// Parsed inline style properties relevant to visibility detection.
#[derive(Debug, Default)]
struct StyleProperties {
    display: Option<String>,
    visibility: Option<String>,
    opacity: Option<String>,
    width: Option<String>,
    height: Option<String>,
    position: Option<String>,
    left: Option<String>,
    top: Option<String>,
    pointer_events: Option<String>,
    overflow: Option<String>,
    clip: Option<String>,
    clip_path: Option<String>,
}

/// Parse an inline `style` attribute into structured properties.
fn parse_inline_style(style: &str) -> StyleProperties {
    let mut props = StyleProperties::default();
    for decl in style.split(';') {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        if let Some((prop, val)) = decl.split_once(':') {
            let prop = prop.trim().to_lowercase();
            let val = val.trim().to_string();
            match prop.as_str() {
                "display" => props.display = Some(val),
                "visibility" => props.visibility = Some(val),
                "opacity" => props.opacity = Some(val),
                "width" => props.width = Some(val),
                "height" => props.height = Some(val),
                "position" => props.position = Some(val),
                "left" => props.left = Some(val),
                "top" => props.top = Some(val),
                "pointer-events" => props.pointer_events = Some(val),
                "overflow" => props.overflow = Some(val),
                "clip" => props.clip = Some(val),
                "clip-path" => props.clip_path = Some(val),
                _ => {}
            }
        }
    }
    props
}

/// Check style properties for hidden/non-interactive indicators.
/// Returns (is_hidden, is_non_interactive).
fn check_style_visibility(style: &StyleProperties) -> (bool, bool) {
    let mut hidden = false;
    let mut non_interactive = false;

    // display: none
    if let Some(ref d) = style.display {
        if d.eq_ignore_ascii_case("none") {
            hidden = true;
        }
    }
    // visibility: hidden | collapse
    if let Some(ref v) = style.visibility {
        let vl = v.to_lowercase();
        if vl == "hidden" || vl == "collapse" {
            hidden = true;
        }
    }
    // opacity: 0
    if let Some(ref o) = style.opacity {
        if o.parse::<f64>().map(|v| v == 0.0).unwrap_or(false) {
            hidden = true;
        }
    }
    // width: 0
    if let Some(ref w) = style.width {
        let wt = w.trim_end_matches("px").trim_end_matches("em")
            .trim_end_matches("rem").trim_end_matches('%').trim();
        if wt.parse::<f64>().map(|v| v == 0.0).unwrap_or(false) {
            hidden = true;
        }
    }
    // height: 0
    if let Some(ref h) = style.height {
        let ht = h.trim_end_matches("px").trim_end_matches("em")
            .trim_end_matches("rem").trim_end_matches('%').trim();
        if ht.parse::<f64>().map(|v| v == 0.0).unwrap_or(false) {
            hidden = true;
        }
    }
    // overflow: hidden with zero dimensions
    if let Some(ref ov) = style.overflow {
        if ov.eq_ignore_ascii_case("hidden") {
            if let (Some(ref w), Some(ref h)) = (&style.width, &style.height) {
                let wv = w.trim_end_matches("px").trim().parse::<f64>().unwrap_or(1.0);
                let hv = h.trim_end_matches("px").trim().parse::<f64>().unwrap_or(1.0);
                if wv == 0.0 || hv == 0.0 {
                    hidden = true;
                }
            }
        }
    }
    // position: absolute/fixed with left/top <= -9999px (off-screen)
    if let Some(ref pos) = style.position {
        if pos.eq_ignore_ascii_case("absolute") || pos.eq_ignore_ascii_case("fixed") {
            if let Some(ref left) = style.left {
                if left.trim_end_matches("px").trim().parse::<f64>().unwrap_or(0.0) <= -9999.0 {
                    hidden = true;
                }
            }
            if let Some(ref top) = style.top {
                if top.trim_end_matches("px").trim().parse::<f64>().unwrap_or(0.0) <= -9999.0 {
                    hidden = true;
                }
            }
        }
    }
    // clip: rect(0,0,0,0)
    if let Some(ref clip) = style.clip {
        let cn = clip.to_lowercase().replace(' ', "");
        if cn.contains("rect(0,0,0,0)") || cn.contains("rect(0px,0px,0px,0px)") {
            hidden = true;
        }
    }
    // clip-path: inset(100%)
    if let Some(ref cp) = style.clip_path {
        if cp.to_lowercase().replace(' ', "").contains("inset(100%)") {
            hidden = true;
        }
    }
    // pointer-events: none -- visible but non-interactive
    if let Some(ref pe) = style.pointer_events {
        if pe.eq_ignore_ascii_case("none") {
            non_interactive = true;
        }
    }

    (hidden, non_interactive)
}

/// Class-name patterns that indicate hidden / screen-reader-only elements.
const HIDDEN_CLASS_PATTERNS: &[&str] = &[
    "hidden", "hide", "invisible", "sr-only", "visually-hidden",
    "screen-reader", "screenreader", "sr_only", "visually_hidden",
];

/// Returns true if any CSS class on the element matches a hidden pattern.
fn has_hidden_class(class_attr: &str) -> bool {
    let lower = class_attr.to_lowercase();
    for cls in lower.split_whitespace() {
        for pat in HIDDEN_CLASS_PATTERNS {
            if cls.contains(pat) {
                return true;
            }
        }
    }
    false
}

// ═══════════════════════════════════════════════════════════════
// DOM extraction from parsed HTML
// ═══════════════════════════════════════════════════════════════

/// Extract DomNode list from a parsed HTML document.
/// Walks the FULL document tree -- all elements, not just interactive ones.
///
/// Visibility detection checks:
/// - `hidden` attribute and `aria-hidden="true"`
/// - `display: none` (on element AND inherited from ancestors via tree walk)
/// - `visibility: hidden` / `visibility: collapse`
/// - `opacity: 0`
/// - `width: 0` or `height: 0` in inline style
/// - `overflow: hidden` with zero dimensions
/// - `position: absolute; left: -9999px` (off-screen)
/// - `clip: rect(0,0,0,0)` or `clip-path: inset(100%)`
/// - `pointer-events: none` (visible but non-interactive)
/// - Class names containing "hidden", "hide", "invisible", "sr-only", "visually-hidden"
fn extract_dom_nodes(dom: &Html) -> Vec<DomNode> {
    use ego_tree::iter::Edge;

    let mut nodes = Vec::new();
    let mut node_id: u64 = 0;

    // Tags that are never "visible" in the snapshot sense but must be in the DOM
    let invisible_tags = ["script", "style", "link", "meta", "head", "noscript", "template"];

    // Tags considered interactive (get highlighted in snapshot)
    let interactive_tags = [
        "a", "button", "input", "select", "textarea", "label", "summary",
        "h1", "h2", "h3", "h4", "h5", "h6", "img", "p",
    ];

    // Parent tracking for tree structure
    let mut id_stack: Vec<u64> = Vec::new();
    let mut node_id_map: HashMap<ego_tree::NodeId, u64> = HashMap::new();

    // Ancestor visibility: stack of (is_ancestor_hidden, is_ancestor_non_interactive).
    // display:none on a parent hides all descendants; pointer-events:none inherits too.
    let mut visibility_stack: Vec<(bool, bool)> = Vec::new();

    for edge in dom.tree.root().traverse() {
        match edge {
            Edge::Open(node_ref) => {
                if let Some(element) = node_ref.value().as_element() {
                    node_id += 1;
                    let tag = element.name().to_string();

                    // Extract attributes
                    let mut attributes = HashMap::new();
                    for attr in element.attrs() {
                        attributes.insert(attr.0.to_string(), attr.1.to_string());
                    }

                    // Get direct text content (not recursive for large trees)
                    let text_content = node_ref.children()
                        .filter_map(|c| c.value().as_text().map(|t| t.trim().to_string()))
                        .collect::<Vec<_>>()
                        .join(" ")
                        .trim()
                        .to_string();

                    // -- Enhanced visibility heuristic --

                    let is_invisible_tag = invisible_tags.contains(&tag.as_str());

                    // HTML attribute checks
                    let attr_hidden = attributes.contains_key("hidden")
                        || attributes.get("aria-hidden").map(|v| v == "true").unwrap_or(false);

                    // Inline style checks (parse once, check many properties)
                    let style_str = attributes.get("style").map(|s| s.as_str()).unwrap_or("");
                    let parsed_style = parse_inline_style(style_str);
                    let (style_hidden, style_non_interactive) = check_style_visibility(&parsed_style);

                    // Class-name checks
                    let class_hidden = attributes.get("class")
                        .map(|c| has_hidden_class(c))
                        .unwrap_or(false);

                    // Combine self + ancestor state
                    let self_hidden = attr_hidden || style_hidden || class_hidden;
                    let (ancestor_hidden, ancestor_non_interactive) = visibility_stack
                        .last().copied().unwrap_or((false, false));

                    let is_hidden = self_hidden || ancestor_hidden;
                    let is_non_interactive = style_non_interactive || ancestor_non_interactive;

                    // Push inherited state for this node's descendants
                    visibility_stack.push((is_hidden, is_non_interactive));

                    let is_visible = !is_hidden && !is_invisible_tag;

                    // Interactive = visible interactive-tag element that is not pointer-events:none
                    let is_interactive_tag = interactive_tags.contains(&tag.as_str())
                        || attributes.contains_key("role")
                        || attributes.contains_key("onclick")
                        || attributes.contains_key("href");
                    let is_interactive = is_visible && is_interactive_tag && !is_non_interactive;

                    // Skip empty non-interactive, non-structural elements to keep size manageable
                    let is_structural = matches!(tag.as_str(),
                        "div" | "form" | "section" | "main" | "nav" | "header" | "footer"
                        | "article" | "aside" | "ul" | "ol" | "li" | "table" | "tr" | "td"
                        | "th" | "thead" | "tbody" | "span" | "fieldset" | "legend"
                    );
                    let should_include = is_interactive_tag
                        || is_invisible_tag  // Always include scripts/styles for JS execution
                        || is_structural
                        || !text_content.is_empty()
                        || tag == "html" || tag == "body" || tag == "head"
                        || attributes.contains_key("id")
                        || attributes.contains_key("class");

                    if !should_include {
                        id_stack.push(0); // placeholder
                        continue;
                    }

                    let parent_id = id_stack.last().copied().filter(|&id| id > 0);

                    // Record parent-child relationship
                    if let Some(pid) = parent_id {
                        if let Some(parent_node) = nodes.iter_mut().find(|n: &&mut DomNode| n.node_id == pid) {
                            parent_node.children.push(node_id);
                        }
                    }

                    node_id_map.insert(node_ref.id(), node_id);
                    id_stack.push(node_id);

                    nodes.push(DomNode {
                        node_id,
                        node_type: DomNodeType::Element,
                        tag_name: tag,
                        attributes,
                        text_content,
                        children: vec![],
                        parent: parent_id,
                        bounding_box: None,
                        is_visible,
                        is_interactive,
                    });
                } else {
                    id_stack.push(id_stack.last().copied().unwrap_or(0));
                    visibility_stack.push(
                        visibility_stack.last().copied().unwrap_or((false, false))
                    );
                }
            }
            Edge::Close(_node_ref) => {
                id_stack.pop();
                visibility_stack.pop();
            }
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

        let visible_buttons: Vec<_> = engine.dom_nodes.iter()
            .filter(|n| n.tag_name == "button" && n.is_visible)
            .collect();
        assert_eq!(visible_buttons.len(), 1, "Should only have 1 visible button");
        assert_eq!(visible_buttons[0].text_content, "Visible");
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
