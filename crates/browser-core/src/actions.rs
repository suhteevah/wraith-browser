use serde::{Deserialize, Serialize};

/// Actions an AI agent can perform on a browser tab.
/// Referenced by element ref_id from DomSnapshot (e.g., "click @e5").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BrowserAction {
    /// Navigate to a URL
    Navigate { url: String },

    /// Click an element by ref_id
    Click { ref_id: u32 },

    /// Fill a text input by ref_id
    Fill { ref_id: u32, text: String },

    /// Select an option from a dropdown by ref_id
    Select { ref_id: u32, value: String },

    /// Press a keyboard key (Enter, Tab, Escape, etc.)
    KeyPress { key: String },

    /// Type text with realistic delays (for sites that detect instant input)
    TypeText { ref_id: u32, text: String, delay_ms: u32 },

    /// Scroll the page (pixels or "to_bottom" / "to_top")
    Scroll { direction: ScrollDirection, amount: i32 },

    /// Hover over an element
    Hover { ref_id: u32 },

    /// Go back in browser history
    GoBack,

    /// Go forward in browser history
    GoForward,

    /// Reload the current page
    Reload,

    /// Wait for a specified duration
    Wait { ms: u64 },

    /// Wait for a CSS selector to appear
    WaitForSelector { selector: String, timeout_ms: u64 },

    /// Wait for navigation to complete
    WaitForNavigation { timeout_ms: u64 },

    /// Execute raw JavaScript
    EvalJs { script: String },

    /// Take a screenshot (returns ActionResult::Screenshot)
    Screenshot { full_page: bool },

    /// Extract the page as markdown (delegates to content-extract crate)
    ExtractContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Result of executing a browser action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionResult {
    /// Action completed successfully
    Success { message: String },

    /// Navigation completed, new URL returned
    Navigated { url: String, title: String },

    /// Screenshot captured
    Screenshot { png_base64: String, width: u32, height: u32 },

    /// Content extracted as markdown
    Content { markdown: String, word_count: usize },

    /// JavaScript evaluation result
    JsResult { value: String },

    /// Action failed
    Failed { error: String },
}
