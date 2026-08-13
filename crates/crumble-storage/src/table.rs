use std::path::Path;

use crate::error::StorageError;
use crate::page::Page;
use crate::page_store::PageStore;
use crate::row::Row;

#[derive(Debug)]
pub struct Table {
    name: String,
    columns: Vec<String>,
    store: PageStore,
}

impl Table {
    pub fn open(
        name: impl Into<String>,
        columns: Vec<String>,
        path: impl AsRef<Path>,
    ) -> Result<Self, StorageError> {
        let store = PageStore::open(path)?;

        Ok(Self {
            name: name.into(),
            columns,
            store,
        })
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
        let page_count = self.store.page_count()?;

        if page_count > 0 {
            let last_index = page_count - 1;
            let mut page = self.store.read_page(last_index)?;

            if page.insert_row(&bytes).is_some() {
                return self.store.write_page(last_index, &page);
            }
        }

        let mut page = Page::new();
        if page.insert_row(&bytes).is_none() {
            return Err(StorageError::RowTooLarge);
        }
        self.store.write_page(page_count, &page)
    }

    pub fn rows(&mut self) -> Result<Vec<Row>, StorageError> {
        let mut rows = Vec::new();
        let page_count = self.store.page_count()?;

        for page_index in 0..page_count {
            let page = self.store.read_page(page_index)?;

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
        let table = Table::open("users", columns, path).unwrap();
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
