//! # TLS Fingerprint Matching
//!
//! Configures HTTP clients to match real browser TLS fingerprints (JA3/JA4)
//! so that bot-detection systems see the same TLS characteristics as a real
//! Chrome, Firefox, or Safari browser.
//!
//! This module provides an abstraction layer over TLS profile configuration.
//! Each [`TlsProfile`] bundles the JA3/JA4 hash, HTTP/2 SETTINGS frame
//! parameters, and the exact header ordering that a specific browser version
//! would emit.
//!
//! ## Usage
//!
//! ```rust
//! use wraith_browser_core::tls_fingerprint::{chrome_131_profile, apply_profile_headers};
//!
//! let profile = chrome_131_profile();
//! let headers = apply_profile_headers(&profile);
//! // Pass `headers` to your HTTP client in order.
//! ```

use rand::Rng;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument};

// ---------------------------------------------------------------------------
// Http2Settings
// ---------------------------------------------------------------------------

/// HTTP/2 SETTINGS frame parameters that match a specific browser.
///
/// Different browsers advertise different HTTP/2 settings, and bot-detection
/// systems fingerprint these values.  Each [`TlsProfile`] includes the
/// correct settings for its browser version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Http2Settings {
    /// SETTINGS_HEADER_TABLE_SIZE (default 65536 for Chrome).
    pub header_table_size: u32,
    /// SETTINGS_ENABLE_PUSH (most modern browsers disable server push).
    pub enable_push: bool,
    /// SETTINGS_MAX_CONCURRENT_STREAMS.
    pub max_concurrent_streams: u32,
    /// SETTINGS_INITIAL_WINDOW_SIZE.
    pub initial_window_size: u32,
    /// SETTINGS_MAX_FRAME_SIZE.
    pub max_frame_size: u32,
    /// SETTINGS_MAX_HEADER_LIST_SIZE.
    pub max_header_list_size: u32,
}

impl Default for Http2Settings {
    fn default() -> Self {
        Self {
            header_table_size: 65536,
            enable_push: false,
            max_concurrent_streams: 1000,
            initial_window_size: 6291456,
            max_frame_size: 16384,
            max_header_list_size: 262144,
        }
    }
}

// ---------------------------------------------------------------------------
// TlsProfile
// ---------------------------------------------------------------------------

/// A complete TLS/HTTP fingerprint profile for a specific browser version.
///
/// Encapsulates all the observable characteristics that bot-detection services
/// use to distinguish real browsers from automated clients: TLS fingerprints
/// (JA3/JA4), HTTP/2 settings, header ordering, and header values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsProfile {
    /// Human-readable profile name, e.g. "Chrome 131 Windows".
    pub name: String,
    /// The `User-Agent` header value.
    pub user_agent: String,
    /// Expected JA3 fingerprint hash for this browser configuration.
    pub ja3_hash: String,
    /// Optional JA4 fingerprint hash (newer fingerprinting standard).
    pub ja4_hash: Option<String>,
    /// HTTP/2 SETTINGS frame parameters.
    pub http2_settings: Http2Settings,
    /// Order of HTTP headers as sent by the browser.
    pub header_order: Vec<String>,
    /// Value for the `Accept` header.
    pub accept_header: String,
    /// Value for the `Accept-Language` header.
    pub accept_language: String,
    /// Value for the `Accept-Encoding` header.
    pub accept_encoding: String,
    /// Value for the `Sec-CH-UA` client hint header (Chromium-based only).
    pub sec_ch_ua: Option<String>,
    /// Value for the `Sec-CH-UA-Platform` client hint header.
    pub sec_ch_ua_platform: Option<String>,
}

// ---------------------------------------------------------------------------
// Built-in profiles
// ---------------------------------------------------------------------------

