use crate::{BufferError, PAGE_SIZE, Page};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

#[derive(Debug)]
pub struct PageStore {
    file: File,
    read_count: u64,
}

impl PageStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, BufferError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        Ok(Self { file, read_count: 0 })
    }

    pub fn read_page(&mut self, page_index: u32) -> Result<Page, BufferError> {
        // this is the heap-file addressing formula. Page 0 is bytes 0..4096, page 1 is 4096..8192, and so on. No index/lookup needed because pages are fixed-size and never move.
        let offset = (page_index as u64) * (PAGE_SIZE as u64);

        self.file.seek(SeekFrom::Start(offset))?;

        let mut bytes = [0u8; PAGE_SIZE];

        self.file.read_exact(&mut bytes)?;
        self.read_count += 1;

        Ok(Page::from_bytes(bytes))
    }

    pub fn read_count(&self) -> u64 {
        self.read_count
    }

    pub fn write_page(&mut self, page_index: u32, page: &Page) -> Result<(), BufferError> {
        let offset = (page_index as u64) * (PAGE_SIZE as u64);

        self.file.seek(SeekFrom::Start(offset))?;

        self.file.write_all(page.as_bytes())?;

        // the infamous fsync
        self.file.sync_data()?;

        Ok(())
    }

    pub fn page_count(&mut self) -> Result<u32, BufferError> {
        let len = self.file.seek(SeekFrom::End(0))?;
        Ok((len / PAGE_SIZE as u64) as u32)
    }
}
