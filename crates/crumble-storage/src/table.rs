use crate::error::StorageError;
use crate::page::Page;
use crate::row::Row;

#[derive(Debug, Clone)]
pub struct Table {
    name: String,
    columns: Vec<String>,
    pages: Vec<Page>,
}

impl Table {
    // lets callers pass either &str or String
    pub fn new(name: impl Into<String>, columns: Vec<String>) -> Self {
        Self {
            name: name.into(),
            columns,
            pages: Vec::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    pub fn rows(&self) -> Result<Vec<Row>, StorageError> {
        let mut rows = Vec::new();

        for page in &self.pages {
            for slot in 0..page.slot_count() {
                let bytes = page
                    .get_row(slot)
                    .expect("slot index within slot_count must be valid");
                rows.push(Row::from_bytes(&bytes)?);
            }
        }
        Ok(rows)
    }

    /**
    this is the simplest possible page-allocation policy ("append-only, one page at a time")
    A real engine tracks free space across all pages to avoid wasted space in earlier pages; we're not doing that yet
    */
    pub fn insert(&mut self, row: Row) -> Result<(), StorageError> {
        if row.values().len() != self.columns().len() {
            return Err(StorageError::ColumnCountMismatch {
                expected: self.columns().len(),
                actual: row.values().len(),
            });
        }
        let bytes = row.to_bytes()?;

        if let Some(page) = self.pages.last_mut() {
            if page.insert_row(&bytes).is_some() {
                return Ok(());
            }
        }

        let mut page = Page::new();

        if page.insert_row(&bytes).is_none() {
            return Err(StorageError::RowTooLarge);
        }

        self.pages.push(page);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;

    #[test]
    fn insert_rejects_wrong_column_count() {
        let mut table = Table::new("users", vec!["name".to_string()]);
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
        let mut table = Table::new("users", vec!["name".to_string()]);
        table.insert(Row::new(vec![Value::String("alice".to_string())]))?;

        assert_eq!(table.pages.len(), 1);
        Ok(())
    }
}