/// Returns a [`TlsProfile`] matching Chrome 131 on Windows 10/11.
#[instrument]
pub fn chrome_131_profile() -> TlsProfile {
    debug!("building Chrome 131 Windows profile");
    TlsProfile {
        name: "Chrome 131 Windows".into(),
        user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                      (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
            .into(),
        ja3_hash: "cd08e31494f9531f560d64c695473da9".into(),
        ja4_hash: Some("t13d1516h2_8daaf6152771_b0da82dd1658".into()),
        http2_settings: Http2Settings {
            header_table_size: 65536,
            enable_push: false,
            max_concurrent_streams: 1000,
            initial_window_size: 6291456,
            max_frame_size: 16384,
            max_header_list_size: 262144,
        },
        header_order: vec![
            "Host".into(),
            "Connection".into(),
            "sec-ch-ua".into(),
            "sec-ch-ua-mobile".into(),
            "sec-ch-ua-platform".into(),
            "Upgrade-Insecure-Requests".into(),
            "User-Agent".into(),
            "Accept".into(),
            "Sec-Fetch-Site".into(),
            "Sec-Fetch-Mode".into(),
            "Sec-Fetch-User".into(),
            "Sec-Fetch-Dest".into(),
            "Accept-Encoding".into(),
            "Accept-Language".into(),
        ],
        accept_header: "text/html,application/xhtml+xml,application/xml;\
                         q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8"
            .into(),
        accept_language: "en-US,en;q=0.9".into(),
        accept_encoding: "gzip, deflate, br, zstd".into(),
        sec_ch_ua: Some(
            "\"Google Chrome\";v=\"131\", \"Chromium\";v=\"131\", \"Not_A Brand\";v=\"24\"".into(),
        ),
        sec_ch_ua_platform: Some("\"Windows\"".into()),
    }
}

/// Returns a [`TlsProfile`] matching Chrome 131 on macOS.
#[instrument]
pub fn chrome_131_mac_profile() -> TlsProfile {
    debug!("building Chrome 131 macOS profile");
    TlsProfile {
        name: "Chrome 131 macOS".into(),
        user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                      (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
            .into(),
        ja3_hash: "cd08e31494f9531f560d64c695473da9".into(),
        ja4_hash: Some("t13d1516h2_8daaf6152771_b0da82dd1658".into()),
        http2_settings: Http2Settings {
            header_table_size: 65536,
            enable_push: false,
            max_concurrent_streams: 1000,
            initial_window_size: 6291456,
            max_frame_size: 16384,
            max_header_list_size: 262144,
        },
        header_order: vec![
            "Host".into(),
            "Connection".into(),
            "sec-ch-ua".into(),
            "sec-ch-ua-mobile".into(),
            "sec-ch-ua-platform".into(),
            "Upgrade-Insecure-Requests".into(),
            "User-Agent".into(),
            "Accept".into(),
            "Sec-Fetch-Site".into(),
            "Sec-Fetch-Mode".into(),
            "Sec-Fetch-User".into(),
            "Sec-Fetch-Dest".into(),
            "Accept-Encoding".into(),
            "Accept-Language".into(),
        ],
        accept_header: "text/html,application/xhtml+xml,application/xml;\
                         q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8"
            .into(),
        accept_language: "en-US,en;q=0.9".into(),
        accept_encoding: "gzip, deflate, br, zstd".into(),
        sec_ch_ua: Some(
            "\"Google Chrome\";v=\"131\", \"Chromium\";v=\"131\", \"Not_A Brand\";v=\"24\"".into(),
        ),
        sec_ch_ua_platform: Some("\"macOS\"".into()),
    }
}

/// Returns a [`TlsProfile`] matching Firefox 132.
#[instrument]
pub fn firefox_132_profile() -> TlsProfile {
    debug!("building Firefox 132 profile");
    TlsProfile {
        name: "Firefox 132".into(),
        user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:132.0) \
                      Gecko/20100101 Firefox/132.0"
            .into(),
        ja3_hash: "579ccef312d18482fc42e2b822ca2430".into(),
        ja4_hash: Some("t13d1715h2_5b57614c22b0_3d5424432f57".into()),
        http2_settings: Http2Settings {
            header_table_size: 65536,
            enable_push: false,
            max_concurrent_streams: 100,
            initial_window_size: 131072,
            max_frame_size: 16384,
            max_header_list_size: 65536,
        },
        header_order: vec![
            "Host".into(),
            "User-Agent".into(),
            "Accept".into(),
            "Accept-Language".into(),
            "Accept-Encoding".into(),
            "Connection".into(),
            "Upgrade-Insecure-Requests".into(),
            "Sec-Fetch-Dest".into(),
            "Sec-Fetch-Mode".into(),
            "Sec-Fetch-Site".into(),
            "Sec-Fetch-User".into(),
            "Priority".into(),
        ],
        accept_header: "text/html,application/xhtml+xml,application/xml;\
                         q=0.9,image/avif,image/webp,*/*;q=0.8"
            .into(),
        accept_language: "en-US,en;q=0.5".into(),
        accept_encoding: "gzip, deflate, br, zstd".into(),
        sec_ch_ua: None,
        sec_ch_ua_platform: None,
    }
}

/// Returns a [`TlsProfile`] matching Safari 18 on macOS.
#[instrument]
pub fn safari_18_profile() -> TlsProfile {
    debug!("building Safari 18 profile");
    TlsProfile {
        name: "Safari 18".into(),
        user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 \
                      (KHTML, like Gecko) Version/18.0 Safari/605.1.15"
            .into(),
        ja3_hash: "773906b0efdefa24a7f2b8eb6985bf37".into(),
        ja4_hash: Some("t13d1715h2_5b57614c22b0_06cda9e17597".into()),
        http2_settings: Http2Settings {
            header_table_size: 4096,
            enable_push: false,
            max_concurrent_streams: 100,
            initial_window_size: 2097152,
            max_frame_size: 16384,
            max_header_list_size: 32768,
        },
        header_order: vec![
            "Host".into(),
            "Accept".into(),
            "Accept-Language".into(),
            "Connection".into(),
            "Accept-Encoding".into(),
            "User-Agent".into(),
        ],
        accept_header: "text/html,application/xhtml+xml,application/xml;\
                         q=0.9,*/*;q=0.8"
            .into(),
        accept_language: "en-US,en;q=0.9".into(),
        accept_encoding: "gzip, deflate, br".into(),
        sec_ch_ua: None,
        sec_ch_ua_platform: None,
    }
}

// ---------------------------------------------------------------------------
// Profile lookup helpers
// ---------------------------------------------------------------------------

/// Returns all built-in TLS profiles.
#[instrument]
pub fn all_profiles() -> Vec<TlsProfile> {
    info!("loading all built-in TLS profiles");
    vec![
        chrome_131_profile(),
        chrome_131_mac_profile(),
        firefox_132_profile(),
        safari_18_profile(),
    ]
}

/// Picks a random built-in TLS profile.
///
/// Useful when you want to vary the fingerprint across sessions without
/// caring which specific browser is emulated.
#[instrument]
pub fn random_profile() -> TlsProfile {
    let profiles = all_profiles();
    let mut rng = rand::thread_rng();
    let idx = rng.gen_range(0..profiles.len());
    debug!(selected = %profiles[idx].name, "selected random TLS profile");
    profiles[idx].clone()
}

/// Looks up a built-in TLS profile by name (case-insensitive).
///
/// Returns `None` if no profile matches the given name.
#[instrument(skip_all, fields(name = %name))]
pub fn profile_by_name(name: &str) -> Option<TlsProfile> {
    let lower = name.to_lowercase();
    let result = all_profiles().into_iter().find(|p| p.name.to_lowercase() == lower);
    match &result {
        Some(p) => debug!(profile = %p.name, "found TLS profile by name"),
        None => debug!("no TLS profile found for name"),
    }
    result
}

// ---------------------------------------------------------------------------
// Header generation
// ---------------------------------------------------------------------------

/// Builds the HTTP headers for the given TLS profile in the correct order.
///
/// The returned vector contains `(header_name, value)` pairs ordered exactly
/// as the real browser would send them.  Headers that are not applicable
/// (e.g. `sec-ch-ua` for Firefox/Safari) are omitted.
#[instrument(skip_all, fields(profile = %profile.name))]
pub fn apply_profile_headers(profile: &TlsProfile) -> Vec<(String, String)> {
    let mut header_map: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    header_map.insert("User-Agent".to_lowercase(), profile.user_agent.clone());
    header_map.insert("Accept".to_lowercase(), profile.accept_header.clone());
    header_map.insert(
        "Accept-Language".to_lowercase(),
        profile.accept_language.clone(),
    );
    header_map.insert(
        "Accept-Encoding".to_lowercase(),
        profile.accept_encoding.clone(),
    );

    if let Some(ref val) = profile.sec_ch_ua {
        header_map.insert("sec-ch-ua".to_lowercase(), val.clone());
    }
    if let Some(ref val) = profile.sec_ch_ua_platform {
        header_map.insert("sec-ch-ua-platform".to_lowercase(), val.clone());
    }

    let mut headers: Vec<(String, String)> = Vec::new();
    for key in &profile.header_order {
        let lower = key.to_lowercase();
        if let Some(val) = header_map.get(&lower) {
            headers.push((key.clone(), val.clone()));
        }
    }

    debug!(count = headers.len(), "applied profile headers");
    headers
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_profiles_returns_at_least_four() {
        let profiles = all_profiles();
        assert!(
            profiles.len() >= 4,
            "expected at least 4 profiles, got {}",
            profiles.len()
        );
    }

    #[test]
    fn test_chrome_131_profile_has_valid_fields() {
        let p = chrome_131_profile();
        assert_eq!(p.name, "Chrome 131 Windows");
        assert!(!p.user_agent.is_empty());
        assert!(!p.ja3_hash.is_empty());
        assert!(p.ja4_hash.is_some());
        assert!(p.sec_ch_ua.is_some());
        assert!(p.sec_ch_ua_platform.is_some());
        assert!(!p.header_order.is_empty());
        assert!(!p.accept_header.is_empty());
        assert!(!p.accept_language.is_empty());
        assert!(!p.accept_encoding.is_empty());
        assert_eq!(p.http2_settings.header_table_size, 65536);
        assert!(!p.http2_settings.enable_push);
    }

    #[test]
    fn test_profile_by_name_case_insensitive() {
        let p = profile_by_name("chrome 131 windows");
        assert!(p.is_some());
        assert_eq!(p.unwrap().name, "Chrome 131 Windows");

        let p2 = profile_by_name("CHROME 131 WINDOWS");
        assert!(p2.is_some());

        let p3 = profile_by_name("Chrome 131 Windows");
        assert!(p3.is_some());

        let none = profile_by_name("nonexistent browser");
        assert!(none.is_none());
    }

    #[test]
    fn test_random_profile_returns_valid() {
        let p = random_profile();
        assert!(!p.name.is_empty());
        assert!(!p.user_agent.is_empty());
        assert!(!p.ja3_hash.is_empty());
        assert!(!p.header_order.is_empty());
    }

    #[test]
    fn test_apply_profile_headers_correct_order() {
        let p = chrome_131_profile();
        let headers = apply_profile_headers(&p);

        // Should contain User-Agent, Accept, Accept-Language, Accept-Encoding,
        // sec-ch-ua, sec-ch-ua-platform at minimum.
        assert!(!headers.is_empty());

        // Find positions of known headers to verify ordering.
        let names: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();

        let ua_pos = names.iter().position(|&n| n == "User-Agent");
        let accept_pos = names.iter().position(|&n| n == "Accept");
        assert!(ua_pos.is_some(), "User-Agent header missing");
        assert!(accept_pos.is_some(), "Accept header missing");

        // In Chrome, User-Agent comes before Accept.
        assert!(
            ua_pos.unwrap() < accept_pos.unwrap(),
            "User-Agent should precede Accept in Chrome header order"
        );

        // Verify values are populated.
        for (_, val) in &headers {
            assert!(!val.is_empty(), "header value should not be empty");
        }
    }

    #[test]
    fn test_each_profile_has_nonempty_ja3_hash() {
        for p in all_profiles() {
            assert!(
                !p.ja3_hash.is_empty(),
                "profile '{}' has empty ja3_hash",
                p.name
            );
        }
    }
}
