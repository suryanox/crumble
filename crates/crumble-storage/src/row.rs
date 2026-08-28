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
    pub xmin: u64,
    pub xmax: Option<u64>,
}

impl Row {
    pub fn new(values: Vec<Value>) -> Self {
        // xmin=0 is a temporary placeholder, every real insert path will
        // need to set this to a real transaction id once BEGIN/COMMIT wiring
        // exists. Existing callers (all of them, right now) are effectively
        // running outside any transaction, which is why 0 is safe as a
        // stand-in rather than requiring every call site to change today.
        Self {
            values,
            xmin: 0,
            xmax: None,
        }
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
