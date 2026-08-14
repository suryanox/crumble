use crumble_buffer::BufferError;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("column count mismatch: expected {expected}, got {actual}")]
    ColumnCountMismatch { expected: usize, actual: usize },

    #[error("table not found: {0}")]
    TableNotFound(String),

    #[error("table already exists: {0}")]
    TableAlreadyExists(String),

    #[error("row encoding failed: {0}")]
    Encoding(String),

    #[error("row too large to fit in a page")]
    RowTooLarge,

    #[error(transparent)]
    Buffer(#[from] BufferError),

    #[error("storage I/O error: {0}")]
    Io(#[from] std::io::Error),
}
