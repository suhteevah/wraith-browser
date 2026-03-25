//! # Browser Fingerprint Manager
//!
//! Browser identity profiles for consistent presentation across sessions.
//! Ensures consistent browser characteristics (user agent, screen dimensions,
//! language, etc.) for reliable automation.
//!
//! ## Capture Flow
//!
//! 1. Launch user's real browser (non-headless) via CDP
//! 2. Navigate to `about:blank`
//! 3. Inject fingerprint extraction script
//! 4. Store fingerprint as JSON
//! 5. On every headless page load, inject fingerprint overrides via CDP

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn, debug, instrument};

use crate::error::{IdentityError, IdentityResult};

/// A complete browser fingerprint captured from the user's real browser.
///
/// Only `id` and `user_agent` are required when deserializing from JSON.
/// All other fields have sensible defaults so that partial fingerprints
/// can be imported without enumerating every field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserFingerprint {
    /// Fingerprint ID
    pub id: String,

    /// Friendly name (e.g., "Matt's Chrome on Windows")
    #[serde(default = "default_name")]
    pub name: String,

    // ─── HTTP Headers ───────────────────────────────────────────
    /// Full User-Agent string from the real browser
    pub user_agent: String,

    /// Accept-Language header value
    #[serde(default = "default_accept_language")]
    pub accept_language: String,

    /// Sec-CH-UA (Client Hints)
    #[serde(default)]
    pub sec_ch_ua: Option<String>,

    /// Sec-CH-UA-Platform
    #[serde(default)]
    pub sec_ch_ua_platform: Option<String>,

    /// Sec-CH-UA-Mobile
    #[serde(default)]
    pub sec_ch_ua_mobile: Option<String>,

    /// Sec-CH-UA-Full-Version-List
    #[serde(default)]
    pub sec_ch_ua_full_version_list: Option<String>,

    // ─── Navigator Properties ───────────────────────────────────
    /// navigator.platform (e.g., "Win32")
    #[serde(default = "default_platform")]
    pub platform: String,

    /// navigator.hardwareConcurrency
    #[serde(default = "default_hardware_concurrency")]
    pub hardware_concurrency: u32,

    /// navigator.deviceMemory (GB, may be None on Firefox)
    #[serde(default)]
    pub device_memory: Option<f64>,

    /// navigator.maxTouchPoints
    #[serde(default)]
    pub max_touch_points: u32,

    /// navigator.language
    #[serde(default = "default_language")]
    pub language: String,

    /// navigator.languages (ordered list)
    #[serde(default = "default_languages")]
    pub languages: Vec<String>,

    /// navigator.vendor
    #[serde(default = "default_vendor")]
    pub vendor: String,

    /// navigator.doNotTrack
    #[serde(default)]
    pub do_not_track: Option<String>,

    // ─── Screen Properties ──────────────────────────────────────
    /// screen.width
    #[serde(default = "default_screen_width")]
    pub screen_width: u32,

    /// screen.height
    #[serde(default = "default_screen_height")]
    pub screen_height: u32,

    /// screen.availWidth
    #[serde(default = "default_screen_width")]
    pub avail_width: u32,

    /// screen.availHeight
    #[serde(default = "default_screen_height")]
    pub avail_height: u32,

    /// screen.colorDepth
    #[serde(default = "default_color_depth")]
    pub color_depth: u32,

    /// screen.pixelDepth
    #[serde(default = "default_color_depth")]
    pub pixel_depth: u32,

    /// window.devicePixelRatio
    #[serde(default = "default_device_pixel_ratio")]
    pub device_pixel_ratio: f64,

    // ─── Timezone ───────────────────────────────────────────────
    /// Intl.DateTimeFormat().resolvedOptions().timeZone
    #[serde(default = "default_timezone")]
    pub timezone: String,

    /// new Date().getTimezoneOffset() (minutes)
    #[serde(default)]
    pub timezone_offset: i32,

    // ─── Graphics ───────────────────────────────────────────────
    /// WebGL renderer string
    #[serde(default)]
    pub webgl_renderer: Option<String>,

    /// WebGL vendor string
    #[serde(default)]
    pub webgl_vendor: Option<String>,

    /// WebGL unmasked renderer (WEBGL_debug_renderer_info)
    #[serde(default)]
    pub webgl_unmasked_renderer: Option<String>,

    /// WebGL unmasked vendor
    #[serde(default)]
    pub webgl_unmasked_vendor: Option<String>,

    /// Canvas fingerprint hash (draw operations → toDataURL → hash)
    #[serde(default)]
    pub canvas_hash: Option<String>,

    /// WebGL fingerprint hash
    #[serde(default)]
    pub webgl_hash: Option<String>,

    /// AudioContext fingerprint hash
    #[serde(default)]
    pub audio_hash: Option<String>,

    // ─── Fonts ──────────────────────────────────────────────────
    /// Detected installed fonts (via CSS fallback measurement)
    #[serde(default)]
    pub fonts: Vec<String>,

    // ─── Feature Detection ──────────────────────────────────────
    /// Plugins (navigator.plugins — usually empty in modern Chrome)
    #[serde(default)]
    pub plugins: Vec<String>,

    /// Supported MIME types
    #[serde(default)]
    pub mime_types: Vec<String>,

    /// Whether WebDriver is detected (navigator.webdriver)
    /// This MUST be false in our spoofed profile
    #[serde(default)]
    pub webdriver: bool,

    /// Whether automation-related properties are present
    #[serde(default)]
    pub automation_detected: bool,

    // ─── Connection Info ────────────────────────────────────────
    /// navigator.connection.effectiveType (e.g., "4g")
    #[serde(default)]
    pub connection_type: Option<String>,

    /// navigator.connection.downlink (Mbps)
    #[serde(default)]
    pub connection_downlink: Option<f64>,

    /// navigator.connection.rtt (ms)
    #[serde(default)]
    pub connection_rtt: Option<u32>,

    // ─── Metadata ───────────────────────────────────────────────
    /// When this fingerprint was captured
    #[serde(default = "Utc::now")]
    pub captured_at: DateTime<Utc>,

    /// Source browser (e.g., "Chrome 131 on Windows 11")
    #[serde(default = "default_source_browser")]
    pub source_browser: String,

    /// Raw JSON of the full capture (for future fields)
    #[serde(default)]
    pub raw_json: serde_json::Value,
}

