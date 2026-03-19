//! # openclaw-browser-core
//!
//! Core browser control layer for Wraith Browser.
//! Provides a unified `BrowserEngine` trait with multiple backends:
//! - `NativeEngine` — pure-Rust HTTP client, no external dependencies
//! - `SevroEngine` — Servo-derived engine with QuickJS, DOM, and layout (default)

// Engine abstraction (always available)
pub mod engine;
pub mod engine_native;
pub mod dom;
pub mod actions;
pub mod error;
pub mod config;
pub mod native;

// Sevro backend (default)
#[cfg(feature = "sevro")]
pub mod engine_sevro;

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
pub mod stealth_http;

// Re-exports
pub use engine::{BrowserEngine, EngineCapabilities, ScreenshotCapability};
pub use engine_native::NativeEngine;
pub use dom::DomSnapshot;
pub use actions::{BrowserAction, ActionResult};
pub use error::BrowserError;
pub use config::BrowserConfig;
pub use native::NativeClient;
pub use selectors::AdaptiveSelector;
pub use network_intel::NetworkCapture;
pub use stealth::HumanBehavior;
pub use swarm::BrowserSwarm;
