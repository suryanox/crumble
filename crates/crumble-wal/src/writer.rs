use crate::error::WalError;
use crate::record::WalRecord;
use std::fs::{File, OpenOptions};
use std::io::{Seek, Write};
use std::path::Path;

#[derive(Debug)]
pub struct WalWriter {
    file: File,
}

impl WalWriter {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WalError> {
        let file = OpenOptions::new().append(true).create(true).open(path)?;

        Ok(Self { file })
    }

    pub fn append(&mut self, record: &WalRecord) -> Result<u64, WalError> {
        // stream_position() reads the current file cursor position before writing
        // since we only ever append, that position is exactly
        // "how many bytes exist before this record,"
        // which is a perfectly good, simple LSN. Captured before the write,
        // not after, since it needs to be this record's starting offset.
        let lsn = self.file.stream_position()?;
        let bytes = bincode::serde::encode_to_vec(&record, bincode::config::standard())
            .map_err(|e| WalError::Encoding(e.to_string()))?;

        let len = bytes.len() as u32;
        self.file.write_all(&len.to_le_bytes())?;
        self.file.write_all(&bytes)?;
        self.file.sync_data()?;

        Ok(lsn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::read_all;
    use std::fs::OpenOptions;
    use std::io::Write;

    #[test]
    fn round_trips_records() -> Result<(), WalError> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let mut writer = WalWriter::open(&path)?;
        let lsn_a = writer.append(&WalRecord::Insert {
            table: "users".to_string(),
            page_index: 0,
            row_bytes: b"alice,35".to_vec(),
        })?;
        let lsn_b = writer.append(&WalRecord::Insert {
            table: "users".to_string(),
            page_index: 0,
            row_bytes: b"bob,22".to_vec(),
        })?;

        assert!(lsn_b > lsn_a, "LSNs must be strictly increasing");

        let records = read_all(&path)?;
        assert_eq!(records.len(), 2);
        Ok(())
    }

    #[test]
    fn missing_wal_file_reads_as_empty() -> Result<(), WalError> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.wal");

        let records = read_all(&path)?;
        assert!(records.is_empty());
        Ok(())
    }

    #[test]
    fn stops_cleanly_at_torn_record() -> Result<(), WalError> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        {
            let mut writer = WalWriter::open(&path)?;
            writer.append(&WalRecord::Insert {
                table: "users".to_string(),
                page_index: 0,
                row_bytes: b"alice,35".to_vec(),
            })?;
        }

        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&100u32.to_le_bytes()).unwrap();
        file.write_all(b"only a few bytes").unwrap();

        let records = read_all(&path)?;

        assert_eq!(
            records.len(),
            1,
            "the complete first record should still recover"
        );
        Ok(())
    }
}
