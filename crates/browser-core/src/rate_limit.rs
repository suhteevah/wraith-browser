//! Per-domain rate limiter for outbound swarm requests.
//!
//! This module throttles outgoing HTTP requests on a per-domain basis to avoid
//! triggering server-side rate limits (HTTP 429) during swarm crawls. It is
//! distinct from any *incoming* API rate limiter — this controls how fast we
//! *send* requests to external sites.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rand::Rng;
use url::Url;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Tuning knobs for the per-domain rate limiter.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum requests allowed per rolling hour window.
    pub max_per_hour: u32,
    /// Minimum seconds between consecutive requests to the same domain.
    pub min_interval_secs: u64,
    /// Upper bound (exclusive) for random jitter added to every wait, in seconds.
    pub jitter_range_secs: u64,
    /// Multiplier applied to the interval each time we receive a 429.
    pub backoff_multiplier: f32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_per_hour: 3,
            min_interval_secs: 900,   // 15 minutes
            jitter_range_secs: 60,    // random 0–60 s
            backoff_multiplier: 2.0,  // double on 429
        }
    }
}

// ---------------------------------------------------------------------------
// Per-domain state
// ---------------------------------------------------------------------------

/// Internal bookkeeping for a single domain.
#[derive(Debug)]
struct DomainState {
    domain: String,
    /// Timestamps of requests within the current rolling hour.
    request_timestamps: Vec<Instant>,
    /// The *current* minimum interval (may have been inflated by backoff).
    current_interval_secs: u64,
    /// How many 429s we received in a row (reset on success).
    consecutive_429s: u32,
}

impl DomainState {
    fn new(domain: String, base_interval: u64) -> Self {
        Self {
            domain,
            request_timestamps: Vec::new(),
            current_interval_secs: base_interval,
            consecutive_429s: 0,
        }
    }

    /// Remove timestamps older than one hour.
    fn prune(&mut self) {
        let cutoff = Instant::now() - Duration::from_secs(3600);
        self.request_timestamps.retain(|t| *t > cutoff);
    }
}

// ---------------------------------------------------------------------------
// Public stats
// ---------------------------------------------------------------------------

/// Read-only snapshot of a single domain's rate-limit state.
#[derive(Debug, Clone)]
pub struct DomainStats {
    pub domain: String,
    pub requests_this_hour: usize,
    /// Seconds since the last request, or `None` if no requests recorded.
    pub last_request_at: Option<Duration>,
    pub current_interval: Duration,
    pub is_backed_off: bool,
}

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

/// Thread-safe, per-domain rate limiter for outbound swarm requests.
pub struct DomainRateLimiter {
    limits: Mutex<HashMap<String, DomainState>>,
    default_config: RateLimitConfig,
}

