use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("HTML parsing failed: {0}")]
    ParseFailed(String),

    #[error("No readable content found at {url}")]
    NoContent { url: String },

    #[error("Markdown conversion failed: {0}")]
    MarkdownFailed(String),

    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}
