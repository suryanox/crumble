use crate::page_store::PageStore;
use crate::{BufferError, Page};
use std::collections::{HashMap, VecDeque};
use std::path::Path;

#[derive(Debug)]
pub struct BufferPool {
    store: PageStore,
    capacity: usize,
    frames: HashMap<u32, Page>,
    lru_order: VecDeque<u32>,
}

impl BufferPool {
    pub fn open(path: impl AsRef<Path>, capacity: usize) -> Result<Self, BufferError> {
        Ok(Self {
            store: PageStore::open(path)?,
            capacity,
            frames: HashMap::new(),
            lru_order: VecDeque::new(),
        })
    }


    pub fn disk_read_count(&self) -> u64 {
        self.store.read_count()
    }

    pub fn page_count(&mut self) -> Result<u32, BufferError> {
        self.store.page_count()
    }

    // cache hit: clone and return and move it at back of lru, cache miss/real disk read
    pub fn fetch_page(&mut self, page_index: u32) -> Result<Page, BufferError> {
        if let Some(page) = self.frames.get(&page_index) {
            let page = page.clone();
            self.touch(page_index);
            return Ok(page);
        }

        let page = self.store.read_page(page_index)?;
        self.insert_into_cache(page_index, page.clone());
        Ok(page)
    }

    // writes through to disk and updates the cache in the same call, so a read immediately after a write never sees stale cached data.
    pub fn write_page(&mut self, page_index: u32, page: &Page) -> Result<(), BufferError> {
        self.store.write_page(page_index, page)?;
        self.insert_into_cache(page_index, page.clone());
        Ok(())
    }

    fn touch(&mut self, page_index: u32) {
        self.lru_order.retain(|&x| x != page_index);
        self.lru_order.push_back(page_index);
    }

    fn insert_into_cache(&mut self, page_index: u32, page: Page) {
        if !self.frames.contains_key(&page_index) && self.frames.len() >= self.capacity {
            if let Some(evicted) = self.lru_order.pop_front() {
                self.frames.remove(&evicted);
            }
        }

        self.frames.insert(page_index, page);
        self.touch(page_index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_fetch_hits_disk_once() -> Result<(), BufferError> {
        let dir = tempfile::tempdir()?;
        let mut pool = BufferPool::open(dir.path().join("test.pages"), 8)?;

        let mut page = Page::new();
        page.insert_row(b"hello");
        pool.write_page(0, &page)?;

        assert_eq!(pool.disk_read_count(), 0);

        pool.fetch_page(0)?;
        assert_eq!(pool.disk_read_count(), 0, "write_page should have cached it already");

        pool.fetch_page(0)?;
        pool.fetch_page(0)?;
        assert_eq!(pool.disk_read_count(), 0, "repeated fetches should stay cache hits");

        Ok(())
    }

    #[test]
    fn evicted_page_forces_real_disk_read() -> Result<(), BufferError> {
        let dir = tempfile::tempdir()?;
        let mut pool = BufferPool::open(dir.path().join("test.pages"), 2)?;

        for i in 0..3u32 {
            let mut page = Page::new();
            page.insert_row(format!("row-{i}").as_bytes());
            pool.write_page(i, &page)?;
        }

        // capacity is 2, so page 0 was evicted when page 2 was written
        pool.fetch_page(0)?;
        assert_eq!(pool.disk_read_count(), 1, "page 0 should have been evicted and re-read");

        Ok(())
    }
}
