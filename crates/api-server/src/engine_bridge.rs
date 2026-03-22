//! # Engine Bridge
//!
//! Bridges the API server to the real `browser-core` [`BrowserEngine`] trait.
//! Each session gets its own engine instance, stored behind
//! `Arc<tokio::sync::Mutex<..>>` for safe concurrent access.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use openclaw_browser_core::actions::{ActionResult, BrowserAction};
use openclaw_browser_core::engine::{create_engine, BrowserEngine};

use crate::error::AppError;
use crate::routes::sessions::{EngineActionResult, EngineSnapshot};

// ---------------------------------------------------------------------------
// Type alias for a boxed, thread-safe engine behind an async mutex
// ---------------------------------------------------------------------------

type EngineHandle = Arc<Mutex<dyn BrowserEngine + Send + Sync>>;

// ---------------------------------------------------------------------------
// EngineBridge
// ---------------------------------------------------------------------------

/// Bridges API-layer session operations to per-session browser engines.
///
/// Each session owns an independent `BrowserEngine` instance created via
/// `create_engine("sevro")`.
pub struct EngineBridge {
    engines: Mutex<HashMap<Uuid, EngineHandle>>,
}

impl EngineBridge {
    /// Create a new bridge with no active sessions.
    pub fn new() -> Self {
        Self {
            engines: Mutex::new(HashMap::new()),
        }
    }

    // -- helpers ------------------------------------------------------------

    /// Get the engine handle for a session, or return `AppError::NotFound`.
    async fn get_engine(&self, session_id: Uuid) -> Result<EngineHandle, AppError> {
        let engines = self.engines.lock().await;
        engines.get(&session_id).cloned().ok_or_else(|| {
            AppError::NotFound(format!("Session {session_id} has no active engine"))
        })
    }

    /// Convert an `ActionResult` to `EngineActionResult`.
    fn map_action_result(result: &ActionResult) -> EngineActionResult {
        match result {
            ActionResult::Success { message } => EngineActionResult {
                success: true,
                message: Some(message.clone()),
            },
            ActionResult::Failed { error } => EngineActionResult {
                success: false,
                message: Some(error.clone()),
            },
            ActionResult::Navigated { url, title } => EngineActionResult {
                success: true,
                message: Some(format!("Navigated to {url} — {title}")),
            },
            ActionResult::Screenshot { .. } => EngineActionResult {
                success: true,
                message: Some("screenshot captured".into()),
            },
            ActionResult::Content { word_count, .. } => EngineActionResult {
                success: true,
                message: Some(format!("extracted {word_count} words")),
            },
            ActionResult::JsResult { value } => EngineActionResult {
                success: true,
                message: Some(value.clone()),
            },
        }
    }

    /// Build an `EngineSnapshot` from the current engine state.
    async fn build_snapshot(
        engine: &dyn BrowserEngine,
    ) -> Result<EngineSnapshot, AppError> {
        let snap = engine
            .snapshot()
            .await
            .map_err(|e| AppError::Internal(format!("snapshot: {e}")))?;
        let html = engine.page_source().await.ok();
        let current_url = engine.current_url().await;

        Ok(EngineSnapshot {
            html,
            text: Some(snap.to_agent_text()),
            screenshot_url: None,
            url: current_url.or_else(|| Some(snap.url.clone())),
            title: Some(snap.title.clone()),
        })
    }

    // -- Session lifecycle --------------------------------------------------

    /// Create a new Sevro engine for the given session.
    pub async fn create_session(&self, session_id: Uuid) -> Result<(), AppError> {
        let mut engines = self.engines.lock().await;
        if engines.contains_key(&session_id) {
            return Err(AppError::BadRequest(format!(
                "Session {session_id} already has an engine"
            )));
        }

        let engine = create_engine("sevro")
            .await
            .map_err(|e| AppError::Internal(format!("create engine: {e}")))?;

        engines.insert(session_id, engine);
        Ok(())
    }

