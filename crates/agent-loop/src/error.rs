use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("Max steps ({max_steps}) exceeded")]
    MaxStepsExceeded { max_steps: usize },
    #[error("Context window budget exceeded ({tokens} tokens)")]
    ContextBudgetExceeded { tokens: usize },
    #[error("LLM call failed: {0}")]
    LlmFailed(String),
    #[error("Action parse failed: {raw}")]
    ActionParseFailed { raw: String },
    #[error("Task aborted by user")]
    Aborted,
    #[error("Browser error: {0}")]
    Browser(#[from] wraith_browser_core::BrowserError),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}
