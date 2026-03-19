//! # Unified Browser Engine Trait
//!
//! All browser backends (NativeClient, ChromeEngine, SevroEngine) implement
//! this trait. The MCP server, agent loop, and CLI operate through
//! `Arc<tokio::sync::Mutex<dyn BrowserEngine>>` — they never know which
//! backend is running.

use crate::dom::DomSnapshot;
use crate::actions::{BrowserAction, ActionResult};
use crate::error::BrowserResult;
use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::warn;

/// Screenshot capability level.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ScreenshotCapability {
    /// Cannot take screenshots
    None,
    /// Viewport-only screenshots
    ViewportOnly,
    /// Full-page screenshots
    FullPage,
}

/// What a browser engine can do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineCapabilities {
    /// Can execute JavaScript (SpiderMonkey, V8, etc.)
    pub javascript: bool,
    /// Screenshot support level
    pub screenshots: ScreenshotCapability,
    /// Can compute element bounding boxes / layout
    pub layout: bool,
    /// Persistent cookie jar
    pub cookies: bool,
    /// Can inject stealth evasions
    pub stealth: bool,
}

/// Unified browser engine interface.
///
/// Object-safe via `async_trait`. Stored behind `Arc<tokio::sync::Mutex<dyn BrowserEngine>>`.
/// All methods are async to accommodate CDP round-trips in ChromeEngine.
/// NativeClient and SevroEngine wrap sync operations in async (zero-cost).
#[async_trait]
pub trait BrowserEngine: Send + Sync {
    /// Navigate to a URL. Waits for DOMContentLoaded equivalent.
    async fn navigate(&mut self, url: &str) -> BrowserResult<()>;

    /// Take a DOM snapshot optimized for AI agent consumption.
    async fn snapshot(&self) -> BrowserResult<DomSnapshot>;

    /// Execute a browser action (click, fill, scroll, etc.).
    async fn execute_action(&mut self, action: BrowserAction) -> BrowserResult<ActionResult>;

    /// Execute arbitrary JavaScript and return the result as a string.
    async fn eval_js(&self, script: &str) -> BrowserResult<String>;

    /// Get the current page's raw HTML source.
    async fn page_source(&self) -> BrowserResult<String>;

    /// Get the current URL.
    async fn current_url(&self) -> Option<String>;

    /// Take a screenshot (returns PNG bytes).
    async fn screenshot(&self) -> BrowserResult<Vec<u8>>;

    /// What this engine can do.
    fn capabilities(&self) -> EngineCapabilities;

    /// Gracefully shut down the engine and release resources.
    async fn shutdown(&mut self) -> BrowserResult<()>;
}

/// Create a browser engine by name.
///
/// - `"native"` — NativeEngine (pure HTTP, always available)
/// - `"chrome"` — ChromeEngine (requires `chrome-legacy` feature + Chrome installed)
/// - `"sevro"` — SevroEngine (future)
/// - `"auto"` — try chrome, fall back to native
pub async fn create_engine(name: &str) -> BrowserResult<Arc<Mutex<dyn BrowserEngine>>> {
    match name {
        "native" => {
            Ok(Arc::new(Mutex::new(crate::engine_native::NativeEngine::new())))
        }
        #[cfg(feature = "chrome-legacy")]
        "chrome" => {
            let engine = crate::engine_chrome::ChromeEngine::launch(
                crate::config::BrowserConfig::default()
            ).await?;
            Ok(Arc::new(Mutex::new(engine)))
        }
        #[cfg(not(feature = "chrome-legacy"))]
        "chrome" => {
            Err(crate::error::BrowserError::CdpError(
                "Chrome engine not available — compile with --features chrome-legacy".to_string()
            ))
        }
        "sevro" => {
            Err(crate::error::BrowserError::CdpError(
                "Sevro engine not yet implemented".to_string()
            ))
        }
        "auto" => {
            // Future: try sevro first
            #[cfg(feature = "chrome-legacy")]
            {
                match crate::engine_chrome::ChromeEngine::launch(
                    crate::config::BrowserConfig::default()
                ).await {
                    Ok(engine) => return Ok(Arc::new(Mutex::new(engine))),
                    Err(e) => {
                        warn!(error = %e, "Chrome not available, falling back to native");
                    }
                }
            }
            Ok(Arc::new(Mutex::new(crate::engine_native::NativeEngine::new())))
        }
        other => Err(crate::error::BrowserError::CdpError(
            format!("Unknown engine: '{}'. Options: native, chrome, sevro, auto", other)
        )),
    }
}
