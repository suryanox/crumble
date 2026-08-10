use crate::error::StorageError;
use crate::row::Row;

#[derive(Debug, Clone)]
pub struct Table {
    name: String,
    columns: Vec<String>,
    rows: Vec<Row>,
}

impl Table {

    // lets callers pass either &str or String
    pub fn new(name: impl Into<String>, columns: Vec<String>) -> Self {
        Self {
            name: name.into(),
            columns,
            rows: Vec::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn insert(&mut self, row: Row) -> Result<(), StorageError> {
        if row.values().len() != self.columns().len() {
            return Err(StorageError::ColumnCountMismatch {
                expected: self.columns().len(),
                actual: row.values().len(),
            });
        }
        self.rows.push(row);
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

        assert_eq!(table.rows().len(), 1);
        Ok(())
    }
}