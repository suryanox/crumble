pub const PAGE_SIZE: usize = 4096;
const HEADER_SIZE: usize = 4;
const SLOT_SIZE: usize = 4;
#[derive(Debug, Clone)]
pub struct Page {
    bytes: [u8; PAGE_SIZE],
}

impl Page {
    // first 4 bytes of the page are the header
    pub fn new() -> Self {
        let mut bytes = [0u8; PAGE_SIZE];
        bytes[2..4].copy_from_slice(&(PAGE_SIZE as u16).to_le_bytes());
        Self { bytes }
    }

    pub fn slot_count(&self) -> u16 {
        u16::from_le_bytes([self.bytes[0], self.bytes[1]])
    }

    fn set_slot_count(&mut self, count: u16) {
        self.bytes[0..2].copy_from_slice(&count.to_le_bytes());
    }

    fn free_space_offset(&self) -> u16 {
        u16::from_le_bytes([self.bytes[2], self.bytes[3]])
    }

    fn set_free_space_offset(&mut self, offset: u16) {
        self.bytes[2..4].copy_from_slice(&offset.to_le_bytes());
    }

    pub fn insert_row(&mut self, data: &[u8]) -> Option<u16> {
        let slot_count = self.slot_count();

        // where the slot directory currently ends (front of page, growing forward)
        let slot_dir_end = HEADER_SIZE + (slot_count as usize) * SLOT_SIZE;

        // same as slot_dir_end, but including the slot we're about to add the boundary check needs to account for it, not just existing slots
        let new_slot_dir_end = slot_dir_end + SLOT_SIZE;

        let free_space_offset = self.free_space_offset() as usize;

        let row_len = data.len();

        if new_slot_dir_end + row_len > free_space_offset {
            return None;
        }

        // rows grow backward, so the new row starts row_len bytes before the current free-space boundary
        let row_offset = free_space_offset - row_len;

        // writing the new slot's offset then length, 2 bytes each, directly into the page's own byte array literally the slot directory living inside bytes, like we corrected it to
        self.bytes[row_offset..free_space_offset].copy_from_slice(data);

        self.bytes[slot_dir_end..slot_dir_end + 2]
            .copy_from_slice(&(row_offset as u16).to_le_bytes());

        self.bytes[slot_dir_end + 2..slot_dir_end + 4]
            .copy_from_slice(&(row_len as u16).to_le_bytes());

        self.set_free_space_offset(row_offset as u16);
        self.set_slot_count(slot_count + 1);

        Some(slot_count)
    }

    pub fn get_row(&self, slot_index: u16) -> Option<&[u8]> {
        // Bounds check first
        if slot_index >= self.slot_count() {
            return None;
        }

        // slot_offset same formula as insert_row's slot_dir_end, but for one specific slot: header, then skip past slot_index earlier slots (each SLOT_SIZE bytes), to land exactly on this slot's 4 bytes.
        let slot_offset = HEADER_SIZE + (slot_index as usize) * SLOT_SIZE;

        // Read row_offset/row_len back out  mirror image of insert_row's writes: same two u16::from_le_bytes reads we already used for the header, just at a different position.
        let row_offset =
            u16::from_le_bytes([self.bytes[slot_offset], self.bytes[slot_offset + 1]]) as usize;
        let row_len =
            u16::from_le_bytes([self.bytes[slot_offset + 2], self.bytes[slot_offset + 3]]) as usize;

        // Returning a slice, not a copy &[u8] borrows straight from self.bytes, zero-copy. Whoever calls this decides whether to clone/deserialize it.
        Some(&self.bytes[row_offset..row_offset + row_len])
    }

    pub fn from_bytes(bytes: [u8; PAGE_SIZE]) -> Self {
        Self { bytes }
    }

    pub fn as_bytes(&self) -> &[u8; PAGE_SIZE] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_row() {
        let mut page = Page::new();
        let data = b"hello";

        let slot = page.insert_row(data).expect("page has room");

        assert_eq!(page.get_row(slot), Some(data.as_slice()));
    }

    #[test]
    fn round_trips_multiple_rows() {
        let mut page = Page::new();

        let slot_a = page.insert_row(b"alice").unwrap();
        let slot_b = page.insert_row(b"bob").unwrap();

        assert_eq!(page.get_row(slot_a), Some(b"alice".as_slice()));
        assert_eq!(page.get_row(slot_b), Some(b"bob".as_slice()));
    }

    #[test]
    fn get_row_out_of_range_is_none() {
        let page = Page::new();
        assert_eq!(page.get_row(0), None);
    }

    #[test]
    fn insert_fails_when_page_is_full() {
        let mut page = Page::new();
        let big_row = vec![0u8; PAGE_SIZE];

        assert_eq!(page.insert_row(&big_row), None);
    }
}
