//! Job priority scoring for swarm queue ordering.
//!
//! Scores job candidates based on configurable criteria (title keywords,
//! location preference, salary range, company lists) and sorts them
//! for priority processing.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::debug;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// On-disk / JSON-loadable scoring configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringConfig {
    pub title_keywords: Vec<KeywordWeight>,
    pub preferred_locations: Vec<String>,
    /// `[min, max]` salary range.
    pub salary_range: Option<(u64, u64)>,
    #[serde(default)]
    pub preferred_companies: Vec<String>,
    #[serde(default)]
    pub blocked_companies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeywordWeight {
    pub keyword: String,
    pub weight: f32,
}

// ---------------------------------------------------------------------------
// Scorer
// ---------------------------------------------------------------------------

/// Reusable scorer built from a [`ScoringConfig`].
#[derive(Debug, Clone)]
pub struct JobScorer {
    pub title_keywords: Vec<(String, f32)>,
    pub preferred_locations: Vec<String>,
    pub min_salary: Option<u64>,
    pub max_salary: Option<u64>,
    pub preferred_companies: Vec<String>,
    pub blocked_companies: Vec<String>,
}

impl JobScorer {
    /// Build a scorer from a [`ScoringConfig`].
    pub fn from_config(cfg: &ScoringConfig) -> Self {
        let (min_salary, max_salary) = match cfg.salary_range {
            Some((lo, hi)) => (Some(lo), Some(hi)),
            None => (None, None),
        };
        Self {
            title_keywords: cfg
                .title_keywords
                .iter()
                .map(|kw| (kw.keyword.to_lowercase(), kw.weight))
                .collect(),
            preferred_locations: cfg
                .preferred_locations
                .iter()
                .map(|l| l.to_lowercase())
                .collect(),
            min_salary,
            max_salary,
            preferred_companies: cfg
                .preferred_companies
                .iter()
                .map(|c| c.to_lowercase())
                .collect(),
            blocked_companies: cfg
                .blocked_companies
                .iter()
                .map(|c| c.to_lowercase())
                .collect(),
        }
    }

