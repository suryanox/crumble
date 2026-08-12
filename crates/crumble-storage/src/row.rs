use crate::StorageError;
use crate::value::Value;
use serde::{Deserialize, Serialize};

/**
NewType instead of values: Vec<Value> as for MVCC and WAL. will need rowID and version/visibility
metadata.
*/
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    values: Vec<Value>,
}

impl Row {
    pub fn new(values: Vec<Value>) -> Self {
        Self { values }
    }

    pub fn values(&self) -> &[Value] {
        &self.values
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, StorageError> {
        bincode::serde::encode_to_vec(self, bincode::config::standard())
            .map_err(|e| StorageError::Encoding(e.to_string()))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, StorageError> {
        let (row, _len) = bincode::serde::decode_from_slice(bytes, bincode::config::standard())
            .map_err(|e| StorageError::Encoding(e.to_string()))?;
        Ok(row)
    }
}
