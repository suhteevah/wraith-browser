//! Time-Travel Debugging — records full browser state at each agent step,
//! enabling branch-from-any-point exploration and deterministic replay.
//!
//! The [`TimelineRecorder`] captures every observation, action, and LLM response
//! as a [`TimelineStep`]. At any point you can branch the timeline, replay to an
//! earlier step, or diff two branches to see where they diverged.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument};
use uuid::Uuid;

/// A single recorded step in the agent timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineStep {
    /// Monotonically increasing step index.
    pub step_number: usize,
    /// The URL the browser was on at this step.
    pub url: String,
    /// The page title at this step.
    pub page_title: String,
    /// The agent-text DOM snapshot captured at this step.
    pub snapshot_text: String,
    /// The `ACTION:` line emitted by the LLM, if any.
    pub action_taken: Option<String>,
    /// The full LLM response for this step, if any.
    pub llm_response: Option<String>,
    /// When this step was recorded.
    pub timestamp: DateTime<Utc>,
    /// Wall-clock duration of this step in milliseconds.
    pub duration_ms: u64,
}

/// A named branch forked from a specific point in the main timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    /// Unique identifier for this branch.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// The step number in the main timeline that this branch was forked from.
    pub branched_from_step: usize,
    /// The steps belonging to this branch (starts with a copy of 0..=branched_from_step).
    pub steps: Vec<TimelineStep>,
    /// When the branch was created.
    pub created_at: DateTime<Utc>,
}

/// Describes a difference between two branches at a given step.
#[derive(Debug, Clone)]
pub struct StepDiff {
    /// The step index being compared.
    pub step_number: usize,
    /// The action from the first branch (or main timeline) at this step.
    pub main_action: Option<String>,
    /// The action from the second branch at this step.
    pub branch_action: Option<String>,
    /// `true` when the two actions differ.
    pub diverged: bool,
}

/// Records the full agent timeline and manages branches for time-travel debugging.
pub struct TimelineRecorder {
    /// The ordered sequence of steps on the main timeline.
    steps: Vec<TimelineStep>,
    /// Unique session identifier.
    session_id: String,
    /// When recording started.
    started_at: DateTime<Utc>,
    /// Named branches forked from the main timeline.
    branches: Vec<Branch>,
}

impl TimelineRecorder {
    /// Create a new recorder for the given session.
    #[instrument(skip_all, fields(%session_id))]
    pub fn new(session_id: String) -> Self {
        info!(session_id = %session_id, "timeline recorder created");
        Self {
            steps: Vec::new(),
            session_id,
            started_at: Utc::now(),
            branches: Vec::new(),
        }
    }

    /// Append a step to the main timeline.
    #[instrument(skip(self, step), fields(step_number = step.step_number))]
    pub fn record_step(&mut self, step: TimelineStep) {
        debug!(step_number = step.step_number, url = %step.url, "recording step");
        self.steps.push(step);
    }

    /// Retrieve a step from the main timeline by index.
    #[instrument(skip(self))]
    pub fn get_step(&self, n: usize) -> Option<&TimelineStep> {
        self.steps.get(n)
    }

    /// Number of steps on the main timeline.
    #[instrument(skip(self))]
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    /// Create a new branch starting from `step_number`, copying steps 0..=step_number
    /// into the branch. Returns the new branch ID.
    #[instrument(skip(self))]
    pub fn branch_from(&mut self, step_number: usize, name: &str) -> Result<String, String> {
        if step_number >= self.steps.len() {
            return Err(format!(
                "step {} does not exist (timeline has {} steps)",
                step_number,
                self.steps.len()
            ));
        }

        let branch_id = Uuid::new_v4().to_string();
        let copied_steps = self.steps[..=step_number].to_vec();

        let branch = Branch {
            id: branch_id.clone(),
            name: name.to_string(),
            branched_from_step: step_number,
            steps: copied_steps,
            created_at: Utc::now(),
        };

        info!(
            branch_id = %branch_id,
            name = %name,
            branched_from_step = step_number,
            "branch created"
        );
        self.branches.push(branch);
        Ok(branch_id)
    }

    /// Look up a branch by its ID.
    #[instrument(skip(self))]
    pub fn get_branch(&self, id: &str) -> Option<&Branch> {
        self.branches.iter().find(|b| b.id == id)
    }

    /// List all branches as `(id, name, branched_from_step)` tuples.
    #[instrument(skip(self))]
    pub fn list_branches(&self) -> Vec<(String, String, usize)> {
        self.branches
            .iter()
            .map(|b| (b.id.clone(), b.name.clone(), b.branched_from_step))
            .collect()
    }

