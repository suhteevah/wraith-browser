//! # JavaScript Runtime (QuickJS via rquickjs)
//!
//! Embeds QuickJS to execute JavaScript against the DOM tree.
//! Bridges DOM operations from JS back to Rust:
//!
//! - `document.querySelector(sel)` → scraper CSS selector on our DOM
//! - `document.getElementById(id)` → attribute lookup in dom_nodes
//! - `element.textContent` → DomNode.text_content
//! - `element.getAttribute(name)` → DomNode.attributes
//! - `element.innerHTML` → reconstruct from children
//! - `console.log(msg)` → tracing::info!
//!
//! Note: This module intentionally uses QuickJS's eval capabilities to
//! execute page scripts — this is the core purpose of a JS engine.
//! The runtime is sandboxed with memory and stack limits.

use rquickjs::{Context, Runtime, Function, Value, Object};
use tracing::{debug, info, warn};

use crate::{DomNode, DomNodeType};

/// The JS execution context — wraps a QuickJS runtime.
pub struct JsRuntime {
    #[allow(dead_code)]
    runtime: Runtime,
    context: Context,
}

impl JsRuntime {
    /// Create a new JS runtime with sandboxing limits.
    pub fn new() -> Result<Self, String> {
        let runtime = Runtime::new()
            .map_err(|e| format!("QuickJS runtime creation failed: {e}"))?;

        // Sandbox limits
        runtime.set_max_stack_size(1024 * 1024); // 1MB stack
        runtime.set_memory_limit(50 * 1024 * 1024); // 50MB heap

        let context = Context::full(&runtime)
            .map_err(|e| format!("QuickJS context creation failed: {e}"))?;

        Ok(Self { runtime, context })
    }

