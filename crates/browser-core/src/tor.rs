//! # Tor Integration
//!
//! Per-profile proxy configuration and Tor circuit management.
//! Works as an interface layer without requiring `arti-client` at compile time,
//! but defines the full API surface for Tor connectivity.
//!
//! ## Architecture
//!
//! ```text
//! BrowserSession ──► TorProxy ──► socks5://127.0.0.1:{port}
//!                       │
//!                       ├── Circuit management (new identity)
//!                       ├── Bridge support (obfs4, meek)
//!                       ├── Exit-node country selection
//!                       └── DNS-over-HTTPS resolver
//! ```
//!
//! ## Status
//!
//! This module defines the interface. Real Tor connectivity requires
//! `arti-client` — add via `--features tor`. Without the feature flag,
//! `new_circuit` returns a mock circuit for testing and development.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn, instrument};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for Tor proxy connectivity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorConfig {
    /// Whether Tor routing is enabled for this profile.
    pub enabled: bool,
    /// SOCKS5 proxy port (default: 9050).
    pub socks_port: u16,
    /// Tor control port for circuit management (default: 9051).
    pub control_port: u16,
    /// Whether to use pluggable transports (bridges) for censorship circumvention.
    pub bridge_mode: bool,
    /// Bridge lines for pluggable transport connections.
    pub bridges: Vec<String>,
    /// Optional list of allowed exit-node country codes (ISO 3166-1 alpha-2).
    pub exit_nodes: Option<Vec<String>>,
    /// Whether to resolve DNS over HTTPS instead of through Tor.
    pub dns_over_https: bool,
    /// DoH resolver URL.
    pub doh_resolver: String,
}

impl Default for TorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            socks_port: 9050,
            control_port: 9051,
            bridge_mode: false,
            bridges: Vec::new(),
            exit_nodes: None,
            dns_over_https: true,
            doh_resolver: "https://cloudflare-dns.com/dns-query".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Circuit
// ---------------------------------------------------------------------------

/// Represents a Tor circuit (entry → middle → exit relay chain).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorCircuit {
    /// Unique circuit identifier.
    pub id: String,
    /// Entry (guard) relay fingerprint or address.
    pub entry_node: String,
    /// Middle relay fingerprint or address.
    pub middle_node: String,
    /// Exit relay fingerprint or address.
    pub exit_node: String,
    /// Country code of the exit relay, if known.
    pub exit_country: Option<String>,
    /// When this circuit was established.
    pub created_at: DateTime<Utc>,
    /// Whether this circuit is currently in use.
    pub is_active: bool,
}

// ---------------------------------------------------------------------------
// Proxy manager
// ---------------------------------------------------------------------------

/// Manages Tor proxy configuration and circuit lifecycle.
pub struct TorProxy {
    /// Active configuration.
    pub config: TorConfig,
    /// All known circuits for this session.
    pub circuits: Vec<TorCircuit>,
    /// ID of the currently active circuit.
    pub active_circuit: Option<String>,
}

impl TorProxy {
    /// Create a new Tor proxy manager with the given configuration.
    #[instrument(skip(config), fields(enabled = config.enabled, port = config.socks_port))]
    pub fn new(config: TorConfig) -> Self {
        info!(
            enabled = config.enabled,
            socks_port = config.socks_port,
            bridge_mode = config.bridge_mode,
            "Initializing TorProxy"
        );
        Self {
            config,
            circuits: Vec::new(),
            active_circuit: None,
        }
    }

    /// Returns the SOCKS5 proxy URL for this Tor instance.
    #[instrument(skip(self))]
    pub fn proxy_url(&self) -> String {
        let url = format!("socks5://127.0.0.1:{}", self.config.socks_port);
        debug!(url = %url, "Generated proxy URL");
        url
    }

