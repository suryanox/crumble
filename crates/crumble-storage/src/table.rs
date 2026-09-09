use crate::column::ColumnDef;
use crate::error::StorageError;
use crate::row::Row;
use crumble_buffer::BufferPool;
use crumble_tx::{TransactionId, TransactionManager, TxStatus, is_visible};
use crumble_wal::{WalRecord, WalWriter, read_all};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

const BUFFER_CAPACITY: usize = 64;
const FROZEN_DEAD: u64 = u64::MAX;
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
                WalRecord::UpdateRow {
                    page_index,
                    slot,
                    row_bytes,
                    ..
                } => {
                    let already_durable = page_index < table.pool.page_count()
                        && table.pool.fetch_page(page_index)?.page_lsn() >= lsn;
                    if !already_durable {
                        table.apply_update_at(page_index, slot, &*row_bytes, lsn)?;
                    }
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
                    if row_is_visible(&row, reader, &self.tx_manager) {
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
                    if row_is_visible(&row, reader, &self.tx_manager) {
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
                if row_is_visible(&row, reader, &self.tx_manager) {
                    Ok(Some(row))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }

    pub fn all_rows_raw(&mut self) -> Result<Vec<((u32, u16), Row)>, StorageError> {
        let mut rows = Vec::new();
        let page_count = self.pool.page_count();

        for page_index in 0..page_count {
            let page = self.pool.fetch_page(page_index)?;
            for slot in 0..page.slot_count() {
                if let Some(bytes) = page.get_row(slot) {
                    rows.push(((page_index, slot), Row::from_bytes(bytes)?));
                }
            }
        }
        Ok(rows)
    }

    pub fn freeze(&mut self) -> Result<HashSet<TransactionId>, StorageError> {
        let raw_rows = self.all_rows_raw()?;
        let mut frozen_xids = HashSet::new();

        for ((page_index, slot), mut row) in raw_rows {
            let mut changed = false;

            if row.xmin != 0 && self.tx_manager.status(row.xmin) == Some(TxStatus::Committed) {
                frozen_xids.insert(row.xmin);
                row.xmin = 0;
                changed = true;
            }

            if row.xmax != 0
                && row.xmax != FROZEN_DEAD
                && self.tx_manager.status(row.xmax) == Some(TxStatus::Committed)
            {
                frozen_xids.insert(row.xmax);
                row.xmax = FROZEN_DEAD;
                changed = true;
            }

            if changed {
                let bytes = row.to_bytes()?;
                let lsn = self.wal.append(&WalRecord::UpdateRow {
                    table: self.name.clone(),
                    page_index,
                    slot,
                    row_bytes: bytes.clone(),
                })?;
                let mut page = self.pool.fetch_page(page_index)?;
                let overwrote = page.update_row(slot, &bytes);
                debug_assert!(
                    overwrote,
                    "freezing only touches fixed-size xmin/xmax fields"
                );
                page.set_page_lsn(lsn);
                self.pool.write_page(page_index, &page)?;
            }
        }

        Ok(frozen_xids)
    }

    fn apply_update_at(
        &mut self,
        page_index: u32,
        slot: u16,
        bytes: &[u8],
        lsn: u64,
    ) -> Result<(), StorageError> {
        let mut page = self.pool.fetch_page(page_index)?;
        let overwrote = page.update_row(slot, bytes);
        debug_assert!(
            overwrote,
            "WAL-replayed update must always fit the original slot's length"
        );
        page.set_page_lsn(lsn);
        Ok(self.pool.write_page(page_index, &page)?)
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

            let other = row.xmax;
            let held_by_someone_else = other != 0 && other != xid;

            if held_by_someone_else && t.tx_manager.status(other) == Some(TxStatus::InProgress) {
                // someone else has it claimed and hasn't resolved yet —
                // signal the outer loop to wait, don't touch the page.
                Some(other)
            } else if held_by_someone_else
                && t.tx_manager.status(other) == Some(TxStatus::Committed)
            {
                // someone else already committed a delete on this row — we lose.
                return Err(StorageError::ConcurrentModification);
            } else {
                // either: unclaimed (0), already ours, or the other claimant
                // aborted (their delete never really happened) — safe to proceed.
                let mut updated_row = row;
                updated_row.xmax = xid;
                let updated_bytes = updated_row.to_bytes()?;

                let mut page = page;
                let overwrote = page.update_row(slot, &updated_bytes);
                debug_assert!(
                    overwrote,
                    "xmax-only update must always fit the original slot's length"
                );

                let table_name = t.name.clone();
                let lsn = t.wal.append(&WalRecord::UpdateRow {
                    table: table_name,
                    page_index,
                    slot,
                    row_bytes: updated_bytes.clone(),
                })?;
                let mut page = page;
                page.update_row(slot, &updated_bytes);
                page.set_page_lsn(lsn);
                t.pool.write_page(page_index, &page)?;
                return Ok(());
            }
        }; // physical lock (`t`) is dropped here, at the end of this block

        if let Some(other) = conflict {
            let tx_manager = table.lock().unwrap().tx_manager.clone();

            if let Err(_deadlock_detected) = tx_manager.wait_for(xid, other) {
                tx_manager.abort(xid);
                return Err(StorageError::Deadlock);
            }
        }
    }
}

fn row_is_visible(row: &Row, reader: TransactionId, tx_manager: &TransactionManager) -> bool {
    if row.xmax == FROZEN_DEAD {
        return false; // permanently dead — no transaction lookup needed at all
    }
    let xmax = if row.xmax == 0 { None } else { Some(row.xmax) };
    is_visible(row.xmin, xmax, reader, tx_manager)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::{ColumnType, col};
    use crate::value::Value;
    use std::thread;
    use std::time::Duration;

    fn temp_table(columns: Vec<ColumnDef>) -> (tempfile::TempDir, Table, TransactionId) {
        let dir = tempfile::tempdir().unwrap();
        let tx_manager = Arc::new(
            TransactionManager::open(dir.path().join("tx.log")).expect("transaction log must open"),
        );
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
        let tx_manager = Arc::new(
            TransactionManager::open(dir.path().join("tx.log")).expect("transaction log must open"),
        );
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
        let tx_manager = Arc::new(
            TransactionManager::open(dir.path().join("tx.log")).expect("transaction log must open"),
        );
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

    #[test]
    fn concurrent_delete_blocks_then_conflicts_after_commit() -> Result<(), StorageError> {
        let dir = tempfile::tempdir().unwrap();
        let tx_manager = Arc::new(
            TransactionManager::open(dir.path().join("tx.log")).expect("transaction log must open"),
        );

        let setup_xid = tx_manager.begin();
        let table = Arc::new(Mutex::new(Table::open(
            "users",
            vec![col("name", ColumnType::String)],
            dir.path(),
            Arc::clone(&tx_manager),
        )?));
        let (page_index, slot) = {
            let mut t = table.lock().unwrap();
            t.insert(
                Row::new(vec![Value::String("alice".to_string())]),
                setup_xid,
            )?
        };
        tx_manager.commit(setup_xid);

        let xid_a = tx_manager.begin();
        delete_at(&table, page_index, slot, xid_a)?;

        let xid_b = tx_manager.begin();
        let table_for_b = Arc::clone(&table);
        let handle = thread::spawn(move || delete_at(&table_for_b, page_index, slot, xid_b));

        thread::sleep(Duration::from_millis(50));
        tx_manager.commit(xid_a);

        let result = handle.join().unwrap();
        assert!(
            matches!(result, Err(StorageError::ConcurrentModification)),
            "B must wake and find A's committed delete already won"
        );
        Ok(())
    }

    #[test]
    fn concurrent_delete_blocks_then_succeeds_after_abort() -> Result<(), StorageError> {
        let dir = tempfile::tempdir().unwrap();
        let tx_manager = Arc::new(
            TransactionManager::open(dir.path().join("tx.log")).expect("transaction log must open"),
        );

        let setup_xid = tx_manager.begin();
        let table = Arc::new(Mutex::new(Table::open(
            "users",
            vec![col("name", ColumnType::String)],
            dir.path(),
            Arc::clone(&tx_manager),
        )?));
        let (page_index, slot) = {
            let mut t = table.lock().unwrap();
            t.insert(
                Row::new(vec![Value::String("alice".to_string())]),
                setup_xid,
            )?
        };
        tx_manager.commit(setup_xid);

        let xid_a = tx_manager.begin();
        delete_at(&table, page_index, slot, xid_a)?;

        let xid_b = tx_manager.begin();
        let table_for_b = Arc::clone(&table);
        let handle = thread::spawn(move || delete_at(&table_for_b, page_index, slot, xid_b));

        thread::sleep(Duration::from_millis(50));
        tx_manager.abort(xid_a);

        let result = handle.join().unwrap();
        assert!(
            result.is_ok(),
            "B must wake and succeed once A's conflicting delete is undone"
        );
        Ok(())
    }

    #[test]
    fn concurrent_inserts_into_different_tables_do_not_corrupt_data() -> Result<(), StorageError> {
        let dir = tempfile::tempdir()?;
        let tx_manager = Arc::new(
            TransactionManager::open(dir.path().join("tx.log")).expect("transaction log must open"),
        );

        let mut handles = Vec::new();
        for table_num in 0..4 {
            let table_dir = dir.path().to_path_buf();
            let tx_manager = Arc::clone(&tx_manager);

            handles.push(thread::spawn(move || -> Result<(), StorageError> {
                let xid = tx_manager.begin();
                let mut table = Table::open(
                    format!("t{table_num}"),
                    vec![col("name", ColumnType::String)],
                    &table_dir,
                    Arc::clone(&tx_manager),
                )?;
                for i in 0..20 {
                    table.insert(Row::new(vec![Value::String(format!("row-{i}"))]), xid)?;
                }
                tx_manager.commit(xid);
                assert_eq!(table.rows(xid)?.len(), 20);
                Ok(())
            }));
        }

        for handle in handles {
            handle.join().unwrap()?;
        }
        Ok(())
    }

    #[test]
    fn deadlock_is_detected_not_hung() -> Result<(), StorageError> {
        let dir = tempfile::tempdir().unwrap();
        let tx_manager = Arc::new(
            TransactionManager::open(dir.path().join("tx.log")).expect("transaction log must open"),
        );

        let setup = tx_manager.begin();
        let table = Arc::new(Mutex::new(Table::open(
            "users",
            vec![col("name", ColumnType::String)],
            dir.path(),
            Arc::clone(&tx_manager),
        )?));
        let (page_row1, slot_row1) = {
            let mut t = table.lock().unwrap();
            t.insert(Row::new(vec![Value::String("row1".to_string())]), setup)?
        };
        let (page_row2, slot_row2) = {
            let mut t = table.lock().unwrap();
            t.insert(Row::new(vec![Value::String("row2".to_string())]), setup)?
        };
        tx_manager.commit(setup);

        let xid_a = tx_manager.begin();
        let xid_b = tx_manager.begin();

        delete_at(&table, page_row1, slot_row1, xid_a)?;
        delete_at(&table, page_row2, slot_row2, xid_b)?;

        let table_a = Arc::clone(&table);
        let handle_a = thread::spawn(move || delete_at(&table_a, page_row2, slot_row2, xid_a));

        thread::sleep(Duration::from_millis(50));

        let table_b = Arc::clone(&table);
        let handle_b = thread::spawn(move || delete_at(&table_b, page_row1, slot_row1, xid_b));

        let result_a = handle_a.join().unwrap();
        let result_b = handle_b.join().unwrap();

        let deadlocks = [&result_a, &result_b]
            .iter()
            .filter(|r| matches!(r, Err(StorageError::Deadlock)))
            .count();
        assert_eq!(
            deadlocks, 1,
            "exactly one side of the cycle must be caught as a deadlock, not both hanging"
        );

        Ok(())
    }

    #[test]
    fn recovers_delete_correctly_not_duplicated_after_simulated_crash() -> Result<(), StorageError>
    {
        let dir = tempfile::tempdir().unwrap();
        let tx_manager = Arc::new(
            TransactionManager::open(dir.path().join("tx.log")).expect("transaction log must open"),
        );

        let setup = tx_manager.begin();
        let (page_index, slot) = {
            let table = Arc::new(Mutex::new(Table::open(
                "users",
                vec![col("name", ColumnType::String)],
                dir.path(),
                Arc::clone(&tx_manager),
            )?));
            let loc = {
                let mut t = table.lock().unwrap();
                t.insert(Row::new(vec![Value::String("alice".to_string())]), setup)?
            };
            tx_manager.commit(setup);

            let del_xid = tx_manager.begin();
            delete_at(&table, loc.0, loc.1, del_xid)?;
            tx_manager.commit(del_xid);
            loc
            // table dropped here, no explicit flush — simulates a crash right
            // after delete_at's write returned.
        };

        let mut recovered = Table::open(
            "users",
            vec![col("name", ColumnType::String)],
            dir.path(),
            Arc::clone(&tx_manager),
        )?;
        let all_rows = recovered.all_rows_raw()?;

        assert_eq!(
            all_rows.len(),
            1,
            "the row must be updated in place, never duplicated into a second slot"
        );
        assert_eq!(
            all_rows[0].0,
            (page_index, slot),
            "must stay at its original location"
        );
        assert_ne!(
            all_rows[0].1.xmax, 0,
            "xmax must correctly show the delete after recovery"
        );

        Ok(())
    }
}
