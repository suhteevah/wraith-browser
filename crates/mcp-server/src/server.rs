//! MCP server handler — implements the rmcp ServerHandler trait.
//! Wired to a real NativeClient for Chrome-free browsing.

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, ListToolsResult,
    ServerCapabilities, Tool, ToolAnnotations,
};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use rmcp::model::ServerInfo;
use rmcp::service::RequestContext;
use schemars::schema_for;
use serde_json::json;
use tokio::sync::Mutex;
use tracing::{info, warn, debug, instrument};

use openclaw_browser_core::NativeClient;
use openclaw_browser_core::actions::{BrowserAction, ActionResult, ScrollDirection};

use crate::tools::*;

/// The Wraith MCP server handler — backed by a real NativeClient.
pub struct WraithHandler {
    tools: Vec<Tool>,
    /// The native browser client (shared, async-mutex for interior mutability)
    browser: Arc<Mutex<NativeClient>>,
}

impl Default for WraithHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl WraithHandler {
    pub fn new() -> Self {
        let rw_open = ToolAnnotations::new().read_only(false).destructive(false).open_world(true);
        let ro_closed = ToolAnnotations::new().read_only(true).open_world(false);
        let ro_open = ToolAnnotations::new().read_only(true).open_world(true);
        let rw_closed = ToolAnnotations::new().read_only(false).destructive(false).open_world(false);
        let rw_destructive = ToolAnnotations::new().read_only(false).destructive(true).open_world(false);

        let tools = vec![
            make_tool("browse_navigate",
                "Navigate to a URL and return a DOM snapshot with interactive elements. Each element has a @ref ID for clicking/filling.",
                &schema_for!(NavigateInput), rw_open.clone()),
            make_tool("browse_click",
                "Click an interactive element by its @ref ID from the latest snapshot. If the element is a link, follows it.",
                &schema_for!(ClickInput), rw_open.clone()),
            make_tool("browse_fill",
                "Fill a form field with text. Use the @ref ID from the snapshot to target the field.",
                &schema_for!(FillInput), rw_open.clone()),
            make_tool("browse_snapshot",
                "Get the current page's DOM snapshot showing all interactive elements with @ref IDs.",
                &schema_for!(SnapshotInput), ro_closed.clone()),
            make_tool("browse_extract",
                "Extract the current page's content as clean markdown optimized for LLM context.",
                &schema_for!(ExtractInput), ro_closed.clone()),
            make_tool("browse_screenshot",
                "Capture a PNG screenshot of the current page. (Requires Chrome backend — not available in native mode.)",
                &schema_for!(ScreenshotInput), ro_closed.clone()),
            make_tool("browse_search",
                "Search the web using metasearch (DuckDuckGo + Brave). Returns titles, URLs, and snippets.",
                &schema_for!(SearchInput), ro_open),
            make_tool("browse_eval_js",
                "Execute JavaScript code on the current page. (Requires Chrome backend — not available in native mode.)",
                &schema_for!(EvalJsInput), rw_destructive),
            make_tool("browse_tabs",
                "Show the current page URL and title.",
                &schema_for!(TabsInput), ro_closed.clone()),
            make_tool("browse_back",
                "Go back to the previous page in browser history.",
                &schema_for!(BackInput), rw_open.clone()),
            make_tool("browse_key_press",
                "Press a keyboard key on the current page.",
                &schema_for!(KeyPressInput), rw_open),
            make_tool("browse_scroll",
                "Scroll the current page up or down. (No-op in native mode — full page is already parsed.)",
                &schema_for!(ScrollInput), rw_closed.clone()),
            make_tool("browse_vault_store",
                "Store a credential (password, API key, token) in the encrypted vault.",
                &schema_for!(VaultStoreInput), rw_closed),
            make_tool("browse_vault_get",
                "Retrieve a credential from the encrypted vault for a given domain.",
                &schema_for!(VaultGetInput), ro_closed),
        ];

        info!(tool_count = tools.len(), "Wraith MCP handler initialized with NativeClient");

        // Create browser and load saved cookies
        let mut browser = NativeClient::new();
        let cookie_path = cookie_file_path();
        if let Err(e) = browser.load_cookies(&cookie_path) {
            warn!(error = %e, "Failed to load saved cookies (starting fresh)");
        }

        Self {
            tools,
            browser: Arc::new(Mutex::new(browser)),
        }
    }