// ─── Default value functions for serde ──────────────────────────────
fn default_name() -> String { "Imported fingerprint".into() }
fn default_accept_language() -> String { "en-US,en;q=0.9".into() }
fn default_platform() -> String { "Win32".into() }
fn default_hardware_concurrency() -> u32 { 8 }
fn default_language() -> String { "en-US".into() }
fn default_languages() -> Vec<String> { vec!["en-US".into(), "en".into()] }
fn default_vendor() -> String { "Google Inc.".into() }
fn default_screen_width() -> u32 { 1920 }
fn default_screen_height() -> u32 { 1080 }
fn default_color_depth() -> u32 { 24 }
fn default_device_pixel_ratio() -> f64 { 1.0 }
fn default_timezone() -> String { "America/New_York".into() }
fn default_source_browser() -> String { "Unknown (imported)".into() }

/// Manages fingerprint capture, storage, and injection.
#[derive(Default)]
pub struct FingerprintManager {
    /// Stored fingerprints (persisted in vault)
    profiles: Vec<BrowserFingerprint>,

    /// Active fingerprint to use for spoofing
    active_id: Option<String>,
}

impl FingerprintManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Capture a fingerprint from a JSON file exported from a real browser.
    ///
    /// Chrome-based live capture was removed in Phase 3 (Chrome removal).
    /// To capture a fingerprint, use the browser's DevTools console to run
    /// the CAPTURE_SCRIPT, save the output as JSON, then load it here.
    ///
    /// Alternatively, use `load_from_file()` to load a previously saved profile.
    #[instrument(skip(self))]
    pub async fn capture_from_real_browser(&mut self) -> IdentityResult<BrowserFingerprint> {
        Err(IdentityError::FingerprintFailed(
            "Live Chrome capture was removed (Chrome dependency eliminated). \
             Use `load_from_file()` to load a fingerprint JSON exported from your browser's DevTools, \
             or use one of the built-in profiles via `generate_default_profile()`."
            .to_string()
        ))
    }

    /// Load a fingerprint from a JSON file (captured via browser DevTools).
    #[instrument(skip(self))]
    pub fn load_from_file(&mut self, path: &std::path::Path) -> IdentityResult<BrowserFingerprint> {
        let data = std::fs::read_to_string(path)
            .map_err(|e| IdentityError::FingerprintFailed(format!("Failed to read {}: {e}", path.display())))?;

        let fp: BrowserFingerprint = serde_json::from_str(&data)
            .map_err(|e| IdentityError::FingerprintFailed(format!("Invalid fingerprint JSON: {e}")))?;

        info!(
            id = %fp.id,
            ua = %fp.user_agent.get(..60).unwrap_or(&fp.user_agent),
            "Fingerprint loaded from file"
        );

        self.add_profile(fp.clone());
        self.set_active(&fp.id);

        Ok(fp)
    }

    /// Generate the CDP commands to inject fingerprint overrides into a headless page.
    /// This MUST be called before any page navigation.
    #[instrument(skip(self))]
    pub fn get_cdp_overrides(&self) -> IdentityResult<CdpOverrides> {
        let fp = self.get_active()?;

        debug!(
            profile = %fp.name,
            ua = %fp.user_agent,
            screen = format!("{}x{}", fp.screen_width, fp.screen_height),
            "Generating CDP overrides from fingerprint"
        );

        Ok(CdpOverrides {
            user_agent: fp.user_agent.clone(),
            accept_language: fp.accept_language.clone(),
            platform: fp.platform.clone(),
            viewport_width: fp.screen_width,
            viewport_height: fp.screen_height,
            device_pixel_ratio: fp.device_pixel_ratio,
            timezone_id: fp.timezone.clone(),

            // JavaScript to inject via CDP Page.addScriptToEvaluateOnNewDocument
            // This runs before any page JS — overrides navigator properties
            injection_script: Self::build_injection_script(fp),

            // Extra HTTP headers to send
            extra_headers: Self::build_extra_headers(fp),
        })
    }

    /// Build the JavaScript that overrides navigator/screen/webgl properties.
    fn build_injection_script(fp: &BrowserFingerprint) -> String {
        format!(r#"
// ═══ Wraith Fingerprint Injection ═══
// Runs before any page JavaScript via CDP Page.addScriptToEvaluateOnNewDocument

(() => {{
    // ─── Kill WebDriver detection ───────────────────────────
    Object.defineProperty(navigator, 'webdriver', {{
        get: () => false,
        configurable: true,
    }});

    // Remove automation indicators
    delete window.cdc_adoQpoasnfa76pfcZLmcfl_Array;
    delete window.cdc_adoQpoasnfa76pfcZLmcfl_Promise;
    delete window.cdc_adoQpoasnfa76pfcZLmcfl_Symbol;

    // ─── Navigator overrides ────────────────────────────────
    const navOverrides = {{
        platform: {platform},
        hardwareConcurrency: {hw_concurrency},
        deviceMemory: {device_memory},
        maxTouchPoints: {max_touch_points},
        language: {language},
        languages: {languages},
        vendor: {vendor},
        doNotTrack: {dnt},
    }};

    for (const [key, value] of Object.entries(navOverrides)) {{
        if (value !== null && value !== undefined) {{
            Object.defineProperty(navigator, key, {{
                get: () => value,
                configurable: true,
            }});
        }}
    }}

    // ─── Screen overrides ───────────────────────────────────
    const screenOverrides = {{
        width: {screen_width},
        height: {screen_height},
        availWidth: {avail_width},
        availHeight: {avail_height},
        colorDepth: {color_depth},
        pixelDepth: {pixel_depth},
    }};

    for (const [key, value] of Object.entries(screenOverrides)) {{
        Object.defineProperty(screen, key, {{
            get: () => value,
            configurable: true,
        }});
    }}

    Object.defineProperty(window, 'devicePixelRatio', {{
        get: () => {dpr},
        configurable: true,
    }});

    // ─── Timezone override ──────────────────────────────────
    const origDateTimeFormat = Intl.DateTimeFormat;
    const origResolvedOptions = Intl.DateTimeFormat.prototype.resolvedOptions;
    Intl.DateTimeFormat.prototype.resolvedOptions = function() {{
        const result = origResolvedOptions.call(this);
        result.timeZone = {timezone};
        return result;
    }};

    // ─── Connection info ────────────────────────────────────
    if (navigator.connection) {{
        Object.defineProperty(navigator.connection, 'effectiveType', {{
            get: () => {conn_type},
            configurable: true,
        }});
        Object.defineProperty(navigator.connection, 'downlink', {{
            get: () => {conn_downlink},
            configurable: true,
        }});
        Object.defineProperty(navigator.connection, 'rtt', {{
            get: () => {conn_rtt},
            configurable: true,
        }});
    }}

    // ─── Permissions API (avoid detection via denied queries) ─
    const origQuery = Permissions.prototype.query;
    Permissions.prototype.query = function(desc) {{
        if (desc.name === 'notifications') {{
            return Promise.resolve({{ state: Notification.permission }});
        }}
        return origQuery.call(this, desc);
    }};

    // ─── Chrome runtime (headless Chrome lacks this) ────────
    if (!window.chrome) {{
        window.chrome = {{
            runtime: {{}},
            loadTimes: function() {{}},
            csi: function() {{}},
            app: {{ isInstalled: false, getIsInstalled: () => false, getDetails: () => null }},
        }};
    }}

    // ─── Plugin spoofing ────────────────────────────────────
    Object.defineProperty(navigator, 'plugins', {{
        get: () => {{
            const plugins = {plugins};
            plugins.__proto__ = PluginArray.prototype;
            return plugins;
        }},
        configurable: true,
    }});

    console.debug('[Wraith] Fingerprint injection complete');
}})();
"#,
            platform = serde_json::to_string(&fp.platform).unwrap_or_default(),
            hw_concurrency = fp.hardware_concurrency,
            device_memory = fp.device_memory.map(|d| d.to_string()).unwrap_or("undefined".to_string()),
            max_touch_points = fp.max_touch_points,
            language = serde_json::to_string(&fp.language).unwrap_or_default(),
            languages = serde_json::to_string(&fp.languages).unwrap_or_default(),
            vendor = serde_json::to_string(&fp.vendor).unwrap_or_default(),
            dnt = fp.do_not_track.as_deref().map(|d| format!("\"{}\"", d)).unwrap_or("null".to_string()),
            screen_width = fp.screen_width,
            screen_height = fp.screen_height,
            avail_width = fp.avail_width,
            avail_height = fp.avail_height,
            color_depth = fp.color_depth,
            pixel_depth = fp.pixel_depth,
            dpr = fp.device_pixel_ratio,
            timezone = serde_json::to_string(&fp.timezone).unwrap_or_default(),
            conn_type = fp.connection_type.as_deref().map(|c| format!("\"{}\"", c)).unwrap_or("\"4g\"".to_string()),
            conn_downlink = fp.connection_downlink.unwrap_or(10.0),
            conn_rtt = fp.connection_rtt.unwrap_or(50),
            plugins = "[]", // TODO: build from fp.plugins
        )
    }

    /// Build extra HTTP headers from the fingerprint.
    fn build_extra_headers(fp: &BrowserFingerprint) -> Vec<(String, String)> {
        let mut headers = vec![
            ("Accept-Language".to_string(), fp.accept_language.clone()),
        ];

        if let Some(ref ch_ua) = fp.sec_ch_ua {
            headers.push(("Sec-CH-UA".to_string(), ch_ua.clone()));
        }
        if let Some(ref platform) = fp.sec_ch_ua_platform {
            headers.push(("Sec-CH-UA-Platform".to_string(), platform.clone()));
        }
        if let Some(ref mobile) = fp.sec_ch_ua_mobile {
            headers.push(("Sec-CH-UA-Mobile".to_string(), mobile.clone()));
        }
        if let Some(ref versions) = fp.sec_ch_ua_full_version_list {
            headers.push(("Sec-CH-UA-Full-Version-List".to_string(), versions.clone()));
        }

        headers
    }

    fn get_active(&self) -> IdentityResult<&BrowserFingerprint> {
        if let Some(ref id) = self.active_id {
            self.profiles.iter().find(|p| &p.id == id)
                .ok_or_else(|| crate::error::IdentityError::FingerprintFailed(
                    format!("Active profile {} not found", id)
                ))
        } else if let Some(first) = self.profiles.first() {
            Ok(first)
        } else {
            Err(crate::error::IdentityError::FingerprintFailed(
                "No fingerprint profiles available — run 'wraith-browser fingerprint capture' first".to_string()
            ))
        }
    }

    pub fn set_active(&mut self, id: &str) {
        self.active_id = Some(id.to_string());
    }

    pub fn add_profile(&mut self, fp: BrowserFingerprint) {
        info!(id = %fp.id, name = %fp.name, "Adding fingerprint profile");
        self.profiles.push(fp);
    }

    pub fn list_profiles(&self) -> &[BrowserFingerprint] {
        &self.profiles
    }
}

