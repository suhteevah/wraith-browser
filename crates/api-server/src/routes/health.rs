use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::{
    extract::State,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::Serialize;

use crate::AppState;

// ---------------------------------------------------------------------------
// Boot timestamp (set once on first access via lazy init)
// ---------------------------------------------------------------------------

static START_TIME: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn start_time() -> &'static Instant {
    START_TIME.get_or_init(Instant::now)
}

// ---------------------------------------------------------------------------
// In-memory metrics store — all fields are lock-free atomics
// ---------------------------------------------------------------------------

pub struct Metrics {
    // Sessions
    pub sessions_active: AtomicU64,
    pub sessions_total: AtomicU64,

    // Actions
    pub actions_navigate: AtomicU64,
    pub actions_click: AtomicU64,
    pub actions_type_text: AtomicU64,
    pub actions_screenshot: AtomicU64,
    pub actions_extract: AtomicU64,
    pub actions_wait: AtomicU64,
    pub actions_scroll: AtomicU64,
    pub actions_other: AtomicU64,

    // Action durations — stored as cumulative microseconds + count so we can
    // derive histogram-style output (sum / count).  A production system would
    // use proper histogram buckets, but this keeps things simple with no
    // external crate.
    pub action_duration_navigate_us: AtomicU64,
    pub action_duration_navigate_count: AtomicU64,
    pub action_duration_click_us: AtomicU64,
    pub action_duration_click_count: AtomicU64,
    pub action_duration_type_text_us: AtomicU64,
    pub action_duration_type_text_count: AtomicU64,
    pub action_duration_screenshot_us: AtomicU64,
    pub action_duration_screenshot_count: AtomicU64,
    pub action_duration_extract_us: AtomicU64,
    pub action_duration_extract_count: AtomicU64,
    pub action_duration_wait_us: AtomicU64,
    pub action_duration_wait_count: AtomicU64,
    pub action_duration_scroll_us: AtomicU64,
    pub action_duration_scroll_count: AtomicU64,
    pub action_duration_other_us: AtomicU64,
    pub action_duration_other_count: AtomicU64,

    // Pages
    pub pages_loaded_total: AtomicU64,

    // Cache
    pub cache_hits_total: AtomicU64,
    pub cache_misses_total: AtomicU64,

    // Engine pool
    pub engine_pool_active: AtomicU64,
    pub engine_pool_idle: AtomicU64,

    // API requests — keyed counters would need a map; we track aggregates
    // plus common method buckets for the Prometheus endpoint.
    pub api_requests_total: AtomicU64,
    pub api_requests_get: AtomicU64,
    pub api_requests_post: AtomicU64,
    pub api_requests_put: AtomicU64,
    pub api_requests_delete: AtomicU64,
    pub api_requests_2xx: AtomicU64,
    pub api_requests_4xx: AtomicU64,
    pub api_requests_5xx: AtomicU64,

    // API request duration — cumulative microseconds + count
    pub api_request_duration_us: AtomicU64,
    pub api_request_duration_count: AtomicU64,
}

