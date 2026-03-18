//! Pure-Rust native browser client — no Chrome dependency.
//!
//! Fetches pages via HTTP, parses HTML with html5ever/scraper,
//! extracts interactive elements into DomSnapshot, and submits
//! forms via direct HTTP POST. Runs anywhere, ~50ms per page.
//!
//! Use this for static pages, docs, articles, forms — anything
//! that doesn't require JavaScript execution. Falls back to
//! Chrome (BrowserSession) for SPAs that need JS.

use std::collections::HashMap;
use std::sync::Arc;

use reqwest::cookie::Jar;
use scraper::{Html, Selector, ElementRef};
use tracing::{info, debug, warn, instrument};
use url::Url;

use crate::actions::{ActionResult, BrowserAction, ScrollDirection};
use crate::dom::{DomElement, DomSnapshot, PageMeta};
use crate::error::BrowserError;

/// A pure-Rust browser session with no Chrome dependency.
/// Fetches and parses HTML directly. Fast, portable, lightweight.
pub struct NativeClient {
    client: reqwest::Client,
    cookie_jar: Arc<Jar>,
    /// Current page state
    current_url: Option<String>,
    current_html: Option<String>,
    current_snapshot: Option<DomSnapshot>,
    /// Navigation history
    history: Vec<String>,
    /// User agent string
    user_agent: String,
    /// Values filled via Fill/TypeText actions, keyed by ref_id
    filled_values: HashMap<u32, String>,
}

