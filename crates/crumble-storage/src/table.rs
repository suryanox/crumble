use crate::error::StorageError;
use crate::row::Row;
use crumble_buffer::BufferPool;
use crumble_wal::{WalRecord, WalWriter, read_all};
use std::path::Path;

const BUFFER_CAPACITY: usize = 64;

#[derive(Debug)]
pub struct Table {
    name: String,
    columns: Vec<String>,
    pool: BufferPool,
    wal: WalWriter,
}

impl Table {
    pub fn open(
        name: impl Into<String>,
        columns: Vec<String>,
        dir: impl AsRef<Path>,
    ) -> Result<Self, StorageError> {
        let name = name.into();
        let dir = dir.as_ref();

        let table_path = dir.join(format!("{name}.tbl"));
        let wal_path = dir.join(format!("{name}.wal"));

        let pool = BufferPool::open(&table_path, BUFFER_CAPACITY)?;
        let wal = WalWriter::open(&wal_path)?;

        let mut table = Self {
            name,
            columns,
            pool,
            wal,
        };

        for record in read_all(&wal_path)? {
            let WalRecord::Insert { row_bytes, .. } = record;
            table.apply_insert_bytes(&row_bytes)?;
        }

        Ok(table)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    pub fn insert(&mut self, row: Row) -> Result<(), StorageError> {
        if row.values().len() != self.columns.len() {
            return Err(StorageError::ColumnCountMismatch {
                expected: self.columns.len(),
                actual: row.values().len(),
            });
        }

        let bytes = row.to_bytes()?;

        self.wal.append(&WalRecord::Insert {
            table: self.name.clone(),
            row_bytes: bytes.clone(),
        })?;

        self.apply_insert_bytes(&bytes)
    }

    fn apply_insert_bytes(&mut self, bytes: &[u8]) -> Result<(), StorageError> {
        let page_count = self.pool.page_count();

        if page_count > 0 {
            let last_index = page_count - 1;
            let mut page = self.pool.fetch_page(last_index)?;

            if page.insert_row(bytes).is_some() {
                return Ok(self.pool.write_page(last_index, &page)?);
            }
        }

        let mut page = crumble_buffer::Page::new();
        if page.insert_row(bytes).is_none() {
            return Err(StorageError::RowTooLarge);
        }
        Ok(self.pool.write_page(page_count, &page)?)
    }

    pub fn rows(&mut self) -> Result<Vec<Row>, StorageError> {
        let mut rows = Vec::new();
        let page_count = self.pool.page_count();

        for page_index in 0..page_count {
            let page = self.pool.fetch_page(page_index)?;

            for slot in 0..page.slot_count() {
                let bytes = page
                    .get_row(slot)
                    .expect("slot index within slot_count must be valid");
                rows.push(Row::from_bytes(bytes)?);
            }
        }

        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;

    fn temp_table(columns: Vec<String>) -> (tempfile::TempDir, Table) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.tbl");
        let table = Table::open("users", columns, dir.path()).unwrap();
        (dir, table)
    }

    #[test]
    fn insert_rejects_wrong_column_count() {
        let (_dir, mut table) = temp_table(vec!["name".to_string()]);
        let row = Row::new(vec![Value::String("a".to_string()), Value::Int(1)]);

        let result = table.insert(row);

        assert!(matches!(
            result,
            Err(StorageError::ColumnCountMismatch {
                expected: 1,
                actual: 2
            })
        ));
    }

    #[test]
    fn insert_accepts_matching_row() -> Result<(), StorageError> {
        let (_dir, mut table) = temp_table(vec!["name".to_string()]);
        table.insert(Row::new(vec![Value::String("alice".to_string())]))?;

        assert_eq!(table.rows()?.len(), 1);
        Ok(())
    }
}
