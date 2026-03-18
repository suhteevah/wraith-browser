use base64::Engine as _;
use tracing::{info, debug, instrument};
use chromiumoxide::Page;
use chromiumoxide::page::ScreenshotParams;
use chromiumoxide_cdp::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide_cdp::cdp::browser_protocol::input::{
    DispatchMouseEventParams, DispatchMouseEventType,
    DispatchKeyEventParams, DispatchKeyEventType,
    InsertTextParams,
};

use crate::dom::{DomSnapshot, DomElement, PageMeta};
use crate::actions::{BrowserAction, ActionResult, ScrollDirection};
use crate::error::{BrowserError, BrowserResult};

/// Handle to a single browser tab. AI agents interact with web pages through this.
#[derive(Debug, Clone)]
pub struct TabHandle {
    pub id: String,
    pub current_url: String,
    pub title: Option<String>,
    pub page: Page,
}

impl TabHandle {
    /// Navigate this tab to a new URL.
    #[instrument(skip(self), fields(tab_id = %self.id))]
    pub async fn navigate(&mut self, url: &str) -> BrowserResult<()> {
        info!(url = %url, "Navigating tab");
        self.page.goto(url).await
            .map_err(|e| BrowserError::NavigationFailed {
                url: url.to_string(),
                reason: e.to_string(),
            })?;

        self.current_url = self.page.url().await
            .unwrap_or(Some(url.to_string()))
            .unwrap_or_else(|| url.to_string());

        self.title = self.page.get_title().await.unwrap_or(None);

        debug!(url = %self.current_url, title = ?self.title, "Navigation complete");
        Ok(())
    }

    /// Take a DOM snapshot optimized for AI agent consumption.
    /// Returns a flat list of interactive elements with semantic roles,
    /// text content, and ref IDs for actions.
    #[instrument(skip(self), fields(tab_id = %self.id))]
    pub async fn snapshot(&self) -> BrowserResult<DomSnapshot> {
        debug!(url = %self.current_url, "Taking DOM snapshot");

        // Use JavaScript to extract interactive elements — more reliable than
        // raw CDP DOM traversal for getting visible, actionable elements
        let js_result: String = self.page.evaluate(SNAPSHOT_SCRIPT).await
            .map_err(|e| BrowserError::JsEvalFailed(format!("Snapshot script failed: {}", e)))?
            .into_value()
            .map_err(|e| BrowserError::JsEvalFailed(format!("Snapshot parse failed: {}", e)))?;

        let raw: RawSnapshot = serde_json::from_str(&js_result)
            .map_err(|e| BrowserError::JsEvalFailed(format!("Snapshot JSON parse failed: {}", e)))?;

        let title = self.page.get_title().await
            .unwrap_or(None)
            .unwrap_or_default();

        let url = self.current_url.clone();

        let elements: Vec<DomElement> = raw.elements.into_iter().enumerate().map(|(i, el)| {
            DomElement {
                ref_id: (i + 1) as u32,
                role: el.role,
                text: el.text,
                href: el.href,
                placeholder: el.placeholder,
                value: el.value,
                enabled: el.enabled,
                visible: el.visible,
                aria_label: el.aria_label,
                selector: el.selector,
                bounds: el.bounds.map(|b| (b[0], b[1], b[2], b[3])),
            }
        }).collect();

        let interactive_count = elements.len();

        let meta = PageMeta {
            page_type: raw.page_type,
            main_content_preview: raw.main_content_preview,
            description: raw.description,
            form_count: raw.form_count,
            has_login_form: raw.has_login_form,
            has_captcha: raw.has_captcha,
            interactive_element_count: interactive_count,
        };

        debug!(
            url = %url,
            elements = interactive_count,
            page_type = ?meta.page_type,
            has_login = meta.has_login_form,
            "DOM snapshot captured"
        );

        Ok(DomSnapshot {
            url,
            title,
            elements,
            meta,
            timestamp: chrono::Utc::now(),
        })
    }

