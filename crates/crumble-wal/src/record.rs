use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WalRecord {
    Insert {
        table: String,
        page_index: u32,
        row_bytes: Vec<u8>,
    },
    Delete {
        table: String,
        page_index: u32,
        slot: u16,
    },
}
