//! # openclaw-browser-core
//!
//! Core browser control layer for Wraith Browser.
//! Provides a unified `BrowserEngine` trait with multiple backends:
//! - `NativeEngine` — pure-Rust HTTP client, no external dependencies
//! - `ChromeEngine` — Chrome via CDP (feature-gated behind `chrome-legacy`)
//! - `SevroEngine` — Servo fork (future)

// Engine abstraction (always available)
pub mod engine;
pub mod engine_native;
pub mod dom;
pub mod actions;
pub mod error;
pub mod config;
pub mod native;

// Chrome backend (feature-gated)
#[cfg(feature = "chrome-legacy")]
pub mod session;
#[cfg(feature = "chrome-legacy")]
pub mod tab;
#[cfg(feature = "chrome-legacy")]
pub mod engine_chrome;

// BLUEPRINT feature modules
pub mod selectors;
pub mod network_intel;
pub mod stealth;
pub mod swarm;
pub mod tls_fingerprint;
pub mod wasm_plugins;
pub mod vision;
pub mod stealth_evasions;
pub mod tor;
pub mod telemetry;

// Re-exports
pub use engine::{BrowserEngine, EngineCapabilities, ScreenshotCapability};
pub use engine_native::NativeEngine;
#[cfg(feature = "chrome-legacy")]
pub use session::BrowserSession;
#[cfg(feature = "chrome-legacy")]
pub use tab::TabHandle;
pub use dom::DomSnapshot;
pub use actions::{BrowserAction, ActionResult};
pub use error::BrowserError;
pub use config::BrowserConfig;
pub use native::NativeClient;
pub use selectors::AdaptiveSelector;
pub use network_intel::NetworkCapture;
pub use stealth::HumanBehavior;
pub use swarm::BrowserSwarm;
