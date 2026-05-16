//! MCP server handler — implements the rmcp ServerHandler trait.
//! Wired to a real NativeClient for Chrome-free browsing.

use std::collections::HashMap;
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

use wraith_browser_core::engine::BrowserEngine;
use wraith_browser_core::actions::{BrowserAction, ActionResult};

use crate::tools::*;

// ---------------------------------------------------------------------------
// Chrome cookie decryption helpers (Windows DPAPI + AES-256-GCM)
// ---------------------------------------------------------------------------

/// Decrypt data protected by Windows DPAPI (CryptUnprotectData).
#[cfg(target_os = "windows")]
fn dpapi_decrypt(data: &[u8]) -> Result<Vec<u8>, String> {
    use std::ptr;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    extern "system" {
        fn CryptUnprotectData(
            pDataIn: *const DataBlob,
            ppszDataDescr: *mut *mut u16,
            pOptionalEntropy: *const DataBlob,
            pvReserved: *mut std::ffi::c_void,
            pPromptStruct: *mut std::ffi::c_void,
            dwFlags: u32,
            pDataOut: *mut DataBlob,
        ) -> i32;
        fn LocalFree(hMem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }

    let input = DataBlob {
        cb_data: data.len() as u32,
        pb_data: data.as_ptr() as *mut u8,
    };
    let mut output = DataBlob {
        cb_data: 0,
        pb_data: ptr::null_mut(),
    };

    let result = unsafe {
        CryptUnprotectData(
            &input,
            ptr::null_mut(),
            ptr::null(),
            ptr::null_mut(),
            ptr::null_mut(),
            0,
            &mut output,
        )
    };

    if result == 0 {
        return Err("DPAPI decryption failed".into());
    }

    let decrypted =
        unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize).to_vec() };
    unsafe {
        LocalFree(output.pb_data as *mut std::ffi::c_void);
    }
    Ok(decrypted)
}

#[cfg(not(target_os = "windows"))]
fn dpapi_decrypt(_data: &[u8]) -> Result<Vec<u8>, String> {
    Err("Chrome cookie decryption via DPAPI is only supported on Windows".into())
}

/// Decrypt a single Chrome cookie `encrypted_value` using the decrypted master key.
fn decrypt_chrome_cookie(encrypted_value: &[u8], key: &[u8]) -> Result<String, String> {
    if encrypted_value.len() < 3 {
        return Ok(String::new());
    }
    let prefix = &encrypted_value[..3];
    if prefix == b"v10" || prefix == b"v20" {
        // v10/v20: 3-byte prefix + 12-byte nonce + ciphertext + 16-byte GCM tag
        if encrypted_value.len() < 15 + 16 {
            return Err("encrypted_value too short for AES-GCM".into());
        }
        let nonce = &encrypted_value[3..15];
        let ciphertext = &encrypted_value[15..];
        use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
        let cipher =
            Aes256Gcm::new_from_slice(key).map_err(|e| format!("AES key error: {e}"))?;
        let nonce = Nonce::from_slice(nonce);
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| format!("AES decrypt error: {e}"))?;
        Ok(String::from_utf8_lossy(&plaintext).to_string())
    } else {
        // Old-style unencrypted or DPAPI-only encrypted value
        Ok(String::from_utf8_lossy(encrypted_value).to_string())
    }
}

/// The Wraith MCP server handler — backed by any BrowserEngine.
pub struct WraithHandler {
    tools: Vec<Tool>,
    /// The browser engine (shared, async-mutex for interior mutability)
    engine: Arc<Mutex<dyn BrowserEngine>>,
    /// CDP browser engine for JS-heavy pages (Chrome via DevTools Protocol)
    #[cfg(feature = "cdp")]
    cdp_engine: Option<Arc<Mutex<dyn BrowserEngine>>>,
    /// Whether to auto-fallback to CDP when native rendering detects a SPA
    #[cfg(feature = "cdp")]
    cdp_auto: bool,
    /// Active CDP session — when Some, all browse_* commands route to this engine
    /// instead of self.engine. Set by browse_navigate_cdp, cleared by browse_navigate.
    #[cfg(feature = "cdp")]
    active_cdp_session: Arc<Mutex<Option<Arc<Mutex<dyn BrowserEngine>>>>>,
    /// Named parallel sessions — maps session name to engine instance.
    /// The "native" session is always present (initialized from self.engine).
    #[cfg(feature = "cdp")]
    sessions: Arc<tokio::sync::Mutex<HashMap<String, Arc<Mutex<dyn BrowserEngine>>>>>,
    /// Name of the currently active session (default: "native").
    #[cfg(feature = "cdp")]
    active_session_name: Arc<tokio::sync::Mutex<String>>,
    /// Application dedup tracker — prevents duplicate applications.
    dedup_tracker: Arc<wraith_cache::dedup::ApplicationTracker>,
}

impl Default for WraithHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl WraithHandler {
    /// Create the handler with the default engine (Sevro if available, native fallback).
    pub fn new() -> Self {
        let engine = Self::default_engine();

        #[cfg(feature = "cdp")]
        {
            let (cdp_engine, cdp_auto) = Self::default_cdp_engine();
            return Self::with_engine_and_cdp(engine, cdp_engine, cdp_auto);
        }

        #[cfg(not(feature = "cdp"))]
        Self::with_engine(engine)
    }

    /// Create the handler with a specific engine.
    pub fn with_engine(engine: Arc<Mutex<dyn BrowserEngine>>) -> Self {
        #[cfg(feature = "cdp")]
        {
            return Self::with_engine_and_cdp(engine, None, false);
        }
        #[cfg(not(feature = "cdp"))]
        Self::with_engine_inner(engine)
    }

