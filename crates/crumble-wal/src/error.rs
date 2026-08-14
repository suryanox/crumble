use thiserror::Error;

#[derive(Debug, Error)]
pub enum WalError {
    #[error("WAL I/0 error: {0}")]
    Io(#[from] std::io::Error),

    #[error("WAL record encoding failed: {0}")]
    Encoding(String),
}
