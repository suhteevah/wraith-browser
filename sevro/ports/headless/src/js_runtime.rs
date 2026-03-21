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

// SAFETY: JsRuntime is always accessed behind Arc<Mutex<SevroEngine>> which
// guarantees single-threaded access. The Rc/NonNull inside QuickJS are never
// shared across threads — the Mutex serializes all access.
unsafe impl Send for JsRuntime {}
unsafe impl Sync for JsRuntime {}

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

    /// Execute all <script> tags from parsed HTML — both inline AND external.
    /// External scripts (src=) are fetched via the provided HTTP client.
    /// Executes in document order (inline and external interleaved correctly).
    pub fn execute_page_scripts(&self, html: &str) -> Result<usize, String> {
        self.execute_page_scripts_with_fetcher(html, None)
    }

    /// Execute page scripts with optional external script fetching.
    /// If `fetched_scripts` is provided, external scripts are looked up by URL.
    pub fn execute_page_scripts_with_fetcher(
        &self,
        html: &str,
        fetched_scripts: Option<&std::collections::HashMap<String, String>>,
    ) -> Result<usize, String> {
        let doc = scraper::Html::parse_document(html);
        let script_sel = scraper::Selector::parse("script")
            .map_err(|_| "selector parse failed".to_string())?;

        let mut executed = 0;
        for script_el in doc.select(&script_sel) {
            // Skip non-executable script types
            if let Some(script_type) = script_el.value().attr("type") {
                if script_type.contains("json") || script_type.contains("template")
                    || script_type == "text/html"
                {
                    continue;
                }
            }

            if let Some(src) = script_el.value().attr("src") {
                // External script — check if we have it fetched
                if let Some(scripts) = fetched_scripts {
                    if let Some(script_text) = scripts.get(src) {
                        match self.run_script(script_text) {
                            Ok(_) => {
                                executed += 1;
                                debug!(src = %src, len = script_text.len(), "Executed external <script>");
                            }
                            Err(e) => {
                                debug!(src = %src, error = %e, "External script failed (non-fatal)");
                            }
                        }
                    } else {
                        debug!(src = %src, "External script not in cache — skipped");
                    }
                }
                // If no fetcher provided, skip external scripts (backward compat)
            } else {
                // Inline script
                let script_text: String = script_el.text().collect();
                let trimmed = script_text.trim();
                if trimmed.is_empty() {
                    continue;
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
    // Assign ref_ids that match the snapshot: 1-based index over visible elements
    let mut ref_counter = 0u32;

    nodes.iter()
        .filter(|n| n.node_type == DomNodeType::Element)
        .map(|n| {
            let mut obj = serde_json::Map::new();
            obj.insert("nodeId".to_string(), serde_json::json!(n.node_id));
            obj.insert("tag".to_string(), serde_json::json!(n.tag_name));
            obj.insert("textContent".to_string(), serde_json::json!(n.text_content));
            obj.insert("isVisible".to_string(), serde_json::json!(n.is_visible));
            obj.insert("isInteractive".to_string(), serde_json::json!(n.is_interactive));

            // Assign ref_id matching snapshot logic (visible elements get sequential IDs)
            if n.is_visible {
                ref_counter += 1;
                obj.insert("__ref_id".to_string(), serde_json::json!(ref_counter));
            }

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
            if let Some(name) = n.attributes.get("name") {
                obj.insert("name".to_string(), serde_json::json!(name));
            }

            // Parent/child relationships for DOM traversal
            if let Some(parent_id) = n.parent {
                obj.insert("parentId".to_string(), serde_json::json!(parent_id));
            }
            if !n.children.is_empty() {
                obj.insert("childIds".to_string(), serde_json::json!(n.children));
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
                is_interactive: true,
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
                is_interactive: true,
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
                is_interactive: true,
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
    fn html_input_element_prototype_value() {
        let rt = JsRuntime::new().unwrap();
        rt.setup_dom_bridge(&make_test_nodes()).unwrap();

        // Gap #4: HTMLInputElement.prototype must exist
        let result = rt.run_script("typeof window.HTMLInputElement").unwrap();
        assert_eq!(result, "function");

        // The value descriptor must be gettable from the prototype
        let result = rt.run_script(
            "var desc = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value'); typeof desc.set"
        ).unwrap();
        assert_eq!(result, "function");

        // Input nodes should have the value descriptor applied
        let result = rt.run_script(
            "var el = document.getElementById('email-input'); \
             var desc = Object.getOwnPropertyDescriptor(el, 'value') || \
                        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value'); \
             desc.set.call(el, 'test@example.com'); \
             el.value"
        ).unwrap();
        assert_eq!(result, "test@example.com");
    }

    #[test]
    fn react_set_value_helper() {
        let rt = JsRuntime::new().unwrap();
        rt.setup_dom_bridge(&make_test_nodes()).unwrap();

        // __wraith_react_set_value should work without crashing
        let result = rt.run_script(
            "var el = document.getElementById('email-input'); \
             __wraith_react_set_value(el, 'hello@world.com')"
        ).unwrap();
        // Should return 'native_events' since no React fiber exists on test nodes
        assert_eq!(result, "native_events");

        // Value should be set
        let result = rt.run_script(
            "document.getElementById('email-input').value"
        ).unwrap();
        assert_eq!(result, "hello@world.com");
    }

    #[test]
    fn document_forms_collection() {
        let mut nodes = make_test_nodes();
        nodes.push(DomNode {
            node_id: 4,
            node_type: DomNodeType::Element,
            tag_name: "form".to_string(),
            attributes: HashMap::from([("id".to_string(), "login-form".to_string())]),
            text_content: String::new(),
            children: vec![],
            parent: Some(1),
            bounding_box: None,
            is_visible: true,
            is_interactive: true,
        });
        let rt = JsRuntime::new().unwrap();
        rt.setup_dom_bridge(&nodes).unwrap();

        let result = rt.run_script("document.forms.length").unwrap();
        assert_eq!(result, "1");

        let result = rt.run_script("document.forms[0].id").unwrap();
        assert_eq!(result, "login-form");
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
