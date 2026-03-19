//! MCP server handler — implements the rmcp ServerHandler trait.
//! Wired to a real NativeClient for Chrome-free browsing.

use std::sync::Arc;
use base64::Engine as _;

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
use tracing::{info, warn, debug};

use openclaw_browser_core::engine::BrowserEngine;
use openclaw_browser_core::actions::{BrowserAction, ActionResult};

use crate::tools::*;

/// The Wraith MCP server handler — backed by any BrowserEngine.
pub struct WraithHandler {
    tools: Vec<Tool>,
    /// The browser engine (shared, async-mutex for interior mutability)
    engine: Arc<Mutex<dyn BrowserEngine>>,
}

impl Default for WraithHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl WraithHandler {
    /// Create the handler with the default engine (Sevro if available, native fallback).
    pub fn new() -> Self {
        Self::with_engine(Self::default_engine())
    }

    /// Create the handler with a specific engine.
    pub fn with_engine(engine: Arc<Mutex<dyn BrowserEngine>>) -> Self {
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
                "Capture a PNG screenshot of the current page. Returns base64-encoded PNG.",
                &schema_for!(ScreenshotInput), ro_closed.clone()),
            make_tool("browse_search",
                "Search the web using metasearch (DuckDuckGo + Brave). Returns titles, URLs, and snippets.",
                &schema_for!(SearchInput), ro_open),
            make_tool("browse_eval_js",
                "Execute JavaScript code on the current page and return the result.",
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
                "Scroll the current page up or down.",
                &schema_for!(ScrollInput), rw_closed.clone()),
            make_tool("browse_vault_store",
                "Store a credential (password, API key, token) in the encrypted vault.",
                &schema_for!(VaultStoreInput), rw_closed),
            make_tool("browse_vault_get",
                "Retrieve a credential from the encrypted vault for a given domain.",
                &schema_for!(VaultGetInput), ro_closed),
        ];

        info!(tool_count = tools.len(), "Wraith MCP handler initialized");
        Self { tools, engine }
    }

    /// Build the default engine: Sevro → NativeEngine fallback.
    fn default_engine() -> Arc<Mutex<dyn BrowserEngine>> {
        #[cfg(feature = "sevro")]
        {
            info!("Using Sevro engine (default)");
            return Arc::new(Mutex::new(
                openclaw_browser_core::engine_sevro::SevroEngineBackend::new()
            ));
        }
        #[cfg(not(feature = "sevro"))]
        {
            info!("Sevro not available, using native engine");
            Arc::new(Mutex::new(
                openclaw_browser_core::engine_native::NativeEngine::new()
            ))
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

                let mut engine = self.engine.lock().await;
                engine.navigate(&input.url).await
                    .map_err(|e| ErrorData::internal_error(format!("Navigation failed: {e}"), None))?;

                let snapshot = engine.snapshot().await
                    .map_err(|e| ErrorData::internal_error(format!("Snapshot failed: {e}"), None))?;

                let response = snapshot.to_agent_text();
                Ok(CallToolResult::success(vec![Content::text(response)]))
            }

            "browse_click" => {
                let input: ClickInput = parse_args(args)?;
                info!(ref_id = input.ref_id, "Clicking element");

                let mut engine = self.engine.lock().await;
                let result = engine.execute_action(BrowserAction::Click { ref_id: input.ref_id }).await
                    .map_err(|e| ErrorData::internal_error(format!("Click failed: {e}"), None))?;

                match result {
                    ActionResult::Navigated { url: _, title: _ } => {
                        // After navigation from a click, return the new page snapshot
                        let snapshot = engine.snapshot().await
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

                let mut engine = self.engine.lock().await;
                let result = engine.execute_action(BrowserAction::Fill {
                    ref_id: input.ref_id,
                    text: input.text,
                }).await
                    .map_err(|e| ErrorData::internal_error(format!("Fill failed: {e}"), None))?;

                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            "browse_snapshot" => {
                debug!("Taking DOM snapshot");
                let engine = self.engine.lock().await;
                let snapshot = engine.snapshot().await
                    .map_err(|e| ErrorData::internal_error(format!("Snapshot failed: {e}"), None))?;

                Ok(CallToolResult::success(vec![Content::text(snapshot.to_agent_text())]))
            }

            "browse_extract" => {
                let input: ExtractInput = parse_args(args)?;
                info!(max_tokens = ?input.max_tokens, "Extracting content");

                let engine = self.engine.lock().await;
                let html = engine.page_source().await
                    .map_err(|e| ErrorData::internal_error(format!("No page loaded: {e}"), None))?;
                let url = engine.current_url().await.unwrap_or_default();

                let result = if let Some(max_tokens) = input.max_tokens {
                    openclaw_content_extract::extract_budgeted(&html, &url, max_tokens)
                } else {
                    openclaw_content_extract::extract(&html, &url)
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
                let engine = self.engine.lock().await;
                match engine.screenshot().await {
                    Ok(png_bytes) => {
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
                        let (width, height) = if png_bytes.len() >= 24 {
                            let w = u32::from_be_bytes([png_bytes[16], png_bytes[17], png_bytes[18], png_bytes[19]]);
                            let h = u32::from_be_bytes([png_bytes[20], png_bytes[21], png_bytes[22], png_bytes[23]]);
                            (w, h)
                        } else {
                            (0, 0)
                        };
                        Ok(CallToolResult::success(vec![Content::text(
                            format!("Screenshot captured ({}x{}, {} bytes base64)", width, height, b64.len())
                        )]))
                    }
                    Err(e) => {
                        Ok(CallToolResult::success(vec![Content::text(
                            format!("Screenshot not available with current engine: {e}")
                        )]))
                    }
                }
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
                let input: EvalJsInput = parse_args(args)?;
                info!(script_len = input.code.len(), "Evaluating JavaScript");

                let engine = self.engine.lock().await;
                match engine.eval_js(&input.code).await {
                    Ok(result) => {
                        Ok(CallToolResult::success(vec![Content::text(
                            format!("JS result: {result}")
                        )]))
                    }
                    Err(e) => {
                        Ok(CallToolResult::success(vec![Content::text(
                            format!("JavaScript execution failed: {e}")
                        )]))
                    }
                }
            }

            "browse_tabs" => {
                let engine = self.engine.lock().await;
                let url = engine.current_url().await.unwrap_or_else(|| "(no page loaded)".to_string());
                let title = engine.snapshot().await
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
                let mut engine = self.engine.lock().await;
                let result = engine.execute_action(BrowserAction::GoBack).await
                    .map_err(|e| ErrorData::internal_error(format!("Back failed: {e}"), None))?;

                match result {
                    ActionResult::Navigated { .. } => {
                        let snapshot = engine.snapshot().await
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
                    let mut engine = self.engine.lock().await;
                    // Get the current snapshot and find a submit button or regular button
                    if let Ok(snapshot) = engine.snapshot().await {
                        let submit_el = snapshot.elements.iter().find(|el| {
                            el.role == "submit" || el.role == "button"
                        });
                        if let Some(el) = submit_el {
                            let ref_id = el.ref_id;
                            let result = engine.execute_action(BrowserAction::Click { ref_id }).await;
                            if let Ok(r) = result {
                                return Ok(CallToolResult::success(vec![Content::text(format_action_result(&r))]));
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
                let direction = match input.direction.to_lowercase().as_str() {
                    "up" => openclaw_browser_core::actions::ScrollDirection::Up,
                    "left" => openclaw_browser_core::actions::ScrollDirection::Left,
                    "right" => openclaw_browser_core::actions::ScrollDirection::Right,
                    _ => openclaw_browser_core::actions::ScrollDirection::Down,
                };
                let amount = input.amount.unwrap_or(500);

                let mut engine = self.engine.lock().await;
                let result = engine.execute_action(BrowserAction::Scroll { direction, amount }).await
                    .map_err(|e| ErrorData::internal_error(format!("Scroll failed: {e}"), None))?;

                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            "browse_vault_store" => {
                let input: VaultStoreInput = parse_args(args)?;
                info!(domain = %input.domain, kind = %input.kind, "Storing credential");

                let vault_path = dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".openclaw")
                    .join("vault.db");

                let vault = openclaw_identity::CredentialVault::open(&vault_path)
                    .map_err(|e| ErrorData::internal_error(format!("Vault open failed: {e}"), None))?;

                // Auto-unlock with empty passphrase for MCP mode
                // (the vault creates itself on first use)
                let _ = vault.unlock(&secrecy::SecretString::from("".to_string()));

                let kind = match input.kind.to_lowercase().as_str() {
                    "password" => openclaw_identity::CredentialKind::Password,
                    "api_key" | "apikey" => openclaw_identity::CredentialKind::ApiKey,
                    "oauth_token" | "oauth" => openclaw_identity::CredentialKind::OAuthToken,
                    "totp_seed" | "totp" => openclaw_identity::CredentialKind::TotpSeed,
                    "session_cookie" | "cookie" => openclaw_identity::CredentialKind::SessionCookie,
                    _ => openclaw_identity::CredentialKind::Generic,
                };

                let request = openclaw_identity::credential::StoreCredentialRequest {
                    domain: input.domain.clone(),
                    kind,
                    identity: input.identity.clone(),
                    secret: secrecy::SecretString::from(input.secret),
                    label: None,
                    url_pattern: None,
                    auto_use: true,
                    metadata: serde_json::Value::Object(serde_json::Map::new()),
                };

                match vault.store(request) {
                    Ok(id) => {
                        Ok(CallToolResult::success(vec![Content::text(
                            format!("Credential stored: {} ({}@{}, {:?})", id, input.identity, input.domain, kind)
                        )]))
                    }
                    Err(e) => {
                        Ok(CallToolResult::success(vec![Content::text(
                            format!("Vault store failed: {e}")
                        )]))
                    }
                }
            }

            "browse_vault_get" => {
                let input: VaultGetInput = parse_args(args)?;
                info!(domain = %input.domain, "Retrieving credential");

                let vault_path = dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".openclaw")
                    .join("vault.db");

                let vault = openclaw_identity::CredentialVault::open(&vault_path)
                    .map_err(|e| ErrorData::internal_error(format!("Vault open failed: {e}"), None))?;

                let _ = vault.unlock(&secrecy::SecretString::from("".to_string()));

                let kind = input.kind.as_deref().map(|k| match k.to_lowercase().as_str() {
                    "password" => openclaw_identity::CredentialKind::Password,
                    "api_key" | "apikey" => openclaw_identity::CredentialKind::ApiKey,
                    "oauth_token" | "oauth" => openclaw_identity::CredentialKind::OAuthToken,
                    "session_cookie" | "cookie" => openclaw_identity::CredentialKind::SessionCookie,
                    _ => openclaw_identity::CredentialKind::Generic,
                });

                match vault.get(&input.domain, kind) {
                    Ok(cred) => {
                        Ok(CallToolResult::success(vec![Content::text(
                            format!(
                                "Credential found for {}:\n  ID: {}\n  Identity: {}\n  Kind: {:?}\n  Secret: {}",
                                input.domain, cred.id, cred.identity, cred.kind,
                                cred.expose_secret_value()
                            )
                        )]))
                    }
                    Err(e) => {
                        Ok(CallToolResult::success(vec![Content::text(
                            format!("No credential found for {}: {e}", input.domain)
                        )]))
                    }
                }
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
             browse_extract to get markdown content, and browse_search to search the web. \
             JavaScript execution available via browse_eval_js (QuickJS engine)."
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

// Cookie persistence is handled by the engine layer now.
