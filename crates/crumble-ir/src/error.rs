
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LowerError {
    #[error("unsupported SQL construct: {0}")]
    Unsupported(String),
}