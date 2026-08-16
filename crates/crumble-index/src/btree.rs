use crumble_wal::{WalRecord, WalWriter, read_all};
use std::path::Path;

use crumble_buffer::{BufferPool, PAGE_SIZE, Page};

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
    wal: WalWriter,
}

impl BTree {
    pub fn open(name: impl Into<String>, dir: impl AsRef<Path>) -> Result<Self, IndexError> {
        let name = name.into();
        let dir = dir.as_ref();

        let pages_path = dir.join(format!("{name}.idx"));
        let wal_path = dir.join(format!("{name}.idx.wal"));

        let pool = BufferPool::open(&pages_path, CAPACITY)?;
        let wal = WalWriter::open(&wal_path)?;

        let mut tree = Self { pool, wal };

        for (lsn, record) in read_all(&wal_path)? {
            let WalRecord::WritePage {
                page_index,
                page_data: page_bytes,
            } = record
            else {
                continue;
            };

            let already_durable = page_index < tree.pool.page_count()
                && tree.pool.fetch_page(page_index)?.page_lsn() >= lsn;

            if !already_durable {
                let bytes: [u8; PAGE_SIZE] = page_bytes
                    .try_into()
                    .map_err(|_| IndexError::Encoding("corrupt page in WAL".to_string()))?;
                tree.pool.write_page(page_index, &Page::from_bytes(bytes))?;
            }
        }

        if tree.pool.page_count() == 0 {
            let mut page = build_leaf_page(&[])?;
            tree.write_page_durable(ROOT_PAGE, &mut page)?;
        }

        Ok(tree)
    }

    /// Every real, committed page write goes through here: log the complete
    /// new page bytes (fsync, wait for it), stamp the resulting LSN into the
    /// page itself, then write it to the buffer pool. Same discipline as
    /// crumble-storage::Table, applied to whole-page writes instead of
    /// single-row inserts.
    fn write_page_durable(&mut self, page_index: u32, page: &mut Page) -> Result<(), IndexError> {
        let lsn = self.wal.append(&WalRecord::WritePage {
            page_index,
            page_data: page.as_bytes().to_vec(),
        })?;

        page.set_page_lsn(lsn);
        self.pool.write_page(page_index, page)?;
        Ok(())
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
            .position(|e| e.key > key)
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
            Ok(mut page) => {
                self.write_page_durable(leaf_page, &mut page)?;
                Ok(())
            }
            Err(IndexError::NodeFull) => self.split_leaf(&path, entries),
            Err(err) => Err(err),
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

        let mut left_page = build_leaf_page(&left)?;
        let mut right_page = build_leaf_page(&right)?;

        if leaf_page == ROOT_PAGE {
            self.split_root(&mut left_page, &mut right_page, separator)
        } else {
            self.write_page_durable(leaf_page, &mut left_page)?;
            let new_page = self.pool.page_count();
            self.write_page_durable(new_page, &mut right_page)?;
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
            Ok(mut page) => {
                self.write_page_durable(parent_page, &mut page)?;
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

        let mut left_page = build_internal_page(&left, leftmost_child)?;
        let mut right_page = build_internal_page(&right, promoted.child_page)?;

        if node_page == ROOT_PAGE {
            self.split_root(&mut left_page, &mut right_page, promoted.key)
        } else {
            self.write_page_durable(node_page, &mut left_page)?;
            let new_page = self.pool.page_count();
            self.write_page_durable(new_page, &mut right_page)?;
            self.propagate(&path[..path.len() - 1], promoted.key, new_page)
        }
    }

    /// The root never moves. Its current contents (already split into left/
    /// right) get relocated to two fresh pages, and page 0 is overwritten
    /// with a brand new internal root routing between them.
    fn split_root(
        &mut self,
        left: &mut Page,
        right: &mut Page,
        separator: IndexKey,
    ) -> Result<(), IndexError> {
        let left_page = self.pool.page_count();
        self.write_page_durable(left_page, left)?;

        let right_page = self.pool.page_count();
        self.write_page_durable(right_page, right)?;

        let mut new_root = build_internal_page(
            &[InternalEntry {
                key: separator,
                child_page: right_page,
            }],
            left_page,
        )?;
        self.write_page_durable(ROOT_PAGE, &mut new_root)?;

        Ok(())
    }

    pub fn delete(
        &mut self,
        key: &IndexKey,
        page_index: u32,
        slot: u16,
    ) -> Result<bool, IndexError> {
        let path = self.path_to_leaf(key)?;

        let leaf_page = *path.last().unwrap();

        let mut entries = read_leaf_entries(&self.pool.fetch_page(leaf_page)?)?;

        let before = entries.len();

        entries.retain(|e| !(&e.key == key && e.page_index == page_index && e.slot == slot));

        if entries.len() == before {
            return Ok(false);
        }

        let mut page = build_leaf_page(&entries)?;
        self.write_page_durable(leaf_page, &mut page)?;
        Ok(true)
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
        let mut tree = BTree::open("test", dir.path())?;

        let results = tree.search(&IndexKey::Int(42))?;
        assert!(results.is_empty());
        Ok(())
    }

    #[test]
    fn insert_then_search_finds_it() -> Result<(), IndexError> {
        let dir = tempfile::tempdir().unwrap();
        let mut tree = BTree::open("test", dir.path())?;

        tree.insert(IndexKey::Int(42), 3, 1)?;
        let results = tree.search(&IndexKey::Int(42))?;

        assert_eq!(results, vec![(3, 1)]);
        Ok(())
    }

    #[test]
    fn many_inserts_force_splits_and_all_remain_findable() -> Result<(), IndexError> {
        let dir = tempfile::tempdir().unwrap();
        let mut tree = BTree::open("test", dir.path())?;

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

    #[test]
    fn recovers_after_simulated_crash() -> Result<(), IndexError> {
        let dir = tempfile::tempdir().unwrap();

        {
            let mut tree = BTree::open("test", dir.path())?;
            for i in 0..500i64 {
                tree.insert(IndexKey::Int(i), i as u32, 0)?;
            }
            // tree dropped here with no explicit flush — simulating a crash
            // right after these inserts returned. Durable only via the WAL.
        }

        let mut recovered = BTree::open("test", dir.path())?;

        for i in 0..500i64 {
            let results = recovered.search(&IndexKey::Int(i))?;
            assert_eq!(
                results,
                vec![(i as u32, 0)],
                "key {i} lost after simulated crash"
            );
        }

        Ok(())
    }
}
