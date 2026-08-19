use crate::error::IndexError;
use crate::key::IndexKey;
use crumble_buffer::Page;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHeader {
    pub is_leaf: bool,
    /// Only meaningful when is_leaf == false: the child page for every
    /// key less than the first entry's key.
    pub leftmost_child: u32,
    /// Only meaningful when is_leaf == true: the next leaf in key order,
    /// or None if this is the rightmost leaf.
    pub next_leaf: Option<u32>,
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

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, IndexError> {
    bincode::serde::encode_to_vec(value, bincode::config::standard())
        .map_err(|e| IndexError::Encoding(e.to_string()))
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, IndexError> {
    let (value, _len) = bincode::serde::decode_from_slice(bytes, bincode::config::standard())
        .map_err(|e| IndexError::Encoding(e.to_string()))?;
    Ok(value)
}

fn write_slot<T: Serialize>(page: &mut Page, value: &T) -> Result<(), IndexError> {
    let bytes = encode(value)?;
    page.insert_row(&bytes).ok_or(IndexError::NodeFull)?;
    Ok(())
}

pub fn build_leaf_page(entries: &[LeafEntry], next_leaf: Option<u32>) -> Result<Page, IndexError> {
    let mut page = Page::new();
    write_slot(&mut page, &NodeHeader { is_leaf: true, leftmost_child: 0, next_leaf })?;

    for entry in entries {
        write_slot(&mut page, entry)?;
    }

    Ok(page)
}

pub fn build_internal_page(
    entries: &[InternalEntry],
    leftmost_child: u32,
) -> Result<Page, IndexError> {
    let mut page = Page::new();
    // next_leaf is always None for internal nodes — meaningless outside leaves.
    write_slot(&mut page, &NodeHeader { is_leaf: false, leftmost_child, next_leaf: None })?;

    for entry in entries {
        write_slot(&mut page, &entry)?;
    }

    Ok(page)
}

pub fn read_header(page: &Page) -> Result<NodeHeader, IndexError> {
    let bytes = page
        .get_row(0)
        .ok_or_else(|| IndexError::Encoding("missing node header".to_string()))?;

    decode(bytes)
}

pub fn read_leaf_entries(page: &Page) -> Result<Vec<LeafEntry>, IndexError> {
    let mut entries = Vec::new();

    // Slot 0 is always the header, slots 1.. are always entries
    for slot in 1..page.slot_count() {
        if let Some(bytes) = page.get_row(slot) {
            entries.push(decode(bytes)?);
        }
    }
    Ok(entries)
}

pub fn read_internal_entries(page: &Page) -> Result<Vec<InternalEntry>, IndexError> {
    let mut entries = Vec::new();
    // Slot 0 is always the header, slots 1.. are always entries
    for slot in 1..page.slot_count() {
        if let Some(bytes) = page.get_row(slot) {
            entries.push(decode(bytes)?);
        }
    }
    Ok(entries)
}
