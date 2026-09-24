use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("format error at line {line} column {col}: {message}")]
    Format { line: usize, col: usize, message: String },
    #[error("semantic issue: {0}")]
    Semantic(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("{0}")]
    Other(String),
}

impl From<rusqlite::Error> for DomainError {
    fn from(e: rusqlite::Error) -> Self {
        DomainError::Storage(e.to_string())
    }
}

impl From<serde_json::Error> for DomainError {
    fn from(e: serde_json::Error) -> Self {
        DomainError::Other(e.to_string())
    }
}

impl From<anyhow::Error> for DomainError {
    fn from(e: anyhow::Error) -> Self {
        DomainError::Other(e.to_string())
    }
}

pub type DomainResult<T> = Result<T, DomainError>;
