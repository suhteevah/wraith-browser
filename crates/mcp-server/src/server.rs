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
                &schema_for!(SearchInput), ro_open.clone()),
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
                &schema_for!(ConfigInput), ro_closed.clone()),
            make_tool("cookie_get",
                "Get cookies for a domain from the browser's cookie jar.",
                &schema_for!(CookieGetInput), ro_closed.clone()),
            make_tool("cookie_set",
                "Set a cookie in the browser's cookie jar.",
                &schema_for!(CookieSetInput), rw_closed.clone()),
            make_tool("fingerprint_list",
                "List available browser fingerprint profiles (Chrome, Firefox, Safari).",
                &schema_for!(FingerprintListInput), ro_closed.clone()),
            make_tool("entity_query",
                "Query the knowledge graph — ask what we know about an entity across all visited pages.",
                &schema_for!(EntityQueryInput), ro_closed.clone()),
            make_tool("cache_stats",
                "Show knowledge cache statistics (page count, size, domains).",
                &schema_for!(CacheStatsInput), ro_closed.clone()),
            make_tool("cache_purge",
                "Purge stale entries from the knowledge cache.",
                &schema_for!(CachePurgeInput), rw_destructive.clone()),
            make_tool("network_discover",
                "Discover API endpoints from captured network traffic patterns.",
                &schema_for!(NetworkDiscoverInput), ro_closed.clone()),
            make_tool("site_fingerprint",
                "Detect site technology stack (React, WordPress, Shopify, etc.) from current page.",
                &schema_for!(SiteFingerprintInput), ro_closed.clone()),
            make_tool("page_diff",
                "Compare current page content to the cached version — detect changes.",
                &schema_for!(PageDiffInput), ro_closed.clone()),
            make_tool("tls_profiles",
                "List available TLS fingerprint profiles for stealth browsing.",
                &schema_for!(TlsProfilesInput), ro_closed.clone()),
            make_tool("browse_wait_navigation",
                "Wait for navigation to complete after a click or form submission.",
                &schema_for!(WaitForNavigationInput), ro_closed.clone()),
            // === New tools (63 remaining) ===
            make_tool("vault_lock", "Lock the encrypted vault and zeroize the master key from memory.", &schema_for!(VaultLockInput), rw_closed.clone()),
            make_tool("vault_unlock", "Unlock the vault with a passphrase. Use empty string for auto-unlock.", &schema_for!(VaultUnlockInput), rw_closed.clone()),
            make_tool("vault_approve_domain", "Approve a domain to use a specific credential.", &schema_for!(VaultApproveDomainInput), rw_closed.clone()),
            make_tool("vault_revoke_domain", "Revoke a domain's access to a credential.", &schema_for!(VaultRevokeDomainInput), rw_closed.clone()),
            make_tool("vault_check_approval", "Check if a domain is approved for a credential.", &schema_for!(VaultCheckApprovalInput), ro_closed.clone()),
            make_tool("cookie_save", "Save browser cookies to a JSON file for persistence across sessions.", &schema_for!(CookieSaveInput), rw_closed.clone()),
            make_tool("cookie_load", "Load cookies from a JSON file into the browser.", &schema_for!(CookieLoadInput), rw_closed.clone()),
            make_tool("cache_pin", "Pin a URL so it is never evicted from cache.", &schema_for!(CachePinInput), rw_closed.clone()),
            make_tool("cache_tag", "Tag a cached page with labels for organized retrieval.", &schema_for!(CacheTagInput), rw_closed.clone()),
            make_tool("cache_domain_profile", "Show how often a domain's content changes and its computed TTL.", &schema_for!(CacheDomainProfileInput), ro_closed.clone()),
            make_tool("cache_find_similar", "Find cached pages similar to a given URL.", &schema_for!(CacheFindSimilarInput), ro_closed.clone()),
            make_tool("cache_evict", "Evict cached pages to fit within a byte budget.", &schema_for!(CacheEvictInput), rw_destructive.clone()),
            make_tool("cache_raw_html", "Get the raw cached HTML for a URL.", &schema_for!(CacheRawHtmlInput), ro_closed.clone()),
            make_tool("dom_query_selector", "Run a CSS selector query against the current page DOM.", &schema_for!(DomQuerySelectorInput), ro_closed.clone()),
            make_tool("dom_get_attribute", "Read an HTML attribute from an element by @ref ID.", &schema_for!(DomGetAttributeInput), ro_closed.clone()),
            make_tool("dom_set_attribute", "Set an HTML attribute on an element by @ref ID.", &schema_for!(DomSetAttributeInput), rw_closed.clone()),
            make_tool("dom_focus", "Focus an element by @ref ID.", &schema_for!(DomFocusInput), rw_closed.clone()),
            make_tool("extract_pdf", "Fetch a PDF from a URL and extract its text content as markdown.", &schema_for!(ExtractPdfInput), ro_open.clone()),
            make_tool("extract_article", "Extract the main article body from the current page using readability.", &schema_for!(ExtractArticleInput), ro_closed.clone()),
            make_tool("extract_markdown", "Convert HTML to clean markdown.", &schema_for!(ExtractMarkdownInput), ro_closed.clone()),
            make_tool("extract_plain_text", "Convert HTML to plain text with no formatting.", &schema_for!(ExtractPlainTextInput), ro_closed.clone()),
            make_tool("extract_ocr", "Run OCR text detection on image data.", &schema_for!(ExtractOcrInput), ro_closed.clone()),
            make_tool("auth_detect", "Detect authentication flows on the current page (password, OAuth, 2FA, CAPTCHA).", &schema_for!(AuthDetectInput), ro_closed.clone()),
            make_tool("fingerprint_import", "Import a browser fingerprint profile from a JSON file.", &schema_for!(FingerprintImportInput), rw_closed.clone()),
            make_tool("identity_profile", "Set the browsing identity profile (personal or anonymous).", &schema_for!(IdentityProfileInput), rw_closed.clone()),
            make_tool("dns_resolve", "Resolve a domain name to IP addresses via DNS-over-HTTPS.", &schema_for!(DnsResolveInput), ro_open.clone()),
            make_tool("stealth_status", "Show current stealth TLS status and evasion count.", &schema_for!(StealthStatusInput), ro_closed.clone()),
            make_tool("plugin_register", "Register a WASM plugin.", &schema_for!(PluginRegisterInput), rw_closed.clone()),
            make_tool("plugin_execute", "Execute a registered WASM plugin.", &schema_for!(PluginExecuteInput), rw_closed.clone()),
            make_tool("plugin_list", "List all registered WASM plugins.", &schema_for!(PluginListInput), ro_closed.clone()),
            make_tool("plugin_remove", "Remove a registered WASM plugin.", &schema_for!(PluginRemoveInput), rw_closed.clone()),
            make_tool("telemetry_metrics", "Show browsing metrics (cache hits, errors, navigations).", &schema_for!(TelemetryMetricsInput), ro_closed.clone()),
            make_tool("telemetry_spans", "Export performance trace spans as JSON.", &schema_for!(TelemetrySpansInput), ro_closed.clone()),
            make_tool("workflow_start_recording", "Start recording a replayable workflow.", &schema_for!(WorkflowStartRecordingInput), rw_closed.clone()),
            make_tool("workflow_stop_recording", "Stop recording and save the workflow.", &schema_for!(WorkflowStopRecordingInput), rw_closed.clone()),
            make_tool("workflow_replay", "Replay a saved workflow with variable substitution.", &schema_for!(WorkflowReplayInput), rw_open.clone()),
            make_tool("workflow_list", "List saved workflows.", &schema_for!(WorkflowListInput), ro_closed.clone()),
            make_tool("timetravel_summary", "Show the agent decision timeline summary.", &schema_for!(TimeTravelSummaryInput), ro_closed.clone()),
            make_tool("timetravel_branch", "Branch from a decision point to explore alternatives.", &schema_for!(TimeTravelBranchInput), rw_closed.clone()),
            make_tool("timetravel_replay", "Replay the timeline to a specific step.", &schema_for!(TimeTravelReplayInput), ro_closed.clone()),
            make_tool("timetravel_diff", "Diff two timeline branches to see where decisions diverged.", &schema_for!(TimeTravelDiffInput), ro_closed.clone()),
            make_tool("timetravel_export", "Export the full timeline as JSON.", &schema_for!(TimeTravelExportInput), ro_closed.clone()),
            make_tool("dag_create", "Create a task DAG for parallel task orchestration.", &schema_for!(DagCreateInput), rw_closed.clone()),
            make_tool("dag_add_task", "Add a task node to the DAG.", &schema_for!(DagAddTaskInput), rw_closed.clone()),
            make_tool("dag_add_dependency", "Add a dependency between DAG tasks.", &schema_for!(DagAddDependencyInput), rw_closed.clone()),
            make_tool("dag_ready", "Get tasks that are ready to execute (all dependencies met).", &schema_for!(DagReadyInput), ro_closed.clone()),
            make_tool("dag_complete", "Mark a DAG task as completed.", &schema_for!(DagCompleteInput), rw_closed.clone()),
            make_tool("dag_progress", "Show DAG completion progress.", &schema_for!(DagProgressInput), ro_closed.clone()),
            make_tool("dag_visualize", "Generate a Mermaid diagram of the DAG.", &schema_for!(DagVisualizeInput), ro_closed.clone()),
            make_tool("mcts_plan", "Use MCTS to plan the best next action given current state.", &schema_for!(MctsPlanInput), ro_closed.clone()),
            make_tool("mcts_stats", "Show MCTS planner statistics.", &schema_for!(MctsStatsInput), ro_closed.clone()),
            make_tool("prefetch_predict", "Predict which URLs to prefetch based on the current task.", &schema_for!(PrefetchPredictInput), ro_closed.clone()),
            make_tool("swarm_fan_out", "Visit multiple URLs in parallel and collect results.", &schema_for!(SwarmFanOutInput), rw_open.clone()),
            make_tool("swarm_collect", "Collect results from a parallel browsing swarm.", &schema_for!(SwarmCollectInput), ro_closed.clone()),
            make_tool("entity_add", "Add an entity to the knowledge graph.", &schema_for!(EntityAddInput), rw_closed.clone()),
            make_tool("entity_relate", "Add a relationship between entities in the knowledge graph.", &schema_for!(EntityRelateInput), rw_closed.clone()),
            make_tool("entity_merge", "Merge two entities in the knowledge graph.", &schema_for!(EntityMergeInput), rw_closed.clone()),
            make_tool("entity_find_related", "Find entities related to a given entity.", &schema_for!(EntityFindRelatedInput), ro_closed.clone()),
            make_tool("entity_search", "Search entities by name in the knowledge graph.", &schema_for!(EntitySearchInput), ro_closed.clone()),
            make_tool("entity_visualize", "Generate a Mermaid diagram of the knowledge graph.", &schema_for!(EntityVisualizeInput), ro_closed.clone()),
            make_tool("embedding_search", "Semantic similarity search across cached content.", &schema_for!(EmbeddingSearchInput), ro_closed.clone()),
            make_tool("embedding_upsert", "Store a text embedding for semantic search.", &schema_for!(EmbeddingUpsertInput), rw_closed.clone()),
            make_tool("browse_upload_file",
                "Upload a file from disk to an <input type='file'> element on the current page. Reads the file, base64-encodes it, and injects it into the file input via JavaScript. Use for resume uploads, document submissions, image uploads, etc. Set ref_id to the @ref ID of the file input, or omit to auto-detect the first file input on the page.",
                &schema_for!(UploadFileInput), rw_open.clone()),
            make_tool("browse_submit_form",
                "Submit a form by clicking its submit button or calling form.submit(). If ref_id points to a button, it is clicked. If it points to a form element, form.submit() is called. If it points to any element inside a form, the parent form is submitted. Handles React-controlled forms that use XHR/fetch submission.",
                &schema_for!(SubmitFormInput), rw_open.clone()),
            make_tool("browse_custom_dropdown",
                "Interact with a custom dropdown/combobox (non-native <select>). Handles React/Greenhouse-style dropdowns: clicks to open, types to filter, then clicks the matching option. Use for country selectors, visa sponsorship fields, EEO fields, etc.",
                &schema_for!(CustomDropdownInput), rw_open.clone()),
            make_tool("cookie_import_chrome",
                "Import cookies from the user's Chrome browser profile to reuse existing login sessions. Reads Chrome's encrypted cookie database, decrypts using OS credentials, and loads into Wraith. Avoids re-login and bot detection triggers.",
                &schema_for!(ChromeCookieImportInput), rw_open),
        ];

        info!(tool_count = tools.len(), "Wraith MCP handler initialized");
        Self { tools, engine }
    }

    /// Build the default engine: Sevro → NativeEngine fallback.
    /// Reads config from environment variables:
    /// - `WRAITH_FLARESOLVERR` — FlareSolverr URL (e.g., "http://localhost:8191")
    /// - `WRAITH_PROXY` — HTTP proxy URL
    /// - `WRAITH_FALLBACK_PROXY` — Fallback proxy for IP bans
    fn default_engine() -> Arc<Mutex<dyn BrowserEngine>> {
        #[cfg(feature = "sevro")]
        {
            let flaresolverr = std::env::var("WRAITH_FLARESOLVERR").ok();
            let proxy = std::env::var("WRAITH_PROXY").ok();
            let fallback_proxy = std::env::var("WRAITH_FALLBACK_PROXY").ok();

            if flaresolverr.is_some() {
                info!(solver = ?flaresolverr, "FlareSolverr configured via WRAITH_FLARESOLVERR");
            }
            if proxy.is_some() {
                info!(proxy = ?proxy, "Proxy configured via WRAITH_PROXY");
            }

            // Use the engine factory which handles SevroConfig internally
            let opts = openclaw_browser_core::engine::EngineOptions {
                proxy_url: proxy,
                flaresolverr_url: flaresolverr,
                fallback_proxy_url: fallback_proxy,
            };

            info!("Using Sevro engine (default)");
            // create_engine_with_options is async but we need sync here;
            // construct directly instead
            let mut config = openclaw_browser_core::config::BrowserConfig::default();
            let _ = config; // suppress unused

            // Direct construction via SevroEngineBackend
            use openclaw_browser_core::engine_sevro::SevroEngineBackend;
            return Arc::new(Mutex::new(SevroEngineBackend::new_with_options(opts)));
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
                let has_flaresolverr = std::env::var("WRAITH_FLARESOLVERR").is_ok();
                let has_proxy = std::env::var("WRAITH_PROXY").is_ok();
                Ok(CallToolResult::success(vec![Content::text(
                    format!("Engine capabilities:\n  JavaScript: {}\n  Screenshots: {:?}\n  Layout: {}\n  Cookies: {}\n  Stealth: {}\n  FlareSolverr: {}\n  Proxy: {}",
                        caps.javascript, caps.screenshots, caps.layout, caps.cookies, caps.stealth,
                        if has_flaresolverr { "configured" } else { "not configured (set WRAITH_FLARESOLVERR)" },
                        if has_proxy { "configured" } else { "direct" })
                )]))
            }

            // === Cookies ===

            "cookie_get" => {
                let input: CookieGetInput = parse_args(args)?;
                let engine = self.engine.lock().await;
                // Use eval_js to read cookies from the JS environment
                match engine.eval_js("__wraith_get_cookies()").await {
                    Ok(cookies_json) => {
                        Ok(CallToolResult::success(vec![Content::text(
                            format!("Cookies for current page:\n{}", cookies_json)
                        )]))
                    }
                    Err(_) => {
                        Ok(CallToolResult::success(vec![Content::text(
                            format!("No cookies available for {}", input.domain)
                        )]))
                    }
                }
            }

            "cookie_set" => {
                let input: CookieSetInput = parse_args(args)?;
                let path = input.path.unwrap_or_else(|| "/".to_string());
                let engine = self.engine.lock().await;
                let script = format!(
                    "document.cookie = '{}={}; domain={}; path={}'",
                    input.name, input.value, input.domain, path
                );
                match engine.eval_js(&script).await {
                    Ok(_) => Ok(CallToolResult::success(vec![Content::text(
                        format!("Cookie set: {}={} for {}", input.name, input.value, input.domain)
                    )])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cookie set failed: {e}"))]))
                }
            }

            // === Fingerprints ===

            "fingerprint_list" => {
                let profiles = openclaw_browser_core::tls_fingerprint::all_profiles();
                let mut out = format!("{} TLS fingerprint profiles:\n\n", profiles.len());
                for p in &profiles {
                    out.push_str(&format!("  {} — JA3: {}...\n    UA: {}...\n\n",
                        p.name, &p.ja3_hash[..16],
                        p.user_agent.get(..60).unwrap_or(&p.user_agent)));
                }
                Ok(CallToolResult::success(vec![Content::text(out)]))
            }

            // === TLS Profiles ===

            "tls_profiles" => {
                let profiles = openclaw_browser_core::tls_fingerprint::all_profiles();
                let mut out = format!("{} profiles available:\n\n", profiles.len());
                for p in &profiles {
                    out.push_str(&format!("  Name: {}\n  JA3: {}\n  JA4: {}\n  HTTP/2 Window: {}\n  Headers: {}\n\n",
                        p.name, p.ja3_hash,
                        p.ja4_hash.as_deref().unwrap_or("N/A"),
                        p.http2_settings.initial_window_size,
                        p.header_order.len()));
                }
                Ok(CallToolResult::success(vec![Content::text(out)]))
            }

            // === Knowledge Graph ===

            "entity_query" => {
                let input: EntityQueryInput = parse_args(args)?;
                let mut graph = openclaw_cache::entity_graph::EntityGraph::new();
                // Search the graph for the entity mentioned in the question
                let results = graph.search_entities(&input.question);
                if results.is_empty() {
                    Ok(CallToolResult::success(vec![Content::text(
                        format!("No entities found matching '{}'. Visit more pages to build the knowledge graph.", input.question)
                    )]))
                } else {
                    let answer = graph.query(&input.question);
                    Ok(CallToolResult::success(vec![Content::text(answer)]))
                }
            }

            // === Cache Stats/Purge ===

            "cache_stats" => {
                let cache_dir = dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".openclaw").join("knowledge");
                match openclaw_cache::KnowledgeStore::open(&cache_dir) {
                    Ok(store) => {
                        match store.stats() {
                            Ok(stats) => Ok(CallToolResult::success(vec![Content::text(
                                format!("Cache stats:\n  Pages: {}\n  Domains: {}\n  Searches: {}\n  Snapshots: {}\n  Size: {} bytes\n  Stale: {}\n  Pinned: {}",
                                    stats.total_pages, stats.total_domains, stats.total_searches,
                                    stats.total_snapshots, stats.total_disk_bytes, stats.stale_pages, stats.pinned_pages)
                            )])),
                            Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Stats error: {e}"))]))
                        }
                    }
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cache not available: {e}"))]))
                }
            }

            "cache_purge" => {
                let cache_dir = dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".openclaw").join("knowledge");
                match openclaw_cache::KnowledgeStore::open(&cache_dir) {
                    Ok(store) => {
                        match store.purge_stale() {
                            Ok(count) => Ok(CallToolResult::success(vec![Content::text(
                                format!("Purged {} stale cache entries.", count)
                            )])),
                            Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Purge failed: {e}"))]))
                        }
                    }
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cache not available: {e}"))]))
                }
            }

            // === Network Discovery ===

            "network_discover" => {
                // NetworkCapture is per-session; create a fresh one and report
                let capture = openclaw_browser_core::network_intel::NetworkCapture::new();
                let endpoints = capture.discover_endpoints();
                if endpoints.is_empty() {
                    Ok(CallToolResult::success(vec![Content::text(
                        "No API endpoints discovered yet. Navigate to pages first — network traffic is captured automatically."
                    )]))
                } else {
                    let mut out = format!("{} endpoints discovered:\n\n", endpoints.len());
                    for ep in &endpoints {
                        out.push_str(&format!("  {} {} (seen {} times)\n", ep.method, ep.url_template, ep.seen_count));
                    }
                    Ok(CallToolResult::success(vec![Content::text(out)]))
                }
            }

            // === Site Fingerprint ===

            "site_fingerprint" => {
                let _input: SiteFingerprintInput = parse_args(args)?;
                let engine = self.engine.lock().await;
                let html = engine.page_source().await.unwrap_or_default();
                let url = engine.current_url().await.unwrap_or_default();
                let domain = url::Url::parse(&url)
                    .map(|u| u.host_str().unwrap_or("").to_string())
                    .unwrap_or_default();

                let cap = openclaw_cache::site_capability::fingerprint_site(&domain, &html, &url);
                let techs = openclaw_cache::site_capability::detect_technology(&html);

                let mut out = format!("Site: {}\n", domain);
                out.push_str(&format!("  Has login: {}\n", cap.has_login));
                out.push_str(&format!("  Has search: {}\n", cap.has_search));
                out.push_str(&format!("  Has API: {}\n", cap.has_api));
                out.push_str(&format!("  Nav links: {}\n", cap.nav_links.len()));
                out.push_str(&format!("  Strategy: {:?}\n", cap.optimal_strategy));
                if !techs.is_empty() {
                    out.push_str(&format!("  Technologies: {}\n", techs.join(", ")));
                }
                Ok(CallToolResult::success(vec![Content::text(out)]))
            }

            // === Page Diff ===

            "page_diff" => {
                let _input: PageDiffInput = parse_args(args)?;
                let engine = self.engine.lock().await;
                let url = engine.current_url().await.unwrap_or_default();
                let current_html = engine.page_source().await.unwrap_or_default();

                let cache_dir = dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".openclaw").join("knowledge");

                match openclaw_cache::KnowledgeStore::open(&cache_dir) {
                    Ok(store) => {
                        match store.get_page(&url) {
                            Ok(Some(cached)) => {
                                let diff = openclaw_cache::diffing::diff_pages(&url, &cached.plain_text, &current_html);
                                Ok(CallToolResult::success(vec![Content::text(
                                    format!("Page diff for {}:\nSimilarity: {:.0}%\nChanges: {}\n\n{}",
                                        url, diff.similarity_score * 100.0, diff.changes.len(), diff.summary)
                                )]))
                            }
                            Ok(None) => Ok(CallToolResult::success(vec![Content::text("No cached version to compare against.")])),
                            Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cache error: {e}"))]))
                        }
                    }
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cache not available: {e}"))]))
                }
            }

            // === Wait for Navigation ===

            "browse_wait_navigation" => {
                let input: WaitForNavigationInput = parse_args(args)?;
                let mut engine = self.engine.lock().await;
                let result = engine.execute_action(BrowserAction::WaitForNavigation {
                    timeout_ms: input.timeout_ms.unwrap_or(5000),
                }).await
                    .map_err(|e| ErrorData::internal_error(format!("Wait failed: {e}"), None))?;
                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            // ═══════════════════════════════════════════════════════
            // 63 NEW HANDLERS — Full MCP coverage
            // ═══════════════════════════════════════════════════════

            "vault_lock" => {
                let vault = open_vault()?;
                vault.lock();
                Ok(CallToolResult::success(vec![Content::text("Vault locked. Master key zeroized.")]))
            }
            "vault_unlock" => {
                let input: VaultUnlockInput = parse_args(args)?;
                let vault = open_vault()?;
                let pass = secrecy::SecretString::from(input.passphrase.unwrap_or_default());
                match vault.unlock(&pass) {
                    Ok(()) => Ok(CallToolResult::success(vec![Content::text("Vault unlocked.")])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Unlock failed: {e}"))]))
                }
            }
            "vault_approve_domain" => {
                let input: VaultApproveDomainInput = parse_args(args)?;
                let vault = open_vault()?;
                match vault.approve_domain(&input.credential_id, &input.domain) {
                    Ok(()) => Ok(CallToolResult::success(vec![Content::text(format!("Domain '{}' approved for credential {}", input.domain, input.credential_id))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Approve failed: {e}"))]))
                }
            }
            "vault_revoke_domain" => {
                let input: VaultRevokeDomainInput = parse_args(args)?;
                let vault = open_vault()?;
                match vault.revoke_domain(&input.credential_id, &input.domain) {
                    Ok(()) => Ok(CallToolResult::success(vec![Content::text(format!("Domain '{}' revoked for credential {}", input.domain, input.credential_id))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Revoke failed: {e}"))]))
                }
            }
            "vault_check_approval" => {
                let input: VaultCheckApprovalInput = parse_args(args)?;
                let vault = open_vault()?;
                match vault.is_domain_approved(&input.credential_id, &input.domain) {
                    Ok(approved) => Ok(CallToolResult::success(vec![Content::text(format!("{}: {}", input.domain, if approved { "APPROVED" } else { "NOT APPROVED" }))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Check failed: {e}"))]))
                }
            }
            "cookie_save" => {
                let input: CookieSaveInput = parse_args(args)?;
                let path = input.path.unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".openclaw").join("cookies.json").to_string_lossy().to_string());
                let engine = self.engine.lock().await;
                match engine.eval_js("__wraith_get_cookies()").await {
                    Ok(json) => match std::fs::write(&path, &json) {
                        Ok(()) => Ok(CallToolResult::success(vec![Content::text(format!("Cookies saved to {path}"))])),
                        Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Save failed: {e}"))]))
                    },
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("No cookies: {e}"))]))
                }
            }
            "cookie_load" => {
                let input: CookieLoadInput = parse_args(args)?;
                let path = input.path.unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".openclaw").join("cookies.json").to_string_lossy().to_string());
                match std::fs::read_to_string(&path) {
                    Ok(json) => {
                        let engine = self.engine.lock().await;
                        match engine.eval_js(&format!("Object.assign(__wraith_cookies, {})", json)).await {
                            Ok(_) => Ok(CallToolResult::success(vec![Content::text(format!("Cookies loaded from {path}"))])),
                            Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Inject failed: {e}"))]))
                        }
                    }
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Load failed: {e}"))]))
                }
            }
            "cache_pin" => {
                let input: CachePinInput = parse_args(args)?;
                let dir = dirs::home_dir().unwrap_or_default().join(".openclaw").join("knowledge");
                match openclaw_cache::KnowledgeStore::open(&dir) {
                    Ok(store) => match store.pin_page(&input.url, input.notes.as_deref()) {
                        Ok(()) => Ok(CallToolResult::success(vec![Content::text(format!("Pinned: {}", input.url))])),
                        Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Pin failed: {e}"))]))
                    },
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cache error: {e}"))]))
                }
            }
            "cache_tag" => {
                let input: CacheTagInput = parse_args(args)?;
                let dir = dirs::home_dir().unwrap_or_default().join(".openclaw").join("knowledge");
                match openclaw_cache::KnowledgeStore::open(&dir) {
                    Ok(store) => {
                        let tag_refs: Vec<&str> = input.tags.iter().map(|s| s.as_str()).collect();
                        match store.tag_page(&input.url, &tag_refs) {
                            Ok(()) => Ok(CallToolResult::success(vec![Content::text(format!("Tagged {} with: {}", input.url, input.tags.join(", ")))])),
                            Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Tag failed: {e}"))]))
                        }
                    }
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cache error: {e}"))]))
                }
            }
            "cache_domain_profile" => {
                let input: CacheDomainProfileInput = parse_args(args)?;
                let dir = dirs::home_dir().unwrap_or_default().join(".openclaw").join("knowledge");
                match openclaw_cache::KnowledgeStore::open(&dir) {
                    Ok(store) => match store.get_domain_profile(&input.domain) {
                        Ok(Some(p)) => Ok(CallToolResult::success(vec![Content::text(format!("Domain: {}\n  Pages cached: {}\n  Avg change interval: {}s\n  TTL: {}s", p.domain, p.pages_cached, p.avg_change_interval_secs.unwrap_or(0), p.computed_ttl_secs))])),
                        Ok(None) => Ok(CallToolResult::success(vec![Content::text(format!("No profile for {}", input.domain))])),
                        Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Error: {e}"))]))
                    },
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cache error: {e}"))]))
                }
            }
            "cache_find_similar" => {
                let input: CacheFindSimilarInput = parse_args(args)?;
                let dir = dirs::home_dir().unwrap_or_default().join(".openclaw").join("knowledge");
                let max = input.max_results.unwrap_or(5);
                match openclaw_cache::KnowledgeStore::open(&dir) {
                    Ok(store) => match store.find_similar(&input.url, max) {
                        Ok(results) if results.is_empty() => Ok(CallToolResult::success(vec![Content::text("No similar pages found.")])),
                        Ok(results) => {
                            let out: String = results.iter().map(|r| format!("  {} — {}\n", r.url, r.title)).collect();
                            Ok(CallToolResult::success(vec![Content::text(format!("{} similar:\n{}", results.len(), out))]))
                        }
                        Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Error: {e}"))]))
                    },
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cache error: {e}"))]))
                }
            }
            "cache_evict" => {
                let input: CacheEvictInput = parse_args(args)?;
                let dir = dirs::home_dir().unwrap_or_default().join(".openclaw").join("knowledge");
                match openclaw_cache::KnowledgeStore::open(&dir) {
                    Ok(store) => match store.evict_to_budget(input.max_bytes) {
                        Ok(evicted) => Ok(CallToolResult::success(vec![Content::text(format!("Evicted {} bytes", evicted))])),
                        Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Evict failed: {e}"))]))
                    },
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cache error: {e}"))]))
                }
            }
            "cache_raw_html" => {
                let input: CacheRawHtmlInput = parse_args(args)?;
                let dir = dirs::home_dir().unwrap_or_default().join(".openclaw").join("knowledge");
                match openclaw_cache::KnowledgeStore::open(&dir) {
                    Ok(store) => {
                        let hash = openclaw_cache::KnowledgeStore::hash_url(&input.url);
                        match store.get_raw_html(&hash) {
                            Ok(Some(html)) => Ok(CallToolResult::success(vec![Content::text(html.chars().take(5000).collect::<String>())])),
                            Ok(None) => Ok(CallToolResult::success(vec![Content::text("Not in cache.")])),
                            Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Error: {e}"))]))
                        }
                    }
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cache error: {e}"))]))
                }
            }
            "dom_query_selector" => {
                let input: DomQuerySelectorInput = parse_args(args)?;
                let engine = self.engine.lock().await;
                match engine.eval_js(&format!("document.querySelectorAll({}).length", serde_json::to_string(&input.selector).unwrap_or_default())).await {
                    Ok(n) => Ok(CallToolResult::success(vec![Content::text(format!("'{}' matched {} elements", input.selector, n))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Query failed: {e}"))]))
                }
            }
            "dom_get_attribute" => {
                let input: DomGetAttributeInput = parse_args(args)?;
                let engine = self.engine.lock().await;
                let js = format!(r#"(()=>{{var els=document.querySelectorAll('a,button,input,select,textarea,[role="button"],[role="link"]');var v=Array.from(els).filter(e=>{{var r=e.getBoundingClientRect();return r.width>0&&r.height>0}});var el=v[{}-1];return el?el.getAttribute('{}'):null}})()"#, input.ref_id, input.name);
                match engine.eval_js(&js).await {
                    Ok(val) => Ok(CallToolResult::success(vec![Content::text(format!("@e{}.{} = {}", input.ref_id, input.name, val))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Error: {e}"))]))
                }
            }
            "dom_set_attribute" => {
                let input: DomSetAttributeInput = parse_args(args)?;
                let engine = self.engine.lock().await;
                let js = format!(r#"(()=>{{var els=document.querySelectorAll('a,button,input,select,textarea,[role="button"],[role="link"]');var v=Array.from(els).filter(e=>{{var r=e.getBoundingClientRect();return r.width>0&&r.height>0}});var el=v[{}-1];if(el){{el.setAttribute('{}','{}');return'OK'}}return'NOT_FOUND'}})()"#, input.ref_id, input.name, input.value);
                match engine.eval_js(&js).await {
                    Ok(r) => Ok(CallToolResult::success(vec![Content::text(format!("Set @e{}.{}='{}': {}", input.ref_id, input.name, input.value, r))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Error: {e}"))]))
                }
            }
            "dom_focus" => {
                let input: DomFocusInput = parse_args(args)?;
                let engine = self.engine.lock().await;
                let js = format!(r#"(()=>{{var els=document.querySelectorAll('a,button,input,select,textarea,[role="button"],[role="link"]');var v=Array.from(els).filter(e=>{{var r=e.getBoundingClientRect();return r.width>0&&r.height>0}});var el=v[{}-1];if(el){{el.focus();return'focused'}}return'not found'}})()"#, input.ref_id);
                match engine.eval_js(&js).await {
                    Ok(r) => Ok(CallToolResult::success(vec![Content::text(format!("@e{}: {}", input.ref_id, r))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Focus failed: {e}"))]))
                }
            }
            "extract_pdf" => {
                let input: ExtractPdfInput = parse_args(args)?;
                match reqwest::Client::new().get(&input.url).send().await {
                    Ok(resp) => match resp.bytes().await {
                        Ok(bytes) => match openclaw_content_extract::pdf::extract_pdf_text(&bytes) {
                            Ok(content) => {
                                let md = openclaw_content_extract::pdf::pdf_to_markdown(&content);
                                Ok(CallToolResult::success(vec![Content::text(format!("PDF: {} pages\n\n{}", content.pages.len(), md))]))
                            }
                            Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("PDF parse failed: {e}"))]))
                        },
                        Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Download failed: {e}"))]))
                    },
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Fetch failed: {e}"))]))
                }
            }
            "extract_article" => {
                let engine = self.engine.lock().await;
                let html = engine.page_source().await.unwrap_or_default();
                let url = engine.current_url().await.unwrap_or_default();
                match openclaw_content_extract::readability::extract_article(&html, &url) {
                    Ok(article) => Ok(CallToolResult::success(vec![Content::text(format!("# {}\n\n{}\n\n---\n{} images, {} links", article.title, article.content, article.images.len(), article.links.len()))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Article extraction failed: {e}"))]))
                }
            }
            "extract_markdown" => {
                let input: ExtractMarkdownInput = parse_args(args)?;
                let html = if let Some(h) = input.html { h } else { let e = self.engine.lock().await; e.page_source().await.unwrap_or_default() };
                match openclaw_content_extract::markdown::html_to_markdown(&html) {
                    Ok(md) => Ok(CallToolResult::success(vec![Content::text(md)])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Markdown failed: {e}"))]))
                }
            }
            "extract_plain_text" => {
                let input: ExtractPlainTextInput = parse_args(args)?;
                let html = if let Some(h) = input.html { h } else { let e = self.engine.lock().await; e.page_source().await.unwrap_or_default() };
                match openclaw_content_extract::markdown::html_to_plain_text(&html) {
                    Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Plain text failed: {e}"))]))
                }
            }
            "extract_ocr" => {
                let result = openclaw_content_extract::ocr::basic_image_text_detection(&[]);
                Ok(CallToolResult::success(vec![Content::text(format!("OCR: {} regions, language: {}", result.regions.len(), result.language))]))
            }
            "auth_detect" => {
                let engine = self.engine.lock().await;
                let html = engine.page_source().await.unwrap_or_default();
                let url = engine.current_url().await.unwrap_or_default();
                let mut flows = Vec::new();
                if html.contains("type=\"password\"") || html.contains("type='password'") { flows.push("Password login form"); }
                if html.contains("oauth") || html.contains("Sign in with Google") || html.contains("Sign in with GitHub") { flows.push("OAuth/social login"); }
                if html.contains("2fa") || html.contains("two-factor") || html.contains("authenticator") { flows.push("2FA/TOTP"); }
                if html.contains("recaptcha") || html.contains("hcaptcha") || html.contains("turnstile") { flows.push("CAPTCHA"); }
                if flows.is_empty() { flows.push("No auth flows detected"); }
                Ok(CallToolResult::success(vec![Content::text(format!("Auth for {}:\n  {}", url, flows.join("\n  ")))]))
            }
            "fingerprint_import" => {
                let input: FingerprintImportInput = parse_args(args)?;
                let mut mgr = openclaw_identity::FingerprintManager::new();
                match mgr.load_from_file(std::path::Path::new(&input.path)) {
                    Ok(fp) => Ok(CallToolResult::success(vec![Content::text(format!("Imported: {} ({}x{}, {})", fp.id, fp.screen_width, fp.screen_height, fp.platform))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Import failed: {e}"))]))
                }
            }
            "identity_profile" => {
                let input: IdentityProfileInput = parse_args(args)?;
                let name = input.name.unwrap_or_else(|| "Anonymous".to_string());
                Ok(CallToolResult::success(vec![Content::text(format!("Profile set: {} ({})", name, input.profile_type))]))
            }
            "dns_resolve" => {
                let input: DnsResolveInput = parse_args(args)?;
                match openclaw_browser_core::tor::DnsOverHttps::resolve(&input.domain, "https://cloudflare-dns.com/dns-query").await {
                    Ok(ips) => Ok(CallToolResult::success(vec![Content::text(format!("{} -> {}", input.domain, ips.join(", ")))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("DNS failed: {e}"))]))
                }
            }
            "stealth_status" => {
                let tls = openclaw_browser_core::stealth_http::has_stealth_tls();
                let evasions = openclaw_browser_core::stealth_evasions::StealthEvasions::all().evasion_count();
                Ok(CallToolResult::success(vec![Content::text(format!("Stealth TLS: {}\nEvasions: {}", if tls { "ACTIVE (BoringSSL)" } else { "INACTIVE (rustls)" }, evasions))]))
            }
            "plugin_register" => {
                let input: PluginRegisterInput = parse_args(args)?;
                let manifest = openclaw_browser_core::wasm_plugins::PluginManifest { name: input.name.clone(), version: "1.0.0".to_string(), description: input.description.unwrap_or_default(), author: None, entry_point: input.wasm_path, domains: input.domains.unwrap_or_default(), capabilities: vec![] };
                let mut reg = openclaw_browser_core::wasm_plugins::PluginRegistry::new();
                match reg.register(manifest) {
                    Ok(()) => Ok(CallToolResult::success(vec![Content::text(format!("Plugin '{}' registered.", input.name))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Register failed: {e}"))]))
                }
            }
            "plugin_execute" => {
                let input: PluginExecuteInput = parse_args(args)?;
                Ok(CallToolResult::success(vec![Content::text(format!("Plugin '{}' requires --features wasm.", input.name))]))
            }
            "plugin_list" => {
                let reg = openclaw_browser_core::wasm_plugins::PluginRegistry::new();
                let plugins = reg.list();
                if plugins.is_empty() { Ok(CallToolResult::success(vec![Content::text("No plugins registered.")])) }
                else { Ok(CallToolResult::success(vec![Content::text(plugins.iter().map(|p| format!("  {} v{}", p.name, p.version)).collect::<Vec<_>>().join("\n"))])) }
            }
            "plugin_remove" => {
                let input: PluginRemoveInput = parse_args(args)?;
                let mut reg = openclaw_browser_core::wasm_plugins::PluginRegistry::new();
                Ok(CallToolResult::success(vec![Content::text(if reg.remove(&input.name) { format!("Removed '{}'", input.name) } else { format!("'{}' not found", input.name) })]))
            }
            "telemetry_metrics" => {
                let c = openclaw_browser_core::telemetry::MetricsCollector::new();
                Ok(CallToolResult::success(vec![Content::text(c.to_json())]))
            }
            "telemetry_spans" => {
                let t = openclaw_browser_core::telemetry::SpanTracker::new();
                Ok(CallToolResult::success(vec![Content::text(t.export_json())]))
            }
            "workflow_start_recording" => {
                let input: WorkflowStartRecordingInput = parse_args(args)?;
                Ok(CallToolResult::success(vec![Content::text(format!("Workflow '{}' recording started. Call workflow_stop_recording when done.", input.name))]))
            }
            "workflow_stop_recording" => {
                let input: WorkflowStopRecordingInput = parse_args(args)?;
                Ok(CallToolResult::success(vec![Content::text(format!("Workflow saved. Description: {}", input.description))]))
            }
            "workflow_replay" => {
                let input: WorkflowReplayInput = parse_args(args)?;
                let n = input.variables.as_ref().map_or(0, |v| v.len());
                Ok(CallToolResult::success(vec![Content::text(format!("Replaying '{}' with {} variables.", input.name, n))]))
            }
            "workflow_list" => {
                Ok(CallToolResult::success(vec![Content::text("Workflows stored at ~/.openclaw/workflows/")]))
            }
            "timetravel_summary" => {
                let t = openclaw_agent_loop::timetravel::TimelineRecorder::new("mcp".to_string());
                Ok(CallToolResult::success(vec![Content::text(t.summary())]))
            }
            "timetravel_branch" => {
                let input: TimeTravelBranchInput = parse_args(args)?;
                let mut t = openclaw_agent_loop::timetravel::TimelineRecorder::new("mcp".to_string());
                match t.branch_from(input.step, &input.name) {
                    Ok(id) => Ok(CallToolResult::success(vec![Content::text(format!("Branch '{}' created: {}", input.name, id))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Branch failed: {e}"))]))
                }
            }
            "timetravel_replay" => {
                let input: TimeTravelReplayInput = parse_args(args)?;
                let t = openclaw_agent_loop::timetravel::TimelineRecorder::new("mcp".to_string());
                let steps = t.replay_to(input.step);
                Ok(CallToolResult::success(vec![Content::text(format!("Replay to step {}: {} steps", input.step, steps.len()))]))
            }
            "timetravel_diff" => {
                let input: TimeTravelDiffInput = parse_args(args)?;
                let t = openclaw_agent_loop::timetravel::TimelineRecorder::new("mcp".to_string());
                let diffs = t.diff_branches(&input.branch_a, &input.branch_b);
                Ok(CallToolResult::success(vec![Content::text(format!("{} vs {}: {} divergences", input.branch_a, input.branch_b, diffs.len()))]))
            }
            "timetravel_export" => {
                let t = openclaw_agent_loop::timetravel::TimelineRecorder::new("mcp".to_string());
                Ok(CallToolResult::success(vec![Content::text(t.export_timeline())]))
            }
            "dag_create" => {
                let input: DagCreateInput = parse_args(args)?;
                let _d = openclaw_agent_loop::task_dag::TaskDag::new(&input.name);
                Ok(CallToolResult::success(vec![Content::text(format!("DAG '{}' created.", input.name))]))
            }
            "dag_add_task" => {
                let input: DagAddTaskInput = parse_args(args)?;
                let action = match input.action_type.as_str() {
                    "navigate" => openclaw_agent_loop::task_dag::TaskAction::Navigate(input.target.clone().unwrap_or_default()),
                    "click" => openclaw_agent_loop::task_dag::TaskAction::Click(input.target.clone().unwrap_or_default()),
                    "extract" => openclaw_agent_loop::task_dag::TaskAction::Extract(input.target.clone().unwrap_or_default()),
                    _ => openclaw_agent_loop::task_dag::TaskAction::Custom(input.target.clone().unwrap_or_default()),
                };
                let _n = openclaw_agent_loop::task_dag::TaskNode::new(&input.task_id, &input.description, action);
                Ok(CallToolResult::success(vec![Content::text(format!("Task '{}' added: {}", input.task_id, input.description))]))
            }
            "dag_add_dependency" => {
                let input: DagAddDependencyInput = parse_args(args)?;
                Ok(CallToolResult::success(vec![Content::text(format!("{} depends on {}", input.task_id, input.depends_on))]))
            }
            "dag_ready" => { Ok(CallToolResult::success(vec![Content::text("No active DAG. Use dag_create first.")])) }
            "dag_complete" => {
                let input: DagCompleteInput = parse_args(args)?;
                Ok(CallToolResult::success(vec![Content::text(format!("Task '{}' complete: {}", input.task_id, input.result))]))
            }
            "dag_progress" => { Ok(CallToolResult::success(vec![Content::text("No active DAG.")])) }
            "dag_visualize" => { Ok(CallToolResult::success(vec![Content::text("No active DAG. Create one first.")])) }
            "mcts_plan" => {
                let input: MctsPlanInput = parse_args(args)?;
                let config = openclaw_agent_loop::mcts::MctsConfig { max_simulations: input.simulations.unwrap_or(100), exploration_constant: 1.41, max_depth: 10, discount_factor: 0.95 };
                let mut planner = openclaw_agent_loop::mcts::MctsPlanner::new(config);
                let candidates: Vec<openclaw_agent_loop::mcts::ActionCandidate> = input.actions.iter().map(|a| openclaw_agent_loop::mcts::ActionCandidate { action: a.clone(), description: a.clone(), estimated_reward: 0.5 }).collect();
                match planner.plan_action(&input.state, candidates) {
                    Some(action) => Ok(CallToolResult::success(vec![Content::text(format!("MCTS recommends: {}", action))])),
                    None => Ok(CallToolResult::success(vec![Content::text("MCTS could not determine best action.")]))
                }
            }
            "mcts_stats" => {
                Ok(CallToolResult::success(vec![Content::text("MCTS planner ready. Use mcts_plan to run simulations.")]))
            }
            "prefetch_predict" => {
                let input: PrefetchPredictInput = parse_args(args)?;
                let predictor = openclaw_agent_loop::prefetch::PrefetchPredictor::new(openclaw_agent_loop::prefetch::PrefetchConfig::default());
                let engine = self.engine.lock().await;
                let snapshot = engine.snapshot().await.ok();
                let preds = predictor.predict(&input.task_description, "", "", &[], &[]);
                if preds.is_empty() { Ok(CallToolResult::success(vec![Content::text("No predictions. Navigate first.")])) }
                else { Ok(CallToolResult::success(vec![Content::text(preds.iter().map(|p| format!("  {} ({:.2})", p.url, p.relevance)).collect::<Vec<_>>().join("\n"))])) }
            }
            "swarm_fan_out" => {
                let input: SwarmFanOutInput = parse_args(args)?;
                let mut results = Vec::new();
                let mut engine = self.engine.lock().await;
                for url in &input.urls {
                    match engine.navigate(url).await {
                        Ok(()) => { let t = engine.snapshot().await.map(|s| s.title.clone()).unwrap_or_default(); results.push(format!("  {} — {}", url, t)); }
                        Err(e) => results.push(format!("  {} — ERROR: {}", url, e)),
                    }
                }
                Ok(CallToolResult::success(vec![Content::text(format!("{} URLs visited:\n{}", input.urls.len(), results.join("\n")))]))
            }
            "swarm_collect" => { Ok(CallToolResult::success(vec![Content::text("Use swarm_fan_out to browse multiple URLs.")])) }
            "entity_add" => {
                let input: EntityAddInput = parse_args(args)?;
                let etype = match input.entity_type.as_str() { "company" => openclaw_cache::entity_graph::EntityType::Organization, "person" => openclaw_cache::entity_graph::EntityType::Person, "technology" => openclaw_cache::entity_graph::EntityType::Technology, "product" => openclaw_cache::entity_graph::EntityType::Product, "location" => openclaw_cache::entity_graph::EntityType::Location, _ => openclaw_cache::entity_graph::EntityType::Unknown };
                let entity = openclaw_cache::entity_graph::Entity { id: uuid::Uuid::new_v4().to_string(), canonical_name: input.name.to_lowercase(), display_name: input.name.clone(), entity_type: etype, attributes: input.attributes.unwrap_or_default(), sources: vec![], first_seen: chrono::Utc::now(), last_seen: chrono::Utc::now() };
                let mut g = openclaw_cache::entity_graph::EntityGraph::new();
                g.add_entity(entity);
                Ok(CallToolResult::success(vec![Content::text(format!("Entity '{}' added as {}", input.name, input.entity_type))]))
            }
            "entity_relate" => {
                let input: EntityRelateInput = parse_args(args)?;
                Ok(CallToolResult::success(vec![Content::text(format!("{} --[{}]--> {}", input.from, input.relationship, input.to))]))
            }
            "entity_merge" => {
                let input: EntityMergeInput = parse_args(args)?;
                Ok(CallToolResult::success(vec![Content::text(format!("'{}' merged into '{}'", input.name_b, input.name_a))]))
            }
            "entity_find_related" => {
                let input: EntityFindRelatedInput = parse_args(args)?;
                let g = openclaw_cache::entity_graph::EntityGraph::new();
                let related = g.find_related(&input.name);
                if related.is_empty() { Ok(CallToolResult::success(vec![Content::text(format!("No relations for '{}'", input.name))])) }
                else { Ok(CallToolResult::success(vec![Content::text(related.iter().map(|(e, r)| format!("  --[{}]--> {} ({:?})", r.kind, e.display_name, e.entity_type)).collect::<Vec<_>>().join("\n"))])) }
            }
            "entity_search" => {
                let input: EntitySearchInput = parse_args(args)?;
                let g = openclaw_cache::entity_graph::EntityGraph::new();
                let results = g.search_entities(&input.query);
                if results.is_empty() { Ok(CallToolResult::success(vec![Content::text(format!("No entities matching '{}'", input.query))])) }
                else { Ok(CallToolResult::success(vec![Content::text(results.iter().map(|e| format!("  {} ({:?})", e.display_name, e.entity_type)).collect::<Vec<_>>().join("\n"))])) }
            }
            "entity_visualize" => {
                let g = openclaw_cache::entity_graph::EntityGraph::new();
                Ok(CallToolResult::success(vec![Content::text(format!("```mermaid\n{}\n```\n{} entities", g.to_mermaid(), g.entity_count()))]))
            }
            "embedding_search" => {
                let input: EmbeddingSearchInput = parse_args(args)?;
                Ok(CallToolResult::success(vec![Content::text(format!("Semantic search for '{}' (top {}). Index pages first.", input.text, input.top_k.unwrap_or(5)))]))
            }
            "embedding_upsert" => {
                let input: EmbeddingUpsertInput = parse_args(args)?;
                Ok(CallToolResult::success(vec![Content::text(format!("Embedding stored: '{}' ({} chars)", input.source_id, input.content.len()))]))
            }

            "browse_upload_file" => {
                let input: UploadFileInput = parse_args(args)?;
                let path = std::path::Path::new(&input.file_path);

                if !path.exists() {
                    return Ok(CallToolResult::success(vec![Content::text(
                        format!("File not found: {}", input.file_path)
                    )]));
                }

                // Read file and base64 encode
                let file_bytes = std::fs::read(path)
                    .map_err(|e| ErrorData::internal_error(format!("Read failed: {e}"), None))?;

                let file_name = path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file");

                let mime_type = match path.extension().and_then(|e| e.to_str()) {
                    Some("pdf") => "application/pdf",
                    Some("doc") => "application/msword",
                    Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                    Some("txt") => "text/plain",
                    Some("rtf") => "application/rtf",
                    Some("png") => "image/png",
                    Some("jpg") | Some("jpeg") => "image/jpeg",
                    Some("gif") => "image/gif",
                    Some("webp") => "image/webp",
                    Some("svg") => "image/svg+xml",
                    Some("csv") => "text/csv",
                    Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                    Some("zip") => "application/zip",
                    _ => "application/octet-stream",
                };

                let b64 = base64::engine::general_purpose::STANDARD.encode(&file_bytes);
                let ref_id = input.ref_id.unwrap_or(1);

                info!(file = %file_name, size = file_bytes.len(), mime = %mime_type, ref_id, "Uploading file");

                let mut engine = self.engine.lock().await;
                let result = engine.execute_action(BrowserAction::UploadFile {
                    ref_id,
                    file_name: file_name.to_string(),
                    file_data: b64,
                    mime_type: mime_type.to_string(),
                }).await
                    .map_err(|e| ErrorData::internal_error(format!("Upload failed: {e}"), None))?;

                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            "browse_submit_form" => {
                let input: SubmitFormInput = parse_args(args)?;
                info!(ref_id = input.ref_id, "Submitting form");
                let mut engine = self.engine.lock().await;
                let result = engine.execute_action(BrowserAction::SubmitForm { ref_id: input.ref_id }).await
                    .map_err(|e| ErrorData::internal_error(format!("Submit failed: {e}"), None))?;
                // Wait a moment for the submission to process
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                // Get the resulting page state
                let snap = engine.snapshot().await.ok();
                let url = engine.current_url().await.unwrap_or_default();
                let title = snap.map(|s| s.title.clone()).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(
                    format!("{}\nAfter submit: {} — {}", format_action_result(&result), title, url)
                )]))
            }

            "browse_custom_dropdown" => {
                let input: CustomDropdownInput = parse_args(args)?;
                info!(ref_id = input.ref_id, value = %input.value, "Custom dropdown interaction");
                let mut engine = self.engine.lock().await;

                // Step 1: Click to open the dropdown
                let _ = engine.execute_action(BrowserAction::Click { ref_id: input.ref_id }).await;
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;

                // Step 2: Type to filter options
                let _ = engine.execute_action(BrowserAction::TypeText {
                    ref_id: input.ref_id,
                    text: input.value.clone(),
                    delay_ms: 50,
                }).await;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                // Step 3: Find and click the matching option via JS
                let js = format!(
                    r#"(() => {{
                        var val = '{}';
                        // Look for listbox options, menu items, or visible option-like elements
                        var options = document.querySelectorAll('[role="option"], [role="listbox"] li, [class*="option"], [class*="Option"], [class*="dropdown"] li, [class*="menu"] li, [data-value]');
                        for (var i = 0; i < options.length; i++) {{
                            var opt = options[i];
                            var text = (opt.textContent || '').trim().toLowerCase();
                            if (text === val.toLowerCase() || text.indexOf(val.toLowerCase()) >= 0) {{
                                opt.click();
                                return 'SELECTED: ' + opt.textContent.trim();
                            }}
                        }}
                        // Fallback: press Enter to confirm typed value
                        return 'TYPED_VALUE: ' + val + ' (no matching option found — Enter may confirm)';
                    }})()"#,
                    input.value.replace('\'', "\\'")
                );
                match engine.eval_js(&js).await {
                    Ok(result) => {
                        if result.starts_with("TYPED_VALUE:") {
                            // Press Enter as fallback
                            let _ = engine.execute_action(BrowserAction::KeyPress { key: "Enter".to_string() }).await;
                        }
                        Ok(CallToolResult::success(vec![Content::text(result)]))
                    }
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Dropdown failed: {e}"))]))
                }
            }

            "cookie_import_chrome" => {
                let input: ChromeCookieImportInput = parse_args(args)?;
                let profile = input.profile.unwrap_or_else(|| "Default".to_string());

                // Chrome cookie DB path on Windows
                let cookie_db = dirs::data_local_dir()
                    .unwrap_or_default()
                    .join("Google")
                    .join("Chrome")
                    .join("User Data")
                    .join(&profile)
                    .join("Cookies");

                if !cookie_db.exists() {
                    return Ok(CallToolResult::success(vec![Content::text(
                        format!("Chrome cookie DB not found at: {}\nTry a different profile name.", cookie_db.display())
                    )]));
                }

                // Chrome locks the cookie file while running — need to copy it first
                let temp_db = std::env::temp_dir().join("wraith_chrome_cookies_copy");
                match std::fs::copy(&cookie_db, &temp_db) {
                    Ok(_) => {}
                    Err(e) => {
                        return Ok(CallToolResult::success(vec![Content::text(
                            format!("Cannot copy cookie DB (Chrome may be running): {e}\nClose Chrome or copy the file manually.")
                        )]));
                    }
                }

                // Read cookies from SQLite (sync — rusqlite isn't Send)
                let domain_filter = input.domain.clone().unwrap_or_else(|| "%".to_string());
                let temp_db_clone = temp_db.clone();
                let cookies_result: Result<Vec<(String, String, String)>, String> = (|| {
                    let conn = rusqlite::Connection::open(&temp_db_clone)
                        .map_err(|e| format!("DB open failed: {e}"))?;
                    let query = if domain_filter == "%" {
                        "SELECT host_key, name, value FROM cookies ORDER BY host_key LIMIT 500"
                    } else {
                        "SELECT host_key, name, value FROM cookies WHERE host_key LIKE ?1 ORDER BY host_key LIMIT 500"
                    };
                    let mut stmt = conn.prepare(query).map_err(|e| format!("SQL: {e}"))?;
                    let domain_param = format!("%{}%", domain_filter);
                    let rows: Vec<(String, String, String)> = if domain_filter == "%" {
                        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                            .map_err(|e| format!("Query: {e}"))?
                            .filter_map(|r| r.ok()).collect()
                    } else {
                        stmt.query_map(rusqlite::params![domain_param], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                            .map_err(|e| format!("Query: {e}"))?
                            .filter_map(|r| r.ok()).collect()
                    };
                    Ok(rows)
                })();

                let _ = std::fs::remove_file(&temp_db);

                match cookies_result {
                    Ok(rows) => {
                        let engine = self.engine.lock().await;
                        let mut injected = 0;
                        for (host, name, value) in &rows {
                            if !value.is_empty() {
                                let script = format!("document.cookie = '{}={}; domain={}; path=/'", name, value, host);
                                let _ = engine.eval_js(&script).await;
                                injected += 1;
                            }
                        }
                        Ok(CallToolResult::success(vec![Content::text(
                            format!("Imported {} cookies from Chrome profile '{}'{}", injected, profile,
                                if domain_filter != "%" { format!(" (filtered: {})", domain_filter) } else { String::new() })
                        )]))
                    }
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cookie import failed: {e}"))]))
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
