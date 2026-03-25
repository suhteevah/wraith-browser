//! # OpenTelemetry Integration
//!
//! Export traces, metrics, and spans to observability backends such as
//! Grafana, Jaeger, or Zipkin. Provides lightweight in-process metrics
//! collection and span tracking without requiring a full OTel SDK.
//!
//! ## Architecture
//!
//! ```text
//! BrowserSession ──► MetricsCollector ──► BrowsingMetrics (snapshot/JSON)
//!                 ──► SpanTracker ──────► TrackedSpan[] (export JSON)
//!                                            │
//!                                            ├── Otlp  (gRPC :4317)
//!                                            ├── Jaeger (UDP)
//!                                            ├── Zipkin (HTTP)
//!                                            └── Console (stdout)
//! ```
//!
//! ## Status
//!
//! This module provides the data model and in-process collection.
//! Actual export to remote backends requires `opentelemetry` crate
//! integration — add via `--features otel`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for telemetry export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// Whether telemetry collection and export is enabled.
    pub enabled: bool,
    /// Collector endpoint (gRPC for OTLP, HTTP for Zipkin).
    pub endpoint: String,
    /// Service name reported to the collector.
    pub service_name: String,
    /// Sampling rate: 1.0 = sample everything, 0.0 = sample nothing.
    pub sample_rate: f64,
    /// Wire format for exporting spans and metrics.
    pub export_format: ExportFormat,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: "http://localhost:4317".to_string(),
            service_name: "wraith-browser".to_string(),
            sample_rate: 1.0,
            export_format: ExportFormat::Otlp,
        }
    }
}

/// Export wire format for spans and metrics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ExportFormat {
    /// OpenTelemetry Protocol over gRPC.
    Otlp,
    /// Jaeger UDP compact Thrift.
    Jaeger,
    /// Zipkin HTTP JSON v2.
    Zipkin,
    /// Pretty-printed to stdout (for debugging).
    Console,
}

// ---------------------------------------------------------------------------
// Browsing metrics
// ---------------------------------------------------------------------------

/// Aggregated metrics for a browsing session.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowsingMetrics {
    /// Total pages visited.
    pub pages_visited: u64,
    /// Cumulative navigation time in milliseconds.
    pub total_navigation_ms: u64,
    /// Number of cache hits during the session.
    pub cache_hits: u64,
    /// Number of cache misses during the session.
    pub cache_misses: u64,
    /// Number of API endpoints discovered via network interception.
    pub api_calls_discovered: u64,
    /// Total browser actions executed (click, fill, scroll, etc.).
    pub actions_executed: u64,
    /// Errors encountered during the session.
    pub errors_encountered: u64,
    /// Total session duration in milliseconds.
    pub session_duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Metrics collector
// ---------------------------------------------------------------------------

/// Collects browsing metrics throughout a session.
pub struct MetricsCollector {
    /// Current accumulated metrics.
    metrics: BrowsingMetrics,
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsCollector {
    /// Create a new collector with zeroed metrics.
    pub fn new() -> Self {
        Self {
            metrics: BrowsingMetrics::default(),
        }
    }

    /// Record a page navigation with its duration.
    #[instrument(skip(self))]
    pub fn record_navigation(&mut self, duration_ms: u64) {
        self.metrics.pages_visited += 1;
        self.metrics.total_navigation_ms += duration_ms;
        debug!(
            pages = self.metrics.pages_visited,
            duration_ms,
            "Navigation recorded"
        );
    }

    /// Record a cache hit.
    #[instrument(skip(self))]
    pub fn record_cache_hit(&mut self) {
        self.metrics.cache_hits += 1;
        debug!(hits = self.metrics.cache_hits, "Cache hit recorded");
    }

    /// Record a cache miss.
    #[instrument(skip(self))]
    pub fn record_cache_miss(&mut self) {
        self.metrics.cache_misses += 1;
        debug!(misses = self.metrics.cache_misses, "Cache miss recorded");
    }

    /// Record a browser action execution.
    #[instrument(skip(self))]
    pub fn record_action(&mut self) {
        self.metrics.actions_executed += 1;
        debug!(actions = self.metrics.actions_executed, "Action recorded");
    }

    /// Record an error.
    #[instrument(skip(self))]
    pub fn record_error(&mut self) {
        self.metrics.errors_encountered += 1;
        debug!(errors = self.metrics.errors_encountered, "Error recorded");
    }

    /// Record an API endpoint discovery.
    #[instrument(skip(self))]
    pub fn record_api_discovery(&mut self) {
        self.metrics.api_calls_discovered += 1;
        debug!(
            apis = self.metrics.api_calls_discovered,
            "API discovery recorded"
        );
    }

    /// Return a snapshot of the current metrics.
    #[instrument(skip(self))]
    pub fn snapshot(&self) -> BrowsingMetrics {
        self.metrics.clone()
    }

    /// Reset all metrics to zero.
    #[instrument(skip(self))]
    pub fn reset(&mut self) {
        info!("Resetting metrics collector");
        self.metrics = BrowsingMetrics::default();
    }

    /// Serialize current metrics to JSON.
    #[instrument(skip(self))]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.metrics).unwrap_or_else(|e| {
            format!("{{\"error\": \"serialization failed: {}\"}}", e)
        })
    }
}

