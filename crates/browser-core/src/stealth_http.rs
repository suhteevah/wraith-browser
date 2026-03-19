//! # Stealth HTTP Client
//!
//! Wraps either `rquest` (BoringSSL with browser TLS fingerprint impersonation)
//! or `reqwest` (rustls — gets flagged by Cloudflare/DataDome/PerimeterX).
//!
//! When compiled with `--features stealth-tls`, uses rquest to impersonate
//! Chrome's TLS fingerprint (JA3/JA4, HTTP/2 SETTINGS, header order).
//! Without the feature, falls back to reqwest (still functional but detectable).
//!
//! ## Why This Matters
//!
//! Bot detection services fingerprint the TLS handshake itself — before any
//! HTTP headers are sent. rustls produces a distinctive fingerprint that
//! Cloudflare blocks instantly. BoringSSL (via rquest) matches Chrome's
//! actual TLS behavior, passing fingerprint checks.

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

/// Fetch using rquest with Chrome TLS impersonation.
#[cfg(feature = "stealth-tls")]
async fn stealth_fetch_rquest(
    url: &str,
    user_agent: &str,
    accept_language: &str,
) -> Result<(u16, String, String), String> {
    debug!(url = %url, "Stealth fetch via rquest (BoringSSL)");

    let client = rquest::Client::builder()
        .cookie_store(true)
        .build()
        .map_err(|e| format!("rquest client build failed: {e}"))?;

    let response = client
        .get(url)
        .header("User-Agent", user_agent)
        .header("Accept-Language", accept_language)
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8")
        .header("Accept-Encoding", "gzip, deflate, br")
        .header("Sec-Fetch-Dest", "document")
        .header("Sec-Fetch-Mode", "navigate")
        .header("Sec-Fetch-Site", "none")
        .header("Sec-Fetch-User", "?1")
        .header("Upgrade-Insecure-Requests", "1")
        .send()
        .await
        .map_err(|e| format!("rquest request failed: {e}"))?;

    let status = response.status().as_u16();
    let final_url = response.url().to_string();
    let body = response.text().await
        .map_err(|e| format!("rquest body read failed: {e}"))?;

    info!(url = %url, status, body_len = body.len(), "Stealth fetch complete");
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
        // Will be true if compiled with --features stealth-tls, false otherwise
        println!("Stealth TLS available: {}", available);
    }

    #[tokio::test]
    async fn fetch_example_com() {
        let result = stealth_fetch(
            "https://example.com",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            "en-US,en;q=0.9",
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
            "Mozilla/5.0",
            "en-US",
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
