use std::path::Path;

use crumble_buffer::BufferPool;

use crate::error::IndexError;
use crate::key::IndexKey;
use crate::node::{
    InternalEntry, build_leaf_page, read_header, read_internal_entries, read_leaf_entries,
};

const CAPACITY: usize = 64;

pub struct BTree {
    pool: BufferPool,
    root_page: u32,
}

impl BTree {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, IndexError> {
        let mut pool = BufferPool::open(path, CAPACITY)?;

        if pool.page_count() == 0 {
            let page = build_leaf_page(&[])?;
            pool.write_page(0, &page)?;
        }

        Ok(Self { pool, root_page: 0 })
    }

    pub fn search(&mut self, key: &IndexKey) -> Result<Vec<(u32, u16)>, IndexError> {
        let mut current = self.root_page;

        loop {
            let page = self.pool.fetch_page(current)?;
            let header = read_header(&page)?;

            if header.is_leaf {
                let entries = read_leaf_entries(&page)?;
                return Ok(entries
                    .into_iter()
                    .filter(|entry| &entry.key == key)
                    .map(|entry| (entry.page_index, entry.slot))
                    .collect());
            }

            let entries = read_internal_entries(&page)?;
            current = child_for_key(&entries, header.leftmost_child, key);
        }
    }
}

/// Standard B+tree internal-node routing: entries are sorted ascending.
/// `leftmost_child` covers everything below entries[0].key. Each entry's
/// child covers [that entry's key, next entry's key).
fn child_for_key(entries: &[InternalEntry], leftmost_child: u32, key: &IndexKey) -> u32 {
    let mut chosen = leftmost_child;

    // we keep advancing chosen rightward through the entries until
    // we find one whose key is bigger than what we're looking for,
    // at which point we stop and descend into whatever chosen currently holds.
    // This naturally lands on the correct
    // child whether the tree has one level or many, since search's
    // loop just keeps calling this at each internal level until it hits a leaf.

    for entry in entries {
        if key < &entry.key {
            break;
        }
        chosen = entry.child_page;
    }

    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_on_empty_tree_returns_nothing() -> Result<(), IndexError> {
        let dir = tempfile::tempdir().unwrap();
        let mut tree = BTree::open(dir.path().join("test.idx"))?;

        let results = tree.search(&IndexKey::Int(42))?;
        assert!(results.is_empty());
        Ok(())
    }
}
