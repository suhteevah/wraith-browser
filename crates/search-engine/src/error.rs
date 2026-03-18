use thiserror::Error;

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("Search provider {provider} failed: {reason}")]
    ProviderFailed { provider: String, reason: String },
    #[error("Local index error: {0}")]
    IndexError(String),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}
