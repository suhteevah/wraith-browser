//! HTTP transport abstraction for pluggable backends.
//!
//! This crate defines the [`HttpTransport`] trait and its associated types.
//! It has no heavy runtime dependencies beyond `async_trait`, so both
//! `browser-core` and `sevro-headless` can depend on it without cycles.
//!
//! Enable the `reqwest-backend` feature to get [`ReqwestTransport`].

use std::collections::BTreeMap;

/// An HTTP request to be sent.
#[derive(Debug, Clone)]
pub struct TransportRequest {
    pub method: TransportMethod,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Option<Vec<u8>>,
}

/// HTTP methods we use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMethod {
    Get,
    Post,
}

/// An HTTP response received.
#[derive(Debug, Clone)]
pub struct TransportResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    /// The final URL after redirects.
    pub url: String,
    /// Set-Cookie headers (may have multiple values, preserved separately
    /// because `BTreeMap` deduplicates keys).
    pub set_cookie_headers: Vec<String>,
}

/// Transport error.
#[derive(Debug)]
pub enum TransportError {
    ConnectionFailed(String),
    Timeout,
    TlsError(String),
    Other(String),
}

impl core::fmt::Display for TransportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TransportError::ConnectionFailed(msg) => write!(f, "connection failed: {msg}"),
            TransportError::Timeout => write!(f, "request timed out"),
            TransportError::TlsError(msg) => write!(f, "TLS error: {msg}"),
            TransportError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for TransportError {}

/// Pluggable HTTP transport trait.
///
/// The default `std` implementation uses [`ReqwestTransport`] (requires
/// `reqwest-backend` feature). A `no_std` bare-metal implementation can use
/// smoltcp raw TCP sockets with manual HTTP/1.1 framing.
///
/// Object-safe via `#[async_trait]` so it can be stored as `Arc<dyn HttpTransport>`.
#[async_trait::async_trait]
pub trait HttpTransport: Send + Sync {
    /// Execute an HTTP request and return the response.
    async fn execute(
        &self,
        request: TransportRequest,
    ) -> Result<TransportResponse, TransportError>;
}

// ---------------------------------------------------------------------------
// reqwest-based implementation (behind `reqwest-backend` feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "reqwest-backend")]
mod reqwest_impl;

#[cfg(feature = "reqwest-backend")]
pub use reqwest_impl::ReqwestTransport;
