//! BrowserEngine implementation wrapping Chrome via CDP (chromiumoxide).
//!
//! Feature-gated behind `chrome-legacy`. This is the existing Chrome path
//! wrapped in the unified BrowserEngine trait. Will be deprecated once
//! SevroEngine is stable.

use crate::config::BrowserConfig;
use crate::dom::DomSnapshot;
use crate::actions::{BrowserAction, ActionResult};
use crate::engine::{BrowserEngine, EngineCapabilities, ScreenshotCapability};
use crate::error::BrowserResult;
use crate::session::BrowserSession;
use async_trait::async_trait;
use tracing::{info, instrument};

/// Browser engine backed by Chrome via the Chrome DevTools Protocol.
pub struct ChromeEngine {
    session: Option<BrowserSession>,
}

impl ChromeEngine {
    /// Launch Chrome and create the engine.
    #[instrument(skip(config), fields(headless = config.headless))]
    pub async fn launch(config: BrowserConfig) -> BrowserResult<Self> {
        let session = BrowserSession::launch(config).await?;
        Ok(Self { session: Some(session) })
    }

    fn session(&self) -> BrowserResult<&BrowserSession> {
        self.session.as_ref().ok_or_else(|| {
            crate::error::BrowserError::CdpError("Chrome engine already shut down".to_string())
        })
    }
}

#[async_trait]
impl BrowserEngine for ChromeEngine {
    #[instrument(skip(self), fields(url = %url))]
    async fn navigate(&mut self, url: &str) -> BrowserResult<()> {
        let session = self.session()?;
        if session.list_tabs().await.is_empty() {
            session.new_tab(url).await?;
        } else {
            let mut tab = session.active_tab().await?;
            tab.navigate(url).await?;
        }
        Ok(())
    }

    async fn snapshot(&self) -> BrowserResult<DomSnapshot> {
        self.session()?.active_tab().await?.snapshot().await
    }

    async fn execute_action(&mut self, action: BrowserAction) -> BrowserResult<ActionResult> {
        let mut tab = self.session()?.active_tab().await?;
        tab.execute(action).await
    }

    async fn eval_js(&self, script: &str) -> BrowserResult<String> {
        self.session()?.active_tab().await?.eval_js(script).await
    }

    async fn page_source(&self) -> BrowserResult<String> {
        self.session()?.active_tab().await?.page_source().await
    }

    async fn current_url(&self) -> Option<String> {
        self.session().ok()?
            .active_tab().await.ok()
            .map(|tab| tab.current_url.clone())
    }

    async fn screenshot(&self) -> BrowserResult<Vec<u8>> {
        let mut tab = self.session()?.active_tab().await?;
        match tab.execute(BrowserAction::Screenshot { full_page: false }).await? {
            ActionResult::Screenshot { png_base64, .. } => {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.decode(&png_base64)
                    .map_err(|e| crate::error::BrowserError::ScreenshotFailed(e.to_string()))
            }
            _ => Err(crate::error::BrowserError::ScreenshotFailed(
                "Unexpected action result".to_string()
            )),
        }
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            javascript: true,
            screenshots: ScreenshotCapability::FullPage,
            layout: true,
            cookies: true,
            stealth: true,
        }
    }

    async fn shutdown(&mut self) -> BrowserResult<()> {
        if let Some(session) = self.session.take() {
            info!("Shutting down Chrome engine");
            session.shutdown().await?;
        }
        Ok(())
    }
}
