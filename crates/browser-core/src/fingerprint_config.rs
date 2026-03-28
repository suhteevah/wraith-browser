//! # Fingerprint Configuration
//!
//! JSON-based fingerprint spoofing system inspired by Camoufox's MaskConfig.
//!
//! Instead of injecting JavaScript to override browser properties (detectable),
//! this module provides a configuration layer that feeds spoofed values directly
//! into the QuickJS DOM bridge at the Rust level — making the changes invisible
//! to JavaScript inspection.
//!
//! ## Design
//!
//! Camoufox patches Firefox's C++ implementation to intercept property getters
//! (e.g., `window.innerWidth`, `navigator.userAgent`) before JS ever sees them.
//! We achieve the same effect by intercepting at the QuickJS DOM bridge level —
//! our Rust code that implements `document`, `window`, `navigator`, `screen` etc.
//!
//! ## Usage
//!
//! ```rust
//! use wraith_browser_core::fingerprint_config::FingerprintConfig;
//!
//! let config = FingerprintConfig::generate();
//! // Or load custom overrides:
//! let config = FingerprintConfig::from_json(r#"{"window.innerWidth": 1920}"#).unwrap();
//!
//! assert_eq!(config.get_f64("window.innerWidth"), Some(1920.0));
//! ```

use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info};

/// A fingerprint configuration that maps browser property paths to spoofed values.
///
/// Property paths use dot notation matching the JavaScript API surface:
/// - `window.innerWidth`, `window.innerHeight`, `window.outerWidth`, etc.
/// - `navigator.userAgent`, `navigator.platform`, `navigator.hardwareConcurrency`
/// - `screen.width`, `screen.height`, `screen.colorDepth`
/// - `canvas:seed` — deterministic seed for canvas noise (Camoufox algorithm)
/// - `AudioContext:sampleRate`, `AudioContext:maxChannelCount`
/// - `webgl.vendor`, `webgl.renderer`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FingerprintConfig {
    /// Property path → JSON value mappings.
    properties: HashMap<String, serde_json::Value>,
}

impl FingerprintConfig {
    /// Create an empty config (no spoofing — all properties return real values).
    pub fn new() -> Self {
        Self {
            properties: HashMap::new(),
        }
    }

