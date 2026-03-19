//! # Unified Browser Engine Trait
//!
//! All browser backends implement this trait. The MCP server, agent loop,
//! and CLI operate through `Arc<tokio::sync::Mutex<dyn BrowserEngine>>` —
//! they never know which backend is running.
//!
//! Available backends:
//! - `NativeEngine` — pure-Rust HTTP client, no JS, ~50ms per page
//! - `SevroEngine` — Servo-derived with QuickJS, DOM, layout (default)

use crate::dom::DomSnapshot;
use crate::actions::{BrowserAction, ActionResult};
use crate::error::BrowserResult;
use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use std::sync::Arc;
use tokio::sync::Mutex;

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
    /// Can execute JavaScript (QuickJS, SpiderMonkey, etc.)
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
/// All methods are async. NativeClient and SevroEngine wrap sync operations
/// in async (zero-cost — no actual suspension).
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
/// - `"sevro"` / `"native-js"` — SevroEngine (default, requires `sevro` feature)
/// - `"auto"` — Sevro if available, otherwise native
pub async fn create_engine(name: &str) -> BrowserResult<Arc<Mutex<dyn BrowserEngine>>> {
    create_engine_with_proxy(name, None).await
}

/// Engine configuration options passed from CLI.
#[derive(Debug, Default, Clone)]
pub struct EngineOptions {
    pub proxy_url: Option<String>,
    pub flaresolverr_url: Option<String>,
    pub fallback_proxy_url: Option<String>,
}

/// Create a browser engine by name with an optional proxy URL.
pub async fn create_engine_with_proxy(
    name: &str,
    proxy_url: Option<String>,
) -> BrowserResult<Arc<Mutex<dyn BrowserEngine>>> {
    create_engine_with_options(name, EngineOptions {
        proxy_url,
        ..Default::default()
    }).await
}

/// Create a browser engine by name with full options.
pub async fn create_engine_with_options(
    name: &str,
    opts: EngineOptions,
) -> BrowserResult<Arc<Mutex<dyn BrowserEngine>>> {
    match name {
        "native" => {
            Ok(Arc::new(Mutex::new(crate::engine_native::NativeEngine::new())))
        }
        #[cfg(feature = "sevro")]
        "sevro" | "native-js" => {
            let mut config = sevro_headless::SevroConfig::default();
            config.proxy_url = opts.proxy_url;
            config.flaresolverr_url = opts.flaresolverr_url;
            config.fallback_proxy_url = opts.fallback_proxy_url;
            Ok(Arc::new(Mutex::new(crate::engine_sevro::SevroEngineBackend::with_config(config))))
        }
        #[cfg(not(feature = "sevro"))]
        "sevro" | "native-js" => {
            Err(crate::error::BrowserError::EngineError(
                "Sevro engine not available — compile with --features sevro".to_string()
            ))
        }
        "auto" => {
            #[cfg(feature = "sevro")]
            {
                let mut config = sevro_headless::SevroConfig::default();
                config.proxy_url = opts.proxy_url;
                config.flaresolverr_url = opts.flaresolverr_url;
                config.fallback_proxy_url = opts.fallback_proxy_url;
                return Ok(Arc::new(Mutex::new(crate::engine_sevro::SevroEngineBackend::with_config(config))));
            }

            #[cfg(not(feature = "sevro"))]
            return Ok(Arc::new(Mutex::new(crate::engine_native::NativeEngine::new())));
        }
        other => Err(crate::error::BrowserError::EngineError(
            format!("Unknown engine: '{}'. Options: native, sevro, auto", other)
        )),
    }
}
