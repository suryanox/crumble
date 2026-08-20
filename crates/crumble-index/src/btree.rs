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
                page_bytes,
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
            let mut page = build_leaf_page(&[], None)?;
            tree.write_page_durable(ROOT_PAGE, &mut page)?;
        }

        Ok(tree)
    }

    fn write_page_durable(&mut self, page_index: u32, page: &mut Page) -> Result<(), IndexError> {
        let lsn = self.wal.append(&WalRecord::WritePage {
            page_index,
            page_bytes: page.as_bytes().to_vec(),
        })?;

        page.set_page_lsn(lsn);
        self.pool.write_page(page_index, page)?;
        Ok(())
    }

    pub fn search(&mut self, key: &IndexKey) -> Result<Vec<(u32, u16)>, IndexError> {
        let leaf_page = self.find_leaf(key)?;
        let page = self.pool.fetch_page(leaf_page)?;
        let entries = read_leaf_entries(&page)?;

        Ok(entries
            .into_iter()
            .filter(|entry| &entry.key == key)
            .map(|entry| (entry.page_index, entry.slot))
            .collect())
    }

    /// Each bound is (key, inclusive). None means unbounded on that side.
    pub fn range_search(
        &mut self,
        lower: Option<(&IndexKey, bool)>,
        upper: Option<(&IndexKey, bool)>,
    ) -> Result<Vec<(u32, u16)>, IndexError> {
        let mut current = match lower {
            Some((key, _)) => self.find_leaf(key)?,
            None => self.leftmost_leaf()?,
        };

        let mut results = Vec::new();

        loop {
            let page = self.pool.fetch_page(current)?;
            let header = read_header(&page)?;
            let entries = read_leaf_entries(&page)?;

            for entry in &entries {
                if let Some((bound, inclusive)) = lower {
                    let too_small = if inclusive {
                        entry.key < *bound
                    } else {
                        entry.key <= *bound
                    };
                    if too_small {
                        continue;
                    }
                }
                if let Some((bound, inclusive)) = upper {
                    let too_big = if inclusive {
                        entry.key > *bound
                    } else {
                        entry.key >= *bound
                    };
                    if too_big {
                        return Ok(results);
                    }
                }
                results.push((entry.page_index, entry.slot));
            }

            match header.next_leaf {
                Some(next) => current = next,
                None => return Ok(results),
            }
        }
    }

    fn find_leaf(&mut self, key: &IndexKey) -> Result<u32, IndexError> {
        let path = self.path_to_leaf(key)?;
        Ok(*path.last().unwrap())
    }

    fn leftmost_leaf(&mut self) -> Result<u32, IndexError> {
        let mut current = ROOT_PAGE;
        loop {
            let page = self.pool.fetch_page(current)?;
            let header = read_header(&page)?;
            if header.is_leaf {
                return Ok(current);
            }
            current = header.leftmost_child;
        }
    }

    pub fn insert(&mut self, key: IndexKey, page_index: u32, slot: u16) -> Result<(), IndexError> {
        let path = self.path_to_leaf(&key)?;
        let leaf_page = *path.last().expect("path always has at least the root");

        let leaf = self.pool.fetch_page(leaf_page)?;
        let next_leaf = read_header(&leaf)?.next_leaf;
        let mut entries = read_leaf_entries(&leaf)?;
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

        match build_leaf_page(&entries, next_leaf) {
            Ok(mut page) => {
                self.write_page_durable(leaf_page, &mut page)?;
                Ok(())
            }
            Err(IndexError::NodeFull) => self.split_leaf(&path, entries, next_leaf),
            Err(err) => Err(err),
        }
    }

    pub fn delete(
        &mut self,
        key: &IndexKey,
        page_index: u32,
        slot: u16,
    ) -> Result<bool, IndexError> {
        let leaf_page = self.find_leaf(key)?;
        let leaf = self.pool.fetch_page(leaf_page)?;
        let next_leaf = read_header(&leaf)?.next_leaf;
        let mut entries = read_leaf_entries(&leaf)?;

        let before = entries.len();
        entries.retain(|e| !(&e.key == key && e.page_index == page_index && e.slot == slot));
        if entries.len() == before {
            return Ok(false);
        }

        let mut page = build_leaf_page(&entries, next_leaf)?;
        self.write_page_durable(leaf_page, &mut page)?;
        Ok(true)
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

    fn split_leaf(
        &mut self,
        path: &[u32],
        entries: Vec<LeafEntry>,
        old_next: Option<u32>,
    ) -> Result<(), IndexError> {
        let leaf_page = *path.last().unwrap();
        let mid = entries.len() / 2;
        let left_entries: Vec<LeafEntry> = entries[..mid].to_vec();
        let right_entries: Vec<LeafEntry> = entries[mid..].to_vec();
        let separator = right_entries[0].key.clone();

        if leaf_page == ROOT_PAGE {
            // both halves relocate to fresh pages; page 0 becomes an internal root
            let left_index = self.pool.page_count();
            let right_index = left_index + 1;

            let mut left_page = build_leaf_page(&left_entries, Some(right_index))?;
            let mut right_page = build_leaf_page(&right_entries, old_next)?;

            self.write_page_durable(left_index, &mut left_page)?;
            self.write_page_durable(right_index, &mut right_page)?;

            self.write_new_root(left_index, right_index, separator)
        } else {
            // left stays put, right takes the next free page
            let right_index = self.pool.page_count();

            let mut left_page = build_leaf_page(&left_entries, Some(right_index))?;
            let mut right_page = build_leaf_page(&right_entries, old_next)?;

            self.write_page_durable(leaf_page, &mut left_page)?;
            self.write_page_durable(right_index, &mut right_page)?;

            self.propagate(&path[..path.len() - 1], separator, right_index)
        }
    }

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

        let left_entries: Vec<InternalEntry> = entries[..mid].to_vec();
        let right_entries: Vec<InternalEntry> = entries[mid + 1..].to_vec();

        if node_page == ROOT_PAGE {
            let left_index = self.pool.page_count();
            let right_index = left_index + 1;

            let mut left_page = build_internal_page(&left_entries, leftmost_child)?;
            let mut right_page = build_internal_page(&right_entries, promoted.child_page)?;

            self.write_page_durable(left_index, &mut left_page)?;
            self.write_page_durable(right_index, &mut right_page)?;

            self.write_new_root(left_index, right_index, promoted.key)
        } else {
            let right_index = self.pool.page_count();

            let mut left_page = build_internal_page(&left_entries, leftmost_child)?;
            let mut right_page = build_internal_page(&right_entries, promoted.child_page)?;

            self.write_page_durable(node_page, &mut left_page)?;
            self.write_page_durable(right_index, &mut right_page)?;

            self.propagate(&path[..path.len() - 1], promoted.key, right_index)
        }
    }

    /// Overwrites page 0 with a fresh internal root routing between two
    /// already-written child pages. Shared by leaf-splits and internal-splits
    /// of the root — next_leaf wiring (if any) is already baked into the
    /// child pages by the caller before this runs.
    fn write_new_root(
        &mut self,
        left: u32,
        right: u32,
        separator: IndexKey,
    ) -> Result<(), IndexError> {
        let mut new_root = build_internal_page(
            &[InternalEntry {
                key: separator,
                child_page: right,
            }],
            left,
        )?;
        self.write_page_durable(ROOT_PAGE, &mut new_root)?;
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

    #[test]
    fn range_search_returns_only_keys_in_bounds() -> Result<(), IndexError> {
        let dir = tempfile::tempdir().unwrap();
        let mut tree = BTree::open("test", dir.path())?;

        for i in 0..500i64 {
            tree.insert(IndexKey::Int(i), i as u32, 0)?;
        }

        let lower = IndexKey::Int(100);
        let upper = IndexKey::Int(110);
        let results = tree.range_search(Some((&lower, true)), Some((&upper, false)))?;

        let mut keys: Vec<u32> = results.iter().map(|(p, _)| *p).collect();
        keys.sort();

        assert_eq!(
            keys,
            (100..110).collect::<Vec<u32>>(),
            "expected [100, 110) inclusive-lower/exclusive-upper"
        );
        Ok(())
    }

    #[test]
    fn range_search_unbounded_lower() -> Result<(), IndexError> {
        let dir = tempfile::tempdir().unwrap();
        let mut tree = BTree::open("test", dir.path())?;

        for i in 0..50i64 {
            tree.insert(IndexKey::Int(i), i as u32, 0)?;
        }

        let upper = IndexKey::Int(5);
        let results = tree.range_search(None, Some((&upper, true)))?;

        let mut keys: Vec<u32> = results.iter().map(|(p, _)| *p).collect();
        keys.sort();

        assert_eq!(keys, vec![0, 1, 2, 3, 4, 5]);
        Ok(())
    }

    #[test]
    fn range_search_unbounded_upper_crosses_multiple_leaves() -> Result<(), IndexError> {
        let dir = tempfile::tempdir().unwrap();
        let mut tree = BTree::open("test", dir.path())?;

        for i in 0..500i64 {
            tree.insert(IndexKey::Int(i), i as u32, 0)?;
        }

        let lower = IndexKey::Int(490);
        let results = tree.range_search(Some((&lower, true)), None)?;

        let mut keys: Vec<u32> = results.iter().map(|(p, _)| *p).collect();
        keys.sort();

        assert_eq!(keys, (490..500).collect::<Vec<u32>>());
        Ok(())
    }
}
