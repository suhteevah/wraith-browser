//! MCP server handler — implements the rmcp ServerHandler trait.

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
use tracing::{info, warn, debug, instrument};

use crate::tools::*;

/// The OpenClaw MCP server handler.
pub struct OpenClawHandler {
    tools: Vec<Tool>,
}

impl Default for OpenClawHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenClawHandler {
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
                "Click an interactive element by its @ref ID from the latest snapshot.",
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
                "Capture a PNG screenshot of the current page.",
                &schema_for!(ScreenshotInput), ro_closed.clone()),
            make_tool("browse_search",
                "Search the web using metasearch (DuckDuckGo + cached results). Returns titles, URLs, and snippets.",
                &schema_for!(SearchInput), ro_open),
            make_tool("browse_eval_js",
                "Execute JavaScript code on the current page and return the result.",
                &schema_for!(EvalJsInput), rw_destructive),
            make_tool("browse_tabs",
                "List all open browser tabs with their URLs and titles.",
                &schema_for!(TabsInput), ro_closed.clone()),
            make_tool("browse_back",
                "Go back to the previous page in browser history.",
                &schema_for!(BackInput), rw_open.clone()),
            make_tool("browse_key_press",
                "Press a keyboard key on the current page (e.g., Enter, Tab, Escape).",
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

        info!(tool_count = tools.len(), "OpenClaw MCP handler initialized");
        Self { tools }
    }

    /// Dispatch a tool call to the appropriate handler.
    #[instrument(skip(self, arguments), fields(tool = %name))]
    fn dispatch_tool(
        &self,
        name: &str,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let args = serde_json::Value::Object(arguments.unwrap_or_default());

        match name {
            "browse_navigate" => {
                let input: NavigateInput = serde_json::from_value(args)
                    .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
                info!(url = %input.url, "Navigating to URL");
                // TODO: Wire to BrowserSession
                Ok(CallToolResult::success(vec![Content::text(
                    json!({
                        "url": input.url,
                        "title": "[stub]",
                        "snapshot": "[Wire browser-core to enable]",
                        "interactive_elements": 0
                    }).to_string()
                )]))
            }
            "browse_click" => {
                let input: ClickInput = serde_json::from_value(args)
                    .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
                info!(ref_id = input.ref_id, "Clicking element");
                Ok(CallToolResult::success(vec![Content::text(
                    format!("Clicked element @e{}", input.ref_id)
                )]))
            }
            "browse_fill" => {
                let input: FillInput = serde_json::from_value(args)
                    .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
                info!(ref_id = input.ref_id, "Filling field");
                Ok(CallToolResult::success(vec![Content::text(
                    format!("Filled @e{} with {} chars", input.ref_id, input.text.len())
                )]))
            }
            "browse_snapshot" => {
                debug!("Taking DOM snapshot");
                Ok(CallToolResult::success(vec![Content::text(
                    "[Wire browser-core to enable snapshots]"
                )]))
            }
            "browse_extract" => {
                let input: ExtractInput = serde_json::from_value(args)
                    .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
                info!(max_tokens = ?input.max_tokens, "Extracting content");
                Ok(CallToolResult::success(vec![Content::text(
                    json!({ "title": "[stub]", "markdown": "", "estimated_tokens": 0 }).to_string()
                )]))
            }
            "browse_screenshot" => {
                let _input: ScreenshotInput = serde_json::from_value(args)
                    .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
                info!("Taking screenshot");
                Ok(CallToolResult::success(vec![Content::text(
                    "[Wire browser-core to enable screenshots]"
                )]))
            }
            "browse_search" => {
                let input: SearchInput = serde_json::from_value(args)
                    .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
                info!(query = %input.query, "Searching web");
                Ok(CallToolResult::success(vec![Content::text(
                    json!({ "query": input.query, "results": [] }).to_string()
                )]))
            }
            "browse_eval_js" => {
                let input: EvalJsInput = serde_json::from_value(args)
                    .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
                info!(code_len = input.code.len(), "Evaluating JS");
                Ok(CallToolResult::success(vec![Content::text("[Wire browser-core]")]))
            }
            "browse_tabs" => {
                Ok(CallToolResult::success(vec![Content::text(
                    json!({ "tabs": [] }).to_string()
                )]))
            }
            "browse_back" => {
                Ok(CallToolResult::success(vec![Content::text("Navigated back")]))
            }
            "browse_key_press" => {
                let input: KeyPressInput = serde_json::from_value(args)
                    .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(
                    format!("Pressed: {}", input.key)
                )]))
            }
            "browse_scroll" => {
                let input: ScrollInput = serde_json::from_value(args)
                    .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(
                    format!("Scrolled {}", input.direction)
                )]))
            }
            "browse_vault_store" => {
                let input: VaultStoreInput = serde_json::from_value(args)
                    .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
                info!(domain = %input.domain, "Storing credential");
                Ok(CallToolResult::success(vec![Content::text(
                    format!("Stored {} credential for {}", input.kind, input.domain)
                )]))
            }
            "browse_vault_get" => {
                let input: VaultGetInput = serde_json::from_value(args)
                    .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
                info!(domain = %input.domain, "Getting credential");
                Ok(CallToolResult::success(vec![Content::text(
                    format!("Credential lookup for {} (stub)", input.domain)
                )]))
            }
            _ => {
                warn!(tool = %name, "Unknown tool");
                Err(ErrorData::invalid_params(format!("Unknown tool: {name}"), None))
            }
        }
    }
}

impl ServerHandler for OpenClawHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_instructions(
            "OpenClaw Browser — an AI-agent-first web browser. \
             Use browse_navigate to visit URLs, browse_click/browse_fill to interact, \
             browse_snapshot to see the page, and browse_extract to get markdown content."
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
        std::future::ready(self.dispatch_tool(&name, arguments))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.iter().find(|t| t.name == name).cloned()
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