    /// Append a step to a specific branch.
    #[instrument(skip(self, step), fields(%branch_id))]
    pub fn record_to_branch(
        &mut self,
        branch_id: &str,
        step: TimelineStep,
    ) -> Result<(), String> {
        let branch = self
            .branches
            .iter_mut()
            .find(|b| b.id == branch_id)
            .ok_or_else(|| format!("branch '{}' not found", branch_id))?;

        debug!(
            branch_id = %branch_id,
            step_number = step.step_number,
            "recording step to branch"
        );
        branch.steps.push(step);
        Ok(())
    }

    /// Return references to steps 0..=`step_number` for deterministic replay.
    #[instrument(skip(self))]
    pub fn replay_to(&self, step_number: usize) -> Vec<&TimelineStep> {
        let end = std::cmp::min(step_number + 1, self.steps.len());
        self.steps[..end].iter().collect()
    }

    /// Compare two branches step-by-step and return the diffs.
    #[instrument(skip(self))]
    pub fn diff_branches(&self, branch_a: &str, branch_b: &str) -> Vec<StepDiff> {
        let steps_a: &[TimelineStep] = self
            .branches
            .iter()
            .find(|b| b.id == branch_a)
            .map(|b| b.steps.as_slice())
            .unwrap_or(&[]);

        let steps_b: &[TimelineStep] = self
            .branches
            .iter()
            .find(|b| b.id == branch_b)
            .map(|b| b.steps.as_slice())
            .unwrap_or(&[]);

        let max_len = std::cmp::max(steps_a.len(), steps_b.len());
        let mut diffs = Vec::with_capacity(max_len);

        for i in 0..max_len {
            let a_action = steps_a.get(i).and_then(|s| s.action_taken.clone());
            let b_action = steps_b.get(i).and_then(|s| s.action_taken.clone());
            let diverged = a_action != b_action;

            diffs.push(StepDiff {
                step_number: i,
                main_action: a_action,
                branch_action: b_action,
                diverged,
            });
        }

        diffs
    }

    /// Serialize the entire timeline (main steps + branches) to JSON.
    #[instrument(skip(self))]
    pub fn export_timeline(&self) -> String {
        #[derive(Serialize)]
        struct Export<'a> {
            session_id: &'a str,
            started_at: &'a DateTime<Utc>,
            steps: &'a [TimelineStep],
            branches: &'a [Branch],
        }

        let export = Export {
            session_id: &self.session_id,
            started_at: &self.started_at,
            steps: &self.steps,
            branches: &self.branches,
        };