    /// Parse a fingerprint config from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let properties: HashMap<String, serde_json::Value> = serde_json::from_str(json)?;
        Ok(Self { properties })
    }

    /// Generate a realistic fingerprint config with randomized but consistent values.
    ///
    /// Uses the same statistical distributions as Camoufox's BrowserForge:
    /// common screen resolutions, typical hardware specs, and plausible
    /// navigator values that pass consistency checks.
    pub fn generate() -> Self {
        let mut rng = rand::thread_rng();
        let mut props = HashMap::new();

        // Pick a common screen resolution (weighted by real-world usage)
        let resolutions: &[(i32, i32, f64)] = &[
            (1920, 1080, 0.40), // Most common
            (1366, 768, 0.15),
            (2560, 1440, 0.12),
            (1536, 864, 0.08),
            (1440, 900, 0.07),
            (1280, 720, 0.06),
            (3840, 2160, 0.05),
            (1600, 900, 0.04),
            (2560, 1080, 0.03),
        ];

        let roll: f64 = rng.gen();
        let mut cumulative = 0.0;
        let (screen_w, screen_h) = resolutions
            .iter()
            .find(|(_, _, weight)| {
                cumulative += weight;
                roll < cumulative
            })
            .map(|(w, h, _)| (*w, *h))
            .unwrap_or((1920, 1080));

        // Window is slightly smaller than screen (taskbar, browser chrome)
        let chrome_height = if screen_h > 1000 { 85 } else { 75 };
        let inner_w = screen_w;
        let inner_h = screen_h - chrome_height;
        let outer_w = screen_w;
        let outer_h = screen_h;

        // Screen properties
        props.insert("screen.width".into(), screen_w.into());
        props.insert("screen.height".into(), screen_h.into());
        props.insert("screen.availWidth".into(), screen_w.into());
        props.insert("screen.availHeight".into(), (screen_h - 40).into()); // Taskbar
        props.insert("screen.colorDepth".into(), 24.into());
        props.insert("screen.pixelDepth".into(), 24.into());

        // Window dimensions
        props.insert("window.innerWidth".into(), inner_w.into());
        props.insert("window.innerHeight".into(), inner_h.into());
        props.insert("window.outerWidth".into(), outer_w.into());
        props.insert("window.outerHeight".into(), outer_h.into());
        props.insert("window.screenX".into(), 0.into());
        props.insert("window.screenY".into(), 0.into());

        // DPR — most common is 1.0 for desktop, occasionally 1.25 or 1.5
        let dpr_options = &[1.0_f64, 1.0, 1.0, 1.25, 1.5, 2.0];
        let dpr = dpr_options[rng.gen_range(0..dpr_options.len())];
        props.insert(
            "window.devicePixelRatio".into(),
            serde_json::Value::from(dpr),
        );

        // Navigator properties (Firefox 136 on Windows)
        props.insert(
            "navigator.userAgent".into(),
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:136.0) Gecko/20100101 Firefox/136.0"
                .into(),
        );
        props.insert("navigator.platform".into(), "Win32".into());
        props.insert("navigator.oscpu".into(), "Windows NT 10.0; Win64; x64".into());
        props.insert("navigator.language".into(), "en-US".into());
        props.insert(
            "navigator.languages".into(),
            serde_json::json!(["en-US", "en"]),
        );
        props.insert("navigator.cookieEnabled".into(), true.into());
        props.insert("navigator.doNotTrack".into(), serde_json::Value::Null);
        props.insert("navigator.maxTouchPoints".into(), 0.into());
        props.insert("navigator.pdfViewerEnabled".into(), true.into());

        // Hardware — vary realistically
        let cores = [4, 8, 8, 12, 16][rng.gen_range(0..5)];
        let memory = [4, 8, 8, 16, 32][rng.gen_range(0..5)];
        props.insert("navigator.hardwareConcurrency".into(), cores.into());
        props.insert("navigator.deviceMemory".into(), memory.into());

        // WebGL — common GPU vendor/renderer pairs
        let gpu_pairs = &[
            ("Google Inc. (NVIDIA)", "ANGLE (NVIDIA, NVIDIA GeForce GTX 1060 Direct3D11 vs_5_0 ps_5_0, D3D11)"),
            ("Google Inc. (NVIDIA)", "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11 vs_5_0 ps_5_0, D3D11)"),
            ("Google Inc. (Intel)", "ANGLE (Intel, Intel(R) UHD Graphics 630 Direct3D11 vs_5_0 ps_5_0, D3D11)"),
            ("Google Inc. (AMD)", "ANGLE (AMD, AMD Radeon RX 580 Direct3D11 vs_5_0 ps_5_0, D3D11)"),
        ];
        let (vendor, renderer) = gpu_pairs[rng.gen_range(0..gpu_pairs.len())];
        props.insert("webgl.vendor".into(), vendor.into());
        props.insert("webgl.renderer".into(), renderer.into());

        // Canvas noise seed (deterministic per-session, Camoufox algorithm)
        let canvas_seed: u32 = rng.gen_range(1..u32::MAX);
        props.insert("canvas:seed".into(), canvas_seed.into());

        // Audio context
        props.insert("AudioContext:sampleRate".into(), 44100.into());
        props.insert("AudioContext:maxChannelCount".into(), 2.into());
        props.insert(
            "AudioContext:outputLatency".into(),
            serde_json::Value::from(0.01),
        );

        // Timezone offset (minutes from UTC — 0 for UTC, 300 for EST, etc.)
        // Default to common US timezones
        let tz_offsets = &[300, 360, 420, 480, 0, 60]; // EST, CST, MST, PST, UTC, CET
        let tz = tz_offsets[rng.gen_range(0..tz_offsets.len())];
        props.insert("timezone.offset".into(), tz.into());

        info!(
            screen = %format!("{}x{}", screen_w, screen_h),
            cores,
            memory,
            dpr,
            canvas_seed,
            "Generated fingerprint config"
        );

        Self { properties: props }
    }

    /// Get a string value for a property path.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.properties.get(key)?.as_str()
    }

    /// Get an f64 value for a property path.
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.properties.get(key)?.as_f64()
    }

    /// Get an i64 value for a property path.
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.properties.get(key)?.as_i64()
    }

    /// Get a u32 value for a property path.
    pub fn get_u32(&self, key: &str) -> Option<u32> {
        self.properties.get(key)?.as_u64().map(|v| v as u32)
    }

    /// Get a bool value for a property path.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.properties.get(key)?.as_bool()
    }

    /// Get the raw JSON value for a property path.
    pub fn get_value(&self, key: &str) -> Option<&serde_json::Value> {
        self.properties.get(key)
    }

    /// Check if a property path has a value set.
    pub fn has(&self, key: &str) -> bool {
        self.properties.contains_key(key)
    }

    /// Set a property value.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) {
        self.properties.insert(key.into(), value.into());
    }

    /// Merge another config's properties into this one (other takes precedence).
    pub fn merge(&mut self, other: &FingerprintConfig) {
        for (k, v) in &other.properties {
            self.properties.insert(k.clone(), v.clone());
        }
    }

    /// Serialize to JSON string.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.properties).unwrap_or_default()
    }

    /// Convert to a HashMap for passing to the QuickJS DOM bridge.
    /// This is the interface between FingerprintConfig (browser-core) and
    /// the DOM bridge (sevro-headless) without creating a circular dependency.
    pub fn to_map(&self) -> HashMap<String, serde_json::Value> {
        self.properties.clone()
    }

    /// Get the canvas noise seed (0 = no noise).
    pub fn canvas_seed(&self) -> u32 {
        self.get_u32("canvas:seed").unwrap_or(0)
    }
}