/// CDP override commands to apply before each navigation.
#[derive(Debug, Clone)]
pub struct CdpOverrides {
    /// User-Agent for Network.setUserAgentOverride
    pub user_agent: String,

    /// Accept-Language header
    pub accept_language: String,

    /// Platform for navigator.platform override
    pub platform: String,

    /// Viewport dimensions for Emulation.setDeviceMetricsOverride
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub device_pixel_ratio: f64,

    /// Timezone for Emulation.setTimezoneOverride
    pub timezone_id: String,

    /// JavaScript to inject via Page.addScriptToEvaluateOnNewDocument
    pub injection_script: String,

    /// Extra HTTP headers via Network.setExtraHTTPHeaders
    pub extra_headers: Vec<(String, String)>,
}

/// The JavaScript that runs in the user's REAL browser to capture their fingerprint.
/// This is injected via CDP Runtime.evaluate when running fingerprint capture.
pub const CAPTURE_SCRIPT: &str = r#"
(async () => {
    const fp = {};

    // Navigator
    fp.userAgent = navigator.userAgent;
    fp.platform = navigator.platform;
    fp.hardwareConcurrency = navigator.hardwareConcurrency;
    fp.deviceMemory = navigator.deviceMemory;
    fp.maxTouchPoints = navigator.maxTouchPoints;
    fp.language = navigator.language;
    fp.languages = Array.from(navigator.languages);
    fp.vendor = navigator.vendor;
    fp.doNotTrack = navigator.doNotTrack;

    // Screen
    fp.screenWidth = screen.width;
    fp.screenHeight = screen.height;
    fp.availWidth = screen.availWidth;
    fp.availHeight = screen.availHeight;
    fp.colorDepth = screen.colorDepth;
    fp.pixelDepth = screen.pixelDepth;
    fp.devicePixelRatio = window.devicePixelRatio;

    // Timezone
    fp.timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    fp.timezoneOffset = new Date().getTimezoneOffset();

    // WebGL
    try {
        const canvas = document.createElement('canvas');
        const gl = canvas.getContext('webgl') || canvas.getContext('experimental-webgl');
        if (gl) {
            fp.webglVendor = gl.getParameter(gl.VENDOR);
            fp.webglRenderer = gl.getParameter(gl.RENDERER);
            const ext = gl.getExtension('WEBGL_debug_renderer_info');
            if (ext) {
                fp.webglUnmaskedVendor = gl.getParameter(ext.UNMASKED_VENDOR_WEBGL);
                fp.webglUnmaskedRenderer = gl.getParameter(ext.UNMASKED_RENDERER_WEBGL);
            }
        }
    } catch(e) {}

    // Canvas fingerprint
    try {
        const canvas = document.createElement('canvas');
        canvas.width = 200; canvas.height = 50;
        const ctx = canvas.getContext('2d');
        ctx.textBaseline = 'top';
        ctx.font = '14px Arial';
        ctx.fillStyle = '#f60';
        ctx.fillRect(125, 1, 62, 20);
        ctx.fillStyle = '#069';
        ctx.fillText('Wraith FP', 2, 15);
        ctx.fillStyle = 'rgba(102, 204, 0, 0.7)';
        ctx.fillText('Wraith FP', 4, 17);
        fp.canvasHash = canvas.toDataURL();
    } catch(e) {}

    // Connection
    if (navigator.connection) {
        fp.connectionType = navigator.connection.effectiveType;
        fp.connectionDownlink = navigator.connection.downlink;
        fp.connectionRtt = navigator.connection.rtt;
    }

    // Client Hints
    try {
        const hints = await navigator.userAgentData?.getHighEntropyValues([
            'platform', 'platformVersion', 'architecture', 'model',
            'uaFullVersion', 'fullVersionList'
        ]);
        if (hints) {
            fp.secChUa = navigator.userAgentData?.brands?.map(
                b => `"${b.brand}";v="${b.version}"`
            ).join(', ');
            fp.secChUaPlatform = `"${hints.platform}"`;
            fp.secChUaMobile = navigator.userAgentData?.mobile ? '?1' : '?0';
            fp.secChUaFullVersionList = hints.fullVersionList?.map(
                b => `"${b.brand}";v="${b.version}"`
            ).join(', ');
        }
    } catch(e) {}

    // Plugins
    fp.plugins = Array.from(navigator.plugins || []).map(p => p.name);
    fp.mimeTypes = Array.from(navigator.mimeTypes || []).map(m => m.type);

    // WebDriver detection
    fp.webdriver = navigator.webdriver;

    return JSON.stringify(fp);
})()
"#;
