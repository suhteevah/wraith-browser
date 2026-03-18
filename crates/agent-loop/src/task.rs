//! Browsing task definitions — what the agent is trying to accomplish.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowsingTask {
    /// Human-readable description of the task
    pub description: String,
    /// Starting URL (optional — agent can search if not provided)
    pub start_url: Option<String>,
    /// Maximum time budget in seconds
    pub timeout_secs: Option<u64>,
    /// Additional context or constraints
    pub context: Option<String>,
}