    /// Dispatch a tool call to the real browser.
    async fn dispatch_tool(
        &self,
        name: &str,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let args = serde_json::Value::Object(arguments.unwrap_or_default());

        match name {
            "browse_navigate" => {
                let input: NavigateInput = parse_args(args)?;
                info!(url = %input.url, "Navigating");

                let mut browser = self.browser.lock().await;
                let snapshot = browser.navigate(&input.url).await
                    .map_err(|e| ErrorData::internal_error(format!("Navigation failed: {e}"), None))?;

                // Auto-save cookies after navigation
                let _ = browser.save_cookies(&cookie_file_path());

                let agent_text = snapshot.to_agent_text();
                let needs_js = browser.needs_javascript();

                let mut response = agent_text;
                if needs_js {
                    response.push_str("\n\n⚠️ THIS PAGE REQUIRES JAVASCRIPT — content above may be incomplete or empty.\n");
                    response.push_str("This is a JavaScript-rendered SPA (React/Next.js/Vue). Native mode cannot execute JS.\n\n");
                    response.push_str("ALTERNATIVES:\n");
                    response.push_str("  1. Try the site's API directly if available (many job sites have public APIs)\n");
                    response.push_str("  2. Try a mobile/simplified version of the URL (add ?force_classic=true, m.site.com, etc.)\n");
                    response.push_str("  3. Use browse_search to find the information via web search instead\n");
                    response.push_str("  4. Look for the data in the HTML source — some SPAs embed JSON data in script tags\n");
                }

                Ok(CallToolResult::success(vec![Content::text(response)]))
            }

            "browse_click" => {
                let input: ClickInput = parse_args(args)?;
                info!(ref_id = input.ref_id, "Clicking element");

                let mut browser = self.browser.lock().await;
                let result = browser.execute(BrowserAction::Click { ref_id: input.ref_id }).await
                    .map_err(|e| ErrorData::internal_error(format!("Click failed: {e}"), None))?;

                match result {
                    ActionResult::Navigated { ref url, ref title } => {
                        // After navigation from a click, return the new page snapshot
                        let snapshot = browser.snapshot()
                            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                        Ok(CallToolResult::success(vec![Content::text(snapshot.to_agent_text())]))
                    }
                    _ => {
                        Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
                    }
                }
            }

            "browse_fill" => {
                let input: FillInput = parse_args(args)?;
                info!(ref_id = input.ref_id, text_len = input.text.len(), "Filling field");

                let mut browser = self.browser.lock().await;
                let result = browser.execute(BrowserAction::Fill {
                    ref_id: input.ref_id,
                    text: input.text,
                }).await
                    .map_err(|e| ErrorData::internal_error(format!("Fill failed: {e}"), None))?;

                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            "browse_snapshot" => {
                debug!("Taking DOM snapshot");
                let browser = self.browser.lock().await;
                let snapshot = browser.snapshot()
                    .map_err(|e| ErrorData::internal_error(format!("Snapshot failed: {e}"), None))?;

                Ok(CallToolResult::success(vec![Content::text(snapshot.to_agent_text())]))
            }

            "browse_extract" => {
                let input: ExtractInput = parse_args(args)?;
                info!(max_tokens = ?input.max_tokens, "Extracting content");

                let browser = self.browser.lock().await;
                let html = browser.page_source()
                    .map_err(|e| ErrorData::internal_error(format!("No page loaded: {e}"), None))?;
                let url = browser.current_url().unwrap_or("");

                let result = if let Some(max_tokens) = input.max_tokens {
                    openclaw_content_extract::extract_budgeted(html, url, max_tokens)
                } else {
                    openclaw_content_extract::extract(html, url)
                };

                match result {
                    Ok(content) => {
                        Ok(CallToolResult::success(vec![Content::text(
                            format!("# {}\n\n{}\n\n---\n{} links | ~{} tokens",
                                content.title,
                                content.markdown,
                                content.links.len(),
                                content.estimated_tokens,
                            )
                        )]))
                    }
                    Err(e) => {
                        Ok(CallToolResult::success(vec![Content::text(
                            format!("Content extraction failed: {e}")
                        )]))
                    }
                }
            }

            "browse_screenshot" => {
                Ok(CallToolResult::success(vec![Content::text(
                    "Screenshots are not available in native mode (no Chrome). \
                     Use browse_snapshot for a text representation of the page, \
                     or browse_extract for markdown content."
                )]))
            }

            "browse_search" => {
                let input: SearchInput = parse_args(args)?;
                let max = input.max_results.unwrap_or(10);
                info!(query = %input.query, max_results = max, "Searching web");

                let results = openclaw_search::search(&input.query, max).await
                    .map_err(|e| ErrorData::internal_error(format!("Search failed: {e}"), None))?;

                if results.is_empty() {
                    return Ok(CallToolResult::success(vec![Content::text(
                        format!("No results found for: {}", input.query)
                    )]));
                }

                let mut output = format!("Search results for: {}\n\n", input.query);
                for (i, r) in results.iter().enumerate() {
                    output.push_str(&format!(
                        "{}. **{}**\n   {}\n   {}\n\n",
                        i + 1, r.title, r.url, r.snippet
                    ));
                }

                Ok(CallToolResult::success(vec![Content::text(output)]))
            }

            "browse_eval_js" => {
                let _input: EvalJsInput = parse_args(args)?;
                Ok(CallToolResult::success(vec![Content::text(
                    "JavaScript execution is not available in native mode (no Chrome engine). \
                     Native mode fetches and parses HTML directly — most pages work without JS."
                )]))
            }

            "browse_tabs" => {
                let browser = self.browser.lock().await;
                let url = browser.current_url().unwrap_or("(no page loaded)");
                let title = browser.snapshot()
                    .map(|s| s.title.clone())
                    .unwrap_or_default();

                Ok(CallToolResult::success(vec![Content::text(
                    json!({
                        "current_tab": {
                            "url": url,
                            "title": title,
                        }
                    }).to_string()
                )]))
            }

            "browse_back" => {
                let mut browser = self.browser.lock().await;
                let result = browser.execute(BrowserAction::GoBack).await
                    .map_err(|e| ErrorData::internal_error(format!("Back failed: {e}"), None))?;

                match result {
                    ActionResult::Navigated { .. } => {
                        let snapshot = browser.snapshot()
                            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                        Ok(CallToolResult::success(vec![Content::text(snapshot.to_agent_text())]))
                    }
                    _ => Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
                }
            }

            "browse_key_press" => {
                let input: KeyPressInput = parse_args(args)?;
                // In native mode, Enter on a form triggers submit
                if input.key.eq_ignore_ascii_case("enter") {
                    let mut browser = self.browser.lock().await;
                    // Get the current snapshot and find a submit button or regular button
                    if let Ok(snapshot) = browser.snapshot() {
                        let submit_el = snapshot.elements.iter().find(|el| {
                            el.role == "submit" || el.role == "button"
                        });
                        if let Some(el) = submit_el {
                            let ref_id = el.ref_id;
                            let result = browser.execute(BrowserAction::Click { ref_id }).await;
                            match result {
                                Ok(r) => return Ok(CallToolResult::success(vec![Content::text(format_action_result(&r))])),
                                Err(_) => {}
                            }
                        } else {
                            return Ok(CallToolResult::success(vec![Content::text(
                                "No submit button found on page".to_string()
                            )]));
                        }
                    }
                }
                Ok(CallToolResult::success(vec![Content::text(
                    format!("Key press '{}' acknowledged (limited in native mode)", input.key)
                )]))
            }

            "browse_scroll" => {
                let input: ScrollInput = parse_args(args)?;
                Ok(CallToolResult::success(vec![Content::text(
                    "Scroll acknowledged. In native mode, the entire page is already parsed — \
                     use browse_snapshot to see all elements or browse_extract for full content."
                )]))
            }

            "browse_vault_store" | "browse_vault_get" => {
                Ok(CallToolResult::success(vec![Content::text(
                    "Vault operations are not yet wired in MCP mode. \
                     Use the CLI: wraith-browser vault store/list"
                )]))
            }

            _ => {
                warn!(tool = %name, "Unknown tool");
                Err(ErrorData::invalid_params(format!("Unknown tool: {name}"), None))
            }
        }
    }
}

impl ServerHandler for WraithHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_instructions(
            "Wraith Browser — an AI-agent-first web browser built in Rust. \
             Pure native mode: no Chrome dependency, ~50ms per page. \
             Use browse_navigate to visit URLs, browse_click/browse_fill to interact \
             with elements using @ref IDs, browse_snapshot to see the page state, \
             browse_extract to get markdown content, and browse_search to search the web."
        )
    }

    fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult {
            tools: self.tools.clone(),
            next_cursor: None,
            meta: None,
        }))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, ErrorData>> + Send + '_ {
        let name = request.name.clone();
        let arguments = request.arguments.clone();
        async move { self.dispatch_tool(&name, arguments).await }
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.iter().find(|t| t.name == name).cloned()
    }
}

