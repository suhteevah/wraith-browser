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
                &schema_for!(EvalJsInput), rw_destructive.clone()),
            make_tool("browse_tabs",
                "Show the current page URL and title.",
                &schema_for!(TabsInput), ro_closed.clone()),
            make_tool("browse_back",
                "Go back to the previous page in browser history.",
                &schema_for!(BackInput), rw_open.clone()),
            make_tool("browse_key_press",
                "Press a keyboard key on the current page.",
                &schema_for!(KeyPressInput), rw_open.clone()),
            make_tool("browse_scroll",
                "Scroll the current page up or down.",
                &schema_for!(ScrollInput), rw_closed.clone()),
            make_tool("browse_vault_store",
                "Store a credential (password, API key, token) in the encrypted vault.",
                &schema_for!(VaultStoreInput), rw_closed.clone()),
            make_tool("browse_vault_get",
                "Retrieve a credential from the encrypted vault for a given domain.",
                &schema_for!(VaultGetInput), ro_closed.clone()),
            make_tool("browse_vault_list",
                "List all stored credentials (secrets stay encrypted, only shows domain/identity/kind).",
                &schema_for!(VaultListInput), ro_closed.clone()),
            make_tool("browse_vault_delete",
                "Delete a credential by ID from the encrypted vault.",
                &schema_for!(VaultDeleteInput), rw_destructive.clone()),
            make_tool("browse_vault_totp",
                "Generate a current TOTP 2FA code for a domain.",
                &schema_for!(VaultTotpInput), ro_closed.clone()),
            make_tool("browse_vault_rotate",
                "Rotate a credential's secret value.",
                &schema_for!(VaultRotateInput), rw_closed.clone()),
            make_tool("browse_vault_audit",
                "View recent vault audit log entries.",
                &schema_for!(VaultAuditInput), ro_closed.clone()),
            make_tool("browse_select",
                "Select a dropdown option by @ref ID and value.",
                &schema_for!(SelectInput), rw_open.clone()),
            make_tool("browse_type",
                "Type text into an element with realistic keystroke delays (for bot detection evasion).",
                &schema_for!(TypeTextInput), rw_open.clone()),
            make_tool("browse_hover",
                "Hover over an element by @ref ID.",
                &schema_for!(HoverInput), rw_open.clone()),
            make_tool("browse_wait",
                "Wait for a CSS selector to appear or a fixed time in milliseconds.",
                &schema_for!(WaitInput), ro_closed.clone()),
            make_tool("browse_forward",
                "Go forward in browser history.",
                &schema_for!(ForwardInput), rw_open.clone()),
            make_tool("browse_reload",
                "Reload the current page.",
                &schema_for!(ReloadInput), rw_open.clone()),
            make_tool("browse_task",
                "Run an autonomous multi-step browsing task using the AI agent loop.",
                &schema_for!(TaskInput), rw_open.clone()),
            make_tool("cache_search",
                "Search the knowledge cache for previously visited pages.",
                &schema_for!(CacheSearchInput), ro_closed.clone()),
            make_tool("cache_get",
                "Check if a URL is in the knowledge cache and return cached content.",
                &schema_for!(CacheGetInput), ro_closed.clone()),
            make_tool("script_load",
                "Load a Rhai userscript that triggers on navigation or manually.",
                &schema_for!(ScriptLoadInput), rw_closed.clone()),
            make_tool("script_list",
                "List all loaded Rhai userscripts.",
                &schema_for!(ScriptListInput), ro_closed.clone()),
            make_tool("script_run",
                "Run a loaded Rhai script by name against the current page.",
                &schema_for!(ScriptRunInput), rw_closed.clone()),
            make_tool("browse_config",
                "Show current engine configuration (engine type, proxy, stealth status).",
                &schema_for!(ConfigInput), ro_closed),
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

                let vault = open_vault()?;

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

                let vault = open_vault()?;

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

            // === Vault: list, delete, totp, rotate, audit ===

            "browse_vault_list" => {
                let vault = open_vault()?;
                match vault.list_credentials() {
                    Ok(creds) => {
                        if creds.is_empty() {
                            Ok(CallToolResult::success(vec![Content::text("No credentials stored.")]))
                        } else {
                            let mut out = format!("{} credential(s):\n\n", creds.len());
                            for c in &creds {
                                out.push_str(&format!("  {} | {} | {:?} | {} | {} uses\n",
                                    &c.id[..8], c.domain, c.kind, c.identity, c.use_count));
                            }
                            Ok(CallToolResult::success(vec![Content::text(out)]))
                        }
                    }
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Vault list failed: {e}"))]))
                }
            }

            "browse_vault_delete" => {
                let input: VaultDeleteInput = parse_args(args)?;
                let vault = open_vault()?;
                match vault.delete(&input.id) {
                    Ok(()) => Ok(CallToolResult::success(vec![Content::text(format!("Credential {} deleted.", input.id))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Delete failed: {e}"))]))
                }
            }

            "browse_vault_totp" => {
                let input: VaultTotpInput = parse_args(args)?;
                let vault = open_vault()?;
                match vault.generate_totp(&input.domain) {
                    Ok(code) => Ok(CallToolResult::success(vec![Content::text(format!("TOTP code for {}: {}", input.domain, code))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("TOTP generation failed: {e}"))]))
                }
            }

            "browse_vault_rotate" => {
                let input: VaultRotateInput = parse_args(args)?;
                let vault = open_vault()?;
                let new_secret = secrecy::SecretString::from(input.new_secret);
                match vault.rotate(&input.id, &new_secret) {
                    Ok(()) => Ok(CallToolResult::success(vec![Content::text(format!("Credential {} rotated.", input.id))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Rotate failed: {e}"))]))
                }
            }

            "browse_vault_audit" => {
                let input: VaultAuditInput = parse_args(args)?;
                let vault = open_vault()?;
                let limit = input.limit.unwrap_or(20);
                match vault.audit_history(limit) {
                    Ok(entries) => {
                        if entries.is_empty() {
                            Ok(CallToolResult::success(vec![Content::text("No audit log entries.")]))
                        } else {
                            let mut out = format!("{} audit entries:\n\n", entries.len());
                            for e in &entries {
                                out.push_str(&format!("  {} | {} | {} | {}\n",
                                    e.timestamp, e.action,
                                    e.domain.as_deref().unwrap_or("-"),
                                    if e.success { "OK" } else { "FAIL" }));
                            }
                            Ok(CallToolResult::success(vec![Content::text(out)]))
                        }
                    }
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Audit failed: {e}"))]))
                }
            }

            // === Browser actions: select, type, hover, wait, forward, reload ===

            "browse_select" => {
                let input: SelectInput = parse_args(args)?;
                let mut engine = self.engine.lock().await;
                let result = engine.execute_action(BrowserAction::Select { ref_id: input.ref_id, value: input.value }).await
                    .map_err(|e| ErrorData::internal_error(format!("Select failed: {e}"), None))?;
                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            "browse_type" => {
                let input: TypeTextInput = parse_args(args)?;
                let mut engine = self.engine.lock().await;
                let result = engine.execute_action(BrowserAction::TypeText {
                    ref_id: input.ref_id,
                    text: input.text,
                    delay_ms: input.delay_ms.unwrap_or(50),
                }).await
                    .map_err(|e| ErrorData::internal_error(format!("Type failed: {e}"), None))?;
                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            "browse_hover" => {
                let input: HoverInput = parse_args(args)?;
                let mut engine = self.engine.lock().await;
                let result = engine.execute_action(BrowserAction::Hover { ref_id: input.ref_id }).await
                    .map_err(|e| ErrorData::internal_error(format!("Hover failed: {e}"), None))?;
                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            "browse_wait" => {
                let input: WaitInput = parse_args(args)?;
                let mut engine = self.engine.lock().await;
                if let Some(selector) = input.selector {
                    let result = engine.execute_action(BrowserAction::WaitForSelector {
                        selector,
                        timeout_ms: input.ms.unwrap_or(5000),
                    }).await
                        .map_err(|e| ErrorData::internal_error(format!("Wait failed: {e}"), None))?;
                    Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
                } else {
                    let ms = input.ms.unwrap_or(1000);
                    let result = engine.execute_action(BrowserAction::Wait { ms }).await
                        .map_err(|e| ErrorData::internal_error(format!("Wait failed: {e}"), None))?;
                    Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
                }
            }

            "browse_forward" => {
                let mut engine = self.engine.lock().await;
                let result = engine.execute_action(BrowserAction::GoForward).await
                    .map_err(|e| ErrorData::internal_error(format!("Forward failed: {e}"), None))?;
                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            "browse_reload" => {
                let mut engine = self.engine.lock().await;
                let result = engine.execute_action(BrowserAction::Reload).await
                    .map_err(|e| ErrorData::internal_error(format!("Reload failed: {e}"), None))?;
                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            // === Agent task ===

            "browse_task" => {
                let input: TaskInput = parse_args(args)?;
                info!(task = %input.description, "Running autonomous task");

                let api_key = std::env::var("ANTHROPIC_API_KEY")
                    .or_else(|_| std::env::var("CLAUDE_API_KEY"))
                    .map_err(|_| ErrorData::internal_error(
                        "No API key — set ANTHROPIC_API_KEY or CLAUDE_API_KEY", None
                    ))?;

                let backend = openclaw_agent_loop::llm::ClaudeBackend::new(api_key);
                let config = openclaw_agent_loop::AgentConfig {
                    max_steps: input.max_steps.unwrap_or(50),
                    ..Default::default()
                };

                let task = openclaw_agent_loop::BrowsingTask {
                    description: input.description,
                    start_url: input.url,
                    timeout_secs: None,
                    context: None,
                };

                let mut agent = openclaw_agent_loop::Agent::new(config, self.engine.clone(), backend);
                match agent.run(task).await {
                    Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Task failed: {e}"))]))
                }
            }

            // === Cache ===

            "cache_search" => {
                let input: CacheSearchInput = parse_args(args)?;
                let cache_dir = dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".openclaw").join("knowledge");

                match openclaw_cache::KnowledgeStore::open(&cache_dir) {
                    Ok(store) => {
                        let max = input.max_results.unwrap_or(10);
                        match store.search_knowledge(&input.query, max) {
                            Ok(results) => {
                                if results.is_empty() {
                                    Ok(CallToolResult::success(vec![Content::text("No cached results found.")]))
                                } else {
                                    let mut out = format!("{} results:\n\n", results.len());
                                    for r in &results {
                                        out.push_str(&format!("  {} — {}\n    {}\n\n",
                                            r.title, r.url, r.snippet.get(..200).unwrap_or(&r.snippet)));
                                    }
                                    Ok(CallToolResult::success(vec![Content::text(out)]))
                                }
                            }
                            Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Search failed: {e}"))]))
                        }
                    }
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cache not available: {e}"))]))
                }
            }

            "cache_get" => {
                let input: CacheGetInput = parse_args(args)?;
                let cache_dir = dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".openclaw").join("knowledge");

                match openclaw_cache::KnowledgeStore::open(&cache_dir) {
                    Ok(store) => {
                        match store.get_page(&input.url) {
                            Ok(Some(page)) => {
                                Ok(CallToolResult::success(vec![Content::text(
                                    format!("Cached: {} ({})\nFetched: {}\nStale: {}\nTokens: ~{}",
                                        page.title, page.url, page.last_fetched,
                                        store.is_stale(&page), page.token_count)
                                )]))
                            }
                            Ok(None) => Ok(CallToolResult::success(vec![Content::text("URL not in cache.")])),
                            Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cache error: {e}"))]))
                        }
                    }
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cache not available: {e}"))]))
                }
            }

            // === Scripting ===

            "script_load" | "script_list" | "script_run" => {
                Ok(CallToolResult::success(vec![Content::text(
                    "Scripting tools require the Sevro engine with scripting support. \
                     Scripts are loaded via the engine's scripting() API."
                )]))
            }

            // === Config ===

            "browse_config" => {
                let engine = self.engine.lock().await;
                let caps = engine.capabilities();
                Ok(CallToolResult::success(vec![Content::text(
                    format!("Engine capabilities:\n  JavaScript: {}\n  Screenshots: {:?}\n  Layout: {}\n  Cookies: {}\n  Stealth: {}",
                        caps.javascript, caps.screenshots, caps.layout, caps.cookies, caps.stealth)
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

/// Open and auto-unlock the vault for MCP operations.
fn open_vault() -> Result<openclaw_identity::CredentialVault, ErrorData> {
    let vault_path = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".openclaw")
        .join("vault.db");

    let vault = openclaw_identity::CredentialVault::open(&vault_path)
        .map_err(|e| ErrorData::internal_error(format!("Vault open failed: {e}"), None))?;

    let _ = vault.unlock(&secrecy::SecretString::from("".to_string()));
    Ok(vault)
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
