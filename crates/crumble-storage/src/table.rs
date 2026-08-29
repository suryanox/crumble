use crate::column::ColumnDef;
use crate::error::StorageError;
use crate::row::Row;
use crumble_buffer::BufferPool;
use crumble_tx::{TransactionId, TransactionManager, TxStatus, is_visible};
use crumble_wal::{WalRecord, WalWriter, read_all};
use std::path::Path;
use std::sync::{Arc, Mutex};

const BUFFER_CAPACITY: usize = 64;
#[derive(Debug)]
pub struct Table {
    name: String,
    columns: Vec<ColumnDef>,
    pool: BufferPool,
    wal: WalWriter,
    tx_manager: Arc<TransactionManager>,
}

impl Table {
    pub fn open(
        name: impl Into<String>,
        columns: Vec<ColumnDef>,
        dir: impl AsRef<Path>,
        tx_manager: Arc<TransactionManager>,
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
            tx_manager,
        };

        // replay
        for (lsn, record) in read_all(&wal_path)? {
            match record {
                WalRecord::Insert {
                    page_index,
                    row_bytes,
                    ..
                } => {
                    let already_durable = page_index < table.pool.page_count()
                        && table.pool.fetch_page(page_index)?.page_lsn() >= lsn;

                    if !already_durable {
                        table.apply_at(page_index, &row_bytes, lsn)?;
                    }
                }
                WalRecord::Delete {
                    page_index, slot, ..
                } => {
                    let already_durable = page_index < table.pool.page_count()
                        && table.pool.fetch_page(page_index)?.page_lsn() >= lsn;

                    if !already_durable {
                        table.apply_delete_at(page_index, lsn, slot)?;
                    }
                }
                WalRecord::WritePage { .. } => {
                    unreachable!("Table's WAL never writes WritePage records — that's BTree-only")
                }
            }
        }

        Ok(table)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    // Table = heap access method. Fully ignorant indexes exist. Just got its insert changed to return (page_index, slot)
    // not because it cares about indexing, but because it's the only one who knows where a row landed, and someone downstream will need that.
    pub fn insert(&mut self, mut row: Row, xid: TransactionId) -> Result<(u32, u16), StorageError> {
        if row.values().len() != self.columns.len() {
            return Err(StorageError::ColumnCountMismatch {
                expected: self.columns.len(),
                actual: row.values().len(),
            });
        }

        for (value, col) in row.values().iter().zip(self.columns.iter()) {
            if !col.matches(value) {
                return Err(StorageError::TypeMismatch {
                    column: col.name.clone(),
                    expected: format!("{:?}", col.ty),
                });
            }
        }

        row.xmin = xid;
        let bytes = row.to_bytes()?;
        let (page_index, slot, mut page) = self.prepare_insert(&bytes)?;

        let lsn = self.wal.append(&WalRecord::Insert {
            table: self.name.clone(),
            page_index,
            row_bytes: bytes,
        })?;
        page.set_page_lsn(lsn);
        self.pool.write_page(page_index, &page)?;
        Ok((page_index, slot))
    }

    /// Decides which page a new row belongs on, and returns that page
    /// with the row already inserted into it — NOT yet written to the pool.
    /// Logging happens against this decision before it's committed.

    fn prepare_insert(
        &mut self,
        bytes: &[u8],
    ) -> Result<(u32, u16, crumble_buffer::Page), StorageError> {
        let page_count = self.pool.page_count();

        if page_count > 0 {
            let last_index = page_count - 1;
            let mut page = self.pool.fetch_page(last_index)?;

            if let Some(slot) = page.insert_row(bytes) {
                return Ok((last_index, slot, page));
            }
        }

        let mut page = crumble_buffer::Page::new();
        let slot = page.insert_row(bytes).ok_or(StorageError::RowNotFound)?;
        Ok((page_count, slot, page))
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
            return Err(StorageError::RowNotFound);
        }