    /// Execute an action on this tab (click, fill, scroll, etc.)
    #[instrument(skip(self), fields(tab_id = %self.id, action = ?action))]
    pub async fn execute(&mut self, action: BrowserAction) -> BrowserResult<ActionResult> {
        info!(action = ?action, "Executing browser action");

        match action {
            BrowserAction::Navigate { url } => {
                self.navigate(&url).await?;
                Ok(ActionResult::Navigated {
                    url: self.current_url.clone(),
                    title: self.title.clone().unwrap_or_default(),
                })
            }

            BrowserAction::Click { ref_id } => {
                // Get element bounds via JS, then dispatch click
                let js = format!(
                    r#"(() => {{
                        const els = document.querySelectorAll(
                            'a, button, input, select, textarea, [role="button"], [role="link"], [role="tab"], [onclick], summary, details, label'
                        );
                        const visible = Array.from(els).filter(el => {{
                            const r = el.getBoundingClientRect();
                            return r.width > 0 && r.height > 0;
                        }});
                        const el = visible[{} - 1];
                        if (!el) return JSON.stringify({{error: "Element @e{} not found"}});
                        const r = el.getBoundingClientRect();
                        return JSON.stringify({{x: r.x + r.width/2, y: r.y + r.height/2}});
                    }})()"#,
                    ref_id, ref_id
                );

                let result: String = self.page.evaluate(js.as_str()).await
                    .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?
                    .into_value()
                    .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?;

                let coords: serde_json::Value = serde_json::from_str(&result)
                    .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?;

                if let Some(_err) = coords.get("error") {
                    return Err(BrowserError::ElementNotFound {
                        selector: format!("@e{}", ref_id),
                    });
                }

                let x = coords["x"].as_f64().unwrap_or(0.0);
                let y = coords["y"].as_f64().unwrap_or(0.0);

                // Dispatch mouse events: move, press, release
                self.page.execute(
                    DispatchMouseEventParams::builder()
                        .r#type(DispatchMouseEventType::MouseMoved)
                        .x(x)
                        .y(y)
                        .build()
                        .unwrap()
                ).await.map_err(|e| BrowserError::CdpError(e.to_string()))?;

                self.page.execute(
                    DispatchMouseEventParams::builder()
                        .r#type(DispatchMouseEventType::MousePressed)
                        .x(x)
                        .y(y)
                        .button(chromiumoxide_cdp::cdp::browser_protocol::input::MouseButton::Left)
                        .click_count(1)
                        .build()
                        .unwrap()
                ).await.map_err(|e| BrowserError::CdpError(e.to_string()))?;

                self.page.execute(
                    DispatchMouseEventParams::builder()
                        .r#type(DispatchMouseEventType::MouseReleased)
                        .x(x)
                        .y(y)
                        .button(chromiumoxide_cdp::cdp::browser_protocol::input::MouseButton::Left)
                        .click_count(1)
                        .build()
                        .unwrap()
                ).await.map_err(|e| BrowserError::CdpError(e.to_string()))?;

                // Update URL in case click triggered navigation
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                self.current_url = self.page.url().await
                    .unwrap_or(Some(self.current_url.clone()))
                    .unwrap_or_else(|| self.current_url.clone());
                self.title = self.page.get_title().await.unwrap_or(None);