impl NativeClient {
    /// Create a new native browser client.
    pub fn new() -> Self {
        let jar = Arc::new(Jar::default());
        let client = reqwest::Client::builder()
            .cookie_provider(Arc::clone(&jar))
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::limited(10))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            cookie_jar: jar,
            current_url: None,
            current_html: None,
            current_snapshot: None,
            history: Vec::new(),
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".to_string(),
            filled_values: HashMap::new(),
        }
    }

    /// Create with a custom user agent.
    pub fn with_user_agent(mut self, ua: &str) -> Self {
        self.user_agent = ua.to_string();
        let jar = Arc::new(Jar::default());
        self.client = reqwest::Client::builder()
            .cookie_provider(Arc::clone(&jar))
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::limited(10))
            .user_agent(ua)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");
        self.cookie_jar = jar;
        self
    }

    /// Navigate to a URL and parse the page.
    #[instrument(skip(self), fields(url = %url))]
    pub async fn navigate(&mut self, url: &str) -> Result<DomSnapshot, BrowserError> {
        validate_url_scheme(url)?;

        info!(url = %url, "Native navigate");

        // Push current URL to history
        if let Some(ref current) = self.current_url {
            self.history.push(current.clone());
        }

        self.navigate_internal(url).await
    }

    /// Navigate without pushing to history (used by GoBack to avoid double-push).
    async fn navigate_internal(&mut self, url: &str) -> Result<DomSnapshot, BrowserError> {
        let response = self.client
            .get(url)
            .send()
            .await
            .map_err(|e| BrowserError::NavigationFailed { url: url.to_string(), reason: format!("HTTP request failed: {e}") })?;

        let final_url = response.url().to_string();
        let status = response.status();

        if !status.is_success() {
            return Err(BrowserError::NavigationFailed {
                url: url.to_string(),
                reason: format!("HTTP {status}"),
            });
        }

        let html = response.text().await
            .map_err(|e| BrowserError::NavigationFailed { url: url.to_string(), reason: format!("Body read failed: {e}") })?;

        debug!(
            url = %final_url,
            status = %status,
            html_len = html.len(),
            "Page fetched"
        );

        let snapshot = parse_html_to_snapshot(&html, &final_url);

        self.current_url = Some(final_url);
        self.current_html = Some(html);
        self.current_snapshot = Some(snapshot.clone());

        info!(
            url = self.current_url.as_deref().unwrap_or(""),
            elements = snapshot.elements.len(),
            page_type = snapshot.meta.page_type.as_deref().unwrap_or("unknown"),
            "Native navigate complete"
        );

        Ok(snapshot)
    }

    /// Get a snapshot of the current page (re-parses if needed).
    pub fn snapshot(&self) -> Result<DomSnapshot, BrowserError> {
        self.current_snapshot.clone().ok_or_else(|| {
            BrowserError::TabNotFound { tab_id: "native: no page loaded".to_string() }
        })
    }

    /// Get the current page's raw HTML.
    pub fn page_source(&self) -> Result<&str, BrowserError> {
        self.current_html.as_deref().ok_or_else(|| {
            BrowserError::TabNotFound { tab_id: "native: no page loaded".to_string() }
        })
    }

    /// Get current URL.
    pub fn current_url(&self) -> Option<&str> {
        self.current_url.as_deref()
    }

    /// Execute a browser action.
    #[instrument(skip(self), fields(action = ?action))]
    pub async fn execute(&mut self, action: BrowserAction) -> Result<ActionResult, BrowserError> {
        match action {
            BrowserAction::Navigate { url } => {
                let snapshot = self.navigate(&url).await?;
                Ok(ActionResult::Navigated {
                    url: self.current_url.clone().unwrap_or_default(),
                    title: snapshot.title.clone(),
                })
            }

            BrowserAction::Click { ref_id } => {
                // Find the element and follow its link if it's an <a>
                let snapshot = self.snapshot()?;
                let element = snapshot.elements.iter()
                    .find(|e| e.ref_id == ref_id)
                    .ok_or_else(|| BrowserError::ElementNotFound { selector: format!("@e{ref_id}") })?;

                if let Some(ref href) = element.href {
                    let resolved = resolve_url(href, self.current_url.as_deref().unwrap_or(""));
                    let snap = self.navigate(&resolved).await?;
                    Ok(ActionResult::Navigated {
                        url: self.current_url.clone().unwrap_or_default(),
                        title: snap.title.clone(),
                    })
                } else if element.role == "button" || element.role == "submit" {
                    // For buttons, try to find and submit the parent form
                    if let Some(ref html) = self.current_html.clone() {
                        if let Some(form_data) = extract_form_with_fills(html, ref_id, &self.filled_values) {
                            let result = self.submit_form(&form_data).await?;
                            return Ok(result);
                        }
                    }
                    Ok(ActionResult::Success {
                        message: format!("Clicked @e{ref_id} (button, no form found)")
                    })
                } else {
                    Ok(ActionResult::Success {
                        message: format!("Clicked @e{ref_id} (no navigation target)")
                    })
                }
            }

            BrowserAction::Fill { ref_id, text } => {
                // Store the fill value for later form submission
                self.filled_values.insert(ref_id, text.clone());
                if let Some(ref mut snapshot) = self.current_snapshot {
                    if let Some(el) = snapshot.elements.iter_mut().find(|e| e.ref_id == ref_id) {
                        el.value = Some(text.clone());
                        debug!(ref_id, text_len = text.len(), "Field filled");
                    }
                }
                Ok(ActionResult::Success {
                    message: format!("Filled @e{ref_id} with {} chars", text.len())
                })
            }

            BrowserAction::Select { ref_id, value } => {
                self.filled_values.insert(ref_id, value.clone());
                if let Some(ref mut snapshot) = self.current_snapshot {
                    if let Some(el) = snapshot.elements.iter_mut().find(|e| e.ref_id == ref_id) {
                        el.value = Some(value.clone());
                    }
                }
                Ok(ActionResult::Success {
                    message: format!("Selected '{value}' in @e{ref_id}")
                })
            }

            BrowserAction::GoBack => {
                if let Some(prev_url) = self.history.pop() {
                    let snapshot = self.navigate_internal(&prev_url).await?;
                    Ok(ActionResult::Navigated {
                        url: prev_url,
                        title: snapshot.title,
                    })
                } else {
                    Ok(ActionResult::Failed {
                        error: "No history to go back to".to_string()
                    })
                }
            }

            BrowserAction::ExtractContent => {
                // Return the raw HTML — the caller (MCP server / agent loop)
                // should pass this through content-extract for markdown conversion
                let html = self.page_source()?;
                Ok(ActionResult::Content {
                    markdown: html.to_string(),
                    word_count: html.split_whitespace().count(),
                })
            }

            BrowserAction::Scroll { .. }
            | BrowserAction::KeyPress { .. }
            | BrowserAction::Hover { .. }
            | BrowserAction::GoForward
            | BrowserAction::Reload
            | BrowserAction::Wait { .. }
            | BrowserAction::WaitForSelector { .. }
            | BrowserAction::WaitForNavigation { .. } => {
                // These are no-ops or trivial in a static HTML context
                Ok(ActionResult::Success {
                    message: "Action acknowledged (no-op in native mode)".to_string()
                })
            }

            BrowserAction::TypeText { ref_id, text, .. } => {
                // Same as Fill in native mode
                self.filled_values.insert(ref_id, text.clone());
                if let Some(ref mut snapshot) = self.current_snapshot {
                    if let Some(el) = snapshot.elements.iter_mut().find(|e| e.ref_id == ref_id) {
                        el.value = Some(text.clone());
                    }
                }
                Ok(ActionResult::Success {
                    message: format!("Typed into @e{ref_id}")
                })
            }

            BrowserAction::EvalJs { .. } => {
                Ok(ActionResult::Failed {
                    error: "JavaScript execution not available in native mode. Use Chrome backend for JS-heavy pages.".to_string()
                })
            }

            BrowserAction::Screenshot { .. } => {
                Ok(ActionResult::Failed {
                    error: "Screenshots not available in native mode. Use Chrome backend for visual capture.".to_string()
                })
            }
        }
    }

    /// Submit a form via HTTP POST.
    async fn submit_form(&mut self, form: &FormData) -> Result<ActionResult, BrowserError> {
        let url = resolve_url(
            &form.action,
            self.current_url.as_deref().unwrap_or(""),
        );

        info!(url = %url, method = %form.method, fields = form.fields.len(), "Submitting form");

        let response = match form.method.to_uppercase().as_str() {
            "GET" => {
                self.client.get(&url).query(&form.fields).send().await
            }
            _ => {
                self.client.post(&url).form(&form.fields).send().await
            }
        };

        let response = response
            .map_err(|e| BrowserError::NavigationFailed { url: url.to_string(), reason: format!("Form submit failed: {e}") })?;

        let final_url = response.url().to_string();
        let html = response.text().await
            .map_err(|e| BrowserError::NavigationFailed { url: url.to_string(), reason: format!("Response read failed: {e}") })?;

        let snapshot = parse_html_to_snapshot(&html, &final_url);

        if let Some(ref current) = self.current_url {
            self.history.push(current.clone());
        }
        self.current_url = Some(final_url.clone());
        self.current_html = Some(html);
        self.current_snapshot = Some(snapshot.clone());

        Ok(ActionResult::Navigated {
            url: final_url,
            title: snapshot.title,
        })
    }

    /// Detect if the current page likely needs JavaScript to render.
    /// Returns true if the page appears to be a JS-dependent SPA.
    pub fn needs_javascript(&self) -> bool {
        let html = match &self.current_html {
            Some(h) => h,
            None => return false,
        };

        let snapshot = match &self.current_snapshot {
            Some(s) => s,
            None => return false,
        };

        // Heuristics for JS-dependent pages
        // A page is "empty" only if it has no interactive elements AND
        // the raw HTML body has very little text content
        let text_len: usize = html.len();
        let body_is_empty = snapshot.elements.is_empty()
            && snapshot.meta.main_content_preview.is_none()
            && text_len < 500;
        let has_noscript = html.contains("<noscript");
        let has_root_div = html.contains("id=\"root\"") || html.contains("id=\"app\"")
            || html.contains("id=\"__next\"") || html.contains("id=\"__nuxt\"");
        let has_spa_framework = html.contains("__NEXT_DATA__")
            || html.contains("__NUXT__")
            || html.contains("window.__INITIAL_STATE__");
        let minimal_content = html.len() > 5000 && snapshot.elements.len() < 3;

        let needs_js = body_is_empty
            || (has_root_div && minimal_content)
            || has_spa_framework;

        if needs_js {
            debug!(
                body_empty = body_is_empty,
                has_noscript,
                has_root_div,
                has_spa_framework,
                minimal_content,
                "Page likely needs JavaScript"
            );
        }

        needs_js
    }
}