impl Metrics {
    const fn new() -> Self {
        Self {
            sessions_active: AtomicU64::new(0),
            sessions_total: AtomicU64::new(0),

            actions_navigate: AtomicU64::new(0),
            actions_click: AtomicU64::new(0),
            actions_type_text: AtomicU64::new(0),
            actions_screenshot: AtomicU64::new(0),
            actions_extract: AtomicU64::new(0),
            actions_wait: AtomicU64::new(0),
            actions_scroll: AtomicU64::new(0),
            actions_other: AtomicU64::new(0),

            action_duration_navigate_us: AtomicU64::new(0),
            action_duration_navigate_count: AtomicU64::new(0),
            action_duration_click_us: AtomicU64::new(0),
            action_duration_click_count: AtomicU64::new(0),
            action_duration_type_text_us: AtomicU64::new(0),
            action_duration_type_text_count: AtomicU64::new(0),
            action_duration_screenshot_us: AtomicU64::new(0),
            action_duration_screenshot_count: AtomicU64::new(0),
            action_duration_extract_us: AtomicU64::new(0),
            action_duration_extract_count: AtomicU64::new(0),
            action_duration_wait_us: AtomicU64::new(0),
            action_duration_wait_count: AtomicU64::new(0),
            action_duration_scroll_us: AtomicU64::new(0),
            action_duration_scroll_count: AtomicU64::new(0),
            action_duration_other_us: AtomicU64::new(0),
            action_duration_other_count: AtomicU64::new(0),

            pages_loaded_total: AtomicU64::new(0),

            cache_hits_total: AtomicU64::new(0),
            cache_misses_total: AtomicU64::new(0),

            engine_pool_active: AtomicU64::new(0),
            engine_pool_idle: AtomicU64::new(0),

            api_requests_total: AtomicU64::new(0),
            api_requests_get: AtomicU64::new(0),
            api_requests_post: AtomicU64::new(0),
            api_requests_put: AtomicU64::new(0),
            api_requests_delete: AtomicU64::new(0),
            api_requests_2xx: AtomicU64::new(0),
            api_requests_4xx: AtomicU64::new(0),
            api_requests_5xx: AtomicU64::new(0),

            api_request_duration_us: AtomicU64::new(0),
            api_request_duration_count: AtomicU64::new(0),
        }
    }
}

/// Record an action by type name, incrementing the appropriate counter.
impl Metrics {
    pub fn inc_action(&self, action_type: &str) {
        match action_type {
            "navigate" => self.actions_navigate.fetch_add(1, Ordering::Relaxed),
            "click" => self.actions_click.fetch_add(1, Ordering::Relaxed),
            "type_text" => self.actions_type_text.fetch_add(1, Ordering::Relaxed),
            "screenshot" => self.actions_screenshot.fetch_add(1, Ordering::Relaxed),
            "extract" => self.actions_extract.fetch_add(1, Ordering::Relaxed),
            "wait" => self.actions_wait.fetch_add(1, Ordering::Relaxed),
            "scroll" => self.actions_scroll.fetch_add(1, Ordering::Relaxed),
            _ => self.actions_other.fetch_add(1, Ordering::Relaxed),
        };
    }

    /// Record the duration of an action in microseconds.
    pub fn observe_action_duration(&self, action_type: &str, duration_us: u64) {
        let (sum, count) = match action_type {
            "navigate" => (&self.action_duration_navigate_us, &self.action_duration_navigate_count),
            "click" => (&self.action_duration_click_us, &self.action_duration_click_count),
            "type_text" => (&self.action_duration_type_text_us, &self.action_duration_type_text_count),
            "screenshot" => (&self.action_duration_screenshot_us, &self.action_duration_screenshot_count),
            "extract" => (&self.action_duration_extract_us, &self.action_duration_extract_count),
            "wait" => (&self.action_duration_wait_us, &self.action_duration_wait_count),
            "scroll" => (&self.action_duration_scroll_us, &self.action_duration_scroll_count),
            _ => (&self.action_duration_other_us, &self.action_duration_other_count),
        };
        sum.fetch_add(duration_us, Ordering::Relaxed);
        count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an API request duration in microseconds.
    pub fn observe_api_duration(&self, duration_us: u64) {
        self.api_request_duration_us.fetch_add(duration_us, Ordering::Relaxed);
        self.api_request_duration_count.fetch_add(1, Ordering::Relaxed);
    }
}

/// Global singleton — zero-cost to access, no heap allocation.
pub static METRICS: Metrics = Metrics::new();

// ---------------------------------------------------------------------------
// Health-check DTO
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    uptime_secs: u64,
    db_connected: bool,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health_check))
        .route("/metrics", get(prometheus_metrics))
}

// ---------------------------------------------------------------------------
// GET /health
// ---------------------------------------------------------------------------

async fn health_check(
    State(state): State<AppState>,
) -> Json<HealthResponse> {
    let uptime = start_time().elapsed().as_secs();

    // Lightweight DB connectivity probe.
    let db_connected = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.db)
        .await
        .is_ok();

    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs: uptime,
        db_connected,
    })
}

