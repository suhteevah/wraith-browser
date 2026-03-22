//! Swarm checkpoint and resume system.
//!
//! Tracks multi-URL crawl runs across workers with SQLite-backed persistence.
//! Supports pause/resume, retry of failed jobs, and progress reporting.

use std::sync::Mutex;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single job within a swarm run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmJob {
    pub job_id: String,
    pub url: String,
    pub status: String,
    pub worker_id: Option<String>,
    pub attempt_count: u32,
    pub current_step: u32,
    pub total_steps: Option<u32>,
}

/// Progress summary for a swarm run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunProgress {
    pub run_id: String,
    pub total: u32,
    pub completed: u32,
    pub failed: u32,
    pub pending: u32,
    pub running: u32,
    pub elapsed_secs: f64,
}

/// SQLite-backed checkpoint store for swarm crawl runs.
pub struct SwarmCheckpoint {
    db: Mutex<rusqlite::Connection>,
}

impl SwarmCheckpoint {
    /// Open (or create) the checkpoint database at `db_path`.
    pub fn new(db_path: &str) -> Self {
        let conn = rusqlite::Connection::open(db_path)
            .expect("failed to open checkpoint database");

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS swarm_runs (
                run_id TEXT PRIMARY KEY,
                playbook_name TEXT,
                started_at TEXT,
                status TEXT DEFAULT 'running',
                total_jobs INTEGER,
                completed INTEGER DEFAULT 0,
                failed INTEGER DEFAULT 0,
                config_json TEXT
            );
            CREATE TABLE IF NOT EXISTS swarm_jobs (
                job_id TEXT PRIMARY KEY,
                run_id TEXT REFERENCES swarm_runs(run_id),
                url TEXT NOT NULL,
                status TEXT DEFAULT 'pending',
                worker_id TEXT,
                attempt_count INTEGER DEFAULT 0,
                last_error TEXT,
                result_json TEXT,
                started_at TEXT,
                completed_at TEXT,
                current_step INTEGER DEFAULT 0,
                total_steps INTEGER
            );
            ",
        )
        .expect("failed to initialise checkpoint schema");