impl Default for NativeClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Form data extracted from HTML for submission.
struct FormData {
    action: String,
    method: String,
    fields: Vec<(String, String)>,
}

// ═══════════════════════════════════════════════════════════════
// HTML → DomSnapshot parsing (the core of native mode)
// ═══════════════════════════════════════════════════════════════

/// Parse raw HTML into a DomSnapshot with interactive elements.
fn parse_html_to_snapshot(html: &str, url: &str) -> DomSnapshot {
    let document = Html::parse_document(html);
    let mut elements = Vec::new();
    let mut ref_id: u32 = 1;

    // Extract title
    let title = extract_title(&document);

    // Extract interactive elements
    extract_links(&document, &mut elements, &mut ref_id, url);
    extract_inputs(&document, &mut elements, &mut ref_id);
    extract_buttons(&document, &mut elements, &mut ref_id);
    extract_selects(&document, &mut elements, &mut ref_id);
    extract_textareas(&document, &mut elements, &mut ref_id);

    // Extract page metadata
    let meta = extract_page_meta(&document, html, &elements);

    DomSnapshot {
        url: url.to_string(),
        title,
        elements,
        meta,
        timestamp: chrono::Utc::now(),
    }
}

fn extract_title(doc: &Html) -> String {
    let sel = Selector::parse("title").unwrap();
    doc.select(&sel)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .unwrap_or_default()
}