    /// Load a scorer from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let cfg: ScoringConfig = serde_json::from_str(json)?;
        Ok(Self::from_config(&cfg))
    }

    /// Score a single job posting.
    pub fn score_job(
        &self,
        title: &str,
        company: &str,
        location: &str,
        salary: Option<u64>,
        description: &str,
    ) -> JobScore {
        let mut breakdown = HashMap::new();

        // --- Title keyword match (0-10 normalised) ---
        let title_lower = title.to_lowercase();
        let raw_title: f32 = self
            .title_keywords
            .iter()
            .filter(|(kw, _)| title_lower.contains(kw.as_str()))
            .map(|(_, w)| w)
            .sum();
        let max_possible_title: f32 = self.title_keywords.iter().map(|(_, w)| w).sum();
        let title_score = if max_possible_title > 0.0 {
            (raw_title / max_possible_title * 10.0).min(10.0)
        } else {
            0.0
        };
        breakdown.insert("title".to_string(), title_score);

        // --- Location match ---
        let loc_lower = location.to_lowercase();
        let location_score = if self.preferred_locations.iter().any(|l| loc_lower.contains(l.as_str())) {
            5.0
        } else if loc_lower.contains("remote") {
            2.0
        } else {
            0.0
        };
        breakdown.insert("location".to_string(), location_score);

        // --- Salary fit ---
        let salary_score = match (salary, self.min_salary, self.max_salary) {
            (Some(s), Some(lo), Some(hi)) => {
                if s >= lo && s <= hi {
                    5.0
                } else {
                    // "close" = within 20 % of either bound
                    let margin = ((hi - lo) as f32 * 0.20) as u64;
                    if s >= lo.saturating_sub(margin) && s <= hi + margin {
                        3.0
                    } else {
                        0.0
                    }
                }
            }
            _ => 0.0,
        };
        breakdown.insert("salary".to_string(), salary_score);

        // --- Company preference ---
        let comp_lower = company.to_lowercase();
        let company_score = if self.blocked_companies.iter().any(|c| comp_lower.contains(c.as_str())) {
            -100.0
        } else if self.preferred_companies.iter().any(|c| comp_lower.contains(c.as_str())) {
            5.0
        } else {
            0.0
        };
        breakdown.insert("company".to_string(), company_score);

        // --- Description keyword density (0-5) ---
        let desc_lower = description.to_lowercase();
        let matched_keywords: usize = self
            .title_keywords
            .iter()
            .filter(|(kw, _)| desc_lower.contains(kw.as_str()))
            .count();
        let total_keywords = self.title_keywords.len().max(1);
        let desc_score = (matched_keywords as f32 / total_keywords as f32 * 5.0).min(5.0);
        breakdown.insert("description".to_string(), desc_score);

        // --- Total (clamped 0-30) ---
        let total = (title_score + location_score + salary_score + company_score + desc_score)
            .clamp(0.0, 30.0);

        let recommendation = if company_score <= -100.0 {
            Recommendation::Block
        } else if total >= 18.0 {
            Recommendation::Apply
        } else if total >= 10.0 {
            Recommendation::Maybe
        } else {
            Recommendation::Skip
        };

        debug!(
            title = %title,
            total = total,
            recommendation = ?recommendation,
            "Job scored"
        );

        JobScore {
            total,
            breakdown,
            recommendation,
        }
    }

    /// Sort a list of candidates in-place, highest score first.
    /// Candidates without a score are scored first.
    pub fn sort_by_priority(&self, jobs: &mut Vec<JobCandidate>) {
        // Ensure every candidate has a score.
        for job in jobs.iter_mut() {
            if job.score.is_none() {
                let s = self.score_job(
                    &job.title,
                    &job.company,
                    &job.location,
                    job.salary,
                    &job.description,
                );
                job.score = Some(s);
            }
        }

        jobs.sort_by(|a, b| {
            let sa = a.score.as_ref().map(|s| s.total).unwrap_or(0.0);
            let sb = b.score.as_ref().map(|s| s.total).unwrap_or(0.0);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
    }
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Detailed score breakdown for a single job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobScore {
    pub total: f32,
    pub breakdown: HashMap<String, f32>,
    pub recommendation: Recommendation,
}

/// Action recommendation derived from the score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Recommendation {
    Apply,
    Maybe,
    Skip,
    Block,
}

/// A job posting candidate flowing through the swarm queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobCandidate {
    pub url: String,
    pub title: String,
    pub company: String,
    pub location: String,
    pub salary: Option<u64>,
    pub description: String,
    pub score: Option<JobScore>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config_json() -> &'static str {
        r#"{
            "title_keywords": [
                {"keyword": "senior", "weight": 2.0},
                {"keyword": "staff", "weight": 3.0},
                {"keyword": "principal", "weight": 4.0},
                {"keyword": "AI", "weight": 2.5},
                {"keyword": "ML", "weight": 2.5},
                {"keyword": "rust", "weight": 3.0}
            ],
            "preferred_locations": ["remote", "San Francisco", "New York"],
            "salary_range": [120000, 300000],
            "preferred_companies": ["OpenAI", "Anthropic", "Google"],
            "blocked_companies": ["Scam Corp"]
        }"#
    }

    fn scorer() -> JobScorer {
        JobScorer::from_json(sample_config_json()).expect("valid config")
    }

    #[test]
    fn test_parse_config() {
        let s = scorer();
        assert_eq!(s.title_keywords.len(), 6);
        assert_eq!(s.preferred_locations.len(), 3);
        assert_eq!(s.min_salary, Some(120_000));
        assert_eq!(s.max_salary, Some(300_000));
        assert_eq!(s.preferred_companies.len(), 3);
        assert_eq!(s.blocked_companies.len(), 1);
    }

    #[test]
    fn test_ideal_job_scores_high() {
        let s = scorer();
        let score = s.score_job(
            "Senior Staff Rust Engineer",
            "Anthropic",
            "San Francisco, CA",
            Some(200_000),
            "We are looking for a senior staff engineer with deep Rust and ML experience.",
        );
        // title: senior(2) + staff(3) + rust(3) = 8/17 * 10 ≈ 4.7
        // location: preferred (SF) => 5
        // salary: in range => 5
        // company: preferred => 5
        // description: senior, staff, rust, ml => 4/6 * 5 ≈ 3.3
        // total ≈ 23
        assert!(score.total >= 18.0, "expected Apply-level score, got {}", score.total);
        assert_eq!(score.recommendation, Recommendation::Apply);
    }

    #[test]
    fn test_blocked_company() {
        let s = scorer();
        let score = s.score_job(
            "Senior Rust Engineer",
            "Scam Corp",
            "Remote",
            Some(150_000),
            "Rust engineer needed.",
        );
        assert_eq!(score.recommendation, Recommendation::Block);
        // Total is clamped to 0 because company = -100
        assert_eq!(score.total, 0.0);
    }

    #[test]
    fn test_mediocre_job() {
        let s = scorer();
        let score = s.score_job(
            "Junior Python Developer",
            "Acme Inc",
            "Austin, TX",
            None,
            "Python web dev role. Django experience required.",
        );
        // No keyword matches, no location, no salary, no company pref
        assert_eq!(score.total, 0.0);
        assert_eq!(score.recommendation, Recommendation::Skip);
    }

    #[test]
    fn test_remote_location_partial() {
        let s = scorer();
        let score = s.score_job(
            "ML Engineer",
            "Startup Co",
            "Remote",
            Some(130_000),
            "ML and AI research position.",
        );
        // location: "remote" is in preferred_locations, so score = 5
        assert_eq!(*score.breakdown.get("location").unwrap(), 5.0);
    }

    #[test]
    fn test_salary_close_range() {
        let s = scorer();
        let score = s.score_job(
            "ML Engineer",
            "Startup Co",
            "Denver, CO",
            Some(110_000), // below min 120k but within 20% margin (36k)
            "ML role.",
        );
        assert_eq!(*score.breakdown.get("salary").unwrap(), 3.0);
    }

    #[test]
    fn test_salary_out_of_range() {
        let s = scorer();
        let score = s.score_job(
            "ML Engineer",
            "Startup Co",
            "Denver, CO",
            Some(50_000), // well below range
            "ML role.",
        );
        assert_eq!(*score.breakdown.get("salary").unwrap(), 0.0);
    }

    #[test]
    fn test_sort_by_priority() {
        let s = scorer();

        let mut jobs = vec![
            JobCandidate {
                url: "https://example.com/bad".into(),
                title: "Junior Python Dev".into(),
                company: "Acme".into(),
                location: "Austin".into(),
                salary: None,
                description: "Python role".into(),
                score: None,
            },
            JobCandidate {
                url: "https://example.com/great".into(),
                title: "Senior Staff Rust Engineer".into(),
                company: "Anthropic".into(),
                location: "San Francisco".into(),
                salary: Some(200_000),
                description: "Rust, ML, AI, senior staff engineer".into(),
                score: None,
            },
            JobCandidate {
                url: "https://example.com/mid".into(),
                title: "Senior Engineer".into(),
                company: "Startup".into(),
                location: "Remote".into(),
                salary: Some(140_000),
                description: "Backend engineer".into(),
                score: None,
            },
        ];

        s.sort_by_priority(&mut jobs);

        assert_eq!(jobs[0].url, "https://example.com/great");
        assert_eq!(jobs[2].url, "https://example.com/bad");

        // All candidates should now have scores.
        assert!(jobs.iter().all(|j| j.score.is_some()));
    }

    #[test]
    fn test_maybe_recommendation() {
        let s = scorer();
        let score = s.score_job(
            "Senior Engineer",
            "RandomCo",
            "Remote",
            Some(150_000),
            "Looking for a senior backend engineer with experience.",
        );
        // title: senior(2)/17*10 ≈ 1.18
        // location: remote => 5
        // salary: in range => 5
        // company: 0
        // description: senior => 1/6*5 ≈ 0.83
        // total ≈ 12
        assert!(score.total >= 10.0 && score.total < 18.0,
            "expected Maybe range, got {}", score.total);
        assert_eq!(score.recommendation, Recommendation::Maybe);
    }
}
