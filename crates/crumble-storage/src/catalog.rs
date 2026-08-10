use crate::error::StorageError;
use crate::table::Table;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct Catalog {
    tables: HashMap<String, Table>,
}

impl Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_table(
        &mut self,
        name: impl Into<String>,
        columns: Vec<String>,
    ) -> Result<(), StorageError> {
        let name = name.into();
        if self.tables.contains_key(&name) {
            return Err(StorageError::TableAlreadyExists(name));
        }

        self.tables.insert(name.clone(), Table::new(name, columns));
        Ok(())
    }

    pub fn get(&self, name: &str) -> Result<&Table, StorageError> {
        self.tables
            .get(name)
            .ok_or_else(|| StorageError::TableNotFound(name.to_string()))
    }

    pub fn get_mut(&mut self, name: &str) -> Result<&mut Table, StorageError> {
        self.tables
            .get_mut(name)
            .ok_or_else(|| StorageError::TableNotFound(name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_table_rejects_duplicate() {
        let mut catalog = Catalog::new();
        catalog
            .create_table("users", vec!["name".to_string()])
            .unwrap();

        let result = catalog.create_table("users", vec!["id".to_string()]);

        assert!(matches!(result, Err(StorageError::TableAlreadyExists(name)) if name == "users"));
    }

    #[test]
    fn get_missing_table_errors() {
        let catalog = Catalog::new();
        let result = catalog.get("ghost");

        assert!(matches!(result, Err(StorageError::TableNotFound(name)) if name == "ghost"));
    }
}