                Ok(ActionResult::Success {
                    message: format!("Clicked @e{}", ref_id),
                })
            }

            BrowserAction::Fill { ref_id, text } => {
                // Focus the element then insert text
                let js = format!(
                    r#"(() => {{
                        const els = document.querySelectorAll(
                            'a, button, input, select, textarea, [role="button"], [role="link"], [role="tab"], [onclick], summary, details, label'
                        );
                        const visible = Array.from(els).filter(el => {{
                            const r = el.getBoundingClientRect();
                            return r.width > 0 && r.height > 0;
                        }});
                        const el = visible[{} - 1];
                        if (!el) return "NOT_FOUND";
                        el.focus();
                        el.value = '';
                        return "OK";
                    }})()"#,
                    ref_id
                );

                let result: String = self.page.evaluate(js.as_str()).await
                    .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?
                    .into_value()
                    .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?;

                if result == "NOT_FOUND" {
                    return Err(BrowserError::ElementNotFound {
                        selector: format!("@e{}", ref_id),
                    });
                }

                // Insert text via CDP
                self.page.execute(InsertTextParams::new(&text)).await
                    .map_err(|e| BrowserError::CdpError(e.to_string()))?;

                // Dispatch input/change events so frameworks pick up the value
                let dispatch_js = format!(
                    r#"(() => {{
                        const els = document.querySelectorAll(
                            'a, button, input, select, textarea, [role="button"], [role="link"], [role="tab"], [onclick], summary, details, label'
                        );
                        const visible = Array.from(els).filter(el => {{
                            const r = el.getBoundingClientRect();
                            return r.width > 0 && r.height > 0;
                        }});
                        const el = visible[{} - 1];
                        if (el) {{
                            el.dispatchEvent(new Event('input', {{bubbles: true}}));
                            el.dispatchEvent(new Event('change', {{bubbles: true}}));
                        }}
                    }})()"#,
                    ref_id
                );
                let _ = self.page.evaluate(dispatch_js.as_str()).await;

                Ok(ActionResult::Success {
                    message: format!("Filled @e{} with text ({} chars)", ref_id, text.len()),
                })
            }

            BrowserAction::Select { ref_id, value } => {
                let js = format!(
                    r#"(() => {{
                        const els = document.querySelectorAll(
                            'a, button, input, select, textarea, [role="button"], [role="link"], [role="tab"], [onclick], summary, details, label'
                        );
                        const visible = Array.from(els).filter(el => {{
                            const r = el.getBoundingClientRect();
                            return r.width > 0 && r.height > 0;
                        }});
                        const el = visible[{} - 1];
                        if (!el || el.tagName !== 'SELECT') return "NOT_FOUND";
                        el.value = {};
                        el.dispatchEvent(new Event('change', {{bubbles: true}}));
                        return "OK";
                    }})()"#,
                    ref_id,
                    serde_json::to_string(&value).unwrap_or_default()
                );

                let result: String = self.page.evaluate(js.as_str()).await
                    .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?
                    .into_value()
                    .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?;

                if result == "NOT_FOUND" {
                    return Err(BrowserError::ElementNotFound {
                        selector: format!("@e{}", ref_id),
                    });
                }

                Ok(ActionResult::Success {
                    message: format!("Selected '{}' on @e{}", value, ref_id),
                })
            }

            BrowserAction::KeyPress { key } => {
                self.page.execute(
                    DispatchKeyEventParams::builder()
                        .r#type(DispatchKeyEventType::KeyDown)
                        .key(&key)
                        .build()
                        .unwrap()
                ).await.map_err(|e| BrowserError::CdpError(e.to_string()))?;

                self.page.execute(
                    DispatchKeyEventParams::builder()
                        .r#type(DispatchKeyEventType::KeyUp)
                        .key(&key)
                        .build()
                        .unwrap()
                ).await.map_err(|e| BrowserError::CdpError(e.to_string()))?;

                Ok(ActionResult::Success {
                    message: format!("Pressed key: {}", key),
                })
            }

            BrowserAction::TypeText { ref_id, text, delay_ms } => {
                // Focus element first
                let focus_js = format!(
                    r#"(() => {{
                        const els = document.querySelectorAll(
                            'a, button, input, select, textarea, [role="button"], [role="link"], [role="tab"], [onclick], summary, details, label'
                        );
                        const visible = Array.from(els).filter(el => {{
                            const r = el.getBoundingClientRect();
                            return r.width > 0 && r.height > 0;
                        }});
                        const el = visible[{} - 1];
                        if (!el) return "NOT_FOUND";
                        el.focus();
                        return "OK";
                    }})()"#,
                    ref_id
                );

                let result: String = self.page.evaluate(focus_js.as_str()).await
                    .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?
                    .into_value()
                    .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?;

                if result == "NOT_FOUND" {
                    return Err(BrowserError::ElementNotFound {
                        selector: format!("@e{}", ref_id),
                    });
                }

                // Type each character with delay
                for ch in text.chars() {
                    self.page.execute(InsertTextParams::new(ch.to_string())).await
                        .map_err(|e| BrowserError::CdpError(e.to_string()))?;
                    if delay_ms > 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms as u64)).await;
                    }
                }

                Ok(ActionResult::Success {
                    message: format!("Typed {} chars into @e{} with {}ms delay", text.len(), ref_id, delay_ms),
                })
            }

            BrowserAction::Scroll { direction, amount } => {
                let (dx, dy) = match direction {
                    ScrollDirection::Down => (0, amount),
                    ScrollDirection::Up => (0, -amount),
                    ScrollDirection::Right => (amount, 0),
                    ScrollDirection::Left => (-amount, 0),
                };

                let js = format!("window.scrollBy({}, {})", dx, dy);
                self.page.evaluate(js.as_str()).await
                    .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?;

                Ok(ActionResult::Success {
                    message: format!("Scrolled {:?} by {}px", direction, amount),
                })
            }

            BrowserAction::Hover { ref_id } => {
                let js = format!(
                    r#"(() => {{
                        const els = document.querySelectorAll(
                            'a, button, input, select, textarea, [role="button"], [role="link"], [role="tab"], [onclick], summary, details, label'
                        );
                        const visible = Array.from(els).filter(el => {{
                            const r = el.getBoundingClientRect();
                            return r.width > 0 && r.height > 0;
                        }});
                        const el = visible[{} - 1];
                        if (!el) return JSON.stringify({{error: true}});
                        const r = el.getBoundingClientRect();
                        return JSON.stringify({{x: r.x + r.width/2, y: r.y + r.height/2}});
                    }})()"#,
                    ref_id
                );

                let result: String = self.page.evaluate(js.as_str()).await
                    .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?
                    .into_value()
                    .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?;

                let coords: serde_json::Value = serde_json::from_str(&result)
                    .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?;

                if coords.get("error").is_some() {
                    return Err(BrowserError::ElementNotFound {
                        selector: format!("@e{}", ref_id),
                    });
                }

                let x = coords["x"].as_f64().unwrap_or(0.0);
                let y = coords["y"].as_f64().unwrap_or(0.0);

                self.page.execute(
                    DispatchMouseEventParams::builder()
                        .r#type(DispatchMouseEventType::MouseMoved)
                        .x(x)
                        .y(y)
                        .build()
                        .unwrap()
                ).await.map_err(|e| BrowserError::CdpError(e.to_string()))?;

                Ok(ActionResult::Success {
                    message: format!("Hovered @e{}", ref_id),
                })
            }

            BrowserAction::GoBack => {
                let js = "window.history.back()";
                self.page.evaluate(js).await
                    .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                self.current_url = self.page.url().await
                    .unwrap_or(Some(self.current_url.clone()))
                    .unwrap_or_else(|| self.current_url.clone());
                Ok(ActionResult::Success { message: "Navigated back".to_string() })
            }

            BrowserAction::GoForward => {
                let js = "window.history.forward()";
                self.page.evaluate(js).await
                    .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                self.current_url = self.page.url().await
                    .unwrap_or(Some(self.current_url.clone()))
                    .unwrap_or_else(|| self.current_url.clone());
                Ok(ActionResult::Success { message: "Navigated forward".to_string() })
            }

            BrowserAction::Reload => {
                self.page.reload().await
                    .map_err(|e| BrowserError::CdpError(e.to_string()))?;
                Ok(ActionResult::Success { message: "Page reloaded".to_string() })
            }

            BrowserAction::Wait { ms } => {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                Ok(ActionResult::Success {
                    message: format!("Waited {}ms", ms),
                })
            }

            BrowserAction::WaitForSelector { selector, timeout_ms } => {
                let js = format!(
                    r#"new Promise((resolve, reject) => {{
                        const start = Date.now();
                        const check = () => {{
                            if (document.querySelector({})) return resolve("found");
                            if (Date.now() - start > {}) return resolve("timeout");
                            requestAnimationFrame(check);
                        }};
                        check();
                    }})"#,
                    serde_json::to_string(&selector).unwrap_or_default(),
                    timeout_ms
                );

                let result: String = self.page.evaluate(js.as_str()).await
                    .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?
                    .into_value()
                    .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?;

                if result == "timeout" {
                    return Err(BrowserError::Timeout {
                        action: format!("wait_for({})", selector),
                        ms: timeout_ms,
                    });
                }

                Ok(ActionResult::Success {
                    message: format!("Selector '{}' found", selector),
                })
            }

            BrowserAction::WaitForNavigation { timeout_ms } => {
                tokio::time::sleep(std::time::Duration::from_millis(timeout_ms.min(5000))).await;
                self.current_url = self.page.url().await
                    .unwrap_or(Some(self.current_url.clone()))
                    .unwrap_or_else(|| self.current_url.clone());
                self.title = self.page.get_title().await.unwrap_or(None);
                Ok(ActionResult::Navigated {
                    url: self.current_url.clone(),
                    title: self.title.clone().unwrap_or_default(),
                })
            }

            BrowserAction::EvalJs { script } => {
                let result = self.eval_js(&script).await?;
                Ok(ActionResult::JsResult { value: result })
            }

            BrowserAction::Screenshot { full_page } => {
                let png = self.screenshot_impl(full_page).await?;
                // Decode PNG header to get actual dimensions (IHDR chunk at bytes 16-23)
                let (width, height) = if png.len() >= 24 {
                    let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
                    let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
                    (w, h)
                } else {
                    (0, 0)
                };
                let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
                Ok(ActionResult::Screenshot {
                    png_base64: b64,
                    width,
                    height,
                })
            }

            BrowserAction::ExtractContent => {
                let html = self.page_source().await?;
                // Return raw HTML — the caller (agent-loop/mcp) delegates to content-extract
                Ok(ActionResult::Content {
                    markdown: html,
                    word_count: 0,
                })
            }
        }
    }

    /// Take a screenshot, returns PNG bytes.
    #[instrument(skip(self), fields(tab_id = %self.id))]
    pub async fn screenshot(&self) -> BrowserResult<Vec<u8>> {
        self.screenshot_impl(false).await
    }

    async fn screenshot_impl(&self, full_page: bool) -> BrowserResult<Vec<u8>> {
        debug!(full_page, "Taking screenshot");
        let params = ScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Png)
            .full_page(full_page)
            .build();

        self.page.screenshot(params).await
            .map_err(|e| BrowserError::ScreenshotFailed(e.to_string()))
    }

    /// Execute arbitrary JavaScript and return the result as a string.
    #[instrument(skip(self, script), fields(tab_id = %self.id))]
    pub async fn eval_js(&self, script: &str) -> BrowserResult<String> {
        debug!(script_len = script.len(), "Evaluating JavaScript");
        let result: serde_json::Value = self.page.evaluate(script).await
            .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?
            .into_value()
            .unwrap_or(serde_json::Value::Null);

        match result {
            serde_json::Value::String(s) => Ok(s),
            other => Ok(other.to_string()),
        }
    }

    /// Get the page's full HTML source.
    pub async fn page_source(&self) -> BrowserResult<String> {
        self.page.content().await
            .map_err(|e| BrowserError::JsEvalFailed(format!("Failed to get page source: {}", e)))
    }

    /// Wait for a selector to appear in the DOM.
    #[instrument(skip(self), fields(tab_id = %self.id))]
    pub async fn wait_for(&self, selector: &str, timeout_ms: u64) -> BrowserResult<()> {
        debug!(selector = %selector, timeout_ms, "Waiting for selector");
        let _action = BrowserAction::WaitForSelector {
            selector: selector.to_string(),
            timeout_ms,
        };
        // Can't call self.execute since it takes &mut self, so inline the logic
        let js = format!(
            r#"new Promise((resolve) => {{
                const start = Date.now();
                const check = () => {{
                    if (document.querySelector({})) return resolve("found");
                    if (Date.now() - start > {}) return resolve("timeout");
                    requestAnimationFrame(check);
                }};
                check();
            }})"#,
            serde_json::to_string(selector).unwrap_or_default(),
            timeout_ms
        );

        let result: String = self.page.evaluate(js.as_str()).await
            .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?
            .into_value()
            .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?;

        if result == "timeout" {
            return Err(BrowserError::Timeout {
                action: format!("wait_for({})", selector),
                ms: timeout_ms,
            });
        }

        Ok(())
    }
}