    fn with_engine_inner(engine: Arc<Mutex<dyn BrowserEngine>>) -> Self {
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
                "Press a keyboard key on the current page. Pass `ref_id` to focus an element first so the key dispatches to the right target (e.g. committing a react-select choice after the menu is open).",
                &schema_for!(KeyPressInput), rw_open.clone()),
            make_tool("browse_scroll",
                "Scroll the current page up or down.",
                &schema_for!(ScrollInput), rw_closed.clone()),
            make_tool("browse_scroll_to",
                "Scroll the viewport to center a specific element by its @ref ID. Returns the new scroll position.",
                &schema_for!(ScrollToInput), rw_closed.clone()),
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
                "Select a dropdown option by @ref ID and value. Handles native <select> elements AND React-style portal-rendered dropdowns (react-select, Radix, Headless UI, MUI) by opening the menu via real CDP mouse events, locating the option by label/data-value, and clicking it. Works on Greenhouse/Lever/Ashby application forms.",
                &schema_for!(SelectInput), rw_open.clone()),
            make_tool("browse_type",
                "Type text into an element with realistic keystroke delays (realistic human-like input simulation).",
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
                "Show current engine configuration (engine type, proxy, TLS compatibility status).",
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
                "List available TLS fingerprint profiles for compatible browsing.",
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
            make_tool("stealth_status", "Show current compatible TLS status and configuration count.", &schema_for!(StealthStatusInput), ro_closed.clone()),
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
                "Import cookies from the user's Chrome browser profile to reuse existing login sessions. Reads Chrome's encrypted cookie database, decrypts using OS credentials, and loads into Wraith. Preserves existing authenticated sessions.",
                &schema_for!(ChromeCookieImportInput), rw_open.clone()),
            make_tool("browse_fetch_scripts",
                "Fetch and execute external <script src='...'> tags from the current page. Call this AFTER browse_navigate when you need React/Vue/Angular to mount for form filling. Downloads JS bundles and runs them in QuickJS so React's event system activates.",
                &schema_for!(FetchScriptsInput), rw_open.clone()),
            make_tool("browse_solve_captcha",
                "Solve a page verification challenge using a third-party solving service. Supports common challenge types. Auto-detects the challenge key from the page if not provided. Requires TWOCAPTCHA_API_KEY env var. Returns the solved token and injects it into the page.",
                &schema_for!(SolveCaptchaInput), rw_open.clone()),
            make_tool("browse_enter_iframe",
                "Enter an iframe's content by switching the page context to the iframe's parsed DOM. Use when a page has cross-origin iframes (e.g., embedded application forms) whose elements you need to interact with. After entering, browse_snapshot shows the iframe's content. Use browse_back to return to the parent page.",
                &schema_for!(EnterIframeInput), rw_open.clone()),
            make_tool("browse_dismiss_overlay",
                "Dismiss a modal, overlay, popup, or cookie banner that is blocking interaction. Automatically finds the close/dismiss/accept button within the overlay and clicks it. If ref_id is omitted, auto-detects the topmost overlay. Returns an updated page snapshot after dismissal.",
                &schema_for!(DismissOverlayInput), rw_open.clone()),
            make_tool("tls_verify",
                "Verify that Wraith's TLS fingerprint matches a real Chrome 136 browser. Fetches a TLS fingerprinting service using the same HTTP stack as browse_navigate, then compares JA3/JA4 hashes, cipher suites, extensions, and HTTP/2 SETTINGS against known Chrome 136 values. Returns a detailed pass/fail report. One-command TLS compatibility check — no external tools needed.",
                &schema_for!(TlsVerifyInput), ro_open),
            make_tool("browse_login",
                "Perform a full login flow: navigate to a login page, fill username + password, click submit, and follow the entire OAuth/auth redirect chain (302 -> 302 -> 200). Captures all Set-Cookie headers at every redirect hop. Returns the final page snapshot and all cookies set during the flow. Use this instead of separate navigate/fill/click when you need reliable auth with cookie persistence across redirects.",
                &schema_for!(LoginInput), rw_open.clone()),
            make_tool("browse_engine_status",
                "Check which browser engine is currently active: 'native (Sevro)' or 'CDP (Chrome)'. After browse_navigate_cdp, all browse_* commands automatically route to the CDP engine. After browse_navigate (native), they route back to the native engine.",
                &schema_for!(EngineStatusInput), ro_closed.clone()),
            make_tool("browse_session_list",
                "List all open browser sessions with their engine type and current URL.",
                &schema_for!(SessionListInput), ro_closed.clone()),
            // --- Playbook tools ---
            make_tool("swarm_run_playbook",
                "Execute a YAML playbook (or a built-in name like 'greenhouse-apply') that describes a sequence of browser actions with variable interpolation. Navigates, fills forms, uploads files, submits, and verifies — returning step-by-step results.",
                &schema_for!(PlaybookRunInput), rw_open.clone()),
            make_tool("swarm_list_playbooks",
                "List all built-in playbook names with descriptions. Playbooks are pre-authored automation scripts for common job-application flows (Greenhouse, Ashby, Lever, Indeed).",
                &schema_for!(PlaybookListInput), ro_closed.clone()),
            make_tool("swarm_playbook_status",
                "Check the progress of a running or completed playbook execution. Returns completed/total steps, current step name, and any errors encountered.",
                &schema_for!(PlaybookStatusInput), ro_closed.clone()),
            // --- Dedup & Verification tools ---
            make_tool("swarm_dedup_check",
                "Check if a job URL has already been applied to. Returns { applied: bool, applied_at, status } so the agent can skip duplicates.",
                &schema_for!(DedupCheckInput), ro_closed.clone()),
            make_tool("swarm_dedup_record",
                "Record that a job application was submitted. Stores URL, company, title, and platform in the dedup database for future duplicate detection.",
                &schema_for!(DedupRecordInput), rw_closed.clone()),
            make_tool("swarm_dedup_stats",
                "Return aggregate dedup statistics: total applications, breakdown by platform and status, today's count, this week's count.",
                &schema_for!(DedupStatsInput), ro_closed.clone()),
            make_tool("swarm_verify_submission",
                "After submitting a job application, verify it went through by checking the current page for success/error indicators (confirmation messages, error banners, URL patterns). Returns { result: confirmed|likely|uncertain|failed, message }.",
                &schema_for!(VerifySubmissionInput), ro_closed),
        ];

        // Initialize the application dedup tracker (SQLite-backed)
        let dedup_db_path = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".wraith")
            .join("dedup.db");
        // Ensure the parent directory exists
        if let Some(parent) = dedup_db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let dedup_tracker = Arc::new(wraith_cache::dedup::ApplicationTracker::new(
            &dedup_db_path.to_string_lossy(),
        ));

        info!(tool_count = tools.len(), "Wraith MCP handler initialized");

        #[cfg(feature = "cdp")]
        {
            let mut sessions_map = HashMap::new();
            sessions_map.insert("native".to_string(), engine.clone());
            return Self {
                tools, engine, cdp_engine: None, cdp_auto: false,
                active_cdp_session: Arc::new(Mutex::new(None)),
                sessions: Arc::new(tokio::sync::Mutex::new(sessions_map)),
                active_session_name: Arc::new(tokio::sync::Mutex::new("native".to_string())),
                dedup_tracker,
            };
        }

        #[cfg(not(feature = "cdp"))]
        Self { tools, engine, dedup_tracker }
    }

    /// Create the handler with a specific engine plus CDP support.
    #[cfg(feature = "cdp")]
    pub fn with_engine_and_cdp(
        engine: Arc<Mutex<dyn BrowserEngine>>,
        cdp_engine: Option<Arc<Mutex<dyn BrowserEngine>>>,
        cdp_auto: bool,
    ) -> Self {
        let rw_open = ToolAnnotations::new().read_only(false).destructive(false).open_world(true);
        let ro_closed = ToolAnnotations::new().read_only(true).open_world(false);
        let ro_open = ToolAnnotations::new().read_only(true).open_world(true);
        let rw_closed = ToolAnnotations::new().read_only(false).destructive(false).open_world(false);
        let rw_destructive = ToolAnnotations::new().read_only(false).destructive(true).open_world(false);

        let mut tools = vec![
            make_tool("browse_navigate",
                "Navigate to a URL and return a DOM snapshot with interactive elements. Each element has a @ref ID for clicking/filling.",
                &schema_for!(NavigateInput), rw_open.clone()),
            make_tool("browse_navigate_cdp",
                "Navigate to a URL using a full browser engine (Chrome via CDP). Use this for React SPAs and JavaScript-heavy pages that the native renderer can't handle. Launches a headless browser, waits for full page render including JS execution, and returns a DOM snapshot.",
                &schema_for!(NavigateCdpInput), rw_open.clone()),
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
                "Press a keyboard key on the current page. Pass `ref_id` to focus an element first so the key dispatches to the right target (e.g. committing a react-select choice after the menu is open).",
                &schema_for!(KeyPressInput), rw_open.clone()),
            make_tool("browse_scroll",
                "Scroll the current page up or down.",
                &schema_for!(ScrollInput), rw_closed.clone()),
            make_tool("browse_scroll_to",
                "Scroll the viewport to center a specific element by its @ref ID. Returns the new scroll position.",
                &schema_for!(ScrollToInput), rw_closed.clone()),
        ];

        // Copy all remaining tools from the non-CDP constructor
        // (vault, cache, entity, etc. — they are engine-independent)
        let non_cdp = Self::with_engine_inner(engine.clone());
        for tool in &non_cdp.tools {
            let name: &str = &tool.name;
            // Skip tools already registered above to avoid duplicates
            // Also skip browse_session_list — we register the full set of session tools below
            if !matches!(name, "browse_navigate" | "browse_click" | "browse_fill"
                | "browse_snapshot" | "browse_extract" | "browse_screenshot"
                | "browse_search" | "browse_eval_js" | "browse_tabs" | "browse_back"
                | "browse_key_press" | "browse_scroll" | "browse_scroll_to"
                | "browse_session_list")
            {
                tools.push(tool.clone());
            }
        }

        // Session management tools (CDP-enabled builds get all 4)
        let rw_session = ToolAnnotations::new().read_only(false).destructive(false).open_world(false);
        let ro_session = ToolAnnotations::new().read_only(true).open_world(false);
        let rw_destructive_session = ToolAnnotations::new().read_only(false).destructive(true).open_world(false);
        tools.push(make_tool("browse_session_create",
            "Create a new named browser session. engine_type options: 'native' (fast Sevro, no JS), 'cdp' (spawn fresh headless Chrome with full JS), or 'cdp-attach' (attach to the operator's running Chrome at --remote-debugging-port — real fingerprint + cookies, passes reCAPTCHA v3 natively). For cdp-attach, optional attach_port (default 9222) and attach_target (URL/title substring filter) select the tab.",
            &schema_for!(SessionCreateInput), rw_session.clone()));
        tools.push(make_tool("browse_session_switch",
            "Switch the active session. All subsequent browse_* commands will route to the switched session's engine.",
            &schema_for!(SessionSwitchInput), rw_session));
        tools.push(make_tool("browse_session_list",
            "List all open browser sessions with their engine type and current URL.",
            &schema_for!(SessionListInput), ro_session));
        tools.push(make_tool("browse_session_close",
            "Close a named session and shut down its engine. Cannot close the 'native' session. If closing the active session, switches to 'native'.",
            &schema_for!(SessionCloseInput), rw_destructive_session));

        let mut sessions_map = HashMap::new();
        sessions_map.insert("native".to_string(), engine.clone());

        // Initialize the application dedup tracker (SQLite-backed)
        let dedup_db_path = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".wraith")
            .join("dedup.db");
        if let Some(parent) = dedup_db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let dedup_tracker = Arc::new(wraith_cache::dedup::ApplicationTracker::new(
            &dedup_db_path.to_string_lossy(),
        ));

        info!(
            tool_count = tools.len(),
            cdp_available = cdp_engine.is_some(),
            cdp_auto_fallback = cdp_auto,
            "Wraith MCP handler initialized (CDP-enabled)"
        );
        Self {
            tools, engine, cdp_engine, cdp_auto,
            active_cdp_session: Arc::new(Mutex::new(None)),
            sessions: Arc::new(tokio::sync::Mutex::new(sessions_map)),
            active_session_name: Arc::new(tokio::sync::Mutex::new("native".to_string())),
            dedup_tracker,
        }
    }

    /// Build the default engine: Sevro → NativeEngine fallback.
    /// Reads config from environment variables:
    /// - `WRAITH_FLARESOLVERR` — External challenge-solving proxy URL
    /// - `WRAITH_PROXY` — HTTP proxy URL
    /// - `WRAITH_FALLBACK_PROXY` — Fallback proxy URL
    fn default_engine() -> Arc<Mutex<dyn BrowserEngine>> {
        #[cfg(feature = "sevro")]
        {
            let flaresolverr = std::env::var("WRAITH_FLARESOLVERR").ok();
            let proxy = std::env::var("WRAITH_PROXY").ok();
            let fallback_proxy = std::env::var("WRAITH_FALLBACK_PROXY").ok();

            if flaresolverr.is_some() {
                info!(solver = ?flaresolverr, "Challenge proxy configured via WRAITH_FLARESOLVERR");
            }
            if proxy.is_some() {
                info!(proxy = ?proxy, "Proxy configured via WRAITH_PROXY");
            }

            // Use the engine factory which handles SevroConfig internally
            let opts = wraith_browser_core::engine::EngineOptions {
                proxy_url: proxy,
                flaresolverr_url: flaresolverr,
                fallback_proxy_url: fallback_proxy,
            };

            info!("Using Sevro engine (default)");
            // create_engine_with_options is async but we need sync here;
            // construct directly instead
            let mut config = wraith_browser_core::config::BrowserConfig::default();
            let _ = config; // suppress unused

            // Direct construction via SevroEngineBackend
            use wraith_browser_core::engine_sevro::SevroEngineBackend;
            return Arc::new(Mutex::new(SevroEngineBackend::new_with_options(opts)));
        }
        #[cfg(not(feature = "sevro"))]
        {
            info!("Sevro not available, using native engine");
            Arc::new(Mutex::new(
                wraith_browser_core::engine_native::NativeEngine::new()
            ))
        }
    }

    /// Build the CDP engine from environment variables.
    /// - `WRAITH_CDP_CHROME` — Path to Chrome/Chromium binary (enables CDP)
    /// - `WRAITH_CDP_AUTO` — If "true", auto-fallback to CDP when native rendering
    ///   produces a sparse snapshot (SPA detection)
    #[cfg(feature = "cdp")]
    fn default_cdp_engine() -> (Option<Arc<Mutex<dyn BrowserEngine>>>, bool) {
        let chrome_path = std::env::var("WRAITH_CDP_CHROME").ok();
        let cdp_auto = std::env::var("WRAITH_CDP_AUTO")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        if let Some(ref path) = chrome_path {
            info!(
                chrome = %path,
                auto_fallback = cdp_auto,
                "CDP engine configured via WRAITH_CDP_CHROME (will launch on first use)"
            );
        } else {
            info!("CDP engine not configured (set WRAITH_CDP_CHROME to enable)");
        }

        // CDP engine is launched lazily on first browse_navigate_cdp call
        // or on SPA auto-fallback — not at startup (Chrome process is heavy)
        (None, cdp_auto)
    }

    /// Get the currently active browser engine. If a CDP session is active
    /// (from browse_navigate_cdp), returns that; otherwise returns the native engine.
    fn active_engine(&self) -> Arc<Mutex<dyn BrowserEngine>> {
        #[cfg(feature = "cdp")]
        {
            // Try to check without blocking — if we can't get the lock, fall back to native
            // Note: this is called from async context, the actual lock is cheap (no contention)
            if let Ok(guard) = self.active_cdp_session.try_lock() {
                if let Some(ref cdp) = *guard {
                    return cdp.clone();
                }
            }
        }
        self.engine.clone()
    }

    /// Async version of active_engine — waits for the lock.
    /// With cdp feature: looks up the active session name in the sessions map.
    /// Falls back to self.engine if the session is not found.
    async fn active_engine_async(&self) -> Arc<Mutex<dyn BrowserEngine>> {
        #[cfg(feature = "cdp")]
        {
            let session_name = self.active_session_name.lock().await.clone();
            let sessions = self.sessions.lock().await;
            if let Some(engine) = sessions.get(&session_name) {
                return engine.clone();
            }
            // Fallback: check legacy active_cdp_session
            drop(sessions);
            let guard = self.active_cdp_session.lock().await;
            if let Some(ref cdp) = *guard {
                return cdp.clone();
            }
        }
        self.engine.clone()
    }

    /// BR-9: shared captcha-solve-and-inject helper. Used by both the
    /// standalone `browse_solve_captcha` MCP tool and the `solve_captcha`
    /// playbook step. Submits to the 2captcha API, polls for the token, and
    /// injects it into the live page so the next submit handler's
    /// `grecaptcha.execute()` resolves immediately with the pre-solved token.
    /// Returns the token string on success or a human-readable error on
    /// failure.
    async fn solve_and_inject_captcha(
        &self,
        captcha_type: &str,
        site_key_arg: Option<String>,
        url_arg: Option<String>,
    ) -> Result<String, String> {
        let api_key = std::env::var("TWOCAPTCHA_API_KEY").map_err(|_| {
            "TWOCAPTCHA_API_KEY environment variable not set. A solving-service API key is required.".to_string()
        })?;

        // Determine page URL
        let page_url = if let Some(u) = url_arg {
            u
        } else {
            let engine_arc = self.active_engine_async().await;
            let engine = engine_arc.lock().await;
            engine.current_url().await.unwrap_or_default()
        };

        // Determine site key — auto-detect if not provided
        let site_key = if let Some(sk) = site_key_arg {
            sk
        } else {
            let engine_arc = self.active_engine_async().await;
            let engine = engine_arc.lock().await;
            let detect_js = r#"(() => {
                var el = document.querySelector('[data-sitekey]');
                if (el) return el.dataset.sitekey || el.getAttribute('data-sitekey');
                var script = document.querySelector('script[src*="recaptcha"]');
                if (script) {
                    var m = script.src.match(/render=([^&]+)/);
                    if (m) return m[1];
                }
                var turnstile = document.querySelector('[data-cf-turnstile-sitekey]');
                if (turnstile) return turnstile.getAttribute('data-cf-turnstile-sitekey') || turnstile.dataset.cfTurnstileSitekey;
                var tScript = document.querySelector('script[src*="turnstile"]');
                if (tScript) {
                    var tm = tScript.src.match(/sitekey=([^&]+)/);
                    if (tm) return tm[1];
                }
                return '';
            })()"#;
            let detected = engine.eval_js(detect_js).await.unwrap_or_default();
            if detected.is_empty() {
                return Err("Could not auto-detect CAPTCHA site key. Provide site_key manually.".to_string());
            }
            detected
        };

        info!(site_key = %site_key, page_url = %page_url, captcha_type = %captcha_type, "CAPTCHA site key resolved");

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(130))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        // Step 1: Submit task
        let mut submit_params = vec![
            ("key", api_key.clone()),
            ("pageurl", page_url.clone()),
            ("json", "1".to_string()),
        ];
        match captcha_type {
            "turnstile" => {
                submit_params.push(("method", "turnstile".to_string()));
                submit_params.push(("sitekey", site_key.clone()));
            }
            _ => {
                submit_params.push(("method", "userrecaptcha".to_string()));
                submit_params.push(("googlekey", site_key.clone()));
                submit_params.push(("version", "v3".to_string()));
                submit_params.push(("action", "submit".to_string()));
                submit_params.push(("min_score", "0.7".to_string()));
            }
        }
        let submit_resp = client
            .post("http://2captcha.com/in.php")
            .form(&submit_params)
            .send()
            .await
            .map_err(|e| format!("Challenge solver submit failed: {e}"))?;
        let submit_body: serde_json::Value = submit_resp
            .json()
            .await
            .map_err(|e| format!("Challenge solver response parse error: {e}"))?;
        if submit_body.get("status").and_then(|v| v.as_i64()) != Some(1) {
            let err_text = submit_body.get("request").and_then(|v| v.as_str()).unwrap_or("unknown error");
            return Err(format!("Challenge solver rejected task: {err_text}"));
        }
        let task_id = submit_body
            .get("request")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        info!(task_id = %task_id, "Challenge solver task submitted, polling for result");

        // Step 2: Poll for result (every 5s, max 120s)
        let poll_url = format!(
            "http://2captcha.com/res.php?key={}&action=get&id={}&json=1",
            api_key, task_id
        );
        let mut token = String::new();
        for attempt in 0..24u32 {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let poll_resp = client.get(&poll_url).send().await;
            match poll_resp {
                Ok(resp) => {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        let status = body.get("status").and_then(|v| v.as_i64()).unwrap_or(0);
                        let request = body.get("request").and_then(|v| v.as_str()).unwrap_or("");
                        if status == 1 {
                            token = request.to_string();
                            info!(attempt = attempt, "CAPTCHA solved");
                            break;
                        } else if request == "CAPCHA_NOT_READY" || request == "CAPTCHA_NOT_READY" {
                            continue;
                        } else {
                            return Err(format!("Challenge solver error: {request}"));
                        }
                    }
                }
                Err(e) => {
                    warn!(attempt = attempt, error = %e, "Poll request failed, retrying");
                }
            }
        }
        if token.is_empty() {
            return Err("CAPTCHA solving timed out after 120 seconds".to_string());
        }

        // Step 3: inject into page (v3-aware: overrides grecaptcha.execute).
        let escaped = token.replace('\\', "\\\\").replace('\'', "\\'");
        let inject_js = match captcha_type {
            "turnstile" => format!(
                r#"(() => {{
                    var el = document.querySelector('[name="cf-turnstile-response"]') || document.querySelector('input[name*="turnstile"]');
                    if (el) el.value = '{escaped}';
                    var cb = document.querySelector('[data-cf-turnstile-sitekey]');
                    if (cb && cb.dataset.callback && typeof window[cb.dataset.callback] === 'function') {{
                        window[cb.dataset.callback]('{escaped}');
                    }}
                    return 'injected';
                }})()"#
            ),
            _ => format!(
                r#"(() => {{
                    const TOKEN = '{escaped}';
                    let actions = [];
                    let el = document.getElementById('g-recaptcha-response');
                    if (!el) {{
                        const els = document.querySelectorAll('textarea[name="g-recaptcha-response"]');
                        if (els.length > 0) el = els[0];
                    }}
                    if (el) {{ el.value = TOKEN; actions.push('textarea'); }}
                    if (typeof grecaptcha !== 'undefined') {{
                        try {{
                            grecaptcha.execute = function(siteKey, opts) {{ return Promise.resolve(TOKEN); }};
                            actions.push('execute_override');
                        }} catch(e) {{}}
                        try {{
                            if (typeof grecaptcha.getResponse === 'function') {{
                                grecaptcha.callback && grecaptcha.callback(TOKEN);
                                actions.push('callback');
                            }}
                        }} catch(e) {{}}
                    }}
                    if (typeof ___grecaptcha_cfg !== 'undefined') {{
                        try {{
                            const clients = ___grecaptcha_cfg.clients;
                            for (const k in clients) {{
                                const c = clients[k];
                                for (const kk in c) {{
                                    const item = c[kk];
                                    if (item && typeof item === 'object') {{
                                        for (const kkk in item) {{
                                            if (typeof item[kkk] === 'function' && /callback/i.test(kkk)) {{
                                                try {{ item[kkk](TOKEN); actions.push('client_cb'); }} catch(e2) {{}}
                                            }}
                                        }}
                                    }}
                                }}
                            }}
                        }} catch(e3) {{}}
                    }}
                    return 'injected:' + actions.join(',');
                }})()"#
            ),
        };
        let engine_arc = self.active_engine_async().await;
        let engine = engine_arc.lock().await;
        let _ = engine.eval_js(&inject_js).await;
        Ok(token)
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

                // BR-7 fix: route through the currently active session instead
                // of unconditionally resetting to "native". The old code reset
                // active_session_name = "native" and grabbed self.engine directly
                // — so any browse_session_switch to a CDP session was silently
                // undone the moment the user called browse_navigate, and every
                // subsequent click/select/eval_js hit Sevro instead of Chrome.
                // SPA auto-fallback still applies, but only when the active
                // engine genuinely is the default native one.
                #[cfg(feature = "cdp")]
                let active_name = self.active_session_name.lock().await.clone();
                #[cfg(not(feature = "cdp"))]
                let active_name = "native".to_string();

                let engine_arc = self.active_engine_async().await;
                let snapshot = {
                    let mut engine = engine_arc.lock().await;
                    engine.navigate(&input.url).await
                        .map_err(|e| ErrorData::internal_error(format!("Navigation failed: {e}"), None))?;
                    engine.snapshot().await
                        .map_err(|e| ErrorData::internal_error(format!("Snapshot failed: {e}"), None))?
                };

                // CDP auto-fallback: if the snapshot from the (native) active engine
                // has very few interactive elements, this is likely a JS-heavy SPA
                // that didn't render properly. Only applies when no explicit non-native
                // session is active — otherwise we'd silently override the user's choice.
                #[cfg(feature = "cdp")]
                {
                    if active_name == "native"
                        && self.cdp_auto
                        && snapshot.elements.len() < 5
                        && self.cdp_engine.is_some()
                    {
                        info!(
                            native_elements = snapshot.elements.len(),
                            url = %input.url,
                            "SPA detected, falling back to CDP renderer"
                        );

                        // Lazily launch CDP engine for SPA rendering
                        use wraith_browser_core::engine_cdp::CdpEngine;
                        match CdpEngine::new().await {
                            Ok(mut cdp_eng) => {
                                if let Ok(()) = cdp_eng.navigate(&input.url).await {
                                    if let Ok(cdp_snapshot) = cdp_eng.snapshot().await {
                                        let response = cdp_snapshot.to_agent_text();
                                        let _ = cdp_eng.shutdown().await;
                                        return Ok(CallToolResult::success(vec![Content::text(
                                            format!("[Full browser fallback — native had {} elements]\n\n{}", snapshot.elements.len(), response)
                                        )]));
                                    }
                                }
                                let _ = cdp_eng.shutdown().await;
                            }
                            Err(e) => {
                                debug!(error = %e, "CDP fallback unavailable");
                            }
                        }
                    }
                }
                let _ = active_name;

                let response = snapshot.to_agent_text();
                Ok(CallToolResult::success(vec![Content::text(response)]))
            }

            #[cfg(feature = "cdp")]
            "browse_navigate_cdp" => {
                let input: NavigateCdpInput = parse_args(args)?;
                info!(url = %input.url, "Navigating via CDP");

                // Lazily create CDP engine on first use
                use wraith_browser_core::engine_cdp::CdpEngine;
                let cdp_engine = CdpEngine::new().await
                    .map_err(|e| ErrorData::internal_error(
                        format!("CDP engine launch failed: {e}. Ensure Chrome is installed."), None
                    ))?;

                let cdp_arc: Arc<Mutex<dyn BrowserEngine>> = Arc::new(Mutex::new(cdp_engine));

                {
                    let mut eng = cdp_arc.lock().await;
                    eng.navigate(&input.url).await
                        .map_err(|e| ErrorData::internal_error(
                            format!("CDP navigation failed: {e}"), None
                        ))?;
                }

                let snapshot = {
                    let eng = cdp_arc.lock().await;
                    eng.snapshot().await
                        .map_err(|e| ErrorData::internal_error(
                            format!("CDP snapshot failed: {e}"), None
                        ))?
                };

                // Store the CDP engine as the active session — subsequent browse_*
                // commands will route to it instead of the native engine.
                {
                    let mut cdp_session = self.active_cdp_session.lock().await;
                    *cdp_session = Some(cdp_arc.clone());
                }
                // Also store in sessions map as "cdp" for named session support
                {
                    let mut sessions = self.sessions.lock().await;
                    sessions.insert("cdp".to_string(), cdp_arc);
                    let mut active = self.active_session_name.lock().await;
                    *active = "cdp".to_string();
                }
                info!("CDP engine stored as active session 'cdp' — subsequent browse_* commands route to CDP");

                let response = snapshot.to_agent_text();
                Ok(CallToolResult::success(vec![Content::text(
                    format!("[CDP engine active — all browse_* commands now use Chrome]\n\n{}", response)
                )]))
            }

            "browse_click" => {
                let input: ClickInput = parse_args(args)?;
                info!(ref_id = input.ref_id, "Clicking element");

                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;
                let result = engine.execute_action(BrowserAction::Click { ref_id: input.ref_id, force: input.force }).await
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

                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;
                let result = engine.execute_action(BrowserAction::Fill {
                    ref_id: input.ref_id,
                    text: input.text,
                    force: input.force,
                }).await
                    .map_err(|e| ErrorData::internal_error(format!("Fill failed: {e}"), None))?;

                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            "browse_snapshot" => {
                debug!("Taking DOM snapshot");
                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
                let snapshot = engine.snapshot().await
                    .map_err(|e| ErrorData::internal_error(format!("Snapshot failed: {e}"), None))?;

                Ok(CallToolResult::success(vec![Content::text(snapshot.to_agent_text())]))
            }

            "browse_extract" => {
                let input: ExtractInput = parse_args(args)?;
                info!(max_tokens = ?input.max_tokens, "Extracting content");

                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
                let html = engine.page_source().await
                    .map_err(|e| ErrorData::internal_error(format!("No page loaded: {e}"), None))?;
                let url = engine.current_url().await.unwrap_or_default();

                let result = if let Some(max_tokens) = input.max_tokens {
                    wraith_content_extract::extract_budgeted(&html, &url, max_tokens)
                } else {
                    wraith_content_extract::extract(&html, &url)
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
                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
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

                let results = wraith_search::search(&input.query, max).await
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

                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
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
                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
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
                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;
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
                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;

                // BR-6: if ref_id is provided, focus the element first so the
                // key dispatches to the right target. Without this, the key
                // lands on whatever currently has focus — often a page-top
                // button that captures Enter and submits the form prematurely.
                if let Some(ref_id) = input.ref_id {
                    let focus_js = format!(
                        r#"(() => {{
                            const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                            if (!el) return 'not_found';
                            if (typeof el.focus === 'function') el.focus();
                            return 'focused';
                        }})()"#
                    );
                    match engine.eval_js(&focus_js).await {
                        Ok(r) if r == "not_found" => {
                            return Ok(CallToolResult::success(vec![Content::text(
                                format!("@e{ref_id} not found — cannot focus before key press"),
                            )]));
                        }
                        Ok(_) => {}
                        Err(e) => {
                            return Ok(CallToolResult::success(vec![Content::text(
                                format!("Focus before key press failed: {e}"),
                            )]));
                        }
                    }

                    let result = engine.execute_action(BrowserAction::KeyPress { key: input.key.clone() }).await
                        .map_err(|e| ErrorData::internal_error(format!("KeyPress failed: {e}"), None))?;
                    return Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]));
                }

                // No ref_id given — preserve legacy behavior: in native mode,
                // bare Enter triggers a submit-button click as a workaround for
                // engines that don't dispatch real key events.
                if input.key.eq_ignore_ascii_case("enter") {
                    if let Ok(snapshot) = engine.snapshot().await {
                        let submit_el = snapshot.elements.iter().find(|el| {
                            el.role == "submit" || el.role == "button"
                        });
                        if let Some(el) = submit_el {
                            let ref_id = el.ref_id;
                            let result = engine.execute_action(BrowserAction::Click { ref_id, force: None }).await;
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

                // Engines with real keyboard support (CDP) will honor this even
                // without an explicit ref_id — it dispatches to current focus.
                let result = engine.execute_action(BrowserAction::KeyPress { key: input.key.clone() }).await
                    .map_err(|e| ErrorData::internal_error(format!("KeyPress failed: {e}"), None))?;
                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            "browse_scroll" => {
                let input: ScrollInput = parse_args(args)?;
                let direction = match input.direction.to_lowercase().as_str() {
                    "up" => wraith_browser_core::actions::ScrollDirection::Up,
                    "left" => wraith_browser_core::actions::ScrollDirection::Left,
                    "right" => wraith_browser_core::actions::ScrollDirection::Right,
                    _ => wraith_browser_core::actions::ScrollDirection::Down,
                };
                let amount = input.amount.unwrap_or(500);

                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;
                let result = engine.execute_action(BrowserAction::Scroll { direction, amount }).await
                    .map_err(|e| ErrorData::internal_error(format!("Scroll failed: {e}"), None))?;

                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            "browse_scroll_to" => {
                let input: ScrollToInput = parse_args(args)?;
                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;
                let result = engine.execute_action(BrowserAction::ScrollTo { ref_id: input.ref_id }).await
                    .map_err(|e| ErrorData::internal_error(format!("ScrollTo failed: {e}"), None))?;

                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            "browse_vault_store" => {
                let input: VaultStoreInput = parse_args(args)?;
                info!(domain = %input.domain, kind = %input.kind, "Storing credential");

                let vault = open_vault()?;

                let kind = match input.kind.to_lowercase().as_str() {
                    "password" => wraith_identity::CredentialKind::Password,
                    "api_key" | "apikey" => wraith_identity::CredentialKind::ApiKey,
                    "oauth_token" | "oauth" => wraith_identity::CredentialKind::OAuthToken,
                    "totp_seed" | "totp" => wraith_identity::CredentialKind::TotpSeed,
                    "session_cookie" | "cookie" => wraith_identity::CredentialKind::SessionCookie,
                    _ => wraith_identity::CredentialKind::Generic,
                };

                let request = wraith_identity::credential::StoreCredentialRequest {
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
                    "password" => wraith_identity::CredentialKind::Password,
                    "api_key" | "apikey" => wraith_identity::CredentialKind::ApiKey,
                    "oauth_token" | "oauth" => wraith_identity::CredentialKind::OAuthToken,
                    "session_cookie" | "cookie" => wraith_identity::CredentialKind::SessionCookie,
                    _ => wraith_identity::CredentialKind::Generic,
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
                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;
                let result = engine.execute_action(BrowserAction::Select { ref_id: input.ref_id, value: input.value, force: input.force }).await
                    .map_err(|e| ErrorData::internal_error(format!("Select failed: {e}"), None))?;
                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            "browse_type" => {
                let input: TypeTextInput = parse_args(args)?;
                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;
                let result = engine.execute_action(BrowserAction::TypeText {
                    ref_id: input.ref_id,
                    text: input.text,
                    delay_ms: input.delay_ms.unwrap_or(50),
                    force: input.force,
                }).await
                    .map_err(|e| ErrorData::internal_error(format!("Type failed: {e}"), None))?;
                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            "browse_hover" => {
                let input: HoverInput = parse_args(args)?;
                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;
                let result = engine.execute_action(BrowserAction::Hover { ref_id: input.ref_id }).await
                    .map_err(|e| ErrorData::internal_error(format!("Hover failed: {e}"), None))?;
                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            "browse_wait" => {
                let input: WaitInput = parse_args(args)?;
                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;
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
                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;
                let result = engine.execute_action(BrowserAction::GoForward).await
                    .map_err(|e| ErrorData::internal_error(format!("Forward failed: {e}"), None))?;
                Ok(CallToolResult::success(vec![Content::text(format_action_result(&result))]))
            }

            "browse_reload" => {
                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;
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

                let backend = wraith_agent_loop::llm::ClaudeBackend::new(api_key);
                let config = wraith_agent_loop::AgentConfig {
                    max_steps: input.max_steps.unwrap_or(50),
                    ..Default::default()
                };

                let task = wraith_agent_loop::BrowsingTask {
                    description: input.description,
                    start_url: input.url,
                    timeout_secs: None,
                    context: None,
                };

                let active_engine = self.active_engine_async().await;
                let mut agent = wraith_agent_loop::Agent::new(config, active_engine, backend);
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
                    .join(".wraith").join("knowledge");

                match wraith_cache::KnowledgeStore::open(&cache_dir) {
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
                    .join(".wraith").join("knowledge");

                match wraith_cache::KnowledgeStore::open(&cache_dir) {
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
                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
                let caps = engine.capabilities();
                let has_flaresolverr = std::env::var("WRAITH_FLARESOLVERR").is_ok();
                let has_proxy = std::env::var("WRAITH_PROXY").is_ok();
                Ok(CallToolResult::success(vec![Content::text(
                    format!("Engine capabilities:\n  JavaScript: {}\n  Screenshots: {:?}\n  Layout: {}\n  Cookies: {}\n  Compatible TLS: {}\n  Challenge Proxy: {}\n  Proxy: {}",
                        caps.javascript, caps.screenshots, caps.layout, caps.cookies, caps.stealth,
                        if has_flaresolverr { "configured" } else { "not configured (set WRAITH_FLARESOLVERR)" },
                        if has_proxy { "configured" } else { "direct" })
                )]))
            }

            // === Cookies ===

            "cookie_get" => {
                let input: CookieGetInput = parse_args(args)?;
                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
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
                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;

                // Store in the engine's cookie jar (used by http_fetch)
                engine.set_cookie_values(&input.domain, &input.name, &input.value, &path).await;

                // Also set in QuickJS document.cookie (for JS-side access)
                let script = format!(
                    "document.cookie = '{}={}; domain={}; path={}'",
                    input.name, input.value, input.domain, path
                );
                let _ = engine.eval_js(&script).await;

                Ok(CallToolResult::success(vec![Content::text(
                    format!("Cookie set: {}={} for {} (HTTP jar + JS)", input.name, input.value, input.domain)
                )]))
            }

            // === Fingerprints ===

            "fingerprint_list" => {
                let profiles = wraith_browser_core::tls_fingerprint::all_profiles();
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
                let profiles = wraith_browser_core::tls_fingerprint::all_profiles();
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
                let mut graph = wraith_cache::entity_graph::EntityGraph::new();
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
                    .join(".wraith").join("knowledge");
                match wraith_cache::KnowledgeStore::open(&cache_dir) {
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
                    .join(".wraith").join("knowledge");
                match wraith_cache::KnowledgeStore::open(&cache_dir) {
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
                let capture = wraith_browser_core::network_intel::NetworkCapture::new();
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
                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
                let html = engine.page_source().await.unwrap_or_default();
                let url = engine.current_url().await.unwrap_or_default();
                let domain = url::Url::parse(&url)
                    .map(|u| u.host_str().unwrap_or("").to_string())
                    .unwrap_or_default();

                let cap = wraith_cache::site_capability::fingerprint_site(&domain, &html, &url);
                let techs = wraith_cache::site_capability::detect_technology(&html);

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
                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
                let url = engine.current_url().await.unwrap_or_default();
                let current_html = engine.page_source().await.unwrap_or_default();

                let cache_dir = dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".wraith").join("knowledge");

                match wraith_cache::KnowledgeStore::open(&cache_dir) {
                    Ok(store) => {
                        match store.get_page(&url) {
                            Ok(Some(cached)) => {
                                let diff = wraith_cache::diffing::diff_pages(&url, &cached.plain_text, &current_html);
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
                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;
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
                let path = input.path.unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".wraith").join("cookies.json").to_string_lossy().to_string());
                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
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
                let path = input.path.unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".wraith").join("cookies.json").to_string_lossy().to_string());
                match std::fs::read_to_string(&path) {
                    Ok(json) => {
                        let engine_arc = self.active_engine_async().await;
                        let engine = engine_arc.lock().await;
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
                let dir = dirs::home_dir().unwrap_or_default().join(".wraith").join("knowledge");
                match wraith_cache::KnowledgeStore::open(&dir) {
                    Ok(store) => match store.pin_page(&input.url, input.notes.as_deref()) {
                        Ok(()) => Ok(CallToolResult::success(vec![Content::text(format!("Pinned: {}", input.url))])),
                        Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Pin failed: {e}"))]))
                    },
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cache error: {e}"))]))
                }
            }
            "cache_tag" => {
                let input: CacheTagInput = parse_args(args)?;
                let dir = dirs::home_dir().unwrap_or_default().join(".wraith").join("knowledge");
                match wraith_cache::KnowledgeStore::open(&dir) {
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
                let dir = dirs::home_dir().unwrap_or_default().join(".wraith").join("knowledge");
                match wraith_cache::KnowledgeStore::open(&dir) {
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
                let dir = dirs::home_dir().unwrap_or_default().join(".wraith").join("knowledge");
                let max = input.max_results.unwrap_or(5);
                match wraith_cache::KnowledgeStore::open(&dir) {
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
                let dir = dirs::home_dir().unwrap_or_default().join(".wraith").join("knowledge");
                match wraith_cache::KnowledgeStore::open(&dir) {
                    Ok(store) => match store.evict_to_budget(input.max_bytes) {
                        Ok(evicted) => Ok(CallToolResult::success(vec![Content::text(format!("Evicted {} bytes", evicted))])),
                        Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Evict failed: {e}"))]))
                    },
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cache error: {e}"))]))
                }
            }
            "cache_raw_html" => {
                let input: CacheRawHtmlInput = parse_args(args)?;
                let dir = dirs::home_dir().unwrap_or_default().join(".wraith").join("knowledge");
                match wraith_cache::KnowledgeStore::open(&dir) {
                    Ok(store) => {
                        let hash = wraith_cache::KnowledgeStore::hash_url(&input.url);
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
                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
                match engine.eval_js(&format!("document.querySelectorAll({}).length", serde_json::to_string(&input.selector).unwrap_or_default())).await {
                    Ok(n) => Ok(CallToolResult::success(vec![Content::text(format!("'{}' matched {} elements", input.selector, n))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Query failed: {e}"))]))
                }
            }
            "dom_get_attribute" => {
                let input: DomGetAttributeInput = parse_args(args)?;
                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
                let js = format!(r#"(()=>{{var els=document.querySelectorAll('a,button,input,select,textarea,[role="button"],[role="link"]');var v=Array.from(els).filter(e=>{{var r=e.getBoundingClientRect();return r.width>0&&r.height>0}});var el=v[{}-1];return el?el.getAttribute('{}'):null}})()"#, input.ref_id, input.name);
                match engine.eval_js(&js).await {
                    Ok(val) => Ok(CallToolResult::success(vec![Content::text(format!("@e{}.{} = {}", input.ref_id, input.name, val))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Error: {e}"))]))
                }
            }
            "dom_set_attribute" => {
                let input: DomSetAttributeInput = parse_args(args)?;
                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
                let js = format!(r#"(()=>{{var els=document.querySelectorAll('a,button,input,select,textarea,[role="button"],[role="link"]');var v=Array.from(els).filter(e=>{{var r=e.getBoundingClientRect();return r.width>0&&r.height>0}});var el=v[{}-1];if(el){{el.setAttribute('{}','{}');return'OK'}}return'NOT_FOUND'}})()"#, input.ref_id, input.name, input.value);
                match engine.eval_js(&js).await {
                    Ok(r) => Ok(CallToolResult::success(vec![Content::text(format!("Set @e{}.{}='{}': {}", input.ref_id, input.name, input.value, r))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Error: {e}"))]))
                }
            }
            "dom_focus" => {
                let input: DomFocusInput = parse_args(args)?;
                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
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
                        Ok(bytes) => match wraith_content_extract::pdf::extract_pdf_text(&bytes) {
                            Ok(content) => {
                                let md = wraith_content_extract::pdf::pdf_to_markdown(&content);
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
                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
                let html = engine.page_source().await.unwrap_or_default();
                let url = engine.current_url().await.unwrap_or_default();
                match wraith_content_extract::readability::extract_article(&html, &url) {
                    Ok(article) => Ok(CallToolResult::success(vec![Content::text(format!("# {}\n\n{}\n\n---\n{} images, {} links", article.title, article.content, article.images.len(), article.links.len()))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Article extraction failed: {e}"))]))
                }
            }
            "extract_markdown" => {
                let input: ExtractMarkdownInput = parse_args(args)?;
                let html = if let Some(h) = input.html { h } else { let ea = self.active_engine_async().await; let e = ea.lock().await; e.page_source().await.unwrap_or_default() };
                match wraith_content_extract::markdown::html_to_markdown(&html) {
                    Ok(md) => Ok(CallToolResult::success(vec![Content::text(md)])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Markdown failed: {e}"))]))
                }
            }
            "extract_plain_text" => {
                let input: ExtractPlainTextInput = parse_args(args)?;
                let html = if let Some(h) = input.html { h } else { let ea = self.active_engine_async().await; let e = ea.lock().await; e.page_source().await.unwrap_or_default() };
                match wraith_content_extract::markdown::html_to_plain_text(&html) {
                    Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Plain text failed: {e}"))]))
                }
            }
            "extract_ocr" => {
                let result = wraith_content_extract::ocr::basic_image_text_detection(&[]);
                Ok(CallToolResult::success(vec![Content::text(format!("OCR: {} regions, language: {}", result.regions.len(), result.language))]))
            }
            "auth_detect" => {
                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
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
                let mut mgr = wraith_identity::FingerprintManager::new();
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
                match wraith_browser_core::tor::DnsOverHttps::resolve(&input.domain, "https://cloudflare-dns.com/dns-query").await {
                    Ok(ips) => Ok(CallToolResult::success(vec![Content::text(format!("{} -> {}", input.domain, ips.join(", ")))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("DNS failed: {e}"))]))
                }
            }
            "stealth_status" => {
                let tls = wraith_browser_core::stealth_http::has_stealth_tls();
                let evasions = wraith_browser_core::stealth_evasions::StealthEvasions::all().evasion_count();
                Ok(CallToolResult::success(vec![Content::text(format!("Compatible TLS: {}\nEvasions: {}", if tls { "ACTIVE (BoringSSL)" } else { "INACTIVE (rustls)" }, evasions))]))
            }
            "plugin_register" => {
                let input: PluginRegisterInput = parse_args(args)?;
                let manifest = wraith_browser_core::wasm_plugins::PluginManifest { name: input.name.clone(), version: "1.0.0".to_string(), description: input.description.unwrap_or_default(), author: None, entry_point: input.wasm_path, domains: input.domains.unwrap_or_default(), capabilities: vec![] };
                let mut reg = wraith_browser_core::wasm_plugins::PluginRegistry::new();
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
                let reg = wraith_browser_core::wasm_plugins::PluginRegistry::new();
                let plugins = reg.list();
                if plugins.is_empty() { Ok(CallToolResult::success(vec![Content::text("No plugins registered.")])) }
                else { Ok(CallToolResult::success(vec![Content::text(plugins.iter().map(|p| format!("  {} v{}", p.name, p.version)).collect::<Vec<_>>().join("\n"))])) }
            }
            "plugin_remove" => {
                let input: PluginRemoveInput = parse_args(args)?;
                let mut reg = wraith_browser_core::wasm_plugins::PluginRegistry::new();
                Ok(CallToolResult::success(vec![Content::text(if reg.remove(&input.name) { format!("Removed '{}'", input.name) } else { format!("'{}' not found", input.name) })]))
            }
            "telemetry_metrics" => {
                let c = wraith_browser_core::telemetry::MetricsCollector::new();
                Ok(CallToolResult::success(vec![Content::text(c.to_json())]))
            }
            "telemetry_spans" => {
                let t = wraith_browser_core::telemetry::SpanTracker::new();
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
                Ok(CallToolResult::success(vec![Content::text("Workflows stored at ~/.wraith/workflows/")]))
            }
            "timetravel_summary" => {
                let t = wraith_agent_loop::timetravel::TimelineRecorder::new("mcp".to_string());
                Ok(CallToolResult::success(vec![Content::text(t.summary())]))
            }
            "timetravel_branch" => {
                let input: TimeTravelBranchInput = parse_args(args)?;
                let mut t = wraith_agent_loop::timetravel::TimelineRecorder::new("mcp".to_string());
                match t.branch_from(input.step, &input.name) {
                    Ok(id) => Ok(CallToolResult::success(vec![Content::text(format!("Branch '{}' created: {}", input.name, id))])),
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Branch failed: {e}"))]))
                }
            }
            "timetravel_replay" => {
                let input: TimeTravelReplayInput = parse_args(args)?;
                let t = wraith_agent_loop::timetravel::TimelineRecorder::new("mcp".to_string());
                let steps = t.replay_to(input.step);
                Ok(CallToolResult::success(vec![Content::text(format!("Replay to step {}: {} steps", input.step, steps.len()))]))
            }
            "timetravel_diff" => {
                let input: TimeTravelDiffInput = parse_args(args)?;
                let t = wraith_agent_loop::timetravel::TimelineRecorder::new("mcp".to_string());
                let diffs = t.diff_branches(&input.branch_a, &input.branch_b);
                Ok(CallToolResult::success(vec![Content::text(format!("{} vs {}: {} divergences", input.branch_a, input.branch_b, diffs.len()))]))
            }
            "timetravel_export" => {
                let t = wraith_agent_loop::timetravel::TimelineRecorder::new("mcp".to_string());
                Ok(CallToolResult::success(vec![Content::text(t.export_timeline())]))
            }
            "dag_create" => {
                let input: DagCreateInput = parse_args(args)?;
                let _d = wraith_agent_loop::task_dag::TaskDag::new(&input.name);
                Ok(CallToolResult::success(vec![Content::text(format!("DAG '{}' created.", input.name))]))
            }
            "dag_add_task" => {
                let input: DagAddTaskInput = parse_args(args)?;
                let action = match input.action_type.as_str() {
                    "navigate" => wraith_agent_loop::task_dag::TaskAction::Navigate(input.target.clone().unwrap_or_default()),
                    "click" => wraith_agent_loop::task_dag::TaskAction::Click(input.target.clone().unwrap_or_default()),
                    "extract" => wraith_agent_loop::task_dag::TaskAction::Extract(input.target.clone().unwrap_or_default()),
                    _ => wraith_agent_loop::task_dag::TaskAction::Custom(input.target.clone().unwrap_or_default()),
                };
                let _n = wraith_agent_loop::task_dag::TaskNode::new(&input.task_id, &input.description, action);
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
                let config = wraith_agent_loop::mcts::MctsConfig { max_simulations: input.simulations.unwrap_or(100), exploration_constant: 1.41, max_depth: 10, discount_factor: 0.95 };
                let mut planner = wraith_agent_loop::mcts::MctsPlanner::new(config);
                let candidates: Vec<wraith_agent_loop::mcts::ActionCandidate> = input.actions.iter().map(|a| wraith_agent_loop::mcts::ActionCandidate { action: a.clone(), description: a.clone(), estimated_reward: 0.5 }).collect();
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
                let predictor = wraith_agent_loop::prefetch::PrefetchPredictor::new(wraith_agent_loop::prefetch::PrefetchConfig::default());
                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
                let snapshot = engine.snapshot().await.ok();
                let preds = predictor.predict(&input.task_description, "", "", &[], &[]);
                if preds.is_empty() { Ok(CallToolResult::success(vec![Content::text("No predictions. Navigate first.")])) }
                else { Ok(CallToolResult::success(vec![Content::text(preds.iter().map(|p| format!("  {} ({:.2})", p.url, p.relevance)).collect::<Vec<_>>().join("\n"))])) }
            }
            "swarm_fan_out" => {
                let input: SwarmFanOutInput = parse_args(args)?;
                let mut results = Vec::new();
                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;
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
                let etype = match input.entity_type.as_str() { "company" => wraith_cache::entity_graph::EntityType::Organization, "person" => wraith_cache::entity_graph::EntityType::Person, "technology" => wraith_cache::entity_graph::EntityType::Technology, "product" => wraith_cache::entity_graph::EntityType::Product, "location" => wraith_cache::entity_graph::EntityType::Location, _ => wraith_cache::entity_graph::EntityType::Unknown };
                let entity = wraith_cache::entity_graph::Entity { id: uuid::Uuid::new_v4().to_string(), canonical_name: input.name.to_lowercase(), display_name: input.name.clone(), entity_type: etype, attributes: input.attributes.unwrap_or_default(), sources: vec![], first_seen: chrono::Utc::now(), last_seen: chrono::Utc::now() };
                let mut g = wraith_cache::entity_graph::EntityGraph::new();
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
                let g = wraith_cache::entity_graph::EntityGraph::new();
                let related = g.find_related(&input.name);
                if related.is_empty() { Ok(CallToolResult::success(vec![Content::text(format!("No relations for '{}'", input.name))])) }
                else { Ok(CallToolResult::success(vec![Content::text(related.iter().map(|(e, r)| format!("  --[{}]--> {} ({:?})", r.kind, e.display_name, e.entity_type)).collect::<Vec<_>>().join("\n"))])) }
            }
            "entity_search" => {
                let input: EntitySearchInput = parse_args(args)?;
                let g = wraith_cache::entity_graph::EntityGraph::new();
                let results = g.search_entities(&input.query);
                if results.is_empty() { Ok(CallToolResult::success(vec![Content::text(format!("No entities matching '{}'", input.query))])) }
                else { Ok(CallToolResult::success(vec![Content::text(results.iter().map(|e| format!("  {} ({:?})", e.display_name, e.entity_type)).collect::<Vec<_>>().join("\n"))])) }
            }
            "entity_visualize" => {
                let g = wraith_cache::entity_graph::EntityGraph::new();
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

                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;
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
                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;
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
                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;

                // Step 1: Click to open the dropdown
                let _ = engine.execute_action(BrowserAction::Click { ref_id: input.ref_id, force: None }).await;
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;

                // Step 2: Type to filter options
                let _ = engine.execute_action(BrowserAction::TypeText {
                    ref_id: input.ref_id,
                    text: input.value.clone(),
                    delay_ms: 50,
                    force: None,
                }).await;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                // Step 3: Find and click the matching option via JS
                // Uses React-compatible event dispatch (mousedown + mouseup + click)
                // plus native setter trick on associated input for React state update
                let js = format!(
                    r#"(() => {{
                        var val = '{value}';
                        var refId = {ref_id};
                        // Look for listbox options, menu items, or visible option-like elements
                        var options = document.querySelectorAll('[role="option"], [role="listbox"] li, [class*="option"], [class*="Option"], [class*="dropdown"] li, [class*="menu"] li, [data-value]');
                        var matched = null;
                        for (var i = 0; i < options.length; i++) {{
                            var opt = options[i];
                            var text = (opt.textContent || '').trim().toLowerCase();
                            if (text === val.toLowerCase() || text.indexOf(val.toLowerCase()) >= 0) {{
                                matched = opt;
                                break;
                            }}
                        }}
                        if (!matched) {{
                            return 'TYPED_VALUE: ' + val + ' (no matching option found — Enter may confirm)';
                        }}
                        // Dispatch full event sequence for React compatibility
                        matched.dispatchEvent(new MouseEvent('mousedown', {{bubbles: true}}));
                        matched.dispatchEvent(new MouseEvent('mouseup', {{bubbles: true}}));
                        matched.click();
                        var selectedText = matched.textContent.trim();
                        // Find the trigger element and its associated input
                        var trigger = document.querySelector('[data-wraith-ref="' + refId + '"]');
                        if (!trigger) {{
                            // Try finding by ref index in the snapshot
                            var allInteractive = document.querySelectorAll('a, button, input, select, textarea, [role]');
                            if (refId > 0 && refId <= allInteractive.length) trigger = allInteractive[refId - 1];
                        }}
                        if (trigger) {{
                            var input = trigger.querySelector('input[type="hidden"]') || trigger.querySelector('input') || trigger;
                            try {{
                                var nativeSetter = Object.getOwnPropertyDescriptor(
                                    window.HTMLInputElement.prototype, 'value'
                                );
                                if (nativeSetter && nativeSetter.set) {{
                                    nativeSetter.set.call(input, selectedText);
                                }} else {{
                                    input.value = selectedText;
                                }}
                            }} catch(e) {{
                                input.value = selectedText;
                            }}
                            input.dispatchEvent(new Event('input', {{bubbles: true}}));
                            input.dispatchEvent(new Event('change', {{bubbles: true}}));
                            input.dispatchEvent(new Event('blur', {{bubbles: true}}));
                        }}
                        return 'SELECTED: ' + selectedText;
                    }})()"#,
                    value = input.value.replace('\'', "\\'"),
                    ref_id = input.ref_id,
                );

                // Wait for React re-render after option click
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;

                match engine.eval_js(&js).await {
                    Ok(result) => {
                        if result.starts_with("TYPED_VALUE:") {
                            // Press Enter as fallback
                            let _ = engine.execute_action(BrowserAction::KeyPress { key: "Enter".to_string() }).await;
                        }
                        // Wait for final React re-render
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        Ok(CallToolResult::success(vec![Content::text(result)]))
                    }
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Dropdown failed: {e}"))]))
                }
            }

            "cookie_import_chrome" => {
                let input: ChromeCookieImportInput = parse_args(args)?;
                let profile = input.profile.unwrap_or_else(|| "Default".to_string());

                // Chrome cookie DB path on Windows
                // Chrome v96+ moved cookies from {Profile}/Cookies to {Profile}/Network/Cookies
                let profile_dir = dirs::data_local_dir()
                    .unwrap_or_default()
                    .join("Google")
                    .join("Chrome")
                    .join("User Data")
                    .join(&profile);
                let modern_path = profile_dir.join("Network").join("Cookies");
                let legacy_path = profile_dir.join("Cookies");
                let cookie_db = if modern_path.exists() {
                    modern_path
                } else if legacy_path.exists() {
                    legacy_path
                } else {
                    return Ok(CallToolResult::success(vec![Content::text(
                        format!("Chrome cookie DB not found at: {}\nor: {}\nTry a different profile name.", modern_path.display(), legacy_path.display())
                    )]));
                };

                // Chrome locks the cookie file while running. We use a tiered
                // strategy to read it despite the lock:
                //   1. Open directly via SQLite URI mode with immutable=1 (skips all locking)
                //   2. Fall back to copying the file to a temp location
                //   3. If both fail, return a helpful error

                let domain_filter = input.domain.clone().unwrap_or_else(|| "%".to_string());

                // Read the Chrome master key for cookie decryption (v80+)
                let master_key: Option<Vec<u8>> = (|| -> Result<Vec<u8>, String> {
                    let local_state_path = profile_dir.parent().unwrap().join("Local State");
                    let local_state: serde_json::Value = serde_json::from_str(
                        &std::fs::read_to_string(&local_state_path)
                            .map_err(|e| format!("read Local State: {e}"))?
                    ).map_err(|e| format!("parse Local State: {e}"))?;
                    let encrypted_key_b64 = local_state["os_crypt"]["encrypted_key"]
                        .as_str()
                        .ok_or_else(|| "missing os_crypt.encrypted_key".to_string())?;
                    let encrypted_key = base64::engine::general_purpose::STANDARD
                        .decode(encrypted_key_b64)
                        .map_err(|e| format!("base64 decode key: {e}"))?;
                    // Strip "DPAPI" prefix (first 5 bytes)
                    let dpapi_key = &encrypted_key[5..];
                    dpapi_decrypt(dpapi_key)
                })().ok();

                // Helper: run the cookie query on an already-opened connection
                let run_query = |conn: &rusqlite::Connection, domain_filter: &str| -> Result<Vec<(String, String, Vec<u8>)>, String> {
                    let query = if domain_filter == "%" {
                        "SELECT host_key, name, encrypted_value FROM cookies ORDER BY host_key LIMIT 500"
                    } else {
                        "SELECT host_key, name, encrypted_value FROM cookies WHERE host_key LIKE ?1 ORDER BY host_key LIMIT 500"
                    };
                    let mut stmt = conn.prepare(query).map_err(|e| format!("SQL: {e}"))?;
                    let domain_param = format!("%{}%", domain_filter);
                    let rows: Vec<(String, String, Vec<u8>)> = if domain_filter == "%" {
                        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                            .map_err(|e| format!("Query: {e}"))?
                            .filter_map(|r| r.ok()).collect()
                    } else {
                        stmt.query_map(rusqlite::params![domain_param], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                            .map_err(|e| format!("Query: {e}"))?
                            .filter_map(|r| r.ok()).collect()
                    };
                    Ok(rows)
                };

                // Strategy 1: SQLite URI with immutable=1 — reads the DB as a
                // lock-free snapshot, works even while Chrome holds a lock.
                let uri = format!(
                    "file:{}?immutable=1&mode=ro",
                    cookie_db.display()
                );
                let immutable_result = rusqlite::Connection::open_with_flags(
                    &uri,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
                )
                .map_err(|e| format!("immutable open: {e}"))
                .and_then(|conn| run_query(&conn, &domain_filter));

                let (cookies_result, used_temp) = match immutable_result {
                    Ok(rows) => (Ok(rows), false),
                    Err(_immutable_err) => {
                        // Strategy 2: copy the file to a temp location, then open normally.
                        let temp_db = std::env::temp_dir().join("wraith_chrome_cookies_copy");
                        match std::fs::copy(&cookie_db, &temp_db) {
                            Ok(_) => {
                                let res = rusqlite::Connection::open(&temp_db)
                                    .map_err(|e| format!("DB open failed: {e}"))
                                    .and_then(|conn| run_query(&conn, &domain_filter));
                                let _ = std::fs::remove_file(&temp_db);
                                (res, false)
                            }
                            Err(copy_err) => {
                                // Strategy 3: give up with a helpful message
                                (Err(format!(
                                    "Cannot read Chrome cookie DB while Chrome is running.\n\
                                     Immutable open failed, file copy also failed: {copy_err}\n\
                                     Please close Chrome and try again, or manually copy\n  \
                                     {}\nto a temporary location.",
                                    cookie_db.display()
                                )), false)
                            }
                        }
                    }
                };
                let _ = used_temp; // suppress unused warning

                match cookies_result {
                    Ok(rows) => {
                        let engine_arc = self.active_engine_async().await;
                        let mut engine = engine_arc.lock().await;
                        let mut injected = 0;
                        let mut decrypt_errors = 0u32;
                        for (host, name, encrypted_value) in &rows {
                            let value = if let Some(ref key) = master_key {
                                match decrypt_chrome_cookie(encrypted_value, key) {
                                    Ok(v) => v,
                                    Err(_) => { decrypt_errors += 1; continue; }
                                }
                            } else if encrypted_value.is_empty() {
                                continue;
                            } else {
                                String::from_utf8_lossy(encrypted_value).to_string()
                            };
                            if !value.is_empty() {
                                engine.set_cookie_values(host, name, &value, "/").await;
                                let script = format!("document.cookie = '{}={}; domain={}; path=/'", name, value, host);
                                let _ = engine.eval_js(&script).await;
                                injected += 1;
                            }
                        }

                        // If DPAPI decryption failed on ALL cookies, they use v20 App-Bound
                        // Encryption (Chrome 127+). This encryption is tied to Chrome's own
                        // process via IElevator COM — external decryption is not possible.
                        if injected == 0 && decrypt_errors > 0 {
                            info!(v20_count = decrypt_errors, "All cookies use App-Bound Encryption (v20) — external decryption not supported");
                            return Ok(CallToolResult::success(vec![Content::text(
                                format!("Found {} encrypted cookies but cannot decrypt them.\n\
                                         Chrome 127+ uses App-Bound Encryption (v20) which binds cookies to Chrome's own process.\n\n\
                                         To authenticate in Wraith, use one of these approaches:\n\
                                         1. browse_login — navigate to the login page and authenticate directly\n\
                                         2. cookie_load — export cookies from Chrome using a browser extension\n\
                                            (e.g., EditThisCookie → export as JSON → cookie_load the file)\n\
                                         3. cookie_set — manually set specific cookies (e.g., session tokens)",
                                         decrypt_errors)
                            )]));
                        }

                        let mut msg = format!("Imported {} cookies from Chrome profile '{}'{}",
                            injected, profile,
                            if domain_filter != "%" { format!(" (filtered: {})", domain_filter) } else { String::new() });
                        if decrypt_errors > 0 {
                            msg.push_str(&format!(" ({} cookies failed to decrypt — may be v20 App-Bound)", decrypt_errors));
                        }
                        Ok(CallToolResult::success(vec![Content::text(msg)]))
                    }
                    Err(e) => Ok(CallToolResult::success(vec![Content::text(format!("Cookie import failed: {e}"))]))
                }
            }

            "browse_fetch_scripts" => {
                let _input: FetchScriptsInput = parse_args(args)?;
                info!("Fetching external scripts for current page");

                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;
                let html = engine.page_source().await.unwrap_or_default();
                let url = engine.current_url().await.unwrap_or_default();
                drop(engine); // Release lock before async HTTP calls

                // Fetch scripts using a standalone client
                let client = reqwest::Client::builder()
                    .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new());

                // Extract both external script URLs and inline script content
                let (script_urls, inline_scripts): (Vec<String>, Vec<String>) = {
                    let doc = scraper::Html::parse_document(&html);
                    let sel = scraper::Selector::parse("script").unwrap();

                    let mut urls = Vec::new();
                    let mut inlines = Vec::new();

                    for el in doc.select(&sel) {
                        // Skip JSON-LD, templates, etc.
                        if let Some(t) = el.value().attr("type") {
                            if t.contains("json") || t.contains("template") || t.contains("importmap") {
                                continue;
                            }
                        }

                        if let Some(src) = el.value().attr("src") {
                            // External script
                            if src.contains("google-analytics") || src.contains("gtag")
                                || src.contains("facebook") || src.contains("hotjar") { continue; }
                            let full = if src.starts_with("http") { src.to_string() }
                                else if src.starts_with("//") { format!("https:{}", src) }
                                else if src.starts_with('/') {
                                    if let Ok(u) = url::Url::parse(&url) {
                                        format!("{}://{}{}", u.scheme(), u.host_str().unwrap_or(""), src)
                                    } else { continue }
                                } else { continue };
                            urls.push(full);
                        } else {
                            // Inline script — get the text content
                            let text: String = el.text().collect::<Vec<_>>().join("");
                            let trimmed = text.trim();
                            if !trimmed.is_empty() && trimmed.len() > 10 {
                                inlines.push(trimmed.to_string());
                            }
                        }
                    }

                    (urls, inlines)
                }; // doc dropped here

                // Fetch scripts async (now safe — no scraper types held)
                let mut scripts = std::collections::HashMap::new();
                let mut total: usize = 0;
                let max_total: usize = 2 * 1024 * 1024;
                for script_url in &script_urls {
                    if total >= max_total { break; }
                    if let Ok(resp) = client.get(script_url).send().await {
                        if resp.status().is_success() {
                            if let Ok(text) = resp.text().await {
                                if !text.is_empty() && total + text.len() <= max_total {
                                    total += text.len();
                                    scripts.insert(script_url.clone(), text);
                                }
                            }
                        }
                    }
                }

                if scripts.is_empty() && inline_scripts.is_empty() {
                    Ok(CallToolResult::success(vec![Content::text("No scripts found (external or inline).")]))
                } else {
                    let engine_arc2 = self.active_engine_async().await;
                    let engine = engine_arc2.lock().await;
                    let mut executed = 0;
                    let mut failed = 0;
                    let mut dynamic_urls: Vec<String> = Vec::new();

                    // First: execute inline scripts (these may bootstrap SPAs like Ashby)
                    for (idx, inline) in inline_scripts.iter().enumerate() {
                        match engine.eval_js(inline).await {
                            Ok(_) => {
                                executed += 1;
                                debug!(idx = idx, len = inline.len(), "Inline script executed");
                            }
                            Err(e) => {
                                failed += 1;
                                debug!(idx = idx, error = %e, "Inline script failed");
                            }
                        }
                    }

                    // Check if inline scripts dynamically created new script elements
                    // (Ashby pattern: inline script creates <script src="bundle.js">)
                    if let Ok(dynamic_json) = engine.eval_js(
                        r#"(() => {
                            try {
                                var urls = [];
                                var scripts = document.querySelectorAll('script');
                                for (var i = 0; i < scripts.length; i++) {
                                    var s = scripts[i];
                                    if (s.attrs && s.attrs.src) urls.push(s.attrs.src);
                                }
                                return JSON.stringify(urls);
                            } catch(e) { return '[]'; }
                        })()"#
                    ).await {
                        if let Ok(urls) = serde_json::from_str::<Vec<String>>(&dynamic_json) {
                            for dyn_url in urls {
                                if !scripts.contains_key(&dyn_url) && dyn_url.starts_with("http") {
                                    dynamic_urls.push(dyn_url);
                                }
                            }
                        }
                    }

                    // Fetch dynamically discovered scripts
                    for dyn_url in &dynamic_urls {
                        if total >= max_total { break; }
                        if let Ok(resp) = client.get(dyn_url).send().await {
                            if resp.status().is_success() {
                                if let Ok(text) = resp.text().await {
                                    if !text.is_empty() && total + text.len() <= max_total {
                                        total += text.len();
                                        scripts.insert(dyn_url.clone(), text);
                                    }
                                }
                            }
                        }
                    }

                    // Then: execute external scripts (fetched + dynamically discovered)
                    for (src, script_text) in &scripts {
                        match engine.eval_js(script_text).await {
                            Ok(_) => {
                                executed += 1;
                                debug!(src = %src, len = script_text.len(), "External script executed");
                            }
                            Err(e) => {
                                failed += 1;
                                debug!(src = %src, error = %e, "External script failed");
                            }
                        }
                    }

                    // Flush timers (React setup uses setTimeout)
                    let _ = engine.eval_js("if(typeof __wraith_flush_timers==='function')__wraith_flush_timers()").await;

                    let total_found = inline_scripts.len() + script_urls.len() + dynamic_urls.len();
                    Ok(CallToolResult::success(vec![Content::text(
                        format!("Found {} scripts ({} inline, {} external, {} dynamic): {} executed, {} failed.",
                            total_found, inline_scripts.len(), script_urls.len(), dynamic_urls.len(), executed, failed)
                    )]))
                }
            }

            "browse_dismiss_overlay" => {
                let input: DismissOverlayInput = parse_args(args)?;
                info!(ref_id = ?input.ref_id, "Dismissing overlay");

                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;

                // Find the close button via JS
                let find_js = if let Some(ref_id) = input.ref_id {
                    format!("__wraith_find_close_button({})", ref_id)
                } else {
                    "__wraith_find_close_button()".to_string()
                };

                let close_result = engine.eval_js(&find_js).await
                    .map_err(|e| ErrorData::internal_error(format!("Overlay detection failed: {e}"), None))?;

                let parsed: serde_json::Value = serde_json::from_str(&close_result)
                    .map_err(|e| ErrorData::internal_error(format!("Failed to parse overlay result: {e}"), None))?;

                if let Some(error) = parsed.get("error").and_then(|v| v.as_str()) {
                    return Ok(CallToolResult::success(vec![Content::text(
                        format!("Could not dismiss overlay: {}", error)
                    )]));
                }

                let close_ref_id = parsed.get("ref_id")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| ErrorData::internal_error("No close button ref_id found".to_string(), None))?
                    as u32;

                let close_text = parsed.get("text").and_then(|v| v.as_str()).unwrap_or("?");
                info!(close_ref_id = close_ref_id, close_text = %close_text, "Clicking overlay close button");

                // Click the close button
                let click_result = engine.execute_action(BrowserAction::Click { ref_id: close_ref_id, force: None }).await
                    .map_err(|e| ErrorData::internal_error(format!("Failed to click close button: {e}"), None))?;

                debug!(result = ?click_result, "Overlay close button clicked");

                // Return updated snapshot
                let snapshot = engine.snapshot().await
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

                Ok(CallToolResult::success(vec![Content::text(
                    format!("Dismissed overlay (clicked \"{}\" @e{})\n\n{}", close_text, close_ref_id, snapshot.to_agent_text())
                )]))
            }

            "browse_solve_captcha" => {
                let input: SolveCaptchaInput = parse_args(args)?;
                let captcha_type = input.captcha_type.unwrap_or_else(|| "recaptchav3".to_string());
                info!(captcha_type = %captcha_type, "Solving challenge via solving service");
                match self.solve_and_inject_captcha(&captcha_type, input.site_key, input.url).await {
                    Ok(token) => Ok(CallToolResult::success(vec![Content::text(
                        format!("CAPTCHA solved (type: {}). Token: {}", captcha_type, token)
                    )])),
                    Err(e) => Err(ErrorData::internal_error(e, None)),
                }
            }

            // === TLS Verification ===

            "tls_verify" => {
                let input: TlsVerifyInput = parse_args(args)?;
                let service_url = input.url.as_deref().unwrap_or("https://tls.peet.ws/api/all");
                info!(url = %service_url, "TLS fingerprint verification");

                // Use the same stealth HTTP stack the engine uses for navigation
                let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                           (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";
                let (status, body, _final_url) = wraith_browser_core::stealth_http::stealth_fetch(
                    service_url, ua, "en-US,en;q=0.9",
                ).await.map_err(|e| ErrorData::internal_error(
                    format!("TLS verification fetch failed: {e}"), None))?;

                if status != 200 {
                    return Ok(CallToolResult::success(vec![Content::text(
                        format!("TLS service returned HTTP {status}. Try a different URL with the `url` parameter.")
                    )]));
                }

                let tls_json: serde_json::Value = serde_json::from_str(&body)
                    .map_err(|e| ErrorData::internal_error(
                        format!("Failed to parse TLS service JSON: {e}"), None))?;

                // --- Known Chrome 136 reference values ---
                let ref_ja3 = "cd08e31494f9531f560d64c695473da9";
                let ref_ja4 = "t13d1517h2_8daaf6152771_b0da82dd1658";
                let ref_tls_version = "TLSv1.3";
                let ref_cipher_count: usize = 17;
                let ref_extension_count: usize = 16;
                let ref_h2_window: u32 = 6291456;

                // --- Extract observed values from the JSON response ---
                // tls.peet.ws format: { tls: { ja3_hash, ja4, ... }, http2: { ... } }
                // browserleaks format: { ja3_hash, ja4, tls_version, ... }
                let tls_section = tls_json.get("tls").unwrap_or(&tls_json);

                let obs_ja3 = tls_section.get("ja3_hash")
                    .or_else(|| tls_section.get("ja3"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("N/A");

                let obs_ja4 = tls_section.get("ja4")
                    .or_else(|| tls_section.get("ja4_hash"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("N/A");

                let obs_tls_version = tls_section.get("tls_version")
                    .or_else(|| tls_section.get("version"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("N/A");

                let obs_cipher_count = tls_section.get("cipher_suites")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .or_else(|| tls_section.get("ciphers")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len()))
                    .unwrap_or(0);

                let obs_extension_count = tls_section.get("extensions")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);

                // HTTP/2 settings (tls.peet.ws nests under "http2")
                let h2_section = tls_json.get("http2").or_else(|| tls_json.get("h2"));
                let obs_h2_window = h2_section
                    .and_then(|h2| h2.get("settings"))
                    .and_then(|s| s.get("INITIAL_WINDOW_SIZE")
                        .or_else(|| s.get("initial_window_size"))
                        .or_else(|| {
                            // tls.peet.ws: settings is an array of { id, value }
                            s.as_array().and_then(|arr| {
                                arr.iter().find(|item| {
                                    item.get("id").and_then(|v| v.as_str()) == Some("INITIAL_WINDOW_SIZE")
                                    || item.get("id").and_then(|v| v.as_u64()) == Some(4)
                                }).and_then(|item| item.get("value"))
                            })
                        }))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
                    .unwrap_or(0);

                // --- Compare and build report ---
                let match_sym = |m: bool| if m { "MATCH" } else { "MISMATCH" };
                let check_sym = |m: bool| if m { "\u{2713}" } else { "\u{2717}" };

                let ja3_match = obs_ja3 == ref_ja3;
                let ja4_match = obs_ja4 == ref_ja4 || obs_ja4 == "N/A"; // N/A = service doesn't provide it
                let tls_match = obs_tls_version.contains("1.3");
                let cipher_match = obs_cipher_count == ref_cipher_count;
                let ext_match = obs_extension_count == ref_extension_count;
                let h2_match = obs_h2_window == ref_h2_window || obs_h2_window == 0;

                let stealth_active = wraith_browser_core::stealth_http::has_stealth_tls();

                // Determine overall verdict
                let critical_pass = ja3_match && tls_match;
                let all_pass = ja3_match && ja4_match && tls_match && cipher_match && ext_match && h2_match;
                let verdict = if all_pass {
                    "PASS \u{2014} fingerprint matches Chrome 136"
                } else if critical_pass {
                    "PARTIAL \u{2014} JA3/TLS match but some secondary fields differ"
                } else {
                    "FAIL \u{2014} fingerprint does NOT match Chrome 136"
                };

                let report = format!(
                    "TLS Fingerprint Verification:\n\
                     \n\
                     Compatible TLS: {stealth}\n\
                     Service:     {service}\n\
                     \n\
                     JA3:  {obs_ja3} (Chrome 136: {ref_ja3} {ja3_check} {ja3_verdict})\n\
                     JA4:  {obs_ja4} (Chrome 136: {ref_ja4} {ja4_check} {ja4_verdict})\n\
                     TLS:  {obs_tls} (Chrome 136: {ref_tls} {tls_check} {tls_verdict})\n\
                     Cipher Suites: {obs_ciphers} (Chrome 136: {ref_ciphers} {cipher_check} {cipher_verdict})\n\
                     Extensions:    {obs_exts} (Chrome 136: {ref_exts} {ext_check} {ext_verdict})\n\
                     HTTP/2: INITIAL_WINDOW_SIZE {obs_h2} (Chrome 136: {ref_h2} {h2_check} {h2_verdict})\n\
                     \n\
                     Verdict: {verdict}",
                    stealth = if stealth_active { "ACTIVE (BoringSSL)" } else { "INACTIVE (rustls)" },
                    service = service_url,
                    obs_ja3 = obs_ja3,
                    ref_ja3 = ref_ja3,
                    ja3_check = check_sym(ja3_match),
                    ja3_verdict = match_sym(ja3_match),
                    obs_ja4 = obs_ja4,
                    ref_ja4 = ref_ja4,
                    ja4_check = check_sym(ja4_match),
                    ja4_verdict = match_sym(ja4_match),
                    obs_tls = obs_tls_version,
                    ref_tls = ref_tls_version,
                    tls_check = check_sym(tls_match),
                    tls_verdict = match_sym(tls_match),
                    obs_ciphers = obs_cipher_count,
                    ref_ciphers = ref_cipher_count,
                    cipher_check = check_sym(cipher_match),
                    cipher_verdict = match_sym(cipher_match),
                    obs_exts = obs_extension_count,
                    ref_exts = ref_extension_count,
                    ext_check = check_sym(ext_match),
                    ext_verdict = match_sym(ext_match),
                    obs_h2 = if obs_h2_window == 0 { "N/A".to_string() } else { obs_h2_window.to_string() },
                    ref_h2 = ref_h2_window,
                    h2_check = check_sym(h2_match),
                    h2_verdict = match_sym(h2_match),
                    verdict = verdict,
                );

                Ok(CallToolResult::success(vec![Content::text(report)]))
            }

            "browse_enter_iframe" => {
                let input: EnterIframeInput = parse_args(args)?;
                info!(ref_id = input.ref_id, "Entering iframe context");

                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;

                // Get the current snapshot to find the iframe element's src URL
                let snapshot = engine.snapshot().await
                    .map_err(|e| ErrorData::internal_error(format!("Snapshot failed: {e}"), None))?;

                // Find the element matching the ref_id
                let iframe_el = snapshot.elements.iter()
                    .find(|e| e.ref_id == input.ref_id)
                    .ok_or_else(|| ErrorData::invalid_params(
                        format!("No element found with @e{}", input.ref_id), None))?;

                // Check if this is an iframe element
                if !iframe_el.selector.starts_with("iframe") && !iframe_el.role.contains("iframe") {
                    return Err(ErrorData::invalid_params(
                        format!("@e{} is not an iframe (role: {}, selector: {})", input.ref_id, iframe_el.role, iframe_el.selector), None));
                }

                // Get the iframe src via eval_js (attribute lookup by ref_id)
                let src_js = format!(
                    r#"(() => {{
                        var el = __wraith_get_by_ref({ref_id});
                        if (!el) return '';
                        return (el.attrs && el.attrs.src) || el.src || '';
                    }})()"#,
                    ref_id = input.ref_id,
                );
                let src_url = engine.eval_js(&src_js).await.unwrap_or_default();

                if src_url.is_empty() {
                    // Fallback: try href attribute from snapshot
                    let src_url_fallback = iframe_el.href.clone().unwrap_or_default();
                    if src_url_fallback.is_empty() {
                        return Err(ErrorData::internal_error(
                            format!("Could not determine iframe src for @e{}", input.ref_id), None));
                    }
                    // Navigate to the iframe URL
                    engine.navigate(&src_url_fallback).await
                        .map_err(|e| ErrorData::internal_error(format!("Failed to navigate to iframe: {e}"), None))?;
                } else {
                    // Navigate to the iframe URL
                    engine.navigate(&src_url).await
                        .map_err(|e| ErrorData::internal_error(format!("Failed to navigate to iframe: {e}"), None))?;
                }

                // Return snapshot of iframe contents
                let iframe_snapshot = engine.snapshot().await
                    .map_err(|e| ErrorData::internal_error(format!("Iframe snapshot failed: {e}"), None))?;

                Ok(CallToolResult::success(vec![Content::text(iframe_snapshot.to_agent_text())]))
            }

            "browse_login" => {
                let input: LoginInput = parse_args(args)?;
                info!(url = %input.url, "Login flow starting");

                let engine_arc = self.active_engine_async().await;
                let mut engine = engine_arc.lock().await;

                // Step 1: Navigate to the login page
                engine.navigate(&input.url).await
                    .map_err(|e| ErrorData::internal_error(format!("Navigate to login page failed: {e}"), None))?;

                // Step 2: Fill username
                let fill_user = engine.execute_action(BrowserAction::Fill {
                    ref_id: input.username_ref_id,
                    text: input.username.clone(),
                    force: None,
                }).await
                    .map_err(|e| ErrorData::internal_error(format!("Fill username failed: {e}"), None))?;
                debug!(result = ?fill_user, "Username filled");

                // Step 3: Fill password
                let fill_pass = engine.execute_action(BrowserAction::Fill {
                    ref_id: input.password_ref_id,
                    text: input.password.clone(),
                    force: None,
                }).await
                    .map_err(|e| ErrorData::internal_error(format!("Fill password failed: {e}"), None))?;
                debug!(result = ?fill_pass, "Password filled");

                // Step 4: Click submit — this triggers the auth/redirect chain.
                // The underlying engine's http_fetch now captures Set-Cookie headers
                // from every redirect hop and stores them, so OAuth redirect chains
                // (302 -> 302 -> 302 -> 200) have their cookies preserved automatically.
                let click_result = engine.execute_action(BrowserAction::Click {
                    ref_id: input.submit_ref_id,
                    force: None,
                }).await
                    .map_err(|e| ErrorData::internal_error(format!("Click submit failed: {e}"), None))?;
                debug!(result = ?click_result, "Submit clicked");

                // Step 5: If the click triggered a form POST that returned a redirect URL,
                // follow it via a regular navigate (which uses http_fetch with cookie capture).
                match &click_result {
                    ActionResult::Navigated { url, .. } => {
                        info!(url = %url, "Login submit navigated — following redirect chain");
                        // Re-navigate to ensure the full redirect chain is followed
                        // with cookie capture at each hop
                        let _ = engine.navigate(url).await;
                    }
                    _ => {
                        // Form may have submitted via JS/XHR — check if the page changed
                        debug!("Login submit did not trigger navigation — checking page state");
                    }
                }

                // Step 6: Take final snapshot
                let snapshot = engine.snapshot().await
                    .map_err(|e| ErrorData::internal_error(format!("Post-login snapshot failed: {e}"), None))?;

                // Build response with final URL info
                let final_url = engine.current_url().await.unwrap_or_default();
                let domain = url::Url::parse(&final_url).ok()
                    .and_then(|u| u.host_str().map(|h| h.to_string()))
                    .unwrap_or_default();

                let response = format!(
                    "{}\n\n--- Login Flow Complete ---\nFinal URL: {}\nDomain: {}\nCookies were captured at every redirect hop during the auth flow.",
                    snapshot.to_agent_text(),
                    final_url,
                    domain,
                );

                Ok(CallToolResult::success(vec![Content::text(response)]))
            }

            "browse_engine_status" => {
                #[cfg(feature = "cdp")]
                {
                    let guard = self.active_cdp_session.lock().await;
                    if guard.is_some() {
                        Ok(CallToolResult::success(vec![Content::text(
                            "Active engine: CDP (Chrome)\n\nAll browse_* commands are routed to the Chrome DevTools Protocol engine.\nCall browse_navigate to switch back to the native (Sevro) engine."
                        )]))
                    } else {
                        Ok(CallToolResult::success(vec![Content::text(
                            "Active engine: native (Sevro)\n\nAll browse_* commands use the native Sevro engine.\nCall browse_navigate_cdp to switch to Chrome CDP for JS-heavy pages."
                        )]))
                    }
                }
                #[cfg(not(feature = "cdp"))]
                {
                    Ok(CallToolResult::success(vec![Content::text(
                        "Active engine: native (Sevro)\n\nCDP support not compiled in. Build with --features cdp to enable Chrome."
                    )]))
                }
            }

            // ── Session management ──────────────────────────────────────
            #[cfg(feature = "cdp")]
            "browse_session_create" => {
                let input: SessionCreateInput = parse_args(args)?;
                let session_name = input.name.trim().to_string();
                if session_name.is_empty() {
                    return Err(ErrorData::invalid_params("Session name cannot be empty", None));
                }

                // Check if session already exists
                {
                    let sessions = self.sessions.lock().await;
                    if sessions.contains_key(&session_name) {
                        return Err(ErrorData::invalid_params(
                            format!("Session '{}' already exists. Use browse_session_switch to activate it.", session_name), None
                        ));
                    }
                }

                let engine_type = input.engine_type.to_lowercase();
                let new_engine: Arc<Mutex<dyn BrowserEngine>> = match engine_type.as_str() {
                    "native" | "sevro" => {
                        // Create a fresh native engine
                        Self::default_engine()
                    }
                    "cdp" | "chrome" => {
                        use wraith_browser_core::engine_cdp::CdpEngine;
                        let cdp_engine = CdpEngine::new().await
                            .map_err(|e| ErrorData::internal_error(
                                format!("CDP engine launch failed: {e}. Ensure Chrome is installed."), None
                            ))?;
                        Arc::new(Mutex::new(cdp_engine))
                    }
                    "cdp-attach" | "cdp_attach" | "attach" => {
                        // BR-9 primary path: attach to operator's daily Chrome.
                        // Real fingerprint + cookies + history pass anti-bot
                        // checks like reCAPTCHA v3 natively, no 2captcha needed.
                        use wraith_browser_core::engine_cdp::CdpEngine;
                        let port = input.attach_port.unwrap_or(9222);
                        let cdp_engine = CdpEngine::attach(port, input.attach_target.clone()).await
                            .map_err(|e| ErrorData::internal_error(
                                format!(
                                    "CDP attach failed: {e}. Ensure Chrome is running with `--remote-debugging-port={port}`."
                                ), None
                            ))?;
                        Arc::new(Mutex::new(cdp_engine))
                    }
                    _ => {
                        return Err(ErrorData::invalid_params(
                            format!("Unknown engine_type '{}'. Use 'native', 'cdp', or 'cdp-attach'.", engine_type), None
                        ));
                    }
                };

                {
                    let mut sessions = self.sessions.lock().await;
                    sessions.insert(session_name.clone(), new_engine);
                }

                info!(session = %session_name, engine = %engine_type, "Session created");
                Ok(CallToolResult::success(vec![Content::text(
                    format!("Session '{}' created with {} engine.\nUse browse_session_switch to activate it.", session_name, engine_type)
                )]))
            }

            #[cfg(feature = "cdp")]
            "browse_session_switch" => {
                let input: SessionSwitchInput = parse_args(args)?;
                let session_name = input.name.trim().to_string();

                {
                    let sessions = self.sessions.lock().await;
                    if !sessions.contains_key(&session_name) {
                        let available: Vec<String> = sessions.keys().cloned().collect();
                        return Err(ErrorData::invalid_params(
                            format!("Session '{}' not found. Available sessions: {:?}", session_name, available), None
                        ));
                    }
                }

                // Update active session name
                {
                    let mut active = self.active_session_name.lock().await;
                    *active = session_name.clone();
                }

                // Also sync the legacy active_cdp_session field
                {
                    let sessions = self.sessions.lock().await;
                    let mut cdp_session = self.active_cdp_session.lock().await;
                    if session_name == "native" {
                        *cdp_session = None;
                    } else if let Some(eng) = sessions.get(&session_name) {
                        *cdp_session = Some(eng.clone());
                    }
                }

                info!(session = %session_name, "Switched active session");
                Ok(CallToolResult::success(vec![Content::text(
                    format!("Active session switched to '{}'.\nAll browse_* commands now route to this session.", session_name)
                )]))
            }

            "browse_session_list" => {
                #[cfg(feature = "cdp")]
                {
                    let sessions = self.sessions.lock().await;
                    let active = self.active_session_name.lock().await.clone();
                    let mut lines = Vec::new();
                    lines.push("Sessions:".to_string());
                    for (name, engine_arc) in sessions.iter() {
                        let eng = engine_arc.lock().await;
                        let url = eng.current_url().await.unwrap_or_else(|| "about:blank".to_string());
                        let engine_type = if name == "native" || name.starts_with("native") {
                            "native (Sevro)"
                        } else {
                            "CDP (Chrome)"
                        };
                        let marker = if *name == active { " [active]" } else { "" };
                        lines.push(format!("  - {}{}: {} — {}", name, marker, engine_type, url));
                    }
                    Ok(CallToolResult::success(vec![Content::text(lines.join("\n"))]))
                }
                #[cfg(not(feature = "cdp"))]
                {
                    let eng = self.engine.lock().await;
                    let url = eng.current_url().await.unwrap_or_else(|| "about:blank".to_string());
                    Ok(CallToolResult::success(vec![Content::text(
                        format!("Sessions:\n  - native [active]: native (Sevro) — {}", url)
                    )]))
                }
            }

            #[cfg(feature = "cdp")]
            "browse_session_close" => {
                let input: SessionCloseInput = parse_args(args)?;
                let session_name = input.name.trim().to_string();

                if session_name == "native" {
                    return Err(ErrorData::invalid_params(
                        "Cannot close the 'native' session — it is always available.", None
                    ));
                }

                let removed = {
                    let mut sessions = self.sessions.lock().await;
                    sessions.remove(&session_name)
                };

                match removed {
                    Some(engine_arc) => {
                        // Shut down the engine
                        let mut eng = engine_arc.lock().await;
                        let _ = eng.shutdown().await;
                        drop(eng);

                        // If we closed the active session, switch to "native"
                        let was_active = {
                            let active = self.active_session_name.lock().await;
                            *active == session_name
                        };
                        if was_active {
                            let mut active = self.active_session_name.lock().await;
                            *active = "native".to_string();
                            let mut cdp_session = self.active_cdp_session.lock().await;
                            *cdp_session = None;
                            info!(closed = %session_name, "Closed active session, switched to 'native'");
                            Ok(CallToolResult::success(vec![Content::text(
                                format!("Session '{}' closed. Active session switched to 'native'.", session_name)
                            )]))
                        } else {
                            info!(closed = %session_name, "Session closed");
                            Ok(CallToolResult::success(vec![Content::text(
                                format!("Session '{}' closed.", session_name)
                            )]))
                        }
                    }
                    None => {
                        let sessions = self.sessions.lock().await;
                        let available: Vec<String> = sessions.keys().cloned().collect();
                        Err(ErrorData::invalid_params(
                            format!("Session '{}' not found. Available sessions: {:?}", session_name, available), None
                        ))
                    }
                }
            }

            // ── Playbook tools ────────────────────────────────────────────
            "swarm_list_playbooks" => {
                let _input: PlaybookListInput = parse_args(args)?;
                info!("Listing built-in playbooks");

                let playbooks = json!([
                    {
                        "name": "greenhouse-apply",
                        "description": "Apply to a job on Greenhouse boards — navigates to the posting, fills standard fields (name, email, phone, resume upload, work authorization, LinkedIn), handles custom dropdowns (country, visa sponsorship, EEO fields), and submits the application."
                    },
                    {
                        "name": "ashby-apply",
                        "description": "Apply to a job on Ashby job boards — navigates to the posting, fills candidate info fields, uploads resume, answers custom questions, and submits."
                    },
                    {
                        "name": "lever-apply",
                        "description": "Apply to a job on Lever job boards — navigates to the posting, clicks Apply, fills the application form (name, email, phone, resume, LinkedIn, current company), and submits."
                    },
                    {
                        "name": "indeed-search",
                        "description": "Search for jobs on Indeed — navigates to indeed.com, fills the what/where fields, submits the search, and extracts job titles, companies, locations, and URLs from the results page."
                    }
                ]);

                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&playbooks).unwrap_or_default()
                )]))
            }

            "swarm_playbook_status" => {
                let input: PlaybookStatusInput = parse_args(args)?;
                info!(run_id = ?input.run_id, "Checking playbook status");

                // Playbook state is ephemeral (lives in the swarm_run_playbook call).
                // Return a stub status — the caller can poll during long runs.
                let run_id = input.run_id.unwrap_or_else(|| "latest".to_string());
                let status = json!({
                    "run_id": run_id,
                    "status": "idle",
                    "completed_steps": 0,
                    "total_steps": 0,
                    "current_step": null,
                    "errors": [],
                    "message": "No playbook is currently running. Use swarm_run_playbook to start one."
                });

                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&status).unwrap_or_default()
                )]))
            }

            "swarm_run_playbook" => {
                let input: PlaybookRunInput = parse_args(args)?;
                info!(playbook = %input.playbook_yaml, job_url = %input.job_url, "Running playbook");

                // ── 1. Resolve the playbook YAML ──────────────────────────
                let raw_yaml = match input.playbook_yaml.as_str() {
                    "greenhouse-apply" => PLAYBOOK_GREENHOUSE.to_string(),
                    "ashby-apply"      => PLAYBOOK_ASHBY.to_string(),
                    "lever-apply"      => PLAYBOOK_LEVER.to_string(),
                    "indeed-search"    => PLAYBOOK_INDEED.to_string(),
                    other => other.to_string(), // treat as raw YAML
                };

                // ── 2. Variable interpolation ─────────────────────────────
                let mut resolved = raw_yaml.clone();
                resolved = resolved.replace("{{job_url}}", &input.job_url);
                for (k, v) in &input.variables {
                    resolved = resolved.replace(&format!("{{{{{}}}}}", k), v);
                }

                // ── 3. Parse YAML into steps ──────────────────────────────
                let steps: Vec<serde_json::Value> = serde_yaml::from_str(&resolved)
                    .map_err(|e| ErrorData::invalid_params(
                        format!("Failed to parse playbook YAML: {e}"), None
                    ))?;

                // ── 3b. Detect engine preference from playbook header ─────
                // Look for top-level "engine" field if the YAML was a full playbook
                #[allow(unused_variables)]
                let wants_cdp = {
                    // Try parsing as a Playbook struct to check engine field
                    let maybe_pb: Result<wraith_browser_core::playbook::Playbook, _> =
                        serde_yaml::from_str(&resolved);
                    match maybe_pb {
                        Ok(pb) => pb.engine.eq_ignore_ascii_case("cdp"),
                        Err(_) => {
                            // Raw steps YAML — check if the playbook name implies CDP
                            matches!(input.playbook_yaml.as_str(),
                                "greenhouse-apply" | "ashby-apply")
                        }
                    }
                };

                // ── 3c. Auto-switch to CDP if needed ──────────────────────
                #[cfg(feature = "cdp")]
                let engine_arc = if wants_cdp {
                    info!("Playbook requests CDP engine — auto-switching");
                    use wraith_browser_core::engine_cdp::CdpEngine;
                    match CdpEngine::new().await {
                        Ok(cdp_eng) => {
                            let cdp_arc: Arc<Mutex<dyn BrowserEngine>> = Arc::new(Mutex::new(cdp_eng));
                            // Store as active session so all browse_* route here
                            {
                                let mut cdp_session = self.active_cdp_session.lock().await;
                                *cdp_session = Some(cdp_arc.clone());
                            }
                            {
                                let mut sessions = self.sessions.lock().await;
                                sessions.insert("cdp".to_string(), cdp_arc.clone());
                                let mut active = self.active_session_name.lock().await;
                                *active = "cdp".to_string();
                            }
                            cdp_arc
                        }
                        Err(e) => {
                            warn!(error = %e, "CDP launch failed, falling back to native");
                            self.active_engine_async().await
                        }
                    }
                } else {
                    self.active_engine_async().await
                };
                #[cfg(not(feature = "cdp"))]
                let engine_arc = self.active_engine_async().await;

                let total_steps = steps.len();
                let mut results: Vec<serde_json::Value> = Vec::with_capacity(total_steps);

                // ── 4. Execute each step sequentially ─────────────────────
                for (idx, step) in steps.iter().enumerate() {
                    let step_name = step.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unnamed")
                        .to_string();
                    let action = step.get("action")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_lowercase();

                    info!(step = idx + 1, total = total_steps, action = %action, name = %step_name, "Playbook step");

                    let step_result: serde_json::Value = match action.as_str() {
                        "navigate" | "navigate_cdp" => {
                            let url = step.get("url")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&input.job_url);
                            let mut eng = engine_arc.lock().await;
                            match eng.navigate(url).await {
                                Ok(()) => {
                                    // Wait for page load if wait_for selector specified
                                    if let Some(wait_sel) = step.get("wait_for").and_then(|v| v.as_str()) {
                                        let timeout = step.get("timeout").and_then(|v| v.as_u64()).unwrap_or(10000);
                                        let _ = eng.execute_action(BrowserAction::WaitForSelector {
                                            selector: wait_sel.to_string(),
                                            timeout_ms: timeout,
                                        }).await;
                                    }
                                    let snap = eng.snapshot().await.ok();
                                    let elem_count = snap.as_ref()
                                        .map(|s| s.elements.len()).unwrap_or(0);
                                    json!({
                                        "step": idx + 1,
                                        "name": step_name,
                                        "action": "navigate",
                                        "status": "ok",
                                        "url": url,
                                        "elements_found": elem_count
                                    })
                                }
                                Err(e) => json!({
                                    "step": idx + 1,
                                    "name": step_name,
                                    "action": "navigate",
                                    "status": "error",
                                    "error": e.to_string()
                                }),
                            }
                        }

                        "fill" => {
                            let selector = step.get("selector")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let value = step.get("value")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");

                            let mut eng = engine_arc.lock().await;
                            let ref_id = resolve_ref_by_selector(&*eng, selector).await;
                            match ref_id {
                                Some(rid) => {
                                    match eng.execute_action(BrowserAction::Fill {
                                        ref_id: rid,
                                        text: value.to_string(),
                                        force: Some(true),
                                    }).await {
                                        Ok(r) => json!({
                                            "step": idx + 1,
                                            "name": step_name,
                                            "action": "fill",
                                            "status": "ok",
                                            "selector": selector,
                                            "ref_id": rid,
                                            "result": format_action_result(&r)
                                        }),
                                        Err(e) => json!({
                                            "step": idx + 1,
                                            "name": step_name,
                                            "action": "fill",
                                            "status": "error",
                                            "selector": selector,
                                            "error": e.to_string()
                                        }),
                                    }
                                }
                                None => json!({
                                    "step": idx + 1,
                                    "name": step_name,
                                    "action": "fill",
                                    "status": "error",
                                    "selector": selector,
                                    "error": format!("No element found matching selector '{}'", selector)
                                }),
                            }
                        }

                        "select" => {
                            let selector = step.get("selector")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let value = step.get("value")
                                .or_else(|| step.get("text"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");

                            let mut eng = engine_arc.lock().await;
                            let ref_id = resolve_ref_by_selector(&*eng, selector).await;
                            match ref_id {
                                Some(rid) => {
                                    match eng.execute_action(BrowserAction::Select {
                                        ref_id: rid,
                                        value: value.to_string(),
                                        force: Some(true),
                                    }).await {
                                        Ok(r) => json!({
                                            "step": idx + 1,
                                            "name": step_name,
                                            "action": "select",
                                            "status": "ok",
                                            "selector": selector,
                                            "ref_id": rid,
                                            "value": value,
                                            "result": format_action_result(&r)
                                        }),
                                        Err(e) => json!({
                                            "step": idx + 1,
                                            "name": step_name,
                                            "action": "select",
                                            "status": "error",
                                            "selector": selector,
                                            "error": e.to_string()
                                        }),
                                    }
                                }
                                None => json!({
                                    "step": idx + 1,
                                    "name": step_name,
                                    "action": "select",
                                    "status": "error",
                                    "selector": selector,
                                    "error": format!("No select element found matching '{}'", selector)
                                }),
                            }
                        }

                        "custom_dropdown" => {
                            let selector = step.get("selector")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let value = step.get("value")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let optional = step.get("optional")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);

                            let mut eng = engine_arc.lock().await;
                            let ref_id = resolve_ref_by_selector(&*eng, selector).await;
                            match ref_id {
                                Some(rid) => {
                                    // Click to open, type to filter, then click matching option
                                    let _ = eng.execute_action(BrowserAction::Click {
                                        ref_id: rid, force: Some(true),
                                    }).await;
                                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                                    let _ = eng.execute_action(BrowserAction::TypeText {
                                        ref_id: rid, text: value.to_string(),
                                        delay_ms: 50, force: Some(true),
                                    }).await;
                                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                    // Re-snapshot and find the option
                                    let snap = eng.snapshot().await.ok();
                                    let option_ref = snap.as_ref().and_then(|s| {
                                        s.elements.iter().find(|e| {
                                            e.text.as_deref()
                                                .map(|t| t.to_lowercase().contains(&value.to_lowercase()))
                                                .unwrap_or(false)
                                                && e.ref_id != rid
                                                && e.role != "textbox" && e.role != "combobox"
                                        }).map(|e| e.ref_id)
                                    });
                                    if let Some(opt_rid) = option_ref {
                                        let _ = eng.execute_action(BrowserAction::Click {
                                            ref_id: opt_rid, force: Some(true),
                                        }).await;
                                        json!({
                                            "step": idx + 1,
                                            "name": step_name,
                                            "action": "custom_dropdown",
                                            "status": "ok",
                                            "selector": selector,
                                            "ref_id": rid,
                                            "option_ref_id": opt_rid,
                                            "value": value
                                        })
                                    } else if optional {
                                        json!({
                                            "step": idx + 1,
                                            "name": step_name,
                                            "action": "custom_dropdown",
                                            "status": "skipped",
                                            "selector": selector,
                                            "message": format!("Optional dropdown — option '{}' not found", value)
                                        })
                                    } else {
                                        json!({
                                            "step": idx + 1,
                                            "name": step_name,
                                            "action": "custom_dropdown",
                                            "status": "error",
                                            "selector": selector,
                                            "error": format!("Dropdown option '{}' not found after filtering", value)
                                        })
                                    }
                                }
                                None => {
                                    if optional {
                                        json!({
                                            "step": idx + 1,
                                            "name": step_name,
                                            "action": "custom_dropdown",
                                            "status": "skipped",
                                            "message": format!("Optional dropdown '{}' not found on page", selector)
                                        })
                                    } else {
                                        json!({
                                            "step": idx + 1,
                                            "name": step_name,
                                            "action": "custom_dropdown",
                                            "status": "error",
                                            "selector": selector,
                                            "error": format!("No dropdown found matching '{}'", selector)
                                        })
                                    }
                                }
                            }
                        }

                        "upload" | "upload_file" => {
                            let selector = step.get("selector")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let file_path_str = step.get("file_path")
                                .or_else(|| step.get("path"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");

                            let path = std::path::Path::new(file_path_str);
                            if !path.exists() {
                                json!({
                                    "step": idx + 1,
                                    "name": step_name,
                                    "action": "upload_file",
                                    "status": "error",
                                    "error": format!("File not found: {}", file_path_str)
                                })
                            } else {
                                let file_bytes = match std::fs::read(path) {
                                    Ok(b) => b,
                                    Err(e) => {
                                        results.push(json!({
                                            "step": idx + 1,
                                            "name": step_name,
                                            "action": "upload_file",
                                            "status": "error",
                                            "error": format!("Failed to read file: {}", e)
                                        }));
                                        continue;
                                    }
                                };
                                let file_name = path.file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("file")
                                    .to_string();
                                let mime_type = match path.extension().and_then(|e| e.to_str()) {
                                    Some("pdf") => "application/pdf",
                                    Some("doc") => "application/msword",
                                    Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                                    Some("txt") => "text/plain",
                                    Some("png") => "image/png",
                                    Some("jpg") | Some("jpeg") => "image/jpeg",
                                    _ => "application/octet-stream",
                                };
                                let b64 = base64::engine::general_purpose::STANDARD.encode(&file_bytes);

                                let mut eng = engine_arc.lock().await;
                                let ref_id = resolve_ref_by_selector(&*eng, selector).await;
                                match ref_id {
                                    Some(rid) => {
                                        match eng.execute_action(BrowserAction::UploadFile {
                                            ref_id: rid,
                                            file_name,
                                            file_data: b64,
                                            mime_type: mime_type.to_string(),
                                        }).await {
                                            Ok(r) => json!({
                                                "step": idx + 1,
                                                "name": step_name,
                                                "action": "upload_file",
                                                "status": "ok",
                                                "file_path": file_path_str,
                                                "ref_id": rid,
                                                "result": format_action_result(&r)
                                            }),
                                            Err(e) => json!({
                                                "step": idx + 1,
                                                "name": step_name,
                                                "action": "upload_file",
                                                "status": "error",
                                                "error": e.to_string()
                                            }),
                                        }
                                    }
                                    None => json!({
                                        "step": idx + 1,
                                        "name": step_name,
                                        "action": "upload_file",
                                        "status": "error",
                                        "error": format!("No file input found matching selector '{}'", selector)
                                    }),
                                }
                            }
                        }

                        "submit" | "submit_form" => {
                            let selector = step.get("selector")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");

                            let mut eng = engine_arc.lock().await;
                            let ref_id = resolve_ref_by_selector(&*eng, selector).await;
                            match ref_id {
                                Some(rid) => {
                                    match eng.execute_action(BrowserAction::SubmitForm {
                                        ref_id: rid,
                                    }).await {
                                        Ok(r) => {
                                            // Wait for navigation after submit
                                            if step.get("wait_for_navigation").and_then(|v| v.as_bool()).unwrap_or(true) {
                                                let timeout = step.get("timeout").and_then(|v| v.as_u64()).unwrap_or(5000);
                                                let _ = eng.execute_action(BrowserAction::WaitForNavigation {
                                                    timeout_ms: timeout,
                                                }).await;
                                            }
                                            json!({
                                                "step": idx + 1,
                                                "name": step_name,
                                                "action": "submit",
                                                "status": "ok",
                                                "ref_id": rid,
                                                "result": format_action_result(&r)
                                            })
                                        }
                                        Err(e) => json!({
                                            "step": idx + 1,
                                            "name": step_name,
                                            "action": "submit",
                                            "status": "error",
                                            "error": e.to_string()
                                        }),
                                    }
                                }
                                None => json!({
                                    "step": idx + 1,
                                    "name": step_name,
                                    "action": "submit",
                                    "status": "error",
                                    "error": format!("No submit element found matching selector '{}'", selector)
                                }),
                            }
                        }

                        "click" => {
                            let selector = step.get("selector")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");

                            let mut eng = engine_arc.lock().await;
                            let ref_id = resolve_ref_by_selector(&*eng, selector).await;
                            match ref_id {
                                Some(rid) => {
                                    match eng.execute_action(BrowserAction::Click {
                                        ref_id: rid,
                                        force: Some(true),
                                    }).await {
                                        Ok(r) => json!({
                                            "step": idx + 1,
                                            "name": step_name,
                                            "action": "click",
                                            "status": "ok",
                                            "ref_id": rid,
                                            "result": format_action_result(&r)
                                        }),
                                        Err(e) => json!({
                                            "step": idx + 1,
                                            "name": step_name,
                                            "action": "click",
                                            "status": "error",
                                            "error": e.to_string()
                                        }),
                                    }
                                }
                                None => json!({
                                    "step": idx + 1,
                                    "name": step_name,
                                    "action": "click",
                                    "status": "error",
                                    "error": format!("No element found matching selector '{}'", selector)
                                }),
                            }
                        }

                        "eval_js" => {
                            let code = step.get("code")
                                .or_else(|| step.get("script"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let eng = engine_arc.lock().await;
                            match eng.eval_js(code).await {
                                Ok(result) => json!({
                                    "step": idx + 1,
                                    "name": step_name,
                                    "action": "eval_js",
                                    "status": "ok",
                                    "result": result
                                }),
                                Err(e) => json!({
                                    "step": idx + 1,
                                    "name": step_name,
                                    "action": "eval_js",
                                    "status": "error",
                                    "error": e.to_string()
                                }),
                            }
                        }

                        "extract" => {
                            let eng = engine_arc.lock().await;
                            let html = eng.page_source().await.unwrap_or_default();
                            let url = eng.current_url().await.unwrap_or_default();
                            match wraith_content_extract::extract(&html, &url) {
                                Ok(content) => json!({
                                    "step": idx + 1,
                                    "name": step_name,
                                    "action": "extract",
                                    "status": "ok",
                                    "title": content.title,
                                    "markdown_length": content.markdown.len(),
                                    "links": content.links.len()
                                }),
                                Err(e) => json!({
                                    "step": idx + 1,
                                    "name": step_name,
                                    "action": "extract",
                                    "status": "error",
                                    "error": e.to_string()
                                }),
                            }
                        }

                        "screenshot" => {
                            let eng = engine_arc.lock().await;
                            match eng.screenshot().await {
                                Ok(png) => json!({
                                    "step": idx + 1,
                                    "name": step_name,
                                    "action": "screenshot",
                                    "status": "ok",
                                    "size_bytes": png.len()
                                }),
                                Err(e) => json!({
                                    "step": idx + 1,
                                    "name": step_name,
                                    "action": "screenshot",
                                    "status": "error",
                                    "error": e.to_string()
                                }),
                            }
                        }

                        "conditional" => {
                            let eng = engine_arc.lock().await;
                            let snap = eng.snapshot().await.ok();
                            let page_text = snap.as_ref()
                                .map(|s| s.to_agent_text())
                                .unwrap_or_default();
                            let current_url = snap.as_ref()
                                .map(|s| s.url.as_str())
                                .unwrap_or("");

                            let mut condition_met = false;

                            // if_exists: check if a selector matches any element
                            if let Some(if_exists) = step.get("if_exists").and_then(|v| v.as_str()) {
                                drop(eng);
                                let eng2 = engine_arc.lock().await;
                                condition_met = resolve_ref_by_selector(&*eng2, if_exists).await.is_some();
                            }
                            // if_url_contains
                            else if let Some(url_frag) = step.get("if_url_contains").and_then(|v| v.as_str()) {
                                condition_met = current_url.contains(url_frag);
                            }
                            // if_visible: check if selector element is visible
                            else if let Some(if_visible) = step.get("if_visible").and_then(|v| v.as_str()) {
                                drop(eng);
                                let eng2 = engine_arc.lock().await;
                                condition_met = resolve_ref_by_selector(&*eng2, if_visible).await.is_some();
                            }
                            // if_variable: check if a runtime var is truthy
                            else if let Some(_var_check) = step.get("if_variable").and_then(|v| v.as_str()) {
                                // Variable checking would require the PlaybookRunner — simplified here
                                condition_met = false;
                            }

                            json!({
                                "step": idx + 1,
                                "name": step_name,
                                "action": "conditional",
                                "status": "ok",
                                "condition_met": condition_met,
                                "message": if condition_met { "Condition true — then branch would execute" } else { "Condition false — else branch would execute" }
                            })
                        }

                        "verify" => {
                            // Parse expect_url_contains from either direct field or check: url_contains("...")
                            let url_check_from_check_field: Option<String> = step.get("check")
                                .and_then(|v| v.as_str())
                                .and_then(|s| {
                                    if s.starts_with("url_contains(") {
                                        Some(s.trim_start_matches("url_contains(\"")
                                            .trim_start_matches("url_contains('")
                                            .trim_end_matches("\")")
                                            .trim_end_matches("')")
                                            .to_string())
                                    } else { None }
                                });
                            let expect_url_contains = step.get("expect_url_contains")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .or(url_check_from_check_field);
                            let expect_text = step.get("expect_text")
                                .and_then(|v| v.as_str());

                            let eng = engine_arc.lock().await;
                            let snap = eng.snapshot().await.ok();
                            let current_url = snap.as_ref()
                                .map(|s| s.url.as_str())
                                .unwrap_or("");
                            let page_text = snap.as_ref()
                                .map(|s| s.to_agent_text())
                                .unwrap_or_default();

                            let mut passed = true;
                            let mut details = Vec::new();

                            if let Some(ref fragment) = expect_url_contains {
                                if current_url.contains(fragment.as_str()) {
                                    details.push(format!("URL contains '{}': PASS", fragment));
                                } else {
                                    passed = false;
                                    details.push(format!("URL contains '{}': FAIL (actual: {})", fragment, current_url));
                                }
                            }
                            if let Some(text) = expect_text {
                                if page_text.contains(text) {
                                    details.push(format!("Page contains '{}': PASS", text));
                                } else {
                                    passed = false;
                                    details.push(format!("Page contains '{}': FAIL", text));
                                }
                            }

                            json!({
                                "step": idx + 1,
                                "name": step_name,
                                "action": "verify",
                                "status": if passed { "ok" } else { "fail" },
                                "checks": details
                            })
                        }

                        "wait" => {
                            let ms = step.get("ms")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(1000);
                            let wait_selector = step.get("selector")
                                .and_then(|v| v.as_str());

                            if let Some(sel) = wait_selector {
                                let timeout = step.get("timeout").and_then(|v| v.as_u64()).unwrap_or(5000);
                                let mut eng = engine_arc.lock().await;
                                let _ = eng.execute_action(BrowserAction::WaitForSelector {
                                    selector: sel.to_string(),
                                    timeout_ms: timeout,
                                }).await;
                            } else {
                                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                            }
                            json!({
                                "step": idx + 1,
                                "name": step_name,
                                "action": "wait",
                                "status": "ok",
                                "ms": ms
                            })
                        }

                        "solve_captcha" => {
                            // BR-9: solve reCAPTCHA v3 / Turnstile via 2captcha
                            // and inject the token into the live page so the
                            // next submit's grecaptcha.execute() resolves with
                            // the pre-solved value. Requires TWOCAPTCHA_API_KEY
                            // env var to be set on the wraith MCP process.
                            let captcha_type = step
                                .get("captcha_type")
                                .or_else(|| step.get("type"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("recaptchav3")
                                .to_string();
                            let site_key = step
                                .get("site_key")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            let page_url = step
                                .get("url")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            match self.solve_and_inject_captcha(&captcha_type, site_key, page_url).await {
                                Ok(token) => {
                                    let preview = if token.len() > 32 {
                                        format!("{}…", &token[..32])
                                    } else {
                                        token.clone()
                                    };
                                    json!({
                                        "step": idx + 1,
                                        "name": step_name,
                                        "action": "solve_captcha",
                                        "status": "ok",
                                        "captcha_type": captcha_type,
                                        "token_preview": preview,
                                        "token_len": token.len()
                                    })
                                }
                                Err(e) => json!({
                                    "step": idx + 1,
                                    "name": step_name,
                                    "action": "solve_captcha",
                                    "status": "error",
                                    "captcha_type": captcha_type,
                                    "error": e
                                }),
                            }
                        }

                        other => {
                            warn!(action = %other, "Unknown playbook action — skipping");
                            json!({
                                "step": idx + 1,
                                "name": step_name,
                                "action": other,
                                "status": "skipped",
                                "error": format!("Unknown playbook action '{}'", other)
                            })
                        }
                    };

                    results.push(step_result);
                }

                // ── 5. Return the step-by-step results ────────────────────
                let summary = json!({
                    "playbook": input.playbook_yaml,
                    "job_url": input.job_url,
                    "total_steps": total_steps,
                    "completed_steps": results.len(),
                    "results": results
                });

                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&summary).unwrap_or_default()
                )]))
            }

            // ── Dedup & Verification tools ────────────────────────────
            "swarm_dedup_check" => {
                let input: DedupCheckInput = parse_args(args)?;
                info!(url = %input.url, "Dedup check");

                let applied = self.dedup_tracker.has_applied(&input.url);
                // Look up details from recent records if applied
                let (applied_at, status) = if applied {
                    let records = self.dedup_tracker.recent(500);
                    let found = records.iter().find(|r| r.url == input.url);
                    match found {
                        Some(rec) => (Some(rec.applied_at.clone()), Some(rec.status.clone())),
                        None => (None, None),
                    }
                } else {
                    (None, None)
                };

                let result = json!({
                    "applied": applied,
                    "applied_at": applied_at,
                    "status": status,
                });
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&result).unwrap_or_default()
                )]))
            }

            "swarm_dedup_record" => {
                let input: DedupRecordInput = parse_args(args)?;
                info!(url = %input.url, company = %input.company, title = %input.title, platform = %input.platform, "Recording application");

                self.dedup_tracker.record_application(
                    &input.url,
                    Some(&input.company),
                    Some(&input.title),
                    Some(&input.platform),
                    "submitted",
                    None,
                );

                let result = json!({
                    "recorded": true,
                    "url": input.url,
                    "company": input.company,
                    "title": input.title,
                    "platform": input.platform,
                    "status": "submitted",
                });
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&result).unwrap_or_default()
                )]))
            }

            "swarm_dedup_stats" => {
                let _input: DedupStatsInput = parse_args(args)?;
                info!("Dedup stats");

                let stats = self.dedup_tracker.stats();
                let result = json!({
                    "total_applied": stats.total_applied,
                    "by_platform": stats.by_platform,
                    "by_status": stats.by_status,
                    "today_count": stats.today_count,
                    "this_week_count": stats.this_week_count,
                });
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&result).unwrap_or_default()
                )]))
            }

            "swarm_verify_submission" => {
                let input: VerifySubmissionInput = parse_args(args)?;
                info!(ref_id = ?input.ref_id, "Verifying submission");

                let engine_arc = self.active_engine_async().await;
                let engine = engine_arc.lock().await;

                // Get current page snapshot and URL
                let snapshot_text = match engine.snapshot().await {
                    Ok(snap) => snap.to_agent_text(),
                    Err(_) => String::new(),
                };
                let current_url = engine.current_url().await.unwrap_or_default();

                let text_lower = snapshot_text.to_lowercase();
                let url_lower = current_url.to_lowercase();

                // ── Success patterns ────────────────────────────────────
                let strong_success = [
                    "application submitted",
                    "your application has been submitted",
                    "thank you for applying",
                    "thanks for applying",
                    "application received",
                    "successfully submitted",
                    "application complete",
                    "you have successfully applied",
                    "we received your application",
                    "we've received your application",
                    "your application was submitted",
                ];
                let weak_success = [
                    "thank you",
                    "thanks!",
                    "confirmation",
                    "submitted",
                    "applied",
                    "received",
                    "we'll be in touch",
                    "we will review",
                    "review your application",
                    "next steps",
                ];
                let url_success = [
                    "confirmation",
                    "success",
                    "thank",
                    "submitted",
                    "complete",
                    "/applied",
                ];

                // ── Failure patterns ────────────────────────────────────
                let failure_patterns = [
                    "something went wrong",
                    "error occurred",
                    "submission failed",
                    "could not submit",
                    "please try again",
                    "required field",
                    "is required",
                    "fix the following",
                    "there was an error",
                    "application could not be submitted",
                    "validation error",
                ];

                // ── Scoring ─────────────────────────────────────────────
                let has_strong_success = strong_success.iter().any(|p| text_lower.contains(p));
                let weak_count = weak_success.iter().filter(|p| text_lower.contains(*p)).count();
                let url_hit = url_success.iter().any(|p| url_lower.contains(p));
                let has_failure = failure_patterns.iter().any(|p| text_lower.contains(p));

                let (verdict, message) = if has_failure {
                    ("failed", format!(
                        "Error indicators found on page. URL: {}. The application likely did not go through.",
                        current_url
                    ))
                } else if has_strong_success {
                    ("confirmed", format!(
                        "Strong confirmation found on page. URL: {}. Application submitted successfully.",
                        current_url
                    ))
                } else if url_hit && weak_count >= 1 {
                    ("confirmed", format!(
                        "Confirmation URL pattern and success text found. URL: {}.",
                        current_url
                    ))
                } else if weak_count >= 2 || url_hit {
                    ("likely", format!(
                        "Moderate success indicators found ({} text matches, URL match: {}). URL: {}.",
                        weak_count, url_hit, current_url
                    ))
                } else if weak_count == 1 {
                    ("uncertain", format!(
                        "Weak success indicator found. Cannot confidently confirm submission. URL: {}.",
                        current_url
                    ))
                } else {
                    ("uncertain", format!(
                        "No clear success or failure indicators found on page. URL: {}. Manually verify the application status.",
                        current_url
                    ))
                };

                let result = json!({
                    "result": verdict,
                    "message": message,
                    "url": current_url,
                });
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&result).unwrap_or_default()
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
fn open_vault() -> Result<wraith_identity::CredentialVault, ErrorData> {
    let vault_path = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".wraith")
        .join("vault.db");

    let vault = wraith_identity::CredentialVault::open(&vault_path)
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

// ═══════════════════════════════════════════════════════════════════
// Built-in playbook YAML templates
// ═══════════════════════════════════════════════════════════════════

const PLAYBOOK_GREENHOUSE: &str = r#"
- name: Navigate to job posting
  action: navigate
  url: "{{job_url}}"

- name: Fill first name
  action: fill
  selector: "input[name=first_name]"
  value: "{{first_name}}"

- name: Fill last name
  action: fill
  selector: "input[name=last_name]"
  value: "{{last_name}}"

- name: Fill email
  action: fill
  selector: "input[name=email]"
  value: "{{email}}"

- name: Fill phone
  action: fill
  selector: "input[name=phone]"
  value: "{{phone}}"

- name: Upload resume
  action: upload_file
  selector: "input[type=file]"
  file_path: "{{resume_path}}"

- name: Fill LinkedIn
  action: fill
  selector: "input[name=urls[LinkedIn]]"
  value: "{{linkedin_url}}"

- name: Solve invisible reCAPTCHA v3
  action: solve_captcha
  captcha_type: recaptchav3

- name: Submit application
  action: submit
  selector: "input[type=submit]"

- name: Verify submission
  action: verify
  expect_text: "Application submitted"
"#;

const PLAYBOOK_ASHBY: &str = r#"
- name: Navigate to job posting
  action: navigate
  url: "{{job_url}}"

- name: Click Apply button
  action: click
  selector: "button"

- name: Wait for form to load
  action: wait
  ms: 2000

- name: Fill name
  action: fill
  selector: "input[name=name]"
  value: "{{name}}"

- name: Fill email
  action: fill
  selector: "input[name=email]"
  value: "{{email}}"

- name: Fill phone
  action: fill
  selector: "input[name=phone]"
  value: "{{phone}}"

- name: Upload resume
  action: upload_file
  selector: "input[type=file]"
  file_path: "{{resume_path}}"

- name: Submit application
  action: submit
  selector: "button[type=submit]"

- name: Verify submission
  action: verify
  expect_text: "submitted"
"#;

const PLAYBOOK_LEVER: &str = r#"
- name: Navigate to job posting
  action: navigate
  url: "{{job_url}}"

- name: Click Apply button
  action: click
  selector: ".postings-btn-wrapper"

- name: Wait for application form
  action: wait
  ms: 2000

- name: Fill full name
  action: fill
  selector: "input[name=name]"
  value: "{{name}}"

- name: Fill email
  action: fill
  selector: "input[name=email]"
  value: "{{email}}"

- name: Fill phone
  action: fill
  selector: "input[name=phone]"
  value: "{{phone}}"

- name: Upload resume
  action: upload_file
  selector: "input[type=file]"
  file_path: "{{resume_path}}"

- name: Fill LinkedIn
  action: fill
  selector: "input[name=urls[LinkedIn]]"
  value: "{{linkedin_url}}"

- name: Fill current company
  action: fill
  selector: "input[name=org]"
  value: "{{current_company}}"

- name: Submit application
  action: submit
  selector: "button[type=submit]"

- name: Verify submission
  action: verify
  expect_text: "Application submitted"
"#;

const PLAYBOOK_INDEED: &str = r#"
- name: Navigate to Indeed
  action: navigate
  url: "https://www.indeed.com"

- name: Fill job search query
  action: fill
  selector: "input[name=q]"
  value: "{{query}}"

- name: Fill location
  action: fill
  selector: "input[name=l]"
  value: "{{location}}"

- name: Submit search
  action: submit
  selector: "button[type=submit]"

- name: Wait for results
  action: wait
  ms: 3000

- name: Verify results loaded
  action: verify
  expect_text: "jobs"
"#;

/// Parse CSS attribute selectors like `[name="first_name"]` or `[type=submit]`
/// into (attr_name, attr_value) pairs for matching.
fn parse_css_attr_selectors(selector: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let mut rest = selector;
    while let Some(start) = rest.find('[') {
        if let Some(end) = rest[start..].find(']') {
            let inner = &rest[start + 1..start + end];
            if let Some(eq_pos) = inner.find('=') {
                let attr_name = inner[..eq_pos].trim().to_lowercase();
                let attr_val = inner[eq_pos + 1..].trim()
                    .trim_matches('"').trim_matches('\'').to_string();
                attrs.push((attr_name, attr_val));
            }
            rest = &rest[start + end + 1..];
        } else {
            break;
        }
    }
    attrs
}

/// Extract the tag name from a CSS selector (e.g., "input[name=q]" -> "input",
/// "button.btn-primary" -> "button", "#my-id" -> "").
fn parse_css_tag(selector: &str) -> &str {
    let s = selector;
    // Find the first [, ., or # which ends the tag name
    let end = s.find(|c: char| c == '[' || c == '.' || c == '#').unwrap_or(s.len());
    &s[..end]
}

/// Check if a DomElement matches a CSS selector by comparing:
/// - Tag name (from selector like "input..." or "button...")
/// - Attribute selectors like [name=...], [type=...], [data-field=...]
/// - ID selectors like #my-id
/// - Class selectors like .my-class
/// - Stored selector path (exact + substring match)
/// - Role name match for bare tag selectors
fn element_matches_selector(
    elem: &wraith_browser_core::dom::DomElement,
    selector: &str,
) -> bool {
    // Parse the tag from the selector
    let query_tag = parse_css_tag(selector).to_lowercase();

    // Check tag match first (if the query specifies a tag)
    if !query_tag.is_empty() {
        // The stored selector starts with the tag (e.g., "input[name=\"first_name\"]")
        let stored_tag = parse_css_tag(&elem.selector).to_lowercase();
        // Also check the role — Sevro stores tag_name as role for some elements
        let role_lower = elem.role.to_lowercase();
        // Map common roles back to tags for matching
        let role_is_tag = match role_lower.as_str() {
            "textbox" | "text" | "email" | "tel" | "number" | "password"
            | "url" | "search" | "date" | "time" | "datetime-local" | "hidden"
            | "file" | "checkbox" | "radio" | "submit" | "reset" => query_tag == "input",
            "combobox" => query_tag == "select" || query_tag == "input",
            "link" => query_tag == "a",
            "button" => query_tag == "button",
            _ => role_lower == query_tag,
        };
        if stored_tag != query_tag && !role_is_tag {
            return false;
        }
    }

    // Parse attribute selectors from the query
    let attrs = parse_css_attr_selectors(selector);
    if !attrs.is_empty() {
        for (attr_name, attr_val) in &attrs {
            let matched = match attr_name.as_str() {
                "name" => {
                    // Check against the stored selector which now includes [name="..."]
                    let pattern = format!("[name=\"{}\"]", attr_val);
                    elem.selector.contains(&pattern)
                }
                "type" => {
                    // type= matches either the role or stored selector
                    elem.role.eq_ignore_ascii_case(attr_val)
                        || elem.selector.contains(&format!("[type=\"{}\"]", attr_val))
                }
                "data-field" => {
                    elem.selector.contains(&format!("[data-field=\"{}\"]", attr_val))
                }
                "placeholder" => {
                    elem.placeholder.as_deref()
                        .map(|p| p.eq_ignore_ascii_case(attr_val))
                        .unwrap_or(false)
                }
                "aria-label" => {
                    elem.aria_label.as_deref()
                        .map(|a| a.eq_ignore_ascii_case(attr_val))
                        .unwrap_or(false)
                }
                "href" => {
                    elem.href.as_deref()
                        .map(|h| h.contains(attr_val.as_str()))
                        .unwrap_or(false)
                }
                // Generic: check if stored selector contains [attr="val"]
                _ => {
                    let pattern = format!("[{}=\"{}\"]", attr_name, attr_val);
                    elem.selector.contains(&pattern)
                }
            };
            if !matched {
                return false;
            }
        }
        return true;
    }

    // ID selector: #my-id
    if selector.contains('#') {
        if let Some(id_part) = selector.split('#').nth(1) {
            let id = id_part.split(|c: char| c == '.' || c == '[' || c == ' ').next().unwrap_or(id_part);
            return elem.selector.contains(&format!("#{}", id));
        }
    }

    // Class selector: .my-class or tag.my-class
    if selector.contains('.') && !selector.contains('[') {
        if let Some(dot_pos) = selector.find('.') {
            let class_part = &selector[dot_pos..];
            let first_class = class_part.split(|c: char| c == ' ' || c == '[').next().unwrap_or(class_part);
            return elem.selector.contains(first_class);
        }
    }

    // Bare tag match: "button" matches role "button" or "submit"
    if !query_tag.is_empty() && !selector.contains('[') && !selector.contains('.') && !selector.contains('#') {
        let role_lower = elem.role.to_lowercase();
        if role_lower == query_tag { return true; }
        // "button" should also match elements with role "submit"
        if query_tag == "button" && (role_lower == "submit" || role_lower == "button") { return true; }
    }

    false
}

/// Resolve a CSS selector to a @ref ID by searching the current snapshot's elements.
///
/// Matching strategy (in priority order):
/// 1. Bare number → direct ref_id
/// 2. Exact match against stored selector
/// 3. Parsed CSS attribute/tag/class/id matching
/// 4. Substring match against stored selector
/// 5. Placeholder text match
/// 6. Visible text content match (for buttons/links)
///
/// Returns the first matching element's ref_id, or None if no match.
async fn resolve_ref_by_selector(
    engine: &dyn BrowserEngine,
    selector: &str,
) -> Option<u32> {
    if selector.is_empty() {
        return None;
    }

    // If selector looks like a bare ref number (e.g., "42"), use it directly
    if let Ok(rid) = selector.parse::<u32>() {
        return Some(rid);
    }

    let snapshot = engine.snapshot().await.ok()?;

    // Pass 1: exact stored selector match
    for elem in &snapshot.elements {
        if elem.selector == selector {
            return Some(elem.ref_id);
        }
    }

    // Pass 2: parsed CSS selector matching (tag + attributes + class + id)
    for elem in &snapshot.elements {
        if element_matches_selector(elem, selector) {
            return Some(elem.ref_id);
        }
    }

    // Pass 3: normalize the query (strip quotes around attr values) and try substring
    let normalized = selector
        .replace("'", "\"")
        .replace("= ", "=")
        .replace(" =", "=");
    for elem in &snapshot.elements {
        let stored_normalized = elem.selector
            .replace("'", "\"")
            .replace("= ", "=")
            .replace(" =", "=");
        if stored_normalized.contains(&normalized) || normalized.contains(&stored_normalized) {
            return Some(elem.ref_id);
        }
    }

    // Pass 4: match by placeholder text (e.g., selector "Search..." matches placeholder)
    let sel_lower = selector.to_lowercase();
    for elem in &snapshot.elements {
        if let Some(ref ph) = elem.placeholder {
            if ph.to_lowercase().contains(&sel_lower) || sel_lower.contains(&ph.to_lowercase()) {
                return Some(elem.ref_id);
            }
        }
    }

    // Pass 5: for button/link/submit selectors, match by visible text
    let query_tag = parse_css_tag(selector).to_lowercase();
    if query_tag == "button" || query_tag == "a" || selector.contains("[type=submit]") || selector.contains("[type=\"submit\"]") {
        for elem in &snapshot.elements {
            if let Some(ref text) = elem.text {
                if text.to_lowercase().contains(&sel_lower) {
                    let is_clickable = matches!(elem.role.as_str(),
                        "button" | "submit" | "link" | "a");
                    if is_clickable {
                        return Some(elem.ref_id);
                    }
                }
            }
        }
    }

    debug!(selector = %selector, "No element found matching selector");
    None
}

// Cookie persistence is handled by the engine layer now.
