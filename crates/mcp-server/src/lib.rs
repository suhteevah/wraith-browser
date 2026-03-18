//! # openclaw-mcp-server
//!
//! MCP (Model Context Protocol) server that exposes OpenClaw Browser
//! capabilities as tools for Claude Code, Cursor, and other AI agents.
//!
//! ## Tools Exposed
//!
//! | Tool | Description |
//! |------|-------------|
//! | `browse_navigate` | Navigate to a URL, return DOM snapshot |
//! | `browse_click` | Click an element by @ref ID |
//! | `browse_fill` | Fill a form field by @ref ID |
//! | `browse_snapshot` | Get current page DOM snapshot |
//! | `browse_extract` | Extract page content as markdown |
//! | `browse_screenshot` | Capture page screenshot |
//! | `browse_search` | Web metasearch |
//! | `browse_eval_js` | Execute JavaScript on the page |
//! | `browse_tabs` | List open tabs |
//! | `browse_back` | Go back in history |
//! | `browse_key_press` | Press a keyboard key |
//! | `browse_scroll` | Scroll the page |
//! | `browse_vault_store` | Store credential in vault |
//! | `browse_vault_get` | Get credential from vault |
//!
//! ## Transport
//!
//! Supports stdio transport for Claude Code integration.

pub mod tools;
pub mod server;

use server::WraithHandler;
use tracing::info;

/// Start the MCP server with the given transport mode.
pub async fn run(transport: Transport) -> anyhow::Result<()> {
    info!(transport = ?transport, "Starting Wraith MCP Server");

    let handler = WraithHandler::new();

    match transport {
        Transport::Stdio => {
            info!("MCP server running on stdio");
            let transport = rmcp::transport::io::stdio();
            let service = rmcp::serve_server(handler, transport)
                .await
                .map_err(|e| anyhow::anyhow!("MCP server init failed: {e}"))?;

            info!("MCP server initialized, waiting for requests");
            service.waiting().await
                .map_err(|e| anyhow::anyhow!("MCP server error: {e}"))?;
        }
    }

    info!("MCP server shutdown");
    Ok(())
}

#[derive(Debug, Clone)]
pub enum Transport {
    Stdio,
}
