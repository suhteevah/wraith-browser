//! BrowserEngine implementation wrapping NativeClient (pure HTTP, no JS).
//!
//! This is the lightest engine — ~50ms per page, zero external dependencies.
//! Cannot execute JavaScript or take screenshots, but handles static sites,
//! documentation, forms, and API responses.

use crate::dom::DomSnapshot;
use crate::actions::{BrowserAction, ActionResult};
use crate::engine::{BrowserEngine, EngineCapabilities, ScreenshotCapability};
use crate::error::{BrowserResult, BrowserError};
use crate::native::NativeClient;
use async_trait::async_trait;
use tracing::instrument;

/// Browser engine backed by pure-Rust HTTP client. No Chrome, no JS.
pub struct NativeEngine {
    client: NativeClient,
}

impl NativeEngine {
    pub fn new() -> Self {
        Self { client: NativeClient::new() }
    }
}

impl Default for NativeEngine {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl BrowserEngine for NativeEngine {
    #[instrument(skip(self), fields(url = %url))]
    async fn navigate(&mut self, url: &str) -> BrowserResult<()> {
        // NativeClient::navigate returns DomSnapshot; discard it.
        // The snapshot is cached internally and retrievable via snapshot().
        self.client.navigate(url).await?;
        Ok(())
    }

    async fn snapshot(&self) -> BrowserResult<DomSnapshot> {
        self.client.snapshot()
    }

    async fn execute_action(&mut self, action: BrowserAction) -> BrowserResult<ActionResult> {
        self.client.execute(action).await
    }

    async fn eval_js(&self, _script: &str) -> BrowserResult<String> {
        Err(BrowserError::JsEvalFailed(
            "JavaScript not available in native engine".to_string()
        ))
    }

    async fn page_source(&self) -> BrowserResult<String> {
        self.client.page_source().map(|s| s.to_string())
    }

    async fn current_url(&self) -> Option<String> {
        self.client.current_url().map(|s| s.to_string())
    }

    async fn screenshot(&self) -> BrowserResult<Vec<u8>> {
        Err(BrowserError::ScreenshotFailed(
            "Screenshots not available in native engine".to_string()
        ))
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            javascript: false,
            screenshots: ScreenshotCapability::None,
            layout: false,
            cookies: true,
            stealth: false,
        }
    }

    async fn shutdown(&mut self) -> BrowserResult<()> {
        Ok(())
    }
}
