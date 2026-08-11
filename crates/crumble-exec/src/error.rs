use crumble_storage::StorageError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExecError {
    #[error(transparent)]
    Storage(#[from] StorageError),

    #[error("column not found: {0}")]
    ColumnNotFound(String),

    #[error("type mismatch: cannot apply operator to give operand types")]
    TypeMismatch,

    #[error("missing value for column: {0}")]
    MissingColumn(String),
}