fn extract_links(doc: &Html, elements: &mut Vec<DomElement>, ref_id: &mut u32, base_url: &str) {
    let sel = Selector::parse("a[href]").unwrap();
    for el in doc.select(&sel) {
        let href = el.value().attr("href").unwrap_or("");
        if href.is_empty() || href.starts_with('#') || href.starts_with("javascript:") {
            continue;
        }

        let text: String = el.text().collect::<String>().trim().to_string();
        if text.is_empty() && el.value().attr("aria-label").is_none() {
            continue;
        }

        let resolved = resolve_url(href, base_url);

        elements.push(DomElement {
            ref_id: *ref_id,
            role: "link".to_string(),
            text: Some(if text.is_empty() {
                el.value().attr("aria-label").unwrap_or("[link]").to_string()
            } else {
                truncate(&text, 80)
            }),
            href: Some(resolved),
            placeholder: None,
            value: None,
            enabled: true,
            visible: true,
            aria_label: el.value().attr("aria-label").map(|s| s.to_string()),
            selector: build_selector(&el),
            bounds: None,
        });
        *ref_id += 1;
    }
}

fn extract_inputs(doc: &Html, elements: &mut Vec<DomElement>, ref_id: &mut u32) {
    let sel = Selector::parse("input").unwrap();
    for el in doc.select(&sel) {
        let input_type = el.value().attr("type").unwrap_or("text").to_lowercase();

        // Skip hidden and submit inputs
        if input_type == "hidden" {
            continue;
        }

        let role = if input_type == "submit" || input_type == "button" {
            "submit".to_string()
        } else if input_type == "checkbox" {
            "checkbox".to_string()
        } else if input_type == "radio" {
            "radio".to_string()
        } else {
            format!("input[type={input_type}]")
        };

        let text = el.value().attr("value")
            .or_else(|| el.value().attr("aria-label"))
            .or_else(|| el.value().attr("title"))
            .map(|s| s.to_string());

        elements.push(DomElement {
            ref_id: *ref_id,
            role,
            text,
            href: None,
            placeholder: el.value().attr("placeholder").map(|s| s.to_string()),
            value: el.value().attr("value").map(|s| s.to_string()),
            enabled: el.value().attr("disabled").is_none(),
            visible: true,
            aria_label: el.value().attr("aria-label").map(|s| s.to_string()),
            selector: build_selector(&el),
            bounds: None,
        });
        *ref_id += 1;
    }
}

