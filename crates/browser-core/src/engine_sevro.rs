//! BrowserEngine implementation wrapping the Sevro headless engine.
//!
//! Feature-gated behind `sevro`. This is the future default engine —
//! full DOM, CSS layout, and SpiderMonkey JS without Chrome.

use crate::dom::{DomSnapshot, DomElement, PageMeta};
use crate::actions::{BrowserAction, ActionResult};
use crate::engine::{BrowserEngine, EngineCapabilities, ScreenshotCapability};
use crate::error::{BrowserResult, BrowserError};
use async_trait::async_trait;
use tracing::{info, debug, instrument};

/// Browser engine backed by Sevro (stripped Servo fork).
///
/// Includes an integrated Rhai scripting engine that triggers userscripts
/// on navigation events (OnNavigate, Always triggers).
pub struct SevroEngineBackend {
    engine: sevro_headless::SevroEngine,
    /// Rhai scripting engine for userscripts
    scripts: openclaw_scripting::ScriptEngine,
}

impl SevroEngineBackend {
    pub fn new() -> Self {
        Self {
            engine: sevro_headless::SevroEngine::default(),
            scripts: openclaw_scripting::ScriptEngine::new(),
        }
    }

    pub fn with_config(config: sevro_headless::SevroConfig) -> Self {
        Self {
            engine: sevro_headless::SevroEngine::new(config),
            scripts: openclaw_scripting::ScriptEngine::new(),
        }
    }

    /// Access the scripting engine to load/manage Rhai scripts.
    pub fn scripting(&mut self) -> &mut openclaw_scripting::ScriptEngine {
        &mut self.scripts
    }

    /// Run triggered scripts for the current page.
    fn run_page_scripts(&self, url: &str, title: &str) {
        let context = openclaw_scripting::ScriptContext {
            url: url.to_string(),
            domain: url::Url::parse(url)
                .map(|u| u.host_str().unwrap_or("").to_string())
                .unwrap_or_default(),
            title: title.to_string(),
            html: self.engine.page_source().unwrap_or("").to_string(),
            text_content: String::new(),
            links: Vec::new(),
            custom_vars: std::collections::HashMap::new(),
        };

        let trigger = openclaw_scripting::ScriptTrigger::OnNavigate {
            url_pattern: url.to_string(),
        };

        let results = self.scripts.run_triggered(&trigger, &context);
        for (name, result) in &results {
            match result {
                Ok(r) if r.success => {
                    debug!(script = %name, output = ?r.output, "Script executed successfully");
                }
                Ok(_) => {
                    debug!(script = %name, "Script completed with failure status");
                }
                Err(e) => {
                    debug!(script = %name, error = %e, "Script execution error");
                }
            }
        }
        if !results.is_empty() {
            info!(count = results.len(), url = %url, "Triggered scripts executed");
        }
    }
}

impl Default for SevroEngineBackend {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl BrowserEngine for SevroEngineBackend {
    #[instrument(skip(self), fields(url = %url))]
    async fn navigate(&mut self, url: &str) -> BrowserResult<()> {
        match self.engine.navigate(url).await {
            Ok(sevro_headless::PageEvent::Error(e)) => Err(BrowserError::NavigationFailed {
                url: url.to_string(),
                reason: e,
            }),
            Ok(sevro_headless::PageEvent::Cancelled) => Err(BrowserError::NavigationFailed {
                url: url.to_string(),
                reason: "Cancelled".to_string(),
            }),
            Ok(_) => {
                // Run any Rhai scripts triggered by this URL
                let title = self.engine.current_url().unwrap_or("").to_string();
                self.run_page_scripts(url, &title);
                Ok(())
            }
            Err(e) => Err(BrowserError::NavigationFailed {
                url: url.to_string(),
                reason: e,
            }),
        }
    }

