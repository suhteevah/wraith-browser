//! Application deduplication tracker.
//!
//! Prevents the agent from applying to the same job posting twice by
//! maintaining a SQLite-backed registry of every application, keyed by
//! a blake3 hash of the canonical URL.

use std::collections::HashMap;
use std::sync::Arc;

use blake3;
use chrono::{Utc, NaiveDateTime, DateTime};
use parking_lot::Mutex;
use rusqlite::params;
use tracing::{info, debug};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A single recorded application.
#[derive(Debug, Clone)]
pub struct ApplicationRecord {
    pub url_hash: String,
    pub url: String,
    pub company: Option<String>,
    pub title: Option<String>,
    pub platform: Option<String>,
    pub status: String,
    pub applied_at: String,
    pub result_json: Option<String>,
    pub worker_id: Option<String>,
}

/// Aggregate statistics across all tracked applications.
#[derive(Debug, Clone, Default)]
pub struct DedupStats {
    pub total_applied: usize,
    pub by_platform: HashMap<String, usize>,
    pub by_status: HashMap<String, usize>,
    pub today_count: usize,
    pub this_week_count: usize,
}

// ---------------------------------------------------------------------------
// Tracker
// ---------------------------------------------------------------------------

/// Persistent, thread-safe application deduplication tracker backed by SQLite.
pub struct ApplicationTracker {
    db: Arc<Mutex<rusqlite::Connection>>,
}

