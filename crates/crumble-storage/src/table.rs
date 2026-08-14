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

        for (lsn, record) in read_all(&wal_path)? {
            let WalRecord::Insert {
                page_index,
                row_bytes,
                ..
            } = record;

            // for each replayed record, read the page's current stamped LSN. If it's >=
            // this record's LSN, that record's effect is already baked into what's on disk (whether via eviction-flush or a clean shutdown)
            let already_durable = page_index < table.pool.page_count()
                && table.pool.fetch_page(page_index)?.page_lsn() >= lsn;

            if !already_durable {
                table.apply_at(page_index, &row_bytes, lsn)?;
            }
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
        let (page_index, mut page) = self.prepare_insert(&bytes)?;

        let lsn = self.wal.append(&WalRecord::Insert {
            table: self.name.clone(),
            page_index,
            row_bytes: bytes,
        })?;

        page.set_page_lsn(lsn);
        Ok(self.pool.write_page(page_index, &page)?)
    }

    /// Decides which page a new row belongs on, and returns that page
    /// with the row already inserted into it — NOT yet written to the pool.
    /// Logging happens against this decision before it's committed.
    fn prepare_insert(
        &mut self,
        bytes: &[u8],
    ) -> Result<(u32, crumble_buffer::Page), StorageError> {
        let page_count = self.pool.page_count();

        if page_count > 0 {
            let last_index = page_count - 1;
            let mut page = self.pool.fetch_page(last_index)?;

            if page.insert_row(bytes).is_some() {
                return Ok((last_index, page));
            }
        }

        let mut page = crumble_buffer::Page::new();
        if page.insert_row(bytes).is_none() {
            return Err(StorageError::RowTooLarge);
        }
        Ok((page_count, page))
    }

    /// Inserts bytes at an EXACT, already-decided page index used by WAL
    /// replay, which must reproduce the original page assignment exactly,
    /// not recompute a fresh one.
    fn apply_at(&mut self, page_index: u32, bytes: &[u8], lsn: u64) -> Result<(), StorageError> {
        let mut page = if page_index < self.pool.page_count() {
            self.pool.fetch_page(page_index)?
        } else {
            crumble_buffer::Page::new()
        };

        if page.insert_row(bytes).is_none() {
            return Err(StorageError::RowTooLarge);
        }

        page.set_page_lsn(lsn);
        Ok(self.pool.write_page(page_index, &page)?)
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

    #[test]
    fn recovers_dirty_writes_after_simulated_crash() -> Result<(), StorageError> {
        let dir = tempfile::tempdir()?;

        {
            let mut table = Table::open("users", vec!["name".to_string()], dir.path())?;
            table.insert(Row::new(vec![Value::String("alice".to_string())]))?;
            table.insert(Row::new(vec![Value::String("bob".to_string())]))?;
            // table is dropped here WITHOUT any explicit flush/checkpoint —
            // simulating a process crash right after these writes returned.
            // Both inserts are only durable via the WAL at this point.
        }

        let mut recovered = Table::open("users", vec!["name".to_string()], dir.path())?;
        let rows = recovered.rows()?;

        assert_eq!(
            rows.len(),
            2,
            "both writes should recover from the WAL alone"
        );
        assert_eq!(rows[0], Row::new(vec![Value::String("alice".to_string())]));
        assert_eq!(rows[1], Row::new(vec![Value::String("bob".to_string())]));

        Ok(())
    }

    #[test]
    fn replay_does_not_duplicate_already_flushed_rows() -> Result<(), StorageError> {
        let dir = tempfile::tempdir()?;

        {
            let mut table = Table::open("users", vec!["name".to_string()], dir.path())?;
            table.insert(Row::new(vec![Value::String("alice".to_string())]))?;
            // Force this page to actually flush to disk (not just cached-dirty),
            // by evicting it: fill the buffer pool past capacity with other pages.
            for i in 0..BUFFER_CAPACITY {
                table.insert(Row::new(vec![Value::String(format!("filler-{i}"))]))?;
            }
        }

        let mut recovered = Table::open("users", vec!["name".to_string()], dir.path())?;
        let rows = recovered.rows()?;
        let alice_count = rows
            .iter()
            .filter(|r| r.values() == [Value::String("alice".to_string())])
            .count();

        assert_eq!(
            alice_count, 1,
            "a row already flushed via eviction must not be replayed again"
        );
        Ok(())
    }
}
