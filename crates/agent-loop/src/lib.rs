//! # openclaw-agent-loop
//!
//! The core AI agent decision loop for OpenClaw Browser.
//! Implements an observe → think → act cycle where:
//!
//! 1. **Observe**: Take a DOM snapshot, extract content, gather page state
//! 2. **Think**: Send observation to LLM, get next action decision
//! 3. **Act**: Execute the decided browser action
//! 4. **Repeat** until task is complete or max_steps reached
//!
//! The "think" step is pluggable — it calls out to any LLM via a trait,
//! so OpenClaw can use Claude, local Ollama models, or any MCP-connected LLM.

pub mod agent;
pub mod llm;
pub mod task;
pub mod history;
pub mod error;

pub use agent::Agent;
pub use task::BrowsingTask;
pub use llm::LlmBackend;

use serde::{Deserialize, Serialize};

/// Configuration for the agent loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Maximum steps before the agent gives up
    pub max_steps: usize,

    /// Maximum total tokens to accumulate in observation history
    pub max_context_tokens: usize,

    /// LLM model to use for decision-making
    pub model: String,

    /// System prompt for the agent
    pub system_prompt: String,

    /// Whether to auto-extract page content after navigation
    pub auto_extract_content: bool,

    /// Whether to take screenshots for vision-capable models
    pub use_vision: bool,

    /// Token budget per page extraction
    pub page_token_budget: usize,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_steps: 50,
            max_context_tokens: 100_000,
            model: "claude-sonnet-4-20250514".to_string(),
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
            auto_extract_content: true,
            use_vision: false,
            page_token_budget: 4_000,
        }
    }
}

const DEFAULT_SYSTEM_PROMPT: &str = r#"You are an AI browser agent controlling a web browser. You receive observations about the current page state and must decide what action to take next to accomplish the user's task.

Available actions:
- navigate <url> — Go to a URL
- click @e<N> — Click element with ref ID N
- fill @e<N> "text" — Type text into input element N
- select @e<N> "value" — Select dropdown option
- scroll down/up <amount> — Scroll the page
- search "query" — Perform a web search
- extract — Get the page content as markdown
- screenshot — Capture the current page
- done "result" — Task complete, return result
- fail "reason" — Task cannot be completed

Respond with exactly ONE action per turn. Think step by step about what to do, then output your action on the last line prefixed with ACTION:

Example:
The page shows a login form. I need to fill in credentials.
ACTION: fill @e3 "username@example.com"
"#;