// ---------------------------------------------------------------------------
// Span tracking
// ---------------------------------------------------------------------------

/// A lightweight span for tracing operations without requiring a full OTel SDK.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedSpan {
    /// Monotonically increasing span ID within this tracker.
    pub id: usize,
    /// Human-readable operation name.
    pub name: String,
    /// Key-value attributes attached to this span.
    pub attributes: Vec<(String, String)>,
    /// When the span was started.
    pub started_at: DateTime<Utc>,
    /// When the span was ended (`None` if still active).
    pub ended_at: Option<DateTime<Utc>>,
    /// Duration in milliseconds (`None` if still active).
    pub duration_ms: Option<u64>,
}

/// Tracks spans for a browsing session, providing structured tracing data
/// without a full OpenTelemetry dependency.
pub struct SpanTracker {
    /// All spans, both active and completed.
    spans: Vec<TrackedSpan>,
    /// Next span ID to assign.
    next_id: usize,
}

impl Default for SpanTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl SpanTracker {
    /// Create a new empty span tracker.
    pub fn new() -> Self {
        Self {
            spans: Vec::new(),
            next_id: 0,
        }
    }

    /// Start a new span with the given name and attributes.
    ///
    /// Returns the span ID, which should be passed to [`end_span`] when the
    /// operation completes.
    #[instrument(skip(self, attributes), fields(span_name = %name))]
    pub fn start_span(&mut self, name: &str, attributes: Vec<(String, String)>) -> usize {
        let id = self.next_id;
        self.next_id += 1;

        let span = TrackedSpan {
            id,
            name: name.to_string(),
            attributes,
            started_at: Utc::now(),
            ended_at: None,
            duration_ms: None,
        };

        debug!(span_id = id, name = %span.name, "Span started");
        self.spans.push(span);
        id
    }

    /// End an active span by its ID, recording the completion time and duration.
    #[instrument(skip(self))]
    pub fn end_span(&mut self, id: usize) {
        if let Some(span) = self.spans.iter_mut().find(|s| s.id == id) {
            let now = Utc::now();
            let duration = (now - span.started_at).num_milliseconds().max(0) as u64;
            span.ended_at = Some(now);
            span.duration_ms = Some(duration);
            debug!(span_id = id, duration_ms = duration, "Span ended");
        }
    }

    /// Returns references to all currently active (un-ended) spans.
    pub fn active_spans(&self) -> Vec<&TrackedSpan> {
        self.spans.iter().filter(|s| s.ended_at.is_none()).collect()
    }

    /// Returns references to all completed spans.
    pub fn completed_spans(&self) -> Vec<&TrackedSpan> {
        self.spans.iter().filter(|s| s.ended_at.is_some()).collect()
    }

