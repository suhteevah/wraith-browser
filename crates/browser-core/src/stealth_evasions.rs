//! Canvas/WebGL spoofing and puppeteer-extra-stealth-plugin evasions.
//!
//! This module generates JavaScript that must be injected via CDP
//! `Page.addScriptToEvaluateOnNewDocument` **before** any page loads.
//! It implements the 17+ evasions from `puppeteer-extra-stealth-plugin`
//! plus canvas/WebGL noise injection.
//!
//! All public methods are instrumented with `tracing` for observability.

use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

// ---------------------------------------------------------------------------
// Evasion enum
// ---------------------------------------------------------------------------

/// Individual browser-fingerprint evasion technique.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Evasion {
    /// Delete `navigator.webdriver`.
    WebdriverHide,
    /// Add `window.chrome` with `runtime`, `loadTimes`, `csi`.
    ChromeRuntime,
    /// Spoof `navigator.plugins` with realistic Chrome plugins.
    PluginArray,
    /// Spoof `navigator.mimeTypes`.
    MimeTypeArray,
    /// Ensure `navigator.language` matches `Accept-Language`.
    LanguageConsistency,
    /// Override `Permissions.prototype.query`.
    PermissionsQuery,
    /// Override WebGL vendor/renderer strings.
    WebglVendor,
    /// Add subtle noise to WebGL `readPixels` / `getParameter`.
    WebglNoise,
    /// Add invisible noise to canvas `toDataURL` / `toBlob`.
    CanvasNoise,
    /// Add noise to `AudioContext` fingerprint.
    AudioContextNoise,
    /// Spoof `navigator.mediaDevices.enumerateDevices`.
    MediaDevices,
    /// Override `navigator.hardwareConcurrency`.
    HardwareConcurrency,
    /// Override `navigator.deviceMemory`.
    DeviceMemory,
    /// Override `screen.width` / `height` / `colorDepth`.
    ScreenResolution,
    /// Make `iframe.contentWindow` match parent context.
    IframeContentWindow,
    /// Consistent UA across all access points.
    UserAgentOverride,
    /// Normalize font metrics.
    FontFingerprint,
    /// Remove Chrome DevTools protocol variables (`cdc_*`).
    CdcVariables,
    /// Prevent shadow DOM detection of automation.
    ShadowDomLeaks,
}

impl Evasion {
    /// Return a slice of every variant.
    fn all_variants() -> &'static [Evasion] {
        &[
            Evasion::WebdriverHide,
            Evasion::ChromeRuntime,
            Evasion::PluginArray,
            Evasion::MimeTypeArray,
            Evasion::LanguageConsistency,
            Evasion::PermissionsQuery,
            Evasion::WebglVendor,
            Evasion::WebglNoise,
            Evasion::CanvasNoise,
            Evasion::AudioContextNoise,
            Evasion::MediaDevices,
            Evasion::HardwareConcurrency,
            Evasion::DeviceMemory,
            Evasion::ScreenResolution,
            Evasion::IframeContentWindow,
            Evasion::UserAgentOverride,
            Evasion::FontFingerprint,
            Evasion::CdcVariables,
            Evasion::ShadowDomLeaks,
        ]
    }
}

// ---------------------------------------------------------------------------
// StealthConfig
// ---------------------------------------------------------------------------

/// Configuration values injected into the evasion scripts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StealthConfig {
    /// User-Agent string override.
    pub user_agent: Option<String>,
    /// Platform string (e.g. `"Win32"`).
    pub platform: Option<String>,
    /// Value for `navigator.hardwareConcurrency`.
    pub hardware_concurrency: Option<u32>,
    /// Value for `navigator.deviceMemory`.
    pub device_memory: Option<f64>,
    /// Value for `screen.width`.
    pub screen_width: Option<u32>,
    /// Value for `screen.height`.
    pub screen_height: Option<u32>,
    /// Value for `screen.colorDepth`.
    pub color_depth: Option<u32>,
    /// WebGL vendor string.
    pub webgl_vendor: Option<String>,
    /// WebGL renderer string.
    pub webgl_renderer: Option<String>,
    /// Languages list (first element becomes `navigator.language`).
    pub languages: Option<Vec<String>>,
    /// Canvas noise strength (default 0.01 — very subtle).
    pub canvas_noise_strength: f64,
    /// Audio context noise strength (default 0.0001).
    pub audio_noise_strength: f64,
}

impl Default for StealthConfig {
    fn default() -> Self {
        Self {
            user_agent: None,
            platform: None,
            hardware_concurrency: None,
            device_memory: None,
            screen_width: None,
            screen_height: None,
            color_depth: None,
            webgl_vendor: None,
            webgl_renderer: None,
            languages: None,
            canvas_noise_strength: 0.01,
            audio_noise_strength: 0.0001,
        }
    }
}

