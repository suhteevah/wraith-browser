//! # openclaw-browser-core
//!
//! Core browser control layer for OpenClaw Browser.
//! Wraps chromiumoxide to provide a high-level async API for AI agent control
//! of headless Chrome via the Chrome DevTools Protocol (CDP).
//!
//! ## Architecture
//!
//! ```text
//! Agent Loop ──► BrowserSession ──► chromiumoxide ──► Chrome/Chromium (CDP)
//!                    │
//!                    ├── TabHandle (per-tab state)
//!                    ├── DomSnapshot (accessibility tree + interactive elements)
//!                    ├── PageContent (extracted markdown/text)
//!                    └── ActionResult (click, fill, navigate, screenshot)
//! ```

pub mod session;
pub mod tab;
pub mod dom;
pub mod actions;
pub mod error;
pub mod config;

pub use session::BrowserSession;
pub use tab::TabHandle;
pub use dom::DomSnapshot;
pub use actions::{BrowserAction, ActionResult};
pub use error::BrowserError;
pub use config::BrowserConfig;