        page.set_page_lsn(lsn);
        Ok(self.pool.write_page(page_index, &page)?)
    }

    pub fn rows(&mut self, reader: TransactionId) -> Result<Vec<Row>, StorageError> {
        let mut rows = Vec::new();
        let page_count = self.pool.page_count();

        for page_index in 0..page_count {
            let page = self.pool.fetch_page(page_index)?;
            for slot in 0..page.slot_count() {
                if let Some(bytes) = page.get_row(slot) {
                    let row = Row::from_bytes(bytes)?;
                    if is_visible(row.xmin, row.xmax, reader, &self.tx_manager) {
                        rows.push(row);
                    }
                }
            }
        }

        Ok(rows)
    }

    pub fn rows_with_location(
        &mut self,
        reader: TransactionId,
    ) -> Result<Vec<((u32, u16), Row)>, StorageError> {
        let mut rows = Vec::new();
        let page_count = self.pool.page_count();

        for page_index in 0..page_count {
            let page = self.pool.fetch_page(page_index)?;
            for slot in 0..page.slot_count() {
                if let Some(bytes) = page.get_row(slot) {
                    let row = Row::from_bytes(bytes)?;
                    if is_visible(row.xmin, row.xmax, reader, &self.tx_manager) {
                        rows.push(((page_index, slot), row));
                    }
                }
            }
        }
        Ok(rows)
    }

    fn apply_delete_at(
        &mut self,
        page_index: u32,
        lsn: u64,
        slot: u16,
    ) -> Result<(), StorageError> {
        let mut page = self.pool.fetch_page(page_index)?;
        page.delete_row(slot);
        page.set_page_lsn(lsn);
        Ok(self.pool.write_page(page_index, &page)?)
    }

    pub fn row_at(
        &mut self,
        page_index: u32,
        slot: u16,
        reader: TransactionId,
    ) -> Result<Option<Row>, StorageError> {
        if page_index >= self.pool.page_count() {
            return Ok(None);
        }

        let page = self.pool.fetch_page(page_index)?;
        match page.get_row(slot) {
            Some(bytes) => {
                let row = Row::from_bytes(bytes)?;
                if is_visible(row.xmin, row.xmax, reader, &self.tx_manager) {
                    Ok(Some(row))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }
}

pub fn delete_at(
    table: &Arc<Mutex<Table>>,
    page_index: u32,
    slot: u16,
    xid: TransactionId,
) -> Result<(), StorageError> {
    loop {
        let conflict = {
            let mut t = table.lock().unwrap();
            let page = t.pool.fetch_page(page_index)?;
            let bytes = page.get_row(slot).ok_or(StorageError::RowNotFound)?;
            let row = Row::from_bytes(bytes)?;

            match row.xmax {
                Some(other)
                    if other != xid && t.tx_manager.status(other) == Some(TxStatus::InProgress) =>
                {
                    Some(other) // signal: need to wait, outside the lock
                }
                Some(other)
                    if other != xid && t.tx_manager.status(other) == Some(TxStatus::Committed) =>
                {
                    return Err(StorageError::ConcurrentModification);
                }
                _ => {
                    let mut updated_row = row;
                    updated_row.xmax = Some(xid);
                    let updated_bytes = updated_row.to_bytes()?;

                    let mut page = page;
                    page.delete_row(slot);
                    let table_name = t.name.clone();
                    let lsn = t.wal.append(&WalRecord::Insert {
                        table: table_name,
                        page_index,
                        row_bytes: updated_bytes,
                    })?;
                    page.set_page_lsn(lsn);
                    t.pool.write_page(page_index, &page)?;
                    return Ok(());
                }
            }
        }; // <-- physical lock (`t`) is dropped here, at the end of this block

        if let Some(other) = conflict {
            let tx_manager = table.lock().unwrap().tx_manager.clone();
            tx_manager.wait_for(other); // blocked here — table is NOT locked during this
            // loop back around: re-lock, re-fetch, re-check from scratch
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::{ColumnType, col};
    use crate::value::Value;

    fn temp_table(columns: Vec<ColumnDef>) -> (tempfile::TempDir, Table, TransactionId) {
        let dir = tempfile::tempdir().unwrap();
        let tx_manager = Arc::new(TransactionManager::new());
        let table = Table::open("users", columns, dir.path(), Arc::clone(&tx_manager)).unwrap();
        let xid = tx_manager.begin();
        (dir, table, xid)
    }

    #[test]
    fn insert_rejects_wrong_column_count() {
        let (_dir, mut table, xid) = temp_table(vec![col("name", ColumnType::String)]);
        let row = Row::new(vec![Value::String("a".to_string()), Value::Int(1)]);

        let result = table.insert(row, xid);

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
        let (_dir, mut table, xid) = temp_table(vec![col("name", ColumnType::String)]);
        table.insert(Row::new(vec![Value::String("alice".to_string())]), xid)?;

        assert_eq!(table.rows(xid)?.len(), 1);
        Ok(())
    }

    #[test]
    fn recovers_dirty_writes_after_simulated_crash() -> Result<(), StorageError> {
        let dir = tempfile::tempdir()?;
        let tx_manager = Arc::new(TransactionManager::new());
        let xid = tx_manager.begin();

        {
            let mut table = Table::open(
                "users",
                vec![col("name", ColumnType::String)],
                dir.path(),
                Arc::clone(&tx_manager),
            )?;
            table.insert(Row::new(vec![Value::String("alice".to_string())]), xid)?;
            table.insert(Row::new(vec![Value::String("bob".to_string())]), xid)?;
        }

        let mut recovered = Table::open(
            "users",
            vec![col("name", ColumnType::String)],
            dir.path(),
            Arc::clone(&tx_manager),
        )?;
        let rows = recovered.rows(xid)?;

        assert_eq!(
            rows.len(),
            2,
            "both writes should recover from the WAL alone"
        );
        assert_eq!(rows[0].values(), &[Value::String("alice".to_string())]);
        assert_eq!(rows[1].values(), &[Value::String("bob".to_string())]);

        Ok(())
    }

    #[test]
    fn replay_does_not_duplicate_already_flushed_rows() -> Result<(), StorageError> {
        let dir = tempfile::tempdir()?;
        let tx_manager = Arc::new(TransactionManager::new());
        let xid = tx_manager.begin();

        {
            let mut table = Table::open(
                "users",
                vec![col("name", ColumnType::String)],
                dir.path(),
                Arc::clone(&tx_manager),
            )?;
            table.insert(Row::new(vec![Value::String("alice".to_string())]), xid)?;
            for i in 0..BUFFER_CAPACITY {
                table.insert(Row::new(vec![Value::String(format!("filler-{i}"))]), xid)?;
            }
        }

        let mut recovered = Table::open(
            "users",
            vec![col("name", ColumnType::String)],
            dir.path(),
            Arc::clone(&tx_manager),
        )?;
        let rows = recovered.rows(xid)?;
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
