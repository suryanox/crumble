use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("column count mismatch: expected {expected}, got {actual}")]
    ColumnCountMismatch { expected: usize, actual: usize },

    #[error("table not found: {0}")]
    TableNotFound(String),

    #[error("table already exists: {0}")]
    TableAlreadyExists(String),
}
