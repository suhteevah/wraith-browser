//! Parallel Browser Swarm
//!
//! Manages a pool of isolated browser contexts for parallel browsing tasks.
//! The [`BrowserSwarm`] struct provides concurrency-limited task spawning,
//! while [`SwarmOrchestrator`] offers a higher-level fan-out API for
//! processing lists of URLs in parallel.

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Semaphore};
use tracing::{debug, info, instrument, warn};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for a [`BrowserSwarm`] instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmConfig {
    /// Maximum number of concurrent worker tasks.
    pub max_workers: usize,
    /// Per-worker timeout in seconds.
    pub worker_timeout_secs: u64,
}

impl Default for SwarmConfig {
    fn default() -> Self {
        Self {
            max_workers: 6,
            worker_timeout_secs: 300,
        }
    }
}

// ---------------------------------------------------------------------------
// Task / Result types
// ---------------------------------------------------------------------------

/// A unit of work to be executed by the swarm.
#[derive(Debug, Clone)]
pub struct SwarmTask {
    /// Unique identifier for this task.
    pub id: String,
    /// Human-readable description of what the task does.
    pub description: String,
    /// Optional starting URL for the browser context.
    pub start_url: Option<String>,
}

/// The outcome of a single [`SwarmTask`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmResult {
    /// Identifier of the task that produced this result.
    pub task_id: String,
    /// Whether the task completed successfully.
    pub success: bool,
    /// Arbitrary result payload (e.g. extracted text, status message).
    pub result: String,
    /// Wall-clock duration of the task in milliseconds.
    pub duration_ms: u64,
    /// Number of pages visited during the task.
    pub pages_visited: usize,
}

// ---------------------------------------------------------------------------
// BrowserSwarm
// ---------------------------------------------------------------------------

/// A concurrency-limited pool for running parallel browser tasks.
///
/// Uses a [`Semaphore`] to cap the number of workers that execute
/// simultaneously, and tracks active worker count via an atomic counter.
pub struct BrowserSwarm {
    config: SwarmConfig,
    semaphore: Arc<Semaphore>,
    active_count: Arc<AtomicUsize>,
    results: Arc<Mutex<Vec<SwarmResult>>>,
}

impl BrowserSwarm {
    /// Create a new swarm with the given configuration.
    pub fn new(config: SwarmConfig) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_workers));
        Self {
            config,
            semaphore,
            active_count: Arc::new(AtomicUsize::new(0)),
            results: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns the number of workers currently executing.
    pub fn active_workers(&self) -> usize {
        self.active_count.load(Ordering::SeqCst)
    }

    /// Returns the maximum number of concurrent workers allowed.
    pub fn max_workers(&self) -> usize {
        self.config.max_workers
    }

    /// Spawn a single task on the swarm.
    ///
    /// The returned [`JoinHandle`](tokio::task::JoinHandle) resolves to
    /// the [`SwarmResult`] produced by the work closure.  Concurrency is
    /// bounded by the configured `max_workers` via a semaphore.
    pub async fn spawn_task<F, Fut>(
        &self,
        task: SwarmTask,
        work: F,
    ) -> tokio::task::JoinHandle<SwarmResult>
    where
        F: FnOnce(SwarmTask) -> Fut + Send + 'static,
        Fut: Future<Output = SwarmResult> + Send,
    {
        let semaphore = Arc::clone(&self.semaphore);
        let active_count = Arc::clone(&self.active_count);
        let results = Arc::clone(&self.results);
        let timeout_secs = self.config.worker_timeout_secs;

        info!(task_id = %task.id, "spawning swarm task");

        tokio::spawn(async move {
            // Acquire permit — blocks if we are at capacity.
            let _permit = semaphore
                .acquire()
                .await
                .expect("semaphore closed unexpectedly");

            active_count.fetch_add(1, Ordering::SeqCst);
            debug!(task_id = %task.id, "worker started");

            let task_id = task.id.clone();
            let result = match tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs),
                work(task),
            )
            .await
            {
                Ok(res) => res,
                Err(_) => {
                    warn!(task_id = %task_id, "worker timed out");
                    SwarmResult {
                        task_id: task_id.clone(),
                        success: false,
                        result: "task timed out".to_string(),
                        duration_ms: timeout_secs * 1000,
                        pages_visited: 0,
                    }
                }
            };

            active_count.fetch_sub(1, Ordering::SeqCst);
            debug!(task_id = %result.task_id, success = result.success, "worker finished");

            // Store a copy for later collection.
            results.lock().await.push(result.clone());

            result
        })
    }

    /// Spawn multiple tasks at once.
    ///
    /// Returns a [`Vec`] of join handles in the same order as the input.
    #[instrument(skip(self, tasks), fields(count = tasks.len()))]
    pub async fn spawn_many<F, Fut>(
        &self,
        tasks: Vec<(SwarmTask, F)>,
    ) -> Vec<tokio::task::JoinHandle<SwarmResult>>
    where
        F: FnOnce(SwarmTask) -> Fut + Send + 'static,
        Fut: Future<Output = SwarmResult> + Send,
    {
        let mut handles = Vec::with_capacity(tasks.len());
        for (task, work) in tasks {
            handles.push(self.spawn_task(task, work).await);
        }
        handles
    }

    /// Drain and return all results that have been collected so far.
    #[instrument(skip(self))]
    pub async fn collect_results(&self) -> Vec<SwarmResult> {
        let mut guard = self.results.lock().await;
        std::mem::take(&mut *guard)
    }

    /// Wait for all provided handles to finish and return their results.
    #[instrument(skip(self, handles), fields(count = handles.len()))]
    pub async fn wait_all(
        &self,
        handles: Vec<tokio::task::JoinHandle<SwarmResult>>,
    ) -> Vec<SwarmResult> {
        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            match handle.await {
                Ok(res) => results.push(res),
                Err(e) => {
                    warn!("join error: {e}");
                    results.push(SwarmResult {
                        task_id: "unknown".to_string(),
                        success: false,
                        result: format!("join error: {e}"),
                        duration_ms: 0,
                        pages_visited: 0,
                    });
                }
            }
        }
        results
    }
}

