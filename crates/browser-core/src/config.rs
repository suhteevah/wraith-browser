use serde::{Deserialize, Serialize};

/// Configuration for browser session launch and behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserConfig {
    /// Path to Chrome/Chromium binary. None = auto-detect.
    pub chrome_path: Option<String>,

    /// Run headless (default: true for AI agents)
    pub headless: bool,

    /// Viewport width in pixels
    pub viewport_width: u32,

    /// Viewport height in pixels
    pub viewport_height: u32,

    /// Default navigation timeout in milliseconds
    pub navigation_timeout_ms: u64,

    /// Default action timeout in milliseconds
    pub action_timeout_ms: u64,

    /// User agent string override
    pub user_agent: Option<String>,

    /// Extra Chrome launch args
    pub extra_args: Vec<String>,

    /// Enable request interception (for ad blocking, etc.)
    pub intercept_requests: bool,

    /// Block these URL patterns (ads, trackers)
    pub blocked_url_patterns: Vec<String>,

    /// Maximum concurrent tabs
    pub max_tabs: usize,

    /// Enable verbose CDP logging
    pub verbose_cdp_logging: bool,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            chrome_path: None,
            headless: true,
            viewport_width: 1920,
            viewport_height: 1080,
            navigation_timeout_ms: 30_000,
            action_timeout_ms: 10_000,
            user_agent: Some(
                "Wraith-Browser/0.1.0 (AI Agent; +https://github.com/suhteevah/wraith-browser)"
                    .to_string(),
            ),
            extra_args: vec![
                "--disable-gpu".to_string(),
                "--no-sandbox".to_string(),
                "--disable-dev-shm-usage".to_string(),
                "--disable-extensions".to_string(),
                "--disable-background-networking".to_string(),
            ],
            intercept_requests: true,
            blocked_url_patterns: vec![
                "*google-analytics.com*".to_string(),
                "*doubleclick.net*".to_string(),
                "*facebook.com/tr*".to_string(),
                "*hotjar.com*".to_string(),
            ],
            max_tabs: 10,
            verbose_cdp_logging: false,
        }
    }
}
