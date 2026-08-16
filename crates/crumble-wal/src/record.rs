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
    WritePage {
        /**
        WritePage has no table/name field — each index already gets its own WAL file (same as each table does),
        so there's nothing to disambiguate within one file. This is a genuinely more general record shape than Insert;
        Table doesn't need to change to use it, but it's worth noticing WritePage could someday subsume
        Insert if Table ever moves to whole-page rewrites too. Not doing that now
        */
        page_index: u32,
        page_data: Vec<u8>,
    },
}
