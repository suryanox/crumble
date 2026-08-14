use crate::page_store::PageStore;
use crate::{BufferError, Page};
use std::collections::{HashMap, VecDeque};
use std::path::Path;

#[derive(Debug)]
struct Frame {
    page: Page,
    dirty: bool,
}

#[derive(Debug)]
pub struct BufferPool {
    store: PageStore,
    capacity: usize,
    frames: HashMap<u32, Frame>,
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
        if let Some(frame) = self.frames.get(&page_index) {
            let page = frame.page.clone();
            self.touch(page_index);
            return Ok(page);
        }

        let page = self.store.read_page(page_index)?;
        self.insert_into_cache(page_index, page.clone(), false)?;
        Ok(page)
    }

    /// Caches the page and marks it dirty. Does NOT write to disk yet —
    /// that only happens on eviction or an explicit flush.
    pub fn write_page(&mut self, page_index: u32, page: &Page) -> Result<(), BufferError> {
        self.insert_into_cache(page_index, page.clone(), true)
    }

    /// Writes one dirty page to disk immediately and clears its dirty flag.
    pub fn flush_page(&mut self, page_index: u32) -> Result<(), BufferError> {
        if let Some(frame) = self.frames.get(&page_index) {
            if frame.dirty {
                self.store.write_page(page_index, &frame.page)?;
                if let Some(frame) = self.frames.get_mut(&page_index) {
                    frame.dirty = false;
                }
            }
        }
        Ok(())
    }

    /// Writes every dirty cached page to disk. This is a checkpoint.
    pub fn flush_all(&mut self) -> Result<(), BufferError> {
        let dirty_indices: Vec<u32> = self
            .frames
            .iter()
            .filter(|(_, frame)| frame.dirty)
            .map(|(&index, _)| index)
            .collect();

        for index in dirty_indices {
            self.flush_page(index)?;
        }

        Ok(())
    }

    fn touch(&mut self, page_index: u32) {
        self.lru_order.retain(|&x| x != page_index);
        self.lru_order.push_back(page_index);
    }

    fn insert_into_cache(&mut self, page_index: u32, page: Page, dirty: bool) -> Result<(), BufferError> {
        if !self.frames.contains_key(&page_index) && self.frames.len() >= self.capacity {
            if let Some(evicted) = self.lru_order.pop_front() {
                self.flush_page(evicted)?; // this is the one line standing between "fast write-back cache" and "silently loses data."
                self.frames.remove(&evicted);
            }
        }

        // if a page is already cached and dirty (written but not yet flushed),
        // and gets written to again before eviction,
        // it must stay dirty (can't accidentally clear a pending write).
        // frame.dirty || dirty captures that: once dirty,
        // stays dirty until an explicit flush
        let entry_dirty = self
            .frames
            .get(&page_index)
            .map(|f| f.dirty || dirty)
            .unwrap_or(dirty);

        self.frames.insert(
            page_index,
            Frame {
                page,
                dirty: entry_dirty,
            },
        );
        self.touch(page_index);

        Ok(())
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