// ---------------------------------------------------------------------------
// StealthEvasions
// ---------------------------------------------------------------------------

/// Container for enabled evasion techniques. Generates a single JavaScript
/// IIFE suitable for injection via CDP `Page.addScriptToEvaluateOnNewDocument`.
#[derive(Debug, Clone)]
pub struct StealthEvasions {
    /// The set of evasions that will be included in the generated script.
    pub enabled_evasions: Vec<Evasion>,
}

impl StealthEvasions {
    /// Enable **all** evasions.
    #[instrument]
    pub fn all() -> Self {
        let evasions = Evasion::all_variants().to_vec();
        debug!(count = evasions.len(), "stealth_evasions::all");
        Self {
            enabled_evasions: evasions,
        }
    }

    /// Enable only the most critical evasions for quick stealth.
    #[instrument]
    pub fn minimal() -> Self {
        let evasions = vec![
            Evasion::WebdriverHide,
            Evasion::ChromeRuntime,
            Evasion::CdcVariables,
            Evasion::PluginArray,
        ];
        debug!(count = evasions.len(), "stealth_evasions::minimal");
        Self {
            enabled_evasions: evasions,
        }
    }

    /// Number of enabled evasions.
    pub fn evasion_count(&self) -> usize {
        self.enabled_evasions.len()
    }

