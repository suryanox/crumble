use crumble_wal::WalError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("index buffer error: {0}")]
    Buffer(#[from] crumble_buffer::BufferError),

    #[error("index node encoding failed: {0}")]
    Encoding(String),

    #[error("index node is full")]
    NodeFull,

    #[error(transparent)]
    Wal(#[from] WalError),
}
