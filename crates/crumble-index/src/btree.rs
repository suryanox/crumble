use std::path::Path;

use crumble_buffer::BufferPool;

use crate::error::IndexError;
use crate::key::IndexKey;
use crate::node::{
    InternalEntry, LeafEntry, build_internal_page, build_leaf_page, read_header,
    read_internal_entries, read_leaf_entries,
};

const CAPACITY: usize = 64;

/**
search always starts at a known page. If a root split just picked some arbitrary
new page as the new root, we'd need to persist "where's the current root"
somewhere else — another file, another failure mode, another thing that can
get out of sync. The standard fix (SQLite does exactly this): the root always
lives at a fixed page index (0), forever. When the root needs to split, its contents move off to two freshly
allocated pages, and page 0 gets overwritten with a brand-new internal node
routing between them. No root-pointer bookkeeping needed, ever.
*/
const ROOT_PAGE: u32 = 0;

#[derive(Debug)]
pub struct BTree {
    pool: BufferPool,
}

impl BTree {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, IndexError> {
        let mut pool = BufferPool::open(path, CAPACITY)?;

        if pool.page_count() == 0 {
            let page = build_leaf_page(&[])?;
            pool.write_page(0, &page)?;
        }

        Ok(Self { pool })
    }

    pub fn search(&mut self, key: &IndexKey) -> Result<Vec<(u32, u16)>, IndexError> {
        let mut current = ROOT_PAGE;

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

    pub fn insert(&mut self, key: IndexKey, page_index: u32, slot: u16) -> Result<(), IndexError> {
        let path = self.path_to_leaf(&key)?;

        let leaf_page = *path.last().expect("path always has at least the root");

        let mut entries = read_leaf_entries(&self.pool.fetch_page(leaf_page)?)?;

        let insert_at = entries
            .iter()
            .position(|e| e.key == key)
            .unwrap_or(entries.len());

        entries.insert(
            insert_at,
            LeafEntry {
                key,
                page_index,
                slot,
            },
        );

        match build_leaf_page(&entries) {
            Ok(page) => {
                self.pool.write_page(leaf_page, &page)?;
                Ok(())
            }
            Err(IndexError::NodeFull) => self.split_leaf(&path, entries),
            Err(e) => Err(e),
        }
    }

    fn path_to_leaf(&mut self, key: &IndexKey) -> Result<Vec<u32>, IndexError> {
        let mut path = vec![ROOT_PAGE];
        let mut current = ROOT_PAGE;

        loop {
            let page = self.pool.fetch_page(current)?;
            let header = read_header(&page)?;
            if header.is_leaf {
                return Ok(path);
            }

            let entries = read_internal_entries(&page)?;
            current = child_for_key(&entries, header.leftmost_child, key);
            path.push(current);
        }
    }

    fn split_leaf(&mut self, path: &[u32], entries: Vec<LeafEntry>) -> Result<(), IndexError> {
        let leaf_page = *path.last().unwrap();
        let mid = entries.len() / 2;
        let left: Vec<LeafEntry> = entries[..mid].to_vec();
        let right: Vec<LeafEntry> = entries[mid..].to_vec();
        let separator = right[0].key.clone();

        let left_page_bytes = build_leaf_page(&left)?;
        let right_page_bytes = build_leaf_page(&right)?;

        if leaf_page == ROOT_PAGE {
            self.split_root(left_page_bytes, right_page_bytes, separator)
        } else {
            self.pool.write_page(leaf_page, &left_page_bytes)?;
            let new_page = self.pool.page_count();
            self.pool.write_page(new_page, &right_page_bytes)?;
            self.propagate(&path[..path.len() - 1], separator, new_page)
        }
    }

    /// Inserts a new routing entry into an ancestor internal node, splitting
    /// (recursively, up to and including the root) if it doesn't fit.
    fn propagate(
        &mut self,
        ancestor_path: &[u32],
        key: IndexKey,
        new_child: u32,
    ) -> Result<(), IndexError> {
        let parent_page = *ancestor_path
            .last()
            .expect("propagate called with no parent");
        let parent = self.pool.fetch_page(parent_page)?;
        let header = read_header(&parent)?;
        let mut entries = read_internal_entries(&parent)?;

        let insert_at = entries
            .iter()
            .position(|e| e.key > key)
            .unwrap_or(entries.len());
        entries.insert(
            insert_at,
            InternalEntry {
                key,
                child_page: new_child,
            },
        );

        match build_internal_page(&entries, header.leftmost_child) {
            Ok(page) => {
                self.pool.write_page(parent_page, &page)?;
                Ok(())
            }
            Err(IndexError::NodeFull) => {
                self.split_internal(ancestor_path, entries, header.leftmost_child)
            }
            Err(err) => Err(err),
        }
    }

    fn split_internal(
        &mut self,
        path: &[u32],
        entries: Vec<InternalEntry>,
        leftmost_child: u32,
    ) -> Result<(), IndexError> {
        let node_page = *path.last().unwrap();
        let mid = entries.len() / 2;
        let promoted = entries[mid].clone();

        let left: Vec<InternalEntry> = entries[..mid].to_vec();
        let right: Vec<InternalEntry> = entries[mid + 1..].to_vec();

        let left_page_bytes = build_internal_page(&left, leftmost_child)?;
        let right_page_bytes = build_internal_page(&right, promoted.child_page)?;

        if node_page == ROOT_PAGE {
            self.split_root(left_page_bytes, right_page_bytes, promoted.key)
        } else {
            self.pool.write_page(node_page, &left_page_bytes)?;
            let new_page = self.pool.page_count();
            self.pool.write_page(new_page, &right_page_bytes)?;
            self.propagate(&path[..path.len() - 1], promoted.key, new_page)
        }
    }

    /// The root never moves. Its current contents (already split into left/
    /// right) get relocated to two fresh pages, and page 0 is overwritten
    /// with a brand new internal root routing between them.
    fn split_root(
        &mut self,
        left: crumble_buffer::Page,
        right: crumble_buffer::Page,
        separator: IndexKey,
    ) -> Result<(), IndexError> {
        let left_page = self.pool.page_count();
        self.pool.write_page(left_page, &left)?;

        let right_page = self.pool.page_count();
        self.pool.write_page(right_page, &right)?;

        let new_root = build_internal_page(
            &[InternalEntry {
                key: separator,
                child_page: right_page,
            }],
            left_page,
        )?;
        self.pool.write_page(ROOT_PAGE, &new_root)?;

        Ok(())
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

    #[test]
    fn insert_then_search_finds_it() -> Result<(), IndexError> {
        let dir = tempfile::tempdir().unwrap();
        let mut tree = BTree::open(dir.path().join("test.idx"))?;

        tree.insert(IndexKey::Int(42), 3, 1)?;
        let results = tree.search(&IndexKey::Int(42))?;

        assert_eq!(results, vec![(3, 1)]);
        Ok(())
    }

    #[test]
    fn many_inserts_force_splits_and_all_remain_findable() -> Result<(), IndexError> {
        let dir = tempfile::tempdir().unwrap();
        let mut tree = BTree::open(dir.path().join("test.idx"))?;

        for i in 0..500i64 {
            tree.insert(IndexKey::Int(i), i as u32, 0)?;
        }

        for i in 0..500i64 {
            let results = tree.search(&IndexKey::Int(i))?;
            assert_eq!(
                results,
                vec![(i as u32, 0)],
                "key {i} not found after splits"
            );
        }

        Ok(())
    }
}