        serde_json::to_string_pretty(&export).unwrap_or_else(|e| {
            format!("{{\"error\": \"serialization failed: {}\"}}", e)
        })
    }

    /// Human-readable summary of the recording session.
    #[instrument(skip(self))]
    pub fn summary(&self) -> String {
        let first_url = self
            .steps
            .first()
            .map(|s| s.url.as_str())
            .unwrap_or("(none)");
        let last_url = self
            .steps
            .last()
            .map(|s| s.url.as_str())
            .unwrap_or("(none)");

        format!(
            "Session {}: {} steps, {} branches, from {} to {}",
            self.session_id,
            self.steps.len(),
            self.branches.len(),
            first_url,
            last_url,
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    /// Helper to build a minimal step.
    fn make_step(n: usize, url: &str, action: Option<&str>) -> TimelineStep {
        TimelineStep {
            step_number: n,
            url: url.to_string(),
            page_title: format!("Page {}", n),
            snapshot_text: format!("snapshot-{}", n),
            action_taken: action.map(|a| a.to_string()),
            llm_response: Some(format!("llm-response-{}", n)),
            timestamp: Utc::now(),
            duration_ms: 100,
        }
    }

    #[test]
    fn test_record_step_and_get_step() {
        let mut rec = TimelineRecorder::new("test-1".into());
        let step = make_step(0, "https://example.com", Some("ACTION: click @e1"));
        rec.record_step(step.clone());

        let got = rec.get_step(0).expect("step should exist");
        assert_eq!(got.step_number, 0);
        assert_eq!(got.url, "https://example.com");
        assert_eq!(got.action_taken.as_deref(), Some("ACTION: click @e1"));

        assert!(rec.get_step(1).is_none());
    }

    #[test]
    fn test_step_count() {
        let mut rec = TimelineRecorder::new("test-2".into());
        assert_eq!(rec.step_count(), 0);

        rec.record_step(make_step(0, "https://a.com", None));
        rec.record_step(make_step(1, "https://b.com", None));
        rec.record_step(make_step(2, "https://c.com", None));
        assert_eq!(rec.step_count(), 3);
    }

    #[test]
    fn test_branch_from_creates_correct_steps() {
        let mut rec = TimelineRecorder::new("test-3".into());
        rec.record_step(make_step(0, "https://a.com", Some("ACTION: navigate https://a.com")));
        rec.record_step(make_step(1, "https://b.com", Some("ACTION: click @e2")));
        rec.record_step(make_step(2, "https://c.com", Some("ACTION: done")));

        let branch_id = rec.branch_from(1, "alt-path").expect("branch should succeed");
        let branch = rec.get_branch(&branch_id).expect("branch should exist");

        assert_eq!(branch.name, "alt-path");
        assert_eq!(branch.branched_from_step, 1);
        // Should contain steps 0 and 1 (i.e., 0..=1).
        assert_eq!(branch.steps.len(), 2);
        assert_eq!(branch.steps[0].url, "https://a.com");
        assert_eq!(branch.steps[1].url, "https://b.com");
    }

    #[test]
    fn test_branch_from_invalid_step_returns_error() {
        let mut rec = TimelineRecorder::new("test-4".into());
        rec.record_step(make_step(0, "https://a.com", None));

        let result = rec.branch_from(5, "bad");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn test_record_to_branch_appends() {
        let mut rec = TimelineRecorder::new("test-5".into());
        rec.record_step(make_step(0, "https://a.com", None));
        rec.record_step(make_step(1, "https://b.com", None));

        let branch_id = rec.branch_from(0, "explore").unwrap();

        let new_step = make_step(1, "https://x.com", Some("ACTION: click @e9"));
        rec.record_to_branch(&branch_id, new_step).unwrap();

        let branch = rec.get_branch(&branch_id).unwrap();
        assert_eq!(branch.steps.len(), 2); // original step 0 + new step
        assert_eq!(branch.steps[1].url, "https://x.com");
    }

    #[test]
    fn test_record_to_branch_unknown_id() {
        let mut rec = TimelineRecorder::new("test-5b".into());
        let result = rec.record_to_branch("nonexistent", make_step(0, "https://a.com", None));
        assert!(result.is_err());
    }

    #[test]
    fn test_replay_to_returns_correct_slice() {
        let mut rec = TimelineRecorder::new("test-6".into());
        for i in 0..5 {
            rec.record_step(make_step(i, &format!("https://{}.com", i), None));
        }

        let replayed = rec.replay_to(2);
        assert_eq!(replayed.len(), 3); // steps 0, 1, 2
        assert_eq!(replayed[0].step_number, 0);
        assert_eq!(replayed[2].step_number, 2);

        // Requesting beyond the end should return all steps.
        let all = rec.replay_to(100);
        assert_eq!(all.len(), 5);
    }

    #[test]
    fn test_diff_branches_detects_divergence() {
        let mut rec = TimelineRecorder::new("test-7".into());
        rec.record_step(make_step(0, "https://a.com", Some("ACTION: click @e1")));
        rec.record_step(make_step(1, "https://b.com", Some("ACTION: click @e2")));

        let id_a = rec.branch_from(1, "branch-a").unwrap();
        let id_b = rec.branch_from(1, "branch-b").unwrap();

        // Add a divergent step to branch-b.
        rec.record_to_branch(
            &id_b,
            make_step(2, "https://z.com", Some("ACTION: click @e99")),
        )
        .unwrap();

        let diffs = rec.diff_branches(&id_a, &id_b);

        // Steps 0 and 1 should match (not diverged), step 2 only exists in branch-b.
        assert!(!diffs[0].diverged);
        assert!(!diffs[1].diverged);
        assert_eq!(diffs.len(), 3);
        assert!(diffs[2].diverged);
        assert!(diffs[2].main_action.is_none());
        assert_eq!(
            diffs[2].branch_action.as_deref(),
            Some("ACTION: click @e99")
        );
    }

    #[test]
    fn test_export_timeline_produces_valid_json() {
        let mut rec = TimelineRecorder::new("test-8".into());
        rec.record_step(make_step(0, "https://a.com", Some("ACTION: navigate https://a.com")));
        rec.record_step(make_step(1, "https://b.com", None));

        let json = rec.export_timeline();
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("should be valid JSON");
        assert_eq!(parsed["session_id"], "test-8");
        assert!(parsed["steps"].is_array());
        assert_eq!(parsed["steps"].as_array().unwrap().len(), 2);
        assert!(parsed["branches"].is_array());
    }

    #[test]
    fn test_summary_format() {
        let mut rec = TimelineRecorder::new("sess-42".into());
        rec.record_step(make_step(0, "https://start.com", None));
        rec.record_step(make_step(1, "https://end.com", None));

        let summary = rec.summary();
        assert!(summary.contains("sess-42"));
        assert!(summary.contains("2 steps"));
        assert!(summary.contains("0 branches"));
        assert!(summary.contains("https://start.com"));
        assert!(summary.contains("https://end.com"));
    }

    #[test]
    fn test_list_branches() {
        let mut rec = TimelineRecorder::new("test-9".into());
        rec.record_step(make_step(0, "https://a.com", None));
        rec.record_step(make_step(1, "https://b.com", None));

        rec.branch_from(0, "first").unwrap();
        rec.branch_from(1, "second").unwrap();

        let branches = rec.list_branches();
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0].1, "first");
        assert_eq!(branches[0].2, 0);
        assert_eq!(branches[1].1, "second");
        assert_eq!(branches[1].2, 1);
    }
}