    async fn snapshot(&self) -> BrowserResult<DomSnapshot> {
        let sevro_nodes = self.engine.dom_snapshot_with_layout();

        let elements: Vec<DomElement> = sevro_nodes.iter()
            .filter(|n| n.node_type == sevro_headless::DomNodeType::Element && n.is_visible)
            .enumerate()
            .map(|(i, node)| {
                let role = match node.tag_name.as_str() {
                    "a" => "link".to_string(),
                    "button" => "button".to_string(),
                    "input" => node.attributes.get("type")
                        .cloned()
                        .unwrap_or_else(|| "textbox".to_string()),
                    "select" => "combobox".to_string(),
                    "textarea" => "textbox".to_string(),
                    other => other.to_string(),
                };

                DomElement {
                    ref_id: (i + 1) as u32,
                    role,
                    text: if node.text_content.is_empty() { None } else { Some(node.text_content.clone()) },
                    href: node.attributes.get("href").cloned(),
                    placeholder: node.attributes.get("placeholder").cloned(),
                    value: node.attributes.get("value").cloned(),
                    enabled: true,
                    visible: node.is_visible,
                    aria_label: node.attributes.get("aria-label").cloned(),
                    selector: format!("{}", node.tag_name),
                    bounds: node.bounding_box.map(|b| (b.x, b.y, b.width, b.height)),
                }
            })
            .collect();

        let url = self.engine.current_url().unwrap_or("").to_string();
        let title = sevro_nodes.iter()
            .find(|n| n.tag_name == "title")
            .map(|n| n.text_content.clone())
            .unwrap_or_default();

        Ok(DomSnapshot {
            url,
            title,
            elements,
            meta: PageMeta {
                page_type: None,
                main_content_preview: None,
                description: None,
                form_count: 0,
                has_login_form: false,
                has_captcha: false,
                interactive_element_count: 0,
            },
            timestamp: chrono::Utc::now(),
        })
    }

    async fn execute_action(&mut self, action: BrowserAction) -> BrowserResult<ActionResult> {
        match action {
            BrowserAction::Navigate { url } => {
                self.navigate(&url).await?;
                Ok(ActionResult::Navigated { url, title: String::new() })
            }
            BrowserAction::Click { ref_id } => {
                self.engine.click_element(ref_id as u64);
                Ok(ActionResult::Success { message: format!("Clicked @e{}", ref_id) })
            }
            BrowserAction::Fill { ref_id, text } => {
                self.engine.fill_element(ref_id as u64, &text);
                Ok(ActionResult::Success { message: format!("Filled @e{}", ref_id) })
            }
            BrowserAction::EvalJs { script } => {
                match self.engine.eval_js(&script).await {
                    Ok(result) => Ok(ActionResult::JsResult { value: result }),
                    Err(e) => Ok(ActionResult::Failed { error: e }),
                }
            }
            BrowserAction::Screenshot { .. } => {
                Err(BrowserError::ScreenshotFailed("Not available in Sevro (Phase 3)".to_string()))
            }
            _ => {
                Ok(ActionResult::Success { message: "Action acknowledged (Sevro stub)".to_string() })
            }
        }
    }

    async fn eval_js(&self, script: &str) -> BrowserResult<String> {
        self.engine.eval_js(script).await
            .map_err(BrowserError::JsEvalFailed)
    }

    async fn page_source(&self) -> BrowserResult<String> {
        self.engine.page_source()
            .map(|s| s.to_string())
            .ok_or_else(|| BrowserError::EngineError("No page loaded".to_string()))
    }

    async fn current_url(&self) -> Option<String> {
        self.engine.current_url().map(|s| s.to_string())
    }

    async fn screenshot(&self) -> BrowserResult<Vec<u8>> {
        Err(BrowserError::ScreenshotFailed("Not available in Sevro (Phase 3)".to_string()))
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            javascript: self.engine.config().enable_javascript,
            screenshots: ScreenshotCapability::None,
            layout: true,
            cookies: true,
            stealth: true,
        }
    }

    async fn shutdown(&mut self) -> BrowserResult<()> {
        info!("Shutting down Sevro engine");
        self.engine.shutdown();
        Ok(())
    }
}
