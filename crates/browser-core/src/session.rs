use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn, debug, instrument};
use futures::StreamExt;

use crate::config::BrowserConfig;
use crate::error::{BrowserError, BrowserResult};
use crate::tab::TabHandle;

/// Manages the lifecycle of a Chrome browser instance and its tabs.
/// This is the primary entry point for AI agents interacting with the browser.
pub struct BrowserSession {
    config: BrowserConfig,
    browser: chromiumoxide::Browser,
    tabs: Arc<RwLock<HashMap<String, TabHandle>>>,
    active_tab_id: Arc<RwLock<Option<String>>>,
    /// Handle to the background handler task — kept alive for the session's lifetime
    _handler_task: tokio::task::JoinHandle<()>,
}

impl BrowserSession {
    /// Launch a new browser session with the given config.
    #[instrument(skip(config), fields(headless = config.headless, max_tabs = config.max_tabs))]
    pub async fn launch(config: BrowserConfig) -> BrowserResult<Self> {
        info!(
            headless = config.headless,
            viewport = format!("{}x{}", config.viewport_width, config.viewport_height),
            "Launching Chrome browser session"
        );

        let mut builder = chromiumoxide::BrowserConfig::builder()
            .window_size(config.viewport_width, config.viewport_height);

        if config.headless {
            // Default is headless
        } else {
            builder = builder.with_head();
        }

        if let Some(ref path) = config.chrome_path {
            builder = builder.chrome_executable(path);
        }

        for arg in &config.extra_args {
            builder = builder.arg(arg.as_str());
        }

        let browser_config = builder
            .build()
            .map_err(|e| BrowserError::LaunchFailed(format!("Config build failed: {}", e)))?;

        let (browser, mut handler) = chromiumoxide::Browser::launch(browser_config)
            .await
            .map_err(|e| BrowserError::LaunchFailed(e.to_string()))?;

        // Spawn the CDP handler — MUST run for the browser to function
        let handler_task = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if let Err(e) = event {
                    tracing::warn!(error = %e, "CDP handler event error");
                }
            }
            tracing::debug!("CDP handler loop exited");
        });

        info!("Chrome browser session launched successfully");

        Ok(Self {
            config,
            browser,
            tabs: Arc::new(RwLock::new(HashMap::new())),
            active_tab_id: Arc::new(RwLock::new(None)),
            _handler_task: handler_task,
        })
    }

    /// Open a new tab and navigate to the given URL.
    #[instrument(skip(self), fields(url = %url))]
    pub async fn new_tab(&self, url: &str) -> BrowserResult<String> {
        let tab_count = self.tabs.read().await.len();
        if tab_count >= self.config.max_tabs {
            warn!(max_tabs = self.config.max_tabs, current = tab_count, "Tab limit reached");
            return Err(BrowserError::CdpError(format!(
                "Tab limit reached: {}/{}",
                tab_count, self.config.max_tabs
            )));
        }

        let tab_id = uuid::Uuid::new_v4().to_string();
        info!(tab_id = %tab_id, url = %url, "Opening new tab");

        // Create a real chromiumoxide Page and navigate
        let page = self.browser.new_page(url).await
            .map_err(|e| BrowserError::NavigationFailed {
                url: url.to_string(),
                reason: e.to_string(),
            })?;

        let current_url = page.url().await
            .unwrap_or(Some(url.to_string()))
            .unwrap_or_else(|| url.to_string());

        let title = page.get_title().await
            .unwrap_or(None);

        let tab = TabHandle {
            id: tab_id.clone(),
            current_url,
            title,
            page,
        };

        let mut tabs = self.tabs.write().await;
        tabs.insert(tab_id.clone(), tab);

        let mut active = self.active_tab_id.write().await;
        *active = Some(tab_id.clone());

        debug!(tab_id = %tab_id, total_tabs = tabs.len(), "Tab opened and set as active");
        Ok(tab_id)
    }

    /// Get the active tab handle.
    pub async fn active_tab(&self) -> BrowserResult<TabHandle> {
        let active_id = self.active_tab_id.read().await;
        let tab_id = active_id
            .as_ref()
            .ok_or_else(|| BrowserError::CdpError("No active tab".to_string()))?;

        let tabs = self.tabs.read().await;
        tabs.get(tab_id)
            .cloned()
            .ok_or_else(|| BrowserError::TabNotFound {
                tab_id: tab_id.clone(),
            })
    }

    /// Close a tab by ID.
    #[instrument(skip(self))]
    pub async fn close_tab(&self, tab_id: &str) -> BrowserResult<()> {
        let mut tabs = self.tabs.write().await;
        if let Some(tab) = tabs.remove(tab_id) {
            tab.page.close().await
                .map_err(|e| BrowserError::CdpError(format!("Close failed: {}", e)))?;
            info!(tab_id = %tab_id, remaining = tabs.len(), "Tab closed");
            Ok(())
        } else {
            Err(BrowserError::TabNotFound {
                tab_id: tab_id.to_string(),
            })
        }
    }

    /// List all open tab IDs with their current URLs.
    pub async fn list_tabs(&self) -> Vec<(String, String)> {
        let tabs = self.tabs.read().await;
        tabs.iter()
            .map(|(id, tab)| (id.clone(), tab.current_url.clone()))
            .collect()
    }

    /// Gracefully shut down the browser session.
    #[instrument(skip(self))]
    pub async fn shutdown(mut self) -> BrowserResult<()> {
        let tab_count = self.tabs.read().await.len();
        info!(tabs_open = tab_count, "Shutting down browser session");
        self.browser.close().await
            .map_err(|e| BrowserError::CdpError(format!("Browser close failed: {}", e)))?;
        info!("Browser session shut down cleanly");
        Ok(())
    }
}