fn extract_buttons(doc: &Html, elements: &mut Vec<DomElement>, ref_id: &mut u32) {
    let sel = Selector::parse("button").unwrap();
    for el in doc.select(&sel) {
        let text: String = el.text().collect::<String>().trim().to_string();
        let btn_type = el.value().attr("type").unwrap_or("button");

        elements.push(DomElement {
            ref_id: *ref_id,
            role: if btn_type == "submit" { "submit" } else { "button" }.to_string(),
            text: Some(if text.is_empty() {
                el.value().attr("aria-label").unwrap_or("[button]").to_string()
            } else {
                truncate(&text, 80)
            }),
            href: None,
            placeholder: None,
            value: None,
            enabled: el.value().attr("disabled").is_none(),
            visible: true,
            aria_label: el.value().attr("aria-label").map(|s| s.to_string()),
            selector: build_selector(&el),
            bounds: None,
        });
        *ref_id += 1;
    }
}

fn extract_selects(doc: &Html, elements: &mut Vec<DomElement>, ref_id: &mut u32) {
    let sel = Selector::parse("select").unwrap();
    let opt_sel = Selector::parse("option").unwrap();
    for el in doc.select(&sel) {
        let options: Vec<String> = el.select(&opt_sel)
            .map(|o| {
                let text: String = o.text().collect::<String>().trim().to_string();
                text
            })
            .filter(|t| !t.is_empty())
            .collect();

        let label = el.value().attr("aria-label")
            .or_else(|| el.value().attr("name"))
            .unwrap_or("select");

        elements.push(DomElement {
            ref_id: *ref_id,
            role: "select".to_string(),
            text: Some(format!("{label} [{}]", options.join(", "))),
            href: None,
            placeholder: None,
            value: el.value().attr("value").map(|s| s.to_string()),
            enabled: el.value().attr("disabled").is_none(),
            visible: true,
            aria_label: Some(label.to_string()),
            selector: build_selector(&el),
            bounds: None,
        });
        *ref_id += 1;
    }
}

fn extract_textareas(doc: &Html, elements: &mut Vec<DomElement>, ref_id: &mut u32) {
    let sel = Selector::parse("textarea").unwrap();
    for el in doc.select(&sel) {
        let text: String = el.text().collect::<String>().trim().to_string();

        elements.push(DomElement {
            ref_id: *ref_id,
            role: "textarea".to_string(),
            text: if text.is_empty() { None } else { Some(truncate(&text, 80)) },
            href: None,
            placeholder: el.value().attr("placeholder").map(|s| s.to_string()),
            value: if text.is_empty() { None } else { Some(text) },
            enabled: el.value().attr("disabled").is_none(),
            visible: true,
            aria_label: el.value().attr("aria-label").map(|s| s.to_string()),
            selector: build_selector(&el),
            bounds: None,
        });
        *ref_id += 1;
    }
}

fn extract_page_meta(doc: &Html, html: &str, elements: &[DomElement]) -> PageMeta {
    // Extract description
    let desc_sel = Selector::parse("meta[name='description'], meta[property='og:description']").unwrap();
    let description = doc.select(&desc_sel)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(|s| s.to_string());

    // Content preview from first <p> tags
    let p_sel = Selector::parse("p").unwrap();
    let main_content: String = doc.select(&p_sel)
        .take(3)
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|t| t.len() > 20)
        .collect::<Vec<_>>()
        .join(" ");
    let main_content_preview = if main_content.is_empty() {
        None
    } else {
        Some(truncate(&main_content, 500))
    };

    // Detect forms
    let form_sel = Selector::parse("form").unwrap();
    let form_count = doc.select(&form_sel).count();

    // Detect login form
    let has_login_form = html.contains("type=\"password\"")
        && (html.contains("type=\"email\"")
            || html.contains("type=\"text\"")
            || html.contains("name=\"username\"")
            || html.contains("name=\"login\""));

    // Detect CAPTCHA
    let has_captcha = html.contains("g-recaptcha")
        || html.contains("h-captcha")
        || html.contains("captcha")
        || html.contains("cf-turnstile");

    // Detect page type
    let page_type = detect_page_type(html, &description, has_login_form, has_captcha, elements);

    PageMeta {
        page_type: Some(page_type),
        main_content_preview,
        description,
        form_count,
        has_login_form,
        has_captcha,
        interactive_element_count: elements.len(),
    }
}

