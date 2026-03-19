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

/// Tool: list all stored credentials (secrets stay encrypted)
#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultListInput {}

/// Tool: delete a credential by ID
#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultDeleteInput {
    /// Credential ID to delete
    pub id: String,
}

/// Tool: generate a TOTP 2FA code
#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultTotpInput {
    /// Domain to generate TOTP code for
    pub domain: String,
}

/// Tool: rotate a credential's secret
#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultRotateInput {
    /// Credential ID to rotate
    pub id: String,
    /// New secret value
    pub new_secret: String,
}

/// Tool: view vault audit log
#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultAuditInput {
    /// Number of entries to show (default: 20)
    pub limit: Option<usize>,
}

/// Tool: run an autonomous browsing task
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskInput {
    /// Task description for the AI agent
    pub description: String,
    /// Starting URL (optional)
    pub url: Option<String>,
    /// Maximum steps (default: 50)
    pub max_steps: Option<usize>,
}

/// Tool: search the knowledge cache
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CacheSearchInput {
    /// Search query
    pub query: String,
    /// Maximum results (default: 10)
    pub max_results: Option<usize>,
}

/// Tool: check if a URL is cached
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CacheGetInput {
    /// URL to check
    pub url: String,
}

/// Tool: select a dropdown option
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SelectInput {
    /// Element ref ID from snapshot
    pub ref_id: u32,
    /// Value to select
    pub value: String,
}

/// Tool: type text with realistic delays
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TypeTextInput {
    /// Element ref ID from snapshot
    pub ref_id: u32,
    /// Text to type
    pub text: String,
    /// Delay between keystrokes in ms (default: 50)
    pub delay_ms: Option<u32>,
}

/// Tool: hover over an element
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HoverInput {
    /// Element ref ID from snapshot
    pub ref_id: u32,
}

/// Tool: wait for a condition
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WaitInput {
    /// CSS selector to wait for (optional)
    pub selector: Option<String>,
    /// Milliseconds to wait (default: 1000)
    pub ms: Option<u64>,
}

/// Tool: reload the current page
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReloadInput {}

/// Tool: go forward in browser history
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ForwardInput {}

/// Tool: load a Rhai userscript
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScriptLoadInput {
    /// Script name
    pub name: String,
    /// Rhai source code
    pub source: String,
    /// Trigger: "always", "manual", or a URL pattern for on_navigate
    pub trigger: Option<String>,
}

/// Tool: list loaded scripts
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScriptListInput {}

/// Tool: run a script by name
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScriptRunInput {
    /// Script name to execute
    pub name: String,
}

/// Tool: get engine configuration and status
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ConfigInput {}
