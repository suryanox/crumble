use serde::{Deserialize, Serialize};
use crate::key::IndexKey;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHeader {
    pub is_leaf: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeafEntry {
    pub key: IndexKey,
    pub page_index: u32,
    pub slot: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InternalEntry {
    pub key: IndexKey,
    pub child_page: u32,
}