    /// Whether Tor routing is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Request a new Tor circuit (new identity).
    ///
    /// In the current stub implementation this creates a mock circuit with
    /// synthetic relay addresses. A real implementation would signal
    /// `arti-client` or send a NEWNYM to the Tor control port.
    #[instrument(skip(self))]
    pub fn new_circuit(&mut self) -> TorCircuit {
        warn!("Creating mock circuit — real Tor requires --features tor or a running Tor daemon");

        // Deactivate any existing active circuit
        if let Some(ref active_id) = self.active_circuit {
            for c in &mut self.circuits {
                if c.id == *active_id {
                    c.is_active = false;
                }
            }
        }

        let circuit_id = Uuid::new_v4().to_string();
        let exit_country = self
            .config
            .exit_nodes
            .as_ref()
            .and_then(|nodes| nodes.first().cloned());

        let circuit = TorCircuit {
            id: circuit_id.clone(),
            entry_node: "mock-guard-relay.example.onion".to_string(),
            middle_node: "mock-middle-relay.example.onion".to_string(),
            exit_node: "mock-exit-relay.example.onion".to_string(),
            exit_country,
            created_at: Utc::now(),
            is_active: true,
        };

        self.active_circuit = Some(circuit_id);
        self.circuits.push(circuit.clone());

        info!(
            circuit_id = %circuit.id,
            exit_country = ?circuit.exit_country,
            "New circuit created"
        );

        circuit
    }

    /// Returns a reference to the currently active circuit, if any.
    #[instrument(skip(self))]
    pub fn active_circuit(&self) -> Option<&TorCircuit> {
        let active_id = self.active_circuit.as_ref()?;
        self.circuits.iter().find(|c| c.id == *active_id && c.is_active)
    }

    /// Chrome launch arguments for routing traffic through Tor.
    #[instrument(skip(self))]
    pub fn chrome_args(&self) -> Vec<String> {
        let proxy = self.proxy_url();
        let args = vec![
            format!("--proxy-server={}", proxy),
            "--host-resolver-rules=MAP * ~NOTFOUND , EXCLUDE 127.0.0.1".to_string(),
            "--no-proxy-server-for-local-network".to_string(),
        ];
        debug!(arg_count = args.len(), "Generated Chrome Tor proxy args");
        args
    }

    /// Returns a `reqwest::Proxy` configured to route through Tor.
    #[instrument(skip(self))]
    pub fn reqwest_proxy(&self) -> Result<reqwest::Proxy, String> {
        let url = self.proxy_url();
        reqwest::Proxy::all(&url).map_err(|e| {
            let msg = format!("Failed to create reqwest proxy for {}: {}", url, e);
            warn!(%msg);
            msg
        })
    }
}

// ---------------------------------------------------------------------------
// DNS-over-HTTPS
// ---------------------------------------------------------------------------

/// Resolves DNS queries over HTTPS to prevent DNS leaks when using Tor.
pub struct DnsOverHttps {
    /// The DoH resolver endpoint URL.
    pub resolver_url: String,
}

impl DnsOverHttps {
    /// Create a new DoH resolver with the given endpoint.
    pub fn new(url: &str) -> Self {
        Self {
            resolver_url: url.to_string(),
        }
    }