/// Parse JSON args into a typed input, returning ErrorData on failure.
fn parse_args<T: serde::de::DeserializeOwned>(args: serde_json::Value) -> Result<T, ErrorData> {
    serde_json::from_value(args)
        .map_err(|e| ErrorData::invalid_params(e.to_string(), None))
}

/// Format an ActionResult as a human-readable string.
fn format_action_result(result: &ActionResult) -> String {
    match result {
        ActionResult::Success { message } => message.clone(),
        ActionResult::Navigated { url, title } => format!("Navigated to: {title}\nURL: {url}"),
        ActionResult::Screenshot { .. } => "Screenshot captured".to_string(),
        ActionResult::Content { markdown, word_count } => {
            format!("{markdown}\n\n[{word_count} words]")
        }
        ActionResult::JsResult { value } => format!("JS result: {value}"),
        ActionResult::Failed { error } => format!("Failed: {error}"),
    }
}

/// Helper to create a Tool definition from a schemars-generated schema.
fn make_tool(
    name: &'static str,
    description: &'static str,
    schema: &schemars::schema::RootSchema,
    annotations: ToolAnnotations,
) -> Tool {
    let schema_value = serde_json::to_value(schema).unwrap_or(json!({}));
    let input_schema: Arc<serde_json::Map<String, serde_json::Value>> = match schema_value {
        serde_json::Value::Object(map) => Arc::new(map),
        _ => Arc::new(serde_json::Map::new()),
    };

    Tool::new(name, description, input_schema)
        .with_annotations(annotations)
}

/// Get the path for cookie persistence.
fn cookie_file_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("wraith-browser")
        .join("cookies.json")
}
