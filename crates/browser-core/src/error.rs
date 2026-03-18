use thiserror::Error;

#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("Chrome launch failed: {0}")]
    LaunchFailed(String),

    #[error("Navigation failed for {url}: {reason}")]
    NavigationFailed { url: String, reason: String },

    #[error("Element not found: {selector}")]
    ElementNotFound { selector: String },

    #[error("Action timeout after {ms}ms: {action}")]
    Timeout { action: String, ms: u64 },

    #[error("CDP protocol error: {0}")]
    CdpError(String),

    #[error("Tab {tab_id} not found")]
    TabNotFound { tab_id: String },

    #[error("Screenshot failed: {0}")]
    ScreenshotFailed(String),

    #[error("JavaScript evaluation failed: {0}")]
    JsEvalFailed(String),

    #[error(transparent)]
    Chrome(#[from] chromiumoxide::error::CdpError),

    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

pub type BrowserResult<T> = Result<T, BrowserError>;