    /// Register the DOM bridge APIs into the JS global scope.
    /// Call this after parsing a page, passing the extracted DOM nodes.
    pub fn setup_dom_bridge(&self, dom_nodes: &[DomNode]) -> Result<(), String> {
        self.context.with(|ctx| {
            let globals = ctx.globals();

            // === console.log ===
            let console = Object::new(ctx.clone())
                .map_err(|e| format!("console object creation failed: {e}"))?;

            console.set("log", Function::new(ctx.clone(), |msg: String| {
                info!(target: "js_console", "{}", msg);
            }).map_err(|e| format!("console.log failed: {e}"))?)
                .map_err(|e| format!("console.log set failed: {e}"))?;

            console.set("warn", Function::new(ctx.clone(), |msg: String| {
                warn!(target: "js_console", "{}", msg);
            }).map_err(|e| format!("console.warn failed: {e}"))?)
                .map_err(|e| format!("console.warn set failed: {e}"))?;

            console.set("error", Function::new(ctx.clone(), |msg: String| {
                warn!(target: "js_console", "ERROR: {}", msg);
            }).map_err(|e| format!("console.error failed: {e}"))?)
                .map_err(|e| format!("console.error set failed: {e}"))?;

            globals.set("console", console)
                .map_err(|e| format!("globals.console failed: {e}"))?;

            // === Build DOM node index for JS queries ===
            let node_data = build_node_json(dom_nodes);
            let node_json = serde_json::to_string(&node_data)
                .map_err(|e| format!("JSON serialize failed: {e}"))?;

            let title = dom_nodes.iter()
                .find(|n| n.tag_name == "title")
                .map(|n| n.text_content.as_str())
                .unwrap_or("");

            // Inject DOM data and bridge functions as JS
            // NOTE: This is intentional JS execution — the entire purpose of
            // this module is to run JavaScript against a DOM tree.
            let bridge_script = include_str!("dom_bridge.js")
                .replace("{node_json}", &node_json)
                .replace("{title}", &title.replace('"', r#"\""#));

            ctx.eval::<(), _>(bridge_script.as_bytes())
                .map_err(|e| format!("DOM bridge injection failed: {e}"))?;

            debug!(nodes = dom_nodes.len(), "DOM bridge initialized in QuickJS");
            Ok(())
        })
    }

    /// Execute a JavaScript string and return the result as a string.
    /// This is the core JS engine function — intentionally executes code.
    pub fn run_script(&self, script: &str) -> Result<String, String> {
        self.context.with(|ctx| {
            let result: Value = ctx.eval(script.as_bytes())
                .map_err(|e| format!("JS execution failed: {e}"))?;

            // Convert result to string
            let s = if result.is_undefined() || result.is_null() {
                "undefined".to_string()
            } else if let Some(s) = result.as_string() {
                s.to_string().unwrap_or_default()
            } else if let Some(b) = result.as_bool() {
                b.to_string()
            } else if let Some(i) = result.as_int() {
                i.to_string()
            } else if let Some(f) = result.as_float() {
                f.to_string()
            } else {
                "[object]".to_string()
            };

            Ok(s)
        })
    }

    /// Execute all inline <script> tags from parsed HTML.
    /// Skips external scripts (src=), JSON-LD, and template scripts.
    pub fn execute_page_scripts(&self, html: &str) -> Result<usize, String> {
        let doc = scraper::Html::parse_document(html);
        let script_sel = scraper::Selector::parse("script:not([src])")
            .map_err(|_| "selector parse failed".to_string())?;

        let mut executed = 0;
        for script_el in doc.select(&script_sel) {
            let script_text: String = script_el.text().collect();
            let trimmed = script_text.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Skip non-executable script types
            if let Some(script_type) = script_el.value().attr("type") {
                if script_type.contains("json") || script_type.contains("template")
                    || script_type == "text/html"
                {
                    continue;
                }
            }

            match self.run_script(trimmed) {
                Ok(_) => {
                    executed += 1;
                    debug!(script_len = trimmed.len(), "Executed inline <script>");
                }
                Err(e) => {
                    debug!(error = %e, "Script execution failed (non-fatal)");
                }
            }
        }

        // Flush pending timers (setTimeout callbacks from scripts)
        if executed > 0 {
            match self.run_script("__wraith_flush_timers()") {
                Ok(_) => debug!("Timer flush complete"),
                Err(e) => debug!(error = %e, "Timer flush failed (non-fatal)"),
            }
        }

        info!(executed, "Inline scripts processed");
        Ok(executed)
    }
}

impl Default for JsRuntime {
    fn default() -> Self {
        Self::new().expect("QuickJS runtime creation failed")
    }
}

/// Convert DomNodes to a JSON-friendly format for injection into JS.
fn build_node_json(nodes: &[DomNode]) -> Vec<serde_json::Value> {
    nodes.iter()
        .filter(|n| n.node_type == DomNodeType::Element)
        .map(|n| {
            let mut obj = serde_json::Map::new();
            obj.insert("nodeId".to_string(), serde_json::json!(n.node_id));
            obj.insert("tag".to_string(), serde_json::json!(n.tag_name));
            obj.insert("textContent".to_string(), serde_json::json!(n.text_content));

            if let Some(id) = n.attributes.get("id") {
                obj.insert("id".to_string(), serde_json::json!(id));
            }
            if let Some(class) = n.attributes.get("class") {
                obj.insert("className".to_string(), serde_json::json!(class));
            }
            if let Some(href) = n.attributes.get("href") {
                obj.insert("href".to_string(), serde_json::json!(href));
            }
            if let Some(value) = n.attributes.get("value") {
                obj.insert("value".to_string(), serde_json::json!(value));
            }

            let attrs: serde_json::Map<String, serde_json::Value> = n.attributes.iter()
                .map(|(k, v)| (k.clone(), serde_json::json!(v)))
                .collect();
            obj.insert("attrs".to_string(), serde_json::Value::Object(attrs));

            serde_json::Value::Object(obj)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_test_nodes() -> Vec<DomNode> {
        vec![
            DomNode {
                node_id: 1,
                node_type: DomNodeType::Element,
                tag_name: "div".to_string(),
                attributes: HashMap::from([
                    ("id".to_string(), "app".to_string()),
                    ("class".to_string(), "container main".to_string()),
                ]),
                text_content: "Hello World".to_string(),
                children: vec![2, 3],
                parent: None,
                bounding_box: None,
                is_visible: true,
            },
            DomNode {
                node_id: 2,
                node_type: DomNodeType::Element,
                tag_name: "a".to_string(),
                attributes: HashMap::from([
                    ("href".to_string(), "/about".to_string()),
                ]),
                text_content: "About".to_string(),
                children: vec![],
                parent: Some(1),
                bounding_box: None,
                is_visible: true,
            },
            DomNode {
                node_id: 3,
                node_type: DomNodeType::Element,
                tag_name: "input".to_string(),
                attributes: HashMap::from([
                    ("type".to_string(), "text".to_string()),
                    ("name".to_string(), "email".to_string()),
                    ("id".to_string(), "email-input".to_string()),
                ]),
                text_content: String::new(),
                children: vec![],
                parent: Some(1),
                bounding_box: None,
                is_visible: true,
            },
        ]
    }

    #[test]
    fn runtime_creates_and_evaluates() {
        let rt = JsRuntime::new().unwrap();
        assert_eq!(rt.run_script("1 + 2").unwrap(), "3");
    }

    #[test]
    fn eval_strings() {
        let rt = JsRuntime::new().unwrap();
        assert_eq!(rt.run_script("'hello'").unwrap(), "hello");
    }

    #[test]
    fn eval_boolean() {
        let rt = JsRuntime::new().unwrap();
        assert_eq!(rt.run_script("true").unwrap(), "true");
    }

    #[test]
    fn dom_bridge_query_by_id() {
        let rt = JsRuntime::new().unwrap();
        rt.setup_dom_bridge(&make_test_nodes()).unwrap();
        let result = rt.run_script("document.getElementById('app').textContent").unwrap();
        assert_eq!(result, "Hello World");
    }

    #[test]
    fn dom_bridge_query_by_tag() {
        let rt = JsRuntime::new().unwrap();
        rt.setup_dom_bridge(&make_test_nodes()).unwrap();
        let result = rt.run_script("document.querySelector('a').textContent").unwrap();
        assert_eq!(result, "About");
    }

    #[test]
    fn dom_bridge_query_by_css_id() {
        let rt = JsRuntime::new().unwrap();
        rt.setup_dom_bridge(&make_test_nodes()).unwrap();
        let result = rt.run_script("document.querySelector('#app').tag").unwrap();
        assert_eq!(result, "div");
    }

    #[test]
    fn dom_bridge_get_elements_by_tag() {
        let rt = JsRuntime::new().unwrap();
        rt.setup_dom_bridge(&make_test_nodes()).unwrap();
        let result = rt.run_script("document.getElementsByTagName('input').length").unwrap();
        assert_eq!(result, "1");
    }

    #[test]
    fn dom_bridge_navigator() {
        let rt = JsRuntime::new().unwrap();
        rt.setup_dom_bridge(&make_test_nodes()).unwrap();
        let result = rt.run_script("navigator.language").unwrap();
        assert_eq!(result, "en-US");
    }

    #[test]
    fn console_log_works() {
        let rt = JsRuntime::new().unwrap();
        rt.setup_dom_bridge(&make_test_nodes()).unwrap();
        rt.run_script("console.log('test message')").unwrap();
    }

    #[test]
    fn modern_js_syntax() {
        let rt = JsRuntime::new().unwrap();
        assert_eq!(rt.run_script("const x = 42; x").unwrap(), "42");
        assert_eq!(rt.run_script("let arr = [1,2,3]; arr.map(x => x*2).join(',')").unwrap(), "2,4,6");
        assert_eq!(rt.run_script("const [a, ...rest] = [1,2,3]; rest.length").unwrap(), "2");
    }

    #[test]
    fn execute_inline_scripts() {
        let rt = JsRuntime::new().unwrap();
        rt.setup_dom_bridge(&[]).unwrap();

        let html = r#"
            <html><body>
                <script>var x = 42;</script>
                <script type="application/ld+json">{"@type":"WebPage"}</script>
                <script>var y = x + 1;</script>
                <script src="external.js"></script>
            </body></html>
        "#;

        let count = rt.execute_page_scripts(html).unwrap();
        assert_eq!(count, 2, "Should execute 2 inline scripts");
        assert_eq!(rt.run_script("y").unwrap(), "43");
    }
}