    /// Resolve a domain name to IP addresses via DNS-over-HTTPS.
    ///
    /// Sends an HTTP GET request to the resolver using the `application/dns-json`
    /// format (RFC 8484 JSON API).
    #[instrument(fields(domain))]
    pub async fn resolve(domain: &str, resolver_url: &str) -> Result<Vec<String>, String> {
        info!(domain = %domain, resolver = %resolver_url, "Resolving via DoH");

        let url = format!(
            "{}?name={}&type=A",
            resolver_url, domain
        );

        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

        let response = client
            .get(&url)
            .header("Accept", "application/dns-json")
            .send()
            .await
            .map_err(|e| format!("DoH request failed: {}", e))?;

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse DoH response: {}", e))?;

        let answers = body
            .get("Answer")
            .and_then(|a| a.as_array())
            .ok_or_else(|| "No Answer section in DoH response".to_string())?;

        let ips: Vec<String> = answers
            .iter()
            .filter_map(|entry| {
                // type 1 = A record
                let rtype = entry.get("type").and_then(|t| t.as_u64())?;
                if rtype == 1 {
                    entry.get("data").and_then(|d| d.as_str()).map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect();

        debug!(domain = %domain, ip_count = ips.len(), "DoH resolution complete");
        Ok(ips)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_url_format() {
        let config = TorConfig {
            socks_port: 9150,
            ..TorConfig::default()
        };
        let proxy = TorProxy::new(config);
        assert_eq!(proxy.proxy_url(), "socks5://127.0.0.1:9150");
    }

    #[test]
    fn default_proxy_url() {
        let proxy = TorProxy::new(TorConfig::default());
        assert_eq!(proxy.proxy_url(), "socks5://127.0.0.1:9050");
    }

    #[test]
    fn is_enabled_reflects_config() {
        let mut config = TorConfig::default();
        config.enabled = true;
        let proxy = TorProxy::new(config);
        assert!(proxy.is_enabled());

        let proxy_disabled = TorProxy::new(TorConfig::default());
        assert!(!proxy_disabled.is_enabled());
    }

    #[test]
    fn chrome_args_contains_proxy_server() {
        let config = TorConfig::default();
        let proxy = TorProxy::new(config);
        let args = proxy.chrome_args();
        assert!(args.iter().any(|a| a.contains("--proxy-server=socks5://")));
        assert!(args.iter().any(|a| a.contains("--host-resolver-rules")));
    }

    #[test]
    fn new_circuit_creates_valid_circuit() {
        let mut proxy = TorProxy::new(TorConfig::default());
        let circuit = proxy.new_circuit();

        assert!(!circuit.id.is_empty());
        assert!(!circuit.entry_node.is_empty());
        assert!(!circuit.middle_node.is_empty());
        assert!(!circuit.exit_node.is_empty());
        assert!(circuit.is_active);
        assert_eq!(proxy.circuits.len(), 1);
        assert!(proxy.active_circuit().is_some());
    }

    #[test]
    fn new_circuit_deactivates_previous() {
        let mut proxy = TorProxy::new(TorConfig::default());
        let first = proxy.new_circuit();
        let first_id = first.id.clone();

        let second = proxy.new_circuit();
        assert_ne!(first_id, second.id);
        assert_eq!(proxy.circuits.len(), 2);

        // First circuit should be deactivated
        let first_circuit = proxy.circuits.iter().find(|c| c.id == first_id).unwrap();
        assert!(!first_circuit.is_active);

        // Second should be active
        let active = proxy.active_circuit().unwrap();
        assert_eq!(active.id, second.id);
    }

    #[test]
    fn exit_country_from_config() {
        let config = TorConfig {
            exit_nodes: Some(vec!["DE".to_string(), "NL".to_string()]),
            ..TorConfig::default()
        };
        let mut proxy = TorProxy::new(config);
        let circuit = proxy.new_circuit();
        assert_eq!(circuit.exit_country, Some("DE".to_string()));
    }

    #[test]
    fn reqwest_proxy_succeeds() {
        let proxy = TorProxy::new(TorConfig::default());
        let result = proxy.reqwest_proxy();
        assert!(result.is_ok());
    }

    #[test]
    fn doh_resolver_creation() {
        let doh = DnsOverHttps::new("https://dns.google/dns-query");
        assert_eq!(doh.resolver_url, "https://dns.google/dns-query");
    }

    #[test]
    fn default_config_values() {
        let config = TorConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.socks_port, 9050);
        assert_eq!(config.control_port, 9051);
        assert!(!config.bridge_mode);
        assert!(config.bridges.is_empty());
        assert!(config.exit_nodes.is_none());
        assert!(config.dns_over_https);
        assert_eq!(config.doh_resolver, "https://cloudflare-dns.com/dns-query");
    }
}