// ---------------------------------------------------------------------------
// SwarmOrchestrator
// ---------------------------------------------------------------------------

/// Higher-level helper that fans a list of URLs out across a [`BrowserSwarm`].
pub struct SwarmOrchestrator;

impl SwarmOrchestrator {
    /// Fan out a list of URLs as parallel swarm tasks.
    ///
    /// For each URL a [`SwarmTask`] is created with the given `description`,
    /// and a simple work closure that records the URL as the result payload.
    /// All tasks are spawned, awaited, and the collected [`SwarmResult`]s are
    /// returned.
    #[instrument(skip(swarm), fields(url_count = urls.len()))]
    pub async fn fan_out_urls(
        swarm: &BrowserSwarm,
        urls: Vec<String>,
        description: &str,
    ) -> Vec<SwarmResult> {
        let tasks: Vec<(SwarmTask, _)> = urls
            .into_iter()
            .enumerate()
            .map(|(i, url)| {
                let task = SwarmTask {
                    id: format!("url-task-{i}"),
                    description: description.to_string(),
                    start_url: Some(url.clone()),
                };
                let work = move |t: SwarmTask| async move {
                    let start = Utc::now();
                    let url = t.start_url.unwrap_or_default();
                    // Simulate minimal work; real implementation would drive a
                    // browser context here.
                    let end = Utc::now();
                    let duration_ms = (end - start).num_milliseconds().max(0) as u64;
                    SwarmResult {
                        task_id: t.id,
                        success: true,
                        result: url,
                        duration_ms,
                        pages_visited: 1,
                    }
                };
                (task, work)
            })
            .collect();

        info!(count = tasks.len(), "fan_out_urls: spawning tasks");
        let handles = swarm.spawn_many(tasks).await;
        swarm.wait_all(handles).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize as StdAtomicUsize;
    use std::time::Duration;

    /// Helper: build a simple work closure that returns a successful result.
    fn ok_work(delay_ms: u64) -> impl FnOnce(SwarmTask) -> std::pin::Pin<Box<dyn Future<Output = SwarmResult> + Send>> + Send + 'static {
        move |task: SwarmTask| {
            Box::pin(async move {
                if delay_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
                SwarmResult {
                    task_id: task.id,
                    success: true,
                    result: "ok".to_string(),
                    duration_ms: delay_ms,
                    pages_visited: 1,
                }
            })
        }
    }

    fn make_task(id: &str) -> SwarmTask {
        SwarmTask {
            id: id.to_string(),
            description: "test task".to_string(),
            start_url: None,
        }
    }

    #[tokio::test]
    async fn new_creates_with_correct_config() {
        let cfg = SwarmConfig {
            max_workers: 4,
            worker_timeout_secs: 120,
        };
        let swarm = BrowserSwarm::new(cfg.clone());
        assert_eq!(swarm.max_workers(), 4);
    }

    #[tokio::test]
    async fn active_workers_starts_at_zero() {
        let swarm = BrowserSwarm::new(SwarmConfig::default());
        assert_eq!(swarm.active_workers(), 0);
    }