impl Default for FingerprintConfig {
    fn default() -> Self {
        Self::generate()
    }
}

// ---------------------------------------------------------------------------
// Canvas noise (ported from Camoufox's CanvasFingerprintManager)
// ---------------------------------------------------------------------------

/// Apply deterministic canvas noise using Camoufox's algorithm.
///
/// This is a direct port of `CanvasFingerprintManager::ApplyCanvasNoise`:
/// - Uses a linear congruential generator (LCG) seeded per-session
/// - Modifies at most one RGB channel per pixel by ±1
/// - Skips zero channels (preserves transparency, avoids CreepJS trap)
/// - Deterministic: same seed + same input = same output (passes consistency checks)
pub fn apply_canvas_noise(data: &mut [u8], seed: u32) {
    if seed == 0 || data.is_empty() {
        return;
    }

    let mut state: u32 = seed;

    // Process RGBA pixels (4 bytes each)
    let mut i = 0;
    while i + 3 < data.len() {
        // LCG step (same constants as Camoufox)
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);

        if state & 0x100 != 0 {
            // Iterate RGB channels (skip alpha at i+3), modify first non-zero
            for c in 0..3 {
                let val = data[i + c] as i32;
                if val == 0 {
                    continue;
                }
                data[i + c] = if state & 0x200 != 0 {
                    val.saturating_add(1).min(255) as u8
                } else {
                    val.saturating_sub(1).max(0) as u8
                };
                break; // Only modify one channel per pixel
            }
        }

        i += 4;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_produces_valid_config() {
        let config = FingerprintConfig::generate();
        assert!(config.get_f64("window.innerWidth").is_some());
        assert!(config.get_f64("window.innerHeight").is_some());
        assert!(config.get_str("navigator.userAgent").is_some());
        assert!(config.get_str("navigator.platform").is_some());
        assert!(config.get_i64("navigator.hardwareConcurrency").is_some());
        assert!(config.get_str("webgl.vendor").is_some());
        assert!(config.canvas_seed() > 0);
    }

    #[test]
    fn test_from_json_overrides() {
        let config = FingerprintConfig::from_json(
            r#"{"window.innerWidth": 1024, "navigator.platform": "Linux x86_64"}"#,
        )
        .unwrap();
        assert_eq!(config.get_f64("window.innerWidth"), Some(1024.0));
        assert_eq!(config.get_str("navigator.platform"), Some("Linux x86_64"));
    }

    #[test]
    fn test_canvas_noise_deterministic() {
        let mut data1 = vec![100u8, 150, 200, 255, 50, 75, 100, 255];
        let mut data2 = data1.clone();

        apply_canvas_noise(&mut data1, 42);
        apply_canvas_noise(&mut data2, 42);

        // Same seed → same output
        assert_eq!(data1, data2);
    }

    #[test]
    fn test_canvas_noise_different_seeds() {
        let original = vec![100u8, 150, 200, 255, 50, 75, 100, 255];
        let mut data1 = original.clone();
        let mut data2 = original.clone();

        apply_canvas_noise(&mut data1, 42);
        apply_canvas_noise(&mut data2, 999);

        // Different seeds → different output (probabilistic, but very likely)
        assert_ne!(data1, data2);
    }

    #[test]
    fn test_canvas_noise_preserves_transparency() {
        let mut data = vec![0u8, 0, 0, 255]; // Fully transparent RGB, opaque alpha
        apply_canvas_noise(&mut data, 42);

        // Zero RGB channels should stay zero (Camoufox behavior)
        assert_eq!(data[0], 0);
        assert_eq!(data[1], 0);
        assert_eq!(data[2], 0);
        // Alpha untouched
        assert_eq!(data[3], 255);
    }

    #[test]
    fn test_canvas_noise_zero_seed_noop() {
        let original = vec![100u8, 150, 200, 255];
        let mut data = original.clone();
        apply_canvas_noise(&mut data, 0);
        assert_eq!(data, original);
    }

    #[test]
    fn test_merge_config() {
        let mut base = FingerprintConfig::generate();
        let override_config =
            FingerprintConfig::from_json(r#"{"window.innerWidth": 1024}"#).unwrap();
        base.merge(&override_config);
        assert_eq!(base.get_f64("window.innerWidth"), Some(1024.0));
    }

    #[test]
    fn test_screen_resolution_is_common() {
        let config = FingerprintConfig::generate();
        let w = config.get_i64("screen.width").unwrap();
        let h = config.get_i64("screen.height").unwrap();
        // Should be one of our defined resolutions
        let valid = matches!(
            (w, h),
            (1920, 1080)
                | (1366, 768)
                | (2560, 1440)
                | (1536, 864)
                | (1440, 900)
                | (1280, 720)
                | (3840, 2160)
                | (1600, 900)
                | (2560, 1080)
        );
        assert!(valid, "unexpected resolution: {}x{}", w, h);
    }
}
