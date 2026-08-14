use serde::{Deserialize, Serialize};

use crumble_storage::Row;


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WalRecord {
    Insert { table: String, row: Row }
}