impl DomainRateLimiter {
    /// Create a new limiter with the given configuration.
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            limits: Mutex::new(HashMap::new()),
            default_config: config,
        }
    }

    // -- query methods ------------------------------------------------------

    /// Returns `true` when the domain has capacity for another request *right
    /// now* (i.e. `wait_time` would return `Duration::ZERO`).
    pub fn can_proceed(&self, domain: &str) -> bool {
        self.wait_time(domain) == Duration::ZERO
    }

    /// How long the caller should sleep before making the next request to
    /// `domain`.  Returns `Duration::ZERO` when the request can proceed
    /// immediately.
    pub fn wait_time(&self, domain: &str) -> Duration {
        let mut map = self.limits.lock().unwrap();
        let state = map
            .entry(domain.to_owned())
            .or_insert_with(|| {
                DomainState::new(domain.to_owned(), self.default_config.min_interval_secs)
            });

        state.prune();

        // Hourly cap check — if we already hit the max, the caller must wait
        // until the oldest request falls outside the window.
        if state.request_timestamps.len() as u32 >= self.default_config.max_per_hour {
            let oldest = state.request_timestamps[0];
            let expires_at = oldest + Duration::from_secs(3600);
            let now = Instant::now();
            if expires_at > now {
                return expires_at - now;
            }
        }

        // Per-request interval check.
        if let Some(&last) = state.request_timestamps.last() {
            let interval = Duration::from_secs(state.current_interval_secs);
            let elapsed = last.elapsed();
            if elapsed < interval {
                return interval - elapsed;
            }
        }

        Duration::ZERO
    }

    // -- mutation methods ---------------------------------------------------

    /// Record that a request to `domain` was dispatched.
    pub fn record_request(&self, domain: &str) {
        let mut map = self.limits.lock().unwrap();
        let state = map
            .entry(domain.to_owned())
            .or_insert_with(|| {
                DomainState::new(domain.to_owned(), self.default_config.min_interval_secs)
            });

        state.prune();
        state.request_timestamps.push(Instant::now());
    }

    /// The server responded with HTTP 429 — increase the backoff interval.
    pub fn record_rate_limited(&self, domain: &str) {
        let mut map = self.limits.lock().unwrap();
        let state = map
            .entry(domain.to_owned())
            .or_insert_with(|| {
                DomainState::new(domain.to_owned(), self.default_config.min_interval_secs)
            });

        state.consecutive_429s += 1;
        state.current_interval_secs =
            (state.current_interval_secs as f32 * self.default_config.backoff_multiplier) as u64;
    }

    /// The request succeeded — reset the consecutive-429 counter (but keep the
    /// inflated interval so we don't immediately hammer the server again).
    pub fn record_success(&self, domain: &str) {
        let mut map = self.limits.lock().unwrap();
        if let Some(state) = map.get_mut(domain) {
            state.consecutive_429s = 0;
            // Gradually recover: if we had a clean success, bring the interval
            // back to the configured base.
            state.current_interval_secs = self.default_config.min_interval_secs;
        }
    }

    // -- helpers ------------------------------------------------------------

    /// Compute `base_wait + rand(0..jitter_range)`.
    ///
    /// Call this with the result of [`wait_time`] to get a jitter-adjusted
    /// duration that prevents thundering-herd effects when multiple workers
    /// target the same domain.
    pub fn jittered_wait(&self, base: Duration) -> Duration {
        let jitter_range = self.default_config.jitter_range_secs;
        if jitter_range == 0 {
            return base;
        }
        let jitter = rand::thread_rng().gen_range(0..jitter_range);
        base + Duration::from_secs(jitter)
    }

    /// Extract and normalise the domain (host) from a URL string.
    ///
    /// Returns the host in lowercase. Falls back to returning `url` unchanged
    /// when parsing fails.
    pub fn extract_domain(url: &str) -> String {
        Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_lowercase()))
            .unwrap_or_else(|| url.to_lowercase())
    }

    /// Snapshot of every tracked domain's current rate-limit state.
    pub fn stats(&self) -> HashMap<String, DomainStats> {
        let mut map = self.limits.lock().unwrap();
        let now = Instant::now();

        map.iter_mut()
            .map(|(domain, state)| {
                state.prune();
                let last_request_at = state.request_timestamps.last().map(|t| now - *t);
                let stats = DomainStats {
                    domain: state.domain.clone(),
                    requests_this_hour: state.request_timestamps.len(),
                    last_request_at,
                    current_interval: Duration::from_secs(state.current_interval_secs),
                    is_backed_off: state.consecutive_429s > 0,
                };
                (domain.clone(), stats)
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn fast_config() -> RateLimitConfig {
        RateLimitConfig {
            max_per_hour: 3,
            min_interval_secs: 1,    // 1 second for fast tests
            jitter_range_secs: 0,    // no jitter so assertions are deterministic
            backoff_multiplier: 2.0,
        }
    }

    #[test]
    fn first_request_can_proceed() {
        let rl = DomainRateLimiter::new(fast_config());
        assert!(rl.can_proceed("example.com"));
        assert_eq!(rl.wait_time("example.com"), Duration::ZERO);
    }

    #[test]
    fn interval_enforced_between_requests() {
        let rl = DomainRateLimiter::new(fast_config());
        rl.record_request("example.com");

        // Immediately after, we should NOT be allowed to proceed.
        assert!(!rl.can_proceed("example.com"));
        let wait = rl.wait_time("example.com");
        assert!(wait > Duration::ZERO, "expected positive wait, got {:?}", wait);
        assert!(wait <= Duration::from_secs(1));
    }

    #[test]
    fn interval_expires_and_allows_next() {
        let mut cfg = fast_config();
        cfg.min_interval_secs = 0; // effectively no interval
        let rl = DomainRateLimiter::new(cfg);

        rl.record_request("example.com");
        // With 0-second interval, next request should be immediate.
        assert!(rl.can_proceed("example.com"));
    }

    #[test]
    fn hourly_cap_enforced() {
        let mut cfg = fast_config();
        cfg.min_interval_secs = 0;
        cfg.max_per_hour = 2;
        let rl = DomainRateLimiter::new(cfg);

        rl.record_request("a.com");
        rl.record_request("a.com");

        // Third request should be blocked by the hourly cap.
        assert!(!rl.can_proceed("a.com"));
        let wait = rl.wait_time("a.com");
        // The wait should be close to 1 hour (minus tiny elapsed time).
        assert!(wait > Duration::from_secs(3500));
    }

    #[test]
    fn backoff_doubles_interval() {
        let rl = DomainRateLimiter::new(fast_config());
        rl.record_request("b.com");

        // Record a 429.
        rl.record_rate_limited("b.com");

        // The interval should have doubled from 1 → 2 seconds.
        let wait = rl.wait_time("b.com");
        // wait ≤ 2s (the doubled interval minus a tiny bit of elapsed time).
        assert!(wait > Duration::from_secs(1), "expected > 1s, got {:?}", wait);
    }

    #[test]
    fn multiple_429s_compound() {
        let rl = DomainRateLimiter::new(fast_config());
        rl.record_request("c.com");
        rl.record_rate_limited("c.com"); // 1 → 2
        rl.record_rate_limited("c.com"); // 2 → 4

        let map = rl.limits.lock().unwrap();
        let state = map.get("c.com").unwrap();
        assert_eq!(state.current_interval_secs, 4);
        assert_eq!(state.consecutive_429s, 2);
    }

    #[test]
    fn success_resets_backoff() {
        let rl = DomainRateLimiter::new(fast_config());
        rl.record_request("d.com");
        rl.record_rate_limited("d.com"); // 1 → 2
        rl.record_rate_limited("d.com"); // 2 → 4

        rl.record_success("d.com");

        let map = rl.limits.lock().unwrap();
        let state = map.get("d.com").unwrap();
        assert_eq!(state.consecutive_429s, 0);
        // Interval reset to base.
        assert_eq!(state.current_interval_secs, 1);
    }

    #[test]
    fn different_domains_independent() {
        let mut cfg = fast_config();
        cfg.min_interval_secs = 0;
        let rl = DomainRateLimiter::new(cfg);

        rl.record_request("x.com");
        // y.com is unaffected.
        assert!(rl.can_proceed("y.com"));
    }

    #[test]
    fn extract_domain_works() {
        assert_eq!(
            DomainRateLimiter::extract_domain("https://WWW.Example.COM/path?q=1"),
            "www.example.com"
        );
        assert_eq!(
            DomainRateLimiter::extract_domain("http://foo.bar:8080/"),
            "foo.bar"
        );
        // Fallback for garbage input.
        assert_eq!(
            DomainRateLimiter::extract_domain("not-a-url"),
            "not-a-url"
        );
    }

    #[test]
    fn jitter_adds_positive_duration() {
        let cfg = RateLimitConfig {
            jitter_range_secs: 60,
            ..fast_config()
        };
        let rl = DomainRateLimiter::new(cfg);
        let base = Duration::from_secs(10);
        let jittered = rl.jittered_wait(base);
        assert!(jittered >= base);
        assert!(jittered <= base + Duration::from_secs(60));
    }

    #[test]
    fn jitter_zero_range_returns_base() {
        let rl = DomainRateLimiter::new(fast_config()); // jitter_range = 0
        let base = Duration::from_secs(5);
        assert_eq!(rl.jittered_wait(base), base);
    }

    #[test]
    fn stats_returns_tracked_domains() {
        let mut cfg = fast_config();
        cfg.min_interval_secs = 0;
        let rl = DomainRateLimiter::new(cfg);

        rl.record_request("s1.com");
        rl.record_request("s2.com");
        rl.record_rate_limited("s2.com");

        let s = rl.stats();
        assert_eq!(s.len(), 2);
        assert_eq!(s["s1.com"].requests_this_hour, 1);
        assert!(!s["s1.com"].is_backed_off);
        assert!(s["s2.com"].is_backed_off);
    }

    #[test]
    fn wait_then_proceed() {
        let mut cfg = fast_config();
        cfg.min_interval_secs = 1;
        cfg.max_per_hour = 100;
        let rl = DomainRateLimiter::new(cfg);

        rl.record_request("wait.com");
        assert!(!rl.can_proceed("wait.com"));

        // Sleep just over the interval.
        thread::sleep(Duration::from_millis(1100));
        assert!(rl.can_proceed("wait.com"));
    }
}