    #[tokio::test]
    async fn spawn_task_increments_and_decrements_active_count() {
        let swarm = BrowserSwarm::new(SwarmConfig {
            max_workers: 2,
            worker_timeout_secs: 10,
        });

        let active = Arc::clone(&swarm.active_count);
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let b2 = Arc::clone(&barrier);

        let handle = swarm
            .spawn_task(make_task("t1"), move |task| async move {
                // Signal that we are running.
                b2.wait().await;
                // Give the test a moment to observe the active count.
                tokio::time::sleep(Duration::from_millis(50)).await;
                SwarmResult {
                    task_id: task.id,
                    success: true,
                    result: "done".to_string(),
                    duration_ms: 0,
                    pages_visited: 1,
                }
            })
            .await;

        // Wait until the worker is actually running.
        barrier.wait().await;
        assert!(active.load(Ordering::SeqCst) >= 1);

        // After completion count should return to 0.
        handle.await.unwrap();
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn spawn_many_runs_all_tasks() {
        let swarm = BrowserSwarm::new(SwarmConfig::default());

        let tasks: Vec<_> = (0..5)
            .map(|i| (make_task(&format!("m{i}")), ok_work(0)))
            .collect();

        let handles = swarm.spawn_many(tasks).await;
        assert_eq!(handles.len(), 5);

        let results = swarm.wait_all(handles).await;
        assert_eq!(results.len(), 5);
        assert!(results.iter().all(|r| r.success));
    }

    #[tokio::test]
    async fn semaphore_limits_concurrency() {
        let max = 2usize;
        let swarm = BrowserSwarm::new(SwarmConfig {
            max_workers: max,
            worker_timeout_secs: 30,
        });

        let peak = Arc::new(StdAtomicUsize::new(0));
        let current = Arc::new(StdAtomicUsize::new(0));

        let mut tasks: Vec<(SwarmTask, _)> = Vec::new();
        for i in 0..6 {
            let p = Arc::clone(&peak);
            let c = Arc::clone(&current);
            let work = move |task: SwarmTask| {
                Box::pin(async move {
                    let val = c.fetch_add(1, Ordering::SeqCst) + 1;
                    // Update peak.
                    loop {
                        let old = p.load(Ordering::SeqCst);
                        if val <= old || p.compare_exchange(old, val, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                            break;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    c.fetch_sub(1, Ordering::SeqCst);
                    SwarmResult {
                        task_id: task.id,
                        success: true,
                        result: "ok".to_string(),
                        duration_ms: 50,
                        pages_visited: 1,
                    }
                }) as std::pin::Pin<Box<dyn Future<Output = SwarmResult> + Send>>
            };
            tasks.push((make_task(&format!("c{i}")), work));
        }

        let handles = swarm.spawn_many(tasks).await;
        let results = swarm.wait_all(handles).await;

        assert_eq!(results.len(), 6);
        assert!(peak.load(Ordering::SeqCst) <= max);
    }

    #[tokio::test]
    async fn collect_results_returns_stored_results() {
        let swarm = BrowserSwarm::new(SwarmConfig::default());

        let handle = swarm.spawn_task(make_task("cr1"), ok_work(0)).await;
        handle.await.unwrap();

        let collected = swarm.collect_results().await;
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].task_id, "cr1");

        // Second call should be empty (drained).
        let again = swarm.collect_results().await;
        assert!(again.is_empty());
    }

    #[tokio::test]
    async fn wait_all_returns_all_results() {
        let swarm = BrowserSwarm::new(SwarmConfig::default());

        let mut handles = Vec::new();
        for i in 0..3 {
            handles.push(
                swarm
                    .spawn_task(make_task(&format!("w{i}")), ok_work(10))
                    .await,
            );
        }

        let results = swarm.wait_all(handles).await;
        assert_eq!(results.len(), 3);
        let ids: Vec<&str> = results.iter().map(|r| r.task_id.as_str()).collect();
        assert!(ids.contains(&"w0"));
        assert!(ids.contains(&"w1"));
        assert!(ids.contains(&"w2"));
    }

    #[tokio::test]
    async fn fan_out_urls_returns_results_for_all_urls() {
        let swarm = BrowserSwarm::new(SwarmConfig::default());
        let urls = vec![
            "https://a.example.com".to_string(),
            "https://b.example.com".to_string(),
            "https://c.example.com".to_string(),
        ];

        let results = SwarmOrchestrator::fan_out_urls(&swarm, urls, "test fan-out").await;
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.success));
        let result_urls: Vec<&str> = results.iter().map(|r| r.result.as_str()).collect();
        assert!(result_urls.contains(&"https://a.example.com"));
        assert!(result_urls.contains(&"https://b.example.com"));
        assert!(result_urls.contains(&"https://c.example.com"));
    }
}