    /// Export all spans as a JSON array.
    #[instrument(skip(self))]
    pub fn export_json(&self) -> String {
        serde_json::to_string_pretty(&self.spans).unwrap_or_else(|e| {
            format!("{{\"error\": \"serialization failed: {}\"}}", e)
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- MetricsCollector --

    #[test]
    fn metrics_collector_increments() {
        let mut mc = MetricsCollector::new();
        mc.record_navigation(150);
        mc.record_navigation(200);
        mc.record_cache_hit();
        mc.record_cache_miss();
        mc.record_action();
        mc.record_action();
        mc.record_action();
        mc.record_error();
        mc.record_api_discovery();

        let snap = mc.snapshot();
        assert_eq!(snap.pages_visited, 2);
        assert_eq!(snap.total_navigation_ms, 350);
        assert_eq!(snap.cache_hits, 1);
        assert_eq!(snap.cache_misses, 1);
        assert_eq!(snap.actions_executed, 3);
        assert_eq!(snap.errors_encountered, 1);
        assert_eq!(snap.api_calls_discovered, 1);
    }

    #[test]
    fn metrics_snapshot_returns_current() {
        let mut mc = MetricsCollector::new();
        mc.record_navigation(100);
        let snap1 = mc.snapshot();
        assert_eq!(snap1.pages_visited, 1);

        mc.record_navigation(200);
        let snap2 = mc.snapshot();
        assert_eq!(snap2.pages_visited, 2);

        // snap1 is a clone, not affected by later mutations
        assert_eq!(snap1.pages_visited, 1);
    }

    #[test]
    fn metrics_reset_zeroes() {
        let mut mc = MetricsCollector::new();
        mc.record_navigation(500);
        mc.record_cache_hit();
        mc.record_error();
        mc.reset();

        let snap = mc.snapshot();
        assert_eq!(snap.pages_visited, 0);
        assert_eq!(snap.total_navigation_ms, 0);
        assert_eq!(snap.cache_hits, 0);
        assert_eq!(snap.errors_encountered, 0);
    }

    #[test]
    fn metrics_to_json() {
        let mut mc = MetricsCollector::new();
        mc.record_navigation(42);
        let json = mc.to_json();
        assert!(json.contains("\"pages_visited\": 1"));
        assert!(json.contains("\"total_navigation_ms\": 42"));
    }

    // -- SpanTracker --

    #[test]
    fn span_start_and_end() {
        let mut tracker = SpanTracker::new();
        let id = tracker.start_span("navigate", vec![
            ("url".to_string(), "https://example.com".to_string()),
        ]);

        assert_eq!(tracker.active_spans().len(), 1);
        assert_eq!(tracker.completed_spans().len(), 0);

        tracker.end_span(id);

        assert_eq!(tracker.active_spans().len(), 0);
        assert_eq!(tracker.completed_spans().len(), 1);

        let completed = &tracker.completed_spans()[0];
        assert_eq!(completed.name, "navigate");
        assert!(completed.duration_ms.is_some());
        assert!(completed.ended_at.is_some());
    }

    #[test]
    fn span_ids_are_sequential() {
        let mut tracker = SpanTracker::new();
        let id0 = tracker.start_span("a", vec![]);
        let id1 = tracker.start_span("b", vec![]);
        let id2 = tracker.start_span("c", vec![]);
        assert_eq!(id0, 0);
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
    }

    #[test]
    fn span_export_json() {
        let mut tracker = SpanTracker::new();
        let id = tracker.start_span("test-op", vec![
            ("key".to_string(), "value".to_string()),
        ]);
        tracker.end_span(id);

        let json = tracker.export_json();
        assert!(json.contains("test-op"));
        assert!(json.contains("key"));
        assert!(json.contains("value"));
    }

    #[test]
    fn span_multiple_active() {
        let mut tracker = SpanTracker::new();
        tracker.start_span("op1", vec![]);
        tracker.start_span("op2", vec![]);
        tracker.start_span("op3", vec![]);

        assert_eq!(tracker.active_spans().len(), 3);
        assert_eq!(tracker.completed_spans().len(), 0);
    }

    // -- Config defaults --

    #[test]
    fn telemetry_config_defaults() {
        let config = TelemetryConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.endpoint, "http://localhost:4317");
        assert_eq!(config.service_name, "wraith-browser");
        assert_eq!(config.sample_rate, 1.0);
        assert_eq!(config.export_format, ExportFormat::Otlp);
    }

    #[test]
    fn export_format_variants() {
        let formats = vec![
            ExportFormat::Otlp,
            ExportFormat::Jaeger,
            ExportFormat::Zipkin,
            ExportFormat::Console,
        ];
        // Ensure they are all distinct
        for (i, a) in formats.iter().enumerate() {
            for (j, b) in formats.iter().enumerate() {
                if i == j {
                    assert_eq!(a, b);
                } else {
                    assert_ne!(a, b);
                }
            }
        }
    }
}