/// Raw snapshot data from the JS extraction script.
#[derive(serde::Deserialize)]
struct RawSnapshot {
    elements: Vec<RawElement>,
    page_type: Option<String>,
    main_content_preview: Option<String>,
    description: Option<String>,
    form_count: usize,
    has_login_form: bool,
    has_captcha: bool,
}

#[derive(serde::Deserialize)]
struct RawElement {
    role: String,
    text: Option<String>,
    href: Option<String>,
    placeholder: Option<String>,
    value: Option<String>,
    enabled: bool,
    visible: bool,
    aria_label: Option<String>,
    selector: String,
    bounds: Option<[f64; 4]>,
}

/// JavaScript that extracts interactive elements and page metadata.
/// Runs in the browser context — returns JSON matching RawSnapshot.
const SNAPSHOT_SCRIPT: &str = r#"(() => {
    // Collect interactive elements
    const selectors = 'a, button, input, select, textarea, [role="button"], [role="link"], [role="tab"], [onclick], summary, details, label';
    const allEls = document.querySelectorAll(selectors);
    const elements = [];

    for (const el of allEls) {
        const rect = el.getBoundingClientRect();
        if (rect.width === 0 && rect.height === 0) continue;

        const style = getComputedStyle(el);
        if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') continue;

        const tag = el.tagName.toLowerCase();
        let role = el.getAttribute('role') || tag;
        if (tag === 'a') role = 'link';
        if (tag === 'input') role = el.type === 'submit' ? 'button' : 'input';
        if (tag === 'textarea') role = 'textarea';
        if (tag === 'select') role = 'select';

        const text = (el.textContent || '').trim().substring(0, 100) || null;
        const href = el.href || null;
        const placeholder = el.placeholder || null;
        const value = el.value || null;
        const ariaLabel = el.getAttribute('aria-label') || null;

        // Build a minimal CSS selector for fallback
        let selector = tag;
        if (el.id) selector += '#' + el.id;
        else if (el.className && typeof el.className === 'string') {
            const cls = el.className.trim().split(/\s+/).slice(0, 2).join('.');
            if (cls) selector += '.' + cls;
        }

        elements.push({
            role,
            text,
            href,
            placeholder,
            value,
            enabled: !el.disabled,
            visible: true,
            aria_label: ariaLabel,
            selector,
            bounds: [rect.x, rect.y, rect.width, rect.height],
        });
    }

    // Also grab headings and main text blocks for context
    const headings = document.querySelectorAll('h1, h2, h3');
    for (const h of headings) {
        const rect = h.getBoundingClientRect();
        if (rect.width === 0 && rect.height === 0) continue;
        const text = (h.textContent || '').trim().substring(0, 150);
        if (!text) continue;
        elements.push({
            role: 'heading',
            text,
            href: null,
            placeholder: null,
            value: null,
            enabled: true,
            visible: true,
            aria_label: null,
            selector: h.tagName.toLowerCase(),
            bounds: [rect.x, rect.y, rect.width, rect.height],
        });
    }

    // Page metadata
    const forms = document.querySelectorAll('form');
    const hasPasswordField = !!document.querySelector('input[type="password"]');
    const hasCaptcha = !!(
        document.querySelector('iframe[src*="recaptcha"]') ||
        document.querySelector('iframe[src*="hcaptcha"]') ||
        document.querySelector('.g-recaptcha') ||
        document.querySelector('.h-captcha') ||
        document.querySelector('[data-turnstile-callback]')
    );

    // Detect page type
    let pageType = null;
    const url = location.href.toLowerCase();
    if (hasPasswordField) pageType = 'login';
    else if (url.includes('search') || document.querySelector('input[type="search"]')) pageType = 'search_results';
    else if (document.querySelector('article') || document.querySelector('[role="article"]')) pageType = 'article';
    else if (forms.length > 1) pageType = 'form';

    // Main content preview
    const article = document.querySelector('article, main, [role="main"], .content, #content');
    const preview = article ? (article.textContent || '').trim().substring(0, 500) : null;

    // Meta description
    const metaDesc = document.querySelector('meta[name="description"]');
    const description = metaDesc ? metaDesc.getAttribute('content') : null;

    return JSON.stringify({
        elements,
        page_type: pageType,
        main_content_preview: preview,
        description,
        form_count: forms.length,
        has_login_form: hasPasswordField,
        has_captcha: hasCaptcha,
    });
})()"#;
