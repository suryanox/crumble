use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use crate::error::WalError;
use crate::record::WalRecord;

pub struct WalWriter {
    file: File,
}

impl WalWriter {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WalError> {
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)?;

        Ok(Self { file })
    }

    pub fn append(&mut self, record: &WalRecord) -> Result<(), WalError> {
        let bytes = bincode::serde::encode_to_vec(
            &record,
            bincode::config::standard()
        ).map_err(|e| WalError::Encoding(e.to_string()))?;

        let len = bytes.len() as u32;
        self.file.write_all(&len.to_le_bytes())?;
        self.file.write_all(&bytes)?;
        self.file.sync_data()?;

        Ok(())

    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::read_all;
    use crumble_storage::{Row, Value};
    use std::fs::OpenOptions;
    use std::io::Write;

    #[test]
    fn round_trips_records() -> Result<(), WalError> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("test.wal");

        let mut writer = WalWriter::open(&path)?;
        writer.append(&WalRecord::Insert {
            table: "users".to_string(),
            row: Row::new(vec![Value::String("alice".to_string()), Value::Int(35)]),
        })?;
        writer.append(&WalRecord::Insert {
            table: "users".to_string(),
            row: Row::new(vec![Value::String("bob".to_string()), Value::Int(22)]),
        })?;

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
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("test.wal");

        {
            let mut writer = WalWriter::open(&path)?;
            writer.append(&WalRecord::Insert {
                table: "users".to_string(),
                row: Row::new(vec![Value::String("alice".to_string()), Value::Int(35)]),
            })?;
        }

        // simulate a crash mid-write: a length prefix claiming a big payload,
        // but the file ends long before that many bytes actually exist.
        let mut file = OpenOptions::new().append(true).open(&path)?;
        file.write_all(&100u32.to_le_bytes())?;
        file.write_all(b"only a few bytes")?;

        let records = read_all(&path)?;

        assert_eq!(records.len(), 1, "the complete first record should still recover");
        Ok(())
    }
}