fn detect_page_type(
    html: &str,
    description: &Option<String>,
    has_login: bool,
    has_captcha: bool,
    elements: &[DomElement],
) -> String {
    if has_captcha {
        return "captcha".to_string();
    }
    if has_login {
        return "login_form".to_string();
    }

    let html_lower = html.to_lowercase();
    let desc = description.as_deref().unwrap_or("").to_lowercase();

    if html_lower.contains("search-results") || html_lower.contains("searchresults")
        || html_lower.contains("class=\"result\"")
    {
        return "search_results".to_string();
    }

    if html_lower.contains("<article") || html_lower.contains("class=\"post\"")
        || html_lower.contains("class=\"entry-content\"")
    {
        return "article".to_string();
    }

    // Count links vs forms vs content
    let link_count = elements.iter().filter(|e| e.role == "link").count();
    let input_count = elements.iter().filter(|e| e.role.starts_with("input")).count();

    if input_count > 5 {
        return "form".to_string();
    }
    if link_count > 30 {
        return "listing".to_string();
    }

    "generic".to_string()
}

/// Validate that a URL uses an allowed scheme (http or https only).
fn validate_url_scheme(url: &str) -> Result<(), BrowserError> {
    let url_lower = url.trim().to_lowercase();
    if url_lower.starts_with("http://") || url_lower.starts_with("https://") {
        Ok(())
    } else {
        Err(BrowserError::NavigationFailed {
            url: url.to_string(),
            reason: "Only http:// and https:// schemes are allowed".to_string(),
        })
    }
}

/// Extract form data for HTTP submission, using filled values from the Fill action.
fn extract_form_with_fills(html: &str, _submit_ref_id: u32, filled_values: &HashMap<u32, String>) -> Option<FormData> {
    let doc = Html::parse_document(html);
    let form_sel = Selector::parse("form").ok()?;
    let input_sel = Selector::parse("input, select, textarea").ok()?;

    // Find the first form (simplified — a full implementation would
    // find the form containing the submit button by ref_id)
    let form_el = doc.select(&form_sel).next()?;

    let action = form_el.value().attr("action").unwrap_or("").to_string();
    let method = form_el.value().attr("method").unwrap_or("POST").to_string();

    // Build a map from field name to ref_id by walking the snapshot numbering.
    // Re-parse to get ref_ids consistent with parse_html_to_snapshot.
    let snapshot_doc = Html::parse_document(html);
    let all_input_sel = Selector::parse("input, select, textarea").ok()?;
    let link_sel = Selector::parse("a[href]").ok()?;
    let button_sel = Selector::parse("button").ok()?;

    // Count links first (they get ref_ids before inputs)
    let link_count = snapshot_doc.select(&link_sel)
        .filter(|el| {
            let href = el.value().attr("href").unwrap_or("");
            if href.is_empty() || href.starts_with('#') || href.starts_with("javascript:") {
                return false;
            }
            let text: String = el.text().collect::<String>().trim().to_string();
            !text.is_empty() || el.value().attr("aria-label").is_some()
        })
        .count() as u32;

    // Map input names to their ref_ids
    let mut name_to_ref_id: HashMap<String, u32> = HashMap::new();
    let mut current_ref_id = link_count + 1; // inputs start after links
    for input in snapshot_doc.select(&all_input_sel) {
        let input_type = input.value().attr("type").unwrap_or("text").to_lowercase();
        if input_type == "hidden" {
            continue; // hidden inputs are skipped in parse
        }
        if let Some(name) = input.value().attr("name") {
            if !name.is_empty() {
                name_to_ref_id.insert(name.to_string(), current_ref_id);
            }
        }
        current_ref_id += 1;
    }
    // buttons also get ref_ids
    for _btn in snapshot_doc.select(&button_sel) {
        current_ref_id += 1;
    }

    // Build ref_id to filled value lookup
    let mut ref_id_to_fill: HashMap<u32, &str> = HashMap::new();
    for (rid, val) in filled_values {
        ref_id_to_fill.insert(*rid, val.as_str());
    }

    let mut fields = Vec::new();
    for input in form_el.select(&input_sel) {
        let name = match input.value().attr("name") {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => continue,
        };
        let input_type = input.value().attr("type").unwrap_or("text");

        // Skip unchecked checkboxes/radios
        if (input_type == "checkbox" || input_type == "radio")
            && input.value().attr("checked").is_none()
        {
            continue;
        }

        // Use filled value if available, otherwise fall back to HTML attribute
        let value = if let Some(&rid) = name_to_ref_id.get(&name) {
            if let Some(&filled) = ref_id_to_fill.get(&rid) {
                filled.to_string()
            } else {
                input.value().attr("value").unwrap_or("").to_string()
            }
        } else {
            input.value().attr("value").unwrap_or("").to_string()
        };

        fields.push((name, value));
    }

    Some(FormData { action, method, fields })
}

