use thiserror::Error;

#[derive(Debug, Error)]
pub enum BufferError {
    #[error("storage I/O error: {0}")]
    Io(#[from] std::io::Error),
}
