use crate::error::StorageError;
use crate::table::Table;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug)]
pub struct Catalog {
    data_dir: PathBuf,
    tables: HashMap<String, Table>,
}

impl Catalog {
    pub fn open(data_dir: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let data_dir = data_dir.into();
        std::fs::create_dir_all(&data_dir)?;

        Ok(Self {
            data_dir,
            tables: HashMap::new(),
        })
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

        let path = self.data_dir.join(format!("{name}.tbl"));
        let table = Table::open(name.clone(), columns, path)?;
        self.tables.insert(name, table);
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

    fn temp_catalog() -> (tempfile::TempDir, Catalog) {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(dir.path()).unwrap();
        (dir, catalog)
    }

    #[test]
    fn create_table_rejects_duplicate() {
        let (_dir, mut catalog) = temp_catalog();
        catalog
            .create_table("users", vec!["name".to_string()])
            .unwrap();

        let result = catalog.create_table("users", vec!["id".to_string()]);

        assert!(matches!(result, Err(StorageError::TableAlreadyExists(name)) if name == "users"));
    }

    #[test]
    fn get_missing_table_errors() {
        let (_dir, catalog) = temp_catalog();
        let result = catalog.get("ghost");

        assert!(matches!(result, Err(StorageError::TableNotFound(name)) if name == "ghost"));
    }
}