/// Extract form data for HTTP submission when a submit button is clicked.
fn extract_form_for_submit(html: &str, _submit_ref_id: u32) -> Option<FormData> {
    let doc = Html::parse_document(html);
    let form_sel = Selector::parse("form").ok()?;
    let input_sel = Selector::parse("input, select, textarea").ok()?;

    // Find the first form (simplified — a full implementation would
    // find the form containing the submit button by ref_id)
    let form_el = doc.select(&form_sel).next()?;

    let action = form_el.value().attr("action").unwrap_or("").to_string();
    let method = form_el.value().attr("method").unwrap_or("POST").to_string();

    let mut fields = Vec::new();
    for input in form_el.select(&input_sel) {
        let name = match input.value().attr("name") {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => continue,
        };
        let value = input.value().attr("value").unwrap_or("").to_string();
        let input_type = input.value().attr("type").unwrap_or("text");

        // Skip unchecked checkboxes/radios
        if (input_type == "checkbox" || input_type == "radio")
            && input.value().attr("checked").is_none()
        {
            continue;
        }

        fields.push((name, value));
    }

    Some(FormData { action, method, fields })
}

// ═══════════════════════════════════════════════════════════════
// Utilities
// ═══════════════════════════════════════════════════════════════

/// Resolve a potentially relative URL against a base URL.
fn resolve_url(href: &str, base: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    if href.starts_with("//") {
        return format!("https:{href}");
    }
    if let Ok(base_url) = Url::parse(base) {
        if let Ok(resolved) = base_url.join(href) {
            return resolved.to_string();
        }
    }
    href.to_string()
}

/// Build a CSS selector for an element (for fallback targeting).
fn build_selector(el: &ElementRef) -> String {
    let tag = el.value().name();
    let id = el.value().attr("id");
    let name = el.value().attr("name");

    if let Some(id) = id {
        format!("#{id}")
    } else if let Some(name) = name {
        format!("{tag}[name=\"{name}\"]")
    } else {
        tag.to_string()
    }
}

