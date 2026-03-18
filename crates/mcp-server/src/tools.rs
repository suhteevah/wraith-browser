//! MCP tool definitions for OpenClaw Browser.
//! Each tool maps to browser-core actions with AI-friendly input/output schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Tool: navigate to a URL and return a DOM snapshot
#[derive(Debug, Deserialize, JsonSchema)]
pub struct NavigateInput {
    /// The URL to navigate to
    pub url: String,
    /// Wait for page load before returning snapshot (default: true)
    pub wait_for_load: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct NavigateOutput {
    pub url: String,
    pub title: String,
    pub snapshot: String,
    pub page_type: Option<String>,
    pub interactive_elements: usize,
}

/// Tool: click an element by @ref ID from the snapshot
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClickInput {
    /// Element ref ID from snapshot (e.g., 5 for @e5)
    pub ref_id: u32,
}

/// Tool: fill a form field by @ref ID
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FillInput {
    /// Element ref ID from snapshot
    pub ref_id: u32,
    /// Text to fill into the field
    pub text: String,
}

/// Tool: get the current page DOM snapshot
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SnapshotInput {}

/// Tool: extract content as markdown
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExtractInput {
    /// Maximum tokens for the extracted content (default: unlimited)
    pub max_tokens: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ExtractOutput {
    pub title: String,
    pub markdown: String,
    pub estimated_tokens: usize,
    pub links_count: usize,
}

/// Tool: capture a screenshot of the current page
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotInput {
    /// Whether to capture full page or just viewport (default: viewport)
    pub full_page: Option<bool>,
}

/// Tool: search the web
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchInput {
    /// The search query
    pub query: String,
    /// Maximum number of results (default: 10)
    pub max_results: Option<usize>,
}

/// Tool: execute JavaScript on the current page
#[derive(Debug, Deserialize, JsonSchema)]
pub struct EvalJsInput {
    /// JavaScript code to execute
    pub code: String,
}

/// Tool: list open browser tabs
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TabsInput {}

/// Tool: go back in browser history
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BackInput {}

/// Tool: press a keyboard key
#[derive(Debug, Deserialize, JsonSchema)]
pub struct KeyPressInput {
    /// Key to press (e.g., "Enter", "Tab", "Escape")
    pub key: String,
}

/// Tool: scroll the page
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScrollInput {
    /// Direction to scroll: "up" or "down"
    pub direction: String,
    /// Number of pixels to scroll (default: 500)
    pub amount: Option<i32>,
}

/// Tool: store a credential in the vault
#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultStoreInput {
    /// Domain the credential is for (e.g., "github.com")
    pub domain: String,
    /// Credential type: "password", "api_key", "oauth_token", "cookie"
    pub kind: String,
    /// Username or identity
    pub identity: String,
    /// The secret value
    pub secret: String,
}

/// Tool: get a credential from the vault
#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultGetInput {
    /// Domain to look up
    pub domain: String,
    /// Optional credential type filter
    pub kind: Option<String>,
}