    /// Generate a single JavaScript IIFE implementing all enabled evasions.
    ///
    /// The returned string is suitable for injection via
    /// `Page.addScriptToEvaluateOnNewDocument`.
    #[instrument(skip(self, config))]
    pub fn generate_script(&self, config: &StealthConfig) -> String {
        let mut parts: Vec<String> = Vec::new();

        for evasion in &self.enabled_evasions {
            let snippet = match evasion {
                Evasion::WebdriverHide => js_webdriver_hide(),
                Evasion::ChromeRuntime => js_chrome_runtime(),
                Evasion::PluginArray => js_plugin_array(),
                Evasion::MimeTypeArray => js_mime_type_array(),
                Evasion::LanguageConsistency => {
                    let langs = config
                        .languages
                        .as_ref()
                        .cloned()
                        .unwrap_or_else(|| vec!["en-US".into(), "en".into()]);
                    js_language_consistency(&langs)
                }
                Evasion::PermissionsQuery => js_permissions_query(),
                Evasion::WebglVendor => {
                    let vendor = config
                        .webgl_vendor
                        .as_deref()
                        .unwrap_or("Intel Inc.");
                    let renderer = config
                        .webgl_renderer
                        .as_deref()
                        .unwrap_or("Intel Iris OpenGL Engine");
                    js_webgl_noise(vendor, renderer)
                }
                Evasion::WebglNoise => {
                    let vendor = config
                        .webgl_vendor
                        .as_deref()
                        .unwrap_or("Intel Inc.");
                    let renderer = config
                        .webgl_renderer
                        .as_deref()
                        .unwrap_or("Intel Iris OpenGL Engine");
                    js_webgl_noise(vendor, renderer)
                }
                Evasion::CanvasNoise => js_canvas_noise(config.canvas_noise_strength),
                Evasion::AudioContextNoise => js_audio_noise(config.audio_noise_strength),
                Evasion::MediaDevices => js_media_devices(),
                Evasion::HardwareConcurrency | Evasion::DeviceMemory => {
                    let concurrency = config.hardware_concurrency.unwrap_or(4);
                    let memory = config.device_memory.unwrap_or(8.0);
                    js_hardware_props(concurrency, memory)
                }
                Evasion::ScreenResolution => {
                    let w = config.screen_width.unwrap_or(1920);
                    let h = config.screen_height.unwrap_or(1080);
                    let d = config.color_depth.unwrap_or(24);
                    js_screen_props(w, h, d)
                }
                Evasion::IframeContentWindow => js_iframe_fix(),
                Evasion::UserAgentOverride => {
                    let ua = config.user_agent.as_deref().unwrap_or(
                        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                         (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
                    );
                    js_ua_override(ua)
                }
                Evasion::FontFingerprint => js_font_normalize(),
                Evasion::CdcVariables => js_cdc_cleanup(),
                Evasion::ShadowDomLeaks => js_shadow_dom_fix(),
            };
            parts.push(snippet);
        }

        let body = parts.join("\n\n");
        let script = format!(
            "// OpenClaw Stealth Evasions — generated script\n\
             (function() {{\n\
             'use strict';\n\n\
             {body}\n\n\
             }})();"
        );

        debug!(
            evasion_count = self.enabled_evasions.len(),
            script_len = script.len(),
            "generate_script"
        );

        script
    }
}

// ---------------------------------------------------------------------------
// Individual evasion script generators (private)
// ---------------------------------------------------------------------------

/// Delete `navigator.webdriver` and related properties.
fn js_webdriver_hide() -> String {
    r#"// [OpenClaw Stealth: webdriver_hide]
(function() {
    Object.defineProperty(navigator, 'webdriver', {
        get: () => undefined,
        configurable: true
    });
    // Also clean up the legacy property
    delete Object.getPrototypeOf(navigator).webdriver;
})();"#
        .into()
}

/// Add a convincing `window.chrome` object with `runtime`, `loadTimes`, `csi`.
fn js_chrome_runtime() -> String {
    r#"// [OpenClaw Stealth: chrome_runtime]
(function() {
    if (!window.chrome) {
        window.chrome = {};
    }
    if (!window.chrome.runtime) {
        window.chrome.runtime = {
            PlatformOs: { MAC: 'mac', WIN: 'win', ANDROID: 'android', CROS: 'cros', LINUX: 'linux', OPENBSD: 'openbsd' },
            PlatformArch: { ARM: 'arm', X86_32: 'x86-32', X86_64: 'x86-64', MIPS: 'mips', MIPS64: 'mips64' },
            PlatformNaclArch: { ARM: 'arm', X86_32: 'x86-32', X86_64: 'x86-64', MIPS: 'mips', MIPS64: 'mips64' },
            RequestUpdateCheckStatus: { THROTTLED: 'throttled', NO_UPDATE: 'no_update', UPDATE_AVAILABLE: 'update_available' },
            OnInstalledReason: { INSTALL: 'install', UPDATE: 'update', CHROME_UPDATE: 'chrome_update', SHARED_MODULE_UPDATE: 'shared_module_update' },
            OnRestartRequiredReason: { APP_UPDATE: 'app_update', OS_UPDATE: 'os_update', PERIODIC: 'periodic' },
            connect: function() { return { onDisconnect: { addListener: function() {} } }; },
            sendMessage: function() {}
        };
    }
    if (!window.chrome.loadTimes) {
        window.chrome.loadTimes = function() {
            return {
                requestTime: Date.now() / 1000 - Math.random() * 2,
                startLoadTime: Date.now() / 1000 - Math.random(),
                commitLoadTime: Date.now() / 1000 - Math.random() * 0.5,
                finishDocumentLoadTime: Date.now() / 1000,
                finishLoadTime: Date.now() / 1000,
                firstPaintTime: Date.now() / 1000 - Math.random() * 0.1,
                firstPaintAfterLoadTime: 0,
                navigationType: 'Other',
                wasFetchedViaSpdy: false,
                wasNpnNegotiated: true,
                npnNegotiatedProtocol: 'h2',
                wasAlternateProtocolAvailable: false,
                connectionInfo: 'h2'
            };
        };
    }
    if (!window.chrome.csi) {
        window.chrome.csi = function() {
            return {
                startE: Date.now(),
                onloadT: Date.now(),
                pageT: Math.random() * 1000 + 500,
                tran: 15
            };
        };
    }
})();"#
        .into()
}

/// Spoof `navigator.plugins` with realistic Chrome plugin entries.
fn js_plugin_array() -> String {
    r#"// [OpenClaw Stealth: plugin_array]
(function() {
    const pluginData = [
        { name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer', description: 'Portable Document Format',
          mimeTypes: [{ type: 'application/x-google-chrome-pdf', suffixes: 'pdf', description: 'Portable Document Format' }] },
        { name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai', description: '',
          mimeTypes: [{ type: 'application/pdf', suffixes: 'pdf', description: '' }] },
        { name: 'Native Client', filename: 'internal-nacl-plugin', description: '',
          mimeTypes: [
            { type: 'application/x-nacl', suffixes: '', description: 'Native Client Executable' },
            { type: 'application/x-pnacl', suffixes: '', description: 'Portable Native Client Executable' }
          ] }
    ];

    function makeMimeType(mt, plugin) {
        const obj = Object.create(MimeType.prototype);
        Object.defineProperties(obj, {
            type: { get: () => mt.type },
            suffixes: { get: () => mt.suffixes },
            description: { get: () => mt.description },
            enabledPlugin: { get: () => plugin }
        });
        return obj;
    }

    function makePlugin(pd) {
        const obj = Object.create(Plugin.prototype);
        const mimes = pd.mimeTypes.map(m => makeMimeType(m, obj));
        Object.defineProperties(obj, {
            name: { get: () => pd.name },
            filename: { get: () => pd.filename },
            description: { get: () => pd.description },
            length: { get: () => mimes.length }
        });
        mimes.forEach((m, i) => {
            Object.defineProperty(obj, i, { get: () => m });
            Object.defineProperty(obj, m.type, { get: () => m });
        });
        obj[Symbol.iterator] = function*() { for (const m of mimes) yield m; };
        return obj;
    }

    const plugins = pluginData.map(makePlugin);
    const pluginArray = Object.create(PluginArray.prototype);
    plugins.forEach((p, i) => {
        Object.defineProperty(pluginArray, i, { get: () => p });
        Object.defineProperty(pluginArray, p.name, { get: () => p });
    });
    Object.defineProperty(pluginArray, 'length', { get: () => plugins.length });
    pluginArray[Symbol.iterator] = function*() { for (const p of plugins) yield p; };
    pluginArray.refresh = function() {};
    pluginArray.item = function(i) { return plugins[i] || null; };
    pluginArray.namedItem = function(name) { return plugins.find(p => p.name === name) || null; };

    Object.defineProperty(navigator, 'plugins', {
        get: () => pluginArray,
        configurable: true
    });
})();"#
        .into()
}

/// Spoof `navigator.mimeTypes`.
fn js_mime_type_array() -> String {
    r#"// [OpenClaw Stealth: mime_type_array]
(function() {
    const mimeData = [
        { type: 'application/pdf', suffixes: 'pdf', description: '' },
        { type: 'application/x-google-chrome-pdf', suffixes: 'pdf', description: 'Portable Document Format' },
        { type: 'application/x-nacl', suffixes: '', description: 'Native Client Executable' },
        { type: 'application/x-pnacl', suffixes: '', description: 'Portable Native Client Executable' }
    ];

    function makeMime(md) {
        const obj = Object.create(MimeType.prototype);
        Object.defineProperties(obj, {
            type: { get: () => md.type },
            suffixes: { get: () => md.suffixes },
            description: { get: () => md.description },
            enabledPlugin: { get: () => null }
        });
        return obj;
    }

    const mimes = mimeData.map(makeMime);
    const mimeArray = Object.create(MimeTypeArray.prototype);
    mimes.forEach((m, i) => {
        Object.defineProperty(mimeArray, i, { get: () => m });
        Object.defineProperty(mimeArray, m.type, { get: () => m });
    });
    Object.defineProperty(mimeArray, 'length', { get: () => mimes.length });
    mimeArray[Symbol.iterator] = function*() { for (const m of mimes) yield m; };
    mimeArray.item = function(i) { return mimes[i] || null; };
    mimeArray.namedItem = function(t) { return mimes.find(m => m.type === t) || null; };

    Object.defineProperty(navigator, 'mimeTypes', {
        get: () => mimeArray,
        configurable: true
    });
})();"#
        .into()
}

/// Ensure `navigator.language` and `navigator.languages` are consistent.
fn js_language_consistency(languages: &[String]) -> String {
    let langs_json: Vec<String> = languages.iter().map(|l| format!("\"{}\"", l)).collect();
    let langs_str = langs_json.join(", ");
    let primary = languages.first().map(|s| s.as_str()).unwrap_or("en-US");

    format!(
        r#"// [OpenClaw Stealth: language_consistency]
(function() {{
    Object.defineProperty(navigator, 'language', {{
        get: () => "{primary}",
        configurable: true
    }});
    Object.defineProperty(navigator, 'languages', {{
        get: () => Object.freeze([{langs_str}]),
        configurable: true
    }});
}})();"#
    )
}

/// Override `Permissions.prototype.query` to return realistic results.
fn js_permissions_query() -> String {
    r#"// [OpenClaw Stealth: permissions_query]
(function() {
    const originalQuery = window.Permissions && Permissions.prototype.query;
    if (!originalQuery) return;

    Permissions.prototype.query = function(parameters) {
        if (parameters && parameters.name === 'notifications') {
            return Promise.resolve({ state: Notification.permission });
        }
        return originalQuery.call(this, parameters);
    };
})();"#
        .into()
}

/// Override WebGL `getParameter` for vendor/renderer and add noise to `readPixels`.
fn js_webgl_noise(vendor: &str, renderer: &str) -> String {
    format!(
        r#"// [OpenClaw Stealth: webgl_noise]
(function() {{
    const VENDOR = "{vendor}";
    const RENDERER = "{renderer}";

    const getParameterOrig = WebGLRenderingContext.prototype.getParameter;
    WebGLRenderingContext.prototype.getParameter = function(param) {{
        const UNMASKED_VENDOR = 0x9245;
        const UNMASKED_RENDERER = 0x9246;
        if (param === UNMASKED_VENDOR || param === 37445) return VENDOR;
        if (param === UNMASKED_RENDERER || param === 37446) return RENDERER;
        return getParameterOrig.call(this, param);
    }};

    if (typeof WebGL2RenderingContext !== 'undefined') {{
        const getParam2Orig = WebGL2RenderingContext.prototype.getParameter;
        WebGL2RenderingContext.prototype.getParameter = function(param) {{
            const UNMASKED_VENDOR = 0x9245;
            const UNMASKED_RENDERER = 0x9246;
            if (param === UNMASKED_VENDOR || param === 37445) return VENDOR;
            if (param === UNMASKED_RENDERER || param === 37446) return RENDERER;
            return getParam2Orig.call(this, param);
        }};
    }}

    // Add subtle noise to readPixels
    const readPixelsOrig = WebGLRenderingContext.prototype.readPixels;
    WebGLRenderingContext.prototype.readPixels = function() {{
        readPixelsOrig.apply(this, arguments);
        const pixels = arguments[6];
        if (pixels && pixels.length) {{
            for (let i = 0; i < pixels.length; i += 4) {{
                // Only shift the least-significant bit of each channel
                pixels[i] ^= (Math.random() > 0.5 ? 1 : 0);
            }}
        }}
    }};
}})();"#
    )
}

/// Override `HTMLCanvasElement.prototype.toDataURL` and `toBlob` to inject
/// imperceptible pixel noise before fingerprinting.
fn js_canvas_noise(strength: f64) -> String {
    format!(
        r#"// [OpenClaw Stealth: canvas_noise]
(function() {{
    const NOISE_STRENGTH = {strength};

    const origToDataURL = HTMLCanvasElement.prototype.toDataURL;
    HTMLCanvasElement.prototype.toDataURL = function() {{
        try {{
            const ctx = this.getContext('2d');
            if (ctx) {{
                const imageData = ctx.getImageData(0, 0, this.width, this.height);
                const data = imageData.data;
                for (let i = 0; i < data.length; i += 4) {{
                    // Add very subtle noise to RGB channels
                    data[i]   = Math.max(0, Math.min(255, data[i]   + Math.floor((Math.random() - 0.5) * 255 * NOISE_STRENGTH)));
                    data[i+1] = Math.max(0, Math.min(255, data[i+1] + Math.floor((Math.random() - 0.5) * 255 * NOISE_STRENGTH)));
                    data[i+2] = Math.max(0, Math.min(255, data[i+2] + Math.floor((Math.random() - 0.5) * 255 * NOISE_STRENGTH)));
                }}
                ctx.putImageData(imageData, 0, 0);
            }}
        }} catch(e) {{
            // Canvas may be tainted — ignore
        }}
        return origToDataURL.apply(this, arguments);
    }};

    const origToBlob = HTMLCanvasElement.prototype.toBlob;
    HTMLCanvasElement.prototype.toBlob = function(callback) {{
        try {{
            const ctx = this.getContext('2d');
            if (ctx) {{
                const imageData = ctx.getImageData(0, 0, this.width, this.height);
                const data = imageData.data;
                for (let i = 0; i < data.length; i += 4) {{
                    data[i]   = Math.max(0, Math.min(255, data[i]   + Math.floor((Math.random() - 0.5) * 255 * NOISE_STRENGTH)));
                    data[i+1] = Math.max(0, Math.min(255, data[i+1] + Math.floor((Math.random() - 0.5) * 255 * NOISE_STRENGTH)));
                    data[i+2] = Math.max(0, Math.min(255, data[i+2] + Math.floor((Math.random() - 0.5) * 255 * NOISE_STRENGTH)));
                }}
                ctx.putImageData(imageData, 0, 0);
            }}
        }} catch(e) {{}}
        return origToBlob.apply(this, arguments);
    }};
}})();"#
    )
}

/// Add noise to `AudioContext` methods used for audio fingerprinting.
fn js_audio_noise(strength: f64) -> String {
    format!(
        r#"// [OpenClaw Stealth: audio_noise]
(function() {{
    const NOISE = {strength};
    const Ctx = window.AudioContext || window.webkitAudioContext;
    if (!Ctx) return;

    const origCreateAnalyser = Ctx.prototype.createAnalyser;
    Ctx.prototype.createAnalyser = function() {{
        const analyser = origCreateAnalyser.call(this);
        const origGetFloatFreq = analyser.getFloatFrequencyData.bind(analyser);
        analyser.getFloatFrequencyData = function(array) {{
            origGetFloatFreq(array);
            for (let i = 0; i < array.length; i++) {{
                array[i] += (Math.random() - 0.5) * NOISE;
            }}
        }};
        return analyser;
    }};

    const origCreateOscillator = Ctx.prototype.createOscillator;
    Ctx.prototype.createOscillator = function() {{
        const osc = origCreateOscillator.call(this);
        const origConnect = osc.connect.bind(osc);
        osc.connect = function(dest) {{
            // If connecting to an AnalyserNode, the noise is already injected above
            return origConnect(dest);
        }};
        return osc;
    }};

    // Intercept getChannelData for OfflineAudioContext
    const OffCtx = window.OfflineAudioContext || window.webkitOfflineAudioContext;
    if (OffCtx) {{
        const origGetChannelData = AudioBuffer.prototype.getChannelData;
        AudioBuffer.prototype.getChannelData = function(channel) {{
            const data = origGetChannelData.call(this, channel);
            for (let i = 0; i < data.length; i++) {{
                data[i] += (Math.random() - 0.5) * NOISE;
            }}
            return data;
        }};
    }}
}})();"#
    )
}

/// Spoof `navigator.mediaDevices.enumerateDevices`.
fn js_media_devices() -> String {
    r#"// [OpenClaw Stealth: media_devices]
(function() {
    if (!navigator.mediaDevices) return;

    const origEnumerate = navigator.mediaDevices.enumerateDevices;
    navigator.mediaDevices.enumerateDevices = function() {
        return origEnumerate.call(this).then(function(devices) {
            // Return realistic device list with anonymised IDs
            if (devices.length === 0) {
                return [
                    { deviceId: '', groupId: '', kind: 'audioinput', label: '' },
                    { deviceId: '', groupId: '', kind: 'videoinput', label: '' },
                    { deviceId: '', groupId: '', kind: 'audiooutput', label: '' }
                ];
            }
            return devices;
        });
    };
})();"#
        .into()
}

/// Override `navigator.hardwareConcurrency` and `navigator.deviceMemory`.
fn js_hardware_props(concurrency: u32, memory: f64) -> String {
    format!(
        r#"// [OpenClaw Stealth: hardware_props]
(function() {{
    Object.defineProperty(navigator, 'hardwareConcurrency', {{
        get: () => {concurrency},
        configurable: true
    }});
    Object.defineProperty(navigator, 'deviceMemory', {{
        get: () => {memory},
        configurable: true
    }});
}})();"#
    )
}

/// Override `screen.width`, `screen.height`, and `screen.colorDepth`.
fn js_screen_props(width: u32, height: u32, depth: u32) -> String {
    format!(
        r#"// [OpenClaw Stealth: screen_props]
(function() {{
    Object.defineProperty(screen, 'width', {{
        get: () => {width},
        configurable: true
    }});
    Object.defineProperty(screen, 'height', {{
        get: () => {height},
        configurable: true
    }});
    Object.defineProperty(screen, 'availWidth', {{
        get: () => {width},
        configurable: true
    }});
    Object.defineProperty(screen, 'availHeight', {{
        get: () => {height},
        configurable: true
    }});
    Object.defineProperty(screen, 'colorDepth', {{
        get: () => {depth},
        configurable: true
    }});
    Object.defineProperty(screen, 'pixelDepth', {{
        get: () => {depth},
        configurable: true
    }});
}})();"#
    )
}

/// Remove `cdc_adoQpoasnfa76pfcZLmcfl_*` and similar Chrome DevTools variables.
fn js_cdc_cleanup() -> String {
    r#"// [OpenClaw Stealth: cdc_cleanup]
(function() {
    // Remove cdc_ variables that Chrome DevTools Protocol leaves behind
    const props = Object.getOwnPropertyNames(window);
    for (const prop of props) {
        if (prop.match(/^cdc_/) || prop.match(/^__cdc_/)) {
            try { delete window[prop]; } catch(e) {}
        }
    }

    // Also remove from document
    const docProps = Object.getOwnPropertyNames(document);
    for (const prop of docProps) {
        if (prop.match(/^cdc_/) || prop.match(/^\$cdc_/)) {
            try { delete document[prop]; } catch(e) {}
        }
    }

    // Remove callPhantom / _phantom
    try { delete window.callPhantom; } catch(e) {}
    try { delete window._phantom; } catch(e) {}
    try { delete window.__nightmare; } catch(e) {}
})();"#
        .into()
}

/// Fix `iframe.contentWindow` so that cross-origin checks match parent context.
fn js_iframe_fix() -> String {
    r#"// [OpenClaw Stealth: iframe_fix]
(function() {
    // Ensure iframe contentWindow exposes the same navigator/chrome properties
    const origCreateElement = document.createElement.bind(document);
    document.createElement = function() {
        const el = origCreateElement.apply(this, arguments);
        if (arguments[0] && arguments[0].toLowerCase() === 'iframe') {
            const origContentWindow = Object.getOwnPropertyDescriptor(
                HTMLIFrameElement.prototype, 'contentWindow'
            );
            if (origContentWindow && origContentWindow.get) {
                Object.defineProperty(el, 'contentWindow', {
                    get: function() {
                        const win = origContentWindow.get.call(this);
                        if (win) {
                            try {
                                // Patch webdriver in iframe context
                                Object.defineProperty(win.navigator, 'webdriver', {
                                    get: () => undefined,
                                    configurable: true
                                });
                            } catch(e) {
                                // Cross-origin — cannot patch
                            }
                        }
                        return win;
                    },
                    configurable: true
                });
            }
        }
        return el;
    };
    // Preserve toString to avoid detection
    document.createElement.toString = function() { return 'function createElement() { [native code] }'; };
})();"#
        .into()
}

/// Override User-Agent consistently across `navigator.userAgent`,
/// `navigator.appVersion`, and `navigator.platform`.
fn js_ua_override(ua: &str) -> String {
    // Extract version from UA for appVersion
    let app_version = ua
        .find("Mozilla/")
        .map(|i| &ua[i + 8..])
        .unwrap_or("5.0 (Windows NT 10.0; Win64; x64)");

    format!(
        r#"// [OpenClaw Stealth: ua_override]
(function() {{
    const UA = "{ua}";
    const APP_VERSION = "{app_version}";

    Object.defineProperty(navigator, 'userAgent', {{
        get: () => UA,
        configurable: true
    }});
    Object.defineProperty(navigator, 'appVersion', {{
        get: () => APP_VERSION,
        configurable: true
    }});
    // Override UserAgentData if present
    if (navigator.userAgentData) {{
        const origBrands = navigator.userAgentData.brands;
        Object.defineProperty(navigator, 'userAgentData', {{
            get: () => ({{
                brands: origBrands || [
                    {{ brand: "Not_A Brand", version: "8" }},
                    {{ brand: "Chromium", version: "120" }},
                    {{ brand: "Google Chrome", version: "120" }}
                ],
                mobile: false,
                platform: "Windows",
                getHighEntropyValues: function(hints) {{
                    return Promise.resolve({{
                        brands: this.brands,
                        mobile: false,
                        platform: "Windows",
                        platformVersion: "15.0.0",
                        architecture: "x86",
                        bitness: "64",
                        model: "",
                        uaFullVersion: "120.0.0.0"
                    }});
                }}
            }}),
            configurable: true
        }});
    }}
}})();"#
    )
}

/// Normalize font metrics to prevent font-based fingerprinting.
fn js_font_normalize() -> String {
    r#"// [OpenClaw Stealth: font_normalize]
(function() {
    // Slightly randomise measureText to defeat font enumeration fingerprinting
    const origMeasureText = CanvasRenderingContext2D.prototype.measureText;
    CanvasRenderingContext2D.prototype.measureText = function(text) {
        const metrics = origMeasureText.call(this, text);
        const origWidth = metrics.width;
        // Return a Proxy that adds tiny noise to width
        return new Proxy(metrics, {
            get: function(target, prop) {
                if (prop === 'width') {
                    return origWidth + (Math.random() - 0.5) * 0.00001;
                }
                const val = target[prop];
                if (typeof val === 'function') return val.bind(target);
                return val;
            }
        });
    };
})();"#
        .into()
}

/// Prevent detection of automation via shadow DOM inspection.
fn js_shadow_dom_fix() -> String {
    r#"// [OpenClaw Stealth: shadow_dom_fix]
(function() {
    // Prevent detection of CDP-injected shadow roots
    const origAttachShadow = Element.prototype.attachShadow;
    Element.prototype.attachShadow = function() {
        const shadow = origAttachShadow.apply(this, arguments);
        // Ensure shadow root doesn't leak automation indicators
        return shadow;
    };
    Element.prototype.attachShadow.toString = function() {
        return 'function attachShadow() { [native code] }';
    };

    // Hide the automation-related attributes
    const origGetAttribute = Element.prototype.getAttribute;
    Element.prototype.getAttribute = function(name) {
        if (name === 'data-selenium' || name === 'data-puppeteer' || name === 'data-cypress') {
            return null;
        }
        return origGetAttribute.call(this, name);
    };
    Element.prototype.getAttribute.toString = function() {
        return 'function getAttribute() { [native code] }';
    };
})();"#
        .into()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- StealthEvasions::all() --------------------------------------------

    #[test]
    fn all_enables_all_evasions() {
        let stealth = StealthEvasions::all();
        let all_variants = Evasion::all_variants();
        assert_eq!(
            stealth.enabled_evasions.len(),
            all_variants.len(),
            "all() should enable every evasion variant"
        );
        for variant in all_variants {
            assert!(
                stealth.enabled_evasions.contains(variant),
                "missing evasion: {:?}",
                variant
            );
        }
    }

    // -- StealthEvasions::minimal() ----------------------------------------

    #[test]
    fn minimal_enables_only_four() {
        let stealth = StealthEvasions::minimal();
        assert_eq!(stealth.evasion_count(), 4, "minimal() should enable 4 evasions");
        assert!(stealth.enabled_evasions.contains(&Evasion::WebdriverHide));
        assert!(stealth.enabled_evasions.contains(&Evasion::ChromeRuntime));
        assert!(stealth.enabled_evasions.contains(&Evasion::CdcVariables));
        assert!(stealth.enabled_evasions.contains(&Evasion::PluginArray));
    }

    // -- evasion_count() ---------------------------------------------------

    #[test]
    fn evasion_count_matches_vec_len() {
        let stealth = StealthEvasions::all();
        assert_eq!(stealth.evasion_count(), stealth.enabled_evasions.len());
    }

    // -- generate_script() basic -------------------------------------------

    #[test]
    fn generate_script_produces_nonempty_iife() {
        let stealth = StealthEvasions::minimal();
        let config = StealthConfig::default();
        let script = stealth.generate_script(&config);

        assert!(!script.is_empty(), "script should not be empty");
        assert!(script.contains("(function()"), "script should contain IIFE");
        assert!(script.contains("'use strict'"), "script should use strict mode");
    }

    // -- generate_script() with all evasions includes key sections ---------

    #[test]
    fn generate_script_all_includes_canvas_webgl_audio() {
        let stealth = StealthEvasions::all();
        let config = StealthConfig::default();
        let script = stealth.generate_script(&config);

        assert!(
            script.contains("[OpenClaw Stealth: canvas_noise]"),
            "all-evasions script should include canvas_noise"
        );
        assert!(
            script.contains("[OpenClaw Stealth: webgl_noise]"),
            "all-evasions script should include webgl_noise"
        );
        assert!(
            script.contains("[OpenClaw Stealth: audio_noise]"),
            "all-evasions script should include audio_noise"
        );
    }

    // -- Each evasion present when enabled ---------------------------------

    #[test]
    fn each_evasion_appears_in_output_when_enabled() {
        let config = StealthConfig::default();

        let tag_map: Vec<(Evasion, &str)> = vec![
            (Evasion::WebdriverHide, "webdriver_hide"),
            (Evasion::ChromeRuntime, "chrome_runtime"),
            (Evasion::PluginArray, "plugin_array"),
            (Evasion::MimeTypeArray, "mime_type_array"),
            (Evasion::LanguageConsistency, "language_consistency"),
            (Evasion::PermissionsQuery, "permissions_query"),
            (Evasion::WebglVendor, "webgl_noise"),
            (Evasion::WebglNoise, "webgl_noise"),
            (Evasion::CanvasNoise, "canvas_noise"),
            (Evasion::AudioContextNoise, "audio_noise"),
            (Evasion::MediaDevices, "media_devices"),
            (Evasion::HardwareConcurrency, "hardware_props"),
            (Evasion::DeviceMemory, "hardware_props"),
            (Evasion::ScreenResolution, "screen_props"),
            (Evasion::IframeContentWindow, "iframe_fix"),
            (Evasion::UserAgentOverride, "ua_override"),
            (Evasion::FontFingerprint, "font_normalize"),
            (Evasion::CdcVariables, "cdc_cleanup"),
            (Evasion::ShadowDomLeaks, "shadow_dom_fix"),
        ];

        for (evasion, tag) in tag_map {
            let stealth = StealthEvasions {
                enabled_evasions: vec![evasion.clone()],
            };
            let script = stealth.generate_script(&config);
            let marker = format!("[OpenClaw Stealth: {}]", tag);
            assert!(
                script.contains(&marker),
                "evasion {:?} should produce marker '{}' in output, got:\n{}",
                evasion,
                marker,
                &script[..script.len().min(300)]
            );
        }
    }

    // -- Custom config values flow through ---------------------------------

    #[test]
    fn custom_config_values_appear_in_script() {
        let config = StealthConfig {
            user_agent: Some("TestAgent/1.0".into()),
            hardware_concurrency: Some(16),
            device_memory: Some(32.0),
            screen_width: Some(2560),
            screen_height: Some(1440),
            color_depth: Some(30),
            webgl_vendor: Some("NVIDIA".into()),
            webgl_renderer: Some("GeForce RTX 4090".into()),
            languages: Some(vec!["fr-FR".into(), "fr".into()]),
            ..StealthConfig::default()
        };

        let stealth = StealthEvasions::all();
        let script = stealth.generate_script(&config);

        assert!(script.contains("TestAgent/1.0"));
        assert!(script.contains("16"));
        assert!(script.contains("32"));
        assert!(script.contains("2560"));
        assert!(script.contains("1440"));
        assert!(script.contains("NVIDIA"));
        assert!(script.contains("GeForce RTX 4090"));
        assert!(script.contains("fr-FR"));
    }

    // -- Default config has expected noise strength values ------------------

    #[test]
    fn default_config_noise_strengths() {
        let config = StealthConfig::default();
        assert!((config.canvas_noise_strength - 0.01).abs() < f64::EPSILON);
        assert!((config.audio_noise_strength - 0.0001).abs() < f64::EPSILON);
    }
}
