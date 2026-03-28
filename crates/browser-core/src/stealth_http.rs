//! # Stealth HTTP Client
//!
//! HTTP client abstraction. Selects between rquest (BoringSSL, broad site
//! compatibility) or reqwest (rustls, standard TLS).
//!
//! When compiled with `--features stealth-tls`, uses rquest with Firefox 136
//! TLS/HTTP2 emulation via BoringSSL — matching real Firefox fingerprints at
//! the TLS level (JA3/JA4, HTTP/2 SETTINGS, cipher suites, extension order).
//! Without the feature, falls back to reqwest.

use tracing::{debug, info, warn};

/// Fetch a URL with stealth TLS (if available) or standard reqwest.
/// Returns (status_code, response_body, final_url).
pub async fn stealth_fetch(
    url: &str,
    user_agent: &str,
    accept_language: &str,
) -> Result<(u16, String, String), String> {
    #[cfg(feature = "stealth-tls")]
    {
        stealth_fetch_rquest(url, user_agent, accept_language).await
    }

    #[cfg(not(feature = "stealth-tls"))]
    {
        standard_fetch_reqwest(url, user_agent, accept_language).await
    }
}

/// Check if stealth TLS is available.
pub fn has_stealth_tls() -> bool {
    cfg!(feature = "stealth-tls")
}

/// Fetch using rquest with Firefox 136 TLS emulation.
///
/// Uses rquest-util's `Emulation::Firefox136` to match real Firefox's TLS
/// fingerprint (JA3/JA4), HTTP/2 SETTINGS, cipher suites, and extension
/// ordering at the BoringSSL level. This is the same technique Camoufox uses —
/// interception at the implementation level rather than header-only spoofing.
#[cfg(feature = "stealth-tls")]
async fn stealth_fetch_rquest(
    url: &str,
    user_agent: &str,
    accept_language: &str,
) -> Result<(u16, String, String), String> {
    use rquest_util::Emulation;

    debug!(url = %url, "Stealth fetch via rquest (BoringSSL + Firefox 136 emulation)");

    // Build client with Firefox 136 TLS/HTTP2 emulation.
    // This configures BoringSSL to emit Firefox's exact:
    //   - TLS ClientHello (cipher suites, extensions, curves, ALPN)
    //   - HTTP/2 SETTINGS frame (window size, max streams, header table size)
    //   - Header ordering and pseudo-header ordering
    // Anti-bot systems (Cloudflare, Akamai, DataDome) that fingerprint TLS
    // will see a genuine Firefox 136 connection.
    let client = rquest::Client::builder()
        .emulation(Emulation::Firefox136)
        .cookie_store(true)
        .build()
        .map_err(|e| format!("rquest client build failed: {e}"))?;

    // Firefox-style headers — no sec-ch-ua (that's Chromium-only),
    // different Accept format, Accept-Language with q=0.5 (Firefox default).
    let response = client
        .get(url)
        .header("User-Agent", user_agent)
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8")
        .header("Accept-Language", accept_language)
        .header("Accept-Encoding", "gzip, deflate, br, zstd")
        .header("Upgrade-Insecure-Requests", "1")
        .header("Sec-Fetch-Dest", "document")
        .header("Sec-Fetch-Mode", "navigate")
        .header("Sec-Fetch-Site", "none")
        .header("Sec-Fetch-User", "?1")
        .header("Priority", "u=0, i")
        .send()
        .await
        .map_err(|e| format!("rquest request failed: {e}"))?;

    let status = response.status().as_u16();
    let final_url = response.url().to_string();
    let body = response.text().await
        .map_err(|e| format!("rquest body read failed: {e}"))?;

    info!(url = %url, status, body_len = body.len(), "Stealth fetch complete (Firefox 136)");
    Ok((status, body, final_url))
}

/// Fetch using standard reqwest (rustls — detectable TLS fingerprint).
#[cfg(not(feature = "stealth-tls"))]
async fn standard_fetch_reqwest(
    url: &str,
    user_agent: &str,
    accept_language: &str,
) -> Result<(u16, String, String), String> {
    warn!(url = %url, "Standard fetch via reqwest (rustls — TLS fingerprint may be flagged by Cloudflare)");

    let client = reqwest::Client::builder()
        .user_agent(user_agent)
        .cookie_store(true)
        .gzip(true)
        .brotli(true)
        .build()
        .map_err(|e| format!("reqwest client build failed: {e}"))?;

    let response = client
        .get(url)
        .header("Accept-Language", accept_language)
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .send()
        .await
        .map_err(|e| format!("reqwest request failed: {e}"))?;

    let status = response.status().as_u16();
    let final_url = response.url().to_string();
    let body = response.text().await
        .map_err(|e| format!("reqwest body read failed: {e}"))?;

    debug!(url = %url, status, body_len = body.len(), "Standard fetch complete");
    Ok((status, body, final_url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stealth_tls_availability() {
        let available = has_stealth_tls();
        println!("Stealth TLS available: {}", available);
    }

    #[tokio::test]
    async fn fetch_example_com() {
        let result = stealth_fetch(
            "https://example.com",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:136.0) Gecko/20100101 Firefox/136.0",
            "en-US,en;q=0.5",
        ).await;

        let (status, body, final_url) = result.expect("fetch should succeed");
        assert_eq!(status, 200);
        assert!(body.contains("Example Domain"));
        assert!(final_url.contains("example.com"));
    }

    #[tokio::test]
    async fn fetch_returns_final_url() {
        let result = stealth_fetch(
            "http://example.com",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:136.0) Gecko/20100101 Firefox/136.0",
            "en-US,en;q=0.5",
        ).await;

        let (status, _body, final_url) = result.expect("fetch should succeed");
        assert_eq!(status, 200);
        assert!(final_url.contains("example.com"));
    }

    #[tokio::test]
    async fn fetch_invalid_url_returns_error() {
        let result = stealth_fetch(
            "https://this-domain-definitely-does-not-exist-xyz123.com",
            "Mozilla/5.0",
            "en-US",
        ).await;
        assert!(result.is_err());
    }
}
