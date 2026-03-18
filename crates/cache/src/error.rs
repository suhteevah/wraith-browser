use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("IO error: {0}")]
    IoError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Compression error: {0}")]
    CompressionError(String),

    #[error("Fulltext index error: {0}")]
    IndexError(String),

    #[error("URL parse error: {0}")]
    UrlError(String),

    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<rusqlite::Error> for CacheError {
    fn from(e: rusqlite::Error) -> Self {
        CacheError::DatabaseError(e.to_string())
    }
}

impl From<serde_json::Error> for CacheError {
    fn from(e: serde_json::Error) -> Self {
        CacheError::SerializationError(e.to_string())
    }
}

pub type CacheResult<T> = Result<T, CacheError>;