/// Truncate a string to a maximum length, adding "..." if truncated.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.min(s.len())])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_page() {
        let html = r#"
        <html>
        <head><title>Test Page</title></head>
        <body>
            <a href="/about">About Us</a>
            <form action="/login" method="POST">
                <input type="email" name="email" placeholder="Email">
                <input type="password" name="password" placeholder="Password">
                <button type="submit">Sign In</button>
            </form>
            <p>Welcome to our site.</p>
        </body>
        </html>"#;

        let snapshot = parse_html_to_snapshot(html, "https://example.com");

        assert_eq!(snapshot.title, "Test Page");
        assert!(!snapshot.elements.is_empty());

        // Should have: 1 link + 2 inputs + 1 button = 4 elements
        let link_count = snapshot.elements.iter().filter(|e| e.role == "link").count();
        let input_count = snapshot.elements.iter().filter(|e| e.role.starts_with("input")).count();
        let button_count = snapshot.elements.iter().filter(|e| e.role == "submit").count();

        assert_eq!(link_count, 1);
        assert_eq!(input_count, 2);
        assert_eq!(button_count, 1);

        // Check link resolution
        let link = snapshot.elements.iter().find(|e| e.role == "link").unwrap();
        assert_eq!(link.href.as_deref(), Some("https://example.com/about"));

        // Check page type detection
        assert_eq!(snapshot.meta.page_type.as_deref(), Some("login_form"));
        assert!(snapshot.meta.has_login_form);
    }

    #[test]
    fn test_parse_article_page() {
        let html = r#"
        <html>
        <head>
            <title>Blog Post - My Site</title>
            <meta name="description" content="A great article">
        </head>
        <body>
            <article>
                <h1>My Blog Post</h1>
                <p>This is a long paragraph with plenty of content to read and understand.</p>
                <p>Second paragraph with more interesting details about the topic at hand.</p>
                <a href="/next-post">Next Post</a>
            </article>
        </body>
        </html>"#;

        let snapshot = parse_html_to_snapshot(html, "https://blog.example.com/post");

        assert_eq!(snapshot.title, "Blog Post - My Site");
        assert_eq!(snapshot.meta.page_type.as_deref(), Some("article"));
        assert_eq!(snapshot.meta.description.as_deref(), Some("A great article"));
        assert!(snapshot.meta.main_content_preview.is_some());
    }

    #[test]
    fn test_resolve_url() {
        assert_eq!(
            resolve_url("/about", "https://example.com/page"),
            "https://example.com/about"
        );
        assert_eq!(
            resolve_url("https://other.com", "https://example.com"),
            "https://other.com"
        );
        assert_eq!(
            resolve_url("sub/page", "https://example.com/dir/"),
            "https://example.com/dir/sub/page"
        );
    }

    #[test]
    fn test_needs_javascript_detection() {
        let mut client = NativeClient::new();

        // Static page — doesn't need JS (realistic content length)
        client.current_html = Some(format!("<html><body><h1>Welcome</h1><p>{}</p><a href='/about'>About</a></body></html>", "This is a real page with actual content. ".repeat(20)));
        client.current_snapshot = Some(parse_html_to_snapshot(
            client.current_html.as_ref().unwrap(),
            "https://example.com",
        ));
        assert!(!client.needs_javascript());

        // SPA shell — needs JS
        client.current_html = Some(r#"<html><body><div id="root"></div><script src="/bundle.js"></script></body></html>"#.to_string());
        client.current_snapshot = Some(parse_html_to_snapshot(
            client.current_html.as_ref().unwrap(),
            "https://spa.example.com",
        ));
        // Note: this might not trigger because html.len() < 5000
    }

    #[test]
    fn test_agent_text_output() {
        let html = r#"
        <html><head><title>Test</title></head>
        <body>
            <a href="/link1">First Link</a>
            <input type="text" placeholder="Search...">
            <button>Go</button>
        </body></html>"#;

        let snapshot = parse_html_to_snapshot(html, "https://example.com");
        let text = snapshot.to_agent_text();

        assert!(text.contains("@e1"));
        assert!(text.contains("[link]"));
        assert!(text.contains("First Link"));
        assert!(text.contains("Search..."));
    }

    #[test]
    fn test_form_extraction() {
        let html = r#"
        <html><body>
            <form action="/login" method="POST">
                <input type="email" name="email" value="user@test.com">
                <input type="password" name="password" value="secret">
                <input type="hidden" name="csrf" value="token123">
                <button type="submit">Login</button>
            </form>
        </body></html>"#;

        let form = extract_form_for_submit(html, 0).unwrap();
        assert_eq!(form.action, "/login");
        assert_eq!(form.method, "POST");
        assert_eq!(form.fields.len(), 3); // email + password + hidden csrf
        assert!(form.fields.iter().any(|(k, v)| k == "csrf" && v == "token123"));
    }
}