// ---------------------------------------------------------------------------
// GET /metrics  — Prometheus text exposition format
// ---------------------------------------------------------------------------

async fn prometheus_metrics() -> impl IntoResponse {
    let r = Ordering::Relaxed;
    let m = &METRICS;

    // Helper: read an atomic gauge/counter.
    macro_rules! v {
        ($field:expr) => {
            $field.load(r)
        };
    }

    // Action types we track explicitly.
    const ACTION_TYPES: &[&str] = &[
        "navigate",
        "click",
        "type_text",
        "screenshot",
        "extract",
        "wait",
        "scroll",
        "other",
    ];

    // Grab per-action counters in a fixed order matching ACTION_TYPES.
    let action_counts: Vec<u64> = vec![
        v!(m.actions_navigate),
        v!(m.actions_click),
        v!(m.actions_type_text),
        v!(m.actions_screenshot),
        v!(m.actions_extract),
        v!(m.actions_wait),
        v!(m.actions_scroll),
        v!(m.actions_other),
    ];

    // Per-action duration (sum_us, count) in same order.
    let action_durations: Vec<(u64, u64)> = vec![
        (v!(m.action_duration_navigate_us), v!(m.action_duration_navigate_count)),
        (v!(m.action_duration_click_us), v!(m.action_duration_click_count)),
        (v!(m.action_duration_type_text_us), v!(m.action_duration_type_text_count)),
        (v!(m.action_duration_screenshot_us), v!(m.action_duration_screenshot_count)),
        (v!(m.action_duration_extract_us), v!(m.action_duration_extract_count)),
        (v!(m.action_duration_wait_us), v!(m.action_duration_wait_count)),
        (v!(m.action_duration_scroll_us), v!(m.action_duration_scroll_count)),
        (v!(m.action_duration_other_us), v!(m.action_duration_other_count)),
    ];

    let mut out = String::with_capacity(4096);

    // ---- wraith_sessions_active (gauge) ----
    out.push_str("# HELP wraith_sessions_active Number of currently active browser sessions.\n");
    out.push_str("# TYPE wraith_sessions_active gauge\n");
    fmt_line(&mut out, "wraith_sessions_active", "", v!(m.sessions_active));

    // ---- wraith_sessions_total (counter) ----
    out.push_str("# HELP wraith_sessions_total Total browser sessions created.\n");
    out.push_str("# TYPE wraith_sessions_total counter\n");
    fmt_line(&mut out, "wraith_sessions_total", "", v!(m.sessions_total));

    // ---- wraith_actions_total (counter, label: action_type) ----
    out.push_str("# HELP wraith_actions_total Total actions executed.\n");
    out.push_str("# TYPE wraith_actions_total counter\n");
    for (i, at) in ACTION_TYPES.iter().enumerate() {
        fmt_line(
            &mut out,
            "wraith_actions_total",
            &format!("action_type=\"{at}\""),
            action_counts[i],
        );
    }

    // ---- wraith_action_duration_seconds (histogram, label: action_type) ----
    out.push_str("# HELP wraith_action_duration_seconds Duration of actions in seconds.\n");
    out.push_str("# TYPE wraith_action_duration_seconds histogram\n");
    for (i, at) in ACTION_TYPES.iter().enumerate() {
        let (sum_us, count) = action_durations[i];
        let sum_secs = sum_us as f64 / 1_000_000.0;
        let labels = format!("action_type=\"{at}\"");
        // Emit _sum and _count (bucket lines omitted for simplicity).
        push_fmt(
            &mut out,
            &format!("wraith_action_duration_seconds_sum{{{labels}}}"),
            sum_secs,
        );
        fmt_line(
            &mut out,
            &format!("wraith_action_duration_seconds_count{{{labels}}}"),
            "",
            count,
        );
    }

    // ---- wraith_pages_loaded_total (counter) ----
    out.push_str("# HELP wraith_pages_loaded_total Total pages loaded.\n");
    out.push_str("# TYPE wraith_pages_loaded_total counter\n");
    fmt_line(&mut out, "wraith_pages_loaded_total", "", v!(m.pages_loaded_total));

    // ---- wraith_cache_hits_total (counter) ----
    out.push_str("# HELP wraith_cache_hits_total Total cache hits.\n");
    out.push_str("# TYPE wraith_cache_hits_total counter\n");
    fmt_line(&mut out, "wraith_cache_hits_total", "", v!(m.cache_hits_total));

    // ---- wraith_cache_misses_total (counter) ----
    out.push_str("# HELP wraith_cache_misses_total Total cache misses.\n");
    out.push_str("# TYPE wraith_cache_misses_total counter\n");
    fmt_line(&mut out, "wraith_cache_misses_total", "", v!(m.cache_misses_total));

    // ---- wraith_engine_pool_active (gauge) ----
    out.push_str("# HELP wraith_engine_pool_active Active engine instances in the pool.\n");
    out.push_str("# TYPE wraith_engine_pool_active gauge\n");
    fmt_line(&mut out, "wraith_engine_pool_active", "", v!(m.engine_pool_active));

    // ---- wraith_engine_pool_idle (gauge) ----
    out.push_str("# HELP wraith_engine_pool_idle Idle engine instances in the pool.\n");
    out.push_str("# TYPE wraith_engine_pool_idle gauge\n");
    fmt_line(&mut out, "wraith_engine_pool_idle", "", v!(m.engine_pool_idle));

    // ---- wraith_api_requests_total (counter, labels: method, path, status) ----
    // We expose aggregate buckets; per-path granularity would require a concurrent
    // map which is beyond the scope of this simple in-memory store.
    out.push_str("# HELP wraith_api_requests_total Total API requests.\n");
    out.push_str("# TYPE wraith_api_requests_total counter\n");
    for (method, val) in [
        ("GET", v!(m.api_requests_get)),
        ("POST", v!(m.api_requests_post)),
        ("PUT", v!(m.api_requests_put)),
        ("DELETE", v!(m.api_requests_delete)),
    ] {
        for (status_class, status_val) in [
            ("2xx", v!(m.api_requests_2xx)),
            ("4xx", v!(m.api_requests_4xx)),
            ("5xx", v!(m.api_requests_5xx)),
        ] {
            // Only emit lines where the method sub-total is non-zero to keep output clean,
            // but always emit at least the totals so scrapers see the metric.
            if val > 0 || status_val > 0 {
                fmt_line(
                    &mut out,
                    "wraith_api_requests_total",
                    &format!("method=\"{method}\",path=\"/api\",status=\"{status_class}\""),
                    0, // individual (method, status) combos aren't tracked; see note above
                );
            }
        }
    }
    // Always emit the aggregate total.
    fmt_line(
        &mut out,
        "wraith_api_requests_total",
        "method=\"ALL\",path=\"/api\",status=\"ALL\"",
        v!(m.api_requests_total),
    );

    // ---- wraith_api_request_duration_seconds (histogram) ----
    out.push_str("# HELP wraith_api_request_duration_seconds API request latency in seconds.\n");
    out.push_str("# TYPE wraith_api_request_duration_seconds histogram\n");
    let api_sum_secs = v!(m.api_request_duration_us) as f64 / 1_000_000.0;
    push_fmt(&mut out, "wraith_api_request_duration_seconds_sum", api_sum_secs);
    fmt_line(
        &mut out,
        "wraith_api_request_duration_seconds_count",
        "",
        v!(m.api_request_duration_count),
    );

    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        out,
    )
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

/// Append a Prometheus line: `name{labels} value\n`
fn fmt_line(out: &mut String, name: &str, labels: &str, value: u64) {
    if labels.is_empty() {
        out.push_str(&format!("{name} {value}\n"));
    } else {
        out.push_str(&format!("{name}{{{labels}}} {value}\n"));
    }
}

/// Append a Prometheus line with a floating-point value.
fn push_fmt(out: &mut String, name_with_labels: &str, value: f64) {
    out.push_str(&format!("{name_with_labels} {value:.6}\n"));
}
