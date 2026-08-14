use crate::error::StorageError;
use crate::table::Table;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, Deserialize)]
struct CatalogMeta {
    tables: HashMap<String, Vec<String>>,
}

#[derive(Debug)]
pub struct Catalog {
    data_dir: PathBuf,
    tables: HashMap<String, Table>,
}

impl Catalog {
    pub fn open(data_dir: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let data_dir = data_dir.into();
        std::fs::create_dir_all(&data_dir)?;

        let meta = Self::load_meta(&data_dir)?;
        let mut tables = HashMap::new();

        for (name, columns) in meta.tables {
            let table = Table::open(name.clone(), columns, &data_dir)?;
            tables.insert(name, table);
        }

        Ok(Self { data_dir, tables })
    }

    fn meta_path(data_dir: &std::path::Path) -> PathBuf {
        data_dir.join("catalog.json") // JSON, not bincode, for this file specifically different reasoning than pages/WAL. Catalog metadata is small, written rarely (only on CREATE TABLE)
    }

    fn load_meta(data_dir: &std::path::Path) -> Result<CatalogMeta, StorageError> {
        let path = Self::meta_path(data_dir);

        match std::fs::read_to_string(&path) {
            Ok(contents) => {
                serde_json::from_str(&contents).map_err(|e| StorageError::Encoding(e.to_string()))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(CatalogMeta::default()),
            Err(err) => Err(err.into()),
        }
    }

    fn save_meta(&self) -> Result<(), StorageError> {
        let meta = CatalogMeta {
            tables: self
                .tables
                .iter()
                .map(|(name, table)| (name.clone(), table.columns().to_vec()))
                .collect(),
        };

        let contents = serde_json::to_string_pretty(&meta)
            .map_err(|e| StorageError::Encoding(e.to_string()))?;
        std::fs::write(Self::meta_path(&self.data_dir), contents)?;
        Ok(())
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

        let table = Table::open(name.clone(), columns, &self.data_dir)?;
        self.tables.insert(name, table);
        self.save_meta()
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