impl ApplicationTracker {
    /// Open (or create) the SQLite database at `db_path` and ensure the
    /// schema exists.
    pub fn new(db_path: &str) -> Self {
        let conn = rusqlite::Connection::open(db_path)
            .expect("failed to open dedup database");

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA busy_timeout=5000;"
        )
        .expect("failed to set pragmas");

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS applied_jobs (
                url_hash    TEXT PRIMARY KEY,
                url         TEXT NOT NULL,
                company     TEXT,
                title       TEXT,
                platform    TEXT,
                status      TEXT DEFAULT 'submitted',
                applied_at  TEXT NOT NULL,
                result_json TEXT,
                worker_id   TEXT
            );"
        )
        .expect("failed to create applied_jobs table");

        info!(path = db_path, "ApplicationTracker ready");

        Self {
            db: Arc::new(Mutex::new(conn)),
        }
    }

    // ----- helpers --------------------------------------------------------

    /// Compute the blake3 hash of a URL (hex-encoded).
    fn url_hash(url: &str) -> String {
        blake3::hash(url.as_bytes()).to_hex().to_string()
    }

    /// Parse a stored datetime string back into `DateTime<Utc>`.
    fn parse_dt(s: &str) -> DateTime<Utc> {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
            .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S"))
            .map(|dt| dt.and_utc())
            .unwrap_or_else(|_| Utc::now())
    }

    // ----- queries --------------------------------------------------------

    /// Returns `true` if the URL has ever been recorded.
    pub fn has_applied(&self, url: &str) -> bool {
        let hash = Self::url_hash(url);
        let db = self.db.lock();
        let count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM applied_jobs WHERE url_hash = ?1",
                params![hash],
                |row| row.get(0),
            )
            .unwrap_or(0);
        count > 0
    }

    /// Returns `true` if the URL was recorded within the last `days` days.
    pub fn has_applied_recently(&self, url: &str, days: u32) -> bool {
        let hash = Self::url_hash(url);
        let cutoff = (Utc::now() - chrono::Duration::days(i64::from(days)))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let db = self.db.lock();
        let count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM applied_jobs WHERE url_hash = ?1 AND applied_at >= ?2",
                params![hash, cutoff],
                |row| row.get(0),
            )
            .unwrap_or(0);
        count > 0
    }

    /// Record a new application. Silently replaces if the same URL hash
    /// already exists (idempotent upsert).
    pub fn record_application(
        &self,
        url: &str,
        company: Option<&str>,
        title: Option<&str>,
        platform: Option<&str>,
        status: &str,
        worker_id: Option<&str>,
    ) {
        let hash = Self::url_hash(url);
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        let db = self.db.lock();
        db.execute(
            "INSERT OR REPLACE INTO applied_jobs
                (url_hash, url, company, title, platform, status, applied_at, worker_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![hash, url, company, title, platform, status, now, worker_id],
        )
        .expect("failed to record application");
        debug!(url, status, "Recorded application");
    }

    /// Update the status (and optional result JSON) for a previously
    /// recorded application.
    pub fn update_status(&self, url: &str, status: &str, result_json: &str) {
        let hash = Self::url_hash(url);
        let db = self.db.lock();
        db.execute(
            "UPDATE applied_jobs SET status = ?1, result_json = ?2 WHERE url_hash = ?3",
            params![status, result_json, hash],
        )
        .expect("failed to update application status");
        debug!(url, status, "Updated application status");
    }

    /// Aggregate statistics across all tracked applications.
    pub fn stats(&self) -> DedupStats {
        let db = self.db.lock();

        let total_applied: usize = db
            .query_row("SELECT COUNT(*) FROM applied_jobs", [], |row| row.get::<_, i64>(0).map(|v| v as usize))
            .unwrap_or(0);

        // By platform
        let mut by_platform = HashMap::new();
        {
            let mut stmt = db
                .prepare(
                    "SELECT COALESCE(platform, 'unknown'), COUNT(*)
                     FROM applied_jobs GROUP BY platform",
                )
                .expect("prepare by_platform");
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
                })
                .expect("query by_platform");
            for row in rows.flatten() {
                by_platform.insert(row.0, row.1);
            }
        }

        // By status
        let mut by_status = HashMap::new();
        {
            let mut stmt = db
                .prepare(
                    "SELECT COALESCE(status, 'unknown'), COUNT(*)
                     FROM applied_jobs GROUP BY status",
                )
                .expect("prepare by_status");
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
                })
                .expect("query by_status");
            for row in rows.flatten() {
                by_status.insert(row.0, row.1);
            }
        }

        // Today
        let today_start = Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let today_count: usize = db
            .query_row(
                "SELECT COUNT(*) FROM applied_jobs WHERE applied_at >= ?1",
                params![today_start],
                |row| row.get::<_, i64>(0).map(|v| v as usize),
            )
            .unwrap_or(0);

        // This week (last 7 days)
        let week_start = (Utc::now() - chrono::Duration::days(7))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let this_week_count: usize = db
            .query_row(
                "SELECT COUNT(*) FROM applied_jobs WHERE applied_at >= ?1",
                params![week_start],
                |row| row.get::<_, i64>(0).map(|v| v as usize),
            )
            .unwrap_or(0);

        DedupStats {
            total_applied,
            by_platform,
            by_status,
            today_count,
            this_week_count,
        }
    }

    /// Return the most recent `limit` applications, newest first.
    pub fn recent(&self, limit: usize) -> Vec<ApplicationRecord> {
        let db = self.db.lock();
        let mut stmt = db
            .prepare(
                "SELECT url_hash, url, company, title, platform, status,
                        applied_at, result_json, worker_id
                 FROM applied_jobs
                 ORDER BY applied_at DESC
                 LIMIT ?1",
            )
            .expect("prepare recent");

        stmt.query_map(params![limit as i64], |row| {
            Ok(ApplicationRecord {
                url_hash: row.get(0)?,
                url: row.get(1)?,
                company: row.get(2)?,
                title: row.get(3)?,
                platform: row.get(4)?,
                status: row.get(5)?,
                applied_at: row.get(6)?,
                result_json: row.get(7)?,
                worker_id: row.get(8)?,
            })
        })
        .expect("query recent")
        .flatten()
        .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create an in-memory tracker so tests don't touch disk.
    fn mem_tracker() -> ApplicationTracker {
        ApplicationTracker::new(":memory:")
    }

    #[test]
    fn has_applied_returns_false_for_unknown_url() {
        let t = mem_tracker();
        assert!(!t.has_applied("https://example.com/job/123"));
    }

    #[test]
    fn record_then_has_applied() {
        let t = mem_tracker();
        let url = "https://acme.com/careers/42";
        t.record_application(url, Some("Acme"), Some("Engineer"), Some("lever"), "submitted", None);
        assert!(t.has_applied(url));
        assert!(!t.has_applied("https://other.com/job/1"));
    }

    #[test]
    fn has_applied_recently_respects_window() {
        let t = mem_tracker();
        let url = "https://corp.io/apply/99";
        t.record_application(url, None, None, None, "submitted", None);

        // Just recorded — should be within 1 day
        assert!(t.has_applied_recently(url, 1));
        // And within 30 days
        assert!(t.has_applied_recently(url, 30));
    }

    #[test]
    fn update_status_changes_record() {
        let t = mem_tracker();
        let url = "https://jobs.example/55";
        t.record_application(url, Some("Ex"), Some("Dev"), Some("greenhouse"), "submitted", Some("w1"));
        t.update_status(url, "confirmed", r#"{"ok":true}"#);

        let recs = t.recent(10);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].status, "confirmed");
        assert_eq!(recs[0].result_json.as_deref(), Some(r#"{"ok":true}"#));
    }

    #[test]
    fn stats_counts_correctly() {
        let t = mem_tracker();
        t.record_application("https://a.com/1", None, None, Some("lever"), "submitted", None);
        t.record_application("https://b.com/2", None, None, Some("lever"), "submitted", None);
        t.record_application("https://c.com/3", None, None, Some("greenhouse"), "confirmed", None);

        let s = t.stats();
        assert_eq!(s.total_applied, 3);
        assert_eq!(s.by_platform.get("lever"), Some(&2));
        assert_eq!(s.by_platform.get("greenhouse"), Some(&1));
        assert_eq!(s.by_status.get("submitted"), Some(&2));
        assert_eq!(s.by_status.get("confirmed"), Some(&1));
        assert_eq!(s.today_count, 3);
        assert_eq!(s.this_week_count, 3);
    }

    #[test]
    fn recent_respects_limit() {
        let t = mem_tracker();
        for i in 0..10 {
            t.record_application(
                &format!("https://x.com/{i}"),
                None,
                None,
                None,
                "submitted",
                None,
            );
        }
        let recs = t.recent(3);
        assert_eq!(recs.len(), 3);
    }
}