        Self {
            db: Mutex::new(conn),
        }
    }

    /// Create a new swarm run with one job per URL. Returns the `run_id`.
    pub fn create_run(&self, playbook_name: &str, urls: &[String]) -> String {
        let run_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let db = self.db.lock().unwrap();

        db.execute(
            "INSERT INTO swarm_runs (run_id, playbook_name, started_at, total_jobs) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![run_id, playbook_name, now, urls.len() as i64],
        )
        .expect("failed to insert swarm run");

        for url in urls {
            let job_id = Uuid::new_v4().to_string();
            db.execute(
                "INSERT INTO swarm_jobs (job_id, run_id, url) VALUES (?1, ?2, ?3)",
                rusqlite::params![job_id, run_id, url],
            )
            .expect("failed to insert swarm job");
        }

        run_id
    }

    /// Fetch the next pending job for a run, if any.
    pub fn next_pending_job(&self, run_id: &str) -> Option<SwarmJob> {
        let db = self.db.lock().unwrap();
        let mut stmt = db
            .prepare(
                "SELECT job_id, url, status, worker_id, attempt_count, current_step, total_steps
                 FROM swarm_jobs
                 WHERE run_id = ?1 AND status = 'pending'
                 LIMIT 1",
            )
            .expect("failed to prepare next_pending_job query");

        stmt.query_row(rusqlite::params![run_id], |row| {
            Ok(SwarmJob {
                job_id: row.get(0)?,
                url: row.get(1)?,
                status: row.get(2)?,
                worker_id: row.get(3)?,
                attempt_count: row.get::<_, i64>(4)? as u32,
                current_step: row.get::<_, i64>(5)? as u32,
                total_steps: row.get::<_, Option<i64>>(6)?.map(|v| v as u32),
            })
        })
        .ok()
    }

    /// Mark a job as running and assign it to a worker.
    pub fn claim_job(&self, job_id: &str, worker_id: &str) {
        let now = Utc::now().to_rfc3339();
        let db = self.db.lock().unwrap();
        db.execute(
            "UPDATE swarm_jobs SET status = 'running', worker_id = ?1, started_at = ?2 WHERE job_id = ?3",
            rusqlite::params![worker_id, now, job_id],
        )
        .expect("failed to claim job");
    }

    /// Update step progress for a running job.
    pub fn update_progress(&self, job_id: &str, step: u32, total: u32) {
        let db = self.db.lock().unwrap();
        db.execute(
            "UPDATE swarm_jobs SET current_step = ?1, total_steps = ?2 WHERE job_id = ?3",
            rusqlite::params![step as i64, total as i64, job_id],
        )
        .expect("failed to update progress");
    }

    /// Mark a job as completed with the given result JSON.
    pub fn complete_job(&self, job_id: &str, result_json: &str) {
        let now = Utc::now().to_rfc3339();
        let db = self.db.lock().unwrap();

        db.execute(
            "UPDATE swarm_jobs SET status = 'completed', result_json = ?1, completed_at = ?2 WHERE job_id = ?3",
            rusqlite::params![result_json, now, job_id],
        )
        .expect("failed to complete job");

        // Bump the run-level counter.
        db.execute(
            "UPDATE swarm_runs SET completed = completed + 1 WHERE run_id = (SELECT run_id FROM swarm_jobs WHERE job_id = ?1)",
            rusqlite::params![job_id],
        )
        .expect("failed to update run completed count");
    }

    /// Mark a job as failed with an error message and increment its attempt count.
    pub fn fail_job(&self, job_id: &str, error: &str) {
        let now = Utc::now().to_rfc3339();
        let db = self.db.lock().unwrap();

        db.execute(
            "UPDATE swarm_jobs SET status = 'failed', last_error = ?1, completed_at = ?2, attempt_count = attempt_count + 1 WHERE job_id = ?3",
            rusqlite::params![error, now, job_id],
        )
        .expect("failed to fail job");

        // Bump the run-level counter.
        db.execute(
            "UPDATE swarm_runs SET failed = failed + 1 WHERE run_id = (SELECT run_id FROM swarm_jobs WHERE job_id = ?1)",
            rusqlite::params![job_id],
        )
        .expect("failed to update run failed count");
    }

    /// Get failed jobs that have not yet exhausted their retry budget.
    pub fn get_retryable(&self, run_id: &str, max_attempts: u32) -> Vec<SwarmJob> {
        let db = self.db.lock().unwrap();
        let mut stmt = db
            .prepare(
                "SELECT job_id, url, status, worker_id, attempt_count, current_step, total_steps
                 FROM swarm_jobs
                 WHERE run_id = ?1 AND status = 'failed' AND attempt_count < ?2",
            )
            .expect("failed to prepare get_retryable query");

        stmt.query_map(rusqlite::params![run_id, max_attempts as i64], |row| {
            Ok(SwarmJob {
                job_id: row.get(0)?,
                url: row.get(1)?,
                status: row.get(2)?,
                worker_id: row.get(3)?,
                attempt_count: row.get::<_, i64>(4)? as u32,
                current_step: row.get::<_, i64>(5)? as u32,
                total_steps: row.get::<_, Option<i64>>(6)?.map(|v| v as u32),
            })
        })
        .expect("failed to execute get_retryable query")
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Get aggregate progress for a run.
    pub fn run_progress(&self, run_id: &str) -> RunProgress {
        let db = self.db.lock().unwrap();

        let (total, completed, failed, started_at): (i64, i64, i64, String) = db
            .query_row(
                "SELECT total_jobs, completed, failed, started_at FROM swarm_runs WHERE run_id = ?1",
                rusqlite::params![run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("failed to query run progress");

        let running: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM swarm_jobs WHERE run_id = ?1 AND status = 'running'",
                rusqlite::params![run_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let pending: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM swarm_jobs WHERE run_id = ?1 AND status = 'pending'",
                rusqlite::params![run_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let started =
            chrono::DateTime::parse_from_rfc3339(&started_at).expect("bad started_at timestamp");
        let elapsed_secs = (Utc::now() - started.with_timezone(&Utc))
            .num_milliseconds() as f64
            / 1000.0;

        RunProgress {
            run_id: run_id.to_string(),
            total: total as u32,
            completed: completed as u32,
            failed: failed as u32,
            pending: pending as u32,
            running: running as u32,
            elapsed_secs,
        }
    }

    /// Resume a run by resetting any stuck "running" jobs back to "pending".
    pub fn resume_run(&self, run_id: &str) {
        let db = self.db.lock().unwrap();
        db.execute(
            "UPDATE swarm_jobs SET status = 'pending', worker_id = NULL WHERE run_id = ?1 AND status = 'running'",
            rusqlite::params![run_id],
        )
        .expect("failed to resume run");

        // Also reset failed jobs back to pending so they can be retried.
        db.execute(
            "UPDATE swarm_jobs SET status = 'pending' WHERE run_id = ?1 AND status = 'failed'",
            rusqlite::params![run_id],
        )
        .expect("failed to reset failed jobs on resume");

        db.execute(
            "UPDATE swarm_runs SET status = 'running' WHERE run_id = ?1",
            rusqlite::params![run_id],
        )
        .expect("failed to update run status on resume");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkpoint() -> SwarmCheckpoint {
        SwarmCheckpoint::new(":memory:")
    }

    #[test]
    fn create_run_and_list_pending() {
        let cp = checkpoint();
        let urls = vec![
            "https://a.com".to_string(),
            "https://b.com".to_string(),
            "https://c.com".to_string(),
        ];
        let run_id = cp.create_run("test-playbook", &urls);

        let progress = cp.run_progress(&run_id);
        assert_eq!(progress.total, 3);
        assert_eq!(progress.pending, 3);
        assert_eq!(progress.completed, 0);
        assert_eq!(progress.failed, 0);
        assert_eq!(progress.running, 0);
    }

    #[test]
    fn claim_and_complete_job() {
        let cp = checkpoint();
        let urls = vec!["https://example.com".to_string()];
        let run_id = cp.create_run("test", &urls);

        let job = cp.next_pending_job(&run_id).expect("should have a pending job");
        assert_eq!(job.status, "pending");

        cp.claim_job(&job.job_id, "worker-1");

        // No more pending jobs.
        assert!(cp.next_pending_job(&run_id).is_none());

        let progress = cp.run_progress(&run_id);
        assert_eq!(progress.running, 1);
        assert_eq!(progress.pending, 0);

        cp.update_progress(&job.job_id, 3, 10);

        cp.complete_job(&job.job_id, r#"{"ok": true}"#);

        let progress = cp.run_progress(&run_id);
        assert_eq!(progress.completed, 1);
        assert_eq!(progress.running, 0);
    }

    #[test]
    fn fail_and_retry_flow() {
        let cp = checkpoint();
        let urls = vec!["https://flaky.com".to_string()];
        let run_id = cp.create_run("retry-test", &urls);

        // First attempt — claim then fail.
        let job = cp.next_pending_job(&run_id).unwrap();
        cp.claim_job(&job.job_id, "w1");
        cp.fail_job(&job.job_id, "timeout");

        let progress = cp.run_progress(&run_id);
        assert_eq!(progress.failed, 1);
        assert_eq!(progress.pending, 0);

        // Should be retryable (attempt_count=1, max=3).
        let retryable = cp.get_retryable(&run_id, 3);
        assert_eq!(retryable.len(), 1);
        assert_eq!(retryable[0].attempt_count, 1);

        // Fail two more times — should no longer be retryable at max_attempts=3.
        // Reset to pending first so we can re-claim.
        cp.resume_run(&run_id);
        let job = cp.next_pending_job(&run_id).unwrap();
        cp.claim_job(&job.job_id, "w2");
        cp.fail_job(&job.job_id, "timeout again");

        cp.resume_run(&run_id);
        let job = cp.next_pending_job(&run_id).unwrap();
        cp.claim_job(&job.job_id, "w3");
        cp.fail_job(&job.job_id, "still broken");

        let retryable = cp.get_retryable(&run_id, 3);
        assert_eq!(retryable.len(), 0, "should be exhausted after 3 attempts");
    }

    #[test]
    fn resume_resets_running_jobs() {
        let cp = checkpoint();
        let urls = vec!["https://stuck.com".to_string(), "https://ok.com".to_string()];
        let run_id = cp.create_run("resume-test", &urls);

        // Claim the first job (simulating a worker that crashed).
        let job = cp.next_pending_job(&run_id).unwrap();
        cp.claim_job(&job.job_id, "crashed-worker");

        let progress = cp.run_progress(&run_id);
        assert_eq!(progress.running, 1);
        assert_eq!(progress.pending, 1);

        // Resume should reset the running job back to pending.
        cp.resume_run(&run_id);

        let progress = cp.run_progress(&run_id);
        assert_eq!(progress.running, 0);
        assert_eq!(progress.pending, 2);
    }
}
