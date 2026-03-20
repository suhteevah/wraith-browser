//! MCP tool input definitions for every Wraith Browser capability.
//! Each struct maps to exactly one MCP tool with AI-friendly descriptions.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════════════════════════
// NAVIGATION
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NavigateInput {
    /// The full URL to navigate to (e.g., "https://www.example.com/page"). Must include protocol.
    pub url: String,
    /// Whether to wait for page load before returning snapshot. Default: true.
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BackInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ForwardInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReloadInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScrollInput {
    /// Direction: "up", "down", "left", or "right"
    pub direction: String,
    /// Pixels to scroll. Default: 500.
    pub amount: Option<i32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WaitInput {
    /// CSS selector to wait for (e.g., "#results", ".job-card"). If omitted, waits for fixed time.
    pub selector: Option<String>,
    /// Milliseconds to wait. Default: 1000 for fixed wait, 5000 timeout for selector wait.
    pub ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WaitForNavigationInput {
    /// Timeout in milliseconds. Default: 5000. Use after clicking a link that triggers page navigation.
    pub timeout_ms: Option<u64>,
}

// ═══════════════════════════════════════════════════════════════════
// INTERACTION
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClickInput {
    /// The @ref ID number from the snapshot (e.g., 5 means the element shown as @e5).
    pub ref_id: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FillInput {
    /// The @ref ID of the form field from the snapshot.
    pub ref_id: u32,
    /// The text to fill into the field. Replaces existing content.
    pub text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SelectInput {
    /// The @ref ID of the <select> dropdown from the snapshot.
    pub ref_id: u32,
    /// The option value to select.
    pub value: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TypeTextInput {
    /// The @ref ID of the input field from the snapshot.
    pub ref_id: u32,
    /// The text to type character by character.
    pub text: String,
    /// Delay between keystrokes in milliseconds. Default: 50. Higher values look more human.
    pub delay_ms: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HoverInput {
    /// The @ref ID of the element to hover over.
    pub ref_id: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KeyPressInput {
    /// Key name: "Enter", "Tab", "Escape", "ArrowDown", "Backspace", etc.
    pub key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DomFocusInput {
    /// The @ref ID of the element to focus.
    pub ref_id: u32,
}

// ═══════════════════════════════════════════════════════════════════
// EXTRACTION & DOM
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SnapshotInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExtractInput {
    /// Maximum token budget for extracted content. Omit for unlimited.
    pub max_tokens: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ExtractOutput {
    pub title: String,
    pub markdown: String,
    pub estimated_tokens: usize,
    pub links_count: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotInput {
    /// true = capture entire scrollable page, false = visible viewport only. Default: false.
    pub full_page: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EvalJsInput {
    /// JavaScript source code to execute in the page context. Returns the last expression's value as a string.
    pub code: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TabsInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DomQuerySelectorInput {
    /// CSS selector to query (e.g., "div.job-card", "#main-content", "a[href*='apply']").
    pub selector: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DomGetAttributeInput {
    /// The @ref ID of the element.
    pub ref_id: u32,
    /// Attribute name (e.g., "href", "class", "data-job-id", "aria-label").
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DomSetAttributeInput {
    /// The @ref ID of the element to modify.
    pub ref_id: u32,
    /// Attribute name.
    pub name: String,
    /// New attribute value.
    pub value: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExtractPdfInput {
    /// URL of the PDF to extract text from. Will be fetched and parsed.
    pub url: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExtractArticleInput {
    /// If true, extract only the main article body (removes nav, ads, sidebars). Default: true.
    pub readability: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExtractMarkdownInput {
    /// Raw HTML string to convert to markdown. If omitted, uses current page source.
    pub html: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExtractPlainTextInput {
    /// Raw HTML string to convert to plain text. If omitted, uses current page source.
    pub html: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExtractOcrInput {
    /// Description of what to OCR. Currently uses the page screenshot.
    pub description: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════
// SEARCH
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchInput {
    /// Search query string (e.g., "rust async runtime benchmarks").
    pub query: String,
    /// Maximum number of results. Default: 10.
    pub max_results: Option<usize>,
}

// ═══════════════════════════════════════════════════════════════════
// VAULT (credential management)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultStoreInput {
    /// Domain the credential is for (e.g., "github.com", "indeed.com").
    pub domain: String,
    /// Type: "password", "api_key", "oauth_token", "totp_seed", "session_cookie", or "generic".
    pub kind: String,
    /// Username, email, or account identifier.
    pub identity: String,
    /// The secret value (password, API key, token, etc.).
    pub secret: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultGetInput {
    /// Domain to look up credentials for.
    pub domain: String,
    /// Optional type filter: "password", "api_key", "oauth_token", etc.
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultListInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultDeleteInput {
    /// Full credential ID (UUID) to delete. Get IDs from vault_list.
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultTotpInput {
    /// Domain to generate a TOTP 2FA code for. Must have a totp_seed credential stored.
    pub domain: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultRotateInput {
    /// Credential ID to rotate.
    pub id: String,
    /// New secret value.
    pub new_secret: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultAuditInput {
    /// Number of audit log entries to return. Default: 20.
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultLockInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultUnlockInput {
    /// Vault passphrase. Use empty string "" for auto-unlock mode (MCP default).
    pub passphrase: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultApproveDomainInput {
    /// Credential ID.
    pub credential_id: String,
    /// Domain to approve for this credential (e.g., "login.indeed.com").
    pub domain: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultRevokeDomainInput {
    /// Credential ID.
    pub credential_id: String,
    /// Domain to revoke.
    pub domain: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultCheckApprovalInput {
    /// Credential ID.
    pub credential_id: String,
    /// Domain to check.
    pub domain: String,
}

// ═══════════════════════════════════════════════════════════════════
// COOKIES
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CookieGetInput {
    /// Domain to get cookies for.
    pub domain: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CookieSetInput {
    /// Cookie domain (e.g., ".indeed.com").
    pub domain: String,
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
    /// Cookie path. Default: "/".
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CookieSaveInput {
    /// File path to save cookies to. Default: ~/.openclaw/cookies.json.
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CookieLoadInput {
    /// File path to load cookies from. Default: ~/.openclaw/cookies.json.
    pub path: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════
// CACHE (knowledge store)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CacheSearchInput {
    /// Full-text search query across all cached pages.
    pub query: String,
    /// Maximum results. Default: 10.
    pub max_results: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CacheGetInput {
    /// URL to look up in cache.
    pub url: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CacheStatsInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CachePurgeInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CachePinInput {
    /// URL to pin (will never be evicted from cache).
    pub url: String,
    /// Optional note explaining why this page is pinned.
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CacheTagInput {
    /// URL to tag.
    pub url: String,
    /// Tags to apply (e.g., ["job-listing", "remote", "rust"]).
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CacheDomainProfileInput {
    /// Domain to check (e.g., "indeed.com"). Shows how often the domain's content changes.
    pub domain: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CacheFindSimilarInput {
    /// URL to find similar cached pages for.
    pub url: String,
    /// Maximum results. Default: 5.
    pub max_results: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CacheEvictInput {
    /// Maximum cache size in bytes. Pages will be evicted (oldest first) until under this budget.
    pub max_bytes: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CacheRawHtmlInput {
    /// URL to get raw cached HTML for.
    pub url: String,
}

// ═══════════════════════════════════════════════════════════════════
// ENTITY GRAPH (knowledge graph)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EntityQueryInput {
    /// Natural language question about an entity (e.g., "what do we know about Stripe?").
    pub question: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EntityAddInput {
    /// Entity name (e.g., "Stripe", "Indeed", "Rust").
    pub name: String,
    /// Entity type: "company", "person", "technology", "product", "location", or "other".
    pub entity_type: String,
    /// Optional attributes as key-value pairs.
    pub attributes: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EntityRelateInput {
    /// Source entity name.
    pub from: String,
    /// Target entity name.
    pub to: String,
    /// Relationship type (e.g., "uses", "employs", "competes_with", "acquired").
    pub relationship: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EntityMergeInput {
    /// First entity name.
    pub name_a: String,
    /// Second entity name (will be merged into first).
    pub name_b: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EntityFindRelatedInput {
    /// Entity name to find connections for.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EntitySearchInput {
    /// Search query (fuzzy name match across all entities).
    pub query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EntityVisualizeInput {}

// ═══════════════════════════════════════════════════════════════════
// EMBEDDINGS (semantic search)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EmbeddingSearchInput {
    /// Text to find semantically similar content for.
    pub text: String,
    /// Maximum results. Default: 5.
    pub top_k: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EmbeddingUpsertInput {
    /// Unique source ID (usually URL or document ID).
    pub source_id: String,
    /// Text content to embed.
    pub content: String,
}

// ═══════════════════════════════════════════════════════════════════
// AUTH (authentication detection)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AuthDetectInput {
    /// URL to analyze for auth flows. If omitted, uses current page.
    pub url: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════
// IDENTITY (fingerprints, profiles)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FingerprintListInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FingerprintImportInput {
    /// File path to the fingerprint JSON file.
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IdentityProfileInput {
    /// Profile type: "personal" (use real name) or "anonymous".
    pub profile_type: String,
    /// Name for personal profile (e.g., "Matt Gates").
    pub name: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════
// NETWORK INTELLIGENCE
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NetworkDiscoverInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DnsResolveInput {
    /// Domain name to resolve via DNS-over-HTTPS (e.g., "indeed.com").
    pub domain: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StealthStatusInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SiteFingerprintInput {
    /// URL to fingerprint. If omitted, uses current page.
    pub url: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PageDiffInput {
    /// URL to diff against cached version. If omitted, uses current page.
    pub url: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TlsProfilesInput {}

// ═══════════════════════════════════════════════════════════════════
// PLUGINS (WASM)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PluginRegisterInput {
    /// Plugin name.
    pub name: String,
    /// Path to the WASM file.
    pub wasm_path: String,
    /// Plugin description.
    pub description: Option<String>,
    /// Domains this plugin is designed for (e.g., ["amazon.com", "ebay.com"]).
    pub domains: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PluginExecuteInput {
    /// Plugin name to execute.
    pub name: String,
    /// JSON input data for the plugin.
    pub input: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PluginListInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PluginRemoveInput {
    /// Plugin name to unregister.
    pub name: String,
}

// ═══════════════════════════════════════════════════════════════════
// SCRIPTING (Rhai)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScriptLoadInput {
    /// Unique script name.
    pub name: String,
    /// Rhai source code.
    pub source: String,
    /// Trigger: "always" (every page), "manual" (explicit only), or a URL substring for on_navigate.
    pub trigger: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScriptListInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScriptRunInput {
    /// Script name to execute against the current page.
    pub name: String,
}

// ═══════════════════════════════════════════════════════════════════
// TELEMETRY
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TelemetryMetricsInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TelemetrySpansInput {}

// ═══════════════════════════════════════════════════════════════════
// WORKFLOW (record & replay)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorkflowStartRecordingInput {
    /// Name for this workflow (e.g., "indeed-job-apply").
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorkflowStopRecordingInput {
    /// Description of what this workflow does.
    pub description: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorkflowReplayInput {
    /// Workflow name to replay.
    pub name: String,
    /// Variable values to substitute (e.g., {"job_title": "Rust Engineer", "location": "Remote"}).
    pub variables: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorkflowListInput {}

// ═══════════════════════════════════════════════════════════════════
// TIME-TRAVEL (agent debugging)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TimeTravelSummaryInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TimeTravelBranchInput {
    /// Step number to branch from (0-indexed).
    pub step: usize,
    /// Name for the new branch.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TimeTravelReplayInput {
    /// Replay up to this step number.
    pub step: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TimeTravelDiffInput {
    /// First branch ID.
    pub branch_a: String,
    /// Second branch ID.
    pub branch_b: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TimeTravelExportInput {}

// ═══════════════════════════════════════════════════════════════════
// TASK DAG (parallel task orchestration)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DagCreateInput {
    /// DAG name.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DagAddTaskInput {
    /// Unique task ID within the DAG.
    pub task_id: String,
    /// Human-readable task description.
    pub description: String,
    /// Action type: "navigate", "click", "fill", "extract", "custom".
    pub action_type: String,
    /// Action target (URL, selector, etc.).
    pub target: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DagAddDependencyInput {
    /// Task that depends on another.
    pub task_id: String,
    /// Task that must complete first.
    pub depends_on: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DagReadyInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DagCompleteInput {
    /// Task ID to mark as completed.
    pub task_id: String,
    /// Result or output from the task.
    pub result: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DagProgressInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DagVisualizeInput {}

// ═══════════════════════════════════════════════════════════════════
// MCTS (Monte Carlo Tree Search planning)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MctsPlanInput {
    /// Current page state description for the planner.
    pub state: String,
    /// Available actions (e.g., ["click @e1", "fill @e3", "navigate /next"]).
    pub actions: Vec<String>,
    /// Number of MCTS simulations. Default: 100.
    pub simulations: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MctsStatsInput {}

// ═══════════════════════════════════════════════════════════════════
// PREFETCH
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PrefetchPredictInput {
    /// Current task description to predict next URLs for.
    pub task_description: String,
}

// ═══════════════════════════════════════════════════════════════════
// SWARM (parallel browsing)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SwarmFanOutInput {
    /// List of URLs to visit in parallel.
    pub urls: Vec<String>,
    /// Maximum concurrent sessions. Default: 4.
    pub max_concurrent: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SwarmCollectInput {}

// ═══════════════════════════════════════════════════════════════════
// AGENT (autonomous task)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskInput {
    /// Natural language task description (e.g., "Find remote Rust jobs on Indeed and extract titles and URLs").
    pub description: String,
    /// Starting URL. If omitted, agent decides where to start.
    pub url: Option<String>,
    /// Maximum action steps before stopping. Default: 50.
    pub max_steps: Option<usize>,
}

// ═══════════════════════════════════════════════════════════════════
// CONFIG
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ConfigInput {}

// ═══════════════════════════════════════════════════════════════════
// FILE UPLOAD
// ═══════════════════════════════════════════════════════════════════

/// Tool: upload a file to an input[type=file] element on the page.
/// Reads the file from disk, base64-encodes it, and injects it via JavaScript.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UploadFileInput {
    /// The @ref ID of the file input element. Use 0 or omit to auto-detect the first file input.
    pub ref_id: Option<u32>,
    /// Absolute path to the file on disk (e.g., "C:/Users/Matt/Documents/resume.pdf").
    pub file_path: String,
}