    /// Destroy the engine for the given session, releasing all resources.
    pub async fn destroy_session(&self, session_id: Uuid) -> Result<(), AppError> {
        let mut engines = self.engines.lock().await;
        let engine = engines.remove(&session_id).ok_or_else(|| {
            AppError::NotFound(format!("Session {session_id} has no active engine"))
        })?;

        // Best-effort shutdown — ignore errors during cleanup.
        let mut guard = engine.lock().await;
        let _ = guard.shutdown().await;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Engine action methods
// ---------------------------------------------------------------------------

impl EngineBridge {
    /// Navigate the session's page to `url` and return a DOM snapshot.
    pub async fn navigate(
        &self,
        session_id: Uuid,
        url: &str,
    ) -> Result<EngineSnapshot, AppError> {
        let engine = self.get_engine(session_id).await?;
        let mut guard = engine.lock().await;

        guard
            .navigate(url)
            .await
            .map_err(|e| AppError::Internal(format!("navigate: {e}")))?;

        Self::build_snapshot(&*guard).await
    }

    /// Click the element identified by accessibility `ref_id`.
    pub async fn click(
        &self,
        session_id: Uuid,
        ref_id: &str,
    ) -> Result<EngineActionResult, AppError> {
        let ref_id_u32: u32 = ref_id
            .parse()
            .map_err(|_| AppError::BadRequest("ref_id must be a u32".into()))?;

        let engine = self.get_engine(session_id).await?;
        let mut guard = engine.lock().await;

        let result = guard
            .execute_action(BrowserAction::Click { ref_id: ref_id_u32, force: None })
            .await
            .map_err(|e| AppError::Internal(format!("click: {e}")))?;

        Ok(Self::map_action_result(&result))
    }

    /// Fill the element identified by `ref_id` with `text`.
    pub async fn fill(
        &self,
        session_id: Uuid,
        ref_id: &str,
        text: &str,
    ) -> Result<EngineActionResult, AppError> {
        let ref_id_u32: u32 = ref_id
            .parse()
            .map_err(|_| AppError::BadRequest("ref_id must be a u32".into()))?;

        let engine = self.get_engine(session_id).await?;
        let mut guard = engine.lock().await;

        let result = guard
            .execute_action(BrowserAction::Fill {
                ref_id: ref_id_u32,
                text: text.to_string(),
                force: None,
            })
            .await
            .map_err(|e| AppError::Internal(format!("fill: {e}")))?;

        Ok(Self::map_action_result(&result))
    }

    /// Return the current DOM snapshot.
    pub async fn snapshot(
        &self,
        session_id: Uuid,
    ) -> Result<EngineSnapshot, AppError> {
        let engine = self.get_engine(session_id).await?;
        let guard = engine.lock().await;

        Self::build_snapshot(&*guard).await
    }

    /// Extract the visible page content as Markdown.
    pub async fn extract_markdown(
        &self,
        session_id: Uuid,
    ) -> Result<String, AppError> {
        let engine = self.get_engine(session_id).await?;
        let mut guard = engine.lock().await;

        let result = guard
            .execute_action(BrowserAction::ExtractContent)
            .await
            .map_err(|e| AppError::Internal(format!("extract: {e}")))?;

        match result {
            ActionResult::Content { markdown, .. } => Ok(markdown),
            ActionResult::Failed { error } => {
                Err(AppError::Internal(format!("extract failed: {error}")))
            }
            _ => Ok(String::new()),
        }
    }

    /// Evaluate arbitrary JavaScript and return the stringified result.
    pub async fn eval_js(
        &self,
        session_id: Uuid,
        script: &str,
    ) -> Result<String, AppError> {
        let engine = self.get_engine(session_id).await?;
        let guard = engine.lock().await;

        guard
            .eval_js(script)
            .await
            .map_err(|e| AppError::Internal(format!("eval_js: {e}")))
    }

    /// Upload a file into the page's file-input element.
    ///
    /// `content_base64` is the base64-encoded file content. We pass it
    /// directly to the engine which handles decoding internally.
    pub async fn upload_file(
        &self,
        session_id: Uuid,
        filename: &str,
        content_base64: &str,
    ) -> Result<EngineActionResult, AppError> {
        let engine = self.get_engine(session_id).await?;
        let mut guard = engine.lock().await;

        // Find the first <input type="file"> from the current snapshot to use
        // as the target ref_id. If none exists, use ref_id 0 as fallback.
        let ref_id = {
            if let Ok(snap) = guard.snapshot().await {
                snap.elements
                    .iter()
                    .find(|el| el.role == "file")
                    .map(|el| el.ref_id)
                    .unwrap_or(0)
            } else {
                0
            }
        };

        let result = guard
            .execute_action(BrowserAction::UploadFile {
                ref_id,
                file_name: filename.to_string(),
                file_data: content_base64.to_string(),
                mime_type: "application/octet-stream".to_string(),
            })
            .await
            .map_err(|e| AppError::Internal(format!("upload_file: {e}")))?;

        Ok(Self::map_action_result(&result))
    }

    /// Submit the form identified by `ref_id`.
    pub async fn submit_form(
        &self,
        session_id: Uuid,
        ref_id: &str,
    ) -> Result<EngineActionResult, AppError> {
        let ref_id_u32: u32 = ref_id
            .parse()
            .map_err(|_| AppError::BadRequest("ref_id must be a u32".into()))?;

        let engine = self.get_engine(session_id).await?;
        let mut guard = engine.lock().await;

        let result = guard
            .execute_action(BrowserAction::SubmitForm { ref_id: ref_id_u32 })
            .await
            .map_err(|e| AppError::Internal(format!("submit_form: {e}")))?;

        Ok(Self::map_action_result(&result))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_and_destroy_session() {
        let bridge = EngineBridge::new();
        let sid = Uuid::new_v4();

        bridge.create_session(sid).await.unwrap();
        // Duplicate create should fail.
        assert!(bridge.create_session(sid).await.is_err());

        bridge.destroy_session(sid).await.unwrap();
        // Double destroy should fail.
        assert!(bridge.destroy_session(sid).await.is_err());
    }

    #[tokio::test]
    async fn navigate_returns_snapshot() {
        let bridge = EngineBridge::new();
        let sid = Uuid::new_v4();

        bridge.create_session(sid).await.unwrap();
        let snap = bridge.navigate(sid, "https://example.com").await.unwrap();
        assert!(snap.url.is_some());
        assert!(snap.title.is_some());

        bridge.destroy_session(sid).await.unwrap();
    }

    #[tokio::test]
    async fn action_on_missing_session_returns_not_found() {
        let bridge = EngineBridge::new();
        let sid = Uuid::new_v4();

        let err = bridge.click(sid, "42").await.unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }
}
